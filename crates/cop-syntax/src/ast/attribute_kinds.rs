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
pub enum AttributeMetaKind {
    Word,
    NameValue(AttributeMetaValue),
    List(Vec<AttributeMeta>),
}

impl AttributeMetaKind {
    pub(super) fn is_word(&self) -> bool {
        matches!(self, Self::Word)
    }

    pub(super) fn children(&self) -> &[AttributeMeta] {
        match self {
            Self::List(children) => children,
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
            Self::List(children) => {
                let arguments = children
                    .iter()
                    .map(AttributeMeta::normalized)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{path}({arguments})")
            }
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
