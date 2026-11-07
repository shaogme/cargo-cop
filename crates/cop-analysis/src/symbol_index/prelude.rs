use std::iter::once;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalItemKind {
    Trait,
    Type,
    Variant,
    Function,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalItemSource {
    ImplicitPrelude,
    ExplicitPath,
    UseAlias,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalItemIdentity {
    canonical_path: &'static str,
    kind: ExternalItemKind,
    fundamental: bool,
}

impl ExternalItemIdentity {
    pub fn canonical_path(&self) -> &str {
        self.canonical_path
    }

    pub fn kind(&self) -> ExternalItemKind {
        self.kind
    }

    pub fn is_fundamental(&self) -> bool {
        self.fundamental
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PreludeProvider {
    Core,
    Std,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PreludeConfig {
    edition: String,
    provider: PreludeProvider,
}

impl PreludeConfig {
    pub(super) fn new(edition: &str, provider: PreludeProvider) -> Self {
        Self {
            edition: edition.to_owned(),
            provider,
        }
    }

    pub(super) fn provider_crate(&self) -> &'static str {
        match self.provider {
            PreludeProvider::Core => "core",
            PreludeProvider::Std => "std",
        }
    }
}

struct CatalogItem {
    name: &'static str,
    canonical_path: &'static str,
    kind: ExternalItemKind,
    first_edition: &'static str,
    core_prelude: bool,
    std_prelude: bool,
    paths: &'static [&'static str],
    fundamental: bool,
}

pub(super) struct ExternalItemCatalog;

const COMMON: &str = "2015";
const EDITION_2021: &str = "2021";
const EDITION_2024: &str = "2024";

macro_rules! trait_item {
    ($name:literal, $canonical_path:literal, $edition:ident, $core:literal, $std:literal) => {
        CatalogItem {
            name: $name,
            canonical_path: $canonical_path,
            kind: ExternalItemKind::Trait,
            first_edition: $edition,
            core_prelude: $core,
            std_prelude: $std,
            paths: &[$canonical_path],
            fundamental: false,
        }
    };
}

macro_rules! type_item {
    ($name:literal, $canonical_path:literal, $edition:ident, $core:literal, $std:literal, [$($path:literal),+]) => {
        CatalogItem {
            name: $name,
            canonical_path: $canonical_path,
            kind: ExternalItemKind::Type,
            first_edition: $edition,
            core_prelude: $core,
            std_prelude: $std,
            paths: &[$($path),+],
            fundamental: false,
        }
    };
}

macro_rules! fundamental_type_item {
    ($name:literal, $canonical_path:literal, $edition:ident, $core:literal, $std:literal, [$($path:literal),+]) => {
        CatalogItem {
            name: $name,
            canonical_path: $canonical_path,
            kind: ExternalItemKind::Type,
            first_edition: $edition,
            core_prelude: $core,
            std_prelude: $std,
            paths: &[$($path),+],
            fundamental: true,
        }
    };
}

macro_rules! variant_item {
    ($name:literal, $canonical_path:literal, $edition:ident) => {
        CatalogItem {
            name: $name,
            canonical_path: $canonical_path,
            kind: ExternalItemKind::Variant,
            first_edition: $edition,
            core_prelude: true,
            std_prelude: true,
            paths: &[$canonical_path],
            fundamental: false,
        }
    };
}

macro_rules! function_item {
    ($name:literal, $canonical_path:literal, $edition:ident) => {
        CatalogItem {
            name: $name,
            canonical_path: $canonical_path,
            kind: ExternalItemKind::Function,
            first_edition: $edition,
            core_prelude: true,
            std_prelude: true,
            paths: &[$canonical_path],
            fundamental: false,
        }
    };
}

const ITEMS: &[CatalogItem] = &[
    trait_item!("AsMut", "core::convert::AsMut", COMMON, true, true),
    trait_item!("AsRef", "core::convert::AsRef", COMMON, true, true),
    trait_item!("AsyncFn", "core::ops::AsyncFn", COMMON, true, true),
    trait_item!("AsyncFnMut", "core::ops::AsyncFnMut", COMMON, true, true),
    trait_item!("AsyncFnOnce", "core::ops::AsyncFnOnce", COMMON, true, true),
    fundamental_type_item!(
        "Box",
        "alloc::boxed::Box",
        COMMON,
        false,
        true,
        ["std::boxed::Box", "alloc::boxed::Box"]
    ),
    trait_item!("Clone", "core::clone::Clone", COMMON, true, true),
    trait_item!("Copy", "core::marker::Copy", COMMON, true, true),
    function_item!("align_of", "core::mem::align_of", COMMON),
    function_item!("align_of_val", "core::mem::align_of_val", COMMON),
    trait_item!("Default", "core::default::Default", COMMON, true, true),
    trait_item!("Debug", "core::fmt::Debug", COMMON, true, true),
    trait_item!("Drop", "core::ops::Drop", COMMON, true, true),
    trait_item!(
        "DoubleEndedIterator",
        "core::iter::DoubleEndedIterator",
        COMMON,
        true,
        true
    ),
    trait_item!("Eq", "core::cmp::Eq", COMMON, true, true),
    variant_item!("Err", "core::result::Result::Err", COMMON),
    trait_item!(
        "ExactSizeIterator",
        "core::iter::ExactSizeIterator",
        COMMON,
        true,
        true
    ),
    trait_item!("Extend", "core::iter::Extend", COMMON, true, true),
    trait_item!("Fn", "core::ops::Fn", COMMON, true, true),
    trait_item!("FnMut", "core::ops::FnMut", COMMON, true, true),
    trait_item!("FnOnce", "core::ops::FnOnce", COMMON, true, true),
    trait_item!("From", "core::convert::From", COMMON, true, true),
    trait_item!(
        "FromIterator",
        "core::iter::FromIterator",
        EDITION_2021,
        true,
        true
    ),
    trait_item!("Future", "core::future::Future", EDITION_2024, true, true),
    trait_item!("Display", "core::fmt::Display", COMMON, false, false),
    trait_item!("Error", "core::error::Error", COMMON, false, false),
    trait_item!("Hash", "core::hash::Hash", COMMON, true, true),
    trait_item!("Into", "core::convert::Into", COMMON, true, true),
    trait_item!(
        "IntoFuture",
        "core::future::IntoFuture",
        EDITION_2024,
        true,
        true
    ),
    trait_item!(
        "IntoIterator",
        "core::iter::IntoIterator",
        COMMON,
        true,
        true
    ),
    trait_item!("Iterator", "core::iter::Iterator", COMMON, true, true),
    variant_item!("None", "core::option::Option::None", COMMON),
    type_item!(
        "Option",
        "core::option::Option",
        COMMON,
        true,
        true,
        ["core::option::Option", "std::option::Option"]
    ),
    trait_item!("Ord", "core::cmp::Ord", COMMON, true, true),
    trait_item!("PartialEq", "core::cmp::PartialEq", COMMON, true, true),
    trait_item!("PartialOrd", "core::cmp::PartialOrd", COMMON, true, true),
    fundamental_type_item!(
        "Pin",
        "core::pin::Pin",
        COMMON,
        false,
        false,
        ["core::pin::Pin", "std::pin::Pin"]
    ),
    variant_item!("Ok", "core::result::Result::Ok", COMMON),
    type_item!(
        "Result",
        "core::result::Result",
        COMMON,
        true,
        true,
        ["core::result::Result", "std::result::Result"]
    ),
    trait_item!("Send", "core::marker::Send", COMMON, true, true),
    trait_item!("Sized", "core::marker::Sized", COMMON, true, true),
    variant_item!("Some", "core::option::Option::Some", COMMON),
    trait_item!("Sync", "core::marker::Sync", COMMON, true, true),
    type_item!(
        "String",
        "alloc::string::String",
        COMMON,
        false,
        true,
        ["std::string::String", "alloc::string::String"]
    ),
    CatalogItem {
        name: "ToOwned",
        canonical_path: "alloc::borrow::ToOwned",
        kind: ExternalItemKind::Trait,
        first_edition: COMMON,
        core_prelude: false,
        std_prelude: true,
        paths: &["alloc::borrow::ToOwned", "std::borrow::ToOwned"],
        fundamental: false,
    },
    CatalogItem {
        name: "ToString",
        canonical_path: "alloc::string::ToString",
        kind: ExternalItemKind::Trait,
        first_edition: COMMON,
        core_prelude: false,
        std_prelude: true,
        paths: &["alloc::string::ToString", "std::string::ToString"],
        fundamental: false,
    },
    trait_item!(
        "TryFrom",
        "core::convert::TryFrom",
        EDITION_2021,
        true,
        true
    ),
    trait_item!(
        "TryInto",
        "core::convert::TryInto",
        EDITION_2021,
        true,
        true
    ),
    trait_item!("Unpin", "core::marker::Unpin", COMMON, true, true),
    type_item!(
        "Vec",
        "alloc::vec::Vec",
        COMMON,
        false,
        true,
        ["std::vec::Vec", "alloc::vec::Vec"]
    ),
    function_item!("drop", "core::mem::drop", COMMON),
    function_item!("size_of", "core::mem::size_of", COMMON),
    function_item!("size_of_val", "core::mem::size_of_val", COMMON),
];

impl ExternalItemCatalog {
    pub(super) fn prelude_item(
        name: &str,
        configuration: &PreludeConfig,
    ) -> Option<ExternalItemIdentity> {
        ITEMS
            .iter()
            .find(|item| {
                item.name == name
                    && edition_includes(configuration.edition.as_str(), item.first_edition)
                    && match configuration.provider {
                        PreludeProvider::Core => item.core_prelude,
                        PreludeProvider::Std => item.core_prelude || item.std_prelude,
                    }
            })
            .map(identity)
    }

    pub(super) fn explicit_item(crate_name: &str, path: &[String]) -> Option<ExternalItemIdentity> {
        let full_path = once(crate_name)
            .chain(path.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("::");
        let normalized_std_path =
            (crate_name == "std").then(|| format!("core::{}", path.join("::")));
        ITEMS
            .iter()
            .find(|item| {
                item.paths.contains(&full_path.as_str())
                    || normalized_std_path
                        .as_deref()
                        .is_some_and(|normalized| normalized == item.canonical_path)
            })
            .map(identity)
    }
}

fn identity(item: &CatalogItem) -> ExternalItemIdentity {
    ExternalItemIdentity {
        canonical_path: item.canonical_path,
        kind: item.kind,
        fundamental: item.fundamental,
    }
}

fn edition_includes(edition: &str, first_edition: &str) -> bool {
    edition >= first_edition
}

#[cfg(test)]
mod tests {
    use super::{ExternalItemCatalog, ExternalItemKind, PreludeConfig, PreludeProvider};

    #[test]
    fn standard_trait_reexports_share_canonical_identity() {
        let config = PreludeConfig::new("2021", PreludeProvider::Std);
        let prelude = ExternalItemCatalog::prelude_item("From", &config).expect("prelude trait");
        let core =
            ExternalItemCatalog::explicit_item("core", &["convert".to_owned(), "From".to_owned()])
                .expect("core trait");
        let standard =
            ExternalItemCatalog::explicit_item("std", &["convert".to_owned(), "From".to_owned()])
                .expect("standard trait");

        assert_eq!(prelude.canonical_path(), core.canonical_path());
        assert_eq!(core.canonical_path(), standard.canonical_path());
        assert_eq!(prelude.kind(), ExternalItemKind::Trait);
    }

    #[test]
    fn prelude_catalog_tracks_edition_and_provider() {
        let core_2015 = PreludeConfig::new("2015", PreludeProvider::Core);
        let core_2021 = PreludeConfig::new("2021", PreludeProvider::Core);
        let core_2024 = PreludeConfig::new("2024", PreludeProvider::Core);
        let std_2024 = PreludeConfig::new("2024", PreludeProvider::Std);

        assert!(ExternalItemCatalog::prelude_item("FromIterator", &core_2015).is_none());
        assert!(ExternalItemCatalog::prelude_item("FromIterator", &core_2021).is_some());
        assert!(ExternalItemCatalog::prelude_item("Future", &core_2021).is_none());
        assert!(ExternalItemCatalog::prelude_item("Future", &core_2024).is_some());
        assert!(ExternalItemCatalog::prelude_item("DoubleEndedIterator", &core_2015).is_some());
        assert!(ExternalItemCatalog::prelude_item("String", &core_2024).is_none());
        assert!(ExternalItemCatalog::prelude_item("String", &std_2024).is_some());
    }

    #[test]
    fn core_prelude_includes_debug_and_hash() {
        let config = PreludeConfig::new("2015", PreludeProvider::Core);

        for (name, canonical_path) in [("Debug", "core::fmt::Debug"), ("Hash", "core::hash::Hash")]
        {
            let item = ExternalItemCatalog::prelude_item(name, &config).expect("core prelude");
            assert_eq!(item.canonical_path(), canonical_path);
            assert_eq!(item.kind(), ExternalItemKind::Trait);
        }
    }

    #[test]
    fn standard_non_trait_items_retain_their_kind() {
        let config = PreludeConfig::new("2021", PreludeProvider::Std);
        assert_eq!(
            ExternalItemCatalog::prelude_item("Option", &config)
                .expect("Option")
                .kind(),
            ExternalItemKind::Type
        );
        assert_eq!(
            ExternalItemCatalog::prelude_item("Some", &config)
                .expect("Some")
                .kind(),
            ExternalItemKind::Variant
        );
        assert_eq!(
            ExternalItemCatalog::prelude_item("drop", &config)
                .expect("drop")
                .kind(),
            ExternalItemKind::Function
        );
    }
}
