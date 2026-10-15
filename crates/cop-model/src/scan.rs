use crate::diagnostic::{Diagnostic, DiagnosticKind, Severity};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ScanStatus {
    Complete,
    Partial,
    Incomplete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanResult {
    status: ScanStatus,
    diagnostics: Vec<Diagnostic>,
    suppressed_warnings: usize,
}

impl ScanStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Incomplete => "incomplete",
        }
    }

    pub fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }

    pub fn is_partial(self) -> bool {
        matches!(self, Self::Partial)
    }

    pub fn is_incomplete(self) -> bool {
        matches!(self, Self::Incomplete)
    }

    pub fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::Incomplete, _) | (_, Self::Incomplete) => Self::Incomplete,
            (Self::Partial, _) | (_, Self::Partial) => Self::Partial,
            (Self::Complete, Self::Complete) => Self::Complete,
        }
    }

    pub fn from_diagnostics(diagnostics: &[Diagnostic]) -> Self {
        diagnostics
            .iter()
            .fold(Self::Complete, |status, diagnostic| {
                let diagnostic_status = match diagnostic.kind() {
                    DiagnosticKind::Tool(_) => Self::Incomplete,
                    DiagnosticKind::Analysis(_) => match diagnostic.severity() {
                        Severity::Warning => Self::Partial,
                        Severity::Error => Self::Incomplete,
                    },
                    DiagnosticKind::Rule(_) => Self::Complete,
                };
                status.combine(diagnostic_status)
            })
    }
}

impl ScanResult {
    pub fn new(
        status: ScanStatus,
        diagnostics: Vec<Diagnostic>,
        suppressed_warnings: usize,
    ) -> Self {
        let status = status.combine(ScanStatus::from_diagnostics(&diagnostics));
        Self {
            status,
            diagnostics,
            suppressed_warnings,
        }
    }

    pub fn status(&self) -> ScanStatus {
        self.status
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn suppressed_warnings(&self) -> usize {
        self.suppressed_warnings
    }

    pub fn diagnostic_count(&self) -> usize {
        self.diagnostics.len()
    }

    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity() == Severity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity() == Severity::Warning)
            .count()
    }

    pub fn rule_diagnostic_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| matches!(diagnostic.kind(), DiagnosticKind::Rule(_)))
            .count()
    }

    pub fn tool_error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| matches!(diagnostic.kind(), DiagnosticKind::Tool(_)))
            .count()
    }

    pub fn analysis_issue_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| matches!(diagnostic.kind(), DiagnosticKind::Analysis(_)))
            .count()
    }

    pub fn has_errors(&self) -> bool {
        self.error_count() > 0
    }

    pub fn has_warnings(&self) -> bool {
        self.warning_count() > 0
    }

    pub fn exit_code(&self) -> u8 {
        if self.status.is_incomplete() {
            return 2;
        }

        if self.has_errors() {
            return 1;
        }

        0
    }

    pub fn is_successful(&self) -> bool {
        self.exit_code() == 0
    }
}
