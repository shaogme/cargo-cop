use crate::{
    ast::{AstNodeId, ParseIssue, SyntaxNode},
    recovery::Recovery,
    RustEdition, SourceFile, Token,
};

use super::super::expression::{PatternInput, SyntaxCollector, SyntaxRequest};
use crate::parser::{
    token_cursor::TokenCursor,
    types::{TypeInput, TypeParser},
};

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

pub(super) struct PatternCollector;

impl PatternCollector {
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
            "let" => (
                start + 1,
                SyntaxCollector::top_level_token(tokens, start + 1, end, ":")
                    .into_iter()
                    .chain(SyntaxCollector::top_level_token(
                        tokens,
                        start + 1,
                        end,
                        "=",
                    ))
                    .min(),
            ),
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

    pub(super) fn collect_let_type(input: BlockPatternInput<'_>) {
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
        let has_let_pattern = first == "let"
            || (matches!(first, "if" | "while")
                && (start + 1..end).any(|index| Recovery::is_identifier(&tokens[index], "let")));
        if !has_let_pattern {
            return;
        }
        let Some(statement_cursor) = TokenCursor::new(tokens, start + 1..end) else {
            return;
        };
        let equals = statement_cursor
            .find_top_level_punctuation("=")
            .unwrap_or(end);
        let Some(pattern_cursor) = TokenCursor::new(tokens, start + 1..equals) else {
            return;
        };
        let Some(colon) = pattern_cursor.find_top_level_punctuation(":") else {
            return;
        };
        let type_end = pattern_cursor
            .find_top_level_punctuation(";")
            .unwrap_or(equals)
            .min(equals);
        if colon + 1 < type_end {
            TypeParser::collect(TypeInput {
                tokens,
                bounds: colon + 1..type_end,
                parent,
                ast_nodes,
                issues,
            });
            SyntaxCollector::collect(
                SyntaxRequest {
                    source,
                    tokens,
                    bounds: colon + 1..type_end,
                    parent,
                    expression_tree: false,
                    edition: None,
                },
                ast_nodes,
                issues,
            );
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
