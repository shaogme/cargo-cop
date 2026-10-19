use std::ops::Range;

use super::attribute_kinds::{AttributeMetaArgument, AttributeMetaKind, AttributeMetaValueKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttributeMetaValue {
    kind: AttributeMetaValueKind,
    bounds: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttributeMeta {
    path: Vec<String>,
    kind: AttributeMetaKind,
    bounds: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attribute {
    path: Vec<String>,
    inner: bool,
    cfg_test: bool,
    meta: AttributeMeta,
    start: usize,
    end: usize,
}

pub(crate) struct AttributeInput {
    pub(crate) path: Vec<String>,
    pub(crate) inner: bool,
    pub(crate) cfg_test: bool,
    pub(crate) meta: AttributeMeta,
    pub(crate) bounds: Range<usize>,
}

impl AttributeMeta {
    pub(crate) fn new(path: Vec<String>, kind: AttributeMetaKind, bounds: Range<usize>) -> Self {
        Self { path, kind, bounds }
    }

    pub fn path(&self) -> &[String] {
        &self.path
    }

    pub fn name(&self) -> Option<&str> {
        self.path.last().map(String::as_str)
    }

    pub fn kind(&self) -> &AttributeMetaKind {
        &self.kind
    }

    pub fn is_word(&self) -> bool {
        self.kind.is_word()
    }

    pub fn arguments(&self) -> &[AttributeMetaArgument] {
        self.kind.arguments()
    }

    pub fn value(&self) -> Option<&AttributeMetaValue> {
        self.kind.value()
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn normalized(&self) -> String {
        self.kind.normalized(&self.path)
    }
}

impl AttributeMetaValue {
    pub(crate) fn new(kind: AttributeMetaValueKind, bounds: Range<usize>) -> Self {
        Self { kind, bounds }
    }

    pub fn kind(&self) -> &AttributeMetaValueKind {
        &self.kind
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn string_value(&self) -> Option<&str> {
        self.kind.string_value()
    }
}

impl Attribute {
    pub(crate) fn new(input: AttributeInput) -> Self {
        Self {
            path: input.path,
            inner: input.inner,
            cfg_test: input.cfg_test,
            meta: input.meta,
            start: input.bounds.start,
            end: input.bounds.end,
        }
    }

    pub fn path(&self) -> &[String] {
        &self.path
    }

    pub fn is_inner(&self) -> bool {
        self.inner
    }

    pub fn is_cfg_test(&self) -> bool {
        self.cfg_test
    }

    pub fn meta(&self) -> &AttributeMeta {
        &self.meta
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }
}
