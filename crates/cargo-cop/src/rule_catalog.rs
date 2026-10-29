use cop_i18n::{t, Locale, RenderError};
use cop_model::{RuleId, UnknownRuleId};
use cop_report::ReportFormat;
use serde_json::{json, Value};

const DEFAULT_SEVERITY: &str = "error";

struct RuleEntry {
    rule_id: RuleId,
    summary: String,
    description: String,
    default_enabled: bool,
    default_severity: &'static str,
}

struct RuleCatalog;

pub(super) enum RuleQueryError {
    UnknownRule(UnknownRuleId),
    Render(RenderError),
}

pub(super) struct RuleQuery;

impl RuleEntry {
    fn rule_id(&self) -> RuleId {
        self.rule_id
    }

    fn summary(&self) -> &str {
        &self.summary
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn default_enabled(&self) -> bool {
        self.default_enabled
    }

    fn default_severity(&self) -> &'static str {
        self.default_severity
    }

    fn new(rule_id: RuleId, locale: Locale) -> Result<Self, RenderError> {
        Ok(Self {
            rule_id,
            summary: RuleCatalog::summary(rule_id, locale)?,
            description: RuleCatalog::description(rule_id, locale)?,
            default_enabled: true,
            default_severity: DEFAULT_SEVERITY,
        })
    }
}

impl RuleCatalog {
    fn list(locale: Locale) -> Result<Vec<RuleEntry>, RenderError> {
        RuleId::all()
            .iter()
            .copied()
            .map(|rule_id| RuleEntry::new(rule_id, locale))
            .collect()
    }

    fn entry(rule_id: RuleId, locale: Locale) -> Result<RuleEntry, RenderError> {
        RuleEntry::new(rule_id, locale)
    }

    fn summary(rule_id: RuleId, locale: Locale) -> Result<String, RenderError> {
        match rule_id {
            RuleId::ModuleFileLayout => t!(
                "rule_catalog.module_file_layout.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::NoLintRelaxation => t!(
                "rule_catalog.no_lint_relaxation.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ItemSpacing => t!(
                "rule_catalog.item_spacing.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ChildModuleVisibility => t!(
                "rule_catalog.child_module_visibility.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::NoParentPathAttribute => t!(
                "rule_catalog.no_parent_path_attribute.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::NoParentPrivateDependency => t!(
                "rule_catalog.no_parent_private_dependency.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ChildModuleLineCount => t!(
                "rule_catalog.child_module_line_count.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ImplLocality => t!(
                "rule_catalog.impl_locality.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::PrivateFieldsForBehaviorTypes => t!(
                "rule_catalog.private_fields_for_behavior_types.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ChildPublicFreeFunctions => t!(
                "rule_catalog.child_public_free_functions.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::DeclarationBeforeUse => t!(
                "rule_catalog.declaration_before_use.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ParameterLimit => t!(
                "rule_catalog.parameter_limit.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::TupleArity => t!(
                "rule_catalog.tuple_arity.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::NoInlineQualifiedPath => t!(
                "rule_catalog.no_inline_qualified_path.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::MergeCommonPrefixImports => t!(
                "rule_catalog.merge_common_prefix_imports.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ExternCrateAtScopeStart => t!(
                "rule_catalog.extern_crate_at_scope_start.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ImportsAtScopeStart => t!(
                "rule_catalog.imports_at_scope_start.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::OutOfLineModulesAtScopeStart => t!(
                "rule_catalog.out_of_line_modules_at_scope_start.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::CfgUseGroupSpacing => t!(
                "rule_catalog.cfg_use_group_spacing.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::CfgInsideUseTree => t!(
                "rule_catalog.cfg_inside_use_tree.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::UnnecessaryPubCrate => t!(
                "rule_catalog.unnecessary_pub_crate.summary",
                locale = locale.as_tag().as_locale(),
                {}
            ),
        }
    }

    fn description(rule_id: RuleId, locale: Locale) -> Result<String, RenderError> {
        match rule_id {
            RuleId::ModuleFileLayout => t!(
                "rule_catalog.module_file_layout.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::NoLintRelaxation => t!(
                "rule_catalog.no_lint_relaxation.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ItemSpacing => t!(
                "rule_catalog.item_spacing.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ChildModuleVisibility => t!(
                "rule_catalog.child_module_visibility.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::NoParentPathAttribute => t!(
                "rule_catalog.no_parent_path_attribute.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::NoParentPrivateDependency => t!(
                "rule_catalog.no_parent_private_dependency.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ChildModuleLineCount => t!(
                "rule_catalog.child_module_line_count.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ImplLocality => t!(
                "rule_catalog.impl_locality.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::PrivateFieldsForBehaviorTypes => t!(
                "rule_catalog.private_fields_for_behavior_types.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ChildPublicFreeFunctions => t!(
                "rule_catalog.child_public_free_functions.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::DeclarationBeforeUse => t!(
                "rule_catalog.declaration_before_use.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ParameterLimit => t!(
                "rule_catalog.parameter_limit.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::TupleArity => t!(
                "rule_catalog.tuple_arity.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::NoInlineQualifiedPath => t!(
                "rule_catalog.no_inline_qualified_path.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::MergeCommonPrefixImports => t!(
                "rule_catalog.merge_common_prefix_imports.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ExternCrateAtScopeStart => t!(
                "rule_catalog.extern_crate_at_scope_start.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::ImportsAtScopeStart => t!(
                "rule_catalog.imports_at_scope_start.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::OutOfLineModulesAtScopeStart => t!(
                "rule_catalog.out_of_line_modules_at_scope_start.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::CfgUseGroupSpacing => t!(
                "rule_catalog.cfg_use_group_spacing.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::CfgInsideUseTree => t!(
                "rule_catalog.cfg_inside_use_tree.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
            RuleId::UnnecessaryPubCrate => t!(
                "rule_catalog.unnecessary_pub_crate.description",
                locale = locale.as_tag().as_locale(),
                {}
            ),
        }
    }
}

impl RuleQuery {
    pub(super) fn render(
        list: bool,
        rule_name: Option<&str>,
        format: ReportFormat,
        locale: Locale,
    ) -> Result<String, RuleQueryError> {
        let entries = match rule_name {
            Some(value) => {
                let rule_id = value.parse().map_err(RuleQueryError::UnknownRule)?;
                vec![RuleCatalog::entry(rule_id, locale)?]
            }
            None => RuleCatalog::list(locale)?,
        };
        match format {
            ReportFormat::Human => Ok(Self::render_human(&entries, list, locale)?),
            ReportFormat::Json => Ok(Self::render_json(&entries, list)),
        }
    }

    fn render_human(
        entries: &[RuleEntry],
        list: bool,
        locale: Locale,
    ) -> Result<String, RenderError> {
        let mut output = String::new();
        if list {
            output.push_str(&t!(
                "rule_catalog.list_title",
                locale = locale.as_tag().as_locale(),
                { count: entries.len() }
            )?);
            for entry in entries {
                let enabled = if entry.default_enabled() {
                    t!(
                        "rule_catalog.enabled",
                        locale = locale.as_tag().as_locale(),
                        {}
                    )?
                } else {
                    t!(
                        "rule_catalog.disabled",
                        locale = locale.as_tag().as_locale(),
                        {}
                    )?
                };
                output.push('\n');
                output.push_str(&format!(
                    "{} [{enabled}, {}] — {}",
                    entry.rule_id().as_str(),
                    entry.default_severity(),
                    entry.summary()
                ));
            }
            return Ok(output);
        }

        let entry = entries.first().expect("single-rule query has one entry");
        output.push_str(entry.rule_id().as_str());
        output.push('\n');
        output.push_str(&t!(
            "rule_catalog.summary_label",
            locale = locale.as_tag().as_locale(),
            {}
        )?);
        output.push_str(": ");
        output.push_str(entry.summary());
        output.push('\n');
        output.push_str(&t!(
            "rule_catalog.description_label",
            locale = locale.as_tag().as_locale(),
            {}
        )?);
        output.push_str(": ");
        output.push_str(entry.description());
        output.push('\n');
        output.push_str(&t!(
            "rule_catalog.default_enabled_label",
            locale = locale.as_tag().as_locale(),
            {}
        )?);
        output.push_str(": ");
        output.push_str(&t!(
            "rule_catalog.enabled",
            locale = locale.as_tag().as_locale(),
            {}
        )?);
        output.push('\n');
        output.push_str(&t!(
            "rule_catalog.default_severity_label",
            locale = locale.as_tag().as_locale(),
            {}
        )?);
        output.push_str(": ");
        output.push_str(entry.default_severity());
        output.push('\n');
        output.push_str(&t!(
            "rule_catalog.configuration_label",
            locale = locale.as_tag().as_locale(),
            {}
        )?);
        output.push_str(&format!(": [rules.{}]", entry.rule_id().as_str()));
        output.push('\n');
        output.push_str(&t!(
            "rule_catalog.overrides_label",
            locale = locale.as_tag().as_locale(),
            {}
        )?);
        output.push_str(&format!(
            ": --disable-rule {}; --severity {}=warning|error",
            entry.rule_id().as_str(),
            entry.rule_id().as_str()
        ));
        Ok(output)
    }

    fn render_json(entries: &[RuleEntry], list: bool) -> String {
        let rules = entries
            .iter()
            .map(|entry| {
                json!({
                    "rule_id": entry.rule_id().as_str(),
                    "summary": entry.summary(),
                    "description": entry.description(),
                    "default_enabled": entry.default_enabled(),
                    "default_severity": entry.default_severity(),
                })
            })
            .collect::<Vec<Value>>();
        let report = if list {
            json!({
                "schema_version": 1,
                "kind": "rule_catalog",
                "rules": rules,
            })
        } else {
            json!({
                "schema_version": 1,
                "kind": "rule_catalog",
                "rule": rules.first(),
            })
        };
        report.to_string()
    }
}

impl From<RenderError> for RuleQueryError {
    fn from(error: RenderError) -> Self {
        Self::Render(error)
    }
}
