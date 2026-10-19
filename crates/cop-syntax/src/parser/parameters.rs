use std::ops::Range;

use cop_model::{LocalizedMessage, SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN};

use crate::{
    ast::{
        AstFactory, AstNodeId, Parameter, ParseIssue, ParseIssueKind, SyntaxNode, SyntaxNodeData,
        SyntaxNodeKind,
    },
    recovery::Recovery,
    Delimiter, SourceFile, Token, TokenKind,
};

use super::{
    attributes::AttributeParser,
    expression::{ParameterPatternInput, SyntaxCollector, SyntaxRequest},
    token_cursor::TokenCursor,
    types::{TypeInput, TypeParser},
};

pub(super) struct ParameterParser;

pub(super) struct ParameterInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) parent: AstNodeId,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) allow_variadic: bool,
    pub(super) ast_nodes: &'a mut Vec<SyntaxNode>,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

impl ParameterParser {
    pub(super) fn recover_unnamed_function(tokens: &[Token], cursor: &mut usize) {
        let Some(parameter_open) = Recovery::function_parameter_open(tokens, *cursor) else {
            Self::recover_item(tokens, cursor);
            return;
        };
        let Some(parameter_close) = Recovery::matching_delimiter(tokens, parameter_open) else {
            *cursor = tokens.len();
            return;
        };
        if let Some(body_open) = Recovery::body_open(tokens, parameter_close + 1) {
            if let Some(body_close) = Recovery::matching_delimiter(tokens, body_open) {
                *cursor = body_close + 1;
            } else {
                *cursor = tokens.len();
            }
        } else {
            *cursor = parameter_close + 1;
            Self::skip_signature_tail(tokens, cursor);
        }
    }

    fn recover_item(tokens: &[Token], cursor: &mut usize) {
        let start = *cursor;
        while *cursor < tokens.len() {
            match tokens[*cursor].kind() {
                TokenKind::OpenDelimiter(_) => {
                    if let Some(close_index) = Recovery::matching_delimiter(tokens, *cursor) {
                        *cursor = close_index + 1;
                    } else {
                        *cursor += 1;
                    }
                }
                TokenKind::Punctuation(value) if value == ";" => {
                    *cursor += 1;
                    return;
                }
                TokenKind::CloseDelimiter(Delimiter::Brace) => return,
                _ => *cursor += 1,
            }
        }
        if *cursor == start {
            *cursor = (*cursor + 1).min(tokens.len());
        }
    }

    fn skip_signature_tail(tokens: &[Token], cursor: &mut usize) {
        while *cursor < tokens.len() {
            if Recovery::is_punctuation(&tokens[*cursor], ";") {
                *cursor += 1;
                return;
            }
            if matches!(
                tokens[*cursor].kind(),
                TokenKind::OpenDelimiter(Delimiter::Brace)
            ) {
                if let Some(close_index) = Recovery::matching_delimiter(tokens, *cursor) {
                    *cursor = close_index + 1;
                    return;
                }
            }
            *cursor += 1;
        }
    }

    pub(super) fn parse(input: ParameterInput<'_>) -> Vec<Parameter> {
        let ParameterInput {
            source,
            tokens,
            parent,
            start,
            end,
            allow_variadic,
            ast_nodes,
            issues,
        } = input;
        let ranges = Recovery::split_top_level(tokens, start, end, ",")
            .into_iter()
            .filter(|range| range.start < range.end)
            .collect::<Vec<_>>();
        let last_parameter = ranges.len().saturating_sub(1);
        ranges
            .into_iter()
            .enumerate()
            .filter_map(|(index, range)| {
                if Self::is_variadic_marker(tokens, &range) {
                    if allow_variadic && index == last_parameter {
                        return None;
                    }
                    if allow_variadic {
                        let token = &tokens[range.start];
                        issues.push(AstFactory::parse_issue(
                            ParseIssueKind::UnexpectedToken,
                            token.start()..token.end(),
                            LocalizedMessage::new(SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN),
                        ));
                        return None;
                    }
                }
                let receiver = Recovery::receiver_parameter(tokens, &range);
                let mut cursor = range.start;
                let attributes =
                    AttributeParser::parse_attributes(source, tokens, &mut cursor, issues);
                let bounds = Recovery::token_bounds(tokens, range.clone());
                let parameter = AstFactory::parameter(attributes, bounds.clone(), receiver);
                let parameter_id = AstFactory::push_ast_node(
                    ast_nodes,
                    Some(parent),
                    SyntaxNodeKind::Parameter,
                    bounds.clone(),
                    parameter.attributes().to_vec(),
                );
                AstFactory::set_ast_node_data(
                    ast_nodes,
                    parameter_id,
                    SyntaxNodeData::Parameter(parameter.is_receiver()),
                );
                SyntaxCollector::collect_parameter_pattern(
                    ParameterPatternInput {
                        source,
                        tokens,
                        parent: parameter_id,
                        parameter: &parameter,
                        issues,
                    },
                    ast_nodes,
                );
                let parameter_cursor =
                    TokenCursor::new(tokens, cursor..range.end).expect("参数 token 范围有效");
                if let Some(colon) = parameter_cursor.find_top_level_punctuation(":") {
                    if colon + 1 < range.end {
                        TypeParser::collect(TypeInput {
                            tokens,
                            bounds: colon + 1..range.end,
                            parent: parameter_id,
                            ast_nodes,
                            issues,
                        });
                    }
                }
                SyntaxCollector::collect(
                    SyntaxRequest {
                        source,
                        tokens,
                        bounds: range,
                        parent: parameter_id,
                        expression_tree: false,
                        edition: None,
                    },
                    ast_nodes,
                    issues,
                );
                Some(parameter)
            })
            .collect()
    }

    fn is_variadic_marker(tokens: &[Token], range: &Range<usize>) -> bool {
        range.end == range.start + 1
            && tokens
                .get(range.start)
                .is_some_and(|token| Recovery::is_punctuation(token, "..."))
    }
}
