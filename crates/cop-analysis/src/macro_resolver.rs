use std::collections::BTreeSet;

use cop_syntax::{
    AstNodeId, ItemSyntaxKind, MacroInvocation, ParseOutput, SyntaxNode, SyntaxNodeKind,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MacroResolution {
    BuiltinCfg,
    NonBuiltin,
    Ambiguous,
}

pub(crate) struct MacroResolver;

impl MacroResolver {
    pub(crate) fn resolve(
        output: &ParseOutput,
        invocation_id: AstNodeId,
        invocation: &MacroInvocation,
    ) -> MacroResolution {
        if !Self::is_unqualified_cfg(invocation) {
            return MacroResolution::Ambiguous;
        }
        let Some(invocation_node) = output.node(invocation_id) else {
            return MacroResolution::Ambiguous;
        };
        let scopes = Self::ancestor_ids(output, invocation_node);
        if output
            .syntax_nodes()
            .iter()
            .any(|node| Self::shadows_cfg(node, &scopes, invocation.start()))
        {
            return MacroResolution::NonBuiltin;
        }
        if output
            .syntax_nodes()
            .iter()
            .any(|node| Self::may_import_cfg(output, node, &scopes))
        {
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

    fn ancestor_ids(output: &ParseOutput, node: &SyntaxNode) -> BTreeSet<AstNodeId> {
        let mut ancestors = BTreeSet::new();
        let mut current = Some(node.id());
        while let Some(id) = current {
            ancestors.insert(id);
            current = output.node(id).and_then(SyntaxNode::parent);
        }
        ancestors
    }

    fn shadows_cfg(
        node: &SyntaxNode,
        invocation_scopes: &BTreeSet<AstNodeId>,
        invocation_start: usize,
    ) -> bool {
        let Some(ItemSyntaxKind::MacroDefinition(definition)) =
            node.item_syntax().map(|item| item.kind())
        else {
            return false;
        };
        if definition.name() != "cfg" || definition.start() >= invocation_start {
            return false;
        }
        definition
            .scope()
            .is_some_and(|scope| invocation_scopes.contains(&scope))
    }

    fn may_import_cfg(
        output: &ParseOutput,
        node: &SyntaxNode,
        invocation_scopes: &BTreeSet<AstNodeId>,
    ) -> bool {
        if !Self::nearest_scope(output, node)
            .is_some_and(|scope| invocation_scopes.contains(&scope))
        {
            return false;
        }
        if Self::may_import_cfg_via_macro_use(node) {
            return true;
        }
        if node
            .item_syntax()
            .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::Use))
            && node.use_tree().is_none()
        {
            return true;
        }
        let Some(tree) = node.use_tree() else {
            return false;
        };
        tree.is_glob()
            || tree.alias() == Some("cfg")
            || tree.path().last().is_some_and(|segment| segment == "cfg")
    }

    fn may_import_cfg_via_macro_use(node: &SyntaxNode) -> bool {
        let imports_macros = node.item_syntax().is_some_and(|item| {
            matches!(
                item.kind(),
                ItemSyntaxKind::ExternCrate { .. } | ItemSyntaxKind::Module { .. }
            )
        });
        imports_macros
            && node.attributes().iter().any(|attribute| {
                let path = attribute.meta().path();
                path.len() == 1 && path.first().is_some_and(|name| name == "macro_use")
            })
    }

    fn nearest_scope(output: &ParseOutput, node: &SyntaxNode) -> Option<AstNodeId> {
        let mut current = Some(node.id());
        while let Some(id) = current {
            let ancestor = output.node(id)?;
            if matches!(
                ancestor.kind(),
                SyntaxNodeKind::Scope | SyntaxNodeKind::Crate
            ) {
                return Some(id);
            }
            current = ancestor.parent();
        }
        None
    }
}
