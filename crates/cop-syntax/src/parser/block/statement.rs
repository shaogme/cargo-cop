use std::ops::Range;

use crate::{
    parser::{
        expression::SyntaxCollector,
        patterns::PatternParser,
        token_cursor::{SyntaxError, SyntaxErrorKind, TokenCursor},
        types::TypeParser,
        use_tree::UseTreeParser,
    },
    Delimiter, SourceFile, Token, TokenKind,
};

#[derive(Clone, Copy)]
pub(super) enum StatementForm {
    Let,
    Use,
    Expression,
}

pub(super) struct ParsedStatement {
    pub(super) end: usize,
    pub(super) form: StatementForm,
    pub(super) has_semicolon: bool,
    pub(super) is_tail: bool,
    pub(super) block_like: bool,
    pub(super) pattern: Option<Range<usize>>,
    pub(super) type_annotation: Option<Range<usize>>,
    pub(super) expression: Option<Range<usize>>,
    pub(super) let_else_block: Option<Range<usize>>,
}

pub(super) struct StatementParser;

impl StatementParser {
    pub(super) fn parse(
        source: &SourceFile,
        tokens: &[Token],
        start: usize,
        end: usize,
    ) -> Result<ParsedStatement, SyntaxError> {
        let mut cursor = TokenCursor::new(tokens, start..end)
            .ok_or_else(|| SyntaxError::new(SyntaxErrorKind::UnexpectedToken, 0..0))?;
        if cursor.at_identifier("let") {
            return Self::parse_let(source, &mut cursor);
        }
        if cursor.at_identifier("use") {
            return Self::parse_use(&mut cursor);
        }
        Self::parse_expression(source, &mut cursor)
    }

    fn parse_let(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
    ) -> Result<ParsedStatement, SyntaxError> {
        cursor.consume_identifier("let");
        let pattern_start = cursor.index();
        PatternParser::parse_prefix(cursor, true)?;
        let pattern = Some(pattern_start..cursor.index());
        let type_annotation = if cursor.consume_punctuation(":") {
            let type_start = cursor.index();
            TypeParser::parse(cursor)?;
            Some(type_start..cursor.index())
        } else {
            None
        };
        let mut expression = None;
        let mut let_else_block = None;
        if cursor.consume_punctuation("=") {
            let expression_start = cursor.index();
            SyntaxCollector::parse_expression_prefix(source, cursor)?;
            expression = Some(expression_start..cursor.index());
            if cursor.consume_identifier("else") {
                if !cursor.at_open_delimiter(Delimiter::Brace) {
                    return Err(Self::error(cursor));
                }
                let block = cursor
                    .skip_delimited_group()
                    .ok_or_else(|| Self::error(cursor))?;
                if !matches!(
                    cursor.tokens()[block.start].kind(),
                    TokenKind::OpenDelimiter(Delimiter::Brace)
                ) {
                    return Err(Self::error(cursor));
                }
                let_else_block = Some(block);
            }
        }
        let has_semicolon = cursor.consume_punctuation(";");
        Ok(ParsedStatement {
            end: cursor.index(),
            form: StatementForm::Let,
            has_semicolon,
            is_tail: false,
            block_like: false,
            pattern,
            type_annotation,
            expression,
            let_else_block,
        })
    }

    fn parse_use(cursor: &mut TokenCursor<'_>) -> Result<ParsedStatement, SyntaxError> {
        cursor.consume_identifier("use");
        Self::parse_use_tree(cursor)?;
        let has_semicolon = cursor.consume_punctuation(";");
        Ok(ParsedStatement {
            end: cursor.index(),
            form: StatementForm::Use,
            has_semicolon,
            is_tail: false,
            block_like: false,
            pattern: None,
            type_annotation: None,
            expression: None,
            let_else_block: None,
        })
    }

    fn parse_use_tree(cursor: &mut TokenCursor<'_>) -> Result<(), SyntaxError> {
        Self::skip_use_tree_attributes(cursor)?;
        cursor.consume_punctuation("::");
        if cursor.at_open_delimiter(Delimiter::Brace) {
            return Self::parse_use_group(cursor);
        }

        Self::consume_use_path_segment(cursor)?;
        loop {
            if !cursor.consume_punctuation("::") {
                break;
            }
            if cursor.at_open_delimiter(Delimiter::Brace) {
                return Self::parse_use_group(cursor);
            }
            if cursor.consume_punctuation("*") {
                return Ok(());
            }
            Self::consume_use_path_segment(cursor)?;
        }

        if cursor.consume_identifier("as") {
            let Some(alias) = cursor.peek().and_then(|token| token.kind().identifier()) else {
                return Err(Self::error(cursor));
            };
            if alias != "_" && !UseTreeParser::is_use_path_identifier(alias) {
                return Err(Self::error(cursor));
            }
            cursor.consume();
        }
        Ok(())
    }

    fn consume_use_path_segment(cursor: &mut TokenCursor<'_>) -> Result<(), SyntaxError> {
        let Some(identifier) = cursor.peek().and_then(|token| token.kind().identifier()) else {
            return Err(Self::error(cursor));
        };
        if !UseTreeParser::is_use_path_identifier(identifier) {
            return Err(Self::error(cursor));
        }
        cursor.consume();
        Ok(())
    }

    fn parse_use_group(cursor: &mut TokenCursor<'_>) -> Result<(), SyntaxError> {
        cursor.consume_open_delimiter(Delimiter::Brace);
        if cursor.consume_close_delimiter(Delimiter::Brace) {
            return Err(Self::error(cursor));
        }
        loop {
            Self::parse_use_tree(cursor)?;
            if cursor.consume_close_delimiter(Delimiter::Brace) {
                return Ok(());
            }
            if !cursor.consume_punctuation(",") {
                return Err(Self::error(cursor));
            }
            if cursor.consume_close_delimiter(Delimiter::Brace) {
                return Ok(());
            }
        }
    }

    fn skip_use_tree_attributes(cursor: &mut TokenCursor<'_>) -> Result<(), SyntaxError> {
        while cursor.consume_punctuation("#") {
            if cursor.consume_punctuation("!") || !cursor.at_open_delimiter(Delimiter::Bracket) {
                return Err(Self::error(cursor));
            }
            cursor
                .skip_delimited_group()
                .ok_or_else(|| Self::error(cursor))?;
        }
        Ok(())
    }

    fn parse_expression(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
    ) -> Result<ParsedStatement, SyntaxError> {
        let expression_start = cursor.index();
        let expression = SyntaxCollector::parse_expression_prefix(source, cursor)?;
        let expression_end = cursor.index();
        let has_semicolon = cursor.consume_punctuation(";");
        let is_tail = !has_semicolon && cursor.is_empty();
        let block_like = SyntaxCollector::expression_is_block_like(&expression);
        Ok(ParsedStatement {
            end: cursor.index(),
            form: StatementForm::Expression,
            has_semicolon,
            is_tail,
            block_like,
            pattern: None,
            type_annotation: None,
            expression: Some(expression_start..expression_end),
            let_else_block: None,
        })
    }

    fn error(cursor: &TokenCursor<'_>) -> SyntaxError {
        let kind = if cursor.is_empty() {
            SyntaxErrorKind::UnexpectedEof
        } else {
            SyntaxErrorKind::UnexpectedToken
        };
        let bounds = cursor.peek().map_or_else(
            || cursor.offset()..cursor.offset(),
            |token| token.start()..token.end(),
        );
        SyntaxError::new(kind, bounds)
    }
}
