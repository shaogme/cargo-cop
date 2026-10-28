use std::{
    collections::BTreeMap,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

use cop_cfg::ScopedCfgValue;
use cop_model::{LocalizedMessage, RuleId, Severity, CONFIG_FILE_MISSING, CONFIG_FILE_READ_ERROR};

use crate::schema::{ConfigDocument, ConfigDocumentParts, ScanDefaults};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigRequest {
    explicit_path: Option<PathBuf>,
    discovery_directory: PathBuf,
    current_directory: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedConfig {
    source_path: Option<PathBuf>,
    allow_warnings: bool,
    scan: ScanDefaults,
    rules: BTreeMap<RuleId, (bool, Severity)>,
    locale: Option<String>,
    cfg_overrides: Vec<ScopedCfgValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigError {
    path: PathBuf,
    message: LocalizedMessage,
}

pub struct ConfigLoader;

impl ConfigRequest {
    pub fn new(
        explicit_path: Option<PathBuf>,
        discovery_directory: PathBuf,
        current_directory: PathBuf,
    ) -> Self {
        Self {
            explicit_path,
            discovery_directory,
            current_directory,
        }
    }
}

impl LoadedConfig {
    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    pub fn scan_defaults(&self) -> &ScanDefaults {
        &self.scan
    }

    pub(super) fn allow_warnings(&self) -> bool {
        self.allow_warnings
    }

    pub(super) fn rules(&self) -> &BTreeMap<RuleId, (bool, Severity)> {
        &self.rules
    }

    pub fn locale(&self) -> Option<&str> {
        self.locale.as_deref()
    }

    pub fn cfg_overrides(&self) -> &[ScopedCfgValue] {
        &self.cfg_overrides
    }
}

impl ConfigError {
    pub(super) fn new(path: PathBuf, message: LocalizedMessage) -> Self {
        Self { path, message }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn message(&self) -> &LocalizedMessage {
        &self.message
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {}",
            self.path.display(),
            self.message.id().as_str()
        )
    }
}

impl Error for ConfigError {}

impl ConfigLoader {
    pub fn read_locale(request: ConfigRequest) -> Result<Option<String>, ConfigError> {
        let explicit_path = request.explicit_path.map(|path| {
            if path.is_absolute() {
                path
            } else {
                request.current_directory.join(path)
            }
        });
        let path = explicit_path
            .clone()
            .unwrap_or_else(|| request.discovery_directory.join("cargo-cop.toml"));
        if !path.is_file() {
            if explicit_path.is_some() {
                return Err(ConfigError::new(
                    path.clone(),
                    LocalizedMessage::new(CONFIG_FILE_MISSING)
                        .with_text("path", path.display().to_string()),
                ));
            }
            return Ok(None);
        }
        let contents = fs::read_to_string(&path).map_err(|error| {
            ConfigError::new(
                path.clone(),
                LocalizedMessage::new(CONFIG_FILE_READ_ERROR)
                    .with_text("path", path.display().to_string())
                    .with_opaque("detail", error.to_string()),
            )
        })?;
        ConfigDocument::read_locale(&path, &contents)
    }

    pub fn load(request: ConfigRequest) -> Result<LoadedConfig, ConfigError> {
        let explicit_path = request.explicit_path.map(|path| {
            if path.is_absolute() {
                path
            } else {
                request.current_directory.join(path)
            }
        });
        let path = explicit_path
            .clone()
            .unwrap_or_else(|| request.discovery_directory.join("cargo-cop.toml"));
        let exists = path.is_file();
        if explicit_path.is_some() && !exists {
            return Err(ConfigError::new(
                path.clone(),
                LocalizedMessage::new(CONFIG_FILE_MISSING)
                    .with_text("path", path.display().to_string()),
            ));
        }
        if !exists {
            let ConfigDocumentParts {
                allow_warnings,
                scan,
                rules,
                locale,
                cfg_overrides,
            } = ConfigDocument::defaults().into_parts();
            return Ok(LoadedConfig {
                source_path: None,
                allow_warnings,
                scan,
                rules,
                locale,
                cfg_overrides,
            });
        }

        let contents = fs::read_to_string(&path).map_err(|error| {
            ConfigError::new(
                path.clone(),
                LocalizedMessage::new(CONFIG_FILE_READ_ERROR)
                    .with_text("path", path.display().to_string())
                    .with_opaque("detail", error.to_string()),
            )
        })?;
        let document = ConfigDocument::parse(&path, &contents)?;
        let ConfigDocumentParts {
            allow_warnings,
            scan,
            rules,
            locale,
            cfg_overrides,
        } = document.into_parts();
        Ok(LoadedConfig {
            source_path: Some(path),
            allow_warnings,
            scan,
            rules,
            locale,
            cfg_overrides,
        })
    }
}
