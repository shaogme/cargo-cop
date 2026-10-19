use cop_syntax::{
    CoverageStatus, ExpressionSyntaxKind, ExpressionUnsupportedKind, MacroInputStatus,
    ParseIssueKind, ParseOptions, ParseOutput, RustParser, SourceFile, StatementSyntaxKind,
    SyntaxNodeKind, TypeSyntaxKind,
};

fn parse(source: SourceFile) -> ParseOutput {
    RustParser::parse(source, ParseOptions::default())
}

#[test]
fn parses_precedence_postfix_control_flow_and_block_expressions() {
    let text = r#"
fn model(mut value: Option<Box<Thing>>, xs: &[i32]) {
    let borrowed = &mut *value.as_mut().unwrap();
    let accessed = borrowed.field.method::<i32>(xs[0])?.tuple.0;
    let assigned = accessed += 1 * 2 + 3;
    let ranged = ..=xs.len();
    let array = [1, 2, 3];
    let repeated = [0; LENGTH];
    let object = Thing { field: value.as_ref()?.field, other: 2 };
    let closure = move |item: &Thing| item.field + 1;
    let branches = if let Some(item) = value.as_ref() { item.field } else { 0 };
    let record_pattern = if let Thing { field: item } = value { item } else { value };
    let Some(_present) = value.as_ref() else { return; };
    match xs.len() {
        0..=1 => return,
        n if n > 1 => { continue 'outer; },
        _ => (),
    }
    match value.as_ref() { Some(_) => { let no_comma = 1; } None => { let no_comma = 2; } }
    let café = 'outer: loop { break 'outer 3; };
    for item in xs { let _ = item; }
    unsafe { let _ = assigned; }
    let _ = (borrowed, accessed, ranged, array, repeated, object, closure, branches, record_pattern, café);
}

"#;
    let parsed = parse(SourceFile::new("src/expressions.rs", text));

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "{:?}",
        parsed.issues()
    );
    assert!(parsed.issues().is_empty(), "{:?}", parsed.issues());
    assert!(!parsed
        .syntax_nodes()
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::Opaque));

    let expressions = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.expression_syntax())
        .map(|syntax| syntax.kind())
        .collect::<Vec<_>>();
    assert!(expressions
        .iter()
        .any(|kind| matches!(kind, ExpressionSyntaxKind::Field { field, .. } if field == "field")));
    assert!(expressions.iter().any(|kind| matches!(kind, ExpressionSyntaxKind::MethodCall { method, generic_arguments, .. } if method == "method" && !generic_arguments.is_empty())));
    assert!(expressions
        .iter()
        .any(|kind| matches!(kind, ExpressionSyntaxKind::Try(_))));
    assert!(expressions
        .iter()
        .any(|kind| matches!(kind, ExpressionSyntaxKind::Index { .. })));
    assert!(expressions
        .iter()
        .any(|kind| matches!(kind, ExpressionSyntaxKind::TupleField { index: 0, .. })));
    assert!(expressions
        .iter()
        .any(|kind| matches!(kind, ExpressionSyntaxKind::Assignment { .. })));
    assert!(expressions.iter().any(
        |kind| matches!(kind, ExpressionSyntaxKind::Binary { operator, .. } if operator == "*")
    ));
    assert!(expressions.iter().any(|kind| matches!(
        kind,
        ExpressionSyntaxKind::Range {
            inclusive: true,
            ..
        }
    )));
    assert!(expressions
        .iter()
        .any(|kind| matches!(kind, ExpressionSyntaxKind::Repeat { .. })));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        matches!(node.expression_syntax().map(|syntax| syntax.kind()),
            Some(ExpressionSyntaxKind::Path(path)) if path.segments()[0].name() == "LENGTH")
    }));
    assert!(expressions
        .iter()
        .any(|kind| matches!(kind, ExpressionSyntaxKind::Struct { .. })));
    assert!(expressions
        .iter()
        .any(|kind| matches!(kind, ExpressionSyntaxKind::Closure { is_move: true, .. })));
    assert!(expressions
        .iter()
        .any(|kind| matches!(kind, ExpressionSyntaxKind::If { .. })));
    assert!(expressions
        .iter()
        .any(|kind| matches!(kind, ExpressionSyntaxKind::Match { arms, .. } if arms.len() == 3)));
    assert!(expressions
        .iter()
        .any(|kind| matches!(kind, ExpressionSyntaxKind::Match { arms, .. } if arms.len() == 2)));
    assert!(expressions.iter().any(|kind| matches!(kind, ExpressionSyntaxKind::Loop { label: Some(label), .. } if label == "'outer")));
    assert!(expressions
        .iter()
        .any(|kind| matches!(kind, ExpressionSyntaxKind::For { .. })));

    assert!(parsed
        .syntax_nodes()
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::MatchArm));
    assert!(
        parsed
            .syntax_nodes()
            .iter()
            .filter(|node| node.kind() == SyntaxNodeKind::Block)
            .count()
            >= 6
    );
    assert!(parsed
        .issues()
        .iter()
        .all(|issue| issue.kind() != ParseIssueKind::UnsupportedSyntax));
    for node in parsed.syntax_nodes() {
        assert!(text.is_char_boundary(node.start()));
        assert!(text.is_char_boundary(node.end()));
        assert!(parsed.source().slice(node.start(), node.end()).is_some());
        if let Some(parent) = node.parent().and_then(|id| parsed.node(id)) {
            assert!(
                parent.start() <= node.start() && node.end() <= parent.end(),
                "parent {:?} {}..{}, child {:?} {}..{}: {}",
                parent.kind(),
                parent.start(),
                parent.end(),
                node.kind(),
                node.start(),
                node.end(),
                parsed
                    .source()
                    .slice(node.start(), node.end())
                    .unwrap_or("")
            );
        }
    }
}

#[test]
fn parses_known_macro_profiles_and_keeps_unknown_inputs_opaque() {
    let text = r#"
static_loader! {
    static LOCALES = {
        locales: "./locales",
        fallback_language: "en-US",
    }
}
fn inspect(value: Value) {
    assert!(matches!(value, Some(item) if item > 0));
    assert_eq!(value, 1, "value: {}", value);
    let message = format!("value {}", value);
    let repeated = vec![value; size()];
    let environment = env!("APP_NAME", "fallback");
    let document = json!({"key": value, "array": [1, value + 2]});
    let unknown = custom_dsl!({ opaque: value });
}
"#;
    let parsed = parse(SourceFile::new("src/macro_profiles.rs", text));
    let macro_nodes = parsed
        .syntax_nodes()
        .iter()
        .filter(|node| node.kind() == SyntaxNodeKind::Macro)
        .collect::<Vec<_>>();
    let parsed_invocations = macro_nodes
        .iter()
        .filter_map(|node| node.macro_invocation())
        .filter(|invocation| invocation.input().status() == MacroInputStatus::Parsed)
        .count();

    assert_eq!(
        parsed_invocations,
        8,
        "{:?}",
        macro_nodes
            .iter()
            .filter_map(|node| node.macro_invocation())
            .map(|invocation| (
                invocation
                    .path()
                    .segments()
                    .last()
                    .map(|segment| segment.name()),
                invocation.input().status()
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        parsed
            .issues()
            .iter()
            .filter(|issue| issue.message().id().as_str() == "syntax-macro-opaque")
            .count(),
        1
    );
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::MacroInput
            && node.children().iter().any(|id| {
                parsed
                    .node(*id)
                    .is_some_and(|child| child.kind() == SyntaxNodeKind::Expression)
            })
    }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.expression_syntax().is_some_and(|syntax| {
            matches!(syntax.kind(), ExpressionSyntaxKind::Call { .. })
                && text
                    .get(syntax.bounds().start..syntax.bounds().end)
                    .is_some_and(|fragment| fragment == "size()")
        })
    }));
}

#[test]
fn preserves_empty_statements_as_typed_syntax_nodes() {
    let parsed = parse(SourceFile::new(
        "src/empty_statements.rs",
        "fn inspect() { ; ; let value = 1; value }",
    ));

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "{:?}",
        parsed.issues()
    );
    assert_eq!(
        parsed
            .syntax_nodes()
            .iter()
            .filter(|node| node.statement_kind() == Some(StatementSyntaxKind::Empty))
            .count(),
        2
    );
}

#[test]
fn models_braced_expression_statements_as_statements_with_block_children() {
    let parsed = parse(SourceFile::new(
        "src/block_statements.rs",
        "fn inspect() { { let first = 1; }; { let last = 2; } }",
    ));
    let statements = parsed
        .syntax_nodes()
        .iter()
        .filter(|node| node.kind() == SyntaxNodeKind::Statement)
        .filter_map(|node| node.statement_kind().map(|kind| (node, kind)))
        .filter(|(_, kind)| matches!(kind, StatementSyntaxKind::Expression { .. }))
        .collect::<Vec<_>>();

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "{:?}",
        parsed.issues()
    );
    assert_eq!(statements.len(), 2);
    assert!(matches!(
        statements[0].1,
        StatementSyntaxKind::Expression {
            has_semicolon: true,
            is_tail: false
        }
    ));
    assert!(matches!(
        statements[1].1,
        StatementSyntaxKind::Expression {
            has_semicolon: false,
            is_tail: true
        }
    ));
    assert!(statements.iter().all(|(statement, _)| statement
        .children()
        .iter()
        .filter_map(|id| parsed.node(*id))
        .any(|child| child.kind() == SyntaxNodeKind::Expression)));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::Block
            && node.children().iter().any(|id| {
                parsed
                    .node(*id)
                    .is_some_and(|child| child.kind() == SyntaxNodeKind::Statement)
            })
    }));
}

#[test]
fn reports_a_standalone_comma_and_recovers_at_the_next_statement() {
    let text = "fn inspect() {, let value = 1; value }";
    let parsed = parse(SourceFile::new("src/comma.rs", text));

    assert_eq!(parsed.coverage(), CoverageStatus::Partial);
    assert!(parsed.issues().iter().any(|issue| {
        issue.kind() == ParseIssueKind::UnexpectedToken
            && parsed
                .source()
                .slice(issue.start(), issue.end())
                .is_some_and(|token| token == ",")
    }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::Statement
            && parsed
                .source()
                .slice(node.start(), node.end())
                .is_some_and(|statement| statement == "let value = 1;")
    }));
}

#[test]
fn recovers_after_a_let_statement_missing_its_semicolon() {
    let text = "fn inspect() { let broken = 1 let kept = 2; kept }";
    let parsed = parse(SourceFile::new("src/let_recovery.rs", text));

    assert_eq!(parsed.coverage(), CoverageStatus::Partial);
    assert!(parsed.issues().iter().any(|issue| {
        issue.code() == cop_syntax::ParseIssueCode::UnexpectedToken
            && issue.start() == text.find("let kept").expect("后续 let")
    }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::Statement
            && parsed
                .source()
                .slice(node.start(), node.end())
                .is_some_and(|statement| statement == "let kept = 2;")
    }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.expression_syntax().is_some_and(|expression| {
            matches!(expression.kind(), ExpressionSyntaxKind::Path(path)
                if path.segments().first().is_some_and(|segment| segment.name() == "kept"))
        })
    }));
}

#[test]
fn recognizes_local_macro_definitions_before_later_statements() {
    let parsed = parse(SourceFile::new(
        "src/local_macro.rs",
        "fn inspect() { macro_rules! local { () => { 0 }; } let kept = 1; }",
    ));

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Partial,
        "{:?}",
        parsed.issues()
    );
    assert!(parsed
        .syntax_nodes()
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::MacroDefinition));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::Statement
            && parsed
                .source()
                .slice(node.start(), node.end())
                .is_some_and(|statement| statement == "let kept = 1;")
    }));
}

#[test]
fn statement_recovery_keeps_following_expression_after_a_match() {
    let parsed = parse(SourceFile::new(
        "src/recovery.rs",
        "fn inspect(value: u8) { match value { 0 => (), _ => 1 + 2 }; let kept = value.field?; }",
    ));

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "{:?}",
        parsed.issues()
    );
    assert!(parsed.syntax_nodes().iter().any(|node| {
        matches!(
            node.expression_syntax().map(|syntax| syntax.kind()),
            Some(ExpressionSyntaxKind::Try(_))
        )
    }));
    assert_eq!(
        parsed
            .syntax_nodes()
            .iter()
            .filter(|node| node.kind() == SyntaxNodeKind::MatchArm)
            .count(),
        2
    );
}

#[test]
fn retains_closure_parameter_and_return_types_with_parameter_attributes() {
    let text = "fn inspect() { let callback = |#[cfg(any())] value: &Input, fallback: Output| -> Result<Output, Error> { let local = value; fallback }; }";
    let parsed = parse(SourceFile::new("src/closures.rs", text));
    let closure = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.expression_syntax())
        .find_map(|expression| match expression.kind() {
            ExpressionSyntaxKind::Closure {
                parameters,
                return_type,
                ..
            } => Some((parameters, return_type)),
            _ => None,
        })
        .expect("闭包语法节点");

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "{:?}",
        parsed.issues()
    );
    assert_eq!(closure.0.len(), 2);
    assert_eq!(closure.0[0].attributes().len(), 1);
    assert!(matches!(
        closure.0[0].ty().map(|ty| ty.kind()),
        Some(TypeSyntaxKind::Reference { .. })
    ));
    assert!(closure.0[1].ty().is_some());
    assert!(matches!(
        closure.1.as_ref().map(|ty| ty.kind()),
        Some(TypeSyntaxKind::Path(path)) if path.segments()[0].name() == "Result"
    ));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::Parameter
            && node
                .attributes()
                .iter()
                .any(|attribute| attribute.meta().normalized() == "cfg(any())")
    }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::Block
            && node
                .children()
                .iter()
                .filter_map(|id| parsed.node(*id))
                .any(|child| child.kind() == SyntaxNodeKind::Statement)
    }));
}

#[test]
fn preserves_expression_attributes_and_marks_unstable_try_blocks_unsupported() {
    let text = r#"
fn inspect(value: Input) {
    let matched = match value {
        #![allow(dead_code)]
        #[cfg(any())] Input::Hidden => { let disabled = hidden_path; disabled }
        _ => (),
    };
    let record = Record { #[cfg(any())] field: hidden_field, visible };
    let future = async move { let inside_async = source.await; inside_async };
    let constant = const { let inside_const = 1; inside_const };
    let attempted = try { let inside_try = source?; inside_try };
    let labelled = 'scope: { let inside_label = source.field; inside_label };
    _ = (matched, record, future, constant, attempted, labelled);
}
"#;
    let parsed = parse(SourceFile::new("src/blocks.rs", text));

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Partial,
        "{:?}",
        parsed.issues()
    );
    let arm = parsed
        .syntax_nodes()
        .iter()
        .find(|node| node.kind() == SyntaxNodeKind::MatchArm)
        .expect("match arm 节点");
    assert!(
        arm.attributes()
            .iter()
            .any(|attribute| attribute.meta().normalized() == "cfg(any())"),
        "{:#?}",
        parsed.syntax_nodes()
    );
    assert!(arm
        .children()
        .iter()
        .filter_map(|id| parsed.node(*id))
        .filter(|node| node.kind() == SyntaxNodeKind::Pattern)
        .all(|node| node.attributes().is_empty()));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::Expression
            && node
                .attributes()
                .iter()
                .any(|attribute| attribute.meta().normalized() == "cfg(any())")
            && parsed
                .source()
                .slice(node.start(), node.end())
                .is_some_and(|source| source == "hidden_field")
    }));
    for name in ["inside_async", "inside_const", "inside_label"] {
        assert!(
            parsed.syntax_nodes().iter().any(|node| {
                node.kind() == SyntaxNodeKind::Statement
                    && parsed
                        .source()
                        .slice(node.start(), node.end())
                        .is_some_and(|source| source.contains(name))
            }),
            "缺少 block 内语句：{name}"
        );
    }
    assert!(parsed.syntax_nodes().iter().any(|node| {
        matches!(
            node.expression_syntax().map(|expression| expression.kind()),
            Some(ExpressionSyntaxKind::LabelBlock { label, .. }) if label == "'scope"
        )
    }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        matches!(
            node.expression_syntax().map(|expression| expression.kind()),
            Some(ExpressionSyntaxKind::Underscore)
        )
    }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        matches!(
            node.expression_syntax().map(|expression| expression.kind()),
            Some(ExpressionSyntaxKind::Unsupported(
                ExpressionUnsupportedKind::TryBlock
            ))
        )
    }));
    assert!(parsed
        .issues()
        .iter()
        .any(|issue| issue.kind() == ParseIssueKind::UnsupportedSyntax));
}

#[test]
fn unary_operators_bind_more_tightly_than_type_casts() {
    let parsed = parse(SourceFile::new(
        "src/precedence.rs",
        "fn inspect(value: i32) { let cast = -value as i64; }",
    ));
    let cast = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.expression_syntax())
        .find(|expression| {
            matches!(expression.kind(), ExpressionSyntaxKind::Cast { .. })
                && parsed
                    .source()
                    .slice(expression.bounds().start, expression.bounds().end)
                    .is_some_and(|source| source == "-value as i64")
        })
        .expect("cast expression");

    assert!(matches!(
        cast.kind(),
        ExpressionSyntaxKind::Cast { expression, .. }
            if matches!(expression.kind(), ExpressionSyntaxKind::Unary { operator, .. } if operator == "-")
    ));
}

#[test]
fn preserves_expression_attribute_ranges_and_recovers_match_arms_locally() {
    let text = "fn inspect(value: i32) { let block = #[cfg(any())] { value }; match value { 0 => 1 +, 1 => 2, _ => 3 }; let kept = 4; }";
    let parsed = parse(SourceFile::new("src/arm_recovery.rs", text));
    let attribute_start = text.find("#[cfg(any())]").expect("外层表达式属性");
    let attributed_expression = parsed.syntax_nodes().iter().find_map(|node| {
        let expression = node.expression_syntax()?;
        (expression.attributes().len() == 1).then_some(expression)
    });

    assert!(
        attributed_expression.is_some(),
        "issues: {:?}",
        parsed.issues()
    );
    let attributed_expression = attributed_expression.expect("属性表达式");
    assert_eq!(attributed_expression.bounds().start, attribute_start);
    assert!(text
        .get(attributed_expression.bounds())
        .is_some_and(|expression| expression.starts_with("#[cfg(any())]")));
    let parsed_match = parsed.syntax_nodes().iter().find_map(|node| {
        node.expression_syntax()
            .and_then(|expression| match expression.kind() {
                ExpressionSyntaxKind::Match { arms, .. } => Some(arms),
                _ => None,
            })
    });

    assert_eq!(parsed.coverage(), CoverageStatus::Partial);
    assert_eq!(parsed_match.map(Vec::len), Some(3));
    assert!(parsed
        .issues()
        .iter()
        .any(|issue| { issue.code() == cop_syntax::ParseIssueCode::UnexpectedToken }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::Statement
            && parsed
                .source()
                .slice(node.start(), node.end())
                .is_some_and(|statement| statement == "let kept = 4;")
    }));
    for node in parsed.syntax_nodes() {
        if let Some(parent) = node.parent().and_then(|id| parsed.node(id)) {
            assert!(
                parent.start() <= node.start() && node.end() <= parent.end(),
                "父节点 {:?} {}..{} 未包含子节点 {:?} {}..{}",
                parent.kind(),
                parent.start(),
                parent.end(),
                node.kind(),
                node.start(),
                node.end()
            );
        }
    }
}
