mod foreign;

use std::ops::Range;

use self::foreign::ForeignItemParser;

use cop_model::{
    LocalizedMessage, MessageId, SYNTAX_EXTERN_CRATE_MISSING_SEMICOLON,
    SYNTAX_FUNCTION_BODY_UNCLOSED, SYNTAX_FUNCTION_EXPECT_NAME, SYNTAX_FUNCTION_MISSING_PARAMETERS,
    SYNTAX_FUNCTION_PARAMETERS_UNCLOSED, SYNTAX_ITEM_UNSUPPORTED, SYNTAX_ORPHANED_OUTER_ATTRIBUTE,
    SYNTAX_USE_MISSING_SEMICOLON,
};

use crate::{
    ast::{
        AstFactory, AstNodeId, Attribute, ExternBlockItem, ExternCrateItem, FunctionItem, Item,
        ItemKind, ItemSyntaxData, MacroDefinition, MacroInvocation, NamedDeclaration, ParseIssue,
        ParseIssueKind, SyntaxNode, SyntaxNodeKind, UnsupportedConstructKind, Visibility,
        VisibilityKind,
    },
    parser::{
        attributes::AttributeParser,
        block::{BlockInput, BlockParser},
        declarations::{self, DeclarationParser},
        expression::{SyntaxCollector, SyntaxRequest},
        parameters::{ParameterInput, ParameterParser},
        types::{TypeInput, TypeParser},
        use_tree::{UseTreeInput, UseTreeParser},
        where_clause::{WhereClauseInput, WhereClauseParser},
    },
    recovery::Recovery,
    Delimiter, RustEdition, SourceFile, Token, TokenKind,
};

use super::super::token_cursor::TokenCursor;

pub(crate) struct ParserContext<'source, 'state> {
    pub(super) source: &'source SourceFile,
    pub(super) tokens: &'source [Token],
    pub(super) edition: RustEdition,
    pub(super) cursor: &'state mut usize,
    pub(super) issues: &'state mut Vec<ParseIssue>,
    pub(super) ast_nodes: &'state mut Vec<SyntaxNode>,
}

pub(super) struct ItemSyntaxParser;

impl ItemSyntaxParser {
    pub(super) fn parse_use(mut context: ParserContext<'_, '_>, parent: AstNodeId) -> Option<()> {
        let start = Self::current_start(&context);
        *context.cursor += 1;
        let start_index = *context.cursor;
        let Some(terminator) = Recovery::semicolon(context.tokens, *context.cursor) else {
            let bounds = start..Self::current_end(&context);
            Self::push_issue(
                &mut context,
                ParseIssueKind::UnexpectedToken,
                bounds,
                SYNTAX_USE_MISSING_SEMICOLON,
            );
            Self::recover_item(&mut context);
            return None;
        };
        let (parsed, _) = UseTreeParser::parse(
            UseTreeInput {
                source: context.source,
                tokens: context.tokens,
                start: start_index,
                limit: terminator,
                parent,
            },
            context.ast_nodes,
            context.issues,
        );
        *context.cursor = terminator + 1;
        parsed.then_some(())
    }

    pub(super) fn parse_extern_crate(mut context: ParserContext<'_, '_>) -> ExternCrateItem {
        *context.cursor += 2;
        let name = Self::take_identifier(&mut context);
        let alias = if Self::consume_identifier(&mut context, "as") {
            Self::take_identifier(&mut context)
        } else {
            None
        };
        if !Self::consume_punctuation(&mut context, ";") {
            let bounds = Self::current_start(&context)..Self::current_end(&context);
            Self::push_issue(
                &mut context,
                ParseIssueKind::UnexpectedToken,
                bounds,
                SYNTAX_EXTERN_CRATE_MISSING_SEMICOLON,
            );
            Self::recover_item(&mut context);
        }
        AstFactory::extern_crate(name, alias)
    }

    pub(super) fn parse_extern_block(
        context: ParserContext<'_, '_>,
        parent: AstNodeId,
        edition: RustEdition,
    ) -> Option<ExternBlockItem> {
        ForeignItemParser::parse_extern_block(context, parent, edition, Self::parse_function)
    }

    pub(super) fn parse_named_declaration(
        context: ParserContext<'_, '_>,
        parent: AstNodeId,
    ) -> NamedDeclaration {
        DeclarationParser::parse(declarations::DeclarationInput {
            source: context.source,
            tokens: context.tokens,
            cursor: context.cursor,
            parent,
            ast_nodes: context.ast_nodes,
            issues: context.issues,
        })
    }

    pub(super) fn parse_function(
        mut context: ParserContext<'_, '_>,
        parent: AstNodeId,
        allow_variadic: bool,
    ) -> Option<FunctionItem> {
        let keyword_index = Recovery::function_keyword(context.tokens, *context.cursor)?;
        *context.cursor = keyword_index + 1;
        let Some(name) = Self::take_identifier(&mut context) else {
            Self::push_current_issue(
                &mut context,
                ParseIssueKind::UnexpectedToken,
                SYNTAX_FUNCTION_EXPECT_NAME,
            );
            ParameterParser::recover_unnamed_function(context.tokens, context.cursor);
            return None;
        };
        let Some(parameter_open) =
            Recovery::function_parameter_open(context.tokens, *context.cursor)
        else {
            let bounds = context.tokens[keyword_index].start()..Self::current_end(&context);
            Self::push_issue(
                &mut context,
                ParseIssueKind::UnexpectedToken,
                bounds,
                SYNTAX_FUNCTION_MISSING_PARAMETERS,
            );
            Self::recover_item(&mut context);
            return None;
        };
        let Some(parameter_close) = Recovery::matching_delimiter(context.tokens, parameter_open)
        else {
            let bounds = context.tokens[parameter_open].start()..Self::current_end(&context);
            Self::push_issue(
                &mut context,
                ParseIssueKind::UnclosedDelimiter,
                bounds,
                SYNTAX_FUNCTION_PARAMETERS_UNCLOSED,
            );
            return None;
        };
        ParameterParser::parse(ParameterInput {
            source: context.source,
            tokens: context.tokens,
            parent,
            start: parameter_open + 1,
            end: parameter_close,
            allow_variadic,
            ast_nodes: context.ast_nodes,
            issues: context.issues,
        });
        let body_open = Recovery::body_open(context.tokens, parameter_close + 1);
        let signature_end = body_open
            .or_else(|| Recovery::semicolon(context.tokens, parameter_close + 1))
            .unwrap_or(parameter_close + 1);
        SyntaxCollector::collect(
            SyntaxRequest {
                source: context.source,
                tokens: context.tokens,
                bounds: parameter_close + 1..signature_end,
                parent,
                expression_tree: false,
                edition: None,
            },
            context.ast_nodes,
            context.issues,
        );
        let signature_cursor = TokenCursor::new(context.tokens, parameter_close + 1..signature_end)
            .expect("函数签名 token 范围有效");
        let where_start = signature_cursor
            .find_top_level_identifier("where")
            .unwrap_or(signature_end);
        if let Some(arrow) = TokenCursor::new(context.tokens, parameter_close + 1..where_start)
            .and_then(|cursor| cursor.find_top_level_punctuation("->"))
        {
            if arrow + 1 < where_start {
                TypeParser::collect_prefix(TypeInput {
                    tokens: context.tokens,
                    bounds: arrow + 1..where_start,
                    parent,
                    ast_nodes: context.ast_nodes,
                    issues: context.issues,
                });
            }
        }
        WhereClauseParser::collect(WhereClauseInput {
            tokens: context.tokens,
            bounds: parameter_close + 1..signature_end,
            parent,
            ast_nodes: context.ast_nodes,
            issues: context.issues,
        });
        if let Some(open_index) = body_open {
            if let Some(close_index) = Recovery::matching_delimiter(context.tokens, open_index) {
                let saved_cursor = *context.cursor;
                *context.cursor = open_index + 1;
                let body_attributes = AttributeParser::parse_attributes(
                    context.source,
                    context.tokens,
                    context.cursor,
                    context.issues,
                );
                let leading_attributes = body_attributes
                    .iter()
                    .filter(|attribute| attribute.is_inner())
                    .cloned()
                    .collect::<Vec<_>>();
                let nodes_start = body_attributes
                    .iter()
                    .find(|attribute| !attribute.is_inner())
                    .and_then(|attribute| {
                        context
                            .tokens
                            .iter()
                            .position(|token| token.start() == attribute.start())
                    })
                    .unwrap_or(*context.cursor);
                *context.cursor = saved_cursor;
                let body_bounds =
                    context.tokens[open_index].start()..context.tokens[close_index].end();
                let body_id = AstFactory::push_ast_node(
                    context.ast_nodes,
                    Some(parent),
                    SyntaxNodeKind::Block,
                    body_bounds,
                    leading_attributes,
                );
                BlockParser::parse(
                    BlockInput {
                        source: context.source,
                        tokens: context.tokens,
                        edition: context.edition,
                        start: nodes_start,
                        end: close_index,
                        parent: body_id,
                    },
                    context.ast_nodes,
                    context.issues,
                );
                *context.cursor = close_index + 1;
            } else {
                *context.cursor = context.tokens.len();
                let bounds = context.tokens[open_index].start()..Self::current_end(&context);
                Self::push_issue(
                    &mut context,
                    ParseIssueKind::UnclosedDelimiter,
                    bounds,
                    SYNTAX_FUNCTION_BODY_UNCLOSED,
                );
            }
        } else {
            *context.cursor = parameter_close + 1;
            Self::skip_signature_tail(&mut context);
        }
        Some(AstFactory::function(Some(name)))
    }

    pub(super) fn parse_macro_item(context: ParserContext<'_, '_>) -> Option<MacroInvocation> {
        let (invocation, next_index) =
            SyntaxCollector::macro_item(context.tokens, *context.cursor)?;
        *context.cursor = next_index;
        Some(invocation)
    }

    pub(super) fn parse_macro_definition(
        context: ParserContext<'_, '_>,
    ) -> Option<MacroDefinition> {
        let (definition, next_index) =
            SyntaxCollector::macro_definition(context.tokens, *context.cursor)?;
        *context.cursor = next_index;
        Some(definition)
    }

    pub(super) fn parse_unsupported(
        mut context: ParserContext<'_, '_>,
        attributes: Vec<Attribute>,
        visibility: Visibility,
        item_start: usize,
        node_id: AstNodeId,
    ) -> Option<Item> {
        if matches!(
            Self::current_kind(&context),
            Some(TokenKind::CloseDelimiter(_))
        ) {
            AstFactory::update_ast_node(
                context.ast_nodes,
                node_id,
                SyntaxNodeKind::Opaque,
                item_start..item_start,
                attributes,
            );
            return None;
        }
        let start_index = *context.cursor;
        let start = Self::current_start(&context);
        Self::recover_item(&mut context);
        let end = context
            .tokens
            .get(context.cursor.saturating_sub(1))
            .map_or(start, Token::end);
        context.issues.push(AstFactory::unsupported_issue(
            UnsupportedConstructKind::Item,
            start..end,
            LocalizedMessage::new(SYNTAX_ITEM_UNSUPPORTED),
        ));
        if start_index == *context.cursor {
            return None;
        }
        let item = AstFactory::item(attributes, visibility, ItemKind::Other, item_start..end);
        AstFactory::update_ast_node(
            context.ast_nodes,
            node_id,
            SyntaxNodeKind::Opaque,
            item_start..end,
            item.attributes().to_vec(),
        );
        AstFactory::set_ast_node_item_data(
            context.ast_nodes,
            node_id,
            ItemSyntaxData::from_item(&item),
        );
        Some(item)
    }

    pub(super) fn record_detached_item(
        context: ParserContext<'_, '_>,
        parent: AstNodeId,
        item: &Item,
    ) {
        AstFactory::push_ast_node(
            context.ast_nodes,
            Some(parent),
            SyntaxNodeKind::Opaque,
            item.start()..item.end(),
            item.attributes().to_vec(),
        );
    }

    pub(super) fn record_detached_attributes(
        mut context: ParserContext<'_, '_>,
        parent: AstNodeId,
        attributes: Vec<Attribute>,
    ) {
        if let Some(item) = Self::attributes_only_item(&mut context, attributes) {
            Self::record_detached_item(context, parent, &item);
        }
    }

    pub(super) fn attributes_only_item(
        context: &mut ParserContext<'_, '_>,
        attributes: Vec<Attribute>,
    ) -> Option<Item> {
        let start = attributes.first()?.start();
        let end = attributes.last()?.end();
        if attributes.iter().any(|attribute| !attribute.is_inner()) {
            Self::push_issue(
                context,
                ParseIssueKind::UnexpectedToken,
                start..end,
                SYNTAX_ORPHANED_OUTER_ATTRIBUTE,
            );
        }
        let visibility = AstFactory::visibility(VisibilityKind::Private, start..start);
        Some(AstFactory::item(
            attributes,
            visibility,
            ItemKind::Other,
            start..end,
        ))
    }

    fn skip_signature_tail(context: &mut ParserContext<'_, '_>) {
        while *context.cursor < context.tokens.len() {
            if Recovery::is_punctuation(&context.tokens[*context.cursor], ";") {
                *context.cursor += 1;
                return;
            }
            if matches!(
                context.tokens[*context.cursor].kind(),
                TokenKind::OpenDelimiter(Delimiter::Brace)
            ) {
                if let Some(close_index) =
                    Recovery::matching_delimiter(context.tokens, *context.cursor)
                {
                    *context.cursor = close_index + 1;
                    return;
                }
            }
            *context.cursor += 1;
        }
    }

    fn recover_item(context: &mut ParserContext<'_, '_>) {
        let start = *context.cursor;
        while *context.cursor < context.tokens.len() {
            match context.tokens[*context.cursor].kind() {
                TokenKind::OpenDelimiter(_) => {
                    if let Some(close_index) =
                        Recovery::matching_delimiter(context.tokens, *context.cursor)
                    {
                        *context.cursor = close_index + 1;
                    } else {
                        *context.cursor += 1;
                    }
                }
                TokenKind::Punctuation(value) if value == ";" => {
                    *context.cursor += 1;
                    return;
                }
                TokenKind::CloseDelimiter(Delimiter::Brace) => return,
                _ => *context.cursor += 1,
            }
        }
        if *context.cursor == start {
            *context.cursor = (start + 1).min(context.tokens.len());
        }
    }

    fn push_current_issue(
        context: &mut ParserContext<'_, '_>,
        kind: ParseIssueKind,
        message: MessageId,
    ) {
        let bounds = Self::current_start(context)..Self::current_end(context);
        Self::push_issue(context, kind, bounds, message);
    }

    fn push_issue(
        context: &mut ParserContext<'_, '_>,
        kind: ParseIssueKind,
        bounds: Range<usize>,
        message: MessageId,
    ) {
        context.issues.push(AstFactory::parse_issue(
            kind,
            bounds,
            LocalizedMessage::new(message),
        ));
    }

    fn consume_identifier(context: &mut ParserContext<'_, '_>, expected: &str) -> bool {
        if Self::current_identifier(context) != Some(expected) {
            return false;
        }
        *context.cursor += 1;
        true
    }

    fn take_identifier(context: &mut ParserContext<'_, '_>) -> Option<String> {
        let identifier = Self::current_kind(context)?.identifier()?.to_owned();
        *context.cursor += 1;
        Some(identifier)
    }

    fn consume_punctuation(context: &mut ParserContext<'_, '_>, expected: &str) -> bool {
        if !context
            .tokens
            .get(*context.cursor)
            .is_some_and(|token| Recovery::is_punctuation(token, expected))
        {
            return false;
        }
        *context.cursor += 1;
        true
    }

    fn current_kind<'source>(context: &ParserContext<'source, '_>) -> Option<&'source TokenKind> {
        context.tokens.get(*context.cursor).map(Token::kind)
    }

    fn current_identifier<'source>(context: &ParserContext<'source, '_>) -> Option<&'source str> {
        match Self::current_kind(context)? {
            TokenKind::Identifier(identifier) => Some(identifier),
            _ => None,
        }
    }

    fn current_start(context: &ParserContext<'_, '_>) -> usize {
        context
            .tokens
            .get(*context.cursor)
            .map_or(context.source.len(), Token::start)
    }

    fn current_end(context: &ParserContext<'_, '_>) -> usize {
        context
            .tokens
            .get(*context.cursor)
            .map_or(context.source.len(), Token::end)
    }
}
