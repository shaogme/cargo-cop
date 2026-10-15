mod block_imports;

use std::collections::BTreeSet;

use cop_analysis::{AnalysisSnapshot, ModuleNode, PathResolutionKind};
use cop_config::ResolvedConfig;
use cop_model::{
    Diagnostic, LocalizedMessage, RuleId, CFG_INSIDE_USE_TREE, CFG_USE_GROUP_SPACING,
    CFG_USE_GROUP_SPLIT, EXTERN_CRATE_AT_SCOPE_START, MERGE_IMPORT_PREFIX,
    NO_INLINE_QUALIFIED_PATH, USE_AT_SCOPE_START,
};
use cop_syntax::{
    CoverageStatus, Delimiter, Item, ItemKind, ParseOutput, SyntaxNode, Token, TokenKind,
};

use crate::support::{PathReference, RuleDiagnosticAdapter, RuleReport, SyntaxSupport, UseLeaf};

pub(super) struct ImportRuleSet;

struct ImportRecord<'a> {
    item: &'a Item,
    item_index: usize,
    leaf: UseLeaf,
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
            if module.parse_output().coverage() != CoverageStatus::Complete {
                continue;
            }
            Self::extern_crate_at_scope_start(module, diagnostics, &adapter);
            Self::imports_at_scope_start(module, diagnostics, &adapter);
            if module.is_order_only() {
                continue;
            }
            Self::inline_qualified_paths(module, snapshot, diagnostics, &adapter);
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
        let mut encountered_other_item = false;
        for item in module.source_items() {
            match item.kind() {
                ItemKind::ExternCrate(_) => {}
                ItemKind::Module(child) if !child.is_inline() => {}
                ItemKind::Use(_) => {
                    if encountered_other_item {
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
                _ => {
                    encountered_other_item = true;
                }
            }
        }
        for item in module.items() {
            block_imports::BlockImportRuleSet::check_item(item, module, diagnostics, adapter);
        }
    }

    fn extern_crate_at_scope_start(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        let mut encountered_non_extern_crate = false;
        for item in module.source_items() {
            if matches!(item.kind(), ItemKind::ExternCrate(_)) {
                if encountered_non_extern_crate {
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
                encountered_non_extern_crate = true;
            }
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
