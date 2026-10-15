use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

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

        ResolvedConfig {
            allow_warnings: config.allow_warnings(),
            scan,
            rules,
            source_path: config.source_path().map(Path::to_path_buf),
            all_features: overrides.all_features,
        }
    }
}
