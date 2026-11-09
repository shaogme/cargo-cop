mod builder;
mod issue_reporter;
mod lint_attributes;
mod macro_scan;
mod node;
mod scope;

use cop_cargo::{SelectedPackage, TargetContext};
use cop_i18n::RenderError;
use cop_model::Diagnostic;

pub use self::{lint_attributes::ConfiguredAttribute, node::ModuleNode};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ModuleGraph {
    nodes: Vec<ModuleNode>,
    diagnostics: Vec<Diagnostic>,
}

impl ModuleGraph {
    pub(crate) fn build(
        package: &SelectedPackage,
        target: &TargetContext,
        context_index: usize,
    ) -> Result<Self, RenderError> {
        let (nodes, diagnostics) =
            builder::GraphBuilder::build_graph(package, target, context_index)?;
        Ok(Self { nodes, diagnostics })
    }

    pub(crate) fn into_parts(self) -> (Vec<ModuleNode>, Vec<Diagnostic>) {
        (self.nodes, self.diagnostics)
    }
}
