use std::{
    borrow::Cow,
    collections::BTreeSet,
    io::Error,
    path::{Path, PathBuf},
};

use cop_cargo::{SelectedPackage, TargetContext};
use cop_i18n::{t, RenderError};
use cop_model::{AnalysisIssueKind, Diagnostic, Severity, ToolErrorKind};
use cop_syntax::{CoverageStatus, ParseIssueCode, ParseOutput, SyntaxNodeKind};

use super::scope::{IssueRequest, ModuleScope, ScopeSupport};
use crate::{ConfiguredAst, ConfiguredAstIssueKind};

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
    message: Cow<'static, str>,
}

pub(super) struct GraphIssueReporter<'a> {
    package: &'a SelectedPackage,
    target: &'a TargetContext,
    diagnostics: Vec<Diagnostic>,
    reported_parse: BTreeSet<ParseIssueKey>,
    reported_unknown: BTreeSet<UnknownIssueKey>,
    reported_malformed: BTreeSet<MalformedIssueKey>,
}

impl<'a> GraphIssueReporter<'a> {
    pub(super) fn new(package: &'a SelectedPackage, target: &'a TargetContext) -> Self {
        Self {
            package,
            target,
            diagnostics: Vec::new(),
            reported_parse: BTreeSet::new(),
            reported_unknown: BTreeSet::new(),
            reported_malformed: BTreeSet::new(),
        }
    }

    pub(super) fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    pub(super) fn report_tool_io(&mut self, message: Cow<'static, str>, error: &Error) {
        self.diagnostics.push(
            Diagnostic::tool(ToolErrorKind::Io, message)
                .with_package(self.package.name())
                .with_package_manifest_path(self.package.manifest_path())
                .with_target(self.target.name())
                .with_target_kind(self.target.kind().to_vec())
                .with_target_triple(self.target.target_triple())
                .with_detail(error.to_string()),
        );
    }

    pub(super) fn report_parse_issues(
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
                message: issue.message_cow()?,
                span: output.source().span(issue.start(), issue.end()).ok(),
                namespace: None,
                cfg_condition: None,
            });
        }
        Ok(())
    }

    pub(super) fn report_configured_ast_issues(
        &mut self,
        scope: &ModuleScope,
        configured_ast: &ConfiguredAst,
    ) -> Result<(), RenderError> {
        for issue in configured_ast.issues() {
            if !ScopeSupport::issue_belongs_to_scope(scope, issue) {
                continue;
            }
            let details = match issue.kind() {
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
                kind: details.kind,
                severity: details.severity,
                message: details.message,
                span: scope.source.source().span(issue.start(), issue.end()).ok(),
                namespace: Some(ScopeSupport::module_namespace(&scope.namespace)),
                cfg_condition: issue.effective_condition().map(str::to_owned),
            });
        }
        Ok(())
    }

    pub(super) fn report(&mut self, issue: IssueRequest) {
        let mut diagnostic = Diagnostic::analysis(issue.kind, issue.severity, issue.message)
            .with_package(self.package.name())
            .with_package_manifest_path(self.package.manifest_path())
            .with_target(self.target.name())
            .with_target_kind(self.target.kind().to_vec())
            .with_target_triple(self.target.target_triple());
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
