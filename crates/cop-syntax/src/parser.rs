mod attributes;
mod expression;

use std::{mem, ops::Range};

use cop_model::{
    LocalizedMessage, MessageId, SYNTAX_DECLARATION_BODY_UNCLOSED, SYNTAX_EXPECT_MODULE_BODY,
    SYNTAX_EXPECT_MODULE_NAME, SYNTAX_EXTERN_CRATE_MISSING_SEMICOLON,
    SYNTAX_FUNCTION_BODY_UNCLOSED, SYNTAX_FUNCTION_EXPECT_NAME, SYNTAX_FUNCTION_MISSING_PARAMETERS,
    SYNTAX_FUNCTION_PARAMETERS_UNCLOSED, SYNTAX_IMPL_BOUNDARY_UNKNOWN, SYNTAX_IMPL_EXPECT_TARGET,
    SYNTAX_INLINE_MODULE_UNCLOSED, SYNTAX_INNER_ATTRIBUTE_POSITION, SYNTAX_ITEM_UNSUPPORTED,
    SYNTAX_NAMED_ITEM_EXPECT_NAME, SYNTAX_ORPHANED_OUTER_ATTRIBUTE, SYNTAX_TRAIT_EXPECT_BODY,
    SYNTAX_TRAIT_EXPECT_NAME, SYNTAX_UNMATCHED_CLOSE_DELIMITER, SYNTAX_USE_MISSING_SEMICOLON,
};

use self::{attributes::AttributeParser, expression::SyntaxCollector};
use crate::{
    ast::{
        AstFactory, Attribute, ExternCrateItem, FunctionItem, ImplItem, Item, ItemKind,
        MacroInvocation, ModuleItem, NamedDeclaration, Parameter, ParseIssue, ParseIssueKind,
        ParseOutput, ParseOutputParts, TraitItem, UseItem, UseTree, Visibility, VisibilityKind,
    },
    recovery::Recovery,
    Delimiter, RustLexer, SourceFile, Token, TokenKind,
};

pub struct RustParser;

struct ScopeItems {
    inner_attributes: Vec<Attribute>,
    items: Vec<Item>,
}

struct Parser<'a> {
    source: &'a SourceFile,
    tokens: &'a [Token],
    cursor: usize,
    issues: Vec<ParseIssue>,
}

impl RustParser {
    pub fn parse(source: SourceFile) -> ParseOutput {
        let lexed = RustLexer::lex(&source);
        let lexical_issues = lexed.issues().iter().map(ParseIssue::from).collect();
        let mut parser = Parser {
            source: &source,
            tokens: lexed.tokens(),
            cursor: 0,
            issues: lexical_issues,
        };
        let scope = parser.parse_items(false);
        let mut issues = mem::take(&mut parser.issues);
        issues.extend(Recovery::delimiter_issues(lexed.tokens()));
        issues.sort_by(|left, right| {
            left.start()
                .cmp(&right.start())
                .then_with(|| left.end().cmp(&right.end()))
                .then_with(|| left.message().cmp(right.message()))
        });
        issues.dedup_by(|left, right| left.kind() == right.kind() && left.start() == right.start());
        let coverage = Recovery::coverage_for(&issues);
        drop(parser);
        AstFactory::output(ParseOutputParts {
            source,
            lexed,
            inner_attributes: scope.inner_attributes,
            items: scope.items,
            issues,
            coverage,
        })
    }
}

impl Parser<'_> {
    fn parse_items(&mut self, stop_at_brace: bool) -> ScopeItems {
        let mut items = Vec::new();
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
                    if !items.is_empty() {
                        self.push_issue(
                            ParseIssueKind::UnexpectedToken,
                            attribute.start()..attribute.end(),
                            SYNTAX_INNER_ATTRIBUTE_POSITION,
                        );
                    }
                    inner_attributes.push(attribute);
                } else {
                    outer_attributes.push(attribute);
                }
            }
            if self.cursor >= self.tokens.len() {
                if !outer_attributes.is_empty() {
                    if let Some(item) = self.attributes_only_item(outer_attributes) {
                        items.push(item);
                    }
                }
                break;
            }
            if matches!(
                self.current_kind(),
                Some(TokenKind::CloseDelimiter(Delimiter::Brace))
            ) {
                if stop_at_brace {
                    if !outer_attributes.is_empty() {
                        if let Some(item) = self.attributes_only_item(outer_attributes) {
                            items.push(item);
                        }
                    }
                    break;
                }
                if !outer_attributes.is_empty() {
                    if let Some(item) = self.attributes_only_item(outer_attributes) {
                        items.push(item);
                    }
                }
                let start = self.current_start();
                self.push_issue(
                    ParseIssueKind::UnexpectedToken,
                    start..self.current_end(),
                    SYNTAX_UNMATCHED_CLOSE_DELIMITER,
                );
                self.cursor += 1;
                continue;
            }
            let previous_cursor = self.cursor;
            if let Some(item) = self.parse_item(outer_attributes) {
                items.push(item);
            }
            if self.cursor == previous_cursor {
                self.recover_item();
            }
        }
        ScopeItems {
            inner_attributes,
            items,
        }
    }

    fn parse_item(&mut self, attributes: Vec<Attribute>) -> Option<Item> {
        if self.cursor >= self.tokens.len() {
            return self.attributes_only_item(attributes);
        }
        if !attributes.is_empty()
            && matches!(
                self.current_kind(),
                Some(TokenKind::CloseDelimiter(Delimiter::Brace))
            )
        {
            return self.attributes_only_item(attributes);
        }
        let item_start = attributes
            .first()
            .map_or(self.current_start(), Attribute::start);
        let visibility = AttributeParser::parse_visibility(
            self.source,
            self.tokens,
            &mut self.cursor,
            &mut self.issues,
        );
        let Some(identifier) = self.current_identifier() else {
            return self.parse_unsupported(attributes, visibility, item_start);
        };
        let kind = match identifier {
            "mod" => ItemKind::Module(self.parse_module()?),
            "use" => ItemKind::Use(self.parse_use()?),
            "extern"
                if self
                    .tokens
                    .get(self.cursor + 1)
                    .is_some_and(|token| Recovery::is_identifier(token, "crate")) =>
            {
                ItemKind::ExternCrate(self.parse_extern_crate())
            }
            "extern" if Recovery::function_keyword(self.tokens, self.cursor).is_some() => {
                ItemKind::Function(self.parse_function()?)
            }
            "struct" => ItemKind::Struct(self.parse_named_declaration()),
            "enum" => ItemKind::Enum(self.parse_named_declaration()),
            "union" => ItemKind::Union(self.parse_named_declaration()),
            "type" => ItemKind::TypeAlias(self.parse_named_declaration()),
            "const" => {
                if Recovery::function_keyword(self.tokens, self.cursor).is_some() {
                    ItemKind::Function(self.parse_function()?)
                } else {
                    ItemKind::Constant(self.parse_named_declaration())
                }
            }
            "static" => ItemKind::Static(self.parse_named_declaration()),
            "trait" => ItemKind::Trait(self.parse_trait()?),
            "impl" => ItemKind::Impl(self.parse_impl()?),
            "unsafe" | "default" if Recovery::impl_keyword(self.tokens, self.cursor).is_some() => {
                ItemKind::Impl(self.parse_impl()?)
            }
            "unsafe" | "auto" if Recovery::trait_keyword(self.tokens, self.cursor).is_some() => {
                ItemKind::Trait(self.parse_trait()?)
            }
            "fn" => ItemKind::Function(self.parse_function()?),
            _ if Recovery::function_keyword(self.tokens, self.cursor).is_some() => {
                ItemKind::Function(self.parse_function()?)
            }
            _ if SyntaxCollector::macro_item_end(self.tokens, self.cursor).is_some() => {
                ItemKind::Macro(self.parse_macro_item()?)
            }
            _ => return self.parse_unsupported(attributes, visibility, item_start),
        };
        let end = self
            .tokens
            .get(self.cursor.saturating_sub(1))
            .map_or(item_start, Token::end);
        Some(AstFactory::item(
            attributes,
            visibility,
            kind,
            item_start..end,
        ))
    }

    fn parse_module(&mut self) -> Option<ModuleItem> {
        self.cursor += 1;
        let Some(name) = self.take_identifier() else {
            self.push_current_issue(ParseIssueKind::UnexpectedToken, SYNTAX_EXPECT_MODULE_NAME);
            self.recover_item();
            return None;
        };
        if self.consume_punctuation(";") {
            return Some(AstFactory::module(name, false, Vec::new(), Vec::new()));
        }
        if !matches!(
            self.current_kind(),
            Some(TokenKind::OpenDelimiter(Delimiter::Brace))
        ) {
            self.push_current_issue(ParseIssueKind::UnexpectedToken, SYNTAX_EXPECT_MODULE_BODY);
            return Some(AstFactory::module(name, false, Vec::new(), Vec::new()));
        }
        self.cursor += 1;
        let scope = self.parse_items(true);
        if self.consume_close(Delimiter::Brace).is_none() {
            self.push_issue(
                ParseIssueKind::UnclosedDelimiter,
                self.current_start()..self.current_end(),
                SYNTAX_INLINE_MODULE_UNCLOSED,
            );
        }
        Some(AstFactory::module(
            name,
            true,
            scope.inner_attributes,
            scope.items,
        ))
    }

    fn parse_use(&mut self) -> Option<UseItem> {
        let start = self.current_start();
        self.cursor += 1;
        let start_index = self.cursor;
        let Some(terminator) = Recovery::semicolon(self.tokens, self.cursor) else {
            self.push_issue(
                ParseIssueKind::UnexpectedToken,
                start..self.current_end(),
                SYNTAX_USE_MISSING_SEMICOLON,
            );
            self.recover_item();
            return None;
        };
        let mut index = start_index;
        let tree = self.parse_use_tree(&mut index, terminator);
        self.cursor = terminator + 1;
        Some(AstFactory::use_item(tree))
    }

    fn parse_extern_crate(&mut self) -> ExternCrateItem {
        self.cursor += 2;
        let name = self.take_identifier();
        let alias = if self.consume_identifier("as") {
            self.take_identifier()
        } else {
            None
        };
        if !self.consume_punctuation(";") {
            self.push_current_issue(
                ParseIssueKind::UnexpectedToken,
                SYNTAX_EXTERN_CRATE_MISSING_SEMICOLON,
            );
            self.recover_item();
        }
        AstFactory::extern_crate(name, alias)
    }

    fn parse_use_tree(&self, index: &mut usize, limit: usize) -> UseTree {
        let start_index = *index;
        let mut path = Vec::new();
        let mut children = Vec::new();
        let mut alias = None;
        let mut glob = false;
        if *index < limit && Recovery::is_punctuation(&self.tokens[*index], "::") {
            path.push("::".to_owned());
            *index += 1;
        }
        while *index < limit {
            if let TokenKind::Identifier(identifier) = self.tokens[*index].kind() {
                if identifier == "as" {
                    *index += 1;
                    alias = self.take_identifier_at(index, limit);
                    break;
                }
                path.push(identifier.clone());
                *index += 1;
                if *index < limit && Recovery::is_punctuation(&self.tokens[*index], "::") {
                    *index += 1;
                    if *index < limit
                        && matches!(
                            self.tokens[*index].kind(),
                            TokenKind::OpenDelimiter(Delimiter::Brace)
                        )
                    {
                        break;
                    }
                    continue;
                }
            }
            break;
        }
        if *index < limit
            && matches!(
                self.tokens[*index].kind(),
                TokenKind::OpenDelimiter(Delimiter::Brace)
            )
        {
            let open_index = *index;
            if let Some(close_index) = Recovery::matching_delimiter(self.tokens, open_index) {
                *index += 1;
                while *index < close_index {
                    let previous = *index;
                    children.push(self.parse_use_tree(index, close_index));
                    if *index == previous {
                        *index += 1;
                    }
                    if *index < close_index && Recovery::is_punctuation(&self.tokens[*index], ",") {
                        *index += 1;
                    } else if *index < close_index {
                        break;
                    }
                }
                *index = close_index + 1;
            }
        } else if *index < limit && Recovery::is_punctuation(&self.tokens[*index], "*") {
            glob = true;
            *index += 1;
        }
        if *index < limit && Recovery::is_identifier(&self.tokens[*index], "as") {
            *index += 1;
            alias = self.take_identifier_at(index, limit);
        }
        let start = self.tokens.get(start_index).map_or(0, Token::start);
        let end = self
            .tokens
            .get(index.saturating_sub(1))
            .map_or(start, Token::end);
        AstFactory::use_tree(path, alias, glob, children, start..end)
    }

    fn parse_named_declaration(&mut self) -> NamedDeclaration {
        let keyword = self.current_identifier().unwrap_or("").to_owned();
        self.cursor += 1;
        if keyword == "static" {
            self.consume_identifier("mut");
        }
        let name = self.take_identifier();
        if name.is_none() {
            self.push_current_issue(
                ParseIssueKind::UnexpectedToken,
                SYNTAX_NAMED_ITEM_EXPECT_NAME,
            );
        }
        let has_body = matches!(keyword.as_str(), "struct" | "enum" | "union");
        let tail = self.skip_declaration_tail(has_body);
        let nodes = SyntaxCollector::collect(self.tokens, tail);
        AstFactory::named_declaration(name, nodes)
    }

    fn parse_trait(&mut self) -> Option<TraitItem> {
        self.cursor = Recovery::trait_keyword(self.tokens, self.cursor)? + 1;
        let name = self.take_identifier();
        if name.is_none() {
            self.push_current_issue(ParseIssueKind::UnexpectedToken, SYNTAX_TRAIT_EXPECT_NAME);
        }
        let Some(open_index) = Recovery::body_open(self.tokens, self.cursor) else {
            let start = self.tokens[self.cursor.saturating_sub(1)].start();
            self.skip_to_semicolon();
            self.push_issue(
                ParseIssueKind::UnsupportedSyntax,
                start..self.current_end(),
                SYNTAX_TRAIT_EXPECT_BODY,
            );
            return Some(AstFactory::trait_item(name, Vec::new(), Vec::new()));
        };
        self.cursor = open_index + 1;
        let scope = self.parse_items(true);
        self.consume_close(Delimiter::Brace);
        Some(AstFactory::trait_item(
            name,
            scope.inner_attributes,
            scope.items,
        ))
    }

    fn parse_impl(&mut self) -> Option<ImplItem> {
        let header_start = self.current_start();
        self.cursor = Recovery::impl_keyword(self.tokens, self.cursor)? + 1;
        let Some(open_index) = Recovery::body_open(self.tokens, self.cursor) else {
            self.skip_to_semicolon();
            self.push_issue(
                ParseIssueKind::UnsupportedSyntax,
                header_start..self.current_end(),
                SYNTAX_IMPL_BOUNDARY_UNKNOWN,
            );
            return Some(AstFactory::impl_item(String::new(), Vec::new(), Vec::new()));
        };
        let target = self
            .source
            .slice(
                self.tokens[self.cursor].start(),
                self.tokens[open_index].start(),
            )
            .unwrap_or("")
            .trim()
            .to_owned();
        if target.is_empty() {
            self.push_issue(
                ParseIssueKind::UnexpectedToken,
                header_start..self.tokens[open_index].start(),
                SYNTAX_IMPL_EXPECT_TARGET,
            );
        }
        self.cursor = open_index + 1;
        let scope = self.parse_items(true);
        self.consume_close(Delimiter::Brace);
        Some(AstFactory::impl_item(
            target,
            scope.inner_attributes,
            scope.items,
        ))
    }

    fn recover_unnamed_function(&mut self) {
        let Some(parameter_open) = Recovery::function_parameter_open(self.tokens, self.cursor)
        else {
            self.recover_item();
            return;
        };
        let Some(parameter_close) = Recovery::matching_delimiter(self.tokens, parameter_open)
        else {
            self.cursor = self.tokens.len();
            return;
        };
        if let Some(body_open) = Recovery::body_open(self.tokens, parameter_close + 1) {
            if let Some(body_close) = Recovery::matching_delimiter(self.tokens, body_open) {
                self.cursor = body_close + 1;
            } else {
                self.cursor = self.tokens.len();
            }
        } else {
            self.cursor = parameter_close + 1;
            self.skip_signature_tail();
        }
    }

    fn parse_function(&mut self) -> Option<FunctionItem> {
        let keyword_index = Recovery::function_keyword(self.tokens, self.cursor)?;
        self.cursor = keyword_index + 1;
        let Some(name) = self.take_identifier() else {
            self.push_current_issue(ParseIssueKind::UnexpectedToken, SYNTAX_FUNCTION_EXPECT_NAME);
            self.recover_unnamed_function();
            return None;
        };
        let Some(parameter_open) = Recovery::function_parameter_open(self.tokens, self.cursor)
        else {
            self.push_issue(
                ParseIssueKind::UnexpectedToken,
                self.tokens[keyword_index].start()..self.current_end(),
                SYNTAX_FUNCTION_MISSING_PARAMETERS,
            );
            self.recover_item();
            return None;
        };
        let Some(parameter_close) = Recovery::matching_delimiter(self.tokens, parameter_open)
        else {
            self.push_issue(
                ParseIssueKind::UnclosedDelimiter,
                self.tokens[parameter_open].start()..self.current_end(),
                SYNTAX_FUNCTION_PARAMETERS_UNCLOSED,
            );
            return None;
        };
        let parameters = self.parse_parameters(parameter_open + 1, parameter_close);
        let body_open = Recovery::body_open(self.tokens, parameter_close + 1);
        let signature_end = body_open
            .or_else(|| Recovery::semicolon(self.tokens, parameter_close + 1))
            .unwrap_or(parameter_close + 1);
        let mut signature_nodes =
            Recovery::split_top_level(self.tokens, parameter_open + 1, parameter_close, ",")
                .into_iter()
                .flat_map(|range| SyntaxCollector::collect(self.tokens, range))
                .collect::<Vec<_>>();
        signature_nodes.extend(SyntaxCollector::collect(
            self.tokens,
            parameter_close + 1..signature_end,
        ));
        let mut body = None;
        if let Some(open_index) = body_open {
            if let Some(close_index) = Recovery::matching_delimiter(self.tokens, open_index) {
                let saved_cursor = self.cursor;
                self.cursor = open_index + 1;
                let leading_attributes = AttributeParser::parse_attributes(
                    self.source,
                    self.tokens,
                    &mut self.cursor,
                    &mut self.issues,
                );
                let nodes_start = self.cursor;
                self.cursor = saved_cursor;
                let nodes = SyntaxCollector::collect(self.tokens, nodes_start..close_index);
                body = Some(AstFactory::body(
                    self.tokens[open_index].start()..self.tokens[close_index].end(),
                    leading_attributes,
                    nodes,
                ));
                self.cursor = close_index + 1;
            } else {
                self.cursor = self.tokens.len();
                self.push_issue(
                    ParseIssueKind::UnclosedDelimiter,
                    self.tokens[open_index].start()..self.current_end(),
                    SYNTAX_FUNCTION_BODY_UNCLOSED,
                );
            }
        } else {
            self.cursor = parameter_close + 1;
            self.skip_signature_tail();
        }
        Some(AstFactory::function(
            Some(name),
            parameters,
            body,
            signature_nodes,
        ))
    }

    fn parse_parameters(&self, start: usize, end: usize) -> Vec<Parameter> {
        Recovery::split_top_level(self.tokens, start, end, ",")
            .into_iter()
            .filter(|range| range.start < range.end)
            .map(|range| {
                let receiver = Recovery::receiver_parameter(self.tokens, &range);
                AstFactory::parameter(Recovery::token_bounds(self.tokens, range), receiver)
            })
            .collect()
    }

    fn parse_macro_item(&mut self) -> Option<MacroInvocation> {
        let (invocation, next_index) = SyntaxCollector::macro_item(self.tokens, self.cursor)?;
        self.cursor = next_index;
        Some(invocation)
    }

    fn parse_unsupported(
        &mut self,
        attributes: Vec<Attribute>,
        visibility: Visibility,
        item_start: usize,
    ) -> Option<Item> {
        if matches!(self.current_kind(), Some(TokenKind::CloseDelimiter(_))) {
            return None;
        }
        let start_index = self.cursor;
        let start = self.current_start();
        self.recover_item();
        let end = self
            .tokens
            .get(self.cursor.saturating_sub(1))
            .map_or(start, Token::end);
        self.push_issue(
            ParseIssueKind::UnsupportedSyntax,
            start..end,
            SYNTAX_ITEM_UNSUPPORTED,
        );
        if start_index == self.cursor {
            return None;
        }
        Some(AstFactory::item(
            attributes,
            visibility,
            ItemKind::Other(AstFactory::other(item_start..end)),
            item_start..end,
        ))
    }

    fn attributes_only_item(&mut self, attributes: Vec<Attribute>) -> Option<Item> {
        let start = attributes.first()?.start();
        let end = attributes.last()?.end();
        if attributes.iter().any(|attribute| !attribute.is_inner()) {
            self.push_issue(
                ParseIssueKind::UnsupportedSyntax,
                start..end,
                SYNTAX_ORPHANED_OUTER_ATTRIBUTE,
            );
        }
        let visibility = AstFactory::visibility(VisibilityKind::Private, start..start);
        Some(AstFactory::item(
            attributes,
            visibility,
            ItemKind::Other(AstFactory::other(start..end)),
            start..end,
        ))
    }

    fn skip_declaration_tail(&mut self, has_body: bool) -> Range<usize> {
        let start = self.cursor;
        let mut generic_depth = 0usize;
        while self.cursor < self.tokens.len() {
            match self.current_kind() {
                Some(TokenKind::Punctuation(value)) if value == "<" => {
                    generic_depth += 1;
                    self.cursor += 1;
                }
                Some(TokenKind::Punctuation(value)) if value == ">" => {
                    generic_depth = generic_depth.saturating_sub(1);
                    self.cursor += 1;
                }
                Some(TokenKind::Punctuation(value)) if value == ">>" => {
                    generic_depth = generic_depth.saturating_sub(2);
                    self.cursor += 1;
                }
                Some(TokenKind::OpenDelimiter(Delimiter::Brace))
                    if generic_depth == 0 && has_body =>
                {
                    if let Some(close_index) =
                        Recovery::matching_delimiter(self.tokens, self.cursor)
                    {
                        self.cursor = close_index + 1;
                        return start..self.cursor;
                    }
                    self.cursor = self.tokens.len();
                    self.push_issue(
                        ParseIssueKind::UnclosedDelimiter,
                        self.tokens[start].start()..self.current_end(),
                        SYNTAX_DECLARATION_BODY_UNCLOSED,
                    );
                    return start..self.cursor;
                }
                Some(TokenKind::OpenDelimiter(_)) => {
                    if let Some(close_index) =
                        Recovery::matching_delimiter(self.tokens, self.cursor)
                    {
                        self.cursor = close_index + 1;
                    } else {
                        self.cursor += 1;
                    }
                }
                Some(TokenKind::Punctuation(value)) if value == ";" => {
                    self.cursor += 1;
                    return start..self.cursor;
                }
                Some(TokenKind::CloseDelimiter(Delimiter::Brace)) | None => {
                    return start..self.cursor;
                }
                _ => self.cursor += 1,
            }
        }
        start..self.cursor
    }

    fn skip_signature_tail(&mut self) {
        while self.cursor < self.tokens.len() {
            if Recovery::is_punctuation(&self.tokens[self.cursor], ";") {
                self.cursor += 1;
                return;
            }
            if matches!(
                self.tokens[self.cursor].kind(),
                TokenKind::OpenDelimiter(Delimiter::Brace)
            ) {
                if let Some(close_index) = Recovery::matching_delimiter(self.tokens, self.cursor) {
                    self.cursor = close_index + 1;
                    return;
                }
            }
            self.cursor += 1;
        }
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

    fn push_current_issue(&mut self, kind: ParseIssueKind, message: MessageId) {
        self.push_issue(kind, self.current_start()..self.current_end(), message);
    }

    fn push_issue(&mut self, kind: ParseIssueKind, bounds: Range<usize>, message: MessageId) {
        self.issues.push(AstFactory::parse_issue(
            kind,
            bounds,
            LocalizedMessage::new(message),
        ));
    }

    fn consume_close(&mut self, delimiter: Delimiter) -> Option<usize> {
        if matches!(
            self.current_kind(),
            Some(TokenKind::CloseDelimiter(found)) if *found == delimiter
        ) {
            let close = self.cursor;
            self.cursor += 1;
            Some(close)
        } else {
            None
        }
    }

    fn consume_identifier(&mut self, expected: &str) -> bool {
        if self.current_identifier() == Some(expected) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn take_identifier(&mut self) -> Option<String> {
        match self.current_kind()?.clone() {
            TokenKind::Identifier(identifier) => {
                self.cursor += 1;
                Some(identifier)
            }
            _ => None,
        }
    }

    fn take_identifier_at(&self, index: &mut usize, limit: usize) -> Option<String> {
        let identifier = match self.tokens.get(*index)?.kind() {
            TokenKind::Identifier(identifier) if *index < limit => identifier.clone(),
            _ => return None,
        };
        *index += 1;
        Some(identifier)
    }

    fn consume_punctuation(&mut self, expected: &str) -> bool {
        if self
            .current_token()
            .is_some_and(|token| Recovery::is_punctuation(token, expected))
        {
            self.cursor += 1;
            true
        } else {
            false
        }
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
