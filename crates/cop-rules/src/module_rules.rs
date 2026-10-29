use cop_i18n::{t, PluralCount, RenderError};
use std::path::Path;

use crate::support::{RuleDiagnosticAdapter, RuleReport, SyntaxSupport};
use cop_analysis::{
    AnalysisSnapshot, CfgCondition, ItemFact, ItemKindFact, ModuleNode, SymbolVisibility,
};
use cop_config::ResolvedConfig;
use cop_model::{Diagnostic, RuleId};

pub(super) struct ModuleRuleSet;

fn check_item_spacing_items(
    items: &[ItemFact],
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) -> Result<(), RenderError> {
    for pair in items.windows(2) {
        let previous = &pair[0];
        let current = &pair[1];
        if !previous.is_enabled() || !current.is_enabled() {
            continue;
        }
        if !SyntaxSupport::conditions_may_coexist(previous.condition(), current.condition()) {
            continue;
        }
        let Some(expected_lines) = SyntaxSupport::expected_spacing_blank_lines(previous, current)
        else {
            continue;
        };
        let previous_bounds = previous.bounds();
        let current_bounds = current.bounds();
        let actual_lines = SyntaxSupport::blank_lines_between(
            module.parse_output(),
            previous_bounds.end,
            current_bounds.start,
        );
        if !expected_lines.contains(&actual_lines) {
            let actual = PluralCount::new((actual_lines as i64) as u64);
            let message = if expected_lines.start() == expected_lines.end() {
                t!("rule.item_spacing", { expected: *expected_lines.start() as i64, actual })?
            } else {
                t!("rule.item_spacing_maximum", { maximum: *expected_lines.end() as i64, actual })?
            };
            adapter.push(
                module,
                diagnostics,
                RuleReport::new(RuleId::ItemSpacing, message, Some(current_bounds)),
            );
        }
    }
    Ok(())
}

fn has_parent_component(path: &str) -> bool {
    path.split(['/', '\\']).any(|component| component == "..")
}

fn merge_condition(parent: Option<&str>, local: Option<&str>) -> Option<String> {
    match (parent, local) {
        (None, None) => None,
        (Some(condition), None) | (None, Some(condition)) => Some(condition.to_owned()),
        (Some(parent), Some(local)) => {
            match (CfgCondition::parse(parent), CfgCondition::parse(local)) {
                (Ok(parent), Ok(local)) => Some(CfgCondition::all([parent, local]).normalized()),
                _ => Some(format!("all({parent}, {local})")),
            }
        }
    }
}

fn check_parent_path_attributes(
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
        if !matches!(item.kind(), ItemKindFact::Module { inline: false }) {
            continue;
        }
        for path_attribute in item.path_attributes() {
            let Some(path) = path_attribute.path() else {
                continue;
            };
            if !has_parent_component(path) {
                continue;
            }
            adapter.push(
                module,
                diagnostics,
                RuleReport::new(
                    RuleId::NoParentPathAttribute,
                    t!("rule.no_parent_path_attribute")?,
                    Some(path_attribute.bounds()),
                )
                .with_cfg_condition(merge_condition(
                    item.condition(),
                    path_attribute.condition(),
                )),
            );
        }
    }
    Ok(())
}

impl ModuleRuleSet {
    pub(super) fn run(
        snapshot: &AnalysisSnapshot,
        configuration: &ResolvedConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Result<(), RenderError> {
        let adapter = RuleDiagnosticAdapter::new(configuration);
        for module in snapshot.modules() {
            Self::no_parent_path_attribute(snapshot, module, diagnostics, &adapter)?;
            if module.is_order_only() {
                continue;
            }
            Self::out_of_line_modules(snapshot, module, diagnostics, &adapter)?;
            Self::module_layout(module, diagnostics, &adapter)?;
            Self::child_module_visibility(snapshot, module, diagnostics, &adapter)?;
            Self::item_spacing(snapshot, module, diagnostics, &adapter)?;
            Self::module_line_count(snapshot, module, diagnostics, &adapter)?;
        }
        Ok(())
    }

    fn module_layout(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        if module.is_crate_root()
            || module.is_inline()
            || !module
                .file_path()
                .file_name()
                .is_some_and(|name| name == "mod.rs")
        {
            return Ok(());
        }
        if let Some(span) = module.declared_at() {
            adapter.push(
                module,
                diagnostics,
                RuleReport::at_span(
                    RuleId::ModuleFileLayout,
                    t!("rule.module_file_layout")?,
                    span.clone(),
                ),
            );
        }
        Ok(())
    }

    fn out_of_line_modules(
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
        let mut preceding_non_modules: Vec<Option<String>> = Vec::new();
        for item in scope.items().iter().filter(|item| item.is_enabled()) {
            let condition = item.condition().map(str::to_owned);
            match item.kind() {
                ItemKindFact::ExternCrate => {}
                ItemKindFact::Module { inline: false } => {
                    if preceding_non_modules.iter().any(|previous| {
                        SyntaxSupport::conditions_may_coexist(
                            condition.as_deref(),
                            previous.as_deref(),
                        )
                    }) {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::OutOfLineModulesAtScopeStart,
                                t!("rule.out_of_line_modules_at_scope_start")?,
                                Some(item.bounds()),
                            ),
                        );
                    }
                }
                _ => preceding_non_modules.push(condition),
            }
        }
        Ok(())
    }

    fn no_parent_path_attribute(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        check_parent_path_attributes(snapshot, module, diagnostics, adapter)
    }

    fn child_module_visibility(
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
            let ItemKindFact::Module { inline } = item.kind() else {
                continue;
            };
            let Some(name) = item.name() else {
                continue;
            };
            if matches!(item.visibility(), SymbolVisibility::Private) {
                continue;
            }
            let visibility_allows_exception = matches!(item.visibility(), SymbolVisibility::Public)
                || matches!(
                    item.visibility(),
                    SymbolVisibility::Restricted(scope) if scope == "(crate)"
                );
            let contents_are_declarative = visibility_allows_exception
                && snapshot
                    .child_module(module, name)
                    .is_some_and(|child_module| {
                        snapshot
                            .facts()
                            .syntax_for_module(child_module)
                            .is_some_and(|syntax| {
                                (if *inline {
                                    syntax.has_declarative_scope_coverage()
                                } else {
                                    syntax.has_declarative_coverage()
                                }) && syntax.root_scope().is_some_and(|scope| {
                                    scope
                                        .items()
                                        .iter()
                                        .filter(|item| item.is_enabled())
                                        .all(ItemFact::is_declarative)
                                })
                            })
                    });
            if contents_are_declarative {
                continue;
            }
            adapter.push(
                module,
                diagnostics,
                RuleReport::new(
                    RuleId::ChildModuleVisibility,
                    t!("rule.child_module_visibility", { module: name })?,
                    Some(item.bounds()),
                ),
            );
        }
        Ok(())
    }

    fn item_spacing(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        let Some(syntax) = snapshot.facts().syntax_for_module(module) else {
            return Ok(());
        };
        for scope in syntax.scopes() {
            check_item_spacing_items(scope.items(), module, diagnostics, adapter)?;
        }
        Ok(())
    }

    fn module_line_count(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        if module.is_crate_root() || module.is_inline() {
            return Ok(());
        }
        let line_count = snapshot
            .facts()
            .syntax_for_module(module)
            .map_or(0, |syntax| syntax.code_line_count());
        if (70..=800).contains(&line_count) {
            return Ok(());
        }
        let file_name = module
            .display_path()
            .file_name()
            .unwrap_or_else(|| Path::new("").as_os_str())
            .to_string_lossy()
            .into_owned();
        adapter.push(
            module,
            diagnostics,
            RuleReport::new(
                RuleId::ChildModuleLineCount,
                t!("rule.child_module_line_count", { file: file_name.as_str(), actual: line_count as i64, minimum: 70, maximum: 800 })?,
                Some(0..0),
            ),
        );
        Ok(())
    }
}
