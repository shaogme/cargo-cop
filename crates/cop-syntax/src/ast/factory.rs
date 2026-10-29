use cop_i18n::RenderError;
use std::ops::Range;

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

    pub(crate) fn macro_definition(
        name: String,
        scope: Option<AstNodeId>,
        bounds: Range<usize>,
    ) -> MacroDefinition {
        MacroDefinition::new(name, scope, bounds)
    }

    pub(crate) fn parse_issue(
        kind: ParseIssueKind,
        bounds: Range<usize>,
        message: Result<String, RenderError>,
    ) -> ParseIssue {
        let code = Self::issue_code(kind, &bounds);
        let boundary_unknown = kind == ParseIssueKind::UnclosedDelimiter;
        ParseIssue::new(kind, code, bounds, boundary_unknown, message)
    }

    pub(crate) fn parse_issue_with_code(
        kind: ParseIssueKind,
        code: ParseIssueCode,
        bounds: Range<usize>,
        message: Result<String, RenderError>,
        boundary_unknown: bool,
    ) -> ParseIssue {
        ParseIssue::new(kind, code, bounds, boundary_unknown, message)
    }

    pub(crate) fn parse_issue_with_boundary(
        kind: ParseIssueKind,
        bounds: Range<usize>,
        message: Result<String, RenderError>,
        boundary_unknown: bool,
    ) -> ParseIssue {
        let code = Self::issue_code(kind, &bounds);
        Self::parse_issue_with_code(kind, code, bounds, message, boundary_unknown)
    }

    pub(crate) fn unsupported_issue(
        construct: UnsupportedConstructKind,
        bounds: Range<usize>,
        message: Result<String, RenderError>,
    ) -> ParseIssue {
        let code = ParseIssueCode::UnsupportedSyntax(construct);
        ParseIssue::new(
            ParseIssueKind::UnsupportedSyntax,
            code,
            bounds,
            false,
            message,
        )
    }

    pub(crate) fn opaque_macro_issue(
        bounds: Range<usize>,
        message: Result<String, RenderError>,
    ) -> ParseIssue {
        ParseIssue::new(
            ParseIssueKind::UnsupportedSyntax,
            ParseIssueCode::OpaqueMacroInput,
            bounds,
            false,
            message,
        )
    }

    fn issue_code(kind: ParseIssueKind, bounds: &Range<usize>) -> ParseIssueCode {
        match kind {
            ParseIssueKind::Lexical => ParseIssueCode::LexicalError,
            ParseIssueKind::UnexpectedToken if bounds.is_empty() => ParseIssueCode::UnexpectedEof,
            ParseIssueKind::UnexpectedToken => ParseIssueCode::UnexpectedToken,
            ParseIssueKind::InvalidAttribute => ParseIssueCode::InvalidAttribute,
            ParseIssueKind::UnsupportedSyntax => {
                ParseIssueCode::UnsupportedSyntax(UnsupportedConstructKind::Token)
            }
            ParseIssueKind::UnsupportedEdition => ParseIssueCode::UnsupportedEdition,
            ParseIssueKind::UnclosedDelimiter => ParseIssueCode::UnclosedDelimiter,
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
