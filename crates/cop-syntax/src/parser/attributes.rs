use std::ops::Range;

use cop_model::{
    LocalizedMessage, MessageId, SYNTAX_ATTRIBUTE_UNCLOSED, SYNTAX_EXPECT_ATTRIBUTE_BRACKET,
    SYNTAX_VISIBILITY_UNCLOSED,
};

use crate::{
    ast::{AstFactory, Attribute, ParseIssue, ParseIssueKind, Visibility, VisibilityKind},
    recovery::Recovery,
    Delimiter, SourceFile, Token, TokenKind,
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
                    ParseIssueKind::UnexpectedToken,
                    start..Self::current_end(source, tokens, *cursor),
                    SYNTAX_EXPECT_ATTRIBUTE_BRACKET,
                );
                continue;
            }
            let Some(close_index) = Recovery::matching_delimiter(tokens, open_index) else {
                let end = tokens.last().map_or(start, Token::end);
                Self::push_issue(
                    issues,
                    ParseIssueKind::UnclosedDelimiter,
                    start..end,
                    SYNTAX_ATTRIBUTE_UNCLOSED,
                );
                *cursor = tokens.len();
                break;
            };
            let path = Self::read_attribute_path(tokens, open_index + 1, close_index);
            let end = tokens[close_index].end();
            attributes.push(AstFactory::attribute(path, inner, start..end));
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
                SYNTAX_VISIBILITY_UNCLOSED,
            );
        }
        let end = tokens
            .get((*cursor).saturating_sub(1))
            .map_or(start, Token::end);
        AstFactory::visibility(VisibilityKind::Public, start..end)
    }

    fn read_attribute_path(tokens: &[Token], mut index: usize, end: usize) -> Vec<String> {
        let mut path = Vec::new();
        while index < end {
            if let TokenKind::Identifier(identifier) = tokens[index].kind() {
                path.push(identifier.clone());
                index += 1;
                if index < end && Recovery::is_punctuation(&tokens[index], "::") {
                    index += 1;
                    continue;
                }
            }
            break;
        }
        path
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
        message: MessageId,
    ) {
        issues.push(AstFactory::parse_issue(
            kind,
            bounds,
            LocalizedMessage::new(message),
        ));
    }
}
