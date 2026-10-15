use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fmt,
    path::{Path, PathBuf},
    process::Command,
};

use cop_model::{
    LocalizedMessage, ToolErrorKind, CARGO_ERROR, CARGO_MANIFEST_UNAVAILABLE, IO_ERROR,
};
use serde::Deserialize;
use serde_json::from_slice;

use crate::DependencyIndex;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CargoDiscoveryRequest {
    manifest_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CargoTarget {
    name: String,
    kind: Vec<String>,
    crate_types: Vec<String>,
    edition: String,
    crate_root: PathBuf,
    test: bool,
    required_features: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CargoPackage {
    id: String,
    name: String,
    version: String,
    manifest_path: PathBuf,
    targets: Vec<CargoTarget>,
    dependencies: DependencyIndex,
    features: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CargoDiscovery {
    manifest_path: PathBuf,
    workspace_root: PathBuf,
    workspace_members: Vec<String>,
    packages: Vec<CargoPackage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CargoError {
    kind: ToolErrorKind,
    message: LocalizedMessage,
}

pub struct CargoProjectLoader;

#[derive(Deserialize)]
pub(crate) struct DependencyDto {
    name: String,
    #[serde(default)]
    rename: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    uses_default_features: bool,
    #[serde(default)]
    features: Vec<String>,
}

#[derive(Deserialize)]
struct TargetDto {
    name: String,
    kind: Vec<String>,
    crate_types: Vec<String>,
    edition: String,
    src_path: PathBuf,
    #[serde(default)]
    test: bool,
    #[serde(default, rename = "required-features")]
    required_features: Vec<String>,
}

#[derive(Deserialize)]
struct PackageDto {
    id: String,
    name: String,
    version: String,
    manifest_path: PathBuf,
    targets: Vec<TargetDto>,
    #[serde(default)]
    dependencies: Vec<DependencyDto>,
    #[serde(default)]
    features: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct MetadataDto {
    workspace_root: PathBuf,
    workspace_members: Vec<String>,
    packages: Vec<PackageDto>,
}

impl CargoDiscoveryRequest {
    pub fn new(manifest_path: PathBuf) -> Self {
        Self { manifest_path }
    }
}

impl CargoDiscovery {
    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn packages(&self) -> &[CargoPackage] {
        &self.packages
    }

    pub fn is_workspace_root_manifest(&self) -> bool {
        self.manifest_path == self.workspace_root.join("Cargo.toml")
    }

    pub(crate) fn workspace_members(&self) -> &[String] {
        &self.workspace_members
    }
}

impl CargoPackage {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    pub fn package_root(&self) -> &Path {
        self.manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
    }

    pub fn targets(&self) -> &[CargoTarget] {
        &self.targets
    }

    pub fn dependencies(&self) -> &DependencyIndex {
        &self.dependencies
    }

    pub fn features(&self) -> &BTreeMap<String, Vec<String>> {
        &self.features
    }
}

impl CargoTarget {
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

    pub fn is_test_harness(&self) -> bool {
        self.test
    }

    pub fn required_features(&self) -> &[String] {
        &self.required_features
    }
}

impl CargoError {
    pub(crate) fn new(kind: ToolErrorKind, message: LocalizedMessage) -> Self {
        Self { kind, message }
    }

    pub fn kind(&self) -> ToolErrorKind {
        self.kind
    }

    pub fn message(&self) -> &LocalizedMessage {
        &self.message
    }
}

impl fmt::Display for CargoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message.id().as_str())
    }
}

impl Error for CargoError {}

impl CargoProjectLoader {
    pub fn discover(request: CargoDiscoveryRequest) -> Result<CargoDiscovery, CargoError> {
        let current_directory = env::current_dir().map_err(|error| {
            CargoError::new(
                ToolErrorKind::Io,
                LocalizedMessage::new(IO_ERROR).with_opaque("detail", error.to_string()),
            )
        })?;
        let requested_path = if request.manifest_path.is_absolute() {
            request.manifest_path
        } else {
            current_directory.join(request.manifest_path)
        };
        let manifest_path = requested_path.canonicalize().map_err(|error| {
            CargoError::new(
                ToolErrorKind::Manifest,
                LocalizedMessage::new(CARGO_MANIFEST_UNAVAILABLE)
                    .with_text("path", requested_path.display().to_string())
                    .with_opaque("detail", error.to_string()),
            )
        })?;

        let output = Command::new("cargo")
            .arg("metadata")
            .arg("--format-version")
            .arg("1")
            .arg("--no-deps")
            .arg("--manifest-path")
            .arg(&manifest_path)
            .output()
            .map_err(|error| {
                CargoError::new(
                    ToolErrorKind::CargoMetadata,
                    LocalizedMessage::new(CARGO_ERROR).with_opaque("detail", error.to_string()),
                )
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(CargoError::new(
                ToolErrorKind::CargoMetadata,
                LocalizedMessage::new(CARGO_ERROR).with_opaque("detail", stderr),
            ));
        }

        let metadata = from_slice::<MetadataDto>(&output.stdout).map_err(|error| {
            CargoError::new(
                ToolErrorKind::CargoMetadata,
                LocalizedMessage::new(CARGO_ERROR).with_opaque("detail", error.to_string()),
            )
        })?;
        let mut packages = metadata
            .packages
            .into_iter()
            .map(CargoPackage::from)
            .collect::<Vec<_>>();
        packages.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.version.cmp(&right.version))
                .then_with(|| left.id.cmp(&right.id))
        });
        let mut workspace_members = metadata.workspace_members;
        workspace_members.sort();

        Ok(CargoDiscovery {
            manifest_path,
            workspace_root: metadata.workspace_root,
            workspace_members,
            packages,
        })
    }
}

impl From<PackageDto> for CargoPackage {
    fn from(package: PackageDto) -> Self {
        let mut targets = package
            .targets
            .into_iter()
            .map(CargoTarget::from)
            .collect::<Vec<_>>();
        targets.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.crate_root.cmp(&right.crate_root))
        });
        Self {
            id: package.id,
            name: package.name,
            version: package.version,
            manifest_path: package.manifest_path,
            targets,
            dependencies: DependencyIndex::from_metadata(package.dependencies),
            features: package.features,
        }
    }
}

impl From<TargetDto> for CargoTarget {
    fn from(target: TargetDto) -> Self {
        Self {
            name: target.name,
            kind: target.kind,
            crate_types: target.crate_types,
            edition: target.edition,
            crate_root: target.src_path,
            test: target.test,
            required_features: target.required_features,
        }
    }
}

impl DependencyDto {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn rename(&self) -> Option<&str> {
        self.rename.as_deref()
    }

    pub(crate) fn kind(&self) -> Option<&str> {
        self.kind.as_deref()
    }

    pub(crate) fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(crate) fn optional(&self) -> bool {
        self.optional
    }

    pub(crate) fn uses_default_features(&self) -> bool {
        self.uses_default_features
    }

    pub(crate) fn features(&self) -> &[String] {
        &self.features
    }
}
