use cop_analysis::{AnalysisSnapshot, ModuleNode, SymbolKind};
use cop_config::ResolvedConfig;
use cop_model::{
    Diagnostic, LocalizedMessage, RuleId, CLOSURE_PARAMETER_LIMIT, DECLARATION_BEFORE_USE,
    FUNCTION_PARAMETER_LIMIT, NO_LINT_RELAXATION, TUPLE_ARITY,
};
use cop_syntax::{
    ClosureNode, CoverageStatus, Delimiter, FunctionItem, Item, ItemKind, ParseOutput, SyntaxNode,
    Token, TokenKind,
};

use crate::support::{PathReference, RuleDiagnosticAdapter, RuleReport, SyntaxSupport};

pub(super) struct SyntaxRuleSet;

impl SyntaxRuleSet {
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
            Self::lint_relaxation(module, diagnostics, &adapter);
            Self::syntax_shapes(module, diagnostics, &adapter);
            Self::declaration_order(module, snapshot, diagnostics, &adapter);
        }
    }

    fn lint_relaxation(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        let output = module.parse_output();
        if module.is_crate_root() || !module.is_inline() {
            if let Some(attribute) = output
                .inner_attributes()
                .iter()
                .find(|attribute| SyntaxSupport::has_lint_relaxation(attribute, output))
            {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::NoLintRelaxation,
                        LocalizedMessage::new(NO_LINT_RELAXATION),
                        Some(attribute.start()..attribute.end()),
                    ),
                );
            }
        }
        for item in module.items() {
            Self::inspect_lint_items(item, output, module, diagnostics, adapter);
        }
    }

    fn inspect_lint_items(
        item: &Item,
        output: &ParseOutput,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for attribute in item.attributes() {
            if SyntaxSupport::has_lint_relaxation(attribute, output) {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::NoLintRelaxation,
                        LocalizedMessage::new(NO_LINT_RELAXATION),
                        Some(attribute.start()..attribute.end()),
                    ),
                );
            }
        }
        match item.kind() {
            ItemKind::Module(child) => {
                for attribute in child.inner_attributes() {
                    if SyntaxSupport::has_lint_relaxation(attribute, output) {
                        adapter.push(
                            module,
                            diagnostics,
                            RuleReport::new(
                                RuleId::NoLintRelaxation,
                                LocalizedMessage::new(NO_LINT_RELAXATION),
                                Some(attribute.start()..attribute.end()),
                            ),
                        );
                    }
                }
            }
            ItemKind::Trait(trait_item) => {
                for nested in trait_item.items() {
                    Self::inspect_lint_items(nested, output, module, diagnostics, adapter);
                }
            }
            ItemKind::Impl(impl_item) => {
                for nested in impl_item.items() {
                    Self::inspect_lint_items(nested, output, module, diagnostics, adapter);
                }
            }
            ItemKind::Function(function) => {
                if let Some(body) = function.body() {
                    for attribute in body.leading_attributes() {
                        if SyntaxSupport::has_lint_relaxation(attribute, output) {
                            adapter.push(
                                module,
                                diagnostics,
                                RuleReport::new(
                                    RuleId::NoLintRelaxation,
                                    LocalizedMessage::new(NO_LINT_RELAXATION),
                                    Some(attribute.start()..attribute.end()),
                                ),
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn syntax_shapes(
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for item in module.items() {
            Self::check_shapes(item, module, diagnostics, adapter);
        }
    }

    fn check_shapes(
        item: &Item,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        if let ItemKind::Function(function) = item.kind() {
            if function.parameters().len() > 5 {
                adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::ParameterLimit,
                        LocalizedMessage::new(FUNCTION_PARAMETER_LIMIT)
                            .with_number("actual", function.parameters().len() as i64)
                            .with_number("maximum", 5),
                        Some(item.start()..item.end()),
                    ),
                );
            }
        }
        if matches!(
            item.kind(),
            ItemKind::Function(_)
                | ItemKind::Struct(_)
                | ItemKind::Enum(_)
                | ItemKind::Union(_)
                | ItemKind::TypeAlias(_)
                | ItemKind::Constant(_)
                | ItemKind::Static(_)
        ) {
            SyntaxSupport::for_each_syntax_node(item, &mut |node| match node {
                SyntaxNode::Closure(closure) if closure.parameters().len() > 5 => adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::ParameterLimit,
                        LocalizedMessage::new(CLOSURE_PARAMETER_LIMIT)
                            .with_number("actual", closure.parameters().len() as i64)
                            .with_number("maximum", 5),
                        Some(closure.start()..closure.end()),
                    ),
                ),
                SyntaxNode::Tuple(tuple) if tuple.arity() > 2 => adapter.push(
                    module,
                    diagnostics,
                    RuleReport::new(
                        RuleId::TupleArity,
                        LocalizedMessage::new(TUPLE_ARITY)
                            .with_number("actual", tuple.arity() as i64)
                            .with_number("maximum", 2),
                        Some(tuple.start()..tuple.end()),
                    ),
                ),
                _ => {}
            });
        }
        match item.kind() {
            ItemKind::Module(_) => {}
            ItemKind::Trait(trait_item) => {
                for nested in trait_item.items() {
                    Self::check_shapes(nested, module, diagnostics, adapter);
                }
            }
            ItemKind::Impl(impl_item) => {
                for nested in impl_item.items() {
                    Self::check_shapes(nested, module, diagnostics, adapter);
                }
            }
            _ => {}
        }
    }

    fn declaration_order(
        module: &ModuleNode,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) {
        for item in module.items() {
            SyntaxSupport::for_each_syntax_node(item, &mut |node| {
                let SyntaxNode::Path(path) = node else {
                    return;
                };
                check_ordered_path(
                    &PathReference {
                        segments: path.segments().to_vec(),
                        start: path.start(),
                        end: path.end(),
                    },
                    module,
                    snapshot,
                    diagnostics,
                    adapter,
                );
            });
            if matches!(item.kind(), ItemKind::Impl(_)) {
                for reference in SyntaxSupport::impl_header_references(module, item) {
                    check_ordered_path(&reference, module, snapshot, diagnostics, adapter);
                }
            }
            if let ItemKind::Use(use_item) = item.kind() {
                for leaf in SyntaxSupport::use_leaves(use_item.tree()) {
                    if !leaf.glob {
                        check_ordered_path(
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
    }
}

fn check_ordered_path(
    reference: &PathReference,
    module: &ModuleNode,
    snapshot: &AnalysisSnapshot,
    diagnostics: &mut Vec<Diagnostic>,
    adapter: &RuleDiagnosticAdapter<'_>,
) {
    if is_binding_position(module, reference.start)
        || is_parameter_binding_position(module, reference)
        || is_shadowed_by_local_binding(module, reference)
    {
        return;
    }
    let candidate = if reference.segments.len() == 1 {
        snapshot
            .symbols()
            .symbols()
            .iter()
            .find(|symbol| {
                symbol.target_name() == module.target_name()
                    && symbol.namespace() == module.namespace()
                    && symbol.file_path() == module.file_path()
                    && identifier_name(symbol.name()) == identifier_name(&reference.segments[0])
                    && is_ordered_declaration(symbol.kind())
            })
            .cloned()
    } else {
        let resolution = snapshot.symbols().resolve_path(module, &reference.segments);
        resolution
            .symbol()
            .filter(|symbol| {
                symbol.namespace() == module.namespace()
                    && symbol.file_path() == module.file_path()
                    && is_ordered_declaration(symbol.kind())
            })
            .cloned()
    };
    let Some(symbol) = candidate else {
        return;
    };
    let Some(declaration_span) = symbol.span() else {
        return;
    };
    if reference.start >= declaration_span.byte_range().start {
        return;
    }
    adapter.push(
        module,
        diagnostics,
        RuleReport::new(
            RuleId::DeclarationBeforeUse,
            LocalizedMessage::new(DECLARATION_BEFORE_USE).with_text("name", symbol.name()),
            Some(reference.start..reference.end),
        ),
    );
}

fn is_ordered_declaration(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Struct
            | SymbolKind::Enum
            | SymbolKind::Union
            | SymbolKind::TypeAlias
            | SymbolKind::Trait
            | SymbolKind::Constant
            | SymbolKind::Static
    )
}

fn is_binding_position(module: &ModuleNode, start: usize) -> bool {
    let previous = module
        .parse_output()
        .tokens()
        .iter()
        .rev()
        .find(|token| token.end() <= start);
    previous.is_some_and(|token| {
        matches!(
            token.kind(),
            TokenKind::Identifier(identifier)
                if matches!(identifier.as_str(), "let" | "fn" | "struct" | "enum" | "type" | "const" | "static" | "trait")
        )
    })
}

struct LocalBinding {
    name: String,
    scope: Vec<usize>,
    token_start: usize,
    visible_from: usize,
}

fn is_parameter_binding_position(module: &ModuleNode, reference: &PathReference) -> bool {
    let Some(name) = reference.segments.first() else {
        return false;
    };
    module
        .items()
        .iter()
        .any(|item| parameter_binding_in_item(item, module.parse_output(), reference.start, name))
}

fn parameter_binding_in_item(item: &Item, output: &ParseOutput, start: usize, name: &str) -> bool {
    match item.kind() {
        ItemKind::Function(function) => function.parameters().iter().any(|parameter| {
            parameter.start() <= start
                && start < parameter.end()
                && pattern_binding(output, parameter.start(), parameter.end(), name) == Some(start)
        }),
        ItemKind::Trait(trait_item) => trait_item
            .items()
            .iter()
            .any(|nested| parameter_binding_in_item(nested, output, start, name)),
        ItemKind::Impl(impl_item) => impl_item
            .items()
            .iter()
            .any(|nested| parameter_binding_in_item(nested, output, start, name)),
        _ => false,
    }
}

fn is_shadowed_by_local_binding(module: &ModuleNode, reference: &PathReference) -> bool {
    let Some(name) = reference.segments.first() else {
        return false;
    };
    let Some(function) = function_containing(module.items(), reference.start) else {
        return false;
    };
    if function.parameters().iter().any(|parameter| {
        pattern_binding(
            module.parse_output(),
            parameter.start(),
            parameter.end(),
            name,
        )
        .is_some()
    }) {
        return true;
    }
    let Some(body) = function.body() else {
        return false;
    };
    let output = module.parse_output();
    if closure_binding_shadows(body.nodes(), reference.start, name, output) {
        return true;
    }
    let tokens = output.tokens();
    let mut scope = Vec::new();
    let mut bindings = Vec::new();
    let Some(mut index) = tokens
        .iter()
        .position(|token| token.start() == body.start())
    else {
        return false;
    };
    while index < tokens.len() && tokens[index].start() <= reference.start {
        if token_text(&tokens[index], output) == "!"
            && index > 0
            && is_macro_path_token(&tokens[index - 1], output)
        {
            if let Some(close) = macro_group_end(tokens, index + 1) {
                index = close + 1;
                continue;
            }
        }
        if token_text(&tokens[index], output) == "let" {
            if let Some(token_start) =
                pattern_binding(output, tokens[index].end(), body.end(), name)
            {
                bindings.push(LocalBinding {
                    name: name.to_owned(),
                    scope: scope.clone(),
                    token_start,
                    visible_from: let_statement_end(tokens, index + 1, body.end(), output),
                });
            }
        }
        match tokens[index].kind() {
            TokenKind::OpenDelimiter(Delimiter::Brace) => scope.push(index),
            TokenKind::CloseDelimiter(Delimiter::Brace) => {
                scope.pop();
            }
            _ => {}
        }
        index += 1;
    }
    bindings.into_iter().any(|binding| {
        (binding.token_start == reference.start || binding.visible_from <= reference.start)
            && identifier_name(&binding.name) == identifier_name(name)
            && scope.starts_with(&binding.scope)
    })
}

fn closure_binding_shadows(
    nodes: &[SyntaxNode],
    offset: usize,
    name: &str,
    output: &ParseOutput,
) -> bool {
    nodes.iter().any(|node| {
        let SyntaxNode::Closure(closure) = node else {
            return false;
        };
        closure_parameter_shadows(closure, offset, name, output)
    })
}

fn closure_parameter_shadows(
    closure: &ClosureNode,
    offset: usize,
    name: &str,
    output: &ParseOutput,
) -> bool {
    if offset < closure.start() || offset >= closure.end() {
        return false;
    }
    let has_binding = closure.parameters().iter().any(|parameter| {
        pattern_binding(output, parameter.start(), parameter.end(), name).is_some()
    });
    if !has_binding {
        return false;
    }
    if closure.parameters().iter().any(|parameter| {
        pattern_binding(output, parameter.start(), parameter.end(), name) == Some(offset)
    }) {
        return true;
    }
    let parameters_end = closure
        .parameters()
        .last()
        .map_or(closure.start(), |parameter| parameter.end());
    offset > parameters_end
}

fn let_statement_end(
    tokens: &[Token],
    start: usize,
    fallback: usize,
    output: &ParseOutput,
) -> usize {
    let mut delimiters = Vec::new();
    for token in tokens.iter().skip(start) {
        if token.start() >= fallback {
            break;
        }
        match token.kind() {
            TokenKind::OpenDelimiter(delimiter) => delimiters.push(*delimiter),
            TokenKind::CloseDelimiter(delimiter) if delimiters.last() == Some(delimiter) => {
                delimiters.pop();
            }
            _ if token_text(token, output) == ";" && delimiters.is_empty() => {
                return token.end();
            }
            _ => {}
        }
    }
    fallback
}

fn function_containing(items: &[Item], offset: usize) -> Option<&FunctionItem> {
    for item in items {
        match item.kind() {
            ItemKind::Function(function)
                if function
                    .body()
                    .is_some_and(|body| body.start() <= offset && offset < body.end()) =>
            {
                return Some(function);
            }
            ItemKind::Trait(trait_item) => {
                if let Some(function) = function_containing(trait_item.items(), offset) {
                    return Some(function);
                }
            }
            ItemKind::Impl(impl_item) => {
                if let Some(function) = function_containing(impl_item.items(), offset) {
                    return Some(function);
                }
            }
            _ => {}
        }
    }
    None
}

fn pattern_binding(output: &ParseOutput, start: usize, end: usize, name: &str) -> Option<usize> {
    let tokens = output
        .tokens()
        .iter()
        .filter(|token| token.start() >= start && token.end() <= end)
        .collect::<Vec<_>>();
    let mut delimiters = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let text = token_text(token, output);
        if delimiters.is_empty() && matches!(text, ":" | "=") {
            break;
        }
        if let TokenKind::Identifier(binding) = token.kind() {
            let previous_is_path = index > 0 && token_text(tokens[index - 1], output) == "::";
            let next_text = tokens.get(index + 1).map(|next| token_text(next, output));
            let is_field_name =
                next_text == Some(":") && delimiters.last() == Some(&Delimiter::Brace);
            let is_path_or_constructor = matches!(next_text, Some("::" | "(" | "{" | "!"));
            if binding != "self"
                && identifier_name(binding) == identifier_name(name)
                && !previous_is_path
                && !is_field_name
                && !is_path_or_constructor
            {
                return Some(token.start());
            }
        }
        match token.kind() {
            TokenKind::OpenDelimiter(delimiter) => delimiters.push(*delimiter),
            TokenKind::CloseDelimiter(delimiter) if delimiters.last() == Some(delimiter) => {
                delimiters.pop();
            }
            _ => {}
        }
    }
    None
}

fn is_macro_path_token(token: &Token, output: &ParseOutput) -> bool {
    matches!(token.kind(), TokenKind::Identifier(_)) || token_text(token, output) == "::"
}

fn macro_group_end(tokens: &[Token], open: usize) -> Option<usize> {
    let mut stack = Vec::new();
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token.kind() {
            TokenKind::OpenDelimiter(delimiter) => stack.push(*delimiter),
            TokenKind::CloseDelimiter(delimiter) => {
                if stack.pop()? != *delimiter {
                    return None;
                }
                if stack.is_empty() {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn token_text<'a>(token: &Token, output: &'a ParseOutput) -> &'a str {
    token.text(output.source())
}

fn identifier_name(name: &str) -> &str {
    name.strip_prefix("r#").unwrap_or(name)
}
