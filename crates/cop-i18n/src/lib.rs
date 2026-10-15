#![forbid(unsafe_code)]

use std::{borrow::Cow, collections::HashMap};

use cop_model::{LocalizedMessage, MessageArgument, MessageId};
use fluent_bundle::FluentValue;
use fluent_templates::static_loader;
use language_tags::LanguageTag;
use log::error;
use unic_langid::LanguageIdentifier;

include!(concat!(env!("OUT_DIR"), "/supported_locales.rs"));

static_loader! {
    static LOCALES = {
        locales: "./locales",
        fallback_language: "zh-CN",
    };
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Locale(&'static str);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocaleError {
    value: String,
}

pub struct MessageFormatter {
    locale: Locale,
}

impl Locale {
    pub const ZH_CN: Self = Self("zh-CN");
    pub const EN_US: Self = Self("en-US");

    pub fn parse_explicit(value: &str) -> Result<Self, LocaleError> {
        resolve_tag(value).ok_or_else(|| LocaleError {
            value: value.to_owned(),
        })
    }

    pub fn resolve(
        cli_value: Option<&str>,
        config_value: Option<&str>,
        environment_values: &[Option<&str>],
    ) -> Result<Self, LocaleError> {
        if let Some(value) = cli_value {
            return Self::parse_explicit(value);
        }
        if let Some(value) = config_value {
            return Self::parse_explicit(value);
        }
        for value in environment_values.iter().flatten() {
            if let Some(locale) = resolve_environment_value(value) {
                return Ok(locale);
            }
        }
        Ok(Self::ZH_CN)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }

    fn language_identifier(self) -> LanguageIdentifier {
        self.as_str().parse().expect("内置 locale tag 有效")
    }
}

impl LocaleError {
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl MessageFormatter {
    pub fn new(locale: Locale) -> Self {
        Self { locale }
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    pub fn format(&self, message: &LocalizedMessage) -> String {
        if let Err(error) = message.validate() {
            error!(
                "invalid message arguments for {}: {}",
                error.id().as_str(),
                error.argument()
            );
            return message.id().as_str().to_owned();
        }
        let arguments = fluent_arguments(message);
        let requested = self.locale.language_identifier();
        if let Some(value) =
            LOCALES.lookup_no_default_fallback(&requested, message.id().as_str(), Some(&arguments))
        {
            return value;
        }
        error!(
            "missing Fluent message {} for {}",
            message.id().as_str(),
            self.locale.as_str()
        );
        if self.locale != Locale::ZH_CN {
            let fallback = Locale::ZH_CN.language_identifier();
            if let Some(value) = LOCALES.lookup_no_default_fallback(
                &fallback,
                message.id().as_str(),
                Some(&arguments),
            ) {
                return value;
            }
        }
        message.id().as_str().to_owned()
    }

    pub fn text(&self, id: MessageId) -> String {
        self.format(&LocalizedMessage::new(id))
    }
}

fn fluent_arguments(
    message: &LocalizedMessage,
) -> HashMap<Cow<'static, str>, FluentValue<'static>> {
    message
        .arguments()
        .iter()
        .map(|(name, argument)| {
            let value = match argument {
                MessageArgument::Text(value) | MessageArgument::Opaque(value) => {
                    FluentValue::from(value.clone())
                }
                MessageArgument::Number(value) => FluentValue::from(*value),
                MessageArgument::Boolean(value) => {
                    FluentValue::from(if *value { "true" } else { "false" })
                }
            };
            (Cow::Owned(name.clone()), value)
        })
        .collect()
}

fn resolve_tag(value: &str) -> Option<Locale> {
    let tag = LanguageTag::parse(value.trim()).ok()?;
    tag.validate().ok()?;
    if tag.extended_language().is_some() {
        return Some(Locale::ZH_CN);
    }

    let mut subtags = vec![tag.primary_language().to_ascii_lowercase()];
    if let Some(script) = tag.script() {
        subtags.push(script.to_owned());
    }
    if let Some(region) = tag.region() {
        subtags.push(region.to_owned());
    }
    subtags.extend(tag.variant_subtags().map(str::to_owned));
    let language_identifier = match subtags.join("-").parse::<LanguageIdentifier>() {
        Ok(language_identifier) => language_identifier,
        Err(_) => return Some(Locale::ZH_CN),
    };
    let requested = language_identifier.to_string();
    let resource_tag = match requested.as_str() {
        "en" => "en-US",
        "zh" => "zh-CN",
        _ => requested.as_str(),
    };
    let Some(locale) = SUPPORTED_LOCALES
        .iter()
        .find(|locale| locale.eq_ignore_ascii_case(resource_tag))
    else {
        return Some(Locale::ZH_CN);
    };
    match *locale {
        "zh-CN" => Some(Locale::ZH_CN),
        "en-US" => Some(Locale::EN_US),
        other => Some(Locale(other)),
    }
}

fn resolve_environment_value(value: &str) -> Option<Locale> {
    let trimmed = value.trim();
    let without_modifier = trimmed.split('@').next()?;
    let without_encoding = without_modifier.split('.').next()?;
    if matches!(
        without_encoding.to_ascii_uppercase().as_str(),
        "C" | "POSIX"
    ) {
        return Some(Locale::ZH_CN);
    }
    let normalized = without_encoding.replace('_', "-");
    resolve_tag(&normalized)
}

#[cfg(test)]
mod tests {
    use super::{Locale, SUPPORTED_LOCALES};

    #[test]
    fn resolves_posix_environment_values_before_lower_priority_values() {
        assert_eq!(
            Locale::resolve(None, None, &[Some("C.UTF-8"), Some("en_US.UTF-8")]),
            Ok(Locale::ZH_CN)
        );
        assert_eq!(
            Locale::resolve(None, None, &[Some("POSIX@latin"), Some("en_US")]),
            Ok(Locale::ZH_CN)
        );
        assert_eq!(
            Locale::resolve(None, None, &[Some("123"), Some("en_US.UTF-8")]),
            Ok(Locale::EN_US)
        );
    }

    #[test]
    fn resolves_explicit_locale_values_before_config_and_environment() {
        assert_eq!(
            Locale::resolve(Some("en"), Some("zh-CN"), &[Some("zh_CN")]),
            Ok(Locale::EN_US)
        );
        assert_eq!(
            Locale::resolve(None, Some("fr-FR"), &[Some("en_US")]),
            Ok(Locale::ZH_CN)
        );
        assert_eq!(
            Locale::parse_explicit("en-US-u-ca-gregory"),
            Ok(Locale::EN_US)
        );
        assert!(Locale::resolve(Some("en-u"), None, &[]).is_err());
    }

    #[test]
    fn resolves_locale_tags_case_insensitively() {
        assert_eq!(Locale::parse_explicit("EN-us"), Ok(Locale::EN_US));
        assert_eq!(Locale::parse_explicit("zh-cn"), Ok(Locale::ZH_CN));
        assert_eq!(
            Locale::resolve(None, None, &[Some("en_us.UTF-8")]),
            Ok(Locale::EN_US)
        );
    }

    #[test]
    fn resolves_every_embedded_locale_from_the_catalog_list() {
        for locale in SUPPORTED_LOCALES {
            let resolved = Locale::parse_explicit(locale).expect("嵌入目录中的 locale 有效");
            let expected = *locale;
            assert_eq!(resolved.as_str(), expected);
        }
    }
}
