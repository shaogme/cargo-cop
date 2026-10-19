use std::ops::Range;

use crate::{
    ast::{AstNodeId, ParseIssue, SyntaxNode},
    parser::types::{TypeInput, TypeParser},
    recovery::Recovery,
    RustEdition, SourceFile, Token,
};

use super::super::expression::{PatternInput, SyntaxCollector, SyntaxRequest};

pub(super) struct BlockPatternInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) edition: RustEdition,
    pub(super) parent: AstNodeId,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) ast_nodes: &'a mut Vec<SyntaxNode>,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

pub(super) struct LetPatternInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) parent: AstNodeId,
    pub(super) pattern: Range<usize>,
    pub(super) type_annotation: Option<Range<usize>>,
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
            TypeParser::collect(TypeInput {
                tokens,
                bounds: bounds.clone(),
                parent,
                ast_nodes: &mut *ast_nodes,
                issues: &mut *issues,
            });
            SyntaxCollector::collect(
                SyntaxRequest {
                    source,
                    tokens,
                    bounds,
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
            start,
            end,
            ast_nodes,
            issues,
        } = input;
        let Some(first) = tokens.get(start).map(|token| token.text(source)) else {
            return;
        };
        let (pattern_start_index, pattern_end_index) = match first {
            "if" | "while" => {
                let Some(let_index) =
                    (start + 1..end).find(|index| Recovery::is_identifier(&tokens[*index], "let"))
                else {
                    return;
                };
                (
                    let_index + 1,
                    SyntaxCollector::top_level_token(tokens, let_index + 1, end, "="),
                )
            }
            "for" => (
                start + 1,
                (start + 1..end).find(|index| Recovery::is_identifier(&tokens[*index], "in")),
            ),
            _ => return,
        };
        let Some(pattern_end_index) = pattern_end_index else {
            return;
        };
        let Some(pattern_start) = tokens.get(pattern_start_index).map(Token::start) else {
            return;
        };
        let pattern_end = tokens[pattern_end_index].start();
        let _pattern_id = SyntaxCollector::push_pattern(
            PatternInput {
                source,
                tokens,
                parent,
                start: pattern_start,
                end: pattern_end,
                bind_bare_identifiers: true,
                issues,
            },
            ast_nodes,
        );
    }

    pub(super) fn collect_match_pattern(input: BlockPatternInput<'_>) {
        let BlockPatternInput {
            source,
            tokens,
            edition,
            parent,
            start,
            end,
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
