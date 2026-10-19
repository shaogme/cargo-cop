#![forbid(unsafe_code)]

use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
};

use cop_i18n::{Locale, MessageFormatter};
use cop_model::{
    Diagnostic, DiagnosticKind, LocalizedMessage, MessageArgument, RuleId, ScanResult,
    JSON_SCHEMA_VERSION, REPORT_CFG_LABEL, REPORT_DETAIL_REFERENCE, REPORT_NAMESPACE_LABEL,
    REPORT_SUMMARY,
};
use serde_json::{json, Map, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportFormat {
    Human,
    Json,
}

pub struct Reporter;

struct DiagnosticTypeFields {
    kind: &'static str,
    rule_id: Option<&'static str>,
    tool_error: Option<&'static str>,
    analysis_issue: Option<&'static str>,
}

impl Reporter {
    pub fn render(
        result: &ScanResult,
        workspace_root: &Path,
        format: ReportFormat,
        locale: Locale,
    ) -> String {
        let formatter = MessageFormatter::new(locale);
        let diagnostics = sorted_diagnostics(result.diagnostics(), workspace_root);
        match format {
            ReportFormat::Human => render_human(result, &diagnostics, workspace_root, &formatter),
            ReportFormat::Json => {
                render_json(result, &diagnostics, workspace_root, locale, &formatter).to_string()
            }
        }
    }

    pub fn sort_diagnostics(diagnostics: &[Diagnostic], workspace_root: &Path) -> Vec<Diagnostic> {
        sorted_diagnostics(diagnostics, workspace_root)
    }
}

fn sorted_diagnostics(diagnostics: &[Diagnostic], workspace_root: &Path) -> Vec<Diagnostic> {
    let mut ordered = diagnostics.to_vec();
    ordered.sort_by(|left, right| compare_diagnostics(left, right, workspace_root));
    ordered
}

fn compare_diagnostics(left: &Diagnostic, right: &Diagnostic, workspace_root: &Path) -> Ordering {
    display_path(left.package_manifest_path(), workspace_root)
        .cmp(&display_path(right.package_manifest_path(), workspace_root))
        .then_with(|| left.target_kind().cmp(right.target_kind()))
        .then_with(|| left.target().cmp(&right.target()))
        .then_with(|| {
            display_path(left.span().map(|span| span.path()), workspace_root).cmp(&display_path(
                right.span().map(|span| span.path()),
                workspace_root,
            ))
        })
        .then_with(|| {
            left.span()
                .map(|span| span.start_line())
                .cmp(&right.span().map(|span| span.start_line()))
        })
        .then_with(|| {
            left.span()
                .map(|span| span.start_column())
                .cmp(&right.span().map(|span| span.start_column()))
        })
        .then_with(|| {
            left.kind()
                .rule_id()
                .map(RuleId::as_str)
                .cmp(&right.kind().rule_id().map(RuleId::as_str))
        })
        .then_with(|| diagnostic_code(left).cmp(&diagnostic_code(right)))
        .then_with(|| {
            left.span()
                .map(|span| span.end_line())
                .cmp(&right.span().map(|span| span.end_line()))
        })
        .then_with(|| {
            left.span()
                .map(|span| span.end_column())
                .cmp(&right.span().map(|span| span.end_column()))
        })
        .then_with(|| left.message().cmp(right.message()))
}

fn render_human(
    result: &ScanResult,
    diagnostics: &[Diagnostic],
    workspace_root: &Path,
    formatter: &MessageFormatter,
) -> String {
    let mut lines = Vec::new();
    for diagnostic in diagnostics {
        let mut heading = format!("{}[{}]", diagnostic.severity(), diagnostic_code(diagnostic));
        if let Some(package) = diagnostic.package() {
            heading.push_str(&format!(" {package}"));
        }
        if let Some(target) = diagnostic.target() {
            heading.push_str(&format!(" ({target})"));
        }
        if let Some(span) = diagnostic.span() {
            heading.push_str(&format!(
                " {}:{}:{}",
                display_path(Some(span.path()), workspace_root),
                span.start_line(),
                span.start_column()
            ));
        }
        heading.push_str(&format!(": {}", formatter.format(diagnostic.message())));
        if let Some(namespace) = diagnostic.namespace() {
            heading.push_str(&format!(
                " [{}: {namespace}]",
                formatter.text(REPORT_NAMESPACE_LABEL)
            ));
        }
        if let Some(condition) = diagnostic.cfg_condition() {
            heading.push_str(&format!(
                " [{}: {condition}]",
                formatter.text(REPORT_CFG_LABEL)
            ));
        }
        lines.push(heading);
    }
    let summary = LocalizedMessage::new(REPORT_SUMMARY)
        .with_text("status", result.status().as_str())
        .with_number("errors", result.error_count() as i64)
        .with_number("warnings", result.warning_count() as i64)
        .with_number("suppressed_warnings", result.suppressed_warnings() as i64)
        .with_number("diagnostics", result.diagnostic_count() as i64);
    lines.push(formatter.format(&summary));
    lines.join("\n")
}

fn render_json(
    result: &ScanResult,
    diagnostics: &[Diagnostic],
    workspace_root: &Path,
    locale: Locale,
    formatter: &MessageFormatter,
) -> Value {
    let values = diagnostics
        .iter()
        .map(|diagnostic| diagnostic_json(diagnostic, workspace_root, formatter))
        .collect::<Vec<_>>();
    json!({
        "schema_version": JSON_SCHEMA_VERSION,
        "locale": locale.as_str(),
        "status": result.status().as_str(),
        "summary": {
            "errors": result.error_count(),
            "warnings": result.warning_count(),
            "suppressed_warnings": result.suppressed_warnings(),
            "diagnostics": result.diagnostic_count(),
        },
        "diagnostics": values,
    })
}

fn diagnostic_json(
    diagnostic: &Diagnostic,
    workspace_root: &Path,
    formatter: &MessageFormatter,
) -> Value {
    let span = diagnostic.span();
    let fields = diagnostic_fields(diagnostic);
    json!({
        "kind": fields.kind,
        "rule_id": fields.rule_id,
        "tool_error": fields.tool_error,
        "analysis_issue": fields.analysis_issue,
        "severity": diagnostic.severity().as_str(),
        "package": diagnostic.package(),
        "package_manifest_path": diagnostic.package_manifest_path().map(|path| {
            display_path(Some(path), workspace_root)
        }),
        "target": diagnostic.target(),
        "target_kind": diagnostic.target_kind(),
        "path": span.map(|value| display_path(Some(value.path()), workspace_root)),
        "start": span.map(|value| json!({
            "line": value.start_line(),
            "column": value.start_column(),
        })),
        "end": span.map(|value| json!({
            "line": value.end_line(),
            "column": value.end_column(),
        })),
        "message_id": diagnostic.message().id().as_str(),
        "arguments": message_arguments_json(diagnostic.message()),
        "message": json_message(diagnostic.message(), formatter),
        "detail": opaque_detail(diagnostic.message()),
        "namespace": diagnostic.namespace(),
        "cfg_condition": diagnostic.cfg_condition(),
    })
}

fn message_arguments_json(message: &LocalizedMessage) -> Value {
    let arguments = message
        .arguments()
        .iter()
        .filter(|(_, argument)| !matches!(argument, MessageArgument::Opaque(_)))
        .map(|(name, argument)| {
            let value = match argument {
                MessageArgument::Text(value) | MessageArgument::Opaque(value) => {
                    Value::String(value.clone())
                }
                MessageArgument::Number(value) => json!(value),
                MessageArgument::Boolean(value) => json!(value),
            };
            (name.clone(), value)
        })
        .collect::<Map<String, Value>>();
    Value::Object(arguments)
}

fn json_message(message: &LocalizedMessage, formatter: &MessageFormatter) -> String {
    let detail_reference = formatter.text(REPORT_DETAIL_REFERENCE);
    let display_message = message
        .arguments()
        .iter()
        .filter(|(_, argument)| matches!(argument, MessageArgument::Opaque(_)))
        .fold(message.clone(), |display_message, (name, _)| {
            display_message.with_opaque(name, detail_reference.clone())
        });
    formatter.format(&display_message)
}

fn opaque_detail(message: &LocalizedMessage) -> Option<&str> {
    match message.arguments().get("detail") {
        Some(MessageArgument::Opaque(value)) => Some(value),
        _ => None,
    }
}

fn diagnostic_fields(diagnostic: &Diagnostic) -> DiagnosticTypeFields {
    match diagnostic.kind() {
        DiagnosticKind::Rule(rule) => DiagnosticTypeFields {
            kind: "rule",
            rule_id: Some(rule.as_str()),
            tool_error: None,
            analysis_issue: None,
        },
        DiagnosticKind::Tool(error) => DiagnosticTypeFields {
            kind: "tool",
            rule_id: None,
            tool_error: Some(error.as_str()),
            analysis_issue: None,
        },
        DiagnosticKind::Analysis(issue) => DiagnosticTypeFields {
            kind: "analysis",
            rule_id: None,
            tool_error: None,
            analysis_issue: Some(issue.as_str()),
        },
    }
}

fn diagnostic_code(diagnostic: &Diagnostic) -> String {
    match diagnostic.kind() {
        DiagnosticKind::Rule(rule) => rule.as_str().to_owned(),
        DiagnosticKind::Tool(error) => format!("tool:{}", error.as_str()),
        DiagnosticKind::Analysis(issue) => format!("analysis:{}", issue.as_str()),
    }
}

fn display_path(path: Option<&Path>, workspace_root: &Path) -> String {
    let Some(path) = path else {
        return String::new();
    };
    let display = path
        .strip_prefix(workspace_root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| PathBuf::from(path));
    display.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use cop_model::{
        Diagnostic, LocalizedMessage, RuleId, ScanResult, ScanStatus, Severity, SourceLocation,
        SourceSpan, ToolErrorKind, CONFIG_FILE_READ_ERROR, ITEM_SPACING, MODULE_FILE_LAYOUT,
    };
    use serde_json::{from_str, json, Value};

    use super::{Locale, ReportFormat, Reporter};

    fn located_rule(path: &str, line: u32) -> Diagnostic {
        let start = SourceLocation::new(path, 0, line, 1).expect("诊断起点");
        let end = SourceLocation::new(path, 1, line, 2).expect("诊断终点");
        let span = SourceSpan::new(start, end).expect("诊断范围");
        Diagnostic::rule(
            RuleId::ItemSpacing,
            Severity::Error,
            LocalizedMessage::new(ITEM_SPACING)
                .with_number("expected", 1)
                .with_number("actual", 0),
        )
        .with_package("sample")
        .with_package_manifest_path("/workspace/Cargo.toml")
        .with_target("sample")
        .with_target_kind(vec!["lib".to_owned()])
        .with_span(span)
    }

    #[test]
    fn sorts_json_diagnostics_by_relative_path_and_position() {
        let result = ScanResult::new(
            ScanStatus::Complete,
            vec![
                located_rule("/workspace/src/z.rs", 1),
                located_rule("/workspace/src/a.rs", 7),
                located_rule("/workspace/src/a.rs", 1),
            ],
            0,
        );

        let rendered = Reporter::render(
            &result,
            Path::new("/workspace"),
            ReportFormat::Json,
            Locale::ZH_CN,
        );
        let report = from_str::<Value>(&rendered).expect("JSON 报告");
        let diagnostics = report["diagnostics"].as_array().expect("诊断数组");

        assert_eq!(report["schema_version"], 3);
        assert_eq!(report["locale"], "zh-CN");
        assert_eq!(diagnostics[0]["path"], "src/a.rs");
        assert_eq!(diagnostics[0]["start"]["line"], 1);
        assert_eq!(diagnostics[1]["start"]["line"], 7);
        assert_eq!(diagnostics[2]["path"], "src/z.rs");
        assert_eq!(diagnostics[0]["tool_error"], Value::Null);
    }

    #[test]
    fn renders_human_namespace_and_cfg_context() {
        let diagnostic = Diagnostic::rule(
            RuleId::ModuleFileLayout,
            Severity::Warning,
            LocalizedMessage::new(MODULE_FILE_LAYOUT),
        )
        .with_namespace("crate::child")
        .with_cfg_condition(r#"feature = "extra""#);
        let result = ScanResult::new(ScanStatus::Complete, vec![diagnostic], 0);

        let rendered = Reporter::render(
            &result,
            Path::new("/workspace"),
            ReportFormat::Human,
            Locale::ZH_CN,
        );

        assert!(rendered.contains("warning[module_file_layout]"));
        assert!(rendered.contains("[命名空间: crate::child]"));
        assert!(rendered.contains(r#"[配置条件: feature = "extra"]"#));
    }

    #[test]
    fn localizes_scan_status_in_human_summary() {
        let result = ScanResult::new(ScanStatus::Partial, Vec::new(), 0);

        let chinese = Reporter::render(
            &result,
            Path::new("/workspace"),
            ReportFormat::Human,
            Locale::ZH_CN,
        )
        .replace(['\u{2068}', '\u{2069}'], "");
        let english = Reporter::render(
            &result,
            Path::new("/workspace"),
            ReportFormat::Human,
            Locale::EN_US,
        )
        .replace(['\u{2068}', '\u{2069}'], "");

        assert!(chinese.contains("状态 部分完成"));
        assert!(english.contains("status partial"));
    }

    #[test]
    fn keeps_opaque_details_out_of_the_localized_json_message() {
        let diagnostic = Diagnostic::tool(
            ToolErrorKind::Configuration,
            LocalizedMessage::new(CONFIG_FILE_READ_ERROR)
                .with_text("path", "cargo-cop.toml")
                .with_opaque("detail", "permission denied"),
        );
        let result = ScanResult::new(ScanStatus::Incomplete, vec![diagnostic], 0);

        let rendered = Reporter::render(
            &result,
            Path::new("/workspace"),
            ReportFormat::Json,
            Locale::EN_US,
        );
        let report = from_str::<Value>(&rendered).expect("JSON 报告");
        let diagnostic = &report["diagnostics"][0];

        let message = diagnostic["message"]
            .as_str()
            .unwrap()
            .replace(['\u{2068}', '\u{2069}'], "");
        assert_eq!(
            message,
            "Could not read configuration file cargo-cop.toml: see the detail field"
        );
        assert_eq!(diagnostic["arguments"], json!({"path": "cargo-cop.toml"}));
        assert_eq!(diagnostic["detail"], "permission denied");
        assert!(!message.contains("permission denied"));
    }
}
