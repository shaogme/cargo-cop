use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use cop_cargo::{DependencyKind, SelectedPackage, TargetContext};
use cop_model::SourceSpan;
use cop_syntax::{Item, ItemKind, SourceFile, UseTree, VisibilityKind};

use crate::{
    cfg_eval::{CfgDecision, CfgEvaluator},
    ModuleNode,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolKind {
    Module,
    Struct,
    Enum,
    Union,
    TypeAlias,
    Trait,
    Constant,
    Static,
    Function,
    UseAlias,
    ExternalCrate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SymbolVisibility {
    Private,
    Public,
    Restricted(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathResolutionKind {
    Local,
    ExternalCrate,
    Unresolved,
    Ambiguous,
    AliasCycle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisibilityResult {
    Public,
    Reachable,
    Private,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolRecord {
    name: String,
    kind: SymbolKind,
    visibility: SymbolVisibility,
    context_index: usize,
    target_name: String,
    namespace: Vec<String>,
    target_namespace: Option<Vec<String>>,
    target_path: Option<Vec<String>>,
    external_crate: Option<String>,
    file_path: PathBuf,
    span: Option<SourceSpan>,
    cfg_condition: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathResolution {
    kind: PathResolutionKind,
    symbol: Option<SymbolRecord>,
    external_crate: Option<String>,
    visibility: VisibilityResult,
    alias_chain: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SymbolIndex {
    symbols: Vec<SymbolRecord>,
    external_crates: BTreeMap<usize, BTreeSet<String>>,
}

struct UseWalk<'a> {
    context_index: usize,
    target_name: &'a str,
    namespace: &'a [String],
    file_path: &'a Path,
    source: &'a SourceFile,
    cfg_condition: Option<&'a str>,
    visibility: SymbolVisibility,
}

struct ResolveState<'a> {
    context_index: usize,
    requester: &'a [String],
    alias_chain: Vec<String>,
    seen_aliases: BTreeSet<(Vec<String>, String)>,
}

impl SymbolRecord {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> SymbolKind {
        self.kind
    }

    pub fn visibility(&self) -> &SymbolVisibility {
        &self.visibility
    }

    pub fn target_name(&self) -> &str {
        &self.target_name
    }

    pub fn namespace(&self) -> &[String] {
        &self.namespace
    }

    pub fn target_namespace(&self) -> Option<&[String]> {
        self.target_namespace.as_deref()
    }

    pub fn target_path(&self) -> Option<&[String]> {
        self.target_path.as_deref()
    }

    pub fn external_crate(&self) -> Option<&str> {
        self.external_crate.as_deref()
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    pub fn span(&self) -> Option<&SourceSpan> {
        self.span.as_ref()
    }

    pub fn cfg_condition(&self) -> Option<&str> {
        self.cfg_condition.as_deref()
    }

    pub fn is_visible_from(&self, namespace: &[String]) -> VisibilityResult {
        match &self.visibility {
            SymbolVisibility::Public => VisibilityResult::Public,
            SymbolVisibility::Private => {
                if namespace.starts_with(&self.namespace) {
                    VisibilityResult::Reachable
                } else {
                    VisibilityResult::Private
                }
            }
            SymbolVisibility::Restricted(scope) => {
                let Some(restricted) = restricted_namespace(scope, &self.namespace) else {
                    return VisibilityResult::Unknown;
                };
                if namespace.starts_with(&restricted) {
                    VisibilityResult::Reachable
                } else {
                    VisibilityResult::Private
                }
            }
        }
    }
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
            let Some(target) = package.targets().get(module.context_index()) else {
                continue;
            };
            index.add_module_symbols(module, target, &module_conditions);
        }
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
                    && module
                        .parse_output()
                        .inner_attributes()
                        .iter()
                        .any(|attribute| {
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
            left.target_name
                .cmp(&right.target_name)
                .then_with(|| left.context_index.cmp(&right.context_index))
                .then_with(|| left.namespace.cmp(&right.namespace))
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.file_path.cmp(&right.file_path))
                .then_with(|| {
                    left.span
                        .as_ref()
                        .map(SourceSpan::start_line)
                        .cmp(&right.span.as_ref().map(SourceSpan::start_line))
                })
        });
        index
    }

    pub fn symbols(&self) -> &[SymbolRecord] {
        &self.symbols
    }

    pub fn resolve_path(&self, module: &ModuleNode, path: &[String]) -> PathResolution {
        let mut state = ResolveState {
            context_index: module.context_index(),
            requester: module.namespace(),
            alias_chain: Vec::new(),
            seen_aliases: BTreeSet::new(),
        };
        self.resolve_inner(&mut state, module.namespace(), path)
    }

    fn add_module_symbols(
        &mut self,
        module: &ModuleNode,
        target: &TargetContext,
        module_conditions: &BTreeMap<(usize, Vec<String>), Option<String>>,
    ) {
        let evaluator = CfgEvaluator::new(target);
        for item in module.items() {
            let effects =
                evaluator.evaluate_attributes(item.attributes(), module.parse_output().source());
            let item_cfg = merge_cfg(module.cfg_condition(), &effects.conditions);
            match item.kind() {
                ItemKind::Module(child) => {
                    let mut target_namespace = module.namespace().to_vec();
                    target_namespace.push(child.name().to_owned());
                    let cfg_condition = module_conditions
                        .get(&(module.context_index(), target_namespace.clone()))
                        .cloned()
                        .flatten()
                        .or(item_cfg);
                    self.symbols.push(SymbolRecord {
                        name: child.name().to_owned(),
                        kind: SymbolKind::Module,
                        visibility: visibility(item.visibility().kind()),
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
                    });
                }
                ItemKind::Use(use_item) => {
                    let walk = UseWalk {
                        context_index: module.context_index(),
                        target_name: module.target_name(),
                        namespace: module.namespace(),
                        file_path: module.file_path(),
                        source: module.parse_output().source(),
                        cfg_condition: item_cfg.as_deref(),
                        visibility: visibility(item.visibility().kind()),
                    };
                    self.add_use_tree(walk, use_item.tree(), &[]);
                }
                ItemKind::ExternCrate(extern_crate) => {
                    if let Some(crate_name) = extern_crate.name() {
                        let name = extern_crate.alias().unwrap_or(crate_name);
                        self.symbols.push(SymbolRecord {
                            name: name.to_owned(),
                            kind: SymbolKind::ExternalCrate,
                            visibility: visibility(item.visibility().kind()),
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
                        });
                    }
                }
                _ => {
                    if let Some((name, kind)) = named_item(item) {
                        self.symbols.push(SymbolRecord {
                            name,
                            kind,
                            visibility: visibility(item.visibility().kind()),
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
                        });
                    }
                }
            }
        }
    }

    fn add_use_tree(&mut self, walk: UseWalk<'_>, tree: &UseTree, prefix: &[String]) {
        let mut path = prefix.to_vec();
        path.extend(tree.path().iter().cloned());
        if !tree.children().is_empty() {
            for child in tree.children() {
                self.add_use_tree(
                    UseWalk {
                        context_index: walk.context_index,
                        target_name: walk.target_name,
                        namespace: walk.namespace,
                        file_path: walk.file_path,
                        source: walk.source,
                        cfg_condition: walk.cfg_condition,
                        visibility: walk.visibility.clone(),
                    },
                    child,
                    &path,
                );
            }
            return;
        }
        if tree.is_glob() || path.is_empty() {
            return;
        }
        let alias = tree.alias().map(str::to_owned).or_else(|| {
            if path.last().is_some_and(|segment| segment == "self") {
                path.pop();
                path.last().cloned()
            } else {
                path.last().cloned()
            }
        });
        if let Some(name) = alias {
            self.symbols.push(SymbolRecord {
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
                cfg_condition: walk.cfg_condition.map(str::to_owned),
            });
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
                visibility: VisibilityResult::Public,
                alias_chain: state.alias_chain.clone(),
            };
        }
        let candidate = match self.find_symbol(state.context_index, &base, first) {
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
                        visibility: VisibilityResult::Public,
                        alias_chain: state.alias_chain.clone(),
                    };
                }
                return unresolved(PathResolutionKind::Unresolved, &state.alias_chain);
            }
        };
        if candidate.kind == SymbolKind::UseAlias {
            let visibility = candidate.is_visible_from(state.requester);
            if visibility == VisibilityResult::Private {
                return PathResolution {
                    kind: PathResolutionKind::Local,
                    symbol: Some(candidate.clone()),
                    external_crate: None,
                    visibility,
                    alias_chain: state.alias_chain.clone(),
                };
            }
            let key = (candidate.namespace.clone(), candidate.name.clone());
            if !state.seen_aliases.insert(key) {
                return unresolved(PathResolutionKind::AliasCycle, &state.alias_chain);
            }
            state.alias_chain.push(candidate.name.clone());
            let mut redirected = candidate.target_path.clone().unwrap_or_default();
            redirected.extend(rest.iter().skip(1).cloned());
            let resolution = self.resolve_inner(state, &candidate.namespace, &redirected);
            state
                .seen_aliases
                .remove(&(candidate.namespace.clone(), candidate.name.clone()));
            return resolution;
        }
        if let Some(external) = &candidate.external_crate {
            return PathResolution {
                kind: PathResolutionKind::ExternalCrate,
                symbol: Some(candidate.clone()),
                external_crate: Some(external.clone()),
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
                visibility,
                alias_chain: state.alias_chain.clone(),
            };
        }
        if rest.len() > 1 {
            if let Some(next_namespace) = candidate.target_namespace.as_ref() {
                return self.resolve_inner(state, next_namespace, &rest[1..]);
            }
            return unresolved(PathResolutionKind::Unresolved, &state.alias_chain);
        }
        PathResolution {
            kind: PathResolutionKind::Local,
            symbol: Some(candidate.clone()),
            external_crate: None,
            visibility,
            alias_chain: state.alias_chain.clone(),
        }
    }

    fn find_symbol(
        &self,
        context_index: usize,
        namespace: &[String],
        name: &str,
    ) -> Result<Option<&SymbolRecord>, ()> {
        for depth in (0..=namespace.len()).rev() {
            let scope = &namespace[..depth];
            let mut found = self.symbols.iter().filter(|symbol| {
                symbol.context_index == context_index
                    && symbol.namespace == scope
                    && identifier_name(&symbol.name) == identifier_name(name)
            });
            let Some(first) = found.next() else {
                continue;
            };
            if found.next().is_some() {
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
        visibility: VisibilityResult::Unknown,
        alias_chain: alias_chain.to_vec(),
    }
}

fn merge_cfg(parent: Option<&str>, conditions: &[String]) -> Option<String> {
    let mut merged = parent.into_iter().map(str::to_owned).collect::<Vec<_>>();
    merged.extend(conditions.iter().cloned());
    (!merged.is_empty()).then(|| merged.join(" && "))
}

fn visibility(kind: &VisibilityKind) -> SymbolVisibility {
    match kind {
        VisibilityKind::Private => SymbolVisibility::Private,
        VisibilityKind::Public => SymbolVisibility::Public,
        VisibilityKind::Restricted(scope) => SymbolVisibility::Restricted(scope.clone()),
    }
}

fn named_item(item: &Item) -> Option<(String, SymbolKind)> {
    match item.kind() {
        ItemKind::Struct(declaration) => Some((declaration.name()?.to_owned(), SymbolKind::Struct)),
        ItemKind::Enum(declaration) => Some((declaration.name()?.to_owned(), SymbolKind::Enum)),
        ItemKind::Union(declaration) => Some((declaration.name()?.to_owned(), SymbolKind::Union)),
        ItemKind::TypeAlias(declaration) => {
            Some((declaration.name()?.to_owned(), SymbolKind::TypeAlias))
        }
        ItemKind::Trait(trait_item) => Some((trait_item.name()?.to_owned(), SymbolKind::Trait)),
        ItemKind::Constant(declaration) => {
            Some((declaration.name()?.to_owned(), SymbolKind::Constant))
        }
        ItemKind::Static(declaration) => Some((declaration.name()?.to_owned(), SymbolKind::Static)),
        ItemKind::Function(function) => Some((function.name()?.to_owned(), SymbolKind::Function)),
        _ => None,
    }
}

fn restricted_namespace(scope: &str, declaration_namespace: &[String]) -> Option<Vec<String>> {
    let restriction = scope.trim_matches(['(', ')', ' ']);
    match restriction {
        "crate" => Some(Vec::new()),
        "self" => Some(declaration_namespace.to_vec()),
        "super" => {
            Some(declaration_namespace[..declaration_namespace.len().saturating_sub(1)].to_vec())
        }
        value if value.starts_with("in ") => {
            let path = value.strip_prefix("in ")?.trim();
            if path == "crate" {
                return Some(Vec::new());
            }
            if path == "self" {
                return Some(declaration_namespace.to_vec());
            }
            let mut namespace = declaration_namespace.to_vec();
            for segment in path.split("::") {
                match segment {
                    "crate" => namespace.clear(),
                    "self" => {}
                    "super" => {
                        namespace.pop()?;
                    }
                    "" => {}
                    name => namespace.push(name.to_owned()),
                }
            }
            Some(namespace)
        }
        _ => None,
    }
}
