use std::{collections::BTreeMap, mem, ops::Range};

use cop_cfg::CfgCondition;
use cop_syntax::{
    AstNodeId, ExpressionSyntax, ExpressionSyntaxKind, ItemSyntaxKind, PatternSyntaxKind,
    StatementSyntaxKind, SyntaxNode, SyntaxNodeKind,
};

use crate::{CfgDecision, ModuleNode};

#[derive(Clone, Debug, Eq, PartialEq)]
struct LocalBinding {
    context_index: usize,
    namespace: Vec<String>,
    name: String,
    target_path: Option<Vec<String>>,
    scope: Range<usize>,
    visible_from: usize,
    condition: Option<String>,
}

struct LocalBindingInput<'a> {
    module: &'a ModuleNode,
    node: &'a SyntaxNode,
    name: String,
    target_path: Option<Vec<String>>,
    scope: &'a Range<usize>,
    visible_from: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PatternBindingSeed {
    node_id: AstNodeId,
    name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PatternScopeRequest {
    bounds: Range<usize>,
    visible_from: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LocalBindingMatch {
    name: String,
    target_path: Option<Vec<String>>,
    scope_start: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct LocalBindings {
    bindings: Vec<LocalBinding>,
    use_prefixes: BTreeMap<AstNodeId, Vec<String>>,
    special_pattern_scopes: BTreeMap<(usize, usize), PatternScopeRequest>,
    pattern_seeds: Vec<PatternBindingSeed>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PatternBindingRequest {
    bounds: Range<usize>,
    visible_from: usize,
}

impl LocalBindingMatch {
    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn target_path(&self) -> Option<&[String]> {
        self.target_path.as_deref()
    }

    pub(super) fn scope_start(&self) -> usize {
        self.scope_start
    }
}

impl LocalBindings {
    pub(crate) fn begin_module(&mut self) {
        self.use_prefixes.clear();
        self.special_pattern_scopes.clear();
        self.pattern_seeds.clear();
    }

    pub(crate) fn visit_node(&mut self, module: &ModuleNode, node: &SyntaxNode) {
        if !belongs_to_module_scope(module, node) || !node_is_potentially_enabled(module, node) {
            return;
        }
        if let Some(expression) = node.expression_syntax() {
            collect_special_pattern_scopes(expression, &mut self.special_pattern_scopes);
        }
        if let Some(pattern) = node.pattern_syntax() {
            if let PatternSyntaxKind::Binding { name, .. } = pattern.kind() {
                self.pattern_seeds.push(PatternBindingSeed {
                    node_id: node.id(),
                    name: name.clone(),
                });
            }
        }
        if node.kind() == SyntaxNodeKind::LocalItem {
            if let Some((name, target_path)) = local_item_binding(node) {
                if let Some(scope) = nearest_lexical_scope(module, node) {
                    self.push_binding(LocalBindingInput {
                        module,
                        node,
                        name,
                        target_path,
                        scope: &scope,
                        visible_from: scope.start,
                    });
                }
            }
        }
        if let Some(tree) = node.use_tree() {
            let mut path = node
                .parent()
                .and_then(|parent| self.use_prefixes.get(&parent).cloned())
                .unwrap_or_default();
            path.extend(tree.path().iter().cloned());
            self.use_prefixes.insert(node.id(), path.clone());
            if is_use_tree_leaf(module, node) {
                if let Some(statement) = nearest_use_statement(module, node) {
                    if let Some(scope) = nearest_lexical_scope(module, statement) {
                        if !tree.is_glob() && !path.is_empty() {
                            if path.last().is_some_and(|segment| segment == "self") {
                                path.pop();
                            }
                            if let Some(name) = tree
                                .alias()
                                .map(str::to_owned)
                                .or_else(|| path.last().cloned())
                            {
                                self.push_binding(LocalBindingInput {
                                    module,
                                    node,
                                    name,
                                    target_path: Some(path),
                                    scope: &scope,
                                    visible_from: scope.start,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn finish_module(&mut self, module: &ModuleNode) {
        let seeds = mem::take(&mut self.pattern_seeds);
        for seed in seeds {
            let Some(node) = module.parse_output().node(seed.node_id) else {
                continue;
            };
            let Some(scope) = pattern_binding_scope(module, node, &self.special_pattern_scopes)
            else {
                continue;
            };
            self.push_binding(LocalBindingInput {
                module,
                node,
                name: seed.name,
                target_path: None,
                scope: &scope.bounds,
                visible_from: scope.visible_from,
            });
        }
    }

    pub(super) fn find(
        &self,
        module: &ModuleNode,
        offset: usize,
        name: &str,
        query_condition: Option<&str>,
    ) -> Option<LocalBindingMatch> {
        self.bindings
            .iter()
            .filter(|binding| {
                binding.context_index == module.context_index()
                    && binding.namespace == module.namespace()
                    && identifier_name(&binding.name) == identifier_name(name)
                    && binding.scope.contains(&offset)
                    && binding.visible_from <= offset
                    && conditions_may_coexist(binding.condition.as_deref(), query_condition)
            })
            .min_by_key(|binding| {
                (
                    binding.scope.end.saturating_sub(binding.scope.start),
                    usize::MAX - binding.visible_from,
                )
            })
            .map(|binding| LocalBindingMatch {
                name: binding.name.clone(),
                target_path: binding.target_path.clone(),
                scope_start: binding.scope.start,
            })
    }

    fn push_binding(&mut self, input: LocalBindingInput<'_>) {
        let LocalBindingInput {
            module,
            node,
            name,
            target_path,
            scope,
            visible_from,
        } = input;
        let Some(configured) = module.configured_ast().node(node.id()) else {
            return;
        };
        if configured.decision() == CfgDecision::False {
            return;
        }
        self.bindings.push(LocalBinding {
            context_index: module.context_index(),
            namespace: module.namespace().to_vec(),
            name,
            target_path,
            scope: scope.clone(),
            visible_from,
            condition: configured.effective_condition().map(str::to_owned),
        });
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

fn collect_special_pattern_scopes(
    expression: &ExpressionSyntax,
    scopes: &mut BTreeMap<(usize, usize), PatternScopeRequest>,
) {
    match expression.kind() {
        ExpressionSyntaxKind::For { pattern, body, .. } => {
            let bounds = pattern.bounds();
            scopes.insert(
                (bounds.start, bounds.end),
                PatternScopeRequest {
                    bounds: body.clone(),
                    visible_from: body.start,
                },
            );
        }
        ExpressionSyntaxKind::If {
            condition,
            then_block,
            ..
        } => collect_condition_pattern_scopes(condition, then_block, scopes),
        ExpressionSyntaxKind::While {
            condition, body, ..
        } => collect_condition_pattern_scopes(condition, body, scopes),
        _ => {}
    }
}

fn collect_condition_pattern_scopes(
    expression: &ExpressionSyntax,
    scope: &Range<usize>,
    scopes: &mut BTreeMap<(usize, usize), PatternScopeRequest>,
) {
    if let ExpressionSyntaxKind::Let { pattern, .. } = expression.kind() {
        let bounds = pattern.bounds();
        scopes.insert(
            (bounds.start, bounds.end),
            PatternScopeRequest {
                bounds: scope.clone(),
                visible_from: scope.start,
            },
        );
    }
    match expression.kind() {
        ExpressionSyntaxKind::Group(inner)
        | ExpressionSyntaxKind::Unary { operand: inner, .. }
        | ExpressionSyntaxKind::Await(inner)
        | ExpressionSyntaxKind::Try(inner) => {
            collect_condition_pattern_scopes(inner, scope, scopes);
        }
        ExpressionSyntaxKind::Binary { left, right, .. }
        | ExpressionSyntaxKind::Assignment { left, right, .. } => {
            collect_condition_pattern_scopes(left, scope, scopes);
            collect_condition_pattern_scopes(right, scope, scopes);
        }
        _ => {}
    }
}

fn pattern_binding_scope(
    module: &ModuleNode,
    pattern: &SyntaxNode,
    special_scopes: &BTreeMap<(usize, usize), PatternScopeRequest>,
) -> Option<PatternBindingRequest> {
    if let Some(request) = special_scopes.get(&(pattern.start(), pattern.end())) {
        return Some(PatternBindingRequest {
            bounds: request.bounds.clone(),
            visible_from: request.visible_from,
        });
    }
    let output = module.parse_output();
    let mut current = pattern.parent();
    while let Some(id) = current {
        let ancestor = output.node(id)?;
        if ancestor.kind() == SyntaxNodeKind::Pattern {
            if let Some(request) = special_scopes.get(&(ancestor.start(), ancestor.end())) {
                return Some(PatternBindingRequest {
                    bounds: request.bounds.clone(),
                    visible_from: request.visible_from,
                });
            }
        }
        if ancestor.kind() == SyntaxNodeKind::MatchArm {
            return Some(PatternBindingRequest {
                bounds: ancestor.start()..ancestor.end(),
                visible_from: ancestor.start(),
            });
        }
        if ancestor.kind() == SyntaxNodeKind::Statement
            && ancestor.statement_kind() == Some(StatementSyntaxKind::Let)
        {
            let scope = nearest_lexical_scope(module, ancestor)?;
            return Some(PatternBindingRequest {
                bounds: scope,
                visible_from: ancestor.end(),
            });
        }
        if ancestor.kind() == SyntaxNodeKind::Parameter {
            if let Some(closure) = enclosing_closure(module, ancestor) {
                return Some(PatternBindingRequest {
                    bounds: closure.start()..closure.end(),
                    visible_from: closure.start(),
                });
            }
            let function = enclosing_function(module, ancestor)?;
            let body = function
                .children()
                .iter()
                .filter_map(|child| output.node(*child))
                .find(|child| child.kind() == SyntaxNodeKind::Block)?;
            return Some(PatternBindingRequest {
                bounds: body.start()..body.end(),
                visible_from: body.start(),
            });
        }
        if id != pattern.id()
            && ancestor
                .item_syntax()
                .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::Module { .. }))
        {
            return None;
        }
        current = ancestor.parent();
    }
    None
}

fn enclosing_closure<'a>(module: &'a ModuleNode, node: &'a SyntaxNode) -> Option<&'a SyntaxNode> {
    enclosing_node(module, node, |ancestor| {
        ancestor.kind() == SyntaxNodeKind::Closure
    })
}

fn enclosing_function<'a>(module: &'a ModuleNode, node: &'a SyntaxNode) -> Option<&'a SyntaxNode> {
    enclosing_node(module, node, |ancestor| {
        ancestor
            .item_syntax()
            .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::Function { .. }))
    })
}

fn enclosing_node<'a>(
    module: &'a ModuleNode,
    node: &'a SyntaxNode,
    predicate: impl Fn(&SyntaxNode) -> bool,
) -> Option<&'a SyntaxNode> {
    let output = module.parse_output();
    let mut current = node.parent();
    while let Some(id) = current {
        let ancestor = output.node(id)?;
        if predicate(ancestor) {
            return Some(ancestor);
        }
        if id != node.id()
            && ancestor
                .item_syntax()
                .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::Module { .. }))
        {
            return None;
        }
        current = ancestor.parent();
    }
    None
}

fn nearest_lexical_scope(module: &ModuleNode, node: &SyntaxNode) -> Option<Range<usize>> {
    let output = module.parse_output();
    let mut current = Some(node.id());
    while let Some(id) = current {
        let ancestor = output.node(id)?;
        if matches!(
            ancestor.kind(),
            SyntaxNodeKind::Block | SyntaxNodeKind::Closure
        ) {
            return Some(ancestor.start()..ancestor.end());
        }
        if id != node.id()
            && ancestor
                .item_syntax()
                .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::Module { .. }))
        {
            return None;
        }
        current = ancestor.parent();
    }
    None
}

fn nearest_use_statement<'a>(
    module: &'a ModuleNode,
    node: &'a SyntaxNode,
) -> Option<&'a SyntaxNode> {
    let output = module.parse_output();
    let mut current = node.parent();
    while let Some(id) = current {
        let ancestor = output.node(id)?;
        if ancestor.kind() == SyntaxNodeKind::Statement {
            return (ancestor.statement_kind() == Some(StatementSyntaxKind::Use))
                .then_some(ancestor);
        }
        current = ancestor.parent();
    }
    None
}

fn is_use_tree_leaf(module: &ModuleNode, node: &SyntaxNode) -> bool {
    node.children().iter().all(|id| {
        module
            .parse_output()
            .node(*id)
            .map_or(true, |child| child.kind() != SyntaxNodeKind::UseTree)
    })
}

fn node_is_potentially_enabled(module: &ModuleNode, node: &SyntaxNode) -> bool {
    module
        .configured_ast()
        .node(node.id())
        .is_some_and(|configured| configured.decision() != CfgDecision::False)
}

fn local_item_binding(item: &SyntaxNode) -> Option<(String, Option<Vec<String>>)> {
    let item_data = item.item_syntax()?;
    let name = match item_data.kind() {
        ItemSyntaxKind::Module { name, .. } => Some(name.as_str()),
        ItemSyntaxKind::Struct(Some(name))
        | ItemSyntaxKind::Enum(Some(name))
        | ItemSyntaxKind::Union(Some(name))
        | ItemSyntaxKind::TypeAlias(Some(name))
        | ItemSyntaxKind::Trait(Some(name))
        | ItemSyntaxKind::Constant(Some(name))
        | ItemSyntaxKind::Static(Some(name))
        | ItemSyntaxKind::Function { name: Some(name) } => Some(name.as_str()),
        _ => None,
    }?;
    Some((name.to_owned(), None))
}

fn identifier_name(name: &str) -> &str {
    name.strip_prefix("r#").unwrap_or(name)
}

fn conditions_may_coexist(left: Option<&str>, right: Option<&str>) -> bool {
    let Some(left) = left else {
        return true;
    };
    let Some(right) = right else {
        return true;
    };
    match (CfgCondition::parse(left), CfgCondition::parse(right)) {
        (Ok(left), Ok(right)) => CfgCondition::may_coexist(&left, &right),
        _ => true,
    }
}
