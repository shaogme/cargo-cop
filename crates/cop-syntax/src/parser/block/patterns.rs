use std::ops::Range;

use crate::{
    ast::{
        AstNodeId, ExpressionSyntax, ExpressionSyntaxKind, ParseIssue, PatternSyntax, SyntaxNode,
    },
    parser::types::{ParsedTypeInput, TypeParser},
    recovery::Recovery,
    RustEdition, SourceFile, Token,
};

use super::{
    super::expression::{PatternInput, SyntaxCollector, SyntaxRequest},
    statement::ParsedTypeAnnotation,
};

pub(super) struct BlockPatternInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) edition: RustEdition,
    pub(super) parent: AstNodeId,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) expression: Option<&'a ExpressionSyntax>,
    pub(super) ast_nodes: &'a mut Vec<SyntaxNode>,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

pub(super) struct LetPatternInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) parent: AstNodeId,
    pub(super) pattern: Range<usize>,
    pub(super) type_annotation: Option<ParsedTypeAnnotation>,
    pub(super) ast_nodes: &'a mut Vec<SyntaxNode>,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

pub(super) struct PatternCollector;

impl PatternCollector {
    pub(super) fn collect_let(input: LetPatternInput<'_>) {
        let LetPatternInput {
            source,
            tokens,
            parent,
            pattern,
            type_annotation,
            ast_nodes,
            issues,
        } = input;
        let pattern_bounds = Recovery::token_bounds(tokens, pattern);
        SyntaxCollector::push_pattern(
            PatternInput {
                source,
                tokens,
                parent,
                start: pattern_bounds.start,
                end: pattern_bounds.end,
                bind_bare_identifiers: true,
                issues: &mut *issues,
            },
            &mut *ast_nodes,
        );
        if let Some(bounds) = type_annotation {
            TypeParser::collect_parsed(ParsedTypeInput {
                syntax: bounds.syntax,
                tokens,
                parent,
                ast_nodes: &mut *ast_nodes,
                issues: &mut *issues,
            });
            SyntaxCollector::collect(
                SyntaxRequest {
                    source,
                    tokens,
                    bounds: bounds.token_bounds,
                    parent,
                    expression_tree: false,
                    edition: None,
                },
                &mut *ast_nodes,
                &mut *issues,
            );
        }
    }

    pub(super) fn collect_statement_pattern(input: BlockPatternInput<'_>) {
        let BlockPatternInput {
            source,
            tokens,
            edition: _,
            parent,
            expression,
            ast_nodes,
            issues,
            ..
        } = input;
        let Some(expression) = expression else {
            return;
        };
        let pattern = match expression.kind() {
            ExpressionSyntaxKind::If { condition, .. }
            | ExpressionSyntaxKind::While { condition, .. } => Self::condition_pattern(condition),
            ExpressionSyntaxKind::For { pattern, .. } => Some(pattern),
            _ => None,
        };
        let Some(pattern) = pattern else {
            return;
        };
        let bounds = pattern.bounds();
        let _pattern_id = SyntaxCollector::push_pattern(
            PatternInput {
                source,
                tokens,
                parent,
                start: bounds.start,
                end: bounds.end,
                bind_bare_identifiers: true,
                issues,
            },
            ast_nodes,
        );
    }

    fn condition_pattern(expression: &ExpressionSyntax) -> Option<&PatternSyntax> {
        match expression.kind() {
            ExpressionSyntaxKind::Let { pattern, .. } => Some(pattern),
            ExpressionSyntaxKind::Group(inner)
            | ExpressionSyntaxKind::Unary { operand: inner, .. }
            | ExpressionSyntaxKind::Await(inner)
            | ExpressionSyntaxKind::Try(inner) => Self::condition_pattern(inner),
            ExpressionSyntaxKind::Binary { left, right, .. }
            | ExpressionSyntaxKind::Assignment { left, right, .. } => {
                Self::condition_pattern(left).or_else(|| Self::condition_pattern(right))
            }
            ExpressionSyntaxKind::Tuple(values) | ExpressionSyntaxKind::Array(values) => {
                values.iter().find_map(Self::condition_pattern)
            }
            ExpressionSyntaxKind::Repeat { value, count } => {
                Self::condition_pattern(value).or_else(|| Self::condition_pattern(count))
            }
            ExpressionSyntaxKind::Struct { fields, .. } => fields
                .iter()
                .filter_map(|field| field.value())
                .find_map(Self::condition_pattern),
            ExpressionSyntaxKind::Range { start, end, .. } => start
                .as_deref()
                .and_then(Self::condition_pattern)
                .or_else(|| end.as_deref().and_then(Self::condition_pattern)),
            ExpressionSyntaxKind::Call { callee, arguments } => Self::condition_pattern(callee)
                .or_else(|| arguments.iter().find_map(Self::condition_pattern)),
            ExpressionSyntaxKind::MethodCall {
                receiver,
                arguments,
                ..
            } => Self::condition_pattern(receiver)
                .or_else(|| arguments.iter().find_map(Self::condition_pattern)),
            ExpressionSyntaxKind::Field { base, .. }
            | ExpressionSyntaxKind::TupleField { base, .. } => Self::condition_pattern(base),
            ExpressionSyntaxKind::Index { base, index } => {
                Self::condition_pattern(base).or_else(|| Self::condition_pattern(index))
            }
            ExpressionSyntaxKind::Cast { expression, .. } => Self::condition_pattern(expression),
            ExpressionSyntaxKind::Return(value) => {
                value.as_deref().and_then(Self::condition_pattern)
            }
            ExpressionSyntaxKind::Break { value, .. } => {
                value.as_deref().and_then(Self::condition_pattern)
            }
            ExpressionSyntaxKind::Literal
            | ExpressionSyntaxKind::Path(_)
            | ExpressionSyntaxKind::Closure { .. }
            | ExpressionSyntaxKind::Block { .. }
            | ExpressionSyntaxKind::LabelBlock { .. }
            | ExpressionSyntaxKind::If { .. }
            | ExpressionSyntaxKind::While { .. }
            | ExpressionSyntaxKind::Match { .. }
            | ExpressionSyntaxKind::Loop { .. }
            | ExpressionSyntaxKind::For { .. }
            | ExpressionSyntaxKind::Continue(_)
            | ExpressionSyntaxKind::Underscore
            | ExpressionSyntaxKind::Error(_)
            | ExpressionSyntaxKind::Unsupported(_)
            | ExpressionSyntaxKind::Macro { .. } => None,
        }
    }

    pub(super) fn collect_match_pattern(input: BlockPatternInput<'_>) {
        let BlockPatternInput {
            source,
            tokens,
            edition,
            parent,
            start,
            end,
            expression: _,
            ast_nodes,
            issues,
        } = input;
        let Some(arrow) = SyntaxCollector::top_level_token(tokens, start, end, "=>") else {
            return;
        };
        let guard = SyntaxCollector::top_level_token(tokens, start, arrow, "if");
        let pattern_end = guard.unwrap_or(arrow);
        let Some(pattern_id) = SyntaxCollector::push_pattern(
            PatternInput {
                source,
                tokens,
                parent,
                start: tokens[start].start(),
                end: tokens[pattern_end].start(),
                bind_bare_identifiers: false,
                issues,
            },
            ast_nodes,
        ) else {
            return;
        };
        SyntaxCollector::collect(
            SyntaxRequest {
                source,
                tokens,
                bounds: start..pattern_end,
                parent: pattern_id,
                expression_tree: false,
                edition: None,
            },
            ast_nodes,
            issues,
        );
        if let Some(guard) = guard {
            SyntaxCollector::collect(
                SyntaxRequest {
                    source,
                    tokens,
                    bounds: guard + 1..arrow,
                    parent,
                    expression_tree: true,
                    edition: Some(edition),
                },
                ast_nodes,
                issues,
            );
        }
    }
}
