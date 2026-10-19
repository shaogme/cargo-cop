use std::path::PathBuf;

use cop_syntax::{
    CoverageStatus, ItemSyntaxKind, LiteralKind, ParseIssueCode, ParseIssueKind, ParseOptions,
    ParseOutput, RustLexer, RustParser, SourceFile, TokenKind, TriviaKind,
};

fn parse(source: SourceFile) -> ParseOutput {
    RustParser::parse(source, ParseOptions::default())
}

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

    let parsed = parse(source);
    assert_eq!(parsed.coverage(), CoverageStatus::Complete);
}

#[test]
fn reports_unicode_columns_and_unclosed_lexical_constructs() {
    let source = SourceFile::new("src/lib.rs", "// 🧭\nfn ready() {}\n");
    let function_offset = source.text().find("fn ready").expect("函数位置");
    assert_eq!(source.line_column(function_offset), Some((2, 1)));

    let parsed = parse(SourceFile::new("src/broken.rs", "fn broken() {"));
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
        let parsed = parse(SourceFile::new("src/lib.rs", source));
        assert_eq!(
            parsed.coverage(),
            CoverageStatus::Partial,
            "边界可定位的非法字面量应标记为部分覆盖: {source}"
        );
    }

    let parsed = parse(SourceFile::new(
        "src/lib.rs",
        r#"const VALID: char = '\u{1f600}'; const BYTE: u8 = b'\xFF';"#,
    ));

    assert_eq!(parsed.coverage(), CoverageStatus::Complete);
}

#[test]
fn marks_missing_and_out_of_radix_digits_as_partial() {
    for source in [
        "const INVALID: u8 = 0x;",
        "const INVALID: u8 = 0b_;",
        "const INVALID: u8 = 0b102;",
        "const INVALID: u8 = 0o8;",
    ] {
        let parsed = parse(SourceFile::new("src/lib.rs", source));
        assert_eq!(
            parsed.coverage(),
            CoverageStatus::Partial,
            "边界可定位的无效基数数字应标记为部分覆盖: {source}"
        );
        assert!(parsed.issues().iter().any(|issue| {
            issue.code() == ParseIssueCode::LexicalError
                && issue.coverage_status() == CoverageStatus::Partial
        }));
    }

    let parsed = parse(SourceFile::new(
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
    let parsed = parse(SourceFile::new(
        "src/lib.rs",
        "pub trait Local<T> {}\nimpl Local<u8> for Vec<u8> {}\n",
    ));
    assert_eq!(parsed.coverage(), CoverageStatus::Complete);
}

#[test]
fn recovers_after_an_unnamed_function_without_swallowing_later_items() {
    let parsed = parse(SourceFile::new("src/lib.rs", "fn () {}\nstruct Kept;\n"));

    assert_eq!(parsed.coverage(), CoverageStatus::Partial);
    assert_eq!(
        parsed
            .syntax_nodes()
            .iter()
            .filter(|node| node.item_syntax().is_some_and(|data| {
                matches!(data.kind(), ItemSyntaxKind::Struct(Some(name)) if name == "Kept")
            }))
            .count(),
        1
    );
}

#[test]
fn rejects_empty_and_malformed_use_trees_with_local_coverage_ranges() {
    for source in ["use ;\n", "use crate::{First,,Second};\n"] {
        let parsed = parse(SourceFile::new("src/lib.rs", source));

        assert_eq!(parsed.coverage(), CoverageStatus::Partial, "{source}");
        assert!(parsed
            .issues()
            .iter()
            .any(|issue| { issue.kind() == ParseIssueKind::UnexpectedToken }));
        assert!(!parsed.syntax_nodes().iter().any(|node| {
            node.item_syntax()
                .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Use))
        }));
        assert_eq!(
            parsed.coverage_for_range(0, source.len()),
            CoverageStatus::Partial
        );
    }
}

#[test]
fn keeps_supported_ranges_analyzable_around_opaque_macro_input() {
    let text = "fn available() {}\nfn opaque() { call!({ let hidden = 0; }); }\n";
    let parsed = parse(SourceFile::new("src/lib.rs", text));
    let supported_end = text.find("fn opaque").expect("opaque 函数位置");
    let opaque_start = supported_end;

    assert_eq!(
        parsed.coverage_for_range(0, supported_end),
        CoverageStatus::Complete
    );
    assert_eq!(
        parsed.coverage_for_range(opaque_start, text.len()),
        CoverageStatus::Partial
    );
    assert!(parsed.is_supported_range(0, supported_end));
    let macro_start = text.find("call!").expect("宏调用");
    assert!(parsed.is_supported_range(macro_start, macro_start + "call!".len()));
    let invocation = parsed
        .syntax_nodes()
        .iter()
        .find_map(|node| node.macro_invocation())
        .expect("宏调用节点");
    let opaque = invocation.input().bounds();
    assert!(!parsed.is_supported_range(opaque.start, opaque.end));
}
