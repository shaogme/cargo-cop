use std::ops::Range;

use crate::{
    ast::{Attribute, MacroInput, MacroInputStatus, PathSyntax, TypeSyntax},
    Token,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Parameter {
    attributes: Vec<Attribute>,
    start: usize,
    end: usize,
    receiver: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImplSyntaxData {
    self_type: Option<TypeSyntax>,
    trait_path: Option<PathSyntax>,
    trait_impl: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroInvocation {
    path: PathSyntax,
    start: usize,
    end: usize,
    input: MacroInput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroDefinition {
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FunctionItem {
    name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModuleItem {
    name: String,
    inline: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExternCrateItem {
    name: Option<String>,
    alias: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExternBlockItem {
    unsafe_: bool,
    abi: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NamedDeclaration {
    name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TraitItem {
    name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImplItem {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ItemKind {
    Module(ModuleItem),
    Use,
    ExternCrate(ExternCrateItem),
    ExternBlock(ExternBlockItem),
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
    MacroDefinition(MacroDefinition),
    Other,
}

impl ModuleItem {
    pub(crate) fn new(name: String, inline: bool) -> Self {
        Self { name, inline }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_inline(&self) -> bool {
        self.inline
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

impl ExternBlockItem {
    pub(crate) fn new(unsafe_: bool, abi: Option<String>) -> Self {
        Self { unsafe_, abi }
    }

    pub fn is_unsafe(&self) -> bool {
        self.unsafe_
    }

    pub fn abi(&self) -> Option<&str> {
        self.abi.as_deref()
    }
}

impl NamedDeclaration {
    pub(crate) fn new(name: Option<String>) -> Self {
        Self { name }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl TraitItem {
    pub(crate) fn new(name: Option<String>) -> Self {
        Self { name }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl ImplItem {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

impl FunctionItem {
    pub(crate) fn new(name: Option<String>) -> Self {
        Self { name }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl Parameter {
    pub(crate) fn new(attributes: Vec<Attribute>, bounds: Range<usize>, receiver: bool) -> Self {
        Self {
            attributes,
            start: bounds.start,
            end: bounds.end,
            receiver,
        }
    }

    pub(crate) fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    pub(crate) fn start(&self) -> usize {
        self.start
    }

    pub(crate) fn end(&self) -> usize {
        self.end
    }

    pub(crate) fn is_receiver(&self) -> bool {
        self.receiver
    }
}

impl ImplSyntaxData {
    pub(crate) fn new(
        self_type: Option<TypeSyntax>,
        trait_path: Option<PathSyntax>,
        trait_impl: bool,
    ) -> Self {
        Self {
            self_type,
            trait_path,
            trait_impl,
        }
    }

    pub fn self_type(&self) -> Option<&TypeSyntax> {
        self.self_type.as_ref()
    }

    pub fn trait_path(&self) -> Option<&PathSyntax> {
        self.trait_path.as_ref()
    }

    pub fn is_trait_impl(&self) -> bool {
        self.trait_impl
    }
}

impl MacroInvocation {
    pub(crate) fn new(path: PathSyntax, bounds: Range<usize>, input: MacroInput) -> Self {
        Self {
            path,
            start: bounds.start,
            end: bounds.end,
            input,
        }
    }

    pub fn path(&self) -> &PathSyntax {
        &self.path
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn input(&self) -> &MacroInput {
        &self.input
    }

    pub(crate) fn with_input_status(&self, tokens: &[Token], status: MacroInputStatus) -> Self {
        Self {
            path: self.path.clone(),
            start: self.start,
            end: self.end,
            input: MacroInput::from_tokens(tokens, self.input.bounds(), status),
        }
    }
}

impl MacroDefinition {
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
