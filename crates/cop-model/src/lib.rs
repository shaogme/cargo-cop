mod diagnostic;
mod location;
mod message;
mod scan;

pub use diagnostic::{
    AnalysisIssueKind, Diagnostic, DiagnosticKind, RuleId, Severity, ToolErrorKind, UnknownRuleId,
};
pub use location::{LocationError, SourceLocation, SourceSpan};
pub use message::*;
pub use message::{
    LocalizedMessage, MessageArgument, MessageArgumentKind, MessageId, MessageSchemaError,
};
pub use scan::{ScanResult, ScanStatus};

pub const JSON_SCHEMA_VERSION: u32 = 3;
