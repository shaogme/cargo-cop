mod attributes;
mod block;
mod declarations;
mod expression;
mod generic_parameters;
mod items;
mod parameters;
mod path;
mod patterns;
mod token_cursor;
mod types;
mod use_tree;
mod where_clause;

use cop_i18n::{t, RenderError};

use crate::{
    ast::{
        AstFactory, AstNodeId, AttributeMeta, AttributeMetaKind, ParseIssue, ParseIssueKind,
        ParseOutput, ParseOutputParts, SyntaxNode, SyntaxNodeKind,
    },
    recovery::Recovery,
    RustEdition, RustLexer, SourceFile,
};

use self::items::{Parser, ParserInput};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseOptions {
    edition: String,
}

impl ParseOptions {
    pub fn new(edition: impl Into<String>) -> Self {
        Self {
            edition: edition.into(),
        }
    }

    pub fn edition(&self) -> &str {
        &self.edition
    }

    fn rust_edition(&self) -> Option<RustEdition> {
        RustEdition::parse(&self.edition)
    }
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self::new("2021")
    }
}

pub struct RustParser;

impl RustParser {
    pub fn parse(source: SourceFile, options: ParseOptions) -> Result<ParseOutput, RenderError> {
        let edition = options.rust_edition();
        let lexed = RustLexer::lex_for_edition(&source, edition)?;
        let mut issues = lexed
            .issues()
            .iter()
            .map(ParseIssue::from)
            .collect::<Vec<_>>();
        let mut ast_nodes = Vec::new();
        let root_id = AstFactory::push_ast_node(
            &mut ast_nodes,
            None,
            SyntaxNodeKind::Crate,
            0..source.len(),
            Vec::new(),
        );
        let inner_attributes = if let Some(edition) = edition {
            let mut parser = Parser::new(ParserInput {
                source: &source,
                tokens: lexed.tokens(),
                edition,
                cursor: 0,
                issues,
                ast_nodes,
            });
            let scope = parser.parse_items(false, root_id);
            let (parsed_nodes, parsed_issues) = parser.into_parts();
            ast_nodes = parsed_nodes;
            issues = parsed_issues;
            scope.inner_attributes
        } else {
            issues.push(AstFactory::parse_issue(
                ParseIssueKind::UnsupportedEdition,
                0..0,
                t!("syntax.unsupported_edition", { edition: options.edition() }),
            ));
            Vec::new()
        };
        if edition == Some(RustEdition::E2024) {
            Self::record_unsafe_attribute_issues(&ast_nodes, &mut issues);
        }
        issues.extend(Recovery::delimiter_issues(lexed.tokens()));
        issues.sort_by(|left, right| {
            left.start()
                .cmp(&right.start())
                .then_with(|| left.end().cmp(&right.end()))
        });
        issues.dedup_by(|left, right| left.kind() == right.kind() && left.start() == right.start());
        Self::record_uncovered_ranges(&mut ast_nodes, root_id, &issues);
        AstFactory::assign_issue_sources(&ast_nodes, &mut issues);
        AstFactory::update_ast_node(
            &mut ast_nodes,
            root_id,
            SyntaxNodeKind::Crate,
            0..source.len(),
            inner_attributes,
        );
        AstFactory::apply_issues(&mut ast_nodes, &issues);
        for issue in &issues {
            issue.message()?;
        }
        let coverage = ast_nodes
            .iter()
            .map(SyntaxNode::coverage)
            .fold(Recovery::coverage_for(&issues), AstFactory::merge_coverage);
        Ok(ParseOutput::new(ParseOutputParts {
            source,
            lexed,
            ast_nodes,
            issues,
            coverage,
        }))
    }

    fn record_uncovered_ranges(
        ast_nodes: &mut Vec<SyntaxNode>,
        root_id: AstNodeId,
        issues: &[ParseIssue],
    ) {
        for issue in issues {
            if ast_nodes.iter().any(|node| {
                node.kind() == SyntaxNodeKind::Opaque
                    && node.start() <= issue.start()
                    && issue.end() <= node.end()
            }) {
                continue;
            }
            let parent = ast_nodes
                .iter()
                .filter(|node| {
                    node.kind() != SyntaxNodeKind::Opaque
                        && node.start() <= issue.start()
                        && issue.end() <= node.end()
                })
                .min_by_key(|node| node.end().saturating_sub(node.start()))
                .map_or(root_id, SyntaxNode::id);
            AstFactory::push_ast_node(
                ast_nodes,
                Some(parent),
                SyntaxNodeKind::Opaque,
                issue.start()..issue.end(),
                Vec::new(),
            );
        }
    }

    fn record_unsafe_attribute_issues(ast_nodes: &[SyntaxNode], issues: &mut Vec<ParseIssue>) {
        for attribute in ast_nodes.iter().flat_map(SyntaxNode::attributes) {
            let requires_unsafe = matches!(
                attribute.path(),
                [name] if matches!(name.as_str(), "no_mangle" | "export_name" | "link_section")
            );
            let wrapped = if attribute.path() == ["unsafe"] {
                matches!(attribute.meta().kind(), AttributeMetaKind::List(items)
                if items.iter().any(|item| {
                    item.meta().is_some_and(|meta| Self::is_unsafe_attribute_name(meta.path()))
                }))
            } else {
                false
            };
            let unwrapped_conditional = attribute.path().len() == 1
                && attribute.path()[0] == "cfg_attr"
                && attribute
                    .meta()
                    .arguments()
                    .iter()
                    .skip(1)
                    .filter_map(|argument| argument.meta())
                    .any(Self::contains_unwrapped_unsafe_attribute);
            if (!requires_unsafe || wrapped) && !unwrapped_conditional {
                continue;
            }
            issues.push(AstFactory::parse_issue(
                ParseIssueKind::UnexpectedToken,
                attribute.start()..attribute.end(),
                t!("syntax.parse_error_unexpected_token"),
            ));
        }
    }

    fn is_unsafe_attribute_name(path: &[String]) -> bool {
        matches!(
            path,
            [name] if matches!(name.as_str(), "no_mangle" | "export_name" | "link_section")
        )
    }

    fn contains_unwrapped_unsafe_attribute(meta: &AttributeMeta) -> bool {
        if Self::is_unsafe_attribute_name(meta.path()) {
            return true;
        }
        meta.name() == Some("cfg_attr")
            && meta
                .arguments()
                .iter()
                .skip(1)
                .filter_map(|argument| argument.meta())
                .any(Self::contains_unwrapped_unsafe_attribute)
    }
}
