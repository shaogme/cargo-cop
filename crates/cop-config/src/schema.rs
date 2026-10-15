use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use cop_model::{
    LocalizedMessage, RuleId, Severity, CONFIG_INVALID_FIELD, CONFIG_INVALID_ROOT,
    CONFIG_PARSE_ERROR, CONFIG_UNSUPPORTED_VERSION, CONFIG_VERSION_TYPE,
};
use toml::{from_str, map::Map, Value};

use crate::ConfigError;

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
}

pub(super) struct ConfigDocumentParts {
    pub(super) allow_warnings: bool,
    pub(super) scan: ScanDefaults,
    pub(super) rules: BTreeMap<RuleId, (bool, Severity)>,
    pub(super) locale: Option<String>,
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
        }
    }

    pub(super) fn parse(path: &Path, contents: &str) -> Result<Self, ConfigError> {
        let value = from_str::<Value>(contents).map_err(|error| {
            ConfigError::new(
                path.to_path_buf(),
                LocalizedMessage::new(CONFIG_PARSE_ERROR)
                    .with_text("path", path.display().to_string())
                    .with_opaque("detail", error.to_string()),
            )
        })?;
        let root = value.as_table().ok_or_else(|| {
            ConfigError::new(
                path.to_path_buf(),
                LocalizedMessage::new(CONFIG_INVALID_ROOT)
                    .with_text("path", path.display().to_string()),
            )
        })?;

        check_fields(
            path,
            "",
            root,
            &["version", "allow_warnings", "i18n", "scan", "rules"],
        )?;
        let version = root
            .get("version")
            .and_then(Value::as_integer)
            .ok_or_else(|| {
                ConfigError::new(
                    path.to_path_buf(),
                    LocalizedMessage::new(CONFIG_VERSION_TYPE)
                        .with_text("path", path.display().to_string()),
                )
            })?;
        if version != 2 {
            return Err(ConfigError::new(
                path.to_path_buf(),
                LocalizedMessage::new(CONFIG_UNSUPPORTED_VERSION)
                    .with_text("path", path.display().to_string())
                    .with_number("version", version)
                    .with_number("supported", 2),
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
        Ok(Self {
            allow_warnings,
            scan,
            rules,
            locale,
        })
    }

    pub(super) fn into_parts(self) -> ConfigDocumentParts {
        ConfigDocumentParts {
            allow_warnings: self.allow_warnings,
            scan: self.scan,
            rules: self.rules,
            locale: self.locale,
        }
    }

    pub(super) fn read_locale(path: &Path, contents: &str) -> Result<Option<String>, ConfigError> {
        let value = from_str::<Value>(contents).map_err(|error| {
            ConfigError::new(
                path.to_path_buf(),
                LocalizedMessage::new(CONFIG_PARSE_ERROR)
                    .with_text("path", path.display().to_string())
                    .with_opaque("detail", error.to_string()),
            )
        })?;
        let root = value.as_table().ok_or_else(|| {
            ConfigError::new(
                path.to_path_buf(),
                LocalizedMessage::new(CONFIG_INVALID_ROOT)
                    .with_text("path", path.display().to_string()),
            )
        })?;
        parse_locale(path, root.get("i18n"))
    }
}

fn parse_locale(path: &Path, value: Option<&Value>) -> Result<Option<String>, ConfigError> {
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

fn parse_scan(path: &Path, value: Option<&Value>) -> Result<ScanDefaults, ConfigError> {
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
) -> Result<BTreeMap<RuleId, (bool, Severity)>, ConfigError> {
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

fn check_fields(
    path: &Path,
    prefix: &str,
    table: &Map<String, Value>,
    expected: &[&str],
) -> Result<(), ConfigError> {
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

fn ensure_unique(path: &Path, field: &str, values: &[String]) -> Result<(), ConfigError> {
    let mut seen = BTreeSet::new();
    if let Some(duplicate) = values.iter().find(|value| !seen.insert(value.as_str())) {
        return Err(field_error(path, field, "duplicate", duplicate));
    }
    Ok(())
}

fn field_error(path: &Path, field: &str, reason: &str, value: &str) -> ConfigError {
    ConfigError::new(
        path.to_path_buf(),
        LocalizedMessage::new(CONFIG_INVALID_FIELD)
            .with_text("path", path.display().to_string())
            .with_text("field", field)
            .with_text("reason", reason)
            .with_text("value", value),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::ConfigDocument;

    #[test]
    fn rejects_invalid_versions_features_and_rule_settings() {
        let cases = [
            ("", "config-version-type"),
            ("version = 1", "config-unsupported-version"),
            (
                "version = 2\nallow_warnings = \"yes\"",
                "config-invalid-field",
            ),
            (
                "version = 2\n[scan]\nfeatures = [\"fast\", \"fast\"]",
                "config-invalid-field",
            ),
            ("version = 2\n[rules.unknown_rule]", "config-invalid-field"),
            (
                "version = 2\n[rules.parameter_limit]\nseverity = \"fatal\"",
                "config-invalid-field",
            ),
        ];

        for (contents, expected) in cases {
            let error = ConfigDocument::parse(Path::new("cargo-cop.toml"), contents)
                .expect_err("无效配置必须失败");
            assert_eq!(error.message().id().as_str(), expected);
        }
    }
}
