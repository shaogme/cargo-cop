use cop_syntax::{
    CoverageStatus, ExpressionSyntaxKind, ItemSyntaxKind, ParseIssueKind, ParseOptions,
    ParseOutput, RustParser, SourceFile, SyntaxNodeKind, VisibilityKind,
};

fn parse(source: SourceFile) -> ParseOutput {
    RustParser::parse(source, ParseOptions::default())
}

#[test]
fn retains_cfg_attributes_on_declarations_and_function_body_nodes() {
    let text = "/// 记录；Record\r\npub struct Record {\r\n    #[cfg(any())]\r\n    pub inactive: u8,\r\n    #[cfg(mystery)]\r\n    pub possible: u16,\r\n}\r\npub struct Tuple(#[cfg(any())] pub u8, #[cfg(mystery)] pub u16);\r\npub enum State {\r\n    #[cfg(any())]\r\n    Gone,\r\n    #[cfg(mystery)]\r\n    Rich { #[cfg(any())] hidden: u8, active: u16 },\r\n}\r\npub fn inspect(#[cfg(any())] hidden: u8, #[cfg(mystery)] possible: u16) {\r\n    #![allow(dead_code)]\r\n    #[cfg(any())] let inactive = 0;\r\n    if true { #[cfg(mystery)] let conditional = 1; }\r\n    match 1 { #[cfg(any())] 0 => 0, #[cfg(mystery)] _ => 1, _ => 2 }\r\n}\r\n";
    let parsed = parse(SourceFile::new("src/lib.rs", text));

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "{:?}",
        parsed.issues()
    );
    let root = parsed
        .syntax_nodes()
        .iter()
        .find(|node| node.kind() == SyntaxNodeKind::Crate)
        .expect("crate 根节点");
    let items = root
        .children()
        .iter()
        .filter_map(|id| parsed.node(*id))
        .filter(|node| node.item_syntax().is_some())
        .collect::<Vec<_>>();
    assert_eq!(items.len(), 4);

    let record_item = match items[0].item_syntax().expect("结构体数据").kind() {
        ItemSyntaxKind::Struct(_) => items[0],
        kind => panic!("预期结构体，得到 {kind:?}"),
    };
    let record_node = record_item;
    let fields = record_node
        .children()
        .iter()
        .filter_map(|id| parsed.node(*id))
        .filter(|node| node.kind() == SyntaxNodeKind::Field)
        .collect::<Vec<_>>();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].attributes()[0].path(), ["cfg"]);
    assert_eq!(
        fields[0].field_visibility().expect("字段可见性").kind(),
        &VisibilityKind::Public
    );

    let state_item = match items[2].item_syntax().expect("枚举数据").kind() {
        ItemSyntaxKind::Enum(_) => items[2],
        kind => panic!("预期枚举，得到 {kind:?}"),
    };
    let state_node = state_item;
    let variants = state_node
        .children()
        .iter()
        .filter_map(|id| parsed.node(*id))
        .filter(|node| node.kind() == SyntaxNodeKind::EnumVariant)
        .collect::<Vec<_>>();
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0].enum_variant_name(), Some("Gone"));
    assert_eq!(
        variants[1]
            .children()
            .iter()
            .filter_map(|id| parsed.node(*id))
            .filter(|node| node.kind() == SyntaxNodeKind::Field)
            .count(),
        2
    );

    let inspect = match items[3].item_syntax().expect("函数数据").kind() {
        ItemSyntaxKind::Function { .. } => items[3],
        kind => panic!("预期函数，得到 {kind:?}"),
    };
    let inspect_node = inspect;
    let parameters = inspect_node
        .children()
        .iter()
        .filter_map(|id| parsed.node(*id))
        .filter(|node| node.kind() == SyntaxNodeKind::Parameter)
        .collect::<Vec<_>>();
    assert_eq!(parameters.len(), 2);
    assert_eq!(parameters[0].attributes()[0].path(), ["cfg"]);
    let body = inspect_node
        .children()
        .iter()
        .filter_map(|id| parsed.node(*id))
        .find(|node| node.kind() == SyntaxNodeKind::Block)
        .expect("函数体节点");
    assert_eq!(body.attributes().len(), 1);
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.attributes()
            .iter()
            .any(|attribute| attribute.meta().normalized().contains("any()"))
    }));
    assert!(parsed
        .syntax_nodes()
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::MatchArm));

    for (index, node) in parsed.syntax_nodes().iter().enumerate() {
        assert_eq!(node.id().index(), index);
        assert!(text.is_char_boundary(node.start()));
        assert!(text.is_char_boundary(node.end()));
        assert!(parsed.source().slice(node.start(), node.end()).is_some());
        if let Some(parent_id) = node.parent() {
            let parent = &parsed.syntax_nodes()[parent_id.index()];
            assert!(
                parent.start() <= node.start() && node.end() <= parent.end(),
                "parent {:?} {}..{}, child {:?} {}..{}",
                parent.kind(),
                parent.start(),
                parent.end(),
                node.kind(),
                node.start(),
                node.end()
            );
        }
        assert!(!node
            .attributes()
            .iter()
            .any(|attribute| { text.get(attribute.start()..attribute.end()).is_none() }));
    }

    assert!(parsed
        .syntax_nodes()
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::EnumVariant));
    assert!(parsed
        .syntax_nodes()
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::Field));
    assert!(parsed
        .syntax_nodes()
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::Parameter));
    assert!(parsed
        .syntax_nodes()
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::MatchArm));
}

#[test]
fn maps_nested_use_tree_attributes_to_their_original_byte_ranges() {
    let text = "use crate::{\r\n    #[cfg(any())] hidden,\r\n    visible::{#[cfg(mystery)] Leaf},\r\n};\r\n";
    let parsed = parse(SourceFile::new("src/lib.rs", text));

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "{:?}",
        parsed.issues()
    );
    let tree = parsed
        .syntax_nodes()
        .iter()
        .find(|node| node.kind() == SyntaxNodeKind::UseTree && node.parent().is_some())
        .expect("use 树根节点");
    assert_eq!(tree.children().len(), 2);
    let hidden = parsed.node(tree.children()[0]).expect("隐藏分支");
    assert_eq!(hidden.attributes()[0].path(), ["cfg"]);
    assert_eq!(
        parsed
            .source()
            .slice(hidden.attributes()[0].start(), hidden.attributes()[0].end()),
        Some("#[cfg(any())]")
    );
    let nested = parsed.node(tree.children()[1]).expect("嵌套分支");
    assert_eq!(nested.children().len(), 1);
    assert_eq!(
        parsed
            .node(nested.children()[0])
            .expect("叶节点")
            .attributes()[0]
            .meta()
            .normalized(),
        "cfg(mystery)"
    );
    assert!(
        parsed
            .syntax_nodes()
            .iter()
            .filter(|node| node.kind() == SyntaxNodeKind::UseTree)
            .count()
            >= 4
    );
}

#[test]
fn retains_block_use_trees_in_statement_nodes() {
    let text = "fn inspect() {\n    use crate::{#[cfg(any())] hidden::Item, visible::Item};\n}\n";
    let parsed = parse(SourceFile::new("src/lib.rs", text));
    let root = parsed
        .syntax_nodes()
        .iter()
        .find(|node| node.kind() == SyntaxNodeKind::Crate)
        .expect("crate 根节点");
    let function_node = root
        .children()
        .iter()
        .filter_map(|id| parsed.node(*id))
        .find(|node| {
            node.item_syntax()
                .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Function { .. }))
        })
        .expect("函数 AST 节点");
    let body = function_node
        .children()
        .iter()
        .filter_map(|id| parsed.node(*id))
        .find(|node| node.kind() == SyntaxNodeKind::Block)
        .expect("函数体节点");
    let import = parsed
        .syntax_nodes()
        .iter()
        .find(|node| node.use_tree().is_some() && node.start() >= body.start())
        .expect("块级 use 语句 AST");
    assert_eq!(import.children().len(), 2);
    let first_child = parsed.node(import.children()[0]).expect("第一个 use 分支");
    assert_eq!(
        parsed.source().slice(
            first_child.attributes()[0].start(),
            first_child.attributes()[0].end()
        ),
        Some("#[cfg(any())]")
    );
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::UseTree && node.start() == first_child.start()
    }));
}

#[test]
fn keeps_macro_input_opaque_and_marks_coverage_partial() {
    let text = "fn inspect() { opaque!({ #[cfg(hidden_macro_cfg)] let value = 0; }); }\n";
    let parsed = parse(SourceFile::new("src/lib.rs", text));
    assert_eq!(parsed.coverage(), CoverageStatus::Partial);
    assert!(parsed
        .issues()
        .iter()
        .any(|issue| issue.kind() == ParseIssueKind::UnsupportedSyntax));
    assert!(parsed
        .syntax_nodes()
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::Macro));
    assert!(!parsed.syntax_nodes().iter().any(|node| {
        node.attributes()
            .iter()
            .any(|attribute| attribute.meta().normalized().contains("hidden_macro_cfg"))
    }));
}

#[test]
fn represents_local_items_and_their_nested_blocks_in_the_node_graph() {
    let parsed = parse(SourceFile::new(
        "src/lib.rs",
        "fn outer() {\n    #[cfg(any())]\n    fn hidden() { let value = 0; }\n    struct Local;\n}\n",
    ));
    assert!(parsed
        .syntax_nodes()
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::LocalItem));
    assert!(parsed
        .syntax_nodes()
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::Block));
    assert_eq!(parsed.coverage(), CoverageStatus::Complete);
}

#[test]
fn represents_generic_parameters_and_patterns_with_local_attributes() {
    let text = "fn inspect<#[cfg(any())] T, U: Trait<Item = (u8, u16)>>(#[cfg(mystery)] (left, right): (T, U)) { let (first, second) = (left, right); match first { #[cfg(any())] 0 => (), _ => () } }";
    let parsed = parse(SourceFile::new("src/lib.rs", text));

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "{:?}",
        parsed.issues()
    );
    let generic_parameters = parsed
        .syntax_nodes()
        .iter()
        .filter(|node| node.kind() == SyntaxNodeKind::GenericParameter)
        .collect::<Vec<_>>();
    assert_eq!(generic_parameters.len(), 2);
    assert_eq!(generic_parameters[0].attributes()[0].path(), ["cfg"]);
    assert_eq!(
        parsed
            .source()
            .slice(generic_parameters[0].start(), generic_parameters[0].end()),
        Some("#[cfg(any())] T")
    );

    let patterns = parsed
        .syntax_nodes()
        .iter()
        .filter(|node| node.kind() == SyntaxNodeKind::Pattern)
        .collect::<Vec<_>>();
    assert_eq!(patterns.len(), 8);
    assert!(patterns.iter().any(|node| {
        parsed
            .source()
            .slice(node.start(), node.end())
            .is_some_and(|text| text == "(left, right)")
    }));
    assert!(patterns.iter().any(|node| {
        parsed
            .source()
            .slice(node.start(), node.end())
            .is_some_and(|text| text == "(first, second)")
    }));
    for pattern_text in ["(left, right)", "(first, second)"] {
        let tuple_pattern = patterns
            .iter()
            .find(|node| {
                parsed
                    .source()
                    .slice(node.start(), node.end())
                    .is_some_and(|text| text == pattern_text)
            })
            .expect("tuple pattern node");
        assert_eq!(tuple_pattern.children().len(), 2);
    }
    assert!(
        patterns.iter().any(|node| {
            parsed
                .source()
                .slice(node.start(), node.end())
                .is_some_and(|text| text == "0")
        }),
        "{patterns:#?}"
    );
}

#[test]
fn keeps_expression_attributes_and_models_field_access() {
    let text = "fn inspect(value: Value) { let _ = #[cfg(expr)] value.member; consume(#[cfg(inner)] value); }";
    let parsed = parse(SourceFile::new("src/lib.rs", text));

    assert_eq!(
        parsed.coverage(),
        CoverageStatus::Complete,
        "{:?}",
        parsed.issues()
    );
    assert!(!parsed
        .issues()
        .iter()
        .any(|issue| issue.kind() == ParseIssueKind::UnsupportedSyntax));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::Expression
            && node
                .attributes()
                .iter()
                .any(|attribute| attribute.meta().normalized() == "cfg(expr)")
    }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        node.kind() == SyntaxNodeKind::Expression
            && node
                .attributes()
                .iter()
                .any(|attribute| attribute.meta().normalized() == "cfg(inner)")
    }));
    assert!(parsed.syntax_nodes().iter().any(|node| {
        matches!(
            node.expression_syntax().map(|syntax| syntax.kind()),
            Some(ExpressionSyntaxKind::Field { field, .. }) if field == "member"
        )
    }));
}

#[test]
fn parses_local_function_generics_parameters_and_body() {
    let parsed = parse(SourceFile::new(
        "src/lib.rs",
        "fn outer() { fn local<T>(value: T) {} }",
    ));
    let local = parsed
        .syntax_nodes()
        .iter()
        .find(|node| {
            node.kind() == SyntaxNodeKind::LocalItem
                && node.item_syntax().is_some_and(|data| {
                    matches!(data.kind(), ItemSyntaxKind::Function { name: Some(name) } if name == "local")
                })
        })
        .expect("局部函数节点");
    let children = local
        .children()
        .iter()
        .filter_map(|id| parsed.node(*id))
        .collect::<Vec<_>>();

    assert!(children
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::GenericParameter));
    assert!(children
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::Parameter));
    assert!(children
        .iter()
        .any(|node| node.kind() == SyntaxNodeKind::Block));
    assert_eq!(parsed.coverage(), CoverageStatus::Complete);
}

#[test]
fn records_orphaned_block_attributes_as_opaque_and_partial() {
    let text = "fn inspect() {\n    #[cfg(hidden)]\n}\n";
    let parsed = parse(SourceFile::new("src/lib.rs", text));

    assert_eq!(parsed.coverage(), CoverageStatus::Partial);
    let opaque = parsed
        .syntax_nodes()
        .iter()
        .find(|node| {
            node.kind() == SyntaxNodeKind::Opaque
                && node
                    .attributes()
                    .iter()
                    .any(|attribute| attribute.meta().normalized() == "cfg(hidden)")
        })
        .expect("孤立属性对应的 opaque 节点");
    assert_eq!(
        parsed.source().slice(opaque.start(), opaque.end()),
        Some("#[cfg(hidden)]")
    );
    assert!(parsed.issues().iter().any(|issue| {
        issue.kind() == ParseIssueKind::UnexpectedToken
            && parsed
                .source()
                .slice(issue.start(), issue.end())
                .is_some_and(|source| source == "#[cfg(hidden)]")
    }));
}
