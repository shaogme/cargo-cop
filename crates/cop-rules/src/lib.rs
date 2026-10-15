#![forbid(unsafe_code)]

mod import_rules;
mod module_rules;
mod semantic_rules;
mod support;
mod syntax_rules;

use cop_analysis::AnalysisSnapshot;
use cop_config::ResolvedConfig;
use cop_model::Diagnostic;

pub struct RuleEngine;

impl RuleEngine {
    pub fn run(snapshot: &AnalysisSnapshot, configuration: &ResolvedConfig) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        module_rules::ModuleRuleSet::run(snapshot, configuration, &mut diagnostics);
        import_rules::ImportRuleSet::run(snapshot, configuration, &mut diagnostics);
        semantic_rules::SemanticRuleSet::run(snapshot, configuration, &mut diagnostics);
        syntax_rules::SyntaxRuleSet::run(snapshot, configuration, &mut diagnostics);
        diagnostics
    }
}
