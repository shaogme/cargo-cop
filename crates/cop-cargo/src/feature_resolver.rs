use cop_i18n::t;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use cop_model::ToolErrorKind;

use crate::{CargoError, CargoFailure, CargoPackage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeatureSelection {
    requested: Vec<String>,
    all_features: bool,
    default_features: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolvedFeatures {
    enabled: BTreeSet<String>,
    activated_dependencies: BTreeSet<String>,
    dependency_features: BTreeMap<String, BTreeSet<String>>,
    feature_dependency_features: BTreeMap<String, BTreeSet<String>>,
}

struct FeatureExpansion<'a> {
    dependency_names: &'a BTreeSet<String>,
    definitions: &'a BTreeMap<String, Vec<String>>,
    pending: &'a mut VecDeque<String>,
    weak_features: &'a mut Vec<(String, String)>,
    resolved: &'a mut ResolvedFeatures,
}

pub(crate) struct FeatureResolver;

impl FeatureSelection {
    pub fn new(requested: Vec<String>, all_features: bool, default_features: bool) -> Self {
        Self {
            requested,
            all_features,
            default_features,
        }
    }

    pub fn requested(&self) -> &[String] {
        &self.requested
    }

    pub fn all_features(&self) -> bool {
        self.all_features
    }

    pub fn default_features(&self) -> bool {
        self.default_features
    }
}

impl ResolvedFeatures {
    pub fn enabled(&self) -> &BTreeSet<String> {
        &self.enabled
    }

    pub fn activated_dependencies(&self) -> &BTreeSet<String> {
        &self.activated_dependencies
    }

    pub fn dependency_features(&self) -> &BTreeMap<String, BTreeSet<String>> {
        &self.dependency_features
    }

    pub fn feature_dependency_features(&self) -> &BTreeMap<String, BTreeSet<String>> {
        &self.feature_dependency_features
    }

    pub fn contains(&self, feature: &str) -> bool {
        self.enabled.contains(feature)
    }
}

impl FeatureResolver {
    pub(crate) fn resolve(
        package: &CargoPackage,
        selection: &FeatureSelection,
    ) -> Result<ResolvedFeatures, CargoFailure> {
        let definitions = package.features();
        let mut resolved = ResolvedFeatures::default();
        let mut pending = VecDeque::new();

        if selection.default_features && definitions.contains_key("default") {
            pending.push_back("default".to_owned());
        }
        if selection.all_features {
            pending.extend(definitions.keys().cloned());
        }
        for requested in &selection.requested {
            let feature = if let Some((package_name, feature)) = requested.split_once('/') {
                if package_name != package.name() {
                    continue;
                }
                feature
            } else if definitions.contains_key(requested) {
                requested.as_str()
            } else {
                continue;
            };
            if !definitions.contains_key(feature) {
                return Err(CargoError::from_localized(
                    ToolErrorKind::Arguments,
                    t!("cargo.feature_not_found", { package: package.name(), feature: feature })?,
                ));
            }
            pending.push_back(feature.to_owned());
        }

        let dependency_names = package
            .dependencies()
            .dependencies()
            .iter()
            .map(|dependency| dependency.dependency_key().to_owned())
            .collect::<BTreeSet<_>>();
        for dependency in package.dependencies().dependencies() {
            if !dependency.is_optional() {
                resolved
                    .activated_dependencies
                    .insert(dependency.dependency_key().to_owned());
            }
        }
        let mut weak_features = Vec::new();
        while let Some(feature) = pending.pop_front() {
            let Some(references) = definitions.get(&feature) else {
                continue;
            };
            if !resolved.enabled.insert(feature.clone()) {
                continue;
            }
            for reference in references {
                let mut expansion = FeatureExpansion {
                    dependency_names: &dependency_names,
                    definitions,
                    pending: &mut pending,
                    weak_features: &mut weak_features,
                    resolved: &mut resolved,
                };
                expansion.apply_reference(reference);
            }
        }

        for (dependency, feature) in weak_features {
            if resolved.activated_dependencies.contains(&dependency) {
                resolved
                    .dependency_features
                    .entry(dependency.clone())
                    .or_default()
                    .insert(feature.clone());
                resolved
                    .feature_dependency_features
                    .entry(dependency)
                    .or_default()
                    .insert(feature);
            }
        }
        Self::add_declared_dependency_features(package, &mut resolved);
        Ok(resolved)
    }
}

impl FeatureExpansion<'_> {
    fn apply_reference(&mut self, reference: &str) {
        if let Some(dependency) = reference.strip_prefix("dep:") {
            if self.dependency_names.contains(dependency) {
                self.resolved
                    .activated_dependencies
                    .insert(dependency.to_owned());
            }
            return;
        }
        if let Some((dependency, feature)) = reference.split_once('/') {
            let weak = dependency.ends_with('?');
            let dependency = dependency.strip_suffix('?').unwrap_or(dependency);
            if !self.dependency_names.contains(dependency) {
                return;
            }
            if weak {
                self.weak_features
                    .push((dependency.to_owned(), feature.to_owned()));
            } else {
                self.resolved
                    .activated_dependencies
                    .insert(dependency.to_owned());
                self.resolved
                    .dependency_features
                    .entry(dependency.to_owned())
                    .or_default()
                    .insert(feature.to_owned());
                self.resolved
                    .feature_dependency_features
                    .entry(dependency.to_owned())
                    .or_default()
                    .insert(feature.to_owned());
            }
            return;
        }
        if self.definitions.contains_key(reference) {
            self.pending.push_back(reference.to_owned());
        } else if self.dependency_names.contains(reference) {
            self.resolved
                .activated_dependencies
                .insert(reference.to_owned());
        }
    }
}

impl FeatureResolver {
    fn add_declared_dependency_features(package: &CargoPackage, resolved: &mut ResolvedFeatures) {
        for dependency in package.dependencies().dependencies() {
            let name = dependency.dependency_key();
            let active =
                !dependency.is_optional() || resolved.activated_dependencies.contains(name);
            if !active {
                continue;
            }
            resolved.activated_dependencies.insert(name.to_owned());
            let features = resolved
                .dependency_features
                .entry(name.to_owned())
                .or_default();
            if dependency.uses_default_features() {
                features.insert("default".to_owned());
            }
            features.extend(dependency.features().iter().cloned());
        }
    }
}
