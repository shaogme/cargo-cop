use std::collections::{BTreeMap, BTreeSet};

use cop_syntax::{
    AstNodeId, ItemSyntaxKind, MacroInvocation, MacroProfileBinding, MacroProfileKey, ParseOutput,
    PathSyntax, SyntaxNode, VisibilityKind,
};

use super::index::{IndexedMacroUseKind, IndexedUse, MacroResolverCatalog, MacroResolverIndex};
use crate::{cfg_eval::CfgDecision, module_graph::ModuleNode, ConfiguredAst};

#[derive(Clone, Eq, PartialEq)]
struct MacroIdentity {
    crate_root: String,
    name: String,
}

#[derive(Clone)]
struct ResolvedMacroProfile {
    identity: MacroIdentity,
    profile: MacroProfileKey,
}

impl ResolvedMacroProfile {
    fn new(crate_root: &str, name: &str, profile: MacroProfileKey) -> Self {
        Self {
            identity: MacroIdentity {
                crate_root: crate_root.to_owned(),
                name: name.to_owned(),
            },
            profile,
        }
    }
}

#[derive(Clone)]
enum ImportedProfile {
    Absent,
    Blocked,
    Resolved(ResolvedMacroProfile),
}

enum RootAliasResolution {
    Absent,
    Shadowed,
    Crate(String),
}

#[derive(Clone, Copy)]
struct MacroLookupContext<'a> {
    output: &'a ParseOutput,
    index: &'a MacroResolverIndex,
    catalog: &'a MacroResolverCatalog,
    configured_ast: &'a ConfiguredAst,
    module: &'a ModuleNode,
    modules: &'a [ModuleNode],
    no_std: bool,
}

#[derive(Clone, Copy)]
struct MacroProfileContext<'a> {
    lookup: MacroLookupContext<'a>,
    no_std: bool,
    edition: &'a str,
}

pub(super) struct MacroProfileResolver;

impl MacroProfileResolver {
    pub(super) fn profile_bindings(
        instance_modules: &[&ModuleNode],
        modules: &[ModuleNode],
        catalog: &MacroResolverCatalog,
        edition: &str,
    ) -> Vec<MacroProfileBinding> {
        let Some(first_module) = instance_modules.first() else {
            return Vec::new();
        };
        let output = first_module.parse_output();
        let Some(index) = catalog.index(first_module.source_instance()) else {
            return Vec::new();
        };
        let mut imported_profiles = BTreeMap::new();
        index
            .macro_invocations()
            .iter()
            .filter_map(|node_id| {
                let node = output.node(*node_id)?;
                let invocation = node.macro_invocation()?;
                let scopes = index.scope_ids(node.id());
                let context_module = Self::module_for_invocation(instance_modules, &scopes);
                let configured_ast = context_module.configured_ast();
                let no_std = modules.iter().any(|candidate| {
                    candidate.context_index() == context_module.context_index()
                        && candidate.is_crate_root()
                        && candidate.is_no_std()
                });
                let lookup = MacroLookupContext {
                    output,
                    index,
                    catalog,
                    configured_ast,
                    module: context_module,
                    modules,
                    no_std,
                };
                let context = MacroProfileContext {
                    lookup,
                    no_std,
                    edition,
                };
                Self::resolve_profile(context, node, invocation, &mut imported_profiles).map(
                    |profile| {
                        MacroProfileBinding::new(invocation.start()..invocation.end(), profile)
                    },
                )
            })
            .collect()
    }

    fn resolve_profile(
        context: MacroProfileContext<'_>,
        invocation_node: &SyntaxNode,
        invocation: &MacroInvocation,
        imported_profiles: &mut BTreeMap<(AstNodeId, String), ImportedProfile>,
    ) -> Option<MacroProfileKey> {
        let MacroLookupContext {
            output: _,
            configured_ast,
            module,
            ..
        } = context.lookup;
        let path = invocation.path();
        if path.qualified_self().is_some() || path.qualified_trait().is_some() {
            return None;
        }
        if configured_ast
            .node(invocation_node.id())
            .is_none_or(|configured| configured.availability_decision() != CfgDecision::True)
        {
            return None;
        }
        let name = path.segments().last()?.name();
        let scope_chain = context.lookup.index.scope_chain(invocation_node.id());
        let scopes = scope_chain.iter().copied().collect::<BTreeSet<_>>();
        if path.segments().len() > 1 || path.has_leading_colon() {
            return Self::resolve_path_profile(context.lookup, path, &scope_chain)
                .map(|resolved| resolved.profile);
        }
        if context
            .lookup
            .index
            .inherited_macro_definitions()
            .contains(name)
        {
            return None;
        }
        if Self::has_visible_macro_definition(
            context.lookup.index,
            configured_ast,
            &scopes,
            name,
            invocation.start(),
        ) {
            return None;
        }

        let import_scope = scope_chain
            .first()
            .copied()
            .unwrap_or(context.lookup.module.scope_id());
        let imported = imported_profiles
            .entry((import_scope, name.to_owned()))
            .or_insert_with(|| Self::imported_profile(context.lookup, &scope_chain, name))
            .clone();
        match imported {
            ImportedProfile::Resolved(profile) => return Some(profile.profile),
            ImportedProfile::Blocked => return None,
            ImportedProfile::Absent => {}
        }
        if Self::has_macro_use_import(
            context.lookup.index,
            configured_ast,
            &scope_chain,
            invocation.start(),
            module.macro_use_context(),
        ) {
            return None;
        }
        if name == "env" {
            return Some(MacroProfileKey::Env);
        }
        let no_implicit_prelude = module.no_implicit_prelude()
            || scopes.iter().any(|scope| {
                configured_ast.node(*scope).is_some_and(|configured| {
                    configured
                        .attribute_evaluation()
                        .has_active_attribute("no_implicit_prelude")
                })
            });
        if no_implicit_prelude {
            if matches!(context.edition, "2018" | "2021" | "2024") {
                return match name {
                    "assert" => Some(MacroProfileKey::Assert),
                    "panic" => Some(MacroProfileKey::Format),
                    _ => None,
                };
            } else if context.edition != "2015" {
                return None;
            }
        }
        let profile = Self::profile_for_prelude(name, context.edition, context.no_std)?;
        Some(profile)
    }

    fn standard_profile<'a>(
        segments: impl Iterator<Item = &'a str>,
        std_available: bool,
    ) -> Option<ResolvedMacroProfile> {
        let segments = segments.collect::<Vec<_>>();
        let segments = segments.strip_prefix(&["::"]).unwrap_or(&segments);
        let [root, name] = segments else {
            return None;
        };
        if !matches!(*root, "std" | "core" | "alloc") {
            return None;
        }
        if *root == "std" && !std_available {
            return None;
        }
        if *root == "alloc" && !matches!(*name, "format" | "vec") {
            return None;
        }
        if *root == "core" && !Self::is_core_prelude_macro(name) {
            return None;
        }
        let profile = Self::profile_for_name(name)?;
        Some(ResolvedMacroProfile::new(root, name, profile))
    }

    fn profile_for_known_path(
        module: &ModuleNode,
        segments: &[&str],
        std_available: bool,
    ) -> Option<ResolvedMacroProfile> {
        if let Some(profile) = Self::standard_profile(segments.iter().copied(), std_available) {
            return Some(profile);
        }
        let [root, name] = segments else {
            return None;
        };
        let (package, profile) = match (module.macro_dependency_package(root)?, *name) {
            ("serde_json", "json") => ("serde_json", MacroProfileKey::Json),
            ("log", "error") => ("log", MacroProfileKey::Error),
            _ => return None,
        };
        Some(ResolvedMacroProfile::new(package, name, profile))
    }

    fn profile_for_name(name: &str) -> Option<MacroProfileKey> {
        match name {
            "assert" => Some(MacroProfileKey::Assert),
            "assert_eq" => Some(MacroProfileKey::AssertEq),
            "assert_ne" => Some(MacroProfileKey::AssertNe),
            "matches" => Some(MacroProfileKey::Matches),
            "env" => Some(MacroProfileKey::Env),
            "format" | "panic" | "println" | "eprintln" => Some(MacroProfileKey::Format),
            "write" | "writeln" => Some(MacroProfileKey::Write),
            "vec" => Some(MacroProfileKey::Vec),
            _ => None,
        }
    }

    fn profile_for_prelude(name: &str, edition: &str, no_std: bool) -> Option<MacroProfileKey> {
        if !matches!(edition, "2015" | "2018" | "2021" | "2024") {
            return None;
        }
        let profile = Self::profile_for_name(name)?;
        if no_std && !Self::is_core_prelude_macro(name) {
            return None;
        }
        Some(profile)
    }

    fn is_core_prelude_macro(name: &str) -> bool {
        matches!(
            name,
            "assert" | "assert_eq" | "assert_ne" | "matches" | "panic" | "write" | "writeln"
        )
    }

    fn module_for_invocation<'a>(
        modules: &'a [&ModuleNode],
        scopes: &BTreeSet<AstNodeId>,
    ) -> &'a ModuleNode {
        modules
            .iter()
            .filter(|candidate| scopes.contains(&candidate.scope_id()))
            .max_by_key(|candidate| candidate.namespace().len())
            .copied()
            .expect("macro invocation belongs to a module scope in its source instance")
    }

    fn resolve_path_profile(
        context: MacroLookupContext<'_>,
        path: &PathSyntax,
        scopes: &[AstNodeId],
    ) -> Option<ResolvedMacroProfile> {
        let MacroLookupContext {
            output: _,
            configured_ast,
            module,
            ..
        } = context;
        let segments = path
            .segments()
            .iter()
            .map(|segment| segment.name())
            .collect::<Vec<_>>();
        let root = segments.first()?;
        if path.has_leading_colon() {
            let std_available = Self::std_is_available(context, scopes);
            return Self::profile_for_known_path(module, &segments, std_available);
        }
        match Self::root_alias_resolution(context, scopes, root) {
            RootAliasResolution::Crate(target) => {
                if segments.len() != 2 {
                    return None;
                }
                let std_available = Self::std_is_available(context, scopes);
                return Self::profile_for_known_path(
                    module,
                    &[target.as_str(), segments.last().copied()?],
                    std_available,
                );
            }
            RootAliasResolution::Absent | RootAliasResolution::Shadowed => {}
        }
        if !Self::has_root_shadow(context, configured_ast, module, scopes, root) {
            let std_available = Self::std_is_available(context, scopes);
            if let Some(profile) = Self::profile_for_known_path(module, &segments, std_available) {
                return Some(profile);
            }
        }
        Self::resolve_module_reexport(context, &segments, 0, &mut BTreeSet::new())
    }

    fn resolve_known_use_path(
        context: MacroLookupContext<'_>,
        path: &[&str],
        scopes: &[AstNodeId],
        depth: usize,
        visited: &mut BTreeSet<(Vec<String>, String)>,
    ) -> Option<ResolvedMacroProfile> {
        let MacroLookupContext {
            output: _,
            configured_ast,
            module,
            ..
        } = context;
        let leading_colon = path.first().is_some_and(|segment| *segment == "::");
        let path = if leading_colon { &path[1..] } else { path };
        if leading_colon {
            let std_available = Self::std_is_available(context, scopes);
            return Self::profile_for_known_path(module, path, std_available);
        }
        if let Some(root) = path.first() {
            if let RootAliasResolution::Crate(target) =
                Self::root_alias_resolution(context, scopes, root)
            {
                if path.len() != 2 {
                    return None;
                }
                let std_available = Self::std_is_available(context, scopes);
                return Self::profile_for_known_path(
                    module,
                    &[target.as_str(), path.last().copied()?],
                    std_available,
                );
            }
        }
        if !path.first().is_some_and(|root| {
            Self::has_root_shadow(context, configured_ast, module, scopes, root)
        }) {
            let std_available = Self::std_is_available(context, scopes);
            if let Some(profile) = Self::profile_for_known_path(module, path, std_available) {
                return Some(profile);
            }
        }
        Self::resolve_module_reexport(context, path, depth, visited)
    }

    fn root_alias_resolution(
        context: MacroLookupContext<'_>,
        scope_chain: &[AstNodeId],
        root: &str,
    ) -> RootAliasResolution {
        for scope in scope_chain {
            let Some(entries) = context.index.scope(*scope) else {
                continue;
            };
            let Some(candidates) = entries.root_shadows.get(root) else {
                continue;
            };
            let mut crate_roots = Vec::new();
            for candidate_id in candidates {
                let Some(configured) = context.configured_ast.node(*candidate_id) else {
                    return RootAliasResolution::Shadowed;
                };
                match configured.availability_decision() {
                    CfgDecision::False => continue,
                    CfgDecision::Unknown => return RootAliasResolution::Shadowed,
                    CfgDecision::True => {}
                }
                let Some(node) = context.output.node(*candidate_id) else {
                    return RootAliasResolution::Shadowed;
                };
                if let Some(ItemSyntaxKind::ExternCrate { name, alias }) =
                    node.item_syntax().map(|item| item.kind())
                {
                    if alias.as_deref().or(name.as_deref()) != Some(root) {
                        return RootAliasResolution::Shadowed;
                    }
                    let Some(name) = name else {
                        return RootAliasResolution::Shadowed;
                    };
                    crate_roots.push(name.to_owned());
                } else if node.use_tree().is_some() {
                    let Some(usage) = entries
                        .uses
                        .get(root)
                        .into_iter()
                        .flatten()
                        .find(|usage| usage.node_id == *candidate_id)
                    else {
                        return RootAliasResolution::Shadowed;
                    };
                    let Some(path) = usage.path.as_deref() else {
                        return RootAliasResolution::Shadowed;
                    };
                    let path = if path.first().is_some_and(|segment| segment == "::") {
                        &path[1..]
                    } else {
                        path
                    };
                    let [crate_root] = path else {
                        return RootAliasResolution::Shadowed;
                    };
                    if !Self::is_known_crate_root(context.module, crate_root)
                        || Self::has_root_shadow(
                            context,
                            context.configured_ast,
                            context.module,
                            scope_chain,
                            crate_root,
                        )
                    {
                        return RootAliasResolution::Shadowed;
                    }
                    crate_roots.push(crate_root.clone());
                } else {
                    return RootAliasResolution::Shadowed;
                }
            }
            let Some(crate_root) = crate_roots.first() else {
                continue;
            };
            if !Self::is_known_crate_root(context.module, crate_root) {
                return RootAliasResolution::Shadowed;
            }
            if crate_roots.iter().all(|candidate| candidate == crate_root) {
                return RootAliasResolution::Crate(crate_root.clone());
            }
            return RootAliasResolution::Shadowed;
        }
        RootAliasResolution::Absent
    }

    fn is_known_crate_root(module: &ModuleNode, root: &str) -> bool {
        matches!(root, "std" | "core" | "alloc") || module.macro_dependency_package(root).is_some()
    }

    fn resolve_module_reexport(
        context: MacroLookupContext<'_>,
        path: &[&str],
        depth: usize,
        visited: &mut BTreeSet<(Vec<String>, String)>,
    ) -> Option<ResolvedMacroProfile> {
        let module = context.module;
        let modules = context.modules;
        if depth > modules.len() || path.len() < 2 {
            return None;
        }
        let name = path.last()?.to_string();
        let mut namespace = module.namespace().to_vec();
        let mut path_start = 0;
        match path.first().copied()? {
            "crate" => {
                namespace.clear();
                path_start = 1;
            }
            "self" => path_start = 1,
            "super" => {
                while path.get(path_start) == Some(&"super") {
                    namespace.pop()?;
                    path_start += 1;
                }
            }
            _ => {}
        }
        let module_segments = &path[path_start..path.len().saturating_sub(1)];
        namespace.extend(module_segments.iter().map(|segment| (*segment).to_owned()));
        if !visited.insert((namespace.clone(), name.clone())) {
            return None;
        }
        let target = modules.iter().find(|candidate| {
            candidate.context_index() == module.context_index()
                && candidate.namespace() == namespace
        })?;
        let target_index = context.catalog.index(target.source_instance())?;
        let target_context = MacroLookupContext {
            output: target.parse_output(),
            index: target_index,
            catalog: context.catalog,
            configured_ast: target.configured_ast(),
            module: target,
            modules,
            no_std: context.no_std,
        };
        let output = target_context.output;
        let mut profiles = Vec::new();
        let entries = target_index.scope(target.scope_id())?;
        for usage in &entries.glob_uses {
            let configured = target.configured_ast().node(usage.node_id)?;
            if configured.availability_decision() != CfgDecision::False {
                return None;
            }
        }
        let uses = entries.uses.get(&name)?;
        for usage in uses {
            let imported_path = usage.path.as_deref()?;
            let node = output.node(usage.node_id)?;
            if !Self::is_crate_visible_reexport(output, node) {
                continue;
            }
            let configured = target.configured_ast().node(usage.node_id)?;
            match configured.availability_decision() {
                CfgDecision::False => continue,
                CfgDecision::Unknown => return None,
                CfgDecision::True => {}
            }
            let ancestors = target_index.scope_chain(usage.node_id);
            let imported_path = imported_path.iter().map(String::as_str).collect::<Vec<_>>();
            let profile = Self::resolve_known_use_path(
                target_context,
                &imported_path,
                &ancestors,
                depth + 1,
                visited,
            )?;
            profiles.push(profile);
        }
        let resolved = profiles.first()?;
        profiles
            .iter()
            .all(|candidate| candidate.identity == resolved.identity)
            .then(|| ResolvedMacroProfile {
                identity: resolved.identity.clone(),
                profile: resolved.profile,
            })
    }

    fn is_crate_visible_reexport(output: &ParseOutput, node: &SyntaxNode) -> bool {
        let mut current = node.parent();
        while let Some(id) = current {
            let Some(parent) = output.node(id) else {
                return false;
            };
            if let Some(item) = parent.item_syntax() {
                if !matches!(item.kind(), ItemSyntaxKind::Use) {
                    return false;
                }
                return match item.visibility().kind() {
                    VisibilityKind::Public => true,
                    VisibilityKind::Restricted(path) => {
                        path == "crate" || path.starts_with("crate::")
                    }
                    VisibilityKind::RestrictedIn(path) => {
                        path == "crate" || path.starts_with("crate::")
                    }
                    VisibilityKind::Private => false,
                };
            }
            current = parent.parent();
        }
        false
    }

    pub(super) fn has_visible_macro_definition(
        index: &MacroResolverIndex,
        configured_ast: &ConfiguredAst,
        scopes: &BTreeSet<AstNodeId>,
        name: &str,
        invocation_start: usize,
    ) -> bool {
        scopes.iter().any(|scope| {
            index
                .scope(*scope)
                .and_then(|entries| entries.definitions.get(name))
                .is_some_and(|definitions| {
                    definitions.iter().any(|(start, definition_id)| {
                        *start < invocation_start
                            && configured_ast
                                .node(*definition_id)
                                .is_none_or(|configured| {
                                    configured.availability_decision() != CfgDecision::False
                                })
                    })
                })
        })
    }

    fn has_root_shadow(
        context: MacroLookupContext<'_>,
        configured_ast: &ConfiguredAst,
        module: &ModuleNode,
        scopes: &[AstNodeId],
        root: &str,
    ) -> bool {
        scopes.iter().any(|scope| {
            context
                .index
                .scope(*scope)
                .and_then(|entries| entries.root_shadows.get(root))
                .is_some_and(|candidates| {
                    candidates.iter().any(|candidate_id| {
                        if configured_ast
                            .node(*candidate_id)
                            .is_some_and(|configured| {
                                configured.availability_decision() == CfgDecision::False
                            })
                        {
                            return false;
                        }
                        let Some(node) = context.output.node(*candidate_id) else {
                            return false;
                        };
                        match node.item_syntax().map(|item| item.kind()) {
                            Some(ItemSyntaxKind::Module { name, .. }) => name == root,
                            Some(ItemSyntaxKind::Struct(Some(name)))
                            | Some(ItemSyntaxKind::Enum(Some(name)))
                            | Some(ItemSyntaxKind::Union(Some(name)))
                            | Some(ItemSyntaxKind::Trait(Some(name)))
                            | Some(ItemSyntaxKind::TypeAlias(Some(name))) => name == root,
                            Some(ItemSyntaxKind::ExternCrate { name, alias }) => {
                                alias.as_deref().or(name.as_deref()) == Some(root)
                                    && !name.as_deref().is_some_and(|name| {
                                        name == root
                                            || module.macro_dependency_package(root) == Some(name)
                                    })
                            }
                            Some(ItemSyntaxKind::Use) => true,
                            _ => false,
                        }
                    })
                })
        })
    }

    fn imported_profile(
        context: MacroLookupContext<'_>,
        scope_chain: &[AstNodeId],
        name: &str,
    ) -> ImportedProfile {
        let MacroLookupContext {
            output: _,
            configured_ast,
            ..
        } = context;
        for scope in scope_chain {
            let Some(entries) = context.index.scope(*scope) else {
                continue;
            };
            let mut uses = entries.uses.get(name).cloned().unwrap_or_default();
            uses.extend(entries.glob_uses.iter().map(|usage| IndexedUse {
                node_id: usage.node_id,
                path: usage.path.clone(),
                is_glob: true,
            }));
            if uses.is_empty() {
                continue;
            }
            let mut profiles = Vec::new();
            for usage in uses {
                let decision = configured_ast
                    .node(usage.node_id)
                    .map(|configured| configured.availability_decision());
                if decision == Some(CfgDecision::False) {
                    continue;
                }
                if usage.is_glob || decision != Some(CfgDecision::True) {
                    profiles.push(None);
                    continue;
                }
                let Some(path) = usage.path.as_deref() else {
                    profiles.push(None);
                    continue;
                };
                let use_scopes = context.index.scope_chain(usage.node_id);
                profiles.push(Self::resolve_known_use_path(
                    context,
                    &path.iter().map(String::as_str).collect::<Vec<_>>(),
                    &use_scopes,
                    0,
                    &mut BTreeSet::new(),
                ));
            }
            if profiles.is_empty() {
                continue;
            }
            let Some(profile) = profiles.first().and_then(Option::as_ref) else {
                return ImportedProfile::Blocked;
            };
            return if profiles.iter().all(|candidate| {
                candidate
                    .as_ref()
                    .is_some_and(|candidate| candidate.identity == profile.identity)
            }) {
                ImportedProfile::Resolved(ResolvedMacroProfile {
                    identity: profile.identity.clone(),
                    profile: profile.profile,
                })
            } else {
                ImportedProfile::Blocked
            };
        }
        ImportedProfile::Absent
    }

    fn has_macro_use_import(
        index: &MacroResolverIndex,
        configured_ast: &ConfiguredAst,
        scopes: &[AstNodeId],
        invocation_start: usize,
        inherited_macro_use: bool,
    ) -> bool {
        inherited_macro_use
            || scopes.iter().any(|scope| {
                index.scope(*scope).is_some_and(|entries| {
                    entries.macro_use_items.iter().any(|item| {
                        let declaration_is_visible = match item.kind {
                            IndexedMacroUseKind::ExternCrate => true,
                            IndexedMacroUseKind::Module => item.start <= invocation_start,
                        };
                        declaration_is_visible
                            && Self::macro_use_item_is_active(configured_ast, item.node_id)
                    })
                })
            })
    }

    fn macro_use_item_is_active(configured_ast: &ConfiguredAst, node_id: AstNodeId) -> bool {
        configured_ast.node(node_id).is_some_and(|configured| {
            configured.availability_decision() != CfgDecision::False
                && configured
                    .attribute_evaluation()
                    .has_active_attribute("macro_use")
        })
    }

    fn std_is_available(context: MacroLookupContext<'_>, scopes: &[AstNodeId]) -> bool {
        if !context.no_std {
            return true;
        }
        let inherited_import = context.modules.iter().any(|candidate| {
            candidate.context_index() == context.module.context_index()
                && context
                    .module
                    .namespace()
                    .starts_with(candidate.namespace())
                && context
                    .catalog
                    .index(candidate.source_instance())
                    .and_then(|index| index.scope(candidate.scope_id()))
                    .is_some_and(|entries| {
                        entries.std_imports.iter().any(|import| {
                            import.crate_name == "std"
                                && candidate.configured_ast().node(import.node_id).is_some_and(
                                    |configured| {
                                        configured.availability_decision() == CfgDecision::True
                                    },
                                )
                        })
                    })
        });
        let local_import = scopes.iter().any(|scope| {
            context.index.scope(*scope).is_some_and(|entries| {
                entries.std_imports.iter().any(|import| {
                    import.crate_name == "std"
                        && context
                            .configured_ast
                            .node(import.node_id)
                            .is_some_and(|configured| {
                                configured.availability_decision() == CfgDecision::True
                            })
                })
            })
        });
        inherited_import || local_import
    }
}
