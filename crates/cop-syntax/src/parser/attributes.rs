use cop_i18n::{t, RenderError};
use std::ops::Range;

use crate::{
    ast::{
        AstFactory, Attribute, AttributeInput, AttributeMeta, AttributeMetaArgument,
        AttributeMetaKind, AttributeMetaValue, AttributeMetaValueKind, ParseIssue, ParseIssueKind,
        Visibility, VisibilityKind,
    },
    recovery::Recovery,
    Delimiter, LiteralKind, RustStringLiteral, SourceFile, Token, TokenKind,
};

pub(super) struct AttributeParser;

impl AttributeParser {
    pub(super) fn parse_attributes(
        source: &SourceFile,
        tokens: &[Token],
        cursor: &mut usize,
        issues: &mut Vec<ParseIssue>,
    ) -> Vec<Attribute> {
        let mut attributes = Vec::new();
        while tokens
            .get(*cursor)
            .is_some_and(|token| Recovery::is_punctuation(token, "#"))
        {
            let start = tokens[*cursor].start();
            *cursor += 1;
            let inner = Self::consume_punctuation(tokens, cursor, "!");
            let open_index = *cursor;
            if !matches!(
                tokens.get(open_index).map(Token::kind),
                Some(TokenKind::OpenDelimiter(Delimiter::Bracket))
            ) {
                Self::push_issue(
                    issues,
                    ParseIssueKind::InvalidAttribute,
                    start..Self::current_end(source, tokens, *cursor),
                    t!("syntax.expect_attribute_bracket"),
                );
                continue;
            }
            let Some(close_index) = Recovery::matching_delimiter(tokens, open_index) else {
                let end = tokens.last().map_or(start, Token::end);
                Self::push_issue(
                    issues,
                    ParseIssueKind::UnclosedDelimiter,
                    start..end,
                    t!("syntax.attribute_unclosed"),
                );
                *cursor = tokens.len();
                break;
            };
            let mut meta_cursor = open_index + 1;
            let meta = match Self::parse_meta_item(source, tokens, &mut meta_cursor, close_index) {
                Ok(meta) if meta_cursor == close_index => meta,
                Ok(_) => {
                    let token = &tokens[meta_cursor];
                    Self::push_issue(
                        issues,
                        ParseIssueKind::InvalidAttribute,
                        token.start()..token.end(),
                        t!("syntax.attribute_invalid"),
                    );
                    Self::fallback_meta(tokens, open_index, close_index, token.start()..token.end())
                }
                Err(bounds) => {
                    Self::push_issue(
                        issues,
                        ParseIssueKind::InvalidAttribute,
                        bounds.clone(),
                        t!("syntax.attribute_invalid"),
                    );
                    Self::fallback_meta(tokens, open_index, close_index, bounds)
                }
            };
            let path = meta.path().to_vec();
            let end = tokens[close_index].end();
            let cfg_test = path.as_slice() == ["cfg"]
                && matches!(meta.kind(), AttributeMetaKind::List(values)
                if values.len() == 1
                    && values[0].meta().is_some_and(|value| {
                        value.path() == ["test"]
                            && matches!(value.kind(), AttributeMetaKind::Word)
                    }));
            attributes.push(AstFactory::attribute(AttributeInput {
                path,
                inner,
                cfg_test,
                meta,
                bounds: start..end,
            }));
            *cursor = close_index + 1;
        }
        attributes
    }

    pub(super) fn parse_visibility(
        source: &SourceFile,
        tokens: &[Token],
        cursor: &mut usize,
        issues: &mut Vec<ParseIssue>,
    ) -> Visibility {
        let start = Self::current_start(source, tokens, *cursor);
        if !Self::consume_identifier(tokens, cursor, "pub") {
            return AstFactory::visibility(VisibilityKind::Private, start..start);
        }
        if matches!(
            tokens.get(*cursor).map(Token::kind),
            Some(TokenKind::OpenDelimiter(Delimiter::Parenthesis))
        ) {
            let open_index = *cursor;
            if let Some(close_index) = Recovery::matching_delimiter(tokens, open_index) {
                let end = tokens[close_index].end();
                let restricted = source
                    .slice(tokens[open_index].start(), end)
                    .unwrap_or("")
                    .to_owned();
                *cursor = close_index + 1;
                return AstFactory::visibility(VisibilityKind::Restricted(restricted), start..end);
            }
            Self::push_issue(
                issues,
                ParseIssueKind::UnclosedDelimiter,
                start..Self::current_end(source, tokens, *cursor),
                t!("syntax.visibility_unclosed"),
            );
        }
        let end = tokens
            .get((*cursor).saturating_sub(1))
            .map_or(start, Token::end);
        AstFactory::visibility(VisibilityKind::Public, start..end)
    }

    fn parse_meta_item(
        source: &SourceFile,
        tokens: &[Token],
        cursor: &mut usize,
        end: usize,
    ) -> Result<AttributeMeta, Range<usize>> {
        let start = tokens.get(*cursor).map_or_else(
            || tokens.get(end.saturating_sub(1)).map_or(0, Token::end),
            Token::start,
        );
        let mut path = Vec::new();
        while *cursor < end {
            if let Some(identifier) = tokens[*cursor].kind().word() {
                path.push(identifier.to_owned());
                *cursor += 1;
                if *cursor < end && Recovery::is_punctuation(&tokens[*cursor], "::") {
                    if *cursor + 1 >= end || tokens[*cursor + 1].kind().word().is_none() {
                        return Err(tokens[*cursor].start()..tokens[*cursor].end());
                    }
                    *cursor += 1;
                    continue;
                }
                break;
            }
            let bounds = tokens
                .get(*cursor)
                .map_or(start..start, |token| token.start()..token.end());
            return Err(bounds);
        }
        if path.is_empty() {
            return Err(start..tokens.get(*cursor).map_or(start, Token::end));
        }
        let path_end = tokens[*cursor - 1].end();
        let kind = if *cursor < end && Recovery::is_punctuation(&tokens[*cursor], "=") {
            *cursor += 1;
            let value = Self::parse_meta_value(source, tokens, cursor, end)?;
            AttributeMetaKind::NameValue(value)
        } else if matches!(
            tokens.get(*cursor).map(Token::kind),
            Some(TokenKind::OpenDelimiter(Delimiter::Parenthesis))
        ) {
            let open = *cursor;
            let close = Recovery::matching_delimiter(tokens, open)
                .filter(|close| *close < end)
                .ok_or_else(|| tokens[open].start()..tokens[end - 1].end())?;
            let mut arguments = Vec::new();
            let mut child_cursor = open + 1;
            while child_cursor < close {
                arguments.push(Self::parse_meta_argument(
                    source,
                    tokens,
                    &mut child_cursor,
                    close,
                )?);
                if child_cursor == close {
                    break;
                }
                if !Recovery::is_punctuation(&tokens[child_cursor], ",") {
                    return Err(tokens[child_cursor].start()..tokens[child_cursor].end());
                }
                child_cursor += 1;
            }
            *cursor = close + 1;
            AttributeMetaKind::List(arguments)
        } else {
            AttributeMetaKind::Word
        };
        let finish = tokens
            .get(cursor.saturating_sub(1))
            .map_or(path_end, Token::end);
        Ok(AttributeMeta::new(path, kind, start..finish))
    }

    fn parse_meta_argument(
        source: &SourceFile,
        tokens: &[Token],
        cursor: &mut usize,
        end: usize,
    ) -> Result<AttributeMetaArgument, Range<usize>> {
        let is_boolean = matches!(
            tokens.get(*cursor).map(Token::kind),
            Some(TokenKind::Identifier(value)) if matches!(value.as_str(), "true" | "false")
        );
        if tokens
            .get(*cursor)
            .is_some_and(|token| token.kind().word().is_some() && !is_boolean)
        {
            Self::parse_meta_item(source, tokens, cursor, end).map(AttributeMetaArgument::Meta)
        } else if is_boolean
            || tokens
                .get(*cursor)
                .is_some_and(|token| Self::is_meta_value_start(token.kind()))
        {
            Self::parse_meta_value(source, tokens, cursor, end).map(AttributeMetaArgument::Value)
        } else {
            Err(tokens
                .get(*cursor)
                .map_or(0..0, |token| token.start()..token.end()))
        }
    }

    fn is_meta_value_start(kind: &TokenKind) -> bool {
        match kind {
            TokenKind::Literal(_) | TokenKind::OpenDelimiter(_) => true,
            TokenKind::Punctuation(value) => matches!(
                value.as_str(),
                "!" | "&" | "&&" | "*" | "+" | "-" | ".." | "..=" | "|" | "||"
            ),
            _ => false,
        }
    }

    fn parse_meta_value(
        source: &SourceFile,
        tokens: &[Token],
        cursor: &mut usize,
        end: usize,
    ) -> Result<AttributeMetaValue, Range<usize>> {
        let value_start = *cursor;
        let value_end = Self::meta_value_end(tokens, value_start, end)?;
        if value_start == value_end {
            return Err(tokens
                .get(value_start)
                .map_or(0..0, |token| token.start()..token.end()));
        }
        let first = &tokens[value_start];
        let last = &tokens[value_end - 1];
        *cursor = value_end;
        let value_kind = match first.kind() {
            TokenKind::Literal(kind) if value_end == value_start + 1 => {
                let text = first.text(source).to_owned();
                let decoded_string = (*kind == LiteralKind::String)
                    .then(|| RustStringLiteral::decode(&text))
                    .flatten();
                AttributeMetaValueKind::Literal {
                    kind: *kind,
                    text,
                    decoded_string,
                }
            }
            TokenKind::Identifier(value)
                if value_end == value_start + 1 && matches!(value.as_str(), "true" | "false") =>
            {
                AttributeMetaValueKind::Boolean(value == "true")
            }
            _ => AttributeMetaValueKind::Expression(first.start()..last.end()),
        };
        Ok(AttributeMetaValue::new(
            value_kind,
            first.start()..last.end(),
        ))
    }

    fn meta_value_end(tokens: &[Token], start: usize, end: usize) -> Result<usize, Range<usize>> {
        Recovery::split_top_level(tokens, start, end, ",")
            .first()
            .filter(|range| range.start == start && range.end > start)
            .map_or_else(
                || {
                    Err(tokens
                        .get(start)
                        .map_or(0..0, |token| token.start()..token.end()))
                },
                |range| Ok(range.end),
            )
    }

    fn fallback_meta(
        tokens: &[Token],
        open: usize,
        close: usize,
        bounds: Range<usize>,
    ) -> AttributeMeta {
        let mut cursor = open + 1;
        let path = Self::read_path(tokens, &mut cursor, close).unwrap_or_default();
        AttributeMeta::new(path, AttributeMetaKind::Word, bounds)
    }

    fn read_path(tokens: &[Token], cursor: &mut usize, end: usize) -> Option<Vec<String>> {
        let mut path = Vec::new();
        while *cursor < end {
            path.push(tokens[*cursor].kind().word()?.to_owned());
            *cursor += 1;
            if *cursor < end && Recovery::is_punctuation(&tokens[*cursor], "::") {
                *cursor += 1;
                if *cursor >= end || tokens[*cursor].kind().word().is_none() {
                    return None;
                }
            } else {
                break;
            }
        }
        (!path.is_empty()).then_some(path)
    }

    fn consume_identifier(tokens: &[Token], cursor: &mut usize, expected: &str) -> bool {
        if matches!(
            tokens.get(*cursor).map(Token::kind),
            Some(TokenKind::Identifier(identifier)) if identifier == expected
        ) {
            *cursor += 1;
            true
        } else {
            false
        }
    }

    fn consume_punctuation(tokens: &[Token], cursor: &mut usize, expected: &str) -> bool {
        if tokens
            .get(*cursor)
            .is_some_and(|token| Recovery::is_punctuation(token, expected))
        {
            *cursor += 1;
            true
        } else {
            false
        }
    }

    fn current_start(source: &SourceFile, tokens: &[Token], cursor: usize) -> usize {
        tokens.get(cursor).map_or(source.len(), Token::start)
    }

    fn current_end(source: &SourceFile, tokens: &[Token], cursor: usize) -> usize {
        tokens.get(cursor).map_or(source.len(), Token::end)
    }

    fn push_issue(
        issues: &mut Vec<ParseIssue>,
        kind: ParseIssueKind,
        bounds: Range<usize>,
        message: Result<String, RenderError>,
    ) {
        issues.push(AstFactory::parse_issue(kind, bounds, message));
    }
}
