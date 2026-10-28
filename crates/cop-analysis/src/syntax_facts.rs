use std::{collections::BTreeMap, ops::Range};

use cop_syntax::{
    AstNodeId, ExpressionSyntaxKind, ItemSyntaxKind, SyntaxNode, SyntaxNodeKind, VisibilityKind,
};

use crate::{CfgDecision, ModuleNode, SymbolVisibility};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ItemKindFact {
    Module { inline: bool },
    ExternCrate,
    Use,
    MacroDefinition,
    Struct,
    Enum,
    Union,
    TypeAlias,
    Trait,
    Constant,
    Static,
    Function,
    Impl,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldFact {
    bounds: Range<usize>,
    condition: Option<String>,
    enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathAttributeFact {
    bounds: Range<usize>,
    condition: Option<String>,
    path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemFact {
    node_id: AstNodeId,
    scope_id: usize,
    bounds: Range<usize>,
    condition: Option<String>,
    enabled: bool,
    kind: ItemKindFact,
    name: Option<String>,
    visibility: SymbolVisibility,
    nested_cfg_ranges: Vec<Range<usize>>,
    path_attributes: Vec<PathAttributeFact>,
    declarative: bool,
    public_fields: Vec<FieldFact>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemScopeFact {
    id: usize,
    bounds: Range<usize>,
    items: Vec<ItemFact>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShapeParameterFact {
    bounds: Range<usize>,
    condition: Option<String>,
    enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockItemFact {
    bounds: Range<usize>,
    condition: Option<String>,
    use_tree: bool,
    supported: bool,
    nested_cfg_ranges: Vec<Range<usize>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockScopeFact {
    bounds: Range<usize>,
    items: Vec<BlockItemFact>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyntaxShapeKind {
    FunctionParameters(usize),
    ClosureParameters(Vec<ShapeParameterFact>),
    TupleArity(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxShapeFact {
    bounds: Range<usize>,
    kind: SyntaxShapeKind,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModuleSyntaxFacts {
    scopes: Vec<ItemScopeFact>,
    shapes: Vec<SyntaxShapeFact>,
    block_scopes: Vec<BlockScopeFact>,
    code_line_count: usize,
    declarative_coverage: bool,
    declarative_scope_coverage: bool,
}

pub(crate) struct SyntaxFactsCollector {
    facts: ModuleSyntaxFacts,
    scope_ids: BTreeMap<AstNodeId, usize>,
    items_by_node: BTreeMap<AstNodeId, (usize, usize)>,
    block_items_by_node: BTreeMap<AstNodeId, (usize, usize)>,
    test_ranges: Vec<Range<usize>>,
    has_issue: bool,
    has_scope_issue: bool,
    has_non_declarative_opaque: bool,
    has_scope_non_declarative_opaque: bool,
}

impl ItemFact {
    pub fn node_id(&self) -> AstNodeId {
        self.node_id
    }

    pub fn scope_id(&self) -> usize {
        self.scope_id
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn condition(&self) -> Option<&str> {
        self.condition.as_deref()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn kind(&self) -> &ItemKindFact {
        &self.kind
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn visibility(&self) -> &SymbolVisibility {
        &self.visibility
    }

    pub fn nested_cfg_ranges(&self) -> &[Range<usize>] {
        &self.nested_cfg_ranges
    }

    pub fn path_attributes(&self) -> &[PathAttributeFact] {
        &self.path_attributes
    }

    pub fn is_declarative(&self) -> bool {
        self.declarative
    }

    pub fn public_fields(&self) -> &[FieldFact] {
        &self.public_fields
    }
}

impl FieldFact {
    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn condition(&self) -> Option<&str> {
        self.condition.as_deref()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

impl PathAttributeFact {
    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn condition(&self) -> Option<&str> {
        self.condition.as_deref()
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
}

impl ItemScopeFact {
    pub fn id(&self) -> usize {
        self.id
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn items(&self) -> &[ItemFact] {
        &self.items
    }
}

impl ShapeParameterFact {
    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn condition(&self) -> Option<&str> {
        self.condition.as_deref()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

impl BlockItemFact {
    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn condition(&self) -> Option<&str> {
        self.condition.as_deref()
    }

    pub fn is_use_tree(&self) -> bool {
        self.use_tree
    }

    pub fn is_supported(&self) -> bool {
        self.supported
    }

    pub fn nested_cfg_ranges(&self) -> &[Range<usize>] {
        &self.nested_cfg_ranges
    }
}

impl BlockScopeFact {
    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn items(&self) -> &[BlockItemFact] {
        &self.items
    }
}

impl SyntaxShapeFact {
    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn kind(&self) -> &SyntaxShapeKind {
        &self.kind
    }
}

impl ModuleSyntaxFacts {
    pub fn scopes(&self) -> &[ItemScopeFact] {
        &self.scopes
    }

    pub fn shapes(&self) -> &[SyntaxShapeFact] {
        &self.shapes
    }

    pub fn block_scopes(&self) -> &[BlockScopeFact] {
        &self.block_scopes
    }

    pub fn root_scope(&self) -> Option<&ItemScopeFact> {
        self.scopes.first()
    }

    pub fn code_line_count(&self) -> usize {
        self.code_line_count
    }

    pub fn has_declarative_coverage(&self) -> bool {
        self.declarative_coverage
    }

    pub fn has_declarative_scope_coverage(&self) -> bool {
        self.declarative_scope_coverage
    }
}

impl SyntaxFactsCollector {
    pub(crate) fn new(module: &ModuleNode) -> Self {
        let root_scope = ItemScopeFact {
            id: 0,
            bounds: module.scope_range(),
            items: Vec::new(),
        };
        Self {
            facts: ModuleSyntaxFacts {
                scopes: vec![root_scope],
                ..ModuleSyntaxFacts::default()
            },
            scope_ids: BTreeMap::from([(module.scope_id(), 0)]),
            items_by_node: BTreeMap::new(),
            block_items_by_node: BTreeMap::new(),
            test_ranges: Vec::new(),
            has_issue: false,
            has_scope_issue: false,
            has_non_declarative_opaque: false,
            has_scope_non_declarative_opaque: false,
        }
    }

    pub(crate) fn visit_node(&mut self, module: &ModuleNode, node: &SyntaxNode) {
        if !belongs_to_module_scope(module, node) {
            return;
        }
        let output = module.parse_output();
        let parent = node.parent().and_then(|id| output.node(id));
        if node
            .attributes()
            .iter()
            .any(|attribute| attribute.is_cfg_test())
        {
            self.test_ranges.push(node.start()..node.end());
        }
        if node.kind() == SyntaxNodeKind::Opaque
            && node
                .parent()
                .and_then(|id| output.node(id))
                .map_or(true, |parent| {
                    parent.kind() != SyntaxNodeKind::MacroDefinition
                })
            && node_is_enabled(module, node)
        {
            self.has_non_declarative_opaque = true;
            if within_bounds(&module.scope_range(), node.start()..node.end()) {
                self.has_scope_non_declarative_opaque = true;
            }
        }
        if node.kind() == SyntaxNodeKind::Scope
            && parent
                .and_then(SyntaxNode::item_syntax)
                .is_some_and(|item| {
                    matches!(item.kind(), ItemSyntaxKind::Trait(_) | ItemSyntaxKind::Impl)
                })
        {
            let children = direct_item_children(module, node);
            if !children.is_empty() {
                let id = self.facts.scopes.len();
                self.scope_ids.insert(node.id(), id);
                self.facts.scopes.push(ItemScopeFact {
                    id,
                    bounds: children
                        .first()
                        .zip(children.last())
                        .map(|(first, last)| first.start()..last.end())
                        .unwrap_or(node.start()..node.end()),
                    items: Vec::new(),
                });
            }
        }
        if let Some(item_data) = node.item_syntax() {
            if let Some(parent_id) = node.parent() {
                if let Some(scope_id) = self.scope_ids.get(&parent_id).copied() {
                    let item_fact = make_item_fact(module, node, scope_id);
                    let scope = &mut self.facts.scopes[scope_id];
                    let item_index = scope.items.len();
                    scope.items.push(item_fact);
                    self.items_by_node.insert(node.id(), (scope_id, item_index));
                }
            }
            if matches!(item_data.kind(), ItemSyntaxKind::Function { .. }) {
                let parameter_count = node
                    .children()
                    .iter()
                    .filter_map(|id| output.node(*id))
                    .filter(|parameter| {
                        parameter.kind() == SyntaxNodeKind::Parameter
                            && module
                                .configured_ast()
                                .is_enabled_range(parameter.start(), parameter.end())
                    })
                    .count();
                self.facts.shapes.push(SyntaxShapeFact {
                    bounds: node.start()..node.end(),
                    kind: SyntaxShapeKind::FunctionParameters(parameter_count),
                });
            }
        }
        if node.kind() == SyntaxNodeKind::Field
            && node.field_visibility().is_some_and(|visibility| {
                !matches!(visibility.kind(), VisibilityKind::Private)
                    && (visibility.start() == visibility.end()
                        || module.configured_ast().is_supported_range(
                            output,
                            visibility.start(),
                            visibility.end(),
                        ))
            })
        {
            if let Some(owner) = nearest_record_item(module, node) {
                if let Some((scope_index, item_index)) = self.items_by_node.get(&owner).copied() {
                    self.facts.scopes[scope_index].items[item_index]
                        .public_fields
                        .push(FieldFact {
                            bounds: node.start()..node.end(),
                            condition: condition_for_node(module, node),
                            enabled: module
                                .configured_ast()
                                .is_enabled_range(node.start(), node.end()),
                        });
                }
            }
        }
        if node.kind() == SyntaxNodeKind::UseTree {
            self.record_nested_cfg_range(module, node);
        }
        if node.kind() == SyntaxNodeKind::Block && inside_function(module, node) {
            let items = node
                .children()
                .iter()
                .filter_map(|id| output.node(*id))
                .filter(|child| node_is_enabled(module, child))
                .map(|child| {
                    let use_tree = child.kind() == SyntaxNodeKind::Statement
                        && child.children().iter().any(|id| {
                            output
                                .node(*id)
                                .is_some_and(|tree| tree.kind() == SyntaxNodeKind::UseTree)
                        });
                    (
                        child.id(),
                        BlockItemFact {
                            bounds: child.start()..child.end(),
                            condition: module
                                .configured_ast()
                                .node(child.id())
                                .and_then(|configured| configured.effective_condition())
                                .map(str::to_owned),
                            use_tree,
                            supported: use_tree
                                && module.configured_ast().is_supported_range(
                                    output,
                                    child.start(),
                                    child.end(),
                                ),
                            nested_cfg_ranges: Vec::new(),
                        },
                    )
                })
                .collect::<Vec<_>>();
            let scope_index = self.facts.block_scopes.len();
            self.facts.block_scopes.push(BlockScopeFact {
                bounds: node.start()..node.end(),
                items: items.iter().map(|(_, item)| item.clone()).collect(),
            });
            for (item_index, (item_id, _)) in items.into_iter().enumerate() {
                self.block_items_by_node
                    .insert(item_id, (scope_index, item_index));
            }
        }
        if let Some(expression) = node.expression_syntax() {
            let bounds = expression.bounds();
            let enabled = module
                .configured_ast()
                .node_for_range(bounds.start, bounds.end)
                .map_or(true, |configured| {
                    configured.decision() != CfgDecision::False
                });
            let supported =
                module
                    .configured_ast()
                    .is_supported_range(output, bounds.start, bounds.end);
            if enabled && supported {
                match expression.kind() {
                    ExpressionSyntaxKind::Closure { parameters, .. } => {
                        self.facts.shapes.push(SyntaxShapeFact {
                            bounds: bounds.clone(),
                            kind: SyntaxShapeKind::ClosureParameters(
                                parameters
                                    .iter()
                                    .map(|parameter| {
                                        let parameter_bounds = parameter.bounds();
                                        ShapeParameterFact {
                                            condition: module
                                                .configured_ast()
                                                .condition_for_span(
                                                    parameter_bounds.start,
                                                    parameter_bounds.end,
                                                )
                                                .map(str::to_owned),
                                            enabled: module.configured_ast().is_enabled_range(
                                                parameter_bounds.start,
                                                parameter_bounds.end,
                                            ),
                                            bounds: parameter_bounds,
                                        }
                                    })
                                    .collect(),
                            ),
                        });
                    }
                    ExpressionSyntaxKind::Tuple(members) => {
                        let arity = members
                            .iter()
                            .filter(|member| {
                                let member_bounds = member.bounds();
                                module
                                    .configured_ast()
                                    .is_enabled_range(member_bounds.start, member_bounds.end)
                            })
                            .count();
                        self.facts.shapes.push(SyntaxShapeFact {
                            bounds,
                            kind: SyntaxShapeKind::TupleArity(arity),
                        });
                    }
                    _ => {}
                }
            }
        }
    }

    pub(crate) fn finish(mut self, module: &ModuleNode) -> ModuleSyntaxFacts {
        self.has_issue = module
            .parse_output()
            .issues()
            .iter()
            .any(|issue| module.configured_ast().is_issue_enabled(issue));
        self.has_scope_issue = module.parse_output().issues().iter().any(|issue| {
            within_bounds(&module.scope_range(), issue.start()..issue.end())
                && module.configured_ast().is_issue_enabled(issue)
        });
        self.facts.code_line_count = count_code_lines(module, &self.test_ranges);
        self.facts.declarative_coverage = !self.has_issue && !self.has_non_declarative_opaque;
        self.facts.declarative_scope_coverage =
            !self.has_scope_issue && !self.has_scope_non_declarative_opaque;
        self.facts
    }

    fn record_nested_cfg_range(&mut self, module: &ModuleNode, tree: &SyntaxNode) {
        let Some(configured) = module.configured_ast().node(tree.id()) else {
            return;
        };
        if configured.decision() == CfgDecision::False {
            return;
        }
        let Some(attribute) = tree
            .attributes()
            .iter()
            .find(|attribute| attribute.path() == ["cfg"])
        else {
            return;
        };
        let output = module.parse_output();
        let mut current = tree.parent();
        while let Some(id) = current {
            let Some(ancestor) = output.node(id) else {
                return;
            };
            if ancestor
                .item_syntax()
                .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::Use))
            {
                if module.configured_ast().is_supported_range(
                    output,
                    attribute.start(),
                    attribute.end(),
                ) {
                    if let Some((scope_index, item_index)) = self.items_by_node.get(&id).copied() {
                        self.facts.scopes[scope_index].items[item_index]
                            .nested_cfg_ranges
                            .push(attribute.start()..attribute.end());
                    }
                }
                return;
            }
            if ancestor.kind() == SyntaxNodeKind::Statement {
                if let Some((scope_index, item_index)) = self.block_items_by_node.get(&id).copied()
                {
                    self.facts.block_scopes[scope_index].items[item_index]
                        .nested_cfg_ranges
                        .push(attribute.start()..attribute.end());
                }
                return;
            }
            current = ancestor.parent();
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

fn make_item_fact(module: &ModuleNode, node: &SyntaxNode, scope_id: usize) -> ItemFact {
    let item = node.item_syntax().expect("item node must have item syntax");
    let (kind, name) = match item.kind() {
        ItemSyntaxKind::Module { name, inline } => {
            (ItemKindFact::Module { inline: *inline }, Some(name.clone()))
        }
        ItemSyntaxKind::ExternCrate { name, alias } => (
            ItemKindFact::ExternCrate,
            alias.clone().or_else(|| name.clone()),
        ),
        ItemSyntaxKind::Use => (ItemKindFact::Use, None),
        ItemSyntaxKind::MacroDefinition(_) => (ItemKindFact::MacroDefinition, None),
        ItemSyntaxKind::Struct(name) => (ItemKindFact::Struct, name.clone()),
        ItemSyntaxKind::Enum(name) => (ItemKindFact::Enum, name.clone()),
        ItemSyntaxKind::Union(name) => (ItemKindFact::Union, name.clone()),
        ItemSyntaxKind::TypeAlias(name) => (ItemKindFact::TypeAlias, name.clone()),
        ItemSyntaxKind::Trait(name) => (ItemKindFact::Trait, name.clone()),
        ItemSyntaxKind::Constant(name) => (ItemKindFact::Constant, name.clone()),
        ItemSyntaxKind::Static(name) => (ItemKindFact::Static, name.clone()),
        ItemSyntaxKind::Function { name } => (ItemKindFact::Function, name.clone()),
        ItemSyntaxKind::Impl => (ItemKindFact::Impl, None),
        _ => (ItemKindFact::Other, None),
    };
    let visibility = match item.visibility().kind() {
        VisibilityKind::Private => SymbolVisibility::Private,
        VisibilityKind::Public => SymbolVisibility::Public,
        VisibilityKind::Restricted(scope) => SymbolVisibility::Restricted(scope.clone()),
    };
    let declarative = match item.kind() {
        ItemSyntaxKind::Module { .. }
        | ItemSyntaxKind::MacroDefinition(_)
        | ItemSyntaxKind::Struct(_)
        | ItemSyntaxKind::Enum(_)
        | ItemSyntaxKind::Union(_)
        | ItemSyntaxKind::TypeAlias(_)
        | ItemSyntaxKind::Trait(_)
        | ItemSyntaxKind::Constant(_)
        | ItemSyntaxKind::Static(_) => true,
        ItemSyntaxKind::Use => match &visibility {
            SymbolVisibility::Private | SymbolVisibility::Public => true,
            SymbolVisibility::Restricted(scope) => scope == "(crate)",
        },
        _ => false,
    };
    let path_attributes = module
        .configured_ast()
        .node(node.id())
        .into_iter()
        .flat_map(|configured| configured.attribute_evaluation().path_attributes())
        .map(|attribute| PathAttributeFact {
            bounds: attribute.start()..attribute.end(),
            condition: attribute.condition().map(str::to_owned),
            path: attribute.path().map(str::to_owned),
        })
        .collect();
    ItemFact {
        node_id: node.id(),
        scope_id,
        bounds: node.start()..node.end(),
        condition: condition_for_node(module, node),
        enabled: node_is_enabled(module, node),
        kind,
        name,
        visibility,
        nested_cfg_ranges: Vec::new(),
        path_attributes,
        declarative,
        public_fields: Vec::new(),
    }
}

fn direct_item_children<'a>(module: &'a ModuleNode, scope: &'a SyntaxNode) -> Vec<&'a SyntaxNode> {
    scope
        .children()
        .iter()
        .filter_map(|id| module.parse_output().node(*id))
        .filter(|node| node.item_syntax().is_some())
        .collect()
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

fn inside_function(module: &ModuleNode, node: &SyntaxNode) -> bool {
    let output = module.parse_output();
    let mut current = node.parent();
    while let Some(id) = current {
        let Some(ancestor) = output.node(id) else {
            return false;
        };
        if ancestor
            .item_syntax()
            .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::Function { .. }))
        {
            return true;
        }
        if ancestor
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
        .is_some_and(|configured| configured.decision() != CfgDecision::False)
}

fn condition_for_node(module: &ModuleNode, node: &SyntaxNode) -> Option<String> {
    module
        .configured_ast()
        .condition_for_span(node.start(), node.end())
        .map(str::to_owned)
}

fn within_bounds(outer: &Range<usize>, inner: Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn count_code_lines(module: &ModuleNode, test_ranges: &[Range<usize>]) -> usize {
    let output = module.parse_output();
    let test_module = output
        .inner_attributes()
        .iter()
        .any(|attribute| attribute.is_cfg_test());
    let mut lines = Vec::new();
    if !test_module {
        for token in output.tokens() {
            if !module
                .configured_ast()
                .is_enabled_range(token.start(), token.end())
                || test_ranges
                    .iter()
                    .any(|range| range.contains(&token.start()))
            {
                continue;
            }
            if let Some((start_line, _)) = output.source().line_column(token.start()) {
                let line_breaks = token
                    .text(output.source())
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count();
                let end_line =
                    start_line.saturating_add(u32::try_from(line_breaks).unwrap_or(u32::MAX));
                for line in start_line..=end_line {
                    if lines.last() != Some(&line) {
                        lines.push(line);
                    }
                }
            }
        }
    }
    lines.len()
}
