use cop_syntax::{
    ExpressionSyntax, ExpressionSyntaxKind, GenericArgumentSyntax, PathSyntax, PatternSyntax,
    PatternSyntaxKind, TraitBoundKind, TraitBoundSyntax, TypeSyntax, TypeSyntaxKind,
};

use super::PathReference;

pub(super) struct PathSyntaxCollector;

impl PathSyntaxCollector {
    pub(super) fn collect_expression(
        expression: &ExpressionSyntax,
        paths: &mut Vec<PathReference>,
    ) {
        collect_expression_paths(expression, paths);
    }

    pub(super) fn collect_pattern(pattern: &PatternSyntax, paths: &mut Vec<PathReference>) {
        collect_pattern_paths(pattern, paths);
    }

    pub(super) fn collect_type(ty: &TypeSyntax, paths: &mut Vec<PathReference>) {
        collect_type_paths(ty, paths);
    }

    pub(super) fn collect_trait_bound(bound: &TraitBoundSyntax, paths: &mut Vec<PathReference>) {
        collect_trait_bound_paths(bound, paths);
    }

    pub(super) fn collect_path(path: &PathSyntax, paths: &mut Vec<PathReference>) {
        collect_path_paths(path, paths);
    }

    pub(super) fn path_segments(path: &PathSyntax) -> Vec<String> {
        path_segment_names(path)
    }
}

fn collect_expression_paths(expression: &ExpressionSyntax, paths: &mut Vec<PathReference>) {
    match expression.kind() {
        ExpressionSyntaxKind::Path(path) => collect_path_paths(path, paths),
        ExpressionSyntaxKind::Group(inner)
        | ExpressionSyntaxKind::Unary { operand: inner, .. }
        | ExpressionSyntaxKind::Await(inner)
        | ExpressionSyntaxKind::Try(inner) => collect_expression_paths(inner, paths),
        ExpressionSyntaxKind::Tuple(values) | ExpressionSyntaxKind::Array(values) => {
            for value in values {
                collect_expression_paths(value, paths);
            }
        }
        ExpressionSyntaxKind::Repeat { value, count } => {
            collect_expression_paths(value, paths);
            collect_expression_paths(count, paths);
        }
        ExpressionSyntaxKind::Struct { path, fields } => {
            collect_path_paths(path, paths);
            for field in fields {
                if let Some(value) = field.value() {
                    collect_expression_paths(value, paths);
                }
            }
        }
        ExpressionSyntaxKind::Binary { left, right, .. }
        | ExpressionSyntaxKind::Assignment { left, right, .. } => {
            collect_expression_paths(left, paths);
            collect_expression_paths(right, paths);
        }
        ExpressionSyntaxKind::Range { start, end, .. } => {
            for value in start.iter().chain(end.iter()) {
                collect_expression_paths(value, paths);
            }
        }
        ExpressionSyntaxKind::Call { callee, arguments } => {
            collect_expression_paths(callee, paths);
            for argument in arguments {
                collect_expression_paths(argument, paths);
            }
        }
        ExpressionSyntaxKind::MethodCall {
            receiver,
            generic_arguments,
            arguments,
            ..
        } => {
            collect_expression_paths(receiver, paths);
            for argument in generic_arguments {
                collect_generic_argument_paths(argument, paths);
            }
            for argument in arguments {
                collect_expression_paths(argument, paths);
            }
        }
        ExpressionSyntaxKind::Field { base, .. }
        | ExpressionSyntaxKind::TupleField { base, .. } => collect_expression_paths(base, paths),
        ExpressionSyntaxKind::Index { base, index } => {
            collect_expression_paths(base, paths);
            collect_expression_paths(index, paths);
        }
        ExpressionSyntaxKind::Cast { expression, target } => {
            collect_expression_paths(expression, paths);
            collect_type_paths(target, paths);
        }
        ExpressionSyntaxKind::Closure {
            parameters, body, ..
        } => {
            for parameter in parameters {
                collect_pattern_paths(parameter.pattern(), paths);
                if let Some(ty) = parameter.ty() {
                    collect_type_paths(ty, paths);
                }
            }
            collect_expression_paths(body, paths);
        }
        ExpressionSyntaxKind::If {
            condition,
            else_expression,
            ..
        } => {
            collect_expression_paths(condition, paths);
            if let Some(expression) = else_expression {
                collect_expression_paths(expression, paths);
            }
        }
        ExpressionSyntaxKind::Match { scrutinee, arms } => {
            collect_expression_paths(scrutinee, paths);
            for arm in arms {
                if let Some(pattern) = arm.pattern() {
                    collect_pattern_paths(pattern, paths);
                }
                if let Some(guard) = arm.guard() {
                    collect_expression_paths(guard, paths);
                }
                collect_expression_paths(arm.body(), paths);
            }
        }
        ExpressionSyntaxKind::While { condition, .. } => {
            collect_expression_paths(condition, paths);
        }
        ExpressionSyntaxKind::For {
            pattern, iterator, ..
        } => {
            collect_pattern_paths(pattern, paths);
            collect_expression_paths(iterator, paths);
        }
        ExpressionSyntaxKind::Return(value) => {
            if let Some(value) = value {
                collect_expression_paths(value, paths);
            }
        }
        ExpressionSyntaxKind::Break { value, .. } => {
            if let Some(value) = value {
                collect_expression_paths(value, paths);
            }
        }
        ExpressionSyntaxKind::Let { pattern, value } => {
            collect_pattern_paths(pattern, paths);
            collect_expression_paths(value, paths);
        }
        ExpressionSyntaxKind::Macro { path, .. } => collect_macro_path_paths(path, paths),
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

fn collect_pattern_paths(pattern: &PatternSyntax, paths: &mut Vec<PathReference>) {
    match pattern.kind() {
        PatternSyntaxKind::Reference { pattern, .. } => collect_pattern_paths(pattern, paths),
        PatternSyntaxKind::Range { start, end, .. } => {
            for value in start.iter().chain(end.iter()) {
                collect_pattern_paths(value, paths);
            }
        }
        PatternSyntaxKind::Tuple(elements)
        | PatternSyntaxKind::Slice(elements)
        | PatternSyntaxKind::Or(elements) => {
            for element in elements {
                collect_pattern_paths(element, paths);
            }
        }
        PatternSyntaxKind::TupleStruct { path, elements } => {
            collect_path_paths(path, paths);
            for element in elements {
                collect_pattern_paths(element, paths);
            }
        }
        PatternSyntaxKind::Struct { path, fields, .. } => {
            collect_path_paths(path, paths);
            for field in fields {
                if let Some(pattern) = field.pattern() {
                    collect_pattern_paths(pattern, paths);
                }
            }
        }
        PatternSyntaxKind::Macro { path, .. } => collect_macro_path_paths(path, paths),
        PatternSyntaxKind::Path(path) => {
            collect_path_paths(path, paths);
        }
        PatternSyntaxKind::Binding { subpattern, .. } => {
            if let Some(pattern) = subpattern {
                collect_pattern_paths(pattern, paths);
            }
        }
        PatternSyntaxKind::Wildcard | PatternSyntaxKind::Literal(_) | PatternSyntaxKind::Rest => {}
    }
}

fn collect_type_paths(ty: &TypeSyntax, paths: &mut Vec<PathReference>) {
    match ty.kind() {
        TypeSyntaxKind::Path(path) => collect_path_paths(path, paths),
        TypeSyntaxKind::Reference { target, .. }
        | TypeSyntaxKind::Pointer { target, .. }
        | TypeSyntaxKind::Parenthesized(target)
        | TypeSyntaxKind::Slice(target) => collect_type_paths(target, paths),
        TypeSyntaxKind::Tuple(types) => {
            for ty in types {
                collect_type_paths(ty, paths);
            }
        }
        TypeSyntaxKind::Array { element, .. } => collect_type_paths(element, paths),
        TypeSyntaxKind::FunctionPointer {
            parameters,
            return_type,
            ..
        } => {
            for parameter in parameters {
                collect_type_paths(parameter, paths);
            }
            if let Some(return_type) = return_type {
                collect_type_paths(return_type, paths);
            }
        }
        TypeSyntaxKind::ImplTrait(bounds)
        | TypeSyntaxKind::DynTrait(bounds)
        | TypeSyntaxKind::TraitBounds(bounds) => {
            for bound in bounds {
                collect_trait_bound_paths(bound, paths);
            }
        }
        TypeSyntaxKind::Macro { path, .. } => collect_macro_path_paths(path, paths),
        TypeSyntaxKind::Never | TypeSyntaxKind::Infer => {}
    }
}

fn collect_trait_bound_paths(bound: &TraitBoundSyntax, paths: &mut Vec<PathReference>) {
    if let TraitBoundKind::Trait(path) = bound.kind() {
        collect_path_paths(path, paths);
    }
    if let Some(arguments) = bound.function_arguments() {
        for parameter in arguments.parameters() {
            collect_type_paths(parameter, paths);
        }
        if let Some(return_type) = arguments.return_type() {
            collect_type_paths(return_type, paths);
        }
    }
}

fn collect_generic_argument_paths(
    argument: &GenericArgumentSyntax,
    paths: &mut Vec<PathReference>,
) {
    match argument {
        GenericArgumentSyntax::Type(ty) => collect_type_paths(ty, paths),
        GenericArgumentSyntax::AssociatedType { bounds, value, .. } => {
            for bound in bounds {
                collect_trait_bound_paths(bound, paths);
            }
            collect_type_paths(value, paths);
        }
        GenericArgumentSyntax::Lifetime(_)
        | GenericArgumentSyntax::Const(_)
        | GenericArgumentSyntax::AssociatedConst { .. } => {}
    }
}

fn collect_path_paths(path: &PathSyntax, paths: &mut Vec<PathReference>) {
    if let Some(self_type) = path.qualified_self() {
        collect_type_paths(self_type, paths);
    }
    if let Some(trait_path) = path.qualified_trait() {
        collect_path_paths(trait_path, paths);
    }
    let mut segments = path
        .segments()
        .iter()
        .map(|segment| segment.name().to_owned())
        .collect::<Vec<_>>();
    if path.has_leading_colon() {
        segments.insert(0, "::".to_owned());
    }
    let bounds = path.bounds();
    paths.push(PathReference {
        segments,
        start: bounds.start,
        end: bounds.end,
        is_macro: false,
    });
    for segment in path.segments() {
        for argument in segment.arguments() {
            collect_generic_argument_paths(argument, paths);
        }
    }
}

fn collect_macro_path_paths(path: &PathSyntax, paths: &mut Vec<PathReference>) {
    if let Some(self_type) = path.qualified_self() {
        collect_type_paths(self_type, paths);
    }
    if let Some(trait_path) = path.qualified_trait() {
        collect_path_paths(trait_path, paths);
    }
    let bounds = path.bounds();
    paths.push(PathReference {
        segments: path_segment_names(path),
        start: bounds.start,
        end: bounds.end,
        is_macro: true,
    });
    for segment in path.segments() {
        for argument in segment.arguments() {
            collect_generic_argument_paths(argument, paths);
        }
    }
}

fn path_segment_names(path: &PathSyntax) -> Vec<String> {
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
