use std::{collections::BTreeMap, ops::Range, slice::from_ref};

use cop_cargo::TargetContext;
use cop_cfg::CfgCondition;
use cop_syntax::{AstNodeId, Attribute, CoverageStatus, ParseIssue, ParseOutput, SyntaxNodeKind};

use crate::cfg_eval::{AttributeEffects, CfgDecision, CfgEvaluator};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfiguredAstIssueKind {
    UnknownCfg,
    MalformedAttribute,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfiguredAstIssue {
    ast_node_id: AstNodeId,
    kind: ConfiguredAstIssueKind,
    start: usize,
    end: usize,
    condition: Option<String>,
    effective_condition: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfiguredLintRelaxation {
    condition: Option<String>,
    decision: CfgDecision,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfiguredPathAttribute {
    condition: Option<String>,
    decision: CfgDecision,
    path: Option<String>,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfiguredAttributeEvaluation {
    decision: Option<CfgDecision>,
    path: Option<String>,
    path_unknown: bool,
    conditions: Vec<String>,
    lint_relaxations: Vec<ConfiguredLintRelaxation>,
    path_attributes: Vec<ConfiguredPathAttribute>,
    malformed_ranges: Vec<Range<usize>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfiguredNode {
    ast_node_id: AstNodeId,
    parent: Option<AstNodeId>,
    kind: SyntaxNodeKind,
    start: usize,
    end: usize,
    attributes: Vec<Attribute>,
    inside_macro_input: bool,
    decision: CfgDecision,
    local_decision: CfgDecision,
    effective_condition: Option<String>,
    attribute_evaluation: ConfiguredAttributeEvaluation,
    issues: Vec<ConfiguredAstIssue>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ConfiguredRangeIndex {
    sorted_node_indices: Vec<usize>,
    max_end_tree: Vec<usize>,
    leaf_count: usize,
    exact_ranges: BTreeMap<(usize, usize), Vec<usize>>,
}

struct ConfiguredRangeQuery<'a> {
    index: &'a ConfiguredRangeIndex,
    nodes: &'a [ConfiguredNode],
    limit: usize,
    end: usize,
    best: Option<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfiguredAst {
    nodes: Vec<ConfiguredNode>,
    by_id: BTreeMap<AstNodeId, usize>,
    range_index: ConfiguredRangeIndex,
    issues: Vec<ConfiguredAstIssue>,
}

impl ConfiguredNode {
    pub fn ast_node_id(&self) -> AstNodeId {
        self.ast_node_id
    }

    pub fn parent(&self) -> Option<AstNodeId> {
        self.parent
    }

    pub fn kind(&self) -> SyntaxNodeKind {
        self.kind
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    pub fn decision(&self) -> CfgDecision {
        self.decision
    }

    pub fn local_decision(&self) -> CfgDecision {
        self.local_decision
    }

    pub fn effective_condition(&self) -> Option<&str> {
        self.effective_condition.as_deref()
    }

    fn inside_macro_input(&self) -> bool {
        self.inside_macro_input
    }

    pub fn attribute_evaluation(&self) -> &ConfiguredAttributeEvaluation {
        &self.attribute_evaluation
    }

    pub fn issues(&self) -> &[ConfiguredAstIssue] {
        &self.issues
    }
}

impl ConfiguredAttributeEvaluation {
    fn from_effects(effects: &AttributeEffects) -> Self {
        Self {
            decision: effects.decision,
            path: effects.path.clone(),
            path_unknown: effects.path_unknown,
            conditions: effects.conditions.clone(),
            lint_relaxations: effects
                .lint_relaxations
                .iter()
                .map(|relaxation| ConfiguredLintRelaxation {
                    condition: relaxation.condition.clone(),
                    decision: relaxation.decision,
                    start: relaxation.start,
                    end: relaxation.end,
                })
                .collect(),
            path_attributes: effects
                .path_attributes
                .iter()
                .map(|path| ConfiguredPathAttribute {
                    condition: path.condition.clone(),
                    decision: path.decision,
                    path: path.path.clone(),
                    start: path.start,
                    end: path.end,
                })
                .collect(),
            malformed_ranges: effects.malformed_ranges.clone(),
        }
    }

    pub fn decision(&self) -> Option<CfgDecision> {
        self.decision
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    pub fn path_unknown(&self) -> bool {
        self.path_unknown
    }

    pub fn conditions(&self) -> &[String] {
        &self.conditions
    }

    pub fn lint_relaxations(&self) -> &[ConfiguredLintRelaxation] {
        &self.lint_relaxations
    }

    pub fn path_attributes(&self) -> &[ConfiguredPathAttribute] {
        &self.path_attributes
    }

    pub fn malformed_ranges(&self) -> &[Range<usize>] {
        &self.malformed_ranges
    }

    pub fn is_malformed(&self) -> bool {
        !self.malformed_ranges.is_empty()
    }
}

impl ConfiguredLintRelaxation {
    pub fn condition(&self) -> Option<&str> {
        self.condition.as_deref()
    }

    pub fn decision(&self) -> CfgDecision {
        self.decision
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }
}

impl ConfiguredPathAttribute {
    pub fn condition(&self) -> Option<&str> {
        self.condition.as_deref()
    }

    pub fn decision(&self) -> CfgDecision {
        self.decision
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }
}

impl ConfiguredAstIssue {
    pub fn ast_node_id(&self) -> AstNodeId {
        self.ast_node_id
    }

    pub fn kind(&self) -> ConfiguredAstIssueKind {
        self.kind
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

    pub fn effective_condition(&self) -> Option<&str> {
        self.effective_condition.as_deref()
    }
}

impl ConfiguredAst {
    pub(crate) fn project(
        output: &ParseOutput,
        target: &TargetContext,
        inherited_condition: Option<&str>,
    ) -> Self {
        let evaluator = CfgEvaluator::new(target);
        let inherited_decision = inherited_condition.map_or(CfgDecision::True, |condition| {
            evaluator.evaluate_condition(condition)
        });
        let mut projected = Self::default();
        for node in output.syntax_nodes() {
            let parent_inside_macro_input = node
                .parent()
                .and_then(|parent| projected.node(parent))
                .is_some_and(|parent_node| {
                    parent_node.inside_macro_input()
                        || parent_node.kind() == SyntaxNodeKind::MacroInput
                });
            let (parent_decision, parent_condition) = match node.parent() {
                Some(parent) => match projected.node(parent) {
                    Some(parent_node) => (
                        parent_node.decision(),
                        parent_node.effective_condition().map(str::to_owned),
                    ),
                    None => (CfgDecision::Unknown, None),
                },
                None => (inherited_decision, inherited_condition.map(str::to_owned)),
            };
            let attributes = if parent_inside_macro_input {
                &[]
            } else {
                node.attributes()
            };
            let effects = evaluator.evaluate_attributes(attributes, output.source());
            let evaluated_decision = effects.decision.unwrap_or(CfgDecision::True);
            let local_decision = if effects.malformed {
                evaluated_decision.and(CfgDecision::Unknown)
            } else {
                evaluated_decision
            };
            let decision = parent_decision.and(local_decision);
            let effective_condition =
                merge_condition(parent_condition.clone(), &effects.conditions);
            let mut node_issues = Vec::new();
            if parent_decision != CfgDecision::False {
                for bounds in &effects.malformed_ranges {
                    node_issues.push(ConfiguredAstIssue {
                        ast_node_id: node.id(),
                        kind: ConfiguredAstIssueKind::MalformedAttribute,
                        start: bounds.start,
                        end: bounds.end,
                        condition: None,
                        effective_condition: effective_condition.clone(),
                    });
                }
                if decision != CfgDecision::False {
                    for unknown in &effects.unknown {
                        node_issues.push(ConfiguredAstIssue {
                            ast_node_id: node.id(),
                            kind: ConfiguredAstIssueKind::UnknownCfg,
                            start: unknown.start,
                            end: unknown.end,
                            condition: Some(unknown.condition.clone()),
                            effective_condition: merge_condition(
                                parent_condition.clone(),
                                from_ref(&unknown.condition),
                            ),
                        });
                    }
                }
            }
            projected.issues.extend(node_issues.iter().cloned());
            let index = projected.nodes.len();
            projected.by_id.insert(node.id(), index);
            projected.nodes.push(ConfiguredNode {
                ast_node_id: node.id(),
                parent: node.parent(),
                kind: node.kind(),
                start: node.start(),
                end: node.end(),
                attributes: node.attributes().to_vec(),
                inside_macro_input: parent_inside_macro_input
                    || node.kind() == SyntaxNodeKind::MacroInput,
                decision,
                local_decision,
                effective_condition,
                attribute_evaluation: ConfiguredAttributeEvaluation::from_effects(&effects),
                issues: node_issues,
            });
        }
        projected.range_index = ConfiguredRangeIndex::new(&projected.nodes);
        projected
    }

    pub fn nodes(&self) -> &[ConfiguredNode] {
        &self.nodes
    }

    pub fn issues(&self) -> &[ConfiguredAstIssue] {
        &self.issues
    }

    pub fn enabled_nodes(&self) -> impl Iterator<Item = &ConfiguredNode> {
        self.nodes
            .iter()
            .filter(|node| node.decision() == CfgDecision::True)
    }

    pub fn potentially_enabled_nodes(&self) -> impl Iterator<Item = &ConfiguredNode> {
        self.nodes
            .iter()
            .filter(|node| node.decision() != CfgDecision::False)
    }

    pub fn is_enabled_range(&self, start: usize, end: usize) -> bool {
        self.node_for_range(start, end)
            .map_or(true, |node| node.decision() != CfgDecision::False)
    }

    pub fn is_issue_enabled(&self, issue: &ParseIssue) -> bool {
        if issue.boundary_unknown() {
            return true;
        }
        issue
            .source_node_id()
            .and_then(|id| self.node(id))
            .map_or_else(
                || self.is_enabled_range(issue.start(), issue.end()),
                |node| node.decision() != CfgDecision::False,
            )
    }

    pub fn coverage_for_range(
        &self,
        output: &ParseOutput,
        start: usize,
        end: usize,
    ) -> CoverageStatus {
        let mut coverage = CoverageStatus::Complete;
        for issue in output.issues().iter().filter(|issue| {
            ranges_overlap(start, end, issue.start(), issue.end()) && self.is_issue_enabled(issue)
        }) {
            coverage = merge_coverage(coverage, issue.coverage_status());
        }
        let inside_inactive_opaque = output.syntax_nodes().iter().any(|node| {
            node.kind() == SyntaxNodeKind::Opaque
                && node.start() <= start
                && end <= node.end()
                && self
                    .node(node.id())
                    .is_some_and(|configured| configured.decision() == CfgDecision::False)
        });
        if inside_inactive_opaque {
            return coverage;
        }
        for node in output.syntax_nodes().iter().filter(|node| {
            node.kind() == SyntaxNodeKind::Opaque
                && ranges_overlap(start, end, node.start(), node.end())
                && self
                    .node(node.id())
                    .is_some_and(|configured| configured.decision() != CfgDecision::False)
        }) {
            coverage = merge_coverage(coverage, node.coverage());
        }
        coverage
    }

    pub fn is_supported_range(&self, output: &ParseOutput, start: usize, end: usize) -> bool {
        self.coverage_for_range(output, start, end) == CoverageStatus::Complete
    }

    pub fn condition_for_span(&self, start: usize, end: usize) -> Option<&str> {
        self.node_for_range(start, end)
            .and_then(ConfiguredNode::effective_condition)
    }

    pub fn node(&self, id: AstNodeId) -> Option<&ConfiguredNode> {
        self.by_id.get(&id).and_then(|index| self.nodes.get(*index))
    }

    pub fn node_for_kind_range(
        &self,
        kind: SyntaxNodeKind,
        start: usize,
        end: usize,
    ) -> Option<&ConfiguredNode> {
        self.range_index
            .exact_ranges
            .get(&(start, end))?
            .iter()
            .map(|index| &self.nodes[*index])
            .find(|node| node.kind() == kind)
    }

    pub fn node_for_range(&self, start: usize, end: usize) -> Option<&ConfiguredNode> {
        if start > end {
            return None;
        }
        self.range_index
            .best_containing_range(&self.nodes, start, end)
            .and_then(|index| self.nodes.get(index))
    }
}

impl ConfiguredRangeIndex {
    fn new(nodes: &[ConfiguredNode]) -> Self {
        let mut sorted_node_indices = (0..nodes.len()).collect::<Vec<_>>();
        sorted_node_indices.sort_unstable_by(|left_index, right_index| {
            let left = &nodes[*left_index];
            let right = &nodes[*right_index];
            left.start()
                .cmp(&right.start())
                .then_with(|| left.end().cmp(&right.end()))
                .then_with(|| left.ast_node_id().cmp(&right.ast_node_id()))
        });
        let mut exact_ranges = BTreeMap::<(usize, usize), Vec<usize>>::new();
        for index in &sorted_node_indices {
            let node = &nodes[*index];
            exact_ranges
                .entry((node.start(), node.end()))
                .or_default()
                .push(*index);
        }
        let leaf_count = sorted_node_indices.len().max(1).next_power_of_two();
        let mut max_end_tree = vec![0; leaf_count * 2];
        for (leaf, index) in sorted_node_indices.iter().enumerate() {
            max_end_tree[leaf_count + leaf] = nodes[*index].end();
        }
        for index in (1..leaf_count).rev() {
            max_end_tree[index] = max_end_tree[index * 2].max(max_end_tree[index * 2 + 1]);
        }
        Self {
            sorted_node_indices,
            max_end_tree,
            leaf_count,
            exact_ranges,
        }
    }

    fn best_containing_range(
        &self,
        nodes: &[ConfiguredNode],
        start: usize,
        end: usize,
    ) -> Option<usize> {
        if let Some(exact) = self.exact_ranges.get(&(start, end)) {
            return exact.iter().copied().min_by_key(|index| {
                let node = &nodes[*index];
                (node.end().saturating_sub(node.start()), node.ast_node_id())
            });
        }
        let limit = self
            .sorted_node_indices
            .partition_point(|index| nodes[*index].start() <= start);
        let leaf_count = self.leaf_count;
        let mut query = ConfiguredRangeQuery {
            index: self,
            nodes,
            limit,
            end,
            best: None,
        };
        query.find(1, 0, leaf_count);
        query.best
    }
}

impl ConfiguredRangeQuery<'_> {
    fn find(&mut self, tree_index: usize, left: usize, right: usize) {
        if left >= self.limit || self.index.max_end_tree[tree_index] < self.end {
            return;
        }
        if right - left == 1 {
            if let Some(index) = self.index.sorted_node_indices.get(left).copied() {
                let candidate = &self.nodes[index];
                let is_better = self.best.map_or(true, |best_index| {
                    let current = &self.nodes[best_index];
                    (
                        candidate.end().saturating_sub(candidate.start()),
                        candidate.ast_node_id(),
                    ) < (
                        current.end().saturating_sub(current.start()),
                        current.ast_node_id(),
                    )
                });
                if is_better {
                    self.best = Some(index);
                }
            }
            return;
        }
        let middle = left + (right - left) / 2;
        self.find(tree_index * 2 + 1, middle, right);
        self.find(tree_index * 2, left, middle);
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

fn ranges_overlap(
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
) -> bool {
    if left_start == left_end || right_start == right_end {
        left_start <= right_end && right_start <= left_end
    } else {
        left_start < right_end && right_start < left_end
    }
}

fn merge_coverage(left: CoverageStatus, right: CoverageStatus) -> CoverageStatus {
    match (left, right) {
        (CoverageStatus::Incomplete, _) | (_, CoverageStatus::Incomplete) => {
            CoverageStatus::Incomplete
        }
        (CoverageStatus::Partial, _) | (_, CoverageStatus::Partial) => CoverageStatus::Partial,
        _ => CoverageStatus::Complete,
    }
}
