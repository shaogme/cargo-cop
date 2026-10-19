use cop_syntax::{
    CoverageStatus, ExpressionSyntaxKind, ForeignItemSyntaxKind, ItemSyntaxKind, ParseIssueKind,
    ParseOptions, PatternSyntaxKind, RustParser, SourceFile, SyntaxNodeKind, TypeSyntaxKind,
};

fn parse(source: &str, edition: &str) -> cop_syntax::ParseOutput {
    RustParser::parse(
        SourceFile::new("src/parser_regressions.rs", source),
        ParseOptions::new(edition),
    )
}

fn unexpected_issues(parsed: &cop_syntax::ParseOutput) -> Vec<(usize, usize)> {
    parsed
        .issues()
        .iter()
        .filter(|issue| issue.kind() == ParseIssueKind::UnexpectedToken)
        .map(|issue| (issue.start(), issue.end()))
        .collect()
}

#[test]
fn parses_explicit_lifetime_receivers_and_preserves_parameter_ranges() {
    let source = "struct S; impl S { fn src<'src>(&'src mut self) {} fn env<'env>(&'env mut self) {} fn a<'a>(&'a mut self) {} fn storage<'storage>(&'storage self) {} fn child<'child>(&'child mut self) {} fn b<'b>(&'b mut self) {} fn index<'index>(&'index mut self) {} fn index_shared<'index>(&'index self) {} fn shared(&self) {} fn mutable(&mut self) {} fn value(mut self) {} fn plain(self) {} }";
    let parsed = parse(source, "2021");
    let parameters = parsed
        .syntax_nodes()
        .iter()
        .filter(|node| node.kind() == SyntaxNodeKind::Parameter)
        .collect::<Vec<_>>();

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "{:?}",
        parsed.issues()
    );
    assert!(parsed.issues().is_empty(), "{:?}", parsed.issues());
    assert_eq!(parameters.len(), 12);
    assert!(parameters
        .iter()
        .all(|node| node.is_receiver_parameter() == Some(true)));
    assert!(parameters.iter().all(|node| {
        node.children()
            .iter()
            .filter_map(|id| parsed.node(*id))
            .find(|child| child.kind() == SyntaxNodeKind::Pattern)
            .and_then(|child| child.pattern_syntax())
            .is_some_and(|pattern| pattern.bounds() == (node.start()..node.end()))
    }));
    assert_eq!(
        parameters
            .iter()
            .filter_map(|node| {
                node.children()
                    .iter()
                    .filter_map(|id| parsed.node(*id))
                    .find(|child| child.kind() == SyntaxNodeKind::Pattern)
                    .and_then(|child| child.pattern_syntax())
            })
            .filter(|pattern| matches!(pattern.kind(), PatternSyntaxKind::Reference { .. }))
            .count(),
        10
    );
    assert!(parameters.iter().all(|node| {
        source
            .get(node.start()..node.end())
            .is_some_and(|text| text.contains("self"))
    }));
}

#[test]
fn parses_2024_let_chains_and_keeps_the_2021_edition_diagnostic() {
    let source = "fn inspect(ready: bool, visit: Visit) { if ready && let Some(field) = visit.target.field { let _ = field; } }";
    let parsed_2024 = parse(source, "2024");
    let parsed_2021 = parse(source, "2021");
    let let_start = source.find("let Some(field)").expect("let 条件位置");
    let let_end = source[let_start..]
        .find(" {")
        .map(|offset| let_start + offset)
        .expect("条件结束位置");

    assert!(
        parsed_2024.issues().is_empty(),
        "{:?}",
        parsed_2024.issues()
    );
    assert!(parsed_2024.syntax_nodes().iter().any(|node| {
        node.expression_syntax()
            .is_some_and(|syntax| matches!(syntax.kind(), ExpressionSyntaxKind::Let { .. }))
    }));
    assert!(unexpected_issues(&parsed_2021)
        .iter()
        .any(|(start, end)| *start >= let_start && *end <= let_end));
}

#[test]
fn parses_foreign_variadics_only_as_final_parameter_markers() {
    let source = "type Callback = unsafe extern \"C\" fn(i32, ...); extern \"C\" { fn printf(format: *const i8, ...); fn malformed(..., value: i32); fn after(value: i32); } fn ordinary(..., value: i32) {} fn kept() {}";
    let parsed = parse(source, "2021");
    let foreign_start = source.find("fn printf").expect("foreign 函数位置");
    let valid_ellipsis = source[foreign_start..]
        .find("...")
        .map(|offset| foreign_start + offset)
        .expect("variadic 标记");
    let malformed_ellipsis = source[valid_ellipsis + 3..]
        .find("...")
        .map(|offset| valid_ellipsis + 3 + offset)
        .expect("非末尾 variadic 标记");
    let ordinary_ellipsis = source[malformed_ellipsis + 3..]
        .find("...")
        .map(|offset| malformed_ellipsis + 3 + offset)
        .expect("普通函数中的非法标记");
    let foreign_functions = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.foreign_item_syntax())
        .filter(|item| matches!(item.kind(), ForeignItemSyntaxKind::Function { .. }))
        .count();
    let foreign_names = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.foreign_item_syntax())
        .filter_map(|item| match item.kind() {
            ForeignItemSyntaxKind::Function { name, .. } => name.as_deref(),
            ForeignItemSyntaxKind::Static { .. } => None,
        })
        .collect::<Vec<_>>();

    assert!(foreign_functions >= 3, "{:?}", parsed.syntax_nodes());
    assert!(foreign_names.contains(&"printf"));
    assert!(foreign_names.contains(&"malformed"));
    assert!(foreign_names.contains(&"after"));
    assert!(unexpected_issues(&parsed)
        .iter()
        .any(|(start, end)| { *start == malformed_ellipsis && *end == malformed_ellipsis + 3 }));
    assert!(unexpected_issues(&parsed)
        .iter()
        .any(|(start, end)| { *start == ordinary_ellipsis && *end == ordinary_ellipsis + 3 }));
    assert!(!unexpected_issues(&parsed)
        .iter()
        .any(|(start, _)| *start == valid_ellipsis));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        matches!(
            node.type_syntax().map(|syntax| syntax.kind()),
            Some(TypeSyntaxKind::FunctionPointer { variadic: true, .. })
        )
    }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.item_syntax().is_some_and(|item| {
            matches!(item.kind(), ItemSyntaxKind::Function { name: Some(name) } if name == "kept")
        })
    }));
    assert!(!parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::Parameter
            && source.get(node.start()..node.end()) == Some("...")
            && node
                .parent()
                .and_then(|id| parsed.node(id))
                .is_some_and(|parent| parent.kind() == SyntaxNodeKind::ForeignItem)
    }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::Parameter
            && source.get(node.start()..node.end()) == Some("...")
            && node
                .parent()
                .and_then(|id| parsed.node(id))
                .is_some_and(|parent| parent.kind() == SyntaxNodeKind::Item)
    }));
}

#[test]
fn reuses_let_type_annotation_ranges_for_ast_collection() {
    let source = "fn inspect(errors: Errors, meta: Meta) { let keys: Vec<_> = errors; let value: LitStr = meta.value()?.parse()?; }";
    let parsed = parse(source, "2021");
    let types = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.type_syntax())
        .collect::<Vec<_>>();

    assert!(parsed.issues().is_empty(), "{:?}", parsed.issues());
    assert!(types.iter().any(|syntax| {
        matches!(syntax.kind(), TypeSyntaxKind::Path(path) if path.segments().first().is_some_and(|segment| segment.name() == "Vec"))
            && parsed.source().slice(syntax.bounds().start, syntax.bounds().end) == Some("Vec<_>")
    }));
    assert!(types.iter().any(|syntax| {
        matches!(syntax.kind(), TypeSyntaxKind::Path(path) if path.segments().first().is_some_and(|segment| segment.name() == "LitStr"))
            && parsed.source().slice(syntax.bounds().start, syntax.bounds().end) == Some("LitStr")
    }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        matches!(
            node.type_syntax().map(|syntax| syntax.kind()),
            Some(TypeSyntaxKind::Path(_))
        )
    }));
}

#[test]
fn retains_diagnostics_for_incomplete_let_annotations() {
    let source = "fn inspect() { let missing_type: = value; let missing_equals: u8 value; }";
    let parsed = parse(source, "2021");
    let missing_type = source.find(": =").expect("缺失类型") + 2;
    let missing_equals = source.rfind("value;").expect("缺少赋值符");

    assert!(!parsed.issues().is_empty());
    assert!(parsed
        .issues()
        .iter()
        .all(|issue| issue.kind() == ParseIssueKind::UnexpectedToken));
    assert!(parsed
        .issues()
        .iter()
        .any(|issue| issue.start() == missing_type && issue.end() == missing_type + 1));
    assert!(parsed
        .issues()
        .iter()
        .any(|issue| issue.start() == missing_equals));
}
