#![forbid(unsafe_code)]

mod analysis;
mod cfg_eval;
mod configured_ast;
mod facts;
mod module_graph;
mod symbol_index;
mod syntax_facts;

pub use analysis::{AnalysisRequest, AnalysisSnapshot, ProjectAnalyzer};
pub use cfg_eval::CfgDecision;
pub use configured_ast::{
    ConfiguredAst, ConfiguredAstIssue, ConfiguredAstIssueKind, ConfiguredAttributeEvaluation,
    ConfiguredLintRelaxation, ConfiguredNode, ConfiguredPathAttribute,
};
pub use cop_cfg::CfgCondition;
pub use facts::{
    AnalysisFacts, AnalysisReference, ApiDependency, CoverageRegion, CoverageRegionKind,
    DeclarationFact, ImplOwnerRecord, ModuleId, ReferenceId, ReferenceKind,
};
pub use module_graph::{ConfiguredAttribute, ModuleNode};
pub use symbol_index::{
    DefinitionId, PathResolution, PathResolutionKind, SymbolIndex, SymbolKind, SymbolRecord,
    SymbolVisibility, VisibilityResult,
};
pub use syntax_facts::{
    BlockItemFact, BlockScopeFact, FieldFact, ItemFact, ItemKindFact, ItemScopeFact,
    ModuleSyntaxFacts, PathAttributeFact, ShapeParameterFact, SyntaxShapeFact, SyntaxShapeKind,
};
