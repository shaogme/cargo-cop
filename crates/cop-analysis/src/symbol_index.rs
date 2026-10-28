mod index;
mod local_bindings;
mod support;

use std::{
    collections::BTreeMap,
    ops::Range,
    path::{Path, PathBuf},
};

use cop_cargo::{DependencyKind, SelectedPackage};

use cop_model::SourceSpan;
use cop_syntax::{AstNodeId, ForeignItemSyntaxKind, ItemSyntaxKind, SyntaxNode, SyntaxNodeKind};

pub(crate) use support::SymbolIndexSupport;

use crate::{
    cfg_eval::{CfgDecision, CfgEvaluator},
    module_graph::ModuleNode,
};
pub use index::{PathResolution, SymbolIndex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolKind {
    Module,
    Struct,
    Enum,
    Union,
    TypeAlias,
    Trait,
    Constant,
    Static,
    Function,
    AssociatedItem,
    UseAlias,
    ExternalCrate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SymbolVisibility {
    Private,
    Public,
    Restricted(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathResolutionKind {
    Local,
    ExternalCrate,
    Unresolved,
    Ambiguous,
    AliasCycle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisibilityResult {
    Public,
    Reachable,
    Private,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DefinitionId(usize);

impl DefinitionId {
    pub fn index(self) -> usize {
        self.0
    }

    pub(crate) fn new(index: usize) -> Self {
        Self(index)
    }
}

struct SymbolRecordData {
    name: String,
    kind: SymbolKind,
    visibility: SymbolVisibility,
    context_index: usize,
    target_name: String,
    namespace: Vec<String>,
    target_namespace: Option<Vec<String>>,
    target_path: Option<Vec<String>>,
    external_crate: Option<String>,
    file_path: PathBuf,
    span: Option<SourceSpan>,
    cfg_condition: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolRecord {
    definition_id: DefinitionId,
    name: String,
    kind: SymbolKind,
    visibility: SymbolVisibility,
    context_index: usize,
    target_name: String,
    namespace: Vec<String>,
    target_namespace: Option<Vec<String>>,
    target_path: Option<Vec<String>>,
    external_crate: Option<String>,
    file_path: PathBuf,
    span: Option<SourceSpan>,
    cfg_condition: Option<String>,
}

impl SymbolRecord {
    fn new(data: SymbolRecordData) -> Self {
        Self {
            definition_id: DefinitionId(usize::MAX),
            name: data.name,
            kind: data.kind,
            visibility: data.visibility,
            context_index: data.context_index,
            target_name: data.target_name,
            namespace: data.namespace,
            target_namespace: data.target_namespace,
            target_path: data.target_path,
            external_crate: data.external_crate,
            file_path: data.file_path,
            span: data.span,
            cfg_condition: data.cfg_condition,
        }
    }

    pub fn definition_id(&self) -> DefinitionId {
        self.definition_id
    }

    fn set_definition_id(&mut self, definition_id: DefinitionId) {
        self.definition_id = definition_id;
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> SymbolKind {
        self.kind
    }

    pub fn visibility(&self) -> &SymbolVisibility {
        &self.visibility
    }

    pub fn target_name(&self) -> &str {
        &self.target_name
    }

    pub fn context_index(&self) -> usize {
        self.context_index
    }

    pub fn namespace(&self) -> &[String] {
        &self.namespace
    }

    pub fn target_namespace(&self) -> Option<&[String]> {
        self.target_namespace.as_deref()
    }

    pub fn target_path(&self) -> Option<&[String]> {
        self.target_path.as_deref()
    }

    pub fn external_crate(&self) -> Option<&str> {
        self.external_crate.as_deref()
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    pub fn span(&self) -> Option<&SourceSpan> {
        self.span.as_ref()
    }

    pub fn cfg_condition(&self) -> Option<&str> {
        self.cfg_condition.as_deref()
    }

    pub fn is_visible_from(&self, namespace: &[String]) -> VisibilityResult {
        let visibility_namespace = if self.kind == SymbolKind::AssociatedItem {
            &self.namespace[..self.namespace.len().saturating_sub(1)]
        } else {
            &self.namespace
        };
        let Some(visible_scope) = self.visibility.visible_scope(visibility_namespace) else {
            return VisibilityResult::Unknown;
        };
        if matches!(self.visibility, SymbolVisibility::Public) {
            VisibilityResult::Public
        } else if namespace.starts_with(&visible_scope) {
            VisibilityResult::Reachable
        } else {
            VisibilityResult::Private
        }
    }
}

impl SymbolVisibility {
    pub(crate) fn visible_scope(&self, declaration_namespace: &[String]) -> Option<Vec<String>> {
        match self {
            Self::Private => Some(declaration_namespace.to_vec()),
            Self::Public => Some(Vec::new()),
            Self::Restricted(scope) => restricted_namespace(scope, declaration_namespace),
        }
    }
}

fn restricted_namespace(scope: &str, declaration_namespace: &[String]) -> Option<Vec<String>> {
    let restriction = scope.trim_matches(['(', ')', ' ']);
    match restriction {
        "crate" => Some(Vec::new()),
        "self" => Some(declaration_namespace.to_vec()),
        "super" => {
            Some(declaration_namespace[..declaration_namespace.len().saturating_sub(1)].to_vec())
        }
        value if value.starts_with("in ") => {
            let path = value.strip_prefix("in ")?.trim();
            if path == "crate" {
                return Some(Vec::new());
            }
            if path == "self" {
                return Some(declaration_namespace.to_vec());
            }
            let mut namespace = declaration_namespace.to_vec();
            for segment in path.split("::") {
                match segment {
                    "crate" => namespace.clear(),
                    "self" => {}
                    "super" => {
                        namespace.pop()?;
                    }
                    "" => {}
                    name => namespace.push(name.to_owned()),
                }
            }
            Some(namespace)
        }
        _ => None,
    }
}

struct PendingAssociatedSymbol {
    module_index: usize,
    owner_path: Vec<String>,
    owner_bounds: Range<usize>,
    name: String,
    visibility: SymbolVisibility,
    context_index: usize,
    target_name: String,
    file_path: PathBuf,
    span: Option<SourceSpan>,
    cfg_condition: Option<String>,
}

struct NodeSymbolRecord {
    name: String,
    kind: SymbolKind,
    visibility: SymbolVisibility,
    target_namespace: Option<Vec<String>>,
    target_path: Option<Vec<String>>,
    external_crate: Option<String>,
    cfg_condition: Option<String>,
}

pub(crate) struct SymbolIndexBuilder {
    index: SymbolIndex,
    module_conditions: BTreeMap<(usize, Vec<String>), Option<String>>,
    use_prefixes: BTreeMap<AstNodeId, Vec<String>>,
    pending_associated: Vec<PendingAssociatedSymbol>,
}

impl SymbolIndexBuilder {
    pub(crate) fn new(package: &SelectedPackage, modules: &[ModuleNode]) -> Self {
        let mut builder = Self {
            index: SymbolIndex::default(),
            module_conditions: BTreeMap::new(),
            use_prefixes: BTreeMap::new(),
            pending_associated: Vec::new(),
        };
        for module in modules {
            builder.module_conditions.insert(
                (module.context_index(), module.namespace().to_vec()),
                module.cfg_condition().map(str::to_owned),
            );
        }
        builder.add_external_crates(package, modules);
        builder
    }

    pub(crate) fn begin_module(&mut self) {
        self.use_prefixes.clear();
        self.index.local_bindings_mut().begin_module();
    }

    pub(crate) fn visit_node(
        &mut self,
        module_index: usize,
        module: &ModuleNode,
        node: &SyntaxNode,
    ) {
        self.index.local_bindings_mut().visit_node(module, node);
        if !belongs_to_module_scope(module, node) || !node_is_enabled(module, node) {
            return;
        }
        if node.parent() == Some(module.scope_id()) {
            self.collect_root_item(module, node);
        }
        if node.kind() == SyntaxNodeKind::ForeignItem {
            self.collect_foreign_item(module, node);
        }
        if let Some(item) = node.item_syntax() {
            self.collect_associated_item(module_index, module, node, item.kind());
        }
        if node.use_tree().is_some() {
            self.collect_use_tree(module, node);
        }
    }

    pub(crate) fn finish_module(&mut self, module: &ModuleNode) {
        self.index.local_bindings_mut().finish_module(module);
    }

    pub(crate) fn finish(mut self, modules: &[ModuleNode]) -> SymbolIndex {
        for pending in self.pending_associated {
            let Some(module) = modules.get(pending.module_index) else {
                continue;
            };
            let owner = self
                .index
                .resolve_path_at(
                    module,
                    &pending.owner_path,
                    Some(pending.owner_bounds.clone()),
                )
                .symbol()
                .filter(|symbol| {
                    matches!(
                        symbol.kind(),
                        SymbolKind::Struct
                            | SymbolKind::Enum
                            | SymbolKind::Union
                            | SymbolKind::Trait
                    )
                })
                .map(|symbol| symbol.name().to_owned());
            let Some(owner) = owner else {
                continue;
            };
            let mut namespace = module.namespace().to_vec();
            namespace.push(owner);
            self.index.push_symbol(SymbolRecord::new(SymbolRecordData {
                name: pending.name,
                kind: SymbolKind::AssociatedItem,
                visibility: pending.visibility,
                context_index: pending.context_index,
                target_name: pending.target_name,
                namespace,
                target_namespace: None,
                target_path: None,
                external_crate: None,
                file_path: pending.file_path,
                span: pending.span,
                cfg_condition: pending.cfg_condition,
            }));
        }
        self.index.sort_and_assign_definition_ids();
        self.index
    }

    fn collect_root_item(&mut self, module: &ModuleNode, node: &SyntaxNode) {
        let Some(item) = node.item_syntax() else {
            return;
        };
        let item_cfg = configured_condition(module, node);
        match item.kind() {
            ItemSyntaxKind::Module { name, .. } => {
                let mut target_namespace = module.namespace().to_vec();
                target_namespace.push(name.to_owned());
                let cfg_condition = self
                    .module_conditions
                    .get(&(module.context_index(), target_namespace.clone()))
                    .cloned()
                    .flatten()
                    .or(item_cfg);
                self.push_node_symbol(
                    module,
                    node,
                    NodeSymbolRecord {
                        name: name.to_owned(),
                        kind: SymbolKind::Module,
                        visibility: SymbolIndexSupport::visibility(item.visibility().kind()),
                        target_namespace: Some(target_namespace),
                        target_path: None,
                        external_crate: None,
                        cfg_condition,
                    },
                );
            }
            ItemSyntaxKind::ExternCrate { name, alias } => {
                if let Some(crate_name) = name {
                    let local_name = alias.as_deref().unwrap_or(crate_name);
                    self.push_node_symbol(
                        module,
                        node,
                        NodeSymbolRecord {
                            name: local_name.to_owned(),
                            kind: SymbolKind::ExternalCrate,
                            visibility: SymbolIndexSupport::visibility(item.visibility().kind()),
                            target_namespace: None,
                            target_path: None,
                            external_crate: Some(crate_name.to_owned()),
                            cfg_condition: item_cfg,
                        },
                    );
                }
            }
            ItemSyntaxKind::Use | ItemSyntaxKind::ExternBlock { .. } => {}
            kind => {
                if let Some((name, symbol_kind)) = SymbolIndexSupport::named_item(kind) {
                    let target_namespace =
                        SymbolIndexSupport::associated_namespace(kind, module.namespace(), &name);
                    self.push_node_symbol(
                        module,
                        node,
                        NodeSymbolRecord {
                            name,
                            kind: symbol_kind,
                            visibility: SymbolIndexSupport::visibility(item.visibility().kind()),
                            target_namespace,
                            target_path: None,
                            external_crate: None,
                            cfg_condition: item_cfg,
                        },
                    );
                }
            }
        }
    }

    fn collect_use_tree(&mut self, module: &ModuleNode, node: &SyntaxNode) {
        let Some(tree) = node.use_tree() else {
            return;
        };
        let mut path = node
            .parent()
            .and_then(|parent| self.use_prefixes.get(&parent).cloned())
            .unwrap_or_default();
        path.extend(tree.path().iter().cloned());
        self.use_prefixes.insert(node.id(), path.clone());
        let Some(root_item) = global_use_item(module, node) else {
            return;
        };
        if has_use_tree_children(module, node) || tree.is_glob() || path.is_empty() {
            return;
        }
        if path.last().is_some_and(|segment| segment == "self") {
            path.pop();
        }
        let Some(name) = tree
            .alias()
            .map(str::to_owned)
            .or_else(|| path.last().cloned())
        else {
            return;
        };
        let Some(root_data) = root_item.item_syntax() else {
            return;
        };
        let cfg_condition = module
            .configured_ast()
            .node(node.id())
            .and_then(|configured| configured.availability_condition())
            .map(str::to_owned)
            .or_else(|| configured_condition(module, root_item));
        self.push_node_symbol(
            module,
            node,
            NodeSymbolRecord {
                name,
                kind: SymbolKind::UseAlias,
                visibility: SymbolIndexSupport::visibility(root_data.visibility().kind()),
                target_namespace: None,
                target_path: Some(path),
                external_crate: None,
                cfg_condition,
            },
        );
    }

    fn collect_associated_item(
        &mut self,
        module_index: usize,
        module: &ModuleNode,
        node: &SyntaxNode,
        kind: &ItemSyntaxKind,
    ) {
        let output = module.parse_output();
        let Some(scope) = node.parent().and_then(|id| output.node(id)) else {
            return;
        };
        if scope.kind() != SyntaxNodeKind::Scope {
            return;
        }
        let Some(owner) = scope.parent().and_then(|id| output.node(id)) else {
            return;
        };
        let Some(owner_item) = owner.item_syntax() else {
            return;
        };
        let Some(name) = SymbolIndexSupport::associated_item_name(kind) else {
            return;
        };
        let Some(configured) = module.configured_ast().node(node.id()) else {
            return;
        };
        if configured.availability_decision() == CfgDecision::False {
            return;
        }
        let cfg_condition = configured
            .availability_condition()
            .map(str::to_owned)
            .or_else(|| module.cfg_condition().map(str::to_owned));
        match owner_item.kind() {
            ItemSyntaxKind::Trait(Some(owner_name)) => {
                let mut namespace = module.namespace().to_vec();
                namespace.push(owner_name.to_owned());
                self.push_node_symbol(
                    module,
                    node,
                    NodeSymbolRecord {
                        name,
                        kind: SymbolKind::AssociatedItem,
                        visibility: SymbolIndexSupport::visibility(
                            node.item_syntax().unwrap().visibility().kind(),
                        ),
                        target_namespace: None,
                        target_path: None,
                        external_crate: None,
                        cfg_condition,
                    },
                );
                if let Some(symbol) = self.index.last_symbol_mut() {
                    symbol.namespace = namespace;
                }
            }
            ItemSyntaxKind::Impl => {
                let Some(implementation) = owner.impl_syntax() else {
                    return;
                };
                let Some(target) = implementation.self_type() else {
                    return;
                };
                let Some(owner_path) = SymbolIndexSupport::type_path(target) else {
                    return;
                };
                let visibility = node
                    .item_syntax()
                    .map(|item| SymbolIndexSupport::visibility(item.visibility().kind()))
                    .unwrap_or(SymbolVisibility::Private);
                self.pending_associated.push(PendingAssociatedSymbol {
                    module_index,
                    owner_path,
                    owner_bounds: target.bounds(),
                    name,
                    visibility,
                    context_index: module.context_index(),
                    target_name: module.target_name().to_owned(),
                    file_path: module.file_path().to_path_buf(),
                    span: module
                        .parse_output()
                        .source()
                        .span(node.start(), node.end())
                        .ok(),
                    cfg_condition,
                });
            }
            _ => {}
        }
    }

    fn collect_foreign_item(&mut self, module: &ModuleNode, node: &SyntaxNode) {
        let output = module.parse_output();
        let Some(scope) = node.parent().and_then(|id| output.node(id)) else {
            return;
        };
        let Some(owner) = scope.parent().and_then(|id| output.node(id)) else {
            return;
        };
        if owner.parent() != Some(module.scope_id())
            || !owner
                .item_syntax()
                .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::ExternBlock { .. }))
        {
            return;
        }
        let Some(data) = node.foreign_item_syntax() else {
            return;
        };
        let (name, kind) = match data.kind() {
            ForeignItemSyntaxKind::Function {
                name: Some(name), ..
            } => (name.clone(), SymbolKind::Function),
            ForeignItemSyntaxKind::Static {
                name: Some(name), ..
            } => (name.clone(), SymbolKind::Static),
            _ => return,
        };
        let cfg_condition = module
            .configured_ast()
            .node(node.id())
            .and_then(|configured| configured.availability_condition())
            .map(str::to_owned)
            .or_else(|| configured_condition(module, owner));
        self.push_node_symbol(
            module,
            node,
            NodeSymbolRecord {
                name,
                kind,
                visibility: SymbolIndexSupport::visibility(data.visibility().kind()),
                target_namespace: None,
                target_path: None,
                external_crate: None,
                cfg_condition,
            },
        );
    }

    fn push_node_symbol(
        &mut self,
        module: &ModuleNode,
        node: &SyntaxNode,
        symbol: NodeSymbolRecord,
    ) {
        self.index.push_symbol(SymbolRecord::new(SymbolRecordData {
            name: symbol.name,
            kind: symbol.kind,
            visibility: symbol.visibility,
            context_index: module.context_index(),
            target_name: module.target_name().to_owned(),
            namespace: module.namespace().to_vec(),
            target_namespace: symbol.target_namespace,
            target_path: symbol.target_path,
            external_crate: symbol.external_crate,
            file_path: module.file_path().to_path_buf(),
            span: module
                .parse_output()
                .source()
                .span(node.start(), node.end())
                .ok(),
            cfg_condition: symbol.cfg_condition,
        }));
    }

    fn add_external_crates(&mut self, package: &SelectedPackage, modules: &[ModuleNode]) {
        for (context_index, target) in package.targets().iter().enumerate() {
            let evaluator = CfgEvaluator::new(target);
            let build_script = target.kind().iter().any(|kind| kind == "custom-build");
            let crates = self
                .index
                .external_crates_mut()
                .entry(context_index)
                .or_default();
            for dependency in package.dependencies().dependencies() {
                let dependency_kind_is_active = match dependency.kind() {
                    DependencyKind::Normal => !build_script,
                    DependencyKind::Build => build_script,
                    DependencyKind::Development => target.cfg_test(),
                    DependencyKind::Other(_) => false,
                };
                if !dependency_kind_is_active
                    || (dependency.is_optional()
                        && !target
                            .features()
                            .activated_dependencies()
                            .contains(dependency.dependency_key()))
                {
                    continue;
                }
                if dependency.target_condition().is_some_and(|condition| {
                    evaluator.dependency_condition(condition) != CfgDecision::True
                }) {
                    continue;
                }
                crates.insert(dependency.rust_name().to_owned());
            }
            crates.extend(["alloc", "core"].into_iter().map(str::to_owned));
            let no_std = modules.iter().any(|module| {
                module.context_index() == context_index
                    && module.is_crate_root()
                    && module.scope_inner_attributes().iter().any(|attribute| {
                        attribute.path().last().is_some_and(|name| name == "no_std")
                    })
            });
            if !no_std {
                crates.insert("std".to_owned());
            }
            if target.crate_types().iter().any(|kind| kind == "proc-macro") {
                crates.insert("proc_macro".to_owned());
            }
        }
    }
}

fn belongs_to_module_scope(module: &ModuleNode, node: &SyntaxNode) -> bool {
    let output = module.parse_output();
    let mut current = Some(node.id());
    while let Some(id) = current {
        if id == module.scope_id() {
            return true;
        }
        let Some(ancestor) = output.node(id) else {
            return false;
        };
        if id != node.id()
            && ancestor
                .item_syntax()
                .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::Module { .. }))
        {
            return false;
        }
        current = ancestor.parent();
    }
    false
}

fn node_is_enabled(module: &ModuleNode, node: &SyntaxNode) -> bool {
    module
        .configured_ast()
        .node(node.id())
        .is_some_and(|configured| configured.availability_decision() != CfgDecision::False)
}

fn configured_condition(module: &ModuleNode, node: &SyntaxNode) -> Option<String> {
    module
        .configured_ast()
        .node(node.id())
        .and_then(|configured| configured.availability_condition())
        .map(str::to_owned)
        .or_else(|| module.cfg_condition().map(str::to_owned))
}

fn global_use_item<'a>(module: &'a ModuleNode, node: &'a SyntaxNode) -> Option<&'a SyntaxNode> {
    let output = module.parse_output();
    let mut current = node.parent();
    while let Some(id) = current {
        let ancestor = output.node(id)?;
        if ancestor.kind() == SyntaxNodeKind::UseTree {
            current = ancestor.parent();
            continue;
        }
        if ancestor
            .item_syntax()
            .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::Use))
            && ancestor.parent() == Some(module.scope_id())
        {
            return Some(ancestor);
        }
        return None;
    }
    None
}

fn has_use_tree_children(module: &ModuleNode, node: &SyntaxNode) -> bool {
    node.children().iter().any(|id| {
        module
            .parse_output()
            .node(*id)
            .is_some_and(|child| child.kind() == SyntaxNodeKind::UseTree)
    })
}
