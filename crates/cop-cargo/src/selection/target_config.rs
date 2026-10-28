use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use cop_cfg::{CfgContext, CfgDecision, CfgEvaluator, CfgExpression, CfgValue, ScopedCfgValue};
use cop_model::{
    LocalizedMessage, ToolErrorKind, CARGO_CONFIG_TARGET_TYPE, CARGO_ERROR,
    CARGO_HOST_TRIPLE_MISSING, CARGO_OPERATION_ERROR, CARGO_TARGET_CFG_INVALID,
    CARGO_TARGET_CFG_QUERY_ERROR,
};
use toml::{from_str, Value};

use crate::metadata::CargoError;

pub(super) struct TargetConfiguration;

impl TargetConfiguration {
    pub(super) fn cfg_values_for_package(
        rustc_values: &[CfgValue],
        overrides: &[ScopedCfgValue],
        package_name: &str,
        target_name: &str,
        target_triple: &str,
    ) -> Vec<CfgValue> {
        rustc_values
            .iter()
            .cloned()
            .chain(
                overrides
                    .iter()
                    .filter(|override_| {
                        override_.applies_to(package_name, target_name, target_triple)
                    })
                    .map(|override_| override_.value().clone()),
            )
            .collect()
    }

    pub(super) fn target_condition_matches(
        condition: &str,
        target_triple: &str,
        values: &[CfgValue],
        features: &BTreeSet<String>,
    ) -> Result<bool, CargoError> {
        let condition = condition.trim();
        if !condition.starts_with("cfg(") {
            return Ok(condition == target_triple);
        }
        let expression = CfgExpression::parse(condition).map_err(|_| {
            CargoError::new(
                ToolErrorKind::CargoMetadata,
                LocalizedMessage::new(CARGO_TARGET_CFG_INVALID).with_text("condition", condition),
            )
        })?;
        let context = CfgContext::new(values, Some(features));
        let decision = CfgEvaluator::new(&context).evaluate(&expression);
        Ok(decision == CfgDecision::True)
    }

    pub(super) fn resolve_target(
        configured_target: Option<&str>,
        current_directory: &Path,
    ) -> Result<String, CargoError> {
        if let Some(configured_target) = configured_target {
            return if configured_target == "host" {
                Self::host_triple()
            } else {
                Ok(configured_target.to_owned())
            };
        }
        if let Ok(target) = env::var("CARGO_BUILD_TARGET") {
            return Ok(target);
        }
        if let Some(target) = Self::cargo_config_target(current_directory)? {
            return Ok(target);
        }
        Self::host_triple()
    }

    pub(super) fn host_triple() -> Result<String, CargoError> {
        let output = Self::rustc_command().arg("-vV").output().map_err(|error| {
            CargoError::new(
                ToolErrorKind::CargoMetadata,
                LocalizedMessage::new(CARGO_ERROR).with_opaque("detail", error.to_string()),
            )
        })?;
        if !output.status.success() {
            return Err(CargoError::new(
                ToolErrorKind::CargoMetadata,
                LocalizedMessage::new(CARGO_ERROR).with_opaque(
                    "detail",
                    String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                ),
            ));
        }
        let version = String::from_utf8_lossy(&output.stdout);
        version
            .lines()
            .find_map(|line| {
                line.strip_prefix("host: ")
                    .map(|host_triple| host_triple.to_owned())
            })
            .ok_or_else(|| {
                CargoError::new(
                    ToolErrorKind::CargoMetadata,
                    LocalizedMessage::new(CARGO_HOST_TRIPLE_MISSING),
                )
            })
    }

    pub(super) fn query_cfg(target: &str) -> Result<Vec<CfgValue>, CargoError> {
        let output = Self::rustc_command()
            .arg("--print")
            .arg("cfg")
            .arg("--target")
            .arg(target)
            .output()
            .map_err(|error| {
                CargoError::new(
                    ToolErrorKind::CargoMetadata,
                    LocalizedMessage::new(CARGO_TARGET_CFG_QUERY_ERROR)
                        .with_text("target", target)
                        .with_opaque("detail", error.to_string()),
                )
            })?;
        if !output.status.success() {
            return Err(CargoError::new(
                ToolErrorKind::CargoMetadata,
                LocalizedMessage::new(CARGO_TARGET_CFG_QUERY_ERROR)
                    .with_text("target", target)
                    .with_opaque("detail", String::from_utf8_lossy(&output.stderr).trim()),
            ));
        }
        let mut cfg_values = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(Self::parse_cfg_value)
            .collect::<Vec<_>>();
        cfg_values.sort();
        Ok(cfg_values)
    }

    fn cargo_config_target(current_directory: &Path) -> Result<Option<String>, CargoError> {
        let cargo_home = env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")));
        let mut target = match cargo_home {
            Some(cargo_home) => Self::read_target_setting(&cargo_home, false)?,
            None => None,
        };
        let mut directories = current_directory
            .ancestors()
            .map(Path::to_path_buf)
            .collect::<Vec<_>>();
        directories.reverse();
        for directory in directories {
            if let Some(configured) = Self::read_target_setting(&directory, true)? {
                target = Some(configured);
            }
        }
        Ok(target)
    }

    fn read_target_setting(
        directory: &Path,
        nested_cargo_directory: bool,
    ) -> Result<Option<String>, CargoError> {
        let config_directory = if nested_cargo_directory {
            directory.join(".cargo")
        } else {
            directory.to_path_buf()
        };
        let toml_path = config_directory.join("config.toml");
        let legacy_path = config_directory.join("config");
        let path = if toml_path.is_file() {
            toml_path
        } else if legacy_path.is_file() {
            legacy_path
        } else {
            return Ok(None);
        };
        let contents = fs::read_to_string(&path).map_err(|error| {
            CargoError::new(
                ToolErrorKind::Io,
                LocalizedMessage::new(CARGO_OPERATION_ERROR)
                    .with_text("path", path.display().to_string())
                    .with_opaque("detail", error.to_string()),
            )
        })?;
        let value = from_str::<Value>(&contents).map_err(|error| {
            CargoError::new(
                ToolErrorKind::CargoMetadata,
                LocalizedMessage::new(CARGO_OPERATION_ERROR)
                    .with_text("path", path.display().to_string())
                    .with_opaque("detail", error.to_string()),
            )
        })?;
        let configured = value
            .get("build")
            .and_then(Value::as_table)
            .and_then(|build| build.get("target"));
        configured
            .map(|configured| {
                configured.as_str().map(str::to_owned).ok_or_else(|| {
                    CargoError::new(
                        ToolErrorKind::CargoMetadata,
                        LocalizedMessage::new(CARGO_CONFIG_TARGET_TYPE)
                            .with_text("path", path.display().to_string()),
                    )
                })
            })
            .transpose()
    }

    fn parse_cfg_value(line: &str) -> Option<CfgValue> {
        if let Some((name, value)) = line.split_once('=') {
            let value = value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or(value);
            Some(CfgValue::new(name, Some(value.to_owned())))
        } else if !line.is_empty() {
            Some(CfgValue::new(line, None))
        } else {
            None
        }
    }

    fn rustc_command() -> Command {
        match env::var_os("RUSTC") {
            Some(rustc) => Command::new(rustc),
            None => Command::new("rustc"),
        }
    }
}
