use std::ops::Range;

use super::PatternSyntaxKind;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatternSyntax {
    kind: PatternSyntaxKind,
    attributes: Vec<Range<usize>>,
    bounds: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatternFieldSyntax {
    name: String,
    pattern: Option<PatternSyntax>,
    attributes: Vec<Range<usize>>,
    bounds: Range<usize>,
}

impl PatternSyntax {
    pub(crate) fn new(
        kind: PatternSyntaxKind,
        attributes: Vec<Range<usize>>,
        bounds: Range<usize>,
    ) -> Self {
        Self {
            kind,
            attributes,
            bounds,
        }
    }

    pub fn kind(&self) -> &PatternSyntaxKind {
        &self.kind
    }

    pub fn is_binding(&self) -> bool {
        matches!(&self.kind, PatternSyntaxKind::Binding { .. })
    }

    pub fn is_or_pattern(&self) -> bool {
        matches!(&self.kind, PatternSyntaxKind::Or(_))
    }

    pub fn is_wildcard(&self) -> bool {
        matches!(&self.kind, PatternSyntaxKind::Wildcard)
    }

    pub fn attributes(&self) -> &[Range<usize>] {
        &self.attributes
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }
}

impl PatternFieldSyntax {
    pub(crate) fn new(
        name: String,
        pattern: Option<PatternSyntax>,
        attributes: Vec<Range<usize>>,
        bounds: Range<usize>,
    ) -> Self {
        Self {
            name,
            pattern,
            attributes,
            bounds,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn pattern(&self) -> Option<&PatternSyntax> {
        self.pattern.as_ref()
    }

    pub fn attributes(&self) -> &[Range<usize>] {
        &self.attributes
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }
}
