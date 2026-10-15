use std::{collections::BTreeSet, ops::Range};

use cop_analysis::ModuleNode;
use cop_config::ResolvedConfig;
use cop_model::{Diagnostic, LocalizedMessage, RuleId, SourceSpan};
use cop_syntax::{
    Attribute, Delimiter, Item, ItemKind, ParseOutput, SyntaxNode, Token, TokenKind, TriviaKind,
    UseTree, VisibilityKind,
};

pub(super) struct RuleReport {
    rule_id: RuleId,
    message: LocalizedMessage,
    bounds: Option<Range<usize>>,
    span: Option<SourceSpan>,
}

pub(super) struct RuleDiagnosticAdapter<'a> {
    configuration: &'a ResolvedConfig,
}

pub(super) struct UseLeaf {
    pub(super) path: Vec<String>,
    pub(super) alias: Option<String>,
    pub(super) glob: bool,
    pub(super) start: usize,
    pub(super) end: usize,
}

pub(super) struct PathReference {
    pub(super) segments: Vec<String>,
    pub(super) start: usize,
    pub(super) end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpacingDefinitionKind {
    Struct,
    Enum,
    Union,
    TypeAlias,
    Trait,
    Constant,
    Static,
    Function,
}

pub(super) struct SyntaxSupport;

impl RuleReport {
    pub(super) fn new(
        rule_id: RuleId,
        message: LocalizedMessage,
        bounds: Option<Range<usize>>,
    ) -> Self {
        Self {
            rule_id,
            message,
            bounds,
            span: None,
        }
    }

    pub(super) fn at_span(rule_id: RuleId, message: LocalizedMessage, span: SourceSpan) -> Self {
        Self {
            rule_id,
            message,
            bounds: None,
            span: Some(span),
        }
    }
}

impl RuleDiagnosticAdapter<'_> {
    pub(super) fn new(configuration: &ResolvedConfig) -> RuleDiagnosticAdapter<'_> {
        RuleDiagnosticAdapter { configuration }
    }

    pub(super) fn push(
        &self,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        report: RuleReport,
    ) {
        let setting = self.configuration.rule(report.rule_id);
        if !setting.enabled() {
            return;
        }
        let mut diagnostic = Diagnostic::rule(report.rule_id, setting.severity(), report.message)
            .with_package(module.package_name())
            .with_package_manifest_path(module.package_manifest_path())
            .with_target(module.target_name())
            .with_target_kind(module.target_kind().to_vec())
            .with_namespace(namespace_name(module.namespace()));
        if let Some(condition) = module.cfg_condition() {
            diagnostic = diagnostic.with_cfg_condition(condition);
        }
        if let Some(span) = report.span {
            diagnostic = diagnostic.with_span(span);
        } else if let Some(bounds) = report.bounds {
            if let Ok(span) = module
                .parse_output()
                .source()
                .span(bounds.start, bounds.end)
            {
                diagnostic = diagnostic.with_span(span);
            }
        }
        diagnostics.push(diagnostic);
    }
}

impl SyntaxSupport {
    pub(super) fn expected_spacing_blank_lines(previous: &Item, current: &Item) -> Option<usize> {
        let previous_kind = Self::spacing_definition_kind(previous.kind())?;
        let current_kind = Self::spacing_definition_kind(current.kind())?;
        Some(match (previous_kind, current_kind) {
            (SpacingDefinitionKind::Constant, SpacingDefinitionKind::Constant)
            | (SpacingDefinitionKind::Static, SpacingDefinitionKind::Static)
            | (SpacingDefinitionKind::TypeAlias, SpacingDefinitionKind::TypeAlias) => 0,
            _ => 1,
        })
    }

    pub(super) fn is_non_private(item: &Item) -> bool {
        !matches!(item.visibility().kind(), VisibilityKind::Private)
    }

    pub(super) fn is_cfg_test(attributes: &[Attribute], output: &ParseOutput) -> bool {
        attributes.iter().any(|attribute| {
            attribute.path().last().is_some_and(|name| name == "cfg")
                && attribute_tokens(attribute, output)
                    .windows(6)
                    .any(|tokens| {
                        tokens[0] == "#"
                            && tokens[1] == "["
                            && tokens[2] == "cfg"
                            && tokens[3] == "("
                            && tokens[4] == "test"
                            && tokens[5] == ")"
                    })
        })
    }

    pub(super) fn has_lint_relaxation(attribute: &Attribute, output: &ParseOutput) -> bool {
        let Some(name) = attribute.path().last() else {
            return false;
        };
        if name == "allow" {
            return true;
        }
        name == "cfg_attr" && cfg_attr_injects_allow(&attribute_tokens(attribute, output))
    }

    pub(super) fn for_each_syntax_node(item: &Item, visitor: &mut dyn FnMut(&SyntaxNode)) {
        match item.kind() {
            ItemKind::Function(function) => {
                Self::visit_nodes(function.signature_nodes(), visitor);
                if let Some(body) = function.body() {
                    Self::visit_nodes(body.nodes(), visitor);
                }
            }
            ItemKind::Struct(declaration)
            | ItemKind::Enum(declaration)
            | ItemKind::Union(declaration)
            | ItemKind::TypeAlias(declaration)
            | ItemKind::Constant(declaration)
            | ItemKind::Static(declaration) => {
                Self::visit_nodes(declaration.nodes(), visitor);
            }
            ItemKind::Trait(trait_item) => {
                for nested in trait_item.items() {
                    Self::for_each_syntax_node(nested, visitor);
                }
            }
            ItemKind::Impl(impl_item) => {
                for nested in impl_item.items() {
                    Self::for_each_syntax_node(nested, visitor);
                }
            }
            _ => {}
        }
    }

    pub(super) fn use_leaves(tree: &UseTree) -> Vec<UseLeaf> {
        let mut leaves = Vec::new();
        Self::collect_use_leaves(tree, &[], &mut leaves);
        leaves
    }

    pub(super) fn impl_header_references(module: &ModuleNode, item: &Item) -> Vec<PathReference> {
        let output = module.parse_output();
        let tokens = Self::token_range(output, item.start(), item.end()).collect::<Vec<_>>();
        let Some(impl_index) = tokens
            .iter()
            .position(|token| token.text(output.source()) == "impl")
        else {
            return Vec::new();
        };
        let Some(body_index) = impl_body_open(&tokens, impl_index, output) else {
            return Vec::new();
        };
        let generic_parameters = impl_generic_parameters(&tokens, impl_index, body_index, output);
        let mut references = Vec::new();
        let mut index = impl_index + 1;
        while index < body_index {
            let Some(token) = tokens.get(index) else {
                break;
            };
            if !matches!(token.kind(), TokenKind::Identifier(_)) {
                index += 1;
                continue;
            }
            let mut segments = vec![token.text(output.source()).to_owned()];
            let mut end_index = index;
            while end_index + 2 < body_index
                && tokens[end_index + 1].text(output.source()) == "::"
                && matches!(tokens[end_index + 2].kind(), TokenKind::Identifier(_))
            {
                segments.push(tokens[end_index + 2].text(output.source()).to_owned());
                end_index += 2;
            }
            let first = segments.first().map(String::as_str).unwrap_or_default();
            let rooted_keyword = matches!(first, "crate" | "self" | "super");
            if (rooted_keyword || !is_path_keyword(first))
                && !generic_parameters.contains(&segments[0])
            {
                references.push(PathReference {
                    segments,
                    start: token.start(),
                    end: tokens[end_index].end(),
                });
                index = end_index + 1;
            } else {
                index += 1;
            }
        }
        references
    }

    pub(super) fn effective_cfg(attributes: &[Attribute], output: &ParseOutput) -> Option<String> {
        let mut conditions = Vec::new();
        for attribute in attributes {
            let Some(name) = attribute.path().last() else {
                continue;
            };
            if name == "cfg_attr" {
                return None;
            }
            if name != "cfg" {
                continue;
            }
            let tokens = attribute_tokens(attribute, output);
            let open = tokens.iter().position(|token| token == "(")?;
            let close = tokens.iter().rposition(|token| token == ")")?;
            if close <= open {
                return None;
            }
            collect_cfg_conjuncts(&tokens[open + 1..close], &mut conditions)?;
        }
        conditions.sort();
        conditions.dedup();
        match conditions.len() {
            0 => Some("true".to_owned()),
            1 => conditions.into_iter().next(),
            _ => Some(format!("all({})", conditions.join(","))),
        }
    }

    pub(super) fn blank_lines_between(
        output: &ParseOutput,
        previous_end: usize,
        next_start: usize,
    ) -> usize {
        let source = output.source();
        let Some((end_line, _)) = source.line_column(previous_end) else {
            return 0;
        };
        let Some((start_line, _)) = source.line_column(next_start) else {
            return 0;
        };
        if start_line <= end_line + 1 {
            return 0;
        }
        let comments = output
            .tokens()
            .iter()
            .flat_map(|token| token.leading_trivia())
            .chain(output.trailing_trivia())
            .filter(|trivia| {
                matches!(
                    trivia.kind(),
                    TriviaKind::LineComment | TriviaKind::BlockComment
                )
            })
            .map(|trivia| trivia.start()..trivia.end())
            .collect::<Vec<_>>();
        (end_line + 1..start_line)
            .filter(|line| {
                let Some(range) = source.line_range(*line) else {
                    return false;
                };
                let contains_comment = comments.iter().any(|comment| {
                    if range.is_empty() {
                        comment.start <= range.start && comment.end > range.start
                    } else {
                        comment.start < range.end && comment.end > range.start
                    }
                });
                !contains_comment
                    && source
                        .slice(range.start, range.end)
                        .is_some_and(|text| text.chars().all(char::is_whitespace))
            })
            .count()
    }

    pub(super) fn token_range(
        output: &ParseOutput,
        start: usize,
        end: usize,
    ) -> impl Iterator<Item = &Token> {
        output
            .tokens()
            .iter()
            .filter(move |token| token.start() >= start && token.end() <= end)
    }

    fn visit_nodes(nodes: &[SyntaxNode], visitor: &mut dyn FnMut(&SyntaxNode)) {
        for node in nodes {
            visitor(node);
        }
    }

    fn collect_use_leaves(tree: &UseTree, prefix: &[String], leaves: &mut Vec<UseLeaf>) {
        let mut path = prefix.to_vec();
        path.extend(tree.path().iter().cloned());
        if tree.children().is_empty() {
            leaves.push(UseLeaf {
                path,
                alias: tree.alias().map(str::to_owned),
                glob: tree.is_glob(),
                start: tree.start(),
                end: tree.end(),
            });
            return;
        }
        for child in tree.children() {
            Self::collect_use_leaves(child, &path, leaves);
        }
    }

    fn spacing_definition_kind(kind: &ItemKind) -> Option<SpacingDefinitionKind> {
        match kind {
            ItemKind::Struct(_) => Some(SpacingDefinitionKind::Struct),
            ItemKind::Enum(_) => Some(SpacingDefinitionKind::Enum),
            ItemKind::Union(_) => Some(SpacingDefinitionKind::Union),
            ItemKind::TypeAlias(_) => Some(SpacingDefinitionKind::TypeAlias),
            ItemKind::Trait(_) => Some(SpacingDefinitionKind::Trait),
            ItemKind::Constant(_) => Some(SpacingDefinitionKind::Constant),
            ItemKind::Static(_) => Some(SpacingDefinitionKind::Static),
            ItemKind::Function(_) => Some(SpacingDefinitionKind::Function),
            _ => None,
        }
    }
}

fn namespace_name(namespace: &[String]) -> String {
    if namespace.is_empty() {
        "crate".to_owned()
    } else {
        format!("crate::{}", namespace.join("::"))
    }
}

fn attribute_tokens(attribute: &Attribute, output: &ParseOutput) -> Vec<String> {
    output
        .tokens()
        .iter()
        .filter(|token| token.start() >= attribute.start() && token.end() <= attribute.end())
        .map(|token| token.text(output.source()).to_owned())
        .collect()
}

fn normalize_cfg_expression(tokens: &[String]) -> Option<String> {
    if tokens.is_empty() {
        return None;
    }
    if tokens.len() == 1 {
        return Some(tokens[0].clone());
    }
    if tokens.len() >= 3 && tokens[1] == "=" {
        return Some(tokens.join(""));
    }
    if tokens.len() < 3 || tokens[1] != "(" || tokens.last()? != ")" {
        return Some(tokens.join(""));
    }
    let name = tokens[0].as_str();
    if !matches!(name, "all" | "any" | "not") {
        return Some(tokens.join(""));
    }
    let arguments = split_cfg_arguments(&tokens[2..tokens.len() - 1])?;
    let mut normalized = if name == "all" {
        let mut flattened = Vec::new();
        for argument in &arguments {
            collect_cfg_conjuncts(argument, &mut flattened)?;
        }
        flattened
    } else {
        arguments
            .iter()
            .map(|argument| normalize_cfg_expression(argument))
            .collect::<Option<Vec<_>>>()?
    };
    if name == "not" && normalized.len() != 1 {
        return None;
    }
    if matches!(name, "all" | "any") {
        normalized.sort();
    }
    Some(format!("{name}({})", normalized.join(",")))
}

fn collect_cfg_conjuncts(tokens: &[String], conditions: &mut Vec<String>) -> Option<()> {
    if tokens.len() >= 3 && tokens[0] == "all" && tokens[1] == "(" && tokens.last()? == ")" {
        for argument in split_cfg_arguments(&tokens[2..tokens.len() - 1])? {
            collect_cfg_conjuncts(&argument, conditions)?;
        }
    } else {
        conditions.push(normalize_cfg_expression(tokens)?);
    }
    Some(())
}

fn cfg_attr_injects_allow(tokens: &[String]) -> bool {
    let Some(open) = tokens.iter().position(|token| token == "(") else {
        return false;
    };
    let Some(close) = tokens.iter().rposition(|token| token == ")") else {
        return false;
    };
    let Some(arguments) = tokens.get(open + 1..close).and_then(split_cfg_arguments) else {
        return false;
    };
    arguments
        .iter()
        .skip(1)
        .any(|attribute| match attribute.first().map(String::as_str) {
            Some("allow") => true,
            Some("cfg_attr") => cfg_attr_injects_allow(attribute),
            _ => false,
        })
}

fn impl_body_open(tokens: &[&Token], impl_index: usize, output: &ParseOutput) -> Option<usize> {
    let mut angle_depth = 0usize;
    let mut nested_depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(impl_index + 1) {
        match token.kind() {
            TokenKind::OpenDelimiter(Delimiter::Parenthesis | Delimiter::Bracket) => {
                nested_depth += 1;
            }
            TokenKind::CloseDelimiter(Delimiter::Parenthesis | Delimiter::Bracket) => {
                nested_depth = nested_depth.saturating_sub(1);
            }
            TokenKind::OpenDelimiter(Delimiter::Brace) if angle_depth == 0 && nested_depth == 0 => {
                return Some(index);
            }
            _ => {}
        }
        match token.text(output.source()) {
            "<" => angle_depth += 1,
            ">" => angle_depth = angle_depth.saturating_sub(1),
            ">>" => angle_depth = angle_depth.saturating_sub(2),
            _ => {}
        }
    }
    None
}

fn impl_generic_parameters(
    tokens: &[&Token],
    impl_index: usize,
    body_index: usize,
    output: &ParseOutput,
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let Some(open_index) = tokens
        .get(impl_index + 1)
        .filter(|token| token.text(output.source()) == "<")
        .map(|_| impl_index + 1)
    else {
        return names;
    };
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().take(body_index).skip(open_index) {
        let text = token.text(output.source());
        if text == "<" {
            depth += 1;
            continue;
        }
        if text == ">" || text == ">>" {
            depth = depth.saturating_sub(if text == ">>" { 2 } else { 1 });
            if depth == 0 {
                break;
            }
            continue;
        }
        if depth != 1 {
            continue;
        }
        let is_parameter = index == open_index + 1
            || tokens[index - 1].text(output.source()) == ","
            || tokens[index - 1].text(output.source()) == "const";
        if is_parameter {
            if let TokenKind::Identifier(name) = token.kind() {
                if name != "const" {
                    names.insert(name.clone());
                }
            }
        }
    }
    names
}

fn is_path_keyword(value: &str) -> bool {
    matches!(
        value,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "union"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "yield"
    )
}

fn split_cfg_arguments(tokens: &[String]) -> Option<Vec<Vec<String>>> {
    if tokens.is_empty() {
        return Some(Vec::new());
    }
    let mut arguments = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token.as_str() {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth = depth.checked_sub(1)?,
            "," if depth == 0 => {
                if start == index {
                    return None;
                }
                arguments.push(tokens[start..index].to_vec());
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    if start < tokens.len() {
        arguments.push(tokens[start..].to_vec());
    } else if start != 0 {
        return None;
    }
    Some(arguments)
}
