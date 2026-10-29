use std::{
    collections::BTreeMap,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

use cop_cfg::ScopedCfgValue;
use cop_i18n::{t, RenderError};
use cop_model::{RuleId, Severity};

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
    message: String,
    detail: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigFailure {
    Config(ConfigError),
    Render(RenderError),
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
    pub(super) fn from_localized(path: PathBuf, message: String) -> ConfigFailure {
        ConfigFailure::Config(Self {
            path,
            message,
            detail: None,
        })
    }

    pub(super) fn from_message(
        path: PathBuf,
        message: Result<String, RenderError>,
    ) -> ConfigFailure {
        match message {
            Ok(message) => Self::from_localized(path, message),
            Err(error) => ConfigFailure::Render(error),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path.display(), self.message)?;
        if let Some(detail) = &self.detail {
            write!(formatter, ": {detail}")?;
        }
        Ok(())
    }
}

impl Error for ConfigError {}

impl fmt::Display for ConfigFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => error.fmt(formatter),
            Self::Render(error) => error.fmt(formatter),
        }
    }
}

impl Error for ConfigFailure {}

impl ConfigFailure {
    pub(super) fn with_detail(self, detail: impl Into<String>) -> Self {
        match self {
            Self::Config(mut error) => {
                error.detail = Some(detail.into());
                Self::Config(error)
            }
            Self::Render(error) => Self::Render(error),
        }
    }
}

impl From<RenderError> for ConfigFailure {
    fn from(error: RenderError) -> Self {
        Self::Render(error)
    }
}

impl ConfigLoader {
    pub fn read_locale(request: ConfigRequest) -> Result<Option<String>, ConfigFailure> {
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
        let path_display = path.display().to_string();
        if !path.is_file() {
            if explicit_path.is_some() {
                return Err(ConfigError::from_localized(
                    path.clone(),
                    t!("config.file_missing", { path: path_display.as_str() })?,
                ));
            }
            return Ok(None);
        }
        let contents = fs::read_to_string(&path).map_err(|error| {
            ConfigError::from_message(
                path.clone(),
                t!("config.file_read_error", { path: path_display.as_str() }),
            )
            .with_detail(error.to_string())
        })?;
        ConfigDocument::read_locale(&path, &contents)
    }

    pub fn load(request: ConfigRequest) -> Result<LoadedConfig, ConfigFailure> {
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
        let path_display = path.display().to_string();
        let exists = path.is_file();
        if explicit_path.is_some() && !exists {
            return Err(ConfigError::from_localized(
                path.clone(),
                t!("config.file_missing", { path: path_display.as_str() })?,
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
            ConfigError::from_message(
                path.clone(),
                t!("config.file_read_error", { path: path_display.as_str() }),
            )
            .with_detail(error.to_string())
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
