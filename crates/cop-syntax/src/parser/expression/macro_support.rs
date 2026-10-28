use std::ops::Range;

use cop_cfg::CfgCondition;
use cop_model::{LocalizedMessage, SYNTAX_MACRO_OPAQUE};

use crate::{
    ast::{
        AstFactory, AstNodeId, CoverageStatus, MacroInputStatus, MacroInvocation, ParseIssue,
        ParseIssueKind, PathSyntax, SyntaxNode, SyntaxNodeData, SyntaxNodeKind,
    },
    parser::{patterns::PatternParser, types::TypeParser},
    recovery::Recovery,
    Delimiter, LiteralKind, SourceFile, Token, TokenKind,
};

use super::{
    pratt::PrattParser,
    tree::{EmitContext, ExpressionTree},
    MacroInputRequest, PatternInput, SyntaxRequest,
};

#[derive(Clone, Copy)]
enum FragmentKind {
    Expression,
    Pattern,
}

struct MacroFragment {
    kind: FragmentKind,
    token_range: Range<usize>,
}

pub(super) struct MacroInputSupport;

impl MacroInputSupport {
    pub(super) fn emit_item(
        request: MacroInputRequest<'_>,
        invocation: &MacroInvocation,
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) -> bool {
        let mut context = EmitContext {
            source: request.source,
            tokens: request.tokens,
            edition: Some(request.edition),
            ast_nodes,
            issues,
        };
        let profile = Self::profile(
            invocation.path(),
            &invocation.input().bounds(),
            context.tokens,
            context.source,
        )
        .filter(|fragments| Self::validate_fragments(fragments, &context));
        let Some(fragments) = profile else {
            return false;
        };
        let updated = invocation.with_input_status(context.tokens, MacroInputStatus::Parsed);
        AstFactory::set_macro_invocation(context.ast_nodes, request.parent, updated);
        AstFactory::set_ast_node_coverage(
            context.ast_nodes,
            request.parent,
            CoverageStatus::Complete,
        );
        let input_id = AstFactory::push_ast_node(
            context.ast_nodes,
            Some(request.parent),
            SyntaxNodeKind::MacroInput,
            invocation.input().bounds(),
            Vec::new(),
        );
        Self::emit_fragments(fragments, input_id, &mut context);
        true
    }

    pub(super) fn emit_expression(
        path: &PathSyntax,
        bounds: Range<usize>,
        input: &Range<usize>,
        parent: AstNodeId,
        context: &mut EmitContext<'_>,
    ) {
        let profile = Self::profile(path, input, context.tokens, context.source)
            .filter(|fragments| Self::validate_fragments(fragments, context));
        let Some(fragments) = profile else {
            Self::emit_opaque(path, bounds, input, parent, context);
            return;
        };
        let invocation = AstFactory::macro_invocation(
            path.clone(),
            bounds.clone(),
            context.tokens,
            input.clone(),
            MacroInputStatus::Parsed,
        );
        let macro_id = AstFactory::push_data_node(
            context.ast_nodes,
            parent,
            SyntaxNodeKind::Macro,
            bounds,
            SyntaxNodeData::Macro(invocation),
        );
        AstFactory::set_ast_node_coverage(context.ast_nodes, macro_id, CoverageStatus::Complete);
        let input_id = AstFactory::push_ast_node(
            context.ast_nodes,
            Some(macro_id),
            SyntaxNodeKind::MacroInput,
            input.clone(),
            Vec::new(),
        );
        Self::emit_fragments(fragments, input_id, context);
        TypeParser::emit_path_argument_macros(
            path,
            macro_id,
            context.tokens,
            context.ast_nodes,
            context.issues,
        );
    }

    pub(super) fn emit_opaque(
        path: &PathSyntax,
        bounds: Range<usize>,
        input: &Range<usize>,
        parent: AstNodeId,
        context: &mut EmitContext<'_>,
    ) {
        let invocation = AstFactory::macro_invocation(
            path.clone(),
            bounds.clone(),
            context.tokens,
            input.clone(),
            MacroInputStatus::Partial,
        );
        let macro_id = AstFactory::push_data_node(
            context.ast_nodes,
            parent,
            SyntaxNodeKind::Macro,
            bounds,
            SyntaxNodeData::Macro(invocation),
        );
        let input_id = AstFactory::push_ast_node(
            context.ast_nodes,
            Some(macro_id),
            SyntaxNodeKind::MacroInput,
            input.clone(),
            Vec::new(),
        );
        AstFactory::push_ast_node(
            context.ast_nodes,
            Some(input_id),
            SyntaxNodeKind::Opaque,
            input.clone(),
            Vec::new(),
        );
        TypeParser::emit_path_argument_macros(
            path,
            macro_id,
            context.tokens,
            context.ast_nodes,
            context.issues,
        );
        context.issues.push(AstFactory::parse_issue(
            ParseIssueKind::UnsupportedSyntax,
            input.clone(),
            LocalizedMessage::new(SYNTAX_MACRO_OPAQUE),
        ));
    }

    fn profile(
        path: &PathSyntax,
        input: &Range<usize>,
        tokens: &[Token],
        source: &SourceFile,
    ) -> Option<Vec<MacroFragment>> {
        let name = path.segments().last()?.name();
        let range = Self::input_token_range(tokens, input)?;
        match name {
            "cfg" => Self::cfg_profile(tokens, range, source),
            "assert" => Self::assert_profile(tokens, range, 1),
            "assert_eq" | "assert_ne" => Self::assert_profile(tokens, range, 2),
            "matches" => Self::matches_profile(tokens, range),
            "format" | "panic" | "println" | "eprintln" => {
                Self::format_profile(tokens, range, false)
            }
            "write" | "writeln" => Self::format_profile(tokens, range, true),
            "error" => Self::error_profile(tokens, range),
            "vec" => Self::vec_profile(tokens, range),
            "env" => Self::env_profile(tokens, range),
            "json" => Self::json_profile(tokens, range),
            "static_loader" => Self::static_loader_profile(tokens, range),
            _ => None,
        }
    }

    fn cfg_profile(
        tokens: &[Token],
        range: Range<usize>,
        source: &SourceFile,
    ) -> Option<Vec<MacroFragment>> {
        let expression = Self::cfg_expression_text(tokens, range, source)?;
        CfgCondition::parse(expression).ok()?;
        Some(Vec::new())
    }

    fn cfg_expression_text<'a>(
        tokens: &[Token],
        range: Range<usize>,
        source: &'a SourceFile,
    ) -> Option<&'a str> {
        if range.is_empty() {
            return None;
        }
        let start = tokens.get(range.start)?.start();
        let end = tokens.get(range.end.checked_sub(1)?)?.end();
        source.slice(start, end)
    }

    fn static_loader_profile(tokens: &[Token], range: Range<usize>) -> Option<Vec<MacroFragment>> {
        let equal = Self::top_level_punctuation(tokens, range.clone(), "=")?;
        if equal != range.start + 2
            || !Self::is_identifier(&tokens[range.start], "static")
            || !matches!(tokens[range.start + 1].kind(), TokenKind::Identifier(_))
            || equal + 1 >= range.end
            || !matches!(
                tokens[equal + 1].kind(),
                TokenKind::OpenDelimiter(Delimiter::Brace)
            )
            || Recovery::matching_delimiter(tokens, equal + 1) != Some(range.end - 1)
        {
            return None;
        }
        let fields = Self::split_top_level(tokens, equal + 2..range.end - 1, ",");
        let mut fragments = Vec::new();
        for field in fields {
            let colon = Self::top_level_punctuation(tokens, field.clone(), ":")?;
            if colon != field.start + 1
                || !matches!(tokens[field.start].kind(), TokenKind::Identifier(_))
                || colon + 1 >= field.end
            {
                return None;
            }
            fragments.push(Self::expression_fragment(colon + 1..field.end));
        }
        Some(fragments)
    }

    fn is_identifier(token: &Token, expected: &str) -> bool {
        matches!(
            token.kind(),
            TokenKind::Identifier(name)
                | TokenKind::EditionKeyword(name)
                | TokenKind::ReservedKeyword(name)
                | TokenKind::WeakKeyword(name)
                if name == expected
        )
    }

    fn assert_profile(
        tokens: &[Token],
        range: Range<usize>,
        required: usize,
    ) -> Option<Vec<MacroFragment>> {
        let arguments = Self::split_top_level(tokens, range, ",");
        if arguments.len() < required {
            return None;
        }
        let mut fragments = arguments
            .iter()
            .take(required)
            .map(|argument| Self::expression_fragment(argument.clone()))
            .collect::<Vec<_>>();
        if arguments.len() > required {
            fragments.extend(Self::format_tail(tokens, &arguments[required..])?);
        }
        Some(fragments)
    }

    fn matches_profile(tokens: &[Token], range: Range<usize>) -> Option<Vec<MacroFragment>> {
        let arguments = Self::split_top_level(tokens, range, ",");
        if arguments.len() != 2 {
            return None;
        }
        let guard = Self::top_level_identifier(tokens, arguments[1].clone(), "if");
        let pattern_end = guard.unwrap_or(arguments[1].end);
        if arguments[1].start == pattern_end {
            return None;
        }
        let mut fragments = vec![
            Self::expression_fragment(arguments[0].clone()),
            MacroFragment {
                kind: FragmentKind::Pattern,
                token_range: arguments[1].start..pattern_end,
            },
        ];
        if let Some(guard_index) = guard {
            if guard_index + 1 >= arguments[1].end {
                return None;
            }
            fragments.push(Self::expression_fragment(guard_index + 1..arguments[1].end));
        }
        Some(fragments)
    }

    fn format_profile(
        tokens: &[Token],
        range: Range<usize>,
        has_target: bool,
    ) -> Option<Vec<MacroFragment>> {
        let mut arguments = Self::split_top_level(tokens, range.clone(), ",");
        if has_target {
            if arguments.len() < 2 {
                return None;
            }
        } else if arguments.is_empty() {
            return Some(Vec::new());
        }
        let mut fragments = Vec::new();
        if has_target {
            fragments.push(Self::expression_fragment(arguments.remove(0)));
        }
        fragments.extend(Self::format_tail(tokens, &arguments)?);
        Some(fragments)
    }

    fn error_profile(tokens: &[Token], range: Range<usize>) -> Option<Vec<MacroFragment>> {
        let mut arguments = Self::split_top_level(tokens, range.clone(), ",");
        if arguments.first().is_some_and(|argument| {
            Self::top_level_punctuation(tokens, argument.clone(), ":").is_some()
        }) {
            let target = arguments.first()?.clone();
            let colon = Self::top_level_punctuation(tokens, target.clone(), ":")?;
            if colon + 1 >= target.end {
                return None;
            }
            let target_value = colon + 1..target.end;
            arguments.remove(0);
            if arguments.is_empty() {
                return None;
            }
            let mut fragments = vec![Self::expression_fragment(target_value)];
            fragments.extend(Self::format_tail(tokens, &arguments)?);
            return Some(fragments);
        }
        Self::format_profile(tokens, range, false)
    }

    fn vec_profile(tokens: &[Token], range: Range<usize>) -> Option<Vec<MacroFragment>> {
        let arguments = Self::split_top_level(tokens, range.clone(), ",");
        if arguments.len() == 1 {
            if let Some(separator) = Self::top_level_punctuation(tokens, arguments[0].clone(), ";")
            {
                if separator == arguments[0].start || separator + 1 >= arguments[0].end {
                    return None;
                }
                return Some(vec![
                    Self::expression_fragment(arguments[0].start..separator),
                    Self::expression_fragment(separator + 1..arguments[0].end),
                ]);
            }
        }
        if arguments.is_empty() {
            return Some(Vec::new());
        }
        Some(
            arguments
                .into_iter()
                .map(Self::expression_fragment)
                .collect(),
        )
    }

    fn env_profile(tokens: &[Token], range: Range<usize>) -> Option<Vec<MacroFragment>> {
        let arguments = Self::split_top_level(tokens, range, ",");
        if !(1..=2).contains(&arguments.len())
            || arguments.iter().any(|argument| {
                argument.end != argument.start + 1
                    || !matches!(
                        tokens[argument.start].kind(),
                        TokenKind::Literal(LiteralKind::String)
                    )
            })
        {
            return None;
        }
        Some(Vec::new())
    }

    fn json_profile(tokens: &[Token], range: Range<usize>) -> Option<Vec<MacroFragment>> {
        let mut fragments = Vec::new();
        Self::collect_json_value(tokens, range, &mut fragments).then_some(fragments)
    }

    fn collect_json_value(
        tokens: &[Token],
        range: Range<usize>,
        fragments: &mut Vec<MacroFragment>,
    ) -> bool {
        if range.is_empty() {
            return false;
        }
        if range.end == range.start + 1 {
            let token = &tokens[range.start];
            if matches!(token.kind(), TokenKind::Identifier(value) if value == "null") {
                return true;
            }
            if matches!(token.kind(), TokenKind::Literal(_))
                || Self::is_identifier(token, "true")
                || Self::is_identifier(token, "false")
            {
                fragments.push(Self::expression_fragment(range));
                return true;
            }
        }
        if matches!(tokens[range.start].kind(), TokenKind::OpenDelimiter(_))
            && Recovery::matching_delimiter(tokens, range.start) == Some(range.end - 1)
        {
            match tokens[range.start].kind() {
                TokenKind::OpenDelimiter(Delimiter::Brace) => {
                    let fields = Self::split_top_level(tokens, range.start + 1..range.end - 1, ",");
                    for field in fields {
                        let Some(colon) = Self::top_level_punctuation(tokens, field.clone(), ":")
                        else {
                            return false;
                        };
                        if colon == field.start || colon + 1 >= field.end {
                            return false;
                        }
                        if !Self::collect_json_value(tokens, colon + 1..field.end, fragments) {
                            return false;
                        }
                    }
                    return true;
                }
                TokenKind::OpenDelimiter(Delimiter::Bracket) => {
                    let values = Self::split_top_level(tokens, range.start + 1..range.end - 1, ",");
                    return values
                        .into_iter()
                        .all(|value| Self::collect_json_value(tokens, value, fragments));
                }
                TokenKind::OpenDelimiter(Delimiter::Parenthesis) => {}
                _ => return false,
            }
        }
        fragments.push(Self::expression_fragment(range));
        true
    }

    fn format_tail(tokens: &[Token], arguments: &[Range<usize>]) -> Option<Vec<MacroFragment>> {
        let format = arguments.first()?;
        if format.end != format.start + 1
            || !matches!(
                tokens[format.start].kind(),
                TokenKind::Literal(LiteralKind::String)
            )
        {
            return None;
        }
        arguments
            .iter()
            .skip(1)
            .map(|argument| Self::named_argument(tokens, argument.clone()))
            .collect()
    }

    fn named_argument(tokens: &[Token], range: Range<usize>) -> Option<MacroFragment> {
        if let Some(equal) = Self::top_level_punctuation(tokens, range.clone(), "=") {
            if equal == range.start || equal + 1 >= range.end {
                return None;
            }
            return Some(Self::expression_fragment(equal + 1..range.end));
        }
        Some(Self::expression_fragment(range))
    }

    fn expression_fragment(token_range: Range<usize>) -> MacroFragment {
        MacroFragment {
            kind: FragmentKind::Expression,
            token_range,
        }
    }

    fn validate_fragments(fragments: &[MacroFragment], context: &EmitContext<'_>) -> bool {
        fragments.iter().all(|fragment| match fragment.kind {
            FragmentKind::Expression => PrattParser::parse_complete(
                context.source,
                context.tokens,
                fragment.token_range.clone(),
            )
            .is_ok(),
            FragmentKind::Pattern => {
                PatternParser::parse_binding_range(context.tokens, fragment.token_range.clone())
                    .is_ok()
            }
        })
    }

    fn emit_fragments(
        fragments: Vec<MacroFragment>,
        parent: AstNodeId,
        context: &mut EmitContext<'_>,
    ) {
        for fragment in fragments {
            match fragment.kind {
                FragmentKind::Expression => {
                    let request = SyntaxRequest {
                        source: context.source,
                        tokens: context.tokens,
                        bounds: fragment.token_range,
                        parent,
                        expression_tree: true,
                        edition: context.edition,
                    };
                    ExpressionTree::collect_expression(request, context.ast_nodes, context.issues);
                }
                FragmentKind::Pattern => {
                    let bounds = Self::byte_bounds(context.tokens, fragment.token_range.clone());
                    ExpressionTree::push_pattern(
                        PatternInput {
                            source: context.source,
                            tokens: context.tokens,
                            parent,
                            start: bounds.start,
                            end: bounds.end,
                            bind_bare_identifiers: true,
                            issues: context.issues,
                        },
                        context.ast_nodes,
                    );
                }
            }
        }
    }

    fn input_token_range(tokens: &[Token], input: &Range<usize>) -> Option<Range<usize>> {
        let start = tokens
            .iter()
            .position(|token| token.start() == input.start)?;
        let end = tokens
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, token)| token.end() == input.end)
            .map(|(index, _)| index + 1)?;
        (start + 1 < end).then_some(start + 1..end - 1)
    }

    fn split_top_level(
        tokens: &[Token],
        range: Range<usize>,
        punctuation: &str,
    ) -> Vec<Range<usize>> {
        let mut segments = Vec::new();
        let mut segment_start = range.start;
        let mut index = range.start;
        while index < range.end {
            if matches!(tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                if let Some(close) =
                    Recovery::matching_delimiter(tokens, index).filter(|close| *close < range.end)
                {
                    index = close + 1;
                    continue;
                }
            }
            if Recovery::is_punctuation(&tokens[index], punctuation) {
                if segment_start < index {
                    segments.push(segment_start..index);
                }
                segment_start = index + 1;
            }
            index += 1;
        }
        if segment_start < range.end {
            segments.push(segment_start..range.end);
        }
        segments
    }

    fn top_level_punctuation(
        tokens: &[Token],
        range: Range<usize>,
        punctuation: &str,
    ) -> Option<usize> {
        let mut index = range.start;
        while index < range.end {
            if matches!(tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                if let Some(close) =
                    Recovery::matching_delimiter(tokens, index).filter(|close| *close < range.end)
                {
                    index = close + 1;
                    continue;
                }
            }
            if Recovery::is_punctuation(&tokens[index], punctuation) {
                return Some(index);
            }
            index += 1;
        }
        None
    }

    fn top_level_identifier(
        tokens: &[Token],
        range: Range<usize>,
        identifier: &str,
    ) -> Option<usize> {
        let mut index = range.start;
        while index < range.end {
            if matches!(tokens[index].kind(), TokenKind::OpenDelimiter(_)) {
                if let Some(close) =
                    Recovery::matching_delimiter(tokens, index).filter(|close| *close < range.end)
                {
                    index = close + 1;
                    continue;
                }
            }
            if matches!(tokens[index].kind(), TokenKind::Identifier(value) if value == identifier) {
                return Some(index);
            }
            index += 1;
        }
        None
    }

    fn byte_bounds(tokens: &[Token], range: Range<usize>) -> Range<usize> {
        let start = tokens.get(range.start).map_or(0, Token::start);
        let end = range
            .end
            .checked_sub(1)
            .and_then(|index| tokens.get(index))
            .map_or(start, Token::end);
        start..end
    }
}
