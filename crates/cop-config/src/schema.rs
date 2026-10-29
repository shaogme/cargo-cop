use cop_i18n::t;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use cop_cfg::{CfgValue, ScopedCfgValue};
use cop_model::{RuleId, Severity};
use toml::{from_str, map::Map, Value};

use crate::{ConfigError, ConfigFailure};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanTargetScope {
    Default,
    All,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanDefaults {
    target_scope: ScanTargetScope,
    target: Option<String>,
    features: Vec<String>,
    default_features: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ConfigDocument {
    allow_warnings: bool,
    scan: ScanDefaults,
    rules: BTreeMap<RuleId, (bool, Severity)>,
    locale: Option<String>,
    cfg_overrides: Vec<ScopedCfgValue>,
}

pub(super) struct ConfigDocumentParts {
    pub(super) allow_warnings: bool,
    pub(super) scan: ScanDefaults,
    pub(super) rules: BTreeMap<RuleId, (bool, Severity)>,
    pub(super) locale: Option<String>,
    pub(super) cfg_overrides: Vec<ScopedCfgValue>,
}

impl ScanDefaults {
    pub fn target_scope(&self) -> ScanTargetScope {
        self.target_scope
    }

    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    pub fn features(&self) -> &[String] {
        &self.features
    }

    pub fn default_features(&self) -> bool {
        self.default_features
    }

    pub(super) fn set_target_scope(&mut self, target_scope: ScanTargetScope) {
        self.target_scope = target_scope;
    }

    pub(super) fn set_target(&mut self, target: String) {
        self.target = Some(target);
    }

    pub(super) fn set_features(&mut self, features: Vec<String>) {
        self.features = features;
    }

    pub(super) fn disable_default_features(&mut self) {
        self.default_features = false;
    }
}

impl Default for ScanDefaults {
    fn default() -> Self {
        Self {
            target_scope: ScanTargetScope::Default,
            target: None,
            features: Vec::new(),
            default_features: true,
        }
    }
}

impl ConfigDocument {
    pub(super) fn defaults() -> Self {
        Self {
            allow_warnings: false,
            scan: ScanDefaults::default(),
            rules: BTreeMap::new(),
            locale: None,
            cfg_overrides: Vec::new(),
        }
    }

    pub(super) fn parse(path: &Path, contents: &str) -> Result<Self, ConfigFailure> {
        let path_display = path.display().to_string();
        let value = from_str::<Value>(contents).map_err(|error| {
            ConfigError::from_message(
                path.to_path_buf(),
                t!("config.parse_error", { path: path_display.as_str() }),
            )
            .with_detail(error.to_string())
        })?;
        let root = value.as_table().ok_or_else(|| {
            ConfigError::from_message(
                path.to_path_buf(),
                t!("config.invalid_root", { path: path_display.as_str() }),
            )
        })?;

        check_fields(
            path,
            "",
            root,
            &["version", "allow_warnings", "i18n", "scan", "rules", "cfg"],
        )?;
        let version = root
            .get("version")
            .and_then(Value::as_integer)
            .ok_or_else(|| {
                ConfigError::from_message(
                    path.to_path_buf(),
                    t!("config.version_type", { path: path_display.as_str() }),
                )
            })?;
        if version != 3 {
            return Err(ConfigError::from_localized(
                path.to_path_buf(),
                t!("config.unsupported_version", { path: path_display.as_str(), version: version, supported: 3 })?,
            ));
        }

        let allow_warnings = match root.get("allow_warnings") {
            None => false,
            Some(value) => value
                .as_bool()
                .ok_or_else(|| field_error(path, "allow_warnings", "must-be-boolean", ""))?,
        };
        let locale = parse_locale(path, root.get("i18n"))?;
        let scan = parse_scan(path, root.get("scan"))?;
        let rules = parse_rules(path, root.get("rules"))?;
        let cfg_overrides = parse_cfg_overrides(path, root.get("cfg"))?;
        Ok(Self {
            allow_warnings,
            scan,
            rules,
            locale,
            cfg_overrides,
        })
    }

    pub(super) fn into_parts(self) -> ConfigDocumentParts {
        ConfigDocumentParts {
            allow_warnings: self.allow_warnings,
            scan: self.scan,
            rules: self.rules,
            locale: self.locale,
            cfg_overrides: self.cfg_overrides,
        }
    }

    pub(super) fn read_locale(
        path: &Path,
        contents: &str,
    ) -> Result<Option<String>, ConfigFailure> {
        let path_display = path.display().to_string();
        let value = from_str::<Value>(contents).map_err(|error| {
            ConfigError::from_message(
                path.to_path_buf(),
                t!("config.parse_error", { path: path_display.as_str() }),
            )
            .with_detail(error.to_string())
        })?;
        let root = value.as_table().ok_or_else(|| {
            ConfigError::from_message(
                path.to_path_buf(),
                t!("config.invalid_root", { path: path_display.as_str() }),
            )
        })?;
        parse_locale(path, root.get("i18n"))
    }
}

fn parse_locale(path: &Path, value: Option<&Value>) -> Result<Option<String>, ConfigFailure> {
    let Some(value) = value else {
        return Ok(None);
    };
    let table = value
        .as_table()
        .ok_or_else(|| field_error(path, "i18n", "must-be-table", ""))?;
    check_fields(path, "i18n", table, &["locale"])?;
    table
        .get("locale")
        .map(|locale| {
            locale
                .as_str()
                .filter(|locale| !locale.trim().is_empty())
                .map(str::to_owned)
                .ok_or_else(|| field_error(path, "i18n.locale", "must-be-nonempty-string", ""))
        })
        .transpose()
}

fn parse_scan(path: &Path, value: Option<&Value>) -> Result<ScanDefaults, ConfigFailure> {
    let Some(value) = value else {
        return Ok(ScanDefaults::default());
    };
    let table = value
        .as_table()
        .ok_or_else(|| field_error(path, "scan", "must-be-table", ""))?;
    check_fields(
        path,
        "scan",
        table,
        &["targets", "target", "features", "default_features"],
    )?;

    let target_scope = match table.get("targets") {
        None => ScanTargetScope::Default,
        Some(value) => match value.as_str() {
            Some("default") => ScanTargetScope::Default,
            Some("all") => ScanTargetScope::All,
            Some(other) => {
                return Err(field_error(path, "scan.targets", "unknown-value", other));
            }
            None => return Err(field_error(path, "scan.targets", "must-be-string", "")),
        },
    };
    let target = table
        .get("target")
        .map(|target| {
            target
                .as_str()
                .filter(|target| !target.trim().is_empty())
                .map(|target| target.to_owned())
                .ok_or_else(|| field_error(path, "scan.target", "must-be-nonempty-string", ""))
        })
        .transpose()?;
    let features = match table.get("features") {
        None => Vec::new(),
        Some(value) => value
            .as_array()
            .ok_or_else(|| field_error(path, "scan.features", "must-be-string-array", ""))?
            .iter()
            .enumerate()
            .map(|(index, feature)| {
                feature
                    .as_str()
                    .filter(|feature| !feature.trim().is_empty())
                    .map(|feature| feature.to_owned())
                    .ok_or_else(|| {
                        field_error(
                            path,
                            &format!("scan.features[{index}]"),
                            "must-be-nonempty-string",
                            "",
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    ensure_unique(path, "scan.features", &features)?;
    let default_features = match table.get("default_features") {
        None => true,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| field_error(path, "scan.default_features", "must-be-boolean", ""))?,
    };

    Ok(ScanDefaults {
        target_scope,
        target,
        features,
        default_features,
    })
}

fn parse_rules(
    path: &Path,
    value: Option<&Value>,
) -> Result<BTreeMap<RuleId, (bool, Severity)>, ConfigFailure> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let table = value
        .as_table()
        .ok_or_else(|| field_error(path, "rules", "must-be-table", ""))?;
    let mut rules = BTreeMap::new();
    for (rule_name, value) in table {
        let rule_id = rule_name
            .parse::<RuleId>()
            .map_err(|_| field_error(path, &format!("rules.{rule_name}"), "unknown-rule-id", ""))?;
        let settings = value
            .as_table()
            .ok_or_else(|| field_error(path, &format!("rules.{rule_name}"), "must-be-table", ""))?;
        check_fields(
            path,
            &format!("rules.{rule_name}"),
            settings,
            &["enabled", "severity"],
        )?;
        let enabled = match settings.get("enabled") {
            None => true,
            Some(value) => value.as_bool().ok_or_else(|| {
                field_error(
                    path,
                    &format!("rules.{rule_name}.enabled"),
                    "must-be-boolean",
                    "",
                )
            })?,
        };
        let severity = match settings.get("severity").and_then(Value::as_str) {
            None if !settings.contains_key("severity") => Severity::Error,
            Some("warning") => Severity::Warning,
            Some("error") => Severity::Error,
            Some(other) => {
                return Err(field_error(
                    path,
                    &format!("rules.{rule_name}.severity"),
                    "unknown-severity",
                    other,
                ));
            }
            None => {
                return Err(field_error(
                    path,
                    &format!("rules.{rule_name}.severity"),
                    "must-be-string",
                    "",
                ));
            }
        };
        rules.insert(rule_id, (enabled, severity));
    }
    Ok(rules)
}

fn parse_cfg_overrides(
    path: &Path,
    value: Option<&Value>,
) -> Result<Vec<ScopedCfgValue>, ConfigFailure> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let entries = value
        .as_array()
        .ok_or_else(|| field_error(path, "cfg", "must-be-array-of-tables", ""))?;
    let mut overrides = Vec::with_capacity(entries.len());
    let mut unique = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let field = format!("cfg[{index}]");
        let table = entry
            .as_table()
            .ok_or_else(|| field_error(path, &field, "must-be-table", ""))?;
        check_fields(path, &field, table, &["package", "target", "name", "value"])?;
        let package = optional_nonempty_string(path, table, &field, "package")?;
        let target = optional_nonempty_string(path, table, &field, "target")?;
        let name = table
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| is_cfg_identifier(name))
            .map(str::to_owned)
            .ok_or_else(|| field_error(path, &format!("{field}.name"), "invalid-cfg-name", ""))?;
        let cfg_value = table
            .get("value")
            .map(|value| {
                value.as_str().map(str::to_owned).ok_or_else(|| {
                    field_error(path, &format!("{field}.value"), "must-be-string", "")
                })
            })
            .transpose()?;
        let override_value = ScopedCfgValue::new(package, target, CfgValue::new(name, cfg_value));
        if !unique.insert(override_value.clone()) {
            return Err(field_error(path, "cfg", "duplicate", ""));
        }
        overrides.push(override_value);
    }
    Ok(overrides)
}

fn optional_nonempty_string(
    path: &Path,
    table: &Map<String, Value>,
    prefix: &str,
    name: &str,
) -> Result<Option<String>, ConfigFailure> {
    table
        .get(name)
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
                .ok_or_else(|| {
                    field_error(
                        path,
                        &format!("{prefix}.{name}"),
                        "must-be-nonempty-string",
                        "",
                    )
                })
        })
        .transpose()
}

fn is_cfg_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
        && !CfgValue::is_builtin_name(name)
}

fn check_fields(
    path: &Path,
    prefix: &str,
    table: &Map<String, Value>,
    expected: &[&str],
) -> Result<(), ConfigFailure> {
    if let Some(field) = table
        .keys()
        .find(|field| !expected.contains(&field.as_str()))
    {
        let field_path = if prefix.is_empty() {
            field.to_string()
        } else {
            format!("{prefix}.{field}")
        };
        return Err(field_error(path, &field_path, "unknown-field", ""));
    }
    Ok(())
}

fn ensure_unique(path: &Path, field: &str, values: &[String]) -> Result<(), ConfigFailure> {
    let mut seen = BTreeSet::new();
    if let Some(duplicate) = values.iter().find(|value| !seen.insert(value.as_str())) {
        return Err(field_error(path, field, "duplicate", duplicate));
    }
    Ok(())
}

fn field_error(path: &Path, field: &str, reason: &str, value: &str) -> ConfigFailure {
    let path_display = path.display().to_string();
    let reason = reason.replace('-', "_");
    ConfigError::from_message(
        path.to_path_buf(),
        t!("config.invalid_field", { path: path_display.as_str(), field: field, reason: reason.as_str(), value: value }),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::ConfigDocument;
    use crate::ConfigFailure;

    fn message(error: &ConfigFailure) -> &str {
        match error {
            ConfigFailure::Config(error) => error.message(),
            ConfigFailure::Render(error) => panic!("unexpected localization failure: {error}"),
        }
    }

    #[test]
    fn rejects_invalid_versions_features_and_rule_settings() {
        let cases = [
            ("", "must be an integer"),
            ("version = 1", "unsupported version"),
            ("version = 3\nallow_warnings = \"yes\"", "must be a boolean"),
            (
                "version = 3\n[scan]\nfeatures = [\"fast\", \"fast\"]",
                "duplicate value",
            ),
            ("version = 3\n[rules.unknown_rule]", "unknown rule ID"),
            (
                "version = 3\n[rules.parameter_limit]\nseverity = \"fatal\"",
                "unknown severity",
            ),
        ];

        for (contents, expected) in cases {
            let error = ConfigDocument::parse(Path::new("cargo-cop.toml"), contents)
                .expect_err("无效配置必须失败");
            assert!(message(&error).contains(expected), "{error}");
        }
    }

    #[test]
    fn loads_scoped_custom_cfg_values_and_rejects_unknown_fields() {
        let contents = "version = 3\n\n[[cfg]]\npackage = \"core\"\ntarget = \"x86_64-unknown-linux-gnu\"\nname = \"backend\"\nvalue = \"fast\"\n\n[[cfg]]\nname = \"custom_gate\"\n";
        let document =
            ConfigDocument::parse(Path::new("cargo-cop.toml"), contents).expect("解析 scoped cfg");

        assert_eq!(document.cfg_overrides.len(), 2);
        assert_eq!(document.cfg_overrides[0].package(), Some("core"));
        assert_eq!(
            document.cfg_overrides[0].target(),
            Some("x86_64-unknown-linux-gnu")
        );
        assert_eq!(document.cfg_overrides[0].value().name(), "backend");
        assert_eq!(document.cfg_overrides[0].value().value(), Some("fast"));
        assert_eq!(document.cfg_overrides[1].value().value(), None);

        let error = ConfigDocument::parse(
            Path::new("cargo-cop.toml"),
            "version = 3\n\n[[cfg]]\nname = \"custom_gate\"\nunknown = true\n",
        )
        .expect_err("未知 cfg 字段必须拒绝");
        assert!(message(&error).contains("unknown field"));
    }

    #[test]
    fn requires_version_three_for_cfg_schema() {
        let error = ConfigDocument::parse(Path::new("cargo-cop.toml"), "version = 2")
            .expect_err("旧 schema 必须显式迁移");

        assert!(message(&error).contains("unsupported version"));
    }

    #[test]
    fn rejects_builtin_names_in_custom_cfg_overrides() {
        for name in ["unix", "feature", "target_os", "test", "proc_macro"] {
            let contents = format!("version = 3\n\n[[cfg]]\nname = \"{name}\"\n");
            let error = ConfigDocument::parse(Path::new("cargo-cop.toml"), &contents)
                .expect_err("内置 cfg 名称不能作为自定义覆盖");

            assert!(message(&error).contains("invalid cfg name"));
        }
    }
}
