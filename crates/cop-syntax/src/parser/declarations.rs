use std::{borrow::Cow, ops::Range};

use cop_i18n::{t, RenderError};

use super::{
    attributes::AttributeParser,
    expression::{SyntaxCollector, SyntaxRequest},
    token_cursor::{AngleScanContext, CursorRange, TokenCursor},
    types::{TypeInput, TypeParser},
    where_clause::{WhereClauseInput, WhereClauseParser},
};
use crate::{
    ast::{
        AstFactory, AstNodeId, Attribute, NamedDeclaration, ParseIssue, ParseIssueKind, SyntaxNode,
        SyntaxNodeData, SyntaxNodeKind,
    },
    recovery::Recovery,
    Delimiter, MacroProfileBinding, SourceFile, Token, TokenKind,
};

pub(super) struct DeclarationParser;

pub(super) struct DeclarationInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) cursor: &'a mut usize,
    pub(super) parent: AstNodeId,
    pub(super) ast_nodes: &'a mut Vec<SyntaxNode>,
    pub(super) issues: &'a mut Vec<ParseIssue>,
    pub(super) macro_profiles: &'a [MacroProfileBinding],
    pub(super) defer_opaque_macro_diagnostics: bool,
}

struct DeclarationContext<'a> {
    source: &'a SourceFile,
    tokens: &'a [Token],
    ast_nodes: &'a mut Vec<SyntaxNode>,
    issues: &'a mut Vec<ParseIssue>,
    macro_profiles: &'a [MacroProfileBinding],
    defer_opaque_macro_diagnostics: bool,
}

impl DeclarationParser {
    pub(super) fn parse(input: DeclarationInput<'_>) -> NamedDeclaration {
        let DeclarationInput {
            source,
            tokens,
            cursor,
            parent,
            ast_nodes,
            issues,
            macro_profiles,
            defer_opaque_macro_diagnostics,
        } = input;
        let mut context = DeclarationContext {
            source,
            tokens,
            ast_nodes,
            issues,
            macro_profiles,
            defer_opaque_macro_diagnostics,
        };
        let keyword = context
            .tokens
            .get(*cursor)
            .and_then(|token| token.kind().identifier().map(str::to_owned))
            .unwrap_or_default();
        *cursor += 1;
        if keyword == "static" {
            Self::consume_identifier(context.tokens, cursor, "mut");
        }
        let name = Self::take_identifier(context.tokens, cursor);
        if name.is_none() {
            Self::push_issue(
                context.issues,
                ParseIssueKind::UnexpectedToken,
                Self::current_start(context.source, context.tokens, *cursor)
                    ..Self::current_end(context.source, context.tokens, *cursor),
                t!("syntax.named_item_expect_name"),
            );
        }
        let has_body = matches!(keyword.as_str(), "struct" | "enum" | "union");
        let tail = Self::skip_declaration_tail(&mut context, cursor, has_body);
        Self::collect_where_clause(&mut context, &keyword, tail.clone(), parent);
        if keyword == "enum" {
            Self::parse_enum_variants(&mut context, tail, parent);
        } else if matches!(keyword.as_str(), "struct" | "union") {
            Self::parse_struct_fields(&mut context, &keyword, tail, parent);
        } else {
            Self::collect_declared_type(&mut context, &keyword, tail.clone(), parent);
            SyntaxCollector::collect(
                SyntaxRequest {
                    source: context.source,
                    tokens: context.tokens,
                    bounds: tail,
                    parent,
                    expression_tree: false,
                    edition: None,
                    macro_profiles: context.macro_profiles,
                    defer_opaque_macro_diagnostics: context.defer_opaque_macro_diagnostics,
                },
                context.ast_nodes,
                context.issues,
            );
        }
        AstFactory::named_declaration(name)
    }

    fn parse_struct_fields(
        context: &mut DeclarationContext<'_>,
        keyword: &str,
        tail: Range<usize>,
        parent: AstNodeId,
    ) {
        let Some((open, close)) = Self::declaration_field_group(context.tokens, keyword, &tail)
        else {
            SyntaxCollector::collect(
                SyntaxRequest {
                    source: context.source,
                    tokens: context.tokens,
                    bounds: tail,
                    parent,
                    expression_tree: false,
                    edition: None,
                    macro_profiles: context.macro_profiles,
                    defer_opaque_macro_diagnostics: context.defer_opaque_macro_diagnostics,
                },
                context.ast_nodes,
                context.issues,
            );
            return;
        };
        SyntaxCollector::collect(
            SyntaxRequest {
                source: context.source,
                tokens: context.tokens,
                bounds: tail.start..open,
                parent,
                expression_tree: false,
                edition: None,
                macro_profiles: context.macro_profiles,
                defer_opaque_macro_diagnostics: context.defer_opaque_macro_diagnostics,
            },
            context.ast_nodes,
            context.issues,
        );
        Self::parse_fields(context, open + 1, close, parent);
    }

    fn parse_enum_variants(
        context: &mut DeclarationContext<'_>,
        tail: Range<usize>,
        parent: AstNodeId,
    ) {
        let Some(open) = tail.clone().find(|index| {
            matches!(
                context.tokens.get(*index).map(Token::kind),
                Some(TokenKind::OpenDelimiter(Delimiter::Brace))
            )
        }) else {
            SyntaxCollector::collect(
                SyntaxRequest {
                    source: context.source,
                    tokens: context.tokens,
                    bounds: tail,
                    parent,
                    expression_tree: false,
                    edition: None,
                    macro_profiles: context.macro_profiles,
                    defer_opaque_macro_diagnostics: context.defer_opaque_macro_diagnostics,
                },
                context.ast_nodes,
                context.issues,
            );
            return;
        };
        SyntaxCollector::collect(
            SyntaxRequest {
                source: context.source,
                tokens: context.tokens,
                bounds: tail.start..open,
                parent,
                expression_tree: false,
                edition: None,
                macro_profiles: context.macro_profiles,
                defer_opaque_macro_diagnostics: context.defer_opaque_macro_diagnostics,
            },
            context.ast_nodes,
            context.issues,
        );
        let Some(close) = Recovery::matching_delimiter(context.tokens, open) else {
            return;
        };
        let Some(variants) = Recovery::split_top_level(
            context.tokens,
            open + 1,
            close,
            ",",
            AngleScanContext::Expression,
        ) else {
            Self::push_issue(
                context.issues,
                ParseIssueKind::UnexpectedToken,
                context.tokens[open].start()..context.tokens[close].end(),
                t!("syntax.item_unsupported"),
            );
            return;
        };
        variants
            .into_iter()
            .filter(|range| range.start < range.end)
            .for_each(|range| {
                let mut cursor = range.start;
                let attributes = AttributeParser::parse_attributes(
                    context.source,
                    context.tokens,
                    &mut cursor,
                    context.issues,
                );
                let start = attributes
                    .first()
                    .map_or_else(|| context.tokens[range.start].start(), Attribute::start);
                let name = Self::peek_identifier(context.tokens, cursor);
                let end = context.tokens[range.end - 1].end();
                let variant_id = AstFactory::push_ast_node(
                    context.ast_nodes,
                    Some(parent),
                    SyntaxNodeKind::EnumVariant,
                    start..end,
                    attributes.clone(),
                );
                let mut field_group = None;
                for index in cursor..range.end {
                    if Recovery::is_punctuation(&context.tokens[index], "=") {
                        break;
                    }
                    if matches!(
                        context.tokens[index].kind(),
                        TokenKind::OpenDelimiter(Delimiter::Brace | Delimiter::Parenthesis)
                    ) {
                        if let Some(field_close) =
                            Recovery::matching_delimiter(context.tokens, index)
                        {
                            if field_close < range.end {
                                field_group = Some(index..field_close + 1);
                                Self::parse_fields(context, index + 1, field_close, variant_id);
                                break;
                            }
                        }
                    }
                }
                let syntax_start = field_group.as_ref().map_or(cursor, |group| group.start);
                let syntax_end = field_group.as_ref().map_or(range.end, |group| group.start);
                SyntaxCollector::collect(
                    SyntaxRequest {
                        source: context.source,
                        tokens: context.tokens,
                        bounds: syntax_start..syntax_end,
                        parent: variant_id,
                        expression_tree: false,
                        edition: None,
                        macro_profiles: context.macro_profiles,
                        defer_opaque_macro_diagnostics: context.defer_opaque_macro_diagnostics,
                    },
                    context.ast_nodes,
                    context.issues,
                );
                AstFactory::set_ast_node_data(
                    context.ast_nodes,
                    variant_id,
                    SyntaxNodeData::EnumVariant(name),
                );
            });
    }

    fn parse_fields(
        context: &mut DeclarationContext<'_>,
        start: usize,
        end: usize,
        parent: AstNodeId,
    ) {
        let Some(fields) =
            Recovery::split_top_level(context.tokens, start, end, ",", AngleScanContext::Type)
        else {
            if let (Some(first), Some(last)) = (
                context.tokens.get(start),
                end.checked_sub(1)
                    .and_then(|index| context.tokens.get(index)),
            ) {
                Self::push_issue(
                    context.issues,
                    ParseIssueKind::UnexpectedToken,
                    first.start()..last.end(),
                    t!("syntax.item_unsupported"),
                );
            }
            return;
        };
        fields
            .into_iter()
            .filter(|range| range.start < range.end)
            .for_each(|range| {
                let mut cursor = range.start;
                let attributes = AttributeParser::parse_attributes(
                    context.source,
                    context.tokens,
                    &mut cursor,
                    context.issues,
                );
                for attribute in &attributes {
                    if attribute.is_inner() {
                        Self::push_issue(
                            context.issues,
                            ParseIssueKind::UnexpectedToken,
                            attribute.start()..attribute.end(),
                            t!("syntax.inner_attribute_position"),
                        );
                    }
                }
                let visibility = AttributeParser::parse_visibility(
                    context.source,
                    context.tokens,
                    &mut cursor,
                    context.issues,
                );
                let bounds = Recovery::token_bounds(context.tokens, range.clone());
                let field_id = AstFactory::push_ast_node(
                    context.ast_nodes,
                    Some(parent),
                    SyntaxNodeKind::Field,
                    bounds.clone(),
                    attributes.clone(),
                );
                let field_cursor = TokenCursor::from_token_range(context.tokens, cursor..range.end)
                    .expect("字段 token 范围有效");
                let type_start = field_cursor
                    .find_top_level_punctuation(":", AngleScanContext::Header)
                    .map_or(cursor, |colon| colon + 1);
                if type_start < range.end {
                    TypeParser::collect(TypeInput {
                        tokens: context.tokens,
                        bounds: CursorRange::from_token_range(
                            context.tokens,
                            type_start..range.end,
                        )
                        .expect("字段类型范围有效"),
                        parent: field_id,
                        ast_nodes: context.ast_nodes,
                        issues: context.issues,
                    });
                }
                SyntaxCollector::collect(
                    SyntaxRequest {
                        source: context.source,
                        tokens: context.tokens,
                        bounds: cursor..range.end,
                        parent: field_id,
                        expression_tree: false,
                        edition: None,
                        macro_profiles: context.macro_profiles,
                        defer_opaque_macro_diagnostics: context.defer_opaque_macro_diagnostics,
                    },
                    context.ast_nodes,
                    context.issues,
                );
                AstFactory::set_ast_node_data(
                    context.ast_nodes,
                    field_id,
                    SyntaxNodeData::Field(visibility),
                );
            });
    }

    fn declaration_field_group(
        tokens: &[Token],
        keyword: &str,
        tail: &Range<usize>,
    ) -> Option<(usize, usize)> {
        let cursor = TokenCursor::from_token_range(tokens, tail.clone())?;
        let open = cursor
            .find_top_level_open_delimiter(Delimiter::Brace, AngleScanContext::Header)
            .or_else(|| {
                (keyword == "struct").then(|| {
                    cursor.find_top_level_open_delimiter(
                        Delimiter::Parenthesis,
                        AngleScanContext::Header,
                    )
                })?
            })?;
        Some((open, Recovery::matching_delimiter(tokens, open)?))
    }

    fn collect_where_clause(
        context: &mut DeclarationContext<'_>,
        keyword: &str,
        tail: Range<usize>,
        parent: AstNodeId,
    ) {
        let field_group = if matches!(keyword, "struct" | "union") {
            Self::declaration_field_group(context.tokens, keyword, &tail)
        } else {
            None
        };
        let header_end = match keyword {
            "struct" | "union" => field_group.map_or_else(
                || {
                    if tail.end > tail.start
                        && Recovery::is_punctuation(&context.tokens[tail.end - 1], ";")
                    {
                        tail.end - 1
                    } else {
                        tail.end
                    }
                },
                |(open, _)| open,
            ),
            "enum" => Self::declaration_body_open(context.tokens, tail.clone()).unwrap_or(tail.end),
            "type" => TokenCursor::from_token_range(context.tokens, tail.clone()).map_or(
                tail.end,
                |cursor| {
                    cursor
                        .find_top_level_punctuation("=", AngleScanContext::Header)
                        .or_else(|| {
                            cursor.find_top_level_punctuation(";", AngleScanContext::Header)
                        })
                        .unwrap_or(tail.end)
                },
            ),
            _ => return,
        };
        Self::collect_where_range(context, tail.start..header_end, parent);
        if keyword == "struct" {
            if let Some((open, close)) = field_group {
                if matches!(
                    context.tokens[open].kind(),
                    TokenKind::OpenDelimiter(Delimiter::Parenthesis)
                ) {
                    let end = if tail.end > close + 1
                        && Recovery::is_punctuation(&context.tokens[tail.end - 1], ";")
                    {
                        tail.end - 1
                    } else {
                        tail.end
                    };
                    Self::collect_where_range(context, close + 1..end, parent);
                }
            }
        }
    }

    fn collect_where_range(
        context: &mut DeclarationContext<'_>,
        bounds: Range<usize>,
        parent: AstNodeId,
    ) {
        WhereClauseParser::collect(WhereClauseInput {
            tokens: context.tokens,
            bounds,
            parent,
            ast_nodes: context.ast_nodes,
            issues: context.issues,
        });
    }

    fn declaration_body_open(tokens: &[Token], tail: Range<usize>) -> Option<usize> {
        TokenCursor::from_token_range(tokens, tail)?
            .find_top_level_open_delimiter(Delimiter::Brace, AngleScanContext::Header)
    }

    fn skip_declaration_tail(
        context: &mut DeclarationContext<'_>,
        cursor: &mut usize,
        has_body: bool,
    ) -> Range<usize> {
        let start = *cursor;
        let range = start..context.tokens.len();
        let scan = TokenCursor::from_token_range(context.tokens, range.clone());
        let body_open = has_body
            .then(|| {
                scan.as_ref().and_then(|cursor| {
                    cursor
                        .find_top_level_open_delimiter(Delimiter::Brace, AngleScanContext::Header)
                        .or_else(|| {
                            cursor.find_top_level_open_delimiter(
                                Delimiter::Brace,
                                AngleScanContext::TokenSequence,
                            )
                        })
                })
            })
            .flatten();
        let semicolon = scan.as_ref().and_then(|cursor| {
            cursor
                .find_top_level_punctuation(";", AngleScanContext::Header)
                .or_else(|| cursor.find_top_level_punctuation(";", AngleScanContext::TokenSequence))
        });
        if let Some(open) = body_open.filter(|open| semicolon.is_none_or(|semi| *open < semi)) {
            if let Some(close_index) = Recovery::matching_delimiter(context.tokens, open) {
                *cursor = close_index + 1;
                return start..*cursor;
            }
            *cursor = context.tokens.len();
            Self::push_issue(
                context.issues,
                ParseIssueKind::UnclosedDelimiter,
                context
                    .tokens
                    .get(start)
                    .map_or(context.source.len(), Token::start)
                    ..context.source.len(),
                t!("syntax.declaration_body_unclosed"),
            );
            return start..*cursor;
        }
        if let Some(semicolon) = semicolon {
            *cursor = semicolon + 1;
            return start..*cursor;
        }
        *cursor = context.tokens.len();
        start..*cursor
    }

    fn take_identifier(tokens: &[Token], cursor: &mut usize) -> Option<String> {
        let identifier = tokens.get(*cursor)?.kind().identifier()?.to_owned();
        *cursor += 1;
        Some(identifier)
    }

    fn collect_declared_type(
        context: &mut DeclarationContext<'_>,
        keyword: &str,
        tail: Range<usize>,
        parent: AstNodeId,
    ) {
        let Some(cursor) = TokenCursor::from_token_range(context.tokens, tail.clone()) else {
            return;
        };
        let declaration_end = if tail
            .end
            .checked_sub(1)
            .and_then(|index| context.tokens.get(index))
            .is_some_and(|token| Recovery::is_punctuation(token, ";"))
        {
            tail.end - 1
        } else {
            tail.end
        };
        let equals = cursor.find_top_level_punctuation("=", AngleScanContext::Header);
        let where_start = cursor
            .find_top_level_identifier("where", AngleScanContext::Header)
            .unwrap_or(declaration_end);
        let colon = TokenCursor::from_token_range(context.tokens, tail.start..where_start)
            .and_then(|cursor| cursor.find_top_level_punctuation(":", AngleScanContext::Header));
        let type_bounds = match keyword {
            "type" => colon.and_then(|index| {
                let bound_end = equals.unwrap_or(where_start).min(where_start);
                (index + 1 < bound_end).then_some(index + 1..bound_end)
            }),
            _ => None,
        };
        if let Some(bounds) = type_bounds {
            TypeParser::collect_bounds(TypeInput {
                tokens: context.tokens,
                bounds: CursorRange::from_token_range(context.tokens, bounds)
                    .expect("trait bound 范围有效"),
                parent,
                ast_nodes: context.ast_nodes,
                issues: context.issues,
            });
        }
        let type_range = match keyword {
            "type" => equals.map(|index| index + 1..declaration_end),
            "const" | "static" => colon.and_then(|index| {
                let end = equals.unwrap_or(declaration_end);
                (index + 1 < end).then_some(index + 1..end)
            }),
            _ => None,
        };
        if let Some(bounds) = type_range.filter(|bounds| bounds.start < bounds.end) {
            let input = TypeInput {
                tokens: context.tokens,
                bounds: CursorRange::from_token_range(context.tokens, bounds)
                    .expect("声明类型范围有效"),
                parent,
                ast_nodes: context.ast_nodes,
                issues: context.issues,
            };
            if keyword == "type" {
                TypeParser::collect_prefix(input);
            } else {
                TypeParser::collect(input);
            }
        }
    }

    fn peek_identifier(tokens: &[Token], cursor: usize) -> Option<String> {
        tokens.get(cursor)?.kind().identifier().map(str::to_owned)
    }

    fn consume_identifier(tokens: &[Token], cursor: &mut usize, expected: &str) -> bool {
        if Self::peek_identifier(tokens, *cursor).as_deref() == Some(expected) {
            *cursor += 1;
            true
        } else {
            false
        }
    }

    fn current_start(source: &SourceFile, tokens: &[Token], cursor: usize) -> usize {
        tokens.get(cursor).map_or(source.len(), Token::start)
    }

    fn current_end(source: &SourceFile, tokens: &[Token], cursor: usize) -> usize {
        tokens.get(cursor).map_or(source.len(), Token::end)
    }

    fn push_issue(
        issues: &mut Vec<ParseIssue>,
        kind: ParseIssueKind,
        bounds: Range<usize>,
        message: Result<Cow<'static, str>, RenderError>,
    ) {
        issues.push(AstFactory::parse_issue(kind, bounds, message));
    }
}
