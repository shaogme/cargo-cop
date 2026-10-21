use std::ops::Range;

use crate::{Delimiter, Token, TokenKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CursorCheckpoint {
    index: usize,
    punctuation_offset: Option<usize>,
}

pub(super) struct TokenCursor<'a> {
    tokens: &'a [Token],
    start: usize,
    end: usize,
    index: usize,
    punctuation_offset: Option<usize>,
    pending_token: Option<Token>,
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
            punctuation_offset: None,
            pending_token: None,
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
        self.index + usize::from(self.punctuation_offset.is_some())
    }

    pub(super) fn is_empty(&self) -> bool {
        self.index >= self.end && self.punctuation_offset.is_none()
    }

    pub(super) fn peek(&self) -> Option<&Token> {
        if self.punctuation_offset.is_some() {
            return self.pending_token.as_ref();
        }
        (self.index < self.end).then(|| &self.tokens[self.index])
    }

    pub(super) fn peek_n(&self, distance: usize) -> Option<&Token> {
        if distance == 0 {
            return self.peek();
        }
        self.index
            .checked_add(distance)
            .filter(|index| *index < self.end)
            .map(|index| &self.tokens[index])
    }

    pub(super) fn consume(&mut self) -> Option<&'a Token> {
        if self.punctuation_offset.is_some() || self.index >= self.end {
            return None;
        }
        let token = &self.tokens[self.index];
        self.index += 1;
        Some(token)
    }

    pub(super) fn checkpoint(&self) -> CursorCheckpoint {
        CursorCheckpoint {
            index: self.index,
            punctuation_offset: self.punctuation_offset,
        }
    }

    pub(super) fn restore(&mut self, checkpoint: CursorCheckpoint) -> bool {
        if checkpoint.index < self.start || checkpoint.index > self.end {
            return false;
        }
        let pending_token = if let Some(offset) = checkpoint.punctuation_offset {
            if checkpoint.index >= self.end {
                return false;
            }
            let Some(token) = self.tokens.get(checkpoint.index) else {
                return false;
            };
            let Some(fragment) = token.punctuation_fragment(offset) else {
                return false;
            };
            if !matches!(fragment.kind(), TokenKind::Punctuation(_)) {
                return false;
            }
            Some(fragment)
        } else {
            None
        };
        self.index = checkpoint.index;
        self.punctuation_offset = checkpoint.punctuation_offset;
        self.pending_token = pending_token;
        true
    }

    pub(super) fn offset(&self) -> usize {
        if let Some(token) = self.pending_token.as_ref() {
            return token.start();
        }
        self.tokens
            .get(self.index)
            .filter(|_| self.index < self.end)
            .map_or_else(|| self.end_offset(), Token::start)
    }

    pub(super) fn consumed_offset(&self) -> usize {
        if let Some(token) = self.pending_token.as_ref() {
            return token.start();
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
        matches!(self.peek().map(Token::kind), Some(TokenKind::Punctuation(value)) if value == expected)
    }

    pub(super) fn at_identifier(&self, expected: &str) -> bool {
        self.peek()
            .and_then(|token| token.kind().word())
            .is_some_and(|value| value == expected)
    }

    pub(super) fn consume_punctuation(&mut self, expected: &str) -> bool {
        if self.at_punctuation(expected) {
            self.advance_current_punctuation();
            true
        } else {
            false
        }
    }

    pub(super) fn consume_punctuation_prefix(&mut self, prefix: &str) -> bool {
        if prefix.is_empty() {
            return false;
        }
        let Some(token) = self.peek() else {
            return false;
        };
        let TokenKind::Punctuation(value) = token.kind() else {
            return false;
        };
        if !value.starts_with(prefix) {
            return false;
        }
        if value.len() == prefix.len() {
            self.advance_current_punctuation();
            return true;
        }

        let next_offset = self.punctuation_offset.unwrap_or(0) + prefix.len();
        let Some(fragment) = self.tokens[self.index].punctuation_fragment(next_offset) else {
            return false;
        };
        self.punctuation_offset = Some(next_offset);
        self.pending_token = Some(fragment);
        true
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
        if self.at_punctuation(">") {
            return self.consume_punctuation(">");
        }
        self.at_punctuation(">>") && self.consume_punctuation_prefix(">")
    }

    pub(super) fn at_angle_open(&self) -> bool {
        self.at_punctuation("<") || self.at_punctuation("<<")
    }

    pub(super) fn consume_angle_open(&mut self) -> bool {
        if self.at_punctuation("<") {
            return self.consume_punctuation("<");
        }
        self.at_punctuation("<<") && self.consume_punctuation_prefix("<")
    }

    fn advance_current_punctuation(&mut self) {
        self.index += 1;
        self.punctuation_offset = None;
        self.pending_token = None;
    }

    pub(super) fn skip_delimited_group(&mut self) -> Option<Range<usize>> {
        if self.punctuation_offset.is_some() {
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
        let mut cursor = TokenCursor::new(self.tokens, bounds.clone())?;
        if self.punctuation_offset.is_some()
            && bounds.start == self.index
            && !cursor.restore(self.checkpoint())
        {
            return None;
        }
        Some(cursor)
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
            if self.punctuation_offset.is_some() {
                self.advance_current_punctuation();
            } else {
                self.index += 1;
            }
        }
        self.index
    }

    pub(super) fn find_top_level_punctuation(&self, expected: &str) -> Option<usize> {
        let mut angle_depth = 0usize;
        let mut index = self.index;
        if let Some(token) = self.pending_token.as_ref() {
            match token.kind() {
                TokenKind::Punctuation(value) if value == "<" => angle_depth += 1,
                TokenKind::Punctuation(value) if value == "<<" => angle_depth += 2,
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
        while index < self.end {
            if matches!(self.tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                index = self.nested_group_end(index)? + 1;
                continue;
            }
            match self.tokens[index].kind() {
                TokenKind::Punctuation(value) if value == "<" => angle_depth += 1,
                TokenKind::Punctuation(value) if value == "<<" => angle_depth += 2,
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
        let mut angle_depth = 0usize;
        let mut index = self.index;
        if let Some(token) = self.pending_token.as_ref() {
            match token.kind() {
                TokenKind::Punctuation(value) if value == "<" => angle_depth += 1,
                TokenKind::Punctuation(value) if value == "<<" => angle_depth += 2,
                TokenKind::Punctuation(value) if value == ">" => {
                    angle_depth = angle_depth.saturating_sub(1);
                }
                TokenKind::Punctuation(value) if value == ">>" => {
                    angle_depth = angle_depth.saturating_sub(2);
                }
                _ => {}
            }
            index += 1;
        }
        while index < self.end {
            if matches!(self.tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                index = self.nested_group_end(index)? + 1;
                continue;
            }
            match self.tokens[index].kind() {
                TokenKind::Punctuation(value) if value == "<" => angle_depth += 1,
                TokenKind::Punctuation(value) if value == "<<" => angle_depth += 2,
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
    fn recovery_discards_a_partial_angle_before_the_next_synchronization_token() {
        let source = SourceFile::new("src/lib.rs", "<<, kept");
        let lexed = RustLexer::lex(&source);
        let tokens = lexed.tokens();
        let comma = tokens
            .iter()
            .position(|token| matches!(token.kind(), TokenKind::Punctuation(value) if value == ","))
            .expect("逗号同步点");
        let mut cursor = TokenCursor::new(tokens, 0..tokens.len()).expect("有界游标");

        assert!(cursor.consume_angle_open());
        assert_eq!(cursor.recover_to(&[","]), comma);
        assert_eq!(cursor.offset(), tokens[comma].start());
        assert!(cursor.at_punctuation(","));
    }

    #[test]
    fn compound_angle_tokens_split_and_restore_by_character() {
        let source = SourceFile::new("src/lib.rs", "<< >>");
        let lexed = RustLexer::lex(&source);
        let tokens = lexed.tokens();
        let mut cursor = TokenCursor::new(tokens, 0..tokens.len()).expect("有界游标");

        assert!(matches!(
            tokens[0].kind(),
            TokenKind::Punctuation(value) if value == "<<"
        ));
        assert!(cursor.consume_angle_open());
        let checkpoint = cursor.checkpoint();
        assert_eq!(cursor.offset(), 1);
        assert_eq!(cursor.consumed_end_index(), 1);
        assert_eq!(cursor.peek().map(|token| token.text(&source)), Some("<"));
        assert!(!cursor.is_empty());
        assert!(cursor.consume_angle_open());
        assert_eq!(cursor.index(), 1);
        assert!(cursor.restore(checkpoint));
        assert_eq!(cursor.offset(), 1);
        assert_eq!(cursor.peek().map(|token| token.text(&source)), Some("<"));
        assert!(cursor.consume_angle_open());

        assert!(matches!(
            tokens[1].kind(),
            TokenKind::Punctuation(value) if value == ">>"
        ));
        assert!(cursor.consume_angle_close());
        let checkpoint = cursor.checkpoint();
        assert_eq!(cursor.offset(), 4);
        assert_eq!(cursor.consumed_end_index(), 2);
        assert_eq!(cursor.peek().map(|token| token.text(&source)), Some(">"));
        assert!(!cursor.is_empty());
        assert!(cursor.consume_angle_close());
        assert_eq!(cursor.offset(), 5);
        assert!(cursor.is_empty());
        assert!(cursor.restore(checkpoint));
        assert_eq!(cursor.offset(), 4);
        assert_eq!(cursor.peek().map(|token| token.text(&source)), Some(">"));
    }

    #[test]
    fn arbitrary_punctuation_prefixes_preserve_the_remaining_source_range() {
        let source = SourceFile::new("src/lib.rs", "<<=");
        let lexed = RustLexer::lex(&source);
        let tokens = lexed.tokens();
        let mut cursor = TokenCursor::new(tokens, 0..tokens.len()).expect("有界游标");

        assert!(cursor.consume_punctuation_prefix("<<"));
        let checkpoint = cursor.checkpoint();
        assert_eq!(cursor.offset(), 2);
        assert_eq!(cursor.consumed_offset(), 2);
        assert_eq!(cursor.consumed_end_index(), 1);
        assert_eq!(cursor.peek().map(|token| token.text(&source)), Some("="));

        assert!(cursor.consume_punctuation("="));
        assert!(cursor.is_empty());
        assert!(cursor.restore(checkpoint));
        assert_eq!(cursor.offset(), 2);
        assert_eq!(cursor.peek().map(|token| token.text(&source)), Some("="));
    }
}
