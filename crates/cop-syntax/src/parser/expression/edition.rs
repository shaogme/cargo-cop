use std::ops::Range;

use crate::{
    ast::{ExpressionSyntax, ExpressionSyntaxKind},
    RustEdition,
};

pub(super) struct EditionParser;

impl EditionParser {
    pub(super) fn invalid_let_condition(
        condition: &ExpressionSyntax,
        edition: RustEdition,
    ) -> Option<Range<usize>> {
        let (let_bounds, valid_chain) = Self::condition_let_status(condition);
        let is_single_let = matches!(condition.kind(), ExpressionSyntaxKind::Let { .. });
        let_bounds.filter(|_| !is_single_let && (edition != RustEdition::E2024 || !valid_chain))
    }

    fn condition_let_status(expression: &ExpressionSyntax) -> (Option<Range<usize>>, bool) {
        match expression.kind() {
            ExpressionSyntaxKind::Let { .. } => (Some(expression.bounds()), true),
            ExpressionSyntaxKind::Group(inner) => Self::condition_let_status(inner),
            ExpressionSyntaxKind::Binary {
                operator,
                left,
                right,
            } => {
                let left_status = Self::condition_let_status(left);
                let right_status = Self::condition_let_status(right);
                let let_bounds = left_status.0.or(right_status.0);
                let valid_chain = operator == "&&" && left_status.1 && right_status.1;
                (let_bounds, valid_chain)
            }
            _ => match Self::find_let_expression(expression) {
                Some(bounds) => (Some(bounds), false),
                None => (None, true),
            },
        }
    }

    fn find_let_expression(expression: &ExpressionSyntax) -> Option<Range<usize>> {
        match expression.kind() {
            ExpressionSyntaxKind::Let { .. } => Some(expression.bounds()),
            ExpressionSyntaxKind::Group(inner) => Self::find_let_expression(inner),
            ExpressionSyntaxKind::Tuple(values) | ExpressionSyntaxKind::Array(values) => {
                values.iter().find_map(Self::find_let_expression)
            }
            ExpressionSyntaxKind::Repeat { value, count } => {
                Self::find_let_expression(value).or_else(|| Self::find_let_expression(count))
            }
            ExpressionSyntaxKind::Struct { fields, .. } => fields
                .iter()
                .filter_map(|field| field.value())
                .find_map(Self::find_let_expression),
            ExpressionSyntaxKind::Unary { operand, .. }
            | ExpressionSyntaxKind::Await(operand)
            | ExpressionSyntaxKind::Try(operand) => Self::find_let_expression(operand),
            ExpressionSyntaxKind::Binary { left, right, .. }
            | ExpressionSyntaxKind::Assignment { left, right, .. } => {
                Self::find_let_expression(left).or_else(|| Self::find_let_expression(right))
            }
            ExpressionSyntaxKind::Range { start, end, .. } => start
                .as_deref()
                .and_then(Self::find_let_expression)
                .or_else(|| end.as_deref().and_then(Self::find_let_expression)),
            ExpressionSyntaxKind::Call { callee, arguments } => Self::find_let_expression(callee)
                .or_else(|| arguments.iter().find_map(Self::find_let_expression)),
            ExpressionSyntaxKind::MethodCall {
                receiver,
                arguments,
                ..
            } => Self::find_let_expression(receiver)
                .or_else(|| arguments.iter().find_map(Self::find_let_expression)),
            ExpressionSyntaxKind::Field { base, .. }
            | ExpressionSyntaxKind::TupleField { base, .. } => Self::find_let_expression(base),
            ExpressionSyntaxKind::Index { base, index } => {
                Self::find_let_expression(base).or_else(|| Self::find_let_expression(index))
            }
            ExpressionSyntaxKind::Cast { expression, .. } => Self::find_let_expression(expression),
            ExpressionSyntaxKind::Return(value) => {
                value.as_deref().and_then(Self::find_let_expression)
            }
            ExpressionSyntaxKind::Break { value, .. } => {
                value.as_deref().and_then(Self::find_let_expression)
            }
            ExpressionSyntaxKind::Literal
            | ExpressionSyntaxKind::Path(_)
            | ExpressionSyntaxKind::Closure { .. }
            | ExpressionSyntaxKind::Block { .. }
            | ExpressionSyntaxKind::LabelBlock { .. }
            | ExpressionSyntaxKind::If { .. }
            | ExpressionSyntaxKind::Match { .. }
            | ExpressionSyntaxKind::Loop { .. }
            | ExpressionSyntaxKind::While { .. }
            | ExpressionSyntaxKind::For { .. }
            | ExpressionSyntaxKind::Continue(_)
            | ExpressionSyntaxKind::Underscore
            | ExpressionSyntaxKind::Error(_)
            | ExpressionSyntaxKind::Unsupported(_)
            | ExpressionSyntaxKind::Macro { .. } => None,
        }
    }
}
