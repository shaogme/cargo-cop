use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use cop_cfg::{CfgValue, ScopedCfgValue};
use cop_model::{RuleId, Severity};

use crate::{
    load::LoadedConfig,
    schema::{ScanDefaults, ScanTargetScope},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleSetting {
    enabled: bool,
    severity: Severity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedConfig {
    allow_warnings: bool,
    scan: ScanDefaults,
    rules: BTreeMap<RuleId, RuleSetting>,
    source_path: Option<PathBuf>,
    all_features: bool,
    cfg_overrides: Vec<ScopedCfgValue>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigOverrides {
    target: Option<String>,
    features: Option<Vec<String>>,
    all_features: bool,
    no_default_features: bool,
    all_targets: bool,
    disabled_rules: BTreeSet<RuleId>,
    severities: BTreeMap<RuleId, Severity>,
    cfg_values: Option<Vec<CfgValue>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigResolutionInput {
    config: LoadedConfig,
    overrides: ConfigOverrides,
}

pub struct ConfigResolver;

impl RuleSetting {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn severity(&self) -> Severity {
        self.severity
    }
}

impl ResolvedConfig {
    pub fn allow_warnings(&self) -> bool {
        self.allow_warnings
    }

    pub fn scan(&self) -> &ScanDefaults {
        &self.scan
    }

    pub fn rule(&self, rule_id: RuleId) -> &RuleSetting {
        &self.rules[&rule_id]
    }

    pub fn rules(&self) -> &BTreeMap<RuleId, RuleSetting> {
        &self.rules
    }

    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    pub fn all_features(&self) -> bool {
        self.all_features
    }

    pub fn cfg_overrides(&self) -> &[ScopedCfgValue] {
        &self.cfg_overrides
    }
}

impl ConfigOverrides {
    pub fn with_target(mut self, target: Option<String>) -> Self {
        self.target = target;
        self
    }

    pub fn with_features(mut self, features: Option<Vec<String>>) -> Self {
        self.features = features;
        self
    }

    pub fn with_all_features(mut self, all_features: bool) -> Self {
        self.all_features = all_features;
        self
    }

    pub fn with_no_default_features(mut self, no_default_features: bool) -> Self {
        self.no_default_features = no_default_features;
        self
    }

    pub fn with_all_targets(mut self, all_targets: bool) -> Self {
        self.all_targets = all_targets;
        self
    }

    pub fn disable_rule(mut self, rule_id: RuleId) -> Self {
        self.disabled_rules.insert(rule_id);
        self
    }

    pub fn set_severity(mut self, rule_id: RuleId, severity: Severity) -> Self {
        self.severities.insert(rule_id, severity);
        self
    }

    pub fn with_cfg_values(mut self, cfg_values: Option<Vec<CfgValue>>) -> Self {
        self.cfg_values = cfg_values;
        self
    }
}

impl ConfigResolutionInput {
    pub fn new(config: LoadedConfig, overrides: ConfigOverrides) -> Self {
        Self { config, overrides }
    }
}

impl ConfigResolver {
    pub fn resolve(input: ConfigResolutionInput) -> ResolvedConfig {
        let ConfigResolutionInput { config, overrides } = input;
        let mut scan = config.scan_defaults().clone();
        if overrides.all_targets {
            scan.set_target_scope(ScanTargetScope::All);
        }
        if let Some(target) = overrides.target {
            scan.set_target(target);
        }
        if let Some(features) = overrides.features {
            scan.set_features(features);
        }
        if overrides.no_default_features {
            scan.disable_default_features();
        }

        let mut rules = BTreeMap::new();
        for rule_id in RuleId::all() {
            let (mut enabled, config_severity) = config
                .rules()
                .get(rule_id)
                .copied()
                .unwrap_or((true, Severity::Error));
            let severity = overrides
                .severities
                .get(rule_id)
                .copied()
                .unwrap_or(config_severity);
            if overrides.disabled_rules.contains(rule_id) {
                enabled = false;
            }
            rules.insert(*rule_id, RuleSetting { enabled, severity });
        }

        let mut cfg_overrides = config.cfg_overrides().to_vec();
        if let Some(cfg_values) = overrides.cfg_values {
            let overridden_names = cfg_values
                .iter()
                .map(|value| value.name().to_owned())
                .collect::<BTreeSet<_>>();
            cfg_overrides.retain(|value| !overridden_names.contains(value.value().name()));
            cfg_overrides.extend(
                cfg_values
                    .into_iter()
                    .map(|value| ScopedCfgValue::new(None, None, value)),
            );
        }

        ResolvedConfig {
            allow_warnings: config.allow_warnings(),
            scan,
            rules,
            source_path: config.source_path().map(Path::to_path_buf),
            all_features: overrides.all_features,
            cfg_overrides,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        process::id,
        sync::atomic::{AtomicU64, Ordering},
    };

    use cop_cfg::CfgValue;

    use super::{ConfigOverrides, ConfigResolutionInput, ConfigResolver};
    use crate::{ConfigLoader, ConfigRequest};

    static NEXT_CONFIG: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn command_line_cfg_values_replace_scoped_config_values_by_key() {
        let sequence = NEXT_CONFIG.fetch_add(1, Ordering::Relaxed);
        let directory = env::temp_dir().join(format!("cargo-cop-config-{}-{sequence}", id()));
        fs::create_dir_all(&directory).expect("创建临时配置目录");
        let path = directory.join("cargo-cop.toml");
        fs::write(
            &path,
            "version = 3\n\n[[cfg]]\npackage = \"sample\"\nname = \"custom_mode\"\nvalue = \"file\"\n\n[[cfg]]\nname = \"other_mode\"\n",
        )
        .expect("写入配置");
        let config = ConfigLoader::load(ConfigRequest::new(
            Some(path),
            directory.clone(),
            directory.clone(),
        ))
        .expect("读取配置");
        let resolved = ConfigResolver::resolve(ConfigResolutionInput::new(
            config,
            ConfigOverrides::default().with_cfg_values(Some(vec![CfgValue::new(
                "custom_mode",
                Some("cli".to_owned()),
            )])),
        ));

        assert_eq!(resolved.cfg_overrides().len(), 2);
        let cli_override = resolved
            .cfg_overrides()
            .iter()
            .find(|value| value.value().name() == "custom_mode")
            .expect("CLI cfg 覆盖");
        assert_eq!(cli_override.package(), None);
        assert_eq!(cli_override.value().value(), Some("cli"));
        assert!(resolved
            .cfg_overrides()
            .iter()
            .any(|value| value.value().name() == "other_mode"));
        fs::remove_dir_all(directory).expect("清理临时配置目录");
    }
}
