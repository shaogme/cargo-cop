use std::ops::Range;

use cop_syntax::{AstNodeId, ItemSyntaxKind, SyntaxNode, SyntaxNodeKind};

use crate::{CfgDecision, ModuleNode};

pub(super) struct SyntaxFactSupport;

impl SyntaxFactSupport {
    pub(super) fn nearest_record_item(module: &ModuleNode, node: &SyntaxNode) -> Option<AstNodeId> {
        let output = module.parse_output();
        let mut current = node.parent();
        while let Some(id) = current {
            let ancestor = output.node(id)?;
            if ancestor.item_syntax().is_some_and(|item| {
                matches!(
                    item.kind(),
                    ItemSyntaxKind::Struct(_) | ItemSyntaxKind::Enum(_) | ItemSyntaxKind::Union(_)
                )
            }) {
                return Some(id);
            }
            if ancestor
                .item_syntax()
                .is_some_and(|item| matches!(item.kind(), ItemSyntaxKind::Module { .. }))
            {
                return None;
            }
            current = ancestor.parent();
        }
        None
    }

    pub(super) fn is_source_block(module: &ModuleNode, node: &SyntaxNode) -> bool {
        let output = module.parse_output();
        let mut current = Some(node.id());
        while let Some(id) = current {
            let Some(ancestor) = output.node(id) else {
                return false;
            };
            if matches!(
                ancestor.kind(),
                SyntaxNodeKind::Macro | SyntaxNodeKind::MacroInput
            ) {
                return false;
            }
            if id == module.scope_id() {
                return true;
            }
            current = ancestor.parent();
        }
        false
    }

    pub(super) fn node_is_enabled(module: &ModuleNode, node: &SyntaxNode) -> bool {
        module
            .configured_ast()
            .node(node.id())
            .is_some_and(|configured| configured.availability_decision() != CfgDecision::False)
    }

    pub(super) fn condition_for_node(module: &ModuleNode, node: &SyntaxNode) -> Option<String> {
        module
            .configured_ast()
            .condition_for_span(node.start(), node.end())
            .map(str::to_owned)
    }

    pub(super) fn within_bounds(outer: &Range<usize>, inner: Range<usize>) -> bool {
        outer.start <= inner.start && inner.end <= outer.end
    }

    pub(super) fn count_code_lines(module: &ModuleNode, test_ranges: &[Range<usize>]) -> usize {
        let output = module.parse_output();
        let test_module = output
            .inner_attributes()
            .iter()
            .any(|attribute| attribute.is_cfg_test());
        let mut lines = Vec::new();
        if !test_module {
            for token in output.tokens() {
                if !module
                    .configured_ast()
                    .is_enabled_range(token.start(), token.end())
                    || test_ranges
                        .iter()
                        .any(|range| range.contains(&token.start()))
                {
                    continue;
                }
                if let Some((start_line, _)) = output.source().line_column(token.start()) {
                    let line_breaks = token
                        .text(output.source())
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count();
                    let end_line =
                        start_line.saturating_add(u32::try_from(line_breaks).unwrap_or(u32::MAX));
                    for line in start_line..=end_line {
                        if lines.last() != Some(&line) {
                            lines.push(line);
                        }
                    }
                }
            }
        }
        lines.len()
    }
}
