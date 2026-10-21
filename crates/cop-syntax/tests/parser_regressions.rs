use cop_syntax::{
    AttributeMetaArgument, AttributeMetaValueKind, CoverageStatus, ExpressionSyntaxKind,
    ForeignItemSyntaxKind, ItemSyntaxKind, LiteralKind, ParseIssueCode, ParseIssueKind,
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
fn rejects_let_conditions_outside_and_chains_in_2024() {
    let source = "fn inspect(ready: bool, visit: Visit) { if ready || let Some(field) = visit.target.field {} }";
    let parsed = parse(source, "2024");
    let let_start = source.find("let Some(field)").expect("let 条件位置");

    assert!(unexpected_issues(&parsed)
        .iter()
        .any(|(start, _)| *start == let_start));
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

#[test]
fn parses_nested_qualified_associated_types_without_spacing() {
    let source = r#"
trait Fs { type Dir; }
trait Platform { type Fs: Fs; }
struct TrashDirs<T>(T);
struct Error;

pub(crate) fn discover<P: Platform>(
    platform: &P,
    include_network: bool,
) -> Result<Vec<TrashDirs<<<P as Platform>::Fs as Fs>::Dir>>, Error> {}

pub(crate) fn project<P: Platform>() -> <<P as Platform>::Fs as Fs>::Dir {}
"#;
    let parsed = parse(source, "2021");
    let expected_type = "Result<Vec<TrashDirs<<<P as Platform>::Fs as Fs>::Dir>>, Error>";
    let direct_type = "<<P as Platform>::Fs as Fs>::Dir";

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "issues: {:?}",
        parsed.issues()
    );
    assert!(parsed.issues().is_empty(), "issues: {:?}", parsed.issues());
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.type_syntax().is_some_and(|syntax| {
            parsed
                .source()
                .slice(syntax.bounds().start, syntax.bounds().end)
                == Some(expected_type)
        })
    }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.type_syntax().is_some_and(|syntax| {
            parsed
                .source()
                .slice(syntax.bounds().start, syntax.bounds().end)
                == Some(direct_type)
        })
    }));
}

#[test]
fn parses_real_world_2024_let_chain_condition() {
    let source = r#"
fn inspect(mut visit: Visit, context: &mut Context, mut out: Output) {
    if visit.value.source() == ValueSource::SchemaDefault
        && visit.target.span.is_none()
        && let Some(field) = visit.target.field
    {
        visit.target.span = Some(
            TreeSupport::schema_call(
                &mut context.resources.schema_access_errors,
                |schema_out| field.span(schema_out),
                out.reborrow(),
            )?
            .into(),
        );
    }
}
"#;
    let parsed = parse(source, "2024");

    assert!(parsed.issues().is_empty(), "{:?}", parsed.issues());
}

#[test]
fn parses_real_world_let_binding_with_nested_generic_type() {
    let source = r#"
fn inspect(errors: Errors) -> TestResult<()> {
    let keys: Vec<_> = errors
        .diagnostics()
        .iter()
        .map(|d| -> TestResult<_> {
            d.path()
                .test_value()?
                .segments()
                .last()
                .test_value()?
                .as_map_key()
                .test_value()
        })
        .collect::<TestResult<Vec<_>>>()?;
    let _ = keys;
    Ok(())
}
"#;
    let parsed = parse(source, "2021");

    assert!(parsed.issues().is_empty(), "{:?}", parsed.issues());
}

#[test]
fn parses_real_world_let_binding_with_explicit_path_type() {
    let source = r#"
fn inspect(meta: Meta) -> TestResult<()> {
    let value: LitStr = meta.value()?.parse()?;
    Self::validate_name(&value.value(), value.span())?;
    result.rename = Some(value);
    Ok(())
}
"#;
    let parsed = parse(source, "2021");

    assert!(parsed.issues().is_empty(), "{:?}", parsed.issues());
}

#[test]
fn parses_explicit_let_type_inside_a_closure_if_branch() {
    let source = r#"
fn inspect(attribute: Attribute, result: &mut State) -> Result<()> {
    attribute.parse_nested_meta(|meta| {
        if meta.path.is_ident("rename") {
            if result.rename.is_some() {
                return Err(meta.error("rename cannot repeat"));
            }
            let value: LitStr = meta.value()?.parse()?;
            Self::validate_name(&value.value(), value.span())?;
            result.rename = Some(value);
        }
        Ok(())
    })?;
    Ok(())
}
"#;
    let parsed = parse(source, "2021");
    let colon = source.find("let value: LitStr").expect("显式类型绑定") + 9;
    let value_statement_count = parsed
        .syntax_nodes()
        .iter()
        .filter(|node| {
            node.kind() == SyntaxNodeKind::Statement
                && source.get(node.start()..node.end())
                    == Some("let value: LitStr = meta.value()?.parse()?;")
        })
        .count();

    assert!(
        !unexpected_issues(&parsed)
            .iter()
            .any(|(start, end)| *start == colon && *end == colon + 1),
        "{:?}",
        parsed.issues()
    );
    assert_eq!(value_statement_count, 1, "{:?}", parsed.syntax_nodes());
}

#[test]
fn parses_nested_generic_let_type_in_an_if_else_branch() {
    let source = r#"
fn map_failures_keep_source_order_and_diagnostic_budget_stops_later_reads() -> TestResult<()> {
    let schema = compile(&ALL_TYPES)?;
    let text = replace(&ALL_TYPES, "labels: {\"tier\": \"production\"}", "labels: {\"z\": \"\", \"a\": \"\"}")?;
    for budget in [0, 1, 2, 3] {
        let mut limits = ConfigLimits::default();
        limits = limits.with_max_diagnostics(budget);
        let mut provider = MemoryEnvironment::required(&ALL_TYPES);
        let errors = parse(&ALL_TYPES, &schema, &text, &mut provider, &limits).test_error()?;
        test_eq!(errors.diagnostics().len(), budget.clamp(1, 2))?;
        if budget <= 1 {
            assert_code(&errors, DiagnosticCode::CONFIG_VALIDATION_ERROR)?;
            test_assert!(errors.is_truncated())?;
            test_assert!(!provider.calls.contains_key("TYPOL_NOTIFY_TOKEN"))?;
        } else if budget == 2 {
            test_eq!(
                errors.diagnostics().last().test_value()?.code(),
                DiagnosticCode::DIAGNOSTIC_LIMIT_REACHED
            )?;
            test_assert!(errors.is_truncated())?;
            test_assert!(!provider.calls.contains_key("TYPOL_NOTIFY_TOKEN"))?;
        } else {
            assert_code(&errors, DiagnosticCode::CONFIG_VALIDATION_ERROR)?;
            test_assert!(!errors.is_truncated())?;
            let keys: Vec<_> = errors
                .diagnostics()
                .iter()
                .map(|d| -> TestResult<_> {
                    d.path()
                        .test_value()?
                        .segments()
                        .last()
                        .test_value()?
                        .as_map_key()
                        .test_value()
                })
                .collect::<TestResult<Vec<_>>>()?;
            test_eq!(keys, ["z", "a"])?;
            let positions: Vec<_> = errors
                .diagnostics()
                .iter()
                .map(|d| Ok(d.primary().test_value()?.span().start()))
                .collect::<TestResult<Vec<_>>>()?;
            test_assert!(TestCheck::item(&positions, 0)? < TestCheck::item(&positions, 1)?)?;
            test_eq!(
                provider
                    .calls
                    .get("TYPOL_NOTIFY_TOKEN")
                    .copied()
                    .test_value()?,
                1
            )?;
        }
    }
    Ok(())
}
"#;
    let parsed = parse(source, "2021");
    let colon = source.find("let keys: Vec<_>").expect("显式类型绑定") + 8;

    assert!(
        !unexpected_issues(&parsed)
            .iter()
            .any(|(start, end)| *start == colon && *end == colon + 1),
        "{:?}",
        parsed.issues()
    );
}

#[test]
fn parses_repr_align_attributes_on_global_and_local_types() {
    let source = r#"
#[repr(align(256))]
struct Aligned(u8);

fn local_type() {
    #[repr(align(64))]
    struct LocalAligned(u64);
}
"#;
    let parsed = parse(source, "2021");
    let repr_attributes = parsed
        .syntax_nodes()
        .iter()
        .flat_map(|node| node.attributes())
        .filter(|attribute| attribute.path() == ["repr"])
        .collect::<Vec<_>>();

    assert_eq!(repr_attributes.len(), 2, "{:?}", parsed.syntax_nodes());
    assert!(
        parsed
            .issues()
            .iter()
            .all(|issue| issue.kind() != ParseIssueKind::InvalidAttribute),
        "{:?}",
        parsed.issues()
    );
    assert_eq!(
        repr_attributes
            .iter()
            .map(|attribute| attribute.meta().normalized())
            .collect::<Vec<_>>(),
        ["repr(align(256))", "repr(align(64))"]
    );
    for (attribute, expected_value) in repr_attributes.iter().zip(["256", "64"]) {
        let [AttributeMetaArgument::Meta(align)] = attribute.meta().arguments() else {
            panic!("repr 列表应保留 align 元数据")
        };
        assert_eq!(align.path(), ["align"]);
        let [AttributeMetaArgument::Value(value)] = align.arguments() else {
            panic!("align 列表应保留直接值参数")
        };
        assert!(matches!(
            value.kind(),
            AttributeMetaValueKind::Literal {
                kind: LiteralKind::Number,
                ..
            }
        ));
        assert_eq!(source.get(value.bounds()), Some(expected_value));
    }
}

#[test]
fn keeps_attribute_diagnostics_for_missing_values_and_unclosed_groups() {
    let parsed = parse(
        "#[meta(,)] struct MissingListValue;\n#[meta(=1)] struct InvalidSeparator;\n#[meta =] struct MissingNamedValue;\n#[meta((1)] struct UnclosedGroup;",
        "2021",
    );
    let invalid_attributes = parsed
        .issues()
        .iter()
        .filter(|issue| issue.code() == ParseIssueCode::InvalidAttribute)
        .count();

    assert_eq!(invalid_attributes, 4, "{:?}", parsed.issues());
    assert!(parsed
        .issues()
        .iter()
        .any(|issue| issue.code() == ParseIssueCode::UnclosedDelimiter));
}

#[test]
fn parses_closure_parameters_with_generic_type_arguments() {
    let source = r#"
fn sort_fields() {
    slots.sort_by(
        |left, right, mut sort_out: Out<'_, '_, BuildError>| {
            let left_id = ty
                .record()
                .fields
                .get(*left)
                .ok_or(BudgetError::invariant(&INDEX_SORT_INVARIANT))
                .store_err_into(sort_out.reborrow())?;
            let right_id = ty
                .record()
                .fields
                .get(*right)
                .ok_or(BudgetError::invariant(&INDEX_SORT_INVARIANT))
                .store_err_into(sort_out.reborrow())?;
            let left = fields
                .get(left_id.index())
                .ok_or(BudgetError::invariant(&INDEX_SORT_INVARIANT))
                .store_err_into(sort_out.reborrow())?;
            let right = fields
                .get(right_id.index())
                .ok_or(BudgetError::invariant(&INDEX_SORT_INVARIANT))
                .store_err_into(sort_out.reborrow())?;
            let order = execution.compare_text(
                &left.record().name,
                &right.record().name,
                sort_out.reborrow(),
            )?;
            Ok(order)
        },
        out.reborrow(),
    )?;
}

fn sort_variants() {
    ordered.sort_by(
        |left, right, mut sort_out: Out<'_, '_, BuildError>| {
            let left_id = ty
                .record()
                .variants
                .get(*left)
                .ok_or(BudgetError::invariant(&VARIANT_INDEX_INVARIANT))
                .store_err_into(sort_out.reborrow())?;
            let right_id = ty
                .record()
                .variants
                .get(*right)
                .ok_or(BudgetError::invariant(&VARIANT_INDEX_INVARIANT))
                .store_err_into(sort_out.reborrow())?;
            let left = variant_records
                .get(left_id.index())
                .ok_or(BudgetError::invariant(&VARIANT_INDEX_INVARIANT))
                .store_err_into(sort_out.reborrow())?;
            let right = variant_records
                .get(right_id.index())
                .ok_or(BudgetError::invariant(&VARIANT_INDEX_INVARIANT))
                .store_err_into(sort_out.reborrow())?;
            execution.compare_text(
                &left.record().name,
                &right.record().name,
                sort_out.reborrow(),
            )
        },
        out.reborrow(),
    )?;
}

fn read_schema(kind: Kind) {
    let _ = |out: Out<'_, '_, HostError>| {
        Err(out.fail(HostError::new(HostOperation::SchemaRead, kind)))
    };
}
"#;
    let parsed = parse(source, "2021");
    let closures = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.expression_syntax())
        .filter_map(|expression| match expression.kind() {
            ExpressionSyntaxKind::Closure { parameters, .. } => Some(parameters.as_slice()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(
        parsed
            .issues()
            .iter()
            .all(|issue| issue.code() != ParseIssueCode::UnexpectedEof),
        "{:?}",
        parsed.issues()
    );
    assert_eq!(
        closures
            .iter()
            .map(|parameters| parameters.len())
            .collect::<Vec<_>>(),
        [3, 3, 1]
    );
    assert_eq!(
        closures
            .iter()
            .map(|parameters| {
                parameters
                    .last()
                    .and_then(|parameter| parameter.ty())
                    .and_then(|ty| source.get(ty.bounds()))
                    .unwrap_or("")
            })
            .collect::<Vec<_>>(),
        [
            "Out<'_, '_, BuildError>",
            "Out<'_, '_, BuildError>",
            "Out<'_, '_, HostError>"
        ]
    );
}

#[test]
fn splits_closure_parameters_after_nested_generic_closers() {
    let source = "fn inspect() { let _ = |value: Outer<Inner<Left, Right>>, tail: u8| (); }";
    let parsed = parse(source, "2021");
    let parameters = parsed
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.expression_syntax())
        .find_map(|expression| match expression.kind() {
            ExpressionSyntaxKind::Closure { parameters, .. } => Some(parameters),
            _ => None,
        })
        .expect("闭包参数");

    assert!(parsed
        .issues()
        .iter()
        .all(|issue| issue.code() != ParseIssueCode::UnexpectedEof));
    assert_eq!(parameters.len(), 2);
    assert_eq!(
        parameters
            .first()
            .and_then(|parameter| parameter.ty())
            .and_then(|ty| source.get(ty.bounds())),
        Some("Outer<Inner<Left, Right>>")
    );
}
