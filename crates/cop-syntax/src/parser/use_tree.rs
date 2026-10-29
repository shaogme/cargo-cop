use cop_i18n::t;
use std::ops::Range;

use crate::{
    ast::{
        AstFactory, AstNodeId, Attribute, ParseIssue, ParseIssueKind, SyntaxNode, SyntaxNodeData,
        SyntaxNodeKind, UseTreeSyntaxData,
    },
    recovery::Recovery,
    Delimiter, SourceFile, Token, TokenKind,
};

use super::attributes::AttributeParser;

pub(super) struct UseTreeParser;

pub(super) struct UseTreeInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) start: usize,
    pub(super) limit: usize,
    pub(super) parent: AstNodeId,
}

struct ParseContext<'a> {
    source: &'a SourceFile,
    tokens: &'a [Token],
    issues: &'a mut Vec<ParseIssue>,
    ast_nodes: &'a mut Vec<SyntaxNode>,
}

impl UseTreeParser {
    pub(super) fn is_use_path_identifier(identifier: &str) -> bool {
        if let Some(raw_identifier) = identifier.strip_prefix("r#") {
            return !matches!(raw_identifier, "_" | "crate" | "self" | "Self" | "super");
        }
        matches!(identifier, "crate" | "self" | "Self" | "super") || is_plain_identifier(identifier)
    }

    pub(super) fn parse(
        input: UseTreeInput<'_>,
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) -> (bool, usize) {
        let UseTreeInput {
            source,
            tokens,
            start,
            limit,
            parent,
        } = input;
        let node_count = ast_nodes.len();
        let mut context = ParseContext {
            source,
            tokens,
            issues,
            ast_nodes,
        };
        let mut cursor = start;
        let parsed = Self::parse_node(&mut context, &mut cursor, limit, parent);
        if parsed.is_some() && cursor != limit {
            let issue_start = token_start(tokens, cursor);
            let end = tokens
                .get(limit.saturating_sub(1))
                .map_or(issue_start, Token::end);
            Self::push_issue(&mut context, issue_start..end);
        }
        if parsed.is_some() && cursor == limit {
            return (true, cursor);
        }

        let root_id = context.ast_nodes.get(node_count).map(SyntaxNode::id);
        let attributes = root_id
            .and_then(|id| context.ast_nodes.get(id.index()))
            .map_or_else(Vec::new, |node| node.attributes().to_vec());
        let start_offset = attributes
            .first()
            .map_or_else(|| token_start(tokens, start), Attribute::start);
        let end_offset = tokens
            .get(limit.saturating_sub(1))
            .map_or(start_offset, Token::end)
            .max(start_offset);
        if let Some(root_id) = root_id {
            AstFactory::update_ast_node(
                context.ast_nodes,
                root_id,
                SyntaxNodeKind::Opaque,
                start_offset..end_offset,
                attributes,
            );
            AstFactory::clear_ast_node_data(context.ast_nodes, root_id);
        }
        (false, limit)
    }

    fn parse_node(
        context: &mut ParseContext<'_>,
        cursor: &mut usize,
        limit: usize,
        parent: AstNodeId,
    ) -> Option<()> {
        let attributes = AttributeParser::parse_attributes(
            context.source,
            context.tokens,
            cursor,
            context.issues,
        );
        let start_index = *cursor;
        let start = attributes.first().map_or_else(
            || token_start(context.tokens, start_index),
            Attribute::start,
        );
        let node_id = AstFactory::push_ast_node(
            context.ast_nodes,
            Some(parent),
            SyntaxNodeKind::UseTree,
            start..start,
            attributes.clone(),
        );
        let mut path = Vec::new();
        let mut alias = None;
        let mut glob = false;
        let mut trailing_separator = None;

        if *cursor < limit && Recovery::is_punctuation(&context.tokens[*cursor], "::") {
            trailing_separator =
                Some(context.tokens[*cursor].start()..context.tokens[*cursor].end());
            path.push("::".to_owned());
            *cursor += 1;
        }

        let mut needs_segment = path.is_empty() || path.as_slice() == ["::"];
        while *cursor < limit {
            if matches!(
                context.tokens[*cursor].kind(),
                TokenKind::OpenDelimiter(Delimiter::Brace)
            ) && (!needs_segment || path.is_empty() || path.as_slice() == ["::"])
            {
                break;
            }
            if Recovery::is_punctuation(&context.tokens[*cursor], "*")
                && needs_segment
                && !path.is_empty()
            {
                glob = true;
                *cursor += 1;
                needs_segment = false;
                break;
            }
            if needs_segment {
                if let Some(identifier) = context.tokens[*cursor].kind().identifier() {
                    if Self::is_use_path_identifier(identifier) {
                        path.push(identifier.to_owned());
                        *cursor += 1;
                        needs_segment = false;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            } else if Recovery::is_punctuation(&context.tokens[*cursor], "::") {
                trailing_separator =
                    Some(context.tokens[*cursor].start()..context.tokens[*cursor].end());
                *cursor += 1;
                needs_segment = true;
                if *cursor < limit
                    && matches!(
                        context.tokens[*cursor].kind(),
                        TokenKind::OpenDelimiter(Delimiter::Brace)
                    )
                {
                    break;
                }
            } else {
                break;
            }
        }

        let has_group = *cursor < limit
            && matches!(
                context.tokens[*cursor].kind(),
                TokenKind::OpenDelimiter(Delimiter::Brace)
            );
        if has_group {
            let open = *cursor;
            let Some(close) =
                Recovery::matching_delimiter(context.tokens, open).filter(|close| *close < limit)
            else {
                Self::push_issue(
                    context,
                    context.tokens[open].start()..context.tokens[limit - 1].end(),
                );
                Self::mark_opaque(context, node_id, start..context.tokens[limit - 1].end());
                *cursor = limit;
                return None;
            };
            if open + 1 == close {
                let end = context.tokens[close].end();
                Self::push_issue(context, context.tokens[open].start()..end);
                Self::mark_opaque(context, node_id, start..end);
                *cursor = close + 1;
                return None;
            }
            *cursor += 1;
            while *cursor < close {
                let previous = *cursor;
                let Some(()) = Self::parse_node(context, cursor, close, node_id) else {
                    let end = context.tokens[close].end();
                    Self::mark_opaque(context, node_id, start..end);
                    *cursor = close;
                    return None;
                };
                if *cursor == previous {
                    Self::push_issue(
                        context,
                        context.tokens[*cursor].start()..context.tokens[*cursor].end(),
                    );
                    Self::mark_opaque(context, node_id, start..context.tokens[*cursor].end());
                    *cursor = close;
                    return None;
                }
                if *cursor < close && Recovery::is_punctuation(&context.tokens[*cursor], ",") {
                    *cursor += 1;
                } else if *cursor < close {
                    let error_start = context.tokens[*cursor].start();
                    let error_end = context.tokens[close].start();
                    Self::push_issue(context, error_start..error_end);
                    Self::mark_opaque(context, node_id, start..error_end);
                    *cursor = close;
                    return None;
                }
            }
            *cursor = close + 1;
        }

        if *cursor < limit
            && !needs_segment
            && Recovery::is_identifier(&context.tokens[*cursor], "as")
        {
            *cursor += 1;
            let Some(found_alias) = Self::take_identifier(context.tokens, cursor, limit) else {
                let error_bounds = context
                    .tokens
                    .get(*cursor)
                    .filter(|_| *cursor < limit)
                    .map(|token| token.start()..token.end())
                    .unwrap_or_else(|| {
                        let end = context
                            .tokens
                            .get(cursor.saturating_sub(1))
                            .map_or(start, Token::end);
                        end..end
                    });
                let opaque_end = context
                    .tokens
                    .get(limit.saturating_sub(1))
                    .map_or(error_bounds.end, Token::end)
                    .max(error_bounds.end);
                Self::push_issue(context, error_bounds);
                Self::mark_opaque(context, node_id, start..opaque_end);
                return None;
            };
            alias = Some(found_alias);
        }

        if alias.as_deref().is_some_and(|name| !is_use_alias(name)) || (glob && alias.is_some()) {
            let alias_bounds = context
                .tokens
                .get(cursor.saturating_sub(1))
                .map_or(start..start, |token| token.start()..token.end());
            Self::push_issue(context, alias_bounds);
            let end = context
                .tokens
                .get(cursor.saturating_sub(1))
                .map_or(start, Token::end);
            Self::mark_opaque(context, node_id, start..end);
            return None;
        }

        let valid_path = !path.is_empty() && !needs_segment;
        if has_group && (glob || (!path.is_empty() && !needs_segment)) {
            let end = context
                .tokens
                .get(limit.saturating_sub(1))
                .map_or(start, Token::end);
            Self::push_issue(context, start..end);
            Self::mark_opaque(context, node_id, start..end);
            return None;
        }
        if has_group && alias.is_some() {
            let end = context
                .tokens
                .get(limit.saturating_sub(1))
                .map_or(start, Token::end);
            Self::push_issue(context, start..end);
            Self::mark_opaque(context, node_id, start..end);
            return None;
        }
        if glob && path.is_empty() {
            Self::push_issue(context, start..token_start(context.tokens, *cursor));
            Self::mark_opaque(
                context,
                node_id,
                start..token_start(context.tokens, *cursor),
            );
            return None;
        }
        if !valid_path && !glob && !has_group && !needs_segment {
            let end = context.tokens.get(*cursor).map_or(start, Token::end);
            let issue_start = context.tokens.get(*cursor).map_or(start, Token::start);
            Self::push_issue(context, issue_start..end);
            Self::mark_opaque(context, node_id, start..end);
            return None;
        }
        if needs_segment && !has_group && !glob {
            let bounds = context
                .tokens
                .get(*cursor)
                .filter(|_| *cursor < limit)
                .map(|token| token.start()..token.end())
                .or(trailing_separator)
                .or_else(|| {
                    context
                        .tokens
                        .get(limit)
                        .map(|token| token.start()..token.end())
                })
                .unwrap_or(start..start);
            let opaque_end = context
                .tokens
                .get(cursor.saturating_sub(1))
                .map_or(bounds.end, Token::end)
                .max(bounds.end);
            Self::push_issue(context, bounds);
            Self::mark_opaque(context, node_id, start..opaque_end);
            return None;
        }

        let end = context
            .tokens
            .get(cursor.saturating_sub(1))
            .map_or(start, Token::end);
        AstFactory::update_ast_node(
            context.ast_nodes,
            node_id,
            SyntaxNodeKind::UseTree,
            start..end,
            attributes,
        );
        AstFactory::set_ast_node_data(
            context.ast_nodes,
            node_id,
            SyntaxNodeData::UseTree(UseTreeSyntaxData::new(path, alias, glob)),
        );
        Some(())
    }

    fn take_identifier(tokens: &[Token], cursor: &mut usize, limit: usize) -> Option<String> {
        if *cursor >= limit {
            return None;
        }
        let identifier = tokens[*cursor].kind().identifier()?.to_owned();
        *cursor += 1;
        Some(identifier)
    }

    fn push_issue(context: &mut ParseContext<'_>, bounds: Range<usize>) {
        context.issues.push(AstFactory::parse_issue(
            ParseIssueKind::UnexpectedToken,
            bounds,
            t!("syntax.use_tree_invalid"),
        ));
    }

    fn mark_opaque(context: &mut ParseContext<'_>, id: AstNodeId, bounds: Range<usize>) {
        let attributes = context
            .ast_nodes
            .get(id.index())
            .map_or_else(Vec::new, |node| node.attributes().to_vec());
        AstFactory::update_ast_node(
            context.ast_nodes,
            id,
            SyntaxNodeKind::Opaque,
            bounds,
            attributes,
        );
    }
}

fn token_start(tokens: &[Token], index: usize) -> usize {
    tokens.get(index).map_or(0, Token::start)
}

fn is_use_alias(identifier: &str) -> bool {
    identifier == "_" || is_plain_identifier(identifier)
}

fn is_plain_identifier(identifier: &str) -> bool {
    if let Some(raw_identifier) = identifier.strip_prefix("r#") {
        return !matches!(raw_identifier, "_" | "crate" | "self" | "Self" | "super");
    }
    if identifier == "_" {
        return false;
    }
    if matches!(identifier, "crate" | "self" | "Self" | "super") {
        return false;
    }
    !matches!(
        identifier,
        "abstract"
            | "as"
            | "become"
            | "box"
            | "break"
            | "const"
            | "continue"
            | "do"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "final"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "macro"
            | "match"
            | "mod"
            | "move"
            | "override"
            | "priv"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "static"
            | "struct"
            | "trait"
            | "true"
            | "type"
            | "typeof"
            | "union"
            | "unsafe"
            | "unsized"
            | "use"
            | "virtual"
            | "where"
            | "while"
            | "yield"
    )
}
