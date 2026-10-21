use crate::{
    ast::{ExpressionFieldSyntax, ExpressionSyntax, ExpressionSyntaxKind, PathSyntax},
    parser::{
        path::PathParser,
        token_cursor::{SyntaxError, SyntaxErrorKind, TokenCursor},
    },
    SourceFile,
};

use super::pratt::PrattParser;

pub(super) struct AtomParser;

impl AtomParser {
    pub(super) fn parse_parenthesized(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        let (_, group) = PrattParser::take_group(cursor)?;
        let inner = group.start + 1..group.end - 1;
        if inner.is_empty() {
            return Ok(PrattParser::new_span(
                cursor,
                start,
                start_offset,
                ExpressionSyntaxKind::Tuple(Vec::new()),
            ));
        }
        let mut inner_cursor = TokenCursor::new(cursor.tokens(), inner.clone())
            .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedToken))?;
        let first = PrattParser::parse(source, &mut inner_cursor, 0)?;
        if inner_cursor.consume_punctuation(",") {
            let mut elements = vec![first];
            while !inner_cursor.is_empty() {
                elements.push(PrattParser::parse(source, &mut inner_cursor, 0)?);
                if !inner_cursor.consume_punctuation(",") && !inner_cursor.is_empty() {
                    return Err(PathParser::error_at(
                        &inner_cursor,
                        SyntaxErrorKind::UnexpectedToken,
                    ));
                }
            }
            Ok(PrattParser::new_span(
                cursor,
                start,
                start_offset,
                ExpressionSyntaxKind::Tuple(elements),
            ))
        } else if inner_cursor.is_empty() {
            Ok(PrattParser::new_span(
                cursor,
                start,
                start_offset,
                ExpressionSyntaxKind::Group(Box::new(first)),
            ))
        } else {
            Err(PathParser::error_at(
                &inner_cursor,
                SyntaxErrorKind::UnexpectedToken,
            ))
        }
    }

    pub(super) fn parse_array(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        let (_, group) = PrattParser::take_group(cursor)?;
        let inner = group.start + 1..group.end - 1;
        if inner.is_empty() {
            return Ok(PrattParser::new_span(
                cursor,
                start,
                start_offset,
                ExpressionSyntaxKind::Array(Vec::new()),
            ));
        }
        let mut inner_cursor = TokenCursor::new(cursor.tokens(), inner.clone())
            .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedToken))?;
        let first = PrattParser::parse(source, &mut inner_cursor, 0)?;
        if inner_cursor.consume_punctuation(";") {
            let count = PrattParser::parse(source, &mut inner_cursor, 0)?;
            if !inner_cursor.is_empty() {
                return Err(PathParser::error_at(
                    &inner_cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ));
            }
            Ok(PrattParser::new_span(
                cursor,
                start,
                start_offset,
                ExpressionSyntaxKind::Repeat {
                    value: Box::new(first),
                    count: Box::new(count),
                },
            ))
        } else {
            let mut elements = vec![first];
            while inner_cursor.consume_punctuation(",") {
                if inner_cursor.is_empty() {
                    break;
                }
                elements.push(PrattParser::parse(source, &mut inner_cursor, 0)?);
            }
            if !inner_cursor.is_empty() {
                return Err(PathParser::error_at(
                    &inner_cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ));
            }
            Ok(PrattParser::new_span(
                cursor,
                start,
                start_offset,
                ExpressionSyntaxKind::Array(elements),
            ))
        }
    }

    pub(super) fn parse_struct(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
        path: PathSyntax,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        let (_, group) = PrattParser::take_group(cursor)?;
        let mut fields = Vec::new();
        let mut field_cursor = TokenCursor::new(cursor.tokens(), group.start + 1..group.end - 1)
            .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedToken))?;
        while !field_cursor.is_empty() {
            let field_start = field_cursor.index();
            let attributes = PrattParser::parse_expression_attributes(&mut field_cursor)?;
            if field_cursor.consume_punctuation("..") {
                let value = PrattParser::parse(source, &mut field_cursor, 0)?;
                fields.push(ExpressionFieldSyntax::new(
                    None,
                    Some(value),
                    false,
                    attributes,
                    PrattParser::span(
                        cursor.tokens(),
                        field_start,
                        field_cursor.consumed_end_index(),
                    ),
                ));
            } else {
                let name_index = field_cursor.index();
                let name_token = field_cursor.peek().ok_or_else(|| {
                    PathParser::error_at(&field_cursor, SyntaxErrorKind::UnexpectedEof)
                })?;
                let Some(name) = name_token.kind().identifier() else {
                    return Err(PathParser::error_at(
                        &field_cursor,
                        SyntaxErrorKind::UnexpectedToken,
                    ));
                };
                let name = name.to_owned();
                let name_bounds = name_token.start()..name_token.end();
                field_cursor.consume();
                let (value, shorthand) = if field_cursor.consume_punctuation(":") {
                    (
                        Some(PrattParser::parse(source, &mut field_cursor, 0)?),
                        false,
                    )
                } else {
                    let path = PathParser::parse_expression(
                        &mut TokenCursor::new(cursor.tokens(), name_index..name_index + 1)
                            .expect("字段名 token 范围有效"),
                    )?;
                    (
                        Some(ExpressionSyntax::new(
                            ExpressionSyntaxKind::Path(path),
                            name_bounds,
                        )),
                        true,
                    )
                };
                fields.push(ExpressionFieldSyntax::new(
                    Some(name),
                    value,
                    shorthand,
                    attributes,
                    PrattParser::span(
                        cursor.tokens(),
                        field_start,
                        field_cursor.consumed_end_index(),
                    ),
                ));
            }
            if !field_cursor.consume_punctuation(",") && !field_cursor.is_empty() {
                return Err(PathParser::error_at(
                    &field_cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ));
            }
        }
        Ok(PrattParser::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::Struct { path, fields },
        ))
    }
}
