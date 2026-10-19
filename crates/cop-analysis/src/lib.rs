#![forbid(unsafe_code)]

mod analysis;
mod cfg_eval;
mod configured_ast;
mod module_graph;
mod symbol_index;

pub use analysis::{AnalysisRequest, AnalysisSnapshot, ProjectAnalyzer};
pub use cfg_eval::CfgDecision;
pub use configured_ast::{
    ConfiguredAst, ConfiguredAstIssue, ConfiguredAstIssueKind, ConfiguredAttributeEvaluation,
    ConfiguredLintRelaxation, ConfiguredNode, ConfiguredPathAttribute,
};
pub use cop_cfg::CfgCondition;
pub use module_graph::{ConfiguredAttribute, ModuleNode};
pub use symbol_index::{
    PathResolution, PathResolutionKind, SymbolIndex, SymbolKind, SymbolRecord, SymbolVisibility,
    VisibilityResult,
};
