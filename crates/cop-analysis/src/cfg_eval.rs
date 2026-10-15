use cop_cargo::TargetContext;
use cop_syntax::{Attribute, SourceFile};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CfgDecision {
    True,
    False,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UnknownCondition {
    pub(crate) condition: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AttributeEffects {
    pub(crate) decision: Option<CfgDecision>,
    pub(crate) path: Option<String>,
    pub(crate) path_unknown: bool,
    pub(crate) conditions: Vec<String>,
    pub(crate) unknown: Vec<UnknownCondition>,
    pub(crate) malformed: bool,
}

pub(crate) struct CfgEvaluator<'a> {
    target: &'a TargetContext,
}

struct PredicateParser<'a, 'e, 't> {
    source: &'a str,
    cursor: usize,
    evaluator: &'e CfgEvaluator<'t>,
}

impl CfgDecision {
    pub(crate) fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            _ => Self::Unknown,
        }
    }

    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            _ => Self::Unknown,
        }
    }

    fn negate(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
        }
    }
}

impl<'a> CfgEvaluator<'a> {
    pub(crate) fn new(target: &'a TargetContext) -> Self {
        Self { target }
    }

    pub(crate) fn evaluate_condition(&self, condition: &str) -> CfgDecision {
        let trimmed = condition.trim();
        let expression = trimmed
            .strip_prefix("cfg(")
            .and_then(|value| value.strip_suffix(')'))
            .unwrap_or(trimmed);
        let mut parser = PredicateParser {
            source: expression,
            cursor: 0,
            evaluator: self,
        };
        match parser.parse_expression() {
            Ok(decision) if parser.finished() => decision,
            _ => CfgDecision::Unknown,
        }
    }

    pub(crate) fn evaluate_attributes(
        &self,
        attributes: &[Attribute],
        source: &SourceFile,
    ) -> AttributeEffects {
        let mut effects = AttributeEffects::default();
        for attribute in attributes {
            let Some(raw) = source.slice(attribute.start(), attribute.end()) else {
                effects.malformed = true;
                continue;
            };
            let Some(body) = raw
                .strip_prefix("#![")
                .or_else(|| raw.strip_prefix("#["))
                .and_then(|value| value.strip_suffix(']'))
            else {
                effects.malformed = true;
                continue;
            };
            self.apply_attribute(body.trim(), attribute, &mut effects, 0);
        }
        effects
    }

    pub(crate) fn dependency_condition(&self, condition: &str) -> CfgDecision {
        let condition = condition.trim();
        if !condition.starts_with("cfg(") {
            return if condition == self.target.target_triple() {
                CfgDecision::True
            } else {
                CfgDecision::False
            };
        }
        self.evaluate_condition(condition)
    }

    fn apply_attribute(
        &self,
        source: &str,
        attribute: &Attribute,
        effects: &mut AttributeEffects,
        depth: usize,
    ) {
        if depth > 16 {
            effects.malformed = true;
            return;
        }
        let Some((name, arguments)) = split_attribute(source) else {
            effects.malformed = true;
            return;
        };
        match name {
            "cfg" => {
                let decision = self.evaluate_condition(arguments);
                effects.decision =
                    Some(effects.decision.unwrap_or(CfgDecision::True).and(decision));
                effects.conditions.push(arguments.trim().to_owned());
                if decision == CfgDecision::Unknown {
                    effects.unknown.push(UnknownCondition {
                        condition: arguments.trim().to_owned(),
                        start: attribute.start(),
                        end: attribute.end(),
                    });
                }
            }
            "cfg_attr" => self.apply_cfg_attr(arguments, attribute, effects, depth),
            "path" => match parse_string_value(arguments) {
                Some(path) if effects.path.is_none() => effects.path = Some(path),
                Some(_) => effects.malformed = true,
                None => effects.malformed = true,
            },
            _ => {}
        }
    }

    fn apply_cfg_attr(
        &self,
        arguments: &str,
        attribute: &Attribute,
        effects: &mut AttributeEffects,
        depth: usize,
    ) {
        let parts = split_top_level(arguments, ',');
        let Some(predicate) = parts.first() else {
            effects.malformed = true;
            return;
        };
        if predicate.is_empty() || parts.len() < 2 {
            effects.malformed = true;
            return;
        }
        let decision = self.evaluate_condition(predicate);
        match decision {
            CfgDecision::False => {}
            CfgDecision::True => {
                let previous_condition_count = effects.conditions.len();
                let had_path = effects.path.is_some();
                for nested in parts.iter().skip(1) {
                    if !nested.is_empty() {
                        self.apply_attribute(nested.trim(), attribute, effects, depth + 1);
                    }
                }
                if effects.conditions.len() > previous_condition_count
                    || (!had_path && effects.path.is_some())
                {
                    effects.conditions.push(predicate.trim().to_owned());
                }
            }
            CfgDecision::Unknown => {
                let mut conditional = AttributeEffects::default();
                for nested in parts.iter().skip(1) {
                    if !nested.is_empty() {
                        self.apply_attribute(nested.trim(), attribute, &mut conditional, depth + 1);
                    }
                }
                let conditional_decision = match conditional.decision {
                    None | Some(CfgDecision::True) => CfgDecision::True,
                    Some(CfgDecision::False | CfgDecision::Unknown) => CfgDecision::Unknown,
                };
                effects.decision = Some(
                    effects
                        .decision
                        .unwrap_or(CfgDecision::True)
                        .and(conditional_decision),
                );
                let path_unknown = conditional.path_unknown || conditional.path.is_some();
                if conditional_decision == CfgDecision::Unknown || path_unknown {
                    effects
                        .conditions
                        .push(format!("cfg_attr({})", predicate.trim()));
                    effects.unknown.push(UnknownCondition {
                        condition: predicate.trim().to_owned(),
                        start: attribute.start(),
                        end: attribute.end(),
                    });
                }
                effects.path_unknown |= path_unknown;
                effects.unknown.extend(conditional.unknown);
                effects.malformed |= conditional.malformed;
            }
        }
    }

    fn evaluate_atom(&self, name: &str, value: Option<&str>) -> CfgDecision {
        if name == "feature" {
            return match value {
                Some(feature) if self.target.features().contains(feature) => CfgDecision::True,
                Some(_) => CfgDecision::False,
                None => CfgDecision::Unknown,
            };
        }
        if name == "test" {
            return match value {
                None if self.target.cfg_test() => CfgDecision::True,
                None => CfgDecision::False,
                Some(_) => CfgDecision::False,
            };
        }
        if name == "proc_macro" {
            return if value.is_none()
                && self
                    .target
                    .crate_types()
                    .iter()
                    .any(|kind| kind == "proc-macro")
            {
                CfgDecision::True
            } else {
                CfgDecision::False
            };
        }
        let values = self.target.cfg_values();
        let known = values.iter().any(|candidate| candidate.name() == name)
            || matches!(
                name,
                "target_arch"
                    | "target_os"
                    | "target_env"
                    | "target_family"
                    | "target_endian"
                    | "target_pointer_width"
                    | "target_vendor"
                    | "target_abi"
                    | "target_feature"
                    | "target_has_atomic"
                    | "target_has_atomic_equal_alignment"
                    | "target_has_atomic_load_store"
                    | "target_has_atomic_primitive_alignment"
                    | "target_thread_local"
                    | "panic"
                    | "debug_assertions"
                    | "unix"
                    | "windows"
            );
        if !known {
            return CfgDecision::Unknown;
        }
        let present = values
            .iter()
            .any(|candidate| candidate.name() == name && candidate.value() == value);
        if present {
            CfgDecision::True
        } else {
            CfgDecision::False
        }
    }
}

impl PredicateParser<'_, '_, '_> {
    fn parse_expression(&mut self) -> Result<CfgDecision, ()> {
        self.skip_space();
        let name = self.identifier()?;
        self.skip_space();
        if self.consume(b'(') {
            let arguments = self.parse_arguments()?;
            return match (name.as_str(), arguments.as_slice()) {
                ("all", values) => Ok(values
                    .iter()
                    .copied()
                    .fold(CfgDecision::True, CfgDecision::and)),
                ("any", values) => Ok(values
                    .iter()
                    .copied()
                    .fold(CfgDecision::False, CfgDecision::or)),
                ("not", [value]) => Ok(value.negate()),
                ("not", _) => Err(()),
                _ => Err(()),
            };
        }
        if self.consume(b'=') {
            self.skip_space();
            let value = self.string_literal()?;
            return Ok(self.evaluator.evaluate_atom(&name, Some(&value)));
        }
        Ok(self.evaluator.evaluate_atom(&name, None))
    }

    fn parse_arguments(&mut self) -> Result<Vec<CfgDecision>, ()> {
        let mut values = Vec::new();
        self.skip_space();
        if self.consume(b')') {
            return Ok(values);
        }
        loop {
            values.push(self.parse_expression()?);
            self.skip_space();
            if self.consume(b')') {
                return Ok(values);
            }
            if !self.consume(b',') {
                return Err(());
            }
            self.skip_space();
            if self.consume(b')') {
                return Ok(values);
            }
        }
    }

    fn identifier(&mut self) -> Result<String, ()> {
        let start = self.cursor;
        while self
            .source
            .as_bytes()
            .get(self.cursor)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-')
        {
            self.cursor += 1;
        }
        (start < self.cursor)
            .then(|| self.source[start..self.cursor].to_owned())
            .ok_or(())
    }

    fn string_literal(&mut self) -> Result<String, ()> {
        if !self.consume(b'"') {
            return Err(());
        }
        let mut value = String::new();
        while let Some(character) = self
            .source
            .get(self.cursor..)
            .and_then(|s| s.chars().next())
        {
            self.cursor += character.len_utf8();
            match character {
                '"' => return Ok(value),
                '\\' => {
                    let escaped = self.source.as_bytes().get(self.cursor).copied().ok_or(())?;
                    self.cursor += 1;
                    match escaped {
                        b'"' => value.push('"'),
                        b'\\' => value.push('\\'),
                        b'n' => value.push('\n'),
                        b'r' => value.push('\r'),
                        b't' => value.push('\t'),
                        b'0' => value.push('\0'),
                        b'x' => value.push(self.hexadecimal_escape()?),
                        b'u' => value.push(self.unicode_escape()?),
                        b'\n' => self.skip_continuation_whitespace(),
                        b'\r' if self.consume(b'\n') => self.skip_continuation_whitespace(),
                        _ => return Err(()),
                    }
                }
                '\n' | '\r' => return Err(()),
                other => value.push(other),
            }
        }
        Err(())
    }

    fn hexadecimal_escape(&mut self) -> Result<char, ()> {
        let digits = self.source.get(self.cursor..self.cursor + 2).ok_or(())?;
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(());
        }
        let value = u8::from_str_radix(digits, 16).map_err(|_| ())?;
        if !value.is_ascii() {
            return Err(());
        }
        self.cursor += 2;
        Ok(char::from(value))
    }

    fn unicode_escape(&mut self) -> Result<char, ()> {
        if !self.consume(b'{') {
            return Err(());
        }
        let mut digits = String::new();
        while let Some(character) = self
            .source
            .get(self.cursor..)
            .and_then(|remaining| remaining.chars().next())
        {
            self.cursor += character.len_utf8();
            match character {
                '}' if !digits.is_empty() => {
                    let value = u32::from_str_radix(&digits, 16).map_err(|_| ())?;
                    return char::from_u32(value).ok_or(());
                }
                '_' => {}
                value if value.is_ascii_hexdigit() => digits.push(value),
                _ => return Err(()),
            }
        }
        Err(())
    }

    fn skip_continuation_whitespace(&mut self) {
        while self
            .source
            .get(self.cursor..)
            .and_then(|remaining| remaining.chars().next())
            .is_some_and(char::is_whitespace)
        {
            let length = self
                .source
                .get(self.cursor..)
                .and_then(|remaining| remaining.chars().next())
                .map_or(0, char::len_utf8);
            self.cursor += length;
        }
    }

    fn skip_space(&mut self) {
        while self
            .source
            .as_bytes()
            .get(self.cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.cursor += 1;
        }
    }

    fn consume(&mut self, byte: u8) -> bool {
        if self.source.as_bytes().get(self.cursor) == Some(&byte) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn finished(&mut self) -> bool {
        self.skip_space();
        self.cursor == self.source.len()
    }
}

fn split_attribute(source: &str) -> Option<(&str, &str)> {
    let open = source.find('(');
    let equals = source.find('=');
    if let Some(open) = open.filter(|open| equals.map_or(true, |equals| *open < equals)) {
        let name = source[..open].trim();
        if name.is_empty() {
            return None;
        }
        let arguments = source[open + 1..].strip_suffix(')')?;
        return Some((name, arguments));
    }
    if let Some(equals) = equals {
        let name = source[..equals].trim();
        return (!name.is_empty()).then_some((name, &source[equals..]));
    }
    let name = source.trim();
    (!name.is_empty()).then_some((name, ""))
}

fn split_top_level(source: &str, separator: char) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in source.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
        } else {
            match character {
                '"' => quoted = true,
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth = depth.saturating_sub(1),
                value if value == separator && depth == 0 => {
                    pieces.push(source[start..index].trim());
                    start = index + character.len_utf8();
                }
                _ => {}
            }
        }
    }
    pieces.push(source[start..].trim());
    pieces
}

fn decode_string_literal(literal: &str) -> Option<String> {
    let characters = literal.chars().collect::<Vec<_>>();
    let mut decoded = String::new();
    let mut cursor = 0;
    while let Some(character) = characters.get(cursor).copied() {
        cursor += 1;
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        let escaped = characters.get(cursor).copied()?;
        cursor += 1;
        match escaped {
            '"' => decoded.push('"'),
            '\\' => decoded.push('\\'),
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            '0' => decoded.push('\0'),
            'x' => {
                let digits = characters.get(cursor..cursor + 2)?;
                if !digits.iter().all(char::is_ascii_hexdigit) {
                    return None;
                }
                let value = u8::from_str_radix(&digits.iter().collect::<String>(), 16).ok()?;
                if !value.is_ascii() {
                    return None;
                }
                decoded.push(char::from(value));
                cursor += 2;
            }
            'u' => {
                if characters.get(cursor) != Some(&'{') {
                    return None;
                }
                cursor += 1;
                let start = cursor;
                let mut digits = String::new();
                while let Some(character) = characters.get(cursor).copied() {
                    cursor += 1;
                    match character {
                        '}' if !digits.is_empty() && digits.len() <= 6 => break,
                        '_' => {}
                        value if value.is_ascii_hexdigit() => digits.push(value),
                        _ => return None,
                    }
                    if cursor == characters.len() {
                        return None;
                    }
                }
                if cursor == start || characters.get(cursor - 1) != Some(&'}') {
                    return None;
                }
                let value = u32::from_str_radix(&digits, 16).ok()?;
                decoded.push(char::from_u32(value)?);
            }
            '\n' => skip_string_continuation_whitespace(&characters, &mut cursor),
            '\r' if characters.get(cursor) == Some(&'\n') => {
                cursor += 1;
                skip_string_continuation_whitespace(&characters, &mut cursor);
            }
            _ => return None,
        }
    }
    Some(decoded)
}

fn skip_string_continuation_whitespace(characters: &[char], cursor: &mut usize) {
    while characters
        .get(*cursor)
        .is_some_and(|character| character.is_whitespace())
    {
        *cursor += 1;
    }
}

fn parse_string_value(source: &str) -> Option<String> {
    let value = source.trim().strip_prefix('=')?.trim();
    if let Some(raw) = value.strip_prefix('r') {
        let hashes = raw.bytes().take_while(|byte| *byte == b'#').count();
        let content = raw.get(hashes..)?.strip_prefix('"')?;
        let ending = format!("\"{}", "#".repeat(hashes));
        return content.strip_suffix(&ending).map(str::to_owned);
    }
    let literal = value.strip_prefix('"')?.strip_suffix('"')?;
    decode_string_literal(literal)
}
