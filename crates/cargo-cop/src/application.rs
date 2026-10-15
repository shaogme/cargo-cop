use std::{
    collections::{BTreeSet, HashSet},
    env,
    ffi::{OsStr, OsString},
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{error::ErrorKind as ClapErrorKind, ArgAction, Parser, ValueEnum};
use cop_analysis::{AnalysisRequest, ProjectAnalyzer};
use cop_cargo::{
    CargoDiscovery, CargoDiscoveryRequest, CargoError, CargoProject, CargoProjectLoader,
    CargoSelection, FeatureSelection, TargetSelector,
};
use cop_config::{
    ConfigError, ConfigLoader, ConfigOverrides, ConfigRequest, ConfigResolutionInput,
    ConfigResolver, ResolvedConfig, ScanTargetScope,
};
use cop_i18n::{Locale, MessageFormatter};
use cop_model::{
    Diagnostic, LocalizedMessage, RuleId, ScanResult, ScanStatus, Severity, ToolErrorKind,
    UnknownRuleId, ARGUMENT_DUPLICATE_RULE, ARGUMENT_SEVERITY_DUPLICATE, ARGUMENT_SEVERITY_FORMAT,
    ARGUMENT_UNKNOWN_RULE, ARGUMENT_UNKNOWN_SEVERITY, CARGO_WORKSPACE_CONFLICT, CLI_HELP,
    CLI_PARSE_ERROR, CLI_VERSION, IO_ERROR, LOCALE_INVALID_CLI, LOCALE_INVALID_CONFIG,
    OUTPUT_WRITE_ERROR,
};
use cop_report::{ReportFormat, Reporter};
use cop_rules::RuleEngine;
use log::{set_logger, set_max_level, Level, LevelFilter, Log, Metadata, Record};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Parser)]
#[command(name = "cargo-cop")]
struct CliOptions {
    #[arg(long, value_name = "PATH")]
    manifest_path: Option<PathBuf>,
    #[arg(long)]
    workspace: bool,
    #[arg(short = 'p', long = "package", action = ArgAction::Append, value_name = "NAME")]
    packages: Vec<String>,
    #[arg(long, value_name = "TRIPLE")]
    target: Option<String>,
    #[arg(long, value_delimiter = ',', action = ArgAction::Append, value_name = "LIST")]
    features: Vec<String>,
    #[arg(long)]
    all_features: bool,
    #[arg(long)]
    no_default_features: bool,
    #[arg(long)]
    all_targets: bool,
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    #[arg(long, value_name = "TAG")]
    locale: Option<String>,
    #[arg(long, value_enum, default_value = "human")]
    format: OutputFormat,
    #[arg(long = "disable-rule", action = ArgAction::Append, value_name = "ID")]
    disabled_rules: Vec<String>,
    #[arg(long = "severity", action = ArgAction::Append, value_name = "ID=warning|error")]
    severities: Vec<String>,
}

struct ApplicationError {
    kind: ToolErrorKind,
    message: LocalizedMessage,
    locale_override: Option<Locale>,
}

struct ScanOutput {
    result: ScanResult,
    workspace_root: PathBuf,
    locale: Locale,
}

struct PreparedScan {
    project: CargoProject,
    configuration: ResolvedConfig,
    locale: Locale,
}

struct StartupContext {
    locale: Locale,
    discovery: Option<CargoDiscovery>,
}

struct StderrLogger;

static STDERR_LOGGER: StderrLogger = StderrLogger;

pub struct Application;

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Error
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            let _ = writeln!(io::stderr().lock(), "{}: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

impl OutputFormat {
    fn report_format(self) -> ReportFormat {
        match self {
            Self::Human => ReportFormat::Human,
            Self::Json => ReportFormat::Json,
        }
    }
}

impl ApplicationError {
    fn new(kind: ToolErrorKind, message: LocalizedMessage) -> Self {
        Self {
            kind,
            message,
            locale_override: None,
        }
    }

    fn with_locale(mut self, locale: Locale) -> Self {
        self.locale_override = Some(locale);
        self
    }

    fn from_cargo(error: CargoError) -> Self {
        Self::new(error.kind(), error.message().clone())
    }

    fn from_config(error: ConfigError) -> Self {
        Self::new(ToolErrorKind::Configuration, error.message().clone())
    }
}

impl Application {
    pub fn run() -> ExitCode {
        let _ = set_logger(&STDERR_LOGGER);
        set_max_level(LevelFilter::Error);
        let mut arguments = env::args_os().collect::<Vec<_>>();
        if arguments.get(1).is_some_and(|argument| argument == "cop") {
            arguments.remove(1);
        }
        let fallback_root = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let lc_all = env::var("LC_ALL").ok();
        let lc_messages = env::var("LC_MESSAGES").ok();
        let lang = env::var("LANG").ok();
        let format = report_format_hint(&arguments);
        let environment_locale = Locale::resolve(
            None,
            None,
            &[lc_all.as_deref(), lc_messages.as_deref(), lang.as_deref()],
        )
        .unwrap_or(Locale::ZH_CN);
        let startup = match Self::startup_locale(&arguments, environment_locale) {
            Ok(startup) => startup,
            Err(error) => {
                return Self::write_error_report(error, format, &fallback_root, environment_locale);
            }
        };
        let locale = startup.locale;
        if contains_option(&arguments, "--help", "-h") {
            let help = MessageFormatter::new(locale).text(CLI_HELP);
            let _ = writeln!(io::stdout().lock(), "{help}");
            return ExitCode::SUCCESS;
        }
        if contains_option(&arguments, "--version", "-V") {
            let version =
                LocalizedMessage::new(CLI_VERSION).with_text("version", env!("CARGO_PKG_VERSION"));
            let text = MessageFormatter::new(locale).format(&version);
            let _ = writeln!(io::stdout().lock(), "{text}");
            return ExitCode::SUCCESS;
        }

        let options = match CliOptions::try_parse_from(arguments) {
            Ok(options) => options,
            Err(error) => {
                let exit_code = error.exit_code().clamp(0, 255) as u8;
                let kind = cli_parse_error_kind(error.kind());
                let message = LocalizedMessage::new(CLI_PARSE_ERROR).with_text("kind", kind);
                let report_error = ApplicationError::new(ToolErrorKind::Arguments, message);
                let _ = Self::write_error_report(report_error, format, &fallback_root, locale);
                return ExitCode::from(exit_code);
            }
        };
        let format = options.format.report_format();
        match Self::scan(options, locale, startup.discovery) {
            Ok(output) => {
                if let Err(error) = Self::write_report(
                    &output.result,
                    &output.workspace_root,
                    format,
                    output.locale,
                ) {
                    Self::write_output_error(output.locale, error);
                    return ExitCode::from(2);
                }
                ExitCode::from(output.result.exit_code())
            }
            Err(error) => Self::write_error_report(error, format, &fallback_root, locale),
        }
    }

    fn startup_locale(
        arguments: &[OsString],
        environment_locale: Locale,
    ) -> Result<StartupContext, ApplicationError> {
        if let Some(value) = option_value(arguments, "--locale") {
            let locale = Locale::parse_explicit(&value).map_err(|_| {
                ApplicationError::new(
                    ToolErrorKind::Arguments,
                    LocalizedMessage::new(LOCALE_INVALID_CLI).with_text("locale", value),
                )
                .with_locale(Locale::ZH_CN)
            })?;
            return Ok(StartupContext {
                locale,
                discovery: None,
            });
        }

        let current_directory = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let (request, discovery) = config_request(arguments, current_directory);
        let config_locale = ConfigLoader::read_locale(request).map_err(|error| {
            ApplicationError::from_config(error).with_locale(environment_locale)
        })?;
        match config_locale {
            Some(value) => {
                let locale = Locale::parse_explicit(&value).map_err(|_| {
                    ApplicationError::new(
                        ToolErrorKind::Configuration,
                        LocalizedMessage::new(LOCALE_INVALID_CONFIG).with_text("locale", value),
                    )
                    .with_locale(Locale::ZH_CN)
                })?;
                Ok(StartupContext { locale, discovery })
            }
            None => Ok(StartupContext {
                locale: environment_locale,
                discovery,
            }),
        }
    }

    fn write_error_report(
        error: ApplicationError,
        format: ReportFormat,
        workspace_root: &Path,
        fallback_locale: Locale,
    ) -> ExitCode {
        let locale = error.locale_override.unwrap_or(fallback_locale);
        let result = ScanResult::new(
            ScanStatus::Incomplete,
            vec![Diagnostic::tool(error.kind, error.message)],
            0,
        );
        if let Err(write_error) = Self::write_report(&result, workspace_root, format, locale) {
            Self::write_output_error(locale, write_error);
        }
        ExitCode::from(result.exit_code())
    }

    fn write_output_error(locale: Locale, error: io::Error) {
        let message =
            LocalizedMessage::new(OUTPUT_WRITE_ERROR).with_opaque("detail", error.to_string());
        let text = MessageFormatter::new(locale).format(&message);
        let _ = writeln!(io::stderr().lock(), "{text}");
    }

    fn scan(
        options: CliOptions,
        fallback_locale: Locale,
        discovery: Option<CargoDiscovery>,
    ) -> Result<ScanOutput, ApplicationError> {
        let prepared = Self::prepare(&options, fallback_locale, discovery)?;
        let mut diagnostics = Vec::new();
        let mut status = ScanStatus::Complete;
        for package in prepared.project.packages() {
            let snapshot = ProjectAnalyzer::analyze(AnalysisRequest::new(package));
            status = status.combine(snapshot.status());
            diagnostics.extend(snapshot.diagnostics().iter().cloned());
            let rule_diagnostics = RuleEngine::run(&snapshot, &prepared.configuration);
            status = status.combine(ScanStatus::from_diagnostics(&rule_diagnostics));
            diagnostics.extend(rule_diagnostics);
        }
        let mut unique_diagnostics = HashSet::new();
        diagnostics.retain(|diagnostic| unique_diagnostics.insert(diagnostic.clone()));
        let suppressed_warnings = if prepared.configuration.allow_warnings() {
            let count = diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity() == Severity::Warning)
                .count();
            diagnostics.retain(|diagnostic| diagnostic.severity() != Severity::Warning);
            count
        } else {
            0
        };
        Ok(ScanOutput {
            result: ScanResult::new(status, diagnostics, suppressed_warnings),
            workspace_root: prepared.project.workspace_root().to_path_buf(),
            locale: prepared.locale,
        })
    }

    fn prepare(
        options: &CliOptions,
        fallback_locale: Locale,
        discovery: Option<CargoDiscovery>,
    ) -> Result<PreparedScan, ApplicationError> {
        if options.workspace && !options.packages.is_empty() {
            return Err(ApplicationError::new(
                ToolErrorKind::Arguments,
                LocalizedMessage::new(CARGO_WORKSPACE_CONFLICT),
            ));
        }
        let overrides = Self::config_overrides(options)?;
        let current_directory = env::current_dir().map_err(|error| {
            ApplicationError::new(
                ToolErrorKind::Io,
                LocalizedMessage::new(IO_ERROR).with_opaque("detail", error.to_string()),
            )
        })?;
        let manifest_path = options
            .manifest_path
            .clone()
            .unwrap_or_else(|| current_directory.join("Cargo.toml"));
        let discovery = match discovery {
            Some(discovery) => discovery,
            None => CargoProjectLoader::discover(CargoDiscoveryRequest::new(manifest_path))
                .map_err(ApplicationError::from_cargo)?,
        };
        let loaded_config = ConfigLoader::load(ConfigRequest::new(
            options.config.clone(),
            discovery.workspace_root().to_path_buf(),
            current_directory.clone(),
        ))
        .map_err(ApplicationError::from_config)?;
        let locale = if let Some(value) = options.locale.as_deref() {
            Locale::parse_explicit(value).map_err(|_| {
                ApplicationError::new(
                    ToolErrorKind::Arguments,
                    LocalizedMessage::new(LOCALE_INVALID_CLI).with_text("locale", value),
                )
                .with_locale(Locale::ZH_CN)
            })?
        } else if let Some(value) = loaded_config.locale() {
            Locale::parse_explicit(value).map_err(|_| {
                ApplicationError::new(
                    ToolErrorKind::Configuration,
                    LocalizedMessage::new(LOCALE_INVALID_CONFIG).with_text("locale", value),
                )
                .with_locale(Locale::ZH_CN)
            })?
        } else {
            fallback_locale
        };
        let configuration =
            ConfigResolver::resolve(ConfigResolutionInput::new(loaded_config, overrides));
        let scan = configuration.scan();
        let feature_selection = FeatureSelection::new(
            scan.features().to_vec(),
            configuration.all_features(),
            scan.default_features(),
        );
        let selection = CargoSelection::new(feature_selection)
            .with_workspace(options.workspace)
            .with_packages(options.packages.clone())
            .with_all_targets(scan.target_scope() == ScanTargetScope::All)
            .with_target(scan.target().map(str::to_owned))
            .with_current_directory(current_directory);
        let project =
            TargetSelector::select(discovery, selection).map_err(ApplicationError::from_cargo)?;
        Ok(PreparedScan {
            project,
            configuration,
            locale,
        })
    }

    fn config_overrides(options: &CliOptions) -> Result<ConfigOverrides, ApplicationError> {
        let mut overrides = ConfigOverrides::default()
            .with_target(options.target.clone())
            .with_features(if options.features.is_empty() {
                None
            } else {
                Some(options.features.clone())
            })
            .with_all_features(options.all_features)
            .with_no_default_features(options.no_default_features)
            .with_all_targets(options.all_targets);
        let mut disabled = BTreeSet::new();
        for value in &options.disabled_rules {
            let rule: RuleId = value.parse().map_err(|error: UnknownRuleId| {
                ApplicationError::new(
                    ToolErrorKind::Arguments,
                    LocalizedMessage::new(ARGUMENT_UNKNOWN_RULE).with_text("rule", error.value()),
                )
            })?;
            if !disabled.insert(rule) {
                return Err(ApplicationError::new(
                    ToolErrorKind::Arguments,
                    LocalizedMessage::new(ARGUMENT_DUPLICATE_RULE).with_text("rule", rule.as_str()),
                ));
            }
            overrides = overrides.disable_rule(rule);
        }
        let mut severities = BTreeSet::new();
        for value in &options.severities {
            let (rule_name, severity_name) = value.split_once('=').ok_or_else(|| {
                ApplicationError::new(
                    ToolErrorKind::Arguments,
                    LocalizedMessage::new(ARGUMENT_SEVERITY_FORMAT).with_text("value", value),
                )
            })?;
            let rule: RuleId = rule_name.parse().map_err(|error: UnknownRuleId| {
                ApplicationError::new(
                    ToolErrorKind::Arguments,
                    LocalizedMessage::new(ARGUMENT_UNKNOWN_RULE).with_text("rule", error.value()),
                )
            })?;
            if !severities.insert(rule) {
                return Err(ApplicationError::new(
                    ToolErrorKind::Arguments,
                    LocalizedMessage::new(ARGUMENT_SEVERITY_DUPLICATE)
                        .with_text("rule", rule.as_str()),
                ));
            }
            let severity = match severity_name {
                "warning" => Severity::Warning,
                "error" => Severity::Error,
                other => {
                    return Err(ApplicationError::new(
                        ToolErrorKind::Arguments,
                        LocalizedMessage::new(ARGUMENT_UNKNOWN_SEVERITY)
                            .with_text("severity", other),
                    ));
                }
            };
            overrides = overrides.set_severity(rule, severity);
        }
        Ok(overrides)
    }

    fn write_report(
        result: &ScanResult,
        workspace_root: &Path,
        format: ReportFormat,
        locale: Locale,
    ) -> io::Result<()> {
        let report = Reporter::render(result, workspace_root, format, locale);
        writeln!(io::stdout().lock(), "{report}")
    }
}

fn option_value(arguments: &[OsString], option: &str) -> Option<String> {
    let prefix = format!("{option}=");
    let mut index = 1usize;
    while index < arguments.len() {
        let argument = arguments[index].as_os_str();
        if argument == OsStr::new("--") {
            break;
        }
        let text = argument.to_str();
        if text == Some(option) {
            return arguments
                .get(index + 1)
                .and_then(|value| value.to_str())
                .filter(|value| !value.starts_with('-'))
                .map(str::to_owned);
        }
        if let Some(value) = text.and_then(|value| value.strip_prefix(&prefix)) {
            return Some(value.to_owned());
        }
        if value_option(argument) {
            let has_inline_value = text.is_some_and(|value| value.contains('='));
            let has_separate_value = arguments
                .get(index + 1)
                .and_then(|value| value.to_str())
                .is_some_and(|value| !value.starts_with('-'));
            index += if has_inline_value || !has_separate_value {
                1
            } else {
                2
            };
        } else {
            index += 1;
        }
    }
    None
}

fn report_format_hint(arguments: &[OsString]) -> ReportFormat {
    match option_value(arguments, "--format").as_deref() {
        Some("json") => ReportFormat::Json,
        _ => ReportFormat::Human,
    }
}

fn contains_option(arguments: &[OsString], long: &str, short: &str) -> bool {
    arguments
        .iter()
        .skip(1)
        .take_while(|argument| argument.as_os_str() != OsStr::new("--"))
        .any(|argument| {
            argument
                .to_str()
                .is_some_and(|value| value == long || value == short)
        })
}

fn value_option(argument: &OsStr) -> bool {
    let Some(argument) = argument.to_str() else {
        return false;
    };
    let option = argument
        .split_once('=')
        .map_or(argument, |(option, _)| option);
    matches!(
        option,
        "--manifest-path"
            | "-p"
            | "--package"
            | "--target"
            | "--features"
            | "--config"
            | "--locale"
            | "--format"
            | "--disable-rule"
            | "--severity"
    )
}

fn cli_parse_error_kind(kind: ClapErrorKind) -> &'static str {
    match kind {
        ClapErrorKind::UnknownArgument | ClapErrorKind::InvalidSubcommand => "unknown",
        ClapErrorKind::MissingRequiredArgument
        | ClapErrorKind::MissingSubcommand
        | ClapErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        | ClapErrorKind::TooFewValues => "missing",
        ClapErrorKind::ArgumentConflict => "conflict",
        ClapErrorKind::InvalidValue
        | ClapErrorKind::NoEquals
        | ClapErrorKind::ValueValidation
        | ClapErrorKind::TooManyValues
        | ClapErrorKind::WrongNumberOfValues
        | ClapErrorKind::InvalidUtf8 => "invalid",
        _ => "other",
    }
}

fn config_request(
    arguments: &[OsString],
    current_directory: PathBuf,
) -> (ConfigRequest, Option<CargoDiscovery>) {
    let manifest_path = option_value(arguments, "--manifest-path")
        .map(PathBuf::from)
        .unwrap_or_else(|| current_directory.join("Cargo.toml"));
    let manifest_directory = manifest_path.clone();
    let manifest_directory = manifest_directory
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| current_directory.clone());
    let explicit_config = option_value(arguments, "--config").map(PathBuf::from);
    let discovery = if explicit_config.is_none() {
        CargoProjectLoader::discover(CargoDiscoveryRequest::new(manifest_path)).ok()
    } else {
        None
    };
    let discovery_directory = discovery
        .as_ref()
        .map(|discovery| discovery.workspace_root().to_path_buf())
        .unwrap_or(manifest_directory);
    (
        ConfigRequest::new(explicit_config, discovery_directory, current_directory),
        discovery,
    )
}
