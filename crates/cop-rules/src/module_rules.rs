use std::path::Path;

use crate::support::{RuleDiagnosticAdapter, RuleReport, SyntaxSupport};
use cop_analysis::{
    AnalysisSnapshot, CfgCondition, ItemFact, ItemKindFact, ModuleNode, SymbolVisibility,
};
use cop_config::ResolvedConfig;
use cop_model::{
    Diagnostic, LocalizedMessage, RuleId, CHILD_MODULE_LINE_COUNT, CHILD_MODULE_VISIBILITY,
    ITEM_SPACING, MODULE_FILE_LAYOUT, NO_PARENT_PATH_ATTRIBUTE, OUT_OF_LINE_MODULES_AT_SCOPE_START,
};

pub(super) struct ModuleRuleSet;

fn check_item_spacing_items(
    items: &[ItemFact],
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
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
        if actual_lines != expected_lines {
            adapter.push(
                module,
                diagnostics,
                RuleReport::new(
                    RuleId::ItemSpacing,
                    LocalizedMessage::new(ITEM_SPACING)
                        .with_number("expected", expected_lines as i64)
                        .with_number("actual", actual_lines as i64),
                    Some(current_bounds),
                ),
            );
        }
    }
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
) {
    let Some(scope) = snapshot
        .facts()
        .syntax_for_module(module)
        .and_then(|syntax| syntax.root_scope())
    else {
        return;
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
                    LocalizedMessage::new(NO_PARENT_PATH_ATTRIBUTE),
                    Some(path_attribute.bounds()),
                )
                .with_cfg_condition(merge_condition(
                    item.condition(),
                    path_attribute.condition(),
                )),
            );
        }
    }
}

impl ModuleRuleSet {
    pub(super) fn run(
        snapshot: &AnalysisSnapshot,
        configuration: &ResolvedConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let adapter = RuleDiagnosticAdapter::new(configuration);
        for module in snapshot.modules() {
            Self::no_parent_path_attribute(snapshot, module, diagnostics, &adapter);
            if module.is_order_only() {
                continue;
            }
            Self::out_of_line_modules(snapshot, module, diagnostics, &adapter);
            Self::module_layout(module, diagnostics, &adapter);
            Self::child_module_visibility(snapshot, module, diagnostics, &adapter);
            Self::item_spacing(snapshot, module, diagnostics, &adapter);
            Self::module_line_count(snapshot, module, diagnostics, &adapter);
        }
    }

    fn module_layout(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        if module.is_crate_root()
            || module.is_inline()
            || !module
                .file_path()
                .file_name()
                .is_some_and(|name| name == "mod.rs")
        {
            return;
        }
        if let Some(span) = module.declared_at() {
            adapter.push(
                module,
                diagnostics,
                RuleReport::at_span(
                    RuleId::ModuleFileLayout,
                    LocalizedMessage::new(MODULE_FILE_LAYOUT),
                    span.clone(),
                ),
            );
        }
    }

    fn out_of_line_modules(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        let Some(scope) = snapshot
            .facts()
            .syntax_for_module(module)
            .and_then(|syntax| syntax.root_scope())
        else {
            return;
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
                                LocalizedMessage::new(OUT_OF_LINE_MODULES_AT_SCOPE_START),
                                Some(item.bounds()),
                            ),
                        );
                    }
                }
                _ => preceding_non_modules.push(condition),
            }
        }
    }

    fn no_parent_path_attribute(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        check_parent_path_attributes(snapshot, module, diagnostics, adapter);
    }

    fn child_module_visibility(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        let Some(scope) = snapshot
            .facts()
            .syntax_for_module(module)
            .and_then(|syntax| syntax.root_scope())
        else {
            return;
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
                    LocalizedMessage::new(CHILD_MODULE_VISIBILITY).with_text("module", name),
                    Some(item.bounds()),
                ),
            );
        }
    }

    fn item_spacing(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        let Some(syntax) = snapshot.facts().syntax_for_module(module) else {
            return;
        };
        for scope in syntax.scopes() {
            check_item_spacing_items(scope.items(), module, diagnostics, adapter);
        }
    }

    fn module_line_count(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        if module.is_crate_root() || module.is_inline() {
            return;
        }
        let line_count = snapshot
            .facts()
            .syntax_for_module(module)
            .map_or(0, |syntax| syntax.code_line_count());
        if (70..=800).contains(&line_count) {
            return;
        }
        let file_name = module
            .display_path()
            .file_name()
            .unwrap_or_else(|| Path::new("").as_os_str());
        adapter.push(
            module,
            diagnostics,
            RuleReport::new(
                RuleId::ChildModuleLineCount,
                LocalizedMessage::new(CHILD_MODULE_LINE_COUNT)
                    .with_text("file", file_name.to_string_lossy())
                    .with_number("actual", line_count as i64)
                    .with_number("minimum", 70)
                    .with_number("maximum", 800),
                Some(0..0),
            ),
        );
    }
}
