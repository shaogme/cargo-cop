use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use crate::metadata::DependencyDto;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyKind {
    Normal,
    Build,
    Development,
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CargoDependency {
    dependency_key: String,
    rust_name: String,
    package_name: String,
    package_path: Option<PathBuf>,
    kind: DependencyKind,
    target_condition: Option<String>,
    optional: bool,
    uses_default_features: bool,
    features: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DependencyIndex {
    dependencies: Vec<CargoDependency>,
}

impl CargoDependency {
    pub fn dependency_key(&self) -> &str {
        &self.dependency_key
    }

    pub fn rust_name(&self) -> &str {
        &self.rust_name
    }

    pub fn package_name(&self) -> &str {
        &self.package_name
    }

    pub fn package_path(&self) -> Option<&Path> {
        self.package_path.as_deref()
    }

    pub fn kind(&self) -> &DependencyKind {
        &self.kind
    }

    pub fn target_condition(&self) -> Option<&str> {
        self.target_condition.as_deref()
    }

    pub fn is_optional(&self) -> bool {
        self.optional
    }

    pub fn uses_default_features(&self) -> bool {
        self.uses_default_features
    }

    pub fn features(&self) -> &[String] {
        &self.features
    }
}

impl DependencyKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Normal => "normal",
            Self::Build => "build",
            Self::Development => "dev",
            Self::Other(kind) => kind,
        }
    }
}

impl DependencyIndex {
    pub fn dependencies(&self) -> &[CargoDependency] {
        &self.dependencies
    }

    pub fn find_rust_name(&self, name: &str) -> &[CargoDependency] {
        let start = self
            .dependencies
            .partition_point(|dependency| dependency.rust_name.as_str() < name);
        let end = start
            + self.dependencies[start..]
                .partition_point(|dependency| dependency.rust_name.as_str() == name);
        &self.dependencies[start..end]
    }

    pub fn standard_crates() -> &'static [&'static str] {
        &["alloc", "core", "proc_macro", "std"]
    }

    pub(crate) fn from_metadata(dependencies: Vec<DependencyDto>) -> Self {
        let mut dependencies = dependencies
            .into_iter()
            .map(|dependency| {
                let dependency_key = dependency.rename().unwrap_or_else(|| dependency.name());
                let rust_name = dependency_key.replace('-', "_");
                let kind = match dependency.kind().unwrap_or("normal") {
                    "normal" => DependencyKind::Normal,
                    "build" => DependencyKind::Build,
                    "dev" => DependencyKind::Development,
                    other => DependencyKind::Other(other.to_owned()),
                };
                CargoDependency {
                    dependency_key: dependency_key.to_owned(),
                    rust_name,
                    package_name: dependency.name().to_owned(),
                    package_path: dependency.path().map(PathBuf::from),
                    kind,
                    target_condition: dependency.target().map(|target| target.to_owned()),
                    optional: dependency.optional(),
                    uses_default_features: dependency.uses_default_features(),
                    features: dependency.features().to_vec(),
                }
            })
            .collect::<Vec<_>>();
        dependencies.sort_by(compare_dependencies);
        Self { dependencies }
    }
}

fn compare_dependencies(left: &CargoDependency, right: &CargoDependency) -> Ordering {
    left.rust_name
        .cmp(&right.rust_name)
        .then_with(|| left.package_name.cmp(&right.package_name))
        .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
        .then_with(|| left.target_condition.cmp(&right.target_condition))
        .then_with(|| left.package_path.cmp(&right.package_path))
        .then_with(|| left.optional.cmp(&right.optional))
}
