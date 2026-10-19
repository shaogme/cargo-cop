mod block_imports;

use std::{collections::BTreeSet, ops::Range};

use cop_analysis::{AnalysisSnapshot, CfgDecision, ModuleNode, PathResolutionKind};
use cop_config::ResolvedConfig;
use cop_model::{
    Diagnostic, LocalizedMessage, RuleId, CFG_INSIDE_USE_TREE, CFG_USE_GROUP_SPACING,
    CFG_USE_GROUP_SPLIT, EXTERN_CRATE_AT_SCOPE_START, MERGE_IMPORT_PREFIX,
    NO_INLINE_QUALIFIED_PATH, USE_AT_SCOPE_START,
};
use cop_syntax::{ItemSyntaxKind, ParseOutput, SyntaxNode, SyntaxNodeKind};

use crate::support::{PathReference, RuleDiagnosticAdapter, RuleReport, SyntaxSupport, UseLeaf};

pub(super) struct ImportRuleSet;

struct ImportRecord<'a> {
    item: &'a SyntaxNode,
    leaves: Vec<UseLeaf>,
    condition: String,
}

impl ImportRuleSet {
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
            Self::extern_crate_at_scope_start(module, diagnostics, &adapter);
            let use_tree_ranges = Self::imports_at_scope_start(module, diagnostics, &adapter);
            Self::inline_qualified_paths(module, snapshot, &use_tree_ranges, diagnostics, &adapter);
            Self::common_prefixes(module, diagnostics, &adapter);
            Self::cfg_group_spacing(module, diagnostics, &adapter);
            Self::cfg_inside_use_tree(module, diagnostics, &adapter);
        }
    }

    fn inline_qualified_paths(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        use_tree_ranges: &[Range<usize>],
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for item in module.item_nodes() {
            SyntaxSupport::for_each_path_reference(module, item, true, &mut |reference| {
                if is_inside_use_tree(reference.start, use_tree_ranges) {
                    return;
                }
                Self::check_qualified_path(reference, module, snapshot, diagnostics, adapter);
            });
        }
    }

    fn check_qualified_path(
        reference: &PathReference,
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        if reference.segments.len() < 2 {
            return;
        }
        let explicitly_scoped = reference
            .segments
            .first()
            .is_some_and(|segment| matches!(segment.as_str(), "crate" | "self" | "super"));
        if !explicitly_scoped {
            let resolution = snapshot.symbols().resolve_path_at(
                module,
                &reference.segments,
                Some(reference.start..reference.end),
            );
            if resolution.kind() != PathResolutionKind::ExternalCrate
                || resolution.external_crate().is_none()
                || !resolution.is_external_crate_qualifier()
            {
                return;
            }
        }
        adapter.push(
            module,
            diagnostics,
            RuleReport::new(
                RuleId::NoInlineQualifiedPath,
                LocalizedMessage::new(NO_INLINE_QUALIFIED_PATH)
                    .with_text("path", path_label(&reference.segments)),
                Some(reference.start..reference.end),
            ),
        );
    }

    fn imports_at_scope_start(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Vec<Range<usize>> {
        let mut preceding_non_imports: Vec<Option<String>> = Vec::new();
        for item in module.item_nodes() {
            let condition = module
                .configured_ast()
                .condition_for_span(item.start(), item.end())
                .map(str::to_owned);
            match item.item_syntax().map(|data| data.kind()) {
                Some(ItemSyntaxKind::ExternCrate { .. }) => {}
                Some(ItemSyntaxKind::Module { inline: false, .. }) => {}
                Some(ItemSyntaxKind::Use) => {
                    if preceding_non_imports.iter().any(|previous| {
                        SyntaxSupport::conditions_may_coexist(
                            condition.as_deref(),
                            previous.as_deref(),
                        )
                    }) {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::ImportsAtScopeStart,
                                LocalizedMessage::new(USE_AT_SCOPE_START),
                                Some(item.start()..item.end()),
                            ),
                        );
                    }
                }
                _ => preceding_non_imports.push(condition),
            }
        }
        let mut use_tree_ranges = Vec::new();
        for item in module.item_nodes() {
            use_tree_ranges.extend(block_imports::BlockImportRuleSet::check_item(
                item,
                module,
                diagnostics,
                adapter,
            ));
        }
        use_tree_ranges.sort_unstable_by_key(|range| range.start);
        use_tree_ranges
    }

    fn extern_crate_at_scope_start(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        let mut preceding_non_extern_crates: Vec<Option<String>> = Vec::new();
        for item in module.item_nodes() {
            let condition = module
                .configured_ast()
                .condition_for_span(item.start(), item.end())
                .map(str::to_owned);
            if item
                .item_syntax()
                .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::ExternCrate { .. }))
            {
                if preceding_non_extern_crates.iter().any(|previous| {
                    SyntaxSupport::conditions_may_coexist(condition.as_deref(), previous.as_deref())
                }) {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::ExternCrateAtScopeStart,
                            LocalizedMessage::new(EXTERN_CRATE_AT_SCOPE_START),
                            Some(item.start()..item.end()),
                        ),
                    );
                }
            } else {
                preceding_non_extern_crates.push(condition);
            }
        }
    }

    fn common_prefixes(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        let output = module.parse_output();
        let items = module.item_nodes();
        let imports = items
            .iter()
            .filter_map(|item| {
                if !item
                    .item_syntax()
                    .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Use))
                {
                    return None;
                }
                let tree = SyntaxSupport::use_tree_node_for_item(module, item)?;
                let leaves = SyntaxSupport::use_leaves(module, tree);
                let condition = module
                    .configured_ast()
                    .condition_for_span(item.start(), item.end())
                    .unwrap_or("all()")
                    .to_owned();
                if leaves.is_empty() {
                    return None;
                }
                let condition = if leaves.len() == 1 {
                    leaves[0].condition.clone().unwrap_or(condition)
                } else {
                    condition
                };
                Some(ImportRecord {
                    item,
                    leaves,
                    condition,
                })
            })
            .collect::<Vec<_>>();
        for current_index in 1..imports.len() {
            let current = &imports[current_index];
            for previous in imports[..current_index].iter().rev() {
                let same_visibility = previous
                    .item
                    .item_syntax()
                    .zip(current.item.item_syntax())
                    .is_some_and(|(previous, current)| {
                        previous.visibility().kind() == current.visibility().kind()
                    });
                if previous.condition != current.condition || !same_visibility {
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
                        LocalizedMessage::new(MERGE_IMPORT_PREFIX)
                            .with_number("line", line_of(output, previous.item.start()) as i64)
                            .with_text("prefix", prefix),
                        Some(current.item.start()..current.item.end()),
                    ),
                );
                break;
            }
        }
    }

    fn cfg_group_spacing(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        let output = module.parse_output();
        let mut previous: Option<(&SyntaxNode, String)> = None;
        let mut completed_groups = BTreeSet::new();
        for item in module.item_nodes() {
            if !item
                .item_syntax()
                .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Use))
            {
                if let Some((_, previous_condition)) = previous.take() {
                    completed_groups.insert(previous_condition);
                }
                continue;
            }
            let condition = module
                .configured_ast()
                .condition_for_span(item.start(), item.end())
                .unwrap_or("all()")
                .to_owned();
            if previous.is_none() && completed_groups.contains(&condition) {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::CfgUseGroupSpacing,
                        LocalizedMessage::new(CFG_USE_GROUP_SPLIT)
                            .with_text("condition", &condition),
                        Some(item.start()..item.end()),
                    ),
                );
            }
            if let Some((previous_item, previous_condition)) = previous {
                if previous_condition != condition {
                    if completed_groups.contains(&condition) {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::CfgUseGroupSpacing,
                                LocalizedMessage::new(CFG_USE_GROUP_SPLIT)
                                    .with_text("condition", &condition),
                                Some(item.start()..item.end()),
                            ),
                        );
                    }
                    completed_groups.insert(previous_condition);
                    let blank_lines = SyntaxSupport::blank_lines_between(
                        output,
                        previous_item.end(),
                        item.start(),
                    );
                    if blank_lines != 1 {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::CfgUseGroupSpacing,
                                LocalizedMessage::new(CFG_USE_GROUP_SPACING)
                                    .with_number("empty_lines", blank_lines as i64),
                                Some(item.start()..item.end()),
                            ),
                        );
                    }
                }
            }
            previous = Some((item, condition));
        }
    }

    fn cfg_inside_use_tree(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for item in module.item_nodes() {
            if !item
                .item_syntax()
                .is_some_and(|data| matches!(data.kind(), ItemSyntaxKind::Use))
            {
                continue;
            }
            let Some(tree) = SyntaxSupport::use_tree_node_for_item(module, item) else {
                continue;
            };
            if let Some(bounds) = nested_cfg_in_use_tree(module, tree) {
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
        }
    }
}

fn nested_cfg_in_use_tree(module: &ModuleNode, tree: &SyntaxNode) -> Option<Range<usize>> {
    for child in tree
        .children()
        .iter()
        .filter_map(|id| module.parse_output().node(*id))
        .filter(|node| node.kind() == SyntaxNodeKind::UseTree)
    {
        if !module
            .configured_ast()
            .node(child.id())
            .is_some_and(|configured| configured.decision() != CfgDecision::False)
        {
            continue;
        }
        if let Some(attribute) = child
            .attributes()
            .iter()
            .find(|attribute| attribute.path() == ["cfg"])
        {
            if module.configured_ast().is_supported_range(
                module.parse_output(),
                attribute.start(),
                attribute.end(),
            ) {
                return Some(attribute.start()..attribute.end());
            }
        }
        if let Some(bounds) = nested_cfg_in_use_tree(module, child) {
            return Some(bounds);
        }
    }
    None
}

fn is_inside_use_tree(offset: usize, use_tree_ranges: &[Range<usize>]) -> bool {
    let range_index = use_tree_ranges.partition_point(|range| range.end <= offset);
    use_tree_ranges
        .get(range_index)
        .is_some_and(|range| range.contains(&offset))
}

fn path_label(segments: &[String]) -> String {
    if segments.first().is_some_and(|segment| segment == "::") {
        format!("::{}", segments[1..].join("::"))
    } else {
        segments.join("::")
    }
}

fn line_of(output: &ParseOutput, offset: usize) -> u32 {
    output
        .source()
        .line_column(offset)
        .map_or(1, |(line, _)| line)
}
