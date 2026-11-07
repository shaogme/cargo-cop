use std::ops::Range;

pub(super) use crate::recovery::AngleScanContext;
use crate::{Delimiter, Token, TokenKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CursorPosition {
    token_index: usize,
    punctuation_offset: Option<usize>,
    byte_offset: usize,
}

impl CursorPosition {
    pub(super) fn token_index(self) -> usize {
        self.token_index
    }

    pub(super) fn byte_offset(self) -> usize {
        self.byte_offset
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CursorRange {
    start: CursorPosition,
    end: CursorPosition,
}

impl CursorRange {
    pub(super) fn new(start: CursorPosition, end: CursorPosition) -> Option<Self> {
        (position_key(start) <= position_key(end)).then_some(Self { start, end })
    }

    pub(super) fn from_token_range(tokens: &[Token], bounds: Range<usize>) -> Option<Self> {
        if bounds.start > bounds.end || bounds.end > tokens.len() {
            return None;
        }
        let fallback = tokens.last().map_or(0, Token::end);
        let start = tokens.get(bounds.start).map_or(fallback, Token::start);
        let end = bounds
            .end
            .checked_sub(1)
            .and_then(|index| tokens.get(index))
            .map_or(start, Token::end);
        Self::new(
            CursorPosition {
                token_index: bounds.start,
                punctuation_offset: None,
                byte_offset: start,
            },
            CursorPosition {
                token_index: bounds.end,
                punctuation_offset: None,
                byte_offset: end,
            },
        )
    }

    pub(super) fn byte_bounds(self) -> Range<usize> {
        self.start.byte_offset()..self.end.byte_offset().max(self.start.byte_offset())
    }

    pub(super) fn start(self) -> CursorPosition {
        self.start
    }

    pub(super) fn end(self) -> CursorPosition {
        self.end
    }
}

fn position_key(position: CursorPosition) -> (usize, usize) {
    (
        position.token_index,
        position.punctuation_offset.unwrap_or(0),
    )
}

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
    range_end: Option<CursorPosition>,
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

pub(super) struct AngleListScanError {
    completed: Vec<CursorRange>,
    remainder: Option<CursorRange>,
    recoverable_errors: Vec<SyntaxError>,
    error: SyntaxError,
}

impl AngleListScanError {
    pub(super) fn completed(&self) -> &[CursorRange] {
        &self.completed
    }

    pub(super) fn remainder(&self) -> Option<CursorRange> {
        self.remainder
    }

    pub(super) fn recoverable_errors(&self) -> &[SyntaxError] {
        &self.recoverable_errors
    }

    pub(super) fn error(&self) -> SyntaxError {
        self.error.clone()
    }
}

pub(super) struct AngleListScan {
    parameters: Vec<CursorRange>,
    errors: Vec<SyntaxError>,
}

impl AngleListScan {
    pub(super) fn parameters(&self) -> &[CursorRange] {
        &self.parameters
    }

    pub(super) fn errors(&self) -> &[SyntaxError] {
        &self.errors
    }
}

impl<'a> TokenCursor<'a> {
    pub(super) fn from_token_range(tokens: &'a [Token], bounds: Range<usize>) -> Option<Self> {
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
            range_end: None,
        })
    }

    pub(super) fn new(tokens: &'a [Token], range: CursorRange) -> Option<Self> {
        let end = range.end.token_index + usize::from(range.end.punctuation_offset.is_some());
        let mut cursor = Self::from_token_range(tokens, range.start.token_index..end)?;
        cursor.range_end = Some(range.end);
        if range.start.punctuation_offset.is_some() {
            let checkpoint = CursorCheckpoint {
                index: range.start.token_index,
                punctuation_offset: range.start.punctuation_offset,
            };
            if !cursor.restore(checkpoint) {
                return None;
            }
        }
        Some(cursor)
    }

    pub(super) fn position(&self) -> CursorPosition {
        CursorPosition {
            token_index: self.index,
            punctuation_offset: self.punctuation_offset,
            byte_offset: self.offset(),
        }
    }

    pub(super) fn consumed_position(&self) -> CursorPosition {
        CursorPosition {
            token_index: self.consumed_end_index(),
            punctuation_offset: None,
            byte_offset: self.consumed_offset(),
        }
    }

    pub(super) fn range(&self, start: CursorPosition, end: CursorPosition) -> Option<CursorRange> {
        CursorRange::new(start, end)
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
        if let Some(end) = self.range_end {
            return position_key(self.position()) >= position_key(end);
        }
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
        if let Some(end) = self.range_end {
            return end.byte_offset;
        }
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

    pub(super) fn split_angle_list(&mut self) -> Result<AngleListScan, Box<AngleListScanError>> {
        if !self.consume_angle_open() {
            return Err(Box::new(AngleListScanError {
                completed: Vec::new(),
                remainder: None,
                recoverable_errors: Vec::new(),
                error: SyntaxError::new(
                    SyntaxErrorKind::UnexpectedToken,
                    self.offset()..self.offset(),
                ),
            }));
        }
        let mut parameters = Vec::new();
        let mut errors = Vec::new();
        let mut item_start = self.position();
        let mut nested_angles = 0usize;
        while !self.is_empty() {
            if matches!(
                self.peek().map(Token::kind),
                Some(TokenKind::OpenDelimiter(_))
            ) {
                if self.skip_delimited_group().is_none() {
                    return Err(self.angle_list_error(
                        AngleListScan { parameters, errors },
                        item_start,
                        SyntaxErrorKind::UnclosedDelimiter,
                        self.offset()..self.offset(),
                    ));
                }
                continue;
            }
            if self.at_punctuation(";") && nested_angles == 0 {
                let token = self.peek().expect("当前 token 存在");
                return Err(self.angle_list_error(
                    AngleListScan { parameters, errors },
                    item_start,
                    SyntaxErrorKind::UnexpectedToken,
                    token.start()..token.end(),
                ));
            }
            if self.at_punctuation(",") && nested_angles == 0 {
                let separator = self.position();
                if position_key(item_start) < position_key(separator) {
                    parameters.push(self.range(item_start, separator).expect("有序范围"));
                } else {
                    let token = self.peek().expect("逗号 token 存在");
                    errors.push(SyntaxError::new(
                        SyntaxErrorKind::UnexpectedToken,
                        token.start()..token.end(),
                    ));
                }
                self.consume_punctuation(",");
                item_start = self.position();
                continue;
            }
            if self.at_angle_open() {
                self.consume_angle_open();
                let Some(depth) = nested_angles.checked_add(1) else {
                    return Err(self.angle_list_error(
                        AngleListScan { parameters, errors },
                        item_start,
                        SyntaxErrorKind::UnexpectedToken,
                        self.consumed_offset()..self.consumed_offset(),
                    ));
                };
                nested_angles = depth;
                continue;
            }
            if self.at_punctuation(">") || self.at_punctuation(">>") {
                if nested_angles == 0 {
                    let close = self.position();
                    if position_key(item_start) < position_key(close) {
                        parameters.push(self.range(item_start, close).expect("有序范围"));
                    } else if parameters.is_empty() && errors.is_empty() {
                        let token = self.peek().expect("闭合尖括号存在");
                        errors.push(SyntaxError::new(
                            SyntaxErrorKind::UnexpectedToken,
                            token.start()..token.end(),
                        ));
                    }
                    self.consume_angle_close();
                    return Ok(AngleListScan { parameters, errors });
                }
                self.consume_angle_close();
                let Some(depth) = nested_angles.checked_sub(1) else {
                    return Err(self.angle_list_error(
                        AngleListScan { parameters, errors },
                        item_start,
                        SyntaxErrorKind::UnexpectedToken,
                        self.consumed_offset()..self.consumed_offset(),
                    ));
                };
                nested_angles = depth;
                continue;
            }
            self.consume();
        }
        Err(self.angle_list_error(
            AngleListScan { parameters, errors },
            item_start,
            SyntaxErrorKind::UnexpectedEof,
            self.offset()..self.offset(),
        ))
    }

    pub(super) fn split_top_level(
        &mut self,
        separator: &str,
        context: AngleScanContext,
    ) -> Result<Vec<CursorRange>, SyntaxError> {
        let mut ranges = Vec::new();
        let mut item_start = self.position();
        let mut nested_angles = 0usize;
        while !self.is_empty() {
            if matches!(
                self.peek().map(Token::kind),
                Some(TokenKind::OpenDelimiter(_))
            ) {
                if self.skip_delimited_group().is_none() {
                    return Err(SyntaxError::new(
                        SyntaxErrorKind::UnclosedDelimiter,
                        self.offset()..self.offset(),
                    ));
                }
                continue;
            }
            if self.at_punctuation(separator) && nested_angles == 0 {
                let separator_position = self.position();
                if position_key(item_start) < position_key(separator_position) {
                    ranges.push(
                        self.range(item_start, separator_position)
                            .expect("有序范围"),
                    );
                }
                self.consume_punctuation(separator);
                item_start = self.position();
                continue;
            }
            if !matches!(
                context,
                AngleScanContext::Expression | AngleScanContext::TokenSequence
            ) && self.at_angle_open()
            {
                self.consume_angle_open();
                nested_angles = nested_angles.checked_add(1).ok_or_else(|| {
                    SyntaxError::new(
                        SyntaxErrorKind::UnexpectedToken,
                        self.consumed_offset()..self.consumed_offset(),
                    )
                })?;
                continue;
            }
            if !matches!(
                context,
                AngleScanContext::Expression | AngleScanContext::TokenSequence
            ) && (self.at_punctuation(">") || self.at_punctuation(">>"))
            {
                let Some(depth) = nested_angles.checked_sub(1) else {
                    let token = self.peek().expect("当前 token 存在");
                    return Err(SyntaxError::new(
                        SyntaxErrorKind::UnexpectedToken,
                        token.start()..token.end(),
                    ));
                };
                self.consume_angle_close();
                nested_angles = depth;
                continue;
            }
            self.consume();
        }
        if nested_angles != 0 {
            return Err(SyntaxError::new(
                SyntaxErrorKind::UnexpectedEof,
                self.offset()..self.offset(),
            ));
        }
        let end = self.consumed_position();
        if position_key(item_start) < position_key(end) {
            ranges.push(self.range(item_start, end).expect("有序范围"));
        }
        Ok(ranges)
    }

    fn angle_list_error(
        &self,
        scan: AngleListScan,
        item_start: CursorPosition,
        kind: SyntaxErrorKind,
        bounds: Range<usize>,
    ) -> Box<AngleListScanError> {
        let end = self.consumed_position();
        let remainder = (position_key(item_start) < position_key(end))
            .then(|| CursorRange::new(item_start, end).expect("有序参数范围"));
        Box::new(AngleListScanError {
            completed: scan.parameters,
            remainder,
            recoverable_errors: scan.errors,
            error: SyntaxError::new(kind, bounds),
        })
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

    pub(super) fn subcursor(&self, bounds: CursorRange) -> Option<TokenCursor<'a>> {
        let self_start = CursorPosition {
            token_index: self.start,
            punctuation_offset: None,
            byte_offset: self.tokens.get(self.start).map_or(0, Token::start),
        };
        let self_end = self.range_end.unwrap_or(CursorPosition {
            token_index: self.end,
            punctuation_offset: None,
            byte_offset: self.end_offset(),
        });
        if position_key(bounds.start) < position_key(self_start)
            || position_key(bounds.end) > position_key(self_end)
        {
            return None;
        }
        Self::new(self.tokens, bounds)
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

    pub(super) fn find_top_level_punctuation(
        &self,
        expected: &str,
        context: AngleScanContext,
    ) -> Option<usize> {
        self.find_top_level(
            context,
            |kind| matches!(kind, TokenKind::Punctuation(value) if value == expected),
        )
    }

    pub(super) fn find_top_level_identifier(
        &self,
        expected: &str,
        context: AngleScanContext,
    ) -> Option<usize> {
        self.find_top_level(
            context,
            |kind| matches!(kind, TokenKind::Identifier(value) if value == expected),
        )
    }

    pub(super) fn find_top_level_open_delimiter(
        &self,
        expected: Delimiter,
        context: AngleScanContext,
    ) -> Option<usize> {
        self.find_top_level(
            context,
            |kind| matches!(kind, TokenKind::OpenDelimiter(found) if *found == expected),
        )
    }

    fn find_top_level(
        &self,
        context: AngleScanContext,
        matches_expected: impl Fn(&TokenKind) -> bool,
    ) -> Option<usize> {
        let mut angle_depth = 0usize;
        let mut index = self.index;
        if let Some(token) = self.pending_token.as_ref() {
            if matches_expected(token.kind()) && angle_depth == 0 {
                return Some(index);
            }
            Self::update_angle_depth(token.kind(), context, &mut angle_depth)?;
            index += 1;
        }
        while index < self.end {
            let kind = self.tokens[index].kind();
            if angle_depth == 0 && matches_expected(kind) {
                return Some(index);
            }
            if matches!(self.tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                index = self.nested_group_end(index)? + 1;
                continue;
            }
            Self::update_angle_depth(kind, context, &mut angle_depth)?;
            index += 1;
        }
        None
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
        let lexed = RustLexer::lex(&source).expect("catalog messages render");
        let tokens = lexed.tokens();
        let open = tokens
            .iter()
            .position(|token| matches!(token.kind(), TokenKind::OpenDelimiter(_)))
            .expect("分隔符起点");
        let close = tokens
            .iter()
            .rposition(|token| matches!(token.kind(), TokenKind::CloseDelimiter(_)))
            .expect("分隔符终点");
        let mut bounded = TokenCursor::from_token_range(tokens, open..close + 1).expect("有界游标");
        let checkpoint = bounded.checkpoint();

        assert_eq!(bounded.skip_delimited_group(), Some(open..close + 1));
        assert!(bounded.is_empty());
        assert!(bounded.restore(checkpoint));
        assert_eq!(bounded.index(), open);

        let mut truncated = TokenCursor::from_token_range(tokens, open..close).expect("截断子范围");
        assert_eq!(truncated.skip_delimited_group(), None);
        assert_eq!(truncated.index(), open);
    }

    #[test]
    fn recovery_stops_at_synchronization_tokens_without_crossing_its_bound() {
        let source = SourceFile::new("src/lib.rs", "broken, kept");
        let lexed = RustLexer::lex(&source).expect("catalog messages render");
        let tokens = lexed.tokens();
        let comma = tokens
            .iter()
            .position(|token| matches!(token.kind(), TokenKind::Punctuation(value) if value == ","))
            .expect("逗号同步点");
        let mut cursor = TokenCursor::from_token_range(tokens, 0..comma + 1).expect("有界游标");

        assert_eq!(cursor.recover_to(&[","]), comma);
        assert_eq!(cursor.offset(), tokens[comma].start());
    }

    #[test]
    fn recovery_discards_a_partial_angle_before_the_next_synchronization_token() {
        let source = SourceFile::new("src/lib.rs", "<<, kept");
        let lexed = RustLexer::lex(&source).expect("catalog messages render");
        let tokens = lexed.tokens();
        let comma = tokens
            .iter()
            .position(|token| matches!(token.kind(), TokenKind::Punctuation(value) if value == ","))
            .expect("逗号同步点");
        let mut cursor = TokenCursor::from_token_range(tokens, 0..tokens.len()).expect("有界游标");

        assert!(cursor.consume_angle_open());
        assert_eq!(cursor.recover_to(&[","]), comma);
        assert_eq!(cursor.offset(), tokens[comma].start());
        assert!(cursor.at_punctuation(","));
    }

    #[test]
    fn compound_angle_tokens_split_and_restore_by_character() {
        let source = SourceFile::new("src/lib.rs", "<< >>");
        let lexed = RustLexer::lex(&source).expect("catalog messages render");
        let tokens = lexed.tokens();
        let mut cursor = TokenCursor::from_token_range(tokens, 0..tokens.len()).expect("有界游标");

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
        let lexed = RustLexer::lex(&source).expect("catalog messages render");
        let tokens = lexed.tokens();
        let mut cursor = TokenCursor::from_token_range(tokens, 0..tokens.len()).expect("有界游标");

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
