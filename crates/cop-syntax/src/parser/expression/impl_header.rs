use std::ops::Range;

use super::ImplHeaderInput;
use crate::{
    ast::{
        AstFactory, AstNodeId, ImplSyntaxData, ParseIssue, PathSegmentSyntax, PathSyntax,
        SyntaxNode, TypeSyntax,
    },
    parser::{
        path::PathParser,
        token_cursor::{AngleScanContext, CursorRange, TokenCursor},
        types::{TypeInput, TypeParser},
    },
    recovery::Recovery,
    SourceFile, Token,
};

pub(super) struct ImplHeaderParser;

impl ImplHeaderParser {
    pub(super) fn path_at(
        tokens: &[Token],
        start: usize,
        limit: usize,
    ) -> Option<(PathSyntax, usize)> {
        let mut index = start;
        let leading_colon = index < limit && Recovery::is_punctuation(&tokens[index], "::");
        if leading_colon {
            index += 1;
        }
        let token = tokens.get(index)?;
        let name = token.kind().identifier()?;
        let mut segments = vec![PathSegmentSyntax::new(
            name.to_owned(),
            Vec::new(),
            token.start()..token.end(),
        )];
        index += 1;
        while index + 1 < limit
            && Recovery::is_punctuation(&tokens[index], "::")
            && tokens[index + 1].kind().identifier().is_some()
        {
            let segment = &tokens[index + 1];
            if let Some(name) = segment.kind().identifier() {
                segments.push(PathSegmentSyntax::new(
                    name.to_owned(),
                    Vec::new(),
                    segment.start()..segment.end(),
                ));
            }
            index += 2;
        }
        let bounds = tokens[start].start()..tokens[index - 1].end();
        Some((
            PathSyntax::new(leading_colon, None, None, segments, bounds),
            index,
        ))
    }

    pub(super) fn collect(
        input: ImplHeaderInput<'_>,
        ast_nodes: &mut Vec<SyntaxNode>,
    ) -> ImplSyntaxData {
        let ImplHeaderInput {
            source,
            tokens,
            bounds,
            parent,
            issues,
        } = input;
        let start = bounds.start;
        let end = bounds.end;
        let generic_end = generic_parameter_end(source, tokens, start, end).unwrap_or(start);
        let trait_for = trait_for_index(tokens, start, end);
        let trait_impl = trait_for.is_some();
        let trait_path = trait_for.and_then(|index| parse_path(tokens, generic_end..index));
        let target_start = trait_for.map_or(generic_end, |index| index + 1);
        let target_bounds = target_start..target_end(tokens, target_start, end);
        let self_type = parse_type(tokens, target_bounds, parent, ast_nodes, issues);
        AstFactory::impl_syntax(self_type, trait_path, trait_impl)
    }
}

fn generic_parameter_end(
    source: &SourceFile,
    tokens: &[Token],
    start: usize,
    end: usize,
) -> Option<usize> {
    if !tokens
        .get(start)
        .is_some_and(|token| token.text(source) == "<")
    {
        return None;
    }
    let mut cursor = TokenCursor::from_token_range(tokens, start..end)?;
    cursor.split_angle_list().ok()?;
    Some(cursor.index())
}

fn trait_for_index(tokens: &[Token], start: usize, end: usize) -> Option<usize> {
    let mut search_start = start;
    while search_start < end {
        let cursor = TokenCursor::from_token_range(tokens, search_start..end)?;
        let index = cursor.find_top_level_identifier("for", AngleScanContext::Header)?;
        if !tokens
            .get(index + 1)
            .is_some_and(|next| Recovery::is_punctuation(next, "<"))
        {
            return Some(index);
        }
        search_start = index + 1;
    }
    None
}

fn target_end(tokens: &[Token], start: usize, end: usize) -> usize {
    TokenCursor::from_token_range(tokens, start..end)
        .and_then(|cursor| cursor.find_top_level_identifier("where", AngleScanContext::Header))
        .unwrap_or(end)
}

fn parse_path(tokens: &[Token], bounds: Range<usize>) -> Option<PathSyntax> {
    let mut cursor = TokenCursor::from_token_range(tokens, bounds)?;
    let path = PathParser::parse(&mut cursor).ok()?;
    cursor.is_empty().then_some(path)
}

fn parse_type(
    tokens: &[Token],
    bounds: Range<usize>,
    parent: AstNodeId,
    ast_nodes: &mut Vec<SyntaxNode>,
    issues: &mut Vec<ParseIssue>,
) -> Option<TypeSyntax> {
    let type_id = TypeParser::collect(TypeInput {
        tokens,
        bounds: CursorRange::from_token_range(tokens, bounds).expect("impl 类型范围有效"),
        parent,
        ast_nodes,
        issues,
    })?;
    ast_nodes
        .get(type_id.index())
        .and_then(SyntaxNode::type_syntax)
        .cloned()
}
