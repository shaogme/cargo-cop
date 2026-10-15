use crate::{CargoError, CfgValue};
use cop_model::{LocalizedMessage, ToolErrorKind, CARGO_TARGET_CFG_INVALID};

pub(crate) struct TargetCfgEvaluator;

struct CfgParser<'a> {
    source: &'a str,
    cursor: usize,
    values: &'a [CfgValue],
}

impl TargetCfgEvaluator {
    pub(crate) fn matches(
        condition: &str,
        target_triple: &str,
        values: &[CfgValue],
    ) -> Result<bool, CargoError> {
        let condition = condition.trim();
        let Some(expression) = condition
            .strip_prefix("cfg(")
            .and_then(|expression| expression.strip_suffix(')'))
        else {
            return Ok(condition == target_triple);
        };
        let mut parser = CfgParser {
            source: expression,
            cursor: 0,
            values,
        };
        let matched = parser
            .parse_expression()
            .and_then(|matched| {
                parser.skip_whitespace();
                if parser.cursor == parser.source.len() {
                    Ok(matched)
                } else {
                    Err(())
                }
            })
            .map_err(|_detail| {
                CargoError::new(
                    ToolErrorKind::CargoMetadata,
                    LocalizedMessage::new(CARGO_TARGET_CFG_INVALID)
                        .with_text("condition", condition),
                )
            })?;
        Ok(matched)
    }
}

impl CfgParser<'_> {
    fn parse_expression(&mut self) -> Result<bool, ()> {
        self.skip_whitespace();
        let name = self.parse_identifier()?;
        self.skip_whitespace();
        if self.consume(b'(') {
            let arguments = self.parse_arguments()?;
            return match (name.as_str(), arguments.as_slice()) {
                ("all", arguments) => Ok(arguments.iter().all(|value| *value)),
                ("any", arguments) => Ok(arguments.iter().any(|value| *value)),
                ("not", [value]) => Ok(!*value),
                ("not", _) => Err(()),
                _ => Err(()),
            };
        }
        if self.consume(b'=') {
            self.skip_whitespace();
            let expected = self.parse_string()?;
            return Ok(self
                .values
                .iter()
                .any(|value| value.name() == name && value.value() == Some(expected.as_str())));
        }
        Ok(self.values.iter().any(|value| value.name() == name))
    }

    fn parse_arguments(&mut self) -> Result<Vec<bool>, ()> {
        let mut arguments = Vec::new();
        loop {
            self.skip_whitespace();
            if self.consume(b')') {
                return Ok(arguments);
            }
            arguments.push(self.parse_expression()?);
            self.skip_whitespace();
            if self.consume(b')') {
                return Ok(arguments);
            }
            if !self.consume(b',') {
                return Err(());
            }
        }
    }

    fn parse_identifier(&mut self) -> Result<String, ()> {
        let start = self.cursor;
        while let Some(byte) = self.source.as_bytes().get(self.cursor) {
            if byte.is_ascii_alphanumeric() || *byte == b'_' {
                self.cursor += 1;
            } else {
                break;
            }
        }
        if start == self.cursor {
            return Err(());
        }
        Ok(self.source[start..self.cursor].to_owned())
    }

    fn parse_string(&mut self) -> Result<String, ()> {
        if !self.consume(b'"') {
            return Err(());
        }
        let mut value = String::new();
        while let Some(character) = self.next_character() {
            match character {
                '"' => return Ok(value),
                '\\' => {
                    if let Some(escaped) = self.parse_escape()? {
                        value.push(escaped);
                    }
                }
                '\n' | '\r' => return Err(()),
                character => value.push(character),
            }
        }
        Err(())
    }

    fn parse_escape(&mut self) -> Result<Option<char>, ()> {
        match self.next_character() {
            Some('"') => Ok(Some('"')),
            Some('\\') => Ok(Some('\\')),
            Some('n') => Ok(Some('\n')),
            Some('r') => Ok(Some('\r')),
            Some('t') => Ok(Some('\t')),
            Some('0') => Ok(Some('\0')),
            Some('x') => {
                let first = self.next_character();
                let second = self.next_character();
                let digits = [first, second];
                if !digits
                    .iter()
                    .all(|digit| digit.is_some_and(|value| value.is_ascii_hexdigit()))
                {
                    return Err(());
                }
                let digits = digits.into_iter().flatten().collect::<String>();
                let value = u8::from_str_radix(&digits, 16).map_err(|_| ())?;
                if !value.is_ascii() {
                    return Err(());
                }
                Ok(Some(char::from(value)))
            }
            Some('u') => self.parse_unicode_escape().map(Some),
            Some('\n') => {
                self.skip_continuation_whitespace();
                Ok(None)
            }
            Some('\r') if self.consume(b'\n') => {
                self.skip_continuation_whitespace();
                Ok(None)
            }
            Some(_) => Err(()),
            None => Err(()),
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, ()> {
        if !self.consume(b'{') {
            return Err(());
        }
        let mut digits = String::new();
        loop {
            match self.next_character() {
                Some('}') if !digits.is_empty() && digits.len() <= 6 => {
                    let value = u32::from_str_radix(&digits, 16).map_err(|_| ())?;
                    return char::from_u32(value).ok_or(());
                }
                Some('}') => return Err(()),
                Some('_') => {}
                Some(character) if character.is_ascii_hexdigit() => digits.push(character),
                Some(_) => return Err(()),
                None => return Err(()),
            }
            if digits.len() > 6 {
                return Err(());
            }
        }
    }

    fn skip_continuation_whitespace(&mut self) {
        while let Some(character) = self
            .source
            .get(self.cursor..)
            .and_then(|remaining| remaining.chars().next())
        {
            if !character.is_whitespace() {
                break;
            }
            self.cursor += character.len_utf8();
        }
    }

    fn next_character(&mut self) -> Option<char> {
        let character = self.source.get(self.cursor..)?.chars().next()?;
        self.cursor += character.len_utf8();
        Some(character)
    }

    fn skip_whitespace(&mut self) {
        while self
            .source
            .as_bytes()
            .get(self.cursor)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            self.cursor += 1;
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.source.as_bytes().get(self.cursor) == Some(&expected) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TargetCfgEvaluator;
    use crate::CfgValue;

    #[test]
    fn evaluates_nested_cfg_operators_and_empty_identities() {
        let values = [
            CfgValue::new("unix", None),
            CfgValue::new("target_os", Some("linux".to_owned())),
        ];

        assert!(TargetCfgEvaluator::matches(
            "cfg(all(unix, any(target_os = \"linux\", windows)))",
            "x86_64-unknown-linux-gnu",
            &values
        )
        .expect("解析嵌套 cfg"));
        assert!(
            TargetCfgEvaluator::matches("cfg(all())", "x86_64-unknown-linux-gnu", &[])
                .expect("解析空 all")
        );
        assert!(
            !TargetCfgEvaluator::matches("cfg(any())", "x86_64-unknown-linux-gnu", &[])
                .expect("解析空 any")
        );
    }

    #[test]
    fn decodes_rust_string_escapes_in_target_cfg_values() {
        let values = [CfgValue::new("target_env", Some("linux".to_owned()))];

        assert!(TargetCfgEvaluator::matches(
            r#"cfg(target_env = "lin\x75x")"#,
            "x86_64-unknown-linux-gnu",
            &values
        )
        .expect("解析十六进制转义"));
        assert!(TargetCfgEvaluator::matches(
            r#"cfg(target_env = "lin\u{75}x")"#,
            "x86_64-unknown-linux-gnu",
            &values
        )
        .expect("解析 Unicode 转义"));
        assert!(TargetCfgEvaluator::matches(
            r#"cfg(target_env = "lin\
                ux")"#,
            "x86_64-unknown-linux-gnu",
            &values
        )
        .expect("解析续行转义"));
    }

    #[test]
    fn rejects_malformed_cfg_predicates() {
        let result =
            TargetCfgEvaluator::matches("cfg(not(unix, windows))", "x86_64-unknown-linux-gnu", &[]);

        assert!(result.is_err());
    }
}
