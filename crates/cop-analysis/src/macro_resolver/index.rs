use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use cop_syntax::{AstNodeId, ItemSyntaxKind, ParseOutput, SyntaxNode, SyntaxNodeKind};

use crate::module_graph::ModuleNode;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IndexedUse {
    pub(super) node_id: AstNodeId,
    pub(super) path: Option<Vec<String>>,
    pub(super) is_glob: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IndexedMacroUseKind {
    ExternCrate,
    Module,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IndexedMacroUse {
    pub(super) node_id: AstNodeId,
    pub(super) start: usize,
    pub(super) kind: IndexedMacroUseKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IndexedStdImport {
    pub(super) node_id: AstNodeId,
    pub(super) crate_name: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ScopeMacroIndex {
    pub(super) definitions: BTreeMap<String, Vec<(usize, AstNodeId)>>,
    pub(super) root_shadows: BTreeMap<String, Vec<AstNodeId>>,
    pub(super) uses: BTreeMap<String, Vec<IndexedUse>>,
    pub(super) glob_uses: Vec<IndexedUse>,
    pub(super) whole_uses: Vec<AstNodeId>,
    pub(super) macro_use_items: Vec<IndexedMacroUse>,
    pub(super) std_imports: Vec<IndexedStdImport>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MacroResolverIndex {
    node_scopes: BTreeMap<AstNodeId, Option<AstNodeId>>,
    scope_parents: BTreeMap<AstNodeId, Option<AstNodeId>>,
    scopes: BTreeMap<AstNodeId, ScopeMacroIndex>,
    macro_invocations: Vec<AstNodeId>,
    inherited_macro_use: bool,
    inherited_macro_definitions: BTreeSet<String>,
}

pub(crate) struct MacroResolverCatalog {
    indexes: BTreeMap<usize, Arc<MacroResolverIndex>>,
}

impl MacroResolverCatalog {
    pub(crate) fn new(modules: &[ModuleNode]) -> Self {
        let mut indexes = BTreeMap::new();
        for module in modules {
            indexes
                .entry(module.source_instance())
                .or_insert_with(|| Arc::clone(module.configured_ast().macro_resolver_index()));
        }
        Self { indexes }
    }

    pub(super) fn index(&self, source_instance: usize) -> Option<&MacroResolverIndex> {
        self.indexes.get(&source_instance).map(Arc::as_ref)
    }
}

impl MacroResolverIndex {
    pub(crate) fn set_inherited_macro_use(&mut self, inherited_macro_use: bool) {
        self.inherited_macro_use = inherited_macro_use;
    }

    pub(super) fn inherited_macro_use(&self) -> bool {
        self.inherited_macro_use
    }

    pub(crate) fn set_inherited_macro_definitions(
        &mut self,
        inherited_macro_definitions: BTreeSet<String>,
    ) {
        self.inherited_macro_definitions = inherited_macro_definitions;
    }

    pub(super) fn inherited_macro_definitions(&self) -> &BTreeSet<String> {
        &self.inherited_macro_definitions
    }

    pub(crate) fn add_node(&mut self, output: &ParseOutput, node: &SyntaxNode) {
        let parent_scope = node
            .parent()
            .and_then(|parent| self.node_scopes.get(&parent).copied().flatten());
        let is_scope = matches!(
            node.kind(),
            SyntaxNodeKind::Scope | SyntaxNodeKind::Crate | SyntaxNodeKind::Block
        );
        let nearest_scope = if is_scope {
            Some(node.id())
        } else {
            parent_scope
        };
        self.node_scopes.insert(node.id(), nearest_scope);
        if is_scope {
            self.scope_parents.insert(node.id(), parent_scope);
            self.scopes.entry(node.id()).or_default();
        }
        if node.macro_invocation().is_some() {
            self.macro_invocations.push(node.id());
        }
        let Some(scope) = nearest_scope else {
            return;
        };
        let entries = self.scopes.entry(scope).or_default();
        if let Some(ItemSyntaxKind::MacroDefinition(definition)) =
            node.item_syntax().map(|item| item.kind())
        {
            entries
                .definitions
                .entry(definition.name().to_owned())
                .or_default()
                .push((definition.start(), node.id()));
        }
        if let Some(item) = node.item_syntax() {
            match item.kind() {
                ItemSyntaxKind::Module { name, .. } => {
                    entries
                        .root_shadows
                        .entry(name.clone())
                        .or_default()
                        .push(node.id());
                    if Self::could_have_macro_use(node) {
                        entries.macro_use_items.push(IndexedMacroUse {
                            node_id: node.id(),
                            start: node.start(),
                            kind: IndexedMacroUseKind::Module,
                        });
                    }
                }
                ItemSyntaxKind::Struct(Some(name))
                | ItemSyntaxKind::Enum(Some(name))
                | ItemSyntaxKind::Union(Some(name))
                | ItemSyntaxKind::Trait(Some(name))
                | ItemSyntaxKind::TypeAlias(Some(name)) => {
                    entries
                        .root_shadows
                        .entry(name.clone())
                        .or_default()
                        .push(node.id());
                }
                ItemSyntaxKind::ExternCrate { name, alias } => {
                    let root = alias.as_deref().or(name.as_deref());
                    if let Some(root) = root {
                        entries
                            .root_shadows
                            .entry(root.to_owned())
                            .or_default()
                            .push(node.id());
                    }
                    if let Some(crate_name) = name.as_deref().filter(|name| *name == "std") {
                        entries.std_imports.push(IndexedStdImport {
                            node_id: node.id(),
                            crate_name: crate_name.to_owned(),
                        });
                    }
                    if Self::could_have_macro_use(node) {
                        entries.macro_use_items.push(IndexedMacroUse {
                            node_id: node.id(),
                            start: node.start(),
                            kind: IndexedMacroUseKind::ExternCrate,
                        });
                    }
                }
                ItemSyntaxKind::Use if node.use_tree().is_none() => {
                    entries.whole_uses.push(node.id());
                }
                _ => {}
            }
        }
        if let Some(tree) = node.use_tree() {
            let path = (!tree.is_glob()).then(|| {
                Self::use_path(output, node)
                    .unwrap_or_default()
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            });
            let indexed = IndexedUse {
                node_id: node.id(),
                path,
                is_glob: tree.is_glob(),
            };
            if tree.is_glob() {
                entries.glob_uses.push(indexed);
            } else {
                let import_name = tree.alias().map(str::to_owned).or_else(|| {
                    indexed
                        .path
                        .as_ref()
                        .and_then(|path| path.last().map(String::as_str).map(str::to_owned))
                });
                let shadow_name = tree.alias().map(str::to_owned).or_else(|| {
                    indexed.path.as_ref().and_then(|path| {
                        if path.last().is_some_and(|segment| segment == "self") {
                            path.get(path.len().saturating_sub(2)).cloned()
                        } else {
                            path.last().cloned()
                        }
                    })
                });
                if let Some(shadow_name) = shadow_name {
                    entries
                        .root_shadows
                        .entry(shadow_name)
                        .or_default()
                        .push(node.id());
                }
                if let Some(import_name) = import_name {
                    entries.uses.entry(import_name).or_default().push(indexed);
                }
            }
        }
    }

    pub(crate) fn macro_invocations(&self) -> &[AstNodeId] {
        &self.macro_invocations
    }

    pub(super) fn scope_chain(&self, node_id: AstNodeId) -> Vec<AstNodeId> {
        let mut chain = Vec::new();
        let mut current = self.node_scopes.get(&node_id).copied().flatten();
        while let Some(scope) = current {
            chain.push(scope);
            current = self.scope_parents.get(&scope).copied().flatten();
        }
        chain
    }

    pub(super) fn scope_ids(&self, node_id: AstNodeId) -> BTreeSet<AstNodeId> {
        self.scope_chain(node_id).into_iter().collect()
    }

    pub(super) fn scope(&self, scope_id: AstNodeId) -> Option<&ScopeMacroIndex> {
        self.scopes.get(&scope_id)
    }

    fn could_have_macro_use(node: &SyntaxNode) -> bool {
        node.attributes().iter().any(|attribute| {
            let path = attribute
                .meta()
                .path()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            path == ["macro_use"] || path == ["cfg_attr"]
        })
    }

    fn use_path<'a>(output: &'a ParseOutput, node: &'a SyntaxNode) -> Option<Vec<&'a str>> {
        let mut path_nodes = Vec::new();
        let mut current = Some(node.id());
        while let Some(id) = current {
            let candidate = output.node(id)?;
            if candidate.use_tree().is_some() {
                path_nodes.push(candidate);
            }
            current = candidate.parent();
        }
        path_nodes.reverse();
        let mut path = Vec::new();
        for path_node in path_nodes {
            let tree = path_node.use_tree()?;
            path.extend(tree.path().iter().map(String::as_str));
            if tree.is_glob() {
                return None;
            }
        }
        Some(path)
    }
}
