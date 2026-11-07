use cop_analysis::{
    AnalysisSnapshot, DefinitionId, PathResolution, PathResolutionKind, PathRootKind, SymbolKind,
    SymbolRecord,
};
use cop_syntax::{PathSyntax, TypeSyntax, TypeSyntaxKind};

use crate::support::SyntaxSupport;

pub(super) enum TypeOrigin<'a> {
    Local(&'a SymbolRecord),
    External,
    Primitive,
}

pub(super) struct TypeOriginResolver;

impl TypeOriginResolver {
    pub(super) fn resolve<'a>(
        snapshot: &'a AnalysisSnapshot,
        resolution: &'a PathResolution,
        visited_aliases: &mut Vec<DefinitionId>,
        conditions: &mut Vec<Option<String>>,
    ) -> Option<TypeOrigin<'a>> {
        Self::resolution_conditions(resolution, conditions);
        match resolution.kind() {
            PathResolutionKind::ExternalCrate => Some(TypeOrigin::External),
            PathResolutionKind::Local => {
                if resolution.root().kind() == PathRootKind::Primitive {
                    return Some(TypeOrigin::Primitive);
                }
                let symbol = resolution.symbol()?;
                if symbol.kind() == SymbolKind::TypeAlias {
                    let definition = symbol.definition_id();
                    if visited_aliases.contains(&definition) {
                        return None;
                    }
                    visited_aliases.push(definition);
                    let target = snapshot.facts().type_alias(definition).and_then(|alias| {
                        let alias_module = snapshot.modules().get(alias.module_id().index())?;
                        conditions.push(alias.condition().map(str::to_owned));
                        let path = path_in_type(alias.right_hand_side())?;
                        let reference =
                            snapshot.facts().reference_at(alias_module, path.bounds())?;
                        Self::resolve(
                            snapshot,
                            reference.resolution(),
                            visited_aliases,
                            conditions,
                        )
                    });
                    visited_aliases.pop();
                    return target;
                }
                is_repository_type(symbol.kind()).then_some(TypeOrigin::Local(symbol))
            }
            PathResolutionKind::Unresolved
            | PathResolutionKind::Ambiguous
            | PathResolutionKind::AliasCycle => None,
        }
    }

    pub(super) fn resolution_conditions(
        resolution: &PathResolution,
        conditions: &mut Vec<Option<String>>,
    ) {
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

    pub(super) fn conditions_may_coexist_with_impl(
        implementation_condition: Option<&str>,
        symbol_conditions: &[Option<String>],
    ) -> bool {
        let mut conditions = Vec::with_capacity(symbol_conditions.len() + 1);
        conditions.push(implementation_condition);
        conditions.extend(symbol_conditions.iter().map(Option::as_deref));
        SyntaxSupport::conditions_may_coexist_all(&conditions)
    }
}

fn is_repository_type(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Union | SymbolKind::TypeAlias
    )
}

fn path_in_type(ty: &TypeSyntax) -> Option<&PathSyntax> {
    match ty.kind() {
        TypeSyntaxKind::Path(path) if path.qualified_self().is_none() => Some(path),
        TypeSyntaxKind::Reference { target, .. }
        | TypeSyntaxKind::Pointer { target, .. }
        | TypeSyntaxKind::Parenthesized(target) => path_in_type(target),
        _ => None,
    }
}
