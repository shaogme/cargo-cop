use std::ops::Range;

use cop_analysis::{
    AnalysisSnapshot, ModuleNode, PathResolutionKind, SymbolKind, SymbolVisibility,
};
use cop_config::ResolvedConfig;
use cop_model::{
    AnalysisIssueKind, Diagnostic, LocalizedMessage, RuleId, Severity, CHILD_PUBLIC_FREE_FUNCTION,
    IMPL_CROSS_FILE, IMPL_EXTERNAL_TRAIT_TARGET, IMPL_INHERENT_TARGET, IMPL_TARGET_UNRESOLVED,
    NO_PARENT_GLOB_IMPORT, NO_PARENT_PRIVATE_PATH, PRIVATE_FIELDS_FOR_BEHAVIOR_TYPE,
};
use cop_syntax::{
    ItemSyntaxKind, PathSyntax, SyntaxNode, SyntaxNodeKind, TypeSyntax, TypeSyntaxKind,
    VisibilityKind,
};

use crate::support::{PathReference, RuleDiagnosticAdapter, RuleReport, SyntaxSupport};

pub(super) struct SemanticRuleSet;

struct ImplTarget {
    path: Vec<String>,
    trait_impl: bool,
    bounds: Range<usize>,
}

impl SemanticRuleSet {
    pub(super) fn run(
        snapshot: &AnalysisSnapshot,
        configuration: &ResolvedConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let adapter = RuleDiagnosticAdapter::new(configuration);
        for module in snapshot.modules() {
            if module.is_order_only() {
                continue;
            }
            Self::parent_private_paths(module, snapshot, diagnostics, &adapter);
            Self::impl_locality(module, snapshot, diagnostics, &adapter);
            Self::private_behavior_fields(module, snapshot, diagnostics, &adapter);
            Self::public_free_functions(module, diagnostics, &adapter);
        }
    }

    fn parent_private_paths(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        if module.is_crate_root() {
            return;
        }
        for item in module.item_nodes() {
            SyntaxSupport::for_each_path_reference(module, item, true, &mut |reference| {
                check_parent_path(reference, module, snapshot, diagnostics, adapter);
            });
            if item
                .item_syntax()
                .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Use))
            {
                let Some(tree) = SyntaxSupport::use_tree_node_for_item(module, item) else {
                    continue;
                };
                for leaf in SyntaxSupport::use_leaves(module, tree) {
                    if leaf.glob && leaf.path.first().is_some_and(|name| name == "super") {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::NoParentPrivateDependency,
                                LocalizedMessage::new(NO_PARENT_GLOB_IMPORT),
                                Some(leaf.start..leaf.end),
                            ),
                        );
                        continue;
                    }
                    check_parent_path(
                        &PathReference {
                            segments: leaf.path,
                            start: leaf.start,
                            end: leaf.end,
                            is_macro: false,
                        },
                        module,
                        snapshot,
                        diagnostics,
                        adapter,
                    );
                }
            }
        }
    }

    fn impl_locality(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for item in module.item_nodes() {
            if !item
                .item_syntax()
                .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Impl))
            {
                continue;
            }
            let Some(target) = parse_impl_target(module, item) else {
                continue;
            };
            let resolution = snapshot.symbols().resolve_path_at(
                module,
                &target.path,
                Some(target.bounds.clone()),
            );
            if let Some(symbol) = resolution.symbol() {
                if is_repository_type(symbol.kind()) {
                    let impl_condition = module
                        .configured_ast()
                        .condition_for_span(item.start(), item.end());
                    if !SyntaxSupport::conditions_may_coexist(
                        impl_condition,
                        symbol.cfg_condition(),
                    ) {
                        continue;
                    }
                    if symbol.file_path() != module.file_path() {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::ImplLocality,
                                LocalizedMessage::new(IMPL_CROSS_FILE)
                                    .with_text("target", symbol.name()),
                                Some(target.bounds),
                            ),
                        );
                    }
                    continue;
                }
            }
            if resolution.kind() == PathResolutionKind::ExternalCrate {
                let root = resolution
                    .external_crate()
                    .or_else(|| target.path.first().map(String::as_str))
                    .unwrap_or("");
                if target.trait_impl && is_allowed_external_root(root) {
                    continue;
                }
                if target.trait_impl {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::ImplLocality,
                            LocalizedMessage::new(IMPL_EXTERNAL_TRAIT_TARGET)
                                .with_text("root", root),
                            Some(target.bounds),
                        ),
                    );
                } else {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::ImplLocality,
                            LocalizedMessage::new(IMPL_INHERENT_TARGET),
                            Some(target.bounds),
                        ),
                    );
                }
                continue;
            }
            if target.trait_impl && is_builtin_type(target.path.last().map(String::as_str)) {
                continue;
            }
            if !target.trait_impl && is_builtin_type(target.path.last().map(String::as_str)) {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::ImplLocality,
                        LocalizedMessage::new(IMPL_INHERENT_TARGET),
                        Some(target.bounds),
                    ),
                );
                continue;
            }
            if matches!(
                resolution.kind(),
                PathResolutionKind::Unresolved
                    | PathResolutionKind::Ambiguous
                    | PathResolutionKind::AliasCycle
            ) {
                let mut issue = Diagnostic::analysis(
                    AnalysisIssueKind::ParserCoverage,
                    Severity::Warning,
                    LocalizedMessage::new(IMPL_TARGET_UNRESOLVED)
                        .with_text("target", target.path.join("::")),
                )
                .with_package(module.package_name())
                .with_package_manifest_path(module.package_manifest_path())
                .with_target(module.target_name())
                .with_target_kind(module.target_kind().to_vec())
                .with_namespace(format!("crate::{}", module.namespace().join("::")));
                if let Ok(span) = module
                    .parse_output()
                    .source()
                    .span(target.bounds.start, target.bounds.end)
                {
                    issue = issue.with_span(span);
                }
                diagnostics.push(issue);
            }
        }
    }

    fn private_behavior_fields(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for item_node in module.item_nodes() {
            let Some(item_data) = item_node.item_syntax() else {
                continue;
            };
            let (name, kind) = match item_data.kind() {
                ItemSyntaxKind::Struct(name) => (name.as_deref(), SymbolKind::Struct),
                ItemSyntaxKind::Enum(name) => (name.as_deref(), SymbolKind::Enum),
                ItemSyntaxKind::Union(name) => (name.as_deref(), SymbolKind::Union),
                _ => continue,
            };
            let Some(name) = name else {
                continue;
            };
            let type_condition = module
                .configured_ast()
                .condition_for_span(item_node.start(), item_node.end());
            let has_impl = snapshot.modules().iter().any(|candidate_module| {
                candidate_module.item_nodes().iter().any(|candidate| {
                    if !candidate
                        .item_syntax()
                        .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Impl))
                    {
                        return false;
                    }
                    let Some(target) = parse_impl_target(candidate_module, candidate) else {
                        return false;
                    };
                    let impl_condition = candidate_module
                        .configured_ast()
                        .condition_for_span(candidate.start(), candidate.end());
                    SyntaxSupport::conditions_may_coexist(type_condition, impl_condition)
                        && snapshot
                            .symbols()
                            .resolve_path_at(
                                candidate_module,
                                &target.path,
                                Some(target.bounds.clone()),
                            )
                            .symbol()
                            .is_some_and(|symbol| {
                                symbol.kind() == kind
                                    && symbol.name() == name
                                    && symbol.file_path() == module.file_path()
                                    && symbol.namespace() == module.namespace()
                            })
                })
            });
            if !has_impl {
                continue;
            }
            for field in public_fields(module, item_node) {
                if !module
                    .configured_ast()
                    .is_enabled_range(field.start(), field.end())
                    || !SyntaxSupport::conditions_may_coexist(
                        type_condition,
                        module
                            .configured_ast()
                            .condition_for_span(field.start(), field.end()),
                    )
                {
                    continue;
                }
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::PrivateFieldsForBehaviorTypes,
                        LocalizedMessage::new(PRIVATE_FIELDS_FOR_BEHAVIOR_TYPE)
                            .with_text("name", name),
                        Some(field.start()..field.end()),
                    ),
                );
            }
        }
    }

    fn public_free_functions(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        if module.is_crate_root() {
            return;
        }
        for item in module.item_nodes() {
            let is_public_function = item.item_syntax().is_some_and(|item_data| {
                matches!(item_data.kind(), ItemSyntaxKind::Function { .. })
                    && !matches!(item_data.visibility().kind(), VisibilityKind::Private)
            });
            if is_public_function {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::ChildPublicFreeFunctions,
                        LocalizedMessage::new(CHILD_PUBLIC_FREE_FUNCTION),
                        Some(item.start()..item.end()),
                    ),
                );
            }
        }
    }
}

fn check_parent_path(
    reference: &PathReference,
    module: &ModuleNode,
    snapshot: &AnalysisSnapshot,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    if !module
        .configured_ast()
        .is_enabled_range(reference.start, reference.end)
        || !module.configured_ast().is_supported_range(
            module.parse_output(),
            reference.start,
            reference.end,
        )
    {
        return;
    }
    if !matches!(
        reference.segments.first().map(String::as_str),
        Some("super" | "crate")
    ) {
        return;
    }
    let resolution = snapshot.symbols().resolve_path_at(
        module,
        &reference.segments,
        Some(reference.start..reference.end),
    );
    let Some(symbol) = resolution.symbol() else {
        return;
    };
    if !SyntaxSupport::conditions_may_coexist(
        module
            .configured_ast()
            .condition_for_span(reference.start, reference.end),
        symbol.cfg_condition(),
    ) {
        return;
    }
    if !matches!(symbol.visibility(), SymbolVisibility::Private)
        || symbol.namespace().len() >= module.namespace().len()
        || !module.namespace().starts_with(symbol.namespace())
    {
        return;
    }
    adapter.push(
        module,
        diagnostics,
        RuleReport::new(
            RuleId::NoParentPrivateDependency,
            LocalizedMessage::new(NO_PARENT_PRIVATE_PATH)
                .with_text("ancestor", symbol.namespace().join("::"))
                .with_text("item", symbol.name()),
            Some(reference.start..reference.end),
        ),
    );
}

fn parse_impl_target(module: &ModuleNode, item: &SyntaxNode) -> Option<ImplTarget> {
    let syntax = item.impl_syntax()?;
    let target = syntax.self_type()?;
    let path = impl_target_path(target)?;
    let target_bounds = target.bounds();
    if !module
        .configured_ast()
        .is_enabled_range(target_bounds.start, target_bounds.end)
        || !module.configured_ast().is_supported_range(
            module.parse_output(),
            target_bounds.start,
            target_bounds.end,
        )
    {
        return None;
    }
    Some(ImplTarget {
        path,
        trait_impl: syntax.is_trait_impl(),
        bounds: target_bounds,
    })
}

fn impl_target_path(ty: &TypeSyntax) -> Option<Vec<String>> {
    match ty.kind() {
        TypeSyntaxKind::Path(path) => {
            if path.qualified_self().is_some() {
                return None;
            }
            Some(path_segments(path))
        }
        TypeSyntaxKind::Reference { target, .. }
        | TypeSyntaxKind::Pointer { target, .. }
        | TypeSyntaxKind::Parenthesized(target) => impl_target_path(target),
        _ => None,
    }
}

fn path_segments(path: &PathSyntax) -> Vec<String> {
    let mut segments = path
        .segments()
        .iter()
        .map(|segment| segment.name().to_owned())
        .collect::<Vec<_>>();
    if path.has_leading_colon() {
        segments.insert(0, "::".to_owned());
    }
    segments
}

fn is_repository_type(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Union | SymbolKind::TypeAlias
    )
}

fn is_allowed_external_root(root: &str) -> bool {
    matches!(root, "std" | "core" | "alloc")
}

fn is_builtin_type(name: Option<&str>) -> bool {
    name.is_some_and(|name| {
        matches!(
            name,
            "bool"
                | "char"
                | "str"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "f32"
                | "f64"
                | "String"
                | "Vec"
                | "Option"
                | "Result"
                | "Box"
                | "Rc"
                | "Arc"
                | "Cell"
                | "RefCell"
                | "HashMap"
                | "HashSet"
                | "BTreeMap"
                | "BTreeSet"
                | "Path"
                | "PathBuf"
                | "Duration"
                | "SystemTime"
                | "Instant"
                | "Cow"
                | "Pin"
                | "Mutex"
                | "RwLock"
                | "Weak"
                | "PhantomData"
        )
    })
}

fn public_fields<'a>(module: &'a ModuleNode, item: &'a SyntaxNode) -> Vec<&'a SyntaxNode> {
    let output = module.parse_output();
    let mut fields = Vec::new();
    let mut pending = item.children().to_vec();
    while let Some(id) = pending.pop() {
        let Some(node) = output.node(id) else {
            continue;
        };
        if node.kind() == SyntaxNodeKind::Field
            && node.field_visibility().is_some_and(|visibility| {
                !matches!(visibility.kind(), VisibilityKind::Private)
                    && (visibility.start() == visibility.end()
                        || module.configured_ast().is_supported_range(
                            output,
                            visibility.start(),
                            visibility.end(),
                        ))
            })
        {
            fields.push(node);
        } else {
            pending.extend(node.children().iter().copied());
        }
    }
    fields
}
