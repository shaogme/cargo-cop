use std::ops::Range;

use crate::Delimiter;

use super::{
    super::{GenericArgumentSyntax, PathSyntax, PatternSyntax, TypeSyntax},
    ClosureParameterSyntax, ExpressionFieldSyntax, ExpressionMatchArmSyntax, ExpressionSyntax,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpressionBlockKind {
    Plain,
    Async,
    Unsafe,
    Const,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpressionUnsupportedKind {
    TryBlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpressionErrorKind {
    UnexpectedToken,
    UnexpectedEof,
    UnclosedDelimiter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExpressionSyntaxKind {
    Literal,
    Path(PathSyntax),
    Group(Box<ExpressionSyntax>),
    Tuple(Vec<ExpressionSyntax>),
    Array(Vec<ExpressionSyntax>),
    Repeat {
        value: Box<ExpressionSyntax>,
        count: Box<ExpressionSyntax>,
    },
    Struct {
        path: PathSyntax,
        fields: Vec<ExpressionFieldSyntax>,
    },
    Unary {
        operator: String,
        operand: Box<ExpressionSyntax>,
    },
    Binary {
        operator: String,
        left: Box<ExpressionSyntax>,
        right: Box<ExpressionSyntax>,
    },
    Assignment {
        operator: String,
        left: Box<ExpressionSyntax>,
        right: Box<ExpressionSyntax>,
    },
    Range {
        start: Option<Box<ExpressionSyntax>>,
        end: Option<Box<ExpressionSyntax>>,
        inclusive: bool,
    },
    Call {
        callee: Box<ExpressionSyntax>,
        arguments: Vec<ExpressionSyntax>,
    },
    MethodCall {
        receiver: Box<ExpressionSyntax>,
        method: String,
        generic_arguments: Vec<GenericArgumentSyntax>,
        arguments: Vec<ExpressionSyntax>,
    },
    Field {
        base: Box<ExpressionSyntax>,
        field: String,
    },
    TupleField {
        base: Box<ExpressionSyntax>,
        index: usize,
    },
    Index {
        base: Box<ExpressionSyntax>,
        index: Box<ExpressionSyntax>,
    },
    Await(Box<ExpressionSyntax>),
    Try(Box<ExpressionSyntax>),
    Cast {
        expression: Box<ExpressionSyntax>,
        target: TypeSyntax,
    },
    Closure {
        is_async: bool,
        is_move: bool,
        parameters: Vec<ClosureParameterSyntax>,
        return_type: Option<TypeSyntax>,
        body: Box<ExpressionSyntax>,
    },
    Block {
        kind: ExpressionBlockKind,
        body: Range<usize>,
    },
    LabelBlock {
        label: String,
        body: Range<usize>,
    },
    If {
        condition: Box<ExpressionSyntax>,
        then_block: Range<usize>,
        else_expression: Option<Box<ExpressionSyntax>>,
    },
    Match {
        scrutinee: Box<ExpressionSyntax>,
        arms: Vec<ExpressionMatchArmSyntax>,
    },
    Loop {
        label: Option<String>,
        body: Range<usize>,
    },
    While {
        label: Option<String>,
        condition: Box<ExpressionSyntax>,
        body: Range<usize>,
    },
    For {
        label: Option<String>,
        pattern: PatternSyntax,
        iterator: Box<ExpressionSyntax>,
        body: Range<usize>,
    },
    Return(Option<Box<ExpressionSyntax>>),
    Break {
        label: Option<String>,
        value: Option<Box<ExpressionSyntax>>,
    },
    Continue(Option<String>),
    Underscore,
    Error(ExpressionErrorKind),
    Unsupported(ExpressionUnsupportedKind),
    Let {
        pattern: PatternSyntax,
        value: Box<ExpressionSyntax>,
    },
    Macro {
        path: PathSyntax,
        delimiter: Delimiter,
        input: Range<usize>,
    },
}
