use std::{collections::BTreeSet, ops::Range};

use cop_analysis::{AnalysisSnapshot, ModuleNode, PathResolutionKind};
use cop_config::ResolvedConfig;
use cop_model::{
    Diagnostic, LocalizedMessage, RuleId, BLOCK_USE_AT_SCOPE_START, CFG_INSIDE_USE_TREE,
    CFG_USE_GROUP_SPACING, CFG_USE_GROUP_SPLIT, MERGE_BLOCK_IMPORT_PREFIX, MERGE_IMPORT_PREFIX,
    NO_INLINE_QUALIFIED_PATH, USE_AT_SCOPE_START,
};
use cop_syntax::{
    CoverageStatus, Delimiter, FunctionBody, Item, ItemKind, ParseOutput, RustParser, SourceFile,
    SyntaxNode, Token, TokenKind,
};

use crate::support::{PathReference, RuleDiagnosticAdapter, RuleReport, SyntaxSupport, UseLeaf};

pub(super) struct ImportRuleSet;

struct ImportRecord<'a> {
    item: &'a Item,
    item_index: usize,
    leaf: UseLeaf,
    condition: String,
}

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

impl ImportRuleSet {
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
            Self::inline_qualified_paths(module, snapshot, diagnostics, &adapter);
            Self::imports_at_scope_start(module, diagnostics, &adapter);
            Self::common_prefixes(module, diagnostics, &adapter);
            Self::cfg_group_spacing(module, diagnostics, &adapter);
            Self::cfg_inside_use_tree(module, diagnostics, &adapter);
        }
    }

    fn inline_qualified_paths(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for item in module.items() {
            SyntaxSupport::for_each_syntax_node(item, &mut |node| {
                let reference = match node {
                    SyntaxNode::Path(path) => PathReference {
                        segments: path.segments().to_vec(),
                        start: path.start(),
                        end: path.end(),
                    },
                    SyntaxNode::Macro(invocation) => PathReference {
                        segments: invocation.path().segments().to_vec(),
                        start: invocation.path().start(),
                        end: invocation.path().end(),
                    },
                    _ => return,
                };
                Self::check_external_path(&reference, module, snapshot, diagnostics, adapter);
            });
            if matches!(item.kind(), ItemKind::Impl(_)) {
                for reference in SyntaxSupport::impl_header_references(module, item) {
                    Self::check_external_path(&reference, module, snapshot, diagnostics, adapter);
                }
            }
        }
    }

    fn check_external_path(
        reference: &PathReference,
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        if reference.segments.len() < 2 {
            return;
        }
        let resolution = snapshot.symbols().resolve_path(module, &reference.segments);
        if resolution.kind() != PathResolutionKind::ExternalCrate
            || !resolution.alias_chain().is_empty()
            || resolution.symbol().is_some()
        {
            return;
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
    ) {
        let mut encountered_non_import = false;
        for item in module.items() {
            if matches!(item.kind(), ItemKind::Use(_)) {
                if encountered_non_import {
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
            } else if !matches!(item.kind(), ItemKind::Module(child) if !child.is_inline()) {
                encountered_non_import = true;
            }
        }
        for item in module.items() {
            Self::block_import_scopes(item, module, diagnostics, adapter);
        }
    }

    fn block_import_scopes(
        item: &Item,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        match item.kind() {
            ItemKind::Function(function) => {
                if let Some(body) = function.body() {
                    check_function_blocks(body, module, diagnostics, adapter);
                }
            }
            ItemKind::Trait(trait_item) => {
                for nested in trait_item.items() {
                    Self::block_import_scopes(nested, module, diagnostics, adapter);
                }
            }
            ItemKind::Impl(impl_item) => {
                for nested in impl_item.items() {
                    Self::block_import_scopes(nested, module, diagnostics, adapter);
                }
            }
            _ => {}
        }
    }

    fn common_prefixes(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        let output = module.parse_output();
        let imports = module
            .items()
            .iter()
            .enumerate()
            .filter_map(|(item_index, item)| {
                let ItemKind::Use(use_item) = item.kind() else {
                    return None;
                };
                let mut leaves = SyntaxSupport::use_leaves(use_item.tree());
                let condition = SyntaxSupport::effective_cfg(item.attributes(), output)?;
                if leaves.len() != 1 {
                    return None;
                }
                Some(ImportRecord {
                    item,
                    item_index,
                    leaf: leaves.remove(0),
                    condition,
                })
            })
            .collect::<Vec<_>>();
        for current_index in 1..imports.len() {
            let current = &imports[current_index];
            if current.leaf.alias.is_some() || current.leaf.glob {
                continue;
            }
            for previous in &imports[..current_index] {
                if previous.leaf.alias.is_some()
                    || previous.leaf.glob
                    || previous.item_index + 1 != current.item_index
                    || previous.condition != current.condition
                    || previous.item.visibility().kind() != current.item.visibility().kind()
                {
                    continue;
                }
                let prefix_length = common_prefix_len(&previous.leaf.path, &current.leaf.path);
                if prefix_length == 0
                    || prefix_length >= previous.leaf.path.len()
                    || prefix_length >= current.leaf.path.len()
                {
                    continue;
                }
                let prefix = previous.leaf.path[..prefix_length].join("::");
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
        let mut previous: Option<(&Item, String)> = None;
        let mut completed_groups = BTreeSet::new();
        for item in module.items() {
            if !matches!(item.kind(), ItemKind::Use(_)) {
                if let Some((_, previous_condition)) = previous.take() {
                    completed_groups.insert(previous_condition);
                }
                continue;
            }
            let Some(condition) = SyntaxSupport::effective_cfg(item.attributes(), output) else {
                previous = None;
                completed_groups.clear();
                continue;
            };
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
        let output = module.parse_output();
        for item in module.items() {
            let ItemKind::Use(use_item) = item.kind() else {
                continue;
            };
            let tree = use_item.tree();
            let tokens =
                SyntaxSupport::token_range(output, tree.start(), tree.end()).collect::<Vec<_>>();
            let mut brace_depth = 0usize;
            for (index, token) in tokens.iter().enumerate() {
                match token.kind() {
                    TokenKind::OpenDelimiter(Delimiter::Brace) => brace_depth += 1,
                    TokenKind::CloseDelimiter(Delimiter::Brace) => {
                        brace_depth = brace_depth.saturating_sub(1);
                    }
                    _ => {}
                }
                if brace_depth == 0 || token.text(output.source()) != "#" {
                    continue;
                }
                if !tokens.get(index + 1).is_some_and(|next| {
                    matches!(next.kind(), TokenKind::OpenDelimiter(Delimiter::Bracket))
                }) {
                    continue;
                }
                let Some(close) = matching_token(tokens.as_slice(), index + 1) else {
                    continue;
                };
                let has_cfg = tokens
                    .get(index + 2)
                    .is_some_and(|nested| nested.text(output.source()) == "cfg");
                if !has_cfg {
                    continue;
                }
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::CfgInsideUseTree,
                        LocalizedMessage::new(CFG_INSIDE_USE_TREE),
                        Some(token.start()..tokens[close].end()),
                    ),
                );
            }
        }
    }
}

fn common_prefix_len(left: &[String], right: &[String]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(left_segment, right_segment)| left_segment == right_segment)
        .count()
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

fn matching_token(tokens: &[&Token], open: usize) -> Option<usize> {
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

fn check_function_blocks(
    body: &FunctionBody,
    module: &ModuleNode,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    let output = module.parse_output();
    let tokens = output.tokens();
    let Some(open_index) = tokens.iter().position(|token| {
        token.start() == body.start()
            && matches!(token.kind(), TokenKind::OpenDelimiter(Delimiter::Brace))
    }) else {
        return;
    };
    let Some(close_index) = matching_delimiter(tokens, open_index) else {
        return;
    };
    let mut scopes = vec![BlockScope {
        open_index,
        close_index,
    }];
    collect_nested_blocks(tokens, output, open_index, close_index, &mut scopes);
    for scope in scopes {
        let imports = check_block_imports(&scope, tokens, module, diagnostics, adapter);
        check_block_common_prefixes(&imports, module, diagnostics, adapter);
        check_block_cfg_spacing(&imports, module, diagnostics, adapter);
    }
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
) -> Vec<BlockImportRecord> {
    let output = module.parse_output();
    let mut seen_non_import = false;
    let mut statement_start = true;
    let mut inside_use = false;
    let mut pending_attribute_start = None;
    let mut imports = Vec::new();
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
    imports
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
