use std::{ops::Range, path::Path};

use cop_analysis::{AnalysisSnapshot, CfgCondition, CfgDecision, ModuleNode};
use cop_config::ResolvedConfig;
use cop_model::{
    Diagnostic, LocalizedMessage, RuleId, CHILD_MODULE_LINE_COUNT, CHILD_MODULE_VISIBILITY,
    ITEM_SPACING, MODULE_FILE_LAYOUT, NO_PARENT_PATH_ATTRIBUTE, OUT_OF_LINE_MODULES_AT_SCOPE_START,
};
use cop_syntax::{ItemSyntaxKind, ParseOutput, SyntaxNode, SyntaxNodeKind, VisibilityKind};

use crate::support::{RuleDiagnosticAdapter, RuleReport, SyntaxSupport};

pub(super) struct ModuleRuleSet;

fn check_item_spacing(
    items: &[&SyntaxNode],
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    check_item_spacing_nodes(items, module, diagnostics, adapter);
}

fn check_item_spacing_nodes(
    nodes: &[&SyntaxNode],
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    for pair in nodes.windows(2) {
        let previous = pair[0];
        let current = pair[1];
        if !SyntaxSupport::conditions_may_coexist(
            module
                .configured_ast()
                .condition_for_span(previous.start(), previous.end()),
            module
                .configured_ast()
                .condition_for_span(current.start(), current.end()),
        ) {
            continue;
        }
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
    for node in nodes {
        if node.item_syntax().is_some_and(|item_data| {
            matches!(
                item_data.kind(),
                ItemSyntaxKind::Trait(_) | ItemSyntaxKind::Impl
            )
        }) {
            let children = SyntaxSupport::child_item_nodes_for_node(module, node);
            check_item_spacing_nodes(&children, module, diagnostics, adapter);
        }
    }
}

fn for_each_test_range(output: &ParseOutput, excluded: &mut Vec<Range<usize>>) {
    let Some(root) = output
        .syntax_nodes()
        .iter()
        .find(|node| node.parent().is_none())
    else {
        return;
    };
    let mut pending = vec![root.id()];
    while let Some(id) = pending.pop() {
        let Some(node) = output.node(id) else {
            continue;
        };
        if SyntaxSupport::is_cfg_test(node.attributes()) {
            excluded.push(node.start()..node.end());
            continue;
        }
        pending.extend(node.children().iter().rev().copied());
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
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    for item in module.item_nodes() {
        let Some(ItemSyntaxKind::Module {
            name: _,
            inline: false,
        }) = item.item_syntax().map(|data| data.kind())
        else {
            continue;
        };
        let Some(configured) = module.configured_ast().node(item.id()) else {
            continue;
        };
        for path_attribute in configured.attribute_evaluation().path_attributes() {
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
                    Some(path_attribute.start()..path_attribute.end()),
                )
                .with_cfg_condition(merge_condition(
                    configured.effective_condition(),
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
            Self::no_parent_path_attribute(module, diagnostics, &adapter);
            if module.is_order_only() {
                continue;
            }
            Self::out_of_line_modules(module, diagnostics, &adapter);
            Self::module_layout(module, diagnostics, &adapter);
            Self::child_module_visibility(snapshot, module, diagnostics, &adapter);
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
        let mut preceding_non_modules: Vec<Option<String>> = Vec::new();
        for item in module.item_nodes() {
            let condition = module
                .configured_ast()
                .condition_for_span(item.start(), item.end())
                .map(str::to_owned);
            match item.item_syntax().map(|data| data.kind()) {
                Some(ItemSyntaxKind::ExternCrate { .. }) => {}
                Some(ItemSyntaxKind::Module { inline: false, .. }) => {
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
                                Some(item.start()..item.end()),
                            ),
                        );
                    }
                }
                _ => preceding_non_modules.push(condition),
            }
        }
    }

    fn no_parent_path_attribute(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        check_parent_path_attributes(module, diagnostics, adapter);
    }

    fn child_module_visibility(
        snapshot: &AnalysisSnapshot,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for item in module.item_nodes() {
            let Some(item_data) = item.item_syntax() else {
                continue;
            };
            let ItemSyntaxKind::Module { name, inline } = item_data.kind() else {
                continue;
            };
            if !SyntaxSupport::is_non_private(item) {
                continue;
            }
            let visibility_allows_exception =
                Self::visibility_allows_exception(item_data.visibility().kind());
            let contents_are_declarative = visibility_allows_exception
                && snapshot
                    .child_module(module, name)
                    .is_some_and(|child_module| {
                        Self::has_declarative_coverage(
                            child_module,
                            inline.then_some(child_module.scope_range()),
                        ) && child_module
                            .item_nodes()
                            .iter()
                            .all(|child| Self::is_declarative_item(child))
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
                    Some(item.start()..item.end()),
                ),
            );
        }
    }

    fn has_declarative_coverage(module: &ModuleNode, scope: Option<Range<usize>>) -> bool {
        let output = module.parse_output();
        let has_issues = output.issues().iter().any(|issue| {
            scope.as_ref().map_or(true, |range| {
                range.start <= issue.start() && issue.end() <= range.end
            }) && module.configured_ast().is_issue_enabled(issue)
        });
        let has_non_declarative_opaque = output
            .syntax_nodes()
            .iter()
            .filter(|node| {
                node.kind() == SyntaxNodeKind::Opaque
                    && scope.as_ref().map_or(true, |range| {
                        range.start <= node.start() && node.end() <= range.end
                    })
                    && module
                        .configured_ast()
                        .node(node.id())
                        .is_some_and(|configured| configured.decision() != CfgDecision::False)
                    && {
                        let issues = output.issues().iter().filter(|issue| {
                            node.start() <= issue.start() && issue.end() <= node.end()
                        });
                        let mut has_issue = false;
                        let mut has_enabled_issue = false;
                        for issue in issues {
                            has_issue = true;
                            has_enabled_issue |= module.configured_ast().is_issue_enabled(issue);
                        }
                        !has_issue || has_enabled_issue
                    }
            })
            .any(|node| {
                node.parent()
                    .and_then(|parent| output.node(parent))
                    .map_or(true, |parent| {
                        parent.kind() != SyntaxNodeKind::MacroDefinition
                    })
            });
        !has_issues && !has_non_declarative_opaque
    }

    fn is_declarative_item(item: &SyntaxNode) -> bool {
        let Some(item_data) = item.item_syntax() else {
            return false;
        };
        match item_data.kind() {
            ItemSyntaxKind::Module { .. }
            | ItemSyntaxKind::MacroDefinition(_)
            | ItemSyntaxKind::Struct(_)
            | ItemSyntaxKind::Enum(_)
            | ItemSyntaxKind::Union(_)
            | ItemSyntaxKind::TypeAlias(_) => true,
            ItemSyntaxKind::Trait(_) | ItemSyntaxKind::Constant(_) | ItemSyntaxKind::Static(_) => {
                true
            }
            ItemSyntaxKind::Use => {
                matches!(item_data.visibility().kind(), VisibilityKind::Private)
                    || Self::visibility_allows_exception(item_data.visibility().kind())
            }
            _ => false,
        }
    }

    fn visibility_allows_exception(visibility: &VisibilityKind) -> bool {
        match visibility {
            VisibilityKind::Public => true,
            VisibilityKind::Restricted(scope) => scope == "(crate)",
            VisibilityKind::Private => false,
        }
    }

    fn item_spacing(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        let items = module.item_nodes();
        check_item_spacing(&items, module, diagnostics, adapter);
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
        let mut test_ranges = Vec::new();
        for_each_test_range(output, &mut test_ranges);
        let test_module = SyntaxSupport::is_cfg_test(output.inner_attributes());
        let mut lines = Vec::new();
        if !test_module {
            for token in output.tokens() {
                if !module
                    .configured_ast()
                    .is_enabled_range(token.start(), token.end())
                    || test_ranges
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
