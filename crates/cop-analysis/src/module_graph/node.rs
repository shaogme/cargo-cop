use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use cop_model::SourceSpan;
use cop_syntax::{AstNodeId, Attribute, ParseOutput, SyntaxNode};

use super::lint_attributes::ConfiguredAttribute;
use crate::{cfg_eval::CfgDecision, ConfiguredAst};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModuleNodeData {
    pub(super) context_index: usize,
    pub(super) cfg_test: bool,
    pub(super) package_name: String,
    pub(super) package_manifest_path: PathBuf,
    pub(super) target_name: String,
    pub(super) target_kind: Vec<String>,
    pub(super) target_triple: String,
    pub(super) file_path: PathBuf,
    pub(super) display_path: PathBuf,
    pub(super) namespace: Vec<String>,
    pub(super) parent_namespace: Vec<String>,
    pub(super) declared_at: Option<SourceSpan>,
    pub(super) cfg_condition: Option<String>,
    pub(super) source_cfg_condition: Option<String>,
    pub(super) no_std: bool,
    pub(super) no_implicit_prelude: bool,
    pub(super) macro_use_context: bool,
    pub(super) inherited_macro_definitions: BTreeSet<String>,
    pub(super) source_instance: usize,
    pub(super) macro_dependency_packages: BTreeMap<String, String>,
    pub(super) module_dir: Option<PathBuf>,
    pub(super) crate_root: bool,
    pub(super) inline: bool,
    pub(super) order_only: bool,
    pub(super) scope_id: AstNodeId,
    pub(super) lint_relaxations: Vec<ConfiguredAttribute>,
    pub(super) configured_ast: ConfiguredAst,
    pub(super) source: Arc<ParseOutput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleNode {
    data: ModuleNodeData,
}

impl ModuleNode {
    pub(super) fn new(data: ModuleNodeData) -> Self {
        Self { data }
    }

    pub(super) fn module_dir(&self) -> Option<&Path> {
        self.data.module_dir.as_deref()
    }

    pub(super) fn source_cfg_condition(&self) -> Option<&str> {
        self.data.source_cfg_condition.as_deref()
    }

    pub(super) fn inherited_macro_definitions(&self) -> &BTreeSet<String> {
        &self.data.inherited_macro_definitions
    }

    pub(super) fn update_syntax(
        &mut self,
        scope_id: AstNodeId,
        source: Arc<ParseOutput>,
        configured_ast: ConfiguredAst,
    ) {
        self.data.scope_id = scope_id;
        self.data.source = source;
        self.data.configured_ast = configured_ast;
    }

    pub fn context_index(&self) -> usize {
        self.data.context_index
    }

    pub fn cfg_test(&self) -> bool {
        self.data.cfg_test
    }

    pub fn package_name(&self) -> &str {
        &self.data.package_name
    }

    pub fn package_manifest_path(&self) -> &Path {
        &self.data.package_manifest_path
    }

    pub fn target_name(&self) -> &str {
        &self.data.target_name
    }

    pub fn target_kind(&self) -> &[String] {
        &self.data.target_kind
    }

    pub fn target_triple(&self) -> &str {
        &self.data.target_triple
    }

    pub fn file_path(&self) -> &Path {
        &self.data.file_path
    }

    pub fn display_path(&self) -> &Path {
        &self.data.display_path
    }

    pub fn namespace(&self) -> &[String] {
        &self.data.namespace
    }

    pub fn parent_namespace(&self) -> &[String] {
        &self.data.parent_namespace
    }

    pub fn declared_at(&self) -> Option<&SourceSpan> {
        self.data.declared_at.as_ref()
    }

    pub fn cfg_condition(&self) -> Option<&str> {
        self.data.cfg_condition.as_deref()
    }

    pub(crate) fn is_no_std(&self) -> bool {
        self.data.no_std
    }

    pub(crate) fn no_implicit_prelude(&self) -> bool {
        self.data.no_implicit_prelude
    }

    pub(crate) fn macro_use_context(&self) -> bool {
        self.data.macro_use_context
    }

    pub(crate) fn source_instance(&self) -> usize {
        self.data.source_instance
    }

    pub(crate) fn macro_dependency_package(&self, crate_name: &str) -> Option<&str> {
        self.data
            .macro_dependency_packages
            .get(crate_name)
            .map(String::as_str)
    }

    pub fn is_crate_root(&self) -> bool {
        self.data.crate_root
    }

    pub fn is_inline(&self) -> bool {
        self.data.inline
    }

    pub fn is_order_only(&self) -> bool {
        self.data.order_only
    }

    pub fn source_item_nodes(&self) -> Vec<&SyntaxNode> {
        let Some(scope) = self.data.source.node(self.data.scope_id) else {
            return Vec::new();
        };
        scope
            .children()
            .iter()
            .filter_map(|id| self.data.source.node(*id))
            .filter(|node| node.item_syntax().is_some())
            .collect()
    }

    pub fn item_nodes(&self) -> Vec<&SyntaxNode> {
        self.source_item_nodes()
            .into_iter()
            .filter(|node| {
                self.data
                    .configured_ast
                    .node(node.id())
                    .is_some_and(|configured| {
                        configured.availability_decision() != CfgDecision::False
                    })
            })
            .collect()
    }

    pub fn scope_inner_attributes(&self) -> &[Attribute] {
        self.data
            .source
            .node(self.data.scope_id)
            .map_or(&[], SyntaxNode::attributes)
    }

    pub fn scope_range(&self) -> Range<usize> {
        self.data
            .source
            .node(self.data.scope_id)
            .map(|node| node.start()..node.end())
            .unwrap_or(0..0)
    }

    pub(crate) fn scope_id(&self) -> AstNodeId {
        self.data.scope_id
    }

    pub fn lint_relaxations(&self) -> &[ConfiguredAttribute] {
        &self.data.lint_relaxations
    }

    pub fn configured_ast(&self) -> &ConfiguredAst {
        &self.data.configured_ast
    }

    pub fn parse_output(&self) -> &ParseOutput {
        &self.data.source
    }
}
