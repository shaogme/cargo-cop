use std::ops::Range;

use super::{path::PathSyntax, TypeSyntaxKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypeSyntax {
    kind: TypeSyntaxKind,
    bounds: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraitBoundKind {
    Trait(PathSyntax),
    Lifetime(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionTraitArgumentsSyntax {
    parameters: Vec<TypeSyntax>,
    return_type: Option<Box<TypeSyntax>>,
    bounds: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraitBoundSyntax {
    maybe: bool,
    lifetimes: Vec<String>,
    kind: TraitBoundKind,
    function_arguments: Option<FunctionTraitArgumentsSyntax>,
}

impl TypeSyntax {
    pub(crate) fn new(kind: TypeSyntaxKind, bounds: Range<usize>) -> Self {
        Self { kind, bounds }
    }

    pub fn kind(&self) -> &TypeSyntaxKind {
        &self.kind
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }
}

impl FunctionTraitArgumentsSyntax {
    pub(crate) fn new(
        parameters: Vec<TypeSyntax>,
        return_type: Option<Box<TypeSyntax>>,
        bounds: Range<usize>,
    ) -> Self {
        Self {
            parameters,
            return_type,
            bounds,
        }
    }

    pub fn parameters(&self) -> &[TypeSyntax] {
        &self.parameters
    }

    pub fn return_type(&self) -> Option<&TypeSyntax> {
        self.return_type.as_deref()
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }
}

impl TraitBoundSyntax {
    pub(crate) fn new(
        maybe: bool,
        lifetimes: Vec<String>,
        kind: TraitBoundKind,
        function_arguments: Option<FunctionTraitArgumentsSyntax>,
    ) -> Self {
        Self {
            maybe,
            lifetimes,
            kind,
            function_arguments,
        }
    }

    pub fn is_maybe(&self) -> bool {
        self.maybe
    }

    pub fn lifetimes(&self) -> &[String] {
        &self.lifetimes
    }

    pub fn kind(&self) -> &TraitBoundKind {
        &self.kind
    }

    pub fn function_arguments(&self) -> Option<&FunctionTraitArgumentsSyntax> {
        self.function_arguments.as_ref()
    }
}
