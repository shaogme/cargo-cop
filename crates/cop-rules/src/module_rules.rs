use std::{ops::Range, path::Path};

use cop_analysis::{AnalysisSnapshot, ModuleNode};
use cop_config::ResolvedConfig;
use cop_model::{
    Diagnostic, LocalizedMessage, RuleId, CHILD_MODULE_LINE_COUNT, CHILD_MODULE_VISIBILITY,
    ITEM_SPACING, MODULE_FILE_LAYOUT, NO_PARENT_PATH_ATTRIBUTE, OUT_OF_LINE_MODULES_AT_SCOPE_START,
};
use cop_syntax::{
    CoverageStatus, Delimiter, Item, ItemKind, LiteralKind, ParseOutput, RustStringLiteral, Token,
    TokenKind, VisibilityKind,
};

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

fn check_parent_path_attribute_tokens(
    tokens: &[&Token],
    output: &ParseOutput,
    depth: usize,
) -> bool {
    if depth > 16 {
        return false;
    }
    if let [name, equals, value] = tokens {
        if name.text(output.source()) == "path"
            && equals.text(output.source()) == "="
            && matches!(value.kind(), TokenKind::Literal(LiteralKind::String))
        {
            return RustStringLiteral::decode(value.text(output.source()))
                .is_some_and(|path| has_parent_component(&path));
        }
        return false;
    }
    let Some((open_index, close_index)) = cfg_attr_bounds(tokens, output) else {
        return false;
    };
    let Some(arguments) = split_cfg_attr_arguments(tokens, open_index, close_index) else {
        return false;
    };
    arguments
        .iter()
        .skip(1)
        .any(|range| check_parent_path_attribute_tokens(&tokens[range.clone()], output, depth + 1))
}

fn cfg_attr_bounds(tokens: &[&Token], output: &ParseOutput) -> Option<(usize, usize)> {
    if tokens.len() < 3 || tokens[0].text(output.source()) != "cfg_attr" {
        return None;
    }
    if !matches!(
        tokens[1].kind(),
        TokenKind::OpenDelimiter(Delimiter::Parenthesis)
    ) || !matches!(
        tokens.last()?.kind(),
        TokenKind::CloseDelimiter(Delimiter::Parenthesis)
    ) {
        return None;
    }
    Some((1, tokens.len() - 1))
}

fn split_cfg_attr_arguments(
    tokens: &[&Token],
    open_index: usize,
    close_index: usize,
) -> Option<Vec<Range<usize>>> {
    let mut ranges = Vec::new();
    let mut start = open_index + 1;
    let mut nesting = 0usize;
    for (index, token) in tokens
        .iter()
        .enumerate()
        .take(close_index)
        .skip(open_index + 1)
    {
        match token.kind() {
            TokenKind::OpenDelimiter(_) => nesting += 1,
            TokenKind::CloseDelimiter(_) => nesting = nesting.checked_sub(1)?,
            TokenKind::Punctuation(punctuation) if punctuation == "," && nesting == 0 => {
                if start == index {
                    return None;
                }
                ranges.push(start..index);
                start = index + 1;
            }
            _ => {}
        }
    }
    if nesting != 0 {
        return None;
    }
    if start < close_index {
        ranges.push(start..close_index);
    }
    (ranges.len() >= 2).then_some(ranges)
}

fn has_parent_component(path: &str) -> bool {
    path.split(['/', '\\']).any(|component| component == "..")
}

fn check_parent_path_attributes(
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    let output = module.parse_output();
    for item in module.source_items() {
        let ItemKind::Module(child) = item.kind() else {
            continue;
        };
        if child.is_inline() {
            continue;
        }
        for attribute in item.attributes() {
            if attribute.is_inner() || attribute.path().len() != 1 {
                continue;
            }
            let attribute_name = attribute.path()[0].as_str();
            if !matches!(attribute_name, "path" | "cfg_attr") {
                continue;
            }
            let tokens = SyntaxSupport::token_range(output, attribute.start(), attribute.end())
                .collect::<Vec<_>>();
            let body_start = if tokens
                .get(1)
                .is_some_and(|token| token.text(output.source()) == "!")
            {
                3
            } else {
                2
            };
            let Some(body_end) = tokens.len().checked_sub(1) else {
                continue;
            };
            let Some(body) = tokens.get(body_start..body_end) else {
                continue;
            };
            if !check_parent_path_attribute_tokens(body, output, 0) {
                continue;
            }
            adapter.push(
                module,
                diagnostics,
                RuleReport::new(
                    RuleId::NoParentPathAttribute,
                    LocalizedMessage::new(NO_PARENT_PATH_ATTRIBUTE),
                    Some(attribute.start()..attribute.end()),
                ),
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
            if module.parse_output().coverage() != CoverageStatus::Complete {
                if !module.is_order_only() {
                    Self::child_module_visibility(snapshot, module, diagnostics, &adapter);
                }
                continue;
            }
            Self::out_of_line_modules(module, diagnostics, &adapter);
            Self::no_parent_path_attribute(module, diagnostics, &adapter);
            if module.is_order_only() {
                continue;
            }
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
        for item in module.items() {
            let ItemKind::Module(child) = item.kind() else {
                continue;
            };
            if !SyntaxSupport::is_non_private(item) {
                continue;
            }
            let visibility_allows_exception =
                Self::visibility_allows_exception(item.visibility().kind());
            let contents_are_declarative = visibility_allows_exception
                && snapshot
                    .child_module(module, child.name())
                    .is_some_and(|child_module| {
                        child_module.parse_output().coverage() == CoverageStatus::Complete
                            && child_module.items().iter().all(Self::is_declarative_item)
                    });
            if contents_are_declarative {
                continue;
            }
            adapter.push(
                module,
                diagnostics,
                RuleReport::new(
                    RuleId::ChildModuleVisibility,
                    LocalizedMessage::new(CHILD_MODULE_VISIBILITY)
                        .with_text("module", child.name()),
                    Some(item.start()..item.end()),
                ),
            );
        }
    }

    fn is_declarative_item(item: &Item) -> bool {
        match item.kind() {
            ItemKind::Module(_)
            | ItemKind::MacroDefinition(_)
            | ItemKind::Struct(_)
            | ItemKind::Enum(_)
            | ItemKind::Union(_)
            | ItemKind::TypeAlias(_) => true,
            ItemKind::Trait(_) | ItemKind::Constant(_) | ItemKind::Static(_) => true,
            ItemKind::Use(_) => {
                matches!(item.visibility().kind(), VisibilityKind::Private)
                    || Self::visibility_allows_exception(item.visibility().kind())
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
