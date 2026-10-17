pub struct RustStringLiteral;

impl RustStringLiteral {
    pub fn decode(literal: &str) -> Option<String> {
        if let Some(content) = raw_string_content(literal) {
            return Some(content.to_owned());
        }
        let content = literal.strip_prefix('"')?.strip_suffix('"')?;
        decode_escaped_content(content)
    }
}

fn raw_string_content(literal: &str) -> Option<&str> {
    let raw = literal.strip_prefix('r')?;
    let hash_count = raw.bytes().take_while(|byte| *byte == b'#').count();
    let content = raw.get(hash_count..)?.strip_prefix('"')?;
    let closing = format!("\"{}", "#".repeat(hash_count));
    content.strip_suffix(&closing)
}

fn decode_escaped_content(content: &str) -> Option<String> {
    let characters = content.chars().collect::<Vec<_>>();
    let mut decoded = String::new();
    let mut cursor = 0;
    while let Some(character) = characters.get(cursor).copied() {
        cursor += 1;
        match character {
            '"' | '\n' | '\r' => return None,
            '\\' => decode_escape(&characters, &mut cursor, &mut decoded)?,
            value => decoded.push(value),
        }
    }
    Some(decoded)
}

fn decode_escape(characters: &[char], cursor: &mut usize, decoded: &mut String) -> Option<()> {
    let escaped = *characters.get(*cursor)?;
    *cursor += 1;
    match escaped {
        '"' => decoded.push('"'),
        '\\' => decoded.push('\\'),
        'n' => decoded.push('\n'),
        'r' => decoded.push('\r'),
        't' => decoded.push('\t'),
        '0' => decoded.push('\0'),
        'x' => decode_hex_escape(characters, cursor, decoded)?,
        'u' => decode_unicode_escape(characters, cursor, decoded)?,
        '\n' => skip_continuation_whitespace(characters, cursor),
        '\r' if characters.get(*cursor) == Some(&'\n') => {
            *cursor += 1;
            skip_continuation_whitespace(characters, cursor);
        }
        _ => return None,
    }
    Some(())
}

fn decode_hex_escape(characters: &[char], cursor: &mut usize, decoded: &mut String) -> Option<()> {
    let digits = characters.get(*cursor..cursor.checked_add(2)?)?;
    if !digits.iter().all(char::is_ascii_hexdigit) {
        return None;
    }
    let value = u8::from_str_radix(&digits.iter().collect::<String>(), 16).ok()?;
    if !value.is_ascii() {
        return None;
    }
    decoded.push(char::from(value));
    *cursor += 2;
    Some(())
}

fn decode_unicode_escape(
    characters: &[char],
    cursor: &mut usize,
    decoded: &mut String,
) -> Option<()> {
    if characters.get(*cursor) != Some(&'{') {
        return None;
    }
    *cursor += 1;
    let mut digits = String::new();
    loop {
        let character = *characters.get(*cursor)?;
        *cursor += 1;
        match character {
            '}' if !digits.is_empty() && digits.len() <= 6 => {
                let value = u32::from_str_radix(&digits, 16).ok()?;
                decoded.push(char::from_u32(value)?);
                return Some(());
            }
            '_' => {}
            value if value.is_ascii_hexdigit() => digits.push(value),
            _ => return None,
        }
    }
}

fn skip_continuation_whitespace(characters: &[char], cursor: &mut usize) {
    while characters
        .get(*cursor)
        .is_some_and(|character| character.is_whitespace())
    {
        *cursor += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::RustStringLiteral;

    #[test]
    fn decodes_standard_escapes() {
        assert_eq!(
            RustStringLiteral::decode("\"a\\x2e\\u{2e}\\\\b\""),
            Some("a..\\b".to_owned())
        );
    }

    #[test]
    fn decodes_raw_strings_without_interpreting_escapes() {
        assert_eq!(
            RustStringLiteral::decode("r##\"a\\n\"##"),
            Some("a\\n".to_owned())
        );
    }

    #[test]
    fn removes_line_continuation_whitespace() {
        assert_eq!(
            RustStringLiteral::decode("\"a\\\n    b\""),
            Some("ab".to_owned())
        );
    }

    #[test]
    fn rejects_malformed_string_literals() {
        assert_eq!(RustStringLiteral::decode("\"\\xFF\""), None);
        assert_eq!(RustStringLiteral::decode("\"unterminated"), None);
    }
}
