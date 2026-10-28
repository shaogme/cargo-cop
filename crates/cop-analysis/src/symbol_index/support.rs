use cop_cfg::CfgCondition;
use cop_syntax::{ItemSyntaxKind, PathSyntax, TypeSyntax, TypeSyntaxKind, VisibilityKind};

use super::{SymbolKind, SymbolVisibility};

pub(crate) struct SymbolIndexSupport;

impl SymbolIndexSupport {
    pub(crate) fn conditions_may_coexist(left: Option<&str>, right: Option<&str>) -> bool {
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

    pub(crate) fn visibility(kind: &VisibilityKind) -> SymbolVisibility {
        match kind {
            VisibilityKind::Private => SymbolVisibility::Private,
            VisibilityKind::Public => SymbolVisibility::Public,
            VisibilityKind::Restricted(scope) => SymbolVisibility::Restricted(scope.clone()),
        }
    }

    pub(crate) fn associated_namespace(
        kind: &ItemSyntaxKind,
        module_namespace: &[String],
        name: &str,
    ) -> Option<Vec<String>> {
        if !matches!(
            kind,
            ItemSyntaxKind::Struct(_)
                | ItemSyntaxKind::Enum(_)
                | ItemSyntaxKind::Union(_)
                | ItemSyntaxKind::Trait(_)
        ) {
            return None;
        }
        let mut namespace = module_namespace.to_vec();
        namespace.push(name.to_owned());
        Some(namespace)
    }

    pub(crate) fn associated_item_name(kind: &ItemSyntaxKind) -> Option<String> {
        match kind {
            ItemSyntaxKind::Function { name: Some(name) }
            | ItemSyntaxKind::TypeAlias(Some(name))
            | ItemSyntaxKind::Constant(Some(name))
            | ItemSyntaxKind::Static(Some(name)) => Some(name.clone()),
            _ => None,
        }
    }

    pub(crate) fn type_path(ty: &TypeSyntax) -> Option<Vec<String>> {
        match ty.kind() {
            TypeSyntaxKind::Path(path) if path.qualified_self().is_none() => {
                Some(Self::path_segments(path))
            }
            TypeSyntaxKind::Reference { target, .. }
            | TypeSyntaxKind::Pointer { target, .. }
            | TypeSyntaxKind::Parenthesized(target) => Self::type_path(target),
            _ => None,
        }
    }

    pub(crate) fn path_segments(path: &PathSyntax) -> Vec<String> {
        let mut segments = path
            .segments()
            .iter()
            .map(|segment| segment.name().to_owned())
            .collect::<Vec<_>>();
        if path.has_leading_colon() {
            segments.insert(0, "::".to_owned());
        }
        segments
    }

    pub(crate) fn named_item(kind: &ItemSyntaxKind) -> Option<(String, SymbolKind)> {
        match kind {
            ItemSyntaxKind::Struct(name) => Some((name.as_ref()?.to_owned(), SymbolKind::Struct)),
            ItemSyntaxKind::Enum(name) => Some((name.as_ref()?.to_owned(), SymbolKind::Enum)),
            ItemSyntaxKind::Union(name) => Some((name.as_ref()?.to_owned(), SymbolKind::Union)),
            ItemSyntaxKind::TypeAlias(name) => {
                Some((name.as_ref()?.to_owned(), SymbolKind::TypeAlias))
            }
            ItemSyntaxKind::Trait(name) => Some((name.as_ref()?.to_owned(), SymbolKind::Trait)),
            ItemSyntaxKind::Constant(name) => {
                Some((name.as_ref()?.to_owned(), SymbolKind::Constant))
            }
            ItemSyntaxKind::Static(name) => Some((name.as_ref()?.to_owned(), SymbolKind::Static)),
            ItemSyntaxKind::Function { name } => {
                Some((name.as_ref()?.to_owned(), SymbolKind::Function))
            }
            _ => None,
        }
    }
}
