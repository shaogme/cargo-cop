pub use u18n::{PluralCount, RenderError};

use std::{error::Error, fmt};

use language_tags::LanguageTag;

include!(concat!(env!("OUT_DIR"), "/u18n_messages.rs"));

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Locale(&'static str);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocaleError {
    value: String,
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
        Ok(Self::EN_US)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }

    pub fn as_tag(self) -> LocaleTag {
        match self.0 {
            "en-US" => LocaleTag::EnUs,
            _ => LocaleTag::ZhCn,
        }
    }
}

impl LocaleError {
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for LocaleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unsupported locale `{}`", self.value)
    }
}

impl Error for LocaleError {}

fn resolve_tag(value: &str) -> Option<Locale> {
    let tag = LanguageTag::parse(value.trim()).ok()?;
    tag.validate().ok()?;
    if tag.primary_language().eq_ignore_ascii_case("zh") {
        Some(Locale::ZH_CN)
    } else if tag.primary_language().eq_ignore_ascii_case("en") {
        Some(Locale::EN_US)
    } else {
        None
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
        return Some(Locale::EN_US);
    }
    let normalized = without_encoding.replace('_', "-");
    let tag = LanguageTag::parse(&normalized).ok()?;
    tag.validate().ok()?;
    if tag.primary_language().eq_ignore_ascii_case("zh") {
        Some(Locale::ZH_CN)
    } else {
        Some(Locale::EN_US)
    }
}

#[cfg(test)]
mod tests {
    use super::Locale;

    #[test]
    fn resolves_posix_environment_values_before_lower_priority_values() {
        assert_eq!(
            Locale::resolve(None, None, &[Some("C.UTF-8"), Some("zh_CN.UTF-8")]),
            Ok(Locale::EN_US)
        );
        assert_eq!(
            Locale::resolve(None, None, &[Some("POSIX@latin"), Some("zh_CN")]),
            Ok(Locale::EN_US)
        );
        assert_eq!(
            Locale::resolve(None, None, &[Some("123"), Some("zh_CN.UTF-8")]),
            Ok(Locale::ZH_CN)
        );
    }

    #[test]
    fn rejects_explicit_locales_outside_supported_languages() {
        assert_eq!(
            Locale::resolve(Some("en"), Some("zh-CN"), &[Some("zh_CN")]),
            Ok(Locale::EN_US)
        );
        assert!(Locale::resolve(None, Some("fr-FR"), &[Some("en_US")]).is_err());
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
}
