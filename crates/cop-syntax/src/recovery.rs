use cop_i18n::t;
use std::ops::Range;

use crate::{
    ast::{AstFactory, CoverageStatus, ParseIssue, ParseIssueKind},
    lexer::{Delimiter, LiteralKind, Token, TokenKind},
};

pub(crate) struct Recovery;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AngleScanContext {
    Type,
    Header,
    Expression,
    TokenSequence,
}

pub(crate) struct ExternBlockHeader {
    pub(crate) unsafe_: bool,
    pub(crate) abi: Option<Range<usize>>,
    pub(crate) extern_index: usize,
    pub(crate) open_index: usize,
}

impl Recovery {
    pub(crate) fn extern_block_header(tokens: &[Token], start: usize) -> Option<ExternBlockHeader> {
        let unsafe_ = Self::is_identifier(tokens.get(start)?, "unsafe");
        let extern_index = start + usize::from(unsafe_);
        if !Self::is_identifier(tokens.get(extern_index)?, "extern") {
            return None;
        }
        let mut open_index = extern_index + 1;
        let abi = match tokens.get(open_index).map(Token::kind) {
            Some(TokenKind::Literal(LiteralKind::String)) => {
                let token = &tokens[open_index];
                open_index += 1;
                Some(token.start()..token.end())
            }
            _ => None,
        };
        matches!(
            tokens.get(open_index).map(Token::kind),
            Some(TokenKind::OpenDelimiter(Delimiter::Brace))
        )
        .then_some(ExternBlockHeader {
            unsafe_,
            abi,
            extern_index,
            open_index,
        })
    }

    pub(crate) fn function_keyword(tokens: &[Token], start: usize) -> Option<usize> {
        let mut expects_abi = false;
        for (index, token) in tokens.iter().enumerate().skip(start).take(8) {
            if Self::is_identifier(token, "fn") {
                return Some(index);
            }
            match token.kind() {
                TokenKind::EditionKeyword(value) if value == "async" => {
                    expects_abi = false;
                }
                TokenKind::Identifier(value)
                    if matches!(value.as_str(), "unsafe" | "const" | "extern" | "default") =>
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
        } else if issues
            .iter()
            .any(|issue| issue.coverage() == CoverageStatus::Incomplete)
        {
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
                    } else if let Some(position) =
                        stack.iter().rposition(|(open, _)| open == delimiter)
                    {
                        for (_, start) in &stack[position + 1..] {
                            issues.push(AstFactory::parse_issue_with_boundary(
                                ParseIssueKind::UnclosedDelimiter,
                                *start..token.start(),
                                t!("syntax.delimiter_unclosed"),
                                false,
                            ));
                        }
                        stack.truncate(position);
                    } else {
                        issues.push(AstFactory::parse_issue(
                            ParseIssueKind::UnexpectedToken,
                            token.start()..token.end(),
                            t!("syntax.unmatched_close_delimiter"),
                        ));
                    }
                }
                _ => {}
            }
        }
        let source_end = tokens.last().map_or(0, Token::end);
        issues.extend(stack.into_iter().map(|(_, start)| {
            AstFactory::parse_issue_with_boundary(
                ParseIssueKind::UnclosedDelimiter,
                start..source_end,
                t!("syntax.delimiter_unclosed"),
                true,
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
                    if stack.last() == Some(delimiter) {
                        stack.pop();
                        if stack.is_empty() {
                            return Some(index);
                        }
                    } else if let Some(position) = stack.iter().rposition(|open| open == delimiter)
                    {
                        stack.truncate(position);
                        if stack.is_empty() {
                            return Some(index);
                        }
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
            if matches!(
                tokens[index].kind(),
                TokenKind::OpenDelimiter(Delimiter::Parenthesis)
            ) && generic_depth == 0
            {
                return Some(index);
            }
            match tokens[index].kind() {
                TokenKind::OpenDelimiter(_) => {
                    index = Self::matching_delimiter(tokens, index)? + 1;
                    continue;
                }
                TokenKind::Punctuation(value) if value == ";" => return None,
                TokenKind::CloseDelimiter(Delimiter::Brace) => return None,
                _ => {}
            }
            Self::update_angle_depth(
                tokens[index].kind(),
                AngleScanContext::Header,
                &mut generic_depth,
            )?;
            index += 1;
        }
        None
    }

    pub(crate) fn body_open(tokens: &[Token], start: usize) -> Option<usize> {
        let mut index = start;
        let mut generic_depth = 0usize;
        while index < tokens.len() {
            if matches!(
                tokens[index].kind(),
                TokenKind::OpenDelimiter(Delimiter::Brace)
            ) && generic_depth == 0
            {
                return Some(index);
            }
            match tokens[index].kind() {
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
            Self::update_angle_depth(
                tokens[index].kind(),
                AngleScanContext::Header,
                &mut generic_depth,
            )?;
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
        context: AngleScanContext,
    ) -> Option<Vec<Range<usize>>> {
        if start > end || end > tokens.len() {
            return None;
        }
        let mut ranges = Vec::new();
        let mut item_start = start;
        let mut index = start;
        let mut angle_depth = 0usize;
        while index < end {
            if let TokenKind::OpenDelimiter(_) = tokens[index].kind() {
                let close_index = Self::matching_delimiter(tokens, index)?;
                if close_index >= end {
                    return None;
                }
                index = close_index + 1;
                continue;
            }
            if angle_depth == 0 && Self::is_punctuation(&tokens[index], separator) {
                ranges.push(item_start..index);
                item_start = index + 1;
                index += 1;
                continue;
            }
            Self::update_angle_depth(tokens[index].kind(), context, &mut angle_depth)?;
            index += 1;
        }
        if angle_depth != 0 {
            return None;
        }
        if item_start < end {
            ranges.push(item_start..end);
        }
        Some(ranges)
    }

    fn update_angle_depth(
        kind: &TokenKind,
        context: AngleScanContext,
        angle_depth: &mut usize,
    ) -> Option<()> {
        if matches!(
            context,
            AngleScanContext::Expression | AngleScanContext::TokenSequence
        ) {
            return Some(());
        }
        match kind {
            TokenKind::Punctuation(value) if value == "<" => {
                *angle_depth = angle_depth.checked_add(1)?;
            }
            TokenKind::Punctuation(value) if value == "<<" => {
                *angle_depth = angle_depth.checked_add(2)?;
            }
            TokenKind::Punctuation(value) if value == ">" => {
                *angle_depth = angle_depth.checked_sub(1)?;
            }
            TokenKind::Punctuation(value) if value == ">>" => {
                *angle_depth = angle_depth.checked_sub(2)?;
            }
            _ => {}
        }
        Some(())
    }

    pub(crate) fn is_punctuation(token: &Token, expected: &str) -> bool {
        matches!(token.kind(), TokenKind::Punctuation(value) if value == expected)
    }

    pub(crate) fn is_identifier(token: &Token, expected: &str) -> bool {
        token.kind().word().is_some_and(|value| value == expected)
    }
}
