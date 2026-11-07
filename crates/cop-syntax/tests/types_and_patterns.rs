use cop_syntax::{
    CoverageStatus, ExpressionSyntaxKind, GenericArgumentSyntax, GenericParameterKind,
    ItemSyntaxKind, ParseIssueCode, ParseIssueKind, ParseOptions, ParseOutput, PatternSyntaxKind,
    RustParser, SourceFile, SyntaxNodeKind, TraitBoundKind, TypeSyntaxKind, WherePredicateKind,
};

fn parse(source: SourceFile) -> ParseOutput {
    RustParser::parse(source, ParseOptions::default()).expect("catalog messages render")
}

struct ExpectedDefault {
    item_name: &'static str,
    parameter_name: &'static str,
    default_type: &'static str,
    parameter_syntax: &'static str,
}

#[test]
fn builds_nested_type_paths_generics_and_signature_nodes() {
    let text = r#"
trait Convert<T> { type Output: Iterator<Item = T>; }
struct Shape<'a, T: Iterator<Item = (u8, u16)>, const N: usize> {
    values: &'a mut [Vec<Vec<T>>; N],
    callback: for<'b> unsafe extern "C" fn(&'b T) -> bool,
    object: &'a (dyn Fn(&'a T) -> T + Send),
}

type Qualified<T> = <T as Convert<u8>>::Output;
type MacroType = type_macro!(opaque);
struct Defaults<'a, 'b: 'a, T = u8, const N: usize = 4> {
    value: &'a T,
    marker: &'b (),
}
fn inspect<'a, T: Convert<u8>>(value: &'a T) -> <T as Convert<u8>>::Output {}
"#;
    let parsed = parse(SourceFile::new("src/lib.rs", text));
    let type_nodes = parsed
        .syntax_nodes()
        .iter()
        .filter(|node| node.kind() == SyntaxNodeKind::Type)
        .collect::<Vec<_>>();

    assert!(type_nodes.len() >= 6, "issues: {:?}", parsed.issues());
    assert!(type_nodes.iter().all(|node| {
        node.type_syntax().is_some_and(|syntax| {
            parsed
                .source()
                .slice(syntax.bounds().start, syntax.bounds().end)
                .is_some()
        })
    }));
    assert!(type_nodes.iter().any(|node| {
        matches!(
            node.type_syntax().map(|syntax| syntax.kind()),
            Some(TypeSyntaxKind::FunctionPointer { unsafe_: true, .. })
        )
    }));
    assert!(type_nodes.iter().any(|node| {
        matches!(
            node.type_syntax().map(|syntax| syntax.kind()),
            Some(TypeSyntaxKind::Macro { .. })
        )
    }));
    let nested_argument = type_nodes
        .iter()
        .filter_map(|node| node.type_syntax())
        .find_map(|syntax| match syntax.kind() {
            TypeSyntaxKind::Reference { target, .. } => match target.kind() {
                TypeSyntaxKind::Array { element, .. } => match element.kind() {
                    TypeSyntaxKind::Path(path) => match path.segments()[0].arguments().first() {
                        Some(GenericArgumentSyntax::Type(inner)) => Some(inner),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        })
        .expect("嵌套泛型类型参数");
    assert_eq!(
        parsed
            .source()
            .slice(nested_argument.bounds().start, nested_argument.bounds().end),
        Some("Vec<T>")
    );
    assert!(type_nodes.iter().any(|node| {
        matches!(
            node.type_syntax().map(|syntax| syntax.kind()),
            Some(TypeSyntaxKind::Reference { mutable: true, .. })
        )
    }));
    assert!(type_nodes.iter().any(|node| {
        matches!(
            node.type_syntax().map(|syntax| syntax.kind()),
            Some(TypeSyntaxKind::Path(path)) if path.qualified_self().is_some()
        )
    }));
    assert!(type_nodes.iter().any(|node| {
        matches!(
            node.type_syntax().map(|syntax| syntax.kind()),
            Some(TypeSyntaxKind::TraitBounds(bounds)) if !bounds.is_empty()
        )
    }));

    let generic_parameters = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.generic_parameter_syntax())
        .collect::<Vec<_>>();
    assert!(generic_parameters.iter().any(|parameter| {
        parameter.kind() == GenericParameterKind::Lifetime && parameter.name() == "'a"
    }));
    assert!(generic_parameters.iter().any(|parameter| {
        parameter.kind() == GenericParameterKind::Type
            && parameter.name() == "T"
            && !parameter.bounds().is_empty()
    }));
    assert!(generic_parameters.iter().any(|parameter| {
        parameter.kind() == GenericParameterKind::Const
            && parameter.name() == "N"
            && parameter.declared_type().is_some()
    }));
    assert!(generic_parameters.iter().any(|parameter| {
        parameter.kind() == GenericParameterKind::Lifetime
            && parameter.name() == "'b"
            && parameter.lifetime_bounds() == ["'a"]
    }));
    assert!(generic_parameters.iter().any(|parameter| {
        parameter.kind() == GenericParameterKind::Type
            && parameter.name() == "T"
            && parameter.default_type().is_some()
    }));
    assert!(generic_parameters.iter().any(|parameter| {
        parameter.kind() == GenericParameterKind::Const
            && parameter.name() == "N"
            && parameter.default_const().is_some()
    }));
}

#[test]
fn retains_generic_default_types_when_closers_share_a_token() {
    let text = r#"
type Alias<T = Local<u8>> = T;
type Multi<T = Local<u8>, U = Outer<Inner<u16>>> = T;
type Qualified<T = <<U as Trait>::Assoc as Other>::Item> = T;
type Bound<T: Trait<Assoc = Local<u8>> = Outer<Inner<u16>>> = T;
type Attributed<#[cfg(any())] T = Local<u8>> = T;
"#;
    let parsed = parse(SourceFile::new("src/generic_defaults.rs", text));
    let parameters = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.generic_parameter_syntax().map(|syntax| (node, syntax)))
        .collect::<Vec<_>>();

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "{:?}",
        parsed.issues()
    );
    assert!(parsed.issues().is_empty(), "{:?}", parsed.issues());
    assert_eq!(parameters.len(), 6);

    let expected = [
        ExpectedDefault {
            item_name: "Alias",
            parameter_name: "T",
            default_type: "Local<u8>",
            parameter_syntax: "T = Local<u8>",
        },
        ExpectedDefault {
            item_name: "Multi",
            parameter_name: "T",
            default_type: "Local<u8>",
            parameter_syntax: "T = Local<u8>",
        },
        ExpectedDefault {
            item_name: "Multi",
            parameter_name: "U",
            default_type: "Outer<Inner<u16>>",
            parameter_syntax: "U = Outer<Inner<u16>>",
        },
        ExpectedDefault {
            item_name: "Qualified",
            parameter_name: "T",
            default_type: "<<U as Trait>::Assoc as Other>::Item",
            parameter_syntax: "T = <<U as Trait>::Assoc as Other>::Item",
        },
        ExpectedDefault {
            item_name: "Bound",
            parameter_name: "T",
            default_type: "Outer<Inner<u16>>",
            parameter_syntax: "T: Trait<Assoc = Local<u8>> = Outer<Inner<u16>>",
        },
        ExpectedDefault {
            item_name: "Attributed",
            parameter_name: "T",
            default_type: "Local<u8>",
            parameter_syntax: "T = Local<u8>",
        },
    ];
    for ((node, parameter), expected) in parameters.iter().zip(expected) {
        assert_eq!(parameter.name(), expected.parameter_name);
        let parent_id = node.parent().expect("参数父节点");
        let item = parsed
            .syntax_nodes()
            .iter()
            .find(|candidate| candidate.id() == parent_id)
            .expect("父级类型别名");
        assert_eq!(item.kind(), SyntaxNodeKind::Item);
        assert!(parsed
            .source()
            .slice(item.start(), item.end())
            .is_some_and(|text| text.contains(expected.item_name)));
        assert!(item.start() <= node.start() && node.end() <= item.end());
        if expected.item_name == "Attributed" {
            assert_eq!(node.attributes().len(), 1);
            assert!(parsed
                .source()
                .slice(node.start(), node.end())
                .is_some_and(|text| text.starts_with("#[cfg(any())] T = Local<u8>")));
        } else {
            assert_eq!(
                parsed.source().slice(node.start(), node.end()),
                Some(expected.parameter_syntax)
            );
        }
        let default = parameter.default_type().expect("默认类型语法");
        assert_eq!(
            parsed
                .source()
                .slice(default.bounds().start, default.bounds().end),
            Some(expected.default_type)
        );
        let parameter_bounds = parameter.bounds_range();
        assert_eq!(
            parsed
                .source()
                .slice(parameter_bounds.start, parameter_bounds.end),
            Some(expected.parameter_syntax)
        );
    }
    let multi_u = parameters
        .iter()
        .find(|(_, parameter)| parameter.name() == "U")
        .map(|(_, parameter)| parameter.default_type().expect("U 默认类型"))
        .expect("U 参数");
    let TypeSyntaxKind::Path(outer) = multi_u.kind() else {
        panic!("外层默认类型应为路径");
    };
    assert!(matches!(
        outer.segments()[0].arguments().first(),
        Some(GenericArgumentSyntax::Type(inner))
            if matches!(inner.kind(), TypeSyntaxKind::Path(path)
                if path.segments()[0].name() == "Inner")
    ));
    let bound_parameter = parameters
        .iter()
        .find(|(_, parameter)| parameter.bounds().len() == 1)
        .expect("associated type bound");
    let TraitBoundKind::Trait(path) = bound_parameter.1.bounds()[0].kind() else {
        panic!("trait bound path");
    };
    assert_eq!(
        parsed
            .source()
            .slice(path.bounds().start, path.bounds().end),
        Some("Trait<Assoc = Local<u8>>")
    );
}

#[test]
fn reports_missing_generic_closer_and_keeps_later_aliases() {
    let text = "type Broken<T = Local<u8> = T;\ntype Kept<U = Outer<Inner<u8>>> = U;\n";
    let parsed = parse(SourceFile::new("src/generic_recovery.rs", text));

    assert_ne!(parsed.coverage(), CoverageStatus::Complete);
    assert!(parsed
        .issues()
        .iter()
        .any(|issue| issue.kind() == ParseIssueKind::UnexpectedToken));
    let kept = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.generic_parameter_syntax())
        .find(|parameter| parameter.name() == "U")
        .expect("错误后的泛型参数仍解析");
    assert_eq!(
        parsed.source().slice(
            kept.default_type().expect("U 默认类型").bounds().start,
            kept.default_type().expect("U 默认类型").bounds().end,
        ),
        Some("Outer<Inner<u8>>")
    );
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::GenericParameter
            && node.generic_parameter_syntax().is_none()
            && parsed
                .source()
                .slice(node.start(), node.end())
                .is_some_and(|text| text.starts_with("T = Local<u8>"))
    }));
}

#[test]
fn reports_empty_generic_parameter_segments_and_keeps_valid_parameters() {
    let text = "type Leading<, T = u8> = T;\ntype Interior<T,, U = u16> = U;\ntype Empty<> = ();\n";
    let parsed = parse(SourceFile::new("src/empty_generic_parameters.rs", text));
    let parameters = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.generic_parameter_syntax())
        .collect::<Vec<_>>();

    assert_eq!(parsed.coverage(), CoverageStatus::Partial);
    assert_eq!(
        parameters
            .iter()
            .map(|parameter| (parameter.name(), parameter.default_type().is_some()))
            .collect::<Vec<_>>(),
        vec![("T", true), ("T", false), ("U", true)]
    );

    let unexpected = parsed
        .issues()
        .iter()
        .filter(|issue| issue.kind() == ParseIssueKind::UnexpectedToken)
        .collect::<Vec<_>>();
    let empty_segment_commas = [
        text.find("<,").expect("首个空参数段") + 1,
        text.find(",,").expect("中间空参数段") + 1,
    ];
    for start in empty_segment_commas {
        assert!(
            unexpected
                .iter()
                .any(|issue| issue.start() == start && issue.end() == start + 1),
            "comma at {start}; issues: {unexpected:?}"
        );
    }
    let empty_list_close = text.find("<>").expect("空参数列表") + 1;
    assert!(unexpected
        .iter()
        .any(|issue| issue.start() == empty_list_close && issue.end() == empty_list_close + 1));
}

#[test]
fn keeps_braced_const_shift_expressions_inside_their_delimiters() {
    let text = "type ConstDefault<const N: usize = { 8 >> 1 }> = [u8; N];\ntype ConstArgument = Buffer<{ 8 >> 1 }>;\n";
    let parsed = parse(SourceFile::new("src/const_generic_shift.rs", text));

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "{:?}",
        parsed.issues()
    );
    assert!(parsed.issues().is_empty(), "{:?}", parsed.issues());
    let parameter = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.generic_parameter_syntax())
        .find(|parameter| parameter.name() == "N")
        .expect("const 泛型参数");
    let default = parameter.default_const().expect("const 默认表达式");
    assert_eq!(
        parsed.source().slice(default.start, default.end),
        Some("{ 8 >> 1 }")
    );
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.type_syntax().is_some_and(|syntax| {
            parsed
                .source()
                .slice(syntax.bounds().start, syntax.bounds().end)
                == Some("Buffer<{ 8 >> 1 }>")
        })
    }));
}

#[test]
fn recursively_models_patterns_and_keeps_macro_input_as_a_leaf() {
    let text = r#"
fn inspect(input: Value) {
    let Some((mut left @ ref right, 1..=4, [first, ..])) = input;
    match input {
        | Point { x: ref value, .. } | Point { x: value, .. } => {},
        Some(value) | None => {},
        pattern_macro!(opaque) => {},
    }
}
fn attributed((#[cfg(any())] visible, hidden): (Value, Value)) {}
"#;
    let parsed = parse(SourceFile::new("src/patterns.rs", text));
    let patterns = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.pattern_syntax())
        .collect::<Vec<_>>();
    assert!(patterns
        .iter()
        .any(|pattern| matches!(pattern.kind(), PatternSyntaxKind::TupleStruct { .. })));
    assert!(patterns
        .iter()
        .any(|pattern| matches!(pattern.kind(), PatternSyntaxKind::Struct { .. })));
    assert!(patterns
        .iter()
        .any(|pattern| matches!(pattern.kind(), PatternSyntaxKind::Macro { .. })));
    assert!(patterns
        .iter()
        .any(|pattern| matches!(pattern.kind(), PatternSyntaxKind::Or(_))));
    assert!(patterns.iter().any(|pattern| {
        matches!(pattern.kind(), PatternSyntaxKind::TupleStruct { elements, .. }
            if elements.iter().any(|element| matches!(element.kind(), PatternSyntaxKind::Tuple(_))))
    }));
    let nested_pattern = patterns
        .iter()
        .find_map(|pattern| match pattern.kind() {
            PatternSyntaxKind::TupleStruct { elements, .. } => elements.first(),
            _ => None,
        })
        .expect("let 模式的 tuple 子模式");
    let elements = match nested_pattern.kind() {
        PatternSyntaxKind::Tuple(elements) => elements,
        kind => panic!("预期 tuple 模式，得到 {kind:?}"),
    };
    assert!(matches!(
        elements[0].kind(),
        PatternSyntaxKind::Binding {
            mutable: true,
            subpattern: Some(_),
            ..
        }
    ));
    assert!(matches!(
        elements[1].kind(),
        PatternSyntaxKind::Range { .. }
    ));
    assert!(matches!(elements[2].kind(), PatternSyntaxKind::Slice(_)));
    assert!(patterns.iter().any(|pattern| {
        matches!(pattern.kind(), PatternSyntaxKind::Tuple(elements)
            if elements.first().is_some_and(|element| !element.attributes().is_empty()))
    }));
    assert!(parsed
        .issues()
        .iter()
        .any(|issue| { issue.code() == ParseIssueCode::OpaqueMacroInput }));
}

#[test]
fn parses_open_ended_integer_ranges_in_match_arms() {
    let text = r#"
fn char_len(byte: u8) -> usize {
    match byte {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    }
}
"#;
    let parsed = parse(SourceFile::new("src/open_ranges.rs", text));

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "issues: {:?}",
        parsed.issues()
    );
    assert!(parsed.issues().is_empty(), "issues: {:?}", parsed.issues());
    let match_arms = parsed
        .syntax_nodes()
        .iter()
        .find_map(
            |node| match node.expression_syntax().map(|syntax| syntax.kind()) {
                Some(ExpressionSyntaxKind::Match { arms, .. }) => Some(arms),
                _ => None,
            },
        )
        .expect("match 表达式");
    assert_eq!(match_arms.len(), 4);
    assert!(match_arms[3]
        .pattern()
        .is_some_and(|pattern| matches!(pattern.kind(), PatternSyntaxKind::Wildcard)));

    let range_patterns = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.pattern_syntax())
        .filter(|pattern| {
            matches!(
                pattern.kind(),
                PatternSyntaxKind::Range {
                    start: Some(_),
                    end: None,
                    inclusive: false,
                }
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(range_patterns.len(), 3, "patterns: {range_patterns:?}");
    for (pattern, expected) in range_patterns.iter().zip(["0xf0..", "0xe0..", "0xc0.."]) {
        let bounds = pattern.bounds();
        assert_eq!(
            parsed.source().slice(bounds.start, bounds.end),
            Some(expected)
        );
    }
}

#[test]
fn parses_open_ended_range_before_match_guard() {
    let text = r#"
fn char_len(byte: u8) -> usize {
    match byte {
        0xf0.. if byte > 0 => 4,
        _ => 1,
    }
}
"#;
    let parsed = parse(SourceFile::new("src/guarded_open_range.rs", text));

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "issues: {:?}",
        parsed.issues()
    );
    assert!(parsed.issues().is_empty(), "issues: {:?}", parsed.issues());
    let guarded_arm = parsed
        .syntax_nodes()
        .iter()
        .find_map(
            |node| match node.expression_syntax().map(|syntax| syntax.kind()) {
                Some(ExpressionSyntaxKind::Match { arms, .. }) => arms.first(),
                _ => None,
            },
        )
        .expect("带 guard 的 match arm");
    assert!(guarded_arm.guard().is_some());
    let pattern = guarded_arm.pattern().expect("guard arm 模式");
    assert!(matches!(
        pattern.kind(),
        PatternSyntaxKind::Range {
            start: Some(_),
            end: None,
            inclusive: false,
        }
    ));
    let bounds = pattern.bounds();
    assert_eq!(
        parsed.source().slice(bounds.start, bounds.end),
        Some("0xf0..")
    );
}

#[test]
fn recovers_from_a_malformed_type_without_consuming_the_next_function() {
    let text = "fn broken(value: Vec<) {}\nfn kept(value: Option<u8>) {}\n";
    let parsed = parse(SourceFile::new("src/recovery.rs", text));
    let kept = parsed.syntax_nodes().iter().find(|node| {
        node.item_syntax().is_some_and(|item| {
            matches!(item.kind(), ItemSyntaxKind::Function { name: Some(name), .. } if name == "kept")
        })
    });

    assert!(kept.is_some(), "{:?}", parsed.syntax_nodes());
    assert!(parsed
        .issues()
        .iter()
        .any(|issue| { issue.kind() == ParseIssueKind::UnexpectedToken }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.type_syntax().is_some_and(|syntax| {
            matches!(syntax.kind(), TypeSyntaxKind::Path(path)
                if path.segments().first().is_some_and(|segment| segment.name() == "Option"))
        })
    }));
}

#[test]
fn recovers_from_a_malformed_pattern_without_consuming_the_next_function() {
    let text = "fn broken(value: Value) { let (left, = value; }\nfn kept() {}\n";
    let parsed = parse(SourceFile::new("src/pattern_recovery.rs", text));
    let kept = parsed.syntax_nodes().iter().find(|node| {
        node.item_syntax().is_some_and(|item| {
            matches!(item.kind(), ItemSyntaxKind::Function { name: Some(name), .. } if name == "kept")
        })
    });

    assert!(kept.is_some(), "{:?}", parsed.syntax_nodes());
    assert!(parsed
        .issues()
        .iter()
        .any(|issue| { issue.kind() == ParseIssueKind::UnexpectedToken }));
}

#[test]
fn models_where_clauses_and_their_predicates_as_distinct_nodes() {
    let text = r#"
struct Wrapper<T>(T) where T: Clone + Send;
type Borrowed<'a, T> where T: 'a = &'a T;
trait HasItem where Self: Sized {
    type Item where Self: Sized;
}
impl<T> Wrapper<T> where T: Clone {}
fn make<'a, T>(value: &'a T) -> &'a T where 'a: 'static, T: Clone {}
fn universal<T>() where for<'a, 'b> (&'a T, &'b T): IntoIterator, T: Clone {}
"#;
    let parsed = parse(SourceFile::new("src/where_clauses.rs", text));
    let clauses = parsed
        .syntax_nodes()
        .iter()
        .filter(|node| node.kind() == SyntaxNodeKind::WhereClause)
        .collect::<Vec<_>>();
    let predicates = parsed
        .syntax_nodes()
        .iter()
        .filter(|node| node.kind() == SyntaxNodeKind::WherePredicate)
        .collect::<Vec<_>>();

    assert_eq!(clauses.len(), 7, "issues: {:?}", parsed.issues());
    assert!(predicates.len() >= 9, "issues: {:?}", parsed.issues());
    assert!(parsed.issues().is_empty(), "issues: {:?}", parsed.issues());
    assert!(clauses.iter().all(|clause| {
        clause.where_clause_syntax().is_some_and(|syntax| {
            parsed
                .source()
                .slice(syntax.bounds().start, syntax.bounds().end)
                .is_some_and(|text| text.starts_with("where "))
        })
    }));
    assert!(predicates.iter().all(|predicate| {
        predicate.parent().is_some_and(|parent| {
            parsed
                .node(parent)
                .is_some_and(|node| node.kind() == SyntaxNodeKind::WhereClause)
        }) && predicate.where_predicate_syntax().is_some()
    }));
    assert!(predicates.iter().any(|predicate| {
        matches!(
            predicate.where_predicate_syntax().map(|syntax| syntax.kind()),
            Some(WherePredicateKind::Lifetime { lifetime, bounds })
                if lifetime == "'a" && bounds == &["'static"]
        )
    }));
    assert!(predicates.iter().any(|predicate| {
        matches!(
            predicate.where_predicate_syntax().map(|syntax| syntax.kind()),
            Some(WherePredicateKind::Type { lifetimes, .. })
                if lifetimes == &["'a", "'b"]
        )
    }));
}

#[test]
fn reports_a_malformed_where_predicate_and_recovers_at_the_function_body() {
    let text = "fn broken<T>() where T: {}\nfn kept() {}\n";
    let parsed = parse(SourceFile::new("src/where_recovery.rs", text));
    let kept = parsed.syntax_nodes().iter().find(|node| {
        node.item_syntax().is_some_and(|item| {
            matches!(item.kind(), ItemSyntaxKind::Function { name: Some(name), .. } if name == "kept")
        })
    });

    assert!(kept.is_some(), "issues: {:?}", parsed.issues());
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::WhereClause
            && node.children().iter().any(|child| {
                parsed
                    .node(*child)
                    .is_some_and(|child_node| child_node.kind() == SyntaxNodeKind::Opaque)
            })
    }));
    assert!(parsed
        .issues()
        .iter()
        .any(|issue| issue.kind() == ParseIssueKind::UnexpectedToken));
}
