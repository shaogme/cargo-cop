use std::{collections::BTreeMap, ops::Range};

use cop_syntax::{
    AstNodeId, ExpressionSyntax, ExpressionSyntaxKind, GenericArgumentSyntax, ItemSyntaxKind,
    PathSyntax, PatternSyntax, PatternSyntaxKind, SyntaxNode, SyntaxNodeKind, TraitBoundKind,
    TraitBoundSyntax, TypeSyntax, TypeSyntaxKind, WherePredicateKind,
};

use crate::{CfgDecision, ModuleNode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceKind {
    Path,
    Call,
    Macro,
    Use,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RawReference {
    pub(super) kind: ReferenceKind,
    pub(super) segments: Vec<String>,
    pub(super) bounds: Range<usize>,
    pub(super) condition: Option<String>,
    pub(super) supported: bool,
    pub(super) alias: Option<String>,
    pub(super) glob: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct SyntaxFactCollector {
    references: Vec<RawReference>,
    use_prefixes: BTreeMap<AstNodeId, Vec<String>>,
}

impl SyntaxFactCollector {
    pub(super) fn is_disabled(module: &ModuleNode, start: usize, end: usize) -> bool {
        range_is_disabled(module, start, end)
    }

    pub(super) fn is_in_module_scope(module: &ModuleNode, node: &SyntaxNode) -> bool {
        node_is_in_module_scope(module, node)
    }

    pub(super) fn visit_node(&mut self, module: &ModuleNode, node: &SyntaxNode) {
        if !node_is_in_module_scope(module, node)
            || range_is_disabled(module, node.start(), node.end())
        {
            return;
        }
        if !has_typed_ancestor(module, node) {
            if let Some(expression) = node.expression_syntax() {
                collect_expression_paths(expression, &mut self.references);
            }
            if let Some(syntax) = node.impl_syntax() {
                if let Some(path) = syntax.trait_path() {
                    collect_path_paths(path, &mut self.references);
                }
            }
            if let Some(syntax) = node.type_syntax() {
                collect_type_paths(syntax, &mut self.references);
            }
            if let Some(syntax) = node.pattern_syntax() {
                collect_pattern_paths(syntax, &mut self.references);
            }
            if let Some(parameter) = node.generic_parameter_syntax() {
                for bound in parameter.bounds() {
                    collect_trait_bound_paths(bound, &mut self.references);
                }
                if let Some(ty) = parameter.declared_type() {
                    collect_type_paths(ty, &mut self.references);
                }
                if let Some(ty) = parameter.default_type() {
                    collect_type_paths(ty, &mut self.references);
                }
            }
            if let Some(predicate) = node.where_predicate_syntax() {
                if let WherePredicateKind::Type {
                    bounded_type,
                    bounds,
                    ..
                } = predicate.kind()
                {
                    collect_type_paths(bounded_type, &mut self.references);
                    for bound in bounds {
                        collect_trait_bound_paths(bound, &mut self.references);
                    }
                }
            }
        }
        if let Some(invocation) = node.macro_invocation() {
            let path = invocation.path();
            self.references.push(RawReference {
                kind: ReferenceKind::Macro,
                segments: path_segments(path),
                condition: None,
                supported: true,
                bounds: path.bounds(),
                alias: None,
                glob: false,
            });
        }
        if let Some(tree) = node.use_tree() {
            let Some(configured) = module.configured_ast().node(node.id()) else {
                return;
            };
            if configured.availability_decision() == CfgDecision::False {
                return;
            }
            let mut path = node
                .parent()
                .and_then(|parent| self.use_prefixes.get(&parent).cloned())
                .unwrap_or_default();
            path.extend(tree.path().iter().cloned());
            self.use_prefixes.insert(node.id(), path.clone());
            let has_child_tree = node.children().iter().any(|id| {
                module
                    .parse_output()
                    .node(*id)
                    .is_some_and(|child| child.kind() == SyntaxNodeKind::UseTree)
            });
            if !has_child_tree
                && module.configured_ast().is_supported_range(
                    module.parse_output(),
                    node.start(),
                    node.end(),
                )
            {
                self.references.push(RawReference {
                    kind: ReferenceKind::Use,
                    segments: path,
                    bounds: node.start()..node.end(),
                    condition: configured.availability_condition().map(str::to_owned),
                    supported: true,
                    alias: tree.alias().map(str::to_owned),
                    glob: tree.is_glob(),
                });
            }
        }
    }

    pub(super) fn finish(mut self, module: &ModuleNode) -> Vec<RawReference> {
        let output = module.parse_output();
        for reference in &mut self.references {
            if reference.condition.is_none() {
                reference.condition = condition_for_range(module, &reference.bounds);
            }
            reference.supported &= module.configured_ast().is_supported_range(
                output,
                reference.bounds.start,
                reference.bounds.end,
            );
        }
        self.references.retain(|reference| {
            !range_is_disabled(module, reference.bounds.start, reference.bounds.end)
        });
        self.references.sort_unstable_by(|left, right| {
            left.bounds
                .start
                .cmp(&right.bounds.start)
                .then_with(|| left.bounds.end.cmp(&right.bounds.end))
                .then_with(|| (left.kind as u8).cmp(&(right.kind as u8)))
                .then_with(|| left.segments.cmp(&right.segments))
        });
        self.references.dedup_by(|left, right| {
            left.bounds == right.bounds
                && left.segments == right.segments
                && left.kind == right.kind
                && left.alias == right.alias
                && left.glob == right.glob
        });
        self.references
    }
}

fn has_typed_ancestor(module: &ModuleNode, node: &SyntaxNode) -> bool {
    let output = module.parse_output();
    let mut current = node.parent();
    while let Some(id) = current {
        let Some(ancestor) = output.node(id) else {
            return false;
        };
        let has_typed_data = ancestor.expression_syntax().is_some()
            || ancestor.type_syntax().is_some()
            || ancestor.pattern_syntax().is_some()
            || ancestor.generic_parameter_syntax().is_some()
            || ancestor.where_predicate_syntax().is_some();
        if has_typed_data && ancestor.start() <= node.start() && node.end() <= ancestor.end() {
            return true;
        }
        current = ancestor.parent();
    }
    false
}

fn node_is_in_module_scope(module: &ModuleNode, node: &SyntaxNode) -> bool {
    let output = module.parse_output();
    let mut current = Some(node.id());
    while let Some(id) = current {
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
        if id == module.scope_id() {
            return true;
        }
        current = ancestor.parent();
    }
    false
}

fn condition_for_range(module: &ModuleNode, bounds: &Range<usize>) -> Option<String> {
    module
        .configured_ast()
        .condition_for_span(bounds.start, bounds.end)
        .map(str::to_owned)
}

fn range_is_disabled(module: &ModuleNode, start: usize, end: usize) -> bool {
    module
        .configured_ast()
        .node_for_range(start, end)
        .is_some_and(|node| node.availability_decision() == CfgDecision::False)
}

fn collect_expression_paths(expression: &ExpressionSyntax, references: &mut Vec<RawReference>) {
    match expression.kind() {
        ExpressionSyntaxKind::Path(path) => collect_path_paths(path, references),
        ExpressionSyntaxKind::Group(inner)
        | ExpressionSyntaxKind::Unary { operand: inner, .. }
        | ExpressionSyntaxKind::Await(inner)
        | ExpressionSyntaxKind::Try(inner) => collect_expression_paths(inner, references),
        ExpressionSyntaxKind::Tuple(values) | ExpressionSyntaxKind::Array(values) => {
            for value in values {
                collect_expression_paths(value, references);
            }
        }
        ExpressionSyntaxKind::Repeat { value, count } => {
            collect_expression_paths(value, references);
            collect_expression_paths(count, references);
        }
        ExpressionSyntaxKind::Struct { path, fields } => {
            collect_path_paths(path, references);
            for field in fields {
                if let Some(value) = field.value() {
                    collect_expression_paths(value, references);
                }
            }
        }
        ExpressionSyntaxKind::Binary { left, right, .. }
        | ExpressionSyntaxKind::Assignment { left, right, .. } => {
            collect_expression_paths(left, references);
            collect_expression_paths(right, references);
        }
        ExpressionSyntaxKind::Range { start, end, .. } => {
            for value in start.iter().chain(end.iter()) {
                collect_expression_paths(value, references);
            }
        }
        ExpressionSyntaxKind::Call { callee, arguments } => {
            if let ExpressionSyntaxKind::Path(path) = callee.kind() {
                collect_path_paths_with_kind(path, ReferenceKind::Call, references);
            } else {
                collect_expression_paths(callee, references);
            }
            for argument in arguments {
                collect_expression_paths(argument, references);
            }
        }
        ExpressionSyntaxKind::MethodCall {
            receiver,
            generic_arguments,
            arguments,
            ..
        } => {
            collect_expression_paths(receiver, references);
            for argument in generic_arguments {
                collect_generic_argument_paths(argument, references);
            }
            for argument in arguments {
                collect_expression_paths(argument, references);
            }
        }
        ExpressionSyntaxKind::Field { base, .. }
        | ExpressionSyntaxKind::TupleField { base, .. } => {
            collect_expression_paths(base, references)
        }
        ExpressionSyntaxKind::Index { base, index } => {
            collect_expression_paths(base, references);
            collect_expression_paths(index, references);
        }
        ExpressionSyntaxKind::Cast { expression, target } => {
            collect_expression_paths(expression, references);
            collect_type_paths(target, references);
        }
        ExpressionSyntaxKind::Closure {
            parameters, body, ..
        } => {
            for parameter in parameters {
                collect_pattern_paths(parameter.pattern(), references);
                if let Some(ty) = parameter.ty() {
                    collect_type_paths(ty, references);
                }
            }
            collect_expression_paths(body, references);
        }
        ExpressionSyntaxKind::If {
            condition,
            else_expression,
            ..
        } => {
            collect_expression_paths(condition, references);
            if let Some(expression) = else_expression {
                collect_expression_paths(expression, references);
            }
        }
        ExpressionSyntaxKind::Match { scrutinee, arms } => {
            collect_expression_paths(scrutinee, references);
            for arm in arms {
                if let Some(pattern) = arm.pattern() {
                    collect_pattern_paths(pattern, references);
                }
                if let Some(guard) = arm.guard() {
                    collect_expression_paths(guard, references);
                }
                collect_expression_paths(arm.body(), references);
            }
        }
        ExpressionSyntaxKind::While { condition, .. } => {
            collect_expression_paths(condition, references);
        }
        ExpressionSyntaxKind::For {
            pattern, iterator, ..
        } => {
            collect_pattern_paths(pattern, references);
            collect_expression_paths(iterator, references);
        }
        ExpressionSyntaxKind::Return(value) => {
            if let Some(value) = value {
                collect_expression_paths(value, references);
            }
        }
        ExpressionSyntaxKind::Break { value, .. } => {
            if let Some(value) = value {
                collect_expression_paths(value, references);
            }
        }
        ExpressionSyntaxKind::Let { pattern, value } => {
            collect_pattern_paths(pattern, references);
            collect_expression_paths(value, references);
        }
        ExpressionSyntaxKind::Macro { path, .. } => collect_macro_path_paths(path, references),
        ExpressionSyntaxKind::Literal
        | ExpressionSyntaxKind::Block { .. }
        | ExpressionSyntaxKind::LabelBlock { .. }
        | ExpressionSyntaxKind::Loop { .. }
        | ExpressionSyntaxKind::Continue(_)
        | ExpressionSyntaxKind::Underscore
        | ExpressionSyntaxKind::Error(_)
        | ExpressionSyntaxKind::Unsupported(_) => {}
    }
}

fn collect_pattern_paths(pattern: &PatternSyntax, references: &mut Vec<RawReference>) {
    match pattern.kind() {
        PatternSyntaxKind::Reference { pattern, .. } => collect_pattern_paths(pattern, references),
        PatternSyntaxKind::Range { start, end, .. } => {
            for value in start.iter().chain(end.iter()) {
                collect_pattern_paths(value, references);
            }
        }
        PatternSyntaxKind::Tuple(elements)
        | PatternSyntaxKind::Slice(elements)
        | PatternSyntaxKind::Or(elements) => {
            for element in elements {
                collect_pattern_paths(element, references);
            }
        }
        PatternSyntaxKind::TupleStruct { path, elements } => {
            collect_path_paths(path, references);
            for element in elements {
                collect_pattern_paths(element, references);
            }
        }
        PatternSyntaxKind::Struct { path, fields, .. } => {
            collect_path_paths(path, references);
            for field in fields {
                if let Some(pattern) = field.pattern() {
                    collect_pattern_paths(pattern, references);
                }
            }
        }
        PatternSyntaxKind::Macro { path, .. } => collect_macro_path_paths(path, references),
        PatternSyntaxKind::Path(path) => collect_path_paths(path, references),
        PatternSyntaxKind::Binding { subpattern, .. } => {
            if let Some(pattern) = subpattern {
                collect_pattern_paths(pattern, references);
            }
        }
        PatternSyntaxKind::Wildcard | PatternSyntaxKind::Literal(_) | PatternSyntaxKind::Rest => {}
    }
}

fn collect_type_paths(ty: &TypeSyntax, references: &mut Vec<RawReference>) {
    match ty.kind() {
        TypeSyntaxKind::Path(path) => collect_path_paths(path, references),
        TypeSyntaxKind::Reference { target, .. }
        | TypeSyntaxKind::Pointer { target, .. }
        | TypeSyntaxKind::Parenthesized(target)
        | TypeSyntaxKind::Slice(target) => collect_type_paths(target, references),
        TypeSyntaxKind::Tuple(types) => {
            for ty in types {
                collect_type_paths(ty, references);
            }
        }
        TypeSyntaxKind::Array { element, .. } => collect_type_paths(element, references),
        TypeSyntaxKind::FunctionPointer {
            parameters,
            return_type,
            ..
        } => {
            for parameter in parameters {
                collect_type_paths(parameter, references);
            }
            if let Some(return_type) = return_type {
                collect_type_paths(return_type, references);
            }
        }
        TypeSyntaxKind::ImplTrait(bounds)
        | TypeSyntaxKind::DynTrait(bounds)
        | TypeSyntaxKind::TraitBounds(bounds) => {
            for bound in bounds {
                collect_trait_bound_paths(bound, references);
            }
        }
        TypeSyntaxKind::Macro { path, .. } => collect_macro_path_paths(path, references),
        TypeSyntaxKind::Never | TypeSyntaxKind::Infer => {}
    }
}

fn collect_trait_bound_paths(bound: &TraitBoundSyntax, references: &mut Vec<RawReference>) {
    if let TraitBoundKind::Trait(path) = bound.kind() {
        collect_path_paths(path, references);
    }
    if let Some(arguments) = bound.function_arguments() {
        for parameter in arguments.parameters() {
            collect_type_paths(parameter, references);
        }
        if let Some(return_type) = arguments.return_type() {
            collect_type_paths(return_type, references);
        }
    }
}

fn collect_generic_argument_paths(
    argument: &GenericArgumentSyntax,
    references: &mut Vec<RawReference>,
) {
    match argument {
        GenericArgumentSyntax::Type(ty) => collect_type_paths(ty, references),
        GenericArgumentSyntax::AssociatedType { bounds, value, .. } => {
            for bound in bounds {
                collect_trait_bound_paths(bound, references);
            }
            collect_type_paths(value, references);
        }
        GenericArgumentSyntax::Lifetime(_)
        | GenericArgumentSyntax::Const(_)
        | GenericArgumentSyntax::AssociatedConst { .. } => {}
    }
}

fn collect_path_paths(path: &PathSyntax, references: &mut Vec<RawReference>) {
    collect_path_paths_with_kind(path, ReferenceKind::Path, references);
}

fn collect_path_paths_with_kind(
    path: &PathSyntax,
    kind: ReferenceKind,
    references: &mut Vec<RawReference>,
) {
    if let Some(self_type) = path.qualified_self() {
        collect_type_paths(self_type, references);
    }
    if let Some(trait_path) = path.qualified_trait() {
        collect_path_paths(trait_path, references);
    }
    let bounds = path.bounds();
    references.push(RawReference {
        kind,
        segments: path_segments(path),
        condition: None,
        supported: true,
        bounds,
        alias: None,
        glob: false,
    });
    for segment in path.segments() {
        for argument in segment.arguments() {
            collect_generic_argument_paths(argument, references);
        }
    }
}

fn collect_macro_path_paths(path: &PathSyntax, references: &mut Vec<RawReference>) {
    if let Some(self_type) = path.qualified_self() {
        collect_type_paths(self_type, references);
    }
    if let Some(trait_path) = path.qualified_trait() {
        collect_path_paths(trait_path, references);
    }
    let bounds = path.bounds();
    references.push(RawReference {
        kind: ReferenceKind::Macro,
        segments: path_segments(path),
        condition: None,
        supported: true,
        bounds,
        alias: None,
        glob: false,
    });
    for segment in path.segments() {
        for argument in segment.arguments() {
            collect_generic_argument_paths(argument, references);
        }
    }
}

fn path_segments(path: &PathSyntax) -> Vec<String> {
    let mut segments = path
        .segments()
        .iter()
        .map(|segment| segment.name().to_owned())
        .collect::<Vec<_>>();
    if path.has_leading_colon() {
        segments.insert(0, "::".to_owned());
    }
    segments
}
