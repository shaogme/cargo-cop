use crate::{
    ast::{ClosureParameterSyntax, ExpressionBlockKind, ExpressionSyntax, ExpressionSyntaxKind},
    parser::{
        path::PathParser,
        patterns::PatternParser,
        token_cursor::{SyntaxError, SyntaxErrorKind, TokenCursor},
        types::TypeParser,
    },
    recovery::Recovery,
    Delimiter, SourceFile, Token, TokenKind,
};

use super::{control::ControlParser, pratt::PrattParser};

pub(super) struct ClosurePrefix {
    pub(super) start: usize,
    pub(super) start_offset: usize,
    pub(super) is_async: bool,
    pub(super) is_move: bool,
}

pub(super) struct ClosureParser;

impl ClosureParser {
    pub(super) fn parse(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        prefix: ClosurePrefix,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        let ClosurePrefix {
            start,
            start_offset,
            is_async,
            is_move,
        } = prefix;
        let mut parameters = Vec::new();
        if cursor.consume_punctuation("||") {
        } else {
            if !cursor.consume_punctuation("|") {
                return Err(PathParser::error_at(
                    cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ));
            }
            let close = Self::find_closure_pipe(cursor)
                .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedEof))?;
            let parameter_bounds = cursor.index()..close;
            for range in Recovery::split_top_level(
                cursor.tokens(),
                parameter_bounds.start,
                parameter_bounds.end,
                ",",
            ) {
                if range.is_empty() {
                    continue;
                }
                let mut parameter_cursor = TokenCursor::new(cursor.tokens(), range.clone())
                    .ok_or_else(|| {
                        PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedToken)
                    })?;
                let attributes = PrattParser::parse_expression_attributes(&mut parameter_cursor)?;
                let pattern_start = parameter_cursor.index();
                let colon = PrattParser::find_top_level_token(
                    cursor.tokens(),
                    pattern_start..range.end,
                    ":",
                );
                let pattern_end = colon.unwrap_or(range.end);
                let pattern = PatternParser::parse_binding_range(
                    cursor.tokens(),
                    pattern_start..pattern_end,
                )?;
                let ty = if let Some(colon) = colon {
                    let mut type_cursor = TokenCursor::new(cursor.tokens(), colon + 1..range.end)
                        .ok_or_else(|| {
                        PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedToken)
                    })?;
                    let ty = TypeParser::parse(&mut type_cursor)?;
                    if !type_cursor.is_empty() {
                        return Err(PathParser::error_at(
                            &type_cursor,
                            SyntaxErrorKind::UnexpectedToken,
                        ));
                    }
                    Some(ty)
                } else {
                    None
                };
                parameters.push(ClosureParameterSyntax::new(
                    pattern,
                    ty,
                    attributes,
                    PrattParser::span(cursor.tokens(), range.start, range.end),
                ));
            }
            while cursor.index() < close {
                cursor.consume();
            }
            cursor.consume_punctuation("|");
        }
        let return_type = if cursor.consume_punctuation("->") {
            let ty = TypeParser::parse(cursor)?;
            if !cursor.at_open_delimiter(Delimiter::Brace) {
                return Err(PathParser::error_at(
                    cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ));
            }
            Some(ty)
        } else {
            None
        };
        let body = PrattParser::parse(source, cursor, 0)?;
        let bounds = PrattParser::span(cursor.tokens(), start, cursor.index());
        Ok(ExpressionSyntax::new(
            ExpressionSyntaxKind::Closure {
                is_async,
                is_move,
                parameters,
                return_type,
                body: Box::new(body.clone()),
            },
            bounds.start.min(start_offset)..body.bounds().end,
        ))
    }

    pub(super) fn parse_async(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        cursor.consume_identifier("async");
        let is_move = cursor.consume_identifier("move");
        if Self::next_is_closure(cursor) {
            return Self::parse(
                source,
                cursor,
                ClosurePrefix {
                    start,
                    start_offset,
                    is_async: true,
                    is_move,
                },
            );
        }
        if cursor.at_open_delimiter(Delimiter::Brace) {
            return ControlParser::parse_block(
                cursor,
                start,
                start_offset,
                ExpressionBlockKind::Async,
            );
        }
        Err(PathParser::error_at(
            cursor,
            SyntaxErrorKind::UnexpectedToken,
        ))
    }

    pub(super) fn is_closure_start(cursor: &TokenCursor<'_>) -> bool {
        cursor
            .peek_n(1)
            .is_some_and(|token| matches!(token.kind(), TokenKind::Punctuation(value) if value == "|" || value == "||"))
    }

    fn find_closure_pipe(cursor: &TokenCursor<'_>) -> Option<usize> {
        let mut scan = TokenCursor::new(cursor.tokens(), cursor.index()..cursor.end())?;
        while !scan.is_empty() {
            if scan.at_punctuation("|") {
                return Some(scan.index());
            }
            if matches!(
                scan.peek().map(Token::kind),
                Some(TokenKind::OpenDelimiter(_))
            ) {
                scan.skip_delimited_group()?;
            } else {
                scan.consume();
            }
        }
        None
    }

    fn next_is_closure(cursor: &TokenCursor<'_>) -> bool {
        cursor
            .peek_n(1)
            .is_some_and(|token| matches!(token.kind(), TokenKind::Punctuation(value) if value == "|" || value == "||"))
    }
}
