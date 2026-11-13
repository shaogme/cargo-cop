use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::{id, Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::{from_str, json, Value};

use cop_analysis::{
    AnalysisRequest, AnalysisSnapshot, CfgDecision, ConfiguredAstIssueKind, ExternalItemKind,
    ExternalItemSource, ModuleNode, PathResolutionKind, PathRootKind, ProjectAnalyzer,
    ReferenceKind,
};
use cop_cargo::{
    CargoDiscoveryRequest, CargoProjectLoader, CargoSelection, FeatureSelection, TargetSelector,
};
use cop_config::{ConfigLoader, ConfigRequest};
use cop_model::{AnalysisIssueKind, DiagnosticKind};
use cop_syntax::{MacroInputStatus, MacroProfileKey};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

struct RunOutput {
    status: i32,
    stdout: String,
    stderr: String,
}

struct ExpectedPathDiagnostic {
    line: usize,
    start_column: usize,
    end_column: usize,
    expected_path: &'static str,
}

struct ModulePathExpectation {
    parent: &'static str,
    child: &'static str,
    path: &'static str,
}

struct VisibilityExcerpt<'a> {
    path: &'a str,
    line: usize,
    text: String,
}

struct ConfiguredNodeExpectation<'a> {
    marker: &'a str,
    kind: &'a str,
    decision: CfgDecision,
}

impl Fixture {
    fn new() -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!("cargo-cop-acceptance-{}-{sequence}", id()));
        fs::create_dir_all(&root).expect("创建临时 fixture 目录");
        Self { root }
    }

    fn write(&self, relative_path: &str, contents: &str) {
        let path = self.root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("创建 fixture 父目录");
        }
        fs::write(path, contents).expect("写入 fixture 文件");
    }

    fn run(&self, arguments: &[&str]) -> RunOutput {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-cop"))
            .args(arguments)
            .current_dir(&self.root)
            .output()
            .expect("运行 cargo-cop");
        RunOutput::from(output)
    }

    fn run_cargo_subcommand(&self, arguments: &[&str]) -> RunOutput {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_cargo-cop"));
        let mut paths =
            env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>();
        paths.insert(0, binary.parent().expect("二进制目录").to_path_buf());
        let path = env::join_paths(paths).expect("组装 PATH");
        let output = Command::new("cargo")
            .arg("cop")
            .args(arguments)
            .current_dir(&self.root)
            .env("PATH", path)
            .output()
            .expect("运行 cargo cop");
        RunOutput::from(output)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl RunOutput {
    fn from(output: Output) -> Self {
        Self {
            status: output.status.code().unwrap_or(255),
            stdout: String::from_utf8(output.stdout).expect("stdout UTF-8"),
            stderr: String::from_utf8(output.stderr).expect("stderr UTF-8"),
        }
    }

    fn has_rule(&self, rule: &str) -> bool {
        self.stdout.contains(&format!("\"rule_id\":\"{rule}\""))
            || self.stdout.contains(&format!("[{rule}]"))
    }
}

fn package_manifest(name: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")
}

fn analyze_fixture(fixture: &Fixture) -> AnalysisSnapshot {
    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo metadata");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), true, true))
            .with_current_directory(fixture.root.clone()),
    )
    .expect("select package and target");
    let package = project.packages().first().expect("selected package");
    ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("analyze selected package")
}

fn code_lines_for_count(line_count: usize) -> String {
    (0..line_count)
        .map(|index| format!("fn item_{index}() {{}}\n"))
        .collect()
}

fn item_spacing_diagnostics(output: &RunOutput) -> Vec<Value> {
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    report["diagnostics"]
        .as_array()
        .expect("诊断数组")
        .iter()
        .filter(|diagnostic| diagnostic["rule_id"] == "item_spacing")
        .cloned()
        .collect()
}

fn rule_diagnostics(output: &RunOutput, rule_id: &str) -> Vec<Value> {
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    report["diagnostics"]
        .as_array()
        .expect("诊断数组")
        .iter()
        .filter(|diagnostic| diagnostic["rule_id"] == rule_id)
        .cloned()
        .collect()
}

fn diagnostic_message(diagnostic: &Value) -> &str {
    diagnostic["message"].as_str().expect("已渲染诊断消息")
}

fn diagnostic_excerpt<'a>(source: &'a str, diagnostic: &Value) -> &'a str {
    let line = usize::try_from(diagnostic["start"]["line"].as_u64().expect("诊断起始行"))
        .expect("诊断起始行范围");
    let start_column = usize::try_from(diagnostic["start"]["column"].as_u64().expect("诊断起始列"))
        .expect("诊断起始列范围");
    let end_column = usize::try_from(diagnostic["end"]["column"].as_u64().expect("诊断结束列"))
        .expect("诊断结束列范围");
    let source_line = source.lines().nth(line - 1).expect("诊断对应源码行");
    &source_line[start_column - 1..end_column - 1]
}

#[derive(Debug, Eq, PartialEq)]
enum SpacingExpectation {
    Exact(i64),
    AtMost(i64),
}

fn spacing_values(diagnostic: &Value) -> (SpacingExpectation, i64) {
    let message = diagnostic_message(diagnostic);
    let (expectation, values) = if let Some((_, values)) = message.split_once("require ") {
        let (expected, actual) = values
            .split_once(" blank lines; found ")
            .expect("英文精确间距消息");
        (
            SpacingExpectation::Exact(expected.parse().expect("预期空行数")),
            actual,
        )
    } else if let Some((_, values)) = message.split_once("at most ") {
        let (maximum, actual) = values
            .split_once(" blank lines; found ")
            .expect("英文间距上限消息");
        (
            SpacingExpectation::AtMost(maximum.parse().expect("最多空行数")),
            actual,
        )
    } else if let Some((_, values)) = message.split_once("应有 ") {
        let (expected, actual) = values
            .split_once(" 个空行，当前有 ")
            .expect("中文精确间距参数");
        (
            SpacingExpectation::Exact(expected.parse().expect("预期空行数")),
            actual,
        )
    } else {
        let (_, values) = message.split_once("最多允许 ").expect("中文间距上限消息");
        let (maximum, actual) = values
            .split_once(" 个空行，当前有 ")
            .expect("中文间距上限参数");
        (
            SpacingExpectation::AtMost(maximum.parse().expect("最多空行数")),
            actual,
        )
    };
    let actual = values
        .split_whitespace()
        .next()
        .expect("实际空行数")
        .parse()
        .expect("实际空行数");
    (expectation, actual)
}

fn reported_import_line(diagnostic: &Value) -> usize {
    let message = diagnostic_message(diagnostic);
    let value = if let Some((_, value)) = message.split_once("line ") {
        value
    } else {
        message.split_once("第 ").expect("中文导入行号").1
    };
    let start = value
        .find(|character: char| character.is_ascii_digit())
        .expect("导入行号");
    let digits = value[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.parse().expect("导入行号")
}

fn has_diagnostic_message_fragment(output: &RunOutput, fragment: &str) -> bool {
    from_str::<Value>(&output.stdout)
        .ok()
        .and_then(|report| report.get("diagnostics").and_then(Value::as_array).cloned())
        .is_some_and(|diagnostics| {
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic_message(diagnostic).contains(fragment))
        })
}

fn diagnostic_name(diagnostic: &Value) -> &str {
    let message = diagnostic_message(diagnostic);
    let (prefix, suffix) = if message.starts_with("The crate-visible declaration ") {
        ("The crate-visible declaration ", " can be private")
    } else {
        ("crate 可见声明 ", " 可以改为私有")
    };
    message
        .strip_prefix(prefix)
        .and_then(|value| value.strip_suffix(suffix))
        .expect("规则消息中包含声明名称")
}

fn message_contains_either(diagnostic: &Value, english: &str, chinese: &str) -> bool {
    let message = diagnostic_message(diagnostic);
    message.contains(english) || message.contains(chinese)
}

fn write_package(root: &Path, name: &str, source: &str) {
    fs::create_dir_all(root.join(name).join("src")).expect("创建 package 源码目录");
    fs::write(root.join(name).join("Cargo.toml"), package_manifest(name))
        .expect("写入 package manifest");
    fs::write(root.join(name).join("src/lib.rs"), source).expect("写入 package 根源码");
}

#[test]
fn cfg_macro_values_are_target_scoped_and_do_not_change_availability() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[test]]\nname = \"integration\"\npath = \"tests/integration.rs\"\n",
    );
    fixture.write(
        "src/lib.rs",
        "pub fn check() { let _ = cfg!(flavor = \"fast\"); let _ = cfg!(any()); let _ = cfg!(test); }\n",
    );
    fixture.write(
        "tests/integration.rs",
        "#[test]\nfn check() { let _ = cfg!(flavor = \"fast\"); let _ = cfg!(test); }\n",
    );
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[[cfg]]\npackage = \"sample\"\ntarget = \"sample\"\nname = \"flavor\"\nvalue = \"fast\"\n\n[[cfg]]\npackage = \"sample\"\ntarget = \"integration\"\nname = \"flavor\"\nvalue = \"slow\"\n",
    );
    let config = ConfigLoader::load(ConfigRequest::new(
        Some(fixture.root.join("cargo-cop.toml")),
        fixture.root.clone(),
        fixture.root.clone(),
    ))
    .expect("读取 cfg 配置");
    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo 元数据");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), false, true))
            .with_all_targets(true)
            .with_cfg_overrides(config.cfg_overrides().to_vec())
            .with_current_directory(fixture.root.clone()),
    )
    .expect("选择所有 targets");
    let package = project.packages().first().expect("选中的 package");
    let snapshot =
        ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("catalog messages render");
    let mut lib_flavor = Vec::new();
    let mut integration_flavor = Vec::new();
    let mut lib_test = Vec::new();
    let mut false_predicates = 0;
    for fact in snapshot.facts().cfg_macros() {
        let module = snapshot
            .modules()
            .get(fact.module_id().index())
            .expect("cfg macro module");
        let configured = module
            .configured_ast()
            .node(fact.ast_node_id())
            .expect("cfg macro AST node");
        assert_eq!(configured.availability_decision(), CfgDecision::True);
        match fact.predicate().normalized().as_str() {
            "flavor = \"fast\"" if module.target_name() == "sample" => {
                lib_flavor.push(fact.decision());
            }
            "flavor = \"fast\"" if module.target_name() == "integration" => {
                integration_flavor.push(fact.decision());
            }
            "test" if module.target_name() == "sample" => lib_test.push(fact.decision()),
            "any()" => {
                assert_eq!(fact.decision(), CfgDecision::False);
                false_predicates += 1;
            }
            _ => {}
        }
    }

    assert!(!lib_flavor.is_empty());
    assert!(lib_flavor
        .iter()
        .all(|decision| *decision == CfgDecision::True));
    assert_eq!(integration_flavor, [CfgDecision::False]);
    assert!(lib_test.contains(&CfgDecision::False));
    assert!(lib_test.contains(&CfgDecision::True));
    assert!(false_predicates > 0);
}

#[test]
fn cfg_macro_unknown_reports_predicate_span_and_cli_values_resolve_it() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = "pub fn check() { let _ = cfg!(custom_mode = \"fast\"); }\n";
    fixture.write("src/lib.rs", source);

    let unknown = fixture.run(&["--format", "json"]);
    let unknown_report = from_str::<Value>(&unknown.stdout).expect("JSON 报告");
    let unknown_diagnostic = unknown_report["diagnostics"]
        .as_array()
        .expect("诊断数组")
        .iter()
        .find(|diagnostic| diagnostic["analysis_issue"] == "unknown_cfg")
        .expect("unknown_cfg 诊断");
    let start = source.find("custom_mode").expect("predicate 起点");

    assert_eq!(unknown.status, 0, "{}", unknown.stdout);
    assert_eq!(unknown_report["status"], "partial");
    assert!(diagnostic_message(unknown_diagnostic).contains("custom_mode = \"fast\""));
    assert_eq!(unknown_diagnostic["start"]["column"], start + 1);
    assert_eq!(
        unknown_diagnostic["end"]["column"],
        start + "custom_mode = \"fast\"".len() + 1
    );

    let resolved = fixture.run(&["--format", "json", "--cfg", "custom_mode=\"fast\""]);
    assert_eq!(resolved.status, 0, "{}", resolved.stdout);
    assert!(!resolved
        .stdout
        .contains("\"analysis_issue\":\"unknown_cfg\""));
}

#[test]
fn cfg_candidate_shadowed_by_macro_rules_stays_opaque() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "macro_rules! cfg { ($($tokens:tt)*) => { false } }\npub fn check() { let _ = cfg!(custom_mode); }\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    let diagnostics = report["diagnostics"].as_array().expect("诊断数组");

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(!diagnostics
        .iter()
        .any(|diagnostic| diagnostic["analysis_issue"] == "unknown_cfg"));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["analysis_issue"] == "parser_coverage"
            && message_contains_either(
                diagnostic,
                "Could not confirm this cfg! call",
                "无法确认此 cfg! 调用",
            )
    }));
}

#[test]
fn cfg_candidate_with_macro_use_import_stays_opaque() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[macro_use]\nextern crate std;\npub fn check() { let _ = cfg!(custom_mode); }\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    let diagnostics = report["diagnostics"].as_array().expect("诊断数组");

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(!diagnostics
        .iter()
        .any(|diagnostic| diagnostic["analysis_issue"] == "unknown_cfg"));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["analysis_issue"] == "parser_coverage"
            && message_contains_either(
                diagnostic,
                "Could not confirm this cfg! call",
                "无法确认此 cfg! 调用",
            )
    }));
}

#[test]
fn cfg_candidate_with_macro_use_module_stays_opaque() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.child_module_line_count]\nenabled = false\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[macro_use]\nmod macros;\npub fn check() { let _ = cfg!(custom_mode); }\n",
    );
    fixture.write(
        "src/macros.rs",
        "macro_rules! cfg { ($($tokens:tt)*) => { false } }\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    let diagnostics = report["diagnostics"].as_array().expect("诊断数组");

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(!diagnostics
        .iter()
        .any(|diagnostic| diagnostic["analysis_issue"] == "unknown_cfg"));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["analysis_issue"] == "parser_coverage"
            && message_contains_either(
                diagnostic,
                "Could not confirm this cfg! call",
                "无法确认此 cfg! 调用",
            )
    }));
}

#[test]
fn cfg_candidate_before_macro_use_module_declaration_is_evaluated() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.child_module_line_count]\nenabled = false\n\n[rules.out_of_line_modules_at_scope_start]\nenabled = false\n",
    );
    fixture.write(
        "src/lib.rs",
        "pub fn check() { let _ = cfg!(custom_mode); }\n#[macro_use]\nmod macros;\n",
    );
    fixture.write(
        "src/macros.rs",
        "macro_rules! cfg { ($($tokens:tt)*) => { false } }\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    let diagnostics = report["diagnostics"].as_array().expect("诊断数组");

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["analysis_issue"] == "unknown_cfg"));
    assert!(!diagnostics.iter().any(|diagnostic| {
        diagnostic["analysis_issue"] == "parser_coverage"
            && message_contains_either(
                diagnostic,
                "Could not confirm this cfg! call",
                "无法确认此 cfg! 调用",
            )
    }));
}

#[test]
fn malformed_builtin_cfg_predicate_reports_the_failing_source_position() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = "pub fn check() { let _ = cfg!(not(unix, windows)); }\n";
    fixture.write("src/lib.rs", source);

    let output = fixture.run(&["--format", "json"]);
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    let diagnostic = report["diagnostics"]
        .as_array()
        .expect("诊断数组")
        .iter()
        .find(|diagnostic| {
            diagnostic["analysis_issue"] == "parser_coverage"
                && message_contains_either(
                    diagnostic,
                    "Could not parse the cfg! predicate",
                    "无法解析此处的 cfg! predicate",
                )
        })
        .expect("malformed cfg predicate 诊断");
    let failing_token = source.find("windows").expect("错误参数位置");

    assert_eq!(output.status, 2, "{}", output.stdout);
    assert_eq!(diagnostic["start"]["column"], failing_token + 1);
    assert_eq!(diagnostic["end"]["column"], failing_token + 2);
    assert!(!output.stdout.contains("\"analysis_issue\":\"unknown_cfg\""));
}

#[test]
fn scoped_custom_cfg_values_apply_to_target_specific_dependencies() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"vendor\"]\nresolver = \"2\"\n\n[target.'cfg(custom_link)'.dependencies]\nvendor = { path = \"vendor\" }\n",
    );
    fixture.write("src/lib.rs", "fn direct() { let _ = vendor::Thing; }\n");
    fixture.write(
        "vendor/Cargo.toml",
        "[package]\nname = \"vendor\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    fixture.write("vendor/src/lib.rs", "pub struct Thing;\n");
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[[cfg]]\npackage = \"app\"\ntarget = \"app\"\nname = \"custom_link\"\n",
    );

    let output = fixture.run(&["--format", "json", "--package", "app"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert!(output.has_rule("no_inline_qualified_path"));
    assert!(!output.stdout.contains("\"analysis_issue\":\"unknown_cfg\""));
}

#[test]
fn scans_direct_binary_and_cargo_subcommand_with_identical_output() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "pub fn ready() {}\n");
    let direct = fixture.run(&["--format", "json"]);
    let cargo = fixture.run_cargo_subcommand(&["--format", "json"]);
    assert_eq!(direct.status, 0, "{}", direct.stderr);
    assert_eq!(cargo.status, 0, "{}", cargo.stderr);
    assert_eq!(direct.stdout, cargo.stdout);
    assert!(direct.stdout.contains("\"schema_version\":5"));
    assert!(direct.stdout.contains("\"status\":\"complete\""));
}

#[test]
fn selects_workspace_packages_and_all_targets() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"first\", \"second\"]\nresolver = \"2\"\n",
    );
    write_package(
        &fixture.root,
        "first",
        "#[allow(dead_code)]\nfn first() {}\n",
    );
    write_package(
        &fixture.root,
        "second",
        "#[allow(dead_code)]\nfn second() {}\n",
    );
    fixture.write(
        "first/examples/example.rs",
        "#[allow(dead_code)]\nfn example_only() {}\nfn main() {}\n",
    );
    fixture.write(
        "first/Cargo.toml",
        "[package]\nname = \"first\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[[example]]\nname = \"example\"\npath = \"examples/example.rs\"\n",
    );
    let workspace = fixture.run(&["--format", "json"]);
    let package = fixture.run(&["--format", "json", "--package", "first"]);
    let all_targets = fixture.run(&["--format", "json", "--package", "first", "--all-targets"]);
    let package_manifest =
        fixture.run(&["--format", "json", "--manifest-path", "first/Cargo.toml"]);
    let package_workspace = fixture.run(&[
        "--format",
        "json",
        "--manifest-path",
        "first/Cargo.toml",
        "--workspace",
    ]);
    assert_eq!(workspace.status, 1, "{}", workspace.stderr);
    assert!(workspace.stdout.contains("\"package\":\"first\""));
    assert!(workspace.stdout.contains("\"package\":\"second\""));
    assert_eq!(package.status, 1, "{}", package.stderr);
    assert!(package.stdout.contains("\"package\":\"first\""));
    assert!(!package.stdout.contains("\"package\":\"second\""));
    assert!(!package.stdout.contains("example_only"));
    assert_eq!(all_targets.status, 1, "{}", all_targets.stderr);
    assert!(all_targets.stdout.contains("examples/example.rs"));
    assert_eq!(package_manifest.status, 1, "{}", package_manifest.stderr);
    assert!(package_manifest.stdout.contains("\"package\":\"first\""));
    assert!(!package_manifest.stdout.contains("\"package\":\"second\""));
    assert_eq!(package_workspace.status, 1, "{}", package_workspace.stderr);
    assert!(package_workspace.stdout.contains("\"package\":\"first\""));
    assert!(package_workspace.stdout.contains("\"package\":\"second\""));
}

#[test]
fn applies_config_severity_and_command_line_precedence() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "#[allow(dead_code)]\nfn unused() {}\n");
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.no_lint_relaxation]\nseverity = \"warning\"\n",
    );
    let configured = fixture.run(&["--format", "json"]);
    let overridden = fixture.run(&["--format", "json", "--severity", "no_lint_relaxation=error"]);
    let disabled = fixture.run(&["--format", "json", "--disable-rule", "no_lint_relaxation"]);
    assert_eq!(configured.status, 0, "{}", configured.stderr);
    assert!(configured.stdout.contains("\"severity\":\"warning\""));
    assert_eq!(overridden.status, 1, "{}", overridden.stderr);
    assert!(overridden.stdout.contains("\"severity\":\"error\""));
    assert_eq!(disabled.status, 0, "{}", disabled.stderr);
    assert!(!disabled.has_rule("no_lint_relaxation"));
}

#[test]
fn displays_warnings_by_default() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "include!(\"generated.rs\");\n");
    fixture.write("src/generated.rs", "pub fn generated() {}\n");

    let output = fixture.run(&["--format", "json"]);
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");

    assert_eq!(output.status, 0, "{}", output.stderr);
    assert_eq!(report["status"], "partial");
    assert_eq!(report["summary"]["warnings"], 1);
    assert_eq!(report["summary"]["suppressed_warnings"], 0);
    assert_eq!(report["diagnostics"][0]["analysis_issue"], "dynamic_input");
}

#[test]
fn excludes_include_macros_inside_false_trait_members() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "trait Conditional { #[cfg(any())] fn disabled() { include!(\"hidden.rs\"); } }\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 0, "{}", output.stderr);
    assert!(!output
        .stdout
        .contains("\"analysis_issue\":\"dynamic_input\""));
}

#[test]
fn suppresses_warnings_but_preserves_errors_and_partial_status() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod child;\n#[cfg(mystery_flag)]\nmod conditional;\ninclude!(\"generated.rs\");\nmake_modules!();\n#[allow(dead_code)]\nfn relaxed() {}\nstruct First;\nstruct Second;\n",
    );
    fixture.write("src/child.rs", "pub fn exposed() {}\n");
    fixture.write("src/conditional.rs", "pub fn conditional() {}\n");
    fixture.write("src/generated.rs", "pub fn generated() {}\n");
    fixture.write(
        "cargo-cop.toml",
        "version = 4\nallow_warnings = true\n\n[rules.no_lint_relaxation]\nseverity = \"warning\"\n\n[rules.item_spacing]\nseverity = \"warning\"\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    let diagnostics = report["diagnostics"].as_array().expect("诊断数组");

    assert_eq!(output.status, 1, "{}", output.stderr);
    assert_eq!(report["status"], "partial");
    assert_eq!(report["summary"]["warnings"], 0);
    assert!(report["summary"]["suppressed_warnings"]
        .as_u64()
        .is_some_and(|count| count >= 5));
    let error_count = report["summary"]["errors"].as_u64().expect("错误数");
    assert!(error_count > 0);
    assert_eq!(diagnostics.len() as u64, error_count);
    assert!(diagnostics
        .iter()
        .all(|diagnostic| diagnostic["severity"] == "error"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["rule_id"] == "child_public_free_functions"));

    fixture.write("src/lib.rs", "mod absent;\ninclude!(\"generated.rs\");\n");
    let incomplete = fixture.run(&["--format", "json"]);
    let incomplete_report = from_str::<Value>(&incomplete.stdout).expect("JSON 部分扫描报告");

    assert_eq!(incomplete.status, 2, "{}", incomplete.stderr);
    assert_eq!(incomplete_report["status"], "incomplete");
    assert_eq!(incomplete_report["summary"]["suppressed_warnings"], 1);
}

#[test]
fn uses_one_resolved_locale_for_help_and_json_messages() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"member\"]\nresolver = \"2\"\n",
    );
    fixture.write("member/Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "member/src/lib.rs",
        "#[allow(dead_code)]\nfn hidden() {}\n\npub struct First;\npub struct Second;\n",
    );
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[i18n]\nlocale = \"en-US\"\n",
    );

    let help = fixture.run(&["--manifest-path", "member/Cargo.toml", "--help"]);
    let configured = fixture.run(&["--manifest-path", "member/Cargo.toml", "--format", "json"]);
    let overridden = fixture.run(&[
        "--manifest-path",
        "member/Cargo.toml",
        "--format",
        "json",
        "--locale",
        "zh-CN",
    ]);
    let english_human = fixture.run(&[
        "--manifest-path",
        "member/Cargo.toml",
        "--format",
        "human",
        "--locale",
        "en-US",
    ]);
    let chinese_human = fixture.run(&[
        "--manifest-path",
        "member/Cargo.toml",
        "--format",
        "human",
        "--locale",
        "zh-CN",
    ]);
    let regional_fallback = fixture.run(&[
        "--manifest-path",
        "member/Cargo.toml",
        "--format",
        "json",
        "--locale",
        "en-150",
    ]);
    let invalid_arguments = fixture.run(&["--format", "json", "--locale", "en-US", "--unknown"]);
    let english = from_str::<Value>(&configured.stdout).expect("英文 JSON 报告");
    let chinese = from_str::<Value>(&overridden.stdout).expect("中文 JSON 报告");
    let argument_error = from_str::<Value>(&invalid_arguments.stdout).expect("参数错误 JSON 报告");
    let english_spacing_diagnostics = rule_diagnostics(&configured, "item_spacing");
    let chinese_spacing_diagnostics = rule_diagnostics(&overridden, "item_spacing");
    let english_diagnostic = &english_spacing_diagnostics[0];
    let chinese_diagnostic = &chinese_spacing_diagnostics[0];
    let english_diagnostics = english["diagnostics"].as_array().expect("英文诊断数组");
    let chinese_diagnostics = chinese["diagnostics"].as_array().expect("中文诊断数组");

    assert!(help.stdout.contains("Usage: cargo-cop"));
    assert_eq!(configured.status, 1, "{}", configured.stderr);
    assert_eq!(overridden.status, configured.status);
    assert_eq!(english["locale"], "en-US");
    assert_eq!(chinese["locale"], "zh-CN");
    assert_eq!(english["status"], chinese["status"]);
    assert_eq!(english["summary"], chinese["summary"]);
    assert_eq!(english_diagnostic["rule_id"], "item_spacing");
    assert_eq!(english_diagnostic["rule_id"], chinese_diagnostic["rule_id"]);
    assert_ne!(english_diagnostic["message"], chinese_diagnostic["message"]);
    assert_eq!(english["diagnostics"].as_array().map(Vec::len), Some(2));
    assert_eq!(english_human.status, configured.status);
    assert_eq!(chinese_human.status, configured.status);
    assert_eq!(regional_fallback.status, configured.status);
    assert_eq!(
        from_str::<Value>(&regional_fallback.stdout).expect("regional locale JSON")["locale"],
        "en-US"
    );
    for diagnostic in english_diagnostics {
        assert!(english_human
            .stdout
            .contains(diagnostic_message(diagnostic)));
    }
    for diagnostic in chinese_diagnostics {
        assert!(chinese_human
            .stdout
            .contains(diagnostic_message(diagnostic)));
    }
    assert_eq!(invalid_arguments.status, 2);
    assert_eq!(argument_error["locale"], "en-US");
    assert_eq!(argument_error["diagnostics"][0]["tool_error"], "arguments");
}

#[test]
fn queries_the_localized_rule_catalog_without_project_discovery() {
    let fixture = Fixture::new();
    fixture.write("cargo-cop.toml", "not a valid configuration file");

    let listed = fixture.run(&["--list-rules", "--format", "json", "--locale", "en-US"]);
    let listed_report = from_str::<Value>(&listed.stdout).expect("规则目录 JSON");
    let listed_rules = listed_report["rules"].as_array().expect("规则数组");
    let rule_ids = listed_rules
        .iter()
        .map(|rule| rule["rule_id"].as_str().expect("规则 ID"))
        .collect::<Vec<_>>();

    assert_eq!(listed.status, 0, "{}", listed.stderr);
    assert_eq!(listed_report["schema_version"], 1);
    assert_eq!(listed_report["kind"], "rule_catalog");
    assert_eq!(
        rule_ids,
        [
            "module_file_layout",
            "no_lint_relaxation",
            "item_spacing",
            "child_module_visibility",
            "no_parent_path_attribute",
            "no_parent_private_dependency",
            "child_module_line_count",
            "impl_locality",
            "trait_impl_locality",
            "private_fields_for_behavior_types",
            "child_public_free_functions",
            "declaration_before_use",
            "parameter_limit",
            "tuple_arity",
            "no_inline_qualified_path",
            "merge_common_prefix_imports",
            "extern_crate_at_scope_start",
            "imports_at_scope_start",
            "out_of_line_modules_at_scope_start",
            "use_group_spacing",
            "cfg_inside_use_tree",
            "unnecessary_pub_crate",
            "no_pub_in_visibility",
        ]
    );
    for rule in listed_rules {
        assert!(!rule["summary"].as_str().expect("规则简述").is_empty());
        assert!(!rule["description"]
            .as_str()
            .expect("规则完整说明")
            .is_empty());
        assert_eq!(rule["default_enabled"], true);
        assert_eq!(rule["default_severity"], "error");
    }
    let inherent_rule = listed_rules
        .iter()
        .find(|rule| rule["rule_id"] == "impl_locality")
        .expect("固有 impl 规则");
    assert!(inherent_rule["description"]
        .as_str()
        .expect("固有 impl 说明")
        .contains("same source file"));
    let trait_rule = listed_rules
        .iter()
        .find(|rule| rule["rule_id"] == "trait_impl_locality")
        .expect("trait impl 规则");
    let trait_description = trait_rule["description"].as_str().expect("trait impl 说明");
    assert!(trait_description.contains("module namespace"));
    assert!(trait_description.contains("primitive"));
    assert!(trait_description.contains("external trait"));

    for rule_id in [
        "impl_locality",
        "trait_impl_locality",
        "no_pub_in_visibility",
        "use_group_spacing",
    ] {
        let explanation = fixture.run(&[
            "--explain-rule",
            rule_id,
            "--format",
            "json",
            "--locale",
            "en-US",
        ]);
        let report = from_str::<Value>(&explanation.stdout).expect("规则说明 JSON");
        assert_eq!(explanation.status, 0, "{}", explanation.stderr);
        assert_eq!(report["rule"]["rule_id"], rule_id);
        assert!(!report["rule"]["description"]
            .as_str()
            .expect("完整规则说明")
            .is_empty());
    }

    let cargo_list =
        fixture.run_cargo_subcommand(&["--list-rules", "--format", "json", "--locale", "en-US"]);
    assert_eq!(cargo_list.status, 0, "{}", cargo_list.stderr);
    assert_eq!(cargo_list.stdout, listed.stdout);

    let chinese_list = fixture.run(&["--list-rules", "--format", "json", "--locale", "zh-CN"]);
    let chinese_report = from_str::<Value>(&chinese_list.stdout).expect("中文规则目录 JSON");
    let chinese_rules = chinese_report["rules"].as_array().expect("中文规则数组");
    assert_eq!(chinese_list.status, 0, "{}", chinese_list.stderr);
    assert_eq!(chinese_rules.len(), listed_rules.len());
    for (english, chinese) in listed_rules.iter().zip(chinese_rules) {
        assert_eq!(english["rule_id"], chinese["rule_id"]);
        assert_ne!(english["summary"], chinese["summary"]);
        assert_ne!(english["description"], chinese["description"]);
    }

    let cargo_subcommand = fixture.run_cargo_subcommand(&[
        "--explain-rule",
        "parameter_limit",
        "--format",
        "json",
        "--locale",
        "zh-CN",
    ]);
    let explained_report = from_str::<Value>(&cargo_subcommand.stdout).expect("中文规则说明 JSON");
    let explained_rule = &explained_report["rule"];
    assert_eq!(cargo_subcommand.status, 0, "{}", cargo_subcommand.stderr);
    assert_eq!(explained_report["schema_version"], 1);
    assert_eq!(explained_report["kind"], "rule_catalog");
    assert_eq!(explained_rule["rule_id"], "parameter_limit");
    assert_eq!(explained_rule["default_enabled"], true);
    assert_eq!(explained_rule["default_severity"], "error");
    assert!(explained_rule["summary"]
        .as_str()
        .expect("规则简述")
        .contains('参'));
    assert!(!explained_rule["description"]
        .as_str()
        .expect("规则完整说明")
        .is_empty());

    let direct_explanation = fixture.run(&[
        "--explain-rule",
        "parameter_limit",
        "--format",
        "json",
        "--locale",
        "zh-CN",
    ]);
    assert_eq!(
        direct_explanation.status, 0,
        "{}",
        direct_explanation.stderr
    );
    assert_eq!(direct_explanation.stdout, cargo_subcommand.stdout);

    let human = fixture.run(&["--explain-rule", "parameter_limit"]);
    assert_eq!(human.status, 0, "{}", human.stderr);
    assert!(human.stdout.contains("[rules.parameter_limit]"));
    assert!(human.stdout.contains("--disable-rule parameter_limit"));
    assert!(human
        .stdout
        .contains("--severity parameter_limit=warning|error"));

    let human_list = fixture.run(&["--list-rules"]);
    assert_eq!(human_list.status, 0, "{}", human_list.stderr);
    assert!(human_list.stdout.contains("Rule catalog (23 rules):"));
    assert!(human_list
        .stdout
        .contains("parameter_limit [enabled, error]"));
    assert!(human_list
        .stdout
        .contains("no_pub_in_visibility [enabled, error]"));
    assert!(human_list
        .stdout
        .contains("use_group_spacing [enabled, error]"));
}

#[test]
fn reports_only_explicit_pub_in_visibility_at_each_local_scope() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = "pub( /* comment */ in crate) struct Root;\npub(crate) struct CrateVisible;\npub(super) struct ParentVisible;\npub(self) struct SelfVisible;\n#[cfg(any())]\npub(in crate) struct Disabled;\n#[cfg(mystery_cfg)]\npub(in crate) struct Unknown;\npub(in crate) mod restricted_api { pub(crate) struct Inner; }\nmod api {\n    pub(in crate::api) use core::fmt::Debug;\n    pub(crate) struct Record {\n        pub(in crate::api) field: u8,\n        pub(super) visible_to_parent: u8,\n    }\n    struct Local;\n    impl Local { pub(in crate::api) fn method() {} }\n    extern \"C\" { pub(in crate::api) fn foreign(); }\n    mod nested { pub(in crate::api::nested) fn nested_item() {} pub(super) fn parent_item() {} }\n}\nmod external;\n";
    fixture.write("src/lib.rs", source);
    fixture.write(
        "src/external.rs",
        "pub(in crate::external) struct External;\npub(self) struct LocalOnly;\n",
    );

    let default = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&default, "no_pub_in_visibility");
    assert_eq!(diagnostics.len(), 9, "{}", default.stdout);

    let mut actual = diagnostics
        .iter()
        .map(|diagnostic| {
            let path = diagnostic["path"].as_str().expect("诊断路径");
            let line = usize::try_from(diagnostic["start"]["line"].as_u64().expect("诊断起始行"))
                .expect("诊断起始行范围");
            let start =
                usize::try_from(diagnostic["start"]["column"].as_u64().expect("诊断起始列"))
                    .expect("诊断起始列范围");
            let end = usize::try_from(diagnostic["end"]["column"].as_u64().expect("诊断结束列"))
                .expect("诊断结束列范围");
            let file = match path {
                "src/lib.rs" => source,
                "src/external.rs" => {
                    "pub(in crate::external) struct External;\npub(self) struct LocalOnly;\n"
                }
                other => panic!("意外的诊断路径：{other}"),
            };
            let line_text = file.lines().nth(line - 1).expect("诊断源码行");
            VisibilityExcerpt {
                path,
                line,
                text: line_text[start - 1..end - 1].to_owned(),
            }
        })
        .collect::<Vec<_>>();
    actual.sort_by(|left, right| (left.path, left.line).cmp(&(right.path, right.line)));
    let reported = actual
        .iter()
        .map(|excerpt| (excerpt.path, excerpt.text.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        reported,
        [
            ("src/external.rs", "pub(in crate::external)"),
            ("src/lib.rs", "pub( /* comment */ in crate)"),
            ("src/lib.rs", "pub(in crate)"),
            ("src/lib.rs", "pub(in crate)"),
            ("src/lib.rs", "pub(in crate::api)"),
            ("src/lib.rs", "pub(in crate::api)"),
            ("src/lib.rs", "pub(in crate::api)"),
            ("src/lib.rs", "pub(in crate::api)"),
            ("src/lib.rs", "pub(in crate::api::nested)"),
        ],
        "{}",
        default.stdout
    );

    let warning = fixture.run(&[
        "--format",
        "json",
        "--severity",
        "no_pub_in_visibility=warning",
    ]);
    assert!(rule_diagnostics(&warning, "no_pub_in_visibility")
        .iter()
        .all(|diagnostic| diagnostic["severity"] == "warning"));

    let disabled = fixture.run(&["--format", "json", "--disable-rule", "no_pub_in_visibility"]);
    assert!(rule_diagnostics(&disabled, "no_pub_in_visibility").is_empty());
}

#[test]
fn rejects_invalid_rule_queries_and_query_option_conflicts() {
    let fixture = Fixture::new();
    let unknown = fixture.run(&[
        "--explain-rule",
        "not_a_rule",
        "--format",
        "json",
        "--locale",
        "zh-CN",
    ]);
    let unknown_report = from_str::<Value>(&unknown.stdout).expect("未知规则错误 JSON");
    assert_eq!(unknown.status, 2);
    assert_eq!(unknown_report["schema_version"], 5);
    assert_eq!(unknown_report["diagnostics"][0]["tool_error"], "arguments");
    assert!(unknown_report["diagnostics"][0]["message"]
        .as_str()
        .expect("本地化错误消息")
        .contains("未知规则"));

    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "pub fn ready() {}\n");
    for arguments in [
        vec![
            "--disable-rule",
            "cfg_use_group_spacing",
            "--format",
            "json",
        ],
        vec![
            "--severity",
            "cfg_use_group_spacing=warning",
            "--format",
            "json",
        ],
        vec![
            "--explain-rule",
            "cfg_use_group_spacing",
            "--format",
            "json",
        ],
    ] {
        let removed_id = fixture.run(&arguments);
        let report = from_str::<Value>(&removed_id.stdout).expect("已移除规则 ID 错误 JSON");
        assert_eq!(removed_id.status, 2, "{}", removed_id.stdout);
        assert!(report["diagnostics"][0]["message"]
            .as_str()
            .expect("未知规则 ID 消息")
            .contains("cfg_use_group_spacing"));
    }

    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.cfg_use_group_spacing]\nenabled = false\n",
    );
    let removed_config = fixture.run(&["--format", "json"]);
    assert_eq!(removed_config.status, 2, "{}", removed_config.stdout);
    assert!(removed_config.stdout.contains("cfg_use_group_spacing"));

    for arguments in [
        vec![
            "--list-rules",
            "--explain-rule",
            "parameter_limit",
            "--format",
            "json",
        ],
        vec!["--list-rules", "--workspace", "--format", "json"],
        vec![
            "--explain-rule",
            "parameter_limit",
            "--config",
            "missing.toml",
            "--format",
            "json",
        ],
    ] {
        let conflict = fixture.run(&arguments);
        let report = from_str::<Value>(&conflict.stdout).expect("参数冲突 JSON");
        assert_eq!(conflict.status, 2, "{}", conflict.stdout);
        assert_eq!(report["schema_version"], 5);
        assert_eq!(report["diagnostics"][0]["tool_error"], "arguments");
    }
}

#[test]
fn resolves_relative_explicit_config_without_merging_discovered_config() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "#[allow(dead_code)]\nfn ready() {}\n");
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.no_lint_relaxation]\nenabled = false\n",
    );
    fixture.write(
        "config/alternate.toml",
        "version = 4\n\n[rules.no_lint_relaxation]\nseverity = \"warning\"\n",
    );

    let explicit = fixture.run(&["--format", "json", "--config", "config/alternate.toml"]);
    let missing = fixture.run(&["--format", "json", "--config", "config/missing.toml"]);

    assert_eq!(explicit.status, 0, "{}", explicit.stdout);
    assert!(explicit.has_rule("no_lint_relaxation"));
    assert!(explicit.stdout.contains("\"severity\":\"warning\""));
    assert_eq!(missing.status, 2, "{}", missing.stdout);
    assert!(missing.stdout.contains("\"tool_error\":\"configuration\""));
}

#[test]
fn rejects_unknown_requested_features_instead_of_ignoring_them() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nknown = []\n",
    );
    fixture.write("src/lib.rs", "pub fn ready() {}\n");

    let output = fixture.run(&[
        "--format",
        "json",
        "--features",
        "typo",
        "--locale",
        "zh-CN",
    ]);

    assert_eq!(output.status, 2, "{}", output.stdout);
    assert!(output.stdout.contains("\"tool_error\":\"arguments\""));
    assert!(output.stdout.contains("未声明 feature"));
}

#[test]
fn reports_unknown_cfg_dynamic_inputs_and_module_path_ambiguity() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[allow(dead_code)]\nfn relaxed() {}\n#[cfg(mystery_flag)]\nmod conditional;\ninclude!(\"generated.rs\");\nmake_modules!();\n",
    );
    fixture.write("src/conditional.rs", "pub fn conditional() {}\n");
    fixture.write("src/generated.rs", "pub fn generated() {}\n");
    let incomplete = fixture.run(&["--format", "json"]);
    assert_eq!(incomplete.status, 1, "{}", incomplete.stderr);
    assert!(incomplete.stdout.contains("\"status\":\"partial\""));
    assert!(incomplete
        .stdout
        .contains("\"analysis_issue\":\"unknown_cfg\""));
    assert!(incomplete
        .stdout
        .contains("\"analysis_issue\":\"dynamic_input\""));
    assert!(incomplete
        .stdout
        .contains("\"analysis_issue\":\"macro_generated_module\""));
    assert!(incomplete.has_rule("no_lint_relaxation"));

    fixture.write("src/lib.rs", "mod duplicate;\nmod absent;\n");
    fixture.write("src/duplicate.rs", "pub fn from_file() {}\n");
    fixture.write("src/duplicate/mod.rs", "pub fn from_directory() {}\n");
    let ambiguous = fixture.run(&["--format", "json"]);
    assert_eq!(ambiguous.status, 2, "{}", ambiguous.stderr);
    assert!(ambiguous
        .stdout
        .contains("\"analysis_issue\":\"ambiguous_module\""));
    assert!(ambiguous
        .stdout
        .contains("\"analysis_issue\":\"missing_module\""));
    assert!(!ambiguous
        .stdout
        .contains(&fixture.root.to_string_lossy().to_string()));
}

#[test]
fn leaves_module_path_unresolved_when_cfg_attr_path_is_unknown() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nselected = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg_attr(custom_mode, path = \"alternate.rs\")]\nmod direct;\n#[cfg_attr(feature = \"selected\", cfg_attr(nested_mode, path = \"alternate_nested.rs\"))]\nmod nested;\n",
    );
    fixture.write("src/direct.rs", "pub fn direct_default_branch() {}\n");
    fixture.write("src/alternate.rs", "pub fn alternate_branch() {}\n");
    fixture.write("src/nested.rs", "pub fn nested_default_branch() {}\n");
    fixture.write(
        "src/alternate_nested.rs",
        "pub fn nested_alternate_branch() {}\n",
    );

    let output = fixture.run(&["--format", "json", "--features", "selected"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert_eq!(
        output
            .stdout
            .matches("\"analysis_issue\":\"unknown_cfg\"")
            .count(),
        2,
        "{}",
        output.stdout
    );
    assert!(
        !output.stdout.contains("parser_coverage"),
        "{}",
        output.stdout
    );
    assert!(
        !output.stdout.contains("missing_module"),
        "{}",
        output.stdout
    );
    assert!(
        !output.stdout.contains("direct_default_branch"),
        "{}",
        output.stdout
    );
    assert!(
        !output.stdout.contains("nested_default_branch"),
        "{}",
        output.stdout
    );
    assert!(
        !output.stdout.contains("alternate_branch"),
        "{}",
        output.stdout
    );
    assert!(
        !output.stdout.contains("nested_alternate_branch"),
        "{}",
        output.stdout
    );
}

#[test]
fn evaluates_cfg_false_true_and_unknown_branches_without_guessing() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\ndefault = []\nenabled = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg(any())]\nmod disabled;\n#[cfg(feature = \"enabled\")]\nmod feature;\n",
    );
    fixture.write("src/feature.rs", "pub fn visible() {}\n");
    let disabled = fixture.run(&["--format", "json"]);
    let enabled = fixture.run(&["--format", "json", "--features", "enabled"]);
    fixture.write(
        "src/lib.rs",
        "#[cfg(mystery_flag)]\nmod conditional;\n#[cfg(any())]\nmod disabled;\n",
    );
    fixture.write("src/conditional.rs", "fn conditional() {}\n");
    let unknown = fixture.run(&["--format", "json"]);
    assert_eq!(disabled.status, 0, "{}", disabled.stderr);
    assert!(!disabled.stdout.contains("feature.rs"));
    assert_eq!(enabled.status, 1, "{}", enabled.stderr);
    assert!(enabled.has_rule("child_public_free_functions"));
    assert_eq!(unknown.status, 1, "{}", unknown.stdout);
    assert!(unknown
        .stdout
        .contains("\"analysis_issue\":\"unknown_cfg\""));
    assert!(!unknown.stdout.contains("missing_module"));
}

#[test]
fn keeps_false_and_unknown_configured_nodes_queryable_by_ast_id() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = "#[cfg(any())]\nmod disabled {\n    #[cfg(mystery_child)]\n    fn nested() {}\n}\n#[cfg(mystery_function)]\nfn uncertain() {}\n#[cfg(any()) trailing]\nfn malformed() {}\n#[path = \"chosen.rs\"]\nmod chosen;\n#[cfg_attr(mystery_path, path = \"alternate.rs\")]\nmod routed;\nfn local_scope() {\n    #[cfg(any())]\n    fn hidden_local() {}\n}\n";
    fixture.write("src/lib.rs", source);
    fixture.write("src/chosen.rs", "pub fn chosen() {}\n");
    fixture.write("src/alternate.rs", "pub fn alternate() {}\n");

    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo 元数据");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), false, true))
            .with_current_directory(fixture.root.clone()),
    )
    .expect("选择 package 与 target");
    let package = project.packages().first().expect("选中的 package");
    let snapshot =
        ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("catalog messages render");
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate 根模块");
    let parsed = module.parse_output();
    let configured = module.configured_ast();

    assert_eq!(configured.nodes().len(), parsed.syntax_nodes().len());
    for node in parsed.syntax_nodes() {
        assert!(
            configured.node(node.id()).is_some(),
            "AST ID {:?}",
            node.id()
        );
    }

    let node_at = |marker: &str| {
        let offset = source.find(marker).expect("源码标记");
        parsed
            .syntax_nodes()
            .iter()
            .filter(|node| node.start() <= offset && offset < node.end())
            .min_by_key(|node| node.end().saturating_sub(node.start()))
            .expect("对应 item 节点")
    };
    let nested = configured
        .node(node_at("fn nested").id())
        .expect("false 后代节点");
    assert_eq!(nested.availability_decision(), CfgDecision::False);
    let mut parent = nested.parent();
    while let Some(parent_id) = parent {
        let ancestor = configured.node(parent_id).expect("父节点配置记录");
        if ancestor.parent().is_none() {
            break;
        }
        assert_eq!(ancestor.availability_decision(), CfgDecision::False);
        parent = ancestor.parent();
    }

    let uncertain = configured
        .node(node_at("fn uncertain").id())
        .expect("unknown 节点");
    assert_eq!(uncertain.availability_decision(), CfgDecision::Unknown);
    assert!(uncertain
        .availability_condition()
        .is_some_and(|condition| condition.contains("mystery_function")));
    let hidden_local = configured
        .node(node_at("fn hidden_local").id())
        .expect("false local item");
    assert_eq!(hidden_local.availability_decision(), CfgDecision::False);

    let chosen = configured
        .node(node_at("mod chosen").id())
        .expect("带 path 属性的 module 节点");
    assert_eq!(chosen.attribute_evaluation().path(), Some("chosen.rs"));
    let routed = configured
        .node(node_at("mod routed").id())
        .expect("带未知 path 条件的 module 节点");
    assert!(routed.attribute_evaluation().path_unknown());
    assert!(routed.issues().iter().any(|issue| {
        issue.kind() == ConfiguredAstIssueKind::UnknownCfg
            && issue
                .condition()
                .is_some_and(|condition| condition.contains("mystery_path"))
    }));
    let malformed = configured
        .node(node_at("fn malformed").id())
        .expect("malformed 属性节点");
    assert!(malformed.attribute_evaluation().is_malformed());
    assert!(malformed.issues().iter().any(|issue| {
        issue.kind() == ConfiguredAstIssueKind::MalformedAttribute
            && parsed
                .source()
                .slice(issue.start(), issue.end())
                .is_some_and(|range| range == "trailing")
    }));
    assert_eq!(
        configured
            .node_for_range(uncertain.start(), uncertain.end())
            .map(|node| node.ast_node_id()),
        Some(uncertain.ast_node_id())
    );
    assert!(configured
        .enabled_nodes()
        .all(|node| node.availability_decision() == CfgDecision::True));
    assert!(configured.potentially_enabled_nodes().count() > configured.enabled_nodes().count());
}

#[test]
fn preserves_cfg_decisions_across_configured_ast_node_kinds() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = "#[cfg(custom_item)] struct Conditional;\nstruct Record { #[cfg(any())] hidden_field: u8, #[cfg(all())] visible_field: u16 }\nenum Choice { #[cfg(custom_variant)] Conditional, Visible }\nfn generic<#[cfg(any())] HiddenGeneric, #[cfg(custom_generic)] PossibleGeneric>() {}\nfn inspect(#[cfg(any())] hidden_parameter: u8, #[cfg(custom_parameter)] possible_parameter: u8) {\n    #[cfg(any())] let hidden_statement = 0;\n    #[cfg(custom_statement)] let possible_statement = 0;\n    #[cfg(any())] fn hidden_local() {}\n    match 0 { #[cfg(any())] 1 => (), #[cfg(custom_arm)] _ => (), _ => () };\n    let _ = #[cfg(custom_expression)] value.member;\n    let _closure = |#[cfg(any())] hidden_closure: u8, #[cfg(custom_closure)] possible_closure: u8| ();\n    use crate::{#[cfg(any())] missing::Hidden as HiddenUse, #[cfg(custom_use)] existing::Visible as VisibleUse};\n}\n";
    fixture.write("src/lib.rs", source);
    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo 元数据");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), false, true))
            .with_current_directory(fixture.root.clone()),
    )
    .expect("选择 package 与 target");
    let package = project.packages().first().expect("选中的 package");
    let snapshot =
        ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("catalog messages render");
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate 根模块");
    let parsed = module.parse_output();
    let configured = module.configured_ast();
    let cases = [
        ConfiguredNodeExpectation {
            marker: "Conditional;",
            kind: "Item",
            decision: CfgDecision::Unknown,
        },
        ConfiguredNodeExpectation {
            marker: "hidden_field",
            kind: "Field",
            decision: CfgDecision::False,
        },
        ConfiguredNodeExpectation {
            marker: "visible_field",
            kind: "Field",
            decision: CfgDecision::True,
        },
        ConfiguredNodeExpectation {
            marker: "Conditional, Visible",
            kind: "EnumVariant",
            decision: CfgDecision::Unknown,
        },
        ConfiguredNodeExpectation {
            marker: "HiddenGeneric",
            kind: "GenericParameter",
            decision: CfgDecision::False,
        },
        ConfiguredNodeExpectation {
            marker: "PossibleGeneric",
            kind: "GenericParameter",
            decision: CfgDecision::Unknown,
        },
        ConfiguredNodeExpectation {
            marker: "hidden_parameter",
            kind: "Parameter",
            decision: CfgDecision::False,
        },
        ConfiguredNodeExpectation {
            marker: "possible_parameter",
            kind: "Parameter",
            decision: CfgDecision::Unknown,
        },
        ConfiguredNodeExpectation {
            marker: "hidden_statement",
            kind: "Statement",
            decision: CfgDecision::False,
        },
        ConfiguredNodeExpectation {
            marker: "possible_statement",
            kind: "Statement",
            decision: CfgDecision::Unknown,
        },
        ConfiguredNodeExpectation {
            marker: "hidden_local",
            kind: "LocalItem",
            decision: CfgDecision::False,
        },
        ConfiguredNodeExpectation {
            marker: "1 =>",
            kind: "MatchArm",
            decision: CfgDecision::False,
        },
        ConfiguredNodeExpectation {
            marker: "#[cfg(custom_arm)]",
            kind: "MatchArm",
            decision: CfgDecision::Unknown,
        },
        ConfiguredNodeExpectation {
            marker: "value.member",
            kind: "Expression",
            decision: CfgDecision::Unknown,
        },
        ConfiguredNodeExpectation {
            marker: "hidden_closure",
            kind: "Parameter",
            decision: CfgDecision::False,
        },
        ConfiguredNodeExpectation {
            marker: "possible_closure",
            kind: "Parameter",
            decision: CfgDecision::Unknown,
        },
        ConfiguredNodeExpectation {
            marker: "HiddenUse",
            kind: "UseTree",
            decision: CfgDecision::False,
        },
        ConfiguredNodeExpectation {
            marker: "VisibleUse",
            kind: "UseTree",
            decision: CfgDecision::Unknown,
        },
    ];

    for expectation in cases {
        let offset = source.find(expectation.marker).expect("源码标记");
        let node = parsed
            .syntax_nodes()
            .iter()
            .filter(|node| {
                format!("{:?}", node.kind()) == expectation.kind
                    && node.start() <= offset
                    && offset < node.end()
            })
            .min_by_key(|node| node.end().saturating_sub(node.start()))
            .unwrap_or_else(|| {
                let containing = parsed
                    .syntax_nodes()
                    .iter()
                    .filter(|node| node.start() <= offset && offset < node.end())
                    .map(|node| {
                        format!(
                            "{:?}: {}",
                            node.kind(),
                            parsed
                                .source()
                                .slice(node.start(), node.end())
                                .unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>();
                panic!(
                    "找不到 {} 节点: {}; {containing:?}",
                    expectation.kind, expectation.marker
                )
            });
        let configured_node = configured.node(node.id()).expect("配置 AST 节点");
        assert_eq!(
            configured_node.availability_decision(),
            expectation.decision,
            "{}",
            expectation.marker
        );
        if expectation.decision != CfgDecision::True {
            assert!(configured_node.availability_condition().is_some());
        }
    }
}

#[test]
fn parses_comments_inside_recursive_cfg_attr_arguments() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nenabled = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg_attr(feature = \"enabled\", /* comma, ) */ cfg_attr(all(feature = \"enabled\" /* comma, ) */, feature = \"enabled\"), cfg(any()) /* trailing, ) */))]\nfn hidden() {}\nfn active() {}\n",
    );

    let output = fixture.run(&["--format", "json", "--features", "enabled"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(output.stdout.contains("\"status\":\"complete\""));
    assert!(!output.stdout.contains("malformed_cfg_attribute"));
    assert!(!output.stdout.contains("hidden"));
}

#[test]
fn quote_interpolation_hash_does_not_report_an_invalid_attribute() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn lower() { parameter.bounds.push(parse2(quote!(#trait_name))?); }\n",
    );

    let output = fixture.run(&["--format", "json", "--locale", "en-US"]);
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    let diagnostics = report["diagnostics"].as_array().expect("诊断数组");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["analysis_issue"] == "parser_coverage"
            && diagnostic_message(diagnostic)
                .contains("Macro input is opaque and its internal Rust syntax is not analyzed")
    }));
    assert!(
        !diagnostics.iter().any(|diagnostic| {
            diagnostic["analysis_issue"] == "parser_coverage"
                && diagnostic_message(diagnostic)
                    .contains("An opening bracket is required after the attribute marker #")
        }),
        "quote! 插值中的 # 不应被解释为属性标记：{}",
        output.stdout
    );
}

#[test]
fn locates_malformed_stacked_cfg_at_the_attribute_that_failed() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[cfg(mystery_parent)]\n#[cfg(not())]\nfn uncertain() {}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    let malformed = report["diagnostics"]
        .as_array()
        .expect("诊断数组")
        .iter()
        .find(|diagnostic| {
            diagnostic["analysis_issue"] == "parser_coverage"
                && message_contains_either(
                    diagnostic,
                    "Could not reliably interpret a cfg/path attribute",
                    "cfg/path 属性无法可靠解释",
                )
        })
        .expect("malformed 属性诊断");

    assert_eq!(malformed["start"]["line"], 2, "{}", output.stdout);
    assert_eq!(malformed["start"]["column"], 3, "{}", output.stdout);
}

#[test]
fn locates_trailing_cfg_attribute_tokens_at_the_unexpected_range() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[cfg(any()) trailing]\nfn hidden() {}\nfn active() {}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    let malformed = report["diagnostics"]
        .as_array()
        .expect("诊断数组")
        .iter()
        .find(|diagnostic| {
            diagnostic["analysis_issue"] == "parser_coverage"
                && message_contains_either(
                    diagnostic,
                    "Could not reliably interpret a cfg/path attribute",
                    "cfg/path 属性无法可靠解释",
                )
        })
        .expect("malformed 属性诊断");

    assert_eq!(malformed["start"]["line"], 1, "{}", output.stdout);
    assert_eq!(malformed["start"]["column"], 14, "{}", output.stdout);
    assert_eq!(malformed["end"]["column"], 22, "{}", output.stdout);
}

#[test]
fn rejects_value_arguments_in_cfg_metadata_positions() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[cfg(any(256))]\nfn malformed_cfg() {}\n#[cfg_attr(all(), 256)]\nfn malformed_nested_attribute() {}\n#[cfg_attr(256, path = \"ignored.rs\")]\nfn malformed_predicate() {}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    let malformed_lines = report["diagnostics"]
        .as_array()
        .expect("诊断数组")
        .iter()
        .filter(|diagnostic| {
            diagnostic["analysis_issue"] == "parser_coverage"
                && message_contains_either(
                    diagnostic,
                    "Could not reliably interpret a cfg/path attribute",
                    "cfg/path 属性无法可靠解释",
                )
        })
        .map(|diagnostic| diagnostic["start"]["line"].as_u64().expect("诊断行号"))
        .collect::<Vec<_>>();

    assert_eq!(malformed_lines, [1, 3, 5], "{}", output.stdout);
}

#[test]
fn filters_false_trait_and_impl_members_and_reports_unknown_members() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nenabled = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "trait Contract {\n    #[cfg(any())]\n    #[allow(dead_code)]\n    fn disabled(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8);\n    fn active();\n}\n\nstruct Service;\n\n#[cfg(feature = \"enabled\")]\nimpl Service {\n    #[cfg(any())]\n    fn disabled(&self, a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) {}\n    #[cfg(custom_mode)]\n    fn possible(&self, a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) {}\n}\n",
    );

    let output = fixture.run(&["--format", "json", "--features", "enabled"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    let parameter_diagnostics = rule_diagnostics(&output, "parameter_limit");
    assert_eq!(parameter_diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(
        parameter_diagnostics[0]["cfg_condition"],
        "all(custom_mode, feature = \"enabled\")"
    );
    assert!(output.stdout.contains("\"analysis_issue\":\"unknown_cfg\""));
    assert!(!output.has_rule("no_lint_relaxation"));
}

#[test]
fn filters_inactive_fields_and_match_arms_and_keeps_unknown_conditions() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nenabled = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg(any())]\nstruct Disabled { #[cfg(hidden_condition)] pub hidden: u8 }\n#[cfg(any())]\nimpl Disabled {}\n\nstruct Record {\n    #[cfg(any())] pub inactive: u8,\n    #[cfg(feature = \"enabled\")] pub gated: u8,\n    #[cfg(custom_field)] pub uncertain: u8,\n    pub current: u8,\n}\n\nimpl Record {}\n\npub fn variants(value: u8) {\n    match value {\n        #[cfg(any())] 0 => (1, 2, 3),\n        #[cfg(custom_arm)] 1 => (1, 2, 3),\n        _ => (),\n    }\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let private_fields = rule_diagnostics(&output, "private_fields_for_behavior_types");
    let tuple_arity = rule_diagnostics(&output, "tuple_arity");
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    let unknown_cfg = report["diagnostics"]
        .as_array()
        .expect("诊断数组")
        .iter()
        .filter(|diagnostic| diagnostic["analysis_issue"] == "unknown_cfg")
        .count();

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(private_fields.len(), 2, "{}", output.stdout);
    assert_eq!(tuple_arity.len(), 1, "{}", output.stdout);
    assert_eq!(unknown_cfg, 2, "{}", output.stdout);
    assert!(private_fields
        .iter()
        .any(|diagnostic| diagnostic["cfg_condition"] == "custom_field"));
    assert_eq!(tuple_arity[0]["cfg_condition"], "custom_arm");
    assert!(!output.stdout.contains("hidden_condition"));
}

#[test]
fn inherits_unknown_ancestor_conditions_and_prunes_false_field_descendants() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[cfg(custom_parent)]\nstruct Record {\n    #[cfg(any())] pub inactive: u8,\n    #[cfg(custom_child)] pub conditional: u16,\n    pub inherited: u32,\n}\nimpl Record {}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let fields = rule_diagnostics(&output, "private_fields_for_behavior_types");
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    let unknown_cfg = report["diagnostics"]
        .as_array()
        .expect("诊断数组")
        .iter()
        .filter(|diagnostic| diagnostic["analysis_issue"] == "unknown_cfg")
        .count();

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(fields.len(), 2, "{}", output.stdout);
    assert!(fields
        .iter()
        .any(|diagnostic| diagnostic["cfg_condition"] == "custom_parent"));
    assert!(fields.iter().any(
        |diagnostic| diagnostic["cfg_condition"]
            .as_str()
            .is_some_and(|condition| {
                condition.contains("custom_child") && condition.contains("custom_parent")
            })
    ));
    assert_eq!(unknown_cfg, 2, "{}", output.stdout);
    assert!(!output.stdout.contains("inactive"));
    assert!(output.stdout.contains("\"status\":\"partial\""));
}

#[test]
fn omits_false_use_tree_branches_from_import_rules_and_symbol_aliases() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod parent { pub struct Secret; }\nuse crate::{#[cfg(any())] parent::Secret as Later};\n\nfn use_alias() { let _ = Later; }\n\nstruct Later;\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert!(output.has_rule("declaration_before_use"));
    assert!(!output.has_rule("cfg_inside_use_tree"));
}

#[test]
fn omits_false_nested_block_use_branches_from_local_symbols() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod parent { pub struct Secret; }\nfn use_alias() {\n    use crate::{#[cfg(any())] parent::Secret as Later};\n    let _ = Later;\n}\n\nstruct Later;\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert!(
        output.has_rule("declaration_before_use"),
        "{}",
        output.stdout
    );
    assert!(!output.has_rule("cfg_inside_use_tree"), "{}", output.stdout);
}

#[test]
fn resolves_aliases_from_grouped_block_use_leaves() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = "mod diagnostics { pub enum CodecStage { Conversion } }\nfn imported() {\n    use crate::diagnostics::{CodecStage as Stage};\n    let _ = Stage;\n}\n";
    fixture.write("src/lib.rs", source);
    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo 元数据");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), false, true))
            .with_current_directory(fixture.root.clone()),
    )
    .expect("选择 package 与 target");
    let package = project.packages().first().expect("选中的 package");
    let snapshot =
        ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("catalog messages render");
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate 根模块");
    let offset = source.find("let _ = Stage").expect("别名引用位置") + "let _ = ".len();
    let resolution = snapshot.symbols().resolve_path_at(
        module,
        &["Stage".to_owned()],
        Some(offset..offset + "Stage".len()),
    );

    assert_eq!(resolution.kind(), PathResolutionKind::Local);
    assert_eq!(resolution.alias_chain(), ["Stage"]);
    assert_eq!(
        resolution.symbol().map(|symbol| symbol.name()),
        Some("CodecStage")
    );
}

#[test]
fn keeps_module_analysis_and_diagnostics_on_the_same_unknown_cfg_condition() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[cfg(custom_gate)]\n#[allow(dead_code)]\nmod child;\n",
    );
    fixture.write("src/child.rs", "pub struct Item;\n\npub fn exposed() {}\n");
    let output = fixture.run(&["--format", "json"]);
    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo 元数据");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), false, true))
            .with_current_directory(fixture.root.clone()),
    )
    .expect("选择 package 与 target");
    let package = project.packages().first().expect("选中的 package");
    let snapshot =
        ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("catalog messages render");
    let root = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate 根模块");
    let child = snapshot
        .modules()
        .iter()
        .find(|module| module.namespace() == ["child"])
        .expect("cfg 条件下发现的子模块");
    let symbol = snapshot
        .symbols()
        .symbols()
        .iter()
        .find(|symbol| symbol.name() == "Item")
        .expect("子模块符号");

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(child.cfg_condition(), Some("custom_gate"));
    assert!(root.lint_relaxations().iter().any(|attribute| {
        attribute.decision() == CfgDecision::Unknown && attribute.condition() == Some("custom_gate")
    }));
    assert_eq!(symbol.cfg_condition(), Some("custom_gate"));
    let diagnostics = rule_diagnostics(&output, "child_public_free_functions");
    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["cfg_condition"], "custom_gate");
    let lint_diagnostics = rule_diagnostics(&output, "no_lint_relaxation");
    assert_eq!(lint_diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(lint_diagnostics[0]["cfg_condition"], "custom_gate");
}

#[test]
fn analyzes_supported_items_around_function_body_opaque_syntax() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "mod child;\n");
    fixture.write(
        "src/child.rs",
        "pub fn opaque_body() { hidden!(); }\npub fn available() {}\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert!(output.stdout.contains("\"status\":\"partial\""));
    assert!(output
        .stdout
        .contains("\"analysis_issue\":\"parser_coverage\""));
    assert_eq!(
        rule_diagnostics(&output, "child_public_free_functions").len(),
        2,
        "{}",
        output.stdout
    );
}

#[test]
fn skips_item_spacing_between_provably_exclusive_cfg_nodes() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[cfg(custom)]\nstruct First;\n#[cfg(not(custom))]\nstruct Second;\n",
    );

    let exclusive = fixture.run(&["--format", "json"]);
    assert!(!exclusive.has_rule("item_spacing"), "{}", exclusive.stdout);

    fixture.write(
        "src/lib.rs",
        "#[cfg(custom)]\nstruct First;\n#[cfg(custom)]\nstruct Second;\n",
    );
    let compatible = fixture.run(&["--format", "json"]);
    assert!(compatible.has_rule("item_spacing"), "{}", compatible.stdout);
}

#[test]
fn evaluates_cfg_attr_cfg_injections_as_implications() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nouter = []\ninner = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg_attr(feature = \"outer\", cfg(feature = \"inner\"))]\nfn gated(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) {}\n",
    );

    let outer_disabled = fixture.run(&["--format", "json"]);
    let injected_false = fixture.run(&["--format", "json", "--features", "outer"]);
    let injected_true = fixture.run(&["--format", "json", "--features", "outer,inner"]);

    assert!(
        outer_disabled.has_rule("parameter_limit"),
        "{}",
        outer_disabled.stdout
    );
    assert!(
        !injected_false.has_rule("parameter_limit"),
        "{}",
        injected_false.stdout
    );
    assert!(
        injected_true.has_rule("parameter_limit"),
        "{}",
        injected_true.stdout
    );

    fixture.write(
        "src/lib.rs",
        "#[cfg_attr(custom_gate, cfg(any()))]\nfn possible(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) {}\n",
    );
    let unknown = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&unknown, "parameter_limit");
    assert_eq!(diagnostics.len(), 1, "{}", unknown.stdout);
    assert_eq!(diagnostics[0]["cfg_condition"], "not(custom_gate)");
    assert!(unknown
        .stdout
        .contains("\"analysis_issue\":\"unknown_cfg\""));
}

#[test]
fn conditionally_active_local_bindings_only_shadow_coexisting_references() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn use_false_binding() {\n    #[cfg(any())] let Later = 0;\n    let _ = Later;\n}\n\nfn use_exclusive_binding() {\n    #[cfg(custom_mode)] let Other = 0;\n    #[cfg(not(custom_mode))] let _ = Other;\n}\n\nstruct Later;\nstruct Other;\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "declaration_before_use");

    assert_eq!(diagnostics.len(), 2, "{}", output.stdout);
    assert!(diagnostics.iter().any(|diagnostic| diagnostic["message"]
        .as_str()
        .is_some_and(|value| value.contains("Later"))));
    assert!(diagnostics.iter().any(|diagnostic| diagnostic["message"]
        .as_str()
        .is_some_and(|value| value.contains("Other"))));
}

#[test]
fn conditionally_active_local_items_shadow_only_when_enabled() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn false_local_item() {\n    #[cfg(any())] struct Later;\n    let _ = Later;\n}\n\nfn active_local_item() {\n    struct Later;\n    let _ = Later;\n}\n\nstruct Later;\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "declaration_before_use");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert!(diagnostics[0]["message"]
        .as_str()
        .is_some_and(|message| message.contains("Later")));
}

#[test]
fn evaluates_hyphenated_features_and_cfg_boolean_identities() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nfast-path = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg(all(feature = \"fast-path\", not(any())))]\nmod all_branch;\n#[cfg(any(feature = \"fast-path\", target_os = \"definitely-not-a-real-os\"))]\nmod any_branch;\n",
    );
    fixture.write("src/all_branch.rs", "pub fn all_enabled() {}\n");
    fixture.write("src/any_branch.rs", "pub fn any_enabled() {}\n");

    let disabled = fixture.run(&["--format", "json"]);
    let enabled = fixture.run(&["--format", "json", "--features", "fast-path"]);

    assert_eq!(disabled.status, 0, "{}", disabled.stdout);
    assert!(!disabled.stdout.contains("src/all_branch.rs"));
    assert_eq!(enabled.status, 1, "{}", enabled.stdout);
    assert_eq!(
        enabled
            .stdout
            .matches("\"rule_id\":\"child_public_free_functions\"")
            .count(),
        2,
        "{}",
        enabled.stdout
    );
}

#[test]
fn decodes_escaped_cfg_feature_names() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nfeature-name = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg(feature = \"feature\\x2dname\")]\nmod escaped;\n#[cfg(feature = \"feature\\u{2d}name\")]\nmod unicode_escaped;\n",
    );
    fixture.write("src/escaped.rs", "pub fn escaped_feature() {}\n");
    fixture.write(
        "src/unicode_escaped.rs",
        "pub fn unicode_escaped_feature() {}\n",
    );

    let enabled = fixture.run(&["--format", "json", "--features", "feature-name"]);

    assert_eq!(enabled.status, 1, "{}", enabled.stdout);
    assert!(enabled.stdout.contains("src/escaped.rs"));
    assert!(enabled.stdout.contains("src/unicode_escaped.rs"));
    assert!(!enabled
        .stdout
        .contains("\"analysis_issue\":\"unknown_cfg\""));
}

#[test]
fn preserves_exact_target_triple_dependency_matching() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"vendor\"]\nresolver = \"2\"\n\n[target.x86_64-unknown-linux-gnu.dependencies]\nvendor = { path = \"vendor\" }\n",
    );
    fixture.write("src/lib.rs", "fn direct() { let _ = vendor::Thing; }\n");
    fixture.write(
        "vendor/Cargo.toml",
        "[package]\nname = \"vendor\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    fixture.write("vendor/src/lib.rs", "pub struct Thing;\n");

    let matching = fixture.run(&[
        "--format",
        "json",
        "--package",
        "app",
        "--target",
        "x86_64-unknown-linux-gnu",
    ]);
    let other = fixture.run(&[
        "--format",
        "json",
        "--package",
        "app",
        "--target",
        "aarch64-unknown-linux-gnu",
    ]);
    let combined = fixture.run(&[
        "--format",
        "json",
        "--package",
        "app",
        "--target",
        "x86_64-unknown-linux-gnu",
        "--target",
        "aarch64-unknown-linux-gnu",
    ]);

    assert_eq!(matching.status, 1, "{}", matching.stdout);
    assert!(matching.has_rule("no_inline_qualified_path"));
    assert_eq!(other.status, 0, "{}", other.stdout);
    assert!(!other.has_rule("no_inline_qualified_path"));
    let diagnostics = rule_diagnostics(&combined, "no_inline_qualified_path");
    assert_eq!(combined.status, 1, "{}", combined.stdout);
    assert_eq!(diagnostics.len(), 1, "{}", combined.stdout);
    assert_eq!(diagnostics[0]["target_triple"], "x86_64-unknown-linux-gnu");
}

#[test]
fn scans_multiple_target_triples_with_separate_cfg_and_diagnostics() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "pub struct Marker;\n#[cfg(windows)]\nmod windows;\n#[cfg(unix)]\nmod unix;\nfn common() { let _ = crate::Marker; }\n",
    );
    fixture.write(
        "src/windows.rs",
        "fn windows_only() { let _ = crate::Marker; }\n",
    );
    fixture.write("src/unix.rs", "fn unix_only() { let _ = crate::Marker; }\n");

    let output = fixture.run(&[
        "--format",
        "json",
        "--target",
        "x86_64-pc-windows-msvc",
        "--target",
        "x86_64-pc-windows-msvc",
        "--target",
        "x86_64-unknown-linux-gnu",
    ]);
    let diagnostics = rule_diagnostics(&output, "no_inline_qualified_path");
    let common = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["path"] == "src/lib.rs")
        .collect::<Vec<_>>();
    let windows = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["path"] == "src/windows.rs")
        .collect::<Vec<_>>();
    let unix = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["path"] == "src/unix.rs")
        .collect::<Vec<_>>();

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(common.len(), 2, "{}", output.stdout);
    assert_eq!(windows.len(), 1, "{}", output.stdout);
    assert_eq!(unix.len(), 1, "{}", output.stdout);
    assert_eq!(
        windows[0]["target_triple"], "x86_64-pc-windows-msvc",
        "{}",
        output.stdout
    );
    assert_eq!(
        unix[0]["target_triple"], "x86_64-unknown-linux-gnu",
        "{}",
        output.stdout
    );
    assert!(common
        .iter()
        .any(|diagnostic| { diagnostic["target_triple"] == "x86_64-pc-windows-msvc" }));
    assert!(common
        .iter()
        .any(|diagnostic| { diagnostic["target_triple"] == "x86_64-unknown-linux-gnu" }));
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic["target"] == "sample" && diagnostic["target_kind"][0] == "lib"
    }));
}

#[test]
fn analyzes_host_only_targets_once_across_a_target_matrix() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\nbuild = \"build.rs\"\n\n[lib]\nproc-macro = true\n\n[[bin]]\nname = \"sample-bin\"\npath = \"src/main.rs\"\n",
    );
    fixture.write("build.rs", "fn main() {}\n");
    fixture.write("src/lib.rs", "extern crate proc_macro;\n");
    fixture.write("src/main.rs", "fn main() {}\n");

    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo 元数据");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), false, true))
            .with_all_targets(true)
            .with_target_triples(Some(vec![
                "x86_64-pc-windows-msvc".to_owned(),
                "x86_64-unknown-linux-gnu".to_owned(),
            ]))
            .with_current_directory(fixture.root.clone()),
    )
    .expect("选择多 triple 上下文");
    let targets = project.packages()[0].targets();
    let host_only_count = targets
        .iter()
        .filter(|target| {
            target
                .kind()
                .iter()
                .any(|kind| matches!(kind.as_str(), "proc-macro" | "custom-build"))
        })
        .count();
    let build_script_count = targets
        .iter()
        .filter(|target| target.kind().iter().any(|kind| kind == "custom-build"))
        .count();
    let proc_macros = targets
        .iter()
        .filter(|target| target.kind().iter().any(|kind| kind == "proc-macro"))
        .collect::<Vec<_>>();
    let binary_triples = targets
        .iter()
        .filter(|target| target.kind().iter().any(|kind| kind == "bin"))
        .map(|target| target.target_triple())
        .collect::<BTreeSet<_>>();

    assert_eq!(host_only_count, 3, "{targets:#?}");
    assert_eq!(build_script_count, 1, "{targets:#?}");
    assert_eq!(proc_macros.len(), 2, "{targets:#?}");
    assert_ne!(proc_macros[0].cfg_test(), proc_macros[1].cfg_test());
    assert_eq!(
        proc_macros[0].target_triple(),
        proc_macros[1].target_triple()
    );
    assert_eq!(
        binary_triples,
        BTreeSet::from(["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu",])
    );
}

#[test]
fn evaluates_cfg_target_dependencies_for_the_selected_target() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"vendor\"]\nresolver = \"2\"\n\n[target.'cfg(unix)'.dependencies]\nvendor = { path = \"vendor\" }\n",
    );
    fixture.write("src/lib.rs", "fn direct() { let _ = vendor::Thing; }\n");
    fixture.write(
        "vendor/Cargo.toml",
        "[package]\nname = \"vendor\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    fixture.write("vendor/src/lib.rs", "pub struct Thing;\n");

    let output = fixture.run(&["--format", "json", "--target", "host"]);

    assert_eq!(
        output.status,
        if cfg!(unix) { 1 } else { 0 },
        "{}",
        output.stdout
    );
    assert_eq!(
        output.has_rule("no_inline_qualified_path"),
        cfg!(unix),
        "{}",
        output.stdout
    );
}

#[test]
fn propagates_weak_dependency_features_only_after_activation() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"dep\"]\nresolver = \"2\"\n\n[features]\nactivate = [\"dep:dep\"]\nforward = [\"dep?/extra\"]\n\n[dependencies]\ndep = { path = \"dep\", optional = true, default-features = false }\n",
    );
    fixture.write("src/lib.rs", "pub fn app_ready() {}\n");
    fixture.write(
        "dep/Cargo.toml",
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\ndefault = []\nextra = []\n",
    );
    fixture.write(
        "dep/src/lib.rs",
        "#[cfg(feature = \"extra\")]\nmod extra;\n",
    );
    fixture.write("dep/src/extra.rs", "pub fn enabled_feature() {}\n");

    let weak_only = fixture.run(&["--format", "json", "--features", "app/forward"]);
    let activated = fixture.run(&["--format", "json", "--features", "app/activate,app/forward"]);

    assert_eq!(weak_only.status, 0, "{}", weak_only.stdout);
    assert!(!weak_only.stdout.contains("dep/src/extra.rs"));
    assert_eq!(activated.status, 1, "{}", activated.stdout);
    assert!(activated.stdout.contains("dep/src/extra.rs"));
    assert!(activated.has_rule("child_public_free_functions"));
}

#[test]
fn uses_only_the_requested_manifest_and_rejects_selection_conflicts() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "pub fn ready() {}\n");
    fs::create_dir_all(fixture.root.join("nested")).expect("创建无 manifest 的子目录");

    let missing = Command::new(env!("CARGO_BIN_EXE_cargo-cop"))
        .arg("--format")
        .arg("json")
        .current_dir(fixture.root.join("nested"))
        .output()
        .expect("在子目录运行 cargo-cop");
    let missing = RunOutput::from(missing);
    let conflict = fixture.run(&["--format", "json", "--workspace", "--package", "sample"]);

    assert_eq!(missing.status, 2, "{}", missing.stdout);
    assert!(missing.stdout.contains("\"tool_error\":\"manifest\""));
    assert_eq!(conflict.status, 2, "{}", conflict.stdout);
    assert!(conflict.stdout.contains("\"tool_error\":\"arguments\""));
}

#[test]
fn does_not_apply_module_layout_to_a_custom_crate_root() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/mod.rs\"\n",
    );
    fixture.write("src/mod.rs", "mod child;\n");
    fixture.write("src/child/mod.rs", "fn private_child() {}\n");

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"module_file_layout\"")
            .count(),
        1,
        "{}",
        output.stdout
    );
    assert!(output.stdout.contains("src/child/mod.rs"));
}

#[test]
fn stops_recursive_module_file_cycles_without_losing_sibling_contexts() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[path = \"a.rs\"] mod first;\n#[path = \"a.rs\"] mod second;\n",
    );
    fixture.write("src/a.rs", "#[path = \"../src/a.rs\"] mod nested;\n");

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 2, "{}", output.stdout);
    assert_eq!(
        output
            .stdout
            .matches("\"analysis_issue\":\"module_cycle\"")
            .count(),
        2,
        "{}",
        output.stdout
    );
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"no_parent_path_attribute\"")
            .count(),
        2,
        "{}",
        output.stdout
    );
    assert!(output.stdout.contains("crate::first::nested"));
    assert!(output.stdout.contains("crate::second::nested"));
    assert!(!output
        .stdout
        .contains(&fixture.root.to_string_lossy().to_string()));
}

#[test]
fn rejects_parent_traversal_in_external_module_path_attributes() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        r##"#[path = "../x.rs"] mod direct;
#[path = "parts/../../x.rs"] mod nested_parent;
#[path = "..\\x.rs"] mod windows_parent;
#[path = "\x2e\x2e/x.rs"] mod hex_parent;
#[path = "\u{2e}\u{2e}/x.rs"] mod unicode_parent;
#[path = r"../x.rs"] mod raw_parent;
#[path = "../\
    x.rs"] mod continued_parent;
#[cfg_attr(any(), path = "../x.rs")] mod conditional;
#[cfg_attr(all(), cfg_attr(any(), path = "../x.rs"))] mod nested_conditional;
#[path = "child/file.rs"] mod allowed_child;
#[path = "./x.rs"] mod allowed_dot;
#[path = "foo..rs"] mod allowed_name;
#[path = ".../x.rs"] mod allowed_ellipsis;
#[path = "../x.rs"] mod inline { fn item() {} }
"##,
    );
    fixture.write("x.rs", "");
    fixture.write("src/..\\x.rs", "");
    fixture.write("src/parts/marker.rs", "");
    fixture.write("src/conditional.rs", "");
    fixture.write("src/nested_conditional.rs", "");
    fixture.write("src/child/file.rs", "");
    fixture.write("src/x.rs", "");
    fixture.write("src/foo..rs", "");
    fixture.write("src/.../x.rs", "");

    let default = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "child_module_line_count",
    ]);
    let warning = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "child_module_line_count",
        "--severity",
        "no_parent_path_attribute=warning",
    ]);
    let disabled = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "child_module_line_count",
        "--disable-rule",
        "no_parent_path_attribute",
    ]);
    let chinese = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "child_module_line_count",
        "--locale",
        "zh-CN",
    ]);
    let default_report = from_str::<Value>(&default.stdout).expect("默认 JSON 报告");
    let chinese_report = from_str::<Value>(&chinese.stdout).expect("中文 JSON 报告");
    let diagnostics = rule_diagnostics(&default, "no_parent_path_attribute");
    let warning_diagnostics = rule_diagnostics(&warning, "no_parent_path_attribute");
    let chinese_diagnostics = rule_diagnostics(&chinese, "no_parent_path_attribute");
    let expected_lines = [1, 2, 3, 4, 5, 6, 7, 9, 10];

    assert_eq!(default.status, 1, "{}", default.stdout);
    assert_eq!(diagnostics.len(), 9, "{}", default.stdout);
    assert_eq!(default_report["summary"]["errors"], 9);
    assert!(diagnostics
        .iter()
        .enumerate()
        .all(|(index, diagnostic)| diagnostic["start"]["line"] == expected_lines[index]));
    assert_eq!(warning.status, 0, "{}", warning.stdout);
    assert_eq!(warning_diagnostics.len(), 9);
    assert!(warning_diagnostics
        .iter()
        .all(|diagnostic| diagnostic["severity"] == "warning"));
    assert_eq!(disabled.status, 0, "{}", disabled.stdout);
    assert!(!disabled.has_rule("no_parent_path_attribute"));
    assert_eq!(chinese.status, default.status);
    assert_eq!(chinese_report["locale"], "zh-CN");
    assert_eq!(chinese_diagnostics.len(), 9);
    assert_eq!(
        chinese_diagnostics[0]["rule_id"],
        "no_parent_path_attribute"
    );
    assert!(chinese_diagnostics[0]["message"]
        .as_str()
        .is_some_and(|message| message.contains("访问父目录")));
}

#[test]
fn reports_parser_coverage_failures_as_incomplete_scans() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "fn broken() {\n");

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 2, "{}", output.stderr);
    assert!(output.stdout.contains("\"status\":\"incomplete\""));
    assert!(output
        .stdout
        .contains("\"analysis_issue\":\"parser_coverage\""));
}

#[test]
fn continues_rules_in_recovered_regions_after_a_local_parser_issue() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "mod child;\n");
    fixture.write("src/child.rs", "fn () {}\npub fn exposed() {}\n");

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 2, "{}", output.stderr);
    assert!(
        output.has_rule("child_public_free_functions"),
        "{}",
        output.stdout
    );
    assert!(output
        .stdout
        .contains("\"analysis_issue\":\"parser_coverage\""));
}

#[test]
fn reports_invalid_radix_literals_as_incomplete_scans() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "const INVALID: u8 = 0x;\n");

    let invalid = fixture.run(&["--format", "json", "--locale", "zh-CN"]);

    assert_eq!(invalid.status, 2, "{}", invalid.stdout);
    assert!(invalid.stdout.contains("\"status\":\"incomplete\""));
    assert!(invalid
        .stdout
        .contains("\"analysis_issue\":\"parser_coverage\""));
    assert!(
        has_diagnostic_message_fragment(&invalid, "基数前缀后需要至少一个数字"),
        "{}",
        invalid.stdout
    );

    fixture.write(
        "src/lib.rs",
        "const HEX: u16 = 0x_FF;\nconst OCTAL: u16 = 0o755;\nconst BINARY: u16 = 0b_1010;\n",
    );

    let valid = fixture.run(&["--format", "json"]);

    assert_eq!(valid.status, 0, "{}", valid.stdout);
    assert!(valid.stdout.contains("\"status\":\"complete\""));
}

#[test]
fn resolves_qualified_paths_with_imports_and_local_shadowing() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use std::mem;\nfn imported() { mem::drop(1); }\nfn direct() { std::mem::drop(1); }\n",
    );
    let output = fixture.run(&["--format", "json"]);
    assert_eq!(output.status, 1, "{}", output.stderr);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"no_inline_qualified_path\"")
            .count(),
        1
    );
    fixture.write(
        "src/lib.rs",
        "mod std { pub struct Marker; }\nfn shadowed() { let _ = std::Marker; }\n",
    );
    let shadowed = fixture.run(&["--format", "json"]);
    assert_eq!(shadowed.status, 0, "{}", shadowed.stderr);
    assert!(!shadowed.has_rule("no_inline_qualified_path"));
}

#[test]
fn reports_current_crate_qualified_paths_and_accepts_imported_names() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = "use crate::diagnostics::CodecStage;\nmod diagnostics { pub enum CodecStage { Conversion } }\nfn read_sequence() { let _ = crate::diagnostics::CodecStage::Conversion; }\n\nfn imported_sequence() { let _ = CodecStage::Conversion; }\n";
    fixture.write("src/lib.rs", source);

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    let diagnostics = rule_diagnostics(&output, "no_inline_qualified_path");
    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    let diagnostic = &diagnostics[0];
    let line = usize::try_from(diagnostic["start"]["line"].as_u64().expect("诊断起始行"))
        .expect("诊断起始行范围");
    let start_column = usize::try_from(diagnostic["start"]["column"].as_u64().expect("诊断起始列"))
        .expect("诊断起始列范围");
    let end_column = usize::try_from(diagnostic["end"]["column"].as_u64().expect("诊断结束列"))
        .expect("诊断结束列范围");
    let source_line = source.lines().nth(line - 1).expect("诊断对应源码行");
    assert_eq!(
        &source_line[start_column - 1..end_column - 1],
        "crate::diagnostics::CodecStage::Conversion"
    );

    fixture.write(
        "src/lib.rs",
        "use crate::diagnostics::CodecStage;\nmod diagnostics { pub enum CodecStage { Conversion } }\nfn read_sequence() { let _ = CodecStage::Conversion; }\n",
    );
    let imported = fixture.run(&["--format", "json"]);

    assert_eq!(imported.status, 0, "{}", imported.stdout);
    assert!(!imported.has_rule("no_inline_qualified_path"));
}

#[test]
fn reports_external_qualified_paths_in_nested_call_arguments() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = "fn build() {\n    consume(\n        wrapper(\n            std::path::PathBuf::from(\n                std::path::Path::new(\"input.txt\"),\n            ),\n        ),\n        std::path::Path::new(\"fallback.txt\"),\n    );\n}\n";
    fixture.write("src/lib.rs", source);

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert!(output.stdout.contains("\"status\":\"complete\""));
    let diagnostics = rule_diagnostics(&output, "no_inline_qualified_path");
    assert_eq!(diagnostics.len(), 3, "{}", output.stdout);
    for (diagnostic, expected) in diagnostics.iter().zip([
        ExpectedPathDiagnostic {
            line: 4,
            start_column: 13,
            end_column: 37,
            expected_path: "std::path::PathBuf::from",
        },
        ExpectedPathDiagnostic {
            line: 5,
            start_column: 17,
            end_column: 37,
            expected_path: "std::path::Path::new",
        },
        ExpectedPathDiagnostic {
            line: 8,
            start_column: 9,
            end_column: 29,
            expected_path: "std::path::Path::new",
        },
    ]) {
        assert_eq!(diagnostic["start"]["line"], expected.line);
        assert_eq!(diagnostic["start"]["column"], expected.start_column);
        assert_eq!(diagnostic["end"]["line"], expected.line);
        assert_eq!(diagnostic["end"]["column"], expected.end_column);
        let source_line = source
            .lines()
            .nth(expected.line - 1)
            .expect("诊断对应源码行");
        assert_eq!(
            &source_line[expected.start_column - 1..expected.end_column - 1],
            expected.expected_path
        );
    }
}

#[test]
fn reports_self_crate_qualified_paths_in_all_test_targets() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("typol-core"));
    fixture.write("src/lib.rs", "pub struct Marker;\n");
    fixture.write(
        "tests/qualified_path.rs",
        "fn takes_self_crate_type(_: typol_core::Marker) {}\n",
    );

    let output = fixture.run(&["--all-targets", "--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "no_inline_qualified_path");

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["target"], "qualified_path");
    assert_eq!(diagnostics[0]["target_kind"][0], "test");
    assert_eq!(diagnostics[0]["path"], "tests/qualified_path.rs");
    assert!(diagnostic_message(&diagnostics[0]).contains("typol_core::Marker"));

    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo 元数据");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), true, true))
            .with_all_targets(true)
            .with_current_directory(fixture.root.clone()),
    )
    .expect("选择 package 与 target");
    let package = project.packages().first().expect("选中的 package");
    let snapshot = ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("分析 target");
    let reference = snapshot
        .facts()
        .references()
        .iter()
        .find(|reference| {
            reference.kind() != ReferenceKind::Use
                && reference
                    .segments()
                    .iter()
                    .map(String::as_str)
                    .eq(["typol_core", "Marker"])
        })
        .expect("自身库的限定路径");
    assert_eq!(
        reference.resolution().kind(),
        PathResolutionKind::ExternalCrate
    );
    assert_eq!(
        reference.resolution().root().kind(),
        PathRootKind::ExternPreludeCrate
    );
    assert_eq!(
        reference.resolution().root().external_crate(),
        Some("typol_core")
    );
}

#[test]
fn reports_self_crate_paths_inside_nested_assert_matches_macros() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("dev-env-provider"));
    fixture.write(
        "src/lib.rs",
        "pub enum ProtocolError { UnsupportedProtocol { found: u32, expected: u32 } }\n\npub enum ProtocolCodecError { Protocol { source: ProtocolError } }\n\npub fn decode_request(_: &[u8]) -> Result<(), ProtocolCodecError> { Ok(()) }\n",
    );
    let source = "fn control(_: dev_env_provider::ProtocolCodecError) {}\n\nfn rejects_a_wrong_version_as_a_typed_protocol_error() {\n    let bytes = [];\n    assert!(matches!(\n        dev_env_provider::decode_request(&bytes),\n        Err(dev_env_provider::ProtocolCodecError::Protocol {\n            source: dev_env_provider::ProtocolError::UnsupportedProtocol {\n                found: 2,\n                expected: 1,\n            },\n        })\n    ));\n}\n";
    fixture.write("tests/protocol_decoder.rs", source);

    let output = fixture.run(&["--all-targets", "--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "no_inline_qualified_path");

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(diagnostics.len(), 4, "{}", output.stdout);
    let mut excerpts = diagnostics
        .iter()
        .map(|diagnostic| {
            assert_eq!(diagnostic["path"], "tests/protocol_decoder.rs");
            assert_eq!(diagnostic["target"], "protocol_decoder");
            assert_eq!(diagnostic["target_kind"], json!(["test"]));
            diagnostic_excerpt(source, diagnostic).to_owned()
        })
        .collect::<Vec<_>>();
    let mut expected = [
        "dev_env_provider::ProtocolCodecError",
        "dev_env_provider::decode_request",
        "dev_env_provider::ProtocolCodecError::Protocol",
        "dev_env_provider::ProtocolError::UnsupportedProtocol",
    ]
    .map(str::to_owned);
    excerpts.sort();
    expected.sort();
    assert_eq!(excerpts, expected, "{}", output.stdout);
}

#[test]
fn resolves_imported_and_shadowed_paths_inside_known_macro_profiles() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use std::mem as memory;\n\nmod core { pub enum Marker { Variant } }\n\npub fn check() {\n    assert_eq!(memory::size_of::<u8>(), 4);\n    assert!(matches!(core::Marker::Variant, core::Marker::Variant));\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo 元数据");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), true, true))
            .with_current_directory(fixture.root.clone()),
    )
    .expect("选择 package 与 target");
    let package = project.packages().first().expect("选中的 package");
    let snapshot = ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("分析 target");
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate 根模块");
    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(!output.has_rule("no_inline_qualified_path"));
    assert_eq!(
        snapshot
            .facts()
            .references_for_module(module)
            .filter(|reference| reference
                .segments()
                .iter()
                .map(String::as_str)
                .eq(["memory", "size_of"]))
            .count(),
        1
    );
    assert_eq!(
        snapshot
            .facts()
            .references_for_module(module)
            .filter(|reference| reference
                .segments()
                .iter()
                .map(String::as_str)
                .eq(["core", "Marker", "Variant"]))
            .count(),
        2
    );
}

#[test]
fn analysis_binds_only_resolved_standard_macro_profiles() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = r#"
assert!(true);

use std::assert_eq as compare;

fn matches(_: bool) {}

pub fn inspect(value: bool) {
    compare!(value, true);
    matches!(value, true);
    custom::matches!(value, true);
    let _ = json!({"key": value});
    let _ = error!("value {}", value);
    crate::exported::imported_match!(value, true);
    wrapper::check!(value, true);
}

mod local_macro {
    macro_rules! matches { ($value:expr, $pattern:pat) => {}; }

    pub fn inspect(value: bool) {
        matches!(value, true);
        std::matches!(value, true);
    }
}

#[macro_use]
mod imported_macros {
    macro_rules! format { ($($tokens:tt)*) => {}; }
}

mod exported_macros {
    #[macro_export]
    macro_rules! exported_matches { ($value:expr, $pattern:pat) => {}; }
}

pub fn inspect_imported_and_exported(value: bool) {
    format!("{}", value);
    crate::exported_matches!(value, true);
}

pub mod exported {
    pub use std::matches as imported_match;
}

pub mod wrapper {
    pub use crate::exported::imported_match as check;
}

mod glob_blocked {
    use unknown_macros::*;

    pub fn inspect() {
        let _ = format!("value");
    }
}
"#;
    fixture.write("src/lib.rs", source);
    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo metadata");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), true, true))
            .with_current_directory(fixture.root.clone()),
    )
    .expect("select package and target");
    let package = project.packages().first().expect("selected package");
    let snapshot = ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("analyze");
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate root module");
    let statuses = module
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .map(|invocation| {
            let text = module
                .parse_output()
                .source()
                .slice(invocation.start(), invocation.end())
                .expect("macro call source range");
            (text, invocation.input().status().clone())
        })
        .collect::<Vec<_>>();

    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("assert!(true)")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Assert)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("compare!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::AssertEq)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("matches!(value")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Matches)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("custom::matches!") && status == &MacroInputStatus::Opaque
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("json!") && status == &MacroInputStatus::Opaque
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("error!") && status == &MacroInputStatus::Opaque
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("matches!(value") && status == &MacroInputStatus::Opaque
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("std::matches!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Matches)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("crate::exported::imported_match!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Matches)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("wrapper::check!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Matches)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("format!(\"value\")") && status == &MacroInputStatus::Opaque
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("format!(\"{}\"") && status == &MacroInputStatus::Opaque
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("crate::exported_matches!") && status == &MacroInputStatus::Opaque
    }));
}

#[test]
fn external_modules_inherit_macro_use_context_from_their_parent_modules() {
    for import_kind in ["extern", "module"] {
        let fixture = Fixture::new();
        if import_kind == "extern" {
            fixture.write(
                "Cargo.toml",
                &format!(
                    "{}\n[dependencies]\nmacro_owner = {{ path = \"macro_owner\" }}\n",
                    package_manifest("inherited_macro_use_sample")
                ),
            );
            fixture.write(
                "macro_owner/Cargo.toml",
                "[package]\nname = \"macro_owner\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            );
            fixture.write(
                "macro_owner/src/lib.rs",
                "#[macro_export]\nmacro_rules! format { ($($input:tt)*) => { () }; }\n#[macro_export]\nmacro_rules! cfg { ($($input:tt)*) => { false }; }\n",
            );
            fixture.write(
                "src/lib.rs",
                "mod child;\n#[macro_use]\nextern crate macro_owner;\n",
            );
        } else {
            fixture.write(
                "Cargo.toml",
                &package_manifest("inherited_macro_use_sample"),
            );
            fixture.write("src/lib.rs", "#[macro_use]\nmod macros;\nmod child;\n");
            fixture.write(
                "src/macros.rs",
                "macro_rules! format { ($($input:tt)*) => { () }; }\nmacro_rules! cfg { ($($input:tt)*) => { false }; }\n",
            );
        }
        fixture.write(
            "src/child.rs",
            "pub fn inspect() { let _ = format!(\"value\"); let _ = cfg!(custom_mode); }\n",
        );

        let snapshot = analyze_fixture(&fixture);
        let child = snapshot
            .modules()
            .iter()
            .find(|module| {
                module
                    .namespace()
                    .last()
                    .is_some_and(|name| name == "child")
            })
            .expect("external child module");
        let format_status = child
            .parse_output()
            .syntax_nodes()
            .iter()
            .filter_map(|node| node.macro_invocation())
            .find(|invocation| {
                invocation
                    .path()
                    .segments()
                    .last()
                    .is_some_and(|segment| segment.name() == "format")
            })
            .map(|invocation| invocation.input().status().clone())
            .expect("format invocation");

        assert_eq!(format_status, MacroInputStatus::Opaque, "{import_kind}");
        assert!(child
            .configured_ast()
            .issues()
            .iter()
            .any(|issue| issue.kind() == ConfiguredAstIssueKind::OpaqueCfgMacro));
    }
}

#[test]
fn external_modules_inherit_prior_textual_macro_definitions() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &package_manifest("textual_macro_scope_sample"),
    );
    fixture.write(
        "src/lib.rs",
        "macro_rules! format { ($($input:tt)*) => { () }; }\nmacro_rules! cfg { ($($input:tt)*) => { false }; }\nmod child;\n",
    );
    fixture.write(
        "src/child.rs",
        "pub fn inspect() { let _ = format!(\"value\"); let _ = cfg!(custom_mode); }\n",
    );

    let snapshot = analyze_fixture(&fixture);
    let child = snapshot
        .modules()
        .iter()
        .find(|module| {
            module
                .namespace()
                .last()
                .is_some_and(|name| name == "child")
        })
        .expect("external child module");
    let statuses = child
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .map(|invocation| {
            let text = child
                .parse_output()
                .source()
                .slice(invocation.start(), invocation.end())
                .expect("macro call source range");
            (text, invocation.input().status().clone())
        })
        .collect::<Vec<_>>();

    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("format!") && status == &MacroInputStatus::Opaque
    }));
    assert!(child
        .configured_ast()
        .issues()
        .iter()
        .any(|issue| issue.kind() == ConfiguredAstIssueKind::OpaqueCfgMacro));
}

#[test]
fn external_modules_do_not_inherit_textual_macros_declared_after_them() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("late_textual_macro_sample"));
    fixture.write(
        "src/lib.rs",
        "mod child;\nmacro_rules! format { ($($input:tt)*) => { () }; }\n",
    );
    fixture.write(
        "src/child.rs",
        "pub fn inspect() { let _ = format!(\"value\"); }\n",
    );

    let snapshot = analyze_fixture(&fixture);
    let child = snapshot
        .modules()
        .iter()
        .find(|module| {
            module
                .namespace()
                .last()
                .is_some_and(|name| name == "child")
        })
        .expect("external child module");
    let status = child
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .find(|invocation| {
            invocation
                .path()
                .segments()
                .last()
                .is_some_and(|segment| segment.name() == "format")
        })
        .map(|invocation| invocation.input().status().clone())
        .expect("format invocation");

    assert_eq!(
        status,
        MacroInputStatus::KnownProfile(MacroProfileKey::Format)
    );
}

#[test]
fn resolves_external_macro_profiles_through_crate_root_aliases() {
    for declaration in ["use serde_json as sj;", "extern crate serde_json as sj;"] {
        let fixture = Fixture::new();
        fixture.write(
            "Cargo.toml",
            &format!(
                "{}\n[dependencies]\nserde_json = \"1\"\n",
                package_manifest("macro_alias_sample")
            ),
        );
        fixture.write(
            "src/lib.rs",
            &format!("{declaration}\npub fn inspect() {{ let _ = sj::json!({{\"key\": 1}}); }}\n"),
        );

        let snapshot = analyze_fixture(&fixture);
        let module = snapshot
            .modules()
            .iter()
            .find(|module| module.is_crate_root())
            .expect("crate root module");
        let status = module
            .parse_output()
            .syntax_nodes()
            .iter()
            .filter_map(|node| node.macro_invocation())
            .find(|invocation| {
                invocation
                    .path()
                    .segments()
                    .last()
                    .is_some_and(|segment| segment.name() == "json")
            })
            .map(|invocation| invocation.input().status().clone())
            .expect("json invocation");

        assert_eq!(
            status,
            MacroInputStatus::KnownProfile(MacroProfileKey::Json),
            "{declaration}"
        );
    }
}

#[test]
fn crate_root_aliases_respect_local_modules_and_unique_macro_identity() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &format!(
            "{}\n[dependencies]\nserde_json = \"1\"\n",
            package_manifest("macro_alias_shadow_sample")
        ),
    );
    fixture.write(
        "src/lib.rs",
        r#"
mod serde_json {
    macro_rules! json { ($($input:tt)*) => { () }; }
    pub(crate) use json;
}

use serde_json as sj;

pub fn inspect() {
    let _ = sj::json!({"key": 1});
}
"#,
    );

    let snapshot = analyze_fixture(&fixture);
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate root module");
    let status = module
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .find(|invocation| {
            invocation
                .path()
                .segments()
                .last()
                .is_some_and(|segment| segment.name() == "json")
        })
        .map(|invocation| invocation.input().status().clone())
        .expect("json invocation");

    assert_eq!(status, MacroInputStatus::Opaque);

    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("macro_identity_ambiguity"));
    fixture.write(
        "src/lib.rs",
        "use std::format as m;\nuse std::panic as m;\npub fn inspect() { let _ = m!(\"value\"); }\n",
    );

    let snapshot = analyze_fixture(&fixture);
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate root module");
    let status = module
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .find(|invocation| {
            invocation
                .path()
                .segments()
                .last()
                .is_some_and(|segment| segment.name() == "m")
        })
        .map(|invocation| invocation.input().status().clone())
        .expect("ambiguous macro invocation");

    assert_eq!(status, MacroInputStatus::Opaque);
}

#[test]
fn type_items_shadow_external_macro_crate_roots() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &format!(
            "{}\n[dependencies]\nserde_json = \"1\"\n",
            package_manifest("macro_type_shadow_sample")
        ),
    );
    fixture.write(
        "src/lib.rs",
        r#"
mod struct_shadow {
    struct serde_json;
    pub fn inspect() { let _ = serde_json::json!({"key": 1}); }
}
mod enum_shadow {
    enum serde_json { Variant }
    pub fn inspect() { let _ = serde_json::json!({"key": 2}); }
}
mod union_shadow {
    union serde_json { value: () }
    pub fn inspect() { let _ = serde_json::json!({"key": 3}); }
}
mod trait_shadow {
    trait serde_json {}
    pub fn inspect() { let _ = serde_json::json!({"key": 4}); }
}
mod alias_shadow {
    type serde_json = ();
    pub fn inspect() { let _ = serde_json::json!({"key": 5}); }
}
"#,
    );

    let snapshot = analyze_fixture(&fixture);
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate root module");
    let statuses = module
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .filter(|invocation| {
            invocation
                .path()
                .segments()
                .last()
                .is_some_and(|segment| segment.name() == "json")
        })
        .map(|invocation| invocation.input().status().clone())
        .collect::<Vec<_>>();

    assert_eq!(statuses.len(), 5);
    assert!(statuses
        .iter()
        .all(|status| status == &MacroInputStatus::Opaque));
}

#[test]
fn macro_profile_resolution_handles_scope_cycles_and_uncertain_imports() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("macro_resolution_edges"));
    fixture.write(
        "src/lib.rs",
        r#"
mod a { pub use crate::b::matches; }
mod b { pub use crate::a::matches; }

pub mod other { pub struct Thing; }

#[cfg(custom_cfg)]
use crate::other::Thing;
#[cfg(custom_cfg)]
use std::assert_eq as maybe_matches;

pub fn inspect(value: Option<u8>) {
    let _ = format!("{:?}", value);
    let _ = maybe_matches!(value, Some(_));
    let _ = a::matches!(value, Some(_));
    {
        use std::assert_eq as matches;
        let _ = matches!(value, Some(_));
    }
    let _ = matches!(value, Some(_));
}
"#,
    );
    let snapshot = analyze_fixture(&fixture);
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate root module");
    let statuses = module
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .map(|invocation| {
            let text = module
                .parse_output()
                .source()
                .slice(invocation.start(), invocation.end())
                .expect("macro call source range");
            (text, invocation.input().status().clone())
        })
        .collect::<Vec<_>>();

    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("format!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Format)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("maybe_matches!") && status == &MacroInputStatus::Opaque
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("a::matches!") && status == &MacroInputStatus::Opaque
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("matches!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::AssertEq)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("matches!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Matches)
    }));
}

#[test]
fn macro_profile_resolution_respects_no_std_and_cfg_disabled_cfg_shadows() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("macro_resolution_cfg"));
    fixture.write(
        "src/lib.rs",
        r#"#![no_std]
#[cfg(any())]
macro_rules! cfg { ($($tokens:tt)*) => { false } }
#[cfg(any())]
use core::cfg as cfg;

pub fn inspect(value: Option<u8>) {
    let _ = std::matches!(value, Some(_));
    let _ = cfg!(unix);
}

#[no_implicit_prelude]
pub mod std_context {
    extern crate std;

    pub fn inspect(value: Option<u8>) {
        let _ = std::matches!(value, Some(_));
    }
}
"#,
    );
    let snapshot = analyze_fixture(&fixture);
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate root module");
    let statuses = module
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .map(|invocation| {
            let text = module
                .parse_output()
                .source()
                .slice(invocation.start(), invocation.end())
                .expect("macro call source range");
            (text, invocation.input().status().clone())
        })
        .collect::<Vec<_>>();

    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("std::matches!") && status == &MacroInputStatus::Opaque
    }));
    assert!(snapshot.facts().cfg_macros().iter().any(|cfg_macro| {
        module
            .parse_output()
            .source()
            .slice(
                cfg_macro.predicate_bounds().start,
                cfg_macro.predicate_bounds().end,
            )
            .is_some_and(|predicate| predicate == "unix")
    }));

    let restricted = snapshot
        .modules()
        .iter()
        .find(|module| {
            module
                .namespace()
                .last()
                .is_some_and(|name| name == "std_context")
        })
        .expect("module with explicit std import");
    assert!(restricted
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .any(|invocation| {
            invocation.input().status() == &MacroInputStatus::KnownProfile(MacroProfileKey::Matches)
                && restricted
                    .parse_output()
                    .source()
                    .slice(invocation.start(), invocation.end())
                    .is_some_and(|text| text.starts_with("std::matches!"))
        }));
}

#[test]
fn no_std_accepts_explicitly_renamed_std_crate_for_macro_profiles() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("no_std_renamed_std_sample"));
    fixture.write(
        "src/lib.rs",
        "#![no_std]\nextern crate std as runtime;\npub fn inspect(value: Option<u8>) { let _ = runtime::matches!(value, Some(_)); }\n",
    );

    let snapshot = analyze_fixture(&fixture);
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate root module");
    let status = module
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .find(|invocation| {
            invocation
                .path()
                .segments()
                .first()
                .is_some_and(|segment| segment.name() == "runtime")
        })
        .map(|invocation| invocation.input().status().clone())
        .expect("renamed std macro invocation");

    assert_eq!(
        status,
        MacroInputStatus::KnownProfile(MacroProfileKey::Matches)
    );
}

#[test]
fn analysis_binds_profiles_from_selected_external_macro_dependencies() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &format!(
            "{}\n[dependencies]\nserde_json = \"1\"\nlog = \"0.4\"\n",
            package_manifest("macro_dependency_sample")
        ),
    );
    fixture.write(
        "src/lib.rs",
        "use serde_json::json as make_json;\nuse log::error as report_error;\n\npub fn inspect() {\n    let _ = serde_json::json!({\"key\": 1});\n    let _ = make_json!({\"key\": 2});\n    log::error!(\"value {}\", 3);\n    report_error!(\"value {}\", 4);\n}\n",
    );
    let snapshot = analyze_fixture(&fixture);
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate root module");
    let statuses = module
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .map(|invocation| {
            let text = module
                .parse_output()
                .source()
                .slice(invocation.start(), invocation.end())
                .expect("macro call source range");
            (text, invocation.input().status().clone())
        })
        .collect::<Vec<_>>();

    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("serde_json::json!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Json)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("make_json!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Json)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("log::error!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Error)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("report_error!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Error)
    }));
}

#[test]
fn macro_use_extern_crate_after_invocation_blocks_standard_macro_profiles() {
    for attribute in ["#[macro_use]", "#[cfg_attr(unix, macro_use)]"] {
        let fixture = Fixture::new();
        fixture.write(
            "Cargo.toml",
            &format!(
                "{}\n[dependencies]\nmacro_owner = {{ path = \"macro_owner\" }}\n",
                package_manifest("late_macro_use_sample")
            ),
        );
        fixture.write(
            "macro_owner/Cargo.toml",
            "[package]\nname = \"macro_owner\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        fixture.write(
            "macro_owner/src/lib.rs",
            "#[macro_export]\nmacro_rules! format { ($($input:tt)*) => { () }; }\n",
        );
        fixture.write(
            "src/lib.rs",
            &format!(
                "pub fn inspect() {{ let _ = format!(\"value\"); }}\n{attribute}\nextern crate macro_owner;\n"
            ),
        );

        let snapshot = analyze_fixture(&fixture);
        let module = snapshot
            .modules()
            .iter()
            .find(|module| module.is_crate_root())
            .expect("crate root module");
        let status = module
            .parse_output()
            .syntax_nodes()
            .iter()
            .filter_map(|node| node.macro_invocation())
            .find(|invocation| {
                invocation
                    .path()
                    .segments()
                    .last()
                    .is_some_and(|segment| segment.name() == "format")
            })
            .map(|invocation| invocation.input().status().clone())
            .expect("format invocation");

        assert_eq!(status, MacroInputStatus::Opaque, "{attribute}");
    }
}

#[test]
fn standard_env_macro_profiles_resolve_explicit_paths_and_aliases() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("std_env_macro_sample"));
    fixture.write(
        "src/lib.rs",
        "use std::env as build_env;\npub fn inspect() { let _ = std::env!(\"APP_NAME\"); let _ = build_env!(\"APP_NAME\"); }\n",
    );

    let snapshot = analyze_fixture(&fixture);
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate root module");
    let statuses = module
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .map(|invocation| {
            let text = module
                .parse_output()
                .source()
                .slice(invocation.start(), invocation.end())
                .expect("macro call source range");
            (text, invocation.input().status().clone())
        })
        .collect::<Vec<_>>();

    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("std::env!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Env)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("build_env!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Env)
    }));
}

#[test]
fn no_implicit_prelude_keeps_edition_specific_standard_macros_available() {
    let legacy_fixture = Fixture::new();
    legacy_fixture.write(
        "Cargo.toml",
        "[package]\nname = \"legacy_macro_prelude\"\nversion = \"0.1.0\"\nedition = \"2015\"\n",
    );
    legacy_fixture.write(
        "src/lib.rs",
        "#[no_implicit_prelude]\nmod legacy { pub fn inspect() { let _ = format!(\"value\"); let _ = vec![1]; } }\n",
    );
    let legacy_snapshot = analyze_fixture(&legacy_fixture);
    let legacy_module = legacy_snapshot
        .modules()
        .iter()
        .find(|module| {
            module
                .namespace()
                .last()
                .is_some_and(|name| name == "legacy")
        })
        .expect("legacy module");
    let legacy_statuses = legacy_module
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .map(|invocation| invocation.input().status())
        .collect::<Vec<_>>();
    assert!(legacy_statuses.contains(&&MacroInputStatus::KnownProfile(MacroProfileKey::Format)));
    assert!(legacy_statuses.contains(&&MacroInputStatus::KnownProfile(MacroProfileKey::Vec)));

    let modern_fixture = Fixture::new();
    modern_fixture.write("Cargo.toml", &package_manifest("modern_macro_prelude"));
    modern_fixture.write(
        "src/lib.rs",
        "#[no_implicit_prelude]\nmod restricted { pub fn inspect(value: bool) { assert!(value); panic!(\"failure\"); let _ = env!(\"APP_NAME\"); assert_eq!(value, true); let _ = format!(\"value\"); } }\n",
    );
    let modern_snapshot = analyze_fixture(&modern_fixture);
    let modern_module = modern_snapshot
        .modules()
        .iter()
        .find(|module| {
            module
                .namespace()
                .last()
                .is_some_and(|name| name == "restricted")
        })
        .expect("restricted module");
    let statuses = modern_module
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .map(|invocation| {
            (
                invocation.path().segments().last().unwrap().name(),
                invocation.input().status().clone(),
            )
        })
        .collect::<Vec<_>>();
    assert!(statuses.iter().any(|(name, status)| {
        *name == "assert" && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Assert)
    }));
    assert!(statuses.iter().any(|(name, status)| {
        *name == "panic" && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Format)
    }));
    assert!(statuses.iter().any(|(name, status)| {
        *name == "env" && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Env)
    }));
    assert!(statuses
        .iter()
        .any(|(name, status)| *name == "assert_eq" && status == &MacroInputStatus::Opaque));
    assert!(statuses
        .iter()
        .any(|(name, status)| *name == "format" && status == &MacroInputStatus::Opaque));
}

#[test]
fn analysis_uses_edition_and_prelude_context_for_macro_profiles() {
    for edition in ["2015", "2018", "2021", "2024"] {
        let fixture = Fixture::new();
        fixture.write(
            "Cargo.toml",
            &format!(
                "[package]\nname = \"edition_sample\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n"
            ),
        );
        fixture.write(
            "src/lib.rs",
            "pub fn inspect(value: Option<u8>) { assert!(matches!(value, Some(_))); let _ = format!(\"{:?}\", value); let _ = vec![value]; }\n",
        );
        let snapshot = analyze_fixture(&fixture);
        let module = snapshot
            .modules()
            .iter()
            .find(|module| module.is_crate_root())
            .expect("crate root module");
        let statuses = module
            .parse_output()
            .syntax_nodes()
            .iter()
            .filter_map(|node| node.macro_invocation())
            .map(|invocation| {
                let text = module
                    .parse_output()
                    .source()
                    .slice(invocation.start(), invocation.end())
                    .expect("macro call source range");
                (text, invocation.input().status().clone())
            })
            .collect::<Vec<_>>();
        assert!(statuses.iter().any(|(text, status)| {
            text.starts_with("assert!")
                && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Assert)
        }));
        assert!(statuses.iter().any(|(text, status)| {
            text.starts_with("matches!")
                && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Matches)
        }));
        assert!(statuses.iter().any(|(text, status)| {
            text.starts_with("format!")
                && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Format)
        }));
        assert!(statuses.iter().any(|(text, status)| {
            text.starts_with("vec!")
                && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Vec)
        }));
    }

    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("no_std_sample"));
    fixture.write(
        "src/lib.rs",
        r#"#![no_std]
#[cfg(any())]
macro_rules! matches { ($($tokens:tt)*) => {}; }

pub fn inspect(value: Option<u8>) {
    matches!(value, Some(_));
    let _ = format!("{:?}", value);
    let _ = vec![value];
}

#[no_implicit_prelude]
mod blocked {
    pub fn inspect(value: Option<u8>) {
        matches!(value, Some(_));
        let _ = env!("APP_NAME");
        core::matches!(value, Some(_));
    }
}
"#,
    );
    let snapshot = analyze_fixture(&fixture);
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate root module");
    let statuses = module
        .parse_output()
        .syntax_nodes()
        .iter()
        .filter_map(|node| node.macro_invocation())
        .map(|invocation| {
            let text = module
                .parse_output()
                .source()
                .slice(invocation.start(), invocation.end())
                .expect("macro call source range");
            (text, invocation.input().status().clone())
        })
        .collect::<Vec<_>>();
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("matches!(value")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Matches)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("format!") && status == &MacroInputStatus::Opaque
    }));
    assert!(statuses
        .iter()
        .any(|(text, status)| { text.starts_with("vec!") && status == &MacroInputStatus::Opaque }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("matches!(value") && status == &MacroInputStatus::Opaque
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("env!") && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Env)
    }));
    assert!(statuses.iter().any(|(text, status)| {
        text.starts_with("core::matches!")
            && status == &MacroInputStatus::KnownProfile(MacroProfileKey::Matches)
    }));
}

#[test]
fn macro_profiles_are_isolated_per_shared_module_instance() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("shared_module_sample"));
    fixture.write(
        "src/lib.rs",
        "#[path = \"shared.rs\"] mod std_context;\n\n#[no_implicit_prelude]\n#[path = \"shared.rs\"] mod no_implicit_context;\n",
    );
    fixture.write(
        "src/shared.rs",
        "pub fn inspect() { let _ = format!(\"value\"); }\n",
    );
    let snapshot = analyze_fixture(&fixture);
    let std_module = snapshot
        .modules()
        .iter()
        .find(|module| {
            module
                .namespace()
                .last()
                .is_some_and(|name| name == "std_context")
        })
        .expect("std-prelude module instance");
    let restricted_module = snapshot
        .modules()
        .iter()
        .find(|module| {
            module
                .namespace()
                .last()
                .is_some_and(|name| name == "no_implicit_context")
        })
        .expect("no_implicit_prelude module instance");
    let status_for = |module: &ModuleNode| {
        module
            .parse_output()
            .syntax_nodes()
            .iter()
            .filter_map(|node| node.macro_invocation())
            .find(|invocation| {
                invocation
                    .path()
                    .segments()
                    .last()
                    .is_some_and(|segment| segment.name() == "format")
            })
            .map(|invocation| invocation.input().status().clone())
            .expect("format! invocation")
    };

    assert_eq!(std_module.file_path(), restricted_module.file_path());
    assert_eq!(
        status_for(std_module),
        MacroInputStatus::KnownProfile(MacroProfileKey::Format)
    );
    assert_eq!(status_for(restricted_module), MacroInputStatus::Opaque);
}

#[test]
fn keeps_opaque_cfg_and_profile_fallback_inputs_out_of_path_references() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod model { pub struct Marker; }\n\nfn inspect() {\n    custom_dsl!({ crate::model::Marker });\n    assert!(crate::model::Marker + , true);\n    let _ = cfg!(crate::model::Marker);\n    let _ = cfg!(target_os = \"linux\");\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo 元数据");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), true, true))
            .with_current_directory(fixture.root.clone()),
    )
    .expect("选择 package 与 target");
    let package = project.packages().first().expect("选中的 package");
    let snapshot = ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("分析 target");
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate 根模块");
    assert_eq!(output.status, 2, "{}", output.stdout);
    assert!(output.stdout.contains("\"status\":\"incomplete\""));
    assert!(output
        .stdout
        .contains("\"analysis_issue\":\"parser_coverage\""));
    assert!(!output.has_rule("no_inline_qualified_path"));
    assert!(!snapshot
        .facts()
        .references_for_module(module)
        .any(|reference| {
            reference.segments().len() >= 3
                && reference
                    .segments()
                    .iter()
                    .take(3)
                    .map(String::as_str)
                    .eq(["crate", "model", "Marker"])
        }));
    assert!(!snapshot.facts().cfg_macros().is_empty());
    assert_eq!(
        snapshot
            .facts()
            .references_for_module(module)
            .filter(|reference| reference.kind() == ReferenceKind::Macro
                && reference
                    .segments()
                    .iter()
                    .map(String::as_str)
                    .eq(["custom_dsl"]))
            .count(),
        1
    );
}

#[test]
fn uses_the_cargo_library_target_name_for_self_library_paths() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"typol-core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"custom_library\"\n",
    );
    fixture.write("src/lib.rs", "pub struct Marker;\n");
    fixture.write(
        "tests/qualified_path.rs",
        "fn takes_self_crate_type(_: custom_library::Marker) {}\n",
    );

    let output = fixture.run(&["--all-targets", "--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "no_inline_qualified_path");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["target"], "qualified_path");
    assert_eq!(diagnostics[0]["path"], "tests/qualified_path.rs");
    assert!(diagnostic_message(&diagnostics[0]).contains("custom_library::Marker"));
}

#[test]
fn accepts_imported_names_from_the_package_library_in_test_targets() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("typol-core"));
    fixture.write("src/lib.rs", "pub struct Marker;\n");
    fixture.write(
        "tests/imported.rs",
        "use typol_core::Marker;\n\nfn takes_self_crate_type(_: Marker) {}\n",
    );

    let output = fixture.run(&["--all-targets", "--format", "json"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(!output.has_rule("no_inline_qualified_path"));
}

#[test]
fn reports_dev_dependency_paths_in_test_targets() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"vendor\"]\nresolver = \"2\"\n\n[dev-dependencies]\nvendor = { path = \"vendor\" }\n",
    );
    fixture.write("src/lib.rs", "pub struct Root;\n");
    fixture.write("vendor/Cargo.toml", &package_manifest("vendor"));
    fixture.write("vendor/src/lib.rs", "pub struct Thing;\n");
    fixture.write(
        "tests/dependency_path.rs",
        "fn takes_dev_dependency_type(_: vendor::Thing) {}\n",
    );

    let output = fixture.run(&["--all-targets", "--format", "json", "--package", "app"]);
    let diagnostics = rule_diagnostics(&output, "no_inline_qualified_path");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["target"], "dependency_path");
    assert_eq!(diagnostics[0]["target_kind"][0], "test");
    assert_eq!(diagnostics[0]["path"], "tests/dependency_path.rs");
    assert!(diagnostic_message(&diagnostics[0]).contains("vendor::Thing"));
}

#[test]
fn scopes_self_library_roots_to_linkable_targets_and_respects_local_shadowing() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("typol-core"));
    fixture.write("src/lib.rs", "pub struct Marker;\n");
    fixture.write("build.rs", "fn takes(_: typol_core::Marker) {}\n");
    fixture.write(
        "examples/qualified_path.rs",
        "fn takes_self_crate_type(_: typol_core::Marker) {}\n",
    );
    fixture.write(
        "tests/shadowed.rs",
        "mod typol_core { pub struct Marker; }\n\nfn local_module() { let _: typol_core::Marker; }\n\nfn local_binding(typol_core: u8) { let _ = typol_core::Marker; }\n",
    );

    let output = fixture.run(&["--all-targets", "--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "no_inline_qualified_path");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["target"], "qualified_path");
    assert_eq!(diagnostics[0]["target_kind"][0], "example");
    assert_eq!(diagnostics[0]["path"], "examples/qualified_path.rs");
}

#[test]
fn reports_external_paths_when_the_crate_is_reexported_by_its_own_name() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"opeo\"]\nresolver = \"2\"\n\n[dependencies]\nopeo = { path = \"opeo\" }\n",
    );
    fixture.write(
        "opeo/Cargo.toml",
        "[package]\nname = \"opeo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    fixture.write(
        "opeo/src/lib.rs",
        "pub struct Out<'source, 'slot, T>(pub &'source &'slot T);\n",
    );
    let callback_module = "    use opeo::Out;\n\n    struct Error;\n\n    type ResultValue = ();\n\n    fn callback(\n        operation: impl for<'call> FnOnce(opeo::Out<'call, 'call, Error>) -> ResultValue,\n    ) {\n        let _ = operation;\n    }\n";
    fixture.write(
        "src/lib.rs",
        &format!("mod callback {{\n{callback_module}}}\n"),
    );

    let direct = fixture.run(&["--all-features", "--all-targets", "--format", "json"]);

    let direct_diagnostics = rule_diagnostics(&direct, "no_inline_qualified_path");
    assert_eq!(direct_diagnostics.len(), 1, "{}", direct.stdout);
    assert!(
        diagnostic_message(&direct_diagnostics[0]).contains("opeo::Out"),
        "{}",
        direct.stdout
    );

    fixture.write(
        "src/lib.rs",
        &format!("pub use opeo;\n\nmod callback {{\n{callback_module}}}\n"),
    );
    let reexported = fixture.run(&["--all-features", "--all-targets", "--format", "json"]);

    let reexported_diagnostics = rule_diagnostics(&reexported, "no_inline_qualified_path");
    assert_eq!(reexported_diagnostics.len(), 1, "{}", reexported.stdout);
    assert!(
        diagnostic_message(&reexported_diagnostics[0]).contains("opeo::Out"),
        "{}",
        reexported.stdout
    );

    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo 元数据");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), true, true))
            .with_current_directory(fixture.root.clone()),
    )
    .expect("选择 package 与 target");
    let package = project.packages().first().expect("选中的 package");
    let snapshot =
        ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("catalog messages render");
    let reference = snapshot
        .facts()
        .references()
        .iter()
        .find(|reference| {
            reference.kind() != ReferenceKind::Use
                && reference
                    .segments()
                    .iter()
                    .map(String::as_str)
                    .eq(["opeo", "Out"])
        })
        .expect("回调类型中的外部 crate 路径");
    let resolution = reference.resolution();
    assert_eq!(resolution.kind(), PathResolutionKind::ExternalCrate);
    assert_eq!(resolution.root().kind(), PathRootKind::UseAlias);
    assert_eq!(resolution.root().external_crate(), Some("opeo"));
    assert!(resolution.is_external_crate_qualifier());
}

#[test]
fn reports_local_roots_in_nested_calls_closures_and_trait_impl_methods() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = "mod diagnostics { pub enum CodecStage { Conversion } }\n\nmod outer {\n    pub mod diagnostics { pub enum CodecStage { Conversion } }\n\n    pub mod child {\n        pub enum LocalStage { Conversion }\n\n        fn consume<T>(_: T) {}\n\n        fn wrap<T>(value: T) -> T { value }\n\n        fn nested_call_and_closure() {\n            consume(wrap(crate::diagnostics::CodecStage::Conversion));\n            let _closure = || crate::diagnostics::CodecStage::Conversion;\n            let _ = self::LocalStage::Conversion;\n            let _ = super::diagnostics::CodecStage::Conversion;\n        }\n\n        trait Operation {\n            fn convert() { let _ = crate::diagnostics::CodecStage::Conversion; }\n        }\n\n        struct Worker;\n\n        impl Operation for Worker {\n            fn convert() { let _ = crate::diagnostics::CodecStage::Conversion; }\n        }\n    }\n}\n";
    fixture.write("src/lib.rs", source);

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    let diagnostics = rule_diagnostics(&output, "no_inline_qualified_path");
    assert_eq!(diagnostics.len(), 6, "{}", output.stdout);
    let paths = diagnostics
        .iter()
        .map(|diagnostic| {
            let line = usize::try_from(diagnostic["start"]["line"].as_u64().expect("诊断起始行"))
                .expect("诊断起始行范围");
            let start_column =
                usize::try_from(diagnostic["start"]["column"].as_u64().expect("诊断起始列"))
                    .expect("诊断起始列范围");
            let end_column =
                usize::try_from(diagnostic["end"]["column"].as_u64().expect("诊断结束列"))
                    .expect("诊断结束列范围");
            let source_line = source.lines().nth(line - 1).expect("诊断对应源码行");
            source_line[start_column - 1..end_column - 1].to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        paths
            .iter()
            .filter(|path| path.as_str() == "crate::diagnostics::CodecStage::Conversion")
            .count(),
        4
    );
    assert!(paths
        .iter()
        .any(|path| path == "self::LocalStage::Conversion"));
    assert!(paths
        .iter()
        .any(|path| path == "super::diagnostics::CodecStage::Conversion"));
}

#[test]
fn reports_external_crate_root_aliases_without_misclassifying_local_names() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"vendor\"]\nresolver = \"2\"\n\n[dependencies]\nvendor = { path = \"vendor\" }\n",
    );
    fixture.write("vendor/Cargo.toml", &package_manifest("vendor"));
    fixture.write("vendor/src/lib.rs", "pub struct Thing;\n");
    fixture.write(
        "src/lib.rs",
        "fn local_alias() { use vendor as external; let _ = external::Thing; }\n",
    );

    let local_alias = fixture.run(&["--format", "json", "--package", "app"]);
    assert_eq!(local_alias.status, 1, "{}", local_alias.stdout);
    assert_eq!(
        local_alias
            .stdout
            .matches("\"rule_id\":\"no_inline_qualified_path\"")
            .count(),
        1,
        "{}",
        local_alias.stdout
    );

    fixture.write(
        "src/lib.rs",
        "fn local_group_alias() { use vendor::{self as external}; let _ = external::Thing; }\n",
    );
    let local_group_alias = fixture.run(&["--format", "json", "--package", "app"]);
    assert_eq!(local_group_alias.status, 1, "{}", local_group_alias.stdout);
    assert!(local_group_alias.has_rule("no_inline_qualified_path"));

    fixture.write(
        "src/lib.rs",
        "use vendor as external;\nfn module_alias() { let _ = external::Thing; }\n",
    );
    let module_alias = fixture.run(&["--format", "json", "--package", "app"]);
    assert_eq!(module_alias.status, 1, "{}", module_alias.stdout);
    assert!(module_alias.has_rule("no_inline_qualified_path"));

    fixture.write(
        "src/lib.rs",
        "use vendor::Thing;\nuse vendor::Thing as Imported;\nfn bare_item() { let _ = Thing; let _ = Imported::Associated; }\n",
    );
    let imported_items = fixture.run(&["--format", "json", "--package", "app"]);
    assert_eq!(imported_items.status, 0, "{}", imported_items.stdout);
    assert!(!imported_items.has_rule("no_inline_qualified_path"));

    fixture.write(
        "src/lib.rs",
        "mod local { pub struct Thing; }\nfn local_module() { mod vendor { pub struct Thing; } let _ = vendor::Thing; }\nfn parameter(vendor: u8) { let _ = vendor::Thing; }\nfn local_binding() { let vendor = 1; let _ = vendor::Thing; }\nfn local_use_alias() { use crate::local as vendor; let _ = vendor::Thing; }\n",
    );
    let local_names = fixture.run(&["--format", "json", "--package", "app"]);
    assert!(!local_names.has_rule("no_inline_qualified_path"));
}

#[test]
fn preserves_absolute_roots_when_a_local_module_shadows_an_external_crate() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod std {}\n\nfn direct() { ::std::mem::drop(1); }\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"no_inline_qualified_path\"")
            .count(),
        1,
        "{}",
        output.stdout
    );
    assert!(output.stdout.contains("::std::mem::drop"));
    assert!(!output.stdout.contains("::::std::mem::drop"));
}

#[test]
fn ignores_declaration_name_shadowed_by_a_local_binding() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn local_pattern() { let mut Later = 1; let _ = Later; }\n\nfn parameter_pattern(Later: u8) -> u8 { Later }\n\nfn closure_pattern() { let closure = |Later: u8| Later; }\n\nstruct Later;\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(!output.has_rule("declaration_before_use"));
}

#[test]
fn recognizes_bindings_in_destructuring_patterns() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "struct Record { value: u8 }\n\nfn destructured() { let (_, Later) = (1, 2); let _ = Later; let Record { value: Another } = make(); let _ = Another; }\n\nstruct Later;\n\nstruct Another;\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(!output.has_rule("declaration_before_use"));
}

#[test]
fn keeps_type_references_in_a_shadowing_let_declaration() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn local_type() { let mut Later: Later; }\n\nstruct Later;\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"declaration_before_use\"")
            .count(),
        1,
        "{}",
        output.stdout
    );
}

#[test]
fn local_bindings_do_not_escape_blocks_or_macro_token_trees() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn scoped_pattern() { { let mut Later = 1; let _ = Later; } opaque!(let Later = 2); let _ = Later; }\n\nstruct Later;\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"declaration_before_use\"")
            .count(),
        1,
        "{}",
        output.stdout
    );
}

#[test]
fn decodes_escaped_module_paths_in_path_and_cfg_attr_attributes() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nselected = []\n",
    );
    fixture.write(
        "src/lib.rs",
        r##"#[path = "hex\x2ers"] mod hex;
#[path = "continued/\
    child.rs"] mod continued;
#[path = r#"raw.rs"#] mod raw;
#[cfg_attr(feature = "selected", path = "unicode\u{2e}rs")]
mod selected;
"##,
    );
    fixture.write("src/hex.rs", "pub fn hex_path() {}\n");
    fixture.write("src/continued/child.rs", "pub fn continued_path() {}\n");
    fixture.write("src/raw.rs", "pub fn raw_path() {}\n");
    fixture.write("src/selected.rs", "fn default_path() {}\n");
    fixture.write("src/unicode.rs", "pub fn unicode_path() {}\n");

    let hex = fixture.run(&["--format", "json"]);
    let unicode = fixture.run(&["--format", "json", "--features", "selected"]);

    assert_eq!(hex.status, 1, "{}", hex.stdout);
    assert!(hex.stdout.contains("src/hex.rs"));
    assert!(hex.stdout.contains("src/continued/child.rs"));
    assert!(hex.stdout.contains("src/raw.rs"));
    assert!(!hex.stdout.contains("missing_module"));
    assert_eq!(unicode.status, 1, "{}", unicode.stdout);
    assert!(unicode.stdout.contains("src/unicode.rs"));
    assert!(!unicode.stdout.contains("missing_module"));
}

#[test]
fn resolves_cli_target_over_configured_target_and_reports_invalid_target() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "pub fn ready() {}\n");
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[scan]\ntarget_triples = [\"not-a-rust-target\"]\n",
    );
    let invalid = fixture.run(&["--format", "json"]);
    let overridden = fixture.run(&["--format", "json", "--target", "host"]);
    let invalid_cli = fixture.run(&["--format", "json", "--target", "not-a-rust-target"]);
    assert_eq!(invalid.status, 2, "{}", invalid.stderr);
    assert!(invalid.stdout.contains("\"kind\":\"tool\""));
    assert!(invalid.stdout.contains("\"tool_error\":\"configuration\""));
    assert_eq!(overridden.status, 0, "{}", overridden.stderr);
    assert_eq!(invalid_cli.status, 2, "{}", invalid_cli.stderr);
    assert!(invalid_cli.stdout.contains("\"tool_error\":\"arguments\""));
}

#[test]
fn output_order_is_byte_stable_across_repeated_runs() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[allow(dead_code)]\nfn alpha() {}\n#[allow(unused_variables)]\nfn beta(value: u8) {}\n",
    );
    let first = fixture.run(&["--format", "json"]);
    let second = fixture.run(&["--format", "json"]);
    assert_eq!(first.status, 1, "{}", first.stderr);
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn preserves_module_context_and_checks_visibility_and_impl_ownership() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "pub mod legacy;\n#[path = \"shared.rs\"] mod left;\n#[path = \"shared.rs\"] mod right;\nmod definitions;\nmod extension;\n",
    );
    fixture.write(
        "src/legacy/mod.rs",
        "pub fn exposed() {}\npub struct State { pub value: u8 }\nimpl State {}\npub struct Data { pub value: u8 }\n",
    );
    fixture.write("src/shared.rs", "pub fn shared() {}\n");
    fixture.write("src/definitions.rs", "pub struct Remote;\n");
    fixture.write("src/extension.rs", "impl crate::definitions::Remote {}\n");

    let output = fixture.run(&["--format", "json"]);
    assert_eq!(output.status, 1, "{}", output.stderr);
    assert!(output.has_rule("module_file_layout"));
    assert!(output.has_rule("child_module_visibility"));
    assert!(output.has_rule("child_public_free_functions"));
    assert!(output.has_rule("private_fields_for_behavior_types"));
    assert!(output.has_rule("impl_locality"));
    assert!(output.stdout.contains("crate::left"));
    assert!(output.stdout.contains("crate::right"));
    assert!(!output
        .stdout
        .contains("类型 \"Data\" 必须将所有字段设为私有"));
}

#[test]
fn allows_declaration_only_public_child_modules_in_inline_and_external_sources() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "pub mod inline_api {\n    pub use crate::types::PublicType;\n    pub(crate) use crate::types::PublicType as CrateType;\n    macro_rules! helper { () => {} }\n    type Alias = u8;\n    pub struct Record;\n    pub enum State { Ready }\n    #[cfg(any())]\n    fn disabled() {}\n    mod nested { fn implementation() {} }\n}\npub(crate) mod crate_api { pub(crate) use crate::types::PublicType; type Alias = u8; }\npub(crate) mod empty_api {}\n#[path = \"shared.rs\"] pub mod first_api;\n#[path = \"shared.rs\"] pub mod second_api;\n#[path = \"crate_shared.rs\"] pub(crate) mod crate_external;\nmod private_inline { struct Local; impl Local {} fn hidden() {} }\nmod private_external;\nmod types { pub struct PublicType; }\n",
    );
    fixture.write("src/shared.rs", "pub struct SharedType;\n");
    fixture.write("src/crate_shared.rs", "pub enum SharedState { Ready }\n");
    fixture.write("src/private_external.rs", "pub fn implementation() {}\n");

    let output = fixture.run(&["--format", "json"]);

    assert!(
        !output.has_rule("child_module_visibility"),
        "{}",
        output.stdout
    );
    assert!(
        !output.stdout.contains("macro_generated_module"),
        "{}",
        output.stdout
    );
}

#[test]
fn rejects_non_declarative_items_and_unsupported_module_visibility() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "pub mod has_function { fn hidden() {} }\npub mod has_impl { struct Local; impl Local {} }\npub mod has_restricted_use { pub(super) use core::fmt::Debug; }\npub mod has_macro_call { make_modules!(); }\npub mod has_unknown_cfg { #[cfg(mystery_flag)] fn possibly_enabled() {} }\npub(super) mod restricted { struct Local; }\npub(in crate) mod path_restricted { struct Local; }\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(
        rule_diagnostics(&output, "child_module_visibility").len(),
        7,
        "{}",
        output.stdout
    );
}

#[test]
fn rejects_public_modules_without_resolved_or_complete_child_sources() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "pub mod missing;\npub mod incomplete;\n#[cfg(any())]\npub mod disabled;\n",
    );
    fixture.write("src/incomplete.rs", "pub struct Record;\nfn broken( {\n");

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(
        rule_diagnostics(&output, "child_module_visibility").len(),
        2,
        "{}",
        output.stdout
    );
    assert!(output
        .stdout
        .contains("\"analysis_issue\":\"missing_module\""));
    assert!(output
        .stdout
        .contains("\"analysis_issue\":\"parser_coverage\""));
}

#[test]
fn evaluates_feature_gated_items_for_public_child_module_visibility() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nimplementation = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "pub mod api { #[cfg(feature = \"implementation\")] fn implementation() {} }\n",
    );

    let without_feature = fixture.run(&["--format", "json"]);
    let with_feature = fixture.run(&["--format", "json", "--features", "implementation"]);

    assert!(
        !without_feature.has_rule("child_module_visibility"),
        "{}",
        without_feature.stdout
    );
    assert!(
        with_feature.has_rule("child_module_visibility"),
        "{}",
        with_feature.stdout
    );
}

#[test]
fn replaces_private_child_module_id_in_configuration_and_cli() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "pub mod api { fn implementation() {} }\n");
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.private_child_modules]\nenabled = false\n",
    );

    let old_config = fixture.run(&["--format", "json"]);
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.child_module_visibility]\nseverity = \"warning\"\n",
    );
    let new_config = fixture.run(&["--format", "json"]);
    let old_cli = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "private_child_modules",
    ]);
    let new_cli = fixture.run(&[
        "--format",
        "json",
        "--severity",
        "child_module_visibility=warning",
    ]);

    assert_eq!(old_config.status, 2, "{}", old_config.stdout);
    assert_eq!(old_cli.status, 2, "{}", old_cli.stdout);
    assert_eq!(new_config.status, 0, "{}", new_config.stdout);
    assert_eq!(new_cli.status, 0, "{}", new_cli.stdout);
    assert!(new_config.has_rule("child_module_visibility"));
    assert!(new_config.stdout.contains("\"severity\":\"warning\""));
    assert!(new_cli.stdout.contains("\"severity\":\"warning\""));
}

#[test]
fn counts_only_active_code_lines_at_module_thresholds() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod below;\nmod boundary;\nmod upper;\nmod above;\n",
    );
    let below = (0..69)
        .map(|index| format!("fn item_{index}() {{}}\n\n"))
        .collect::<String>()
        + "#[cfg(test)]\nmod tests {\nfn ignored() {}\n}\n// 注释行；comment-only line\n";
    let boundary = (0..70)
        .map(|index| format!("fn item_{index}() {{}}\n\n"))
        .collect::<String>();
    let upper = format!(
        "fn count_lines() {{\n{}}}\n",
        (0..798)
            .map(|index| format!("    let _value_{index} = {index};\n"))
            .collect::<String>()
    );
    let above = format!(
        "fn count_lines() {{\n{}}}\n",
        (0..799)
            .map(|index| format!("    let _value_{index} = {index};\n"))
            .collect::<String>()
    );
    fixture.write("src/below.rs", &below);
    fixture.write("src/boundary.rs", &boundary);
    fixture.write("src/upper.rs", &upper);
    fixture.write("src/above.rs", &above);

    let output = fixture.run(&["--format", "json"]);
    assert_eq!(output.status, 1, "{}", output.stderr);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"child_module_line_count\"")
            .count(),
        2
    );
    assert!(has_diagnostic_message_fragment(&output, "69"));
    assert!(has_diagnostic_message_fragment(&output, "801"));
    assert!(!has_diagnostic_message_fragment(&output, "is 70;"));
    assert!(!has_diagnostic_message_fragment(&output, "is 800;"));
}

#[test]
fn all_targets_reports_one_child_module_line_count_across_test_cfg_contexts() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "mod filesystem;\n");
    let source_with_shared_lines = |shared_line_count: usize| {
        let body = (0..shared_line_count - 2)
            .map(|index| format!("    let _value_{index} = {index};\n"))
            .collect::<String>();
        format!(
            "fn count_lines() {{\n{body}    #[cfg(not(test))]\n    let _release_only = true;\n}}\n"
        )
    };
    fixture.write("src/filesystem.rs", &source_with_shared_lines(800));

    let arguments = ["--format", "json", "--locale", "en-US", "--all-targets"];
    let baseline = fixture.run(&arguments);
    let baseline_diagnostics = rule_diagnostics(&baseline, "child_module_line_count");

    assert_eq!(baseline.status, 1, "{}", baseline.stdout);
    assert_eq!(baseline_diagnostics.len(), 1, "{}", baseline.stdout);
    assert!(diagnostic_message(&baseline_diagnostics[0]).contains("is 802;"));

    fixture.write("src/filesystem.rs", &source_with_shared_lines(801));
    let extended = fixture.run(&arguments);
    let extended_diagnostics = rule_diagnostics(&extended, "child_module_line_count");

    assert_eq!(extended.status, 1, "{}", extended.stdout);
    assert_eq!(extended_diagnostics.len(), 1, "{}", extended.stdout);
    assert!(diagnostic_message(&extended_diagnostics[0]).contains("is 803;"));
}

#[test]
fn all_targets_checks_modules_reachable_in_only_one_test_mode() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[cfg(test)] mod test_only;\n#[cfg(not(test))] mod non_test_only;\n",
    );
    fixture.write("src/test_only.rs", &code_lines_for_count(801));
    fixture.write("src/non_test_only.rs", &code_lines_for_count(801));

    let output = fixture.run(&["--format", "json", "--all-targets"]);
    let diagnostics = rule_diagnostics(&output, "child_module_line_count");

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(diagnostics.len(), 2, "{}", output.stdout);
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic_message(diagnostic).contains("is 801;")
            && matches!(
                diagnostic["path"].as_str(),
                Some("src/test_only.rs" | "src/non_test_only.rs")
            )
    }));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["path"] == "src/test_only.rs"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["path"] == "src/non_test_only.rs"));
}

#[test]
fn test_and_bench_targets_check_test_only_child_modules_once() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[test]]\nname = \"integration\"\npath = \"tests/integration.rs\"\n\n[[bench]]\nname = \"performance\"\npath = \"benches/performance.rs\"\n",
    );
    fixture.write("src/lib.rs", "");
    fixture.write("tests/integration.rs", "mod test_child;\n");
    fixture.write("tests/test_child.rs", &code_lines_for_count(801));
    fixture.write("benches/performance.rs", "mod bench_child;\n");
    fixture.write("benches/bench_child.rs", &code_lines_for_count(801));

    let output = fixture.run(&["--format", "json", "--all-targets"]);
    let diagnostics = rule_diagnostics(&output, "child_module_line_count");

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(diagnostics.len(), 2, "{}", output.stdout);
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic_message(diagnostic).contains("is 801;")
            && diagnostic["target_kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "test" || kind == "bench"))
    }));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["target"] == "integration"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["target"] == "performance"));
}

#[test]
fn child_module_line_count_preserves_target_triple_and_namespace_instances() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[[bin]]\nname = \"worker\"\npath = \"src/main.rs\"\n",
    );
    fixture.write(
        "src/lib.rs",
        "mod shared;\n#[path = \"shared.rs\"] mod alias;\n",
    );
    fixture.write("src/main.rs", "mod shared;\nfn main() {}\n");
    fixture.write("src/shared.rs", &code_lines_for_count(801));

    let output = fixture.run(&[
        "--format",
        "json",
        "--all-targets",
        "--target",
        "x86_64-unknown-linux-gnu",
        "--target",
        "aarch64-unknown-linux-gnu",
        "--target",
        "x86_64-unknown-linux-gnu",
    ]);
    let diagnostics = rule_diagnostics(&output, "child_module_line_count");

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(diagnostics.len(), 6, "{}", output.stdout);
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic["path"] == "src/shared.rs" && diagnostic_message(diagnostic).contains("is 801;")
    }));
    for triple in ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"] {
        let triple_diagnostics = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic["target_triple"] == triple)
            .collect::<Vec<_>>();
        assert_eq!(triple_diagnostics.len(), 3, "{}", output.stdout);
        assert!(triple_diagnostics.iter().any(|diagnostic| {
            diagnostic["target"] == "sample" && diagnostic["namespace"] == "crate::shared"
        }));
        assert!(triple_diagnostics.iter().any(|diagnostic| {
            diagnostic["target"] == "sample" && diagnostic["namespace"] == "crate::alias"
        }));
        assert!(triple_diagnostics.iter().any(|diagnostic| {
            diagnostic["target"] == "worker" && diagnostic["namespace"] == "crate::shared"
        }));
        assert!(triple_diagnostics.iter().all(|diagnostic| {
            if diagnostic["target"] == "sample" {
                diagnostic["target_kind"] == json!(["lib"])
            } else {
                diagnostic["target_kind"] == json!(["bin"])
            }
        }));
    }
}

#[test]
fn child_module_line_count_reports_only_outside_the_inclusive_bounds() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod sixty_nine;\nmod seventy;\nmod eight_hundred;\nmod eight_hundred_one;\n",
    );
    fixture.write("src/sixty_nine.rs", &code_lines_for_count(69));
    fixture.write("src/seventy.rs", &code_lines_for_count(70));
    fixture.write("src/eight_hundred.rs", &code_lines_for_count(800));
    fixture.write("src/eight_hundred_one.rs", &code_lines_for_count(801));

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "child_module_line_count");

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(diagnostics.len(), 2, "{}", output.stdout);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["path"] == "src/sixty_nine.rs"
            && diagnostic_message(diagnostic).contains("is 69;")
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["path"] == "src/eight_hundred_one.rs"
            && diagnostic_message(diagnostic).contains("is 801;")
    }));
    assert!(diagnostics
        .iter()
        .all(|diagnostic| diagnostic["path"] != "src/seventy.rs"
            && diagnostic["path"] != "src/eight_hundred.rs"));
}

#[test]
fn child_module_line_count_drops_the_minimum_for_direct_external_children() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod below;\nmod boundary;\nmod upper;\nmod above;\nmod path_child;\n",
    );
    let write_parent = |name: &str, line_count: usize, declaration: &str| {
        let parent = format!("{}{declaration}", code_lines_for_count(line_count - 1));
        fixture.write(&format!("src/{name}.rs"), &parent);
        fixture.write(&format!("src/{name}/child.rs"), &code_lines_for_count(70));
    };
    write_parent("below", 69, "mod child;\n");
    write_parent("boundary", 70, "mod child;\n");
    write_parent("upper", 800, "mod child;\n");
    write_parent("above", 801, "mod child;\n");
    let path_parent = format!(
        "{}#[path = \"selected.rs\"]\nmod child;\n",
        code_lines_for_count(68)
    );
    fixture.write("src/path_child.rs", &path_parent);
    fixture.write("src/path_child/selected.rs", &code_lines_for_count(70));

    let output = fixture.run(&["--format", "json", "--locale", "en-US"]);
    let diagnostics = rule_diagnostics(&output, "child_module_line_count");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["path"], "src/above.rs");
    assert!(diagnostic_message(&diagnostics[0]).contains("at most 800"));

    let chinese = fixture.run(&["--format", "json", "--locale", "zh-CN"]);
    let chinese_diagnostics = rule_diagnostics(&chinese, "child_module_line_count");
    assert_eq!(chinese_diagnostics.len(), 1, "{}", chinese.stdout);
    assert!(diagnostic_message(&chinese_diagnostics[0]).contains("不得超过 800 行"));
}

#[test]
fn child_module_line_count_requires_the_minimum_without_a_direct_external_child() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod inline_only;\nmod inline_descendant;\nmod cfg_disabled;\nmod missing;\nmod unknown_cfg;\n",
    );
    fixture.write(
        "src/inline_only.rs",
        &format!(
            "{}mod inline {{ fn value() {{}} }}\n",
            code_lines_for_count(68)
        ),
    );
    fixture.write(
        "src/inline_descendant.rs",
        &format!("{}mod inline {{ mod child; }}\n", code_lines_for_count(68)),
    );
    fixture.write(
        "src/inline_descendant/inline/child.rs",
        &code_lines_for_count(70),
    );
    fixture.write(
        "src/cfg_disabled.rs",
        &format!("{}#[cfg(any())]\nmod child;\n", code_lines_for_count(69)),
    );
    fixture.write("src/cfg_disabled/child.rs", &code_lines_for_count(70));
    fixture.write(
        "src/missing.rs",
        &format!("{}mod absent;\n", code_lines_for_count(68)),
    );
    fixture.write(
        "src/unknown_cfg.rs",
        &format!(
            "{}#[cfg(maybe_child)]\nmod child;\n",
            code_lines_for_count(67)
        ),
    );
    fixture.write("src/unknown_cfg/child.rs", &code_lines_for_count(70));

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "child_module_line_count");

    assert_eq!(diagnostics.len(), 4, "{}", output.stdout);
    assert!(diagnostics.iter().all(|diagnostic| {
        matches!(
            diagnostic["path"].as_str(),
            Some(
                "src/inline_only.rs"
                    | "src/inline_descendant.rs"
                    | "src/cfg_disabled.rs"
                    | "src/missing.rs"
            )
        ) && diagnostic_message(diagnostic).contains("between 70 and 800")
    }));
    assert!(output
        .stdout
        .contains("\"analysis_issue\":\"missing_module\""));
}

#[test]
fn all_targets_uses_test_only_children_for_grouped_line_count_limits() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "mod small;\nmod large;\n");
    fixture.write(
        "src/small.rs",
        &format!(
            "{}#[cfg(test)]\nmod test_child;\n",
            code_lines_for_count(69)
        ),
    );
    fixture.write(
        "src/large.rs",
        &format!(
            "{}#[cfg(test)]\nmod test_child;\n",
            code_lines_for_count(801)
        ),
    );
    fixture.write("src/small/test_child.rs", &code_lines_for_count(70));
    fixture.write("src/large/test_child.rs", &code_lines_for_count(70));

    let output = fixture.run(&["--format", "json", "--all-targets"]);
    let diagnostics = rule_diagnostics(&output, "child_module_line_count");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["path"], "src/large.rs");
    assert!(diagnostic_message(&diagnostics[0]).contains("at most 800"));
}

#[test]
fn excludes_false_cfg_items_from_child_module_line_counts() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "mod child;\n");
    let inactive_body = (0..80)
        .map(|index| format!("    let _value_{index} = {index};\n"))
        .collect::<String>();
    let active_items = (0..70)
        .map(|index| format!("pub fn active_{index}() {{}}\n\n"))
        .collect::<String>();
    let child = format!("{active_items}\n#[cfg(any())]\nfn inactive() {{\n{inactive_body}}}\n");
    fixture.write("src/child.rs", &child);

    let output = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "child_public_free_functions",
    ]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(!output.has_rule("child_module_line_count"));
}

#[test]
fn detects_syntax_limits_and_ignores_macro_and_literal_contents() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn forward(value: Later) -> Later { value }\n\nfn five(a: u8, b: u8, c: u8, d: u8, e: u8) {}\n\nfn six(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) {}\n\nstruct Later;\n\nstruct Holder;\n\nimpl Holder {\n    fn receiver_six(&self, a: u8, b: u8, c: u8, d: u8, e: u8) {}\n}\n\nfn nested() {\n    let closure = |a, b, c, d, e, f| (a, b, c, d, e, f);\n    let _single = (1,);\n    let _wide = (1, 2, 3);\n    let _text = \"std::mem::drop(1)\";\n    opaque!(std::mem::drop(1));\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    assert_eq!(output.status, 1, "{}", output.stderr);
    assert!(output.has_rule("declaration_before_use"));
    assert!(output.has_rule("parameter_limit"));
    assert!(output.has_rule("tuple_arity"));
    assert!(!output.has_rule("no_inline_qualified_path"));
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"parameter_limit\"")
            .count(),
        3
    );
}

#[test]
fn exempts_extern_function_items_but_checks_nested_closures_and_foreign_declarations() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        r#"extern fn rust_abi(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) {}

extern "C" fn c_abi(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) {}

unsafe extern "C" fn unsafe_c_abi(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) {}

fn plain(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) {}

unsafe fn unsafe_only(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) {}

extern "C" fn with_closure(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) {
    let _closure = |a, b, c, d, e, f| ();
}

struct Service;

impl Service {
    fn receiver_six(&self, a: u8, b: u8, c: u8, d: u8, e: u8) {}

    extern fn rust_method(&self, a: u8, b: u8, c: u8, d: u8, e: u8) {}

    extern "C" fn c_method(&self, a: u8, b: u8, c: u8, d: u8, e: u8) {}

    unsafe extern "C" fn unsafe_c_method(&self, a: u8, b: u8, c: u8, d: u8, e: u8) {}
}

extern "C" {
    fn foreign(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8);
}
"#,
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "parameter_limit");
    let lines = diagnostics
        .iter()
        .map(|diagnostic| diagnostic["start"]["line"].as_u64().expect("诊断行号"))
        .collect::<Vec<_>>();

    assert_eq!(diagnostics.len(), 4, "{}", output.stdout);
    assert_eq!(lines, [7, 9, 12, 18]);
}

#[test]
fn reports_opaque_macro_input_without_evaluating_its_cfg_tokens() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn opaque() { hidden!({ #[cfg(macro_only_cfg)] let value = 0; }); assert!(#[cfg(macro_only_cfg)] true); }\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(output.stdout.contains("\"status\":\"partial\""));
    assert!(output
        .stdout
        .contains("\"analysis_issue\":\"parser_coverage\""));
    assert!(!output.stdout.contains("\"analysis_issue\":\"unknown_cfg\""));
    assert!(!output.stdout.contains("macro_only_cfg"));
}

#[test]
fn filters_false_closure_parameters_and_marks_unknown_parameter_limits() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn closures() {\n    let _reduced = |#[cfg(any())] hidden: u8, a: u8, b: u8, c: u8, d: u8, e: u8| ();\n    let _possible = |#[cfg(custom_parameter)] hidden: u8, a: u8, b: u8, c: u8, d: u8, e: u8| ();\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "parameter_limit");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["cfg_condition"], "custom_parameter");
    assert!(output.stdout.contains("\"analysis_issue\":\"unknown_cfg\""));
}

#[test]
fn skips_opaque_macro_coverage_inside_false_cfg_ancestors() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[cfg(any())]\nfn inactive() { hidden!({ #[cfg(macro_only_cfg)] let value = 0; }); }\nfn active() {}\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(output.stdout.contains("\"status\":\"complete\""));
    assert!(!output
        .stdout
        .contains("\"analysis_issue\":\"parser_coverage\""));
    assert!(!output.stdout.contains("macro_only_cfg"));
}

#[test]
fn does_not_count_turbofish_call_arguments_as_tuple_expressions() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn generic<T>(first: T, second: T, third: T) {}\n\nfn const_generic<const FLAG: bool>(first: u8, second: u8, third: u8) {}\n\nfn caller() {\n    generic::<Vec<Vec<u8>>>(1, 2, 3);\n    const_generic::<{ 2 > 1 }>(1, 2, 3);\n    let _ = true > (1, 2, 3);\n    let _ = generic::<Vec<Vec<u8>>>(1, 2, 3) >> (1, 2, 3);\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(
        output.stdout.matches("\"rule_id\":\"tuple_arity\"").count(),
        2,
        "{}",
        output.stdout
    );
    assert!(output
        .stdout
        .contains("\"start\":{\"column\":20,\"line\":8}"));
}

#[test]
fn checks_import_spacing_in_module_and_nested_block_scopes() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use std::fmt::Debug;\nuse std::fmt::Display;\n\n#[cfg(unix)]\nuse crate::left::One;\n#[cfg(windows)]\nuse crate::right::Two;\nuse std::{fmt, #[cfg(unix)] io};\n\nfn body() {\n    let value = 1;\n    use std::path::Path;\n    {\n        use std::time::Duration;\n        let nested = 2;\n        use std::collections::HashMap;\n    }\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    assert_eq!(output.status, 1, "{}", output.stderr);
    assert!(output.stdout.contains("\"status\":\"complete\""));
    assert!(output.has_rule("merge_common_prefix_imports"));
    assert!(!output.has_rule("use_group_spacing"));
    assert!(output.has_rule("cfg_inside_use_tree"));
    assert!(output.has_rule("imports_at_scope_start"));
}

#[test]
fn merges_module_import_leaf_pairs_with_the_longest_common_prefix() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use crate::{model::nested::First, model::Second};\nuse crate::{model::nested::Third, model::Fourth};\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "merge_common_prefix_imports");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["start"]["line"], 2);
    assert_eq!(reported_import_line(&diagnostics[0]), 1);
    assert!(diagnostic_message(&diagnostics[0]).contains("crate::model::nested"));
}

#[test]
fn merges_module_multi_leaf_imports_with_single_leaf_imports() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use crate::{model::First, model::Second};\nuse crate::model::Third;\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "merge_common_prefix_imports");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["start"]["line"], 2);
    assert_eq!(reported_import_line(&diagnostics[0]), 1);
    assert!(diagnostic_message(&diagnostics[0]).contains("crate::model"));
}

#[test]
fn compares_only_ordinary_leaf_pairs_in_module_imports() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use crate::{model::Alias as FirstAlias, model::*};\nuse crate::{model::Alias as SecondAlias, model::*};\nuse crate::{model::First, model::Alias as ThirdAlias, model::*};\nuse crate::{model::Second, model::Alias as FourthAlias, model::*};\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "merge_common_prefix_imports");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["start"]["line"], 4);
    assert_eq!(reported_import_line(&diagnostics[0]), 3);
    assert!(diagnostic_message(&diagnostics[0]).contains("crate::model"));
}

#[test]
fn merges_nonadjacent_module_imports_with_a_different_root_between() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use crate::parser::block::patterns::{PatternInput, PatternParser};\n\nuse super::super::expression::{ExpressionInput, ExpressionParser};\n\nuse crate::parser::types::{TypeInput, TypeParser};\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "merge_common_prefix_imports");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["start"]["line"], 5);
    assert_eq!(reported_import_line(&diagnostics[0]), 1);
    assert!(diagnostic_message(&diagnostics[0]).contains("crate::parser"));
}

#[test]
fn reports_each_nonadjacent_import_once_using_the_nearest_compatible_import() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use crate::model::First;\nfn separator() {}\nuse crate::model::Second;\nuse std::fmt::Debug;\nfn another_separator() {}\nuse crate::model::Third;\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "merge_common_prefix_imports");

    assert_eq!(diagnostics.len(), 2, "{}", output.stdout);
    assert_eq!(diagnostics[0]["start"]["line"], 3);
    assert_eq!(reported_import_line(&diagnostics[0]), 1);
    assert_eq!(diagnostics[1]["start"]["line"], 6);
    assert_eq!(reported_import_line(&diagnostics[1]), 3);
    assert!(diagnostic_message(&diagnostics[1]).contains("crate::model"));
}

#[test]
fn ignores_incompatible_module_imports_and_false_cfg_leaves() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nfirst = []\nsecond = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "use core::{model::First, model::Second};\n#[cfg(any())]\nuse crate::{model::InactiveFirst, model::InactiveSecond};\nuse std::{model::Third, model::Fourth};\npub use crate::{model::Fifth, model::Sixth};\n#[cfg(feature = \"first\")]\nuse crate::{model::Seventh, model::Eighth};\n#[cfg(feature = \"second\")]\nuse crate::{model::Ninth, model::Tenth};\nfn separator() {}\nuse crate::{model::Eleventh, model::Twelfth};\n",
    );

    let output = fixture.run(&["--format", "json", "--features", "first,second"]);

    assert!(
        rule_diagnostics(&output, "merge_common_prefix_imports").is_empty(),
        "{}",
        output.stdout
    );
}

#[test]
fn ignores_parser_unsupported_module_use_leaves() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use crate::{#[broken(,)] model::First, model::*};\nuse crate::model::Third;\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert!(
        rule_diagnostics(&output, "merge_common_prefix_imports").is_empty(),
        "{}",
        output.stdout
    );
}

#[test]
fn does_not_compare_branches_inside_one_module_use_tree() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "use crate::{model::First, model::Second};\n");

    let output = fixture.run(&["--format", "json"]);

    assert!(
        rule_diagnostics(&output, "merge_common_prefix_imports").is_empty(),
        "{}",
        output.stdout
    );
}

#[test]
fn merges_block_multi_leaf_imports_and_keeps_source_order_for_equal_prefixes() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn body() {\n    use crate::{alpha::left::First, beta::right::Second};\n    use crate::{alpha::left::Third, beta::right::Fourth};\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "merge_common_prefix_imports");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["start"]["line"], 3);
    assert_eq!(reported_import_line(&diagnostics[0]), 2);
    assert!(diagnostic_message(&diagnostics[0]).contains("crate::alpha::left"));
}

#[test]
fn merges_block_multi_leaf_imports_with_single_leaf_imports() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn body() {\n    use crate::{model::First, model::Second};\n    use crate::model::Third;\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "merge_common_prefix_imports");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["start"]["line"], 3);
    assert_eq!(reported_import_line(&diagnostics[0]), 2);
    assert!(diagnostic_message(&diagnostics[0]).contains("crate::model"));
}

#[test]
fn preserves_single_leaf_block_import_diagnostic_parameters() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn body() {\n    use crate::model::First;\n    use crate::model::Second;\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "merge_common_prefix_imports");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["start"]["line"], 3);
    assert_eq!(reported_import_line(&diagnostics[0]), 2);
    assert!(diagnostic_message(&diagnostics[0]).contains("crate::model"));
}

#[test]
fn ignores_block_imports_that_are_not_at_scope_start_for_prefix_merging() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn body() {\n    let value = 1;\n    use crate::{model::First, model::Second};\n    use crate::{model::Third, model::Fourth};\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert!(
        rule_diagnostics(&output, "merge_common_prefix_imports").is_empty(),
        "{}",
        output.stdout
    );
}

#[test]
fn accepts_extern_crates_modules_and_imports_in_module_scope_order() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nbeta = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg(feature = \"beta\")]\nextern crate std as standard;\nextern crate core as primitive;\n#[path = \"parts/alpha.rs\"]\nmod alpha;\n#[cfg(feature = \"beta\")]\nmod beta;\nuse standard::fmt::Debug;\nuse primitive::marker::Copy;\nmod nested_scope {\n    mod child;\n    use crate::alpha::Alpha;\n    fn nested() {}\n}\nmod inline_after_imports {\n    fn ready() {}\n}\nfn run() {}\n",
    );
    fixture.write("src/parts/alpha.rs", "pub struct Alpha;\n");
    fixture.write("src/beta.rs", "pub struct Beta;\n");
    fixture.write("src/nested_scope/child.rs", "pub struct Child;\n");

    let output = fixture.run(&[
        "--format",
        "json",
        "--features",
        "beta",
        "--disable-rule",
        "child_module_line_count",
    ]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(output.stdout.contains("\"status\":\"complete\""));
    assert!(!output.has_rule("out_of_line_modules_at_scope_start"));
    assert!(!output.has_rule("imports_at_scope_start"));
    assert!(!output.has_rule("extern_crate_at_scope_start"));
}

#[test]
fn resolves_extern_crate_aliases_for_use_and_qualified_paths() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "extern crate std as standard;\nuse standard::mem;\nfn direct() { standard::mem::drop(1); mem::drop(2); }\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"no_inline_qualified_path\"")
            .count(),
        1,
        "{}",
        output.stdout
    );
    assert!(output.stdout.contains("standard::mem::drop"));
    assert!(!output.has_rule("imports_at_scope_start"));
    assert!(!output.has_rule("out_of_line_modules_at_scope_start"));
}

#[test]
fn reports_module_scope_order_violations_on_the_late_items() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod before_crate;\n#[cfg(any())]\nextern crate std as after_module;\n\nuse std::fmt::Debug;\n#[cfg(any())]\nextern crate std as after_use;\n\nfn run() {}\nuse core::fmt::Debug;\nmod after_use;\n#[cfg(any())]\nmod inactive_scope {\n    fn first() {}\n    use std::fmt::Debug;\n}\n",
    );
    fixture.write("src/before_crate.rs", "pub struct BeforeCrate;\n");
    fixture.write("src/after_use.rs", "pub struct AfterUse;\n");

    let chinese = fixture.run(&[
        "--format",
        "json",
        "--locale",
        "zh-CN",
        "--disable-rule",
        "child_module_line_count",
        "--disable-rule",
        "use_group_spacing",
    ]);
    let english = fixture.run(&[
        "--format",
        "json",
        "--locale",
        "en-US",
        "--disable-rule",
        "child_module_line_count",
        "--disable-rule",
        "use_group_spacing",
    ]);

    assert_eq!(chinese.status, 1, "{}", chinese.stdout);
    assert_eq!(english.status, 1, "{}", english.stdout);
    let extern_crates = rule_diagnostics(&chinese, "extern_crate_at_scope_start");
    assert!(extern_crates.is_empty(), "{}", chinese.stdout);
    let english_extern_crates = rule_diagnostics(&english, "extern_crate_at_scope_start");
    assert!(english_extern_crates.is_empty(), "{}", english.stdout);

    let imports = rule_diagnostics(&chinese, "imports_at_scope_start");
    assert_eq!(imports.len(), 1, "{}", chinese.stdout);
    assert_eq!(
        imports
            .iter()
            .map(|diagnostic| diagnostic["start"]["line"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![10]
    );
    assert!(imports[0]["message"]
        .as_str()
        .unwrap()
        .contains("模块作用域中的 use 声明"));
    let english_imports = rule_diagnostics(&english, "imports_at_scope_start");
    assert!(english_imports[0]["message"]
        .as_str()
        .unwrap()
        .contains("Module-scope use declarations"));

    let modules = rule_diagnostics(&chinese, "out_of_line_modules_at_scope_start");
    assert_eq!(modules.len(), 1, "{}", chinese.stdout);
    assert_eq!(modules[0]["start"]["line"], 11);
    assert!(modules[0]["message"]
        .as_str()
        .unwrap()
        .contains("外部模块声明必须位于 extern crate 声明之后"));
    let english_modules = rule_diagnostics(&english, "out_of_line_modules_at_scope_start");
    assert!(english_modules[0]["message"]
        .as_str()
        .unwrap()
        .contains("Out-of-line module declarations must follow extern crate declarations"));
}

#[test]
fn checks_late_extern_crates_in_each_module_scope_but_not_function_blocks() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "extern crate std as root_prefix;\nmod external_scope;\nfn root_item() {}\n#[cfg(any())]\nextern crate core as root_late;\nmod inline_scope {\n    extern crate std as inline_prefix;\n    fn inline_item() {}\n    #[cfg(any())]\n    extern crate core as inline_late;\n    fn nested_block() {\n        extern crate alloc;\n    }\n}\n",
    );
    fixture.write(
        "src/external_scope.rs",
        "extern crate std as child_prefix;\nfn child_item() {}\n#[cfg(any())]\nextern crate core as child_late;\n",
    );

    let output = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "child_module_line_count",
        "--disable-rule",
        "item_spacing",
    ]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    let extern_crates = rule_diagnostics(&output, "extern_crate_at_scope_start");
    assert!(extern_crates.is_empty(), "{}", output.stdout);
    assert!(!output.has_rule("imports_at_scope_start"));
    assert!(!output.has_rule("out_of_line_modules_at_scope_start"));
}

#[test]
fn configures_extern_crate_order_rule_and_allows_disabling_it() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "fn run() {}\nextern crate std;\n");
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.extern_crate_at_scope_start]\nenabled = false\nseverity = \"warning\"\n",
    );

    let configured_disabled = fixture.run(&["--format", "json"]);
    assert_eq!(
        configured_disabled.status, 0,
        "{}",
        configured_disabled.stdout
    );
    assert!(!configured_disabled.has_rule("extern_crate_at_scope_start"));

    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.extern_crate_at_scope_start]\nenabled = true\nseverity = \"warning\"\n",
    );
    let warning = fixture.run(&["--format", "json"]);
    let error = fixture.run(&[
        "--format",
        "json",
        "--severity",
        "extern_crate_at_scope_start=error",
    ]);
    let cli_disabled = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "extern_crate_at_scope_start",
    ]);

    assert_eq!(warning.status, 0, "{}", warning.stdout);
    assert!(warning.has_rule("extern_crate_at_scope_start"));
    assert!(warning.stdout.contains("\"severity\":\"warning\""));
    assert_eq!(error.status, 1, "{}", error.stdout);
    assert!(error.stdout.contains("\"severity\":\"error\""));
    assert_eq!(cli_disabled.status, 0, "{}", cli_disabled.stdout);
    assert!(!cli_disabled.has_rule("extern_crate_at_scope_start"));
}

#[test]
fn skips_target_specific_order_checks_inside_files_disabled_by_cfg_attributes() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "mod disabled;\n");
    fixture.write(
        "src/disabled.rs",
        "#![cfg(any())]\nfn first() {}\nuse std::fmt::Debug;\n",
    );

    let output = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "child_module_line_count",
    ]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    let imports = rule_diagnostics(&output, "imports_at_scope_start");
    assert!(imports.is_empty(), "{}", output.stdout);

    fixture.write(
        "src/lib.rs",
        "#![cfg(any())]\nfn first() {}\nuse std::fmt::Debug;\n",
    );
    let root_output = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "child_module_line_count",
    ]);
    assert_eq!(root_output.status, 0, "{}", root_output.stdout);
    let root_imports = rule_diagnostics(&root_output, "imports_at_scope_start");
    assert!(root_imports.is_empty(), "{}", root_output.stdout);
}

#[test]
fn reports_each_out_of_line_module_after_the_module_prefix() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use crate::types::Thing;\nmod after_import;\nfn run() {}\nmod after_function;\n",
    );
    fixture.write("src/after_import.rs", "pub struct First;\n");
    fixture.write("src/after_function.rs", "pub struct Second;\n");

    let output = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "child_module_line_count",
    ]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"out_of_line_modules_at_scope_start\"")
            .count(),
        2,
        "{}",
        output.stdout
    );
    assert!(!output.has_rule("imports_at_scope_start"));
}

#[test]
fn keeps_inline_module_and_block_import_order_checks_separate() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod inline_first {}\nuse crate::Thing;\nfn body() {\n    let value = 1;\n    use crate::Other;\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"imports_at_scope_start\"")
            .count(),
        2,
        "{}",
        output.stdout
    );
    assert!(!output.has_rule("out_of_line_modules_at_scope_start"));
}

#[test]
fn configures_out_of_line_module_order_rule_and_allows_disabling_it() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "fn run() {}\nmod child;\n");
    fixture.write("src/child.rs", "pub struct Child;\n");
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.out_of_line_modules_at_scope_start]\nseverity = \"warning\"\n",
    );

    let warning = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "child_module_line_count",
    ]);
    let disabled = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "child_module_line_count",
        "--disable-rule",
        "out_of_line_modules_at_scope_start",
    ]);

    assert_eq!(warning.status, 0, "{}", warning.stdout);
    assert!(warning.has_rule("out_of_line_modules_at_scope_start"));
    assert!(warning.stdout.contains("\"severity\":\"warning\""));
    assert_eq!(disabled.status, 0, "{}", disabled.stdout);
    assert!(!disabled.has_rule("out_of_line_modules_at_scope_start"));
}

#[test]
fn normalizes_stacked_cfg_conditions_before_import_grouping() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\na = []\nb = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg(all(feature = \"a\", feature = \"b\"))]\nuse crate::module::First;\n\n#[cfg(feature = \"b\")]\n#[cfg(feature = \"a\")]\nuse core::fmt::Debug;\n\n#[cfg(feature = \"b\")]\n#[cfg(feature = \"a\")]\nuse crate::module::Second;\n",
    );

    let output = fixture.run(&["--format", "json", "--features", "a,b"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"merge_common_prefix_imports\"")
            .count(),
        1,
        "{}",
        output.stdout
    );
    let diagnostics = rule_diagnostics(&output, "merge_common_prefix_imports");
    assert_eq!(diagnostics[0]["start"]["line"], 8);
    assert_eq!(reported_import_line(&diagnostics[0]), 1);
    assert!(diagnostic_message(&diagnostics[0]).contains("crate::module"));
    assert!(!output.has_rule("use_group_spacing"));
}

#[test]
fn skips_targets_until_their_required_features_are_enabled() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nextra = []\n\n[[example]]\nname = \"gated\"\npath = \"examples/gated.rs\"\nrequired-features = [\"extra\"]\n",
    );
    fixture.write("src/lib.rs", "pub fn ready() {}\n");
    fixture.write(
        "examples/gated.rs",
        "#[allow(dead_code)]\nfn gated_only() {}\nfn main() {}\n",
    );

    let skipped = fixture.run(&["--format", "json", "--all-targets"]);
    let selected = fixture.run(&["--format", "json", "--all-targets", "--features", "extra"]);
    assert_eq!(skipped.status, 0, "{}", skipped.stderr);
    assert!(!skipped.stdout.contains("examples/gated.rs"));
    assert_eq!(selected.status, 1, "{}", selected.stderr);
    assert!(selected.stdout.contains("examples/gated.rs"));
}

#[test]
fn rejects_invalid_configuration_without_falling_back_to_defaults() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "pub fn ready() {}\n");
    fixture.write("cargo-cop.toml", "version = 4\nunknown = true\n");

    let output = fixture.run(&["--format", "json"]);
    assert_eq!(output.status, 2, "{}", output.stderr);
    assert!(output.stdout.contains("\"kind\":\"tool\""));
    assert!(output.stdout.contains("\"tool_error\":\"configuration\""));
    assert!(!output.stdout.contains("\"status\":\"complete\""));
}

#[test]
fn checks_parent_private_access_spacing_and_nested_lint_attributes() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nextra = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "struct First;\nstruct Second;\n\nstruct Third;\n\n/// 保留文档注释；Retained documentation\nstruct Fourth;\n\n\nstruct Fifth;\n\n#[cfg_attr(all(unix, feature = \"extra\"), cfg_attr(unix, allow(dead_code)))]\nfn relaxed() {}\n\nmod parent;\n",
    );
    fixture.write(
        "src/parent.rs",
        "struct Secret;\n\npub struct Visible;\nmod child;\n",
    );
    fixture.write(
        "src/parent/child.rs",
        "use super::Secret as Hidden;\nuse super::Visible as Public;\nuse super::*;\nfn references() { let _ = super::Secret; let _ = crate::parent::Secret; }\n",
    );

    let inactive = fixture.run(&["--format", "json"]);
    assert!(
        !inactive.has_rule("no_lint_relaxation"),
        "{}",
        inactive.stdout
    );

    let output = fixture.run(&["--format", "json", "--features", "extra"]);
    assert_eq!(output.status, 1, "{}", output.stderr);
    assert!(output.has_rule("no_parent_private_dependency"));
    assert!(output.has_rule("item_spacing"));
    assert!(output.has_rule("no_lint_relaxation"));
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"item_spacing\"")
            .count(),
        2,
        "{}",
        output.stdout
    );
}

#[test]
fn accepts_adjacent_same_kind_const_static_and_type_definitions() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "const FIRST: u8 = 1;\nconst SECOND: u8 = 2;\n\nconst THIRD: u8 = 3;\n\nstatic FIRST_STATIC: u8 = 1;\nstatic SECOND_STATIC: u8 = 2;\n\nstatic THIRD_STATIC: u8 = 3;\n\ntype FirstAlias = u8;\ntype SecondAlias = u8;\n\ntype ThirdAlias = u8;\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(
        item_spacing_diagnostics(&output).is_empty(),
        "{}",
        output.stdout
    );
}

#[test]
fn reports_maximum_spacing_for_repeated_special_definitions() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "const FIRST: u8 = 1;\n\n\nconst SECOND: u8 = 2;\n\n\n\nconst THIRD: u8 = 3;\n\nstatic FIRST_STATIC: u8 = 1;\n\n\nstatic SECOND_STATIC: u8 = 2;\n\n\n\nstatic THIRD_STATIC: u8 = 3;\n\ntype FirstAlias = u8;\n\n\ntype SecondAlias = u8;\n\n\n\ntype ThirdAlias = u8;\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = item_spacing_diagnostics(&output);
    let chinese = fixture.run(&["--format", "json", "--locale", "zh-CN"]);
    let chinese_diagnostics = item_spacing_diagnostics(&chinese);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(chinese.status, 1, "{}", chinese.stdout);
    assert_eq!(diagnostics.len(), 6, "{}", output.stdout);
    assert!(diagnostics
        .iter()
        .all(|diagnostic| spacing_values(diagnostic).0 == SpacingExpectation::AtMost(1)));
    let actual_lines = diagnostics
        .iter()
        .map(|diagnostic| Some(spacing_values(diagnostic).1))
        .collect::<Vec<_>>();
    assert_eq!(
        actual_lines,
        [Some(2), Some(3), Some(2), Some(3), Some(2), Some(3)]
    );
    assert_eq!(chinese_diagnostics.len(), 6, "{}", chinese.stdout);
    assert!(chinese_diagnostics.iter().all(|diagnostic| {
        diagnostic_message(diagnostic).contains("最多允许 1 个空行")
            && spacing_values(diagnostic).0 == SpacingExpectation::AtMost(1)
    }));
}

#[test]
fn requires_one_line_between_different_and_regular_definition_kinds() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "const VALUE: u8 = 1;\nstatic FIRST: u8 = 1;\nstatic SECOND: u8 = 2;\n\n\ntype Alias = u8;\nconst OTHER: u8 = 2;\nconst fn compute() {}\n\nstruct First;\nstruct Second;\n\nenum Choice { One }\n\nunion Number { value: u32 }\n\ntrait Contract {}\n\nfn run() {}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = item_spacing_diagnostics(&output);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(diagnostics.len(), 5, "{}", output.stdout);
    assert!(diagnostics
        .iter()
        .all(|diagnostic| spacing_values(diagnostic).0 == SpacingExpectation::Exact(1)));
    let actual_lines = diagnostics
        .iter()
        .map(|diagnostic| Some(spacing_values(diagnostic).1))
        .collect::<Vec<_>>();
    assert_eq!(actual_lines, [Some(0), Some(2), Some(0), Some(0), Some(0)]);
}

#[test]
fn ignores_item_spacing_pairs_separated_by_cfg_disabled_items() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = include_str!("fixtures/cfg_filtered_item_spacing.rs").replace(
        "#[cfg(all(not(test), target_arch = \"aarch64\"))]",
        "#[cfg(any())]",
    );
    let source =
        format!("{source}\n#[cfg(not(test))]\nfn inspect_raw(raw: u8) {{ consume(&raw); }}\n");
    fixture.write("src/main.rs", &source);
    fixture.write("src/app.rs", "fn placeholder() {}\n");
    fixture.write("src/memory_helpers.rs", "fn placeholder() {}\n");

    let output = fixture.run(&[
        "--format",
        "json",
        "--all-targets",
        "--all-features",
        "--workspace",
        "--disable-rule",
        "child_module_line_count",
    ]);
    let diagnostics = item_spacing_diagnostics(&output);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["start"]["line"] == 151);
    assert!(diagnostic.is_none(), "{}", output.stdout);
    assert!(!has_diagnostic_message_fragment(
        &output,
        "The syntax construct was incomplete at end of input",
    ));
    assert!(!has_diagnostic_message_fragment(
        &output,
        "语法结构在输入结束前未完成",
    ));
}

#[test]
fn treats_cfg_disabled_items_as_spacing_boundaries_in_all_scopes() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "struct RootBefore;\n#[cfg(any())]\nstruct RootDisabled;\n\n\nstruct RootAfter;\n\ntrait Contract {\n    fn before();\n    #[cfg(any())]\n    const DISABLED: u8 = 0;\n\n\n    fn after();\n}\n\nstruct Service;\n\nimpl Service {\n    fn before(&self) {}\n    #[cfg(any())]\n    const DISABLED: u8 = 0;\n\n\n    fn after(&self) {}\n}\n\n#[cfg(test)]\nmod tests {\n    struct Before;\n    #[cfg(any())]\n    struct Disabled;\n\n\n    struct After;\n}\n",
    );

    let regular_targets = fixture.run(&["--format", "json"]);
    let all_targets = fixture.run(&["--format", "json", "--all-targets"]);

    assert!(
        item_spacing_diagnostics(&regular_targets).is_empty(),
        "{}",
        regular_targets.stdout
    );
    assert!(
        item_spacing_diagnostics(&all_targets).is_empty(),
        "{}",
        all_targets.stdout
    );
}

#[test]
fn requires_one_line_between_impl_blocks_and_every_checked_definition_kind() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "struct Target;\n\nimpl Target {}\nstruct StructAfterImpl;\nimpl Target {}\nenum EnumAfterImpl { Value }\nimpl Target {}\nunion UnionAfterImpl { value: u8 }\nimpl Target {}\ntype AliasAfterImpl = u8;\nimpl Target {}\ntrait TraitAfterImpl {}\nimpl Target {}\nconst CONSTANT: u8 = 0;\nimpl Target {}\nstatic STATIC: u8 = 0;\nimpl Target {}\nfn function_after_impl() {}\nimpl Target {}\nimpl Target {}\nstruct StructAfterImplPair;\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = item_spacing_diagnostics(&output);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(diagnostics.len(), 18, "{}", output.stdout);
    assert!(diagnostics.iter().all(|diagnostic| {
        spacing_values(diagnostic).0 == SpacingExpectation::Exact(1)
            && Some(spacing_values(diagnostic).1) == Some(0)
    }));
}

#[test]
fn checks_item_spacing_independently_in_modules_traits_and_impls() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "const ROOT_FIRST: u8 = 1;\nmod nested {\n    const FIRST: u8 = 1;\n    const SECOND: u8 = 2;\n    const THIRD: u8 = 3;\n\n    const FOURTH: u8 = 4;\n}\nconst ROOT_SECOND: u8 = 2;\n\ntrait Contract {\n    const FIRST: u8 = 1;\n    const SECOND: u8 = 2;\n\n    fn first();\n    fn second();\n}\n\nstruct Service;\n\nimpl Service {\n    const FIRST: u8 = 1;\n    const SECOND: u8 = 2;\n\n    fn first(&self) {}\n    fn second(&self) {}\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = item_spacing_diagnostics(&output);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(diagnostics.len(), 2, "{}", output.stdout);
    let expected_actual = diagnostics
        .iter()
        .map(|diagnostic| (spacing_values(diagnostic).0, spacing_values(diagnostic).1))
        .collect::<Vec<_>>();
    assert_eq!(
        expected_actual,
        [
            (SpacingExpectation::Exact(1), 0),
            (SpacingExpectation::Exact(1), 0)
        ]
    );
}

#[test]
fn counts_comment_lines_and_keeps_attributes_with_their_definitions() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "const FIRST: u8 = 1;\n// 行注释不是空行 / A line comment is not a blank line.\nconst SECOND: u8 = 2;\n\n/* 块注释属于源码 trivia / A block comment is source trivia. */\n#[doc = \"attached documentation\"]\nstatic FIRST_STATIC: u8 = 1;\n\n/* 上面的空行仍计数 / The blank line above still counts. */\nstatic SECOND_STATIC: u8 = 2;\n\ntype FirstAlias = u8;\n/// 文档注释附属于下一个别名 / Documentation belongs to the next alias.\ntype SecondAlias = u8;\n\nstruct First;\n/// 文档注释不能替代必需间距 / Documentation does not replace required spacing.\nstruct Second;\n\nstruct Third;\n\n/// 此属性仍附属于下一个定义 / This attribute stays with the next definition.\n#[derive(Clone)]\nstruct Fourth;\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = item_spacing_diagnostics(&output);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    let expected_actual = diagnostics
        .iter()
        .map(|diagnostic| (spacing_values(diagnostic).0, spacing_values(diagnostic).1))
        .collect::<Vec<_>>();
    assert_eq!(expected_actual, [(SpacingExpectation::Exact(1), 0)]);
}

#[test]
fn activates_renamed_optional_dependencies_only_when_requested() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"dep\"]\nresolver = \"2\"\n\n[dependencies]\nlocal_alias = { package = \"local-dep\", path = \"dep\", optional = true }\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg(feature = \"local_alias\")]\nmod enabled;\n",
    );
    fixture.write("src/enabled.rs", "pub fn enabled_only() {}\n");
    fixture.write(
        "dep/Cargo.toml",
        "[package]\nname = \"local-dep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    fixture.write("dep/src/lib.rs", "pub fn dependency() {}\n");

    let disabled = fixture.run(&["--format", "json", "--package", "app"]);
    let enabled = fixture.run(&[
        "--format",
        "json",
        "--package",
        "app",
        "--features",
        "app/local_alias",
    ]);
    assert_eq!(disabled.status, 0, "{}", disabled.stderr);
    assert!(!disabled.stdout.contains("enabled.rs"));
    assert_eq!(enabled.status, 1, "{}", enabled.stderr);
    assert!(enabled.stdout.contains("enabled.rs"));
    assert!(enabled.has_rule("child_public_free_functions"));
}

#[test]
fn resolves_path_attributes_relative_to_the_current_source_file() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "mod parent;\n");
    fixture.write("src/parent.rs", "#[path = \"a.rs\"] mod child;\n");
    fixture.write("src/a.rs", "fn source_file_sibling() {}\n");
    fixture.write("src/parent/a.rs", "fn module_directory_child() {}\n");

    let snapshot = analyze_fixture(&fixture);
    let child = snapshot
        .modules()
        .iter()
        .find(|module| module.namespace() == ["parent", "child"])
        .expect("path 属性指定的子模块");

    assert_eq!(child.file_path(), fixture.root.join("src/a.rs"));
}

#[test]
fn resolves_root_path_attributes_from_a_custom_root_file() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &format!(
            "{}\n[lib]\npath = \"src/custom_root.rs\"\n",
            package_manifest("sample")
        ),
    );
    fixture.write(
        "src/custom_root.rs",
        "#[path = \"selected.rs\"] mod child;\n",
    );
    fixture.write("src/selected.rs", "fn selected_from_root_directory() {}\n");
    fixture.write(
        "src/custom_root/selected.rs",
        "fn legacy_root_directory() {}\n",
    );

    let snapshot = analyze_fixture(&fixture);
    let child = snapshot
        .modules()
        .iter()
        .find(|module| module.namespace() == ["child"])
        .expect("自定义 crate root 的 path 模块");

    assert_eq!(child.file_path(), fixture.root.join("src/selected.rs"));
}

#[test]
fn resolves_cfg_attr_paths_from_the_source_file_directory() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nselected = []\n",
    );
    fixture.write("src/lib.rs", "#[path = \"outer.rs\"] mod outer;\n");
    fixture.write(
        "src/outer.rs",
        "#[cfg_attr(feature = \"selected\", path = \"children/selected.rs\" /* 忽略注释中的分隔符 / Delimiters in this comment are ignored, ) */)]\nmod logical;\n",
    );
    fixture.write("src/logical.rs", "pub fn default_path() {}\n");
    fixture.write("src/outer/logical.rs", "pub fn legacy_path() {}\n");
    fixture.write("src/children/selected.rs", "pub fn selected_path() {}\n");

    let default_path = fixture.run(&["--format", "json"]);
    let selected_path = fixture.run(&["--format", "json", "--features", "selected"]);
    assert_eq!(default_path.status, 1, "{}", default_path.stderr);
    assert!(default_path.stdout.contains("src/logical.rs"));
    assert!(!default_path.stdout.contains("src/outer/logical.rs"));
    assert!(default_path.stdout.contains("crate::outer::logical"));
    assert_eq!(
        selected_path.status, 1,
        "{} {}",
        selected_path.stderr, selected_path.stdout
    );
    assert!(selected_path.stdout.contains("src/children/selected.rs"));
    assert!(selected_path.stdout.contains("crate::outer::logical"));
    assert!(!selected_path.stdout.contains("missing_module"));
}

#[test]
fn resolves_inline_path_modules_from_the_declared_directory() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[path = \"thread_files\"] mod thread { #[path = \"tls.rs\"] mod local_data; mod child; }\n#[cfg_attr(all(), path = \"configured_files\")] mod configured { mod child; }\n",
    );
    fixture.write("src/thread_files/tls.rs", "fn selected_tls() {}\n");
    fixture.write("src/thread_files/child.rs", "fn selected_child() {}\n");
    fixture.write("src/thread/tls.rs", "fn legacy_tls() {}\n");
    fixture.write("src/thread/child.rs", "fn legacy_child() {}\n");
    fixture.write(
        "src/configured_files/child.rs",
        "fn selected_configured_child() {}\n",
    );
    fixture.write(
        "src/configured/child.rs",
        "fn legacy_configured_child() {}\n",
    );

    let snapshot = analyze_fixture(&fixture);
    let local_data = snapshot
        .modules()
        .iter()
        .find(|module| module.namespace() == ["thread", "local_data"])
        .expect("inline path 中的显式外部模块");
    let child = snapshot
        .modules()
        .iter()
        .find(|module| module.namespace() == ["thread", "child"])
        .expect("inline path 中的默认外部模块");
    let configured_child = snapshot
        .modules()
        .iter()
        .find(|module| module.namespace() == ["configured", "child"])
        .expect("cfg_attr inline path 中的默认外部模块");

    assert_eq!(
        local_data.file_path(),
        fixture.root.join("src/thread_files/tls.rs")
    );
    assert_eq!(
        child.file_path(),
        fixture.root.join("src/thread_files/child.rs")
    );
    assert_eq!(
        configured_child.file_path(),
        fixture.root.join("src/configured_files/child.rs")
    );
}

#[test]
fn resolves_inline_block_path_from_a_regular_source_file_module_directory() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "mod regular;\n");
    fixture.write(
        "src/regular.rs",
        "mod inner { #[path = \"selected.rs\"] mod leaf; }\n",
    );
    fixture.write(
        "src/regular/inner/selected.rs",
        "fn selected_from_inline_module_directory() {}\n",
    );
    fixture.write(
        "src/regular/selected.rs",
        "fn legacy_source_file_directory() {}\n",
    );
    fixture.write(
        "src/regular/inner/leaf/selected.rs",
        "fn legacy_leaf_module_directory() {}\n",
    );

    let snapshot = analyze_fixture(&fixture);
    let leaf = snapshot
        .modules()
        .iter()
        .find(|module| module.namespace() == ["regular", "inner", "leaf"])
        .expect("普通 .rs 文件中 inline 块内的显式 path 模块");

    assert_eq!(
        leaf.file_path(),
        fixture.root.join("src/regular/inner/selected.rs")
    );
}

#[test]
fn resolves_default_module_directories_from_each_module_source_kind() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "mod regular;\nmod grouped;\n#[path = \"named.rs\"] mod logical;\n#[path = \"same.rs\"] mod same;\n");
    fixture.write("src/regular.rs", "mod child;\nmod inline { mod nested; }\n");
    fixture.write("src/regular/child.rs", "fn regular_child() {}\n");
    fixture.write("src/regular/inline/nested.rs", "fn inline_child() {}\n");
    fixture.write("src/grouped/mod.rs", "mod child;\n");
    fixture.write("src/grouped/child.rs", "fn grouped_child() {}\n");
    fixture.write(
        "src/grouped/grouped/child.rs",
        "fn legacy_grouped_child() {}\n",
    );
    fixture.write("src/named.rs", "mod child;\n");
    fixture.write("src/same.rs", "mod child;\n");
    fixture.write("src/child.rs", "fn path_loaded_child() {}\n");
    fixture.write("src/logical/child.rs", "fn legacy_logical_child() {}\n");
    fixture.write("src/same/child.rs", "fn legacy_same_child() {}\n");

    let snapshot = analyze_fixture(&fixture);
    let expected = [
        ModulePathExpectation {
            parent: "regular",
            child: "child",
            path: "src/regular/child.rs",
        },
        ModulePathExpectation {
            parent: "regular",
            child: "inline::nested",
            path: "src/regular/inline/nested.rs",
        },
        ModulePathExpectation {
            parent: "grouped",
            child: "child",
            path: "src/grouped/child.rs",
        },
        ModulePathExpectation {
            parent: "logical",
            child: "child",
            path: "src/child.rs",
        },
        ModulePathExpectation {
            parent: "same",
            child: "child",
            path: "src/child.rs",
        },
    ];
    for expectation in expected {
        let namespace = format!("{}::{}", expectation.parent, expectation.child);
        let module = snapshot
            .modules()
            .iter()
            .find(|module| module.namespace().join("::") == namespace)
            .unwrap_or_else(|| panic!("模块 {namespace}"));
        assert_eq!(module.file_path(), fixture.root.join(expectation.path));
    }
}

#[test]
fn unknown_inline_path_does_not_guess_but_absolute_paths_restore_context() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let absolute_file = fixture.root.join("absolute/selected.rs");
    let absolute_file_literal = format!("{absolute_file:?}");
    fixture.write(
        "src/lib.rs",
        &format!(
            "#[cfg_attr(mystery_path, path = \"unknown_dir\")] mod dynamic {{ mod guessed; #[path = {absolute_file_literal}] mod recovered; }}\n"
        ),
    );
    fixture.write("src/unknown_dir/guessed.rs", "fn guessed_from_path() {}\n");
    fixture.write("src/dynamic/guessed.rs", "fn guessed_from_name() {}\n");
    fixture.write("absolute/selected.rs", "mod restored;\n");
    fixture.write(
        "absolute/restored.rs",
        "fn restored_from_absolute_path() {}\n",
    );

    let snapshot = analyze_fixture(&fixture);
    assert!(snapshot.status().is_partial());
    assert!(snapshot
        .modules()
        .iter()
        .any(|module| module.namespace() == ["dynamic"]));
    assert!(snapshot
        .modules()
        .iter()
        .any(|module| module.namespace() == ["dynamic", "recovered"]
            && module.file_path() == absolute_file));
    assert!(snapshot
        .modules()
        .iter()
        .any(|module| module.namespace() == ["dynamic", "recovered", "restored"]));
    assert!(!snapshot
        .modules()
        .iter()
        .any(|module| module.namespace() == ["dynamic", "guessed"]));
    assert!(!snapshot.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind(),
        DiagnosticKind::Analysis(AnalysisIssueKind::MissingModule)
    )));
    assert!(snapshot.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind(),
        DiagnosticKind::Analysis(AnalysisIssueKind::UnknownCfg)
    )));
}

#[test]
fn resolves_keyword_named_dependency_aliases_with_raw_identifiers() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"dep\"]\nresolver = \"2\"\n\n[dependencies]\ntype = { package = \"dep-package\", path = \"dep\" }\n",
    );
    fixture.write("src/lib.rs", "fn direct() { let _ = r#type::Thing; }\n");
    fixture.write(
        "dep/Cargo.toml",
        "[package]\nname = \"dep-package\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    fixture.write("dep/src/lib.rs", "pub struct Thing;\n");

    let direct = fixture.run(&["--format", "json", "--package", "app"]);
    fixture.write(
        "src/lib.rs",
        "use r#type::Thing;\nfn imported() { let _ = Thing; }\n",
    );
    let imported = fixture.run(&["--format", "json", "--package", "app"]);
    assert_eq!(direct.status, 1, "{}", direct.stderr);
    assert!(direct.has_rule("no_inline_qualified_path"));
    assert_eq!(imported.status, 0, "{}", imported.stderr);
    assert!(!imported.has_rule("no_inline_qualified_path"));
}

#[test]
fn classifies_trait_impl_locality_by_resolved_trait_and_target_ownership() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"vendor\"]\nresolver = \"2\"\n\n[dependencies]\nvendor = { path = \"vendor\" }\n",
    );
    fixture.write(
        "src/lib.rs",
        "pub trait Local {}\npub struct LocalType;\nimpl Local for LocalType {}\nimpl Clone for LocalType {}\nimpl std::fmt::Debug for LocalType {}\n\npub mod sibling {\n    pub trait Shared {}\n    pub struct Remote;\n}\n\npub mod child {\n    use crate::sibling::Shared as SharedAlias;\n    use vendor::External as ExternalTraitAlias;\n    pub struct LocalChild;\n    impl SharedAlias for LocalChild {}\n    impl crate::Local for LocalChild {}\n    impl ExternalTraitAlias for LocalChild {}\n}\n\nmod inline { pub struct InlineType; }\nimpl Local for inline::InlineType {}\nimpl Local for sibling::Remote {}\n\ntype ExternalVec = std::vec::Vec<u8>;\nimpl Local for ExternalVec {}\ntype LocalAlias = LocalType;\nimpl Local for LocalAlias {}\ntype RemoteAliasLayer = sibling::Remote;\ntype RemoteAlias = RemoteAliasLayer;\nimpl Local for RemoteAlias {}\n\nimpl vendor::External for LocalType {}\nimpl Local for vendor::Thing {}\nimpl missing::Trait for LocalType {}\nimpl Local for missing::Target {}\n\npub mod prelude_types {\n    pub trait Local {}\n    impl Local for Vec<u8> {}\n    impl Local for String {}\n    impl Local for u32 {}\n    impl Local for () {}\n    impl Local for [u8; 4] {}\n}\n\n#[cfg(feature = \"enabled\")]\npub trait Conditional {}\n#[cfg(not(feature = \"enabled\"))]\nimpl Conditional for LocalType {}\n#[cfg(feature = \"enabled\")]\npub struct GatedType;\n#[cfg(not(feature = \"enabled\"))]\nimpl Local for GatedType {}\n",
    );
    fixture.write(
        "vendor/Cargo.toml",
        "[package]\nname = \"vendor\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    fixture.write(
        "vendor/src/lib.rs",
        "pub trait External {}\npub struct Thing;\n",
    );

    let output = fixture.run(&["--format", "json", "--package", "app"]);
    assert_eq!(output.status, 1, "{}", output.stderr);
    let locality = rule_diagnostics(&output, "trait_impl_locality");
    assert_eq!(locality.len(), 3, "{}", output.stdout);
    assert!(rule_diagnostics(&output, "impl_locality").is_empty());
    assert_eq!(
        locality
            .iter()
            .filter(|diagnostic| {
                diagnostic_message(diagnostic).contains("outside the current module scope")
            })
            .count(),
        3
    );
    assert!(locality
        .iter()
        .all(|diagnostic| diagnostic_message(diagnostic).contains("target")));
    assert!(output
        .stdout
        .contains("Could not resolve the trait ownership"));
    assert!(output
        .stdout
        .contains("Could not resolve the type ownership"));
    assert!(output.stdout.contains("\"status\":\"partial\""));
    assert!(!output.stdout.contains("impl trait Conditional"));
    assert!(!output.stdout.contains("impl target GatedType"));
}

#[test]
fn preserves_standard_trait_identity_across_prelude_paths_and_aliases() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    fixture.write(
        "src/lib.rs",
        "use core::convert::From as CoreFrom;\nuse std::convert::From as StdFrom;\npub struct Source;\npub struct Target;\nimpl From<Source> for Target {}\nimpl core::convert::From<Source> for Target {}\nimpl std::convert::From<Source> for Target {}\nimpl CoreFrom<Source> for Target {}\nimpl StdFrom<Source> for Target {}\n",
    );
    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo 元数据");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), false, true))
            .with_current_directory(fixture.root.clone()),
    )
    .expect("选择 package 与 target");
    let package = project.packages().first().expect("选中的 package");
    let snapshot = ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("分析 target");
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate 根模块");
    let paths = [
        vec!["From".to_owned()],
        vec!["core".to_owned(), "convert".to_owned(), "From".to_owned()],
        vec!["std".to_owned(), "convert".to_owned(), "From".to_owned()],
        vec!["CoreFrom".to_owned()],
        vec!["StdFrom".to_owned()],
    ];
    let resolutions = paths
        .iter()
        .map(|path| snapshot.symbols().resolve_path(module, path))
        .collect::<Vec<_>>();
    let identities = resolutions
        .iter()
        .map(|resolution| {
            resolution
                .external_item_identity()
                .expect("catalog identity")
        })
        .collect::<Vec<_>>();

    assert!(identities
        .iter()
        .all(|identity| identity.canonical_path() == "core::convert::From"));
    assert!(identities
        .iter()
        .all(|identity| identity.kind() == ExternalItemKind::Trait));
    assert_eq!(
        resolutions[0].external_item_source(),
        Some(ExternalItemSource::ImplicitPrelude)
    );
    assert_eq!(
        resolutions[1].external_item_source(),
        Some(ExternalItemSource::ExplicitPath)
    );
    assert_eq!(
        resolutions[2].external_item_source(),
        Some(ExternalItemSource::ExplicitPath)
    );
    assert_eq!(
        resolutions[3].external_item_source(),
        Some(ExternalItemSource::UseAlias)
    );
    assert_eq!(
        resolutions[4].external_item_source(),
        Some(ExternalItemSource::UseAlias)
    );
}

#[test]
fn resolves_debug_and_hash_from_the_implicit_prelude() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "pub struct Target;\n\nimpl Debug for Target {}\n\nimpl Hash for Target {}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let report = from_str::<Value>(&output.stdout).expect("JSON 报告");
    let diagnostics = report["diagnostics"].as_array().expect("诊断数组");

    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["analysis_issue"] != "parser_coverage"),
        "{}",
        output.stdout
    );
}

#[test]
fn warns_when_known_standard_non_traits_appear_in_impl_trait_positions() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use core::option::Option as Maybe;\npub struct Target;\nimpl Option for Target {}\nimpl Some for Target {}\nimpl std::option::Option for Target {}\nimpl Maybe for Target {}\npub mod local {\n    pub trait Option {}\n    pub struct Target;\n    impl Option for Target {}\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = from_str::<Value>(&output.stdout).expect("JSON 报告")["diagnostics"]
        .as_array()
        .expect("诊断数组")
        .iter()
        .filter(|diagnostic| {
            diagnostic["analysis_issue"] == "parser_coverage"
                && message_contains_either(
                    diagnostic,
                    "Could not resolve the trait ownership",
                    "无法解析 impl trait",
                )
        })
        .cloned()
        .collect::<Vec<_>>();

    assert_eq!(diagnostics.len(), 4, "{}", output.stdout);
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["start"]["line"] == 3));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["start"]["line"] == 4));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["start"]["line"] == 5));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["start"]["line"] == 6));
}

#[test]
fn applies_no_implicit_prelude_to_modules_and_descendants_only() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "pub struct Root;\nimpl Clone for Root {}\nuse core::clone::Clone;\n#[no_implicit_prelude]\nmod blocked {\n    pub struct Target;\n    impl Clone for Target {}\n    impl core::clone::Clone for Target {}\n    impl std::fmt::Debug for Target {}\n    impl super::Clone for Target {}\n    impl Debug for Target {}\n    mod nested {\n        pub struct Target;\n        impl Clone for Target {}\n        impl Debug for Target {}\n    }\n}\nmod sibling {\n    pub struct Target;\n    impl Clone for Target {}\n}\n#[no_implicit_prelude]\nmod explicit {\n    extern crate core;\n    use core::clone::Clone;\n    pub struct Target;\n    impl Clone for Target {}\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = from_str::<Value>(&output.stdout).expect("JSON 报告")["diagnostics"]
        .as_array()
        .expect("诊断数组")
        .iter()
        .filter(|diagnostic| {
            diagnostic["analysis_issue"] == "parser_coverage"
                && message_contains_either(
                    diagnostic,
                    "Could not resolve the trait ownership",
                    "无法解析 impl trait",
                )
        })
        .cloned()
        .collect::<Vec<_>>();

    assert_eq!(diagnostics.len(), 4, "{}", output.stdout);
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["start"]["line"] == 8));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["start"]["line"] == 11));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["start"]["line"] == 9));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic["start"]["line"] == 15));
}

#[test]
fn applies_cfg_attr_no_std_and_edition_specific_preludes() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[features]\nno_std_mode = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#![cfg_attr(feature = \"no_std_mode\", no_std)]\npub struct Target;\nimpl From<Target> for Target {}\nimpl Future for Target {}\nimpl Vec for Target {}\n",
    );
    let discovery =
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(fixture.root.join("Cargo.toml")))
            .expect("Cargo 元数据");
    let project = TargetSelector::select(
        discovery,
        CargoSelection::new(FeatureSelection::new(Vec::new(), true, false))
            .with_current_directory(fixture.root.clone()),
    )
    .expect("选择 package 与 target");
    let package = project.packages().first().expect("选中的 package");
    let snapshot = ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("分析 target");
    let module = snapshot
        .modules()
        .iter()
        .find(|module| module.is_crate_root())
        .expect("crate 根模块");
    let symbols = snapshot.symbols();
    let from = symbols.resolve_path(module, &["From".to_owned()]);
    let future = symbols.resolve_path(module, &["Future".to_owned()]);
    let vector = symbols.resolve_path(module, &["Vec".to_owned()]);

    assert_eq!(
        from.external_item_identity()
            .expect("core prelude From")
            .canonical_path(),
        "core::convert::From"
    );
    assert_eq!(
        future
            .external_item_identity()
            .expect("core 2024 prelude Future")
            .canonical_path(),
        "core::future::Future"
    );
    assert_eq!(vector.kind(), PathResolutionKind::Unresolved);

    for edition in ["2015", "2018", "2021", "2024"] {
        let edition_fixture = Fixture::new();
        edition_fixture.write(
            "Cargo.toml",
            &format!(
                "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n"
            ),
        );
        edition_fixture.write("src/lib.rs", "pub struct Target;\n");
        let discovery = CargoProjectLoader::discover(CargoDiscoveryRequest::new(
            edition_fixture.root.join("Cargo.toml"),
        ))
        .expect("Cargo 元数据");
        let project = TargetSelector::select(
            discovery,
            CargoSelection::new(FeatureSelection::new(Vec::new(), false, true))
                .with_current_directory(edition_fixture.root.clone()),
        )
        .expect("选择 package 与 target");
        let package = project.packages().first().expect("选中的 package");
        let snapshot =
            ProjectAnalyzer::analyze(AnalysisRequest::new(package)).expect("分析 target");
        let module = snapshot
            .modules()
            .iter()
            .find(|module| module.is_crate_root())
            .expect("crate 根模块");
        let future = snapshot
            .symbols()
            .resolve_path(module, &["Future".to_owned()]);
        let from_iterator = snapshot
            .symbols()
            .resolve_path(module, &["FromIterator".to_owned()]);
        let from = snapshot
            .symbols()
            .resolve_path(module, &["From".to_owned()]);
        let debug = snapshot
            .symbols()
            .resolve_path(module, &["Debug".to_owned()]);
        let hash = snapshot
            .symbols()
            .resolve_path(module, &["Hash".to_owned()]);
        let double_ended_iterator = snapshot
            .symbols()
            .resolve_path(module, &["DoubleEndedIterator".to_owned()]);

        assert_eq!(
            future.external_item_identity().is_some(),
            edition == "2024",
            "edition {edition}"
        );
        assert_eq!(
            from_iterator.external_item_identity().is_some(),
            matches!(edition, "2021" | "2024"),
            "edition {edition}"
        );
        assert!(from.external_item_identity().is_some(), "edition {edition}");
        assert_eq!(
            debug
                .external_item_identity()
                .expect("Debug prelude trait")
                .canonical_path(),
            "core::fmt::Debug",
            "edition {edition}"
        );
        assert_eq!(
            hash.external_item_identity()
                .expect("Hash prelude trait")
                .canonical_path(),
            "core::hash::Hash",
            "edition {edition}"
        );
        assert!(
            double_ended_iterator.external_item_identity().is_some(),
            "edition {edition}"
        );
    }
}

#[test]
fn checks_trait_impl_locality_by_repository_target_ownership() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "pub trait Local {}\npub mod child {\n    pub struct Target;\n    impl crate::Local for Target {}\n}\nimpl Local for child::Target {}\npub mod other {\n    pub struct Other;\n}\nimpl std::fmt::Debug for child::Target {}\nimpl std::fmt::Debug for other::Other {}\nimpl std::fmt::Debug for String {}\nimpl std::fmt::Debug for u8 {}\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    let locality = rule_diagnostics(&output, "trait_impl_locality");
    assert_eq!(locality.len(), 5, "{}", output.stdout);
    assert!(locality.iter().any(|diagnostic| {
        diagnostic_message(diagnostic).contains("Target")
            && diagnostic_message(diagnostic).contains("outside the current module scope")
    }));
    assert!(locality.iter().any(|diagnostic| {
        diagnostic_message(diagnostic).contains("Other")
            && diagnostic_message(diagnostic).contains("outside the current module scope")
    }));
    assert!(locality.iter().any(|diagnostic| {
        diagnostic_message(diagnostic).contains("String")
            && diagnostic_message(diagnostic).contains("outside the current module scope")
    }));
    assert!(locality
        .iter()
        .any(|diagnostic| diagnostic["start"]["line"] == 6 && diagnostic["start"]["column"] == 16));
    assert!(
        locality
            .iter()
            .any(|diagnostic| diagnostic["start"]["line"] == 10
                && diagnostic["start"]["column"] == 26)
    );
    assert!(
        locality
            .iter()
            .any(|diagnostic| diagnostic["start"]["line"] == 11
                && diagnostic["start"]["column"] == 26)
    );
    assert!(
        locality
            .iter()
            .any(|diagnostic| diagnostic["start"]["line"] == 12
                && diagnostic["start"]["column"] == 26)
    );
}

#[test]
fn allows_only_private_inline_sealed_trait_scope_exceptions() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = r#"mod sealed { pub trait Sealed {} }
use sealed::Sealed as SealedAlias;
impl SealedAlias for String {}
mod remote_types { pub struct RemoteTarget; }
impl sealed::Sealed for remote_types::RemoteTarget {}
mod unrelated_impl {
    impl crate::sealed::Sealed for String {}
}
pub mod public_container { pub trait Sealed {} }
impl public_container::Sealed for String {}
pub(crate) mod restricted_container { pub trait Sealed {} }
impl restricted_container::Sealed for String {}
mod non_inline;
impl non_inline::Sealed for String {}
mod outer { pub mod nested { pub trait Sealed {} } }
impl outer::nested::Sealed for String {}
mod restricted_trait { pub(crate) trait Sealed {} }
impl restricted_trait::Sealed for String {}
mod other_trait { pub trait NotSealed {} }
impl other_trait::NotSealed for String {}
"#;
    fixture.write("src/lib.rs", source);
    fixture.write("src/non_inline.rs", "pub trait Sealed {}\n");
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.child_module_line_count]\nenabled = false\n\n[rules.imports_at_scope_start]\nenabled = false\n\n[rules.item_spacing]\nenabled = false\n\n[rules.no_inline_qualified_path]\nenabled = false\n\n[rules.out_of_line_modules_at_scope_start]\nenabled = false\n\n[rules.unnecessary_pub_crate]\nenabled = false\n",
    );

    let output = fixture.run(&["--format", "json"]);

    let locality = rule_diagnostics(&output, "trait_impl_locality");
    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(locality.len(), 8, "{}", output.stdout);
    let target_diagnostic = locality
        .iter()
        .find(|diagnostic| diagnostic["start"]["line"] == 5)
        .expect("sealed trait 不应隐藏本地 target 越界诊断");
    let target_path = "remote_types::RemoteTarget";
    assert!(diagnostic_message(target_diagnostic).contains("RemoteTarget"));
    assert!(diagnostic_message(target_diagnostic).contains("outside the current module scope"));
    assert_eq!(
        target_diagnostic["start"]["column"],
        source.lines().nth(4).unwrap().find(target_path).unwrap() + 1
    );
    for (line, path) in [
        (7, "crate::sealed::Sealed"),
        (10, "public_container::Sealed"),
        (12, "restricted_container::Sealed"),
        (14, "non_inline::Sealed"),
        (16, "outer::nested::Sealed"),
        (18, "restricted_trait::Sealed"),
        (20, "other_trait::NotSealed"),
    ] {
        let diagnostic = locality
            .iter()
            .find(|diagnostic| diagnostic["start"]["line"] == line)
            .unwrap_or_else(|| {
                panic!(
                    "missing locality diagnostic at line {line}: {}",
                    output.stdout
                )
            });
        let trait_name = path.rsplit("::").next().unwrap();
        assert!(diagnostic_message(diagnostic).contains("trait definition"));
        assert!(diagnostic_message(diagnostic).contains(trait_name));
        assert!(diagnostic_message(diagnostic).contains("outside the current module scope"));
        assert_eq!(
            diagnostic["start"]["column"],
            source.lines().nth(line - 1).unwrap().find(path).unwrap() + 1
        );
    }
    assert!(!locality
        .iter()
        .any(|diagnostic| diagnostic["start"]["line"] == 3));
}

struct TraitScopeCase {
    line: usize,
    column: usize,
    trait_name: &'static str,
    target_name: &'static str,
}

#[test]
fn reports_trait_scope_mismatch_for_standard_library_targets() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#![no_std]\nextern crate alloc;\npub mod conversion {\n    pub trait EnvironmentScalar {}\n    pub trait EnvironmentSourcedValue {}\n\n    mod pipeline {\n        use alloc::string::String;\n\n        impl<T> crate::conversion::EnvironmentSourcedValue for Option<T> {}\n        impl crate::conversion::EnvironmentScalar for String {}\n        impl crate::conversion::EnvironmentScalar for bool {}\n        impl crate::conversion::EnvironmentScalar for f32 {}\n        impl crate::conversion::EnvironmentScalar for f64 {}\n    }\n}\n",
    );
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.item_spacing]\nenabled = false\n\n[rules.no_inline_qualified_path]\nenabled = false\n",
    );

    let output = fixture.run(&["--format", "json"]);

    let locality = rule_diagnostics(&output, "trait_impl_locality");
    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(locality.len(), 5, "{}", output.stdout);
    for case in [
        TraitScopeCase {
            line: 10,
            column: 17,
            trait_name: "EnvironmentSourcedValue",
            target_name: "Option<T>",
        },
        TraitScopeCase {
            line: 11,
            column: 14,
            trait_name: "EnvironmentScalar",
            target_name: "String",
        },
        TraitScopeCase {
            line: 12,
            column: 14,
            trait_name: "EnvironmentScalar",
            target_name: "bool",
        },
        TraitScopeCase {
            line: 13,
            column: 14,
            trait_name: "EnvironmentScalar",
            target_name: "f32",
        },
        TraitScopeCase {
            line: 14,
            column: 14,
            trait_name: "EnvironmentScalar",
            target_name: "f64",
        },
    ] {
        let diagnostic = locality
            .iter()
            .find(|diagnostic| diagnostic["start"]["line"] == case.line)
            .unwrap_or_else(|| {
                panic!(
                    "missing diagnostic on line {}: {}",
                    case.line, output.stdout
                )
            });
        assert!(
            diagnostic_message(diagnostic).contains(case.trait_name)
                && diagnostic_message(diagnostic).contains("trait definition")
                && diagnostic_message(diagnostic).contains(case.target_name),
            "expected the out-of-scope trait {} for target {}: {}",
            case.trait_name,
            case.target_name,
            output.stdout
        );
        assert_eq!(
            diagnostic["start"]["column"], case.column,
            "{}",
            output.stdout
        );
    }
}

#[test]
fn accepts_external_trait_impl_with_local_generic_argument() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"veloq_std\"]\nresolver = \"2\"\n\n[dependencies]\nveloq_std = { path = \"veloq_std\" }\n",
    );
    fixture.write(
        "src/lib.rs",
        "use veloq_std::result::Result as StdResult;\n\npub enum Outcome<T, E> {\n    Ok(T),\n    Err(E),\n}\n\nimpl<T, E> From<Outcome<T, E>> for StdResult<T, E> {\n    fn from(outcome: Outcome<T, E>) -> Self {\n        match outcome {\n            Outcome::Ok(value) => Self::Ok(value),\n            Outcome::Err(error) => Self::Err(error),\n        }\n    }\n}\n",
    );
    fixture.write(
        "veloq_std/Cargo.toml",
        "[package]\nname = \"veloq_std\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    fixture.write(
        "veloq_std/src/lib.rs",
        "pub mod result {\n    pub type Result<T, E> = core::result::Result<T, E>;\n}\n",
    );

    let output = fixture.run(&["--format", "json", "--package", "app"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(rule_diagnostics(&output, "trait_impl_locality").is_empty());
}

#[test]
fn resolves_trait_impl_anchors_through_orphan_order_and_generic_aliases() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"remote\"]\nresolver = \"2\"\n\n[dependencies]\nremote = { path = \"remote\" }\n",
    );
    fixture.write("src/lib.rs", "mod implementations;\nmod types;\n");
    fixture.write(
        "src/types.rs",
        "pub struct Local<T>(T);\npub struct Marker;\npub type Identity<T> = T;\npub type Layer<T> = Identity<T>;\npub type Reordered<T, U> = Layer<Local<U>>;\npub type Unqualified<T> = Local<T>;\npub type Defaulted<T = Local<u8>> = Identity<T>;\npub type Unresolved<T> = Missing<T>;\npub type Cycle<T> = Cycle<T>;\npub type Pair<T> = (Local<T>, T);\npub type Generic<'a, const N: usize, T> = Local<(&'a [u8; N], T)>;\npub type AliasSelf<T> = Local<T>;\n",
    );
    let implementations = "use crate::types::{AliasSelf, Cycle, Defaulted, Generic, Identity, Layer, Local, Pair, Reordered, Unqualified, Unresolved};\nuse core::pin::Pin;\nuse remote::{Foreign, HasAssoc, NonType};\n\nimpl Foreign<Local<u8>> for Vec<u8> {}\nimpl Foreign<Vec<Local<u8>>> for Vec<u8> {}\nimpl Foreign<Box<Local<u8>>> for Vec<u8> {}\nimpl Foreign<&mut Local<u8>> for Vec<u8> {}\nimpl Foreign<()> for Box<Local<u8>> {}\nimpl Foreign<()> for Vec<Local<u8>> {}\nimpl<T> Foreign<Local<T>> for Vec<T> {}\nimpl<T> Foreign<Local<T>> for Box<T> {}\nimpl Foreign<Identity<Local<u8>>> for Vec<u8> {}\nimpl Foreign<Layer<Local<u8>>> for Vec<u8> {}\nimpl Foreign<Reordered<u8, Local<u16>>> for Vec<u8> {}\nimpl Foreign<Unqualified<Local<u8>>> for Vec<u8> {}\nimpl Foreign<Defaulted> for Vec<u8> {}\nimpl Foreign<Pin<Local<u8>>> for Vec<u8> {}\nimpl Foreign<Local<u8>> for Pin<Local<u8>> {}\nimpl<T> Foreign<Vec<T>, Local<T>> for Vec<T> {}\nimpl<T> Foreign<Box<T>, Local<T>> for Vec<T> {}\nimpl<T> Foreign<T, Local> for Vec<T> {}\nimpl Foreign<Unresolved<Local<u8>>> for Vec<u8> {}\nimpl Foreign<Identity<u8, Local<u8>>> for Vec<u8> {}\nimpl Foreign<Cycle<u8>> for Vec<u8> {}\nimpl NonType<'static, 3, Local<u8>> for Vec<u8> {}\nimpl NonType<'static, 3, Vec<Local<u8>>> for Vec<u8> {}\nimpl HasAssoc<Item = Local<u8>> for Vec<u8> {}\nimpl Foreign<(Local<u8>,)> for Vec<u8> {}\nimpl Foreign<Generic<'static, 3, u8>> for Vec<u8> {}\nimpl Foreign<Pair<u8>> for Vec<u8> {}\nimpl Foreign<()> for AliasSelf<u8> {}\nimpl Foreign<&Local<u8>> for Vec<u8> {}\n";
    fixture.write("src/implementations.rs", implementations);
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.child_module_line_count]\nenabled = false\n\n[rules.item_spacing]\nenabled = false\n",
    );
    fixture.write(
        "remote/Cargo.toml",
        "[package]\nname = \"remote\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    fixture.write(
        "remote/src/lib.rs",
        "pub trait Foreign<T, U = ()> {}\npub trait NonType<'a, const N: usize, T> {}\npub trait HasAssoc { type Item; }\n",
    );

    let output = fixture.run(&["--format", "json", "--package", "app"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    let locality = rule_diagnostics(&output, "trait_impl_locality");
    assert_eq!(locality.len(), 26, "{}", output.stdout);
    for diagnostic in &locality {
        let message = diagnostic_message(diagnostic);
        assert!(
            message.contains("outside the current module scope"),
            "{message}"
        );
    }
    for line in [
        5, 7, 8, 9, 11, 13, 14, 15, 16, 17, 18, 19, 20, 26, 30, 32, 33,
    ] {
        assert!(
            locality
                .iter()
                .any(|diagnostic| diagnostic["path"] == "src/implementations.rs"
                    && diagnostic["start"]["line"] == line
                    && diagnostic_message(diagnostic).contains("Local")),
            "expected a type anchor at line {line}: {}",
            output.stdout
        );
    }
    let direct_anchor = locality
        .iter()
        .find(|diagnostic| diagnostic["start"]["line"] == 5)
        .expect("直接本地类型锚点");
    let direct_anchor_column = implementations
        .lines()
        .nth(4)
        .and_then(|line| line.find("Local"))
        .expect("impl 中的 Local")
        + 1;
    assert_eq!(
        direct_anchor["start"]["column"], direct_anchor_column,
        "{}",
        output.stdout
    );
    for (line, target) in [
        (6, "Vec"),
        (10, "Vec"),
        (12, "Box"),
        (21, "Vec"),
        (22, "Vec"),
        (27, "Vec"),
        (28, "Vec"),
        (29, "Vec"),
        (31, "Vec"),
    ] {
        assert!(
            locality
                .iter()
                .any(|diagnostic| diagnostic["path"] == "src/implementations.rs"
                    && diagnostic["start"]["line"] == line
                    && diagnostic_message(diagnostic).contains(target)),
            "expected a {target} fallback at line {line}: {}",
            output.stdout
        );
    }
    let report = from_str::<Value>(&output.stdout).expect("JSON 诊断");
    let unresolved = report["diagnostics"]
        .as_array()
        .expect("诊断数组")
        .iter()
        .filter(|diagnostic| {
            diagnostic["kind"] == "analysis"
                && diagnostic["path"] == "src/implementations.rs"
                && [23, 24, 25].contains(&diagnostic["start"]["line"].as_u64().unwrap_or(0))
        })
        .count();
    assert_eq!(unresolved, 3, "{}", output.stdout);
}

#[test]
fn skips_structural_targets_and_uses_local_trait_fallback_for_other_targets() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"vendor\"]\nresolver = \"2\"\n\n[dependencies]\nvendor = { path = \"vendor\" }\n",
    );
    fixture.write(
        "src/lib.rs",
        "type ExternalLayer = std::vec::Vec<u8>;\ntype ExternalAlias = ExternalLayer;\ntype PrimitiveLayer = u32;\ntype PrimitiveAlias = PrimitiveLayer;\npub mod traits {\n    pub trait Local {}\n    impl Local for vendor::Thing {}\n    impl Local for Vec<u8> {}\n    impl Local for String {}\n    impl Local for u32 {}\n    impl Local for &u8 {}\n    impl Local for *const u8 {}\n    impl Local for (u8, u8) {}\n    impl Local for [u8] {}\n    impl Local for [u8; 4] {}\n    impl Local for fn() {}\n    impl Local for ! {}\n}\npub mod elsewhere {\n    impl crate::traits::Local for vendor::Thing {}\n    impl crate::traits::Local for Vec<u8> {}\n    impl crate::traits::Local for String {}\n    impl crate::traits::Local for u32 {}\n    impl crate::traits::Local for &u8 {}\n    impl crate::traits::Local for *const u8 {}\n    impl crate::traits::Local for (u8, u8) {}\n    impl crate::traits::Local for [u8] {}\n    impl crate::traits::Local for [u8; 4] {}\n    impl crate::traits::Local for fn() {}\n    impl crate::traits::Local for ! {}\n    impl crate::traits::Local for crate::ExternalAlias {}\n    impl crate::traits::Local for crate::PrimitiveAlias {}\n}\n",
    );
    fixture.write(
        "vendor/Cargo.toml",
        "[package]\nname = \"vendor\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    fixture.write("vendor/src/lib.rs", "pub struct Thing;\n");

    let output = fixture.run(&["--format", "json", "--package", "app"]);

    assert_eq!(output.status, 1, "{}", output.stderr);
    let locality = rule_diagnostics(&output, "trait_impl_locality");
    assert_eq!(locality.len(), 6, "{}", output.stdout);
    for target in [
        "vendor::Thing",
        "Vec<u8>",
        "String",
        "u32",
        "crate::ExternalAlias",
        "crate::PrimitiveAlias",
    ] {
        assert!(locality
            .iter()
            .any(|diagnostic| diagnostic_message(diagnostic).contains(target)));
    }
    for target in [
        "&u8",
        "*const u8",
        "(u8, u8)",
        "[u8]",
        "[u8; 4]",
        "fn()",
        "!",
    ] {
        assert!(!locality
            .iter()
            .any(|diagnostic| diagnostic_message(diagnostic).contains(target)));
    }
}

#[test]
fn configures_inherent_and_trait_impl_locality_independently() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod child;\nmod types;\n\npub trait Local {}\n\npub struct RootType;\n\nimpl types::Remote {}\n",
    );
    fixture.write(
        "src/child.rs",
        "use crate::Local;\npub struct Child;\n\nimpl Local for Child {}\n\nimpl Local for crate::RootType {}\n",
    );
    fixture.write("src/types.rs", "pub struct Remote;\n");
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.child_module_line_count]\nenabled = false\n\n[rules.no_inline_qualified_path]\nenabled = false\n",
    );

    let default = fixture.run(&["--format", "json"]);
    let inherent_disabled = fixture.run(&["--format", "json", "--disable-rule", "impl_locality"]);
    let trait_disabled =
        fixture.run(&["--format", "json", "--disable-rule", "trait_impl_locality"]);
    let inherent_warning =
        fixture.run(&["--format", "json", "--severity", "impl_locality=warning"]);
    let trait_warning = fixture.run(&[
        "--format",
        "json",
        "--severity",
        "trait_impl_locality=warning",
    ]);
    fixture.write(
        "cargo-cop.toml",
        "version = 4\n\n[rules.child_module_line_count]\nenabled = false\n\n[rules.no_inline_qualified_path]\nenabled = false\n\n[rules.impl_locality]\nenabled = false\n\n[rules.trait_impl_locality]\nseverity = \"warning\"\n",
    );
    let configured = fixture.run(&["--format", "json"]);

    assert_eq!(default.status, 1, "{}", default.stdout);
    assert_eq!(rule_diagnostics(&default, "impl_locality").len(), 1);
    assert_eq!(rule_diagnostics(&default, "trait_impl_locality").len(), 1);

    assert_eq!(inherent_disabled.status, 1, "{}", inherent_disabled.stdout);
    assert!(rule_diagnostics(&inherent_disabled, "impl_locality").is_empty());
    assert_eq!(
        rule_diagnostics(&inherent_disabled, "trait_impl_locality").len(),
        1
    );
    assert_eq!(trait_disabled.status, 1, "{}", trait_disabled.stdout);
    assert_eq!(rule_diagnostics(&trait_disabled, "impl_locality").len(), 1);
    assert!(rule_diagnostics(&trait_disabled, "trait_impl_locality").is_empty());

    assert_eq!(inherent_warning.status, 1, "{}", inherent_warning.stdout);
    assert_eq!(
        rule_diagnostics(&inherent_warning, "impl_locality")[0]["severity"],
        "warning"
    );
    assert_eq!(
        rule_diagnostics(&inherent_warning, "trait_impl_locality")[0]["severity"],
        "error"
    );
    assert_eq!(trait_warning.status, 1, "{}", trait_warning.stdout);
    assert_eq!(
        rule_diagnostics(&trait_warning, "impl_locality")[0]["severity"],
        "error"
    );
    assert_eq!(
        rule_diagnostics(&trait_warning, "trait_impl_locality")[0]["severity"],
        "warning"
    );
    assert_eq!(configured.status, 0, "{}", configured.stdout);
    assert!(rule_diagnostics(&configured, "impl_locality").is_empty());
    assert_eq!(
        rule_diagnostics(&configured, "trait_impl_locality")[0]["severity"],
        "warning"
    );
}

#[test]
fn resolves_local_types_named_like_prelude_types_before_the_prelude() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "trait Local {}\nstruct Vec<T>(T);\nstruct String;\nimpl Local for Vec<u8> {}\nimpl Local for String {}\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert!(
        rule_diagnostics(&output, "impl_locality").is_empty(),
        "{}",
        output.stdout
    );
    assert!(rule_diagnostics(&output, "trait_impl_locality").is_empty());
}

#[test]
fn isolates_test_cfg_contexts_and_deduplicates_shared_diagnostics() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[allow(dead_code)]\npub struct Value;\nimpl Value {}\n#[cfg(test)]\nmod tests {\n    struct TestValue;\n    impl TestValue {}\n}\n",
    );

    let output = fixture.run(&["--format", "json", "--all-targets"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"no_lint_relaxation\"")
            .count(),
        1,
        "{}",
        output.stdout
    );
    assert!(!output.stdout.contains("\"kind\":\"analysis\""));
    assert!(output.stdout.contains("\"status\":\"complete\""));
}

#[test]
fn keeps_same_named_binary_and_library_targets_separate() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "struct Shared;\n\nimpl Shared {}\n");
    fixture.write(
        "src/main.rs",
        "struct Shared;\n\nimpl Shared {}\n\nfn main() {}\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(output.stdout.contains("\"status\":\"complete\""));
    assert!(output.stdout.contains("\"diagnostics\":0"));
}

#[test]
fn marks_rule_phase_resolution_failures_as_partial() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "trait Local {}\n\nimpl Local for missing::Type {}\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(output.stdout.contains("\"status\":\"partial\""));
    assert!(output
        .stdout
        .contains("\"analysis_issue\":\"parser_coverage\""));
}

#[test]
fn counts_lines_inside_multiline_raw_string_tokens() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "mod threshold;\n");
    let mut source = (0..40)
        .map(|index| format!("fn item_{index}() {{}}\n"))
        .collect::<String>();
    source.push_str("\nconst DATA: &str = r#\"\n");
    for index in 0..29 {
        source.push_str(&format!("payload {index}\n"));
    }
    source.push_str("\"#;\n");
    fixture.write("src/threshold.rs", &source);

    let output = fixture.run(&["--format", "json", "--disable-rule", "item_spacing"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(!output.has_rule("child_module_line_count"));
}

#[test]
fn resolves_dependencies_by_build_script_target_kind() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\nbuild = \"build.rs\"\n\n[workspace]\nmembers = [\"build-dep\", \"runtime-dep\"]\nresolver = \"2\"\n\n[dependencies]\nruntime_dep = { path = \"runtime-dep\" }\n\n[build-dependencies]\nbuild_dep = { path = \"build-dep\" }\n",
    );
    fixture.write("src/lib.rs", "pub fn ready() {}\n");
    fixture.write(
        "build.rs",
        "fn main() { let _build = build_dep::Thing; let _runtime = runtime_dep::Thing; }\n",
    );
    fixture.write("build-dep/Cargo.toml", &package_manifest("build-dep"));
    fixture.write("build-dep/src/lib.rs", "pub struct Thing;\n");
    fixture.write("runtime-dep/Cargo.toml", &package_manifest("runtime-dep"));
    fixture.write("runtime-dep/src/lib.rs", "pub struct Thing;\n");

    let output = fixture.run(&["--format", "json", "--package", "app", "--all-targets"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"no_inline_qualified_path\"")
            .count(),
        1,
        "{}",
        output.stdout
    );
    assert!(output.stdout.contains("build_dep::Thing"));
    assert!(!output.stdout.contains("runtime_dep::Thing"));
}

#[test]
fn checks_spacing_inside_trait_and_impl_scopes() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "trait Contract {\n    fn first();\n    fn second();\n}\n\nstruct Service;\n\nimpl Service {\n    fn first(&self) {}\n    fn second(&self) {}\n}\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"item_spacing\"")
            .count(),
        2,
        "{}",
        output.stdout
    );
}

#[test]
fn checks_public_fields_after_where_clauses() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "struct Data<T>\nwhere\n    T: Copy,\n{\n    pub value: T,\n}\nimpl<T> Data<T>\nwhere\n    T: Copy,\n{}\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert!(output.has_rule("private_fields_for_behavior_types"));
}

#[test]
fn separates_cfg_import_groups_from_unconditional_imports() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nenabled = []\nother = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg(feature = \"enabled\")]\nuse std::fmt::Debug;\nuse std::fmt::Display;\n\n#[cfg(all(feature = \"other\"))]\nuse std::fmt::Write;\n",
    );

    let missing_separator = fixture.run(&["--format", "json", "--features", "enabled"]);
    assert_eq!(missing_separator.status, 1, "{}", missing_separator.stdout);
    assert_eq!(
        missing_separator
            .stdout
            .matches("\"rule_id\":\"use_group_spacing\"")
            .count(),
        1,
        "{}",
        missing_separator.stdout
    );

    fixture.write(
        "src/lib.rs",
        "#[cfg(feature = \"enabled\")]\nuse std::fmt::Debug;\n\nuse std::fmt::Display;\n\n#[cfg(all(feature = \"other\"))]\nuse std::fmt::Write;\n",
    );
    let separated = fixture.run(&["--format", "json", "--features", "enabled,other"]);
    assert_eq!(separated.status, 0, "{}", separated.stdout);
    assert!(!separated.has_rule("use_group_spacing"));
}

#[test]
fn treats_explicit_all_cfg_as_the_unconditional_use_group() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use crate::Unconditional;\n#[cfg(all())]\nuse crate::ExplicitAll;\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert!(
        rule_diagnostics(&output, "use_group_spacing").is_empty(),
        "{}",
        output.stdout
    );
}

#[test]
fn ignores_comment_lines_when_counting_group_spacing() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nalpha = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg(feature = \"alpha\")]\nuse crate::Conditional;\n// This comment does not count as a blank line.\n\nuse crate::Unconditional;\n",
    );

    let output = fixture.run(&["--format", "json", "--features", "alpha"]);

    assert!(
        rule_diagnostics(&output, "use_group_spacing").is_empty(),
        "{}",
        output.stdout
    );
}

#[test]
fn skips_spacing_checks_across_target_disabled_import_groups() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[cfg(any(target_os = \"linux\", target_os = \"android\"))]\nuse std::fmt::Debug;\n\n#[cfg(target_os = \"windows\")]\nuse std::fmt::Display;\n\nuse std::fmt::Write;\n",
    );

    let output = fixture.run(&["--format", "json", "--target", "x86_64-unknown-linux-gnu"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(!output.has_rule("use_group_spacing"));
}

#[test]
fn skips_block_spacing_checks_across_target_disabled_import_groups() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn body() {\n    #[cfg(any(target_os = \"linux\", target_os = \"android\"))]\n    use std::fmt::Debug;\n\n    #[cfg(target_os = \"windows\")]\n    use std::fmt::Display;\n\n    use std::fmt::Write;\n}\n",
    );

    let output = fixture.run(&["--format", "json", "--target", "x86_64-unknown-linux-gnu"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(!output.has_rule("use_group_spacing"));
}

#[test]
fn checks_block_cfg_import_spacing_without_disabled_boundaries() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    for (separator, expected_count) in [("", 1), ("\n", 0), ("\n\n", 1)] {
        fixture.write(
            "src/lib.rs",
            &format!(
                "fn body() {{\n    #[cfg(target_os = \"linux\")]\n    use std::fmt::Debug;{separator}\n    use std::fmt::Display;\n}}\n"
            ),
        );

        let output = fixture.run(&["--format", "json", "--target", "x86_64-unknown-linux-gnu"]);
        let diagnostics = rule_diagnostics(&output, "use_group_spacing");

        assert_eq!(diagnostics.len(), expected_count, "{}", output.stdout);
        if separator == "\n\n" {
            assert!(diagnostic_message(&diagnostics[0]).contains("found 2 blank lines"));
        }
    }
}

#[test]
fn orders_module_use_groups_by_source_visibility_form() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "pub use public::A;\n\npub(crate) use restricted::B;\npub(in crate::inner) use restricted::C;\npub(in super) use restricted::D;\n\npub(super) use parent::E;\n\npub(self) use private::F;\nuse private::G;\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert!(output.status != 2, "{}", output.stdout);
    assert!(
        rule_diagnostics(&output, "use_group_spacing").is_empty(),
        "{}",
        output.stdout
    );
}

#[test]
fn classifies_spaced_and_commented_super_and_self_visibility_groups() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod spaced_super { pub( super ) use a::A; pub(crate) use b::B; }\nmod commented_super { pub( /* comment */ super ) use a::A; pub(crate) use b::B; }\nmod spaced_self { pub( self ) use a::A; pub(crate) use b::B; }\nmod commented_self { pub( /* comment */ self ) use a::A; pub(crate) use b::B; }\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "use_group_spacing");
    let order_errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic_message(diagnostic).contains("appears out of order"))
        .count();
    let spacing_errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic_message(diagnostic).contains("blank lines"))
        .count();

    assert!(output.status != 2, "{}", output.stdout);
    assert_eq!(order_errors, 4, "{}", output.stdout);
    assert_eq!(spacing_errors, 4, "{}", output.stdout);
}

#[test]
fn reports_visibility_inversions_and_repeated_categories_once_per_group() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use private::First;\n\npub use public::Item;\n\npub(crate) use restricted::Item;\n\npub(super) use parent::Item;\n\nuse private::Second;\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "use_group_spacing")
        .into_iter()
        .filter(|diagnostic| diagnostic_message(diagnostic).contains("appears out of order"))
        .collect::<Vec<_>>();

    assert_eq!(diagnostics.len(), 2, "{}", output.stdout);
    assert_eq!(diagnostics[0]["start"]["line"], 3);
    assert_eq!(diagnostics[1]["start"]["line"], 9);
}

#[test]
fn non_use_nodes_break_module_use_group_runs() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "use private::Item;\nfn boundary() {}\npub use public::Item;\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert!(
        rule_diagnostics(&output, "use_group_spacing").is_empty(),
        "{}",
        output.stdout
    );
}

#[test]
fn checks_cfg_groups_in_const_and_static_initializer_blocks() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "const VALUE: () = {\n    #[cfg(target_os = \"linux\")]\n    use crate::LinuxConst;\n    use crate::AlwaysConst;\n};\n\nstatic OTHER: () = {\n    #[cfg(target_os = \"linux\")]\n    use crate::LinuxStatic;\n    use crate::AlwaysStatic;\n};\n",
    );

    let output = fixture.run(&["--format", "json", "--target", "x86_64-unknown-linux-gnu"]);
    let diagnostics = rule_diagnostics(&output, "use_group_spacing");

    assert_eq!(diagnostics.len(), 2, "{}", output.stdout);
    assert_eq!(diagnostics[0]["start"]["line"], 4);
    assert_eq!(diagnostics[1]["start"]["line"], 10);
}

#[test]
fn checks_late_block_uses_independently_of_import_position_results() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "fn body() {\n    let value = ();\n    #[cfg(unix)]\n    use crate::Unix;\n    use crate::Always;\n}\n",
    );

    let output = fixture.run(&["--format", "json", "--target", "x86_64-unknown-linux-gnu"]);

    assert_eq!(
        rule_diagnostics(&output, "use_group_spacing").len(),
        1,
        "{}",
        output.stdout
    );
    assert!(output.has_rule("imports_at_scope_start"));
}

#[test]
fn reports_repeated_available_cfg_import_groups() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nalpha = []\nbeta = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg(feature = \"alpha\")]\nuse std::fmt::Debug;\n\n#[cfg(feature = \"beta\")]\nuse std::fmt::Display;\n\n#[cfg(feature = \"alpha\")]\nuse std::fmt::Write;\n\nfn body() {\n    #[cfg(feature = \"alpha\")]\n    use std::path::Path;\n\n    #[cfg(feature = \"beta\")]\n    use std::path::PathBuf;\n\n    #[cfg(feature = \"alpha\")]\n    use std::path::Component;\n}\n",
    );

    let output = fixture.run(&["--format", "json", "--features", "alpha,beta"]);
    let split_diagnostics = rule_diagnostics(&output, "use_group_spacing")
        .into_iter()
        .filter(|diagnostic| {
            diagnostic_message(diagnostic).contains("is split by another use group")
        })
        .count();

    assert_eq!(split_diagnostics, 2, "{}", output.stdout);
}

#[test]
fn unknown_cfg_conditions_participate_in_use_grouping() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nalpha = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg(custom_flag = \"x\")]\nuse crate::First;\n\n#[cfg(feature = \"alpha\")]\nuse crate::Middle;\n\n#[cfg(custom_flag = \"x\")]\nuse crate::Last;\n",
    );

    let output = fixture.run(&["--format", "json", "--features", "alpha"]);
    let splits = rule_diagnostics(&output, "use_group_spacing")
        .into_iter()
        .filter(|diagnostic| {
            diagnostic_message(diagnostic).contains("is split by another use group")
        })
        .count();

    assert_eq!(splits, 1, "{}", output.stdout);
    assert!(output.stdout.contains("\"analysis_issue\":\"unknown_cfg\""));
}

#[test]
fn ignores_target_disabled_imports_when_checking_cfg_group_splits() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "#[cfg(target_os = \"linux\")]\nuse std::fmt::Debug;\n\n#[cfg(target_os = \"windows\")]\nuse std::fmt::Display;\n\n#[cfg(target_os = \"linux\")]\nuse core::cmp::Eq;\n\nfn body() {\n    #[cfg(target_os = \"linux\")]\n    use std::path::Path;\n\n    #[cfg(target_os = \"windows\")]\n    use std::path::PathBuf;\n\n    #[cfg(target_os = \"linux\")]\n    use core::marker::Copy;\n}\n",
    );

    let output = fixture.run(&["--format", "json", "--target", "x86_64-unknown-linux-gnu"]);

    assert_eq!(output.status, 0, "{}", output.stdout);
    assert!(!output.has_rule("use_group_spacing"));
}

#[test]
fn reports_missing_item_names_as_incomplete_syntax() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "impl {}\nstruct {}\ntrait {}\nfn () {}\n");

    let output = fixture.run(&["--format", "json", "--locale", "zh-CN"]);

    assert_eq!(output.status, 2, "{}", output.stdout);
    assert!(output.stdout.contains("\"status\":\"incomplete\""));
    assert!(output
        .stdout
        .contains("\"analysis_issue\":\"parser_coverage\""));
    assert!(has_diagnostic_message_fragment(&output, "需要名称"));
}

#[test]
fn resolves_all_features_with_optional_dependency_activation() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"dep\"]\nresolver = \"2\"\n\n[features]\ndefault = [\"base\"]\nbase = []\ninclude_dep = [\"dep:dep\", \"dep/extra\"]\n\n[dependencies]\ndep = { path = \"dep\", optional = true, default-features = false }\n",
    );
    fixture.write(
        "src/lib.rs",
        "#[cfg(feature = \"base\")]\nmod baseline;\n#[cfg(feature = \"include_dep\")]\nfn direct_dependency_path() { let _ = dep::Thing; }\n",
    );
    fixture.write("src/baseline.rs", "pub fn enabled_by_default() {}\n");
    fixture.write(
        "dep/Cargo.toml",
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nextra = []\n",
    );
    fixture.write("dep/src/lib.rs", "pub struct Thing;\n");

    let without_defaults = fixture.run(&[
        "--format",
        "json",
        "--package",
        "app",
        "--no-default-features",
    ]);
    let all_features = fixture.run(&[
        "--format",
        "json",
        "--package",
        "app",
        "--all-features",
        "--no-default-features",
    ]);

    assert_eq!(without_defaults.status, 0, "{}", without_defaults.stdout);
    assert!(!without_defaults.stdout.contains("baseline.rs"));
    assert!(!without_defaults.has_rule("no_inline_qualified_path"));
    assert_eq!(all_features.status, 1, "{}", all_features.stdout);
    assert!(all_features.stdout.contains("baseline.rs"));
    assert!(all_features.has_rule("no_inline_qualified_path"));
}

#[test]
fn reports_only_unnecessary_pub_crate_declarations() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = "mod outer {\n    pub(crate) fn unused() {}\n    pub(crate) fn used_inside() {}\n    pub(crate) fn used_outside() {}\n    pub(crate) struct ApiType;\n    pub(crate) fn exposed(_: ApiType) {}\n    pub(crate) struct Owner;\n    impl Owner {\n        pub(crate) fn hidden_method() {}\n        pub(crate) fn child_method() {}\n        pub(crate) fn visible_method() {}\n    }\n    mod child { fn internal() { super::used_inside(); super::Owner::child_method(); } }\n}\nmod sibling {\n    fn call() {\n        crate::outer::used_outside();\n        let _ = crate::outer::exposed;\n        crate::outer::Owner::visible_method();\n    }\n}\nmod types { pub struct Public; }\nmod alias_scope { pub(crate) use crate::types::Public as Alias; }\nmod alias_sibling { fn parameter(_: crate::alias_scope::Alias) {} }\nmod local_alias {\n    pub(crate) use crate::types::Public as LocalAlias;\n    fn parameter(_: LocalAlias) {}\n}\n";
    fixture.write("src/lib.rs", source);

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "unnecessary_pub_crate");
    let names = diagnostics.iter().map(diagnostic_name).collect::<Vec<_>>();

    assert_eq!(
        names,
        [
            "unused",
            "used_inside",
            "hidden_method",
            "child_method",
            "LocalAlias"
        ],
        "{}\n{}",
        output.stdout,
        output.stderr
    );
    for diagnostic in diagnostics {
        let line = usize::try_from(diagnostic["start"]["line"].as_u64().expect("诊断起始行"))
            .expect("诊断起始行范围");
        let start = usize::try_from(diagnostic["start"]["column"].as_u64().expect("诊断起始列"))
            .expect("诊断起始列范围");
        let end = usize::try_from(diagnostic["end"]["column"].as_u64().expect("诊断结束列"))
            .expect("诊断结束列范围");
        let source_line = source.lines().nth(line - 1).expect("诊断对应源码行");
        assert_eq!(&source_line[start - 1..end - 1], "pub(crate)");
    }
    assert!(output.stdout.contains("\"status\":\"complete\""));
}

#[test]
fn keeps_candidate_visible_through_used_api_despite_unrelated_self() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &package_manifest("sample").replace("2021", "2024"),
    );
    fixture.write("src/lib.rs", "mod put;\n");
    fixture.write(
        "src/put.rs",
        include_str!("fixtures/put_candidate_caller.rs"),
    );
    fixture.write(
        "src/put/candidates.rs",
        include_str!("fixtures/candidates.rs"),
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "unnecessary_pub_crate");
    let reported_candidates = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic_name(diagnostic) == "Candidate")
        .collect::<Vec<_>>();

    assert!(
        reported_candidates.is_empty(),
        "Candidate was reported as unnecessary:\n{}\n{}",
        output.stdout,
        output.stderr
    );
    let reported_api = diagnostics
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic_name(diagnostic),
                "CandidateSelection" | "candidates"
            )
        })
        .collect::<Vec<_>>();

    assert!(
        reported_api.is_empty(),
        "The used API was not recognized:\n{}\n{}",
        output.stdout,
        output.stderr
    );
    assert!(output.stdout.contains("\"status\":\"complete\""));
}

#[test]
fn tracks_crate_visible_apis_in_control_flow_and_method_calls() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        r#"mod api {
    pub(crate) struct Runner;
    pub(crate) struct Payload;

    impl Runner {
        pub(crate) fn load(&self) -> Payload { Payload }
        pub(crate) fn create() -> Payload { Payload }
    }
}
mod client {
    use crate::api::Runner;

    fn run(runner: &Runner) {
        if true { let _ = runner.load(); }
        for _ in 0..1 { let _ = Runner::create(); }
    }
}
mod implementation_only {
    pub(crate) struct Helper;

    impl Helper {
        pub(crate) fn call(&self) {}
    }

    fn use_local(helper: &Helper) { helper.call(); }
}
"#,
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "unnecessary_pub_crate");
    let mut names = diagnostics.iter().map(diagnostic_name).collect::<Vec<_>>();
    names.sort_unstable();

    assert_eq!(names, ["Helper", "call"], "{}", output.stdout);
    assert!(output.stdout.contains("\"status\":\"complete\""));
}

#[test]
fn reports_pub_crate_constant_despite_cfg_test_macro_coverage() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &format!(
            "{}\n[[test]]\nname = \"process_regression\"\npath = \"tests/fixture_root.rs\"\n",
            package_manifest("sample")
        ),
    );
    fixture.write(
        "tests/fixture_root.rs",
        "#[path = \"fixtures/process_pub_crate_regression.rs\"]\nmod process;\npub struct LinuxSyscalls;\n",
    );
    fixture.write(
        "tests/fixtures/process_pub_crate_regression.rs",
        include_str!("fixtures/process_pub_crate_regression.rs"),
    );

    let output = fixture.run(&["--format", "json", "--all-targets"]);
    let diagnostics = rule_diagnostics(&output, "unnecessary_pub_crate");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic_name(diagnostic) == "MAX_PROCESS_POINTERS" }),
        "{}\n{}",
        output.stdout,
        output.stderr
    );
}

#[test]
fn configures_unnecessary_pub_crate_through_config_and_cli() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "mod inner { pub(crate) fn unused() {} }\n");

    let default = fixture.run(&["--format", "json"]);
    let warning = fixture.run(&[
        "--format",
        "json",
        "--severity",
        "unnecessary_pub_crate=warning",
    ]);
    let disabled = fixture.run(&[
        "--format",
        "json",
        "--disable-rule",
        "unnecessary_pub_crate",
    ]);

    assert_eq!(rule_diagnostics(&default, "unnecessary_pub_crate").len(), 1);
    assert_eq!(
        rule_diagnostics(&warning, "unnecessary_pub_crate")[0]["severity"],
        "warning"
    );
    assert!(rule_diagnostics(&disabled, "unnecessary_pub_crate").is_empty());
}

#[test]
fn accounts_for_cfg_and_unknown_conditions_when_checking_pub_crate() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nenabled = []\n",
    );
    fixture.write(
        "src/lib.rs",
        "mod api {\n    #[cfg(feature = \"enabled\")]\n    pub(crate) fn inactive_outside_use() {}\n    #[cfg(mystery_cfg)]\n    pub(crate) fn uncertain() {}\n}\n#[cfg(not(feature = \"enabled\"))]\nmod inactive_user {\n    fn call() { crate::api::inactive_outside_use(); }\n}\n#[cfg(mystery_cfg)]\nmod uncertain_user {\n    fn call() { crate::api::uncertain(); }\n}\n",
    );

    let output = fixture.run(&["--format", "json", "--features", "enabled"]);
    let diagnostics = rule_diagnostics(&output, "unnecessary_pub_crate");
    let names = diagnostics.iter().map(diagnostic_name).collect::<Vec<_>>();

    assert_eq!(names, ["inactive_outside_use"], "{}", output.stdout);
}

#[test]
fn does_not_propagate_api_dependencies_across_incompatible_cfgs() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod api { #[cfg(custom_cfg)] pub(crate) fn expose() -> crate::types::Hidden { panic!() } }\nmod types { #[cfg(not(custom_cfg))] pub(crate) struct Hidden; }\n#[cfg(custom_cfg)] mod client { fn call() { let _ = crate::api::expose(); } }\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "unnecessary_pub_crate");
    let names = diagnostics.iter().map(diagnostic_name).collect::<Vec<_>>();

    assert_eq!(names, ["Hidden"], "{}\n{}", output.stdout, output.stderr);
    assert!(output.stdout.contains("\"status\":\"partial\""));
}

#[test]
fn skips_pub_crate_candidates_when_macro_expansion_is_unavailable() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "mod api { pub(crate) fn used_by_macro() {} }\nmacro_rules! call_api { () => { crate::api::used_by_macro() }; }\nfn invoke() { call_api!(); }\n",
    );

    let output = fixture.run(&["--format", "json"]);

    assert!(
        rule_diagnostics(&output, "unnecessary_pub_crate").is_empty(),
        "{}",
        output.stdout
    );
}

#[test]
fn isolates_pub_crate_references_between_target_contexts() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"sample-bin\"\npath = \"src/main.rs\"\n",
    );
    fixture.write("src/lib.rs", "mod inner { pub(crate) fn candidate() {} }\n");
    fixture.write(
        "src/main.rs",
        "mod inner { pub(crate) fn candidate() {} }\nmod outside { fn call() { crate::inner::candidate(); } }\nfn main() {}\n",
    );

    let output = fixture.run(&["--format", "json", "--all-targets"]);
    let diagnostics = rule_diagnostics(&output, "unnecessary_pub_crate");

    assert_eq!(diagnostics.len(), 1, "{}", output.stdout);
    assert_eq!(diagnostics[0]["target"], "sample");
}

#[test]
fn respects_restricted_field_visibility_when_propagating_pub_crate() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = "mod api {\n    pub(crate) struct Container {\n        pub(in crate::api) local: LocalType,\n        pub exposed: crate::types::ExposedType,\n        private: PrivateType,\n    }\n    pub(crate) struct LocalType;\n    pub(crate) struct PrivateType;\n    pub(crate) struct BodyOnly;\n    pub(crate) fn retained() { let _ = crate::api::BodyOnly; }\n}\nmod types { pub(crate) struct ExposedType; }\nmod consumer { fn use_api(_: crate::api::Container) {} fn call() { crate::api::retained(); } }\n";
    fixture.write("src/lib.rs", source);

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "unnecessary_pub_crate");
    let names = diagnostics.iter().map(diagnostic_name).collect::<Vec<_>>();

    assert_eq!(
        names,
        ["LocalType", "PrivateType", "BodyOnly"],
        "{}",
        output.stdout
    );
    for diagnostic in diagnostics {
        let line = usize::try_from(diagnostic["start"]["line"].as_u64().expect("诊断起始行"))
            .expect("诊断起始行范围");
        let start = usize::try_from(diagnostic["start"]["column"].as_u64().expect("诊断起始列"))
            .expect("诊断起始列范围");
        let end = usize::try_from(diagnostic["end"]["column"].as_u64().expect("诊断结束列"))
            .expect("诊断结束列范围");
        let source_line = source.lines().nth(line - 1).expect("诊断对应源码行");
        assert_eq!(&source_line[start - 1..end - 1], "pub(crate)");
    }
    assert!(output.stdout.contains("\"status\":\"complete\""));
}

#[test]
fn propagates_pub_crate_through_signature_enum_and_trait_api_types() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        r#"mod api {
    pub(crate) fn build() -> Option<crate::types::Alias> { None }
    pub(crate) fn constrained<T: crate::types::Bound>() {}
    pub(crate) fn where_bound<T>() where T: crate::types::WhereBound {}
    pub(crate) enum State { Tuple(crate::types::Payload), Named { value: crate::types::Payload } }
    pub(crate) union Shared { pub payload: crate::types::UnionPayload }
    pub(crate) trait Contract: crate::types::Super {
        type Item: crate::types::Payload;
        fn value(&self) -> Option<Self::Item>;
        const DEFAULT: crate::types::Payload;
    }
    pub(crate) struct Carrier;
    impl Carrier {
        pub(crate) fn load() -> crate::types::MethodResult { panic!() }
    }
}
mod types {
    pub(crate) struct Deep;
    pub(crate) type Alias = Deep;
    pub(crate) struct Payload;
    pub(crate) struct UnionPayload;
    pub(crate) struct MethodResult;
    pub(crate) trait Bound {}
    pub(crate) trait WhereBound {}
    pub(crate) trait Super {}
}
mod client {
    fn call() {
        let _ = crate::api::build();
        crate::api::constrained::<()>();
        crate::api::where_bound::<()>();
        let _ = crate::api::Carrier::load();
    }
    fn state(_: crate::api::State) {}
    fn shared(_: crate::api::Shared) {}
    fn contract(_: &dyn crate::api::Contract) {}
}
"#,
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "unnecessary_pub_crate");

    assert!(
        diagnostics.is_empty(),
        "{}\n{}",
        output.stdout,
        output.stderr
    );
    assert!(output.stdout.contains("\"status\":\"complete\""));
}

#[test]
fn propagates_pub_crate_through_supported_foreign_item_types() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        r#"mod ffi {
    extern "C" {
        pub(crate) fn send(value: crate::types::ForeignInput) -> crate::types::ForeignOutput;
        pub(crate) static VALUE: crate::types::ForeignStatic;
    }
}
mod types {
    pub(crate) struct ForeignInput;
    pub(crate) struct ForeignOutput;
    pub(crate) struct ForeignStatic;
}
mod client {
    fn call() {
        let _ = crate::ffi::send;
        let _ = crate::ffi::VALUE;
    }
}
"#,
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "unnecessary_pub_crate");

    assert!(
        diagnostics.is_empty(),
        "{}\n{}",
        output.stdout,
        output.stderr
    );
    assert!(output.stdout.contains("\"status\":\"complete\""));
}

#[test]
fn propagates_pub_crate_dependency_chains_and_ignores_unretained_apis() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = "mod graph {\n    pub(crate) struct A(pub(crate) B);\n    pub(crate) struct B(pub(crate) C);\n    pub(crate) struct C(pub(crate) A);\n    pub(crate) struct D(pub(crate) E);\n    pub(crate) struct E;\n}\nmod client { fn root(_: crate::graph::A) {} }\nmod orphan_api { pub(crate) fn unused(_: crate::orphan_types::NeverUsed) {} }\nmod orphan_types { pub(crate) struct NeverUsed; }\n";
    fixture.write("src/lib.rs", source);

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "unnecessary_pub_crate");
    let names = diagnostics.iter().map(diagnostic_name).collect::<Vec<_>>();

    assert_eq!(
        names,
        ["D", "E", "unused", "NeverUsed"],
        "{}\n{}",
        output.stdout,
        output.stderr
    );
    for diagnostic in diagnostics {
        let line = usize::try_from(diagnostic["start"]["line"].as_u64().expect("诊断起始行"))
            .expect("诊断起始行范围");
        let start = usize::try_from(diagnostic["start"]["column"].as_u64().expect("诊断起始列"))
            .expect("诊断起始列范围");
        let end = usize::try_from(diagnostic["end"]["column"].as_u64().expect("诊断结束列"))
            .expect("诊断结束列范围");
        let source_line = source.lines().nth(line - 1).expect("诊断对应源码行");
        assert_eq!(&source_line[start - 1..end - 1], "pub(crate)");
    }
    assert!(output.stdout.contains("\"status\":\"complete\""));
}

#[test]
fn tracks_api_dependencies_by_definition_identity_with_raw_identifiers() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    let source = "mod source_types { pub(crate) struct Shared; }\nmod shadow_types { pub(crate) struct Shared; }\nmod raw_types { pub(crate) struct r#type; }\nmod api { pub(crate) fn input(_: crate::source_types::Shared) {} pub(crate) fn raw(_: crate::raw_types::r#type) {} }\nmod client { fn call() { crate::api::input(loop {}); crate::api::raw(loop {}); } }\n";
    fixture.write("src/lib.rs", source);

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = rule_diagnostics(&output, "unnecessary_pub_crate");
    let names = diagnostics.iter().map(diagnostic_name).collect::<Vec<_>>();

    assert_eq!(names, ["Shared"], "{}\n{}", output.stdout, output.stderr);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic["path"], "src/lib.rs");
    assert_eq!(diagnostic["start"]["line"], 2);
    let line = source.lines().nth(1).expect("同名但无依赖的声明行");
    let start = usize::try_from(diagnostic["start"]["column"].as_u64().expect("诊断起始列"))
        .expect("诊断起始列范围");
    let end = usize::try_from(diagnostic["end"]["column"].as_u64().expect("诊断结束列"))
        .expect("诊断结束列范围");
    assert_eq!(&line[start - 1..end - 1], "pub(crate)");
    assert!(output.stdout.contains("\"status\":\"complete\""));
}
