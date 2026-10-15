use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::{location::SourceSpan, LocalizedMessage};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RuleId {
    ModuleFileLayout,
    NoLintRelaxation,
    ItemSpacing,
    PrivateChildModules,
    NoParentPrivateDependency,
    ChildModuleLineCount,
    ImplLocality,
    PrivateFieldsForBehaviorTypes,
    ChildPublicFreeFunctions,
    DeclarationBeforeUse,
    ParameterLimit,
    TupleArity,
    NoInlineQualifiedPath,
    MergeCommonPrefixImports,
    ImportsAtScopeStart,
    OutOfLineModulesAtScopeStart,
    CfgUseGroupSpacing,
    CfgInsideUseTree,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownRuleId {
    value: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ToolErrorKind {
    Arguments,
    Manifest,
    Configuration,
    CargoMetadata,
    Io,
    Output,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AnalysisIssueKind {
    ParserCoverage,
    UnknownCfg,
    MacroGeneratedModule,
    DynamicInput,
    MissingModule,
    AmbiguousModule,
    ModuleCycle,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DiagnosticKind {
    Rule(RuleId),
    Tool(ToolErrorKind),
    Analysis(AnalysisIssueKind),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Diagnostic {
    kind: DiagnosticKind,
    severity: Severity,
    package: Option<String>,
    package_manifest_path: Option<PathBuf>,
    target: Option<String>,
    target_kind: Vec<String>,
    span: Option<SourceSpan>,
    message: LocalizedMessage,
    namespace: Option<String>,
    cfg_condition: Option<String>,
}

impl RuleId {
    pub const ALL: [Self; 18] = [
        Self::ModuleFileLayout,
        Self::NoLintRelaxation,
        Self::ItemSpacing,
        Self::PrivateChildModules,
        Self::NoParentPrivateDependency,
        Self::ChildModuleLineCount,
        Self::ImplLocality,
        Self::PrivateFieldsForBehaviorTypes,
        Self::ChildPublicFreeFunctions,
        Self::DeclarationBeforeUse,
        Self::ParameterLimit,
        Self::TupleArity,
        Self::NoInlineQualifiedPath,
        Self::MergeCommonPrefixImports,
        Self::ImportsAtScopeStart,
        Self::OutOfLineModulesAtScopeStart,
        Self::CfgUseGroupSpacing,
        Self::CfgInsideUseTree,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModuleFileLayout => "module_file_layout",
            Self::NoLintRelaxation => "no_lint_relaxation",
            Self::ItemSpacing => "item_spacing",
            Self::PrivateChildModules => "private_child_modules",
            Self::NoParentPrivateDependency => "no_parent_private_dependency",
            Self::ChildModuleLineCount => "child_module_line_count",
            Self::ImplLocality => "impl_locality",
            Self::PrivateFieldsForBehaviorTypes => "private_fields_for_behavior_types",
            Self::ChildPublicFreeFunctions => "child_public_free_functions",
            Self::DeclarationBeforeUse => "declaration_before_use",
            Self::ParameterLimit => "parameter_limit",
            Self::TupleArity => "tuple_arity",
            Self::NoInlineQualifiedPath => "no_inline_qualified_path",
            Self::MergeCommonPrefixImports => "merge_common_prefix_imports",
            Self::ImportsAtScopeStart => "imports_at_scope_start",
            Self::OutOfLineModulesAtScopeStart => "out_of_line_modules_at_scope_start",
            Self::CfgUseGroupSpacing => "cfg_use_group_spacing",
            Self::CfgInsideUseTree => "cfg_inside_use_tree",
        }
    }

    pub fn all() -> &'static [Self] {
        &Self::ALL
    }
}

impl FromStr for RuleId {
    type Err = UnknownRuleId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|rule| rule.as_str() == value)
            .ok_or_else(|| UnknownRuleId {
                value: value.to_owned(),
            })
    }
}

impl UnknownRuleId {
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for UnknownRuleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown rule ID: {}", self.value)
    }
}

impl Error for UnknownRuleId {}

impl fmt::Display for RuleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl ToolErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Arguments => "arguments",
            Self::Manifest => "manifest",
            Self::Configuration => "configuration",
            Self::CargoMetadata => "cargo_metadata",
            Self::Io => "io",
            Self::Output => "output",
        }
    }
}

impl AnalysisIssueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ParserCoverage => "parser_coverage",
            Self::UnknownCfg => "unknown_cfg",
            Self::MacroGeneratedModule => "macro_generated_module",
            Self::DynamicInput => "dynamic_input",
            Self::MissingModule => "missing_module",
            Self::AmbiguousModule => "ambiguous_module",
            Self::ModuleCycle => "module_cycle",
        }
    }
}

impl DiagnosticKind {
    pub fn rule_id(self) -> Option<RuleId> {
        match self {
            Self::Rule(rule_id) => Some(rule_id),
            Self::Tool(_) | Self::Analysis(_) => None,
        }
    }

    pub fn is_tool_error(self) -> bool {
        matches!(self, Self::Tool(_))
    }

    pub fn is_analysis_issue(self) -> bool {
        matches!(self, Self::Analysis(_))
    }
}

impl Diagnostic {
    pub fn rule(rule_id: RuleId, severity: Severity, message: LocalizedMessage) -> Self {
        Self::new(DiagnosticKind::Rule(rule_id), severity, message)
    }

    pub fn tool(error_kind: ToolErrorKind, message: LocalizedMessage) -> Self {
        Self::new(DiagnosticKind::Tool(error_kind), Severity::Error, message)
    }

    pub fn analysis(
        issue_kind: AnalysisIssueKind,
        severity: Severity,
        message: LocalizedMessage,
    ) -> Self {
        Self::new(DiagnosticKind::Analysis(issue_kind), severity, message)
    }

    pub fn with_package(mut self, package: impl Into<String>) -> Self {
        self.package = Some(package.into());
        self
    }

    pub fn with_package_manifest_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.package_manifest_path = Some(path.into());
        self
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn with_target_kind(mut self, target_kind: Vec<String>) -> Self {
        self.target_kind = target_kind;
        self.target_kind.sort();
        self.target_kind.dedup();
        self
    }

    pub fn with_span(mut self, span: SourceSpan) -> Self {
        self.span = Some(span);
        self
    }

    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    pub fn with_cfg_condition(mut self, cfg_condition: impl Into<String>) -> Self {
        self.cfg_condition = Some(cfg_condition.into());
        self
    }

    pub fn kind(&self) -> DiagnosticKind {
        self.kind
    }

    pub fn severity(&self) -> Severity {
        self.severity
    }

    pub fn package(&self) -> Option<&str> {
        self.package.as_deref()
    }

    pub fn package_manifest_path(&self) -> Option<&Path> {
        self.package_manifest_path.as_deref()
    }

    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    pub fn target_kind(&self) -> &[String] {
        &self.target_kind
    }

    pub fn span(&self) -> Option<&SourceSpan> {
        self.span.as_ref()
    }

    pub fn message(&self) -> &LocalizedMessage {
        &self.message
    }

    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    pub fn cfg_condition(&self) -> Option<&str> {
        self.cfg_condition.as_deref()
    }

    fn new(kind: DiagnosticKind, severity: Severity, message: LocalizedMessage) -> Self {
        Self {
            kind,
            severity,
            package: None,
            package_manifest_path: None,
            target: None,
            target_kind: Vec::new(),
            span: None,
            message,
            namespace: None,
            cfg_condition: None,
        }
    }
}
