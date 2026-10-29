use cop_syntax::{
    AstNodeId, ItemSyntaxKind, MacroInvocation, ParseOutput, SyntaxNode, SyntaxNodeKind,
};

use crate::{CfgDecision, ConfiguredAst};

pub(super) struct MacroScanner;

impl MacroScanner {
    pub(super) fn include_invocations(
        item: &SyntaxNode,
        output: &ParseOutput,
        configured_ast: &ConfiguredAst,
    ) -> Vec<MacroInvocation> {
        let mut invocations = Vec::new();
        let root = item;
        let Some(item_data) = root.item_syntax() else {
            return invocations;
        };
        match item_data.kind() {
            ItemSyntaxKind::Module { .. }
            | ItemSyntaxKind::Use
            | ItemSyntaxKind::ExternCrate { .. }
            | ItemSyntaxKind::MacroDefinition(_)
            | ItemSyntaxKind::Other => return invocations,
            ItemSyntaxKind::Macro(invocation) => {
                Self::collect_include(invocation, &mut invocations);
                return invocations;
            }
            ItemSyntaxKind::Struct(_)
            | ItemSyntaxKind::Enum(_)
            | ItemSyntaxKind::Union(_)
            | ItemSyntaxKind::TypeAlias(_)
            | ItemSyntaxKind::Trait(_)
            | ItemSyntaxKind::Constant(_)
            | ItemSyntaxKind::Static(_)
            | ItemSyntaxKind::ExternBlock { .. }
            | ItemSyntaxKind::Function { .. }
            | ItemSyntaxKind::Impl => {}
        }
        Self::collect_item_macros(root.id(), output, configured_ast, &mut invocations);
        invocations
    }

    fn collect_item_macros(
        root: AstNodeId,
        output: &ParseOutput,
        configured_ast: &ConfiguredAst,
        invocations: &mut Vec<MacroInvocation>,
    ) {
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            let Some(node) = output.node(id) else {
                continue;
            };
            if node.id() != root
                && node.item_syntax().is_some_and(|item_data| {
                    matches!(item_data.kind(), ItemSyntaxKind::Module { .. })
                })
            {
                continue;
            }
            if configured_ast
                .node(node.id())
                .is_none_or(|configured| configured.availability_decision() == CfgDecision::False)
            {
                continue;
            }
            if node.kind() == SyntaxNodeKind::Macro {
                if let Some(invocation) = node.macro_invocation() {
                    Self::collect_include(invocation, invocations);
                }
            }
            pending.extend(node.children().iter().rev().copied());
        }
    }

    fn collect_include(invocation: &MacroInvocation, invocations: &mut Vec<MacroInvocation>) {
        if !Self::is_include_macro(invocation) {
            return;
        }
        let already_seen = invocations
            .iter()
            .any(|found| found.start() == invocation.start() && found.end() == invocation.end());
        if !already_seen {
            invocations.push(invocation.clone());
        }
    }

    fn is_include_macro(invocation: &MacroInvocation) -> bool {
        invocation
            .path()
            .segments()
            .last()
            .is_some_and(|segment| segment.name() == "include")
    }
}
