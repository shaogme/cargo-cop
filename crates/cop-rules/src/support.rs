use std::ops::Range;

use cop_analysis::{AnalysisFacts, CfgCondition, CfgDecision, ItemFact, ItemKindFact, ModuleNode};
use cop_config::ResolvedConfig;
use cop_model::{Diagnostic, LocalizedMessage, RuleId, SourceSpan};
use cop_syntax::{ParseOutput, TriviaKind};

pub(super) struct RuleReport {
    rule_id: RuleId,
    message: LocalizedMessage,
    bounds: Option<Range<usize>>,
    span: Option<SourceSpan>,
    cfg_condition: Option<String>,
}

pub(super) struct RuleDiagnosticAdapter<'a> {
    configuration: &'a ResolvedConfig,
}

pub(super) struct UseLeaf {
    pub(super) path: Vec<String>,
    pub(super) alias: Option<String>,
    pub(super) glob: bool,
    pub(super) condition: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpacingDefinitionKind {
    Struct,
    Enum,
    Union,
    TypeAlias,
    Trait,
    Constant,
    Static,
    Function,
    Impl,
}

pub(super) struct SyntaxSupport;

impl SyntaxSupport {
    pub(super) fn conditions_may_coexist_all(conditions: &[Option<&str>]) -> bool {
        let mut parsed = Vec::new();
        for condition in conditions.iter().flatten() {
            let Ok(condition) = CfgCondition::parse(condition) else {
                return true;
            };
            parsed.push(condition);
        }
        !matches!(CfgCondition::all(parsed), CfgCondition::Constant(false))
    }

    pub(super) fn common_import_prefix(left: &[UseLeaf], right: &[UseLeaf]) -> Option<String> {
        let mut best_prefix: Option<Vec<String>> = None;
        for left_leaf in left {
            if left_leaf.alias.is_some() || left_leaf.glob {
                continue;
            }
            for right_leaf in right {
                if right_leaf.alias.is_some() || right_leaf.glob {
                    continue;
                }
                let prefix_length = Self::common_prefix_len(&left_leaf.path, &right_leaf.path);
                if prefix_length == 0
                    || prefix_length >= left_leaf.path.len()
                    || prefix_length >= right_leaf.path.len()
                    || best_prefix
                        .as_ref()
                        .is_some_and(|prefix| prefix.len() >= prefix_length)
                {
                    continue;
                }
                best_prefix = Some(left_leaf.path[..prefix_length].to_vec());
            }
        }
        best_prefix.map(|prefix| prefix.join("::"))
    }

    fn common_prefix_len(left: &[String], right: &[String]) -> usize {
        left.iter()
            .zip(right)
            .take_while(|(left_segment, right_segment)| left_segment == right_segment)
            .count()
    }
}

impl RuleReport {
    pub(super) fn new(
        rule_id: RuleId,
        message: LocalizedMessage,
        bounds: Option<Range<usize>>,
    ) -> Self {
        Self {
            rule_id,
            message,
            bounds,
            span: None,
            cfg_condition: None,
        }
    }

    pub(super) fn at_span(rule_id: RuleId, message: LocalizedMessage, span: SourceSpan) -> Self {
        Self {
            rule_id,
            message,
            bounds: None,
            span: Some(span),
            cfg_condition: None,
        }
    }

    pub(super) fn with_cfg_condition(mut self, condition: Option<String>) -> Self {
        self.cfg_condition = condition;
        self
    }
}

impl RuleDiagnosticAdapter<'_> {
    pub(super) fn new(configuration: &ResolvedConfig) -> RuleDiagnosticAdapter<'_> {
        RuleDiagnosticAdapter { configuration }
    }

    pub(super) fn push(
        &self,
        module: &ModuleNode,
        diagnostics: &mut Vec<Diagnostic>,
        report: RuleReport,
    ) {
        let setting = self.configuration.rule(report.rule_id);
        if !setting.enabled() {
            return;
        }
        let explicit_condition = report.cfg_condition.clone();
        let cfg_bounds = report
            .bounds
            .clone()
            .or_else(|| report.span.as_ref().map(|span| span.byte_range()));
        if cfg_bounds.as_ref().is_some_and(|bounds| {
            module
                .configured_ast()
                .node_for_range(bounds.start, bounds.end)
                .is_some_and(|node| node.availability_decision() == CfgDecision::False)
        }) {
            return;
        }
        let mut diagnostic = Diagnostic::rule(report.rule_id, setting.severity(), report.message)
            .with_package(module.package_name())
            .with_package_manifest_path(module.package_manifest_path())
            .with_target(module.target_name())
            .with_target_kind(module.target_kind().to_vec())
            .with_namespace(namespace_name(module.namespace()));
        if let Some(condition) = explicit_condition
            .as_deref()
            .or_else(|| {
                cfg_bounds.as_ref().and_then(|bounds| {
                    module
                        .configured_ast()
                        .node_for_range(bounds.start, bounds.end)
                        .and_then(|node| node.availability_condition())
                })
            })
            .or(module.cfg_condition())
        {
            diagnostic = diagnostic.with_cfg_condition(condition);
        }
        if let Some(span) = report.span {
            diagnostic = diagnostic.with_span(span);
        } else if let Some(bounds) = report.bounds {
            if let Ok(span) = module
                .parse_output()
                .source()
                .span(bounds.start, bounds.end)
            {
                diagnostic = diagnostic.with_span(span);
            }
        }
        diagnostics.push(diagnostic);
    }
}

impl SyntaxSupport {
    pub(super) fn conditions_may_coexist(left: Option<&str>, right: Option<&str>) -> bool {
        let Some(left) = left else {
            return true;
        };
        let Some(right) = right else {
            return true;
        };
        match (CfgCondition::parse(left), CfgCondition::parse(right)) {
            (Ok(left), Ok(right)) => CfgCondition::may_coexist(&left, &right),
            _ => true,
        }
    }

    pub(super) fn expected_spacing_blank_lines(
        previous: &ItemFact,
        current: &ItemFact,
    ) -> Option<usize> {
        let previous_kind = Self::spacing_definition_kind(previous.kind())?;
        let current_kind = Self::spacing_definition_kind(current.kind())?;
        Some(match (previous_kind, current_kind) {
            (SpacingDefinitionKind::Constant, SpacingDefinitionKind::Constant)
            | (SpacingDefinitionKind::Static, SpacingDefinitionKind::Static)
            | (SpacingDefinitionKind::TypeAlias, SpacingDefinitionKind::TypeAlias) => 0,
            _ => 1,
        })
    }

    pub(super) fn use_facts(
        facts: &AnalysisFacts,
        module: &ModuleNode,
        item_bounds: Range<usize>,
    ) -> Vec<UseLeaf> {
        facts
            .use_references_for_module(module)
            .filter(|reference| {
                let bounds = reference.bounds();
                item_bounds.start <= bounds.start && bounds.end <= item_bounds.end
            })
            .map(|reference| UseLeaf {
                path: reference.segments().to_vec(),
                alias: reference.alias().map(str::to_owned),
                glob: reference.is_glob(),
                condition: reference.condition().map(str::to_owned),
            })
            .collect()
    }

    pub(super) fn blank_lines_between(
        output: &ParseOutput,
        previous_end: usize,
        next_start: usize,
    ) -> usize {
        let source = output.source();
        let Some((end_line, _)) = source.line_column(previous_end) else {
            return 0;
        };
        let Some((start_line, _)) = source.line_column(next_start) else {
            return 0;
        };
        if start_line <= end_line + 1 {
            return 0;
        }
        let comments = output
            .tokens()
            .iter()
            .flat_map(|token| token.leading_trivia())
            .chain(output.trailing_trivia())
            .filter(|trivia| {
                matches!(
                    trivia.kind(),
                    TriviaKind::LineComment | TriviaKind::BlockComment
                )
            })
            .map(|trivia| trivia.start()..trivia.end())
            .collect::<Vec<_>>();
        (end_line + 1..start_line)
            .filter(|line| {
                let Some(range) = source.line_range(*line) else {
                    return false;
                };
                let contains_comment = comments.iter().any(|comment| {
                    if range.is_empty() {
                        comment.start <= range.start && comment.end > range.start
                    } else {
                        comment.start < range.end && comment.end > range.start
                    }
                });
                !contains_comment
                    && source
                        .slice(range.start, range.end)
                        .is_some_and(|text| text.chars().all(char::is_whitespace))
            })
            .count()
    }

    fn spacing_definition_kind(kind: &ItemKindFact) -> Option<SpacingDefinitionKind> {
        match kind {
            ItemKindFact::Struct => Some(SpacingDefinitionKind::Struct),
            ItemKindFact::Enum => Some(SpacingDefinitionKind::Enum),
            ItemKindFact::Union => Some(SpacingDefinitionKind::Union),
            ItemKindFact::TypeAlias => Some(SpacingDefinitionKind::TypeAlias),
            ItemKindFact::Trait => Some(SpacingDefinitionKind::Trait),
            ItemKindFact::Constant => Some(SpacingDefinitionKind::Constant),
            ItemKindFact::Static => Some(SpacingDefinitionKind::Static),
            ItemKindFact::Function => Some(SpacingDefinitionKind::Function),
            ItemKindFact::Impl => Some(SpacingDefinitionKind::Impl),
            _ => None,
        }
    }
}

fn namespace_name(namespace: &[String]) -> String {
    if namespace.is_empty() {
        "crate".to_owned()
    } else {
        format!("crate::{}", namespace.join("::"))
    }
}
