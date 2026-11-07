use std::{collections::BTreeMap, ops::Range};

use cop_analysis::{
    AnalysisSnapshot, DefinitionId, ExternalItemKind, ImplOwnerRecord, ModuleNode, PathResolution,
    PathResolutionKind, PathRootKind, SymbolKind, SymbolRecord,
};
use cop_syntax::{
    GenericArgumentSyntax, GenericParameterKind, GenericParameterSyntax, PathSyntax, TypeSyntax,
    TypeSyntaxKind,
};

#[derive(Clone, Debug)]
pub(super) struct LocalAnchor<'a> {
    symbol: &'a SymbolRecord,
    bounds: Range<usize>,
    conditions: Vec<Option<String>>,
}

impl<'a> LocalAnchor<'a> {
    pub(super) fn symbol(&self) -> &'a SymbolRecord {
        self.symbol
    }

    pub(super) fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub(super) fn conditions(&self) -> &[Option<String>] {
        &self.conditions
    }
}

#[derive(Clone, Debug)]
pub(super) struct UnknownType {
    bounds: Range<usize>,
}

impl UnknownType {
    pub(super) fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }
}

#[derive(Clone, Debug)]
pub(super) enum TargetOrigin<'a> {
    Local(LocalAnchor<'a>),
    Other(Vec<Option<String>>),
    Unknown(UnknownType),
}

#[derive(Clone, Debug)]
pub(super) enum OrphanAnchor<'a> {
    Local(LocalAnchor<'a>),
    NoAnchor,
    Unknown(UnknownType),
}

#[derive(Clone)]
struct TypeView<'a> {
    syntax: &'a TypeSyntax,
    module: &'a ModuleNode,
    variables: BTreeMap<String, TypeView<'a>>,
    projected_bounds: Option<Range<usize>>,
}

impl<'a> TypeView<'a> {
    fn root(syntax: &'a TypeSyntax, module: &'a ModuleNode) -> Self {
        Self {
            syntax,
            module,
            variables: BTreeMap::new(),
            projected_bounds: None,
        }
    }

    fn bounds(&self) -> Range<usize> {
        self.projected_bounds
            .clone()
            .unwrap_or_else(|| self.syntax.bounds())
    }

    fn child(&self, syntax: &'a TypeSyntax) -> Self {
        Self {
            syntax,
            module: self.module,
            variables: self.variables.clone(),
            projected_bounds: self.projected_bounds.clone(),
        }
    }
}

enum TypeInspection<'a> {
    Local(&'a SymbolRecord, Range<usize>),
    Other { uncovered_parameter: bool },
    NonNominal(Range<usize>),
    Unknown(Range<usize>),
}

enum ScanStep<'a> {
    Continue,
    Found(LocalAnchor<'a>),
    Unknown(UnknownType),
    InvalidOrphan,
}

pub(super) struct TraitImplLocalityResolver<'a> {
    snapshot: &'a AnalysisSnapshot,
    module: &'a ModuleNode,
    implementation: &'a ImplOwnerRecord,
    impl_type_parameters: Vec<String>,
    conditions: Vec<Option<String>>,
    visited_aliases: Vec<DefinitionId>,
}

impl<'a> TraitImplLocalityResolver<'a> {
    pub(super) fn new(
        snapshot: &'a AnalysisSnapshot,
        module: &'a ModuleNode,
        implementation: &'a ImplOwnerRecord,
    ) -> Self {
        let impl_type_parameters = implementation
            .generic_parameters()
            .iter()
            .filter(|parameter| parameter.kind() == GenericParameterKind::Type)
            .map(|parameter| parameter.name().to_owned())
            .collect();
        Self {
            snapshot,
            module,
            implementation,
            impl_type_parameters,
            conditions: Vec::new(),
            visited_aliases: Vec::new(),
        }
    }

    pub(super) fn resolve_target(&mut self) -> TargetOrigin<'a> {
        let Some(target) = self.implementation.target() else {
            return TargetOrigin::Unknown(UnknownType {
                bounds: self.implementation.bounds(),
            });
        };
        let view = TypeView::root(target, self.module);
        match self.inspect(&view, false) {
            TypeInspection::Local(symbol, bounds) => TargetOrigin::Local(LocalAnchor {
                symbol,
                bounds,
                conditions: self.conditions.clone(),
            }),
            TypeInspection::Other {
                uncovered_parameter: true,
            } => TargetOrigin::Unknown(UnknownType {
                bounds: target.bounds(),
            }),
            TypeInspection::Other {
                uncovered_parameter: false,
            } => TargetOrigin::Other(self.conditions.clone()),
            TypeInspection::NonNominal(bounds) => TargetOrigin::Unknown(UnknownType { bounds }),
            TypeInspection::Unknown(bounds) => TargetOrigin::Unknown(UnknownType { bounds }),
        }
    }

    pub(super) fn resolve_orphan_anchor(&mut self) -> OrphanAnchor<'a> {
        let Some(target) = self.implementation.target() else {
            return OrphanAnchor::Unknown(UnknownType {
                bounds: self.implementation.bounds(),
            });
        };
        let mut uncovered_parameter = false;
        match self.scan_type(
            &TypeView::root(target, self.module),
            &mut uncovered_parameter,
        ) {
            ScanStep::Continue => {}
            ScanStep::Found(anchor) => return OrphanAnchor::Local(anchor),
            ScanStep::Unknown(unknown) => return OrphanAnchor::Unknown(unknown),
            ScanStep::InvalidOrphan => return OrphanAnchor::NoAnchor,
        }
        let Some(trait_path) = self.implementation.trait_syntax() else {
            return OrphanAnchor::Unknown(UnknownType {
                bounds: self.implementation.bounds(),
            });
        };
        let Some(last_segment) = trait_path.segments().last() else {
            return OrphanAnchor::Unknown(UnknownType {
                bounds: trait_path.bounds(),
            });
        };
        for argument in last_segment.arguments() {
            let GenericArgumentSyntax::Type(argument_type) = argument else {
                continue;
            };
            match self.scan_type(
                &TypeView::root(argument_type, self.module),
                &mut uncovered_parameter,
            ) {
                ScanStep::Continue => {}
                ScanStep::Found(anchor) => return OrphanAnchor::Local(anchor),
                ScanStep::Unknown(unknown) => return OrphanAnchor::Unknown(unknown),
                ScanStep::InvalidOrphan => return OrphanAnchor::NoAnchor,
            }
        }
        OrphanAnchor::NoAnchor
    }

    fn scan_type(&mut self, view: &TypeView<'a>, uncovered_parameter: &mut bool) -> ScanStep<'a> {
        match self.inspect(view, false) {
            TypeInspection::Local(symbol, bounds) => {
                if *uncovered_parameter {
                    ScanStep::InvalidOrphan
                } else {
                    ScanStep::Found(LocalAnchor {
                        symbol,
                        bounds,
                        conditions: self.conditions.clone(),
                    })
                }
            }
            TypeInspection::Other {
                uncovered_parameter: found,
            } => {
                *uncovered_parameter |= found;
                ScanStep::Continue
            }
            TypeInspection::NonNominal(_) => ScanStep::Continue,
            TypeInspection::Unknown(bounds) => ScanStep::Unknown(UnknownType { bounds }),
        }
    }

    fn inspect(&mut self, view: &TypeView<'a>, covered: bool) -> TypeInspection<'a> {
        match view.syntax.kind() {
            TypeSyntaxKind::Path(path) => self.inspect_path(view, path, covered),
            TypeSyntaxKind::Reference { target, .. } => self.inspect(&view.child(target), covered),
            TypeSyntaxKind::Parenthesized(target) => self.inspect(&view.child(target), covered),
            TypeSyntaxKind::Pointer { .. }
            | TypeSyntaxKind::Tuple(_)
            | TypeSyntaxKind::Slice(_)
            | TypeSyntaxKind::Array { .. }
            | TypeSyntaxKind::FunctionPointer { .. }
            | TypeSyntaxKind::ImplTrait(_)
            | TypeSyntaxKind::DynTrait(_)
            | TypeSyntaxKind::TraitBounds(_)
            | TypeSyntaxKind::Never => TypeInspection::NonNominal(view.bounds()),
            TypeSyntaxKind::Infer | TypeSyntaxKind::Macro { .. } => {
                TypeInspection::Unknown(view.bounds())
            }
        }
    }

    fn inspect_path(
        &mut self,
        view: &TypeView<'a>,
        path: &'a PathSyntax,
        covered: bool,
    ) -> TypeInspection<'a> {
        if path.qualified_self().is_some() {
            return TypeInspection::Unknown(view.bounds());
        }
        if let Some(name) = bare_type_parameter(path) {
            if let Some(replacement) = view.variables.get(name).cloned() {
                return self.inspect(&replacement, covered);
            }
            if self
                .impl_type_parameters
                .iter()
                .any(|parameter| parameter == name)
            {
                return TypeInspection::Other {
                    uncovered_parameter: !covered,
                };
            }
        }
        let Some(reference) = self
            .snapshot
            .facts()
            .reference_at(view.module, path.bounds())
        else {
            return TypeInspection::Unknown(view.bounds());
        };
        push_resolution_conditions(reference.resolution(), &mut self.conditions);
        match reference.resolution().kind() {
            PathResolutionKind::ExternalCrate => {
                let identity = reference.resolution().external_item_identity();
                if identity.is_some_and(|item| item.kind() != ExternalItemKind::Type) {
                    return TypeInspection::Unknown(view.bounds());
                }
                if identity.is_some_and(|item| item.is_fundamental()) {
                    return self.inspect_fundamental_arguments(view, path, covered);
                }
                TypeInspection::Other {
                    uncovered_parameter: false,
                }
            }
            PathResolutionKind::Local => {
                if reference.resolution().root().kind() == PathRootKind::Primitive {
                    return TypeInspection::Other {
                        uncovered_parameter: false,
                    };
                }
                let Some(symbol) = reference.resolution().symbol() else {
                    return TypeInspection::Unknown(view.bounds());
                };
                match symbol.kind() {
                    SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Union => {
                        TypeInspection::Local(symbol, view.bounds())
                    }
                    SymbolKind::TypeAlias => self.inspect_alias(view, path, symbol),
                    _ => TypeInspection::Unknown(view.bounds()),
                }
            }
            PathResolutionKind::Unresolved
            | PathResolutionKind::Ambiguous
            | PathResolutionKind::AliasCycle => TypeInspection::Unknown(view.bounds()),
        }
    }

    fn inspect_fundamental_arguments(
        &mut self,
        view: &TypeView<'a>,
        path: &'a PathSyntax,
        covered: bool,
    ) -> TypeInspection<'a> {
        let mut uncovered_parameter = false;
        if let Some(segment) = path.segments().last() {
            for argument in segment.arguments() {
                if let GenericArgumentSyntax::Type(ty) = argument {
                    match self.inspect(&view.child(ty), covered) {
                        TypeInspection::Local(symbol, bounds) => {
                            return TypeInspection::Local(symbol, bounds)
                        }
                        TypeInspection::Other {
                            uncovered_parameter: found,
                        } => uncovered_parameter |= found,
                        TypeInspection::NonNominal(_) => {}
                        TypeInspection::Unknown(bounds) => return TypeInspection::Unknown(bounds),
                    }
                }
            }
        }
        TypeInspection::Other {
            uncovered_parameter,
        }
    }

    fn inspect_alias(
        &mut self,
        view: &TypeView<'a>,
        path: &'a PathSyntax,
        symbol: &'a SymbolRecord,
    ) -> TypeInspection<'a> {
        let definition = symbol.definition_id();
        if self.visited_aliases.contains(&definition) {
            return TypeInspection::Unknown(view.bounds());
        }
        let Some(alias) = self.snapshot.facts().type_alias(definition) else {
            return TypeInspection::Unknown(view.bounds());
        };
        let Some(alias_module) = self.snapshot.modules().get(alias.module_id().index()) else {
            return TypeInspection::Unknown(view.bounds());
        };
        let Some(variables) = bind_alias_arguments(alias.parameters(), path, view, alias_module)
        else {
            return TypeInspection::Unknown(view.bounds());
        };
        self.conditions.push(alias.condition().map(str::to_owned));
        self.visited_aliases.push(definition);
        let rhs = TypeView {
            syntax: alias.right_hand_side(),
            module: alias_module,
            variables,
            projected_bounds: Some(view.bounds()),
        };
        let inspected = self.inspect(&rhs, false);
        self.visited_aliases.pop();
        inspected
    }
}

fn bare_type_parameter(path: &PathSyntax) -> Option<&str> {
    if path.has_leading_colon() || path.segments().len() != 1 {
        return None;
    }
    let segment = path.segments().first()?;
    segment.arguments().is_empty().then_some(segment.name())
}

fn bind_alias_arguments<'a>(
    parameters: &'a [GenericParameterSyntax],
    path: &'a PathSyntax,
    use_site: &TypeView<'a>,
    alias_module: &'a ModuleNode,
) -> Option<BTreeMap<String, TypeView<'a>>> {
    let arguments = path.segments().last()?.arguments();
    let mut argument_index = 0;
    let mut variables = BTreeMap::new();
    for parameter in parameters {
        let argument = arguments.get(argument_index).filter(|argument| {
            matches!(
                (parameter.kind(), argument),
                (
                    GenericParameterKind::Lifetime,
                    GenericArgumentSyntax::Lifetime(_)
                ) | (GenericParameterKind::Type, GenericArgumentSyntax::Type(_))
                    | (GenericParameterKind::Const, GenericArgumentSyntax::Const(_))
            )
        });
        let Some(argument) = argument else {
            match parameter.kind() {
                GenericParameterKind::Type => {
                    let default = parameter.default_type()?;
                    variables.insert(
                        parameter.name().to_owned(),
                        TypeView {
                            syntax: default,
                            module: alias_module,
                            variables: variables.clone(),
                            projected_bounds: Some(use_site.bounds()),
                        },
                    );
                }
                GenericParameterKind::Const => {
                    parameter.default_const()?;
                }
                GenericParameterKind::Lifetime => return None,
            }
            continue;
        };
        argument_index += 1;
        if let (GenericParameterKind::Type, GenericArgumentSyntax::Type(ty)) =
            (parameter.kind(), argument)
        {
            variables.insert(
                parameter.name().to_owned(),
                TypeView {
                    syntax: ty,
                    module: use_site.module,
                    variables: use_site.variables.clone(),
                    projected_bounds: use_site.projected_bounds.clone(),
                },
            );
        }
    }
    (argument_index == arguments.len()).then_some(variables)
}

fn push_resolution_conditions(resolution: &PathResolution, conditions: &mut Vec<Option<String>>) {
    if let Some(symbol) = resolution.symbol() {
        conditions.push(symbol.cfg_condition().map(str::to_owned));
    }
    conditions.extend(
        resolution
            .alias_symbols()
            .iter()
            .map(|symbol| symbol.cfg_condition().map(str::to_owned)),
    );
}
