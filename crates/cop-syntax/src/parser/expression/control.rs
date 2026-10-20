use std::ops::Range;

use crate::{
    ast::{
        ExpressionBlockKind, ExpressionErrorKind, ExpressionMatchArmSyntax, ExpressionSyntax,
        ExpressionSyntaxKind, ExpressionUnsupportedKind,
    },
    parser::{
        path::PathParser,
        patterns::{PatternParseContext, PatternParser},
        token_cursor::{SyntaxError, SyntaxErrorKind, TokenCursor},
    },
    Delimiter, SourceFile, Token, TokenKind,
};

use super::pratt::PrattParser;

pub(super) struct ControlParser;

impl ControlParser {
    pub(super) fn parse_if(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        cursor.consume_identifier("if");
        let condition = PrattParser::parse_control_header(source, cursor)?;
        let then_block = Self::consume_block_bounds(cursor)?;
        let else_expression = if cursor.consume_identifier("else") {
            Some(Box::new(PrattParser::parse(source, cursor, u8::MAX)?))
        } else {
            None
        };
        Ok(PrattParser::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::If {
                condition: Box::new(condition),
                then_block,
                else_expression,
            },
        ))
    }

    pub(super) fn parse_match(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        cursor.consume_identifier("match");
        let scrutinee = PrattParser::parse_control_header(source, cursor)?;
        let (_, group) = PrattParser::take_group(cursor)?;
        let mut contents = TokenCursor::new(cursor.tokens(), group.start + 1..group.end - 1)
            .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedToken))?;
        let attributes = PrattParser::parse_inner_attributes(&mut contents)?;
        let arms =
            Self::parse_match_arms(source, cursor.tokens(), contents.index()..contents.end())?;
        Ok(PrattParser::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
        )
        .with_attributes(attributes))
    }

    fn parse_match_arms(
        source: &SourceFile,
        tokens: &[Token],
        bounds: Range<usize>,
    ) -> Result<Vec<ExpressionMatchArmSyntax>, SyntaxError> {
        let mut cursor = TokenCursor::new(tokens, bounds)
            .ok_or_else(|| SyntaxError::new(SyntaxErrorKind::UnexpectedToken, 0..0))?;
        let mut arms = Vec::new();
        while !cursor.is_empty() {
            let arm_start = cursor.index();
            let attributes = PrattParser::parse_expression_attributes(&mut cursor)?;
            let pattern = match PatternParser::parse_prefix(
                &mut cursor,
                false,
                PatternParseContext::MatchArm,
            ) {
                Ok(pattern) => pattern,
                Err(error) => {
                    let arm_bounds = Self::recover_match_arm(&mut cursor, arm_start);
                    let body = Self::error_expression(&error);
                    arms.push(ExpressionMatchArmSyntax::new(
                        None, None, body, attributes, arm_bounds,
                    ));
                    continue;
                }
            };
            let guard = if cursor.consume_identifier("if") {
                match PrattParser::parse(source, &mut cursor, 0) {
                    Ok(guard) => Some(guard),
                    Err(error) => {
                        let arm_bounds = Self::recover_match_arm(&mut cursor, arm_start);
                        arms.push(ExpressionMatchArmSyntax::new(
                            Some(pattern),
                            None,
                            Self::error_expression(&error),
                            attributes,
                            arm_bounds,
                        ));
                        continue;
                    }
                }
            } else {
                None
            };
            if !cursor.consume_punctuation("=>") {
                let error = PathParser::error_at(&cursor, SyntaxErrorKind::UnexpectedToken);
                let arm_bounds = Self::recover_match_arm(&mut cursor, arm_start);
                arms.push(ExpressionMatchArmSyntax::new(
                    Some(pattern),
                    guard,
                    Self::error_expression(&error),
                    attributes,
                    arm_bounds,
                ));
                continue;
            }
            let body_start = cursor.index();
            let body_offset = cursor.offset();
            let body = match if cursor.at_open_delimiter(Delimiter::Brace) {
                Self::parse_block(
                    &mut cursor,
                    body_start,
                    body_offset,
                    ExpressionBlockKind::Plain,
                )
            } else {
                PrattParser::parse(source, &mut cursor, 0)
            } {
                Ok(body) => body,
                Err(error) => {
                    let arm_bounds = Self::recover_match_arm(&mut cursor, arm_start);
                    arms.push(ExpressionMatchArmSyntax::new(
                        Some(pattern),
                        guard,
                        Self::error_expression(&error),
                        attributes,
                        arm_bounds,
                    ));
                    continue;
                }
            };
            let arm_end = cursor.index();
            arms.push(ExpressionMatchArmSyntax::new(
                Some(pattern),
                guard,
                body.clone(),
                attributes,
                PrattParser::span(tokens, arm_start, arm_end),
            ));
            if cursor.consume_punctuation(",") || cursor.is_empty() {
                continue;
            }
            if !matches!(
                body.kind(),
                ExpressionSyntaxKind::Block { .. }
                    | ExpressionSyntaxKind::If { .. }
                    | ExpressionSyntaxKind::Match { .. }
                    | ExpressionSyntaxKind::Loop { .. }
                    | ExpressionSyntaxKind::While { .. }
                    | ExpressionSyntaxKind::For { .. }
            ) {
                let token = cursor.peek().expect("非空游标应有下一个 token");
                let error =
                    SyntaxError::new(SyntaxErrorKind::UnexpectedToken, token.start()..token.end());
                let error_bounds = error.bounds();
                arms.push(ExpressionMatchArmSyntax::new(
                    None,
                    None,
                    Self::error_expression(&error),
                    Vec::new(),
                    error_bounds,
                ));
            }
        }
        Ok(arms)
    }

    fn recover_match_arm(cursor: &mut TokenCursor<'_>, arm_start: usize) -> Range<usize> {
        cursor.recover_to(&[","]);
        if cursor.at_punctuation(",") {
            cursor.consume_punctuation(",");
        }
        PrattParser::span(cursor.tokens(), arm_start, cursor.index())
    }

    fn error_expression(error: &SyntaxError) -> ExpressionSyntax {
        let kind = match error.kind() {
            SyntaxErrorKind::UnexpectedToken => ExpressionErrorKind::UnexpectedToken,
            SyntaxErrorKind::UnexpectedEof => ExpressionErrorKind::UnexpectedEof,
            SyntaxErrorKind::UnclosedDelimiter => ExpressionErrorKind::UnclosedDelimiter,
        };
        ExpressionSyntax::new(ExpressionSyntaxKind::Error(kind), error.bounds())
    }

    pub(super) fn parse_loop(
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
        label: Option<String>,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        cursor.consume_identifier("loop");
        let body = Self::consume_block_bounds(cursor)?;
        Ok(PrattParser::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::Loop { label, body },
        ))
    }

    pub(super) fn parse_while(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
        label: Option<String>,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        cursor.consume_identifier("while");
        let condition = PrattParser::parse_control_header(source, cursor)?;
        let body = Self::consume_block_bounds(cursor)?;
        Ok(PrattParser::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::While {
                label,
                condition: Box::new(condition),
                body,
            },
        ))
    }

    pub(super) fn parse_for(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
        label: Option<String>,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        cursor.consume_identifier("for");
        let in_index =
            PrattParser::find_top_level_token(cursor.tokens(), cursor.index()..cursor.end(), "in")
                .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedToken))?;
        let pattern =
            PatternParser::parse_binding_range(cursor.tokens(), cursor.index()..in_index)?;
        while cursor.index() <= in_index {
            cursor.consume();
        }
        let iterator = PrattParser::parse_control_header(source, cursor)?;
        let body = Self::consume_block_bounds(cursor)?;
        Ok(PrattParser::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::For {
                label,
                pattern,
                iterator: Box::new(iterator),
                body,
            },
        ))
    }

    pub(super) fn parse_return(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        cursor.consume_identifier("return");
        let value = if PrattParser::is_expression_boundary(cursor) {
            None
        } else {
            Some(Box::new(PrattParser::parse(source, cursor, 0)?))
        };
        Ok(PrattParser::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::Return(value),
        ))
    }

    pub(super) fn parse_break(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        cursor.consume_identifier("break");
        let label = match cursor.peek().map(Token::kind) {
            Some(TokenKind::Lifetime(label)) => {
                let label = label.clone();
                cursor.consume();
                Some(label)
            }
            _ => None,
        };
        let value = if PrattParser::is_expression_boundary(cursor) {
            None
        } else {
            Some(Box::new(PrattParser::parse(source, cursor, 0)?))
        };
        Ok(PrattParser::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::Break { label, value },
        ))
    }

    pub(super) fn parse_continue(
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        cursor.consume_identifier("continue");
        let label = match cursor.peek().map(Token::kind) {
            Some(TokenKind::Lifetime(label)) => {
                let label = label.clone();
                cursor.consume();
                Some(label)
            }
            _ => None,
        };
        Ok(PrattParser::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::Continue(label),
        ))
    }

    pub(super) fn parse_let(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
        allow_struct_literals: bool,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        cursor.consume_identifier("let");
        let equals =
            PrattParser::find_top_level_token(cursor.tokens(), cursor.index()..cursor.end(), "=")
                .ok_or_else(|| PathParser::error_at(cursor, SyntaxErrorKind::UnexpectedToken))?;
        let pattern = PatternParser::parse_binding_range(cursor.tokens(), cursor.index()..equals)?;
        while cursor.index() < equals {
            cursor.consume();
        }
        cursor.consume_punctuation("=");
        let value =
            PrattParser::parse_with_struct_literals(source, cursor, 5, allow_struct_literals)?;
        Ok(PrattParser::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::Let {
                pattern,
                value: Box::new(value),
            },
        ))
    }

    pub(super) fn parse_labelled_block(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        let label = match cursor.consume().map(Token::kind) {
            Some(TokenKind::Lifetime(value)) => value.clone(),
            _ => {
                return Err(PathParser::error_at(
                    cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ))
            }
        };
        cursor.consume_punctuation(":");
        if cursor.at_identifier("loop") {
            return Self::parse_loop(cursor, start, start_offset, Some(label));
        }
        if cursor.at_identifier("while") {
            return Self::parse_while(source, cursor, start, start_offset, Some(label));
        }
        if cursor.at_identifier("for") {
            return Self::parse_for(source, cursor, start, start_offset, Some(label));
        }
        let (_, group) = PrattParser::take_group(cursor)?;
        let body = PrattParser::byte_span(cursor.tokens(), group);
        Ok(PrattParser::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::LabelBlock { label, body },
        ))
    }

    pub(super) fn parse_block(
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
        kind: ExpressionBlockKind,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        let (_, group) = PrattParser::take_group(cursor)?;
        let body = PrattParser::byte_span(cursor.tokens(), group);
        Ok(PrattParser::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::Block { kind, body },
        ))
    }

    pub(super) fn parse_unsupported_block(
        cursor: &mut TokenCursor<'_>,
        start: usize,
        start_offset: usize,
        kind: ExpressionUnsupportedKind,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        PrattParser::take_group(cursor)?;
        Ok(PrattParser::new_span(
            cursor,
            start,
            start_offset,
            ExpressionSyntaxKind::Unsupported(kind),
        ))
    }

    pub(super) fn consume_block_bounds(
        cursor: &mut TokenCursor<'_>,
    ) -> Result<Range<usize>, SyntaxError> {
        if !cursor.at_open_delimiter(Delimiter::Brace) {
            return Err(PathParser::error_at(
                cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        let (_, group) = PrattParser::take_group(cursor)?;
        Ok(PrattParser::byte_span(cursor.tokens(), group))
    }
}
