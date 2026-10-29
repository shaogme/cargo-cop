mod declaration;

use cop_i18n::{t, RenderError};
use std::{mem, ops::Range};

use self::declaration::{ItemSyntaxParser, ParserContext};
use super::{
    attributes::AttributeParser,
    block::LocalItemInput,
    expression::{ImplHeaderInput, MacroInputRequest, SyntaxCollector},
    generic_parameters::{GenericParameterInput, GenericParameterParser},
    token_cursor::TokenCursor,
    types::{TypeInput, TypeParser},
    where_clause::{WhereClauseInput, WhereClauseParser},
};
use crate::{
    ast::{
        AstFactory, AstNodeId, Attribute, ExternCrateItem, FunctionItem, ImplItem, Item, ItemKind,
        ItemSyntaxData, MacroDefinition, MacroInvocation, ModuleItem, NamedDeclaration, ParseIssue,
        ParseIssueKind, SyntaxNode, SyntaxNodeData, SyntaxNodeKind, TraitItem, Visibility,
    },
    recovery::Recovery,
    Delimiter, RustEdition, SourceFile, Token, TokenKind,
};

pub(super) struct ScopeItems {
    pub(super) inner_attributes: Vec<Attribute>,
}

pub(super) struct ParserInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) edition: RustEdition,
    pub(super) cursor: usize,
    pub(super) issues: Vec<ParseIssue>,
    pub(super) ast_nodes: Vec<SyntaxNode>,
}

pub(super) struct Parser<'a> {
    source: &'a SourceFile,
    tokens: &'a [Token],
    edition: RustEdition,
    cursor: usize,
    issues: Vec<ParseIssue>,
    ast_nodes: Vec<SyntaxNode>,
}

impl<'a> Parser<'a> {
    pub(super) fn new(input: ParserInput<'a>) -> Self {
        Self {
            source: input.source,
            tokens: input.tokens,
            edition: input.edition,
            cursor: input.cursor,
            issues: input.issues,
            ast_nodes: input.ast_nodes,
        }
    }

    pub(super) fn into_parts(self) -> (Vec<SyntaxNode>, Vec<ParseIssue>) {
        (self.ast_nodes, self.issues)
    }

    fn context(&mut self) -> ParserContext<'a, '_> {
        ParserContext {
            source: self.source,
            tokens: self.tokens,
            edition: self.edition,
            cursor: &mut self.cursor,
            issues: &mut self.issues,
            ast_nodes: &mut self.ast_nodes,
        }
    }

    pub(super) fn parse_items(&mut self, stop_at_brace: bool, parent: AstNodeId) -> ScopeItems {
        let mut has_items = false;
        let mut inner_attributes = Vec::new();
        while self.cursor < self.tokens.len() {
            let attributes = AttributeParser::parse_attributes(
                self.source,
                self.tokens,
                &mut self.cursor,
                &mut self.issues,
            );
            let mut outer_attributes = Vec::new();
            for attribute in attributes {
                if attribute.is_inner() {
                    if has_items {
                        self.push_issue(
                            ParseIssueKind::UnexpectedToken,
                            attribute.start()..attribute.end(),
                            t!("syntax.inner_attribute_position"),
                        );
                    }
                    inner_attributes.push(attribute);
                } else {
                    outer_attributes.push(attribute);
                }
            }
            if self.cursor >= self.tokens.len() {
                self.record_detached_attributes(parent, outer_attributes);
                break;
            }
            if matches!(
                self.current_kind(),
                Some(TokenKind::CloseDelimiter(Delimiter::Brace))
            ) {
                if stop_at_brace {
                    self.record_detached_attributes(parent, outer_attributes);
                    break;
                }
                self.record_detached_attributes(parent, outer_attributes);
                let start = self.current_start();
                self.push_issue(
                    ParseIssueKind::UnexpectedToken,
                    start..self.current_end(),
                    t!("syntax.unmatched_close_delimiter"),
                );
                self.cursor += 1;
                continue;
            }
            let previous_cursor = self.cursor;
            has_items |= self.parse_item(outer_attributes, parent, false).is_some();
            if self.cursor == previous_cursor {
                self.recover_item();
            }
        }
        ScopeItems { inner_attributes }
    }

    pub(super) fn parse_local_item(input: LocalItemInput<'a>) -> usize {
        let LocalItemInput {
            source,
            tokens,
            edition,
            start,
            parent,
            attributes,
            ast_nodes,
            issues,
        } = input;
        let mut parser = Self::new(ParserInput {
            source,
            tokens,
            edition,
            cursor: start,
            issues: mem::take(issues),
            ast_nodes: mem::take(ast_nodes),
        });
        parser.parse_item(attributes, parent, true);
        let end = parser.cursor;
        *issues = mem::take(&mut parser.issues);
        *ast_nodes = mem::take(&mut parser.ast_nodes);
        end
    }

    fn parse_item(
        &mut self,
        attributes: Vec<Attribute>,
        parent: AstNodeId,
        is_local: bool,
    ) -> Option<Item> {
        if self.cursor >= self.tokens.len()
            || (!attributes.is_empty()
                && matches!(
                    self.current_kind(),
                    Some(TokenKind::CloseDelimiter(Delimiter::Brace))
                ))
        {
            let item = self.attributes_only_item(attributes)?;
            self.record_detached_item(parent, &item);
            return Some(item);
        }
        let item_start = attributes
            .first()
            .map_or(self.current_start(), Attribute::start);
        let node_id = AstFactory::push_ast_node(
            &mut self.ast_nodes,
            Some(parent),
            SyntaxNodeKind::Item,
            item_start..item_start,
            attributes.clone(),
        );
        let visibility = AttributeParser::parse_visibility(
            self.source,
            self.tokens,
            &mut self.cursor,
            &mut self.issues,
        );
        let parsed_kind = if self.current_identifier().is_none() {
            if Recovery::function_keyword(self.tokens, self.cursor).is_some() {
                self.parse_function(node_id).map(ItemKind::Function)
            } else {
                return self.parse_unsupported(attributes, visibility, item_start, node_id);
            }
        } else {
            let identifier = self.current_identifier().unwrap_or_default();
            match identifier {
                "mod" => self.parse_module(node_id).map(ItemKind::Module),
                "use" => self.parse_use(node_id).map(|()| ItemKind::Use),
                "extern"
                    if self
                        .tokens
                        .get(self.cursor + 1)
                        .is_some_and(|token| Recovery::is_identifier(token, "crate")) =>
                {
                    Some(ItemKind::ExternCrate(self.parse_extern_crate()))
                }
                "extern" if Recovery::extern_block_header(self.tokens, self.cursor).is_some() => {
                    let edition = self.edition;
                    ItemSyntaxParser::parse_extern_block(self.context(), node_id, edition)
                        .map(ItemKind::ExternBlock)
                }
                "extern" if Recovery::function_keyword(self.tokens, self.cursor).is_some() => {
                    self.parse_function(node_id).map(ItemKind::Function)
                }
                "unsafe" if Recovery::extern_block_header(self.tokens, self.cursor).is_some() => {
                    let edition = self.edition;
                    ItemSyntaxParser::parse_extern_block(self.context(), node_id, edition)
                        .map(ItemKind::ExternBlock)
                }
                "struct" => Some(ItemKind::Struct(self.parse_named_declaration(node_id))),
                "enum" => Some(ItemKind::Enum(self.parse_named_declaration(node_id))),
                "union" => Some(ItemKind::Union(self.parse_named_declaration(node_id))),
                "type" => Some(ItemKind::TypeAlias(self.parse_named_declaration(node_id))),
                "const" => {
                    if Recovery::function_keyword(self.tokens, self.cursor).is_some() {
                        self.parse_function(node_id).map(ItemKind::Function)
                    } else {
                        Some(ItemKind::Constant(self.parse_named_declaration(node_id)))
                    }
                }
                "static" => Some(ItemKind::Static(self.parse_named_declaration(node_id))),
                "trait" => self.parse_trait(node_id).map(ItemKind::Trait),
                "impl" => self.parse_impl(node_id).map(ItemKind::Impl),
                "unsafe" | "default"
                    if Recovery::impl_keyword(self.tokens, self.cursor).is_some() =>
                {
                    self.parse_impl(node_id).map(ItemKind::Impl)
                }
                "unsafe" | "auto"
                    if Recovery::trait_keyword(self.tokens, self.cursor).is_some() =>
                {
                    self.parse_trait(node_id).map(ItemKind::Trait)
                }
                "fn" => self.parse_function(node_id).map(ItemKind::Function),
                _ if Recovery::function_keyword(self.tokens, self.cursor).is_some() => {
                    self.parse_function(node_id).map(ItemKind::Function)
                }
                _ if SyntaxCollector::macro_definition_end(self.tokens, self.cursor).is_some() => {
                    self.parse_macro_definition(parent)
                        .map(ItemKind::MacroDefinition)
                }
                _ if SyntaxCollector::macro_item_end(self.tokens, self.cursor).is_some() => {
                    self.parse_macro_item().map(ItemKind::Macro)
                }
                _ => return self.parse_unsupported(attributes, visibility, item_start, node_id),
            }
        };
        let Some(kind) = parsed_kind else {
            let end = self
                .tokens
                .get(self.cursor.saturating_sub(1))
                .map_or(item_start, Token::end);
            AstFactory::update_ast_node(
                &mut self.ast_nodes,
                node_id,
                SyntaxNodeKind::Opaque,
                item_start..end,
                attributes,
            );
            return None;
        };
        let end = self
            .tokens
            .get(self.cursor.saturating_sub(1))
            .map_or(item_start, Token::end);
        let item = AstFactory::item(attributes, visibility, kind, item_start..end);
        let node_kind = if matches!(item.kind(), ItemKind::Other) {
            SyntaxNodeKind::Opaque
        } else if matches!(item.kind(), ItemKind::MacroDefinition(_)) {
            SyntaxNodeKind::MacroDefinition
        } else if matches!(item.kind(), ItemKind::Macro(_)) {
            SyntaxNodeKind::Macro
        } else if is_local {
            SyntaxNodeKind::LocalItem
        } else {
            SyntaxNodeKind::Item
        };
        AstFactory::update_ast_node(
            &mut self.ast_nodes,
            node_id,
            node_kind,
            item_start..end,
            item.attributes().to_vec(),
        );
        AstFactory::set_ast_node_item_data(
            &mut self.ast_nodes,
            node_id,
            ItemSyntaxData::from_item(&item),
        );
        if let ItemKind::Macro(invocation) = item.kind() {
            AstFactory::set_ast_node_data(
                &mut self.ast_nodes,
                node_id,
                SyntaxNodeData::Macro(invocation.clone()),
            );
        }
        if let ItemKind::Macro(invocation) = item.kind() {
            let parsed = SyntaxCollector::collect_item_macro_input(
                MacroInputRequest {
                    source: self.source,
                    tokens: self.tokens,
                    parent: node_id,
                    edition: self.edition,
                },
                invocation,
                &mut self.ast_nodes,
                &mut self.issues,
            );
            if !parsed {
                let input = invocation.input().bounds();
                let input_id = AstFactory::push_ast_node(
                    &mut self.ast_nodes,
                    Some(node_id),
                    SyntaxNodeKind::MacroInput,
                    input.clone(),
                    Vec::new(),
                );
                AstFactory::push_ast_node(
                    &mut self.ast_nodes,
                    Some(input_id),
                    SyntaxNodeKind::Opaque,
                    input.clone(),
                    Vec::new(),
                );
                self.issues.push(AstFactory::opaque_macro_issue(
                    input,
                    t!("syntax.macro_opaque"),
                ));
            }
        } else if let ItemKind::MacroDefinition(definition) = item.kind() {
            AstFactory::push_ast_node(
                &mut self.ast_nodes,
                Some(node_id),
                SyntaxNodeKind::Opaque,
                definition.start()..definition.end(),
                Vec::new(),
            );
        }
        GenericParameterParser::collect(GenericParameterInput {
            source: self.source,
            tokens: self.tokens,
            parent: node_id,
            item: &item,
            ast_nodes: &mut self.ast_nodes,
            issues: &mut self.issues,
        });
        Some(item)
    }

    fn parse_module(&mut self, parent: AstNodeId) -> Option<ModuleItem> {
        self.cursor += 1;
        let Some(name) = self.take_identifier() else {
            self.push_current_issue(
                ParseIssueKind::UnexpectedToken,
                t!("syntax.expect_module_name"),
            );
            self.recover_item();
            return None;
        };
        if self.consume_punctuation(";") {
            let end = self.tokens[self.cursor - 1].end();
            let start = self.ast_nodes[parent.index()].start();
            AstFactory::push_ast_node(
                &mut self.ast_nodes,
                Some(parent),
                SyntaxNodeKind::Scope,
                start..end,
                Vec::new(),
            );
            return Some(AstFactory::module(name, false));
        }
        if !matches!(
            self.current_kind(),
            Some(TokenKind::OpenDelimiter(Delimiter::Brace))
        ) {
            self.push_current_issue(
                ParseIssueKind::UnexpectedToken,
                t!("syntax.expect_module_body"),
            );
            return Some(AstFactory::module(name, false));
        }
        let open_index = self.cursor;
        self.cursor += 1;
        let scope_id = AstFactory::push_ast_node(
            &mut self.ast_nodes,
            Some(parent),
            SyntaxNodeKind::Scope,
            self.tokens[open_index].start()..self.source.len(),
            Vec::new(),
        );
        let scope = self.parse_items(true, scope_id);
        let close_index = self.consume_close(Delimiter::Brace);
        if close_index.is_none() {
            self.push_issue(
                ParseIssueKind::UnclosedDelimiter,
                self.current_start()..self.current_end(),
                t!("syntax.inline_module_unclosed"),
            );
        }
        let scope_end = close_index.map_or(self.current_end(), |index| self.tokens[index].end());
        AstFactory::update_ast_node(
            &mut self.ast_nodes,
            scope_id,
            SyntaxNodeKind::Scope,
            self.tokens[open_index].start()..scope_end,
            scope.inner_attributes.clone(),
        );
        Some(AstFactory::module(name, true))
    }

    fn parse_use(&mut self, parent: AstNodeId) -> Option<()> {
        ItemSyntaxParser::parse_use(self.context(), parent)
    }

    fn parse_extern_crate(&mut self) -> ExternCrateItem {
        ItemSyntaxParser::parse_extern_crate(self.context())
    }

    fn parse_named_declaration(&mut self, parent: AstNodeId) -> NamedDeclaration {
        ItemSyntaxParser::parse_named_declaration(self.context(), parent)
    }

    fn parse_trait(&mut self, parent: AstNodeId) -> Option<TraitItem> {
        self.cursor = Recovery::trait_keyword(self.tokens, self.cursor)? + 1;
        let name = self.take_identifier();
        if name.is_none() {
            self.push_current_issue(
                ParseIssueKind::UnexpectedToken,
                t!("syntax.trait_expect_name"),
            );
        }
        let Some(open_index) = Recovery::body_open(self.tokens, self.cursor) else {
            let start = self.tokens[self.cursor.saturating_sub(1)].start();
            self.skip_to_semicolon();
            self.push_issue(
                ParseIssueKind::UnexpectedToken,
                start..self.current_end(),
                t!("syntax.trait_expect_body"),
            );
            return Some(AstFactory::trait_item(name));
        };
        let trait_header_end = TokenCursor::new(self.tokens, self.cursor..open_index)
            .and_then(|cursor| cursor.find_top_level_identifier("where"))
            .unwrap_or(open_index);
        if let Some(colon) = TokenCursor::new(self.tokens, self.cursor..trait_header_end)
            .and_then(|cursor| cursor.find_top_level_punctuation(":"))
        {
            if colon + 1 < trait_header_end {
                TypeParser::collect_bounds(TypeInput {
                    tokens: self.tokens,
                    bounds: colon + 1..trait_header_end,
                    parent,
                    ast_nodes: &mut self.ast_nodes,
                    issues: &mut self.issues,
                });
            }
        }
        WhereClauseParser::collect(WhereClauseInput {
            tokens: self.tokens,
            bounds: self.cursor..open_index,
            parent,
            ast_nodes: &mut self.ast_nodes,
            issues: &mut self.issues,
        });
        let scope_id = AstFactory::push_ast_node(
            &mut self.ast_nodes,
            Some(parent),
            SyntaxNodeKind::Scope,
            self.tokens[open_index].start()..self.source.len(),
            Vec::new(),
        );
        self.cursor = open_index + 1;
        let scope = self.parse_items(true, scope_id);
        let close_index = self.consume_close(Delimiter::Brace);
        let scope_end = close_index.map_or(self.current_end(), |index| self.tokens[index].end());
        AstFactory::update_ast_node(
            &mut self.ast_nodes,
            scope_id,
            SyntaxNodeKind::Scope,
            self.tokens[open_index].start()..scope_end,
            scope.inner_attributes.clone(),
        );
        Some(AstFactory::trait_item(name))
    }

    fn parse_impl(&mut self, parent: AstNodeId) -> Option<ImplItem> {
        let header_start = self.current_start();
        let impl_index = Recovery::impl_keyword(self.tokens, self.cursor)?;
        self.cursor = impl_index + 1;
        let Some(open_index) = Recovery::body_open(self.tokens, self.cursor) else {
            self.skip_to_semicolon();
            self.push_issue(
                ParseIssueKind::UnexpectedToken,
                header_start..self.current_end(),
                t!("syntax.impl_boundary_unknown"),
            );
            return Some(AstFactory::impl_item());
        };
        let impl_syntax = SyntaxCollector::collect_impl_header(
            ImplHeaderInput {
                source: self.source,
                tokens: self.tokens,
                bounds: impl_index + 1..open_index,
                parent,
                issues: &mut self.issues,
            },
            &mut self.ast_nodes,
        );
        if impl_syntax.self_type().is_none() {
            self.push_issue(
                ParseIssueKind::UnexpectedToken,
                header_start..self.tokens[open_index].start(),
                t!("syntax.impl_expect_target"),
            );
        }
        WhereClauseParser::collect(WhereClauseInput {
            tokens: self.tokens,
            bounds: impl_index + 1..open_index,
            parent,
            ast_nodes: &mut self.ast_nodes,
            issues: &mut self.issues,
        });
        AstFactory::set_ast_node_data(
            &mut self.ast_nodes,
            parent,
            SyntaxNodeData::Impl(impl_syntax),
        );
        let scope_id = AstFactory::push_ast_node(
            &mut self.ast_nodes,
            Some(parent),
            SyntaxNodeKind::Scope,
            self.tokens[open_index].start()..self.source.len(),
            Vec::new(),
        );
        self.cursor = open_index + 1;
        let scope = self.parse_items(true, scope_id);
        let close_index = self.consume_close(Delimiter::Brace);
        let scope_end = close_index.map_or(self.current_end(), |index| self.tokens[index].end());
        AstFactory::update_ast_node(
            &mut self.ast_nodes,
            scope_id,
            SyntaxNodeKind::Scope,
            self.tokens[open_index].start()..scope_end,
            scope.inner_attributes.clone(),
        );
        Some(AstFactory::impl_item())
    }

    fn parse_function(&mut self, parent: AstNodeId) -> Option<FunctionItem> {
        ItemSyntaxParser::parse_function(self.context(), parent, false)
    }

    fn parse_macro_item(&mut self) -> Option<MacroInvocation> {
        ItemSyntaxParser::parse_macro_item(self.context())
    }

    fn parse_macro_definition(&mut self, parent: AstNodeId) -> Option<MacroDefinition> {
        ItemSyntaxParser::parse_macro_definition(self.context(), parent)
    }

    fn parse_unsupported(
        &mut self,
        attributes: Vec<Attribute>,
        visibility: Visibility,
        item_start: usize,
        node_id: AstNodeId,
    ) -> Option<Item> {
        ItemSyntaxParser::parse_unsupported(
            self.context(),
            attributes,
            visibility,
            item_start,
            node_id,
        )
    }

    fn record_detached_item(&mut self, parent: AstNodeId, item: &Item) {
        ItemSyntaxParser::record_detached_item(self.context(), parent, item);
    }

    fn record_detached_attributes(&mut self, parent: AstNodeId, attributes: Vec<Attribute>) {
        ItemSyntaxParser::record_detached_attributes(self.context(), parent, attributes);
    }

    fn attributes_only_item(&mut self, attributes: Vec<Attribute>) -> Option<Item> {
        let mut context = self.context();
        ItemSyntaxParser::attributes_only_item(&mut context, attributes)
    }

    fn recover_item(&mut self) {
        let start = self.cursor;
        while self.cursor < self.tokens.len() {
            match self.tokens[self.cursor].kind() {
                TokenKind::OpenDelimiter(_) => {
                    if let Some(close_index) =
                        Recovery::matching_delimiter(self.tokens, self.cursor)
                    {
                        self.cursor = close_index + 1;
                    } else {
                        self.cursor += 1;
                    }
                }
                TokenKind::Punctuation(value) if value == ";" => {
                    self.cursor += 1;
                    return;
                }
                TokenKind::CloseDelimiter(Delimiter::Brace) => return,
                _ => self.cursor += 1,
            }
        }
        if self.cursor == start {
            self.cursor = (start + 1).min(self.tokens.len());
        }
    }

    fn skip_to_semicolon(&mut self) {
        if let Some(index) = Recovery::semicolon(self.tokens, self.cursor) {
            self.cursor = index + 1;
        } else {
            self.recover_item();
        }
    }

    fn push_current_issue(&mut self, kind: ParseIssueKind, message: Result<String, RenderError>) {
        self.push_issue(kind, self.current_start()..self.current_end(), message);
    }

    fn push_issue(
        &mut self,
        kind: ParseIssueKind,
        bounds: Range<usize>,
        message: Result<String, RenderError>,
    ) {
        self.issues
            .push(AstFactory::parse_issue(kind, bounds, message));
    }

    fn consume_close(&mut self, delimiter: Delimiter) -> Option<usize> {
        if !matches!(
            self.current_kind(),
            Some(TokenKind::CloseDelimiter(found)) if *found == delimiter
        ) {
            return None;
        }
        let close = self.cursor;
        self.cursor += 1;
        Some(close)
    }

    fn take_identifier(&mut self) -> Option<String> {
        let identifier = self.current_kind()?.identifier()?.to_owned();
        self.cursor += 1;
        Some(identifier)
    }

    fn consume_punctuation(&mut self, expected: &str) -> bool {
        if !self
            .current_token()
            .is_some_and(|token| Recovery::is_punctuation(token, expected))
        {
            return false;
        }
        self.cursor += 1;
        true
    }

    fn current_token(&self) -> Option<&Token> {
        self.tokens.get(self.cursor)
    }

    fn current_kind(&self) -> Option<&TokenKind> {
        self.current_token().map(Token::kind)
    }

    fn current_identifier(&self) -> Option<&str> {
        match self.current_kind()? {
            TokenKind::Identifier(identifier) => Some(identifier),
            _ => None,
        }
    }

    fn current_start(&self) -> usize {
        self.current_token().map_or(self.source.len(), Token::start)
    }

    fn current_end(&self) -> usize {
        self.current_token().map_or(self.source.len(), Token::end)
    }
}
