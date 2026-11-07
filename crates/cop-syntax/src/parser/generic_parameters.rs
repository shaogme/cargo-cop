use cop_i18n::t;

use crate::{
    ast::{
        AstFactory, AstNodeId, GenericParameterKind, GenericParameterParts, GenericParameterSyntax,
        Item, ItemKind, ParseIssue, ParseIssueKind, SyntaxNode, SyntaxNodeData, SyntaxNodeKind,
    },
    recovery::Recovery,
    SourceFile, Token, TokenKind,
};

use super::{
    attributes::AttributeParser,
    path::PathParser,
    token_cursor::{CursorRange, SyntaxError, SyntaxErrorKind, TokenCursor},
    types::TypeParser,
};

pub(super) struct GenericParameterParser;

pub(super) struct GenericParameterInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) parent: AstNodeId,
    pub(super) item: &'a Item,
    pub(super) ast_nodes: &'a mut Vec<SyntaxNode>,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

struct GenericParameterNodeInput<'a> {
    source: &'a SourceFile,
    tokens: &'a [Token],
    parent: AstNodeId,
    range: CursorRange,
    ast_nodes: &'a mut Vec<SyntaxNode>,
    issues: &'a mut Vec<ParseIssue>,
}

impl GenericParameterParser {
    pub(super) fn collect(input: GenericParameterInput<'_>) {
        let GenericParameterInput {
            source,
            tokens,
            parent,
            item,
            ast_nodes,
            issues,
        } = input;
        let keyword = match item.kind() {
            ItemKind::Function(_) => "fn",
            ItemKind::Struct(_) => "struct",
            ItemKind::Enum(_) => "enum",
            ItemKind::Union(_) => "union",
            ItemKind::TypeAlias(_) => "type",
            ItemKind::Trait(_) => "trait",
            ItemKind::Impl(_) => "impl",
            _ => return,
        };
        let Some(first) = tokens
            .iter()
            .position(|token| token.start() >= item.start() && token.end() <= item.end())
        else {
            return;
        };
        let after = tokens
            .iter()
            .enumerate()
            .skip(first)
            .find(|(_, token)| token.start() >= item.end())
            .map_or(tokens.len(), |(index, _)| index);
        let Some(keyword_index) = (first..after).find(|index| {
            matches!(tokens[*index].kind(), TokenKind::Identifier(value) if value == keyword)
        }) else {
            return;
        };
        let open = if keyword == "impl" {
            keyword_index + 1
        } else {
            keyword_index + 2
        };
        if !tokens
            .get(open)
            .is_some_and(|token| Recovery::is_punctuation(token, "<"))
        {
            return;
        }
        let Some(mut cursor) = TokenCursor::from_token_range(tokens, open..after) else {
            return;
        };
        match cursor.split_angle_list() {
            Ok(scan) => {
                for range in scan.parameters() {
                    Self::push_parameter(GenericParameterNodeInput {
                        source,
                        tokens,
                        parent,
                        range: *range,
                        ast_nodes,
                        issues,
                    });
                }
                for error in scan.errors() {
                    Self::record_error(error.clone(), parent, ast_nodes, issues);
                }
            }
            Err(scan_error) => {
                for range in scan_error.completed() {
                    Self::push_parameter(GenericParameterNodeInput {
                        source,
                        tokens,
                        parent,
                        range: *range,
                        ast_nodes,
                        issues,
                    });
                }
                for error in scan_error.recoverable_errors() {
                    Self::record_error(error.clone(), parent, ast_nodes, issues);
                }
                if let Some(range) = scan_error.remainder() {
                    Self::push_unparsed_parameter(GenericParameterNodeInput {
                        source,
                        tokens,
                        parent,
                        range,
                        ast_nodes,
                        issues,
                    });
                }
                Self::record_error(scan_error.error(), parent, ast_nodes, issues);
            }
        }
    }

    fn push_unparsed_parameter(input: GenericParameterNodeInput<'_>) {
        let GenericParameterNodeInput {
            source,
            tokens,
            parent,
            range,
            ast_nodes,
            issues,
        } = input;
        let byte_bounds = range.byte_bounds();
        if byte_bounds.is_empty() {
            return;
        }
        let mut attribute_cursor = range.start().token_index();
        let attributes =
            AttributeParser::parse_attributes(source, tokens, &mut attribute_cursor, issues);
        let start = attributes
            .first()
            .map_or(byte_bounds.start, |attribute| attribute.start());
        AstFactory::push_ast_node(
            ast_nodes,
            Some(parent),
            SyntaxNodeKind::GenericParameter,
            start..byte_bounds.end,
            attributes,
        );
    }

    fn push_parameter(input: GenericParameterNodeInput<'_>) {
        let GenericParameterNodeInput {
            source,
            tokens,
            parent,
            range,
            ast_nodes,
            issues,
        } = input;
        let byte_bounds = range.byte_bounds();
        if byte_bounds.is_empty() {
            return;
        }
        let mut attribute_cursor = range.start().token_index();
        let attributes =
            AttributeParser::parse_attributes(source, tokens, &mut attribute_cursor, issues);
        let node_start = attributes
            .first()
            .map_or(byte_bounds.start, |attribute| attribute.start());
        let parameter_id = AstFactory::push_ast_node(
            ast_nodes,
            Some(parent),
            SyntaxNodeKind::GenericParameter,
            node_start..byte_bounds.end,
            attributes,
        );
        let Some(syntax_start) = CursorRange::from_token_range(
            tokens,
            attribute_cursor..range.start().token_index().max(attribute_cursor),
        )
        .map(|attribute_range| attribute_range.end()) else {
            return;
        };
        let Some(syntax_range) = CursorRange::new(syntax_start, range.end()) else {
            return;
        };
        let Some(parameter_cursor) =
            TokenCursor::new(tokens, range).and_then(|cursor| cursor.subcursor(syntax_range))
        else {
            return;
        };
        match Self::parse_parameter_syntax(parameter_cursor) {
            Ok(syntax) => AstFactory::set_ast_node_data(
                ast_nodes,
                parameter_id,
                SyntaxNodeData::GenericParameter(syntax),
            ),
            Err(error) => Self::record_error(error, parameter_id, ast_nodes, issues),
        }
    }

    fn parse_parameter_syntax(
        mut cursor: TokenCursor<'_>,
    ) -> Result<GenericParameterSyntax, SyntaxError> {
        let start_offset = cursor.offset();
        let (kind, name) = match cursor.consume().map(Token::kind) {
            Some(TokenKind::Lifetime(name)) => (GenericParameterKind::Lifetime, name.clone()),
            Some(TokenKind::Identifier(value)) if value == "const" => {
                let name = match cursor.consume().map(Token::kind) {
                    Some(TokenKind::Identifier(name)) => name.clone(),
                    _ => {
                        return Err(PathParser::error_at(
                            &cursor,
                            SyntaxErrorKind::UnexpectedToken,
                        ))
                    }
                };
                (GenericParameterKind::Const, name)
            }
            Some(TokenKind::Identifier(name)) => (GenericParameterKind::Type, name.clone()),
            _ => {
                return Err(PathParser::error_at(
                    &cursor,
                    SyntaxErrorKind::UnexpectedToken,
                ))
            }
        };
        let mut trait_bounds = Vec::new();
        let mut lifetime_bounds = Vec::new();
        let mut declared_type = None;
        let mut default_type = None;
        let mut default_const = None;
        if cursor.consume_punctuation(":") {
            if kind == GenericParameterKind::Const {
                declared_type = Some(TypeParser::parse(&mut cursor)?);
            } else if kind == GenericParameterKind::Lifetime {
                loop {
                    match cursor.peek().map(Token::kind) {
                        Some(TokenKind::Lifetime(lifetime)) => {
                            lifetime_bounds.push(lifetime.clone());
                            cursor.consume();
                        }
                        _ => {
                            return Err(PathParser::error_at(
                                &cursor,
                                SyntaxErrorKind::UnexpectedToken,
                            ))
                        }
                    }
                    if !cursor.consume_punctuation("+") {
                        break;
                    }
                }
            } else {
                trait_bounds = TypeParser::parse_bounds(&mut cursor)?;
            }
        }
        if cursor.consume_punctuation("=") {
            if kind == GenericParameterKind::Const {
                let start = cursor.offset();
                while !cursor.is_empty() {
                    if matches!(
                        cursor.peek().map(Token::kind),
                        Some(TokenKind::OpenDelimiter(_))
                    ) {
                        if cursor.skip_delimited_group().is_none() {
                            return Err(PathParser::error_at(
                                &cursor,
                                SyntaxErrorKind::UnclosedDelimiter,
                            ));
                        }
                    } else {
                        cursor.consume();
                    }
                }
                default_const = Some(start..cursor.consumed_offset());
            } else {
                default_type = Some(TypeParser::parse(&mut cursor)?);
            }
        }
        if !cursor.is_empty() {
            return Err(PathParser::error_at(
                &cursor,
                SyntaxErrorKind::UnexpectedToken,
            ));
        }
        let end_offset = cursor.consumed_position().byte_offset();
        Ok(GenericParameterSyntax::new(GenericParameterParts {
            kind,
            name,
            trait_bounds,
            lifetime_bounds,
            declared_type,
            default_type,
            default_const,
            bounds_range: start_offset..end_offset,
        }))
    }

    fn record_error(
        error: SyntaxError,
        parent: AstNodeId,
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) {
        let bounds = error.bounds();
        let kind = if error.kind() == SyntaxErrorKind::UnclosedDelimiter {
            ParseIssueKind::UnclosedDelimiter
        } else {
            ParseIssueKind::UnexpectedToken
        };
        if !bounds.is_empty() {
            AstFactory::push_ast_node(
                ast_nodes,
                Some(parent),
                SyntaxNodeKind::Opaque,
                bounds.clone(),
                Vec::new(),
            );
        }
        issues.push(AstFactory::parse_issue(
            kind,
            bounds,
            t!("syntax.item_unsupported"),
        ));
    }
}
