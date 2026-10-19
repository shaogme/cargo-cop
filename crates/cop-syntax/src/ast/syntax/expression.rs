mod kind;

use std::ops::Range;

use super::{PatternSyntax, TypeSyntax};

pub use kind::{
    ExpressionBlockKind, ExpressionErrorKind, ExpressionSyntaxKind, ExpressionUnsupportedKind,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpressionSyntax {
    kind: ExpressionSyntaxKind,
    bounds: Range<usize>,
    attributes: Vec<Range<usize>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpressionFieldSyntax {
    name: Option<String>,
    value: Option<Box<ExpressionSyntax>>,
    shorthand: bool,
    attributes: Vec<Range<usize>>,
    bounds: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClosureParameterSyntax {
    pattern: PatternSyntax,
    ty: Option<TypeSyntax>,
    attributes: Vec<Range<usize>>,
    bounds: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpressionMatchArmSyntax {
    pattern: Option<PatternSyntax>,
    guard: Option<Box<ExpressionSyntax>>,
    body: ExpressionSyntax,
    attributes: Vec<Range<usize>>,
    bounds: Range<usize>,
}

impl ExpressionSyntax {
    pub(crate) fn new(kind: ExpressionSyntaxKind, bounds: Range<usize>) -> Self {
        Self {
            kind,
            bounds,
            attributes: Vec::new(),
        }
    }

    pub fn kind(&self) -> &ExpressionSyntaxKind {
        &self.kind
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn attributes(&self) -> &[Range<usize>] {
        &self.attributes
    }

    pub(crate) fn with_attributes(mut self, attributes: Vec<Range<usize>>) -> Self {
        let mut combined = attributes;
        combined.extend(self.attributes);
        self.attributes = combined;
        self
    }

    pub(crate) fn with_outer_attributes(mut self, attributes: Vec<Range<usize>>) -> Self {
        if let Some(first) = attributes.iter().map(|bounds| bounds.start).min() {
            self.bounds.start = self.bounds.start.min(first);
        }
        self.with_attributes(attributes)
    }
}

impl ExpressionFieldSyntax {
    pub(crate) fn new(
        name: Option<String>,
        value: Option<ExpressionSyntax>,
        shorthand: bool,
        attributes: Vec<Range<usize>>,
        bounds: Range<usize>,
    ) -> Self {
        Self {
            name,
            value: value.map(Box::new),
            shorthand,
            attributes,
            bounds,
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn value(&self) -> Option<&ExpressionSyntax> {
        self.value.as_deref()
    }

    pub fn is_shorthand(&self) -> bool {
        self.shorthand
    }

    pub fn attributes(&self) -> &[Range<usize>] {
        &self.attributes
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }
}

impl ExpressionMatchArmSyntax {
    pub(crate) fn new(
        pattern: Option<PatternSyntax>,
        guard: Option<ExpressionSyntax>,
        body: ExpressionSyntax,
        attributes: Vec<Range<usize>>,
        bounds: Range<usize>,
    ) -> Self {
        Self {
            pattern,
            guard: guard.map(Box::new),
            body,
            attributes,
            bounds,
        }
    }

    pub fn pattern(&self) -> Option<&PatternSyntax> {
        self.pattern.as_ref()
    }

    pub fn guard(&self) -> Option<&ExpressionSyntax> {
        self.guard.as_deref()
    }

    pub fn body(&self) -> &ExpressionSyntax {
        &self.body
    }

    pub fn attributes(&self) -> &[Range<usize>] {
        &self.attributes
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }
}

impl ClosureParameterSyntax {
    pub(crate) fn new(
        pattern: PatternSyntax,
        ty: Option<TypeSyntax>,
        attributes: Vec<Range<usize>>,
        bounds: Range<usize>,
    ) -> Self {
        Self {
            pattern,
            ty,
            attributes,
            bounds,
        }
    }

    pub fn pattern(&self) -> &PatternSyntax {
        &self.pattern
    }

    pub fn ty(&self) -> Option<&TypeSyntax> {
        self.ty.as_ref()
    }

    pub fn attributes(&self) -> &[Range<usize>] {
        &self.attributes
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }
}
