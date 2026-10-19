use std::ops::Range;

use crate::{
    ast::{ExpressionSyntax, ExpressionSyntaxKind},
    RustEdition,
};

pub(super) struct EditionParser;

enum ConditionLetStatus {
    Absent,
    Valid(Range<usize>),
    Invalid(Range<usize>),
}

impl EditionParser {
    pub(super) fn invalid_let_condition(
        condition: &ExpressionSyntax,
        edition: RustEdition,
    ) -> Option<Range<usize>> {
        let is_single_let = matches!(condition.kind(), ExpressionSyntaxKind::Let { .. });
        if is_single_let {
            return None;
        }
        match Self::condition_let_status(condition) {
            ConditionLetStatus::Absent => None,
            ConditionLetStatus::Valid(bounds) => (edition != RustEdition::E2024).then_some(bounds),
            ConditionLetStatus::Invalid(bounds) => Some(bounds),
        }
    }

    fn condition_let_status(expression: &ExpressionSyntax) -> ConditionLetStatus {
        match expression.kind() {
            ExpressionSyntaxKind::Let { .. } => ConditionLetStatus::Valid(expression.bounds()),
            ExpressionSyntaxKind::Group(inner) => Self::condition_let_status(inner),
            ExpressionSyntaxKind::Binary {
                operator,
                left,
                right,
            } => {
                let left_status = Self::condition_let_status(left);
                let right_status = Self::condition_let_status(right);
                Self::combine_condition_let_status(operator, left_status, right_status)
            }
            _ => Self::find_let_expression(expression)
                .map_or(ConditionLetStatus::Absent, ConditionLetStatus::Invalid),
        }
    }

    fn combine_condition_let_status(
        operator: &str,
        left: ConditionLetStatus,
        right: ConditionLetStatus,
    ) -> ConditionLetStatus {
        match (left, right) {
            (ConditionLetStatus::Absent, ConditionLetStatus::Absent) => ConditionLetStatus::Absent,
            (left, right) => {
                let bounds =
                    Self::first_let_bounds(&left, &right).expect("包含 let 的状态必须携带源码范围");
                if operator == "&&"
                    && !matches!(left, ConditionLetStatus::Invalid(_))
                    && !matches!(right, ConditionLetStatus::Invalid(_))
                {
                    ConditionLetStatus::Valid(bounds)
                } else {
                    ConditionLetStatus::Invalid(bounds)
                }
            }
        }
    }

    fn first_let_bounds(
        left: &ConditionLetStatus,
        right: &ConditionLetStatus,
    ) -> Option<Range<usize>> {
        match left {
            ConditionLetStatus::Valid(bounds) | ConditionLetStatus::Invalid(bounds) => {
                Some(bounds.clone())
            }
            ConditionLetStatus::Absent => match right {
                ConditionLetStatus::Valid(bounds) | ConditionLetStatus::Invalid(bounds) => {
                    Some(bounds.clone())
                }
                ConditionLetStatus::Absent => None,
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
