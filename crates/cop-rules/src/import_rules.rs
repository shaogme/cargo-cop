mod block_imports;

use cop_i18n::{t, PluralCount, RenderError};
use std::{collections::BTreeSet, ops::Range};

use cop_analysis::{
    AnalysisReference, AnalysisSnapshot, ItemFact, ItemKindFact, ModuleNode, PathResolutionKind,
    ReferenceKind,
};
use cop_config::ResolvedConfig;
use cop_model::{Diagnostic, RuleId};
use cop_syntax::ParseOutput;

use crate::support::{RuleDiagnosticAdapter, RuleReport, SyntaxSupport, UseLeaf};

pub(super) struct ImportRuleSet;

struct ImportRecord<'a> {
    item: &'a ItemFact,
    leaves: Vec<UseLeaf>,
    condition: String,
}

impl ImportRuleSet {
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
            Self::extern_crate_at_scope_start(snapshot, module, diagnostics, &adapter)?;
            let use_tree_ranges =
                Self::imports_at_scope_start(module, snapshot, diagnostics, &adapter)?;
            Self::inline_qualified_paths(
                module,
                snapshot,
                &use_tree_ranges,
                diagnostics,
                &adapter,
            )?;
            Self::common_prefixes(module, snapshot, diagnostics, &adapter)?;
            Self::cfg_group_spacing(snapshot, module, diagnostics, &adapter)?;
            Self::cfg_inside_use_tree(snapshot, module, diagnostics, &adapter)?;
        }
        Ok(())
    }

    fn inline_qualified_paths(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        use_tree_ranges: &[Range<usize>],
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        for reference in snapshot
            .facts()
            .references_for_module(module)
            .filter(|reference| reference.kind() != ReferenceKind::Use)
        {
            let bounds = reference.bounds();
            if !is_inside_use_tree(bounds.start, use_tree_ranges) {
                Self::check_qualified_path(reference, module, diagnostics, adapter)?;
            }
        }
        Ok(())
    }

    fn check_qualified_path(
        reference: &AnalysisReference,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        let segments = reference.segments();
        if segments.len() < 2 {
            return Ok(());
        }
        let explicitly_scoped = segments
            .first()
            .is_some_and(|segment| matches!(segment.as_str(), "crate" | "self" | "super"));
        if !explicitly_scoped {
            let resolution = reference.resolution();
            if resolution.kind() != PathResolutionKind::ExternalCrate
                || resolution.external_crate().is_none()
                || !resolution.is_external_crate_qualifier()
            {
                return Ok(());
            }
        }
        let path = path_label(segments);
        adapter.push(
            module,
            diagnostics,
            RuleReport::new(
                RuleId::NoInlineQualifiedPath,
                t!("rule.no_inline_qualified_path", { path: path.as_str() })?,
                Some(reference.bounds()),
            ),
        );
        Ok(())
    }

    fn imports_at_scope_start(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<Vec<Range<usize>>, RenderError> {
        let scope = snapshot
            .facts()
            .syntax_for_module(module)
            .and_then(|syntax| syntax.root_scope());
        let mut preceding_non_imports: Vec<Option<String>> = Vec::new();
        for item in scope.into_iter().flat_map(|scope| scope.items()) {
            if !item.is_enabled() {
                continue;
            }
            let condition = item.condition().map(str::to_owned);
            match item.kind() {
                ItemKindFact::ExternCrate => {}
                ItemKindFact::Module { inline: false } => {}
                ItemKindFact::Use => {
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
                                t!("rule.use_at_scope_start")?,
                                Some(item.bounds()),
                            ),
                        );
                    }
                }
                _ => preceding_non_imports.push(condition),
            }
        }
        block_imports::BlockImportRuleSet::check_module(snapshot, module, diagnostics, adapter)
    }

    fn extern_crate_at_scope_start(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        let scope = snapshot
            .facts()
            .syntax_for_module(module)
            .and_then(|syntax| syntax.root_scope());
        let mut preceding_non_extern_crates: Vec<Option<String>> = Vec::new();
        for item in scope.into_iter().flat_map(|scope| scope.items()) {
            if !item.is_enabled() {
                continue;
            }
            let condition = item.condition().map(str::to_owned);
            if matches!(item.kind(), ItemKindFact::ExternCrate) {
                if preceding_non_extern_crates.iter().any(|previous| {
                    SyntaxSupport::conditions_may_coexist(condition.as_deref(), previous.as_deref())
                }) {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::ExternCrateAtScopeStart,
                            t!("rule.extern_crate_at_scope_start")?,
                            Some(item.bounds()),
                        ),
                    );
                }
            } else {
                preceding_non_extern_crates.push(condition);
            }
        }
        Ok(())
    }

    fn common_prefixes(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        let output = module.parse_output();
        let Some(scope) = snapshot
            .facts()
            .syntax_for_module(module)
            .and_then(|syntax| syntax.root_scope())
        else {
            return Ok(());
        };
        let imports = scope
            .items()
            .iter()
            .filter_map(|item| {
                if !item.is_enabled() || !matches!(item.kind(), ItemKindFact::Use) {
                    return None;
                }
                let leaves = SyntaxSupport::use_facts(snapshot.facts(), module, item.bounds());
                let condition = item.condition().unwrap_or("all()").to_owned();
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
                let same_visibility = previous.item.visibility() == current.item.visibility();
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
                        t!("rule.merge_import_prefix", { line: line_of(output, previous.item.bounds().start) as i64, prefix: prefix.as_str() })?,
                        Some(current.item.bounds()),
                    ),
                );
                break;
            }
        }
        Ok(())
    }

    fn cfg_group_spacing(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        let output = module.parse_output();
        let Some(scope) = snapshot
            .facts()
            .syntax_for_module(module)
            .and_then(|syntax| syntax.root_scope())
        else {
            return Ok(());
        };
        let mut previous: Option<(&ItemFact, String)> = None;
        let mut completed_groups = BTreeSet::new();
        for item in scope.items().iter().filter(|item| item.is_enabled()) {
            if !matches!(item.kind(), ItemKindFact::Use) {
                if let Some((_, previous_condition)) = previous.take() {
                    completed_groups.insert(previous_condition);
                }
                continue;
            }
            let condition = item.condition().unwrap_or("all()").to_owned();
            if previous.is_none() && completed_groups.contains(&condition) {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::CfgUseGroupSpacing,
                        t!("rule.cfg_use_group_split", { condition: &condition })?,
                        Some(item.bounds()),
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
                                t!("rule.cfg_use_group_split", { condition: &condition })?,
                                Some(item.bounds()),
                            ),
                        );
                    }
                    completed_groups.insert(previous_condition);
                    let blank_lines = SyntaxSupport::blank_lines_between(
                        output,
                        previous_item.bounds().end,
                        item.bounds().start,
                    );
                    if blank_lines != 1 {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::CfgUseGroupSpacing,
                                t!("rule.cfg_use_group_spacing", { empty_lines: PluralCount::new((blank_lines as i64) as u64) })?,
                                Some(item.bounds()),
                            ),
                        );
                    }
                }
            }
            previous = Some((item, condition));
        }
        Ok(())
    }

    fn cfg_inside_use_tree(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
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
            if !matches!(item.kind(), ItemKindFact::Use) {
                continue;
            }
            if let Some(bounds) = item.nested_cfg_ranges().first() {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::CfgInsideUseTree,
                        t!("rule.cfg_inside_use_tree")?,
                        Some(bounds.clone()),
                    ),
                );
            }
        }
        Ok(())
    }
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
