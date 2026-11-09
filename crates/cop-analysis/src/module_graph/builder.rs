use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

use cop_cargo::{DependencyKind, SelectedPackage, TargetContext};
use cop_i18n::{t, RenderError};
use cop_model::{AnalysisIssueKind, Diagnostic, Severity};
use cop_syntax::{
    ItemSyntaxKind, ParseOptions, ParseOutput, RustParser, SourceFile, SyntaxNode, SyntaxNodeKind,
};

use super::{
    issue_reporter::GraphIssueReporter,
    lint_attributes::LintAttributeCollector,
    macro_scan::MacroScanner,
    node::{ModuleNode, ModuleNodeData},
    scope::{
        IssueRequest, ModulePathRequest, ModuleScope, ModuleVisit, PreviousScope, ResolvedModule,
        ScopeSupport,
    },
};
use crate::{
    cfg_eval::{CfgDecision, CfgEvaluator},
    macro_resolver::{MacroResolver, MacroResolverCatalog},
    ConfiguredAst,
};

pub(super) struct GraphBuilder<'a> {
    package: &'a SelectedPackage,
    target: &'a TargetContext,
    context_index: usize,
    nodes: Vec<ModuleNode>,
    issues: GraphIssueReporter<'a>,
    macro_dependency_packages: BTreeMap<String, String>,
    parsed: BTreeMap<PathBuf, Arc<ParseOutput>>,
    configured: BTreeMap<usize, ConfiguredAst>,
    active_files: BTreeSet<PathBuf>,
    next_source_instance: usize,
}

fn macro_dependency_packages(
    package: &SelectedPackage,
    target: &TargetContext,
) -> BTreeMap<String, String> {
    let evaluator = CfgEvaluator::new(target);
    let build_script = target.kind().iter().any(|kind| kind == "custom-build");
    package
        .dependencies()
        .dependencies()
        .iter()
        .filter(|dependency| {
            let active_kind = match dependency.kind() {
                DependencyKind::Normal => !build_script,
                DependencyKind::Development => target.cfg_test(),
                DependencyKind::Build => build_script,
                DependencyKind::Other(_) => false,
            };
            active_kind
                && (!dependency.is_optional()
                    || target
                        .features()
                        .activated_dependencies()
                        .contains(dependency.dependency_key()))
                && dependency.target_condition().is_none_or(|condition| {
                    evaluator.dependency_condition(condition) == CfgDecision::True
                })
        })
        .map(|dependency| {
            (
                dependency.rust_name().to_owned(),
                dependency.package_name().to_owned(),
            )
        })
        .collect()
}

impl<'a> GraphBuilder<'a> {
    pub(super) fn build_graph(
        package: &'a SelectedPackage,
        target: &'a TargetContext,
        context_index: usize,
    ) -> Result<(Vec<ModuleNode>, Vec<Diagnostic>), RenderError> {
        Self {
            package,
            target,
            context_index,
            nodes: Vec::new(),
            issues: GraphIssueReporter::new(package, target),
            macro_dependency_packages: macro_dependency_packages(package, target),
            parsed: BTreeMap::new(),
            configured: BTreeMap::new(),
            active_files: BTreeSet::new(),
            next_source_instance: 1,
        }
        .build()
    }

    fn build(mut self) -> Result<(Vec<ModuleNode>, Vec<Diagnostic>), RenderError> {
        let root = ScopeSupport::normalize_path(self.target.crate_root());
        let Some(source) = self.load_source(root.clone())? else {
            return Ok((self.nodes, self.issues.into_diagnostics()));
        };
        let root_scope_id = source
            .syntax_nodes()
            .iter()
            .find(|node| node.kind() == SyntaxNodeKind::Crate)
            .map(SyntaxNode::id)
            .expect("解析结果包含 crate 根节点");
        let root_scope = ModuleScope {
            file_path: root.clone(),
            module_dir: Some(root.parent().unwrap_or(Path::new(".")).to_path_buf()),
            namespace: Vec::new(),
            declared_at: None,
            cfg_condition: None,
            source_cfg_condition: None,
            is_root: true,
            is_inline: false,
            active: true,
            no_implicit_prelude: false,
            macro_use_context: false,
            inherited_macro_definitions: BTreeSet::new(),
            source_instance: 0,
            scope_id: root_scope_id,
            source,
        };
        self.active_files.insert(root);
        self.walk_scope(root_scope)?;
        self.finalize_macro_profiles()?;
        Ok((self.nodes, self.issues.into_diagnostics()))
    }

    fn walk_scope(&mut self, mut scope: ModuleScope) -> Result<(), RenderError> {
        let source = scope.source.clone();
        let configured_ast = self
            .configured
            .entry(scope.source_instance)
            .or_insert_with(|| {
                ConfiguredAst::project(
                    &source,
                    self.target,
                    scope.source_cfg_condition.as_deref(),
                    scope.macro_use_context,
                    scope.inherited_macro_definitions.clone(),
                )
            })
            .clone();
        let Some(scope_node) = source.node(scope.scope_id) else {
            return Ok(());
        };
        let scope_effects = configured_ast
            .node(scope_node.id())
            .map(|node| node.attribute_evaluation().clone())
            .unwrap_or_default();
        scope.no_implicit_prelude |= scope_effects.has_active_attribute("no_implicit_prelude");
        let no_std = scope.is_root && scope_effects.has_active_attribute("no_std");
        let scope_condition = configured_ast
            .node(scope_node.id())
            .and_then(|node| node.availability_condition())
            .map(str::to_owned);
        scope.active &= scope_effects.decision() != Some(CfgDecision::False);
        scope.active &= configured_ast
            .node(scope_node.id())
            .is_some_and(|node| node.availability_decision() != CfgDecision::False);
        scope.cfg_condition = scope_condition;
        self.issues
            .report_parse_issues(&scope.file_path, &source, &configured_ast)?;
        self.issues
            .report_configured_ast_issues(&scope, &configured_ast)?;
        for item in ScopeSupport::scope_children(&source, scope.scope_id) {
            let Some(item_data) = item.item_syntax() else {
                continue;
            };
            let configured_item = configured_ast.node(item.id());
            let effects = configured_item
                .map(|node| node.attribute_evaluation().clone())
                .unwrap_or_default();
            let effective_condition = configured_item
                .and_then(|node| node.availability_condition())
                .map(str::to_owned);
            let item_enabled = configured_item
                .is_some_and(|node| node.availability_decision() != CfgDecision::False);
            if !scope.active {
                if let ItemSyntaxKind::Module { name, inline } = item_data.kind() {
                    if *inline {
                        self.visit_module(ModuleVisit {
                            scope: &scope,
                            configured_ast: &configured_ast,
                            item,
                            name,
                            inline: *inline,
                            effects,
                            effective_condition,
                        })?;
                    }
                }
                continue;
            }
            if !item_enabled {
                if let ItemSyntaxKind::Module { name, inline } = item_data.kind() {
                    if *inline {
                        self.visit_module(ModuleVisit {
                            scope: &scope,
                            configured_ast: &configured_ast,
                            item,
                            name,
                            inline: *inline,
                            effects,
                            effective_condition,
                        })?;
                    }
                }
                continue;
            }
            for invocation in MacroScanner::include_invocations(item, &source, &configured_ast) {
                self.issues.report(IssueRequest {
                    kind: AnalysisIssueKind::DynamicInput,
                    severity: Severity::Warning,
                    message: t!("analysis.include_dynamic_input")?,
                    span: source
                        .source()
                        .span(invocation.start(), invocation.end())
                        .ok(),
                    namespace: Some(ScopeSupport::module_namespace(&scope.namespace)),
                    cfg_condition: scope.cfg_condition.clone(),
                });
            }
            if let ItemSyntaxKind::Module { name, inline } = item_data.kind() {
                self.visit_module(ModuleVisit {
                    scope: &scope,
                    configured_ast: &configured_ast,
                    item,
                    name,
                    inline: *inline,
                    effects,
                    effective_condition,
                })?;
            } else if let ItemSyntaxKind::Macro(invocation) = item_data.kind() {
                let macro_name = invocation
                    .path()
                    .segments()
                    .last()
                    .map(|segment| segment.name())
                    .unwrap_or("");
                let issue_kind = if macro_name == "include" {
                    continue;
                } else {
                    AnalysisIssueKind::MacroGeneratedModule
                };
                self.issues.report(IssueRequest {
                    kind: issue_kind,
                    severity: Severity::Warning,
                    message: t!("analysis.macro_generated_module", { name: macro_name })?,
                    span: source
                        .source()
                        .span(invocation.start(), invocation.end())
                        .ok(),
                    namespace: Some(ScopeSupport::module_namespace(&scope.namespace)),
                    cfg_condition: scope.cfg_condition.clone(),
                });
            }
        }
        let mut lint_relaxations = LintAttributeCollector::collect(&configured_ast);
        lint_relaxations
            .retain(|attribute| ScopeSupport::lint_attribute_belongs_to_scope(&scope, attribute));
        let node = ModuleNode::new(ModuleNodeData {
            context_index: self.context_index,
            cfg_test: self.target.cfg_test(),
            package_name: self.package.name().to_owned(),
            package_manifest_path: self.package.manifest_path().to_path_buf(),
            target_name: self.target.name().to_owned(),
            target_kind: self.target.kind().to_vec(),
            target_triple: self.target.target_triple().to_owned(),
            file_path: scope.file_path.clone(),
            display_path: ScopeSupport::relative_display(
                self.package.package_root(),
                &scope.file_path,
            ),
            parent_namespace: scope.namespace[..scope.namespace.len().saturating_sub(1)].to_vec(),
            namespace: scope.namespace,
            declared_at: scope.declared_at,
            cfg_condition: scope.cfg_condition,
            source_cfg_condition: scope.source_cfg_condition,
            no_std,
            no_implicit_prelude: scope.no_implicit_prelude,
            macro_use_context: scope.macro_use_context,
            inherited_macro_definitions: scope.inherited_macro_definitions,
            source_instance: scope.source_instance,
            macro_dependency_packages: self.macro_dependency_packages.clone(),
            module_dir: scope.module_dir,
            crate_root: scope.is_root,
            inline: scope.is_inline,
            order_only: !scope.active,
            scope_id: scope.scope_id,
            lint_relaxations,
            configured_ast,
            source: scope.source,
        });
        self.nodes.push(node);
        Ok(())
    }

    fn visit_module(&mut self, visit: ModuleVisit<'_>) -> Result<(), RenderError> {
        let ModuleVisit {
            scope,
            configured_ast,
            item,
            name,
            inline,
            effects,
            effective_condition,
        } = visit;
        let macro_use_context = scope.macro_use_context
            || MacroResolver::has_macro_use_import_for_module(
                configured_ast.macro_resolver_index(),
                configured_ast,
                scope.scope_id,
                item.start(),
            );
        let mut inherited_macro_definitions = scope.inherited_macro_definitions.clone();
        inherited_macro_definitions.extend(MacroResolver::macro_definitions_before(
            configured_ast.macro_resolver_index(),
            configured_ast,
            scope.scope_id,
            item.start(),
        ));
        let mut namespace = scope.namespace.clone();
        namespace.push(name.to_owned());
        let declared_at = scope.source.source().span(item.start(), item.end()).ok();
        let cfg_condition = effective_condition;
        if inline {
            let Some(inline_scope) = item
                .children()
                .iter()
                .filter_map(|id| scope.source.node(*id))
                .find(|node| node.kind() == SyntaxNodeKind::Scope)
            else {
                return Ok(());
            };
            let module_dir = if effects.path_unknown() {
                None
            } else if let Some(path) = effects.path() {
                ScopeSupport::resolve_path_attribute(scope, path)
            } else {
                scope
                    .module_dir
                    .as_ref()
                    .map(|directory| ScopeSupport::normalize_path(&directory.join(name)))
            };
            let source_cfg_condition = scope.source_cfg_condition.clone();
            self.walk_scope(ModuleScope {
                file_path: scope.file_path.clone(),
                module_dir,
                namespace,
                declared_at,
                cfg_condition,
                source_cfg_condition,
                is_root: false,
                is_inline: true,
                active: scope.active && effects.decision() != Some(CfgDecision::False),
                no_implicit_prelude: scope.no_implicit_prelude
                    || effects.has_active_attribute("no_implicit_prelude"),
                macro_use_context,
                inherited_macro_definitions,
                source_instance: scope.source_instance,
                scope_id: inline_scope.id(),
                source: scope.source.clone(),
            })?;
            return Ok(());
        }
        let Some(resolved) = self.module_path(
            scope,
            ModulePathRequest {
                name,
                effects: &effects,
                declaration_span: declared_at.clone(),
                cfg_condition: cfg_condition.clone(),
            },
        )?
        else {
            return Ok(());
        };
        if self.active_files.contains(&resolved.file_path) {
            self.issues.report(IssueRequest {
                kind: AnalysisIssueKind::ModuleCycle,
                severity: Severity::Error,
                message: t!("analysis.module_cycle", { name: name })?,
                span: declared_at,
                namespace: Some(ScopeSupport::module_namespace(&namespace)),
                cfg_condition,
            });
            return Ok(());
        }
        self.active_files.insert(resolved.file_path.clone());
        let Some(source) = self.load_source(resolved.file_path.clone())? else {
            self.active_files.remove(&resolved.file_path);
            return Ok(());
        };
        let source_instance = self.next_source_instance;
        self.next_source_instance += 1;
        let source_cfg_condition = cfg_condition.clone();
        self.walk_scope(ModuleScope {
            file_path: resolved.file_path.clone(),
            module_dir: Some(resolved.module_dir),
            namespace,
            declared_at,
            cfg_condition,
            source_cfg_condition,
            is_root: false,
            is_inline: false,
            active: true,
            no_implicit_prelude: scope.no_implicit_prelude
                || effects.has_active_attribute("no_implicit_prelude"),
            macro_use_context,
            inherited_macro_definitions,
            source_instance,
            scope_id: source
                .syntax_nodes()
                .iter()
                .find(|node| node.kind() == SyntaxNodeKind::Crate)
                .map(SyntaxNode::id)
                .expect("解析结果包含 crate 根节点"),
            source,
        })?;
        self.active_files.remove(&resolved.file_path);
        Ok(())
    }

    fn finalize_macro_profiles(&mut self) -> Result<(), RenderError> {
        let catalog = MacroResolverCatalog::new(&self.nodes);
        let mut instances = BTreeMap::<usize, Vec<usize>>::new();
        for (index, module) in self.nodes.iter().enumerate() {
            instances
                .entry(module.source_instance())
                .or_default()
                .push(index);
        }
        let instance_bindings = instances
            .iter()
            .map(|(instance, indices)| {
                let instance_modules = indices
                    .iter()
                    .map(|index| &self.nodes[*index])
                    .collect::<Vec<_>>();
                let bindings = MacroResolver::profile_bindings(
                    &instance_modules,
                    &self.nodes,
                    &catalog,
                    self.target.edition(),
                );
                (*instance, bindings)
            })
            .collect::<BTreeMap<_, _>>();
        for indices in instances.values() {
            let first = &self.nodes[indices[0]];
            let source = first.parse_output().source().clone();
            let bindings = instance_bindings
                .get(&first.source_instance())
                .cloned()
                .unwrap_or_default();
            let output = Arc::new(RustParser::parse(
                source,
                ParseOptions::new(self.target.edition()).with_macro_profiles(bindings),
            )?);
            let configured_ast = ConfiguredAst::project(
                &output,
                self.target,
                first.source_cfg_condition(),
                first.macro_use_context(),
                first.inherited_macro_definitions().clone(),
            );
            self.configured
                .insert(first.source_instance(), configured_ast.clone());
            for index in indices {
                let previous_scope = self.nodes[*index]
                    .parse_output()
                    .node(self.nodes[*index].scope_id())
                    .map(|node| PreviousScope {
                        kind: node.kind(),
                        bounds: node.start()..node.end(),
                    });
                let scope_id = previous_scope
                    .and_then(|previous| {
                        output
                            .syntax_nodes()
                            .iter()
                            .find(|node| {
                                node.kind() == previous.kind
                                    && (node.start()..node.end()) == previous.bounds
                            })
                            .map(SyntaxNode::id)
                    })
                    .unwrap_or(self.nodes[*index].scope_id());
                let file_path = self.nodes[*index].file_path().to_path_buf();
                let module_scope = ModuleScope {
                    file_path: file_path.clone(),
                    module_dir: self.nodes[*index].module_dir().map(Path::to_path_buf),
                    namespace: self.nodes[*index].namespace().to_vec(),
                    declared_at: self.nodes[*index].declared_at().cloned(),
                    cfg_condition: self.nodes[*index].cfg_condition().map(str::to_owned),
                    source_cfg_condition: self.nodes[*index]
                        .source_cfg_condition()
                        .map(str::to_owned),
                    is_root: self.nodes[*index].is_crate_root(),
                    is_inline: self.nodes[*index].is_inline(),
                    active: !self.nodes[*index].is_order_only(),
                    no_implicit_prelude: self.nodes[*index].no_implicit_prelude(),
                    macro_use_context: self.nodes[*index].macro_use_context(),
                    inherited_macro_definitions: self.nodes[*index]
                        .inherited_macro_definitions()
                        .clone(),
                    source_instance: self.nodes[*index].source_instance(),
                    scope_id,
                    source: output.clone(),
                };
                self.nodes[*index].update_syntax(scope_id, output.clone(), configured_ast.clone());
                self.issues
                    .report_parse_issues(&file_path, &output, &configured_ast)?;
                self.issues
                    .report_configured_ast_issues(&module_scope, &configured_ast)?;
            }
        }
        Ok(())
    }

    fn module_path(
        &mut self,
        scope: &ModuleScope,
        request: ModulePathRequest<'_>,
    ) -> Result<Option<ResolvedModule>, RenderError> {
        if request.effects.path_unknown() {
            return Ok(None);
        }
        if let Some(path) = request.effects.path() {
            let Some(candidate) = ScopeSupport::resolve_path_attribute(scope, path) else {
                return Ok(None);
            };
            let Some(exists) = self.source_file_exists(&candidate)? else {
                return Ok(None);
            };
            if exists {
                let module_dir = candidate.parent().unwrap_or(Path::new(".")).to_path_buf();
                return Ok(Some(ResolvedModule {
                    file_path: candidate,
                    module_dir,
                }));
            }
            self.issues.report(IssueRequest {
                kind: AnalysisIssueKind::MissingModule,
                severity: Severity::Error,
                message: t!("analysis.path_module_missing")?,
                span: request.declaration_span,
                namespace: Some(ScopeSupport::module_namespace(&scope.namespace)),
                cfg_condition: request.cfg_condition.clone(),
            });
            return Ok(None);
        }
        let Some(module_dir) = scope.module_dir.as_ref() else {
            return Ok(None);
        };
        let first = module_dir.join(format!("{}.rs", request.name));
        let second = module_dir.join(request.name).join("mod.rs");
        let Some(first_exists) = self.source_file_exists(&first)? else {
            return Ok(None);
        };
        let Some(second_exists) = self.source_file_exists(&second)? else {
            return Ok(None);
        };
        let resolved = match (first_exists, second_exists) {
            (true, true) => {
                self.issues.report(IssueRequest {
                    kind: AnalysisIssueKind::AmbiguousModule,
                    severity: Severity::Error,
                    message: t!("analysis.module_layout_ambiguous", { name: request.name })?,
                    span: request.declaration_span,
                    namespace: Some(ScopeSupport::module_namespace(&scope.namespace)),
                    cfg_condition: request.cfg_condition,
                });
                None
            }
            (true, false) => Some(ResolvedModule {
                file_path: ScopeSupport::normalize_path(&first),
                module_dir: ScopeSupport::normalize_path(&module_dir.join(request.name)),
            }),
            (false, true) => Some(ResolvedModule {
                file_path: ScopeSupport::normalize_path(&second),
                module_dir: ScopeSupport::normalize_path(second.parent().unwrap_or(module_dir)),
            }),
            (false, false) => {
                self.issues.report(IssueRequest {
                    kind: AnalysisIssueKind::MissingModule,
                    severity: Severity::Error,
                    message: t!("analysis.module_missing", { name: request.name })?,
                    span: request.declaration_span,
                    namespace: Some(ScopeSupport::module_namespace(&scope.namespace)),
                    cfg_condition: request.cfg_condition,
                });
                None
            }
        };
        Ok(resolved)
    }

    fn load_source(&mut self, path: PathBuf) -> Result<Option<Arc<ParseOutput>>, RenderError> {
        let path = ScopeSupport::normalize_path(&path);
        let path_display = path.display().to_string();
        if let Some(source) = self.parsed.get(&path) {
            return Ok(Some(source.clone()));
        }
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                self.issues.report_tool_io(
                    t!("analysis.module_source_read_error", { path: path_display.as_str() })?,
                    &error,
                );
                return Ok(None);
            }
        };
        let output = Arc::new(RustParser::parse_structure(
            SourceFile::new(path.clone(), text),
            ParseOptions::new(self.target.edition()),
        )?);
        self.parsed.insert(path, output.clone());
        Ok(Some(output))
    }

    fn source_file_exists(&mut self, path: &Path) -> Result<Option<bool>, RenderError> {
        let path_display = path.display().to_string();
        match fs::metadata(path) {
            Ok(metadata) => Ok(Some(metadata.is_file())),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Some(false)),
            Err(error) => {
                self.issues.report_tool_io(
                    t!("analysis.module_file_metadata_error", { path: path_display.as_str() })?,
                    &error,
                );
                Ok(None)
            }
        }
    }
}
