use std::ops::Range;

use cop_cfg::CfgCondition;
use cop_syntax::{
    ExpressionSyntax, ExpressionSyntaxKind, ItemSyntaxKind, PatternSyntax, PatternSyntaxKind,
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

struct PatternBindingScope<'a> {
    module: &'a ModuleNode,
    scope: &'a Range<usize>,
    visible_from: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LocalBindingMatch {
    name: String,
    target_path: Option<Vec<String>>,
    scope_start: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct LocalBindings {
    bindings: Vec<LocalBinding>,
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
    pub(super) fn build(modules: &[ModuleNode]) -> Self {
        let mut bindings = Vec::new();
        for module in modules {
            for item in module.item_nodes() {
                Self::collect_item_node(item, module, &mut bindings);
            }
        }
        Self { bindings }
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

    fn collect_item_node(
        item_node: &SyntaxNode,
        module: &ModuleNode,
        bindings: &mut Vec<LocalBinding>,
    ) {
        if !node_is_potentially_enabled(module, item_node) {
            return;
        }
        let Some(item_data) = item_node.item_syntax() else {
            return;
        };
        match item_data.kind() {
            ItemSyntaxKind::Function { .. } => {
                Self::collect_function_bindings(item_node, module, bindings)
            }
            ItemSyntaxKind::Trait(_) | ItemSyntaxKind::Impl => {
                let output = module.parse_output();
                let Some(scope) = item_node
                    .children()
                    .iter()
                    .filter_map(|id| output.node(*id))
                    .find(|node| node.kind() == SyntaxNodeKind::Scope)
                else {
                    return;
                };
                for child in scope
                    .children()
                    .iter()
                    .filter_map(|id| output.node(*id))
                    .filter(|node| node.item_syntax().is_some())
                {
                    Self::collect_item_node(child, module, bindings);
                }
            }
            _ => {}
        }
    }

    fn collect_function_bindings(
        item_node: &SyntaxNode,
        module: &ModuleNode,
        bindings: &mut Vec<LocalBinding>,
    ) {
        let output = module.parse_output();
        let Some(body) = item_node
            .children()
            .iter()
            .filter_map(|id| output.node(*id))
            .find(|node| node.kind() == SyntaxNodeKind::Block)
        else {
            return;
        };
        let scope = body.start()..body.end();
        for parameter in item_node
            .children()
            .iter()
            .filter_map(|id| output.node(*id))
            .filter(|node| node.kind() == SyntaxNodeKind::Parameter)
        {
            if !node_is_potentially_enabled(module, parameter) {
                continue;
            }
            for pattern in parameter
                .children()
                .iter()
                .filter_map(|id| output.node(*id))
                .filter(|node| node.kind() == SyntaxNodeKind::Pattern)
            {
                Self::add_pattern_bindings(module, pattern, &scope, body.start(), bindings);
            }
        }
        Self::collect_scope_node(body, module, &scope, bindings);
    }

    fn collect_scope_node(
        node: &SyntaxNode,
        module: &ModuleNode,
        scope: &Range<usize>,
        bindings: &mut Vec<LocalBinding>,
    ) {
        if !node_is_potentially_enabled(module, node) {
            return;
        }
        if node.kind() == SyntaxNodeKind::LocalItem {
            Self::collect_local_item(node, module, scope, bindings);
            return;
        }
        if node.kind() == SyntaxNodeKind::Statement {
            Self::collect_statement(node, module, scope, bindings);
        }
        if node.kind() == SyntaxNodeKind::MatchArm {
            let arm_scope = node.start()..node.end();
            for pattern in node
                .children()
                .iter()
                .filter_map(|id| module.parse_output().node(*id))
                .filter(|child| child.kind() == SyntaxNodeKind::Pattern)
            {
                Self::add_pattern_bindings(module, pattern, &arm_scope, arm_scope.start, bindings);
            }
        }
        if let Some(expression) = node.expression_syntax() {
            Self::collect_expression_bindings(node, expression, module, bindings);
        }
        if node.kind() == SyntaxNodeKind::Closure {
            Self::collect_closure_parameters(node, module, bindings);
        }
        let child_scope = if matches!(node.kind(), SyntaxNodeKind::Block | SyntaxNodeKind::Closure)
        {
            node.start()..node.end()
        } else {
            scope.clone()
        };
        for child in node
            .children()
            .iter()
            .filter_map(|id| module.parse_output().node(*id))
        {
            if child.kind() == SyntaxNodeKind::Parameter {
                continue;
            }
            Self::collect_scope_node(child, module, &child_scope, bindings);
        }
    }

    fn collect_statement(
        statement: &SyntaxNode,
        module: &ModuleNode,
        scope: &Range<usize>,
        bindings: &mut Vec<LocalBinding>,
    ) {
        let output = module.parse_output();
        let children = statement
            .children()
            .iter()
            .filter_map(|id| output.node(*id))
            .collect::<Vec<_>>();
        if statement.statement_kind() == Some(StatementSyntaxKind::Let) {
            for pattern in children
                .iter()
                .copied()
                .filter(|node| node.kind() == SyntaxNodeKind::Pattern)
            {
                Self::add_pattern_bindings(module, pattern, scope, statement.end(), bindings);
            }
        }
        for tree in children
            .iter()
            .copied()
            .filter(|node| node.kind() == SyntaxNodeKind::UseTree)
        {
            Self::collect_use_tree(module, tree, &[], scope, bindings);
        }
    }

    fn collect_local_item(
        item: &SyntaxNode,
        module: &ModuleNode,
        scope: &Range<usize>,
        bindings: &mut Vec<LocalBinding>,
    ) {
        if let Some((name, target_path)) = local_item_binding(item) {
            Self::push_binding(
                LocalBindingInput {
                    module,
                    node: item,
                    name,
                    target_path,
                    scope,
                    visible_from: scope.start,
                },
                bindings,
            );
        }
        Self::collect_item_node(item, module, bindings);
    }

    fn collect_closure_parameters(
        closure: &SyntaxNode,
        module: &ModuleNode,
        bindings: &mut Vec<LocalBinding>,
    ) {
        let scope = closure.start()..closure.end();
        for parameter in closure
            .children()
            .iter()
            .filter_map(|id| module.parse_output().node(*id))
            .filter(|node| node.kind() == SyntaxNodeKind::Parameter)
        {
            if !node_is_potentially_enabled(module, parameter) {
                continue;
            }
            for pattern in parameter
                .children()
                .iter()
                .filter_map(|id| module.parse_output().node(*id))
                .filter(|node| node.kind() == SyntaxNodeKind::Pattern)
            {
                Self::add_pattern_bindings(module, pattern, &scope, closure.start(), bindings);
            }
        }
    }

    fn add_pattern_bindings(
        module: &ModuleNode,
        pattern: &SyntaxNode,
        scope: &Range<usize>,
        visible_from: usize,
        bindings: &mut Vec<LocalBinding>,
    ) {
        if !node_is_potentially_enabled(module, pattern) {
            return;
        }
        let Some(syntax) = pattern.pattern_syntax() else {
            return;
        };
        Self::collect_pattern_bindings(
            pattern,
            syntax,
            PatternBindingScope {
                module,
                scope,
                visible_from,
            },
            bindings,
        );
    }

    fn collect_pattern_bindings(
        node: &SyntaxNode,
        syntax: &PatternSyntax,
        binding_scope: PatternBindingScope<'_>,
        bindings: &mut Vec<LocalBinding>,
    ) {
        let PatternBindingScope {
            module,
            scope,
            visible_from,
        } = binding_scope;
        if !node_is_potentially_enabled(module, node) {
            return;
        }
        if let PatternSyntaxKind::Binding { name, .. } = syntax.kind() {
            Self::push_binding(
                LocalBindingInput {
                    module,
                    node,
                    name: name.clone(),
                    target_path: None,
                    scope,
                    visible_from,
                },
                bindings,
            );
        }
        for child in node
            .children()
            .iter()
            .filter_map(|id| module.parse_output().node(*id))
            .filter(|child| child.kind() == SyntaxNodeKind::Pattern)
        {
            if let Some(child_syntax) = child.pattern_syntax() {
                Self::collect_pattern_bindings(
                    child,
                    child_syntax,
                    PatternBindingScope {
                        module,
                        scope,
                        visible_from,
                    },
                    bindings,
                );
            }
        }
    }

    fn collect_expression_bindings(
        root: &SyntaxNode,
        expression: &ExpressionSyntax,
        module: &ModuleNode,
        bindings: &mut Vec<LocalBinding>,
    ) {
        match expression.kind() {
            ExpressionSyntaxKind::For { pattern, body, .. } => {
                Self::add_syntax_pattern_bindings(
                    root,
                    pattern,
                    PatternBindingScope {
                        module,
                        scope: body,
                        visible_from: body.start,
                    },
                    bindings,
                );
            }
            ExpressionSyntaxKind::If {
                condition,
                then_block,
                ..
            } => {
                Self::collect_condition_bindings(root, condition, then_block, module, bindings);
            }
            ExpressionSyntaxKind::While {
                condition, body, ..
            } => Self::collect_condition_bindings(root, condition, body, module, bindings),
            _ => {}
        }
    }

    fn collect_condition_bindings(
        root: &SyntaxNode,
        expression: &ExpressionSyntax,
        scope: &Range<usize>,
        module: &ModuleNode,
        bindings: &mut Vec<LocalBinding>,
    ) {
        if let ExpressionSyntaxKind::Let { pattern, .. } = expression.kind() {
            Self::add_syntax_pattern_bindings(
                root,
                pattern,
                PatternBindingScope {
                    module,
                    scope,
                    visible_from: scope.start,
                },
                bindings,
            );
        }
        match expression.kind() {
            ExpressionSyntaxKind::Group(inner)
            | ExpressionSyntaxKind::Unary { operand: inner, .. }
            | ExpressionSyntaxKind::Await(inner)
            | ExpressionSyntaxKind::Try(inner) => {
                Self::collect_condition_bindings(root, inner, scope, module, bindings);
            }
            ExpressionSyntaxKind::Binary { left, right, .. }
            | ExpressionSyntaxKind::Assignment { left, right, .. } => {
                Self::collect_condition_bindings(root, left, scope, module, bindings);
                Self::collect_condition_bindings(root, right, scope, module, bindings);
            }
            _ => {}
        }
    }

    fn add_syntax_pattern_bindings(
        root: &SyntaxNode,
        syntax: &PatternSyntax,
        binding_scope: PatternBindingScope<'_>,
        bindings: &mut Vec<LocalBinding>,
    ) {
        let module = binding_scope.module;
        let output = module.parse_output();
        let mut pending = vec![root.id()];
        while let Some(id) = pending.pop() {
            let Some(node) = output.node(id) else {
                continue;
            };
            if node.kind() == SyntaxNodeKind::Pattern
                && node.start() == syntax.bounds().start
                && node.end() == syntax.bounds().end
            {
                Self::add_pattern_bindings(
                    module,
                    node,
                    binding_scope.scope,
                    binding_scope.visible_from,
                    bindings,
                );
                return;
            }
            pending.extend(node.children().iter().rev().copied());
        }
    }

    fn collect_use_tree(
        module: &ModuleNode,
        tree: &SyntaxNode,
        prefix: &[String],
        scope: &Range<usize>,
        bindings: &mut Vec<LocalBinding>,
    ) {
        if !node_is_potentially_enabled(module, tree) {
            return;
        }
        let Some(tree_data) = tree.use_tree() else {
            return;
        };
        let mut path = prefix.to_vec();
        path.extend(tree_data.path().iter().cloned());
        let children = tree
            .children()
            .iter()
            .filter_map(|id| module.parse_output().node(*id))
            .filter(|child| child.kind() == SyntaxNodeKind::UseTree)
            .collect::<Vec<_>>();
        if !children.is_empty() {
            for child in children {
                Self::collect_use_tree(module, child, &path, scope, bindings);
            }
            return;
        }
        if tree_data.is_glob() || path.is_empty() {
            return;
        }
        if path.last().is_some_and(|segment| segment == "self") {
            path.pop();
        }
        let name = tree_data
            .alias()
            .map(str::to_owned)
            .or_else(|| path.last().cloned());
        if let Some(name) = name {
            Self::push_binding(
                LocalBindingInput {
                    module,
                    node: tree,
                    name,
                    target_path: Some(path),
                    scope,
                    visible_from: scope.start,
                },
                bindings,
            );
        }
    }

    fn push_binding(input: LocalBindingInput<'_>, bindings: &mut Vec<LocalBinding>) {
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
        bindings.push(LocalBinding {
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
