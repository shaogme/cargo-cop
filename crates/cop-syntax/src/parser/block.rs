mod patterns;
mod statement;

use std::ops::Range;

use cop_model::{
    LocalizedMessage, SYNTAX_INNER_ATTRIBUTE_POSITION, SYNTAX_ORPHANED_OUTER_ATTRIBUTE,
    SYNTAX_UNEXPECTED_TOKEN,
};

use crate::{
    ast::{
        AstFactory, AstNodeId, Attribute, ParseIssue, ParseIssueCode, ParseIssueKind,
        StatementSyntaxKind, SyntaxNode, SyntaxNodeData, SyntaxNodeKind,
    },
    recovery::Recovery,
    Delimiter, RustEdition, SourceFile, Token, TokenKind,
};

use super::{
    attributes::AttributeParser,
    expression::{SyntaxCollector, SyntaxRequest},
    items::Parser,
    token_cursor::{SyntaxErrorKind, TokenCursor},
    use_tree::{UseTreeInput, UseTreeParser},
};

use self::{
    patterns::{BlockPatternInput, PatternCollector},
    statement::{StatementForm, StatementParser},
};

pub(super) struct BlockParser;

pub(super) struct LocalItemInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) edition: RustEdition,
    pub(super) start: usize,
    pub(super) parent: AstNodeId,
    pub(super) attributes: Vec<Attribute>,
    pub(super) ast_nodes: &'a mut Vec<SyntaxNode>,
    pub(super) issues: &'a mut Vec<ParseIssue>,
}

pub(super) struct BlockInput<'a> {
    pub(super) source: &'a SourceFile,
    pub(super) tokens: &'a [Token],
    pub(super) edition: RustEdition,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) parent: AstNodeId,
}

struct BlockParseContext<'a> {
    source: &'a SourceFile,
    tokens: &'a [Token],
    edition: RustEdition,
    issues: &'a mut Vec<ParseIssue>,
    ast_nodes: &'a mut Vec<SyntaxNode>,
}

impl BlockParser {
    pub(super) fn parse(
        input: BlockInput<'_>,
        ast_nodes: &mut Vec<SyntaxNode>,
        issues: &mut Vec<ParseIssue>,
    ) {
        let BlockInput {
            source,
            tokens,
            edition,
            start,
            end,
            parent,
        } = input;
        let mut context = BlockParseContext {
            source,
            tokens,
            edition,
            issues,
            ast_nodes,
        };
        Self::parse_scope(&mut context, start..end, parent);
    }

    fn parse_scope(context: &mut BlockParseContext<'_>, bounds: Range<usize>, parent: AstNodeId) {
        let mut cursor = bounds.start;
        let attributes = AttributeParser::parse_attributes(
            context.source,
            context.tokens,
            &mut cursor,
            context.issues,
        );
        let inner_attributes = attributes
            .iter()
            .filter(|attribute| attribute.is_inner())
            .cloned()
            .collect::<Vec<_>>();
        if !inner_attributes.is_empty() {
            AstFactory::set_ast_node_attributes(context.ast_nodes, parent, inner_attributes);
        }
        let scope_parent = parent;
        if attributes.iter().any(|attribute| !attribute.is_inner()) {
            cursor = attributes
                .iter()
                .find(|attribute| !attribute.is_inner())
                .and_then(|attribute| {
                    context
                        .tokens
                        .iter()
                        .position(|token| token.start() == attribute.start())
                })
                .unwrap_or(cursor);
        } else if attributes.is_empty() || !attributes.iter().any(|attribute| attribute.is_inner())
        {
            cursor = bounds.start;
        }

        while cursor < bounds.end {
            if Recovery::is_punctuation(&context.tokens[cursor], ";") {
                let token = &context.tokens[cursor];
                let statement = AstFactory::push_ast_node(
                    context.ast_nodes,
                    Some(scope_parent),
                    SyntaxNodeKind::Statement,
                    token.start()..token.end(),
                    Vec::new(),
                );
                AstFactory::set_ast_node_data(
                    context.ast_nodes,
                    statement,
                    SyntaxNodeData::Statement(StatementSyntaxKind::Empty),
                );
                cursor += 1;
                continue;
            }
            if Recovery::is_punctuation(&context.tokens[cursor], ",") {
                let token = &context.tokens[cursor];
                AstFactory::push_ast_node(
                    context.ast_nodes,
                    Some(scope_parent),
                    SyntaxNodeKind::Opaque,
                    token.start()..token.end(),
                    Vec::new(),
                );
                context.issues.push(AstFactory::parse_issue(
                    ParseIssueKind::UnexpectedToken,
                    token.start()..token.end(),
                    LocalizedMessage::new(SYNTAX_UNEXPECTED_TOKEN),
                ));
                cursor += 1;
                continue;
            }
            if cursor >= bounds.end {
                break;
            }
            let mut attribute_cursor = cursor;
            let attributes = AttributeParser::parse_attributes(
                context.source,
                context.tokens,
                &mut attribute_cursor,
                context.issues,
            );
            if attribute_cursor >= bounds.end {
                if !attributes.is_empty() {
                    for attribute in attributes {
                        let attribute_bounds = attribute.start()..attribute.end();
                        AstFactory::push_ast_node(
                            context.ast_nodes,
                            Some(scope_parent),
                            SyntaxNodeKind::Opaque,
                            attribute_bounds.clone(),
                            vec![attribute.clone()],
                        );
                        let message = if attribute.is_inner() {
                            SYNTAX_INNER_ATTRIBUTE_POSITION
                        } else {
                            SYNTAX_ORPHANED_OUTER_ATTRIBUTE
                        };
                        context.issues.push(AstFactory::parse_issue(
                            ParseIssueKind::UnexpectedToken,
                            attribute_bounds,
                            LocalizedMessage::new(message),
                        ));
                    }
                }
                break;
            }
            let start_index = attribute_cursor;
            let kind = Self::statement_kind(context.tokens, start_index);
            if kind == SyntaxNodeKind::LocalItem {
                let statement_end = Parser::parse_local_item(LocalItemInput {
                    source: context.source,
                    tokens: &context.tokens[..bounds.end],
                    edition: context.edition,
                    start: start_index,
                    parent: scope_parent,
                    attributes,
                    ast_nodes: context.ast_nodes,
                    issues: context.issues,
                });
                cursor = if statement_end > start_index {
                    statement_end
                } else {
                    start_index + 1
                };
                continue;
            }
            let parsed = match StatementParser::parse(
                context.source,
                context.tokens,
                start_index,
                bounds.end,
            ) {
                Ok(parsed) if parsed.end > start_index => parsed,
                Ok(_) => {
                    cursor = start_index + 1;
                    continue;
                }
                Err(error) => {
                    let issue_kind = if error.kind() == SyntaxErrorKind::UnclosedDelimiter {
                        ParseIssueKind::UnclosedDelimiter
                    } else {
                        ParseIssueKind::UnexpectedToken
                    };
                    context.issues.push(AstFactory::parse_issue_with_boundary(
                        issue_kind,
                        error.bounds(),
                        LocalizedMessage::new(SYNTAX_UNEXPECTED_TOKEN),
                        error.kind() == SyntaxErrorKind::UnclosedDelimiter,
                    ));
                    cursor = Self::recover_statement(context.tokens, start_index, bounds.end);
                    continue;
                }
            };
            let statement_end = parsed.end;
            let start = attributes.first().map_or_else(
                || token_start(context.tokens, start_index),
                Attribute::start,
            );
            let end = context.tokens[statement_end - 1].end();
            let node_id = AstFactory::push_ast_node(
                context.ast_nodes,
                Some(scope_parent),
                kind,
                start..end,
                attributes,
            );
            let first = context.tokens[start_index].text(context.source);
            let statement_kind = match parsed.form {
                StatementForm::Let => StatementSyntaxKind::Let,
                StatementForm::Use => StatementSyntaxKind::Use,
                StatementForm::Expression => StatementSyntaxKind::Expression {
                    has_semicolon: parsed.has_semicolon,
                    is_tail: parsed.is_tail,
                },
            };
            AstFactory::set_ast_node_data(
                context.ast_nodes,
                node_id,
                SyntaxNodeData::Statement(statement_kind),
            );
            if matches!(parsed.form, StatementForm::Let | StatementForm::Use)
                && !parsed.has_semicolon
            {
                let issue_offset = context.tokens.get(statement_end).map_or(end, Token::start);
                context.issues.push(AstFactory::parse_issue_with_code(
                    ParseIssueKind::UnexpectedToken,
                    ParseIssueCode::UnexpectedToken,
                    issue_offset..issue_offset,
                    LocalizedMessage::new(SYNTAX_UNEXPECTED_TOKEN),
                    false,
                ));
            }
            if matches!(parsed.form, StatementForm::Expression)
                && !parsed.has_semicolon
                && !parsed.is_tail
                && !parsed.block_like
            {
                context.issues.push(AstFactory::parse_issue_with_code(
                    ParseIssueKind::UnexpectedToken,
                    ParseIssueCode::UnexpectedToken,
                    end..end,
                    LocalizedMessage::new(SYNTAX_UNEXPECTED_TOKEN),
                    false,
                ));
            }
            if kind == SyntaxNodeKind::Statement && first == "use" {
                let tree_end = if parsed.has_semicolon {
                    statement_end - 1
                } else {
                    statement_end
                };
                UseTreeParser::parse(
                    UseTreeInput {
                        source: context.source,
                        tokens: context.tokens,
                        start: start_index + 1,
                        limit: tree_end,
                        parent: node_id,
                    },
                    context.ast_nodes,
                    context.issues,
                );
            } else {
                if kind == SyntaxNodeKind::Statement {
                    PatternCollector::collect_statement_pattern(BlockPatternInput {
                        source: context.source,
                        tokens: context.tokens,
                        edition: context.edition,
                        parent: node_id,
                        start: start_index,
                        end: statement_end,
                        ast_nodes: context.ast_nodes,
                        issues: context.issues,
                    });
                    PatternCollector::collect_let_type(BlockPatternInput {
                        source: context.source,
                        tokens: context.tokens,
                        edition: context.edition,
                        parent: node_id,
                        start: start_index,
                        end: statement_end,
                        ast_nodes: context.ast_nodes,
                        issues: context.issues,
                    });
                }
                if kind == SyntaxNodeKind::MatchArm {
                    PatternCollector::collect_match_pattern(BlockPatternInput {
                        source: context.source,
                        tokens: context.tokens,
                        edition: context.edition,
                        parent: node_id,
                        start: start_index,
                        end: statement_end,
                        ast_nodes: context.ast_nodes,
                        issues: context.issues,
                    });
                }
                let generated_start = context.ast_nodes.len();
                if let Some(expression) = parsed.expression {
                    let _expression_parent = SyntaxCollector::collect_expression(
                        SyntaxRequest {
                            source: context.source,
                            tokens: context.tokens,
                            bounds: expression,
                            parent: node_id,
                            expression_tree: true,
                            edition: Some(context.edition),
                        },
                        context.ast_nodes,
                        context.issues,
                    );
                }
                Self::push_let_else_block(context, parsed.let_else_block, node_id);
                Self::parse_embedded_blocks(context, generated_start);
            }
            cursor = statement_end;
            if cursor < bounds.end
                && (Recovery::is_punctuation(&context.tokens[cursor], ";")
                    || Recovery::is_punctuation(&context.tokens[cursor], ","))
            {
                cursor += 1;
            }
        }
    }

    fn parse_embedded_blocks(context: &mut BlockParseContext<'_>, first_node: usize) {
        let mut index = first_node;
        while index < context.ast_nodes.len() {
            let node = &context.ast_nodes[index];
            let node_kind = node.kind();
            let node_id = node.id();
            let node_start = node.start();
            let node_end = node.end();
            if node_kind == SyntaxNodeKind::Block {
                if let Some(open) = context
                    .tokens
                    .iter()
                    .position(|token| token.start() == node_start)
                {
                    if let Some(close) = Recovery::matching_delimiter(context.tokens, open) {
                        if context.tokens[close].end() == node_end {
                            Self::parse_scope(context, open + 1..close, node_id);
                        }
                    }
                }
            }
            index += 1;
        }
    }

    fn recover_statement(tokens: &[Token], start: usize, end: usize) -> usize {
        let Some(mut cursor) = TokenCursor::new(tokens, start..end) else {
            return start.saturating_add(1).min(end);
        };
        cursor.recover_to(&[";"]);
        if cursor.at_punctuation(";") {
            cursor.consume_punctuation(";");
        }
        if cursor.index() == start && !cursor.is_empty() {
            cursor.consume();
        }
        cursor.index()
    }

    fn push_let_else_block(
        context: &mut BlockParseContext<'_>,
        block: Option<Range<usize>>,
        parent: AstNodeId,
    ) {
        let Some(block) = block else {
            return;
        };
        let Some(first) = context.tokens.get(block.start) else {
            return;
        };
        let Some(last) = block
            .end
            .checked_sub(1)
            .and_then(|index| context.tokens.get(index))
        else {
            return;
        };
        AstFactory::push_ast_node(
            context.ast_nodes,
            Some(parent),
            SyntaxNodeKind::Block,
            first.start()..last.end(),
            Vec::new(),
        );
    }

    fn statement_kind(tokens: &[Token], start: usize) -> SyntaxNodeKind {
        if Self::local_item_keyword(tokens, start).is_some() {
            SyntaxNodeKind::LocalItem
        } else {
            SyntaxNodeKind::Statement
        }
    }

    fn local_item_keyword(tokens: &[Token], start: usize) -> Option<(usize, String)> {
        if SyntaxCollector::macro_definition_end(tokens, start).is_some() {
            return Some((start, "macro_rules".to_owned()));
        }
        let mut index = start;
        if Recovery::is_identifier(tokens.get(index)?, "pub") {
            index += 1;
            if matches!(
                tokens.get(index).map(Token::kind),
                Some(TokenKind::OpenDelimiter(Delimiter::Parenthesis))
            ) {
                index = Recovery::matching_delimiter(tokens, index)? + 1;
            }
        }
        if Recovery::is_identifier(tokens.get(index)?, "extern")
            && tokens
                .get(index + 1)
                .is_some_and(|token| Recovery::is_identifier(token, "crate"))
        {
            return Some((index, "extern crate".to_owned()));
        }
        while let Some(token) = tokens.get(index) {
            if matches!(token.kind(), TokenKind::EditionKeyword(value) if value == "async") {
                index += 1;
                continue;
            }
            if let TokenKind::Identifier(identifier) = token.kind() {
                if identifier == "const"
                    && tokens
                        .get(index + 1)
                        .is_some_and(|next| Recovery::is_identifier(next, "fn"))
                {
                    index += 1;
                    continue;
                }
                if identifier == "const"
                    && matches!(
                        tokens.get(index + 1).map(Token::kind),
                        Some(TokenKind::OpenDelimiter(Delimiter::Brace))
                    )
                {
                    return None;
                }
                if matches!(
                    identifier.as_str(),
                    "fn" | "mod"
                        | "trait"
                        | "impl"
                        | "struct"
                        | "enum"
                        | "union"
                        | "type"
                        | "const"
                        | "static"
                ) {
                    return Some((index, identifier.clone()));
                }
                if matches!(
                    identifier.as_str(),
                    "const" | "unsafe" | "default" | "auto" | "extern"
                ) {
                    index += 1;
                    if identifier == "extern"
                        && matches!(
                            tokens.get(index).map(Token::kind),
                            Some(TokenKind::Literal(_))
                        )
                    {
                        index += 1;
                    }
                    continue;
                }
            }
            return None;
        }
        None
    }
}

fn token_start(tokens: &[Token], index: usize) -> usize {
    tokens.get(index).map_or(0, Token::start)
}
