use std::ops::Range;

use crate::{
    ast::{
        AstFactory, ClosureNode, MacroDefinition, MacroInvocation, Parameter, PathNode, SyntaxNode,
    },
    recovery::Recovery,
    Delimiter, Token, TokenKind,
};

pub(super) struct SyntaxCollector;

struct ClosureParse {
    node: ClosureNode,
    next_index: usize,
    body_bounds: Range<usize>,
}

impl SyntaxCollector {
    pub(super) fn collect(tokens: &[Token], bounds: Range<usize>) -> Vec<SyntaxNode> {
        Self::collect_inner(tokens, bounds)
    }

    pub(super) fn macro_item(tokens: &[Token], start: usize) -> Option<(MacroInvocation, usize)> {
        let (path, path_end) = Self::path_at(tokens, start, tokens.len())?;
        if !tokens
            .get(path_end)
            .is_some_and(|token| Recovery::is_punctuation(token, "!"))
        {
            return None;
        }
        let (group_index, macro_name) = if path
            .segments()
            .last()
            .is_some_and(|name| name == "macro_rules")
            && matches!(tokens.get(path_end + 1)?.kind(), TokenKind::Identifier(_))
        {
            (path_end + 2, true)
        } else {
            (path_end + 1, false)
        };
        let group_end = Self::group_end(tokens, group_index, tokens.len())?;
        let mut end = group_end + 1;
        let mut end_offset = tokens[group_end].end();
        if !macro_name
            && tokens
                .get(end)
                .is_some_and(|token| Recovery::is_punctuation(token, ";"))
        {
            end_offset = tokens[end].end();
            end += 1;
        }
        let path_start = path.start();
        let invocation = AstFactory::macro_invocation(path, path_start..end_offset);
        Some((invocation, end))
    }

    pub(super) fn macro_definition(
        tokens: &[Token],
        start: usize,
    ) -> Option<(MacroDefinition, usize)> {
        let (path, path_end) = Self::path_at(tokens, start, tokens.len())?;
        if !path
            .segments()
            .last()
            .is_some_and(|name| name == "macro_rules")
            || !tokens
                .get(path_end)
                .is_some_and(|token| Recovery::is_punctuation(token, "!"))
            || !matches!(tokens.get(path_end + 1)?.kind(), TokenKind::Identifier(_))
        {
            return None;
        }
        let group_index = path_end + 2;
        let group_end = Self::group_end(tokens, group_index, tokens.len())?;
        let mut next_index = group_end + 1;
        let mut end = tokens[group_end].end();
        if tokens
            .get(next_index)
            .is_some_and(|token| Recovery::is_punctuation(token, ";"))
        {
            end = tokens[next_index].end();
            next_index += 1;
        }
        Some((AstFactory::macro_definition(path.start()..end), next_index))
    }

    pub(super) fn macro_definition_end(tokens: &[Token], start: usize) -> Option<usize> {
        Self::macro_definition(tokens, start).map(|(_, next_index)| next_index)
    }

    pub(super) fn macro_item_end(tokens: &[Token], start: usize) -> Option<usize> {
        let (path, path_end) = Self::path_at(tokens, start, tokens.len())?;
        if !tokens
            .get(path_end)
            .is_some_and(|token| Recovery::is_punctuation(token, "!"))
        {
            return None;
        }
        let group_index = if path
            .segments()
            .last()
            .is_some_and(|name| name == "macro_rules")
        {
            path_end + 2
        } else {
            path_end + 1
        };
        Self::group_end(tokens, group_index, tokens.len())
    }

    fn collect_inner(tokens: &[Token], bounds: Range<usize>) -> Vec<SyntaxNode> {
        let mut nodes = Vec::new();
        let mut index = bounds.start;
        while index < bounds.end {
            if let Some((path, path_end)) = Self::path_at(tokens, index, bounds.end) {
                if path_end < bounds.end && Recovery::is_punctuation(&tokens[path_end], "!") {
                    if let Some(group_end) = Self::group_end(tokens, path_end + 1, bounds.end) {
                        let macro_start = path.start();
                        let macro_end = tokens[group_end].end();
                        nodes.push(SyntaxNode::Macro(AstFactory::macro_invocation(
                            path,
                            macro_start..macro_end,
                        )));
                        index = group_end + 1;
                        continue;
                    }
                }
                let segments = path.segments().to_vec();
                let node_bounds = path.start()..path.end();
                if !segments.is_empty() && (segments.len() > 1 || !is_keyword(&segments[0])) {
                    nodes.push(SyntaxNode::Path(AstFactory::path(segments, node_bounds)));
                }
                index = path_end;
                continue;
            }
            if let Some(closure) = Self::closure_at(tokens, index, bounds.end) {
                nodes.push(SyntaxNode::Closure(closure.node));
                if closure.body_bounds.start < closure.body_bounds.end {
                    nodes.extend(Self::collect_inner(tokens, closure.body_bounds));
                }
                index = closure.next_index;
                continue;
            }
            if matches!(
                tokens[index].kind(),
                TokenKind::OpenDelimiter(Delimiter::Parenthesis)
            ) {
                if let Some(close_index) = Recovery::matching_delimiter(tokens, index) {
                    if close_index < bounds.end {
                        let inner_bounds = index + 1..close_index;
                        let inner_ranges = Recovery::split_top_level(
                            tokens,
                            inner_bounds.start,
                            inner_bounds.end,
                            ",",
                        );
                        let has_comma = Recovery::has_top_level_separator(
                            tokens,
                            inner_bounds.start,
                            inner_bounds.end,
                            ",",
                        );
                        if has_comma && Self::is_tuple_group(tokens, index) {
                            nodes.push(SyntaxNode::Tuple(AstFactory::tuple(
                                inner_ranges.len(),
                                tokens[index].start()..tokens[close_index].end(),
                            )));
                        }
                        nodes.extend(Self::collect_inner(tokens, inner_bounds));
                        index = close_index + 1;
                        continue;
                    }
                }
            }
            if matches!(tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                if let Some(close_index) = Recovery::matching_delimiter(tokens, index) {
                    if close_index < bounds.end {
                        nodes.extend(Self::collect_inner(tokens, index + 1..close_index));
                        index = close_index + 1;
                        continue;
                    }
                }
            }
            index += 1;
        }
        nodes
    }

    fn closure_at(tokens: &[Token], start: usize, limit: usize) -> Option<ClosureParse> {
        let token = tokens.get(start)?;
        let is_empty_closure = Recovery::is_punctuation(token, "||");
        if is_empty_closure && !Self::is_closure_boundary(tokens, start) {
            return None;
        }
        let (parameter_start, parameter_end) = if is_empty_closure {
            (start + 1, start + 1)
        } else if Recovery::is_punctuation(token, "|") && Self::is_closure_boundary(tokens, start) {
            let close =
                (start + 1..limit).find(|index| Recovery::is_punctuation(&tokens[*index], "|"))?;
            (start + 1, close)
        } else {
            return None;
        };
        let parameters = Self::parameters(tokens, parameter_start, parameter_end);
        let body_start = if is_empty_closure {
            start + 1
        } else {
            parameter_end + 1
        };
        let body_end = Self::closure_body_end(tokens, body_start, limit);
        let end_offset = tokens
            .get(body_end.saturating_sub(1))
            .map_or(token.end(), Token::end);
        let closure = AstFactory::closure(parameters, token.start()..end_offset);
        Some(ClosureParse {
            node: closure,
            next_index: body_end,
            body_bounds: body_start..body_end,
        })
    }

    fn is_closure_boundary(tokens: &[Token], index: usize) -> bool {
        if index == 0 {
            return true;
        }
        let previous = &tokens[index - 1];
        matches!(
            previous.kind(),
            TokenKind::Punctuation(value)
                if matches!(value.as_str(), "=" | "," | ";" | "=>")
        ) || matches!(
            previous.kind(),
            TokenKind::OpenDelimiter(Delimiter::Parenthesis | Delimiter::Brace)
        ) || Recovery::is_identifier(previous, "move")
            || Recovery::is_identifier(previous, "return")
    }

    fn closure_body_end(tokens: &[Token], start: usize, limit: usize) -> usize {
        let mut index = start;
        while index < limit {
            if matches!(tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                if let Some(close_index) = Recovery::matching_delimiter(tokens, index) {
                    if close_index < limit {
                        index = close_index + 1;
                        continue;
                    }
                }
            }
            if Recovery::is_punctuation(&tokens[index], ",")
                || Recovery::is_punctuation(&tokens[index], ";")
                || matches!(
                    tokens[index].kind(),
                    TokenKind::CloseDelimiter(Delimiter::Brace)
                )
            {
                break;
            }
            index += 1;
        }
        index
    }

    fn is_tuple_group(tokens: &[Token], index: usize) -> bool {
        let Some(previous) = index.checked_sub(1).and_then(|index| tokens.get(index)) else {
            return true;
        };
        match previous.kind() {
            TokenKind::Identifier(identifier) => {
                matches!(identifier.as_str(), "return" | "yield")
            }
            TokenKind::CloseDelimiter(_) => false,
            TokenKind::Punctuation(punctuation) => {
                if matches!(punctuation.as_str(), ">" | ">>")
                    && Self::is_turbofish_call(tokens, index)
                {
                    false
                } else {
                    !matches!(punctuation.as_str(), "!" | "?" | "." | "::")
                }
            }
            TokenKind::OpenDelimiter(_)
            | TokenKind::Lifetime(_)
            | TokenKind::Literal(_)
            | TokenKind::Unknown(_) => true,
        }
    }

    fn is_turbofish_call(tokens: &[Token], open_index: usize) -> bool {
        let Some(close_index) = open_index.checked_sub(1) else {
            return false;
        };
        let mut angle_depth = match tokens.get(close_index).map(Token::kind) {
            Some(TokenKind::Punctuation(punctuation)) if punctuation == ">" => 1usize,
            Some(TokenKind::Punctuation(punctuation)) if punctuation == ">>" => 2usize,
            _ => return false,
        };
        let mut nested_delimiters = Vec::new();
        let mut index = close_index;
        while let Some(previous_index) = index.checked_sub(1) {
            index = previous_index;
            match tokens[index].kind() {
                TokenKind::CloseDelimiter(delimiter) => {
                    nested_delimiters.push(*delimiter);
                    continue;
                }
                TokenKind::OpenDelimiter(delimiter) => {
                    if nested_delimiters.pop() != Some(*delimiter) {
                        return false;
                    }
                    continue;
                }
                _ if !nested_delimiters.is_empty() => continue,
                _ => {}
            }
            match tokens[index].kind() {
                TokenKind::Punctuation(punctuation) if punctuation == ">" => {
                    angle_depth += 1;
                }
                TokenKind::Punctuation(punctuation) if punctuation == ">>" => {
                    angle_depth += 2;
                }
                TokenKind::Punctuation(punctuation) if punctuation == "<" => {
                    angle_depth = angle_depth.saturating_sub(1);
                    if angle_depth == 0 {
                        return index >= 2
                            && Recovery::is_punctuation(&tokens[index - 1], "::")
                            && matches!(tokens[index - 2].kind(), TokenKind::Identifier(_));
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn path_at(tokens: &[Token], start: usize, limit: usize) -> Option<(PathNode, usize)> {
        let mut index = start;
        let mut segments = Vec::new();
        if index < limit && Recovery::is_punctuation(&tokens[index], "::") {
            segments.push("::".to_owned());
            index += 1;
        }
        let first = match tokens.get(index)?.kind() {
            TokenKind::Identifier(identifier) => identifier.clone(),
            _ => return None,
        };
        segments.push(first);
        index += 1;
        while index + 1 < limit
            && Recovery::is_punctuation(&tokens[index], "::")
            && matches!(tokens[index + 1].kind(), TokenKind::Identifier(_))
        {
            if let TokenKind::Identifier(identifier) = tokens[index + 1].kind() {
                segments.push(identifier.clone());
            }
            index += 2;
        }
        let path = AstFactory::path(segments, tokens[start].start()..tokens[index - 1].end());
        Some((path, index))
    }

    fn group_end(tokens: &[Token], index: usize, limit: usize) -> Option<usize> {
        if !matches!(tokens.get(index)?.kind(), TokenKind::OpenDelimiter(_)) {
            return None;
        }
        Recovery::matching_delimiter(tokens, index).filter(|close_index| *close_index < limit)
    }

    fn parameters(tokens: &[Token], start: usize, end: usize) -> Vec<Parameter> {
        Recovery::split_top_level(tokens, start, end, ",")
            .into_iter()
            .filter(|range| range.start < range.end)
            .map(|range| {
                let start = tokens[range.start].start();
                let end = range
                    .end
                    .checked_sub(1)
                    .and_then(|index| tokens.get(index))
                    .map_or(start, Token::end);
                AstFactory::parameter(start..end, false)
            })
            .collect()
    }
}

fn is_keyword(identifier: &str) -> bool {
    matches!(
        identifier,
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
