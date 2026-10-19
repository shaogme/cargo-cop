mod atoms;
mod closure;
mod control;
mod edition;
mod impl_header;
mod macro_support;
mod pratt;
mod tree;

use std::ops::Range;

use cop_model::{LocalizedMessage, SYNTAX_MACRO_OPAQUE};

use crate::{
    ast::{
        AstFactory, AstNodeId, ExpressionSyntax, ExpressionSyntaxKind, ImplSyntaxData,
        MacroDefinition, MacroInputStatus, MacroInvocation, Parameter, ParseIssue, ParseIssueKind,
        SyntaxNode, SyntaxNodeData, SyntaxNodeKind,
    },
    recovery::Recovery,
    RustEdition, SourceFile, Token, TokenKind,
};

use super::{
    attributes::AttributeParser,
    token_cursor::{SyntaxError, TokenCursor},
};

use self::{impl_header::ImplHeaderParser, pratt::PrattParser, tree::ExpressionTree};

pub(super) struct SyntaxCollector;

pub(super) struct SyntaxRequest<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) bounds: Range<usize>,
    pub(super) parent: AstNodeId,
    pub(super) expression_tree: bool,
    pub(super) edition: Option<RustEdition>,
}

pub(super) struct MacroInputRequest<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) parent: AstNodeId,
    pub(super) edition: RustEdition,
}

pub(super) struct ImplHeaderInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) bounds: Range<usize>,
    pub(super) parent: AstNodeId,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

pub(super) struct PatternInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) parent: AstNodeId,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) bind_bare_identifiers: bool,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

pub(super) struct ParameterPatternInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) parent: AstNodeId,
    pub(super) parameter: &'a Parameter,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

impl SyntaxCollector {
    pub(super) fn collect_item_macro_input(
        request: MacroInputRequest<'_>,
        invocation: &MacroInvocation,
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) -> bool {
        macro_support::MacroInputSupport::emit_item(request, invocation, ast_nodes, issues)
    }

    pub(super) fn top_level_token(
        tokens: &[Token],
        start: usize,
        end: usize,
        punctuation: &str,
    ) -> Option<usize> {
        ExpressionTree::top_level_token(tokens, start, end, punctuation)
    }

    pub(super) fn collect(
        request: SyntaxRequest<'_>,
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) {
        if request.expression_tree {
            ExpressionTree::collect_expression(request, ast_nodes, issues);
        } else {
            Self::collect_macros(request, ast_nodes, issues);
        }
    }

    pub(super) fn collect_impl_header(
        input: ImplHeaderInput<'_>,
        ast_nodes: &mut Vec<SyntaxNode>,
    ) -> ImplSyntaxData {
        ImplHeaderParser::collect(input, ast_nodes)
    }

    pub(super) fn collect_expression(
        request: SyntaxRequest<'_>,
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) -> Option<AstNodeId> {
        ExpressionTree::collect_expression(request, ast_nodes, issues)
    }

    pub(super) fn parse_expression_prefix(
        source: &SourceFile,
        cursor: &mut TokenCursor<'_>,
    ) -> Result<ExpressionSyntax, SyntaxError> {
        PrattParser::parse_prefix(source, cursor)
    }

    pub(super) fn expression_is_block_like(expression: &ExpressionSyntax) -> bool {
        matches!(
            expression.kind(),
            ExpressionSyntaxKind::Block { .. }
                | ExpressionSyntaxKind::LabelBlock { .. }
                | ExpressionSyntaxKind::If { .. }
                | ExpressionSyntaxKind::Match { .. }
                | ExpressionSyntaxKind::Loop { .. }
                | ExpressionSyntaxKind::While { .. }
                | ExpressionSyntaxKind::For { .. }
                | ExpressionSyntaxKind::Unsupported(_)
        )
    }

    pub(super) fn collect_parameter_pattern(
        input: ParameterPatternInput<'_>,
        ast_nodes: &mut Vec<SyntaxNode>,
    ) {
        ExpressionTree::collect_parameter_pattern(input, ast_nodes)
    }

    pub(super) fn push_pattern(
        input: PatternInput<'_>,
        ast_nodes: &mut Vec<SyntaxNode>,
    ) -> Option<AstNodeId> {
        ExpressionTree::push_pattern(input, ast_nodes)
    }

    pub(super) fn macro_item(tokens: &[Token], start: usize) -> Option<(MacroInvocation, usize)> {
        let (path, path_end) = ImplHeaderParser::path_at(tokens, start, tokens.len())?;
        if !tokens
            .get(path_end)
            .is_some_and(|token| Recovery::is_punctuation(token, "!"))
        {
            return None;
        }
        let (group_index, macro_name) = if path
            .segments()
            .last()
            .is_some_and(|name| name.name() == "macro_rules")
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
        let opaque_bounds = tokens[group_index].start()..tokens[group_end].end();
        let path_bounds = path.bounds();
        let invocation = AstFactory::macro_invocation(
            path,
            path_bounds.start..end_offset,
            tokens,
            opaque_bounds,
            MacroInputStatus::Partial,
        );
        Some((invocation, end))
    }

    pub(super) fn macro_definition(
        tokens: &[Token],
        start: usize,
    ) -> Option<(MacroDefinition, usize)> {
        let (path, path_end) = ImplHeaderParser::path_at(tokens, start, tokens.len())?;
        if !path
            .segments()
            .last()
            .is_some_and(|name| name.name() == "macro_rules")
            || !tokens
                .get(path_end)
                .is_some_and(|token| Recovery::is_punctuation(token, "!"))
            || !matches!(tokens.get(path_end + 1)?.kind(), TokenKind::Identifier(_))
        {
            return None;
        }
        let group_end = Self::group_end(tokens, path_end + 2, tokens.len())?;
        let mut next_index = group_end + 1;
        let mut end = tokens[group_end].end();
        if tokens
            .get(next_index)
            .is_some_and(|token| Recovery::is_punctuation(token, ";"))
        {
            end = tokens[next_index].end();
            next_index += 1;
        }
        Some((
            AstFactory::macro_definition(path.bounds().start..end),
            next_index,
        ))
    }

    pub(super) fn macro_definition_end(tokens: &[Token], start: usize) -> Option<usize> {
        Self::macro_definition(tokens, start).map(|(_, next_index)| next_index)
    }

    pub(super) fn macro_item_end(tokens: &[Token], start: usize) -> Option<usize> {
        let (path, path_end) = ImplHeaderParser::path_at(tokens, start, tokens.len())?;
        if !tokens
            .get(path_end)
            .is_some_and(|token| Recovery::is_punctuation(token, "!"))
        {
            return None;
        }
        let group_index = if path
            .segments()
            .last()
            .is_some_and(|name| name.name() == "macro_rules")
        {
            path_end + 2
        } else {
            path_end + 1
        };
        Self::group_end(tokens, group_index, tokens.len())
    }

    fn collect_macros(
        request: SyntaxRequest<'_>,
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) {
        let SyntaxRequest {
            source,
            tokens,
            bounds,
            parent,
            ..
        } = request;
        let mut index = bounds.start;
        while index < bounds.end {
            if matches!(tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                index += 1;
                continue;
            }
            if Recovery::is_punctuation(&tokens[index], "#") {
                let mut attribute_cursor = index;
                if !AttributeParser::parse_attributes(source, tokens, &mut attribute_cursor, issues)
                    .is_empty()
                {
                    index = attribute_cursor;
                    continue;
                }
            }
            if let Some((path, next)) = ImplHeaderParser::path_at(tokens, index, bounds.end) {
                let is_macro = next < bounds.end && Recovery::is_punctuation(&tokens[next], "!");
                if is_macro {
                    if let Some(group_end) = Self::group_end(tokens, next + 1, bounds.end) {
                        let opaque = tokens[next + 1].start()..tokens[group_end].end();
                        let path_bounds = path.bounds();
                        let invocation = AstFactory::macro_invocation(
                            path,
                            path_bounds.start..tokens[group_end].end(),
                            tokens,
                            opaque.clone(),
                            MacroInputStatus::Partial,
                        );
                        let macro_id = AstFactory::push_data_node(
                            ast_nodes,
                            parent,
                            SyntaxNodeKind::Macro,
                            invocation.start()..invocation.end(),
                            SyntaxNodeData::Macro(invocation),
                        );
                        let input_id = AstFactory::push_ast_node(
                            ast_nodes,
                            Some(macro_id),
                            SyntaxNodeKind::MacroInput,
                            opaque.clone(),
                            Vec::new(),
                        );
                        AstFactory::push_ast_node(
                            ast_nodes,
                            Some(input_id),
                            SyntaxNodeKind::Opaque,
                            opaque.clone(),
                            Vec::new(),
                        );
                        issues.push(AstFactory::parse_issue(
                            ParseIssueKind::UnsupportedSyntax,
                            opaque,
                            LocalizedMessage::new(SYNTAX_MACRO_OPAQUE),
                        ));
                        index = group_end + 1;
                        continue;
                    }
                }
                index = next;
                continue;
            }
            index += 1;
        }
    }

    fn group_end(tokens: &[Token], index: usize, limit: usize) -> Option<usize> {
        if !matches!(tokens.get(index)?.kind(), TokenKind::OpenDelimiter(_)) {
            return None;
        }
        Recovery::matching_delimiter(tokens, index).filter(|close| *close < limit)
    }
}
