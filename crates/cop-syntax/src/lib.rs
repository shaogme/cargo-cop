#![forbid(unsafe_code)]

mod ast;
mod lexer;
mod parser;
mod recovery;
mod source;
mod string_literal;

pub use ast::{
    AstNodeId, Attribute, AttributeMeta, AttributeMetaArgument, AttributeMetaKind,
    AttributeMetaValue, AttributeMetaValueKind, CfgCandidate, ClosureParameterSyntax,
    CoverageStatus, ExpressionBlockKind, ExpressionErrorKind, ExpressionFieldSyntax,
    ExpressionMatchArmSyntax, ExpressionSyntax, ExpressionSyntaxKind, ExpressionUnsupportedKind,
    ForeignFunctionSafety, ForeignItemSyntaxData, ForeignItemSyntaxKind,
    FunctionTraitArgumentsSyntax, GenericArgumentSyntax, GenericParameterKind,
    GenericParameterSyntax, ImplSyntaxData, ItemSyntaxData, ItemSyntaxKind, MacroDefinition,
    MacroInput, MacroInputStatus, MacroInvocation, MacroTokenTree, ParseIssue, ParseIssueCode,
    ParseIssueKind, ParseOutput, PathSegmentSyntax, PathSyntax, PatternFieldSyntax, PatternSyntax,
    PatternSyntaxKind, StatementSyntaxKind, SyntaxNode, SyntaxNodeData, SyntaxNodeKind,
    TraitBoundKind, TraitBoundSyntax, TypeSyntax, TypeSyntaxKind, UnaryOperator,
    UnsupportedConstructKind, UseTreeSyntaxData, Visibility, VisibilityKind, WhereClauseSyntax,
    WherePredicateKind, WherePredicateSyntax,
};
pub use lexer::{
    Delimiter, LexIssue, LexOutput, LiteralKind, RustEdition, RustLexer, Token, TokenKind, Trivia,
    TriviaKind,
};
pub use parser::{ParseOptions, RustParser};
pub use source::SourceFile;
pub use string_literal::RustStringLiteral;
