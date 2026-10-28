use cop_cargo::{DependencyKind, SelectedPackage};
use cop_model::{
    AnalysisIssueKind, Diagnostic, LocalizedMessage, ScanStatus, Severity, CFG_DEPENDENCY_UNKNOWN,
};

use crate::{
    cfg_eval::{CfgDecision, CfgEvaluator},
    facts::AnalysisFacts,
    module_graph::ModuleGraph,
    ModuleNode, SymbolIndex,
};

pub struct AnalysisRequest<'a> {
    package: &'a SelectedPackage,
}

pub struct ProjectAnalyzer;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisSnapshot {
    modules: Vec<ModuleNode>,
    facts: AnalysisFacts,
    diagnostics: Vec<Diagnostic>,
    status: ScanStatus,
}

impl<'a> AnalysisRequest<'a> {
    pub fn new(package: &'a SelectedPackage) -> Self {
        Self { package }
    }

    pub fn package(&self) -> &SelectedPackage {
        self.package
    }
}

impl ProjectAnalyzer {
    pub fn analyze(request: AnalysisRequest<'_>) -> AnalysisSnapshot {
        let mut modules = Vec::new();
        let mut diagnostics = Vec::new();
        for (context_index, target) in request.package.targets().iter().enumerate() {
            let graph = ModuleGraph::build(request.package(), target, context_index);
            let (mut target_modules, mut target_diagnostics) = graph.into_parts();
            modules.append(&mut target_modules);
            diagnostics.append(&mut target_diagnostics);
            let evaluator = CfgEvaluator::new(target);
            let build_script = target.kind().iter().any(|kind| kind == "custom-build");
            for dependency in request.package.dependencies().dependencies() {
                let dependency_kind_is_active = match dependency.kind() {
                    DependencyKind::Normal => !build_script,
                    DependencyKind::Build => build_script,
                    DependencyKind::Development => target.cfg_test(),
                    DependencyKind::Other(_) => false,
                };
                if !dependency_kind_is_active
                    || (dependency.is_optional()
                        && !target
                            .features()
                            .activated_dependencies()
                            .contains(dependency.dependency_key()))
                {
                    continue;
                }
                if let Some(condition) = dependency.target_condition() {
                    if evaluator.dependency_condition(condition) == CfgDecision::Unknown {
                        diagnostics.push(
                            Diagnostic::analysis(
                                AnalysisIssueKind::UnknownCfg,
                                Severity::Warning,
                                LocalizedMessage::new(CFG_DEPENDENCY_UNKNOWN)
                                    .with_text("dependency", dependency.rust_name()),
                            )
                            .with_package(request.package.name())
                            .with_package_manifest_path(request.package.manifest_path())
                            .with_target(target.name())
                            .with_target_kind(target.kind().to_vec())
                            .with_cfg_condition(evaluator.normalized_condition(condition)),
                        );
                    }
                }
            }
        }
        modules.sort_by(|left, right| {
            left.target_name()
                .cmp(right.target_name())
                .then_with(|| left.context_index().cmp(&right.context_index()))
                .then_with(|| left.namespace().cmp(right.namespace()))
                .then_with(|| left.file_path().cmp(right.file_path()))
        });
        diagnostics.sort_by(|left, right| {
            left.target()
                .cmp(&right.target())
                .then_with(|| {
                    left.span()
                        .map(|span| span.path())
                        .cmp(&right.span().map(|span| span.path()))
                })
                .then_with(|| {
                    left.span()
                        .map(|span| span.start_line())
                        .cmp(&right.span().map(|span| span.start_line()))
                })
                .then_with(|| {
                    left.span()
                        .map(|span| span.start_column())
                        .cmp(&right.span().map(|span| span.start_column()))
                })
                .then_with(|| format!("{:?}", left.kind()).cmp(&format!("{:?}", right.kind())))
                .then_with(|| left.message().cmp(right.message()))
        });
        let facts = AnalysisFacts::build(request.package(), &modules);
        let status = ScanStatus::from_diagnostics(&diagnostics);
        AnalysisSnapshot {
            modules,
            facts,
            diagnostics,
            status,
        }
    }
}

impl AnalysisSnapshot {
    pub fn modules(&self) -> &[ModuleNode] {
        &self.modules
    }

    pub fn child_module(&self, parent: &ModuleNode, name: &str) -> Option<&ModuleNode> {
        let mut children = self.modules.iter().filter(|module| {
            module.context_index() == parent.context_index()
                && module.parent_namespace() == parent.namespace()
                && module
                    .namespace()
                    .last()
                    .is_some_and(|segment| segment == name)
        });
        let child = children.next()?;
        children.next().is_none().then_some(child)
    }

    pub fn symbols(&self) -> &SymbolIndex {
        self.facts.symbols()
    }

    pub fn facts(&self) -> &AnalysisFacts {
        &self.facts
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn status(&self) -> ScanStatus {
        self.status
    }
}
