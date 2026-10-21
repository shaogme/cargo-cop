use std::ops::Range;

use crate::{
    ast::{GenericArgumentSyntax, PathSegmentSyntax, PathSyntax},
    Delimiter, Token, TokenKind,
};

use super::{
    token_cursor::{SyntaxError, SyntaxErrorKind, TokenCursor},
    types::TypeParser,
};

pub(super) struct PathParser;

impl PathParser {
    pub(super) fn parse_expression(
        cursor: &mut TokenCursor<'_>,
    ) -> Result<PathSyntax, SyntaxError> {
        let start = cursor.index();
        let start_offset = cursor.offset();
        if cursor.at_angle_open() {
            return Self::parse_qualified(cursor, start, start_offset);
        }
        let leading_colon = cursor.consume_punctuation("::");
        let mut segments = vec![Self::parse_expression_segment(cursor)?];
        loop {
            if !cursor.consume_punctuation("::") {
                break;
            }
            if cursor.consume_angle_open() {
                let arguments = Self::parse_arguments(cursor)?;
                Self::expect_angle_close(cursor)?;
                let previous = segments.pop().expect("路径至少包含一个段");
                segments.push(PathSegmentSyntax::new(
                    previous.name().to_owned(),
                    arguments,
                    previous.bounds().start..cursor.offset(),
                ));
                continue;
            }
            segments.push(Self::parse_expression_segment(cursor)?);
        }
        let bounds = Self::span(
            cursor.tokens(),
            start,
            cursor.consumed_end_index(),
            start_offset,
            cursor.offset(),
        );
        Ok(PathSyntax::new(leading_colon, None, None, segments, bounds))
    }

    pub(super) fn parse(cursor: &mut TokenCursor<'_>) -> Result<PathSyntax, SyntaxError> {
        let start = cursor.index();
        let start_offset = cursor.offset();
        if cursor.at_angle_open() {
            return Self::parse_qualified(cursor, start, start_offset);
        }
        let leading_colon = cursor.consume_punctuation("::");
        let mut segments = Vec::new();
        segments.push(Self::parse_segment(cursor)?);
        loop {
            if !cursor.consume_punctuation("::") {
                break;
            }
            if cursor.consume_angle_open() {
                let arguments = Self::parse_arguments(cursor)?;
                Self::expect_angle_close(cursor)?;
                let last = segments.pop().expect("路径至少包含一个段");
                segments.push(PathSegmentSyntax::new(
                    last.name().to_owned(),
                    arguments,
                    last.bounds().start..cursor.offset(),
                ));
                if !cursor.at_punctuation("::") {
                    break;
                }
                continue;
            }
            segments.push(Self::parse_segment(cursor)?);
        }
        let bounds = Self::span(
            cursor.tokens(),
            start,
            cursor.consumed_end_index(),
            start_offset,
            cursor.offset(),
        );
        Ok(PathSyntax::new(leading_colon, None, None, segments, bounds))
    }

    fn parse_qualified(
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
    ) -> Result<PathSyntax, SyntaxError> {
        cursor.consume_angle_open();
        let self_type = TypeParser::parse(cursor)?;
        let trait_path = if cursor.consume_identifier("as") {
            Some(Box::new(Self::parse(cursor)?))
        } else {
            None
        };
        Self::expect_angle_close(cursor)?;
        if !cursor.consume_punctuation("::") {
            return Err(Self::error_at(cursor, SyntaxErrorKind::UnexpectedToken));
        }
        let mut segments = vec![Self::parse_segment(cursor)?];
        while cursor.consume_punctuation("::") {
            segments.push(Self::parse_segment(cursor)?);
        }
        let bounds = Self::span(
            cursor.tokens(),
            start,
            cursor.consumed_end_index(),
            start_offset,
            cursor.offset(),
        );
        Ok(PathSyntax::new(
            false,
            Some(Box::new(self_type)),
            trait_path,
            segments,
            bounds,
        ))
    }

    fn parse_segment(cursor: &mut TokenCursor<'_>) -> Result<PathSegmentSyntax, SyntaxError> {
        let start = cursor.index();
        let start_offset = cursor.offset();
        let token = cursor
            .peek()
            .ok_or_else(|| Self::error_at(cursor, SyntaxErrorKind::UnexpectedEof))?;
        let Some(name) = token.kind().identifier() else {
            return Err(Self::error_at(cursor, SyntaxErrorKind::UnexpectedToken));
        };
        let name = name.to_owned();
        cursor.consume();
        let arguments = if cursor.consume_angle_open() {
            let arguments = Self::parse_arguments(cursor)?;
            Self::expect_angle_close(cursor)?;
            arguments
        } else {
            Vec::new()
        };
        let bounds = Self::span(
            cursor.tokens(),
            start,
            cursor.consumed_end_index(),
            start_offset,
            cursor.offset(),
        );
        Ok(PathSegmentSyntax::new(name, arguments, bounds))
    }

    fn parse_expression_segment(
        cursor: &mut TokenCursor<'_>,
    ) -> Result<PathSegmentSyntax, SyntaxError> {
        let start = cursor.index();
        let start_offset = cursor.offset();
        let token = cursor
            .peek()
            .ok_or_else(|| Self::error_at(cursor, SyntaxErrorKind::UnexpectedEof))?;
        let Some(name) = token.kind().identifier() else {
            return Err(Self::error_at(cursor, SyntaxErrorKind::UnexpectedToken));
        };
        let name = name.to_owned();
        cursor.consume();
        Ok(PathSegmentSyntax::new(
            name,
            Vec::new(),
            Self::span(
                cursor.tokens(),
                start,
                cursor.consumed_end_index(),
                start_offset,
                cursor.offset(),
            ),
        ))
    }

    fn parse_arguments(
        cursor: &mut TokenCursor<'_>,
    ) -> Result<Vec<GenericArgumentSyntax>, SyntaxError> {
        let mut arguments = Vec::new();
        while !cursor.is_empty() && !cursor.at_punctuation(">") && !cursor.at_punctuation(">>") {
            let checkpoint = cursor.index();
            let argument = Self::parse_argument(cursor)?;
            if cursor.index() == checkpoint && !cursor.is_empty() {
                return Err(Self::error_at(cursor, SyntaxErrorKind::UnexpectedToken));
            }
            arguments.push(argument);
            if cursor.consume_punctuation(",") {
                continue;
            }
            if !cursor.at_punctuation(">") && !cursor.at_punctuation(">>") {
                return Err(Self::error_at(cursor, SyntaxErrorKind::UnexpectedToken));
            }
        }
        Ok(arguments)
    }

    fn parse_argument(cursor: &mut TokenCursor<'_>) -> Result<GenericArgumentSyntax, SyntaxError> {
        if let Some(TokenKind::Lifetime(name)) = cursor.peek().map(Token::kind) {
            let name = name.to_owned();
            cursor.consume();
            return Ok(GenericArgumentSyntax::Lifetime(name));
        }
        if cursor.at_open_delimiter(Delimiter::Brace) {
            let start_offset = cursor.offset();
            let group = cursor
                .skip_delimited_group()
                .ok_or_else(|| Self::error_at(cursor, SyntaxErrorKind::UnclosedDelimiter))?;
            return Ok(GenericArgumentSyntax::Const(Self::span(
                cursor.tokens(),
                group.start,
                group.end,
                start_offset,
                cursor.tokens()[group.end - 1].end(),
            )));
        }
        if matches!(cursor.peek().map(Token::kind), Some(TokenKind::Literal(_)))
            || cursor.at_punctuation("-")
        {
            return Self::parse_const_argument(cursor);
        }
        if let Some(name) = cursor.peek().and_then(|token| token.kind().identifier()) {
            let name = name.to_owned();
            let checkpoint = cursor.checkpoint();
            cursor.consume();
            if cursor.consume_punctuation("=") {
                if matches!(cursor.peek().map(Token::kind), Some(TokenKind::Literal(_)))
                    || cursor.at_open_delimiter(Delimiter::Brace)
                    || cursor.at_punctuation("-")
                {
                    let value = Self::const_range(cursor)?;
                    return Ok(GenericArgumentSyntax::AssociatedConst { name, value });
                }
                let value = TypeParser::parse(cursor)?;
                return Ok(GenericArgumentSyntax::AssociatedType {
                    name,
                    bounds: Vec::new(),
                    value,
                });
            }
            if cursor.consume_punctuation(":") {
                let bounds = TypeParser::parse_bounds(cursor)?;
                if cursor.consume_punctuation("=") {
                    let value = TypeParser::parse(cursor)?;
                    return Ok(GenericArgumentSyntax::AssociatedType {
                        name,
                        bounds,
                        value,
                    });
                }
                cursor.restore(checkpoint);
            } else {
                cursor.restore(checkpoint);
            }
        }
        Ok(GenericArgumentSyntax::Type(TypeParser::parse(cursor)?))
    }

    fn parse_const_argument(
        cursor: &mut TokenCursor<'_>,
    ) -> Result<GenericArgumentSyntax, SyntaxError> {
        Self::const_range(cursor).map(GenericArgumentSyntax::Const)
    }

    fn const_range(cursor: &mut TokenCursor<'_>) -> Result<Range<usize>, SyntaxError> {
        let start = cursor.index();
        let start_offset = cursor.offset();
        while !cursor.is_empty() && !cursor.at_punctuation(",") {
            if cursor.at_punctuation(">") || cursor.at_punctuation(">>") {
                break;
            }
            if matches!(
                cursor.peek().map(Token::kind),
                Some(TokenKind::OpenDelimiter(_))
            ) {
                if cursor.skip_delimited_group().is_none() {
                    return Err(Self::error_at(cursor, SyntaxErrorKind::UnclosedDelimiter));
                }
                continue;
            }
            cursor.consume();
        }
        if cursor.index() == start {
            return Err(Self::error_at(cursor, SyntaxErrorKind::UnexpectedToken));
        }
        Ok(Self::span(
            cursor.tokens(),
            start,
            cursor.consumed_end_index(),
            start_offset,
            cursor.offset(),
        ))
    }

    fn expect_angle_close(cursor: &mut TokenCursor<'_>) -> Result<(), SyntaxError> {
        if cursor.consume_angle_close() {
            Ok(())
        } else {
            Err(Self::error_at(
                cursor,
                if cursor.is_empty() {
                    SyntaxErrorKind::UnexpectedEof
                } else {
                    SyntaxErrorKind::UnexpectedToken
                },
            ))
        }
    }

    pub(super) fn error_at(cursor: &TokenCursor<'_>, kind: SyntaxErrorKind) -> SyntaxError {
        let token = cursor.peek();
        SyntaxError::new(
            kind,
            token.map_or(cursor.offset()..cursor.offset(), |token| {
                token.start()..token.end()
            }),
        )
    }

    pub(super) fn span(
        tokens: &[Token],
        start: usize,
        end: usize,
        start_offset: usize,
        end_offset: usize,
    ) -> Range<usize> {
        let start = tokens
            .get(start)
            .map_or(start_offset, |token| token.start().max(start_offset));
        let end = end
            .checked_sub(1)
            .and_then(|index| tokens.get(index))
            .map_or(end_offset, |token| end_offset.min(token.end()));
        start..end.max(start)
    }
}
