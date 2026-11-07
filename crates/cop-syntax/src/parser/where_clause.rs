use cop_i18n::t;
use std::ops::Range;

use crate::{
    ast::{
        AstFactory, AstNodeId, ParseIssue, ParseIssueKind, SyntaxNode, SyntaxNodeData,
        SyntaxNodeKind, WhereClauseSyntax, WherePredicateKind, WherePredicateSyntax,
    },
    Token, TokenKind,
};

use super::{
    token_cursor::{AngleScanContext, CursorRange, SyntaxError, SyntaxErrorKind, TokenCursor},
    types::TypeParser,
};

pub(super) struct WhereClauseParser;

pub(super) struct WhereClauseInput<'a> {
    pub(super) tokens: &'a [Token],
    pub(super) bounds: Range<usize>,
    pub(super) parent: AstNodeId,
    pub(super) ast_nodes: &'a mut Vec<SyntaxNode>,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

struct PredicateError;

impl PredicateError {
    fn at(cursor: &TokenCursor<'_>, kind: SyntaxErrorKind) -> SyntaxError {
        let token = cursor.peek();
        SyntaxError::new(
            kind,
            token.map_or(cursor.offset()..cursor.offset(), |token| {
                token.start()..token.end()
            }),
        )
    }
}

impl WhereClauseParser {
    pub(super) fn collect(input: WhereClauseInput<'_>) -> Option<AstNodeId> {
        let WhereClauseInput {
            tokens,
            bounds,
            parent,
            ast_nodes,
            issues,
        } = input;
        let cursor = TokenCursor::from_token_range(tokens, bounds.clone())?;
        let where_index = cursor.find_top_level_identifier("where", AngleScanContext::Header)?;
        let clause_bounds =
            CursorRange::from_token_range(tokens, where_index..bounds.end)?.byte_bounds();
        let clause_id = AstFactory::push_data_node(
            ast_nodes,
            parent,
            SyntaxNodeKind::WhereClause,
            clause_bounds.clone(),
            SyntaxNodeData::WhereClause(WhereClauseSyntax::new(clause_bounds)),
        );
        let predicate_start = where_index + 1;
        let mut predicate_cursor =
            TokenCursor::from_token_range(tokens, predicate_start..bounds.end)?;
        let predicates = match predicate_cursor.split_top_level(",", AngleScanContext::Header) {
            Ok(predicates) => predicates,
            Err(error) => {
                Self::push_issue(issues, ParseIssueKind::UnexpectedToken, error.bounds());
                Vec::new()
            }
        };
        let mut parsed_any = !predicates.is_empty();
        for range in predicates {
            Self::collect_predicate(tokens, range, clause_id, ast_nodes, issues);
        }
        if predicate_start == bounds.end {
            parsed_any = false;
        }
        if !parsed_any {
            let where_token = &tokens[where_index];
            Self::push_issue(
                issues,
                ParseIssueKind::UnexpectedToken,
                where_token.start()..where_token.end(),
            );
        }
        Some(clause_id)
    }

    fn collect_predicate(
        tokens: &[Token],
        range: CursorRange,
        parent: AstNodeId,
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) {
        let bounds = range.byte_bounds();
        match Self::parse_predicate(tokens, range) {
            Ok(syntax) => {
                AstFactory::push_data_node(
                    ast_nodes,
                    parent,
                    SyntaxNodeKind::WherePredicate,
                    bounds,
                    SyntaxNodeData::WherePredicate(syntax),
                );
            }
            Err(error) => {
                let error_bounds = error.bounds();
                let kind = if error.kind() == SyntaxErrorKind::UnclosedDelimiter {
                    ParseIssueKind::UnclosedDelimiter
                } else {
                    ParseIssueKind::UnexpectedToken
                };
                if bounds.start < bounds.end {
                    AstFactory::push_ast_node(
                        ast_nodes,
                        Some(parent),
                        SyntaxNodeKind::Opaque,
                        bounds,
                        Vec::new(),
                    );
                }
                Self::push_issue(issues, kind, error_bounds);
            }
        }
    }

    fn parse_predicate(
        tokens: &[Token],
        range: CursorRange,
    ) -> Result<WherePredicateSyntax, SyntaxError> {
        let mut cursor = TokenCursor::new(tokens, range)
            .ok_or_else(|| SyntaxError::new(SyntaxErrorKind::UnexpectedToken, 0..0))?;
        if let Some(TokenKind::Lifetime(lifetime)) = cursor.peek().map(Token::kind) {
            if cursor.peek_n(1).is_some_and(
                |token| matches!(token.kind(), TokenKind::Punctuation(value) if value == ":"),
            ) {
                let lifetime = lifetime.clone();
                cursor.consume();
                cursor.consume_punctuation(":");
                let mut bounds = Vec::new();
                loop {
                    match cursor.peek().map(Token::kind) {
                        Some(TokenKind::Lifetime(bound)) => {
                            bounds.push(bound.clone());
                            cursor.consume();
                        }
                        _ => {
                            return Err(PredicateError::at(
                                &cursor,
                                SyntaxErrorKind::UnexpectedToken,
                            ))
                        }
                    }
                    if !cursor.consume_punctuation("+") {
                        break;
                    }
                }
                if !cursor.is_empty() {
                    return Err(PredicateError::at(
                        &cursor,
                        SyntaxErrorKind::UnexpectedToken,
                    ));
                }
                return Ok(WherePredicateSyntax::new(
                    WherePredicateKind::Lifetime { lifetime, bounds },
                    range.byte_bounds(),
                ));
            }
        }
        let lifetimes = Self::parse_lifetime_binders(&mut cursor)?;
        let type_start = cursor.index();
        let colon = cursor
            .find_top_level_punctuation(":", AngleScanContext::Type)
            .ok_or_else(|| PredicateError::at(&cursor, SyntaxErrorKind::UnexpectedToken))?;
        if colon == type_start {
            return Err(PredicateError::at(
                &cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        let bounded_type = TypeParser::parse_range(
            tokens,
            CursorRange::from_token_range(tokens, type_start..colon)
                .expect("where bounded type 范围有效"),
        )?;
        let mut bounds_cursor =
            TokenCursor::from_token_range(tokens, colon + 1..range.end().token_index())
                .ok_or_else(|| SyntaxError::new(SyntaxErrorKind::UnexpectedToken, 0..0))?;
        let bounds = TypeParser::parse_bounds(&mut bounds_cursor)?;
        if !bounds_cursor.is_empty() {
            return Err(PredicateError::at(
                &bounds_cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        if bounds.is_empty() {
            return Err(PredicateError::at(
                &bounds_cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        Ok(WherePredicateSyntax::new(
            WherePredicateKind::Type {
                lifetimes,
                bounded_type,
                bounds,
            },
            range.byte_bounds(),
        ))
    }

    fn parse_lifetime_binders(cursor: &mut TokenCursor<'_>) -> Result<Vec<String>, SyntaxError> {
        if !cursor.consume_identifier("for") {
            return Ok(Vec::new());
        }
        if !cursor.consume_angle_open() {
            return Err(PredicateError::at(cursor, SyntaxErrorKind::UnexpectedToken));
        }
        let mut lifetimes = Vec::new();
        loop {
            match cursor.peek().map(Token::kind) {
                Some(TokenKind::Lifetime(name)) => {
                    lifetimes.push(name.clone());
                    cursor.consume();
                }
                _ => return Err(PredicateError::at(cursor, SyntaxErrorKind::UnexpectedToken)),
            }
            if !cursor.consume_punctuation(",") {
                break;
            }
        }
        if !cursor.consume_angle_close() {
            return Err(PredicateError::at(cursor, SyntaxErrorKind::UnexpectedToken));
        }
        Ok(lifetimes)
    }

    fn push_issue(issues: &mut Vec<ParseIssue>, kind: ParseIssueKind, bounds: Range<usize>) {
        issues.push(AstFactory::parse_issue(
            kind,
            bounds,
            t!("syntax.item_unsupported"),
        ));
    }
}
