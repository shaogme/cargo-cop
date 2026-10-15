#![forbid(unsafe_code)]

mod analysis;
mod cfg_eval;
mod module_graph;
mod symbol_index;

pub use analysis::{AnalysisRequest, AnalysisSnapshot, ProjectAnalyzer};
pub use cfg_eval::CfgDecision;
pub use module_graph::ModuleNode;
pub use symbol_index::{
    PathResolution, PathResolutionKind, SymbolIndex, SymbolKind, SymbolRecord, SymbolVisibility,
    VisibilityResult,
};
