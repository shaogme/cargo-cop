use cop_model::{
    LocalizedMessage, MessageId, SYNTAX_MACRO_OPAQUE, SYNTAX_PARSE_ERROR_UNCLOSED_DELIMITER,
    SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN, SYNTAX_UNSUPPORTED_ASSOCIATED_ITEM,
};

use crate::{
    ast::{
        AstFactory, AstNodeId, Attribute, ExternBlockItem, ForeignFunctionSafety,
        ForeignItemSyntaxData, ForeignItemSyntaxKind, FunctionItem, ParseIssueKind, SyntaxNodeData,
        SyntaxNodeKind, UnsupportedConstructKind, Visibility,
    },
    parser::{
        attributes::AttributeParser,
        expression::SyntaxCollector,
        types::{TypeInput, TypeParser},
    },
    recovery::Recovery,
    Delimiter, RustEdition, RustStringLiteral, Token, TokenKind,
};

use super::ParserContext;

type ParseFunction =
    for<'source, 'state> fn(ParserContext<'source, 'state>, AstNodeId) -> Option<FunctionItem>;

struct ForeignFunctionInput<'source, 'state> {
    context: ParserContext<'source, 'state>,
    node_id: AstNodeId,
    item_start: usize,
    attributes: Vec<Attribute>,
    visibility: Visibility,
    safety: ForeignFunctionSafety,
    close_index: usize,
    parse_function: ParseFunction,
}

struct ForeignDeclarationInput<'source, 'state> {
    context: ParserContext<'source, 'state>,
    node_id: AstNodeId,
    item_start: usize,
    attributes: Vec<Attribute>,
    visibility: Visibility,
    block_unsafe: bool,
    close_index: usize,
    parse_function: ParseFunction,
}

struct ForeignStaticInput<'source, 'state> {
    context: ParserContext<'source, 'state>,
    node_id: AstNodeId,
    item_start: usize,
    attributes: Vec<Attribute>,
    visibility: Visibility,
    close_index: usize,
}

pub(super) struct ForeignItemParser;

impl ForeignItemParser {
    pub(super) fn parse_extern_block(
        mut context: ParserContext<'_, '_>,
        parent: AstNodeId,
        edition: RustEdition,
        parse_function: ParseFunction,
    ) -> Option<ExternBlockItem> {
        let header = Recovery::extern_block_header(context.tokens, *context.cursor)?;
        if edition == RustEdition::E2024 && !header.unsafe_ {
            let token = &context.tokens[header.extern_index];
            context.issues.push(AstFactory::parse_issue(
                ParseIssueKind::UnexpectedToken,
                token.start()..token.end(),
                LocalizedMessage::new(SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN),
            ));
        }
        let abi = header.abi.as_ref().and_then(|bounds| {
            context
                .source
                .slice(bounds.start, bounds.end)
                .and_then(RustStringLiteral::decode)
        });
        let close_index = Recovery::matching_delimiter(context.tokens, header.open_index);
        let scope_end = close_index
            .and_then(|index| context.tokens.get(index))
            .map_or_else(|| context.source.len(), Token::end);
        let scope_id = AstFactory::push_ast_node(
            context.ast_nodes,
            Some(parent),
            SyntaxNodeKind::Scope,
            context.tokens[header.open_index].start()..scope_end,
            Vec::new(),
        );
        *context.cursor = header.open_index + 1;
        if let Some(close_index) = close_index {
            Self::parse_foreign_items(
                &mut context,
                scope_id,
                close_index,
                header.unsafe_,
                parse_function,
            );
            *context.cursor = close_index + 1;
        } else {
            context.issues.push(AstFactory::parse_issue(
                ParseIssueKind::UnclosedDelimiter,
                context.tokens[header.open_index].start()..context.source.len(),
                LocalizedMessage::new(SYNTAX_PARSE_ERROR_UNCLOSED_DELIMITER),
            ));
            *context.cursor = context.tokens.len();
        }
        Some(AstFactory::extern_block(header.unsafe_, abi))
    }

    fn parse_foreign_items(
        context: &mut ParserContext<'_, '_>,
        scope_id: AstNodeId,
        close_index: usize,
        block_unsafe: bool,
        parse_function: ParseFunction,
    ) {
        let mut has_items = false;
        let mut inner_attributes = Vec::new();
        while *context.cursor < close_index {
            let attributes = AttributeParser::parse_attributes(
                context.source,
                context.tokens,
                context.cursor,
                context.issues,
            );
            let mut outer_attributes = Vec::new();
            for attribute in attributes {
                if attribute.is_inner() {
                    if has_items {
                        context.issues.push(AstFactory::parse_issue(
                            ParseIssueKind::UnexpectedToken,
                            attribute.start()..attribute.end(),
                            LocalizedMessage::new(SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN),
                        ));
                    }
                    inner_attributes.push(attribute);
                } else {
                    outer_attributes.push(attribute);
                }
            }
            if *context.cursor >= close_index {
                break;
            }
            let item_start = outer_attributes
                .first()
                .map_or_else(|| context.tokens[*context.cursor].start(), Attribute::start);
            let item_node = AstFactory::push_ast_node(
                context.ast_nodes,
                Some(scope_id),
                SyntaxNodeKind::ForeignItem,
                item_start..item_start,
                outer_attributes.clone(),
            );
            let previous = *context.cursor;
            if Self::parse_foreign_macro(context, item_node, item_start, outer_attributes.clone()) {
                has_items = true;
            } else {
                let visibility = AttributeParser::parse_visibility(
                    context.source,
                    context.tokens,
                    context.cursor,
                    context.issues,
                );
                Self::parse_foreign_declaration(ForeignDeclarationInput {
                    context: ParserContext {
                        source: context.source,
                        tokens: context.tokens,
                        edition: context.edition,
                        cursor: context.cursor,
                        issues: context.issues,
                        ast_nodes: context.ast_nodes,
                    },
                    node_id: item_node,
                    item_start,
                    attributes: outer_attributes,
                    visibility,
                    block_unsafe,
                    close_index,
                    parse_function,
                });
                has_items = true;
            }
            if *context.cursor == previous {
                Self::recover_foreign_item(context, close_index);
            }
        }
        let scope_start = context.ast_nodes[scope_id.index()].start();
        AstFactory::update_ast_node(
            context.ast_nodes,
            scope_id,
            SyntaxNodeKind::Scope,
            scope_start..context.tokens[close_index].end(),
            inner_attributes,
        );
    }

    fn parse_foreign_macro(
        context: &mut ParserContext<'_, '_>,
        node_id: AstNodeId,
        item_start: usize,
        attributes: Vec<Attribute>,
    ) -> bool {
        let Some((invocation, next_index)) =
            SyntaxCollector::macro_item(context.tokens, *context.cursor)
        else {
            return false;
        };
        *context.cursor = next_index;
        AstFactory::update_ast_node(
            context.ast_nodes,
            node_id,
            SyntaxNodeKind::Macro,
            item_start..invocation.end(),
            attributes,
        );
        AstFactory::set_ast_node_data(
            context.ast_nodes,
            node_id,
            SyntaxNodeData::Macro(invocation.clone()),
        );
        let opaque = invocation.input().bounds();
        let input_id = AstFactory::push_ast_node(
            context.ast_nodes,
            Some(node_id),
            SyntaxNodeKind::MacroInput,
            opaque.clone(),
            Vec::new(),
        );
        AstFactory::push_ast_node(
            context.ast_nodes,
            Some(input_id),
            SyntaxNodeKind::Opaque,
            opaque.clone(),
            Vec::new(),
        );
        context.issues.push(AstFactory::parse_issue(
            ParseIssueKind::UnsupportedSyntax,
            opaque,
            LocalizedMessage::new(SYNTAX_MACRO_OPAQUE),
        ));
        true
    }

    fn parse_foreign_declaration(input: ForeignDeclarationInput<'_, '_>) {
        let ForeignDeclarationInput {
            mut context,
            node_id,
            item_start,
            attributes,
            visibility,
            block_unsafe,
            close_index,
            parse_function,
        } = input;
        let mut safety = ForeignFunctionSafety::ImplicitUnsafe;
        if Self::consume_identifier(&mut context, "safe") {
            safety = ForeignFunctionSafety::Safe;
        } else if Self::consume_identifier(&mut context, "unsafe") {
            safety = ForeignFunctionSafety::Unsafe;
        }
        if safety != ForeignFunctionSafety::ImplicitUnsafe && !block_unsafe {
            let start = context
                .tokens
                .get(context.cursor.saturating_sub(1))
                .map_or(item_start, Token::start);
            let end = context
                .tokens
                .get(context.cursor.saturating_sub(1))
                .map_or(start, Token::end);
            context.issues.push(AstFactory::parse_issue(
                ParseIssueKind::UnexpectedToken,
                start..end,
                LocalizedMessage::new(SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN),
            ));
        }
        if safety != ForeignFunctionSafety::ImplicitUnsafe
            && Self::current_identifier(&context) == Some("static")
        {
            let token = context
                .tokens
                .get(context.cursor.saturating_sub(1))
                .expect("foreign item safety qualifier exists");
            context.issues.push(AstFactory::parse_issue(
                ParseIssueKind::UnexpectedToken,
                token.start()..token.end(),
                LocalizedMessage::new(SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN),
            ));
        }
        match Self::current_identifier(&context) {
            Some("fn") => Self::parse_foreign_function(ForeignFunctionInput {
                context,
                node_id,
                item_start,
                attributes,
                visibility,
                safety,
                close_index,
                parse_function,
            }),
            Some("static") => Self::parse_foreign_static(ForeignStaticInput {
                context,
                node_id,
                item_start,
                attributes,
                visibility,
                close_index,
            }),
            _ => Self::parse_unsupported_foreign(
                &mut context,
                node_id,
                item_start,
                attributes,
                close_index,
            ),
        }
    }

    fn parse_foreign_function(input: ForeignFunctionInput<'_, '_>) {
        let ForeignFunctionInput {
            mut context,
            node_id,
            item_start,
            attributes,
            visibility,
            safety,
            close_index,
            parse_function,
        } = input;
        let start = *context.cursor;
        let parameter_open = Recovery::function_parameter_open(context.tokens, start);
        let parameter_close =
            parameter_open.and_then(|open| Recovery::matching_delimiter(context.tokens, open));
        let semicolon = parameter_close.and_then(|close| {
            let tail_start = close + 1;
            let begins_sibling = matches!(
                context.tokens.get(tail_start).map(Token::kind),
                Some(TokenKind::Identifier(value))
                    if matches!(value.as_str(), "fn" | "static" | "type")
            ) && !context
                .tokens
                .get(close)
                .is_some_and(|token| Recovery::is_punctuation(token, "->"));
            (!begins_sibling)
                .then(|| Self::foreign_semicolon(context.tokens, tail_start, close_index))
                .flatten()
        });
        let body_open = parameter_close
            .and_then(|close| Recovery::body_open(context.tokens, close + 1))
            .filter(|open| *open < close_index);
        if semicolon.is_none()
            || body_open.is_some_and(|open| semicolon.map_or(true, |end| open < end))
        {
            let issue_start = body_open
                .and_then(|open| context.tokens.get(open))
                .map_or_else(|| Self::current_start(&context), Token::start);
            let issue_end = body_open
                .and_then(|open| context.tokens.get(open))
                .map_or_else(|| Self::current_end(&context), Token::end);
            context.issues.push(AstFactory::parse_issue(
                ParseIssueKind::UnexpectedToken,
                issue_start..issue_end,
                LocalizedMessage::new(SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN),
            ));
            Self::recover_foreign_item(&mut context, close_index);
            let end = context
                .tokens
                .get(context.cursor.saturating_sub(1))
                .map_or(item_start, Token::end);
            AstFactory::update_ast_node(
                context.ast_nodes,
                node_id,
                SyntaxNodeKind::Opaque,
                item_start..end,
                attributes,
            );
            return;
        }
        if let Some(function) = parse_function(
            ParserContext {
                source: context.source,
                tokens: context.tokens,
                edition: context.edition,
                cursor: context.cursor,
                issues: context.issues,
                ast_nodes: context.ast_nodes,
            },
            node_id,
        ) {
            let end = context
                .tokens
                .get(context.cursor.saturating_sub(1))
                .map_or(item_start, Token::end);
            AstFactory::update_ast_node(
                context.ast_nodes,
                node_id,
                SyntaxNodeKind::ForeignItem,
                item_start..end,
                attributes,
            );
            AstFactory::set_ast_node_data(
                context.ast_nodes,
                node_id,
                SyntaxNodeData::ForeignItem(ForeignItemSyntaxData::new(
                    visibility,
                    ForeignItemSyntaxKind::Function {
                        name: function.name().map(str::to_owned),
                        safety,
                    },
                )),
            );
        }
    }

    fn parse_foreign_static(input: ForeignStaticInput<'_, '_>) {
        let ForeignStaticInput {
            mut context,
            node_id,
            item_start,
            attributes,
            visibility,
            close_index,
        } = input;
        let static_index = *context.cursor;
        *context.cursor += 1;
        let mutable = Self::consume_identifier(&mut context, "mut");
        let name = Self::take_identifier(&mut context);
        let colon = context
            .tokens
            .get(*context.cursor)
            .is_some_and(|token| Recovery::is_punctuation(token, ":"));
        if colon {
            *context.cursor += 1;
        }
        let semicolon = Self::foreign_semicolon(context.tokens, *context.cursor, close_index);
        if let Some(semicolon) = semicolon {
            if colon && *context.cursor < semicolon {
                TypeParser::collect(TypeInput {
                    tokens: context.tokens,
                    bounds: *context.cursor..semicolon,
                    parent: node_id,
                    ast_nodes: context.ast_nodes,
                    issues: context.issues,
                });
            } else {
                let token = &context.tokens[static_index];
                context.issues.push(AstFactory::parse_issue(
                    ParseIssueKind::UnexpectedToken,
                    token.start()..token.end(),
                    LocalizedMessage::new(SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN),
                ));
            }
            *context.cursor = semicolon + 1;
        } else {
            Self::push_current_issue(
                &mut context,
                ParseIssueKind::UnexpectedToken,
                SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN,
            );
            Self::recover_foreign_item(&mut context, close_index);
        }
        let end = context
            .tokens
            .get(context.cursor.saturating_sub(1))
            .map_or(item_start, Token::end);
        AstFactory::update_ast_node(
            context.ast_nodes,
            node_id,
            SyntaxNodeKind::ForeignItem,
            item_start..end,
            attributes,
        );
        AstFactory::set_ast_node_data(
            context.ast_nodes,
            node_id,
            SyntaxNodeData::ForeignItem(ForeignItemSyntaxData::new(
                visibility,
                ForeignItemSyntaxKind::Static { name, mutable },
            )),
        );
    }

    fn parse_unsupported_foreign(
        context: &mut ParserContext<'_, '_>,
        node_id: AstNodeId,
        item_start: usize,
        attributes: Vec<Attribute>,
        close_index: usize,
    ) {
        let start = Self::current_start(context);
        Self::recover_foreign_item(context, close_index);
        let end = context
            .tokens
            .get(context.cursor.saturating_sub(1))
            .map_or(start, Token::end);
        context.issues.push(AstFactory::unsupported_issue(
            UnsupportedConstructKind::AssociatedItem,
            start..end,
            LocalizedMessage::new(SYNTAX_UNSUPPORTED_ASSOCIATED_ITEM),
        ));
        AstFactory::update_ast_node(
            context.ast_nodes,
            node_id,
            SyntaxNodeKind::Opaque,
            item_start..end,
            attributes,
        );
    }

    fn recover_foreign_item(context: &mut ParserContext<'_, '_>, close_index: usize) {
        let start = *context.cursor;
        while *context.cursor < close_index {
            if *context.cursor > start
                && matches!(
                    context.tokens[*context.cursor].kind(),
                    TokenKind::Identifier(value)
                        if matches!(value.as_str(), "fn" | "static" | "type")
                )
            {
                return;
            }
            match context.tokens[*context.cursor].kind() {
                TokenKind::OpenDelimiter(_) => {
                    if let Some(close) =
                        Recovery::matching_delimiter(context.tokens, *context.cursor)
                    {
                        *context.cursor = (close + 1).min(close_index);
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
            *context.cursor = (*context.cursor + 1).min(close_index);
        }
    }

    fn foreign_semicolon(tokens: &[Token], start: usize, close_index: usize) -> Option<usize> {
        let mut index = start;
        while index < close_index {
            if index > start
                && matches!(
                    tokens[index].kind(),
                    TokenKind::Identifier(value) if matches!(value.as_str(), "fn" | "static" | "type")
                )
                && !tokens
                    .get(index.wrapping_sub(1))
                    .is_some_and(|previous| Recovery::is_punctuation(previous, "->"))
            {
                return None;
            }
            match tokens[index].kind() {
                TokenKind::OpenDelimiter(_) => {
                    index = Recovery::matching_delimiter(tokens, index)? + 1;
                }
                TokenKind::Punctuation(value) if value == ";" => return Some(index),
                TokenKind::CloseDelimiter(Delimiter::Brace) => return None,
                _ => index += 1,
            }
        }
        None
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

    fn current_identifier<'source>(context: &ParserContext<'source, '_>) -> Option<&'source str> {
        Self::current_kind(context)?.identifier()
    }

    fn current_kind<'source>(context: &ParserContext<'source, '_>) -> Option<&'source TokenKind> {
        context.tokens.get(*context.cursor).map(Token::kind)
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

    fn push_current_issue(
        context: &mut ParserContext<'_, '_>,
        kind: ParseIssueKind,
        message: MessageId,
    ) {
        let bounds = Self::current_start(context)..Self::current_end(context);
        context.issues.push(AstFactory::parse_issue(
            kind,
            bounds,
            LocalizedMessage::new(message),
        ));
    }
}
