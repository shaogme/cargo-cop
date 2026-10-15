use std::path::PathBuf;

use cop_syntax::{
    CoverageStatus, ItemKind, LiteralKind, RustLexer, RustParser, SourceFile, TokenKind, TriviaKind,
};

#[test]
fn keeps_nested_comments_literals_and_lifetimes_separate_from_code() {
    let source = SourceFile::new(
        PathBuf::from("src/lib.rs"),
        r####"/* outer /* nested */ comment */
fn inspect<'a>(value: &'a str) {
    let raw = r###"// std::mem::drop(1)"###;
    let character = 'λ';
    let byte = b'x';
}
"####,
    );
    let lexed = RustLexer::lex(&source);

    assert!(lexed.issues().is_empty());
    assert!(matches!(
        lexed.tokens()[0].leading_trivia()[0].kind(),
        TriviaKind::BlockComment
    ));
    assert!(lexed
        .tokens()
        .iter()
        .any(|token| matches!(token.kind(), TokenKind::Lifetime(value) if value == "'a")));
    assert!(lexed
        .tokens()
        .iter()
        .any(|token| matches!(token.kind(), TokenKind::Literal(LiteralKind::String))));
    assert!(lexed
        .tokens()
        .iter()
        .any(|token| matches!(token.kind(), TokenKind::Literal(LiteralKind::Character))));
    assert!(lexed
        .tokens()
        .iter()
        .any(|token| matches!(token.kind(), TokenKind::Literal(LiteralKind::Byte))));

    let parsed = RustParser::parse(source);
    assert_eq!(parsed.coverage(), CoverageStatus::Complete);
}

#[test]
fn reports_unicode_columns_and_unclosed_lexical_constructs() {
    let source = SourceFile::new("src/lib.rs", "// 🧭\nfn ready() {}\n");
    let function_offset = source.text().find("fn ready").expect("函数位置");
    assert_eq!(source.line_column(function_offset), Some((2, 1)));

    let parsed = RustParser::parse(SourceFile::new("src/broken.rs", "fn broken() {"));
    assert_eq!(parsed.coverage(), CoverageStatus::Incomplete);
    assert!(!parsed.issues().is_empty());
}

#[test]
fn rejects_invalid_character_escape_forms_and_accepts_valid_scalar_escapes() {
    for source in [
        r#"const INVALID: char = '\u{}';"#,
        r#"const INVALID: char = '\u{110000}';"#,
        r#"const INVALID: char = '\u{1234567}';"#,
        r#"const INVALID: char = '\xFF';"#,
        r#"const INVALID: u8 = b'\u{41}';"#,
        "const INVALID: u8 = b'界';",
    ] {
        let parsed = RustParser::parse(SourceFile::new("src/lib.rs", source));
        assert_eq!(
            parsed.coverage(),
            CoverageStatus::Incomplete,
            "字面量应被识别为无效: {source}"
        );
    }

    let parsed = RustParser::parse(SourceFile::new(
        "src/lib.rs",
        r#"const VALID: char = '\u{1f600}'; const BYTE: u8 = b'\xFF';"#,
    ));

    assert_eq!(parsed.coverage(), CoverageStatus::Complete);
}

#[test]
fn marks_missing_and_out_of_radix_digits_as_incomplete() {
    for source in [
        "const INVALID: u8 = 0x;",
        "const INVALID: u8 = 0b_;",
        "const INVALID: u8 = 0b102;",
        "const INVALID: u8 = 0o8;",
    ] {
        let parsed = RustParser::parse(SourceFile::new("src/lib.rs", source));
        assert_eq!(
            parsed.coverage(),
            CoverageStatus::Incomplete,
            "无效基数数字应标记为不完整: {source}"
        );
        assert!(!parsed.issues().is_empty());
    }

    let parsed = RustParser::parse(SourceFile::new(
        "src/lib.rs",
        "const HEX: u16 = 0x_FF; const OCTAL: u16 = 0o755; const BINARY: u16 = 0b_1010;",
    ));

    assert_eq!(parsed.coverage(), CoverageStatus::Complete);
    assert!(parsed.issues().is_empty());
}

#[test]
fn maps_crlf_lines_and_final_empty_lines_without_splitting_unicode() {
    let source = SourceFile::new("src/lib.rs", "界\r\nx\n");

    assert_eq!(source.line_count(), 3);
    assert_eq!(source.line_text(1), Some("界"));
    assert_eq!(source.line_text(2), Some("x"));
    assert_eq!(source.line_text(3), Some(""));
    assert_eq!(source.line_column(0), Some((1, 1)));
    assert_eq!(source.line_column(3), Some((1, 2)));
    assert_eq!(source.line_column(5), Some((2, 1)));
    assert_eq!(source.line_column(7), Some((3, 1)));
    assert_eq!(source.line_column(1), None);
    assert_eq!(source.line_column(8), None);
}

#[test]
fn parses_generic_impl_headers_without_stalling() {
    let parsed = RustParser::parse(SourceFile::new(
        "src/lib.rs",
        "pub trait Local<T> {}\nimpl Local<u8> for Vec<u8> {}\n",
    ));
    assert_eq!(parsed.coverage(), CoverageStatus::Complete);
}

#[test]
fn recovers_after_an_unnamed_function_without_swallowing_later_items() {
    let parsed = RustParser::parse(SourceFile::new("src/lib.rs", "fn () {}\nstruct Kept;\n"));

    assert_eq!(parsed.coverage(), CoverageStatus::Partial);
    assert_eq!(parsed.items().len(), 1);
    assert!(matches!(
        parsed.items()[0].kind(),
        ItemKind::Struct(declaration) if declaration.name() == Some("Kept")
    ));
}
