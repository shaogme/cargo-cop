use cop_i18n::{t, PluralCount, RenderError};
use std::{collections::BTreeSet, ops::Range};

use cop_analysis::{AnalysisSnapshot, BlockScopeFact, ModuleNode};
use cop_model::{Diagnostic, RuleId};

use cop_syntax::ParseOutput;

use crate::support::{RuleDiagnosticAdapter, RuleReport, SyntaxSupport, UseLeaf};

struct BlockImportRecord {
    bounds: Range<usize>,
    condition: Option<String>,
    leaves: Vec<UseLeaf>,
}

pub(super) struct BlockImportRuleSet;

struct BlockScopeCheck<'a, 'b> {
    module: &'a ModuleNode,
    snapshot: &'a AnalysisSnapshot,
    diagnostics: &'a mut Vec<Diagnostic>,
    adapter: &'a RuleDiagnosticAdapter<'b>,
}

impl BlockImportRuleSet {
    pub(super) fn check_module(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<Vec<Range<usize>>, RenderError> {
        let Some(syntax) = snapshot.facts().syntax_for_module(module) else {
            return Ok(Vec::new());
        };
        let mut use_tree_ranges = Vec::new();
        let mut checker = BlockScopeCheck {
            module,
            snapshot,
            diagnostics,
            adapter,
        };
        for scope in syntax.block_scopes() {
            checker.check(scope, &mut use_tree_ranges)?;
        }
        use_tree_ranges.sort_unstable_by_key(|range| range.start);
        Ok(use_tree_ranges)
    }
}

impl BlockScopeCheck<'_, '_> {
    fn check(
        &mut self,
        scope: &BlockScopeFact,
        use_tree_ranges: &mut Vec<Range<usize>>,
    ) -> Result<(), RenderError> {
        let mut preceding_non_imports = Vec::new();
        let mut imports = Vec::new();
        for item in scope.items() {
            let condition = item.condition().map(str::to_owned);
            if !item.is_use_tree() {
                preceding_non_imports.push(condition);
                continue;
            }
            let bounds = item.bounds();
            use_tree_ranges.push(bounds.clone());
            if let Some(cfg_range) = item.nested_cfg_ranges().first() {
                self.adapter.push(
                    self.module,
                    self.diagnostics,
                    RuleReport::new(
                        RuleId::CfgInsideUseTree,
                        t!("rule.cfg_inside_use_tree")?,
                        Some(cfg_range.clone()),
                    ),
                );
            }
            let import_is_at_scope_start = !preceding_non_imports.iter().any(|previous| {
                SyntaxSupport::conditions_may_coexist(condition.as_deref(), previous.as_deref())
            });
            if !import_is_at_scope_start {
                self.adapter.push(
                    self.module,
                    self.diagnostics,
                    RuleReport::new(
                        RuleId::ImportsAtScopeStart,
                        t!("rule.block_use_at_scope_start")?,
                        Some(bounds.clone()),
                    ),
                );
            }
            if import_is_at_scope_start && item.is_supported() {
                if let Some(import) =
                    parse_block_import(self.snapshot, self.module, bounds, condition)
                {
                    imports.push(import);
                }
            }
        }
        check_block_common_prefixes(&imports, self.module, self.diagnostics, self.adapter)?;
        check_block_cfg_spacing(&imports, self.module, self.diagnostics, self.adapter)?;
        Ok(())
    }
}

fn parse_block_import(
    snapshot: &AnalysisSnapshot,
    module: &ModuleNode,
    bounds: Range<usize>,
    statement_condition: Option<String>,
) -> Option<BlockImportRecord> {
    let leaves = SyntaxSupport::use_facts(snapshot.facts(), module, bounds.clone());
    if leaves.is_empty() {
        return None;
    }
    let condition = if leaves.len() == 1 {
        leaves[0].condition.clone().or(statement_condition)
    } else {
        statement_condition
    }
    .unwrap_or_else(|| "all()".to_owned());
    Some(BlockImportRecord {
        bounds,
        condition: Some(condition),
        leaves,
    })
}

fn check_block_common_prefixes(
    imports: &[BlockImportRecord],
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) -> Result<(), RenderError> {
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
                    t!("rule.merge_block_import_prefix", { line: line_of(module.parse_output(), previous.bounds.start) as i64, prefix: prefix.as_str() })?,
                    Some(current.bounds.clone()),
                ),
            );
            break;
        }
    }
    Ok(())
}

fn check_block_cfg_spacing(
    imports: &[BlockImportRecord],
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) -> Result<(), RenderError> {
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
                    t!("rule.cfg_use_group_split", { condition: condition })?,
                    Some(import.bounds.clone()),
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
                            t!("rule.cfg_use_group_split", { condition: condition })?,
                            Some(import.bounds.clone()),
                        ),
                    );
                }
                completed_groups.insert(previous_condition.clone());
                let blank_lines = SyntaxSupport::blank_lines_between(
                    module.parse_output(),
                    previous_import.bounds.end,
                    import.bounds.start,
                );
                if blank_lines != 1 {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::CfgUseGroupSpacing,
                            t!("rule.cfg_use_group_spacing", { empty_lines: PluralCount::new((blank_lines as i64) as u64) })?,
                            Some(import.bounds.clone()),
                        ),
                    );
                }
            }
        }
        previous = Some(import);
    }
    Ok(())
}

fn line_of(output: &ParseOutput, offset: usize) -> u32 {
    output
        .source()
        .line_column(offset)
        .map_or(1, |(line, _)| line)
}
