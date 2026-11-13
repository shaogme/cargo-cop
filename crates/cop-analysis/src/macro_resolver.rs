mod index;
mod profile;

pub(crate) use self::index::{MacroResolverCatalog, MacroResolverIndex};

use std::collections::BTreeSet;

use cop_syntax::{AstNodeId, MacroInvocation, MacroProfileBinding, ParseOutput};

use crate::{
    cfg_eval::CfgDecision,
    macro_resolver::{index::IndexedMacroUseKind, profile::MacroProfileResolver},
    module_graph::ModuleNode,
    ConfiguredAst,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MacroResolution {
    BuiltinCfg,
    NonBuiltin,
    Ambiguous,
}

pub(crate) struct MacroResolver;

impl MacroResolver {
    pub(crate) fn profile_bindings(
        instance_modules: &[&ModuleNode],
        modules: &[ModuleNode],
        catalog: &MacroResolverCatalog,
        edition: &str,
    ) -> Vec<MacroProfileBinding> {
        MacroProfileResolver::profile_bindings(instance_modules, modules, catalog, edition)
    }

    pub(crate) fn has_macro_use_import_for_module(
        index: &MacroResolverIndex,
        configured_ast: &ConfiguredAst,
        scope_id: AstNodeId,
        child_start: usize,
    ) -> bool {
        index.scope(scope_id).is_some_and(|entries| {
            entries.macro_use_items.iter().any(|item| {
                let declaration_is_visible = match item.kind {
                    IndexedMacroUseKind::ExternCrate => true,
                    IndexedMacroUseKind::Module => item.start < child_start,
                };
                declaration_is_visible
                    && Self::macro_use_item_is_active(configured_ast, item.node_id)
            })
        })
    }

    pub(crate) fn macro_definitions_before(
        index: &MacroResolverIndex,
        configured_ast: &ConfiguredAst,
        scope_id: AstNodeId,
        declaration_start: usize,
    ) -> BTreeSet<String> {
        index
            .scope(scope_id)
            .into_iter()
            .flat_map(|entries| entries.definitions.iter())
            .filter(|(_, definitions)| {
                definitions.iter().any(|(start, node_id)| {
                    *start < declaration_start
                        && configured_ast.node(*node_id).is_none_or(|configured| {
                            configured.availability_decision() != CfgDecision::False
                        })
                })
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn macro_use_item_is_active(configured_ast: &ConfiguredAst, node_id: AstNodeId) -> bool {
        configured_ast.node(node_id).is_some_and(|configured| {
            configured.availability_decision() != CfgDecision::False
                && configured
                    .attribute_evaluation()
                    .has_active_attribute("macro_use")
        })
    }

    pub(crate) fn resolve(
        index: &MacroResolverIndex,
        output: &ParseOutput,
        configured_ast: &ConfiguredAst,
        invocation_id: AstNodeId,
        invocation: &MacroInvocation,
    ) -> MacroResolution {
        if !Self::is_unqualified_cfg(invocation) {
            return MacroResolution::Ambiguous;
        }
        let Some(invocation_node) = output.node(invocation_id) else {
            return MacroResolution::Ambiguous;
        };
        let scopes = index.scope_ids(invocation_node.id());
        if index.inherited_macro_definitions().contains("cfg") {
            return MacroResolution::NonBuiltin;
        }
        if MacroProfileResolver::has_visible_macro_definition(
            index,
            configured_ast,
            &scopes,
            "cfg",
            invocation.start(),
        ) {
            return MacroResolution::NonBuiltin;
        }
        if Self::may_import_cfg(index, configured_ast, &scopes, invocation.start()) {
            return MacroResolution::Ambiguous;
        }
        MacroResolution::BuiltinCfg
    }

    fn is_unqualified_cfg(invocation: &MacroInvocation) -> bool {
        let path = invocation.path();
        !path.has_leading_colon()
            && path.qualified_self().is_none()
            && path.qualified_trait().is_none()
            && path.segments().len() == 1
            && path
                .segments()
                .first()
                .is_some_and(|segment| segment.name() == "cfg" && segment.arguments().is_empty())
    }

    fn may_import_cfg(
        index: &MacroResolverIndex,
        configured_ast: &ConfiguredAst,
        invocation_scopes: &BTreeSet<AstNodeId>,
        invocation_start: usize,
    ) -> bool {
        index.inherited_macro_use()
            || invocation_scopes.iter().any(|scope| {
                let Some(entries) = index.scope(*scope) else {
                    return false;
                };
                let is_enabled = |node_id: AstNodeId| {
                    configured_ast.node(node_id).is_none_or(|configured| {
                        configured.availability_decision() != CfgDecision::False
                    })
                };
                let has_macro_use = entries.macro_use_items.iter().any(|item| {
                    let declaration_is_visible = match item.kind {
                        IndexedMacroUseKind::ExternCrate => true,
                        IndexedMacroUseKind::Module => item.start < invocation_start,
                    };
                    let active = configured_ast.node(item.node_id).is_some_and(|configured| {
                        configured
                            .attribute_evaluation()
                            .has_active_attribute("macro_use")
                    });
                    declaration_is_visible && is_enabled(item.node_id) && active
                });
                let has_uncertain_use = entries
                    .whole_uses
                    .iter()
                    .any(|node_id| is_enabled(*node_id))
                    || entries
                        .glob_uses
                        .iter()
                        .any(|usage| is_enabled(usage.node_id))
                    || entries.uses.iter().any(|(import_name, uses)| {
                        uses.iter().any(|usage| {
                            is_enabled(usage.node_id)
                                && (import_name == "cfg"
                                    || usage
                                        .path
                                        .as_ref()
                                        .and_then(|path| path.last())
                                        .is_some_and(|name| name == "cfg"))
                        })
                    });
                has_macro_use || has_uncertain_use
            })
    }
}
