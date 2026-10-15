use std::mem;

use cop_model::{
    LocalizedMessage, SYNTAX_EXPECT_LITERAL_OR_LIFETIME, SYNTAX_EXPECT_RADIX_DIGIT,
    SYNTAX_INVALID_BYTE_CHARACTER, SYNTAX_INVALID_RADIX_DIGIT, SYNTAX_UNCLOSED_BLOCK_COMMENT,
    SYNTAX_UNCLOSED_LITERAL, SYNTAX_UNCLOSED_RAW_STRING, SYNTAX_UNKNOWN_CHARACTER,
};

use crate::SourceFile;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Delimiter {
    Parenthesis,
    Bracket,
    Brace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiteralKind {
    Number,
    String,
    ByteString,
    Character,
    Byte,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    Lifetime(String),
    Literal(LiteralKind),
    Punctuation(String),
    OpenDelimiter(Delimiter),
    CloseDelimiter(Delimiter),
    Unknown(char),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TriviaKind {
    Whitespace,
    LineComment,
    BlockComment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Trivia {
    kind: TriviaKind,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
    leading_trivia: Vec<Trivia>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LexIssue {
    start: usize,
    end: usize,
    message: LocalizedMessage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LexOutput {
    tokens: Vec<Token>,
    trailing_trivia: Vec<Trivia>,
    issues: Vec<LexIssue>,
}

pub struct RustLexer;

struct Scanner<'a> {
    source: &'a SourceFile,
    cursor: usize,
    tokens: Vec<Token>,
    pending_trivia: Vec<Trivia>,
    issues: Vec<LexIssue>,
}

impl Trivia {
    pub fn kind(&self) -> TriviaKind {
        self.kind
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }
}

impl Token {
    pub fn kind(&self) -> &TokenKind {
        &self.kind
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn leading_trivia(&self) -> &[Trivia] {
        &self.leading_trivia
    }

    pub fn text<'a>(&self, source: &'a SourceFile) -> &'a str {
        source.slice(self.start, self.end).unwrap_or("")
    }
}

impl LexIssue {
    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn message(&self) -> &LocalizedMessage {
        &self.message
    }
}

impl LexOutput {
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    pub fn trailing_trivia(&self) -> &[Trivia] {
        &self.trailing_trivia
    }

    pub fn issues(&self) -> &[LexIssue] {
        &self.issues
    }
}

impl RustLexer {
    pub fn lex(source: &SourceFile) -> LexOutput {
        Scanner {
            source,
            cursor: 0,
            tokens: Vec::new(),
            pending_trivia: Vec::new(),
            issues: Vec::new(),
        }
        .run()
    }
}

impl Scanner<'_> {
    fn run(mut self) -> LexOutput {
        while self.cursor < self.source.len() {
            self.scan_trivia();
            if self.cursor == self.source.len() {
                break;
            }
            self.scan_token();
        }
        LexOutput {
            tokens: self.tokens,
            trailing_trivia: self.pending_trivia,
            issues: self.issues,
        }
    }

    fn scan_trivia(&mut self) {
        loop {
            let start = self.cursor;
            let Some(character) = self.current_character() else {
                return;
            };
            if character.is_whitespace() {
                self.advance_whitespace();
                self.pending_trivia.push(Trivia {
                    kind: TriviaKind::Whitespace,
                    start,
                    end: self.cursor,
                });
            } else if self.starts_with("//") {
                self.cursor += 2;
                while let Some(character) = self.current_character() {
                    if character == '\n' {
                        break;
                    }
                    self.advance_character();
                }
                self.pending_trivia.push(Trivia {
                    kind: TriviaKind::LineComment,
                    start,
                    end: self.cursor,
                });
            } else if self.starts_with("/*") {
                self.scan_block_comment(start);
            } else {
                return;
            }
        }
    }

    fn scan_block_comment(&mut self, start: usize) {
        self.cursor += 2;
        let mut depth = 1usize;
        while self.cursor < self.source.len() && depth > 0 {
            if self.starts_with("/*") {
                depth += 1;
                self.cursor += 2;
            } else if self.starts_with("*/") {
                depth -= 1;
                self.cursor += 2;
            } else {
                self.advance_character();
            }
        }
        if depth != 0 {
            self.issues.push(LexIssue {
                start,
                end: self.cursor,
                message: LocalizedMessage::new(SYNTAX_UNCLOSED_BLOCK_COMMENT),
            });
        }
        self.pending_trivia.push(Trivia {
            kind: TriviaKind::BlockComment,
            start,
            end: self.cursor,
        });
    }

    fn scan_token(&mut self) {
        let start = self.cursor;
        let character = self.current_character().unwrap_or('\0');
        let kind = if let Some((length, literal_kind)) = self.raw_string_prefix() {
            self.scan_raw_string(length, start, literal_kind)
        } else if let Some((length, literal_kind)) = self.quoted_prefix() {
            self.cursor += length;
            self.scan_quoted(start, literal_kind)
        } else if character == 'r' && self.peek_character(1) == Some('#') {
            self.scan_raw_identifier(start)
        } else if character == 'b' && self.peek_character(1) == Some('\'') {
            self.cursor += 1;
            self.scan_byte_character(start)
        } else if character == '\'' {
            self.scan_lifetime_or_character()
        } else if character.is_ascii_digit() {
            self.scan_number(start)
        } else if is_identifier_start(character) {
            self.scan_identifier(start)
        } else {
            self.scan_symbol()
        };
        if let TokenKind::Unknown(character) = &kind {
            self.issues.push(LexIssue {
                start,
                end: self.cursor,
                message: LocalizedMessage::new(SYNTAX_UNKNOWN_CHARACTER)
                    .with_text("character", character.to_string()),
            });
        }
        self.tokens.push(Token {
            kind,
            start,
            end: self.cursor,
            leading_trivia: mem::take(&mut self.pending_trivia),
        });
    }

    fn raw_string_prefix(&self) -> Option<(usize, LiteralKind)> {
        let remaining = &self.source.text()[self.cursor..];
        for (prefix, kind) in [
            ("br", LiteralKind::ByteString),
            ("cr", LiteralKind::String),
            ("r", LiteralKind::String),
        ] {
            if let Some(after) = remaining.strip_prefix(prefix) {
                let hash_count = after.bytes().take_while(|byte| *byte == b'#').count();
                if after.as_bytes().get(hash_count) == Some(&b'"') {
                    return Some((prefix.len() + hash_count + 1, kind));
                }
            }
        }
        None
    }

    fn quoted_prefix(&self) -> Option<(usize, LiteralKind)> {
        let remaining = &self.source.text()[self.cursor..];
        if remaining.starts_with("b\"") {
            Some((1, LiteralKind::ByteString))
        } else if remaining.starts_with("c\"") {
            Some((1, LiteralKind::String))
        } else if remaining.starts_with('"') {
            Some((0, LiteralKind::String))
        } else {
            None
        }
    }

    fn scan_raw_string(
        &mut self,
        prefix_length: usize,
        start: usize,
        literal_kind: LiteralKind,
    ) -> TokenKind {
        let raw_prefix = &self.source.text()[self.cursor..self.cursor + prefix_length];
        let hash_count = raw_prefix.bytes().filter(|byte| *byte == b'#').count();
        self.cursor += prefix_length;
        let mut terminated = false;
        while self.cursor < self.source.len() {
            if self.current_character() == Some('"') {
                let after_quote = self.cursor + 1;
                let closing_hashes = self.source.text()[after_quote..]
                    .bytes()
                    .take_while(|byte| *byte == b'#')
                    .count();
                if closing_hashes >= hash_count {
                    self.cursor = after_quote + hash_count;
                    terminated = true;
                    break;
                }
            }
            self.advance_character();
        }
        if !terminated {
            self.issues.push(LexIssue {
                start,
                end: self.cursor,
                message: LocalizedMessage::new(SYNTAX_UNCLOSED_RAW_STRING),
            });
        }
        TokenKind::Literal(literal_kind)
    }

    fn scan_quoted(&mut self, start: usize, literal_kind: LiteralKind) -> TokenKind {
        let quote = if matches!(literal_kind, LiteralKind::String | LiteralKind::ByteString) {
            '"'
        } else {
            '\''
        };
        if self.current_character() == Some(quote) {
            self.advance_character();
        }
        let mut terminated = false;
        while let Some(character) = self.current_character() {
            self.advance_character();
            if character == '\\' {
                self.advance_character();
            } else if character == quote {
                terminated = true;
                break;
            } else if character == '\n' && quote == '\'' {
                break;
            }
        }
        if !terminated {
            self.issues.push(LexIssue {
                start,
                end: self.cursor,
                message: LocalizedMessage::new(SYNTAX_UNCLOSED_LITERAL),
            });
        }
        TokenKind::Literal(literal_kind)
    }

    fn scan_lifetime_or_character(&mut self) -> TokenKind {
        let start = self.cursor;
        if let Some(end) = self.character_literal_end(start, false) {
            self.cursor = end;
            return TokenKind::Literal(LiteralKind::Character);
        }
        self.cursor = start + 1;
        while self.current_character().is_some_and(is_identifier_continue) {
            self.advance_character();
        }
        if self.cursor == start + 1 {
            self.advance_character();
            self.issues.push(LexIssue {
                start,
                end: self.cursor,
                message: LocalizedMessage::new(SYNTAX_EXPECT_LITERAL_OR_LIFETIME),
            });
        }
        TokenKind::Lifetime(self.source.text()[start..self.cursor].to_owned())
    }

    fn scan_byte_character(&mut self, start: usize) -> TokenKind {
        if let Some(end) = self.character_literal_end(self.cursor, true) {
            self.cursor = end;
        } else {
            self.advance_character();
            self.issues.push(LexIssue {
                start,
                end: self.cursor,
                message: LocalizedMessage::new(SYNTAX_INVALID_BYTE_CHARACTER),
            });
        }
        TokenKind::Literal(LiteralKind::Byte)
    }

    fn character_literal_end(&self, quote_offset: usize, is_byte: bool) -> Option<usize> {
        let text = self.source.text();
        if text.get(quote_offset..)?.starts_with('\'') {
            let mut cursor = quote_offset + 1;
            let first = text.get(cursor..)?.chars().next()?;
            if first == '\\' {
                cursor += first.len_utf8();
                let escape = text.get(cursor..)?.chars().next()?;
                cursor += escape.len_utf8();
                match escape {
                    'x' => {
                        let mut value = 0u8;
                        for _ in 0..2 {
                            let digit = text.get(cursor..)?.chars().next()?;
                            if !digit.is_ascii_hexdigit() {
                                return None;
                            }
                            value = value
                                .checked_mul(16)?
                                .checked_add(digit.to_digit(16)? as u8)?;
                            cursor += 1;
                        }
                        if !is_byte && !value.is_ascii() {
                            return None;
                        }
                    }
                    'u' => {
                        if is_byte {
                            return None;
                        }
                        if text.as_bytes().get(cursor) != Some(&b'{') {
                            return None;
                        }
                        cursor += 1;
                        let mut digits = String::new();
                        while let Some(character) = text.get(cursor..)?.chars().next() {
                            if character == '}' {
                                cursor += 1;
                                break;
                            }
                            if character == '_' {
                                cursor += 1;
                                continue;
                            }
                            if !character.is_ascii_hexdigit() || digits.len() == 6 {
                                return None;
                            }
                            digits.push(character);
                            cursor += character.len_utf8();
                        }
                        if digits.is_empty() {
                            return None;
                        }
                        let value = u32::from_str_radix(&digits, 16).ok()?;
                        char::from_u32(value)?;
                    }
                    'n' | 'r' | 't' | '0' | '\\' | '\'' | '"' => {}
                    _ => return None,
                }
            } else {
                if first == '\n' || first == '\r' || (is_byte && !first.is_ascii()) {
                    return None;
                }
                cursor += first.len_utf8();
            }
            if text.as_bytes().get(cursor) == Some(&b'\'') {
                return Some(cursor + 1);
            }
        }
        None
    }

    fn scan_number(&mut self, start: usize) -> TokenKind {
        let radix = match self.source.text().as_bytes().get(self.cursor + 1) {
            Some(b'x' | b'X') => Some(16),
            Some(b'o' | b'O') => Some(8),
            Some(b'b' | b'B') => Some(2),
            _ => None,
        };
        if let Some(radix) = radix {
            self.cursor += 2;
            let digits_start = self.cursor;
            let mut has_digit = false;
            let mut has_invalid_digit = false;
            while let Some(character) = self.current_character() {
                if character == '_' {
                    self.advance_character();
                } else if character.is_ascii_hexdigit() {
                    has_digit |= character.is_digit(radix);
                    has_invalid_digit |= character.is_ascii_digit() && !character.is_digit(radix);
                    self.advance_character();
                } else {
                    break;
                }
            }
            if !has_digit {
                self.issues.push(LexIssue {
                    start,
                    end: self.cursor,
                    message: LocalizedMessage::new(SYNTAX_EXPECT_RADIX_DIGIT),
                });
            }
            if has_invalid_digit {
                self.issues.push(LexIssue {
                    start: digits_start,
                    end: self.cursor,
                    message: LocalizedMessage::new(SYNTAX_INVALID_RADIX_DIGIT)
                        .with_number("radix", radix as i64),
                });
            }
            self.consume_numeric_suffix();
            return TokenKind::Literal(LiteralKind::Number);
        }

        self.consume_decimal_digits();
        if self.current_character() == Some('.') && self.peek_character(1) != Some('.') {
            let next = self.peek_character(1);
            let has_float_suffix = self.has_float_suffix_after_dot();
            if next.is_some_and(|character| {
                character.is_ascii_digit() || character == 'e' || character == 'E'
            }) || next.is_none()
                || next.is_some_and(|character| !is_identifier_start(character))
                || has_float_suffix
            {
                self.advance_character();
                self.consume_decimal_digits();
            }
        }
        if matches!(self.current_character(), Some('e' | 'E')) {
            let exponent_start = self.cursor;
            self.advance_character();
            if matches!(self.current_character(), Some('+' | '-')) {
                self.advance_character();
            }
            if self
                .current_character()
                .is_some_and(|character| character.is_ascii_digit())
            {
                self.consume_decimal_digits();
            } else {
                self.cursor = exponent_start;
            }
        }
        self.consume_numeric_suffix();
        TokenKind::Literal(LiteralKind::Number)
    }

    fn consume_decimal_digits(&mut self) {
        while self
            .current_character()
            .is_some_and(|character| character.is_ascii_digit() || character == '_')
        {
            self.advance_character();
        }
    }

    fn consume_numeric_suffix(&mut self) {
        while self
            .current_character()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            self.advance_character();
        }
    }

    fn has_float_suffix_after_dot(&self) -> bool {
        let remaining = &self.source.text()[self.cursor + 1..];
        ["f32", "f64"].iter().any(|suffix| {
            remaining.strip_prefix(suffix).is_some_and(|rest| {
                rest.chars()
                    .next()
                    .map_or(true, |character| !is_identifier_continue(character))
            })
        })
    }

    fn scan_identifier(&mut self, start: usize) -> TokenKind {
        self.advance_character();
        while self.current_character().is_some_and(is_identifier_continue) {
            self.advance_character();
        }
        TokenKind::Identifier(self.source.text()[start..self.cursor].to_owned())
    }

    fn scan_raw_identifier(&mut self, start: usize) -> TokenKind {
        self.cursor += 2;
        if self.current_character().is_some_and(is_identifier_start) {
            self.advance_character();
            while self.current_character().is_some_and(is_identifier_continue) {
                self.advance_character();
            }
            TokenKind::Identifier(self.source.text()[start..self.cursor].to_owned())
        } else {
            TokenKind::Punctuation("r#".to_owned())
        }
    }

    fn scan_symbol(&mut self) -> TokenKind {
        let character = self.current_character().unwrap_or('\0');
        let delimiter = match character {
            '(' => Some((true, Delimiter::Parenthesis)),
            '[' => Some((true, Delimiter::Bracket)),
            '{' => Some((true, Delimiter::Brace)),
            ')' => Some((false, Delimiter::Parenthesis)),
            ']' => Some((false, Delimiter::Bracket)),
            '}' => Some((false, Delimiter::Brace)),
            _ => None,
        };
        if let Some((is_open, delimiter)) = delimiter {
            self.advance_character();
            return if is_open {
                TokenKind::OpenDelimiter(delimiter)
            } else {
                TokenKind::CloseDelimiter(delimiter)
            };
        }
        const PUNCTUATION: &[&str] = &[
            "<<=", ">>=", "...", "..=", "::", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "+=",
            "-=", "*=", "/=", "%=", "^=", "&=", "|=", "<<", ">>", "..", "#", "$", "!",
        ];
        let remaining = &self.source.text()[self.cursor..];
        if let Some(punctuation) = PUNCTUATION
            .iter()
            .find(|punctuation| remaining.starts_with(**punctuation))
        {
            self.cursor += punctuation.len();
            return TokenKind::Punctuation((*punctuation).to_owned());
        }
        if ";,:.=+-*/%&|^!<>?@~".contains(character) {
            self.advance_character();
            TokenKind::Punctuation(character.to_string())
        } else {
            self.advance_character();
            TokenKind::Unknown(character)
        }
    }

    fn starts_with(&self, value: &str) -> bool {
        self.source.text()[self.cursor..].starts_with(value)
    }

    fn current_character(&self) -> Option<char> {
        self.source.text().get(self.cursor..)?.chars().next()
    }

    fn peek_character(&self, distance: usize) -> Option<char> {
        self.source.text().get(self.cursor..)?.chars().nth(distance)
    }

    fn advance_whitespace(&mut self) {
        while self.current_character().is_some_and(char::is_whitespace) {
            self.advance_character();
        }
    }

    fn advance_character(&mut self) {
        if let Some(character) = self.current_character() {
            self.cursor += character.len_utf8();
        }
    }
}

fn is_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

fn is_identifier_continue(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}
