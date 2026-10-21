mod expression;
mod path;
mod pattern;
mod types;

use std::ops::Range;

use crate::Delimiter;

pub use expression::{
    ClosureParameterSyntax, ExpressionBlockKind, ExpressionErrorKind, ExpressionFieldSyntax,
    ExpressionMatchArmSyntax, ExpressionSyntax, ExpressionSyntaxKind, ExpressionUnsupportedKind,
    UnaryOperator,
};
pub use path::{GenericArgumentSyntax, PathSegmentSyntax, PathSyntax};
pub use pattern::{PatternFieldSyntax, PatternSyntax};
pub use types::{FunctionTraitArgumentsSyntax, TraitBoundKind, TraitBoundSyntax, TypeSyntax};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TypeSyntaxKind {
    Path(PathSyntax),
    Reference {
        lifetime: Option<String>,
        mutable: bool,
        target: Box<TypeSyntax>,
    },
    Pointer {
        mutable: bool,
        target: Box<TypeSyntax>,
    },
    Tuple(Vec<TypeSyntax>),
    Parenthesized(Box<TypeSyntax>),
    Slice(Box<TypeSyntax>),
    Array {
        element: Box<TypeSyntax>,
        length: Range<usize>,
    },
    FunctionPointer {
        lifetimes: Vec<String>,
        unsafe_: bool,
        abi: Option<Range<usize>>,
        parameters: Vec<TypeSyntax>,
        variadic: bool,
        return_type: Option<Box<TypeSyntax>>,
    },
    ImplTrait(Vec<TraitBoundSyntax>),
    DynTrait(Vec<TraitBoundSyntax>),
    TraitBounds(Vec<TraitBoundSyntax>),
    Never,
    Infer,
    Macro {
        path: PathSyntax,
        delimiter: Delimiter,
        input: Range<usize>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatternSyntaxKind {
    Wildcard,
    Binding {
        name: String,
        by_ref: bool,
        mutable: bool,
        subpattern: Option<Box<PatternSyntax>>,
    },
    Reference {
        mutable: bool,
        pattern: Box<PatternSyntax>,
    },
    Literal(Range<usize>),
    Range {
        start: Option<Box<PatternSyntax>>,
        end: Option<Box<PatternSyntax>>,
        inclusive: bool,
    },
    Tuple(Vec<PatternSyntax>),
    TupleStruct {
        path: PathSyntax,
        elements: Vec<PatternSyntax>,
    },
    Struct {
        path: PathSyntax,
        fields: Vec<PatternFieldSyntax>,
        rest: bool,
    },
    Slice(Vec<PatternSyntax>),
    Path(PathSyntax),
    Or(Vec<PatternSyntax>),
    Rest,
    Macro {
        path: PathSyntax,
        delimiter: Delimiter,
        input: Range<usize>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenericParameterKind {
    Lifetime,
    Type,
    Const,
}

pub(crate) struct GenericParameterParts {
    pub(crate) kind: GenericParameterKind,
    pub(crate) name: String,
    pub(crate) trait_bounds: Vec<TraitBoundSyntax>,
    pub(crate) lifetime_bounds: Vec<String>,
    pub(crate) declared_type: Option<TypeSyntax>,
    pub(crate) default_type: Option<TypeSyntax>,
    pub(crate) default_const: Option<Range<usize>>,
    pub(crate) bounds_range: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenericParameterSyntax {
    kind: GenericParameterKind,
    name: String,
    trait_bounds: Vec<TraitBoundSyntax>,
    lifetime_bounds: Vec<String>,
    declared_type: Option<TypeSyntax>,
    default_type: Option<TypeSyntax>,
    default_const: Option<Range<usize>>,
    bounds_range: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WhereClauseSyntax {
    bounds: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WherePredicateKind {
    Lifetime {
        lifetime: String,
        bounds: Vec<String>,
    },
    Type {
        lifetimes: Vec<String>,
        bounded_type: TypeSyntax,
        bounds: Vec<TraitBoundSyntax>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WherePredicateSyntax {
    kind: WherePredicateKind,
    bounds: Range<usize>,
}

impl GenericParameterSyntax {
    pub(crate) fn new(parts: GenericParameterParts) -> Self {
        let GenericParameterParts {
            kind,
            name,
            trait_bounds,
            lifetime_bounds,
            declared_type,
            default_type,
            default_const,
            bounds_range,
        } = parts;
        Self {
            kind,
            name,
            trait_bounds,
            lifetime_bounds,
            declared_type,
            default_type,
            default_const,
            bounds_range,
        }
    }

    pub fn kind(&self) -> GenericParameterKind {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn bounds(&self) -> &[TraitBoundSyntax] {
        &self.trait_bounds
    }

    pub fn lifetime_bounds(&self) -> &[String] {
        &self.lifetime_bounds
    }

    pub fn declared_type(&self) -> Option<&TypeSyntax> {
        self.declared_type.as_ref()
    }

    pub fn default_type(&self) -> Option<&TypeSyntax> {
        self.default_type.as_ref()
    }

    pub fn default_const(&self) -> Option<Range<usize>> {
        self.default_const.clone()
    }

    pub fn bounds_range(&self) -> Range<usize> {
        self.bounds_range.clone()
    }
}

impl WhereClauseSyntax {
    pub(crate) fn new(bounds: Range<usize>) -> Self {
        Self { bounds }
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }
}

impl WherePredicateSyntax {
    pub(crate) fn new(kind: WherePredicateKind, bounds: Range<usize>) -> Self {
        Self { kind, bounds }
    }

    pub fn kind(&self) -> &WherePredicateKind {
        &self.kind
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }
}
