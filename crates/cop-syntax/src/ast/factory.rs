use std::ops::Range;

use cop_model::{
    LocalizedMessage, MessageArgument, MessageId, SYNTAX_MACRO_OPAQUE,
    SYNTAX_PARSE_ERROR_INVALID_ATTRIBUTE, SYNTAX_PARSE_ERROR_LEXICAL_ERROR,
    SYNTAX_PARSE_ERROR_UNCLOSED_DELIMITER, SYNTAX_PARSE_ERROR_UNEXPECTED_EOF,
    SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN, SYNTAX_PARSE_ERROR_UNSUPPORTED_EDITION,
    SYNTAX_UNCLOSED_BLOCK_COMMENT, SYNTAX_UNCLOSED_LITERAL, SYNTAX_UNCLOSED_RAW_STRING,
    SYNTAX_UNSUPPORTED_ASSOCIATED_ITEM, SYNTAX_UNSUPPORTED_ATTRIBUTE,
    SYNTAX_UNSUPPORTED_EXPRESSION, SYNTAX_UNSUPPORTED_ITEM, SYNTAX_UNSUPPORTED_PATTERN,
    SYNTAX_UNSUPPORTED_STATEMENT, SYNTAX_UNSUPPORTED_TOKEN, SYNTAX_UNSUPPORTED_TYPE,
    SYNTAX_UNSUPPORTED_USE_TREE, SYNTAX_UNSUPPORTED_VISIBILITY,
};

use super::{
    AstNodeId, Attribute, AttributeInput, CoverageStatus, ExternBlockItem, ExternCrateItem,
    FunctionItem, ImplItem, ImplSyntaxData, Item, ItemKind, ItemSyntaxData, MacroDefinition,
    MacroInput, MacroInputStatus, MacroInvocation, ModuleItem, NamedDeclaration, Parameter,
    ParseIssue, ParseIssueCode, ParseIssueKind, PathSyntax, SyntaxNode, SyntaxNodeData,
    SyntaxNodeKind, Token, TraitItem, TypeSyntax, UnsupportedConstructKind, Visibility,
    VisibilityKind,
};

pub(crate) struct AstFactory;

impl AstFactory {
    pub(crate) fn attribute(input: AttributeInput) -> Attribute {
        Attribute::new(input)
    }

    pub(crate) fn visibility(kind: VisibilityKind, bounds: Range<usize>) -> Visibility {
        Visibility::new(kind, bounds)
    }

    pub(crate) fn item(
        attributes: Vec<Attribute>,
        visibility: Visibility,
        kind: ItemKind,
        bounds: Range<usize>,
    ) -> Item {
        Item::new(attributes, visibility, kind, bounds)
    }

    pub(crate) fn module(name: String, inline: bool) -> ModuleItem {
        ModuleItem::new(name, inline)
    }

    pub(crate) fn extern_crate(name: Option<String>, alias: Option<String>) -> ExternCrateItem {
        ExternCrateItem::new(name, alias)
    }

    pub(crate) fn extern_block(unsafe_: bool, abi: Option<String>) -> ExternBlockItem {
        ExternBlockItem::new(unsafe_, abi)
    }

    pub(crate) fn named_declaration(name: Option<String>) -> NamedDeclaration {
        NamedDeclaration::new(name)
    }

    pub(crate) fn trait_item(name: Option<String>) -> TraitItem {
        TraitItem::new(name)
    }

    pub(crate) fn impl_item() -> ImplItem {
        ImplItem::new()
    }

    pub(crate) fn function(name: Option<String>) -> FunctionItem {
        FunctionItem::new(name)
    }

    pub(crate) fn parameter(
        attributes: Vec<Attribute>,
        bounds: Range<usize>,
        receiver: bool,
    ) -> Parameter {
        Parameter::new(attributes, bounds, receiver)
    }

    pub(crate) fn impl_syntax(
        self_type: Option<TypeSyntax>,
        trait_path: Option<PathSyntax>,
        trait_impl: bool,
    ) -> ImplSyntaxData {
        ImplSyntaxData::new(self_type, trait_path, trait_impl)
    }

    pub(crate) fn macro_invocation(
        path: PathSyntax,
        bounds: Range<usize>,
        tokens: &[Token],
        input_bounds: Range<usize>,
        status: MacroInputStatus,
    ) -> MacroInvocation {
        MacroInvocation::new(
            path,
            bounds,
            MacroInput::from_tokens(tokens, input_bounds, status),
        )
    }

    pub(crate) fn macro_definition(bounds: Range<usize>) -> MacroDefinition {
        MacroDefinition::new(bounds)
    }

    pub(crate) fn parse_issue(
        kind: ParseIssueKind,
        bounds: Range<usize>,
        message: LocalizedMessage,
    ) -> ParseIssue {
        let code = Self::issue_code(kind, &bounds, message.id());
        let boundary_unknown = kind == ParseIssueKind::UnclosedDelimiter
            || (kind == ParseIssueKind::Lexical
                && matches!(
                    message.id(),
                    SYNTAX_UNCLOSED_BLOCK_COMMENT
                        | SYNTAX_UNCLOSED_LITERAL
                        | SYNTAX_UNCLOSED_RAW_STRING
                ));
        Self::parse_issue_with_code(kind, code, bounds, message, boundary_unknown)
    }

    pub(crate) fn parse_issue_with_code(
        kind: ParseIssueKind,
        code: ParseIssueCode,
        bounds: Range<usize>,
        message: LocalizedMessage,
        boundary_unknown: bool,
    ) -> ParseIssue {
        let stable_message = Self::issue_message(code, &message);
        ParseIssue::new(kind, code, bounds, boundary_unknown, stable_message)
    }

    pub(crate) fn parse_issue_with_boundary(
        kind: ParseIssueKind,
        bounds: Range<usize>,
        message: LocalizedMessage,
        boundary_unknown: bool,
    ) -> ParseIssue {
        let code = Self::issue_code(kind, &bounds, message.id());
        Self::parse_issue_with_code(kind, code, bounds, message, boundary_unknown)
    }

    pub(crate) fn unsupported_issue(
        construct: UnsupportedConstructKind,
        bounds: Range<usize>,
        message: LocalizedMessage,
    ) -> ParseIssue {
        let code = ParseIssueCode::UnsupportedSyntax(construct);
        ParseIssue::new(
            ParseIssueKind::UnsupportedSyntax,
            code,
            bounds,
            false,
            Self::issue_message(code, &message),
        )
    }

    fn issue_code(
        kind: ParseIssueKind,
        bounds: &Range<usize>,
        message: MessageId,
    ) -> ParseIssueCode {
        match kind {
            ParseIssueKind::Lexical => ParseIssueCode::LexicalError,
            ParseIssueKind::UnexpectedToken if bounds.is_empty() => ParseIssueCode::UnexpectedEof,
            ParseIssueKind::UnexpectedToken => ParseIssueCode::UnexpectedToken,
            ParseIssueKind::InvalidAttribute => ParseIssueCode::InvalidAttribute,
            ParseIssueKind::UnsupportedSyntax if message == SYNTAX_MACRO_OPAQUE => {
                ParseIssueCode::OpaqueMacroInput
            }
            ParseIssueKind::UnsupportedSyntax => {
                ParseIssueCode::UnsupportedSyntax(UnsupportedConstructKind::Token)
            }
            ParseIssueKind::UnsupportedEdition => ParseIssueCode::UnsupportedEdition,
            ParseIssueKind::UnclosedDelimiter => ParseIssueCode::UnclosedDelimiter,
        }
    }

    fn issue_message(code: ParseIssueCode, original: &LocalizedMessage) -> LocalizedMessage {
        match code {
            ParseIssueCode::LexicalError => LocalizedMessage::new(SYNTAX_PARSE_ERROR_LEXICAL_ERROR),
            ParseIssueCode::UnexpectedToken => {
                LocalizedMessage::new(SYNTAX_PARSE_ERROR_UNEXPECTED_TOKEN)
            }
            ParseIssueCode::UnexpectedEof => {
                LocalizedMessage::new(SYNTAX_PARSE_ERROR_UNEXPECTED_EOF)
            }
            ParseIssueCode::UnclosedDelimiter => {
                LocalizedMessage::new(SYNTAX_PARSE_ERROR_UNCLOSED_DELIMITER)
            }
            ParseIssueCode::InvalidAttribute => {
                LocalizedMessage::new(SYNTAX_PARSE_ERROR_INVALID_ATTRIBUTE)
            }
            ParseIssueCode::UnsupportedEdition => {
                let edition = original
                    .arguments()
                    .get("edition")
                    .and_then(|value| match value {
                        MessageArgument::Text(value) => Some(value.as_str()),
                        _ => None,
                    })
                    .unwrap_or("unknown");
                LocalizedMessage::new(SYNTAX_PARSE_ERROR_UNSUPPORTED_EDITION)
                    .with_text("edition", edition)
            }
            ParseIssueCode::OpaqueMacroInput => LocalizedMessage::new(SYNTAX_MACRO_OPAQUE),
            ParseIssueCode::UnsupportedSyntax(construct) => {
                LocalizedMessage::new(match construct {
                    UnsupportedConstructKind::Item => SYNTAX_UNSUPPORTED_ITEM,
                    UnsupportedConstructKind::AssociatedItem => SYNTAX_UNSUPPORTED_ASSOCIATED_ITEM,
                    UnsupportedConstructKind::Type => SYNTAX_UNSUPPORTED_TYPE,
                    UnsupportedConstructKind::Pattern => SYNTAX_UNSUPPORTED_PATTERN,
                    UnsupportedConstructKind::Statement => SYNTAX_UNSUPPORTED_STATEMENT,
                    UnsupportedConstructKind::Expression => SYNTAX_UNSUPPORTED_EXPRESSION,
                    UnsupportedConstructKind::Attribute => SYNTAX_UNSUPPORTED_ATTRIBUTE,
                    UnsupportedConstructKind::Visibility => SYNTAX_UNSUPPORTED_VISIBILITY,
                    UnsupportedConstructKind::UseTree => SYNTAX_UNSUPPORTED_USE_TREE,
                    UnsupportedConstructKind::Token => SYNTAX_UNSUPPORTED_TOKEN,
                })
            }
        }
    }

    pub(crate) fn push_ast_node(
        nodes: &mut Vec<SyntaxNode>,
        parent: Option<AstNodeId>,
        kind: SyntaxNodeKind,
        bounds: Range<usize>,
        attributes: Vec<Attribute>,
    ) -> AstNodeId {
        let id = AstNodeId::new(nodes.len());
        if let Some(parent_id) = parent {
            if let Some(parent_node) = nodes.get_mut(parent_id.index()) {
                parent_node.add_child(id);
            }
        }
        nodes.push(SyntaxNode::new(id, parent, kind, bounds, attributes));
        id
    }

    pub(crate) fn push_data_node(
        nodes: &mut Vec<SyntaxNode>,
        parent: AstNodeId,
        kind: SyntaxNodeKind,
        bounds: Range<usize>,
        data: SyntaxNodeData,
    ) -> AstNodeId {
        let id = Self::push_ast_node(nodes, Some(parent), kind, bounds, Vec::new());
        Self::set_ast_node_data(nodes, id, data);
        id
    }

    pub(crate) fn set_ast_node_data(nodes: &mut [SyntaxNode], id: AstNodeId, data: SyntaxNodeData) {
        if let Some(node) = nodes.get_mut(id.index()) {
            node.set_data(Some(data));
        }
    }

    pub(crate) fn set_macro_invocation(
        nodes: &mut [SyntaxNode],
        id: AstNodeId,
        invocation: MacroInvocation,
    ) {
        let item_data = nodes
            .get(id.index())
            .and_then(SyntaxNode::item_syntax)
            .map(|item| item.with_macro_invocation(invocation.clone()));
        if let Some(node) = nodes.get_mut(id.index()) {
            node.set_data(Some(SyntaxNodeData::Macro(invocation)));
            if let Some(item_data) = item_data {
                node.set_item_data(item_data);
            }
        }
    }

    pub(crate) fn clear_ast_node_data(nodes: &mut [SyntaxNode], id: AstNodeId) {
        if let Some(node) = nodes.get_mut(id.index()) {
            node.set_data(None);
        }
    }

    pub(crate) fn set_ast_node_item_data(
        nodes: &mut [SyntaxNode],
        id: AstNodeId,
        item_data: ItemSyntaxData,
    ) {
        if let Some(node) = nodes.get_mut(id.index()) {
            node.set_item_data(item_data);
        }
    }

    pub(crate) fn set_ast_node_attributes(
        nodes: &mut [SyntaxNode],
        id: AstNodeId,
        attributes: Vec<Attribute>,
    ) {
        if let Some(node) = nodes.get_mut(id.index()) {
            node.set_attributes(attributes);
        }
    }

    pub(crate) fn set_ast_node_coverage(
        nodes: &mut [SyntaxNode],
        id: AstNodeId,
        coverage: CoverageStatus,
    ) {
        if let Some(node) = nodes.get_mut(id.index()) {
            node.set_coverage(coverage);
        }
    }

    pub(crate) fn assign_issue_sources(nodes: &[SyntaxNode], issues: &mut [ParseIssue]) {
        for issue in issues {
            issue.set_source_node_id(
                nodes
                    .iter()
                    .filter(|node| node.start() <= issue.start() && issue.end() <= node.end())
                    .min_by(|left, right| {
                        left.end()
                            .saturating_sub(left.start())
                            .cmp(&right.end().saturating_sub(right.start()))
                            .then_with(|| {
                                (left.kind() != SyntaxNodeKind::Opaque)
                                    .cmp(&(right.kind() != SyntaxNodeKind::Opaque))
                            })
                            .then_with(|| right.id().index().cmp(&left.id().index()))
                    })
                    .map(SyntaxNode::id),
            );
        }
    }

    pub(crate) fn apply_issues(nodes: &mut [SyntaxNode], issues: &[ParseIssue]) {
        for issue in issues {
            let Some(mut node_id) = issue.source_node_id() else {
                continue;
            };
            while let Some(node) = nodes.get_mut(node_id.index()) {
                node.set_coverage(Self::merge_coverage(node.coverage(), issue.coverage()));
                let Some(parent) = node.parent() else {
                    break;
                };
                node_id = parent;
            }
        }
        for index in (0..nodes.len()).rev() {
            let child_coverage = nodes[index]
                .children()
                .iter()
                .filter_map(|id| nodes.get(id.index()))
                .map(SyntaxNode::coverage)
                .fold(CoverageStatus::Complete, Self::merge_coverage);
            let coverage = Self::merge_coverage(nodes[index].coverage(), child_coverage);
            nodes[index].set_coverage(coverage);
        }
    }

    pub(crate) fn merge_coverage(left: CoverageStatus, right: CoverageStatus) -> CoverageStatus {
        match (left, right) {
            (CoverageStatus::Incomplete, _) | (_, CoverageStatus::Incomplete) => {
                CoverageStatus::Incomplete
            }
            (CoverageStatus::Partial, _) | (_, CoverageStatus::Partial) => CoverageStatus::Partial,
            _ => CoverageStatus::Complete,
        }
    }

    pub(crate) fn update_ast_node(
        nodes: &mut [SyntaxNode],
        id: AstNodeId,
        kind: SyntaxNodeKind,
        bounds: Range<usize>,
        attributes: Vec<Attribute>,
    ) {
        if let Some(node) = nodes.get_mut(id.index()) {
            node.update(kind, bounds, attributes);
        }
    }
}
