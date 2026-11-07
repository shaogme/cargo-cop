use cop_analysis::{
    AnalysisSnapshot, ImplOwnerRecord, ModuleNode, SymbolKind, SymbolRecord, SymbolVisibility,
};
use cop_syntax::ItemSyntaxKind;

use crate::{semantic_rules::target_origin::TypeOriginResolver, support::SyntaxSupport};

pub(super) struct DeclarationScope;

impl DeclarationScope {
    pub(super) fn symbol_is_in_module(symbol: &SymbolRecord, module: &ModuleNode) -> bool {
        symbol.context_index() == module.context_index() && symbol.namespace() == module.namespace()
    }

    pub(super) fn sealed_trait_exception(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        implementation: &ImplOwnerRecord,
        trait_symbol: &SymbolRecord,
    ) -> bool {
        if trait_symbol.name() != "Sealed"
            || !matches!(trait_symbol.visibility(), SymbolVisibility::Public)
        {
            return false;
        }
        let Some(trait_span) = trait_symbol.span() else {
            return false;
        };
        let trait_bounds = trait_span.byte_range();
        let Some(trait_module) = snapshot.modules().iter().find(|candidate| {
            candidate.context_index() == module.context_index()
                && candidate.target_name() == module.target_name()
                && candidate.namespace() == trait_symbol.namespace()
                && candidate.parent_namespace() == module.namespace()
                && candidate.file_path() == trait_symbol.file_path()
                && candidate.is_inline()
                && candidate.scope_range().start <= trait_bounds.start
                && trait_bounds.end <= candidate.scope_range().end
        }) else {
            return false;
        };
        let Some(declaration_span) = trait_module.declared_at() else {
            return false;
        };
        let Some(module_symbol) = snapshot.symbols().symbols().iter().find(|symbol| {
            symbol.kind() == SymbolKind::Module
                && symbol.context_index() == module.context_index()
                && symbol.target_name() == module.target_name()
                && symbol.namespace() == module.namespace()
                && symbol.target_namespace() == Some(trait_module.namespace())
                && symbol.file_path() == trait_module.file_path()
                && symbol.span() == Some(declaration_span)
        }) else {
            return false;
        };
        if !matches!(module_symbol.visibility(), SymbolVisibility::Private) {
            return false;
        }
        let conditions = [
            trait_module.cfg_condition().map(str::to_owned),
            module_symbol.cfg_condition().map(str::to_owned),
            trait_symbol.cfg_condition().map(str::to_owned),
        ];
        TypeOriginResolver::conditions_may_coexist_with_impl(
            implementation.condition(),
            &conditions,
        )
    }

    pub(super) fn cfg_exclusive_declaration_exists(
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
                !SyntaxSupport::conditions_may_coexist(
                    implementation_condition,
                    condition.as_deref(),
                )
            })
    }
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
