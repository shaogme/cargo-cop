use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{id, Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::{from_str, Value};

use cop_analysis::{
    AnalysisRequest, CfgDecision, ConfiguredAstIssueKind, PathResolutionKind, ProjectAnalyzer,
};
use cop_cargo::{
    CargoDiscoveryRequest, CargoProjectLoader, CargoSelection, FeatureSelection, TargetSelector,
};

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

fn write_package(root: &Path, name: &str, source: &str) {
    fs::create_dir_all(root.join(name).join("src")).expect("创建 package 源码目录");
    fs::write(root.join(name).join("Cargo.toml"), package_manifest(name))
        .expect("写入 package manifest");
    fs::write(root.join(name).join("src/lib.rs"), source).expect("写入 package 根源码");
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
    assert!(direct.stdout.contains("\"schema_version\":3"));
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
        "version = 2\n\n[rules.no_lint_relaxation]\nseverity = \"warning\"\n",
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
        "version = 2\nallow_warnings = true\n\n[rules.no_lint_relaxation]\nseverity = \"warning\"\n\n[rules.item_spacing]\nseverity = \"warning\"\n",
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
        "pub struct First;\npub struct Second;\n",
    );
    fixture.write(
        "cargo-cop.toml",
        "version = 2\n\n[i18n]\nlocale = \"en-US\"\n",
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
    let invalid_arguments = fixture.run(&["--format", "json", "--locale", "en-US", "--unknown"]);
    let english = from_str::<Value>(&configured.stdout).expect("英文 JSON 报告");
    let chinese = from_str::<Value>(&overridden.stdout).expect("中文 JSON 报告");
    let argument_error = from_str::<Value>(&invalid_arguments.stdout).expect("参数错误 JSON 报告");
    let english_diagnostic = &english["diagnostics"][0];
    let chinese_diagnostic = &chinese["diagnostics"][0];

    assert!(help.stdout.contains("Usage: cargo-cop"));
    assert_eq!(configured.status, 1, "{}", configured.stderr);
    assert_eq!(overridden.status, configured.status);
    assert_eq!(english["locale"], "en-US");
    assert_eq!(chinese["locale"], "zh-CN");
    assert_eq!(english["status"], chinese["status"]);
    assert_eq!(english["summary"], chinese["summary"]);
    assert_eq!(english_diagnostic["message_id"], "item-spacing");
    assert_eq!(
        english_diagnostic["message_id"],
        chinese_diagnostic["message_id"]
    );
    assert_eq!(
        english_diagnostic["arguments"],
        chinese_diagnostic["arguments"]
    );
    assert_ne!(english_diagnostic["message"], chinese_diagnostic["message"]);
    assert_eq!(invalid_arguments.status, 2);
    assert_eq!(argument_error["locale"], "en-US");
    assert_eq!(
        argument_error["diagnostics"][0]["message_id"],
        "cli-parse-error"
    );
}

#[test]
fn resolves_relative_explicit_config_without_merging_discovered_config() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write("src/lib.rs", "#[allow(dead_code)]\nfn ready() {}\n");
    fixture.write(
        "cargo-cop.toml",
        "version = 2\n\n[rules.no_lint_relaxation]\nenabled = false\n",
    );
    fixture.write(
        "config/alternate.toml",
        "version = 2\n\n[rules.no_lint_relaxation]\nseverity = \"warning\"\n",
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
    let snapshot = ProjectAnalyzer::analyze(AnalysisRequest::new(package));
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
    assert_eq!(nested.decision(), CfgDecision::False);
    let mut parent = nested.parent();
    while let Some(parent_id) = parent {
        let ancestor = configured.node(parent_id).expect("父节点配置记录");
        if ancestor.parent().is_none() {
            break;
        }
        assert_eq!(ancestor.decision(), CfgDecision::False);
        parent = ancestor.parent();
    }

    let uncertain = configured
        .node(node_at("fn uncertain").id())
        .expect("unknown 节点");
    assert_eq!(uncertain.decision(), CfgDecision::Unknown);
    assert!(uncertain
        .effective_condition()
        .is_some_and(|condition| condition.contains("mystery_function")));
    let hidden_local = configured
        .node(node_at("fn hidden_local").id())
        .expect("false local item");
    assert_eq!(hidden_local.decision(), CfgDecision::False);

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
        .all(|node| node.decision() == CfgDecision::True));
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
    let snapshot = ProjectAnalyzer::analyze(AnalysisRequest::new(package));
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
            configured_node.decision(),
            expectation.decision,
            "{}",
            expectation.marker
        );
        if expectation.decision != CfgDecision::True {
            assert!(configured_node.effective_condition().is_some());
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
        .find(|diagnostic| diagnostic["message_id"] == "malformed-cfg-attribute")
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
        .find(|diagnostic| diagnostic["message_id"] == "malformed-cfg-attribute")
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
        .filter(|diagnostic| diagnostic["message_id"] == "malformed-cfg-attribute")
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
    let snapshot = ProjectAnalyzer::analyze(AnalysisRequest::new(package));
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
    let snapshot = ProjectAnalyzer::analyze(AnalysisRequest::new(package));
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

    assert_eq!(matching.status, 1, "{}", matching.stdout);
    assert!(matching.has_rule("no_inline_qualified_path"));
    assert_eq!(other.status, 0, "{}", other.stdout);
    assert!(!other.has_rule("no_inline_qualified_path"));
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
    fixture.write("src/a.rs", "#[path = \"../a.rs\"] mod nested;\n");
    fs::create_dir_all(fixture.root.join("src/a")).expect("创建循环模块的相对目录");

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
        chinese_diagnostics[0]["message_id"],
        "no-parent-path-attribute"
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
        invalid
            .stdout
            .contains("词法分析遇到无效或无法识别的 token"),
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
        "version = 2\n\n[scan]\ntarget = \"not-a-rust-target\"\n",
    );
    let invalid = fixture.run(&["--format", "json"]);
    let overridden = fixture.run(&["--format", "json", "--target", "host"]);
    assert_eq!(invalid.status, 2, "{}", invalid.stderr);
    assert!(invalid.stdout.contains("\"kind\":\"tool\""));
    assert!(invalid.stdout.contains("\"tool_error\":\"cargo_metadata\""));
    assert_eq!(overridden.status, 0, "{}", overridden.stderr);
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
        "version = 2\n\n[rules.private_child_modules]\nenabled = false\n",
    );

    let old_config = fixture.run(&["--format", "json"]);
    fixture.write(
        "cargo-cop.toml",
        "version = 2\n\n[rules.child_module_visibility]\nseverity = \"warning\"\n",
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
    assert!(output.stdout.contains("\"actual\":69"));
    assert!(output.stdout.contains("\"actual\":801"));
    assert!(!output.stdout.contains("\"actual\":70"));
    assert!(!output.stdout.contains("\"actual\":800"));
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
    assert!(output.has_rule("cfg_use_group_spacing"));
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
    assert_eq!(diagnostics[0]["arguments"]["line"], 1);
    assert_eq!(
        diagnostics[0]["arguments"]["prefix"],
        "crate::model::nested"
    );
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
    assert_eq!(diagnostics[0]["arguments"]["line"], 1);
    assert_eq!(diagnostics[0]["arguments"]["prefix"], "crate::model");
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
    assert_eq!(diagnostics[0]["arguments"]["line"], 3);
    assert_eq!(diagnostics[0]["arguments"]["prefix"], "crate::model");
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
    assert_eq!(diagnostics[0]["arguments"]["line"], 1);
    assert_eq!(diagnostics[0]["arguments"]["prefix"], "crate::parser");
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
    assert_eq!(diagnostics[0]["arguments"]["line"], 1);
    assert_eq!(diagnostics[1]["start"]["line"], 6);
    assert_eq!(diagnostics[1]["arguments"]["line"], 3);
    assert_eq!(diagnostics[1]["arguments"]["prefix"], "crate::model");
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
    assert_eq!(diagnostics[0]["arguments"]["line"], 2);
    assert_eq!(diagnostics[0]["arguments"]["prefix"], "crate::alpha::left");
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
    assert_eq!(diagnostics[0]["arguments"]["line"], 2);
    assert_eq!(diagnostics[0]["arguments"]["prefix"], "crate::model");
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
    assert_eq!(diagnostics[0]["arguments"]["line"], 2);
    assert_eq!(diagnostics[0]["arguments"]["prefix"], "crate::model");
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
        "cfg_use_group_spacing",
    ]);
    let english = fixture.run(&[
        "--format",
        "json",
        "--locale",
        "en-US",
        "--disable-rule",
        "child_module_line_count",
        "--disable-rule",
        "cfg_use_group_spacing",
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
        "version = 2\n\n[rules.extern_crate_at_scope_start]\nenabled = false\nseverity = \"warning\"\n",
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
        "version = 2\n\n[rules.extern_crate_at_scope_start]\nenabled = true\nseverity = \"warning\"\n",
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
        "version = 2\n\n[rules.out_of_line_modules_at_scope_start]\nseverity = \"warning\"\n",
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
    assert_eq!(diagnostics[0]["arguments"]["line"], 1);
    assert_eq!(diagnostics[0]["arguments"]["prefix"], "crate::module");
    assert!(!output.has_rule("cfg_use_group_spacing"));
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
    fixture.write("cargo-cop.toml", "version = 2\nunknown = true\n");

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
        "const FIRST: u8 = 1;\nconst SECOND: u8 = 2;\n\nstatic FIRST_STATIC: u8 = 1;\nstatic SECOND_STATIC: u8 = 2;\n\ntype FirstAlias = u8;\ntype SecondAlias = u8;\n",
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
fn reports_expected_and_actual_spacing_for_repeated_special_definitions() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", &package_manifest("sample"));
    fixture.write(
        "src/lib.rs",
        "const FIRST: u8 = 1;\n\nconst SECOND: u8 = 2;\n\n\nconst THIRD: u8 = 3;\n\nstatic FIRST_STATIC: u8 = 1;\n\nstatic SECOND_STATIC: u8 = 2;\n\n\nstatic THIRD_STATIC: u8 = 3;\n\ntype FirstAlias = u8;\n\ntype SecondAlias = u8;\n\n\ntype ThirdAlias = u8;\n",
    );

    let output = fixture.run(&["--format", "json"]);
    let diagnostics = item_spacing_diagnostics(&output);

    assert_eq!(output.status, 1, "{}", output.stdout);
    assert_eq!(diagnostics.len(), 6, "{}", output.stdout);
    assert!(diagnostics
        .iter()
        .all(|diagnostic| diagnostic["arguments"]["expected"].as_i64() == Some(0)));
    let actual_lines = diagnostics
        .iter()
        .map(|diagnostic| diagnostic["arguments"]["actual"].as_i64())
        .collect::<Vec<_>>();
    assert_eq!(
        actual_lines,
        [Some(1), Some(2), Some(1), Some(2), Some(1), Some(2)]
    );
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
        .all(|diagnostic| diagnostic["arguments"]["expected"].as_i64() == Some(1)));
    let actual_lines = diagnostics
        .iter()
        .map(|diagnostic| diagnostic["arguments"]["actual"].as_i64())
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
    assert!(
        !output
            .stdout
            .contains("\"message_id\":\"syntax-parse-error-unexpected-eof\""),
        "{}",
        output.stdout
    );
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
        diagnostic["arguments"]["expected"].as_i64() == Some(1)
            && diagnostic["arguments"]["actual"].as_i64() == Some(0)
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
    assert_eq!(diagnostics.len(), 3, "{}", output.stdout);
    let expected_actual = diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic["arguments"]["expected"].as_i64(),
                diagnostic["arguments"]["actual"].as_i64(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        expected_actual,
        [(Some(0), Some(1)), (Some(1), Some(0)), (Some(1), Some(0))]
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
    assert_eq!(diagnostics.len(), 2, "{}", output.stdout);
    let expected_actual = diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic["arguments"]["expected"].as_i64(),
                diagnostic["arguments"]["actual"].as_i64(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(expected_actual, [(Some(0), Some(1)), (Some(1), Some(0))]);
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
fn resolves_cfg_attr_paths_from_the_out_of_line_module_directory() {
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
    fixture.write("src/outer/logical.rs", "pub fn default_path() {}\n");
    fixture.write(
        "src/outer/children/selected.rs",
        "pub fn selected_path() {}\n",
    );

    let default_path = fixture.run(&["--format", "json"]);
    let selected_path = fixture.run(&["--format", "json", "--features", "selected"]);
    assert_eq!(default_path.status, 1, "{}", default_path.stderr);
    assert!(default_path.stdout.contains("src/outer/logical.rs"));
    assert!(default_path.stdout.contains("crate::outer::logical"));
    assert_eq!(
        selected_path.status, 1,
        "{} {}",
        selected_path.stderr, selected_path.stdout
    );
    assert!(selected_path
        .stdout
        .contains("src/outer/children/selected.rs"));
    assert!(selected_path.stdout.contains("crate::outer::logical"));
    assert!(!selected_path.stdout.contains("missing_module"));
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
fn restricts_trait_impl_exceptions_to_builtin_and_standard_library_types() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"vendor\"]\nresolver = \"2\"\n\n[dependencies]\nvendor = { path = \"vendor\" }\n",
    );
    fixture.write(
        "src/lib.rs",
        "pub trait Local {}\nuse std::vec::Vec;\nimpl Local for Vec<u8> {}\nimpl Local for vendor::Thing {}\n",
    );
    fixture.write(
        "vendor/Cargo.toml",
        "[package]\nname = \"vendor\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    fixture.write("vendor/src/lib.rs", "pub struct Thing;\n");

    let output = fixture.run(&["--format", "json", "--package", "app", "--locale", "zh-CN"]);
    assert_eq!(output.status, 1, "{}", output.stderr);
    assert_eq!(
        output
            .stdout
            .matches("\"rule_id\":\"impl_locality\"")
            .count(),
        1
    );
    assert!(output.stdout.contains("不属于允许的标准库类型"));
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
            .matches("\"rule_id\":\"cfg_use_group_spacing\"")
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
    assert!(!separated.has_rule("cfg_use_group_spacing"));
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
    assert!(output
        .stdout
        .contains("syntax-parse-error-unexpected-token"));
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
    let names = diagnostics
        .iter()
        .map(|diagnostic| {
            diagnostic["arguments"]["name"]
                .as_str()
                .expect("诊断声明名称")
        })
        .collect::<Vec<_>>();

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
    let names = diagnostics
        .iter()
        .map(|diagnostic| diagnostic["arguments"]["name"].as_str().expect("声明名称"))
        .collect::<Vec<_>>();

    assert_eq!(names, ["inactive_outside_use"], "{}", output.stdout);
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
