mod item;
mod kind;

use std::ops::Range;

use cop_model::LocalizedMessage;

use crate::{
    lexer::{LexIssue, LexOutput, Token, Trivia},
    source::SourceFile,
};

pub use item::{Item, Visibility, VisibilityKind};
pub use kind::{
    ClosureNode, ExternCrateItem, FunctionBody, FunctionItem, ImplItem, ItemKind, MacroInvocation,
    ModuleItem, NamedDeclaration, OtherItem, Parameter, PathNode, SyntaxNode, TraitItem, TupleNode,
    UseItem, UseTree,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverageStatus {
    Complete,
    Partial,
    Incomplete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseIssueKind {
    Lexical,
    UnexpectedToken,
    UnsupportedSyntax,
    UnclosedDelimiter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseIssue {
    kind: ParseIssueKind,
    start: usize,
    end: usize,
    message: LocalizedMessage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attribute {
    path: Vec<String>,
    inner: bool,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseOutput {
    source: SourceFile,
    lexed: LexOutput,
    inner_attributes: Vec<Attribute>,
    items: Vec<Item>,
    issues: Vec<ParseIssue>,
    coverage: CoverageStatus,
}

pub(crate) struct ParseOutputParts {
    pub(crate) source: SourceFile,
    pub(crate) lexed: LexOutput,
    pub(crate) inner_attributes: Vec<Attribute>,
    pub(crate) items: Vec<Item>,
    pub(crate) issues: Vec<ParseIssue>,
    pub(crate) coverage: CoverageStatus,
}

pub(crate) struct AstFactory;

impl AstFactory {
    pub(crate) fn attribute(path: Vec<String>, inner: bool, bounds: Range<usize>) -> Attribute {
        Attribute {
            path,
            inner,
            start: bounds.start,
            end: bounds.end,
        }
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

    pub(crate) fn module(
        name: String,
        inline: bool,
        inner_attributes: Vec<Attribute>,
        items: Vec<Item>,
    ) -> ModuleItem {
        ModuleItem::new(name, inline, inner_attributes, items)
    }

    pub(crate) fn use_item(tree: UseTree) -> UseItem {
        UseItem::new(tree)
    }

    pub(crate) fn extern_crate(name: Option<String>, alias: Option<String>) -> ExternCrateItem {
        ExternCrateItem::new(name, alias)
    }

    pub(crate) fn use_tree(
        path: Vec<String>,
        alias: Option<String>,
        glob: bool,
        children: Vec<UseTree>,
        bounds: Range<usize>,
    ) -> UseTree {
        UseTree::new(path, alias, glob, children, bounds)
    }

    pub(crate) fn named_declaration(
        name: Option<String>,
        nodes: Vec<SyntaxNode>,
    ) -> NamedDeclaration {
        NamedDeclaration::new(name, nodes)
    }

    pub(crate) fn trait_item(
        name: Option<String>,
        inner_attributes: Vec<Attribute>,
        items: Vec<Item>,
    ) -> TraitItem {
        TraitItem::new(name, inner_attributes, items)
    }

    pub(crate) fn impl_item(
        target: String,
        inner_attributes: Vec<Attribute>,
        items: Vec<Item>,
    ) -> ImplItem {
        ImplItem::new(target, inner_attributes, items)
    }

    pub(crate) fn function(
        name: Option<String>,
        parameters: Vec<Parameter>,
        body: Option<FunctionBody>,
        signature_nodes: Vec<SyntaxNode>,
    ) -> FunctionItem {
        FunctionItem::new(name, parameters, body, signature_nodes)
    }

    pub(crate) fn parameter(bounds: Range<usize>, receiver: bool) -> Parameter {
        Parameter::new(bounds, receiver)
    }

    pub(crate) fn body(
        bounds: Range<usize>,
        leading_attributes: Vec<Attribute>,
        nodes: Vec<SyntaxNode>,
    ) -> FunctionBody {
        FunctionBody::new(bounds, leading_attributes, nodes)
    }

    pub(crate) fn path(segments: Vec<String>, bounds: Range<usize>) -> PathNode {
        PathNode::new(segments, bounds)
    }

    pub(crate) fn tuple(arity: usize, bounds: Range<usize>) -> TupleNode {
        TupleNode::new(arity, bounds)
    }

    pub(crate) fn closure(parameters: Vec<Parameter>, bounds: Range<usize>) -> ClosureNode {
        ClosureNode::new(parameters, bounds)
    }

    pub(crate) fn macro_invocation(path: PathNode, bounds: Range<usize>) -> MacroInvocation {
        MacroInvocation::new(path, bounds)
    }

    pub(crate) fn other(bounds: Range<usize>) -> OtherItem {
        OtherItem::new(bounds)
    }

    pub(crate) fn parse_issue(
        kind: ParseIssueKind,
        bounds: Range<usize>,
        message: LocalizedMessage,
    ) -> ParseIssue {
        ParseIssue {
            kind,
            start: bounds.start,
            end: bounds.end,
            message,
        }
    }

    pub(crate) fn output(parts: ParseOutputParts) -> ParseOutput {
        ParseOutput {
            source: parts.source,
            lexed: parts.lexed,
            inner_attributes: parts.inner_attributes,
            items: parts.items,
            issues: parts.issues,
            coverage: parts.coverage,
        }
    }
}

impl From<&LexIssue> for ParseIssue {
    fn from(issue: &LexIssue) -> Self {
        AstFactory::parse_issue(
            ParseIssueKind::Lexical,
            issue.start()..issue.end(),
            issue.message().clone(),
        )
    }
}

impl ParseIssue {
    pub fn kind(&self) -> ParseIssueKind {
        self.kind
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn message(&self) -> &LocalizedMessage {
        &self.message
    }
}

impl Attribute {
    pub fn path(&self) -> &[String] {
        &self.path
    }

    pub fn is_inner(&self) -> bool {
        self.inner
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }
}

impl ParseOutput {
    pub fn source(&self) -> &SourceFile {
        &self.source
    }

    pub fn tokens(&self) -> &[Token] {
        self.lexed.tokens()
    }

    pub fn trailing_trivia(&self) -> &[Trivia] {
        self.lexed.trailing_trivia()
    }

    pub fn inner_attributes(&self) -> &[Attribute] {
        &self.inner_attributes
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn issues(&self) -> &[ParseIssue] {
        &self.issues
    }

    pub fn coverage(&self) -> CoverageStatus {
        self.coverage
    }
}
