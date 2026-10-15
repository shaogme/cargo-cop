use std::ops::Range;

use cop_analysis::{
    AnalysisSnapshot, ModuleNode, PathResolutionKind, SymbolKind, SymbolVisibility,
};
use cop_config::ResolvedConfig;
use cop_model::{
    AnalysisIssueKind, Diagnostic, LocalizedMessage, RuleId, Severity, CHILD_PUBLIC_FREE_FUNCTION,
    IMPL_CROSS_FILE, IMPL_EXTERNAL_TRAIT_TARGET, IMPL_INHERENT_TARGET, IMPL_TARGET_UNRESOLVED,
    NO_PARENT_GLOB_IMPORT, NO_PARENT_PRIVATE_PATH, PRIVATE_FIELDS_FOR_BEHAVIOR_TYPE,
};
use cop_syntax::{
    CoverageStatus, Delimiter, Item, ItemKind, ParseOutput, SyntaxNode, Token, TokenKind,
};

use crate::support::{PathReference, RuleDiagnosticAdapter, RuleReport, SyntaxSupport};

pub(super) struct SemanticRuleSet;

struct ImplTarget {
    path: Vec<String>,
    trait_impl: bool,
    bounds: Range<usize>,
}

impl SemanticRuleSet {
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
            Self::parent_private_paths(module, snapshot, diagnostics, &adapter);
            Self::impl_locality(module, snapshot, diagnostics, &adapter);
            Self::private_behavior_fields(module, snapshot, diagnostics, &adapter);
            Self::public_free_functions(module, diagnostics, &adapter);
        }
    }

    fn parent_private_paths(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        if module.is_crate_root() {
            return;
        }
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
                check_parent_path(&reference, module, snapshot, diagnostics, adapter);
            });
            if matches!(item.kind(), ItemKind::Impl(_)) {
                for reference in SyntaxSupport::impl_header_references(module, item) {
                    check_parent_path(&reference, module, snapshot, diagnostics, adapter);
                }
            }
            if let ItemKind::Use(use_item) = item.kind() {
                for leaf in SyntaxSupport::use_leaves(use_item.tree()) {
                    if leaf.glob && leaf.path.first().is_some_and(|name| name == "super") {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::NoParentPrivateDependency,
                                LocalizedMessage::new(NO_PARENT_GLOB_IMPORT),
                                Some(leaf.start..leaf.end),
                            ),
                        );
                        continue;
                    }
                    check_parent_path(
                        &PathReference {
                            segments: leaf.path,
                            start: leaf.start,
                            end: leaf.end,
                        },
                        module,
                        snapshot,
                        diagnostics,
                        adapter,
                    );
                }
            }
        }
    }

    fn impl_locality(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for item in module.items() {
            if !matches!(item.kind(), ItemKind::Impl(_)) {
                continue;
            }
            let Some(target) = parse_impl_target(module, item) else {
                continue;
            };
            let resolution = snapshot.symbols().resolve_path(module, &target.path);
            if let Some(symbol) = resolution.symbol() {
                if is_repository_type(symbol.kind()) {
                    if symbol.file_path() != module.file_path() {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::ImplLocality,
                                LocalizedMessage::new(IMPL_CROSS_FILE)
                                    .with_text("target", symbol.name()),
                                Some(target.bounds),
                            ),
                        );
                    }
                    continue;
                }
            }
            if resolution.kind() == PathResolutionKind::ExternalCrate {
                let root = resolution
                    .external_crate()
                    .or_else(|| target.path.first().map(String::as_str))
                    .unwrap_or("");
                if target.trait_impl && is_allowed_external_root(root) {
                    continue;
                }
                if target.trait_impl {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::ImplLocality,
                            LocalizedMessage::new(IMPL_EXTERNAL_TRAIT_TARGET)
                                .with_text("root", root),
                            Some(target.bounds),
                        ),
                    );
                } else {
                    adapter.push(
                        module,
                        diagnostics,
                        RuleReport::new(
                            RuleId::ImplLocality,
                            LocalizedMessage::new(IMPL_INHERENT_TARGET),
                            Some(target.bounds),
                        ),
                    );
                }
                continue;
            }
            if target.trait_impl && is_builtin_type(target.path.last().map(String::as_str)) {
                continue;
            }
            if !target.trait_impl && is_builtin_type(target.path.last().map(String::as_str)) {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::ImplLocality,
                        LocalizedMessage::new(IMPL_INHERENT_TARGET),
                        Some(target.bounds),
                    ),
                );
                continue;
            }
            if matches!(
                resolution.kind(),
                PathResolutionKind::Unresolved
                    | PathResolutionKind::Ambiguous
                    | PathResolutionKind::AliasCycle
            ) {
                let mut issue = Diagnostic::analysis(
                    AnalysisIssueKind::ParserCoverage,
                    Severity::Warning,
                    LocalizedMessage::new(IMPL_TARGET_UNRESOLVED)
                        .with_text("target", target.path.join("::")),
                )
                .with_package(module.package_name())
                .with_package_manifest_path(module.package_manifest_path())
                .with_target(module.target_name())
                .with_target_kind(module.target_kind().to_vec())
                .with_namespace(format!("crate::{}", module.namespace().join("::")));
                if let Ok(span) = module
                    .parse_output()
                    .source()
                    .span(target.bounds.start, target.bounds.end)
                {
                    issue = issue.with_span(span);
                }
                diagnostics.push(issue);
            }
        }
    }

    fn private_behavior_fields(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for item in module.items() {
            let (name, kind) = match item.kind() {
                ItemKind::Struct(declaration) => (declaration.name(), SymbolKind::Struct),
                ItemKind::Enum(declaration) => (declaration.name(), SymbolKind::Enum),
                ItemKind::Union(declaration) => (declaration.name(), SymbolKind::Union),
                _ => continue,
            };
            let Some(name) = name else {
                continue;
            };
            let has_impl = snapshot.modules().iter().any(|candidate_module| {
                candidate_module.items().iter().any(|candidate| {
                    let ItemKind::Impl(_) = candidate.kind() else {
                        return false;
                    };
                    let Some(target) = parse_impl_target(candidate_module, candidate) else {
                        return false;
                    };
                    snapshot
                        .symbols()
                        .resolve_path(candidate_module, &target.path)
                        .symbol()
                        .is_some_and(|symbol| {
                            symbol.kind() == kind
                                && symbol.name() == name
                                && symbol.file_path() == module.file_path()
                                && symbol.namespace() == module.namespace()
                        })
                })
            });
            if !has_impl {
                continue;
            }
            for field in public_fields(module, item) {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::PrivateFieldsForBehaviorTypes,
                        LocalizedMessage::new(PRIVATE_FIELDS_FOR_BEHAVIOR_TYPE)
                            .with_text("name", name),
                        Some(field.start()..field.end()),
                    ),
                );
            }
        }
    }

    fn public_free_functions(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        if module.is_crate_root() {
            return;
        }
        for item in module.items() {
            if matches!(item.kind(), ItemKind::Function(_)) && SyntaxSupport::is_non_private(item) {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::ChildPublicFreeFunctions,
                        LocalizedMessage::new(CHILD_PUBLIC_FREE_FUNCTION),
                        Some(item.start()..item.end()),
                    ),
                );
            }
        }
    }
}

fn check_parent_path(
    reference: &PathReference,
    module: &ModuleNode,
    snapshot: &AnalysisSnapshot,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    if !matches!(
        reference.segments.first().map(String::as_str),
        Some("super" | "crate")
    ) {
        return;
    }
    let resolution = snapshot.symbols().resolve_path(module, &reference.segments);
    let Some(symbol) = resolution.symbol() else {
        return;
    };
    if !matches!(symbol.visibility(), SymbolVisibility::Private)
        || symbol.namespace().len() >= module.namespace().len()
        || !module.namespace().starts_with(symbol.namespace())
    {
        return;
    }
    adapter.push(
        module,
        diagnostics,
        RuleReport::new(
            RuleId::NoParentPrivateDependency,
            LocalizedMessage::new(NO_PARENT_PRIVATE_PATH)
                .with_text("ancestor", symbol.namespace().join("::"))
                .with_text("item", symbol.name()),
            Some(reference.start..reference.end),
        ),
    );
}

fn parse_impl_target(module: &ModuleNode, item: &Item) -> Option<ImplTarget> {
    let output = module.parse_output();
    let tokens = output
        .tokens()
        .iter()
        .filter(|token| token.start() >= item.start() && token.end() <= item.end())
        .collect::<Vec<_>>();
    let impl_index = tokens
        .iter()
        .position(|token| token.text(output.source()) == "impl")?;
    let body_index = tokens
        .iter()
        .position(|token| matches!(token.kind(), TokenKind::OpenDelimiter(Delimiter::Brace)))?;
    let mut generic_depth = 0usize;
    let mut trait_for = None;
    for (index, token) in tokens
        .iter()
        .enumerate()
        .take(body_index)
        .skip(impl_index + 1)
    {
        let text = token.text(output.source());
        match text {
            "<" => generic_depth += 1,
            ">" => generic_depth = generic_depth.saturating_sub(1),
            ">>" => generic_depth = generic_depth.saturating_sub(2),
            "for"
                if generic_depth == 0
                    && tokens
                        .get(index + 1)
                        .is_some_and(|token| token.text(output.source()) != "<") =>
            {
                trait_for = Some(index);
                break;
            }
            _ => {}
        }
    }
    let trait_impl = trait_for.is_some();
    let mut target_start = trait_for.map_or(impl_index + 1, |index| index + 1);
    if !trait_impl
        && tokens
            .get(target_start)
            .is_some_and(|token| token.text(output.source()) == "<")
    {
        let mut depth = 0usize;
        for (index, token) in tokens
            .iter()
            .enumerate()
            .take(body_index)
            .skip(target_start)
        {
            match token.text(output.source()) {
                "<" => depth += 1,
                ">" => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        target_start = index + 1;
                        break;
                    }
                }
                ">>" => {
                    depth = depth.saturating_sub(2);
                    if depth == 0 {
                        target_start = index + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    let mut path = Vec::new();
    let mut expect_segment = true;
    for token in tokens
        .iter()
        .skip(target_start)
        .take(body_index - target_start)
    {
        let text = token.text(output.source());
        if matches!(text, "<" | "(" | "&" | "*" | "mut" | "const" | "dyn") {
            if path.is_empty() {
                continue;
            }
            if text == "<" || text == "(" {
                break;
            }
        }
        match token.kind() {
            TokenKind::Identifier(identifier)
                if expect_segment
                    && !matches!(identifier.as_str(), "where" | "unsafe" | "default") =>
            {
                path.push(identifier.clone());
                expect_segment = false;
            }
            TokenKind::Punctuation(punctuation) if punctuation == "::" => {
                expect_segment = true;
            }
            _ if path.is_empty() => {}
            _ => break,
        }
    }
    if path.is_empty() {
        return None;
    }
    let start = tokens[target_start].start();
    let end = tokens[body_index].start();
    Some(ImplTarget {
        path,
        trait_impl,
        bounds: start..end,
    })
}

fn is_repository_type(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Union | SymbolKind::TypeAlias
    )
}

fn is_allowed_external_root(root: &str) -> bool {
    matches!(root, "std" | "core" | "alloc")
}

fn is_builtin_type(name: Option<&str>) -> bool {
    name.is_some_and(|name| {
        matches!(
            name,
            "bool"
                | "char"
                | "str"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "f32"
                | "f64"
                | "String"
                | "Vec"
                | "Option"
                | "Result"
                | "Box"
                | "Rc"
                | "Arc"
                | "Cell"
                | "RefCell"
                | "HashMap"
                | "HashSet"
                | "BTreeMap"
                | "BTreeSet"
                | "Path"
                | "PathBuf"
                | "Duration"
                | "SystemTime"
                | "Instant"
                | "Cow"
                | "Pin"
                | "Mutex"
                | "RwLock"
                | "Weak"
                | "PhantomData"
        )
    })
}

fn public_fields<'a>(module: &'a ModuleNode, item: &Item) -> Vec<&'a Token> {
    let output = module.parse_output();
    let tokens = output
        .tokens()
        .iter()
        .filter(|token| token.start() >= item.start() && token.end() <= item.end())
        .collect::<Vec<_>>();
    let Some(open_index) = field_group_open(&tokens, output) else {
        return Vec::new();
    };
    let Some(close_index) = matching_group(&tokens, open_index) else {
        return Vec::new();
    };
    split_field_ranges(&tokens, open_index + 1, close_index)
        .into_iter()
        .filter_map(|range| {
            let mut index = range.start;
            while index + 1 < range.end
                && tokens[index].text(output.source()) == "#"
                && matches!(
                    tokens[index + 1].kind(),
                    TokenKind::OpenDelimiter(Delimiter::Bracket)
                )
            {
                index = matching_group(&tokens, index + 1)? + 1;
            }
            tokens
                .get(index)
                .copied()
                .filter(|token| token.text(output.source()) == "pub")
        })
        .collect()
}

fn field_group_open(tokens: &[&Token], output: &ParseOutput) -> Option<usize> {
    let declaration_keyword = tokens
        .iter()
        .position(|token| matches!(token.text(output.source()), "struct" | "enum" | "union"))?;
    let name_index = (declaration_keyword + 1..tokens.len())
        .find(|index| matches!(tokens[*index].kind(), TokenKind::Identifier(_)))?;
    let mut angle_depth = 0usize;
    let mut tuple_group = None;
    for (index, token) in tokens.iter().enumerate().skip(name_index + 1) {
        match token.text(output.source()) {
            "<" => angle_depth += 1,
            ">" => angle_depth = angle_depth.saturating_sub(1),
            ">>" => angle_depth = angle_depth.saturating_sub(2),
            "{" if angle_depth == 0 => return Some(index),
            "(" if angle_depth == 0 && tuple_group.is_none() => tuple_group = Some(index),
            _ => {}
        }
    }
    tuple_group
}

fn matching_group(tokens: &[&Token], open: usize) -> Option<usize> {
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

fn split_field_ranges(tokens: &[&Token], start: usize, end: usize) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut field_start = start;
    let mut delimiter_depth = 0usize;
    let mut angle_depth = 0usize;
    for (index, token) in tokens.iter().enumerate().take(end).skip(start) {
        match token.kind() {
            TokenKind::OpenDelimiter(_) => delimiter_depth += 1,
            TokenKind::CloseDelimiter(_) => delimiter_depth = delimiter_depth.saturating_sub(1),
            TokenKind::Punctuation(value) if value == "<" => angle_depth += 1,
            TokenKind::Punctuation(value) if value == ">" => {
                angle_depth = angle_depth.saturating_sub(1);
            }
            TokenKind::Punctuation(value) if value == ">>" => {
                angle_depth = angle_depth.saturating_sub(2);
            }
            TokenKind::Punctuation(value)
                if value == "," && delimiter_depth == 0 && angle_depth == 0 =>
            {
                if field_start < index {
                    ranges.push(field_start..index);
                }
                field_start = index + 1;
            }
            _ => {}
        }
    }
    if field_start < end {
        ranges.push(field_start..end);
    }
    ranges
}
