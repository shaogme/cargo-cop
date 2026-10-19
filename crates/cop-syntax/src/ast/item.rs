use std::ops::Range;

use crate::ast::{Attribute, ItemKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VisibilityKind {
    Private,
    Public,
    Restricted(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Visibility {
    kind: VisibilityKind,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    attributes: Vec<Attribute>,
    visibility: Visibility,
    kind: ItemKind,
    start: usize,
    end: usize,
}

impl Visibility {
    pub(crate) fn new(kind: VisibilityKind, bounds: Range<usize>) -> Self {
        Self {
            kind,
            start: bounds.start,
            end: bounds.end,
        }
    }

    pub fn kind(&self) -> &VisibilityKind {
        &self.kind
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }
}

impl Item {
    pub(crate) fn new(
        attributes: Vec<Attribute>,
        visibility: Visibility,
        kind: ItemKind,
        bounds: Range<usize>,
    ) -> Self {
        Self {
            attributes,
            visibility,
            kind,
            start: bounds.start,
            end: bounds.end,
        }
    }

    pub(crate) fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    pub(crate) fn visibility(&self) -> &Visibility {
        &self.visibility
    }

    pub(crate) fn kind(&self) -> &ItemKind {
        &self.kind
    }

    pub(crate) fn start(&self) -> usize {
        self.start
    }

    pub(crate) fn end(&self) -> usize {
        self.end
    }
}
