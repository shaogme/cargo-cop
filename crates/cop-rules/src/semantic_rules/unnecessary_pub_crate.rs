use cop_i18n::{t, RenderError};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ops::Range,
};

use cop_analysis::{
    AnalysisReference, AnalysisSnapshot, DefinitionId, ModuleNode, PathResolution,
    PathResolutionKind, ReferenceId, ReferenceKind, SymbolKind, SymbolRecord, SymbolVisibility,
};
use cop_model::{Diagnostic, RuleId};

use crate::support::{RuleDiagnosticAdapter, RuleReport, SyntaxSupport};

pub(super) struct UnnecessaryPubCrateRule;

#[derive(Clone)]
struct PubCrateCandidate {
    symbol: SymbolRecord,
    visibility_bounds: Range<usize>,
    private_namespace: Vec<String>,
}

struct PathScanOptions<'a> {
    condition: Option<&'a str>,
    force_required: bool,
    api_dependency: bool,
}

struct CandidateAnalysis {
    candidates: Vec<PubCrateCandidate>,
    candidate_by_definition: BTreeMap<DefinitionId, usize>,
    candidates_by_context: BTreeMap<usize, Vec<usize>>,
    candidates_by_name: BTreeMap<String, Vec<usize>>,
    candidates_by_namespace: BTreeMap<(usize, Vec<String>), Vec<usize>>,
    api_dependency_references: BTreeSet<ReferenceId>,
    required: Vec<bool>,
    uncertain: Vec<bool>,
}

impl UnnecessaryPubCrateRule {
    pub(super) fn run(
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        let mut analysis = CandidateAnalysis::new(snapshot);
        analysis.scan_references(snapshot);
        analysis.propagate_api_dependencies(snapshot);
        analysis.report(snapshot, diagnostics, adapter)
    }
}

impl CandidateAnalysis {
    fn new(snapshot: &AnalysisSnapshot) -> Self {
        let candidates = snapshot
            .symbols()
            .symbols()
            .iter()
            .filter(|symbol| {
                matches!(
                    symbol.visibility(),
                    SymbolVisibility::Restricted(scope) if scope == "(crate)"
                )
            })
            .filter_map(|symbol| Self::candidate(snapshot, symbol))
            .collect::<Vec<_>>();
        let count = candidates.len();
        let candidate_by_definition = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| (candidate.symbol.definition_id(), index))
            .collect();
        let mut candidates_by_context = BTreeMap::new();
        let mut candidates_by_name = BTreeMap::new();
        let mut candidates_by_namespace = BTreeMap::new();
        for (index, candidate) in candidates.iter().enumerate() {
            let context = candidate.symbol.context_index();
            candidates_by_context
                .entry(context)
                .or_insert_with(Vec::new)
                .push(index);
            candidates_by_name
                .entry(identifier_name(candidate.symbol.name()).to_owned())
                .or_insert_with(Vec::new)
                .push(index);
            candidates_by_namespace
                .entry((context, candidate.symbol.namespace().to_vec()))
                .or_insert_with(Vec::new)
                .push(index);
        }
        let api_dependency_references = candidates
            .iter()
            .flat_map(|candidate| {
                snapshot
                    .facts()
                    .api_dependencies_for(candidate.symbol.definition_id())
            })
            .map(|dependency| dependency.reference())
            .collect();
        Self {
            candidates,
            required: vec![false; count],
            uncertain: vec![false; count],
            candidate_by_definition,
            candidates_by_context,
            candidates_by_name,
            candidates_by_namespace,
            api_dependency_references,
        }
    }

    fn candidate(snapshot: &AnalysisSnapshot, symbol: &SymbolRecord) -> Option<PubCrateCandidate> {
        let declaration = snapshot
            .facts()
            .declaration_for_definition(symbol.definition_id())?;
        if !matches!(
            declaration.visibility(),
            SymbolVisibility::Restricted(scope) if scope == "(crate)"
        ) {
            return None;
        }
        let visibility_bounds = declaration.visibility_bounds();
        if visibility_bounds.start == visibility_bounds.end {
            return None;
        }
        let mut private_namespace = symbol.namespace().to_vec();
        if symbol.kind() == SymbolKind::AssociatedItem {
            private_namespace.pop()?;
        }
        Some(PubCrateCandidate {
            symbol: symbol.clone(),
            visibility_bounds,
            private_namespace,
        })
    }

    fn declaration_module<'a>(
        snapshot: &'a AnalysisSnapshot,
        symbol: &SymbolRecord,
    ) -> Option<&'a ModuleNode> {
        let declaration = snapshot
            .facts()
            .declaration_for_definition(symbol.definition_id())?;
        snapshot.modules().get(declaration.module_id().index())
    }

    fn scan_references(&mut self, snapshot: &AnalysisSnapshot) {
        for region in snapshot.facts().coverage_regions() {
            if let Some(module) = snapshot.modules().get(region.module_id().index()) {
                self.mark_context_uncertain(module, region.condition());
            }
        }
        for module in snapshot.modules() {
            for reference in snapshot.facts().references_for_module(module) {
                if reference.kind() == ReferenceKind::MethodCall {
                    if reference.is_supported() {
                        if let Some(method) = reference.segments().first() {
                            self.mark_method_call(module, method, reference.condition());
                        }
                    } else {
                        self.mark_context_uncertain(module, reference.condition());
                    }
                } else {
                    self.scan_path(
                        module,
                        reference,
                        PathScanOptions {
                            condition: reference.condition(),
                            force_required: false,
                            api_dependency: self
                                .api_dependency_references
                                .contains(&reference.id()),
                        },
                    );
                }
                if reference.kind() == ReferenceKind::Use && reference.is_glob() {
                    let resolution = reference.resolution();
                    if Self::is_unresolved(resolution) {
                        self.mark_context_uncertain(module, reference.condition());
                    }
                    self.record_glob_targets(resolution, module, reference.condition());
                }
            }
        }
    }

    fn mark_context_uncertain(&mut self, module: &ModuleNode, condition: Option<&str>) {
        if let Some(indices) = self.candidates_by_context.get(&module.context_index()) {
            for index in indices {
                let candidate = &self.candidates[*index];
                if SyntaxSupport::conditions_may_coexist(
                    candidate.symbol.cfg_condition(),
                    condition,
                ) {
                    self.uncertain[*index] = true;
                }
            }
        }
    }

    fn scan_path(
        &mut self,
        module: &ModuleNode,
        reference: &AnalysisReference,
        options: PathScanOptions<'_>,
    ) {
        if !reference.is_supported() {
            self.mark_context_uncertain(module, options.condition);
            return;
        }
        let condition = options.condition.or(reference.condition());
        if reference.segments().is_empty() {
            return;
        }
        for (prefix_index, resolution) in reference.prefix_resolutions().iter().enumerate() {
            if !options.api_dependency {
                self.record_resolution(resolution, module, condition, options.force_required);
            }
            if Self::is_unresolved(resolution) {
                let self_prefix_of_qualified_path = prefix_index == 0
                    && reference
                        .segments()
                        .first()
                        .is_some_and(|segment| segment == "Self")
                    && reference.segments().len() > 1;
                if !self_prefix_of_qualified_path {
                    self.mark_unresolved_candidates(
                        module,
                        &reference.segments()[..=prefix_index],
                        condition,
                    );
                }
            }
        }
    }

    fn record_resolution(
        &mut self,
        resolution: &PathResolution,
        module: &ModuleNode,
        condition: Option<&str>,
        force_required: bool,
    ) {
        let mut candidate_indices = BTreeSet::new();
        if let Some(symbol) = resolution.symbol() {
            if let Some(index) = self.candidate_by_definition.get(&symbol.definition_id()) {
                candidate_indices.insert(*index);
            }
        }
        for symbol in resolution.alias_symbols() {
            if let Some(index) = self.candidate_by_definition.get(&symbol.definition_id()) {
                candidate_indices.insert(*index);
            }
        }
        for index in candidate_indices {
            let candidate = &self.candidates[index];
            if candidate.symbol.context_index() == module.context_index()
                && SyntaxSupport::conditions_may_coexist(
                    candidate.symbol.cfg_condition(),
                    condition,
                )
                && (force_required || !module.namespace().starts_with(&candidate.private_namespace))
            {
                self.required[index] = true;
            }
        }
    }

    fn record_glob_targets(
        &mut self,
        resolution: &PathResolution,
        module: &ModuleNode,
        condition: Option<&str>,
    ) {
        let Some(target_namespace) = resolution.symbol().and_then(SymbolRecord::target_namespace)
        else {
            return;
        };
        if let Some(indices) = self
            .candidates_by_namespace
            .get(&(module.context_index(), target_namespace.to_vec()))
        {
            for index in indices {
                let candidate = &self.candidates[*index];
                if SyntaxSupport::conditions_may_coexist(
                    candidate.symbol.cfg_condition(),
                    condition,
                ) && !module.namespace().starts_with(&candidate.private_namespace)
                {
                    self.required[*index] = true;
                }
            }
        }
    }

    fn mark_method_call(&mut self, module: &ModuleNode, method: &str, condition: Option<&str>) {
        let Some(indices) = self.candidates_by_name.get(identifier_name(method)) else {
            return;
        };
        for index in indices {
            let candidate = &self.candidates[*index];
            if candidate.symbol.kind() == SymbolKind::AssociatedItem
                && candidate.symbol.context_index() == module.context_index()
                && SyntaxSupport::conditions_may_coexist(
                    candidate.symbol.cfg_condition(),
                    condition,
                )
                && !module.namespace().starts_with(&candidate.private_namespace)
            {
                self.required[*index] = true;
            }
        }
    }

    fn is_unresolved(resolution: &PathResolution) -> bool {
        matches!(
            resolution.kind(),
            PathResolutionKind::Unresolved
                | PathResolutionKind::Ambiguous
                | PathResolutionKind::AliasCycle
        )
    }

    fn mark_unresolved_candidates(
        &mut self,
        module: &ModuleNode,
        segments: &[String],
        condition: Option<&str>,
    ) {
        let mut candidates = BTreeSet::new();
        for segment in segments {
            if let Some(indices) = self.candidates_by_name.get(identifier_name(segment)) {
                candidates.extend(indices.iter().copied());
            }
        }
        for index in candidates {
            let candidate = &self.candidates[index];
            if candidate.symbol.context_index() == module.context_index()
                && SyntaxSupport::conditions_may_coexist(
                    candidate.symbol.cfg_condition(),
                    condition,
                )
            {
                self.uncertain[index] = true;
            }
        }
    }

    fn propagate_api_dependencies(&mut self, snapshot: &AnalysisSnapshot) {
        let mut pending = self
            .candidates
            .iter()
            .enumerate()
            .filter(|(index, _)| self.required[*index] && !self.uncertain[*index])
            .map(|(_, candidate)| candidate.symbol.definition_id())
            .collect::<VecDeque<_>>();
        let mut processed = BTreeSet::new();
        while let Some(source) = pending.pop_front() {
            if !processed.insert(source) {
                continue;
            }
            let Some(&source_index) = self.candidate_by_definition.get(&source) else {
                continue;
            };
            let source_candidate = &self.candidates[source_index];
            for dependency in snapshot.facts().api_dependencies_for(source) {
                if dependency.source() != source {
                    continue;
                }
                let Some(&target_index) = self.candidate_by_definition.get(&dependency.target())
                else {
                    continue;
                };
                let target = &self.candidates[target_index];
                if dependency
                    .visible_scope()
                    .starts_with(&target.private_namespace)
                    || !SyntaxSupport::conditions_may_coexist_all(&[
                        source_candidate.symbol.cfg_condition(),
                        dependency.condition(),
                        target.symbol.cfg_condition(),
                    ])
                    || self.required[target_index]
                {
                    continue;
                }
                self.required[target_index] = true;
                if !self.uncertain[target_index] {
                    pending.push_back(target.symbol.definition_id());
                }
            }
        }
    }

    fn report(
        &self,
        snapshot: &AnalysisSnapshot,
        diagnostics: &mut Vec<Diagnostic>,
        adapter: &RuleDiagnosticAdapter<'_>,
    ) -> Result<(), RenderError> {
        for (index, candidate) in self.candidates.iter().enumerate() {
            if self.required[index] || self.uncertain[index] {
                continue;
            }
            let Some(module) = Self::declaration_module(snapshot, &candidate.symbol) else {
                continue;
            };
            adapter.push(
                module,
                diagnostics,
                RuleReport::new(
                    RuleId::UnnecessaryPubCrate,
                    t!("rule.unnecessary_pub_crate", { name: candidate.symbol.name() })?,
                    Some(candidate.visibility_bounds.clone()),
                ),
            );
        }
        Ok(())
    }
}

fn identifier_name(name: &str) -> &str {
    name.strip_prefix("r#").unwrap_or(name)
}
