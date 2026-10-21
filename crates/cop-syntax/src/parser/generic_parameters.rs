use std::ops::Range;

use crate::{
    ast::{
        AstFactory, AstNodeId, GenericParameterKind, GenericParameterParts, GenericParameterSyntax,
        Item, ItemKind, ParseIssue, SyntaxNode, SyntaxNodeData, SyntaxNodeKind,
    },
    recovery::Recovery,
    SourceFile, Token, TokenKind,
};

use super::{
    attributes::AttributeParser, path::PathParser, token_cursor::TokenCursor, types::TypeParser,
};

pub(super) struct GenericParameterParser;

pub(super) struct GenericParameterInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) parent: AstNodeId,
    pub(super) item: &'a Item,
    pub(super) ast_nodes: &'a mut Vec<SyntaxNode>,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

struct GenericParameterNodeInput<'a> {
    source: &'a SourceFile,
    tokens: &'a [Token],
    parent: AstNodeId,
    range: Range<usize>,
    ast_nodes: &'a mut Vec<SyntaxNode>,
    issues: &'a mut Vec<ParseIssue>,
}

impl GenericParameterParser {
    pub(super) fn collect(input: GenericParameterInput<'_>) {
        let GenericParameterInput {
            source,
            tokens,
            parent,
            item,
            ast_nodes,
            issues,
        } = input;
        let keyword = match item.kind() {
            ItemKind::Function(_) => "fn",
            ItemKind::Struct(_) => "struct",
            ItemKind::Enum(_) => "enum",
            ItemKind::Union(_) => "union",
            ItemKind::TypeAlias(_) => "type",
            ItemKind::Trait(_) => "trait",
            ItemKind::Impl(_) => "impl",
            _ => return,
        };
        let Some(first) = tokens
            .iter()
            .position(|token| token.start() >= item.start() && token.end() <= item.end())
        else {
            return;
        };
        let after = tokens
            .iter()
            .enumerate()
            .skip(first)
            .find(|(_, token)| token.start() >= item.end())
            .map_or(tokens.len(), |(index, _)| index);
        let Some(keyword_index) = (first..after).find(|index| {
            matches!(tokens[*index].kind(), TokenKind::Identifier(value) if value == keyword)
        }) else {
            return;
        };
        let open = if keyword == "impl" {
            keyword_index + 1
        } else {
            keyword_index + 2
        };
        if !tokens
            .get(open)
            .is_some_and(|token| Recovery::is_punctuation(token, "<"))
        {
            return;
        }
        let Some(close) = matching_angle(tokens, open, after) else {
            return;
        };
        let mut range_start = open + 1;
        let mut index = open + 1;
        let mut angle_depth = 0usize;
        while index < close {
            if matches!(tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                if let Some(group_end) = Recovery::matching_delimiter(tokens, index) {
                    if group_end < close {
                        index = group_end + 1;
                        continue;
                    }
                }
            }
            match tokens[index].kind() {
                TokenKind::Punctuation(value) if value == "<" => angle_depth += 1,
                TokenKind::Punctuation(value) if value == "<<" => angle_depth += 2,
                TokenKind::Punctuation(value) if value == ">" => {
                    angle_depth = angle_depth.saturating_sub(1);
                }
                TokenKind::Punctuation(value) if value == ">>" => {
                    angle_depth = angle_depth.saturating_sub(2);
                }
                _ => {}
            }
            if angle_depth == 0 && Recovery::is_punctuation(&tokens[index], ",") {
                Self::push_parameter(GenericParameterNodeInput {
                    source,
                    tokens,
                    parent,
                    range: range_start..index,
                    ast_nodes,
                    issues,
                });
                range_start = index + 1;
            }
            index += 1;
        }
        Self::push_parameter(GenericParameterNodeInput {
            source,
            tokens,
            parent,
            range: range_start..close,
            ast_nodes,
            issues,
        });
    }

    fn push_parameter(input: GenericParameterNodeInput<'_>) {
        let GenericParameterNodeInput {
            source,
            tokens,
            parent,
            range,
            ast_nodes,
            issues,
        } = input;
        let Some(first) = tokens.get(range.start) else {
            return;
        };
        let Some(last) = range.end.checked_sub(1).and_then(|index| tokens.get(index)) else {
            return;
        };
        let mut cursor = range.start;
        let attributes = AttributeParser::parse_attributes(source, tokens, &mut cursor, issues);
        let start = attributes
            .first()
            .map_or(first.start(), |attribute| attribute.start());
        let parameter_id = AstFactory::push_ast_node(
            ast_nodes,
            Some(parent),
            SyntaxNodeKind::GenericParameter,
            start..last.end(),
            attributes,
        );
        if let Some(syntax) = Self::parse_parameter_syntax(tokens, cursor..range.end) {
            AstFactory::set_ast_node_data(
                ast_nodes,
                parameter_id,
                SyntaxNodeData::GenericParameter(syntax),
            );
        }
    }

    fn parse_parameter_syntax(
        tokens: &[Token],
        range: Range<usize>,
    ) -> Option<GenericParameterSyntax> {
        let mut cursor = TokenCursor::new(tokens, range.clone())?;
        let start_offset = cursor.offset();
        let (kind, name) = match cursor.consume()?.kind() {
            TokenKind::Lifetime(name) => (GenericParameterKind::Lifetime, name.clone()),
            TokenKind::Identifier(value) if value == "const" => {
                let name = match cursor.consume()?.kind() {
                    TokenKind::Identifier(name) => name.clone(),
                    _ => return None,
                };
                (GenericParameterKind::Const, name)
            }
            TokenKind::Identifier(name) => (GenericParameterKind::Type, name.clone()),
            _ => return None,
        };
        let mut trait_bounds = Vec::new();
        let mut lifetime_bounds = Vec::new();
        let mut declared_type = None;
        let mut default_type = None;
        let mut default_const = None;
        if cursor.consume_punctuation(":") {
            if kind == GenericParameterKind::Const {
                let type_end = (cursor.index()..range.end)
                    .find(|index| Recovery::is_punctuation(&tokens[*index], "="))
                    .unwrap_or(range.end);
                declared_type = TypeParser::parse_range(tokens, cursor.index()..type_end).ok();
                while cursor.index() < type_end {
                    cursor.consume();
                }
            } else if kind == GenericParameterKind::Lifetime {
                loop {
                    let TokenKind::Lifetime(lifetime) = cursor.peek()?.kind() else {
                        return None;
                    };
                    lifetime_bounds.push(lifetime.clone());
                    cursor.consume();
                    if !cursor.consume_punctuation("+") {
                        break;
                    }
                }
            } else {
                trait_bounds = TypeParser::parse_bounds(&mut cursor).ok()?;
            }
        }
        if cursor.consume_punctuation("=") {
            if kind == GenericParameterKind::Const {
                let start = cursor.index();
                default_const = Some(PathParser::span(
                    tokens,
                    start,
                    range.end,
                    cursor.offset(),
                    tokens
                        .get(range.end.saturating_sub(1))
                        .map_or(cursor.offset(), Token::end),
                ));
                while !cursor.is_empty() {
                    cursor.consume();
                }
            } else {
                default_type = Some(TypeParser::parse(&mut cursor).ok()?);
            }
        }
        if !cursor.is_empty() {
            return None;
        }
        Some(GenericParameterSyntax::new(GenericParameterParts {
            kind,
            name,
            trait_bounds,
            lifetime_bounds,
            declared_type,
            default_type,
            default_const,
            bounds_range: start_offset
                ..tokens
                    .get(range.end.saturating_sub(1))
                    .map_or(start_offset, Token::end),
        }))
    }
}

fn matching_angle(tokens: &[Token], open: usize, end: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = open;
    while index < end {
        if matches!(tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
            if let Some(close) = Recovery::matching_delimiter(tokens, index) {
                if close < end {
                    index = close + 1;
                    continue;
                }
            }
        }
        match tokens[index].kind() {
            TokenKind::Punctuation(value) if value == "<" => depth += 1,
            TokenKind::Punctuation(value) if value == "<<" => depth += 2,
            TokenKind::Punctuation(value) if value == ">" => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            TokenKind::Punctuation(value) if value == ">>" => {
                depth = depth.checked_sub(2)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}
