use cop_syntax::{
    GenericArgumentSyntax, GenericParameterKind, ParseOptions, ParseOutput, PatternSyntaxKind,
    RustParser, SourceFile, SyntaxNodeKind, TypeSyntaxKind, WherePredicateKind,
};

fn parse(source: SourceFile) -> ParseOutput {
    RustParser::parse(source, ParseOptions::default())
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
        .any(|issue| { issue.message().id().as_str() == "syntax-macro-opaque" }));
}

#[test]
fn recovers_from_a_malformed_type_without_consuming_the_next_function() {
    let text = "fn broken(value: Vec<) {}\nfn kept(value: Option<u8>) {}\n";
    let parsed = parse(SourceFile::new("src/recovery.rs", text));
    let kept = parsed.syntax_nodes().iter().find(|node| {
        node.item_syntax().is_some_and(|item| {
            matches!(item.kind(), cop_syntax::ItemSyntaxKind::Function { name: Some(name) } if name == "kept")
        })
    });

    assert!(kept.is_some(), "{:?}", parsed.syntax_nodes());
    assert!(parsed
        .issues()
        .iter()
        .any(|issue| { issue.kind() == cop_syntax::ParseIssueKind::UnexpectedToken }));
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
            matches!(item.kind(), cop_syntax::ItemSyntaxKind::Function { name: Some(name) } if name == "kept")
        })
    });

    assert!(kept.is_some(), "{:?}", parsed.syntax_nodes());
    assert!(parsed
        .issues()
        .iter()
        .any(|issue| { issue.kind() == cop_syntax::ParseIssueKind::UnexpectedToken }));
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
            matches!(item.kind(), cop_syntax::ItemSyntaxKind::Function { name: Some(name) } if name == "kept")
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
        .any(|issue| issue.kind() == cop_syntax::ParseIssueKind::UnexpectedToken));
}
