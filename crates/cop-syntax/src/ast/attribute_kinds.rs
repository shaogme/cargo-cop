use std::ops::Range;

use super::attribute::{AttributeMeta, AttributeMetaValue};
use crate::LiteralKind;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttributeMetaValueKind {
    Literal {
        kind: LiteralKind,
        text: String,
        decoded_string: Option<String>,
    },
    Boolean(bool),
    Expression(Range<usize>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttributeMetaArgument {
    Meta(AttributeMeta),
    Value(AttributeMetaValue),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttributeMetaKind {
    Word,
    NameValue(AttributeMetaValue),
    List(Vec<AttributeMetaArgument>),
}

impl AttributeMetaKind {
    pub(super) fn is_word(&self) -> bool {
        matches!(self, Self::Word)
    }

    pub(super) fn arguments(&self) -> &[AttributeMetaArgument] {
        match self {
            Self::List(arguments) => arguments,
            Self::Word | Self::NameValue(_) => &[],
        }
    }

    pub(super) fn value(&self) -> Option<&AttributeMetaValue> {
        match self {
            Self::NameValue(value) => Some(value),
            Self::Word | Self::List(_) => None,
        }
    }

    pub(super) fn normalized(&self, path: &[String]) -> String {
        let path = path.join("::");
        match self {
            Self::Word => path,
            Self::NameValue(value) => format!("{path} = {}", value.kind().normalized_text()),
            Self::List(arguments) => {
                let arguments = arguments
                    .iter()
                    .map(AttributeMetaArgument::normalized)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{path}({arguments})")
            }
        }
    }
}

impl AttributeMetaArgument {
    pub fn meta(&self) -> Option<&AttributeMeta> {
        match self {
            Self::Meta(meta) => Some(meta),
            Self::Value(_) => None,
        }
    }

    pub fn value(&self) -> Option<&AttributeMetaValue> {
        match self {
            Self::Value(value) => Some(value),
            Self::Meta(_) => None,
        }
    }

    pub fn bounds(&self) -> Range<usize> {
        match self {
            Self::Meta(meta) => meta.bounds(),
            Self::Value(value) => value.bounds(),
        }
    }

    fn normalized(&self) -> String {
        match self {
            Self::Meta(meta) => meta.normalized(),
            Self::Value(value) => value.kind().normalized_text(),
        }
    }
}

impl AttributeMetaValueKind {
    pub(super) fn normalized_text(&self) -> String {
        match self {
            Self::Literal { text, .. } => text.clone(),
            Self::Boolean(value) => value.to_string(),
            Self::Expression(_) => "<expression>".to_owned(),
        }
    }

    pub(super) fn string_value(&self) -> Option<&str> {
        match self {
            Self::Literal {
                kind: LiteralKind::String,
                decoded_string: Some(value),
                ..
            } => Some(value),
            Self::Literal { .. } | Self::Boolean(_) | Self::Expression(_) => None,
        }
    }
}
