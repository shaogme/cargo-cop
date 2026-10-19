use cop_cfg::CfgCondition;

use crate::{cfg_eval::CfgDecision, ConfiguredAst};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfiguredAttribute {
    start: usize,
    end: usize,
    condition: Option<String>,
    decision: CfgDecision,
}

pub(super) struct LintAttributeCollector;

impl ConfiguredAttribute {
    pub(crate) fn new(
        start: usize,
        end: usize,
        condition: Option<String>,
        decision: CfgDecision,
    ) -> Self {
        Self {
            start,
            end,
            condition,
            decision,
        }
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn condition(&self) -> Option<&str> {
        self.condition.as_deref()
    }

    pub fn decision(&self) -> CfgDecision {
        self.decision
    }
}

impl LintAttributeCollector {
    pub(super) fn collect(configured_ast: &ConfiguredAst) -> Vec<ConfiguredAttribute> {
        let mut relaxations = configured_ast
            .nodes()
            .iter()
            .filter(|node| node.decision() != CfgDecision::False)
            .flat_map(|node| {
                node.attribute_evaluation()
                    .lint_relaxations()
                    .iter()
                    .filter_map(move |relaxation| {
                        let decision = node.decision().and(relaxation.decision());
                        (decision != CfgDecision::False).then(|| {
                            ConfiguredAttribute::new(
                                relaxation.start(),
                                relaxation.end(),
                                merge_condition(
                                    node.effective_condition().map(str::to_owned),
                                    &relaxation
                                        .condition()
                                        .map(str::to_owned)
                                        .into_iter()
                                        .collect::<Vec<_>>(),
                                ),
                                decision,
                            )
                        })
                    })
            })
            .collect::<Vec<_>>();
        relaxations.sort_unstable_by_key(|attribute| (attribute.start(), attribute.end()));
        relaxations.dedup();
        relaxations
    }
}

fn merge_condition(parent: Option<String>, conditions: &[String]) -> Option<String> {
    let all = parent
        .into_iter()
        .chain(conditions.iter().cloned())
        .collect::<Vec<_>>();
    if all.is_empty() {
        return None;
    }
    let parsed = all
        .iter()
        .map(|condition| CfgCondition::parse(condition))
        .collect::<Result<Vec<_>, _>>();
    match parsed {
        Ok(conditions) => Some(CfgCondition::all(conditions).normalized()),
        Err(_) => Some(format!("all({})", all.join(", "))),
    }
}
