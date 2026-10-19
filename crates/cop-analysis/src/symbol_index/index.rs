use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::Path,
};

use cop_cargo::{DependencyKind, SelectedPackage};
use cop_cfg::CfgCondition;
use cop_model::SourceSpan;
use cop_syntax::{
    ForeignItemSyntaxKind, ItemSyntaxKind, SourceFile, SyntaxNode, SyntaxNodeKind, VisibilityKind,
};

use super::{
    local_bindings::LocalBindings, PathResolutionKind, SymbolKind, SymbolRecord, SymbolRecordData,
    SymbolVisibility, VisibilityResult,
};

use crate::{
    cfg_eval::{CfgDecision, CfgEvaluator},
    ModuleNode,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathResolution {
    kind: PathResolutionKind,
    symbol: Option<SymbolRecord>,
    external_crate: Option<String>,
    external_path_depth: Option<usize>,
    external_crate_qualifier: bool,
    visibility: VisibilityResult,
    alias_chain: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SymbolIndex {
    symbols: Vec<SymbolRecord>,
    external_crates: BTreeMap<usize, BTreeSet<String>>,
    local_bindings: LocalBindings,
}

struct UseWalk<'a> {
    module: &'a ModuleNode,
    context_index: usize,
    target_name: &'a str,
    namespace: &'a [String],
    file_path: &'a Path,
    source: &'a SourceFile,
    cfg_condition: Option<String>,
    visibility: SymbolVisibility,
}

struct ResolveState<'a> {
    module: &'a ModuleNode,
    context_index: usize,
    requester: &'a [String],
    offset: Option<usize>,
    query_condition: Option<String>,
    alias_chain: Vec<String>,
    seen_aliases: BTreeSet<(Vec<String>, String)>,
    seen_local_aliases: BTreeSet<(usize, String)>,
}

impl PathResolution {
    pub fn kind(&self) -> PathResolutionKind {
        self.kind
    }

    pub fn symbol(&self) -> Option<&SymbolRecord> {
        self.symbol.as_ref()
    }

    pub fn external_crate(&self) -> Option<&str> {
        self.external_crate.as_deref()
    }

    pub fn visibility(&self) -> VisibilityResult {
        self.visibility
    }

    pub fn alias_chain(&self) -> &[String] {
        &self.alias_chain
    }

    pub fn is_external_crate_qualifier(&self) -> bool {
        self.external_crate_qualifier
    }
}

impl SymbolIndex {
    pub(crate) fn build(package: &SelectedPackage, modules: &[ModuleNode]) -> Self {
        let mut index = Self::default();
        let mut module_conditions = BTreeMap::new();
        for module in modules {
            module_conditions.insert(
                (module.context_index(), module.namespace().to_vec()),
                module.cfg_condition().map(str::to_owned),
            );
        }
        for module in modules {
            index.add_module_symbols(module, &module_conditions);
        }
        index.local_bindings = LocalBindings::build(modules);
        for (context_index, target) in package.targets().iter().enumerate() {
            let evaluator = CfgEvaluator::new(target);
            let build_script = target.kind().iter().any(|kind| kind == "custom-build");
            let crates = index.external_crates.entry(context_index).or_default();
            for dependency in package.dependencies().dependencies() {
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
                if dependency.target_condition().is_some_and(|condition| {
                    evaluator.dependency_condition(condition) != CfgDecision::True
                }) {
                    continue;
                }
                crates.insert(dependency.rust_name().to_owned());
            }
            crates.extend(["alloc", "core"].into_iter().map(str::to_owned));
            let no_std = modules.iter().any(|module| {
                module.context_index() == context_index
                    && module.is_crate_root()
                    && module.scope_inner_attributes().iter().any(|attribute| {
                        attribute.path().last().is_some_and(|name| name == "no_std")
                    })
            });
            if !no_std {
                crates.insert("std".to_owned());
            }
            if target.crate_types().iter().any(|kind| kind == "proc-macro") {
                crates.insert("proc_macro".to_owned());
            }
        }
        index.symbols.sort_by(|left, right| {
            left.target_name()
                .cmp(right.target_name())
                .then_with(|| left.context_index().cmp(&right.context_index()))
                .then_with(|| left.namespace().cmp(right.namespace()))
                .then_with(|| left.name().cmp(right.name()))
                .then_with(|| left.file_path().cmp(right.file_path()))
                .then_with(|| {
                    left.span()
                        .map(SourceSpan::start_line)
                        .cmp(&right.span().map(SourceSpan::start_line))
                })
        });
        index
    }

    pub fn symbols(&self) -> &[SymbolRecord] {
        &self.symbols
    }

    pub fn resolve_path(&self, module: &ModuleNode, path: &[String]) -> PathResolution {
        self.resolve_path_at(module, path, None)
    }

    pub fn resolve_path_at(
        &self,
        module: &ModuleNode,
        path: &[String],
        source_range: Option<Range<usize>>,
    ) -> PathResolution {
        let query_condition = source_range.as_ref().and_then(|range| {
            module
                .configured_ast()
                .condition_for_span(range.start, range.end)
                .map(str::to_owned)
        });
        let mut state = ResolveState {
            module,
            context_index: module.context_index(),
            requester: module.namespace(),
            offset: source_range.map(|range| range.start),
            query_condition,
            alias_chain: Vec::new(),
            seen_aliases: BTreeSet::new(),
            seen_local_aliases: BTreeSet::new(),
        };
        self.resolve_inner(&mut state, module.namespace(), path)
    }

    fn add_module_symbols(
        &mut self,
        module: &ModuleNode,
        module_conditions: &BTreeMap<(usize, Vec<String>), Option<String>>,
    ) {
        for item in module.item_nodes() {
            let Some(item_data) = item.item_syntax() else {
                continue;
            };
            let item_cfg = module
                .configured_ast()
                .node(item.id())
                .and_then(|node| node.effective_condition())
                .map(str::to_owned)
                .or_else(|| module.cfg_condition().map(str::to_owned));
            match item_data.kind() {
                ItemSyntaxKind::Module { name, .. } => {
                    let mut target_namespace = module.namespace().to_vec();
                    target_namespace.push(name.to_owned());
                    let cfg_condition = module_conditions
                        .get(&(module.context_index(), target_namespace.clone()))
                        .cloned()
                        .flatten()
                        .or(item_cfg);
                    self.symbols.push(SymbolRecord::new(SymbolRecordData {
                        name: name.to_owned(),
                        kind: SymbolKind::Module,
                        visibility: visibility(item_data.visibility().kind()),
                        context_index: module.context_index(),
                        target_name: module.target_name().to_owned(),
                        namespace: module.namespace().to_vec(),
                        target_namespace: Some(target_namespace),
                        target_path: None,
                        external_crate: None,
                        file_path: module.file_path().to_path_buf(),
                        span: module
                            .parse_output()
                            .source()
                            .span(item.start(), item.end())
                            .ok(),
                        cfg_condition,
                    }));
                }
                ItemSyntaxKind::Use => {
                    let walk = UseWalk {
                        module,
                        context_index: module.context_index(),
                        target_name: module.target_name(),
                        namespace: module.namespace(),
                        file_path: module.file_path(),
                        source: module.parse_output().source(),
                        cfg_condition: item_cfg,
                        visibility: visibility(item_data.visibility().kind()),
                    };
                    let tree = item
                        .children()
                        .iter()
                        .filter_map(|id| module.parse_output().node(*id))
                        .find(|node| node.kind() == SyntaxNodeKind::UseTree);
                    if let Some(tree) = tree {
                        self.add_use_tree(walk, tree, &[]);
                    }
                }
                ItemSyntaxKind::ExternCrate {
                    name: crate_name,
                    alias,
                } => {
                    if let Some(crate_name) = crate_name {
                        let name = alias.as_deref().unwrap_or(crate_name);
                        self.symbols.push(SymbolRecord::new(SymbolRecordData {
                            name: name.to_owned(),
                            kind: SymbolKind::ExternalCrate,
                            visibility: visibility(item_data.visibility().kind()),
                            context_index: module.context_index(),
                            target_name: module.target_name().to_owned(),
                            namespace: module.namespace().to_vec(),
                            target_namespace: None,
                            target_path: None,
                            external_crate: Some(crate_name.to_owned()),
                            file_path: module.file_path().to_path_buf(),
                            span: module
                                .parse_output()
                                .source()
                                .span(item.start(), item.end())
                                .ok(),
                            cfg_condition: item_cfg,
                        }));
                    }
                }
                ItemSyntaxKind::ExternBlock { .. } => {
                    self.add_foreign_symbols(module, item, item_cfg);
                }
                _ => {
                    if let Some((name, kind)) = named_item(item_data.kind()) {
                        self.symbols.push(SymbolRecord::new(SymbolRecordData {
                            name,
                            kind,
                            visibility: visibility(item_data.visibility().kind()),
                            context_index: module.context_index(),
                            target_name: module.target_name().to_owned(),
                            namespace: module.namespace().to_vec(),
                            target_namespace: None,
                            target_path: None,
                            external_crate: None,
                            file_path: module.file_path().to_path_buf(),
                            span: module
                                .parse_output()
                                .source()
                                .span(item.start(), item.end())
                                .ok(),
                            cfg_condition: item_cfg,
                        }));
                    }
                }
            }
        }
    }

    fn add_foreign_symbols(
        &mut self,
        module: &ModuleNode,
        item: &SyntaxNode,
        item_cfg: Option<String>,
    ) {
        let scope = item
            .children()
            .iter()
            .filter_map(|id| module.parse_output().node(*id))
            .find(|node| node.kind() == SyntaxNodeKind::Scope);
        let Some(scope) = scope else {
            return;
        };
        for foreign in scope
            .children()
            .iter()
            .filter_map(|id| module.parse_output().node(*id))
        {
            let Some(configured) = module.configured_ast().node(foreign.id()) else {
                continue;
            };
            if configured.decision() == CfgDecision::False {
                continue;
            }
            let Some(data) = foreign.foreign_item_syntax() else {
                continue;
            };
            let (name, kind) = match data.kind() {
                ForeignItemSyntaxKind::Function {
                    name: Some(name), ..
                } => (name.clone(), SymbolKind::Function),
                ForeignItemSyntaxKind::Static {
                    name: Some(name), ..
                } => (name.clone(), SymbolKind::Static),
                _ => continue,
            };
            self.symbols.push(SymbolRecord::new(SymbolRecordData {
                name,
                kind,
                visibility: visibility(data.visibility().kind()),
                context_index: module.context_index(),
                target_name: module.target_name().to_owned(),
                namespace: module.namespace().to_vec(),
                target_namespace: None,
                target_path: None,
                external_crate: None,
                file_path: module.file_path().to_path_buf(),
                span: module
                    .parse_output()
                    .source()
                    .span(foreign.start(), foreign.end())
                    .ok(),
                cfg_condition: configured
                    .effective_condition()
                    .map(str::to_owned)
                    .or_else(|| item_cfg.clone()),
            }));
        }
    }

    fn add_use_tree(&mut self, walk: UseWalk<'_>, tree: &SyntaxNode, prefix: &[String]) {
        let Some(configured) = walk.module.configured_ast().node(tree.id()) else {
            return;
        };
        if configured.decision() == CfgDecision::False {
            return;
        }
        let Some(tree_data) = tree.use_tree() else {
            return;
        };
        let cfg_condition = configured
            .effective_condition()
            .map(str::to_owned)
            .or(walk.cfg_condition.clone());
        let mut path = prefix.to_vec();
        path.extend(tree_data.path().iter().cloned());
        let children = tree
            .children()
            .iter()
            .filter_map(|id| walk.module.parse_output().node(*id))
            .filter(|child| child.kind() == SyntaxNodeKind::UseTree)
            .collect::<Vec<_>>();
        if !children.is_empty() {
            for child in children {
                self.add_use_tree(
                    UseWalk {
                        module: walk.module,
                        context_index: walk.context_index,
                        target_name: walk.target_name,
                        namespace: walk.namespace,
                        file_path: walk.file_path,
                        source: walk.source,
                        cfg_condition: cfg_condition.clone(),
                        visibility: walk.visibility.clone(),
                    },
                    child,
                    &path,
                );
            }
            return;
        }
        if tree_data.is_glob() || path.is_empty() {
            return;
        }
        if path.last().is_some_and(|segment| segment == "self") {
            path.pop();
        }
        let alias = tree_data
            .alias()
            .map(str::to_owned)
            .or_else(|| path.last().cloned());
        if let Some(name) = alias {
            self.symbols.push(SymbolRecord::new(SymbolRecordData {
                name,
                kind: SymbolKind::UseAlias,
                visibility: walk.visibility,
                context_index: walk.context_index,
                target_name: walk.target_name.to_owned(),
                namespace: walk.namespace.to_vec(),
                target_namespace: None,
                target_path: Some(path),
                external_crate: None,
                file_path: walk.file_path.to_path_buf(),
                span: walk.source.span(tree.start(), tree.end()).ok(),
                cfg_condition,
            }));
        }
    }

    fn resolve_inner(
        &self,
        state: &mut ResolveState<'_>,
        base_namespace: &[String],
        path: &[String],
    ) -> PathResolution {
        let mut base = base_namespace.to_vec();
        let mut rest = path;
        let rooted = rest
            .first()
            .is_some_and(|segment| segment == "crate" || segment == "::");
        if rest
            .first()
            .is_some_and(|segment| segment == "crate" || segment == "::")
        {
            base.clear();
            rest = &rest[1..];
        } else if rest.first().is_some_and(|segment| segment == "self") {
            rest = &rest[1..];
        } else {
            while rest.first().is_some_and(|segment| segment == "super") {
                if base.pop().is_none() {
                    return unresolved(PathResolutionKind::Unresolved, &state.alias_chain);
                }
                rest = &rest[1..];
            }
        }
        let Some(first) = rest.first() else {
            return PathResolution {
                kind: PathResolutionKind::Local,
                symbol: None,
                external_crate: None,
                external_path_depth: None,
                external_crate_qualifier: false,
                visibility: VisibilityResult::Reachable,
                alias_chain: state.alias_chain.clone(),
            };
        };
        if rooted
            && path.first().is_some_and(|segment| segment == "::")
            && self
                .external_crates
                .get(&state.context_index)
                .is_some_and(|crates| crates.contains(identifier_name(first)))
        {
            return PathResolution {
                kind: PathResolutionKind::ExternalCrate,
                symbol: None,
                external_crate: Some(identifier_name(first).to_owned()),
                external_path_depth: Some(rest.len().saturating_sub(1)),
                external_crate_qualifier: true,
                visibility: VisibilityResult::Public,
                alias_chain: state.alias_chain.clone(),
            };
        }
        let has_explicit_scope = path
            .first()
            .is_some_and(|segment| matches!(segment.as_str(), "crate" | "::" | "self" | "super"));
        if !has_explicit_scope && base == state.requester {
            if let Some(offset) = state.offset {
                if let Some(binding) = self.local_bindings.find(
                    state.module,
                    offset,
                    first,
                    state.query_condition.as_deref(),
                ) {
                    if let Some(target_path) = binding.target_path() {
                        let key = (binding.scope_start(), binding.name().to_owned());
                        if !state.seen_local_aliases.insert(key.clone()) {
                            return unresolved(PathResolutionKind::AliasCycle, &state.alias_chain);
                        }
                        state.alias_chain.push(binding.name().to_owned());
                        let mut redirected = target_path.to_vec();
                        let suffix_length = rest.len().saturating_sub(1);
                        redirected.extend(rest.iter().skip(1).cloned());
                        let mut resolution =
                            self.resolve_inner(state, state.requester, &redirected);
                        if resolution.external_path_depth == Some(suffix_length) {
                            resolution.external_crate_qualifier = true;
                        } else if resolution.external_crate.is_some() {
                            resolution.external_crate_qualifier = false;
                        }
                        state.seen_local_aliases.remove(&key);
                        return resolution;
                    }
                    return PathResolution {
                        kind: PathResolutionKind::Local,
                        symbol: None,
                        external_crate: None,
                        external_path_depth: None,
                        external_crate_qualifier: false,
                        visibility: VisibilityResult::Reachable,
                        alias_chain: state.alias_chain.clone(),
                    };
                }
            }
        }
        let candidate = match self.find_symbol(
            state.context_index,
            &base,
            first,
            state.query_condition.as_deref(),
        ) {
            Ok(Some(candidate)) => candidate,
            Err(()) => return unresolved(PathResolutionKind::Ambiguous, &state.alias_chain),
            Ok(None) => {
                if (rooted || base == base_namespace)
                    && self
                        .external_crates
                        .get(&state.context_index)
                        .is_some_and(|crates| crates.contains(identifier_name(first)))
                {
                    return PathResolution {
                        kind: PathResolutionKind::ExternalCrate,
                        symbol: None,
                        external_crate: Some(identifier_name(first).to_owned()),
                        external_path_depth: Some(rest.len().saturating_sub(1)),
                        external_crate_qualifier: true,
                        visibility: VisibilityResult::Public,
                        alias_chain: state.alias_chain.clone(),
                    };
                }
                return unresolved(PathResolutionKind::Unresolved, &state.alias_chain);
            }
        };
        if candidate.kind() == SymbolKind::UseAlias {
            let visibility = candidate.is_visible_from(state.requester);
            if visibility == VisibilityResult::Private {
                return PathResolution {
                    kind: PathResolutionKind::Local,
                    symbol: Some(candidate.clone()),
                    external_crate: None,
                    external_path_depth: None,
                    external_crate_qualifier: false,
                    visibility,
                    alias_chain: state.alias_chain.clone(),
                };
            }
            let key = (candidate.namespace().to_vec(), candidate.name().to_owned());
            if !state.seen_aliases.insert(key) {
                return unresolved(PathResolutionKind::AliasCycle, &state.alias_chain);
            }
            state.alias_chain.push(candidate.name().to_owned());
            let mut redirected = candidate.target_path().unwrap_or_default().to_vec();
            let suffix_length = rest.len().saturating_sub(1);
            redirected.extend(rest.iter().skip(1).cloned());
            let request_offset = state.offset.take();
            let mut resolution = self.resolve_inner(state, candidate.namespace(), &redirected);
            state.offset = request_offset;
            if resolution.external_path_depth == Some(suffix_length) {
                resolution.external_crate_qualifier = true;
            } else if resolution.external_crate.is_some() {
                resolution.external_crate_qualifier = false;
            }
            state
                .seen_aliases
                .remove(&(candidate.namespace().to_vec(), candidate.name().to_owned()));
            return resolution;
        }
        if let Some(external) = candidate.external_crate() {
            return PathResolution {
                kind: PathResolutionKind::ExternalCrate,
                symbol: Some(candidate.clone()),
                external_crate: Some(external.to_owned()),
                external_path_depth: Some(rest.len().saturating_sub(1)),
                external_crate_qualifier: true,
                visibility: candidate.is_visible_from(state.requester),
                alias_chain: state.alias_chain.clone(),
            };
        }
        let visibility = candidate.is_visible_from(state.requester);
        if visibility == VisibilityResult::Private {
            return PathResolution {
                kind: PathResolutionKind::Local,
                symbol: Some(candidate.clone()),
                external_crate: None,
                external_path_depth: None,
                external_crate_qualifier: false,
                visibility,
                alias_chain: state.alias_chain.clone(),
            };
        }
        if rest.len() > 1 {
            if let Some(next_namespace) = candidate.target_namespace() {
                return self.resolve_inner(state, next_namespace, &rest[1..]);
            }
            return unresolved(PathResolutionKind::Unresolved, &state.alias_chain);
        }
        PathResolution {
            kind: PathResolutionKind::Local,
            symbol: Some(candidate.clone()),
            external_crate: None,
            external_path_depth: None,
            external_crate_qualifier: false,
            visibility,
            alias_chain: state.alias_chain.clone(),
        }
    }

    fn find_symbol(
        &self,
        context_index: usize,
        namespace: &[String],
        name: &str,
        query_condition: Option<&str>,
    ) -> Result<Option<&SymbolRecord>, ()> {
        for depth in (0..=namespace.len()).rev() {
            let scope = &namespace[..depth];
            let mut found = self.symbols.iter().filter(|symbol| {
                symbol.context_index() == context_index
                    && symbol.namespace() == scope
                    && identifier_name(symbol.name()) == identifier_name(name)
                    && conditions_may_coexist(symbol.cfg_condition(), query_condition)
            });
            let Some(first) = found.next() else {
                continue;
            };
            if found.any(|candidate| {
                conditions_may_coexist(first.cfg_condition(), candidate.cfg_condition())
            }) {
                return Err(());
            }
            return Ok(Some(first));
        }
        Ok(None)
    }
}

fn identifier_name(name: &str) -> &str {
    name.strip_prefix("r#").unwrap_or(name)
}

fn unresolved(kind: PathResolutionKind, alias_chain: &[String]) -> PathResolution {
    PathResolution {
        kind,
        symbol: None,
        external_crate: None,
        external_path_depth: None,
        external_crate_qualifier: false,
        visibility: VisibilityResult::Unknown,
        alias_chain: alias_chain.to_vec(),
    }
}

fn conditions_may_coexist(left: Option<&str>, right: Option<&str>) -> bool {
    let Some(left) = left else {
        return true;
    };
    let Some(right) = right else {
        return true;
    };
    match (CfgCondition::parse(left), CfgCondition::parse(right)) {
        (Ok(left), Ok(right)) => CfgCondition::may_coexist(&left, &right),
        _ => true,
    }
}

fn visibility(kind: &VisibilityKind) -> SymbolVisibility {
    match kind {
        VisibilityKind::Private => SymbolVisibility::Private,
        VisibilityKind::Public => SymbolVisibility::Public,
        VisibilityKind::Restricted(scope) => SymbolVisibility::Restricted(scope.clone()),
    }
}

fn named_item(kind: &ItemSyntaxKind) -> Option<(String, SymbolKind)> {
    match kind {
        ItemSyntaxKind::Struct(name) => Some((name.as_ref()?.to_owned(), SymbolKind::Struct)),
        ItemSyntaxKind::Enum(name) => Some((name.as_ref()?.to_owned(), SymbolKind::Enum)),
        ItemSyntaxKind::Union(name) => Some((name.as_ref()?.to_owned(), SymbolKind::Union)),
        ItemSyntaxKind::TypeAlias(name) => Some((name.as_ref()?.to_owned(), SymbolKind::TypeAlias)),
        ItemSyntaxKind::Trait(name) => Some((name.as_ref()?.to_owned(), SymbolKind::Trait)),
        ItemSyntaxKind::Constant(name) => Some((name.as_ref()?.to_owned(), SymbolKind::Constant)),
        ItemSyntaxKind::Static(name) => Some((name.as_ref()?.to_owned(), SymbolKind::Static)),
        ItemSyntaxKind::Function { name } => {
            Some((name.as_ref()?.to_owned(), SymbolKind::Function))
        }
        _ => None,
    }
}
