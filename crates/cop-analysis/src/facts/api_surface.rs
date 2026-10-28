use std::{collections::BTreeMap, iter::once, mem, ops::Range};

use cop_syntax::{AstNodeId, ItemSyntaxKind, SyntaxNode, SyntaxNodeKind, VisibilityKind};

use crate::{
    module_graph::ModuleNode,
    symbol_index::{SymbolIndexSupport, SymbolVisibility},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingDeclaration {
    pub(super) module_index: usize,
    pub(super) node_id: AstNodeId,
    pub(super) bounds: Range<usize>,
    pub(super) visibility_bounds: Range<usize>,
    pub(super) visibility: SymbolVisibility,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordApiRange {
    bounds: Range<usize>,
    first_member: usize,
    visible_scope: Vec<String>,
    enum_fields_inherit_scope: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ApiSurfaceRange {
    pub(super) bounds: Range<usize>,
    pub(super) visible_scope: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ApiSurfaceCollector {
    pending_declarations: Vec<PendingDeclaration>,
    ranges_by_declaration: BTreeMap<(usize, AstNodeId), Vec<ApiSurfaceRange>>,
    record_ranges: BTreeMap<(usize, AstNodeId), RecordApiRange>,
}

impl ApiSurfaceCollector {
    pub(super) fn visit_node(
        &mut self,
        module_index: usize,
        module: &ModuleNode,
        node: &SyntaxNode,
    ) {
        if !belongs_to_module_scope(module, node) {
            return;
        }
        self.collect_declaration(module_index, node);
        self.collect_ranges(module_index, module, node);
    }

    pub(super) fn take_pending_declarations(&mut self) -> Vec<PendingDeclaration> {
        mem::take(&mut self.pending_declarations)
    }

    pub(super) fn finish_ranges(mut self) -> BTreeMap<(usize, AstNodeId), Vec<ApiSurfaceRange>> {
        for (key, record) in mem::take(&mut self.record_ranges) {
            self.ranges_by_declaration
                .entry(key)
                .or_default()
                .push(ApiSurfaceRange {
                    bounds: record.bounds.start..record.first_member,
                    visible_scope: record.visible_scope,
                });
        }
        self.ranges_by_declaration
    }

    fn collect_declaration(&mut self, module_index: usize, node: &SyntaxNode) {
        let visibility = node
            .item_syntax()
            .map(|item| item.visibility())
            .or_else(|| node.foreign_item_syntax().map(|item| item.visibility()));
        let Some(visibility) = visibility else {
            return;
        };
        self.pending_declarations.push(PendingDeclaration {
            module_index,
            node_id: node.id(),
            bounds: node.start()..node.end(),
            visibility_bounds: visibility.start()..visibility.end(),
            visibility: SymbolIndexSupport::visibility(visibility.kind()),
        });
    }

    fn collect_ranges(&mut self, module_index: usize, module: &ModuleNode, node: &SyntaxNode) {
        let key = (module_index, node.id());
        if let Some(item) = node.item_syntax() {
            let visible_scope =
                visibility_scope(item.visibility().kind(), module.namespace()).unwrap_or_default();
            match item.kind() {
                ItemSyntaxKind::Function { .. } => {
                    self.ranges_by_declaration.insert(
                        key,
                        once(api_range(
                            node.start()..function_signature_end(module, node),
                            visible_scope.clone(),
                        ))
                        .collect(),
                    );
                }
                ItemSyntaxKind::TypeAlias(_) => {
                    self.ranges_by_declaration.insert(
                        key,
                        once(api_range(node.start()..node.end(), visible_scope.clone())).collect(),
                    );
                }
                ItemSyntaxKind::Struct(_) | ItemSyntaxKind::Union(_) | ItemSyntaxKind::Enum(_) => {
                    self.ranges_by_declaration.insert(key, Vec::new());
                    self.record_ranges.insert(
                        key,
                        RecordApiRange {
                            bounds: node.start()..node.end(),
                            first_member: node.end(),
                            visible_scope,
                            enum_fields_inherit_scope: matches!(
                                item.kind(),
                                ItemSyntaxKind::Enum(_)
                            ),
                        },
                    );
                }
                ItemSyntaxKind::Trait(_) => {
                    let scope = direct_child(module, node, SyntaxNodeKind::Scope);
                    if let Some(scope) = scope {
                        self.ranges_by_declaration.insert(
                            key,
                            once(api_range(
                                node.start()..scope.start(),
                                visible_scope.clone(),
                            ))
                            .collect(),
                        );
                    }
                }
                ItemSyntaxKind::Constant(_) | ItemSyntaxKind::Static(_) => {
                    self.ranges_by_declaration
                        .insert(key, direct_type_ranges(module, node, visible_scope.clone()));
                }
                _ => {}
            }
            self.collect_trait_item_ranges(module_index, module, node);
        }
        if matches!(
            node.kind(),
            SyntaxNodeKind::Field | SyntaxNodeKind::EnumVariant
        ) {
            if let Some(owner) = nearest_record_item(module, node) {
                let record_key = (module_index, owner);
                if let Some(record) = self.record_ranges.get_mut(&record_key) {
                    record.first_member = record.first_member.min(node.start());
                    let field_scope = if record.enum_fields_inherit_scope {
                        Some(record.visible_scope.clone())
                    } else {
                        node.field_visibility()
                            .map(|visibility| {
                                visibility_scope(visibility.kind(), module.namespace())
                                    .unwrap_or_default()
                            })
                            .and_then(|scope| intersect_scopes(&record.visible_scope, &scope))
                    };
                    if node.kind() == SyntaxNodeKind::Field {
                        let field_is_visible = record.enum_fields_inherit_scope
                            || node.field_visibility().is_some_and(|visibility| {
                                !matches!(visibility.kind(), VisibilityKind::Private)
                            });
                        if field_is_visible {
                            if let Some(visible_scope) = field_scope {
                                if let Some(ranges) =
                                    self.ranges_by_declaration.get_mut(&record_key)
                                {
                                    ranges.push(api_range(node.start()..node.end(), visible_scope));
                                }
                            }
                        }
                    }
                }
            }
        }
        if node.kind() == SyntaxNodeKind::Type {
            if let Some(foreign_item) = nearest_foreign_item(module, node) {
                if let Some(foreign_node) = module.parse_output().node(foreign_item) {
                    if let Some(visibility) = foreign_node
                        .foreign_item_syntax()
                        .map(|item| item.visibility())
                    {
                        let scope = visibility_scope(visibility.kind(), module.namespace())
                            .unwrap_or_default();
                        self.ranges_by_declaration
                            .entry((module_index, foreign_item))
                            .or_default()
                            .push(api_range(node.start()..node.end(), scope));
                    }
                }
            }
        }
    }

    fn collect_trait_item_ranges(
        &mut self,
        module_index: usize,
        module: &ModuleNode,
        node: &SyntaxNode,
    ) {
        let output = module.parse_output();
        let Some(scope) = node.parent().and_then(|id| output.node(id)) else {
            return;
        };
        let Some(owner) = scope.parent().and_then(|id| output.node(id)) else {
            return;
        };
        if !owner
            .item_syntax()
            .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::Trait(_)))
        {
            return;
        }
        let Some(item) = node.item_syntax() else {
            return;
        };
        let owner_scope = owner
            .item_syntax()
            .and_then(|owner_item| {
                visibility_scope(owner_item.visibility().kind(), module.namespace())
            })
            .unwrap_or_default();
        let ranges = match item.kind() {
            ItemSyntaxKind::Function { .. } => once(api_range(
                node.start()..function_signature_end(module, node),
                owner_scope.clone(),
            ))
            .collect(),
            ItemSyntaxKind::TypeAlias(_) => {
                once(api_range(node.start()..node.end(), owner_scope.clone())).collect()
            }
            ItemSyntaxKind::Constant(_) | ItemSyntaxKind::Static(_) => {
                direct_type_ranges(module, node, owner_scope)
            }
            _ => Vec::new(),
        };
        self.ranges_by_declaration
            .entry((module_index, owner.id()))
            .or_default()
            .extend(ranges);
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

fn direct_child<'a>(
    module: &'a ModuleNode,
    parent: &SyntaxNode,
    kind: SyntaxNodeKind,
) -> Option<&'a SyntaxNode> {
    parent
        .children()
        .iter()
        .filter_map(|id| module.parse_output().node(*id))
        .find(|node| node.kind() == kind)
}

fn function_signature_end(module: &ModuleNode, item: &SyntaxNode) -> usize {
    direct_child(module, item, SyntaxNodeKind::Block).map_or(item.end(), SyntaxNode::start)
}

fn direct_type_ranges(
    module: &ModuleNode,
    declaration: &SyntaxNode,
    visible_scope: Vec<String>,
) -> Vec<ApiSurfaceRange> {
    declaration
        .children()
        .iter()
        .filter_map(|id| module.parse_output().node(*id))
        .filter(|node| node.kind() == SyntaxNodeKind::Type)
        .map(|node| api_range(node.start()..node.end(), visible_scope.clone()))
        .collect()
}

fn api_range(bounds: Range<usize>, visible_scope: Vec<String>) -> ApiSurfaceRange {
    ApiSurfaceRange {
        bounds,
        visible_scope,
    }
}

fn visibility_scope(kind: &VisibilityKind, namespace: &[String]) -> Option<Vec<String>> {
    SymbolIndexSupport::visibility(kind).visible_scope(namespace)
}

fn intersect_scopes(left: &[String], right: &[String]) -> Option<Vec<String>> {
    if left.starts_with(right) {
        Some(left.to_vec())
    } else if right.starts_with(left) {
        Some(right.to_vec())
    } else {
        None
    }
}

fn nearest_record_item(module: &ModuleNode, node: &SyntaxNode) -> Option<AstNodeId> {
    let output = module.parse_output();
    let mut current = node.parent();
    while let Some(id) = current {
        let ancestor = output.node(id)?;
        if ancestor.item_syntax().is_some_and(|item| {
            matches!(
                item.kind(),
                ItemSyntaxKind::Struct(_) | ItemSyntaxKind::Enum(_) | ItemSyntaxKind::Union(_)
            )
        }) {
            return Some(id);
        }
        if ancestor
            .item_syntax()
            .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::Module { .. }))
        {
            return None;
        }
        current = ancestor.parent();
    }
    None
}

fn nearest_foreign_item(module: &ModuleNode, node: &SyntaxNode) -> Option<AstNodeId> {
    let output = module.parse_output();
    let mut current = node.parent();
    while let Some(id) = current {
        let ancestor = output.node(id)?;
        if ancestor.foreign_item_syntax().is_some() {
            return Some(id);
        }
        if ancestor
            .item_syntax()
            .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::Module { .. }))
        {
            return None;
        }
        current = ancestor.parent();
    }
    None
}
