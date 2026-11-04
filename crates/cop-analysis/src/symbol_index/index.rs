use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

use super::{
    local_bindings::LocalBindings, DefinitionId, PathResolutionKind, SymbolIndexSupport,
    SymbolKind, SymbolRecord, VisibilityResult,
};
use cop_model::SourceSpan;

use crate::ModuleNode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathRootKind {
    ExternalCrate,
    ExplicitScope,
    LocalDefinition,
    Primitive,
    UseAlias,
    Unresolved,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathRootIdentity {
    kind: PathRootKind,
    external_crate: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathResolution {
    kind: PathResolutionKind,
    root: PathRootIdentity,
    symbol: Option<SymbolRecord>,
    external_crate: Option<String>,
    external_path_depth: Option<usize>,
    external_crate_qualifier: bool,
    visibility: VisibilityResult,
    alias_chain: Vec<String>,
    alias_symbols: Vec<SymbolRecord>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SymbolIndex {
    symbols: Vec<SymbolRecord>,
    external_crates: BTreeMap<usize, BTreeSet<String>>,
    prelude_names: BTreeMap<usize, BTreeSet<String>>,
    local_bindings: LocalBindings,
}

struct ResolveState<'a> {
    module: &'a ModuleNode,
    context_index: usize,
    requester: &'a [String],
    offset: Option<usize>,
    query_condition: Option<String>,
    alias_chain: Vec<String>,
    alias_symbols: Vec<SymbolRecord>,
    seen_aliases: BTreeSet<(Vec<String>, String)>,
    seen_local_aliases: BTreeSet<(usize, String)>,
}

impl PathResolution {
    pub fn kind(&self) -> PathResolutionKind {
        self.kind
    }

    pub fn root(&self) -> &PathRootIdentity {
        &self.root
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

    pub fn alias_symbols(&self) -> &[SymbolRecord] {
        &self.alias_symbols
    }

    pub fn is_external_crate_qualifier(&self) -> bool {
        self.external_crate_qualifier
    }

    fn with_root(mut self, kind: PathRootKind, external_crate: Option<String>) -> Self {
        self.root = PathRootIdentity {
            kind,
            external_crate,
        };
        self
    }

    fn with_alias_root(mut self) -> Self {
        let external_crate = self.root.external_crate.clone();
        self.root = PathRootIdentity {
            kind: PathRootKind::UseAlias,
            external_crate,
        };
        self
    }
}

impl PathRootIdentity {
    pub fn kind(&self) -> PathRootKind {
        self.kind
    }

    pub fn external_crate(&self) -> Option<&str> {
        self.external_crate.as_deref()
    }
}

impl SymbolIndex {
    pub fn symbols(&self) -> &[SymbolRecord] {
        &self.symbols
    }

    pub(super) fn push_symbol(&mut self, symbol: SymbolRecord) {
        self.symbols.push(symbol);
    }

    pub(super) fn last_symbol_mut(&mut self) -> Option<&mut SymbolRecord> {
        self.symbols.last_mut()
    }

    pub(super) fn local_bindings_mut(&mut self) -> &mut LocalBindings {
        &mut self.local_bindings
    }

    pub(super) fn external_crates_mut(&mut self) -> &mut BTreeMap<usize, BTreeSet<String>> {
        &mut self.external_crates
    }

    pub(super) fn prelude_names_mut(&mut self) -> &mut BTreeMap<usize, BTreeSet<String>> {
        &mut self.prelude_names
    }

    pub(super) fn prelude_names(edition: &str, standard_library: bool) -> BTreeSet<String> {
        let mut names = [
            "AsMut",
            "AsRef",
            "AsyncFn",
            "AsyncFnMut",
            "AsyncFnOnce",
            "Clone",
            "Copy",
            "Default",
            "Drop",
            "Eq",
            "Err",
            "ExactSizeIterator",
            "Extend",
            "Fn",
            "FnMut",
            "FnOnce",
            "From",
            "Into",
            "IntoIterator",
            "Iterator",
            "None",
            "Ok",
            "Option",
            "Ord",
            "PartialEq",
            "PartialOrd",
            "Result",
            "Send",
            "Sized",
            "Some",
            "Sync",
            "Unpin",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
        if edition == "2021" || edition == "2024" {
            names.extend(
                ["FromIterator", "TryFrom", "TryInto"]
                    .into_iter()
                    .map(str::to_owned),
            );
        }
        if edition == "2024" {
            names.extend(["Future", "IntoFuture"].into_iter().map(str::to_owned));
        }
        if standard_library {
            names.extend(
                ["Box", "String", "ToOwned", "ToString", "Vec"]
                    .into_iter()
                    .map(str::to_owned),
            );
        }
        names
    }

    pub(super) fn sort_and_assign_definition_ids(&mut self) {
        self.symbols.sort_by(|left, right| {
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
        for (index, symbol) in self.symbols.iter_mut().enumerate() {
            symbol.set_definition_id(DefinitionId::new(index));
        }
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
            alias_symbols: Vec::new(),
            seen_aliases: BTreeSet::new(),
            seen_local_aliases: BTreeSet::new(),
        };
        self.resolve_inner(&mut state, module.namespace(), path)
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
                    return unresolved_for_path(PathResolutionKind::Unresolved, state, path);
                }
                rest = &rest[1..];
            }
        }
        let Some(first) = rest.first() else {
            return PathResolution {
                kind: PathResolutionKind::Local,
                root: root_identity_for_explicit_scope(path),
                symbol: None,
                external_crate: None,
                external_path_depth: None,
                external_crate_qualifier: false,
                visibility: VisibilityResult::Reachable,
                alias_chain: state.alias_chain.clone(),
                alias_symbols: state.alias_symbols.clone(),
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
                root: external_crate_root(identifier_name(first)),
                symbol: None,
                external_crate: Some(identifier_name(first).to_owned()),
                external_path_depth: Some(rest.len().saturating_sub(1)),
                external_crate_qualifier: true,
                visibility: VisibilityResult::Public,
                alias_chain: state.alias_chain.clone(),
                alias_symbols: state.alias_symbols.clone(),
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
                            return unresolved_for_path(
                                PathResolutionKind::AliasCycle,
                                state,
                                path,
                            );
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
                        return resolution.with_alias_root();
                    }
                    return PathResolution {
                        kind: PathResolutionKind::Local,
                        root: PathRootIdentity {
                            kind: PathRootKind::LocalDefinition,
                            external_crate: None,
                        },
                        symbol: None,
                        external_crate: None,
                        external_path_depth: None,
                        external_crate_qualifier: false,
                        visibility: VisibilityResult::Reachable,
                        alias_chain: state.alias_chain.clone(),
                        alias_symbols: state.alias_symbols.clone(),
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
            Err(()) => return unresolved_for_path(PathResolutionKind::Ambiguous, state, path),
            Ok(None) => {
                if !has_explicit_scope && rest.len() == 1 && is_primitive_type(first) {
                    return primitive_resolution(state);
                }
                if !has_explicit_scope
                    && rest.len() == 1
                    && self
                        .prelude_names
                        .get(&state.context_index)
                        .is_some_and(|names| names.contains(identifier_name(first)))
                {
                    let prelude_crate = if self
                        .external_crates
                        .get(&state.context_index)
                        .is_some_and(|crates| crates.contains("std"))
                    {
                        "std"
                    } else {
                        "core"
                    };
                    return PathResolution {
                        kind: PathResolutionKind::ExternalCrate,
                        root: external_crate_root(prelude_crate),
                        symbol: None,
                        external_crate: Some(prelude_crate.to_owned()),
                        external_path_depth: Some(0),
                        external_crate_qualifier: false,
                        visibility: VisibilityResult::Public,
                        alias_chain: state.alias_chain.clone(),
                        alias_symbols: state.alias_symbols.clone(),
                    };
                }
                if (rooted || base == base_namespace)
                    && self
                        .external_crates
                        .get(&state.context_index)
                        .is_some_and(|crates| crates.contains(identifier_name(first)))
                {
                    return PathResolution {
                        kind: PathResolutionKind::ExternalCrate,
                        root: external_crate_root(identifier_name(first)),
                        symbol: None,
                        external_crate: Some(identifier_name(first).to_owned()),
                        external_path_depth: Some(rest.len().saturating_sub(1)),
                        external_crate_qualifier: true,
                        visibility: VisibilityResult::Public,
                        alias_chain: state.alias_chain.clone(),
                        alias_symbols: state.alias_symbols.clone(),
                    };
                }
                return unresolved_for_path(PathResolutionKind::Unresolved, state, path);
            }
        };
        if candidate.kind() == SymbolKind::UseAlias {
            let visibility = candidate.is_visible_from(state.requester);
            if visibility == VisibilityResult::Private {
                return PathResolution {
                    kind: PathResolutionKind::Local,
                    root: PathRootIdentity {
                        kind: PathRootKind::UseAlias,
                        external_crate: None,
                    },
                    symbol: Some(candidate.clone()),
                    external_crate: None,
                    external_path_depth: None,
                    external_crate_qualifier: false,
                    visibility,
                    alias_chain: state.alias_chain.clone(),
                    alias_symbols: state.alias_symbols.clone(),
                };
            }
            let key = (candidate.namespace().to_vec(), candidate.name().to_owned());
            if !state.seen_aliases.insert(key) {
                if let Some(external_crate) =
                    self.self_reexported_external_crate(candidate, state.context_index)
                {
                    return PathResolution {
                        kind: PathResolutionKind::ExternalCrate,
                        root: external_crate_root(&external_crate),
                        symbol: Some(candidate.clone()),
                        external_crate: Some(external_crate),
                        external_path_depth: Some(rest.len().saturating_sub(1)),
                        external_crate_qualifier: true,
                        visibility: candidate.is_visible_from(state.requester),
                        alias_chain: state.alias_chain.clone(),
                        alias_symbols: state.alias_symbols.clone(),
                    };
                }
                return unresolved_for_path(PathResolutionKind::AliasCycle, state, path);
            }
            state.alias_chain.push(candidate.name().to_owned());
            state.alias_symbols.push(candidate.clone());
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
            state.alias_symbols.pop();
            return resolution.with_alias_root();
        }
        if let Some(external) = candidate.external_crate() {
            return PathResolution {
                kind: PathResolutionKind::ExternalCrate,
                root: external_crate_root(external),
                symbol: Some(candidate.clone()),
                external_crate: Some(external.to_owned()),
                external_path_depth: Some(rest.len().saturating_sub(1)),
                external_crate_qualifier: true,
                visibility: candidate.is_visible_from(state.requester),
                alias_chain: state.alias_chain.clone(),
                alias_symbols: state.alias_symbols.clone(),
            };
        }
        let visibility = candidate.is_visible_from(state.requester);
        if visibility == VisibilityResult::Private {
            return PathResolution {
                kind: PathResolutionKind::Local,
                root: PathRootIdentity {
                    kind: PathRootKind::LocalDefinition,
                    external_crate: None,
                },
                symbol: Some(candidate.clone()),
                external_crate: None,
                external_path_depth: None,
                external_crate_qualifier: false,
                visibility,
                alias_chain: state.alias_chain.clone(),
                alias_symbols: state.alias_symbols.clone(),
            };
        }
        if rest.len() > 1 {
            if let Some(next_namespace) = candidate.target_namespace() {
                let resolution = self.resolve_inner(state, next_namespace, &rest[1..]);
                if has_explicit_scope {
                    return resolution.with_root(PathRootKind::ExplicitScope, None);
                }
                return resolution.with_root(PathRootKind::LocalDefinition, None);
            }
            return unresolved_for_path(PathResolutionKind::Unresolved, state, path);
        }
        PathResolution {
            kind: PathResolutionKind::Local,
            root: root_identity_for_explicit_scope(path),
            symbol: Some(candidate.clone()),
            external_crate: None,
            external_path_depth: None,
            external_crate_qualifier: false,
            visibility,
            alias_chain: state.alias_chain.clone(),
            alias_symbols: state.alias_symbols.clone(),
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
                    && SymbolIndexSupport::conditions_may_coexist(
                        symbol.cfg_condition(),
                        query_condition,
                    )
            });
            let Some(first) = found.next() else {
                continue;
            };
            if found.any(|candidate| {
                SymbolIndexSupport::conditions_may_coexist(
                    first.cfg_condition(),
                    candidate.cfg_condition(),
                )
            }) {
                return Err(());
            }
            return Ok(Some(first));
        }
        Ok(None)
    }

    fn self_reexported_external_crate(
        &self,
        candidate: &SymbolRecord,
        context_index: usize,
    ) -> Option<String> {
        if !candidate.namespace().is_empty() {
            return None;
        }
        let name = identifier_name(candidate.name());
        let target_path = candidate.target_path()?;
        let target = if target_path.first().is_some_and(|segment| segment == "::") {
            &target_path[1..]
        } else {
            target_path
        };
        if target.len() != 1 || identifier_name(&target[0]) != name {
            return None;
        }
        self.external_crates
            .get(&context_index)
            .filter(|crates| crates.contains(name))
            .map(|_| name.to_owned())
    }
}

fn identifier_name(name: &str) -> &str {
    name.strip_prefix("r#").unwrap_or(name)
}

fn unresolved(kind: PathResolutionKind, state: &ResolveState<'_>) -> PathResolution {
    PathResolution {
        kind,
        root: PathRootIdentity {
            kind: PathRootKind::Unresolved,
            external_crate: None,
        },
        symbol: None,
        external_crate: None,
        external_path_depth: None,
        external_crate_qualifier: false,
        visibility: VisibilityResult::Unknown,
        alias_chain: state.alias_chain.clone(),
        alias_symbols: state.alias_symbols.clone(),
    }
}

fn unresolved_for_path(
    kind: PathResolutionKind,
    state: &ResolveState<'_>,
    path: &[String],
) -> PathResolution {
    let mut resolution = unresolved(kind, state);
    if is_explicit_scope(path) {
        resolution.root = PathRootIdentity {
            kind: PathRootKind::ExplicitScope,
            external_crate: None,
        };
    }
    resolution
}

fn external_crate_root(name: &str) -> PathRootIdentity {
    PathRootIdentity {
        kind: PathRootKind::ExternalCrate,
        external_crate: Some(name.to_owned()),
    }
}

fn primitive_resolution(state: &ResolveState<'_>) -> PathResolution {
    PathResolution {
        kind: PathResolutionKind::Local,
        root: PathRootIdentity {
            kind: PathRootKind::Primitive,
            external_crate: None,
        },
        symbol: None,
        external_crate: None,
        external_path_depth: None,
        external_crate_qualifier: false,
        visibility: VisibilityResult::Reachable,
        alias_chain: state.alias_chain.clone(),
        alias_symbols: state.alias_symbols.clone(),
    }
}

fn is_primitive_type(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "char"
            | "str"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
    )
}

fn root_identity_for_explicit_scope(path: &[String]) -> PathRootIdentity {
    let kind = if is_explicit_scope(path) {
        PathRootKind::ExplicitScope
    } else {
        PathRootKind::LocalDefinition
    };
    PathRootIdentity {
        kind,
        external_crate: None,
    }
}

fn is_explicit_scope(path: &[String]) -> bool {
    path.first()
        .is_some_and(|segment| matches!(segment.as_str(), "crate" | "::" | "self" | "super"))
}
