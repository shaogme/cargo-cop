use std::ops::Range;

use super::types::{TraitBoundSyntax, TypeSyntax};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GenericArgumentSyntax {
    Lifetime(String),
    Type(TypeSyntax),
    Const(Range<usize>),
    AssociatedType {
        name: String,
        bounds: Vec<TraitBoundSyntax>,
        value: TypeSyntax,
    },
    AssociatedConst {
        name: String,
        value: Range<usize>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathSegmentSyntax {
    name: String,
    arguments: Vec<GenericArgumentSyntax>,
    bounds: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathSyntax {
    leading_colon: bool,
    qualified_self: Option<Box<TypeSyntax>>,
    qualified_trait: Option<Box<PathSyntax>>,
    segments: Vec<PathSegmentSyntax>,
    bounds: Range<usize>,
}

impl PathSyntax {
    pub(crate) fn new(
        leading_colon: bool,
        qualified_self: Option<Box<TypeSyntax>>,
        qualified_trait: Option<Box<PathSyntax>>,
        segments: Vec<PathSegmentSyntax>,
        bounds: Range<usize>,
    ) -> Self {
        Self {
            leading_colon,
            qualified_self,
            qualified_trait,
            segments,
            bounds,
        }
    }

    pub fn has_leading_colon(&self) -> bool {
        self.leading_colon
    }

    pub fn qualified_self(&self) -> Option<&TypeSyntax> {
        self.qualified_self.as_deref()
    }

    pub fn qualified_trait(&self) -> Option<&PathSyntax> {
        self.qualified_trait.as_deref()
    }

    pub fn segments(&self) -> &[PathSegmentSyntax] {
        &self.segments
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }
}

impl PathSegmentSyntax {
    pub(crate) fn new(
        name: String,
        arguments: Vec<GenericArgumentSyntax>,
        bounds: Range<usize>,
    ) -> Self {
        Self {
            name,
            arguments,
            bounds,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn arguments(&self) -> &[GenericArgumentSyntax] {
        &self.arguments
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }
}
