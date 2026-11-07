mod declaration_scope;
mod target_origin;
mod unnecessary_pub_crate;

use std::{borrow::Cow, ops::Range};

use cop_analysis::{
    AnalysisReference, AnalysisSnapshot, ExternalItemKind, ImplOwnerRecord, ItemKindFact,
    ModuleNode, PathResolutionKind, SymbolKind, SymbolVisibility,
};
use cop_config::ResolvedConfig;
use cop_i18n::{t, RenderError};
use cop_model::{AnalysisIssueKind, Diagnostic, RuleId, Severity};
use cop_syntax::{PathSyntax, TypeSyntax, TypeSyntaxKind};

use self::{
    declaration_scope::DeclarationScope,
    target_origin::{TypeOrigin, TypeOriginResolver},
};
use crate::{
    support::{RuleDiagnosticAdapter, RuleReport, SyntaxSupport},
    trait_impl_locality::{OrphanAnchor, TargetOrigin, TraitImplLocalityResolver, UnknownType},
};

pub(super) struct SemanticRuleSet;

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
        let trait_path = implementation.trait_syntax().map(path_segments);
        let trait_path = trait_path.as_deref();
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
            if DeclarationScope::cfg_exclusive_declaration_exists(
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
            if trait_resolution
                .external_item_identity()
                .is_some_and(|item| item.kind() != ExternalItemKind::Trait)
            {
                let path =
                    trait_path.map_or_else(|| "<unknown>".to_owned(), |path| path.join("::"));
                push_impl_analysis_issue(
                    module,
                    diagnostics,
                    trait_bounds,
                    t!("rule.impl_trait_unresolved", { trait_path: path.as_str() })?,
                );
                return Ok(());
            }
            None
        } else {
            let Some(trait_symbol) = trait_resolution
                .symbol()
                .filter(|symbol| symbol.kind() == SymbolKind::Trait)
            else {
                if DeclarationScope::cfg_exclusive_declaration_exists(
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
            TypeOriginResolver::resolution_conditions(trait_resolution, &mut trait_conditions);
            if !TypeOriginResolver::conditions_may_coexist_with_impl(
                implementation.condition(),
                &trait_conditions,
            ) {
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
        let mut resolver = TraitImplLocalityResolver::new(snapshot, module, implementation);
        if let Some(trait_symbol) = local_trait {
            match resolver.resolve_target() {
                TargetOrigin::Local(anchor) => {
                    if !TypeOriginResolver::conditions_may_coexist_with_impl(
                        implementation.condition(),
                        anchor.conditions(),
                    ) {
                        return Ok(());
                    }
                    if !DeclarationScope::symbol_is_in_module(anchor.symbol(), module) {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::TraitImplLocality,
                                t!("rule.impl_target_outside_scope", {
                                    target: anchor.symbol().name()
                                })?,
                                Some(anchor.bounds()),
                            ),
                        );
                    }
                }
                TargetOrigin::Other(conditions) => {
                    if !TypeOriginResolver::conditions_may_coexist_with_impl(
                        implementation.condition(),
                        &conditions,
                    ) {
                        return Ok(());
                    }
                    if !DeclarationScope::symbol_is_in_module(trait_symbol, module)
                        && !DeclarationScope::sealed_trait_exception(
                            snapshot,
                            module,
                            implementation,
                            trait_symbol,
                        )
                    {
                        let target =
                            implementation_target_name(module, implementation, &target_bounds);
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::TraitImplLocality,
                                t!("rule.impl_trait_definition_outside_scope", {
                                    trait_name: trait_symbol.name(),
                                    target: target.as_str()
                                })?,
                                Some(trait_bounds.clone()),
                            ),
                        );
                    }
                }
                TargetOrigin::Unknown(unknown) => {
                    if DeclarationScope::cfg_exclusive_declaration_exists(
                        snapshot,
                        module,
                        implementation
                            .target()
                            .and_then(type_path_segments)
                            .as_deref(),
                        implementation.condition(),
                        false,
                    ) {
                        return Ok(());
                    }
                    push_unknown_type_issue(module, diagnostics, implementation, unknown)?;
                }
            }
        } else {
            match resolver.resolve_orphan_anchor() {
                OrphanAnchor::Local(anchor) => {
                    if !TypeOriginResolver::conditions_may_coexist_with_impl(
                        implementation.condition(),
                        anchor.conditions(),
                    ) {
                        return Ok(());
                    }
                    if !DeclarationScope::symbol_is_in_module(anchor.symbol(), module) {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::TraitImplLocality,
                                t!("rule.impl_target_outside_scope", {
                                    target: anchor.symbol().name()
                                })?,
                                Some(anchor.bounds()),
                            ),
                        );
                    }
                }
                OrphanAnchor::NoAnchor => {
                    report_external_target_outside_scope(
                        module,
                        implementation,
                        target_bounds,
                        diagnostics,
                        adapter,
                    )?;
                }
                OrphanAnchor::Unknown(unknown) => {
                    if DeclarationScope::cfg_exclusive_declaration_exists(
                        snapshot,
                        module,
                        implementation
                            .target()
                            .and_then(type_path_segments)
                            .as_deref(),
                        implementation.condition(),
                        false,
                    ) {
                        return Ok(());
                    }
                    push_unknown_type_issue(module, diagnostics, implementation, unknown)?;
                }
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
            if DeclarationScope::cfg_exclusive_declaration_exists(
                snapshot,
                module,
                implementation
                    .target()
                    .and_then(type_path_segments)
                    .as_deref(),
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
        let target_origin = TypeOriginResolver::resolve(
            snapshot,
            resolution,
            &mut visited_aliases,
            &mut conditions,
        );
        if !TypeOriginResolver::conditions_may_coexist_with_impl(
            implementation.condition(),
            &conditions,
        ) {
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
                if !DeclarationScope::cfg_exclusive_declaration_exists(
                    snapshot,
                    module,
                    implementation
                        .target()
                        .and_then(type_path_segments)
                        .as_deref(),
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
        .or_else(|| {
            implementation
                .target()
                .and_then(type_path_segments)
                .map(|path| path.join("::"))
        })
        .unwrap_or_else(|| "<unknown>".to_owned())
}

fn report_external_target_outside_scope(
    module: &ModuleNode,
    implementation: &ImplOwnerRecord,
    bounds: Range<usize>,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) -> Result<(), RenderError> {
    let target = implementation_target_name(module, implementation, &bounds);
    adapter.push(
        module,
        diagnostics,
        RuleReport::new(
            RuleId::TraitImplLocality,
            t!("rule.impl_target_outside_scope", { target: target.as_str() })?,
            Some(bounds),
        ),
    );
    Ok(())
}

fn push_unknown_type_issue(
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    implementation: &ImplOwnerRecord,
    unknown: UnknownType,
) -> Result<(), RenderError> {
    let bounds = unknown.bounds();
    let target = module
        .parse_output()
        .source()
        .slice(bounds.start, bounds.end)
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| implementation_target_name(module, implementation, &bounds));
    let message = t!("rule.impl_target_unresolved", { target: target.as_str() })?;
    push_impl_analysis_issue(module, diagnostics, bounds, message);
    Ok(())
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

fn type_path_segments(ty: &TypeSyntax) -> Option<Vec<String>> {
    path_in_type(ty).map(path_segments)
}

fn path_in_type(ty: &TypeSyntax) -> Option<&PathSyntax> {
    match ty.kind() {
        TypeSyntaxKind::Path(path) if path.qualified_self().is_none() => Some(path),
        TypeSyntaxKind::Reference { target, .. }
        | TypeSyntaxKind::Pointer { target, .. }
        | TypeSyntaxKind::Parenthesized(target) => path_in_type(target),
        _ => None,
    }
}

fn push_unresolved_target_issue(
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    implementation: &ImplOwnerRecord,
    bounds: Range<usize>,
) -> Result<(), RenderError> {
    let target = implementation
        .target()
        .and_then(type_path_segments)
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
