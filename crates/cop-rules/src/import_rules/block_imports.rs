use std::{collections::BTreeSet, ops::Range};

use cop_analysis::ModuleNode;
use cop_model::{
    Diagnostic, LocalizedMessage, RuleId, BLOCK_USE_AT_SCOPE_START, CFG_INSIDE_USE_TREE,
    CFG_USE_GROUP_SPACING, CFG_USE_GROUP_SPLIT, MERGE_BLOCK_IMPORT_PREFIX,
};
use cop_syntax::{
    Delimiter, FunctionBody, Item, ItemKind, ParseOutput, RustParser, SourceFile, Token, TokenKind,
};

use crate::support::{RuleDiagnosticAdapter, RuleReport, SyntaxSupport, UseLeaf};

struct BlockScope {
    open_index: usize,
    close_index: usize,
}

struct BlockImportRecord {
    start: usize,
    end: usize,
    condition: Option<String>,
    leaves: Vec<UseLeaf>,
}

pub(super) struct BlockImportRuleSet;

impl BlockImportRuleSet {
    pub(super) fn check_item(
        item: &Item,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Vec<Range<usize>> {
        match item.kind() {
            ItemKind::Function(function) => {
                if let Some(body) = function.body() {
                    check_function_blocks(body, module, diagnostics, adapter)
                } else {
                    Vec::new()
                }
            }
            ItemKind::Trait(trait_item) => trait_item
                .items()
                .iter()
                .flat_map(|nested| Self::check_item(nested, module, diagnostics, adapter))
                .collect(),
            ItemKind::Impl(impl_item) => impl_item
                .items()
                .iter()
                .flat_map(|nested| Self::check_item(nested, module, diagnostics, adapter))
                .collect(),
            _ => Vec::new(),
        }
    }
}

fn check_function_blocks(
    body: &FunctionBody,
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) -> Vec<Range<usize>> {
    let output = module.parse_output();
    let tokens = output.tokens();
    let Some(open_index) = tokens.iter().position(|token| {
        token.start() == body.start()
            && matches!(token.kind(), TokenKind::OpenDelimiter(Delimiter::Brace))
    }) else {
        return Vec::new();
    };
    let Some(close_index) = matching_delimiter(tokens, open_index) else {
        return Vec::new();
    };
    let mut scopes = vec![BlockScope {
        open_index,
        close_index,
    }];
    collect_nested_blocks(tokens, output, open_index, close_index, &mut scopes);
    let mut use_tree_ranges = Vec::new();
    for scope in scopes {
        let (imports, scope_use_tree_ranges) =
            check_block_imports(&scope, tokens, module, diagnostics, adapter);
        use_tree_ranges.extend(scope_use_tree_ranges);
        check_block_common_prefixes(&imports, module, diagnostics, adapter);
        check_block_cfg_spacing(&imports, module, diagnostics, adapter);
    }
    use_tree_ranges
}

fn collect_nested_blocks(
    tokens: &[Token],
    output: &ParseOutput,
    open_index: usize,
    close_index: usize,
    scopes: &mut Vec<BlockScope>,
) {
    let mut index = open_index + 1;
    while index < close_index {
        if tokens[index].text(output.source()) == "#"
            && tokens.get(index + 1).is_some_and(|token| {
                matches!(token.kind(), TokenKind::OpenDelimiter(Delimiter::Bracket))
            })
        {
            if let Some(attribute_end) = matching_delimiter(tokens, index + 1) {
                index = attribute_end + 1;
                continue;
            }
        }
        if tokens[index].text(output.source()) == "!"
            && tokens
                .get(index + 1)
                .is_some_and(|token| matches!(token.kind(), TokenKind::OpenDelimiter(_)))
        {
            if let Some(macro_end) = matching_delimiter(tokens, index + 1) {
                index = macro_end + 1;
                continue;
            }
        }
        if matches!(
            tokens[index].kind(),
            TokenKind::OpenDelimiter(Delimiter::Brace)
        ) {
            let Some(nested_close) = matching_delimiter(tokens, index) else {
                index += 1;
                continue;
            };
            if nested_close >= close_index {
                index += 1;
                continue;
            }
            scopes.push(BlockScope {
                open_index: index,
                close_index: nested_close,
            });
            collect_nested_blocks(tokens, output, index, nested_close, scopes);
            index = nested_close + 1;
            continue;
        }
        index += 1;
    }
}

fn check_block_imports(
    scope: &BlockScope,
    tokens: &[Token],
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) -> (Vec<BlockImportRecord>, Vec<Range<usize>>) {
    let output = module.parse_output();
    let mut seen_non_import = false;
    let mut statement_start = true;
    let mut inside_use = false;
    let mut pending_attribute_start = None;
    let mut imports = Vec::new();
    let mut use_tree_ranges = Vec::new();
    let mut index = scope.open_index + 1;
    while index < scope.close_index {
        let token = &tokens[index];
        if token.text(output.source()) == "#"
            && tokens.get(index + 1).is_some_and(|next| {
                matches!(next.kind(), TokenKind::OpenDelimiter(Delimiter::Bracket))
            })
        {
            if let Some(close) = matching_delimiter(tokens, index + 1) {
                if statement_start && pending_attribute_start.is_none() {
                    pending_attribute_start = Some(token.start());
                }
                index = close + 1;
                continue;
            }
        }
        if token.text(output.source()) == "!"
            && tokens
                .get(index + 1)
                .is_some_and(|next| matches!(next.kind(), TokenKind::OpenDelimiter(_)))
        {
            if let Some(close) = matching_delimiter(tokens, index + 1) {
                if !inside_use {
                    seen_non_import = true;
                    statement_start = false;
                }
                index = close + 1;
                continue;
            }
        }
        if matches!(token.kind(), TokenKind::OpenDelimiter(Delimiter::Brace)) {
            if let Some(close) = matching_delimiter(tokens, index) {
                if !inside_use {
                    seen_non_import = true;
                    statement_start = true;
                }
                index = close + 1;
                continue;
            }
        }
        if matches!(
            token.kind(),
            TokenKind::OpenDelimiter(Delimiter::Parenthesis | Delimiter::Bracket)
        ) {
            if let Some(close) = matching_delimiter(tokens, index) {
                index = close + 1;
                continue;
            }
        }
        let text = token.text(output.source());
        if text == ";" {
            statement_start = true;
            inside_use = false;
            pending_attribute_start = None;
        } else if text == "use" && statement_start {
            if seen_non_import {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::ImportsAtScopeStart,
                        LocalizedMessage::new(BLOCK_USE_AT_SCOPE_START),
                        Some(token.start()..token.end()),
                    ),
                );
            }
            if let Some(cfg_bounds) = nested_cfg_attribute(tokens, index, scope.close_index, output)
            {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::CfgInsideUseTree,
                        LocalizedMessage::new(CFG_INSIDE_USE_TREE),
                        Some(cfg_bounds),
                    ),
                );
            }
            if let Some(end_index) = use_semicolon(tokens, index, scope.close_index, output) {
                let start = pending_attribute_start.unwrap_or(token.start());
                let end = tokens[end_index].end();
                use_tree_ranges.push(start..end);
                if !seen_non_import {
                    if let Some(import) = parse_block_import(module, start, end) {
                        imports.push(import);
                    }
                }
            }
            statement_start = false;
            inside_use = true;
        } else if statement_start {
            statement_start = false;
            seen_non_import = true;
            pending_attribute_start = None;
        }
        index += 1;
    }
    (imports, use_tree_ranges)
}

fn parse_block_import(module: &ModuleNode, start: usize, end: usize) -> Option<BlockImportRecord> {
    let source = module.parse_output().source();
    let snippet = source.slice(start, end)?.to_owned();
    let parsed = RustParser::parse(SourceFile::new(source.path().to_path_buf(), snippet));
    let item = parsed.items().first()?;
    let ItemKind::Use(use_item) = item.kind() else {
        return None;
    };
    Some(BlockImportRecord {
        start,
        end,
        condition: SyntaxSupport::effective_cfg(item.attributes(), &parsed),
        leaves: SyntaxSupport::use_leaves(use_item.tree()),
    })
}

fn use_semicolon(
    tokens: &[Token],
    use_index: usize,
    scope_end: usize,
    output: &ParseOutput,
) -> Option<usize> {
    let mut index = use_index + 1;
    while index < scope_end {
        if tokens[index].text(output.source()) == ";" {
            return Some(index);
        }
        if matches!(tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
            index = matching_delimiter(tokens, index)? + 1;
        } else {
            index += 1;
        }
    }
    None
}

fn check_block_common_prefixes(
    imports: &[BlockImportRecord],
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    for current_index in 1..imports.len() {
        let current = &imports[current_index];
        if current.leaves.len() != 1 {
            continue;
        }
        let current_leaf = &current.leaves[0];
        if current_leaf.alias.is_some() || current_leaf.glob {
            continue;
        }
        let Some(condition) = current.condition.as_ref() else {
            continue;
        };
        for previous in &imports[..current_index] {
            if previous.leaves.len() != 1 || previous.condition.as_ref() != Some(condition) {
                continue;
            }
            let previous_leaf = &previous.leaves[0];
            if previous_leaf.alias.is_some() || previous_leaf.glob {
                continue;
            }
            let prefix_length = common_prefix_len(&previous_leaf.path, &current_leaf.path);
            if prefix_length == 0
                || prefix_length >= previous_leaf.path.len()
                || prefix_length >= current_leaf.path.len()
            {
                continue;
            }
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
                        .with_text("prefix", previous_leaf.path[..prefix_length].join("::")),
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

fn matching_delimiter(tokens: &[Token], open: usize) -> Option<usize> {
    let delimiter = match tokens.get(open)?.kind() {
        TokenKind::OpenDelimiter(delimiter) => *delimiter,
        _ => return None,
    };
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token.kind() {
            TokenKind::OpenDelimiter(found) if *found == delimiter => depth += 1,
            TokenKind::CloseDelimiter(found) if *found == delimiter => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn nested_cfg_attribute(
    tokens: &[Token],
    use_index: usize,
    scope_end: usize,
    output: &ParseOutput,
) -> Option<Range<usize>> {
    let mut index = use_index + 1;
    let mut brace_depth = 0usize;
    while index < scope_end {
        let token = &tokens[index];
        if token.text(output.source()) == ";" && brace_depth == 0 {
            return None;
        }
        match token.kind() {
            TokenKind::OpenDelimiter(Delimiter::Brace) => brace_depth += 1,
            TokenKind::CloseDelimiter(Delimiter::Brace) => {
                brace_depth = brace_depth.saturating_sub(1);
            }
            TokenKind::OpenDelimiter(Delimiter::Parenthesis | Delimiter::Bracket) => {
                if let Some(close) = matching_delimiter(tokens, index) {
                    if brace_depth == 0 {
                        index = close + 1;
                        continue;
                    }
                }
            }
            _ => {}
        }
        if brace_depth > 0
            && token.text(output.source()) == "#"
            && tokens.get(index + 1).is_some_and(|next| {
                matches!(next.kind(), TokenKind::OpenDelimiter(Delimiter::Bracket))
            })
        {
            let close = matching_delimiter(tokens, index + 1)?;
            if tokens
                .get(index + 2)
                .is_some_and(|attribute_name| attribute_name.text(output.source()) == "cfg")
            {
                return Some(token.start()..tokens[close].end());
            }
        }
        index += 1;
    }
    None
}

fn common_prefix_len(left: &[String], right: &[String]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(left_segment, right_segment)| left_segment == right_segment)
        .count()
}

fn line_of(output: &ParseOutput, offset: usize) -> u32 {
    output
        .source()
        .line_column(offset)
        .map_or(1, |(line, _)| line)
}
