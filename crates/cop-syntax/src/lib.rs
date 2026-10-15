#![forbid(unsafe_code)]

mod ast;
mod lexer;
mod parser;
mod recovery;
mod source;

pub use ast::{
    Attribute, ClosureNode, CoverageStatus, ExternCrateItem, FunctionBody, FunctionItem, ImplItem,
    Item, ItemKind, MacroInvocation, ModuleItem, NamedDeclaration, OtherItem, Parameter,
    ParseIssue, ParseIssueKind, ParseOutput, PathNode, SyntaxNode, TraitItem, TupleNode, UseItem,
    UseTree, Visibility, VisibilityKind,
};
pub use lexer::{
    Delimiter, LexIssue, LexOutput, LiteralKind, RustLexer, Token, TokenKind, Trivia, TriviaKind,
};
pub use parser::RustParser;
pub use source::SourceFile;
