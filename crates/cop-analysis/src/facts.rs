mod api_surface;
mod collector;
mod records;

use std::{collections::BTreeMap, mem, ops::Range, path::PathBuf};

use cop_cargo::SelectedPackage;
use cop_syntax::{AstNodeId, CoverageStatus, SyntaxNode, SyntaxNodeKind, TypeSyntaxKind};

pub use collector::ReferenceKind;
pub use records::{CfgMacroFact, CoverageRegion, CoverageRegionKind, ImplOwnerRecord};

use api_surface::{ApiSurfaceCollector, ApiSurfaceRange, PendingDeclaration};
use collector::SyntaxFactCollector;
use records::ImplOwnerRecordData;

use crate::{
    module_graph::ModuleNode,
    symbol_index::{
        DefinitionId, PathResolution, SymbolIndex, SymbolIndexBuilder, SymbolIndexSupport,
        SymbolKind, SymbolRecord, SymbolVisibility,
    },
    syntax_facts::{ModuleSyntaxFacts, SyntaxFactsCollector},
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ModuleId(usize);

impl ModuleId {
    pub fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ReferenceId(usize);

impl ReferenceId {
    pub fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisReference {
    id: ReferenceId,
    module_id: ModuleId,
    kind: ReferenceKind,
    segments: Vec<String>,
    bounds: Range<usize>,
    condition: Option<String>,
    supported: bool,
    alias: Option<String>,
    glob: bool,
    resolution: PathResolution,
    prefix_resolutions: Vec<PathResolution>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiDependency {
    source: DefinitionId,
    target: DefinitionId,
    reference: ReferenceId,
    visible_scope: Vec<String>,
    condition: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclarationFact {
    module_id: ModuleId,
    node_id: AstNodeId,
    visibility_bounds: Range<usize>,
    visibility: SymbolVisibility,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ModuleKey {
    context_index: usize,
    target_name: String,
    namespace: Vec<String>,
    file_path: PathBuf,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct DefinitionLocation {
    module_id: ModuleId,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AnalysisFacts {
    symbols: SymbolIndex,
    references: Vec<AnalysisReference>,
    references_by_module: Vec<Vec<ReferenceId>>,
    references_by_definition: BTreeMap<DefinitionId, Vec<ReferenceId>>,
    module_ids: BTreeMap<ModuleKey, ModuleId>,
    coverage_regions: Vec<CoverageRegion>,
    impl_owners: Vec<ImplOwnerRecord>,
    impls_by_definition: BTreeMap<DefinitionId, Vec<usize>>,
    api_dependencies: BTreeMap<DefinitionId, Vec<ApiDependency>>,
    type_alias_targets: BTreeMap<DefinitionId, PathResolution>,
    syntax_by_module: Vec<ModuleSyntaxFacts>,
    definitions_by_location: BTreeMap<DefinitionLocation, DefinitionId>,
    declarations_by_definition: BTreeMap<DefinitionId, DeclarationFact>,
    pending_declarations: Vec<PendingDeclaration>,
    api_ranges_by_declaration: BTreeMap<(usize, AstNodeId), Vec<ApiSurfaceRange>>,
    cfg_macros: Vec<CfgMacroFact>,
}

impl AnalysisReference {
    pub fn id(&self) -> ReferenceId {
        self.id
    }

    pub fn module_id(&self) -> ModuleId {
        self.module_id
    }

    pub fn kind(&self) -> ReferenceKind {
        self.kind
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn condition(&self) -> Option<&str> {
        self.condition.as_deref()
    }

    pub fn is_supported(&self) -> bool {
        self.supported
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn is_glob(&self) -> bool {
        self.glob
    }

    pub fn resolution(&self) -> &PathResolution {
        &self.resolution
    }

    pub fn prefix_resolutions(&self) -> &[PathResolution] {
        &self.prefix_resolutions
    }
}

impl ApiDependency {
    pub fn source(&self) -> DefinitionId {
        self.source
    }

    pub fn target(&self) -> DefinitionId {
        self.target
    }

    pub fn reference(&self) -> ReferenceId {
        self.reference
    }

    pub fn visible_scope(&self) -> &[String] {
        &self.visible_scope
    }

    pub fn condition(&self) -> Option<&str> {
        self.condition.as_deref()
    }
}

impl DeclarationFact {
    pub fn module_id(&self) -> ModuleId {
        self.module_id
    }

    pub fn node_id(&self) -> AstNodeId {
        self.node_id
    }

    pub fn visibility_bounds(&self) -> Range<usize> {
        self.visibility_bounds.clone()
    }

    pub fn visibility(&self) -> &SymbolVisibility {
        &self.visibility
    }
}

impl AnalysisFacts {
    pub(crate) fn build(package: &SelectedPackage, modules: &[ModuleNode]) -> Self {
        let mut facts = Self {
            references_by_module: vec![Vec::new(); modules.len()],
            ..Self::default()
        };
        let mut symbol_builder = SymbolIndexBuilder::new(package, modules);
        let mut api_surface_collector = ApiSurfaceCollector::default();
        let mut raw_references_by_module = Vec::with_capacity(modules.len());
        for (index, module) in modules.iter().enumerate() {
            facts.module_ids.insert(module_key(module), ModuleId(index));
        }
        for (index, module) in modules.iter().enumerate() {
            let module_id = ModuleId(index);
            facts.cfg_macros.extend(
                module
                    .configured_ast()
                    .cfg_macros()
                    .iter()
                    .map(|cfg_macro| {
                        CfgMacroFact::new(
                            module_id,
                            cfg_macro.ast_node_id(),
                            cfg_macro.predicate().clone(),
                            cfg_macro.predicate_bounds(),
                            cfg_macro.decision(),
                        )
                    }),
            );
            let mut syntax_collector = SyntaxFactsCollector::new(module);
            let mut reference_collector = SyntaxFactCollector::default();
            symbol_builder.begin_module();
            for node in module.parse_output().syntax_nodes() {
                syntax_collector.visit_node(module, node);
                reference_collector.visit_node(module, node);
                facts.collect_coverage_node(module, module_id, node);
                facts.collect_impl_owner_node(module, module_id, node);
                api_surface_collector.visit_node(index, module, node);
                symbol_builder.visit_node(index, module, node);
            }
            symbol_builder.finish_module(module);
            facts.syntax_by_module.push(syntax_collector.finish(module));
            raw_references_by_module.push(reference_collector.finish(module));
        }
        facts.pending_declarations = api_surface_collector.take_pending_declarations();
        facts.api_ranges_by_declaration = api_surface_collector.finish_ranges();
        facts.collect_parser_issues(modules);
        facts.symbols = symbol_builder.finish(modules);
        for (index, module) in modules.iter().enumerate() {
            let module_id = ModuleId(index);
            for raw in mem::take(&mut raw_references_by_module[index]) {
                let resolution =
                    facts
                        .symbols
                        .resolve_path_at(module, &raw.segments, Some(raw.bounds.clone()));
                let mut prefix_resolutions = Vec::with_capacity(raw.segments.len());
                for prefix_length in 1..=raw.segments.len() {
                    let prefix = if prefix_length == raw.segments.len() {
                        resolution.clone()
                    } else {
                        facts.symbols.resolve_path_at(
                            module,
                            &raw.segments[..prefix_length],
                            Some(raw.bounds.clone()),
                        )
                    };
                    prefix_resolutions.push(prefix);
                }
                let id = ReferenceId(facts.references.len());
                let reference = AnalysisReference {
                    id,
                    module_id,
                    kind: raw.kind,
                    segments: raw.segments,
                    bounds: raw.bounds,
                    condition: raw.condition,
                    supported: raw.supported,
                    alias: raw.alias,
                    glob: raw.glob,
                    resolution,
                    prefix_resolutions,
                };
                facts.record_reference_relations(&reference);
                facts.references_by_module[index].push(id);
                facts.references.push(reference);
            }
            facts.resolve_impl_owners(module, module_id);
        }
        facts.resolve_type_alias_targets(modules);
        for symbol in facts.symbols.symbols() {
            let Some(module) = declaration_module_for(modules, symbol) else {
                continue;
            };
            let Some(module_id) = facts.module_id(module) else {
                continue;
            };
            let Some(bounds) = symbol.span().map(|span| span.byte_range()) else {
                continue;
            };
            facts.definitions_by_location.insert(
                DefinitionLocation {
                    module_id,
                    start: bounds.start,
                    end: bounds.end,
                },
                symbol.definition_id(),
            );
        }
        facts.finish_declarations(modules);
        facts.collect_api_dependencies(modules);
        facts.cfg_macros.sort_by(|left, right| {
            left.module_id()
                .cmp(&right.module_id())
                .then_with(|| {
                    left.predicate_bounds()
                        .start
                        .cmp(&right.predicate_bounds().start)
                })
                .then_with(|| {
                    left.predicate_bounds()
                        .end
                        .cmp(&right.predicate_bounds().end)
                })
                .then_with(|| left.ast_node_id().cmp(&right.ast_node_id()))
        });
        facts
    }

    pub fn symbols(&self) -> &SymbolIndex {
        &self.symbols
    }

    pub fn references(&self) -> &[AnalysisReference] {
        &self.references
    }

    pub fn references_for_module(
        &self,
        module: &ModuleNode,
    ) -> impl Iterator<Item = &AnalysisReference> {
        self.module_id(module)
            .into_iter()
            .flat_map(|module_id| self.references_by_module[module_id.0].iter())
            .filter_map(|reference_id| self.references.get(reference_id.0))
    }

    pub fn reference_at(
        &self,
        module: &ModuleNode,
        bounds: Range<usize>,
    ) -> Option<&AnalysisReference> {
        self.references_for_module(module)
            .find(|reference| reference.bounds == bounds && reference.kind != ReferenceKind::Use)
    }

    pub fn use_references_for_module(
        &self,
        module: &ModuleNode,
    ) -> impl Iterator<Item = &AnalysisReference> {
        self.references_for_module(module)
            .filter(|reference| reference.kind == ReferenceKind::Use)
    }

    pub fn references_to(&self, definition: DefinitionId) -> &[ReferenceId] {
        self.references_by_definition
            .get(&definition)
            .map_or(&[], Vec::as_slice)
    }

    pub fn coverage_regions(&self) -> &[CoverageRegion] {
        &self.coverage_regions
    }

    pub fn cfg_macros(&self) -> &[CfgMacroFact] {
        &self.cfg_macros
    }

    pub fn impl_owners(&self) -> &[ImplOwnerRecord] {
        &self.impl_owners
    }

    pub fn api_dependencies_for(&self, definition: DefinitionId) -> &[ApiDependency] {
        self.api_dependencies
            .get(&definition)
            .map_or(&[], Vec::as_slice)
    }

    pub fn impls_for_definition(
        &self,
        definition: DefinitionId,
    ) -> impl Iterator<Item = &ImplOwnerRecord> {
        self.impls_by_definition
            .get(&definition)
            .into_iter()
            .flatten()
            .filter_map(|index| self.impl_owners.get(*index))
    }

    pub fn impls_for_module(&self, module: &ModuleNode) -> impl Iterator<Item = &ImplOwnerRecord> {
        self.module_id(module).into_iter().flat_map(|module_id| {
            self.impl_owners
                .iter()
                .filter(move |item| item.module_id() == module_id)
        })
    }

    pub fn module_id(&self, module: &ModuleNode) -> Option<ModuleId> {
        self.module_ids.get(&module_key(module)).copied()
    }

    pub fn syntax_for_module(&self, module: &ModuleNode) -> Option<&ModuleSyntaxFacts> {
        self.module_id(module)
            .and_then(|module_id| self.syntax_by_module.get(module_id.0))
    }

    pub fn definition(&self, id: DefinitionId) -> Option<&SymbolRecord> {
        self.symbols.symbols().get(id.index())
    }

    pub fn definition_at(
        &self,
        module: &ModuleNode,
        bounds: Range<usize>,
    ) -> Option<&SymbolRecord> {
        let module_id = self.module_id(module)?;
        let definition_id = self.definitions_by_location.get(&DefinitionLocation {
            module_id,
            start: bounds.start,
            end: bounds.end,
        })?;
        self.definition(*definition_id)
    }

    pub fn type_alias_target(&self, definition: DefinitionId) -> Option<&PathResolution> {
        self.type_alias_targets.get(&definition)
    }

    pub fn declaration_for_definition(&self, definition: DefinitionId) -> Option<&DeclarationFact> {
        self.declarations_by_definition.get(&definition)
    }

    pub fn has_impl_for(&self, definition: DefinitionId, condition: Option<&str>) -> bool {
        self.impls_for_definition(definition).any(|implementation| {
            SymbolIndexSupport::conditions_may_coexist(implementation.condition(), condition)
        })
    }

    fn record_reference_relations(&mut self, reference: &AnalysisReference) {
        let mut definitions = BTreeMap::new();
        for resolution in &reference.prefix_resolutions {
            if let Some(symbol) = resolution.symbol() {
                definitions.insert(symbol.definition_id(), ());
            }
            for symbol in resolution.alias_symbols() {
                definitions.insert(symbol.definition_id(), ());
            }
        }
        for definition in definitions.keys() {
            self.references_by_definition
                .entry(*definition)
                .or_default()
                .push(reference.id);
        }
    }

    fn collect_coverage_node(
        &mut self,
        module: &ModuleNode,
        module_id: ModuleId,
        node: &SyntaxNode,
    ) {
        let coverage = module.configured_ast().coverage_for_range(
            module.parse_output(),
            node.start(),
            node.end(),
        );
        if matches!(node.kind(), SyntaxNodeKind::Opaque | SyntaxNodeKind::Macro)
            && coverage != CoverageStatus::Complete
            && !SyntaxFactCollector::is_disabled(module, node.start(), node.end())
            && SyntaxFactCollector::is_in_module_scope(module, node)
        {
            self.coverage_regions.push(CoverageRegion::new(
                module_id,
                node.start()..node.end(),
                module
                    .configured_ast()
                    .condition_for_span(node.start(), node.end())
                    .map(str::to_owned),
                if node.kind() == SyntaxNodeKind::Opaque {
                    CoverageRegionKind::OpaqueSyntax
                } else {
                    CoverageRegionKind::MacroInvocation
                },
            ));
        }
    }

    fn collect_parser_issues(&mut self, modules: &[ModuleNode]) {
        for (index, module) in modules.iter().enumerate() {
            for issue in module.parse_output().issues() {
                if SyntaxFactCollector::is_disabled(module, issue.start(), issue.end()) {
                    continue;
                }
                self.coverage_regions.push(CoverageRegion::new(
                    ModuleId(index),
                    issue.start()..issue.end(),
                    module
                        .configured_ast()
                        .condition_for_span(issue.start(), issue.end())
                        .map(str::to_owned),
                    CoverageRegionKind::ParserIssue,
                ));
            }
        }
    }

    fn collect_impl_owner_node(
        &mut self,
        module: &ModuleNode,
        module_id: ModuleId,
        item: &SyntaxNode,
    ) {
        let Some(syntax) = item.impl_syntax() else {
            return;
        };
        if !SyntaxFactCollector::is_in_module_scope(module, item) {
            return;
        }
        let target = syntax.self_type();
        let target_bounds = target.map(|target| target.bounds());
        let target_path = target.and_then(SymbolIndexSupport::type_path);
        let trait_path = syntax.trait_path();
        let trait_bounds = trait_path.map(|path| path.bounds());
        self.impl_owners
            .push(ImplOwnerRecord::new(ImplOwnerRecordData {
                module_id,
                bounds: item.start()..item.end(),
                condition: module
                    .configured_ast()
                    .condition_for_span(item.start(), item.end())
                    .map(str::to_owned),
                owner: None,
                target_bounds,
                target_path,
                trait_bounds: trait_bounds.clone(),
                trait_path: trait_path.map(SymbolIndexSupport::path_segments),
                target_enabled: module
                    .configured_ast()
                    .is_enabled_range(item.start(), item.end()),
                target_supported: target.is_some_and(|target| {
                    module.configured_ast().is_supported_range(
                        module.parse_output(),
                        target.bounds().start,
                        target.bounds().end,
                    )
                }),
                target_is_structural: target.is_some_and(|target| {
                    matches!(
                        target.kind(),
                        TypeSyntaxKind::Reference { .. }
                            | TypeSyntaxKind::Pointer { .. }
                            | TypeSyntaxKind::Tuple(_)
                            | TypeSyntaxKind::Slice(_)
                            | TypeSyntaxKind::Array { .. }
                            | TypeSyntaxKind::FunctionPointer { .. }
                            | TypeSyntaxKind::Never
                    )
                }),
                trait_enabled: syntax.is_trait_impl()
                    && module
                        .configured_ast()
                        .is_enabled_range(item.start(), item.end()),
                trait_supported: trait_bounds.as_ref().is_some_and(|bounds| {
                    module.configured_ast().is_supported_range(
                        module.parse_output(),
                        bounds.start,
                        bounds.end,
                    )
                }),
                trait_impl: syntax.is_trait_impl(),
            }));
    }

    fn resolve_type_alias_targets(&mut self, modules: &[ModuleNode]) {
        let aliases = self
            .symbols
            .symbols()
            .iter()
            .filter(|symbol| symbol.kind() == SymbolKind::TypeAlias)
            .cloned()
            .collect::<Vec<_>>();
        for alias in aliases {
            let (Some(target_path), Some(bounds)) = (
                alias.target_path(),
                alias.span().map(|span| span.byte_range()),
            ) else {
                continue;
            };
            let Some(module) = declaration_module_for(modules, &alias) else {
                continue;
            };
            let resolution = self
                .references_for_module(module)
                .filter(|reference| {
                    reference.kind != ReferenceKind::Use
                        && reference.segments == target_path
                        && bounds.start <= reference.bounds.start
                        && reference.bounds.end <= bounds.end
                })
                .min_by_key(|reference| reference.bounds.end - reference.bounds.start)
                .map(|reference| reference.resolution.clone());
            if let Some(resolution) = resolution {
                self.type_alias_targets
                    .insert(alias.definition_id(), resolution);
            }
        }
    }

    fn resolve_impl_owners(&mut self, module: &ModuleNode, module_id: ModuleId) {
        for record_index in 0..self.impl_owners.len() {
            let Some(implementation) = self.impl_owners.get(record_index) else {
                continue;
            };
            if implementation.module_id() != module_id {
                continue;
            }
            let (Some(target_path), Some(target_bounds)) =
                (implementation.target_path(), implementation.target_bounds())
            else {
                continue;
            };
            let target_path = target_path.to_vec();
            let owner = self
                .references_for_module(module)
                .filter(|reference| {
                    reference.kind != ReferenceKind::Use
                        && reference.segments == target_path
                        && target_bounds.start <= reference.bounds.start
                        && reference.bounds.end <= target_bounds.end
                })
                .min_by_key(|reference| reference.bounds.end - reference.bounds.start)
                .and_then(|reference| reference.resolution.symbol())
                .map(SymbolRecord::definition_id);
            if let Some(implementation) = self.impl_owners.get_mut(record_index) {
                implementation.set_owner(owner);
            }
            if let Some(owner) = owner {
                self.impls_by_definition
                    .entry(owner)
                    .or_default()
                    .push(record_index);
            }
        }
    }

    fn finish_declarations(&mut self, modules: &[ModuleNode]) {
        let candidates = self.symbols.symbols().iter().filter(|symbol| {
            matches!(
                symbol.visibility(),
                SymbolVisibility::Restricted(scope) if scope == "(crate)"
            )
        });
        for symbol in candidates {
            let Some(module) = declaration_module_for(modules, symbol) else {
                continue;
            };
            let Some(module_id) = self.module_id(module) else {
                continue;
            };
            let Some(symbol_bounds) = symbol.span().map(|span| span.byte_range()) else {
                continue;
            };
            let Some(declaration) = self
                .pending_declarations
                .iter()
                .filter(|declaration| {
                    declaration.module_index == module_id.index()
                        && declaration.bounds.start <= symbol_bounds.start
                        && symbol_bounds.end <= declaration.bounds.end
                })
                .min_by_key(|declaration| {
                    declaration
                        .bounds
                        .end
                        .saturating_sub(declaration.bounds.start)
                })
            else {
                continue;
            };
            self.declarations_by_definition.insert(
                symbol.definition_id(),
                DeclarationFact {
                    module_id,
                    node_id: declaration.node_id,
                    visibility_bounds: declaration.visibility_bounds.clone(),
                    visibility: declaration.visibility.clone(),
                },
            );
        }
    }

    fn collect_api_dependencies(&mut self, modules: &[ModuleNode]) {
        let candidates = self
            .symbols
            .symbols()
            .iter()
            .filter(|symbol| {
                matches!(
                    symbol.visibility(),
                    SymbolVisibility::Restricted(scope) if scope == "(crate)"
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        for candidate in candidates {
            let Some(declaration) = self
                .declarations_by_definition
                .get(&candidate.definition_id())
            else {
                continue;
            };
            let Some(module) = modules.get(declaration.module_id.index()) else {
                continue;
            };
            let Some(symbol_bounds) = candidate.span().map(|span| span.byte_range()) else {
                continue;
            };
            let references = if candidate.kind() == SymbolKind::UseAlias {
                let visible_scope = candidate
                    .visibility()
                    .visible_scope(candidate.namespace())
                    .unwrap_or_default();
                self.use_references_for_module(module)
                    .filter(|reference| reference.bounds == symbol_bounds)
                    .map(|reference| (reference, visible_scope.clone()))
                    .collect::<Vec<_>>()
            } else {
                let Some(ranges) = self
                    .api_ranges_by_declaration
                    .get(&(declaration.module_id.index(), declaration.node_id))
                else {
                    continue;
                };
                self.references_for_module(module)
                    .filter(|reference| reference.kind != ReferenceKind::Use)
                    .flat_map(|reference| {
                        ranges
                            .iter()
                            .filter(|range| {
                                range.bounds.start <= reference.bounds.start
                                    && reference.bounds.end <= range.bounds.end
                            })
                            .map(move |range| (reference, range.visible_scope.clone()))
                    })
                    .collect::<Vec<_>>()
            };
            let mut dependency_facts = Vec::new();
            for (reference, visible_scope) in references {
                for resolution in &reference.prefix_resolutions {
                    if let Some(symbol) = resolution.symbol() {
                        dependency_facts.push(ApiDependency {
                            source: candidate.definition_id(),
                            target: symbol.definition_id(),
                            reference: reference.id,
                            visible_scope: visible_scope.clone(),
                            condition: reference.condition.clone(),
                        });
                    }
                    for alias in resolution.alias_symbols() {
                        dependency_facts.push(ApiDependency {
                            source: candidate.definition_id(),
                            target: alias.definition_id(),
                            reference: reference.id,
                            visible_scope: visible_scope.clone(),
                            condition: reference.condition.clone(),
                        });
                    }
                }
            }
            let dependencies = self
                .api_dependencies
                .entry(candidate.definition_id())
                .or_default();
            dependencies.extend(dependency_facts);
            dependencies.sort_by(|left, right| {
                left.target
                    .cmp(&right.target)
                    .then_with(|| left.reference.cmp(&right.reference))
                    .then_with(|| left.visible_scope.cmp(&right.visible_scope))
                    .then_with(|| left.condition.cmp(&right.condition))
            });
            dependencies.dedup();
        }
    }
}

fn module_key(module: &ModuleNode) -> ModuleKey {
    ModuleKey {
        context_index: module.context_index(),
        target_name: module.target_name().to_owned(),
        namespace: module.namespace().to_vec(),
        file_path: module.file_path().to_path_buf(),
    }
}

fn declaration_module_for<'a>(
    modules: &'a [ModuleNode],
    symbol: &SymbolRecord,
) -> Option<&'a ModuleNode> {
    let namespace = if symbol.kind() == SymbolKind::AssociatedItem {
        &symbol.namespace()[..symbol.namespace().len().checked_sub(1)?]
    } else {
        symbol.namespace()
    };
    modules.iter().find(|module| {
        module.context_index() == symbol.context_index()
            && module.target_name() == symbol.target_name()
            && module.file_path() == symbol.file_path()
            && module.namespace() == namespace
    })
}
