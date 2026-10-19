use std::{collections::BTreeSet, ops::Range};

use cop_analysis::{CfgDecision, ModuleNode};
use cop_model::{
    Diagnostic, LocalizedMessage, RuleId, BLOCK_USE_AT_SCOPE_START, CFG_INSIDE_USE_TREE,
    CFG_USE_GROUP_SPACING, CFG_USE_GROUP_SPLIT, MERGE_BLOCK_IMPORT_PREFIX,
};
use cop_syntax::{ItemSyntaxKind, ParseOutput, SyntaxNode, SyntaxNodeKind};

use crate::support::{RuleDiagnosticAdapter, RuleReport, SyntaxSupport, UseLeaf};

struct BlockImportRecord {
    start: usize,
    end: usize,
    condition: Option<String>,
    leaves: Vec<UseLeaf>,
}

pub(super) struct BlockImportRuleSet;

impl BlockImportRuleSet {
    pub(super) fn check_item(
        item: &SyntaxNode,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Vec<Range<usize>> {
        check_item_node(item, module, diagnostics, adapter)
    }
}

fn check_item_node(
    item: &SyntaxNode,
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) -> Vec<Range<usize>> {
    if !node_is_potentially_enabled(module, item) {
        return Vec::new();
    }
    match item.item_syntax().map(|data| data.kind()) {
        Some(ItemSyntaxKind::Function { .. }) => {
            let body = item
                .children()
                .iter()
                .filter_map(|id| module.parse_output().node(*id))
                .find(|node| node.kind() == SyntaxNodeKind::Block);
            body.map_or_else(Vec::new, |body| {
                check_function_blocks(body, module, diagnostics, adapter)
            })
        }
        Some(ItemSyntaxKind::Trait(_) | ItemSyntaxKind::Impl) => {
            let output = module.parse_output();
            let Some(scope) = item
                .children()
                .iter()
                .filter_map(|id| output.node(*id))
                .find(|node| node.kind() == SyntaxNodeKind::Scope)
            else {
                return Vec::new();
            };
            scope
                .children()
                .iter()
                .filter_map(|id| output.node(*id))
                .filter(|node| node.item_syntax().is_some())
                .flat_map(|nested| check_item_node(nested, module, diagnostics, adapter))
                .collect()
        }
        _ => Vec::new(),
    }
}

fn check_function_blocks(
    body: &SyntaxNode,
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) -> Vec<Range<usize>> {
    let mut pending = vec![body.id()];
    let mut use_tree_ranges = Vec::new();
    while let Some(id) = pending.pop() {
        let Some(node) = module.parse_output().node(id) else {
            continue;
        };
        if !node_is_potentially_enabled(module, node) {
            continue;
        }
        if node.kind() == SyntaxNodeKind::Block {
            use_tree_ranges.extend(check_block_scope(node, module, diagnostics, adapter));
        }
        if node.kind() == SyntaxNodeKind::Opaque
            || node.kind() == SyntaxNodeKind::Macro
            || node
                .item_syntax()
                .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Module { .. }))
        {
            continue;
        }
        pending.extend(node.children().iter().rev().copied());
    }
    use_tree_ranges.sort_unstable_by_key(|range| range.start);
    use_tree_ranges
}

fn check_block_scope(
    scope: &SyntaxNode,
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) -> Vec<Range<usize>> {
    let mut preceding_non_imports = Vec::new();
    let mut imports = Vec::new();
    let mut use_tree_ranges = Vec::new();
    for child in scope
        .children()
        .iter()
        .filter_map(|id| module.parse_output().node(*id))
    {
        if !node_is_potentially_enabled(module, child) {
            continue;
        }
        let condition = configured_condition(module, child);
        let tree = (child.kind() == SyntaxNodeKind::Statement)
            .then(|| {
                child
                    .children()
                    .iter()
                    .filter_map(|id| module.parse_output().node(*id))
                    .find(|node| node.kind() == SyntaxNodeKind::UseTree)
            })
            .flatten();
        let Some(tree) = tree else {
            preceding_non_imports.push(condition);
            continue;
        };
        if preceding_non_imports.iter().any(|previous| {
            SyntaxSupport::conditions_may_coexist(condition.as_deref(), previous.as_deref())
        }) {
            adapter.push(
                module,
                diagnostics,
                RuleReport::new(
                    RuleId::ImportsAtScopeStart,
                    LocalizedMessage::new(BLOCK_USE_AT_SCOPE_START),
                    Some(child.start()..child.end()),
                ),
            );
        }
        use_tree_ranges.push(child.start()..child.end());
        if let Some(bounds) = nested_cfg_attribute(module, tree) {
            adapter.push(
                module,
                diagnostics,
                RuleReport::new(
                    RuleId::CfgInsideUseTree,
                    LocalizedMessage::new(CFG_INSIDE_USE_TREE),
                    Some(bounds),
                ),
            );
        }
        let import_is_at_scope_start = !preceding_non_imports.iter().any(|previous| {
            SyntaxSupport::conditions_may_coexist(condition.as_deref(), previous.as_deref())
        });
        if import_is_at_scope_start {
            if let Some(import) = parse_block_import(module, child, tree) {
                imports.push(import);
            }
        }
    }
    check_block_common_prefixes(&imports, module, diagnostics, adapter);
    check_block_cfg_spacing(&imports, module, diagnostics, adapter);
    use_tree_ranges
}

fn parse_block_import(
    module: &ModuleNode,
    statement: &SyntaxNode,
    tree: &SyntaxNode,
) -> Option<BlockImportRecord> {
    if !module.configured_ast().is_supported_range(
        module.parse_output(),
        statement.start(),
        statement.end(),
    ) {
        return None;
    }
    let leaves = SyntaxSupport::use_leaves(module, tree);
    if leaves.is_empty() {
        return None;
    }
    let statement_condition = configured_condition(module, statement);
    let condition = if leaves.len() == 1 {
        leaves[0].condition.clone().or(statement_condition)
    } else {
        statement_condition
    }
    .unwrap_or_else(|| "all()".to_owned());
    Some(BlockImportRecord {
        start: statement.start(),
        end: statement.end(),
        condition: Some(condition),
        leaves,
    })
}

fn nested_cfg_attribute(module: &ModuleNode, tree: &SyntaxNode) -> Option<Range<usize>> {
    tree.children()
        .iter()
        .filter_map(|id| module.parse_output().node(*id))
        .filter(|child| child.kind() == SyntaxNodeKind::UseTree)
        .find_map(|child| nested_cfg_in_tree(module, child))
}

fn nested_cfg_in_tree(module: &ModuleNode, tree: &SyntaxNode) -> Option<Range<usize>> {
    if !node_is_potentially_enabled(module, tree) {
        return None;
    }
    if let Some(attribute) = tree
        .attributes()
        .iter()
        .find(|attribute| attribute.path() == ["cfg"])
    {
        return Some(attribute.start()..attribute.end());
    }
    tree.children()
        .iter()
        .filter_map(|id| module.parse_output().node(*id))
        .filter(|child| child.kind() == SyntaxNodeKind::UseTree)
        .find_map(|child| nested_cfg_in_tree(module, child))
}

fn check_block_common_prefixes(
    imports: &[BlockImportRecord],
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    for current_index in 1..imports.len() {
        let current = &imports[current_index];
        let Some(condition) = current.condition.as_ref() else {
            continue;
        };
        for previous in &imports[..current_index] {
            if previous.condition.as_ref() != Some(condition) {
                continue;
            }
            let Some(prefix) =
                SyntaxSupport::common_import_prefix(&previous.leaves, &current.leaves)
            else {
                continue;
            };
            adapter.push(
                module,
                diagnostics,
                RuleReport::new(
                    RuleId::MergeCommonPrefixImports,
                    LocalizedMessage::new(MERGE_BLOCK_IMPORT_PREFIX)
                        .with_number(
                            "line",
                            line_of(module.parse_output(), previous.start) as i64,
                        )
                        .with_text("prefix", prefix),
                    Some(current.start..current.end),
                ),
            );
            break;
        }
    }
}

fn check_block_cfg_spacing(
    imports: &[BlockImportRecord],
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    let mut previous: Option<&BlockImportRecord> = None;
    let mut completed_groups = BTreeSet::new();
    for import in imports {
        let Some(condition) = import.condition.as_ref() else {
            previous = None;
            completed_groups.clear();
            continue;
        };
        if previous.is_none() && completed_groups.contains(condition) {
            adapter.push(
                module,
                diagnostics,
                RuleReport::new(
                    RuleId::CfgUseGroupSpacing,
                    LocalizedMessage::new(CFG_USE_GROUP_SPLIT).with_text("condition", condition),
                    Some(import.start..import.end),
                ),
            );
        }
        if let Some(previous_import) = previous {
            let previous_condition = previous_import.condition.as_ref().unwrap_or(condition);
            if previous_condition != condition {
                if completed_groups.contains(condition) {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::CfgUseGroupSpacing,
                            LocalizedMessage::new(CFG_USE_GROUP_SPLIT)
                                .with_text("condition", condition),
                            Some(import.start..import.end),
                        ),
                    );
                }
                completed_groups.insert(previous_condition.clone());
                let blank_lines = SyntaxSupport::blank_lines_between(
                    module.parse_output(),
                    previous_import.end,
                    import.start,
                );
                if blank_lines != 1 {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::CfgUseGroupSpacing,
                            LocalizedMessage::new(CFG_USE_GROUP_SPACING)
                                .with_number("empty_lines", blank_lines as i64),
                            Some(import.start..import.end),
                        ),
                    );
                }
            }
        }
        previous = Some(import);
    }
}

fn configured_condition(module: &ModuleNode, node: &SyntaxNode) -> Option<String> {
    module
        .configured_ast()
        .node(node.id())
        .filter(|configured| configured.decision() != CfgDecision::False)
        .and_then(|configured| configured.effective_condition())
        .map(str::to_owned)
}

fn node_is_potentially_enabled(module: &ModuleNode, node: &SyntaxNode) -> bool {
    module
        .configured_ast()
        .node(node.id())
        .is_some_and(|configured| configured.decision() != CfgDecision::False)
}

fn line_of(output: &ParseOutput, offset: usize) -> u32 {
    output
        .source()
        .line_column(offset)
        .map_or(1, |(line, _)| line)
}
