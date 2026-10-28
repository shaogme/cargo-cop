mod unnecessary_pub_crate;

use crate::support::{RuleDiagnosticAdapter, RuleReport, SyntaxSupport};
use cop_analysis::{
    AnalysisReference, AnalysisSnapshot, ItemKindFact, ModuleNode, PathResolutionKind, SymbolKind,
    SymbolVisibility,
};
use cop_config::ResolvedConfig;
use cop_model::{
    AnalysisIssueKind, Diagnostic, LocalizedMessage, RuleId, Severity, CHILD_PUBLIC_FREE_FUNCTION,
    IMPL_CROSS_FILE, IMPL_EXTERNAL_TRAIT_TARGET, IMPL_INHERENT_TARGET, IMPL_TARGET_UNRESOLVED,
    NO_PARENT_GLOB_IMPORT, NO_PARENT_PRIVATE_PATH, PRIVATE_FIELDS_FOR_BEHAVIOR_TYPE,
};

pub(super) struct SemanticRuleSet;

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
            Self::public_free_functions(snapshot, module, diagnostics, &adapter);
        }
        unnecessary_pub_crate::UnnecessaryPubCrateRule::run(snapshot, diagnostics, &adapter);
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
        for reference in snapshot.facts().references_for_module(module) {
            let bounds = reference.bounds();
            if reference.is_glob()
                && reference
                    .segments()
                    .first()
                    .is_some_and(|name| name == "super")
            {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::NoParentPrivateDependency,
                        LocalizedMessage::new(NO_PARENT_GLOB_IMPORT),
                        Some(bounds),
                    ),
                );
                continue;
            }
            check_parent_path(reference, module, diagnostics, adapter);
        }
    }

    fn impl_locality(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for implementation in snapshot.facts().impls_for_module(module) {
            if !implementation.is_target_supported() {
                continue;
            }
            let Some(bounds) = implementation.target_bounds() else {
                continue;
            };
            let Some(path) = implementation.target_path() else {
                continue;
            };
            let Some(reference) = snapshot.facts().reference_at(module, bounds.clone()) else {
                continue;
            };
            let resolution = reference.resolution();
            if let Some(symbol) = resolution.symbol() {
                if is_repository_type(symbol.kind()) {
                    if !SyntaxSupport::conditions_may_coexist(
                        implementation.condition(),
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
                                Some(bounds),
                            ),
                        );
                    }
                    continue;
                }
            }
            if resolution.kind() == PathResolutionKind::ExternalCrate {
                let root = resolution
                    .external_crate()
                    .or_else(|| path.first().map(String::as_str))
                    .unwrap_or("");
                if implementation.is_trait_impl() && is_allowed_external_root(root) {
                    continue;
                }
                if implementation.is_trait_impl() {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::ImplLocality,
                            LocalizedMessage::new(IMPL_EXTERNAL_TRAIT_TARGET)
                                .with_text("root", root),
                            Some(bounds),
                        ),
                    );
                } else {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::ImplLocality,
                            LocalizedMessage::new(IMPL_INHERENT_TARGET),
                            Some(bounds),
                        ),
                    );
                }
                continue;
            }
            if implementation.is_trait_impl() && is_builtin_type(path.last().map(String::as_str)) {
                continue;
            }
            if !implementation.is_trait_impl() && is_builtin_type(path.last().map(String::as_str)) {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::ImplLocality,
                        LocalizedMessage::new(IMPL_INHERENT_TARGET),
                        Some(bounds),
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
                        .with_text("target", path.join("::")),
                )
                .with_package(module.package_name())
                .with_package_manifest_path(module.package_manifest_path())
                .with_target(module.target_name())
                .with_target_kind(module.target_kind().to_vec())
                .with_namespace(format!("crate::{}", module.namespace().join("::")));
                if let Ok(span) = module
                    .parse_output()
                    .source()
                    .span(bounds.start, bounds.end)
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
        let Some(scope) = snapshot
            .facts()
            .syntax_for_module(module)
            .and_then(|syntax| syntax.root_scope())
        else {
            return;
        };
        for item in scope.items().iter().filter(|item| item.is_enabled()) {
            if !matches!(
                item.kind(),
                ItemKindFact::Struct | ItemKindFact::Enum | ItemKindFact::Union
            ) {
                continue;
            }
            let Some(name) = item.name() else {
                continue;
            };
            let type_condition = item.condition();
            let definition = snapshot.facts().definition_at(module, item.bounds());
            let has_impl = definition.is_some_and(|definition| {
                snapshot
                    .facts()
                    .has_impl_for(definition.definition_id(), type_condition)
            });
            if !has_impl {
                continue;
            }
            for field in item.public_fields() {
                if !field.is_enabled()
                    || !SyntaxSupport::conditions_may_coexist(type_condition, field.condition())
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
                        Some(field.bounds()),
                    ),
                );
            }
        }
    }

    fn public_free_functions(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        if module.is_crate_root() {
            return;
        }
        let Some(scope) = snapshot
            .facts()
            .syntax_for_module(module)
            .and_then(|syntax| syntax.root_scope())
        else {
            return;
        };
        for item in scope.items().iter().filter(|item| item.is_enabled()) {
            let is_public_function = matches!(item.kind(), ItemKindFact::Function)
                && !matches!(item.visibility(), SymbolVisibility::Private);
            if is_public_function {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::ChildPublicFreeFunctions,
                        LocalizedMessage::new(CHILD_PUBLIC_FREE_FUNCTION),
                        Some(item.bounds()),
                    ),
                );
            }
        }
    }
}

fn check_parent_path(
    reference: &AnalysisReference,
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    let bounds = reference.bounds();
    if !module
        .configured_ast()
        .is_enabled_range(bounds.start, bounds.end)
        || !module.configured_ast().is_supported_range(
            module.parse_output(),
            bounds.start,
            bounds.end,
        )
    {
        return;
    }
    if !matches!(
        reference.segments().first().map(String::as_str),
        Some("super" | "crate")
    ) {
        return;
    }
    let resolution = reference.resolution();
    let Some(symbol) = resolution.symbol() else {
        return;
    };
    if !SyntaxSupport::conditions_may_coexist(reference.condition(), symbol.cfg_condition()) {
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
            Some(bounds),
        ),
    );
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
