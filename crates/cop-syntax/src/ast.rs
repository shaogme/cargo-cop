mod attribute;
mod attribute_kinds;
mod factory;
mod item;
mod kind;
mod macro_input;
mod syntax;

use std::ops::Range;

use cop_model::{
    LocalizedMessage, SYNTAX_UNCLOSED_BLOCK_COMMENT, SYNTAX_UNCLOSED_LITERAL,
    SYNTAX_UNCLOSED_RAW_STRING,
};

use crate::{
    lexer::{LexIssue, LexOutput, Token, Trivia},
    source::SourceFile,
};

pub(crate) use attribute::AttributeInput;
pub use attribute::{Attribute, AttributeMeta, AttributeMetaValue};
pub use attribute_kinds::{AttributeMetaArgument, AttributeMetaKind, AttributeMetaValueKind};
pub(crate) use factory::AstFactory;
pub(crate) use item::Item;
pub use item::{Visibility, VisibilityKind};
pub(crate) use kind::{
    ExternBlockItem, ExternCrateItem, FunctionItem, ImplItem, ItemKind, ModuleItem,
    NamedDeclaration, Parameter, TraitItem,
};
pub use kind::{ImplSyntaxData, MacroDefinition, MacroInvocation};
pub use macro_input::{CfgCandidate, MacroInput, MacroInputStatus, MacroTokenTree};
pub(crate) use syntax::GenericParameterParts;
pub use syntax::{
    ClosureParameterSyntax, ExpressionBlockKind, ExpressionErrorKind, ExpressionFieldSyntax,
    ExpressionMatchArmSyntax, ExpressionSyntax, ExpressionSyntaxKind, ExpressionUnsupportedKind,
    FunctionTraitArgumentsSyntax, GenericArgumentSyntax, GenericParameterKind,
    GenericParameterSyntax, PathSegmentSyntax, PathSyntax, PatternFieldSyntax, PatternSyntax,
    PatternSyntaxKind, TraitBoundKind, TraitBoundSyntax, TypeSyntax, TypeSyntaxKind, UnaryOperator,
    WhereClauseSyntax, WherePredicateKind, WherePredicateSyntax,
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
    InvalidAttribute,
    UnsupportedSyntax,
    UnsupportedEdition,
    UnclosedDelimiter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedConstructKind {
    Item,
    AssociatedItem,
    Type,
    Pattern,
    Statement,
    Expression,
    Attribute,
    Visibility,
    UseTree,
    Token,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseIssueCode {
    LexicalError,
    UnexpectedToken,
    UnexpectedEof,
    UnclosedDelimiter,
    InvalidAttribute,
    UnsupportedEdition,
    UnsupportedSyntax(UnsupportedConstructKind),
    OpaqueMacroInput,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AstNodeId(usize);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseIssue {
    kind: ParseIssueKind,
    code: ParseIssueCode,
    start: usize,
    end: usize,
    source_node_id: Option<AstNodeId>,
    boundary_unknown: bool,
    message: LocalizedMessage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyntaxNodeKind {
    Crate,
    Item,
    Scope,
    Field,
    EnumVariant,
    Parameter,
    GenericParameter,
    WhereClause,
    WherePredicate,
    Type,
    Pattern,
    Statement,
    LocalItem,
    MatchArm,
    Expression,
    Closure,
    Macro,
    MacroInput,
    MacroDefinition,
    Block,
    Opaque,
    UseTree,
    ForeignItem,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatementSyntaxKind {
    Empty,
    Let,
    Use,
    Expression { has_semicolon: bool, is_tail: bool },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UseTreeSyntaxData {
    path: Vec<String>,
    alias: Option<String>,
    glob: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ItemSyntaxKind {
    Module {
        name: String,
        inline: bool,
    },
    Use,
    ExternCrate {
        name: Option<String>,
        alias: Option<String>,
    },
    Struct(Option<String>),
    Enum(Option<String>),
    Union(Option<String>),
    TypeAlias(Option<String>),
    Trait(Option<String>),
    Constant(Option<String>),
    Static(Option<String>),
    Function {
        name: Option<String>,
    },
    ExternBlock {
        unsafe_: bool,
        abi: Option<String>,
    },
    Impl,
    Macro(MacroInvocation),
    MacroDefinition(MacroDefinition),
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForeignFunctionSafety {
    ImplicitUnsafe,
    Safe,
    Unsafe,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ForeignItemSyntaxKind {
    Function {
        name: Option<String>,
        safety: ForeignFunctionSafety,
    },
    Static {
        name: Option<String>,
        mutable: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForeignItemSyntaxData {
    visibility: Visibility,
    kind: ForeignItemSyntaxKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyntaxNodeData {
    Field(Visibility),
    EnumVariant(Option<String>),
    Impl(ImplSyntaxData),
    Parameter(bool),
    Expression(ExpressionSyntax),
    Statement(StatementSyntaxKind),
    Pattern(PatternSyntax),
    Type(TypeSyntax),
    GenericParameter(GenericParameterSyntax),
    WhereClause(WhereClauseSyntax),
    WherePredicate(WherePredicateSyntax),
    Macro(MacroInvocation),
    UseTree(UseTreeSyntaxData),
    ForeignItem(ForeignItemSyntaxData),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemSyntaxData {
    visibility: Visibility,
    kind: ItemSyntaxKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxNode {
    id: AstNodeId,
    parent: Option<AstNodeId>,
    children: Vec<AstNodeId>,
    kind: SyntaxNodeKind,
    start: usize,
    end: usize,
    attributes: Vec<Attribute>,
    coverage: CoverageStatus,
    item_data: Option<ItemSyntaxData>,
    data: Option<SyntaxNodeData>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseOutput {
    source: SourceFile,
    lexed: LexOutput,
    ast_nodes: Vec<SyntaxNode>,
    issues: Vec<ParseIssue>,
    coverage: CoverageStatus,
}

pub(crate) struct ParseOutputParts {
    pub(crate) source: SourceFile,
    pub(crate) lexed: LexOutput,
    pub(crate) ast_nodes: Vec<SyntaxNode>,
    pub(crate) issues: Vec<ParseIssue>,
    pub(crate) coverage: CoverageStatus,
}

impl From<&LexIssue> for ParseIssue {
    fn from(issue: &LexIssue) -> Self {
        let mut parsed = AstFactory::parse_issue(
            ParseIssueKind::Lexical,
            issue.start()..issue.end(),
            issue.message().clone(),
        );
        parsed.boundary_unknown = matches!(
            issue.message().id(),
            SYNTAX_UNCLOSED_BLOCK_COMMENT | SYNTAX_UNCLOSED_LITERAL | SYNTAX_UNCLOSED_RAW_STRING
        );
        parsed
    }
}

impl ParseIssue {
    pub(crate) fn new(
        kind: ParseIssueKind,
        code: ParseIssueCode,
        bounds: Range<usize>,
        boundary_unknown: bool,
        message: LocalizedMessage,
    ) -> Self {
        Self {
            kind,
            code,
            start: bounds.start,
            end: bounds.end,
            source_node_id: None,
            boundary_unknown,
            message,
        }
    }

    pub(crate) fn set_source_node_id(&mut self, source_node_id: Option<AstNodeId>) {
        self.source_node_id = source_node_id;
    }

    pub fn kind(&self) -> ParseIssueKind {
        self.kind
    }

    pub fn code(&self) -> ParseIssueCode {
        self.code
    }

    pub fn unsupported_construct(&self) -> Option<UnsupportedConstructKind> {
        match self.code {
            ParseIssueCode::UnsupportedSyntax(construct) => Some(construct),
            _ => None,
        }
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn source_node_id(&self) -> Option<AstNodeId> {
        self.source_node_id
    }

    pub fn boundary_unknown(&self) -> bool {
        self.boundary_unknown
    }

    pub fn coverage_status(&self) -> CoverageStatus {
        self.coverage()
    }

    pub(crate) fn coverage(&self) -> CoverageStatus {
        if self.boundary_unknown {
            CoverageStatus::Incomplete
        } else {
            CoverageStatus::Partial
        }
    }

    pub fn message(&self) -> &LocalizedMessage {
        &self.message
    }
}

impl AstNodeId {
    pub(crate) fn new(index: usize) -> Self {
        Self(index)
    }

    pub fn index(self) -> usize {
        self.0
    }
}

impl SyntaxNode {
    pub(crate) fn new(
        id: AstNodeId,
        parent: Option<AstNodeId>,
        kind: SyntaxNodeKind,
        bounds: Range<usize>,
        attributes: Vec<Attribute>,
    ) -> Self {
        Self {
            id,
            parent,
            children: Vec::new(),
            kind,
            start: bounds.start,
            end: bounds.end,
            attributes,
            coverage: if matches!(kind, SyntaxNodeKind::Opaque | SyntaxNodeKind::Macro) {
                CoverageStatus::Partial
            } else {
                CoverageStatus::Complete
            },
            item_data: None,
            data: None,
        }
    }

    pub(crate) fn add_child(&mut self, child: AstNodeId) {
        self.children.push(child);
    }

    pub(crate) fn set_data(&mut self, data: Option<SyntaxNodeData>) {
        self.data = data;
    }

    pub(crate) fn set_item_data(&mut self, item_data: ItemSyntaxData) {
        self.item_data = Some(item_data);
    }

    pub(crate) fn set_attributes(&mut self, attributes: Vec<Attribute>) {
        self.attributes = attributes;
    }

    pub(crate) fn set_coverage(&mut self, coverage: CoverageStatus) {
        self.coverage = coverage;
    }

    pub(crate) fn update(
        &mut self,
        kind: SyntaxNodeKind,
        bounds: Range<usize>,
        attributes: Vec<Attribute>,
    ) {
        self.kind = kind;
        self.start = bounds.start;
        self.end = bounds.end;
        self.attributes = attributes;
        self.coverage = if matches!(kind, SyntaxNodeKind::Opaque | SyntaxNodeKind::Macro) {
            CoverageStatus::Partial
        } else {
            CoverageStatus::Complete
        };
    }

    pub fn id(&self) -> AstNodeId {
        self.id
    }

    pub fn parent(&self) -> Option<AstNodeId> {
        self.parent
    }

    pub fn children(&self) -> &[AstNodeId] {
        &self.children
    }

    pub fn kind(&self) -> SyntaxNodeKind {
        self.kind
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    pub fn coverage(&self) -> CoverageStatus {
        self.coverage
    }

    pub fn data(&self) -> Option<&SyntaxNodeData> {
        self.data.as_ref()
    }

    pub fn pattern_syntax(&self) -> Option<&PatternSyntax> {
        match self.data.as_ref()? {
            SyntaxNodeData::Pattern(pattern) => Some(pattern),
            _ => None,
        }
    }

    pub fn type_syntax(&self) -> Option<&TypeSyntax> {
        match self.data.as_ref()? {
            SyntaxNodeData::Type(value) => Some(value),
            _ => None,
        }
    }

    pub fn expression_syntax(&self) -> Option<&ExpressionSyntax> {
        match self.data.as_ref()? {
            SyntaxNodeData::Expression(value) => Some(value),
            _ => None,
        }
    }

    pub fn generic_parameter_syntax(&self) -> Option<&GenericParameterSyntax> {
        match self.data.as_ref()? {
            SyntaxNodeData::GenericParameter(value) => Some(value),
            _ => None,
        }
    }

    pub fn where_clause_syntax(&self) -> Option<&WhereClauseSyntax> {
        match self.data.as_ref()? {
            SyntaxNodeData::WhereClause(value) => Some(value),
            _ => None,
        }
    }

    pub fn where_predicate_syntax(&self) -> Option<&WherePredicateSyntax> {
        match self.data.as_ref()? {
            SyntaxNodeData::WherePredicate(value) => Some(value),
            _ => None,
        }
    }

    pub fn impl_syntax(&self) -> Option<&ImplSyntaxData> {
        match self.data.as_ref()? {
            SyntaxNodeData::Impl(syntax) => Some(syntax),
            _ => None,
        }
    }

    pub fn macro_invocation(&self) -> Option<&MacroInvocation> {
        match self.data.as_ref()? {
            SyntaxNodeData::Macro(invocation) => Some(invocation),
            _ => None,
        }
    }

    pub fn use_tree(&self) -> Option<&UseTreeSyntaxData> {
        match self.data.as_ref()? {
            SyntaxNodeData::UseTree(tree) => Some(tree),
            _ => None,
        }
    }

    pub fn item_syntax(&self) -> Option<&ItemSyntaxData> {
        self.item_data.as_ref()
    }

    pub fn foreign_item_syntax(&self) -> Option<&ForeignItemSyntaxData> {
        match self.data.as_ref()? {
            SyntaxNodeData::ForeignItem(item) => Some(item),
            _ => None,
        }
    }

    pub fn field_visibility(&self) -> Option<&Visibility> {
        match self.data.as_ref()? {
            SyntaxNodeData::Field(visibility) => Some(visibility),
            _ => None,
        }
    }

    pub fn enum_variant_name(&self) -> Option<&str> {
        match self.data.as_ref()? {
            SyntaxNodeData::EnumVariant(name) => name.as_deref(),
            _ => None,
        }
    }

    pub fn is_receiver_parameter(&self) -> Option<bool> {
        match self.data.as_ref()? {
            SyntaxNodeData::Parameter(receiver) => Some(*receiver),
            _ => None,
        }
    }

    pub fn statement_kind(&self) -> Option<StatementSyntaxKind> {
        match self.data.as_ref()? {
            SyntaxNodeData::Statement(kind) => Some(*kind),
            _ => None,
        }
    }
}

impl ItemSyntaxData {
    pub(crate) fn from_item(item: &Item) -> Self {
        let kind = match item.kind() {
            ItemKind::Module(module) => ItemSyntaxKind::Module {
                name: module.name().to_owned(),
                inline: module.is_inline(),
            },
            ItemKind::Use => ItemSyntaxKind::Use,
            ItemKind::ExternCrate(extern_crate) => ItemSyntaxKind::ExternCrate {
                name: extern_crate.name().map(str::to_owned),
                alias: extern_crate.alias().map(str::to_owned),
            },
            ItemKind::ExternBlock(extern_block) => ItemSyntaxKind::ExternBlock {
                unsafe_: extern_block.is_unsafe(),
                abi: extern_block.abi().map(str::to_owned),
            },
            ItemKind::Struct(declaration) => {
                ItemSyntaxKind::Struct(declaration.name().map(str::to_owned))
            }
            ItemKind::Enum(declaration) => {
                ItemSyntaxKind::Enum(declaration.name().map(str::to_owned))
            }
            ItemKind::Union(declaration) => {
                ItemSyntaxKind::Union(declaration.name().map(str::to_owned))
            }
            ItemKind::TypeAlias(declaration) => {
                ItemSyntaxKind::TypeAlias(declaration.name().map(str::to_owned))
            }
            ItemKind::Trait(trait_item) => {
                ItemSyntaxKind::Trait(trait_item.name().map(str::to_owned))
            }
            ItemKind::Constant(declaration) => {
                ItemSyntaxKind::Constant(declaration.name().map(str::to_owned))
            }
            ItemKind::Static(declaration) => {
                ItemSyntaxKind::Static(declaration.name().map(str::to_owned))
            }
            ItemKind::Function(function) => ItemSyntaxKind::Function {
                name: function.name().map(str::to_owned),
            },
            ItemKind::Impl(_) => ItemSyntaxKind::Impl,
            ItemKind::Macro(invocation) => ItemSyntaxKind::Macro(invocation.clone()),
            ItemKind::MacroDefinition(definition) => {
                ItemSyntaxKind::MacroDefinition(definition.clone())
            }
            ItemKind::Other => ItemSyntaxKind::Other,
        };
        Self {
            visibility: item.visibility().clone(),
            kind,
        }
    }

    pub fn visibility(&self) -> &Visibility {
        &self.visibility
    }

    pub fn kind(&self) -> &ItemSyntaxKind {
        &self.kind
    }

    pub(crate) fn with_macro_invocation(&self, invocation: MacroInvocation) -> Self {
        let kind = match &self.kind {
            ItemSyntaxKind::Macro(_) => ItemSyntaxKind::Macro(invocation),
            _ => self.kind.clone(),
        };
        Self {
            visibility: self.visibility.clone(),
            kind,
        }
    }
}

impl UseTreeSyntaxData {
    pub(crate) fn new(path: Vec<String>, alias: Option<String>, glob: bool) -> Self {
        Self { path, alias, glob }
    }

    pub fn path(&self) -> &[String] {
        &self.path
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn is_glob(&self) -> bool {
        self.glob
    }
}

impl ForeignItemSyntaxData {
    pub(crate) fn new(visibility: Visibility, kind: ForeignItemSyntaxKind) -> Self {
        Self { visibility, kind }
    }

    pub fn visibility(&self) -> &Visibility {
        &self.visibility
    }

    pub fn kind(&self) -> &ForeignItemSyntaxKind {
        &self.kind
    }
}

impl ParseOutput {
    pub(crate) fn new(parts: ParseOutputParts) -> Self {
        Self {
            source: parts.source,
            lexed: parts.lexed,
            ast_nodes: parts.ast_nodes,
            issues: parts.issues,
            coverage: parts.coverage,
        }
    }

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
        self.ast_nodes
            .iter()
            .find(|node| node.kind() == SyntaxNodeKind::Crate)
            .map_or(&[], SyntaxNode::attributes)
    }

    pub fn syntax_nodes(&self) -> &[SyntaxNode] {
        &self.ast_nodes
    }

    pub fn node(&self, id: AstNodeId) -> Option<&SyntaxNode> {
        self.ast_nodes.get(id.index())
    }

    pub fn issues(&self) -> &[ParseIssue] {
        &self.issues
    }

    pub fn coverage(&self) -> CoverageStatus {
        self.coverage
    }

    pub fn coverage_for_range(&self, start: usize, end: usize) -> CoverageStatus {
        let mut coverage = self
            .ast_nodes
            .iter()
            .filter(|node| {
                node.kind() == SyntaxNodeKind::Opaque
                    && ranges_overlap(start, end, node.start(), node.end())
            })
            .map(SyntaxNode::coverage)
            .fold(CoverageStatus::Complete, AstFactory::merge_coverage);
        for issue in self
            .issues
            .iter()
            .filter(|issue| ranges_overlap(start, end, issue.start(), issue.end()))
        {
            coverage = AstFactory::merge_coverage(coverage, issue.coverage());
        }
        coverage
    }

    pub fn coverage_for_node(&self, id: AstNodeId) -> Option<CoverageStatus> {
        self.node(id).map(SyntaxNode::coverage)
    }

    pub fn is_supported_range(&self, start: usize, end: usize) -> bool {
        !self
            .issues
            .iter()
            .any(|issue| ranges_overlap(start, end, issue.start(), issue.end()))
            && !self.ast_nodes.iter().any(|node| {
                node.kind() == SyntaxNodeKind::Opaque
                    && ranges_overlap(start, end, node.start(), node.end())
            })
    }
}

fn ranges_overlap(
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
) -> bool {
    if left_start == left_end || right_start == right_end {
        left_start <= right_end && right_start <= left_end
    } else {
        left_start < right_end && right_start < left_end
    }
}
