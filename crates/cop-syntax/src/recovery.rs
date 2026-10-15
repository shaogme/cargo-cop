use std::ops::Range;

use cop_model::{LocalizedMessage, SYNTAX_DELIMITER_UNCLOSED, SYNTAX_UNMATCHED_CLOSE_DELIMITER};

use crate::{
    ast::{AstFactory, CoverageStatus, ParseIssue, ParseIssueKind},
    lexer::{Delimiter, Token, TokenKind},
};

pub(crate) struct Recovery;

impl Recovery {
    pub(crate) fn function_keyword(tokens: &[Token], start: usize) -> Option<usize> {
        let mut expects_abi = false;
        for (index, token) in tokens.iter().enumerate().skip(start).take(8) {
            if Self::is_identifier(token, "fn") {
                return Some(index);
            }
            match token.kind() {
                TokenKind::Identifier(value)
                    if matches!(
                        value.as_str(),
                        "async" | "unsafe" | "const" | "extern" | "default"
                    ) =>
                {
                    expects_abi = value == "extern";
                }
                TokenKind::Literal(_) if expects_abi => expects_abi = false,
                _ => return None,
            }
        }
        None
    }

    pub(crate) fn impl_keyword(tokens: &[Token], start: usize) -> Option<usize> {
        for (index, token) in tokens.iter().enumerate().skip(start).take(3) {
            if Self::is_identifier(token, "impl") {
                return Some(index);
            }
            if !matches!(
                token.kind(),
                TokenKind::Identifier(value) if matches!(value.as_str(), "unsafe" | "default")
            ) {
                return None;
            }
        }
        None
    }

    pub(crate) fn trait_keyword(tokens: &[Token], start: usize) -> Option<usize> {
        for (index, token) in tokens.iter().enumerate().skip(start).take(3) {
            if Self::is_identifier(token, "trait") {
                return Some(index);
            }
            if !matches!(
                token.kind(),
                TokenKind::Identifier(value) if matches!(value.as_str(), "unsafe" | "auto")
            ) {
                return None;
            }
        }
        None
    }

    pub(crate) fn coverage_for(issues: &[ParseIssue]) -> CoverageStatus {
        if issues.is_empty() {
            CoverageStatus::Complete
        } else if issues.iter().any(|issue| {
            issue.kind() == ParseIssueKind::Lexical
                || issue.kind() == ParseIssueKind::UnclosedDelimiter
        }) {
            CoverageStatus::Incomplete
        } else {
            CoverageStatus::Partial
        }
    }

    pub(crate) fn delimiter_issues(tokens: &[Token]) -> Vec<ParseIssue> {
        let mut stack: Vec<(Delimiter, usize)> = Vec::new();
        let mut issues = Vec::new();
        for token in tokens {
            match token.kind() {
                TokenKind::OpenDelimiter(delimiter) => stack.push((*delimiter, token.start())),
                TokenKind::CloseDelimiter(delimiter) => {
                    if stack.last().is_some_and(|(open, _)| open == delimiter) {
                        stack.pop();
                    } else {
                        issues.push(AstFactory::parse_issue(
                            ParseIssueKind::UnclosedDelimiter,
                            token.start()..token.end(),
                            LocalizedMessage::new(SYNTAX_UNMATCHED_CLOSE_DELIMITER),
                        ));
                    }
                }
                _ => {}
            }
        }
        issues.extend(stack.into_iter().map(|(_, start)| {
            AstFactory::parse_issue(
                ParseIssueKind::UnclosedDelimiter,
                start..start,
                LocalizedMessage::new(SYNTAX_DELIMITER_UNCLOSED),
            )
        }));
        issues
    }

    pub(crate) fn matching_delimiter(tokens: &[Token], open_index: usize) -> Option<usize> {
        let TokenKind::OpenDelimiter(open_delimiter) = tokens.get(open_index)?.kind() else {
            return None;
        };
        let mut stack = vec![*open_delimiter];
        for (index, token) in tokens.iter().enumerate().skip(open_index + 1) {
            match token.kind() {
                TokenKind::OpenDelimiter(delimiter) => stack.push(*delimiter),
                TokenKind::CloseDelimiter(delimiter) => {
                    let expected = stack.pop()?;
                    if expected != *delimiter {
                        return None;
                    }
                    if stack.is_empty() {
                        return Some(index);
                    }
                }
                _ => {}
            }
        }
        None
    }

    pub(crate) fn function_parameter_open(tokens: &[Token], start: usize) -> Option<usize> {
        let mut index = start;
        let mut generic_depth = 0usize;
        while index < tokens.len() {
            match tokens[index].kind() {
                TokenKind::Punctuation(value) if value == "<" => generic_depth += 1,
                TokenKind::Punctuation(value) if value == ">" => {
                    generic_depth = generic_depth.saturating_sub(1);
                }
                TokenKind::Punctuation(value) if value == ">>" => {
                    generic_depth = generic_depth.saturating_sub(2);
                }
                TokenKind::OpenDelimiter(Delimiter::Parenthesis) if generic_depth == 0 => {
                    return Some(index);
                }
                TokenKind::OpenDelimiter(_) => {
                    index = Self::matching_delimiter(tokens, index)? + 1;
                    continue;
                }
                TokenKind::Punctuation(value) if value == ";" => return None,
                TokenKind::CloseDelimiter(Delimiter::Brace) => return None,
                _ => {}
            }
            index += 1;
        }
        None
    }

    pub(crate) fn body_open(tokens: &[Token], start: usize) -> Option<usize> {
        let mut index = start;
        let mut generic_depth = 0usize;
        while index < tokens.len() {
            match tokens[index].kind() {
                TokenKind::Punctuation(value) if value == "<" => generic_depth += 1,
                TokenKind::Punctuation(value) if value == ">" => {
                    generic_depth = generic_depth.saturating_sub(1);
                }
                TokenKind::Punctuation(value) if value == ">>" => {
                    generic_depth = generic_depth.saturating_sub(2);
                }
                TokenKind::OpenDelimiter(Delimiter::Brace) if generic_depth == 0 => {
                    return Some(index);
                }
                TokenKind::OpenDelimiter(_) => {
                    if let Some(close_index) = Self::matching_delimiter(tokens, index) {
                        index = close_index + 1;
                        continue;
                    }
                    return None;
                }
                TokenKind::Punctuation(punctuation) if punctuation == ";" => return None,
                TokenKind::CloseDelimiter(Delimiter::Brace) => return None,
                _ => {}
            }
            index += 1;
        }
        None
    }

    pub(crate) fn semicolon(tokens: &[Token], start: usize) -> Option<usize> {
        let mut index = start;
        while index < tokens.len() {
            match tokens[index].kind() {
                TokenKind::OpenDelimiter(_) => {
                    if let Some(close_index) = Self::matching_delimiter(tokens, index) {
                        index = close_index + 1;
                        continue;
                    }
                    return None;
                }
                TokenKind::Punctuation(punctuation) if punctuation == ";" => return Some(index),
                TokenKind::CloseDelimiter(Delimiter::Brace) => return None,
                _ => index += 1,
            }
        }
        None
    }

    pub(crate) fn receiver_parameter(tokens: &[Token], range: &Range<usize>) -> bool {
        let mut index = range.start;
        if tokens
            .get(index)
            .is_some_and(|token| Self::is_punctuation(token, "&"))
        {
            index += 1;
            if matches!(
                tokens.get(index).map(Token::kind),
                Some(TokenKind::Lifetime(_))
            ) {
                index += 1;
            }
            if tokens
                .get(index)
                .is_some_and(|token| Self::is_identifier(token, "mut"))
            {
                index += 1;
            }
        } else if tokens
            .get(index)
            .is_some_and(|token| Self::is_identifier(token, "mut"))
        {
            index += 1;
        }
        if !tokens
            .get(index)
            .is_some_and(|token| Self::is_identifier(token, "self"))
        {
            return false;
        }
        index += 1;
        index == range.end
            || tokens
                .get(index)
                .is_some_and(|token| Self::is_punctuation(token, ":"))
    }

    pub(crate) fn token_bounds(tokens: &[Token], range: Range<usize>) -> Range<usize> {
        let start = tokens.get(range.start).map_or(0, Token::start);
        let end = range
            .end
            .checked_sub(1)
            .and_then(|index| tokens.get(index))
            .map_or(start, Token::end);
        start..end
    }

    pub(crate) fn split_top_level(
        tokens: &[Token],
        start: usize,
        end: usize,
        separator: &str,
    ) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        let mut item_start = start;
        let mut index = start;
        while index < end {
            if let Some(close_index) = Self::generic_arguments_end(tokens, index, end) {
                index = close_index + 1;
                continue;
            }
            if let TokenKind::OpenDelimiter(_) = tokens[index].kind() {
                if let Some(close_index) = Self::matching_delimiter(tokens, index) {
                    if close_index < end {
                        index = close_index + 1;
                        continue;
                    }
                }
            }
            if Self::is_punctuation(&tokens[index], separator) {
                ranges.push(item_start..index);
                item_start = index + 1;
            }
            index += 1;
        }
        if item_start < end {
            ranges.push(item_start..end);
        }
        ranges
    }

    pub(crate) fn has_top_level_separator(
        tokens: &[Token],
        start: usize,
        end: usize,
        separator: &str,
    ) -> bool {
        let mut index = start;
        while index < end {
            if let Some(close_index) = Self::generic_arguments_end(tokens, index, end) {
                index = close_index + 1;
                continue;
            }
            if Self::is_punctuation(&tokens[index], separator) {
                return true;
            }
            if matches!(tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                if let Some(close_index) = Self::matching_delimiter(tokens, index) {
                    if close_index < end {
                        index = close_index + 1;
                        continue;
                    }
                }
            }
            index += 1;
        }
        false
    }

    fn generic_arguments_end(tokens: &[Token], open_index: usize, end: usize) -> Option<usize> {
        if !Self::is_punctuation(tokens.get(open_index)?, "<") {
            return None;
        }
        let previous = open_index
            .checked_sub(1)
            .and_then(|index| tokens.get(index))?;
        let turbofish = Self::is_punctuation(previous, "::");
        if !matches!(previous.kind(), TokenKind::Identifier(_)) && !turbofish {
            return None;
        }
        let mut depth = 1usize;
        let mut index = open_index + 1;
        while index < end {
            if matches!(tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                if let Some(close_index) = Self::matching_delimiter(tokens, index) {
                    if close_index < end {
                        index = close_index + 1;
                        continue;
                    }
                }
            }
            match tokens[index].kind() {
                TokenKind::Punctuation(value) if value == "<" => depth += 1,
                TokenKind::Punctuation(value) if value == ">" => {
                    depth = depth.saturating_sub(1);
                    if depth == 0
                        && Self::can_follow_generic_arguments(tokens, index + 1, turbofish)
                    {
                        return Some(index);
                    }
                }
                TokenKind::Punctuation(value) if value == ">>" => {
                    depth = depth.saturating_sub(2);
                    if depth == 0
                        && Self::can_follow_generic_arguments(tokens, index + 1, turbofish)
                    {
                        return Some(index);
                    }
                }
                _ => {}
            }
            index += 1;
        }
        None
    }

    fn can_follow_generic_arguments(tokens: &[Token], index: usize, turbofish: bool) -> bool {
        let Some(token) = tokens.get(index) else {
            return true;
        };
        match token.kind() {
            TokenKind::CloseDelimiter(_) | TokenKind::OpenDelimiter(Delimiter::Bracket) => true,
            TokenKind::OpenDelimiter(Delimiter::Parenthesis) => turbofish,
            TokenKind::Punctuation(value) => matches!(
                value.as_str(),
                "::" | "!" | "?" | "." | "+" | "=" | "," | ";" | ">" | ">>"
            ),
            TokenKind::Identifier(value) => matches!(value.as_str(), "as" | "where"),
            _ => false,
        }
    }

    pub(crate) fn is_punctuation(token: &Token, expected: &str) -> bool {
        matches!(token.kind(), TokenKind::Punctuation(value) if value == expected)
    }

    pub(crate) fn is_identifier(token: &Token, expected: &str) -> bool {
        matches!(token.kind(), TokenKind::Identifier(value) if value == expected)
    }
}
