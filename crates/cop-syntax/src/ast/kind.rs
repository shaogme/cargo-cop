use std::ops::Range;

use crate::ast::{Attribute, Item};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Parameter {
    start: usize,
    end: usize,
    receiver: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathNode {
    segments: Vec<String>,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TupleNode {
    arity: usize,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClosureNode {
    parameters: Vec<Parameter>,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroInvocation {
    path: PathNode,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyntaxNode {
    Path(PathNode),
    Tuple(TupleNode),
    Closure(ClosureNode),
    Macro(MacroInvocation),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionBody {
    start: usize,
    end: usize,
    leading_attributes: Vec<Attribute>,
    nodes: Vec<SyntaxNode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionItem {
    name: Option<String>,
    parameters: Vec<Parameter>,
    body: Option<FunctionBody>,
    signature_nodes: Vec<SyntaxNode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleItem {
    name: String,
    inline: bool,
    inner_attributes: Vec<Attribute>,
    items: Vec<Item>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UseTree {
    path: Vec<String>,
    alias: Option<String>,
    glob: bool,
    children: Vec<UseTree>,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UseItem {
    tree: UseTree,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternCrateItem {
    name: Option<String>,
    alias: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedDeclaration {
    name: Option<String>,
    nodes: Vec<SyntaxNode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraitItem {
    name: Option<String>,
    inner_attributes: Vec<Attribute>,
    items: Vec<Item>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImplItem {
    target: String,
    inner_attributes: Vec<Attribute>,
    items: Vec<Item>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtherItem {
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ItemKind {
    Module(ModuleItem),
    Use(UseItem),
    ExternCrate(ExternCrateItem),
    Struct(NamedDeclaration),
    Enum(NamedDeclaration),
    Union(NamedDeclaration),
    TypeAlias(NamedDeclaration),
    Trait(TraitItem),
    Constant(NamedDeclaration),
    Static(NamedDeclaration),
    Function(FunctionItem),
    Impl(ImplItem),
    Macro(MacroInvocation),
    Other(OtherItem),
}

impl ModuleItem {
    pub(crate) fn new(
        name: String,
        inline: bool,
        inner_attributes: Vec<Attribute>,
        items: Vec<Item>,
    ) -> Self {
        Self {
            name,
            inline,
            inner_attributes,
            items,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_inline(&self) -> bool {
        self.inline
    }

    pub fn inner_attributes(&self) -> &[Attribute] {
        &self.inner_attributes
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }
}

impl UseItem {
    pub(crate) fn new(tree: UseTree) -> Self {
        Self { tree }
    }

    pub fn tree(&self) -> &UseTree {
        &self.tree
    }
}

impl ExternCrateItem {
    pub(crate) fn new(name: Option<String>, alias: Option<String>) -> Self {
        Self { name, alias }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }
}

impl UseTree {
    pub(crate) fn new(
        path: Vec<String>,
        alias: Option<String>,
        glob: bool,
        children: Vec<Self>,
        bounds: Range<usize>,
    ) -> Self {
        Self {
            path,
            alias,
            glob,
            children,
            start: bounds.start,
            end: bounds.end,
        }
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

    pub fn children(&self) -> &[UseTree] {
        &self.children
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }
}

impl NamedDeclaration {
    pub(crate) fn new(name: Option<String>, nodes: Vec<SyntaxNode>) -> Self {
        Self { name, nodes }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn nodes(&self) -> &[SyntaxNode] {
        &self.nodes
    }
}

impl TraitItem {
    pub(crate) fn new(
        name: Option<String>,
        inner_attributes: Vec<Attribute>,
        items: Vec<Item>,
    ) -> Self {
        Self {
            name,
            inner_attributes,
            items,
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn inner_attributes(&self) -> &[Attribute] {
        &self.inner_attributes
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }
}

impl ImplItem {
    pub(crate) fn new(target: String, inner_attributes: Vec<Attribute>, items: Vec<Item>) -> Self {
        Self {
            target,
            inner_attributes,
            items,
        }
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn inner_attributes(&self) -> &[Attribute] {
        &self.inner_attributes
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }
}

impl FunctionItem {
    pub(crate) fn new(
        name: Option<String>,
        parameters: Vec<Parameter>,
        body: Option<FunctionBody>,
        signature_nodes: Vec<SyntaxNode>,
    ) -> Self {
        Self {
            name,
            parameters,
            body,
            signature_nodes,
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    pub fn body(&self) -> Option<&FunctionBody> {
        self.body.as_ref()
    }

    pub fn signature_nodes(&self) -> &[SyntaxNode] {
        &self.signature_nodes
    }
}

impl Parameter {
    pub(crate) fn new(bounds: Range<usize>, receiver: bool) -> Self {
        Self {
            start: bounds.start,
            end: bounds.end,
            receiver,
        }
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn is_receiver(&self) -> bool {
        self.receiver
    }
}

impl FunctionBody {
    pub(crate) fn new(
        bounds: Range<usize>,
        leading_attributes: Vec<Attribute>,
        nodes: Vec<SyntaxNode>,
    ) -> Self {
        Self {
            start: bounds.start,
            end: bounds.end,
            leading_attributes,
            nodes,
        }
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn leading_attributes(&self) -> &[Attribute] {
        &self.leading_attributes
    }

    pub fn nodes(&self) -> &[SyntaxNode] {
        &self.nodes
    }
}

impl PathNode {
    pub(crate) fn new(segments: Vec<String>, bounds: Range<usize>) -> Self {
        Self {
            segments,
            start: bounds.start,
            end: bounds.end,
        }
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }
}

impl TupleNode {
    pub(crate) fn new(arity: usize, bounds: Range<usize>) -> Self {
        Self {
            arity,
            start: bounds.start,
            end: bounds.end,
        }
    }

    pub fn arity(&self) -> usize {
        self.arity
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }
}

impl ClosureNode {
    pub(crate) fn new(parameters: Vec<Parameter>, bounds: Range<usize>) -> Self {
        Self {
            parameters,
            start: bounds.start,
            end: bounds.end,
        }
    }

    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }
}

impl MacroInvocation {
    pub(crate) fn new(path: PathNode, bounds: Range<usize>) -> Self {
        Self {
            path,
            start: bounds.start,
            end: bounds.end,
        }
    }

    pub fn path(&self) -> &PathNode {
        &self.path
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }
}

impl OtherItem {
    pub(crate) fn new(bounds: Range<usize>) -> Self {
        Self {
            start: bounds.start,
            end: bounds.end,
        }
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }
}
