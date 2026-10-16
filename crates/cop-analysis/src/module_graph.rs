use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

use cop_cargo::{SelectedPackage, TargetContext};
use cop_model::{
    AnalysisIssueKind, Diagnostic, LocalizedMessage, Severity, SourceSpan, ToolErrorKind,
    INCLUDE_DYNAMIC_INPUT, MACRO_GENERATED_MODULE, MALFORMED_CFG_ATTRIBUTE, MODULE_CYCLE,
    MODULE_FILE_METADATA_ERROR, MODULE_LAYOUT_AMBIGUOUS, MODULE_MISSING, MODULE_SOURCE_READ_ERROR,
    PATH_MODULE_MISSING, UNKNOWN_CFG,
};
use cop_syntax::{
    Attribute, CoverageStatus, FunctionBody, Item, ItemKind, MacroInvocation, ModuleItem,
    NamedDeclaration, ParseIssueKind, ParseOutput, RustParser, SourceFile, SyntaxNode,
};

use crate::cfg_eval::{AttributeEffects, CfgDecision, CfgEvaluator};

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
    items: Vec<Item>,
    source_items: Vec<Item>,
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

struct GraphBuilder<'a> {
    package: &'a SelectedPackage,
    target: &'a TargetContext,
    context_index: usize,
    nodes: Vec<ModuleNode>,
    diagnostics: Vec<Diagnostic>,
    parsed: BTreeMap<PathBuf, Arc<ParseOutput>>,
    reported_parse: BTreeSet<ParseIssueKey>,
    active_files: BTreeSet<PathBuf>,
}

struct ModuleScope {
    file_path: PathBuf,
    module_dir: PathBuf,
    namespace: Vec<String>,
    inline_path: Vec<String>,
    declared_at: Option<SourceSpan>,
    cfg_condition: Option<String>,
    is_root: bool,
    source_is_root: bool,
    is_inline: bool,
    active: bool,
    items: Vec<Item>,
    inner_attributes: Vec<Attribute>,
    source: Arc<ParseOutput>,
}

struct ModuleVisit<'a> {
    scope: &'a ModuleScope,
    item: &'a Item,
    module: &'a ModuleItem,
    effects: AttributeEffects,
}

struct ModulePathRequest<'a> {
    name: &'a str,
    effects: &'a AttributeEffects,
    declaration_span: Option<SourceSpan>,
}

struct IssueRequest {
    kind: AnalysisIssueKind,
    severity: Severity,
    message: LocalizedMessage,
    span: Option<SourceSpan>,
    namespace: Option<String>,
    cfg_condition: Option<String>,
}

impl ModuleNode {
    pub(crate) fn context_index(&self) -> usize {
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

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn source_items(&self) -> &[Item] {
        &self.source_items
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
    ) -> Self {
        GraphBuilder {
            package,
            target,
            context_index,
            nodes: Vec::new(),
            diagnostics: Vec::new(),
            parsed: BTreeMap::new(),
            reported_parse: BTreeSet::new(),
            active_files: BTreeSet::new(),
        }
        .build()
    }

    pub(crate) fn into_parts(self) -> (Vec<ModuleNode>, Vec<Diagnostic>) {
        (self.nodes, self.diagnostics)
    }
}

impl GraphBuilder<'_> {
    fn build(mut self) -> ModuleGraph {
        let root = normalize_path(self.target.crate_root());
        let Some(source) = self.load_source(root.clone()) else {
            return ModuleGraph {
                nodes: self.nodes,
                diagnostics: self.diagnostics,
            };
        };
        let root_scope = ModuleScope {
            file_path: root.clone(),
            module_dir: root.parent().unwrap_or(Path::new(".")).to_path_buf(),
            namespace: Vec::new(),
            inline_path: Vec::new(),
            declared_at: None,
            cfg_condition: None,
            is_root: true,
            source_is_root: true,
            is_inline: false,
            active: true,
            items: source.items().to_vec(),
            inner_attributes: source.inner_attributes().to_vec(),
            source,
        };
        self.active_files.insert(root);
        self.walk_scope(root_scope);
        ModuleGraph {
            nodes: self.nodes,
            diagnostics: self.diagnostics,
        }
    }

    fn walk_scope(&mut self, mut scope: ModuleScope) {
        let evaluator = CfgEvaluator::new(self.target);
        let scope_effects =
            evaluator.evaluate_attributes(&scope.inner_attributes, scope.source.source());
        if scope.active {
            self.report_effects(&scope, &scope.inner_attributes, &scope_effects);
        }
        scope.active &= scope_effects.decision != Some(CfgDecision::False);
        scope.cfg_condition = merge_conditions(scope.cfg_condition, &scope_effects.conditions);
        let mut active_items = Vec::new();
        let source = scope.source.clone();
        for item in &scope.items {
            if !scope.active {
                if let ItemKind::Module(module) = item.kind() {
                    if module.is_inline() {
                        self.visit_module(ModuleVisit {
                            scope: &scope,
                            item,
                            module,
                            effects: AttributeEffects::default(),
                        });
                    }
                }
                continue;
            }
            let effects = evaluator.evaluate_attributes(item.attributes(), source.source());
            self.report_effects(&scope, item.attributes(), &effects);
            if effects.decision == Some(CfgDecision::False) {
                if let ItemKind::Module(module) = item.kind() {
                    if module.is_inline() {
                        self.visit_module(ModuleVisit {
                            scope: &scope,
                            item,
                            module,
                            effects,
                        });
                    }
                }
                continue;
            }
            active_items.push(item.clone());
            for invocation in item_include_macros(item) {
                self.report(IssueRequest {
                    kind: AnalysisIssueKind::DynamicInput,
                    severity: Severity::Warning,
                    message: LocalizedMessage::new(INCLUDE_DYNAMIC_INPUT),
                    span: source
                        .source()
                        .span(invocation.start(), invocation.end())
                        .ok(),
                    namespace: Some(namespace_string(&scope.namespace)),
                    cfg_condition: scope.cfg_condition.clone(),
                });
            }
            if let ItemKind::Module(module) = item.kind() {
                self.visit_module(ModuleVisit {
                    scope: &scope,
                    item,
                    module,
                    effects,
                });
            } else if let ItemKind::Macro(invocation) = item.kind() {
                let macro_name = invocation
                    .path()
                    .segments()
                    .last()
                    .map(String::as_str)
                    .unwrap_or("");
                let issue_kind = if macro_name == "include" {
                    continue;
                } else {
                    AnalysisIssueKind::MacroGeneratedModule
                };
                self.report(IssueRequest {
                    kind: issue_kind,
                    severity: Severity::Warning,
                    message: LocalizedMessage::new(MACRO_GENERATED_MODULE)
                        .with_text("name", macro_name),
                    span: source
                        .source()
                        .span(invocation.start(), invocation.end())
                        .ok(),
                    namespace: Some(namespace_string(&scope.namespace)),
                    cfg_condition: scope.cfg_condition.clone(),
                });
            }
        }
        let node = ModuleNode {
            context_index: self.context_index,
            package_name: self.package.name().to_owned(),
            package_manifest_path: self.package.manifest_path().to_path_buf(),
            target_name: self.target.name().to_owned(),
            target_kind: self.target.kind().to_vec(),
            file_path: scope.file_path.clone(),
            display_path: relative_display(self.package.package_root(), &scope.file_path),
            parent_namespace: scope.namespace[..scope.namespace.len().saturating_sub(1)].to_vec(),
            namespace: scope.namespace,
            declared_at: scope.declared_at,
            cfg_condition: scope.cfg_condition,
            crate_root: scope.is_root,
            inline: scope.is_inline,
            order_only: !scope.active,
            items: active_items,
            source_items: scope.items,
            source: scope.source,
        };
        self.nodes.push(node);
    }

    fn visit_module(&mut self, visit: ModuleVisit<'_>) {
        let ModuleVisit {
            scope,
            item,
            module,
            effects,
        } = visit;
        let mut namespace = scope.namespace.clone();
        namespace.push(module.name().to_owned());
        let declared_at = scope.source.source().span(item.start(), item.end()).ok();
        let cfg_condition = merge_conditions(scope.cfg_condition.clone(), &effects.conditions);
        if module.is_inline() {
            let mut inline_path = scope.inline_path.clone();
            inline_path.push(module.name().to_owned());
            self.walk_scope(ModuleScope {
                file_path: scope.file_path.clone(),
                module_dir: scope.module_dir.join(module.name()),
                namespace,
                inline_path,
                declared_at,
                cfg_condition,
                is_root: false,
                source_is_root: scope.source_is_root,
                is_inline: true,
                active: scope.active && effects.decision != Some(CfgDecision::False),
                items: module.items().to_vec(),
                inner_attributes: module.inner_attributes().to_vec(),
                source: scope.source.clone(),
            });
            return;
        }
        let Some(path) = self.module_path(
            scope,
            ModulePathRequest {
                name: module.name(),
                effects: &effects,
                declaration_span: declared_at.clone(),
            },
        ) else {
            return;
        };
        if self.active_files.contains(&path) {
            self.report(IssueRequest {
                kind: AnalysisIssueKind::ModuleCycle,
                severity: Severity::Error,
                message: LocalizedMessage::new(MODULE_CYCLE).with_text("name", module.name()),
                span: declared_at,
                namespace: Some(namespace_string(&namespace)),
                cfg_condition,
            });
            return;
        }
        self.active_files.insert(path.clone());
        let Some(source) = self.load_source(path.clone()) else {
            self.active_files.remove(&path);
            return;
        };
        self.walk_scope(ModuleScope {
            file_path: path.clone(),
            module_dir: scope.module_dir.join(module.name()),
            namespace,
            inline_path: Vec::new(),
            declared_at,
            cfg_condition,
            is_root: false,
            source_is_root: false,
            is_inline: false,
            active: true,
            items: source.items().to_vec(),
            inner_attributes: source.inner_attributes().to_vec(),
            source,
        });
        self.active_files.remove(&path);
    }

    fn module_path(
        &mut self,
        scope: &ModuleScope,
        request: ModulePathRequest<'_>,
    ) -> Option<PathBuf> {
        if request.effects.path_unknown {
            return None;
        }
        if let Some(path) = &request.effects.path {
            let base = path_attribute_base(scope);
            let candidate = normalize_path(&base.join(path));
            if self.source_file_exists(&candidate)? {
                return Some(candidate);
            }
            self.report(IssueRequest {
                kind: AnalysisIssueKind::MissingModule,
                severity: Severity::Error,
                message: LocalizedMessage::new(PATH_MODULE_MISSING),
                span: request.declaration_span,
                namespace: Some(namespace_string(&scope.namespace)),
                cfg_condition: scope.cfg_condition.clone(),
            });
            return None;
        }
        let first = scope.module_dir.join(format!("{}.rs", request.name));
        let second = scope.module_dir.join(request.name).join("mod.rs");
        let first_exists = self.source_file_exists(&first)?;
        let second_exists = self.source_file_exists(&second)?;
        match (first_exists, second_exists) {
            (true, true) => {
                self.report(IssueRequest {
                    kind: AnalysisIssueKind::AmbiguousModule,
                    severity: Severity::Error,
                    message: LocalizedMessage::new(MODULE_LAYOUT_AMBIGUOUS)
                        .with_text("name", request.name),
                    span: request.declaration_span,
                    namespace: Some(namespace_string(&scope.namespace)),
                    cfg_condition: scope.cfg_condition.clone(),
                });
                None
            }
            (true, false) => Some(normalize_path(&first)),
            (false, true) => Some(normalize_path(&second)),
            (false, false) => {
                self.report(IssueRequest {
                    kind: AnalysisIssueKind::MissingModule,
                    severity: Severity::Error,
                    message: LocalizedMessage::new(MODULE_MISSING).with_text("name", request.name),
                    span: request.declaration_span,
                    namespace: Some(namespace_string(&scope.namespace)),
                    cfg_condition: scope.cfg_condition.clone(),
                });
                None
            }
        }
    }

    fn load_source(&mut self, path: PathBuf) -> Option<Arc<ParseOutput>> {
        let path = normalize_path(&path);
        if let Some(source) = self.parsed.get(&path) {
            return Some(source.clone());
        }
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                self.diagnostics.push(
                    Diagnostic::tool(
                        ToolErrorKind::Io,
                        LocalizedMessage::new(MODULE_SOURCE_READ_ERROR)
                            .with_text("path", path.display().to_string())
                            .with_opaque("detail", error.to_string()),
                    )
                    .with_package(self.package.name())
                    .with_package_manifest_path(self.package.manifest_path())
                    .with_target(self.target.name())
                    .with_target_kind(self.target.kind().to_vec()),
                );
                return None;
            }
        };
        let output = Arc::new(RustParser::parse(SourceFile::new(path.clone(), text)));
        self.report_parse_issues(&path, &output);
        self.parsed.insert(path, output.clone());
        Some(output)
    }

    fn source_file_exists(&mut self, path: &Path) -> Option<bool> {
        match fs::metadata(path) {
            Ok(metadata) => Some(metadata.is_file()),
            Err(error) if error.kind() == ErrorKind::NotFound => Some(false),
            Err(error) => {
                self.diagnostics.push(
                    Diagnostic::tool(
                        ToolErrorKind::Io,
                        LocalizedMessage::new(MODULE_FILE_METADATA_ERROR)
                            .with_text("path", path.display().to_string())
                            .with_opaque("detail", error.to_string()),
                    )
                    .with_package(self.package.name())
                    .with_package_manifest_path(self.package.manifest_path())
                    .with_target(self.target.name())
                    .with_target_kind(self.target.kind().to_vec()),
                );
                None
            }
        }
    }

    fn report_parse_issues(&mut self, path: &Path, output: &ParseOutput) {
        if output.coverage() == CoverageStatus::Complete {
            return;
        }
        for issue in output.issues() {
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
                severity: match issue.kind() {
                    ParseIssueKind::UnsupportedSyntax => Severity::Warning,
                    ParseIssueKind::Lexical
                    | ParseIssueKind::UnexpectedToken
                    | ParseIssueKind::UnclosedDelimiter => Severity::Error,
                },
                message: issue.message().clone(),
                span: output.source().span(issue.start(), issue.end()).ok(),
                namespace: None,
                cfg_condition: None,
            });
        }
    }

    fn report_effects(
        &mut self,
        scope: &ModuleScope,
        attrs: &[Attribute],
        effects: &AttributeEffects,
    ) {
        if effects.malformed {
            self.report(IssueRequest {
                kind: AnalysisIssueKind::ParserCoverage,
                severity: Severity::Error,
                message: LocalizedMessage::new(MALFORMED_CFG_ATTRIBUTE),
                span: attrs
                    .first()
                    .and_then(|attr| scope.source.source().span(attr.start(), attr.end()).ok()),
                namespace: Some(namespace_string(&scope.namespace)),
                cfg_condition: scope.cfg_condition.clone(),
            });
        }
        for condition in &effects.unknown {
            self.report(IssueRequest {
                kind: AnalysisIssueKind::UnknownCfg,
                severity: Severity::Warning,
                message: LocalizedMessage::new(UNKNOWN_CFG)
                    .with_text("condition", &condition.condition),
                span: scope
                    .source
                    .source()
                    .span(condition.start, condition.end)
                    .ok(),
                namespace: Some(namespace_string(&scope.namespace)),
                cfg_condition: Some(condition.condition.clone()),
            });
        }
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

fn normalize_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn relative_display(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
}

fn namespace_string(namespace: &[String]) -> String {
    if namespace.is_empty() {
        "crate".to_owned()
    } else {
        format!("crate::{}", namespace.join("::"))
    }
}

fn merge_conditions(parent: Option<String>, conditions: &[String]) -> Option<String> {
    let mut all = parent.into_iter().collect::<Vec<_>>();
    all.extend(conditions.iter().cloned());
    (!all.is_empty()).then(|| all.join(" && "))
}

fn item_include_macros(item: &Item) -> Vec<MacroInvocation> {
    let mut invocations = Vec::new();
    match item.kind() {
        ItemKind::Macro(invocation) => collect_include(invocation, &mut invocations),
        ItemKind::Function(function) => {
            collect_syntax_macros(function.signature_nodes(), &mut invocations);
            if let Some(body) = function.body() {
                collect_body_macros(body, &mut invocations);
            }
        }
        ItemKind::Struct(declaration)
        | ItemKind::Enum(declaration)
        | ItemKind::Union(declaration)
        | ItemKind::TypeAlias(declaration)
        | ItemKind::Constant(declaration)
        | ItemKind::Static(declaration) => {
            collect_named_macros(declaration, &mut invocations);
        }
        ItemKind::Trait(trait_item) => {
            for nested in trait_item.items() {
                invocations.extend(item_include_macros(nested));
            }
        }
        ItemKind::Impl(impl_item) => {
            for nested in impl_item.items() {
                invocations.extend(item_include_macros(nested));
            }
        }
        ItemKind::Module(_)
        | ItemKind::MacroDefinition(_)
        | ItemKind::Use(_)
        | ItemKind::ExternCrate(_)
        | ItemKind::Other(_) => {}
    }
    invocations
}

fn collect_body_macros(body: &FunctionBody, invocations: &mut Vec<MacroInvocation>) {
    collect_syntax_macros(body.nodes(), invocations);
}

fn collect_named_macros(declaration: &NamedDeclaration, invocations: &mut Vec<MacroInvocation>) {
    collect_syntax_macros(declaration.nodes(), invocations);
}

fn collect_syntax_macros(nodes: &[SyntaxNode], invocations: &mut Vec<MacroInvocation>) {
    for node in nodes {
        if let SyntaxNode::Macro(invocation) = node {
            collect_include(invocation, invocations);
        }
    }
}

fn collect_include(invocation: &MacroInvocation, invocations: &mut Vec<MacroInvocation>) {
    if invocation
        .path()
        .segments()
        .last()
        .is_some_and(|segment| segment == "include")
    {
        invocations.push(invocation.clone());
    }
}

fn path_attribute_base(scope: &ModuleScope) -> PathBuf {
    let mut base = scope
        .file_path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    if !scope.source_is_root
        && scope
            .file_path
            .file_name()
            .is_some_and(|name| name.to_string_lossy() != "mod.rs")
    {
        if let Some(stem) = scope.file_path.file_stem() {
            base.push(stem);
        }
    }
    base.extend(scope.inline_path.iter());
    base
}
