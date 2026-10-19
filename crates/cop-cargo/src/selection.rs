use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use cop_cfg::{CfgContext, CfgDecision, CfgEvaluator, CfgExpression, CfgValue};
use cop_model::{
    LocalizedMessage, ToolErrorKind, CARGO_CONFIG_TARGET_TYPE, CARGO_ERROR,
    CARGO_FEATURE_CONTEXT_MISSING, CARGO_FEATURE_NOT_DECLARED, CARGO_FEATURE_NOT_FOUND,
    CARGO_FEATURE_PACKAGE_NOT_SELECTED, CARGO_HOST_TRIPLE_MISSING, CARGO_NO_SCAN_PACKAGES,
    CARGO_OPERATION_ERROR, CARGO_PACKAGE_AMBIGUOUS, CARGO_PACKAGE_DUPLICATE,
    CARGO_PACKAGE_NOT_FOUND, CARGO_TARGET_CFG_INVALID, CARGO_TARGET_CFG_MISSING,
    CARGO_TARGET_CFG_QUERY_ERROR, CARGO_WORKSPACE_CONFLICT, IO_ERROR,
};
use toml::{from_str, Value};

use crate::{
    feature_resolver::FeatureResolver, CargoDiscovery, CargoError, CargoPackage, CargoTarget,
    DependencyIndex, DependencyKind, FeatureSelection, ResolvedFeatures,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CargoSelection {
    workspace: bool,
    packages: Vec<String>,
    all_targets: bool,
    target: Option<String>,
    features: Option<FeatureSelection>,
    current_directory: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetContext {
    name: String,
    kind: Vec<String>,
    crate_types: Vec<String>,
    edition: String,
    crate_root: PathBuf,
    target_triple: String,
    features: ResolvedFeatures,
    cfg_values: Vec<CfgValue>,
    cfg_test: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedPackage {
    id: String,
    name: String,
    manifest_path: PathBuf,
    package_root: PathBuf,
    dependencies: DependencyIndex,
    targets: Vec<TargetContext>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CargoProject {
    workspace_root: PathBuf,
    packages: Vec<SelectedPackage>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct PackageFeatureState {
    requested_features: BTreeSet<String>,
    default_features: bool,
    all_features: bool,
    resolved: ResolvedFeatures,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FeatureContextKey {
    target_triple: String,
    cfg_test: bool,
    build_script: bool,
}

struct FeatureContext<'a> {
    target_triple: &'a str,
    cfg_values: &'a [CfgValue],
    cfg_test: bool,
    build_script: bool,
    root_packages: &'a BTreeSet<usize>,
}

pub struct TargetSelector;

impl CargoSelection {
    pub fn new(features: FeatureSelection) -> Self {
        Self {
            features: Some(features),
            ..Self::default()
        }
    }

    pub fn with_workspace(mut self, workspace: bool) -> Self {
        self.workspace = workspace;
        self
    }

    pub fn with_packages(mut self, packages: Vec<String>) -> Self {
        self.packages = packages;
        self
    }

    pub fn with_all_targets(mut self, all_targets: bool) -> Self {
        self.all_targets = all_targets;
        self
    }

    pub fn with_target(mut self, target: Option<String>) -> Self {
        self.target = target;
        self
    }

    pub fn with_current_directory(mut self, current_directory: PathBuf) -> Self {
        self.current_directory = Some(current_directory);
        self
    }
}

impl CargoProject {
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn packages(&self) -> &[SelectedPackage] {
        &self.packages
    }
}

impl SelectedPackage {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    pub fn package_root(&self) -> &Path {
        &self.package_root
    }

    pub fn dependencies(&self) -> &DependencyIndex {
        &self.dependencies
    }

    pub fn targets(&self) -> &[TargetContext] {
        &self.targets
    }
}

impl TargetContext {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> &[String] {
        &self.kind
    }

    pub fn crate_types(&self) -> &[String] {
        &self.crate_types
    }

    pub fn edition(&self) -> &str {
        &self.edition
    }

    pub fn crate_root(&self) -> &Path {
        &self.crate_root
    }

    pub fn target_triple(&self) -> &str {
        &self.target_triple
    }

    pub fn features(&self) -> &ResolvedFeatures {
        &self.features
    }

    pub fn cfg_values(&self) -> &[CfgValue] {
        &self.cfg_values
    }

    pub fn cfg_test(&self) -> bool {
        self.cfg_test
    }
}

impl TargetSelector {
    pub fn select(
        discovery: CargoDiscovery,
        selection: CargoSelection,
    ) -> Result<CargoProject, CargoError> {
        if selection.workspace && !selection.packages.is_empty() {
            return Err(CargoError::new(
                ToolErrorKind::Arguments,
                LocalizedMessage::new(CARGO_WORKSPACE_CONFLICT),
            ));
        }
        validate_unique_packages(&selection.packages)?;
        let selected_packages = Self::select_packages(&discovery, &selection)?;
        let feature_selection = selection
            .features
            .unwrap_or_else(|| FeatureSelection::new(Vec::new(), false, true));
        validate_feature_requests(&selected_packages, &feature_selection)?;
        let current_directory = match selection.current_directory {
            Some(directory) => directory,
            None => env::current_dir().map_err(|error| {
                CargoError::new(
                    ToolErrorKind::Io,
                    LocalizedMessage::new(IO_ERROR).with_opaque("detail", error.to_string()),
                )
            })?,
        };
        let target_triple = resolve_target(selection.target.as_deref(), &current_directory)?;
        let selected_targets = selected_packages
            .iter()
            .enumerate()
            .flat_map(|(package_index, package)| {
                package
                    .targets()
                    .iter()
                    .filter(|target| target_is_selected(target, selection.all_targets))
                    .map(move |target| (package_index, target))
            })
            .collect::<Vec<_>>();
        let host_triple = if selected_targets
            .iter()
            .any(|(_, target)| target_uses_host(target))
        {
            Some(rustc_host_triple()?)
        } else {
            None
        };
        let mut cfg_by_target = BTreeMap::<String, Vec<CfgValue>>::new();
        let mut context_roots = BTreeMap::<FeatureContextKey, BTreeSet<usize>>::new();
        for (package_index, target) in &selected_targets {
            let effective_triple = if target_uses_host(target) {
                host_triple.as_deref().unwrap_or(&target_triple)
            } else {
                &target_triple
            };
            if !cfg_by_target.contains_key(effective_triple) {
                let cfg_values = query_cfg(effective_triple)?;
                cfg_by_target.insert(effective_triple.to_owned(), cfg_values);
            }
            for cfg_test in target_test_modes(target, selection.all_targets) {
                let key = FeatureContextKey {
                    target_triple: effective_triple.to_owned(),
                    cfg_test,
                    build_script: target.kind().iter().any(|kind| kind == "custom-build"),
                };
                context_roots.entry(key).or_default().insert(*package_index);
            }
        }
        let mut feature_contexts = BTreeMap::new();
        for (key, root_packages) in context_roots {
            let cfg_values = cfg_by_target.get(&key.target_triple).ok_or_else(|| {
                CargoError::new(
                    ToolErrorKind::CargoMetadata,
                    LocalizedMessage::new(CARGO_TARGET_CFG_MISSING)
                        .with_text("target", &key.target_triple),
                )
            })?;
            let context = FeatureContext {
                target_triple: &key.target_triple,
                cfg_values,
                cfg_test: key.cfg_test,
                build_script: key.build_script,
                root_packages: &root_packages,
            };
            let resolved_features =
                resolve_workspace_features(&selected_packages, &feature_selection, context)?;
            feature_contexts.insert(key, resolved_features);
        }
        let mut packages = Vec::new();
        for (package_index, package) in selected_packages.into_iter().enumerate() {
            let mut targets = Vec::new();
            for target in package
                .targets()
                .iter()
                .filter(|target| target_is_selected(target, selection.all_targets))
            {
                let effective_triple = if target_uses_host(target) {
                    host_triple.as_deref().unwrap_or(&target_triple)
                } else {
                    &target_triple
                };
                let build_script = target.kind().iter().any(|kind| kind == "custom-build");
                let cfg_values = cfg_by_target.get(effective_triple).ok_or_else(|| {
                    CargoError::new(
                        ToolErrorKind::CargoMetadata,
                        LocalizedMessage::new(CARGO_TARGET_CFG_MISSING)
                            .with_text("target", effective_triple),
                    )
                })?;
                for cfg_test in target_test_modes(target, selection.all_targets) {
                    let key = FeatureContextKey {
                        target_triple: effective_triple.to_owned(),
                        cfg_test,
                        build_script,
                    };
                    let features = feature_contexts
                        .get(&key)
                        .and_then(|features| features.get(package_index))
                        .ok_or_else(|| {
                            CargoError::new(
                                ToolErrorKind::CargoMetadata,
                                LocalizedMessage::new(CARGO_FEATURE_CONTEXT_MISSING)
                                    .with_text("package", package.name()),
                            )
                        })?;
                    if target_required_features_are_enabled(target, features) {
                        targets.push(make_target_context(
                            target,
                            effective_triple,
                            features,
                            cfg_values,
                            cfg_test,
                        ));
                    }
                }
            }
            packages.push(SelectedPackage::from_package(package, targets));
        }
        Ok(CargoProject {
            workspace_root: discovery.workspace_root().to_path_buf(),
            packages,
        })
    }

    fn select_packages(
        discovery: &CargoDiscovery,
        selection: &CargoSelection,
    ) -> Result<Vec<CargoPackage>, CargoError> {
        let members = discovery
            .workspace_members()
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let candidates = discovery
            .packages()
            .iter()
            .filter(|package| members.contains(package.id()))
            .collect::<Vec<_>>();
        let selected = if !selection.packages.is_empty() {
            select_named_packages(&candidates, &selection.packages)?
        } else if selection.workspace || discovery.is_workspace_root_manifest() {
            candidates
        } else {
            candidates
                .into_iter()
                .filter(|package| package.manifest_path() == discovery.manifest_path())
                .collect()
        };
        if selected.is_empty() {
            return Err(CargoError::new(
                ToolErrorKind::Arguments,
                LocalizedMessage::new(CARGO_NO_SCAN_PACKAGES),
            ));
        }
        Ok(selected.into_iter().cloned().collect())
    }
}

fn resolve_workspace_features(
    packages: &[CargoPackage],
    selection: &FeatureSelection,
    context: FeatureContext<'_>,
) -> Result<Vec<ResolvedFeatures>, CargoError> {
    let mut states = packages
        .iter()
        .enumerate()
        .map(|(package_index, package)| {
            let is_root_package = context.root_packages.contains(&package_index);
            let mut requested_features = BTreeSet::new();
            if is_root_package {
                for requested in selection.requested() {
                    let feature = if let Some((package_name, feature)) = requested.split_once('/') {
                        (package_name == package.name()).then_some(feature)
                    } else if package.features().contains_key(requested) {
                        Some(requested.as_str())
                    } else {
                        None
                    };
                    if let Some(feature) = feature {
                        requested_features.insert(feature.to_owned());
                    }
                }
            }
            PackageFeatureState {
                requested_features,
                default_features: is_root_package && selection.default_features(),
                all_features: is_root_package && selection.all_features(),
                resolved: ResolvedFeatures::default(),
            }
        })
        .collect::<Vec<_>>();

    loop {
        let mut changed = false;
        for (package, state) in packages.iter().zip(states.iter_mut()) {
            let features = FeatureSelection::new(
                state.requested_features.iter().cloned().collect(),
                state.all_features,
                state.default_features,
            );
            let resolved = FeatureResolver::resolve(package, &features)?;
            if state.resolved != resolved {
                state.resolved = resolved;
                changed = true;
            }
        }

        for (parent_index, package) in packages.iter().enumerate() {
            let dependencies = package.dependencies().dependencies();
            for dependency in dependencies {
                let dependency_kind_is_active = match dependency.kind() {
                    DependencyKind::Normal => !context.build_script,
                    DependencyKind::Development => context.cfg_test,
                    DependencyKind::Build => context.build_script,
                    DependencyKind::Other(_) => false,
                };
                if !dependency_kind_is_active
                    || !states[parent_index]
                        .resolved
                        .activated_dependencies()
                        .contains(dependency.dependency_key())
                {
                    continue;
                }
                if let Some(condition) = dependency.target_condition() {
                    if !target_condition_matches(
                        condition,
                        context.target_triple,
                        context.cfg_values,
                        states[parent_index].resolved.enabled(),
                    )? {
                        continue;
                    }
                }
                let Some(package_path) = dependency.package_path() else {
                    continue;
                };
                let Some(child_index) = packages.iter().position(|candidate| {
                    candidate.name() == dependency.package_name()
                        && candidate.package_root() == package_path
                }) else {
                    continue;
                };
                let requested = states[parent_index]
                    .resolved
                    .feature_dependency_features()
                    .get(dependency.dependency_key())
                    .cloned()
                    .unwrap_or_default();
                let child_state = &mut states[child_index];
                if dependency.uses_default_features() && !child_state.default_features {
                    child_state.default_features = true;
                    changed = true;
                }
                for feature in requested
                    .into_iter()
                    .chain(dependency.features().iter().cloned())
                {
                    if feature == "default" {
                        if !child_state.default_features {
                            child_state.default_features = true;
                            changed = true;
                        }
                    } else if child_state.requested_features.insert(feature) {
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            return Ok(states.into_iter().map(|state| state.resolved).collect());
        }
    }
}

impl SelectedPackage {
    fn from_package(package: CargoPackage, targets: Vec<TargetContext>) -> Self {
        let package_root = package.package_root().to_path_buf();
        Self {
            id: package.id().to_owned(),
            name: package.name().to_owned(),
            manifest_path: package.manifest_path().to_path_buf(),
            package_root,
            dependencies: package.dependencies().clone(),
            targets,
        }
    }
}

fn select_named_packages<'a>(
    candidates: &[&'a CargoPackage],
    names: &[String],
) -> Result<Vec<&'a CargoPackage>, CargoError> {
    let mut selected = Vec::new();
    for name in names {
        let matches = candidates
            .iter()
            .copied()
            .filter(|package| package.name() == name)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [package] => selected.push(*package),
            [] => {
                return Err(CargoError::new(
                    ToolErrorKind::Arguments,
                    LocalizedMessage::new(CARGO_PACKAGE_NOT_FOUND).with_text("name", name),
                ));
            }
            _ => {
                return Err(CargoError::new(
                    ToolErrorKind::Arguments,
                    LocalizedMessage::new(CARGO_PACKAGE_AMBIGUOUS).with_text("name", name),
                ));
            }
        }
    }
    selected.sort_by(|left, right| {
        left.name()
            .cmp(right.name())
            .then(left.id().cmp(right.id()))
    });
    Ok(selected)
}

fn validate_unique_packages(packages: &[String]) -> Result<(), CargoError> {
    let mut seen = BTreeSet::new();
    if let Some(duplicate) = packages
        .iter()
        .find(|package| !seen.insert(package.as_str()))
    {
        return Err(CargoError::new(
            ToolErrorKind::Arguments,
            LocalizedMessage::new(CARGO_PACKAGE_DUPLICATE).with_text("name", duplicate),
        ));
    }
    Ok(())
}

fn validate_feature_requests(
    packages: &[CargoPackage],
    selection: &FeatureSelection,
) -> Result<(), CargoError> {
    for requested in selection.requested() {
        if let Some((package_name, feature)) = requested.split_once('/') {
            let package = packages
                .iter()
                .find(|package| package.name() == package_name)
                .ok_or_else(|| {
                    CargoError::new(
                        ToolErrorKind::Arguments,
                        LocalizedMessage::new(CARGO_FEATURE_PACKAGE_NOT_SELECTED)
                            .with_text("package", package_name),
                    )
                })?;
            if !package.features().contains_key(feature) {
                return Err(CargoError::new(
                    ToolErrorKind::Arguments,
                    LocalizedMessage::new(CARGO_FEATURE_NOT_FOUND)
                        .with_text("package", package_name)
                        .with_text("feature", feature),
                ));
            }
        } else if !packages
            .iter()
            .any(|package| package.features().contains_key(requested))
        {
            return Err(CargoError::new(
                ToolErrorKind::Arguments,
                LocalizedMessage::new(CARGO_FEATURE_NOT_DECLARED).with_text("feature", requested),
            ));
        }
    }
    Ok(())
}

fn target_is_selected(target: &CargoTarget, all_targets: bool) -> bool {
    all_targets
        || target
            .kind()
            .iter()
            .any(|kind| matches!(kind.as_str(), "lib" | "bin" | "proc-macro"))
}

fn target_uses_host(target: &CargoTarget) -> bool {
    target
        .kind()
        .iter()
        .any(|kind| matches!(kind.as_str(), "proc-macro" | "custom-build"))
}

fn target_required_features_are_enabled(target: &CargoTarget, features: &ResolvedFeatures) -> bool {
    target
        .required_features()
        .iter()
        .all(|required| features.contains(required))
}

fn target_test_modes(target: &CargoTarget, all_targets: bool) -> Vec<bool> {
    let test_target = target
        .kind()
        .iter()
        .any(|kind| matches!(kind.as_str(), "test" | "bench"));
    if test_target {
        vec![true]
    } else if all_targets && target.is_test_harness() {
        vec![false, true]
    } else {
        vec![false]
    }
}

fn make_target_context(
    target: &CargoTarget,
    target_triple: &str,
    features: &ResolvedFeatures,
    cfg_values: &[CfgValue],
    cfg_test: bool,
) -> TargetContext {
    TargetContext {
        name: target.name().to_owned(),
        kind: target.kind().to_vec(),
        crate_types: target.crate_types().to_vec(),
        edition: target.edition().to_owned(),
        crate_root: target.crate_root().to_path_buf(),
        target_triple: target_triple.to_owned(),
        features: features.clone(),
        cfg_values: cfg_values.to_vec(),
        cfg_test,
    }
}

fn target_condition_matches(
    condition: &str,
    target_triple: &str,
    values: &[CfgValue],
    features: &BTreeSet<String>,
) -> Result<bool, CargoError> {
    let condition = condition.trim();
    if !condition.starts_with("cfg(") {
        return Ok(condition == target_triple);
    }
    let expression = CfgExpression::parse(condition).map_err(|_| {
        CargoError::new(
            ToolErrorKind::CargoMetadata,
            LocalizedMessage::new(CARGO_TARGET_CFG_INVALID).with_text("condition", condition),
        )
    })?;
    let context = CfgContext::new(values, Some(features));
    let decision = CfgEvaluator::new(&context).evaluate(&expression);
    Ok(decision == CfgDecision::True)
}

fn resolve_target(
    configured_target: Option<&str>,
    current_directory: &Path,
) -> Result<String, CargoError> {
    if let Some(configured_target) = configured_target {
        return if configured_target == "host" {
            rustc_host_triple()
        } else {
            Ok(configured_target.to_owned())
        };
    }
    if let Ok(target) = env::var("CARGO_BUILD_TARGET") {
        return Ok(target);
    }
    if let Some(target) = cargo_config_target(current_directory)? {
        return Ok(target);
    }
    rustc_host_triple()
}

fn cargo_config_target(current_directory: &Path) -> Result<Option<String>, CargoError> {
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")));
    let mut target = match cargo_home {
        Some(cargo_home) => read_target_setting(&cargo_home, false)?,
        None => None,
    };
    let mut directories = current_directory
        .ancestors()
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    directories.reverse();
    for directory in directories {
        if let Some(configured) = read_target_setting(&directory, true)? {
            target = Some(configured);
        }
    }
    Ok(target)
}

fn read_target_setting(
    directory: &Path,
    nested_cargo_directory: bool,
) -> Result<Option<String>, CargoError> {
    let config_directory = if nested_cargo_directory {
        directory.join(".cargo")
    } else {
        directory.to_path_buf()
    };
    let toml_path = config_directory.join("config.toml");
    let legacy_path = config_directory.join("config");
    let path = if toml_path.is_file() {
        toml_path
    } else if legacy_path.is_file() {
        legacy_path
    } else {
        return Ok(None);
    };
    let contents = fs::read_to_string(&path).map_err(|error| {
        CargoError::new(
            ToolErrorKind::Io,
            LocalizedMessage::new(CARGO_OPERATION_ERROR)
                .with_text("path", path.display().to_string())
                .with_opaque("detail", error.to_string()),
        )
    })?;
    let value = from_str::<Value>(&contents).map_err(|error| {
        CargoError::new(
            ToolErrorKind::CargoMetadata,
            LocalizedMessage::new(CARGO_OPERATION_ERROR)
                .with_text("path", path.display().to_string())
                .with_opaque("detail", error.to_string()),
        )
    })?;
    let configured = value
        .get("build")
        .and_then(Value::as_table)
        .and_then(|build| build.get("target"));
    configured
        .map(|configured| {
            configured
                .as_str()
                .map(|target| target.to_owned())
                .ok_or_else(|| {
                    CargoError::new(
                        ToolErrorKind::CargoMetadata,
                        LocalizedMessage::new(CARGO_CONFIG_TARGET_TYPE)
                            .with_text("path", path.display().to_string()),
                    )
                })
        })
        .transpose()
}

fn rustc_host_triple() -> Result<String, CargoError> {
    let output = rustc_command().arg("-vV").output().map_err(|error| {
        CargoError::new(
            ToolErrorKind::CargoMetadata,
            LocalizedMessage::new(CARGO_ERROR).with_opaque("detail", error.to_string()),
        )
    })?;
    if !output.status.success() {
        return Err(CargoError::new(
            ToolErrorKind::CargoMetadata,
            LocalizedMessage::new(CARGO_ERROR).with_opaque(
                "detail",
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ),
        ));
    }
    let version = String::from_utf8_lossy(&output.stdout);
    version
        .lines()
        .find_map(|line| {
            line.strip_prefix("host: ")
                .map(|host_triple| host_triple.to_owned())
        })
        .ok_or_else(|| {
            CargoError::new(
                ToolErrorKind::CargoMetadata,
                LocalizedMessage::new(CARGO_HOST_TRIPLE_MISSING),
            )
        })
}

fn query_cfg(target: &str) -> Result<Vec<CfgValue>, CargoError> {
    let output = rustc_command()
        .arg("--print")
        .arg("cfg")
        .arg("--target")
        .arg(target)
        .output()
        .map_err(|error| {
            CargoError::new(
                ToolErrorKind::CargoMetadata,
                LocalizedMessage::new(CARGO_TARGET_CFG_QUERY_ERROR)
                    .with_text("target", target)
                    .with_opaque("detail", error.to_string()),
            )
        })?;
    if !output.status.success() {
        return Err(CargoError::new(
            ToolErrorKind::CargoMetadata,
            LocalizedMessage::new(CARGO_TARGET_CFG_QUERY_ERROR)
                .with_text("target", target)
                .with_opaque("detail", String::from_utf8_lossy(&output.stderr).trim()),
        ));
    }
    let mut cfg_values = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_cfg_value)
        .collect::<Vec<_>>();
    cfg_values.sort();
    Ok(cfg_values)
}

fn parse_cfg_value(line: &str) -> Option<CfgValue> {
    if let Some((name, value)) = line.split_once('=') {
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(value);
        Some(CfgValue::new(name, Some(value.to_owned())))
    } else if !line.is_empty() {
        Some(CfgValue::new(line, None))
    } else {
        None
    }
}

fn rustc_command() -> Command {
    match env::var_os("RUSTC") {
        Some(rustc) => Command::new(rustc),
        None => Command::new("rustc"),
    }
}
