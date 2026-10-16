use std::{collections::BTreeMap, ops::Range};

use cop_syntax::{Delimiter, FunctionBody, Item, ItemKind, SourceFile, Token, TokenKind};

use crate::ModuleNode;

#[derive(Clone, Debug, Eq, PartialEq)]
struct LocalBinding {
    context_index: usize,
    namespace: Vec<String>,
    name: String,
    target_path: Option<Vec<String>>,
    scope: Range<usize>,
    visible_from: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LocalBindingMatch {
    name: String,
    target_path: Option<Vec<String>>,
    scope_start: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct LocalBindings {
    bindings: Vec<LocalBinding>,
}

impl LocalBindingMatch {
    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn target_path(&self) -> Option<&[String]> {
        self.target_path.as_deref()
    }

    pub(super) fn scope_start(&self) -> usize {
        self.scope_start
    }
}

impl LocalBindings {
    pub(super) fn build(modules: &[ModuleNode]) -> Self {
        let mut bindings = Vec::new();
        for module in modules {
            for item in module.items() {
                Self::collect_item(item, module, &mut bindings);
            }
        }
        Self { bindings }
    }

    pub(super) fn find(
        &self,
        context_index: usize,
        namespace: &[String],
        offset: usize,
        name: &str,
    ) -> Option<LocalBindingMatch> {
        self.bindings
            .iter()
            .filter(|binding| {
                binding.context_index == context_index
                    && binding.namespace == namespace
                    && identifier_name(&binding.name) == identifier_name(name)
                    && binding.scope.contains(&offset)
                    && binding.visible_from <= offset
            })
            .min_by_key(|binding| {
                (
                    binding.scope.end.saturating_sub(binding.scope.start),
                    usize::MAX - binding.visible_from,
                )
            })
            .map(|binding| LocalBindingMatch {
                name: binding.name.clone(),
                target_path: binding.target_path.clone(),
                scope_start: binding.scope.start,
            })
    }

    fn collect_item(item: &Item, module: &ModuleNode, bindings: &mut Vec<LocalBinding>) {
        match item.kind() {
            ItemKind::Function(function) => {
                let Some(body) = function.body() else {
                    return;
                };
                let scope = body.start()..body.end();
                for parameter in function.parameters() {
                    if let Some(name) = parameter_binding_name(
                        module.parse_output().tokens(),
                        parameter.start(),
                        parameter.end(),
                        module.parse_output().source(),
                    ) {
                        bindings.push(LocalBinding {
                            context_index: module.context_index(),
                            namespace: module.namespace().to_vec(),
                            name,
                            target_path: None,
                            scope: scope.clone(),
                            visible_from: body.start(),
                        });
                    }
                }
                collect_block_bindings(module, body, bindings);
            }
            ItemKind::Trait(trait_item) => {
                for nested in trait_item.items() {
                    Self::collect_item(nested, module, bindings);
                }
            }
            ItemKind::Impl(impl_item) => {
                for nested in impl_item.items() {
                    Self::collect_item(nested, module, bindings);
                }
            }
            _ => {}
        }
    }
}

fn collect_block_bindings(
    module: &ModuleNode,
    body: &FunctionBody,
    bindings: &mut Vec<LocalBinding>,
) {
    let output = module.parse_output();
    let tokens = output.tokens();
    let Some(start_index) = tokens
        .iter()
        .position(|token| token.start() >= body.start())
    else {
        return;
    };
    let end_index = tokens
        .iter()
        .position(|token| token.start() >= body.end())
        .unwrap_or(tokens.len());
    let mut brace_stack = Vec::new();
    let mut brace_pairs = BTreeMap::new();
    for (index, token) in tokens.iter().enumerate().take(end_index).skip(start_index) {
        match token.kind() {
            TokenKind::OpenDelimiter(Delimiter::Brace) => brace_stack.push(index),
            TokenKind::CloseDelimiter(Delimiter::Brace) => {
                if let Some(open) = brace_stack.pop() {
                    brace_pairs.insert(open, index);
                }
            }
            _ => {}
        }
    }
    let mut active_scopes = Vec::new();
    active_scopes.push(body.start()..body.end());
    let mut index = start_index;
    while index < end_index {
        let token = &tokens[index];
        if let TokenKind::CloseDelimiter(Delimiter::Brace) = token.kind() {
            if active_scopes.len() > 1 {
                active_scopes.pop();
            }
            index += 1;
            continue;
        }
        if let TokenKind::OpenDelimiter(Delimiter::Brace) = token.kind() {
            if let Some(close) = brace_pairs.get(&index) {
                active_scopes.push(token.start()..tokens[*close].end());
            }
            index += 1;
            continue;
        }
        if token.text(output.source()) == "!"
            && tokens
                .get(index + 1)
                .is_some_and(|next| matches!(next.kind(), TokenKind::OpenDelimiter(_)))
        {
            if let Some(close) = matching_delimiter_index(tokens, index + 1, end_index) {
                index = close + 1;
                continue;
            }
        }
        if token.text(output.source()) == "use" {
            if let Some(semicolon) = statement_end(tokens, index + 1, end_index) {
                if let Some((name, target_path)) =
                    local_use_binding(tokens, index, semicolon, output.source())
                {
                    if let Some(scope) = active_scopes.last() {
                        bindings.push(LocalBinding {
                            context_index: module.context_index(),
                            namespace: module.namespace().to_vec(),
                            name,
                            target_path: Some(target_path),
                            scope: scope.clone(),
                            visible_from: scope.start,
                        });
                    }
                }
            }
        } else if token.text(output.source()) == "let"
            && is_item_boundary(tokens, index, start_index, output.source())
        {
            if let Some(semicolon) = statement_end(tokens, index + 1, end_index) {
                if let Some(name) = let_binding_name(tokens, index + 1, semicolon) {
                    if let Some(scope) = active_scopes.last() {
                        bindings.push(LocalBinding {
                            context_index: module.context_index(),
                            namespace: module.namespace().to_vec(),
                            name,
                            target_path: None,
                            scope: scope.clone(),
                            visible_from: tokens[semicolon].end(),
                        });
                    }
                }
            }
        } else if let Some(name) = local_item_name(tokens, index, output.source()) {
            if let Some(scope) = active_scopes.last() {
                bindings.push(LocalBinding {
                    context_index: module.context_index(),
                    namespace: module.namespace().to_vec(),
                    name,
                    target_path: None,
                    scope: scope.clone(),
                    visible_from: scope.start,
                });
            }
        }
        index += 1;
    }
}

fn parameter_binding_name(
    tokens: &[Token],
    start: usize,
    end: usize,
    source: &SourceFile,
) -> Option<String> {
    let mut name = None;
    for token in tokens
        .iter()
        .filter(|token| token.start() >= start && token.end() <= end)
    {
        match token.kind() {
            TokenKind::OpenDelimiter(_) => return None,
            TokenKind::Punctuation(punctuation) if punctuation == ":" => break,
            TokenKind::Identifier(identifier)
                if !matches!(identifier.as_str(), "mut" | "ref" | "_") =>
            {
                if identifier == "self" {
                    continue;
                }
                if name.is_some() {
                    return None;
                }
                name = Some(identifier.clone());
            }
            _ => {}
        }
        if token.text(source) == "," {
            break;
        }
    }
    name
}

fn local_use_binding(
    tokens: &[Token],
    use_index: usize,
    semicolon: usize,
    source: &SourceFile,
) -> Option<(String, Vec<String>)> {
    let mut as_index = None;
    for (index, token) in tokens
        .iter()
        .enumerate()
        .take(semicolon)
        .skip(use_index + 1)
    {
        if token.text(source) == "as" {
            as_index = Some(index);
            break;
        }
        if matches!(token.kind(), TokenKind::OpenDelimiter(_)) {
            return None;
        }
    }
    let path_end = as_index.unwrap_or(semicolon);
    let mut path = Vec::new();
    let mut index = use_index + 1;
    if index < path_end && tokens[index].text(source) == "::" {
        path.push("::".to_owned());
        index += 1;
    }
    let mut expects_identifier = true;
    while index < path_end {
        if expects_identifier {
            let TokenKind::Identifier(segment) = tokens[index].kind() else {
                return None;
            };
            path.push(segment.clone());
        } else if tokens[index].text(source) != "::" {
            return None;
        }
        expects_identifier = !expects_identifier;
        index += 1;
    }
    if path.is_empty() || expects_identifier {
        return None;
    }
    let name = if let Some(as_index) = as_index {
        match tokens.get(as_index + 1)?.kind() {
            TokenKind::Identifier(alias) if as_index + 2 == semicolon => alias.clone(),
            _ => return None,
        }
    } else if path.last().is_some_and(|segment| segment == "self") && path.len() > 1 {
        path[path.len() - 2].clone()
    } else {
        path.last()?.clone()
    };
    Some((name, path))
}

fn let_binding_name(tokens: &[Token], start: usize, semicolon: usize) -> Option<String> {
    let mut name = None;
    for token in tokens.iter().take(semicolon).skip(start) {
        match token.kind() {
            TokenKind::OpenDelimiter(_) => return None,
            TokenKind::Punctuation(punctuation) if punctuation == "=" || punctuation == ":" => {
                break;
            }
            TokenKind::Identifier(identifier)
                if !matches!(identifier.as_str(), "mut" | "ref" | "_") =>
            {
                if name.is_some() {
                    return None;
                }
                name = Some(identifier.clone());
            }
            _ => {}
        }
    }
    name
}

fn local_item_name(tokens: &[Token], index: usize, source: &SourceFile) -> Option<String> {
    let keyword = tokens[index].text(source);
    if !matches!(
        keyword,
        "mod" | "struct" | "enum" | "union" | "type" | "trait" | "fn" | "const" | "static"
    ) {
        return None;
    }
    let name_index = if keyword == "const"
        && tokens
            .get(index + 1)
            .is_some_and(|token| token.text(source) == "fn")
    {
        index + 2
    } else {
        index + 1
    };
    match tokens.get(name_index)?.kind() {
        TokenKind::Identifier(name) if !matches!(name.as_str(), "mut" | "ref") => {
            Some(name.clone())
        }
        _ => None,
    }
}

fn is_item_boundary(tokens: &[Token], index: usize, start: usize, source: &SourceFile) -> bool {
    if index <= start {
        return true;
    }
    let previous = &tokens[index - 1];
    matches!(previous.kind(), TokenKind::OpenDelimiter(Delimiter::Brace))
        || matches!(previous.kind(), TokenKind::CloseDelimiter(Delimiter::Brace))
        || previous.text(source) == ";"
        || matches!(
            previous.kind(),
            TokenKind::CloseDelimiter(Delimiter::Bracket)
        )
}

fn statement_end(tokens: &[Token], start: usize, limit: usize) -> Option<usize> {
    let mut delimiters = Vec::new();
    for (index, token) in tokens.iter().enumerate().take(limit).skip(start) {
        match token.kind() {
            TokenKind::OpenDelimiter(delimiter) => delimiters.push(*delimiter),
            TokenKind::CloseDelimiter(delimiter) => {
                if delimiters.last() == Some(delimiter) {
                    delimiters.pop();
                }
            }
            TokenKind::Punctuation(punctuation) if punctuation == ";" && delimiters.is_empty() => {
                return Some(index);
            }
            _ => {}
        }
    }
    None
}

fn matching_delimiter_index(tokens: &[Token], open: usize, limit: usize) -> Option<usize> {
    let delimiter = match tokens.get(open)?.kind() {
        TokenKind::OpenDelimiter(delimiter) => *delimiter,
        _ => return None,
    };
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().take(limit).skip(open) {
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

fn identifier_name(name: &str) -> &str {
    name.strip_prefix("r#").unwrap_or(name)
}
