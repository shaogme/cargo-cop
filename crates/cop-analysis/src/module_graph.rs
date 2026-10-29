mod lint_attributes;
mod macro_scan;
mod scope;

use cop_i18n::{t, RenderError};

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use cop_cargo::{SelectedPackage, TargetContext};
use cop_model::{AnalysisIssueKind, Diagnostic, Severity, SourceSpan, ToolErrorKind};
use cop_syntax::{
    AstNodeId, Attribute, CoverageStatus, ItemSyntaxKind, ParseIssueCode, ParseOptions,
    ParseOutput, RustParser, SourceFile, SyntaxNode, SyntaxNodeKind,
};

pub use self::lint_attributes::ConfiguredAttribute;
use self::{
    lint_attributes::LintAttributeCollector,
    macro_scan::MacroScanner,
    scope::{IssueRequest, ModulePathRequest, ModuleScope, ModuleVisit, ScopeSupport},
};
use crate::{cfg_eval::CfgDecision, ConfiguredAst, ConfiguredAstIssueKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleNode {
    context_index: usize,
    package_name: String,
    package_manifest_path: PathBuf,
    target_name: String,
    target_kind: Vec<String>,
    file_path: PathBuf,
    display_path: PathBuf,
    namespace: Vec<String>,
    parent_namespace: Vec<String>,
    declared_at: Option<SourceSpan>,
    cfg_condition: Option<String>,
    crate_root: bool,
    inline: bool,
    order_only: bool,
    scope_id: AstNodeId,
    lint_relaxations: Vec<ConfiguredAttribute>,
    configured_ast: ConfiguredAst,
    source: Arc<ParseOutput>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ModuleGraph {
    nodes: Vec<ModuleNode>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ParseIssueKey {
    path: PathBuf,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct UnknownIssueKey {
    path: PathBuf,
    start: usize,
    end: usize,
    condition: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct MalformedIssueKey {
    path: PathBuf,
    start: usize,
    end: usize,
}

struct ConfiguredIssueDetails {
    kind: AnalysisIssueKind,
    severity: Severity,
    message: String,
}

struct GraphBuilder<'a> {
    package: &'a SelectedPackage,
    target: &'a TargetContext,
    context_index: usize,
    nodes: Vec<ModuleNode>,
    diagnostics: Vec<Diagnostic>,
    parsed: BTreeMap<PathBuf, Arc<ParseOutput>>,
    reported_parse: BTreeSet<ParseIssueKey>,
    reported_unknown: BTreeSet<UnknownIssueKey>,
    reported_malformed: BTreeSet<MalformedIssueKey>,
    active_files: BTreeSet<PathBuf>,
}

impl ModuleNode {
    pub fn context_index(&self) -> usize {
        self.context_index
    }

    pub fn package_name(&self) -> &str {
        &self.package_name
    }

    pub fn package_manifest_path(&self) -> &Path {
        &self.package_manifest_path
    }

    pub fn target_name(&self) -> &str {
        &self.target_name
    }

    pub fn target_kind(&self) -> &[String] {
        &self.target_kind
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    pub fn display_path(&self) -> &Path {
        &self.display_path
    }

    pub fn namespace(&self) -> &[String] {
        &self.namespace
    }

    pub fn parent_namespace(&self) -> &[String] {
        &self.parent_namespace
    }

    pub fn declared_at(&self) -> Option<&SourceSpan> {
        self.declared_at.as_ref()
    }

    pub fn cfg_condition(&self) -> Option<&str> {
        self.cfg_condition.as_deref()
    }

    pub fn is_crate_root(&self) -> bool {
        self.crate_root
    }

    pub fn is_inline(&self) -> bool {
        self.inline
    }

    pub fn is_order_only(&self) -> bool {
        self.order_only
    }

    pub fn source_item_nodes(&self) -> Vec<&SyntaxNode> {
        let Some(scope) = self.source.node(self.scope_id) else {
            return Vec::new();
        };
        scope
            .children()
            .iter()
            .filter_map(|id| self.source.node(*id))
            .filter(|node| node.item_syntax().is_some())
            .collect()
    }

    pub fn item_nodes(&self) -> Vec<&SyntaxNode> {
        self.source_item_nodes()
            .into_iter()
            .filter(|node| {
                self.configured_ast
                    .node(node.id())
                    .is_some_and(|configured| {
                        configured.availability_decision() != CfgDecision::False
                    })
            })
            .collect()
    }

    pub fn scope_inner_attributes(&self) -> &[Attribute] {
        self.source
            .node(self.scope_id)
            .map_or(&[], SyntaxNode::attributes)
    }

    pub fn scope_range(&self) -> Range<usize> {
        self.source
            .node(self.scope_id)
            .map(|node| node.start()..node.end())
            .unwrap_or(0..0)
    }

    pub(crate) fn scope_id(&self) -> AstNodeId {
        self.scope_id
    }

    pub fn lint_relaxations(&self) -> &[ConfiguredAttribute] {
        &self.lint_relaxations
    }

    pub fn configured_ast(&self) -> &ConfiguredAst {
        &self.configured_ast
    }

    pub fn parse_output(&self) -> &ParseOutput {
        &self.source
    }
}

impl ModuleGraph {
    pub(crate) fn build(
        package: &SelectedPackage,
        target: &TargetContext,
        context_index: usize,
    ) -> Result<Self, RenderError> {
        GraphBuilder {
            package,
            target,
            context_index,
            nodes: Vec::new(),
            diagnostics: Vec::new(),
            parsed: BTreeMap::new(),
            reported_parse: BTreeSet::new(),
            reported_unknown: BTreeSet::new(),
            reported_malformed: BTreeSet::new(),
            active_files: BTreeSet::new(),
        }
        .build()
    }

    pub(crate) fn into_parts(self) -> (Vec<ModuleNode>, Vec<Diagnostic>) {
        (self.nodes, self.diagnostics)
    }
}

impl GraphBuilder<'_> {
    fn build(mut self) -> Result<ModuleGraph, RenderError> {
        let root = ScopeSupport::normalize_path(self.target.crate_root());
        let Some(source) = self.load_source(root.clone())? else {
            return Ok(ModuleGraph {
                nodes: self.nodes,
                diagnostics: self.diagnostics,
            });
        };
        let root_scope_id = source
            .syntax_nodes()
            .iter()
            .find(|node| node.kind() == SyntaxNodeKind::Crate)
            .map(SyntaxNode::id)
            .expect("解析结果包含 crate 根节点");
        let root_scope = ModuleScope {
            file_path: root.clone(),
            module_dir: root.parent().unwrap_or(Path::new(".")).to_path_buf(),
            namespace: Vec::new(),
            inline_path: Vec::new(),
            declared_at: None,
            cfg_condition: None,
            source_cfg_condition: None,
            is_root: true,
            source_is_root: true,
            is_inline: false,
            active: true,
            scope_id: root_scope_id,
            source,
        };
        self.active_files.insert(root);
        self.walk_scope(root_scope)?;
        Ok(ModuleGraph {
            nodes: self.nodes,
            diagnostics: self.diagnostics,
        })
    }

    fn walk_scope(&mut self, mut scope: ModuleScope) -> Result<(), RenderError> {
        let source = scope.source.clone();
        let configured_ast =
            ConfiguredAst::project(&source, self.target, scope.source_cfg_condition.as_deref());
        let Some(scope_node) = source.node(scope.scope_id) else {
            return Ok(());
        };
        let scope_effects = configured_ast
            .node(scope_node.id())
            .map(|node| node.attribute_evaluation().clone())
            .unwrap_or_default();
        let scope_condition = configured_ast
            .node(scope_node.id())
            .and_then(|node| node.availability_condition())
            .map(str::to_owned);
        scope.active &= scope_effects.decision() != Some(CfgDecision::False);
        scope.active &= configured_ast
            .node(scope_node.id())
            .is_some_and(|node| node.availability_decision() != CfgDecision::False);
        scope.cfg_condition = scope_condition;
        self.report_parse_issues(&scope.file_path, &source, &configured_ast)?;
        self.report_configured_ast_issues(&scope, &configured_ast)?;
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
                self.report(IssueRequest {
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
                self.report(IssueRequest {
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
        let node = ModuleNode {
            context_index: self.context_index,
            package_name: self.package.name().to_owned(),
            package_manifest_path: self.package.manifest_path().to_path_buf(),
            target_name: self.target.name().to_owned(),
            target_kind: self.target.kind().to_vec(),
            file_path: scope.file_path.clone(),
            display_path: ScopeSupport::relative_display(
                self.package.package_root(),
                &scope.file_path,
            ),
            parent_namespace: scope.namespace[..scope.namespace.len().saturating_sub(1)].to_vec(),
            namespace: scope.namespace,
            declared_at: scope.declared_at,
            cfg_condition: scope.cfg_condition,
            crate_root: scope.is_root,
            inline: scope.is_inline,
            order_only: !scope.active,
            scope_id: scope.scope_id,
            lint_relaxations,
            configured_ast,
            source: scope.source,
        };
        self.nodes.push(node);
        Ok(())
    }

    fn visit_module(&mut self, visit: ModuleVisit<'_>) -> Result<(), RenderError> {
        let ModuleVisit {
            scope,
            item,
            name,
            inline,
            effects,
            effective_condition,
        } = visit;
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
            let mut inline_path = scope.inline_path.clone();
            inline_path.push(name.to_owned());
            let source_cfg_condition = scope.source_cfg_condition.clone();
            self.walk_scope(ModuleScope {
                file_path: scope.file_path.clone(),
                module_dir: scope.module_dir.join(name),
                namespace,
                inline_path,
                declared_at,
                cfg_condition,
                source_cfg_condition,
                is_root: false,
                source_is_root: scope.source_is_root,
                is_inline: true,
                active: scope.active && effects.decision() != Some(CfgDecision::False),
                scope_id: inline_scope.id(),
                source: scope.source.clone(),
            })?;
            return Ok(());
        }
        let Some(path) = self.module_path(
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
        if self.active_files.contains(&path) {
            self.report(IssueRequest {
                kind: AnalysisIssueKind::ModuleCycle,
                severity: Severity::Error,
                message: t!("analysis.module_cycle", { name: name })?,
                span: declared_at,
                namespace: Some(ScopeSupport::module_namespace(&namespace)),
                cfg_condition,
            });
            return Ok(());
        }
        self.active_files.insert(path.clone());
        let Some(source) = self.load_source(path.clone())? else {
            self.active_files.remove(&path);
            return Ok(());
        };
        let source_cfg_condition = cfg_condition.clone();
        self.walk_scope(ModuleScope {
            file_path: path.clone(),
            module_dir: scope.module_dir.join(name),
            namespace,
            inline_path: Vec::new(),
            declared_at,
            cfg_condition,
            source_cfg_condition,
            is_root: false,
            source_is_root: false,
            is_inline: false,
            active: true,
            scope_id: source
                .syntax_nodes()
                .iter()
                .find(|node| node.kind() == SyntaxNodeKind::Crate)
                .map(SyntaxNode::id)
                .expect("解析结果包含 crate 根节点"),
            source,
        })?;
        self.active_files.remove(&path);
        Ok(())
    }

    fn module_path(
        &mut self,
        scope: &ModuleScope,
        request: ModulePathRequest<'_>,
    ) -> Result<Option<PathBuf>, RenderError> {
        if request.effects.path_unknown() {
            return Ok(None);
        }
        if let Some(path) = request.effects.path() {
            let base = ScopeSupport::path_attribute_base(scope);
            let candidate = ScopeSupport::normalize_path(&base.join(path));
            let Some(exists) = self.source_file_exists(&candidate)? else {
                return Ok(None);
            };
            if exists {
                return Ok(Some(candidate));
            }
            self.report(IssueRequest {
                kind: AnalysisIssueKind::MissingModule,
                severity: Severity::Error,
                message: t!("analysis.path_module_missing")?,
                span: request.declaration_span,
                namespace: Some(ScopeSupport::module_namespace(&scope.namespace)),
                cfg_condition: request.cfg_condition.clone(),
            });
            return Ok(None);
        }
        let first = scope.module_dir.join(format!("{}.rs", request.name));
        let second = scope.module_dir.join(request.name).join("mod.rs");
        let Some(first_exists) = self.source_file_exists(&first)? else {
            return Ok(None);
        };
        let Some(second_exists) = self.source_file_exists(&second)? else {
            return Ok(None);
        };
        let path = match (first_exists, second_exists) {
            (true, true) => {
                self.report(IssueRequest {
                    kind: AnalysisIssueKind::AmbiguousModule,
                    severity: Severity::Error,
                    message: t!("analysis.module_layout_ambiguous", { name: request.name })?,
                    span: request.declaration_span,
                    namespace: Some(ScopeSupport::module_namespace(&scope.namespace)),
                    cfg_condition: request.cfg_condition,
                });
                None
            }
            (true, false) => Some(ScopeSupport::normalize_path(&first)),
            (false, true) => Some(ScopeSupport::normalize_path(&second)),
            (false, false) => {
                self.report(IssueRequest {
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
        Ok(path)
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
                self.diagnostics.push(
                    Diagnostic::tool(
                        ToolErrorKind::Io,
                        t!("analysis.module_source_read_error", { path: path_display.as_str() })?,
                    )
                    .with_package(self.package.name())
                    .with_package_manifest_path(self.package.manifest_path())
                    .with_target(self.target.name())
                    .with_target_kind(self.target.kind().to_vec())
                    .with_detail(error.to_string()),
                );
                return Ok(None);
            }
        };
        let output = Arc::new(RustParser::parse(
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
                self.diagnostics.push(
                    Diagnostic::tool(
                        ToolErrorKind::Io,
                        t!("analysis.module_file_metadata_error", { path: path_display.as_str() })?,
                    )
                    .with_package(self.package.name())
                    .with_package_manifest_path(self.package.manifest_path())
                    .with_target(self.target.name())
                    .with_target_kind(self.target.kind().to_vec())
                    .with_detail(error.to_string()),
                );
                Ok(None)
            }
        }
    }

    fn report_parse_issues(
        &mut self,
        path: &Path,
        output: &ParseOutput,
        configured_ast: &ConfiguredAst,
    ) -> Result<(), RenderError> {
        if output.coverage() == CoverageStatus::Complete {
            return Ok(());
        }
        for issue in output.issues() {
            if !configured_ast.is_issue_enabled(issue) {
                continue;
            }
            if issue.code() == ParseIssueCode::OpaqueMacroInput
                && output.syntax_nodes().iter().any(|node| {
                    node.kind() == SyntaxNodeKind::Macro
                        && node.start() <= issue.start()
                        && issue.end() <= node.end()
                        && node.macro_invocation().is_some_and(|invocation| {
                            invocation
                                .path()
                                .segments()
                                .last()
                                .is_some_and(|segment| segment.name() == "include")
                        })
                })
            {
                continue;
            }
            let key = ParseIssueKey {
                path: path.to_path_buf(),
                start: issue.start(),
                end: issue.end(),
            };
            if !self.reported_parse.insert(key) {
                continue;
            }
            self.report(IssueRequest {
                kind: AnalysisIssueKind::ParserCoverage,
                severity: match issue.code() {
                    ParseIssueCode::UnsupportedSyntax(_) | ParseIssueCode::OpaqueMacroInput => {
                        Severity::Warning
                    }
                    ParseIssueCode::LexicalError
                    | ParseIssueCode::UnexpectedToken
                    | ParseIssueCode::UnexpectedEof
                    | ParseIssueCode::UnclosedDelimiter
                    | ParseIssueCode::InvalidAttribute
                    | ParseIssueCode::UnsupportedEdition => Severity::Error,
                },
                message: issue.message()?.to_owned(),
                span: output.source().span(issue.start(), issue.end()).ok(),
                namespace: None,
                cfg_condition: None,
            });
        }
        Ok(())
    }

    fn report_configured_ast_issues(
        &mut self,
        scope: &ModuleScope,
        configured_ast: &ConfiguredAst,
    ) -> Result<(), RenderError> {
        for issue in configured_ast.issues() {
            if !ScopeSupport::issue_belongs_to_scope(scope, issue) {
                continue;
            }
            let kind = match issue.kind() {
                ConfiguredAstIssueKind::UnknownCfg => {
                    let condition = issue.condition().unwrap_or_default();
                    if !self.reported_unknown.insert(UnknownIssueKey {
                        path: scope.file_path.clone(),
                        start: issue.start(),
                        end: issue.end(),
                        condition: condition.to_owned(),
                    }) {
                        continue;
                    }
                    ConfiguredIssueDetails {
                        kind: AnalysisIssueKind::UnknownCfg,
                        severity: Severity::Warning,
                        message: t!("analysis.unknown_cfg", { condition: condition })?,
                    }
                }
                ConfiguredAstIssueKind::MalformedAttribute => {
                    if !self.reported_malformed.insert(MalformedIssueKey {
                        path: scope.file_path.clone(),
                        start: issue.start(),
                        end: issue.end(),
                    }) {
                        continue;
                    }
                    ConfiguredIssueDetails {
                        kind: AnalysisIssueKind::ParserCoverage,
                        severity: Severity::Error,
                        message: t!("analysis.malformed_cfg_attribute")?,
                    }
                }
                ConfiguredAstIssueKind::MalformedCfgExpression => {
                    if !self.reported_malformed.insert(MalformedIssueKey {
                        path: scope.file_path.clone(),
                        start: issue.start(),
                        end: issue.end(),
                    }) {
                        continue;
                    }
                    ConfiguredIssueDetails {
                        kind: AnalysisIssueKind::ParserCoverage,
                        severity: Severity::Error,
                        message: t!("analysis.malformed_cfg_expression")?,
                    }
                }
                ConfiguredAstIssueKind::OpaqueCfgMacro => {
                    if !self.reported_parse.insert(ParseIssueKey {
                        path: scope.file_path.clone(),
                        start: issue.start(),
                        end: issue.end(),
                    }) {
                        continue;
                    }
                    ConfiguredIssueDetails {
                        kind: AnalysisIssueKind::ParserCoverage,
                        severity: Severity::Warning,
                        message: t!("analysis.cfg_macro_identity_unknown")?,
                    }
                }
            };
            self.report(IssueRequest {
                kind: kind.kind,
                severity: kind.severity,
                message: kind.message,
                span: scope.source.source().span(issue.start(), issue.end()).ok(),
                namespace: Some(ScopeSupport::module_namespace(&scope.namespace)),
                cfg_condition: issue.effective_condition().map(str::to_owned),
            });
        }
        Ok(())
    }

    fn report(&mut self, issue: IssueRequest) {
        let mut diagnostic = Diagnostic::analysis(issue.kind, issue.severity, issue.message)
            .with_package(self.package.name())
            .with_package_manifest_path(self.package.manifest_path())
            .with_target(self.target.name())
            .with_target_kind(self.target.kind().to_vec());
        if let Some(span) = issue.span {
            diagnostic = diagnostic.with_span(span);
        }
        if let Some(namespace) = issue.namespace {
            diagnostic = diagnostic.with_namespace(namespace);
        }
        if let Some(condition) = issue.cfg_condition {
            diagnostic = diagnostic.with_cfg_condition(condition);
        }
        self.diagnostics.push(diagnostic);
    }
}
