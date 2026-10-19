use std::{
    fs::canonicalize,
    path::{Path, PathBuf},
    sync::Arc,
};

use cop_model::{AnalysisIssueKind, LocalizedMessage, Severity, SourceSpan};
use cop_syntax::{AstNodeId, ItemSyntaxKind, ParseOutput, SyntaxNode, SyntaxNodeKind};

use super::lint_attributes::ConfiguredAttribute;
use crate::{ConfiguredAstIssue, ConfiguredAttributeEvaluation};

pub(super) struct ModuleScope {
    pub(super) file_path: PathBuf,
    pub(super) module_dir: PathBuf,
    pub(super) namespace: Vec<String>,
    pub(super) inline_path: Vec<String>,
    pub(super) declared_at: Option<SourceSpan>,
    pub(super) cfg_condition: Option<String>,
    pub(super) source_cfg_condition: Option<String>,
    pub(super) is_root: bool,
    pub(super) source_is_root: bool,
    pub(super) is_inline: bool,
    pub(super) active: bool,
    pub(super) scope_id: AstNodeId,
    pub(super) source: Arc<ParseOutput>,
}

pub(super) struct ModuleVisit<'a> {
    pub(super) scope: &'a ModuleScope,
    pub(super) item: &'a SyntaxNode,
    pub(super) name: &'a str,
    pub(super) inline: bool,
    pub(super) effects: ConfiguredAttributeEvaluation,
    pub(super) effective_condition: Option<String>,
}

pub(super) struct ModulePathRequest<'a> {
    pub(super) name: &'a str,
    pub(super) effects: &'a ConfiguredAttributeEvaluation,
    pub(super) declaration_span: Option<SourceSpan>,
    pub(super) cfg_condition: Option<String>,
}

pub(super) struct IssueRequest {
    pub(super) kind: AnalysisIssueKind,
    pub(super) severity: Severity,
    pub(super) message: LocalizedMessage,
    pub(super) span: Option<SourceSpan>,
    pub(super) namespace: Option<String>,
    pub(super) cfg_condition: Option<String>,
}

pub(super) struct ScopeSupport;

impl ScopeSupport {
    pub(super) fn module_namespace(namespace: &[String]) -> String {
        if namespace.is_empty() {
            "crate".to_owned()
        } else {
            format!("crate::{}", namespace.join("::"))
        }
    }

    pub(super) fn scope_children(output: &ParseOutput, scope_id: AstNodeId) -> Vec<&SyntaxNode> {
        output
            .node(scope_id)
            .into_iter()
            .flat_map(SyntaxNode::children)
            .filter_map(|id| output.node(*id))
            .collect()
    }

    pub(super) fn issue_belongs_to_scope(scope: &ModuleScope, issue: &ConfiguredAstIssue) -> bool {
        let contains_issue =
            |start: usize, end: usize| start <= issue.start() && issue.end() <= end;
        if scope
            .source
            .node(scope.scope_id)
            .into_iter()
            .flat_map(SyntaxNode::attributes)
            .any(|attribute| contains_issue(attribute.start(), attribute.end()))
        {
            return true;
        }
        Self::scope_children(&scope.source, scope.scope_id)
            .into_iter()
            .any(|item| {
                if item
                    .attributes()
                    .iter()
                    .any(|attribute| contains_issue(attribute.start(), attribute.end()))
                {
                    return true;
                }
                if !contains_issue(item.start(), item.end()) {
                    return false;
                }
                !item.item_syntax().is_some_and(|data| {
                    matches!(data.kind(), ItemSyntaxKind::Module { inline: true, .. })
                })
            })
    }

    pub(super) fn lint_attribute_belongs_to_scope(
        scope: &ModuleScope,
        attribute: &ConfiguredAttribute,
    ) -> bool {
        let contains_attribute =
            |start: usize, end: usize| start <= attribute.start() && attribute.end() <= end;
        if !scope.is_inline
            && scope.source.node(scope.scope_id).is_some_and(|node| {
                node.attributes()
                    .iter()
                    .any(|inner| contains_attribute(inner.start(), inner.end()))
            })
        {
            return true;
        }
        Self::scope_children(&scope.source, scope.scope_id)
            .into_iter()
            .any(|item| {
                if item.attributes().iter().any(|item_attribute| {
                    contains_attribute(item_attribute.start(), item_attribute.end())
                }) {
                    return true;
                }
                if !contains_attribute(item.start(), item.end()) {
                    return false;
                }
                match item.item_syntax().map(|data| data.kind()) {
                    Some(ItemSyntaxKind::Module { inline: true, .. }) => item
                        .children()
                        .iter()
                        .filter_map(|id| scope.source.node(*id))
                        .filter(|node| node.kind() == SyntaxNodeKind::Scope)
                        .flat_map(SyntaxNode::attributes)
                        .any(|inner| contains_attribute(inner.start(), inner.end())),
                    Some(ItemSyntaxKind::Module { .. }) => false,
                    _ => true,
                }
            })
    }

    pub(super) fn normalize_path(path: &Path) -> PathBuf {
        canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    pub(super) fn relative_display(root: &Path, path: &Path) -> PathBuf {
        path.strip_prefix(root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.to_path_buf())
    }

    pub(super) fn path_attribute_base(scope: &ModuleScope) -> PathBuf {
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
}
