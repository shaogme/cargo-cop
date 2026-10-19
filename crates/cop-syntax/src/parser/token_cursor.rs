use std::ops::Range;

use crate::{Delimiter, Token, TokenKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CursorCheckpoint {
    index: usize,
    pending_angle_close: bool,
}

pub(super) struct TokenCursor<'a> {
    tokens: &'a [Token],
    start: usize,
    end: usize,
    index: usize,
    pending_angle_close: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SyntaxErrorKind {
    UnexpectedToken,
    UnexpectedEof,
    UnclosedDelimiter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SyntaxError {
    kind: SyntaxErrorKind,
    bounds: Range<usize>,
}

impl SyntaxError {
    pub(super) fn new(kind: SyntaxErrorKind, bounds: Range<usize>) -> Self {
        Self { kind, bounds }
    }

    pub(super) fn kind(&self) -> SyntaxErrorKind {
        self.kind
    }

    pub(super) fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }
}

impl<'a> TokenCursor<'a> {
    pub(super) fn new(tokens: &'a [Token], bounds: Range<usize>) -> Option<Self> {
        if bounds.start > bounds.end || bounds.end > tokens.len() {
            return None;
        }
        Some(Self {
            tokens,
            start: bounds.start,
            end: bounds.end,
            index: bounds.start,
            pending_angle_close: false,
        })
    }

    pub(super) fn start(&self) -> usize {
        self.start
    }

    pub(super) fn end(&self) -> usize {
        self.end
    }

    pub(super) fn tokens(&self) -> &'a [Token] {
        self.tokens
    }

    pub(super) fn index(&self) -> usize {
        self.index
    }

    pub(super) fn consumed_end_index(&self) -> usize {
        self.index + usize::from(self.pending_angle_close)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.index >= self.end && !self.pending_angle_close
    }

    pub(super) fn peek(&self) -> Option<&'a Token> {
        (self.index < self.end).then(|| &self.tokens[self.index])
    }

    pub(super) fn peek_n(&self, distance: usize) -> Option<&'a Token> {
        self.index
            .checked_add(distance)
            .filter(|index| *index < self.end)
            .map(|index| &self.tokens[index])
    }

    pub(super) fn consume(&mut self) -> Option<&'a Token> {
        if self.pending_angle_close || self.index >= self.end {
            return None;
        }
        let token = &self.tokens[self.index];
        self.index += 1;
        Some(token)
    }

    pub(super) fn checkpoint(&self) -> CursorCheckpoint {
        CursorCheckpoint {
            index: self.index,
            pending_angle_close: self.pending_angle_close,
        }
    }

    pub(super) fn restore(&mut self, checkpoint: CursorCheckpoint) -> bool {
        if checkpoint.index < self.start || checkpoint.index > self.end {
            return false;
        }
        self.index = checkpoint.index;
        self.pending_angle_close = checkpoint.pending_angle_close
            && checkpoint.index < self.end
            && matches!(self.tokens[checkpoint.index].kind(), TokenKind::Punctuation(value) if value == ">>");
        true
    }

    pub(super) fn offset(&self) -> usize {
        if self.pending_angle_close {
            return self.tokens[self.index].start() + 1;
        }
        self.tokens
            .get(self.index)
            .filter(|_| self.index < self.end)
            .map_or_else(|| self.end_offset(), Token::start)
    }

    pub(super) fn consumed_offset(&self) -> usize {
        if self.pending_angle_close {
            return self.tokens[self.index].start() + 1;
        }
        if self.index > self.start {
            return self.tokens[self.index - 1].end();
        }
        self.offset_at_start()
    }

    pub(super) fn end_offset(&self) -> usize {
        if self.start == self.end {
            return self
                .tokens
                .get(self.start)
                .map_or_else(|| self.offset_at_start(), Token::start);
        }
        self.tokens[self.end - 1].end()
    }

    pub(super) fn at_punctuation(&self, expected: &str) -> bool {
        if self.pending_angle_close {
            return expected == ">";
        }
        matches!(self.peek().map(Token::kind), Some(TokenKind::Punctuation(value)) if value == expected)
    }

    pub(super) fn at_identifier(&self, expected: &str) -> bool {
        !self.pending_angle_close
            && self
                .peek()
                .and_then(|token| token.kind().word())
                .is_some_and(|value| value == expected)
    }

    pub(super) fn consume_punctuation(&mut self, expected: &str) -> bool {
        if self.at_punctuation(expected) {
            if self.pending_angle_close {
                self.pending_angle_close = false;
                self.index += 1;
            } else {
                self.index += 1;
            }
            true
        } else {
            false
        }
    }

    pub(super) fn consume_identifier(&mut self, expected: &str) -> bool {
        if self.at_identifier(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    pub(super) fn at_open_delimiter(&self, expected: Delimiter) -> bool {
        matches!(self.peek().map(Token::kind), Some(TokenKind::OpenDelimiter(value)) if *value == expected)
    }

    pub(super) fn at_close_delimiter(&self, expected: Delimiter) -> bool {
        matches!(self.peek().map(Token::kind), Some(TokenKind::CloseDelimiter(value)) if *value == expected)
    }

    pub(super) fn consume_open_delimiter(&mut self, expected: Delimiter) -> bool {
        if self.at_open_delimiter(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    pub(super) fn consume_close_delimiter(&mut self, expected: Delimiter) -> bool {
        if self.at_close_delimiter(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    pub(super) fn consume_angle_close(&mut self) -> bool {
        if self.pending_angle_close {
            self.pending_angle_close = false;
            self.index += 1;
            return true;
        }
        match self.peek().map(Token::kind) {
            Some(TokenKind::Punctuation(value)) if value == ">" => {
                self.index += 1;
                true
            }
            Some(TokenKind::Punctuation(value)) if value == ">>" => {
                self.pending_angle_close = true;
                true
            }
            _ => false,
        }
    }

    pub(super) fn skip_delimited_group(&mut self) -> Option<Range<usize>> {
        if self.pending_angle_close {
            return None;
        }
        let open = self.index;
        match self.peek()?.kind() {
            TokenKind::OpenDelimiter(_) => {}
            _ => return None,
        }
        let mut stack = Vec::new();
        let mut index = open;
        while index < self.end {
            match self.tokens[index].kind() {
                TokenKind::OpenDelimiter(found) => stack.push(*found),
                TokenKind::CloseDelimiter(found) => {
                    if stack.last() != Some(found) {
                        return None;
                    }
                    stack.pop();
                    if stack.is_empty() {
                        self.index = index + 1;
                        return Some(open..self.index);
                    }
                }
                _ => {}
            }
            index += 1;
        }
        None
    }

    pub(super) fn subcursor(&self, bounds: Range<usize>) -> Option<TokenCursor<'a>> {
        if bounds.start < self.start || bounds.end > self.end {
            return None;
        }
        TokenCursor::new(self.tokens, bounds)
    }

    pub(super) fn recover_to(&mut self, punctuation: &[&str]) -> usize {
        while !self.is_empty() {
            if punctuation.iter().any(|value| self.at_punctuation(value)) {
                break;
            }
            if matches!(
                self.peek().map(Token::kind),
                Some(TokenKind::OpenDelimiter(_))
            ) && self.skip_delimited_group().is_some()
            {
                continue;
            }
            self.index += 1;
        }
        self.index
    }

    pub(super) fn find_top_level_punctuation(&self, expected: &str) -> Option<usize> {
        if self.pending_angle_close {
            return None;
        }
        let mut angle_depth = 0usize;
        let mut index = self.index;
        while index < self.end {
            if matches!(self.tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                index = self.nested_group_end(index)? + 1;
                continue;
            }
            match self.tokens[index].kind() {
                TokenKind::Punctuation(value) if value == "<" => angle_depth += 1,
                TokenKind::Punctuation(value) if value == ">" => {
                    angle_depth = angle_depth.saturating_sub(1);
                }
                TokenKind::Punctuation(value) if value == ">>" => {
                    angle_depth = angle_depth.saturating_sub(2);
                }
                TokenKind::Punctuation(value) if value == expected && angle_depth == 0 => {
                    return Some(index);
                }
                _ => {}
            }
            index += 1;
        }
        None
    }

    pub(super) fn find_top_level_identifier(&self, expected: &str) -> Option<usize> {
        if self.pending_angle_close {
            return None;
        }
        let mut angle_depth = 0usize;
        let mut index = self.index;
        while index < self.end {
            if matches!(self.tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                index = self.nested_group_end(index)? + 1;
                continue;
            }
            match self.tokens[index].kind() {
                TokenKind::Punctuation(value) if value == "<" => angle_depth += 1,
                TokenKind::Punctuation(value) if value == ">" => {
                    angle_depth = angle_depth.saturating_sub(1);
                }
                TokenKind::Punctuation(value) if value == ">>" => {
                    angle_depth = angle_depth.saturating_sub(2);
                }
                TokenKind::Identifier(value) if value == expected && angle_depth == 0 => {
                    return Some(index);
                }
                _ => {}
            }
            index += 1;
        }
        None
    }

    fn nested_group_end(&self, open: usize) -> Option<usize> {
        match self.tokens.get(open)?.kind() {
            TokenKind::OpenDelimiter(_) => {}
            _ => return None,
        }
        let mut stack = Vec::new();
        for index in open..self.end {
            match self.tokens[index].kind() {
                TokenKind::OpenDelimiter(delimiter) => stack.push(*delimiter),
                TokenKind::CloseDelimiter(delimiter) => {
                    if stack.last() != Some(delimiter) {
                        return None;
                    }
                    stack.pop();
                    if stack.is_empty() {
                        return Some(index);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn offset_at_start(&self) -> usize {
        self.tokens
            .get(self.start)
            .map_or_else(|| self.tokens.last().map_or(0, Token::end), Token::start)
    }
}

#[cfg(test)]
mod tests {
    use crate::{RustLexer, SourceFile, TokenKind};

    use super::TokenCursor;

    #[test]
    fn checkpoints_restore_and_delimiter_skips_stay_inside_the_range() {
        let source = SourceFile::new("src/lib.rs", "left, (inside, nested), right");
        let lexed = RustLexer::lex(&source);
        let tokens = lexed.tokens();
        let open = tokens
            .iter()
            .position(|token| matches!(token.kind(), TokenKind::OpenDelimiter(_)))
            .expect("分隔符起点");
        let close = tokens
            .iter()
            .rposition(|token| matches!(token.kind(), TokenKind::CloseDelimiter(_)))
            .expect("分隔符终点");
        let mut bounded = TokenCursor::new(tokens, open..close + 1).expect("有界游标");
        let checkpoint = bounded.checkpoint();

        assert_eq!(bounded.skip_delimited_group(), Some(open..close + 1));
        assert!(bounded.is_empty());
        assert!(bounded.restore(checkpoint));
        assert_eq!(bounded.index(), open);

        let mut truncated = TokenCursor::new(tokens, open..close).expect("截断子范围");
        assert_eq!(truncated.skip_delimited_group(), None);
        assert_eq!(truncated.index(), open);
    }

    #[test]
    fn recovery_stops_at_synchronization_tokens_without_crossing_its_bound() {
        let source = SourceFile::new("src/lib.rs", "broken, kept");
        let lexed = RustLexer::lex(&source);
        let tokens = lexed.tokens();
        let comma = tokens
            .iter()
            .position(|token| matches!(token.kind(), TokenKind::Punctuation(value) if value == ","))
            .expect("逗号同步点");
        let mut cursor = TokenCursor::new(tokens, 0..comma + 1).expect("有界游标");

        assert_eq!(cursor.recover_to(&[","]), comma);
        assert_eq!(cursor.offset(), tokens[comma].start());
    }

    #[test]
    fn nested_generic_closers_consume_compound_punctuation_by_byte() {
        let source = SourceFile::new("src/lib.rs", ">>");
        let lexed = RustLexer::lex(&source);
        let tokens = lexed.tokens();
        let mut cursor = TokenCursor::new(tokens, 0..tokens.len()).expect("有界游标");

        assert!(matches!(
            tokens[0].kind(),
            TokenKind::Punctuation(value) if value == ">>"
        ));
        assert!(cursor.consume_angle_close());
        assert_eq!(cursor.offset(), 1);
        assert!(!cursor.is_empty());
        assert!(cursor.consume_angle_close());
        assert_eq!(cursor.offset(), 2);
        assert!(cursor.is_empty());
    }
}
