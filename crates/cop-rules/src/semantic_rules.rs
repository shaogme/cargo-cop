mod unnecessary_pub_crate;

use std::{borrow::Cow, ops::Range};

use crate::support::{RuleDiagnosticAdapter, RuleReport, SyntaxSupport};
use cop_analysis::{
    AnalysisFacts, AnalysisReference, AnalysisSnapshot, DefinitionId, ImplOwnerRecord,
    ItemKindFact, ModuleNode, PathResolution, PathResolutionKind, PathRootKind, SymbolKind,
    SymbolRecord, SymbolVisibility,
};
use cop_config::ResolvedConfig;
use cop_i18n::{t, RenderError};
use cop_model::{AnalysisIssueKind, Diagnostic, RuleId, Severity};
use cop_syntax::ItemSyntaxKind;

pub(super) struct SemanticRuleSet;

enum TypeOrigin<'a> {
    Local(&'a SymbolRecord),
    External,
    Primitive,
}

impl SemanticRuleSet {
    pub(super) fn run(
        snapshot: &AnalysisSnapshot,
        configuration: &ResolvedConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Result<(), RenderError> {
        let adapter = RuleDiagnosticAdapter::new(configuration);
        for module in snapshot.modules() {
            if module.is_order_only() {
                continue;
            }
            Self::parent_private_paths(module, snapshot, diagnostics, &adapter)?;
            Self::inherent_impl_locality(module, snapshot, diagnostics, &adapter)?;
            Self::trait_impl_locality(module, snapshot, diagnostics, &adapter)?;
            Self::private_behavior_fields(module, snapshot, diagnostics, &adapter)?;
            Self::public_free_functions(snapshot, module, diagnostics, &adapter)?;
        }
        unnecessary_pub_crate::UnnecessaryPubCrateRule::run(snapshot, diagnostics, &adapter)?;
        Ok(())
    }

    fn parent_private_paths(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        if module.is_crate_root() {
            return Ok(());
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
                        t!("rule.no_parent_glob_import")?,
                        Some(bounds),
                    ),
                );
                continue;
            }
            check_parent_path(reference, module, diagnostics, adapter)?;
        }
        Ok(())
    }

    fn inherent_impl_locality(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        for implementation in snapshot.facts().impls_for_module(module) {
            if !implementation.is_target_enabled() {
                continue;
            }
            if !implementation.is_trait_impl() {
                Self::check_inherent_impl_locality(
                    module,
                    snapshot,
                    diagnostics,
                    adapter,
                    implementation,
                )?;
            }
        }
        Ok(())
    }

    fn trait_impl_locality(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        for implementation in snapshot.facts().impls_for_module(module) {
            if implementation.is_target_enabled() && implementation.is_trait_impl() {
                Self::check_trait_impl_locality(
                    module,
                    snapshot,
                    diagnostics,
                    adapter,
                    implementation,
                )?;
            }
        }
        Ok(())
    }

    fn check_trait_impl_locality(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
        implementation: &ImplOwnerRecord,
    ) -> Result<(), RenderError> {
        let trait_path = implementation.trait_path();
        let trait_bounds = implementation
            .trait_bounds()
            .unwrap_or_else(|| implementation.bounds());
        if !implementation.is_trait_enabled() {
            return Ok(());
        }
        if !implementation.is_trait_supported() {
            let path = trait_path.map_or_else(|| "<unknown>".to_owned(), |path| path.join("::"));
            push_impl_analysis_issue(
                module,
                diagnostics,
                trait_bounds,
                t!("rule.impl_trait_unresolved", { trait_path: path.as_str() })?,
            );
            return Ok(());
        }
        let Some(trait_reference) = snapshot.facts().reference_at(module, trait_bounds.clone())
        else {
            if cfg_exclusive_declaration_exists(
                snapshot,
                module,
                trait_path,
                implementation.condition(),
                true,
            ) {
                return Ok(());
            }
            let path = trait_path.map_or_else(|| "<unknown>".to_owned(), |path| path.join("::"));
            push_impl_analysis_issue(
                module,
                diagnostics,
                trait_bounds,
                t!("rule.impl_trait_unresolved", { trait_path: path.as_str() })?,
            );
            return Ok(());
        };
        let trait_resolution = trait_reference.resolution();
        let local_trait = if trait_resolution.kind() == PathResolutionKind::ExternalCrate {
            None
        } else {
            let Some(trait_symbol) = trait_resolution
                .symbol()
                .filter(|symbol| symbol.kind() == SymbolKind::Trait)
            else {
                if cfg_exclusive_declaration_exists(
                    snapshot,
                    module,
                    trait_path,
                    implementation.condition(),
                    true,
                ) {
                    return Ok(());
                }
                let path =
                    trait_path.map_or_else(|| "<unknown>".to_owned(), |path| path.join("::"));
                push_impl_analysis_issue(
                    module,
                    diagnostics,
                    trait_bounds,
                    t!("rule.impl_trait_unresolved", { trait_path: path.as_str() })?,
                );
                return Ok(());
            };
            let mut trait_conditions = Vec::new();
            resolution_conditions(trait_resolution, &mut trait_conditions);
            if !conditions_may_coexist_with_impl(implementation.condition(), &trait_conditions) {
                return Ok(());
            }
            Some(trait_symbol)
        };
        if !implementation.is_target_enabled() {
            return Ok(());
        }
        let target_bounds = implementation
            .target_bounds()
            .unwrap_or_else(|| implementation.bounds());
        if !implementation.is_target_supported() {
            push_unresolved_target_issue(module, diagnostics, implementation, target_bounds)?;
            return Ok(());
        }
        if implementation.is_target_structural() {
            return Ok(());
        }
        let Some(target_reference) = snapshot.facts().reference_at(module, target_bounds.clone())
        else {
            if cfg_exclusive_declaration_exists(
                snapshot,
                module,
                implementation.target_path(),
                implementation.condition(),
                false,
            ) {
                return Ok(());
            }
            push_unresolved_target_issue(module, diagnostics, implementation, target_bounds)?;
            return Ok(());
        };
        let mut target_conditions = Vec::new();
        let mut visited_aliases = Vec::new();
        let target_origin = resolve_type_origin(
            snapshot.facts(),
            target_reference.resolution(),
            &mut visited_aliases,
            &mut target_conditions,
        );
        if !conditions_may_coexist_with_impl(implementation.condition(), &target_conditions) {
            return Ok(());
        }
        match target_origin {
            Some(TypeOrigin::Local(target)) if !symbol_is_in_module(target, module) => {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::TraitImplLocality,
                        t!("rule.impl_target_outside_scope", { target: target.name() })?,
                        Some(target_bounds),
                    ),
                );
            }
            Some(TypeOrigin::External) => {
                if let Some(trait_symbol) = local_trait {
                    if !symbol_is_in_module(trait_symbol, module) {
                        let target =
                            implementation_target_name(module, implementation, &target_bounds);
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::TraitImplLocality,
                                t!("rule.impl_target_outside_scope", { target: target.as_str() })?,
                                Some(target_bounds),
                            ),
                        );
                    }
                } else {
                    let target = implementation_target_name(module, implementation, &target_bounds);
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::TraitImplLocality,
                            t!("rule.impl_target_outside_scope", { target: target.as_str() })?,
                            Some(target_bounds),
                        ),
                    );
                }
            }
            Some(TypeOrigin::Primitive) => {
                if local_trait
                    .is_some_and(|trait_symbol| !symbol_is_in_module(trait_symbol, module))
                {
                    let target = implementation_target_name(module, implementation, &target_bounds);
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::TraitImplLocality,
                            t!("rule.impl_target_outside_scope", { target: target.as_str() })?,
                            Some(target_bounds),
                        ),
                    );
                }
            }
            Some(TypeOrigin::Local(_)) => {}
            None => {
                if cfg_exclusive_declaration_exists(
                    snapshot,
                    module,
                    implementation.target_path(),
                    implementation.condition(),
                    false,
                ) {
                    return Ok(());
                }
                push_unresolved_target_issue(module, diagnostics, implementation, target_bounds)?;
            }
        }
        Ok(())
    }

    fn check_inherent_impl_locality(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
        implementation: &ImplOwnerRecord,
    ) -> Result<(), RenderError> {
        let bounds = implementation
            .target_bounds()
            .unwrap_or_else(|| implementation.bounds());
        if !implementation.is_target_supported() {
            push_unresolved_target_issue(module, diagnostics, implementation, bounds)?;
            return Ok(());
        }
        if implementation.is_target_structural() {
            adapter.push(
                module,
                diagnostics,
                RuleReport::new(
                    RuleId::ImplLocality,
                    t!("rule.impl_inherent_target")?,
                    Some(bounds),
                ),
            );
            return Ok(());
        }
        let Some(reference) = snapshot.facts().reference_at(module, bounds.clone()) else {
            if cfg_exclusive_declaration_exists(
                snapshot,
                module,
                implementation.target_path(),
                implementation.condition(),
                false,
            ) {
                return Ok(());
            }
            push_unresolved_target_issue(module, diagnostics, implementation, bounds)?;
            return Ok(());
        };
        let resolution = reference.resolution();
        let mut conditions = Vec::new();
        let mut visited_aliases = Vec::new();
        let target_origin = resolve_type_origin(
            snapshot.facts(),
            resolution,
            &mut visited_aliases,
            &mut conditions,
        );
        if !conditions_may_coexist_with_impl(implementation.condition(), &conditions) {
            return Ok(());
        }
        match target_origin {
            Some(TypeOrigin::Local(symbol)) => {
                if symbol.file_path() != module.file_path() {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::ImplLocality,
                            t!("rule.impl_cross_file", { target: symbol.name() })?,
                            Some(bounds),
                        ),
                    );
                }
            }
            Some(TypeOrigin::External) | Some(TypeOrigin::Primitive) => {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::ImplLocality,
                        t!("rule.impl_inherent_target")?,
                        Some(bounds),
                    ),
                );
            }
            None => {
                if !cfg_exclusive_declaration_exists(
                    snapshot,
                    module,
                    implementation.target_path(),
                    implementation.condition(),
                    false,
                ) {
                    push_unresolved_target_issue(module, diagnostics, implementation, bounds)?;
                }
            }
        }
        Ok(())
    }

    fn private_behavior_fields(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        let Some(scope) = snapshot
            .facts()
            .syntax_for_module(module)
            .and_then(|syntax| syntax.root_scope())
        else {
            return Ok(());
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
                        t!("rule.private_fields_for_behavior_type", { name: name })?,
                        Some(field.bounds()),
                    ),
                );
            }
        }
        Ok(())
    }

    fn public_free_functions(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        if module.is_crate_root() {
            return Ok(());
        }
        let Some(scope) = snapshot
            .facts()
            .syntax_for_module(module)
            .and_then(|syntax| syntax.root_scope())
        else {
            return Ok(());
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
                        t!("rule.child_public_free_function")?,
                        Some(item.bounds()),
                    ),
                );
            }
        }
        Ok(())
    }
}

fn check_parent_path(
    reference: &AnalysisReference,
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) -> Result<(), RenderError> {
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
        return Ok(());
    }
    if !matches!(
        reference.segments().first().map(String::as_str),
        Some("super" | "crate")
    ) {
        return Ok(());
    }
    let resolution = reference.resolution();
    let Some(symbol) = resolution.symbol() else {
        return Ok(());
    };
    if !SyntaxSupport::conditions_may_coexist(reference.condition(), symbol.cfg_condition()) {
        return Ok(());
    }
    if !matches!(symbol.visibility(), SymbolVisibility::Private)
        || symbol.namespace().len() >= module.namespace().len()
        || !module.namespace().starts_with(symbol.namespace())
    {
        return Ok(());
    }
    let ancestor = symbol.namespace().join("::");
    adapter.push(
        module,
        diagnostics,
        RuleReport::new(
            RuleId::NoParentPrivateDependency,
            t!("rule.no_parent_private_path", { ancestor: ancestor.as_str(), item: symbol.name() })?,
            Some(bounds),
        ),
    );
    Ok(())
}

fn is_repository_type(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Union | SymbolKind::TypeAlias
    )
}

fn resolve_type_origin<'a>(
    facts: &'a AnalysisFacts,
    resolution: &'a PathResolution,
    visited_aliases: &mut Vec<DefinitionId>,
    conditions: &mut Vec<Option<String>>,
) -> Option<TypeOrigin<'a>> {
    resolution_conditions(resolution, conditions);
    match resolution.kind() {
        PathResolutionKind::ExternalCrate => Some(TypeOrigin::External),
        PathResolutionKind::Local => {
            if resolution.root().kind() == PathRootKind::Primitive {
                return Some(TypeOrigin::Primitive);
            }
            let symbol = resolution.symbol()?;
            if symbol.kind() == SymbolKind::TypeAlias {
                let definition = symbol.definition_id();
                if visited_aliases.contains(&definition) {
                    return None;
                }
                visited_aliases.push(definition);
                let target = facts.type_alias_target(definition).and_then(|target| {
                    resolve_type_origin(facts, target, visited_aliases, conditions)
                });
                visited_aliases.pop();
                return target;
            }
            is_repository_type(symbol.kind()).then_some(TypeOrigin::Local(symbol))
        }
        PathResolutionKind::Unresolved
        | PathResolutionKind::Ambiguous
        | PathResolutionKind::AliasCycle => None,
    }
}

fn resolution_conditions(resolution: &PathResolution, conditions: &mut Vec<Option<String>>) {
    if let Some(symbol) = resolution.symbol() {
        conditions.push(symbol.cfg_condition().map(str::to_owned));
    }
    conditions.extend(
        resolution
            .alias_symbols()
            .iter()
            .map(|symbol| symbol.cfg_condition().map(str::to_owned)),
    );
}

fn conditions_may_coexist_with_impl(
    implementation_condition: Option<&str>,
    symbol_conditions: &[Option<String>],
) -> bool {
    let mut conditions = Vec::with_capacity(symbol_conditions.len() + 1);
    conditions.push(implementation_condition);
    conditions.extend(symbol_conditions.iter().map(Option::as_deref));
    SyntaxSupport::conditions_may_coexist_all(&conditions)
}

fn implementation_target_name(
    module: &ModuleNode,
    implementation: &ImplOwnerRecord,
    bounds: &Range<usize>,
) -> String {
    module
        .parse_output()
        .source()
        .slice(bounds.start, bounds.end)
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .map(str::to_owned)
        .or_else(|| implementation.target_path().map(|path| path.join("::")))
        .unwrap_or_else(|| "<unknown>".to_owned())
}

fn symbol_is_in_module(symbol: &SymbolRecord, module: &ModuleNode) -> bool {
    symbol.context_index() == module.context_index() && symbol.namespace() == module.namespace()
}

fn cfg_exclusive_declaration_exists(
    snapshot: &AnalysisSnapshot,
    module: &ModuleNode,
    path: Option<&[String]>,
    implementation_condition: Option<&str>,
    trait_declaration: bool,
) -> bool {
    let Some(path) = path else {
        return false;
    };
    let Some(name) = path.last().map(String::as_str) else {
        return false;
    };
    let namespaces = declaration_namespaces(module, path);
    let declarations = snapshot
        .modules()
        .iter()
        .filter(|candidate_module| {
            candidate_module.context_index() == module.context_index()
                && namespaces.contains(&candidate_module.namespace().to_vec())
        })
        .flat_map(|candidate_module| {
            candidate_module
                .source_item_nodes()
                .into_iter()
                .filter_map(move |node| {
                    let item = node.item_syntax()?;
                    let declaration_name = match item.kind() {
                        ItemSyntaxKind::Trait(name) if trait_declaration => name.as_deref(),
                        ItemSyntaxKind::Struct(name)
                        | ItemSyntaxKind::Enum(name)
                        | ItemSyntaxKind::Union(name)
                        | ItemSyntaxKind::TypeAlias(name)
                            if !trait_declaration =>
                        {
                            name.as_deref()
                        }
                        _ => None,
                    };
                    (declaration_name == Some(name)).then(|| {
                        candidate_module
                            .configured_ast()
                            .node(node.id())
                            .and_then(|configured| configured.availability_condition())
                            .map(str::to_owned)
                    })
                })
        })
        .collect::<Vec<_>>();
    !declarations.is_empty()
        && declarations.iter().all(|condition| {
            !SyntaxSupport::conditions_may_coexist(implementation_condition, condition.as_deref())
        })
}

fn declaration_namespaces(module: &ModuleNode, path: &[String]) -> Vec<Vec<String>> {
    let Some((_, prefixes)) = path.split_last() else {
        return Vec::new();
    };
    let mut base = module.namespace().to_vec();
    let mut rest = prefixes;
    let mut search_ancestors = false;
    match rest.first().map(String::as_str) {
        Some("crate") | Some("::") => {
            base.clear();
            rest = &rest[1..];
        }
        Some("self") => rest = &rest[1..],
        Some("super") => {
            while rest.first().is_some_and(|segment| segment == "super") {
                if base.pop().is_none() {
                    return Vec::new();
                }
                rest = &rest[1..];
            }
        }
        _ => search_ancestors = true,
    }
    let mut namespaces = Vec::new();
    let first_depth = if search_ancestors {
        module.namespace().len()
    } else {
        base.len()
    };
    let last_depth = if search_ancestors { 0 } else { base.len() };
    for depth in (last_depth..=first_depth).rev() {
        let mut namespace = if search_ancestors {
            module.namespace()[..depth].to_vec()
        } else {
            base.clone()
        };
        namespace.extend(rest.iter().cloned());
        if !namespaces.contains(&namespace) {
            namespaces.push(namespace);
        }
    }
    namespaces
}

fn push_unresolved_target_issue(
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    implementation: &ImplOwnerRecord,
    bounds: Range<usize>,
) -> Result<(), RenderError> {
    let target = implementation
        .target_path()
        .map(|path| path.join("::"))
        .unwrap_or_else(|| "<unknown>".to_owned());
    let message = t!("rule.impl_target_unresolved", { target: target.as_str() })?;
    push_impl_analysis_issue(module, diagnostics, bounds, message);
    Ok(())
}

fn push_impl_analysis_issue(
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    bounds: Range<usize>,
    message: Cow<'static, str>,
) {
    let mut issue = Diagnostic::analysis(
        AnalysisIssueKind::ParserCoverage,
        Severity::Warning,
        message,
    )
    .with_package(module.package_name())
    .with_package_manifest_path(module.package_manifest_path())
    .with_target(module.target_name())
    .with_target_kind(module.target_kind().to_vec())
    .with_target_triple(module.target_triple())
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
