use std::{ops::Range, path::Path};

use cop_analysis::{AnalysisSnapshot, ModuleNode};
use cop_config::ResolvedConfig;
use cop_model::{
    Diagnostic, LocalizedMessage, RuleId, CHILD_MODULE_LINE_COUNT, ITEM_SPACING,
    MODULE_FILE_LAYOUT, OUT_OF_LINE_MODULES_AT_SCOPE_START, PRIVATE_CHILD_MODULE,
};
use cop_syntax::{CoverageStatus, Item, ItemKind, ParseOutput};

use crate::support::{RuleDiagnosticAdapter, RuleReport, SyntaxSupport};

pub(super) struct ModuleRuleSet;

fn check_item_spacing(
    items: &[Item],
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    for pair in items.windows(2) {
        let previous = &pair[0];
        let current = &pair[1];
        let Some(expected_lines) = SyntaxSupport::expected_spacing_blank_lines(previous, current)
        else {
            continue;
        };
        let actual_lines = SyntaxSupport::blank_lines_between(
            module.parse_output(),
            previous.end(),
            current.start(),
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
                    Some(current.start()..current.end()),
                ),
            );
        }
    }
    for item in items {
        match item.kind() {
            ItemKind::Trait(trait_item) => {
                check_item_spacing(trait_item.items(), module, diagnostics, adapter);
            }
            ItemKind::Impl(impl_item) => {
                check_item_spacing(impl_item.items(), module, diagnostics, adapter);
            }
            _ => {}
        }
    }
}

fn for_each_test_range(items: &[Item], output: &ParseOutput, excluded: &mut Vec<Range<usize>>) {
    for item in items {
        if SyntaxSupport::is_cfg_test(item.attributes(), output) {
            excluded.push(item.start()..item.end());
            continue;
        }
        match item.kind() {
            ItemKind::Module(child) => for_each_test_range(child.items(), output, excluded),
            ItemKind::Trait(trait_item) => {
                for_each_test_range(trait_item.items(), output, excluded);
            }
            ItemKind::Impl(impl_item) => for_each_test_range(impl_item.items(), output, excluded),
            _ => {}
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
            if module.parse_output().coverage() != CoverageStatus::Complete {
                continue;
            }
            Self::out_of_line_modules(module, diagnostics, &adapter);
            if module.is_order_only() {
                continue;
            }
            Self::module_layout(module, diagnostics, &adapter);
            Self::private_children(module, diagnostics, &adapter);
            Self::item_spacing(module, diagnostics, &adapter);
            Self::module_line_count(module, diagnostics, &adapter);
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
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        let mut encountered_after_module_prefix = false;
        for item in module.source_items() {
            match item.kind() {
                ItemKind::ExternCrate(_) => {}
                ItemKind::Module(child) if !child.is_inline() => {
                    if encountered_after_module_prefix {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::OutOfLineModulesAtScopeStart,
                                LocalizedMessage::new(OUT_OF_LINE_MODULES_AT_SCOPE_START),
                                Some(item.start()..item.end()),
                            ),
                        );
                    }
                }
                _ => encountered_after_module_prefix = true,
            }
        }
    }

    fn private_children(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for item in module.items() {
            let ItemKind::Module(child) = item.kind() else {
                continue;
            };
            if SyntaxSupport::is_non_private(item) {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::PrivateChildModules,
                        LocalizedMessage::new(PRIVATE_CHILD_MODULE)
                            .with_text("module", child.name()),
                        Some(item.start()..item.end()),
                    ),
                );
            }
        }
    }

    fn item_spacing(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        check_item_spacing(module.items(), module, diagnostics, adapter);
    }

    fn module_line_count(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        if module.is_crate_root() || module.is_inline() {
            return;
        }
        let output = module.parse_output();
        let mut excluded = Vec::new();
        for_each_test_range(output.items(), output, &mut excluded);
        let test_module = SyntaxSupport::is_cfg_test(output.inner_attributes(), output);
        let mut lines = Vec::new();
        if !test_module {
            for token in output.tokens() {
                if excluded
                    .iter()
                    .any(|range: &Range<usize>| range.contains(&token.start()))
                {
                    continue;
                }
                if let Some((start_line, _)) = output.source().line_column(token.start()) {
                    let line_breaks = token
                        .text(output.source())
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count();
                    let line_breaks = u32::try_from(line_breaks).unwrap_or(u32::MAX);
                    let end_line = start_line.saturating_add(line_breaks);
                    for line in start_line..=end_line {
                        if lines.last() != Some(&line) {
                            lines.push(line);
                        }
                    }
                }
            }
        }
        let line_count = lines.len();
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
