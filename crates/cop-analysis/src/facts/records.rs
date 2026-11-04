use std::ops::Range;

use cop_cfg::CfgExpression;
use cop_syntax::AstNodeId;

use super::ModuleId;
use crate::{CfgDecision, DefinitionId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverageRegionKind {
    ParserIssue,
    OpaqueSyntax,
    MacroInvocation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoverageRegion {
    module_id: ModuleId,
    bounds: Range<usize>,
    condition: Option<String>,
    kind: CoverageRegionKind,
}

impl CoverageRegion {
    pub(super) fn new(
        module_id: ModuleId,
        bounds: Range<usize>,
        condition: Option<String>,
        kind: CoverageRegionKind,
    ) -> Self {
        Self {
            module_id,
            bounds,
            condition,
            kind,
        }
    }

    pub fn module_id(&self) -> ModuleId {
        self.module_id
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn condition(&self) -> Option<&str> {
        self.condition.as_deref()
    }

    pub fn kind(&self) -> CoverageRegionKind {
        self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImplOwnerRecord {
    module_id: ModuleId,
    bounds: Range<usize>,
    condition: Option<String>,
    owner: Option<DefinitionId>,
    target_bounds: Option<Range<usize>>,
    target_path: Option<Vec<String>>,
    trait_bounds: Option<Range<usize>>,
    trait_path: Option<Vec<String>>,
    target_enabled: bool,
    target_supported: bool,
    target_is_structural: bool,
    trait_enabled: bool,
    trait_supported: bool,
    trait_impl: bool,
}

#[derive(Debug)]
pub(super) struct ImplOwnerRecordData {
    pub(super) module_id: ModuleId,
    pub(super) bounds: Range<usize>,
    pub(super) condition: Option<String>,
    pub(super) owner: Option<DefinitionId>,
    pub(super) target_bounds: Option<Range<usize>>,
    pub(super) target_path: Option<Vec<String>>,
    pub(super) trait_bounds: Option<Range<usize>>,
    pub(super) trait_path: Option<Vec<String>>,
    pub(super) target_enabled: bool,
    pub(super) target_supported: bool,
    pub(super) target_is_structural: bool,
    pub(super) trait_enabled: bool,
    pub(super) trait_supported: bool,
    pub(super) trait_impl: bool,
}

impl ImplOwnerRecord {
    pub(super) fn new(data: ImplOwnerRecordData) -> Self {
        Self {
            module_id: data.module_id,
            bounds: data.bounds,
            condition: data.condition,
            owner: data.owner,
            target_bounds: data.target_bounds,
            target_path: data.target_path,
            trait_bounds: data.trait_bounds,
            trait_path: data.trait_path,
            target_enabled: data.target_enabled,
            target_supported: data.target_supported,
            target_is_structural: data.target_is_structural,
            trait_enabled: data.trait_enabled,
            trait_supported: data.trait_supported,
            trait_impl: data.trait_impl,
        }
    }

    pub(super) fn set_owner(&mut self, owner: Option<DefinitionId>) {
        self.owner = owner;
    }

    pub fn module_id(&self) -> ModuleId {
        self.module_id
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn condition(&self) -> Option<&str> {
        self.condition.as_deref()
    }

    pub fn owner(&self) -> Option<DefinitionId> {
        self.owner
    }

    pub fn target_bounds(&self) -> Option<Range<usize>> {
        self.target_bounds.clone()
    }

    pub fn target_path(&self) -> Option<&[String]> {
        self.target_path.as_deref()
    }

    pub fn is_target_supported(&self) -> bool {
        self.target_supported
    }

    pub fn is_target_enabled(&self) -> bool {
        self.target_enabled
    }

    pub fn is_target_structural(&self) -> bool {
        self.target_is_structural
    }

    pub fn trait_bounds(&self) -> Option<Range<usize>> {
        self.trait_bounds.clone()
    }

    pub fn trait_path(&self) -> Option<&[String]> {
        self.trait_path.as_deref()
    }

    pub fn is_trait_enabled(&self) -> bool {
        self.trait_enabled
    }

    pub fn is_trait_supported(&self) -> bool {
        self.trait_supported
    }

    pub fn is_trait_impl(&self) -> bool {
        self.trait_impl
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CfgMacroFact {
    module_id: ModuleId,
    ast_node_id: AstNodeId,
    predicate: CfgExpression,
    predicate_bounds: Range<usize>,
    decision: CfgDecision,
}

impl CfgMacroFact {
    pub(super) fn new(
        module_id: ModuleId,
        ast_node_id: AstNodeId,
        predicate: CfgExpression,
        predicate_bounds: Range<usize>,
        decision: CfgDecision,
    ) -> Self {
        Self {
            module_id,
            ast_node_id,
            predicate,
            predicate_bounds,
            decision,
        }
    }

    pub fn module_id(&self) -> ModuleId {
        self.module_id
    }

    pub fn ast_node_id(&self) -> AstNodeId {
        self.ast_node_id
    }

    pub fn predicate(&self) -> &CfgExpression {
        &self.predicate
    }

    pub fn predicate_bounds(&self) -> Range<usize> {
        self.predicate_bounds.clone()
    }

    pub fn decision(&self) -> CfgDecision {
        self.decision
    }
}
