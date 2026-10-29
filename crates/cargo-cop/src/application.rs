use std::{
    collections::{BTreeSet, HashSet},
    env,
    error::Error,
    ffi::{OsStr, OsString},
    fmt,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use crate::rule_catalog::{RuleQuery, RuleQueryError};
use clap::{error::ErrorKind as ClapErrorKind, ArgAction, Parser, ValueEnum};
use cop_analysis::{AnalysisRequest, ProjectAnalyzer};
use cop_cargo::{
    CargoDiscovery, CargoDiscoveryRequest, CargoFailure, CargoProject, CargoProjectLoader,
    CargoSelection, FeatureSelection, TargetSelector,
};
use cop_cfg::{CfgExpression, CfgValue};
use cop_config::{
    ConfigFailure, ConfigLoader, ConfigOverrides, ConfigRequest, ConfigResolutionInput,
    ConfigResolver, ResolvedConfig, ScanTargetScope,
};
use cop_i18n::{set_current_locale, t, Locale, RenderError};
use cop_model::{
    Diagnostic, RuleId, ScanResult, ScanStatus, Severity, ToolErrorKind, UnknownRuleId,
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
    #[arg(long = "cfg", action = ArgAction::Append, value_name = "CFG")]
    cfg_values: Vec<String>,
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
    #[arg(
        long,
        conflicts_with = "explain_rule",
        conflicts_with_all = [
            "manifest_path",
            "workspace",
            "packages",
            "target",
            "features",
            "cfg_values",
            "all_features",
            "no_default_features",
            "all_targets",
            "config",
            "disabled_rules",
            "severities"
        ]
    )]
    list_rules: bool,
    #[arg(
        long,
        value_name = "ID",
        conflicts_with = "list_rules",
        conflicts_with_all = [
            "manifest_path",
            "workspace",
            "packages",
            "target",
            "features",
            "cfg_values",
            "all_features",
            "no_default_features",
            "all_targets",
            "config",
            "disabled_rules",
            "severities"
        ]
    )]
    explain_rule: Option<String>,
    #[arg(long = "disable-rule", action = ArgAction::Append, value_name = "ID")]
    disabled_rules: Vec<String>,
    #[arg(long = "severity", action = ArgAction::Append, value_name = "ID=warning|error")]
    severities: Vec<String>,
}

#[derive(Debug)]
pub(super) struct ApplicationError {
    kind: ToolErrorKind,
    message: String,
    detail: Option<String>,
    locale_override: Option<Locale>,
}

#[derive(Debug)]
pub(super) enum ApplicationFailure {
    Domain(ApplicationError),
    Render(RenderError),
    Io(io::Error),
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

pub(super) struct Application;

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
    fn new(kind: ToolErrorKind, message: String) -> Self {
        Self {
            kind,
            message,
            detail: None,
            locale_override: None,
        }
    }

    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    fn with_locale(mut self, locale: Locale) -> Self {
        self.locale_override = Some(locale);
        self
    }
}

impl ApplicationFailure {
    fn localized(kind: ToolErrorKind, message: Result<String, RenderError>) -> Self {
        match message {
            Ok(message) => Self::Domain(ApplicationError::new(kind, message)),
            Err(error) => Self::Render(error),
        }
    }

    fn from_cargo(failure: CargoFailure) -> Self {
        match failure {
            CargoFailure::Cargo(error) => {
                let domain_error = ApplicationError::new(error.kind(), error.message().to_owned());
                let domain_error = match error.detail() {
                    Some(detail) => domain_error.with_detail(detail.to_owned()),
                    None => domain_error,
                };
                Self::Domain(domain_error)
            }
            CargoFailure::Render(error) => Self::Render(error),
        }
    }

    fn from_config(failure: ConfigFailure) -> Self {
        match failure {
            ConfigFailure::Config(error) => {
                let domain_error =
                    ApplicationError::new(ToolErrorKind::Configuration, error.message().to_owned());
                let domain_error = match error.detail() {
                    Some(detail) => domain_error.with_detail(detail.to_owned()),
                    None => domain_error,
                };
                Self::Domain(domain_error)
            }
            ConfigFailure::Render(error) => Self::Render(error),
        }
    }

    fn with_locale(self, locale: Locale) -> Self {
        match self {
            Self::Domain(error) => Self::Domain(error.with_locale(locale)),
            failure => failure,
        }
    }

    fn with_detail(self, detail: impl Into<String>) -> Self {
        match self {
            Self::Domain(error) => Self::Domain(error.with_detail(detail)),
            failure => failure,
        }
    }
}

impl fmt::Display for ApplicationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain(error) => {
                formatter.write_str(&error.message)?;
                if let Some(detail) = &error.detail {
                    write!(formatter, ": {detail}")?;
                }
                Ok(())
            }
            Self::Render(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl Error for ApplicationFailure {}

impl From<RenderError> for ApplicationFailure {
    fn from(error: RenderError) -> Self {
        Self::Render(error)
    }
}

impl From<io::Error> for ApplicationFailure {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl Application {
    pub(super) fn run() -> Result<ExitCode, ApplicationFailure> {
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
        .unwrap_or(Locale::EN_US);
        set_current_locale(environment_locale.as_tag());
        let query_requested = rule_query_requested(&arguments);
        let startup = match Self::startup_locale(&arguments, environment_locale, !query_requested) {
            Ok(startup) => startup,
            Err(ApplicationFailure::Domain(error)) => {
                return Self::write_error_report(error, format, &fallback_root, environment_locale);
            }
            Err(error) => return Err(error),
        };
        let locale = startup.locale;
        if contains_option(&arguments, "--help", "-h") {
            let help = t!("cli.help")?;
            writeln!(io::stdout().lock(), "{help}")?;
            return Ok(ExitCode::SUCCESS);
        }
        if contains_option(&arguments, "--version", "-V") {
            let version = t!("cli.version", { version: env!("CARGO_PKG_VERSION") })?;
            writeln!(io::stdout().lock(), "{version}")?;
            return Ok(ExitCode::SUCCESS);
        }

        let options = match CliOptions::try_parse_from(arguments) {
            Ok(options) => options,
            Err(error) => {
                let exit_code = error.exit_code().clamp(0, 255) as u8;
                let kind = cli_parse_error_kind(error.kind());
                let message = t!("cli.parse_error", { kind: kind })?;
                let report_error = ApplicationError::new(ToolErrorKind::Arguments, message);
                Self::write_error_report(report_error, format, &fallback_root, locale)?;
                return Ok(ExitCode::from(exit_code));
            }
        };
        let format = options.format.report_format();
        if options.list_rules || options.explain_rule.is_some() {
            return Self::query_rules(options, locale, &fallback_root);
        }
        match Self::scan(options, locale, startup.discovery) {
            Ok(output) => {
                match Self::write_report(
                    &output.result,
                    &output.workspace_root,
                    format,
                    output.locale,
                ) {
                    Ok(()) => Ok(ExitCode::from(output.result.exit_code())),
                    Err(ApplicationFailure::Io(error)) => {
                        Self::write_output_error(output.locale, error)?;
                        Ok(ExitCode::from(2))
                    }
                    Err(error) => Err(error),
                }
            }
            Err(ApplicationFailure::Domain(error)) => {
                Self::write_error_report(error, format, &fallback_root, locale)
            }
            Err(error) => Err(error),
        }
    }

    fn startup_locale(
        arguments: &[OsString],
        environment_locale: Locale,
        read_project_config: bool,
    ) -> Result<StartupContext, ApplicationFailure> {
        if let Some(value) = option_value(arguments, "--locale") {
            let locale = Locale::parse_explicit(&value).map_err(|_| {
                ApplicationFailure::localized(
                    ToolErrorKind::Arguments,
                    t!("locale.invalid_cli", { locale: value.as_str() }),
                )
                .with_locale(environment_locale)
            })?;
            set_current_locale(locale.as_tag());
            return Ok(StartupContext {
                locale,
                discovery: None,
            });
        }

        if !read_project_config {
            return Ok(StartupContext {
                locale: environment_locale,
                discovery: None,
            });
        }

        let current_directory = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let (request, discovery) = config_request(arguments, current_directory);
        let config_locale =
            ConfigLoader::read_locale(request).map_err(ApplicationFailure::from_config)?;
        match config_locale {
            Some(value) => {
                let locale = Locale::parse_explicit(&value).map_err(|_| {
                    ApplicationFailure::localized(
                        ToolErrorKind::Configuration,
                        t!("locale.invalid_config", { locale: value.as_str() }),
                    )
                    .with_locale(environment_locale)
                })?;
                set_current_locale(locale.as_tag());
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
    ) -> Result<ExitCode, ApplicationFailure> {
        let locale = error.locale_override.unwrap_or(fallback_locale);
        let mut diagnostic = Diagnostic::tool(error.kind, error.message);
        if let Some(detail) = error.detail {
            diagnostic = diagnostic.with_detail(detail);
        }
        let result = ScanResult::new(ScanStatus::Incomplete, vec![diagnostic], 0);
        Self::write_report(&result, workspace_root, format, locale)?;
        Ok(ExitCode::from(result.exit_code()))
    }

    fn write_output_error(locale: Locale, error: io::Error) -> Result<(), ApplicationFailure> {
        let message = t!(
            "report.output_write_error",
            locale = locale.as_tag().as_locale(),
            {}
        )?;
        writeln!(io::stderr().lock(), "{message}: {error}")?;
        Ok(())
    }

    fn query_rules(
        options: CliOptions,
        locale: Locale,
        fallback_root: &Path,
    ) -> Result<ExitCode, ApplicationFailure> {
        let report_format = options.format.report_format();
        let output = match RuleQuery::render(
            options.list_rules,
            options.explain_rule.as_deref(),
            report_format,
            locale,
        ) {
            Ok(output) => output,
            Err(RuleQueryError::UnknownRule(error)) => {
                let message = t!("cli.argument_unknown_rule", { rule: error.value() })?;
                let report_error = ApplicationError::new(ToolErrorKind::Arguments, message);
                return Self::write_error_report(
                    report_error,
                    report_format,
                    fallback_root,
                    locale,
                );
            }
            Err(RuleQueryError::Render(error)) => return Err(ApplicationFailure::Render(error)),
        };
        match writeln!(io::stdout().lock(), "{output}") {
            Ok(()) => Ok(ExitCode::SUCCESS),
            Err(error) => {
                Self::write_output_error(locale, error)?;
                Ok(ExitCode::from(2))
            }
        }
    }

    fn scan(
        options: CliOptions,
        fallback_locale: Locale,
        discovery: Option<CargoDiscovery>,
    ) -> Result<ScanOutput, ApplicationFailure> {
        let prepared = Self::prepare(&options, fallback_locale, discovery)?;
        let mut diagnostics = Vec::new();
        let mut status = ScanStatus::Complete;
        for package in prepared.project.packages() {
            let snapshot = ProjectAnalyzer::analyze(AnalysisRequest::new(package))?;
            status = status.combine(snapshot.status());
            diagnostics.extend(snapshot.diagnostics().iter().cloned());
            let rule_diagnostics = RuleEngine::run(&snapshot, &prepared.configuration)?;
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
    ) -> Result<PreparedScan, ApplicationFailure> {
        if options.workspace && !options.packages.is_empty() {
            return Err(ApplicationFailure::localized(
                ToolErrorKind::Arguments,
                t!("cargo.workspace_conflict"),
            ));
        }
        let overrides = Self::config_overrides(options)?;
        let current_directory = env::current_dir().map_err(|error| {
            ApplicationFailure::localized(ToolErrorKind::Io, t!("cli.io_error"))
                .with_detail(error.to_string())
        })?;
        let manifest_path = options
            .manifest_path
            .clone()
            .unwrap_or_else(|| current_directory.join("Cargo.toml"));
        let discovery = match discovery {
            Some(discovery) => discovery,
            None => CargoProjectLoader::discover(CargoDiscoveryRequest::new(manifest_path))
                .map_err(ApplicationFailure::from_cargo)?,
        };
        let loaded_config = ConfigLoader::load(ConfigRequest::new(
            options.config.clone(),
            discovery.workspace_root().to_path_buf(),
            current_directory.clone(),
        ))
        .map_err(ApplicationFailure::from_config)?;
        let locale = if let Some(value) = options.locale.as_deref() {
            Locale::parse_explicit(value).map_err(|_| {
                ApplicationFailure::localized(
                    ToolErrorKind::Arguments,
                    t!("locale.invalid_cli", { locale: value }),
                )
                .with_locale(fallback_locale)
            })?
        } else if let Some(value) = loaded_config.locale() {
            Locale::parse_explicit(value).map_err(|_| {
                ApplicationFailure::localized(
                    ToolErrorKind::Configuration,
                    t!("locale.invalid_config", { locale: value }),
                )
                .with_locale(fallback_locale)
            })?
        } else {
            fallback_locale
        };
        set_current_locale(locale.as_tag());
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
            .with_cfg_overrides(configuration.cfg_overrides().to_vec())
            .with_current_directory(current_directory);
        let project =
            TargetSelector::select(discovery, selection).map_err(ApplicationFailure::from_cargo)?;
        Ok(PreparedScan {
            project,
            configuration,
            locale,
        })
    }

    fn config_overrides(options: &CliOptions) -> Result<ConfigOverrides, ApplicationFailure> {
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
        if !options.cfg_values.is_empty() {
            let mut values = BTreeSet::new();
            for value in &options.cfg_values {
                let expression = CfgExpression::parse(value).map_err(|_| {
                    ApplicationFailure::localized(
                        ToolErrorKind::Arguments,
                        t!("cargo.target_cfg_invalid", { condition: value }),
                    )
                })?;
                let cfg_value = match expression {
                    CfgExpression::Flag { name, .. } => CfgValue::new(name, None),
                    CfgExpression::KeyValue { name, value, .. } => CfgValue::new(name, Some(value)),
                    CfgExpression::All { .. }
                    | CfgExpression::Any { .. }
                    | CfgExpression::Not { .. } => {
                        return Err(ApplicationFailure::localized(
                            ToolErrorKind::Arguments,
                            t!("cargo.target_cfg_invalid", { condition: value }),
                        ));
                    }
                };
                if CfgValue::is_builtin_name(cfg_value.name()) {
                    return Err(ApplicationFailure::localized(
                        ToolErrorKind::Arguments,
                        t!("cargo.target_cfg_invalid", { condition: value }),
                    ));
                }
                values.insert(cfg_value);
            }
            overrides = overrides.with_cfg_values(Some(values.into_iter().collect()));
        }
        let mut disabled = BTreeSet::new();
        for value in &options.disabled_rules {
            let rule: RuleId = value.parse().map_err(|error: UnknownRuleId| {
                ApplicationFailure::localized(
                    ToolErrorKind::Arguments,
                    t!("cli.argument_unknown_rule", { rule: error.value() }),
                )
            })?;
            if !disabled.insert(rule) {
                return Err(ApplicationFailure::localized(
                    ToolErrorKind::Arguments,
                    t!("cli.argument_duplicate_rule", { rule: rule.as_str() }),
                ));
            }
            overrides = overrides.disable_rule(rule);
        }
        let mut severities = BTreeSet::new();
        for value in &options.severities {
            let (rule_name, severity_name) = value.split_once('=').ok_or_else(|| {
                ApplicationFailure::localized(
                    ToolErrorKind::Arguments,
                    t!("cli.argument_severity_format", { value: value.as_str() }),
                )
            })?;
            let rule: RuleId = rule_name.parse().map_err(|error: UnknownRuleId| {
                ApplicationFailure::localized(
                    ToolErrorKind::Arguments,
                    t!("cli.argument_unknown_rule", { rule: error.value() }),
                )
            })?;
            if !severities.insert(rule) {
                return Err(ApplicationFailure::localized(
                    ToolErrorKind::Arguments,
                    t!("cli.argument_severity_duplicate", { rule: rule.as_str() }),
                ));
            }
            let severity = match severity_name {
                "warning" => Severity::Warning,
                "error" => Severity::Error,
                other => {
                    return Err(ApplicationFailure::localized(
                        ToolErrorKind::Arguments,
                        t!("cli.argument_unknown_severity", { severity: other }),
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
    ) -> Result<(), ApplicationFailure> {
        let report = Reporter::render(result, workspace_root, format, locale)?;
        writeln!(io::stdout().lock(), "{report}")?;
        Ok(())
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

fn rule_query_requested(arguments: &[OsString]) -> bool {
    ["--list-rules", "--explain-rule"].iter().any(|option| {
        arguments
            .iter()
            .skip(1)
            .take_while(|argument| argument.as_os_str() != OsStr::new("--"))
            .any(|argument| {
                argument.to_str().is_some_and(|value| {
                    value == *option || value.starts_with(&format!("{option}="))
                })
            })
    })
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
            | "--cfg"
            | "--config"
            | "--locale"
            | "--format"
            | "--explain-rule"
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
