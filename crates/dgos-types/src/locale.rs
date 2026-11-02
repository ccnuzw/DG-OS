use serde::{Deserialize, Serialize};

/// The persisted host-level language preference (V1-FR-016, D118).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LanguagePreference {
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en-US")]
    EnUs,
    #[serde(rename = "system")]
    System,
}

impl LanguagePreference {
    pub const DEFAULT: Self = Self::ZhCn;

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::EnUs => "en-US",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "zh-CN" => Some(Self::ZhCn),
            "en-US" => Some(Self::EnUs),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

/// The resolved resource locale. It never contains the `system` preference.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Locale {
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en-US")]
    EnUs,
}

impl Locale {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::EnUs => "en-US",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct LanguagePreferences {
    pub preference: LanguagePreference,
    pub locale: Locale,
    pub catalog_version: &'static str,
}

pub fn resolve_system_locale(hint: Option<&str>) -> Locale {
    let lower = hint.unwrap_or_default().to_ascii_lowercase();
    if lower == "zh" || lower.starts_with("zh-") || lower.starts_with("zh_") {
        Locale::ZhCn
    } else {
        Locale::EnUs
    }
}

pub fn resolve_locale(preference: LanguagePreference, system_hint: Option<&str>) -> Locale {
    match preference {
        LanguagePreference::ZhCn => Locale::ZhCn,
        LanguagePreference::EnUs => Locale::EnUs,
        LanguagePreference::System => resolve_system_locale(system_hint),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_locale_only_resolves_to_supported_catalogs() {
        assert_eq!(resolve_system_locale(Some("zh-CN")), Locale::ZhCn);
        assert_eq!(resolve_system_locale(Some("zh-Hans")), Locale::ZhCn);
        assert_eq!(resolve_system_locale(Some("en-US")), Locale::EnUs);
        assert_eq!(resolve_system_locale(Some("ja-JP")), Locale::EnUs);
        assert_eq!(resolve_system_locale(None), Locale::EnUs);
    }

    #[test]
    fn preference_serializes_to_the_stable_wire_values() {
        assert_eq!(
            serde_json::to_string(&LanguagePreference::ZhCn).unwrap(),
            "\"zh-CN\""
        );
        assert_eq!(
            serde_json::to_string(&LanguagePreference::EnUs).unwrap(),
            "\"en-US\""
        );
        assert_eq!(
            LanguagePreference::parse("system"),
            Some(LanguagePreference::System)
        );
    }
}
