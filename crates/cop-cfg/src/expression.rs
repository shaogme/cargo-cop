use std::ops::Range;

use crate::CfgDecision;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CfgExpression {
    Flag {
        name: String,
        range: Range<usize>,
    },
    KeyValue {
        name: String,
        value: String,
        range: Range<usize>,
    },
    All {
        expressions: Vec<CfgExpression>,
        range: Range<usize>,
    },
    Any {
        expressions: Vec<CfgExpression>,
        range: Range<usize>,
    },
    Not {
        expression: Box<CfgExpression>,
        range: Range<usize>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CfgCondition {
    Constant(bool),
    Expression(CfgExpression),
    All(Vec<CfgCondition>),
    Any(Vec<CfgCondition>),
    Not(Box<CfgCondition>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CfgParseError {
    offset: usize,
}

struct ExpressionParser<'a> {
    source: &'a str,
    cursor: usize,
    base_offset: usize,
}

impl CfgExpression {
    pub fn parse(source: &str) -> Result<Self, CfgParseError> {
        let leading = source.len() - source.trim_start().len();
        let trimmed = source.trim();
        let (expression, base_offset) = match trimmed
            .strip_prefix("cfg(")
            .and_then(|value| value.strip_suffix(')'))
        {
            Some(expression) => (expression, leading + 4),
            None => (trimmed, leading),
        };
        let mut parser = ExpressionParser {
            source: expression,
            cursor: 0,
            base_offset,
        };
        let parsed = parser.parse_expression()?;
        parser.skip_space()?;
        if parser.cursor != parser.source.len() {
            return Err(parser.error());
        }
        Ok(parsed)
    }

    pub fn source_range(&self) -> Range<usize> {
        match self {
            Self::Flag { range, .. }
            | Self::KeyValue { range, .. }
            | Self::All { range, .. }
            | Self::Any { range, .. }
            | Self::Not { range, .. } => range.clone(),
        }
    }

    pub fn normalized(&self) -> String {
        match self {
            Self::Flag { name, .. } => name.clone(),
            Self::KeyValue { name, value, .. } => {
                format!("{name} = \"{}\"", escape_string(value))
            }
            Self::All { expressions, .. } => format_list("all", expressions),
            Self::Any { expressions, .. } => format_list("any", expressions),
            Self::Not { expression, .. } => format!("not({})", expression.normalized()),
        }
    }
}

impl CfgCondition {
    pub fn parse(source: &str) -> Result<Self, CfgParseError> {
        CfgExpression::parse(source).map(Self::from_expression)
    }

    pub fn from_expression(expression: CfgExpression) -> Self {
        match expression {
            CfgExpression::All { expressions, .. } => {
                Self::all(expressions.into_iter().map(Self::from_expression))
            }
            CfgExpression::Any { expressions, .. } => {
                Self::any(expressions.into_iter().map(Self::from_expression))
            }
            CfgExpression::Not { expression, .. } => Self::from_expression(*expression).negate(),
            expression => Self::Expression(expression),
        }
    }

    pub fn all(conditions: impl IntoIterator<Item = Self>) -> Self {
        Self::combine(conditions, true)
    }

    pub fn any(conditions: impl IntoIterator<Item = Self>) -> Self {
        Self::combine(conditions, false)
    }

    pub fn negate(self) -> Self {
        match self {
            Self::Constant(value) => Self::Constant(!value),
            Self::Not(condition) => *condition,
            condition => Self::Not(Box::new(condition)),
        }
    }

    pub fn normalized(&self) -> String {
        match self {
            Self::Constant(true) => "all()".to_owned(),
            Self::Constant(false) => "any()".to_owned(),
            Self::Expression(expression) => expression.normalized(),
            Self::All(conditions) => format_condition_list("all", conditions),
            Self::Any(conditions) => format_condition_list("any", conditions),
            Self::Not(condition) => format!("not({})", condition.normalized()),
        }
    }

    pub fn evaluate<F>(&self, evaluate_expression: &F) -> CfgDecision
    where
        F: Fn(&CfgExpression) -> CfgDecision,
    {
        match self {
            Self::Constant(true) => CfgDecision::True,
            Self::Constant(false) => CfgDecision::False,
            Self::Expression(expression) => evaluate_expression(expression),
            Self::All(conditions) => conditions
                .iter()
                .map(|condition| condition.evaluate(evaluate_expression))
                .fold(CfgDecision::True, CfgDecision::and),
            Self::Any(conditions) => conditions
                .iter()
                .map(|condition| condition.evaluate(evaluate_expression))
                .fold(CfgDecision::False, CfgDecision::or),
            Self::Not(condition) => condition.evaluate(evaluate_expression).negate(),
        }
    }

    pub fn may_coexist(left: &Self, right: &Self) -> bool {
        !matches!(
            Self::all([left.clone(), right.clone()]),
            Self::Constant(false)
        )
    }

    fn combine(conditions: impl IntoIterator<Item = Self>, conjunction: bool) -> Self {
        let mut flattened = Vec::new();
        for condition in conditions {
            match (conjunction, condition) {
                (true, Self::Constant(false)) | (false, Self::Constant(true)) => {
                    return Self::Constant(!conjunction);
                }
                (true, Self::Constant(true)) | (false, Self::Constant(false)) => {}
                (true, Self::All(nested)) | (false, Self::Any(nested)) => {
                    flattened.extend(nested);
                }
                (_, condition) => flattened.push(condition),
            }
        }
        flattened.sort_by_key(Self::normalized);
        flattened.dedup_by(|left, right| left.normalized() == right.normalized());
        if flattened.iter().any(|condition| {
            flattened
                .iter()
                .any(|other| is_negation_pair(condition, other))
        }) {
            return Self::Constant(!conjunction);
        }
        match flattened.len() {
            0 => Self::Constant(conjunction),
            1 => flattened.pop().unwrap_or(Self::Constant(conjunction)),
            _ if conjunction => Self::All(flattened),
            _ => Self::Any(flattened),
        }
    }
}

fn is_negation_pair(left: &CfgCondition, right: &CfgCondition) -> bool {
    matches!(left, CfgCondition::Not(condition) if condition.normalized() == right.normalized())
        || matches!(right, CfgCondition::Not(condition) if condition.normalized() == left.normalized())
}

fn format_condition_list(name: &str, conditions: &[CfgCondition]) -> String {
    let values = conditions
        .iter()
        .map(CfgCondition::normalized)
        .collect::<Vec<_>>()
        .join(", ");
    format!("{name}({values})")
}

impl CfgParseError {
    pub fn offset(self) -> usize {
        self.offset
    }
}

impl ExpressionParser<'_> {
    fn parse_expression(&mut self) -> Result<CfgExpression, CfgParseError> {
        self.skip_space()?;
        let start = self.cursor;
        let name = self.parse_identifier()?;
        self.skip_space()?;
        if self.consume(b'(') {
            let expressions = self.parse_arguments()?;
            let range = self.absolute_range(start..self.cursor);
            return match (name.as_str(), expressions.len()) {
                ("all", _) => Ok(CfgExpression::All { expressions, range }),
                ("any", _) => Ok(CfgExpression::Any { expressions, range }),
                ("not", 1) => Ok(CfgExpression::Not {
                    expression: Box::new(expressions.into_iter().next().ok_or(self.error())?),
                    range,
                }),
                _ => Err(self.error()),
            };
        }
        if self.consume(b'=') {
            self.skip_space()?;
            let value = self.parse_string()?;
            return Ok(CfgExpression::KeyValue {
                name,
                value,
                range: self.absolute_range(start..self.cursor),
            });
        }
        Ok(CfgExpression::Flag {
            name,
            range: self.absolute_range(start..self.cursor),
        })
    }

    fn parse_arguments(&mut self) -> Result<Vec<CfgExpression>, CfgParseError> {
        let mut expressions = Vec::new();
        self.skip_space()?;
        if self.consume(b')') {
            return Ok(expressions);
        }
        loop {
            expressions.push(self.parse_expression()?);
            self.skip_space()?;
            if self.consume(b')') {
                return Ok(expressions);
            }
            if !self.consume(b',') {
                return Err(self.error());
            }
            self.skip_space()?;
            if self.consume(b')') {
                return Ok(expressions);
            }
        }
    }

    fn parse_identifier(&mut self) -> Result<String, CfgParseError> {
        let start = self.cursor;
        while self
            .source
            .as_bytes()
            .get(self.cursor)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-')
        {
            self.cursor += 1;
        }
        if start == self.cursor {
            return Err(self.error());
        }
        Ok(self.source[start..self.cursor].to_owned())
    }

    fn parse_string(&mut self) -> Result<String, CfgParseError> {
        if self.source.as_bytes().get(self.cursor) == Some(&b'r') {
            return self.parse_raw_string();
        }
        let Some(quote) = self.source.as_bytes().get(self.cursor).copied() else {
            return Err(self.error());
        };
        if quote != b'"' {
            return Err(self.error());
        }
        self.cursor += 1;
        let mut value = String::new();
        loop {
            match self.next_character() {
                Some('"') => return Ok(value),
                Some('\\') => self.parse_escape(&mut value)?,
                Some('\n' | '\r') | None => return Err(self.error()),
                Some(character) => value.push(character),
            }
        }
    }

    fn parse_raw_string(&mut self) -> Result<String, CfgParseError> {
        self.cursor += 1;
        let hash_start = self.cursor;
        while self.source.as_bytes().get(self.cursor) == Some(&b'#') {
            self.cursor += 1;
        }
        if !self.consume(b'"') {
            return Err(self.error());
        }
        let content_start = self.cursor;
        let closing = format!("\"{}", "#".repeat(self.cursor - hash_start - 1));
        let Some(relative_end) = self.source[self.cursor..].find(&closing) else {
            return Err(self.error());
        };
        let content_end = self.cursor + relative_end;
        let value = self.source[content_start..content_end].to_owned();
        self.cursor = content_end + closing.len();
        Ok(value)
    }

    fn parse_escape(&mut self, output: &mut String) -> Result<(), CfgParseError> {
        match self.next_character() {
            Some('"') => output.push('"'),
            Some('\\') => output.push('\\'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('t') => output.push('\t'),
            Some('0') => output.push('\0'),
            Some('x') => {
                let first = self.next_character().ok_or(self.error())?;
                let second = self.next_character().ok_or(self.error())?;
                if !first.is_ascii_hexdigit() || !second.is_ascii_hexdigit() {
                    return Err(self.error());
                }
                let digits = [first, second].iter().collect::<String>();
                let byte = u8::from_str_radix(&digits, 16).map_err(|_| self.error())?;
                if !byte.is_ascii() {
                    return Err(self.error());
                }
                output.push(char::from(byte));
            }
            Some('u') => self.parse_unicode_escape(output)?,
            Some('\n') => self.skip_continuation_whitespace(),
            Some('\r') if self.consume(b'\n') => self.skip_continuation_whitespace(),
            _ => return Err(self.error()),
        }
        Ok(())
    }

    fn parse_unicode_escape(&mut self, output: &mut String) -> Result<(), CfgParseError> {
        if !self.consume(b'{') {
            return Err(self.error());
        }
        let mut digits = String::new();
        loop {
            match self.next_character() {
                Some('}') if !digits.is_empty() && digits.len() <= 6 => {
                    let value = u32::from_str_radix(&digits, 16).map_err(|_| self.error())?;
                    output.push(char::from_u32(value).ok_or(self.error())?);
                    return Ok(());
                }
                Some('_') => {}
                Some(character) if character.is_ascii_hexdigit() => digits.push(character),
                _ => return Err(self.error()),
            }
            if digits.len() > 6 {
                return Err(self.error());
            }
        }
    }

    fn skip_space(&mut self) -> Result<(), CfgParseError> {
        loop {
            while self
                .source
                .as_bytes()
                .get(self.cursor)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.cursor += 1;
            }
            if self.source[self.cursor..].starts_with("//") {
                while self
                    .source
                    .as_bytes()
                    .get(self.cursor)
                    .is_some_and(|byte| *byte != b'\n')
                {
                    self.cursor += 1;
                }
                continue;
            }
            if self.source[self.cursor..].starts_with("/*") {
                self.skip_block_comment()?;
                continue;
            }
            return Ok(());
        }
    }

    fn skip_block_comment(&mut self) -> Result<(), CfgParseError> {
        self.cursor += 2;
        let mut depth = 1usize;
        while depth > 0 {
            if self.source[self.cursor..].starts_with("/*") {
                self.cursor += 2;
                depth += 1;
            } else if self.source[self.cursor..].starts_with("*/") {
                self.cursor += 2;
                depth -= 1;
            } else if self.next_character().is_none() {
                return Err(self.error());
            }
        }
        Ok(())
    }

    fn skip_continuation_whitespace(&mut self) {
        while self
            .source
            .get(self.cursor..)
            .and_then(|remaining| remaining.chars().next())
            .is_some_and(char::is_whitespace)
        {
            if let Some(character) = self.next_character() {
                let _ = character;
            }
        }
    }

    fn next_character(&mut self) -> Option<char> {
        let character = self.source.get(self.cursor..)?.chars().next()?;
        self.cursor += character.len_utf8();
        Some(character)
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.source.as_bytes().get(self.cursor) == Some(&expected) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn absolute_range(&self, range: Range<usize>) -> Range<usize> {
        self.base_offset + range.start..self.base_offset + range.end
    }

    fn error(&self) -> CfgParseError {
        CfgParseError {
            offset: self.base_offset + self.cursor,
        }
    }
}

fn format_list(name: &str, expressions: &[CfgExpression]) -> String {
    let values = expressions
        .iter()
        .map(CfgExpression::normalized)
        .collect::<Vec<_>>()
        .join(", ");
    format!("{name}({values})")
}

fn escape_string(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\0' => escaped.push_str("\\0"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{{{:x}}}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{CfgCondition, CfgExpression};

    #[test]
    fn parses_nested_operators_comments_and_empty_identities() {
        let source = "cfg(all(/* outer /* nested */ comment */ unix, any(), not(windows)))";
        let expression = CfgExpression::parse(source).expect("解析表达式");

        assert_eq!(expression.normalized(), "all(unix, any(), not(windows))");
        assert_eq!(expression.source_range(), 4..source.len() - 1);
    }

    #[test]
    fn decodes_escaped_and_raw_string_values() {
        let escaped = CfgExpression::parse(r#"target_env = "lin\x75x""#).expect("解析转义字符串");
        let raw = CfgExpression::parse(r##"target_env = r#"linux"#"##).expect("解析原始字符串");

        assert_eq!(escaped.normalized(), r#"target_env = "linux""#);
        assert_eq!(raw.normalized(), r#"target_env = "linux""#);
        assert_eq!(
            CfgExpression::parse("target_env = \"don't\"")
                .expect("解析引号")
                .normalized(),
            "target_env = \"don't\""
        );
    }

    #[test]
    fn rejects_malformed_operator_shapes_and_trailing_input() {
        assert!(CfgExpression::parse("not(unix, windows)").is_err());
        assert!(CfgExpression::parse("not()").is_err());
        assert!(CfgExpression::parse("unix extra").is_err());
        assert!(CfgExpression::parse("all(unix").is_err());
    }

    #[test]
    fn only_proves_coexistence_false_for_known_boolean_complements() {
        let unix = CfgCondition::parse("unix").expect("解析布尔 cfg");
        let not_unix = CfgCondition::parse("not(unix)").expect("解析取反 cfg");
        let x86 = CfgCondition::parse("target_arch = \"x86\"").expect("解析目标值");
        let arm = CfgCondition::parse("target_arch = \"arm\"").expect("解析目标值");

        assert!(!CfgCondition::may_coexist(&unix, &not_unix));
        assert!(!CfgCondition::may_coexist(
            &CfgCondition::parse("all(unix, debug_assertions)").expect("解析组合 cfg"),
            &not_unix,
        ));
        assert!(CfgCondition::may_coexist(&x86, &arm));
    }
}
