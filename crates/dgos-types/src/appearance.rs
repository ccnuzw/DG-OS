//! Appearance preferences (V1-FR-005 §10.1, D065).
//!
//! The two preferences are closed sets validated by the host. A renderer
//! patch arrives as raw JSON so every rejection goes through the command
//! envelope with a registered error code instead of failing inside Tauri's
//! argument decoding.

use crate::error::DgosError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Logical `system_settings` keys. Only these two are ever read or written.
pub const THEME_KEY: &str = "appearance.theme";
pub const DENSITY_KEY: &str = "appearance.density";

pub const DENSITIES: [u16; 5] = [75, 100, 125, 150, 175];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Light,
    Dark,
    Auto,
}

impl Theme {
    pub fn parse(value: &Value) -> Option<Self> {
        match value.as_str()? {
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            "auto" => Some(Theme::Auto),
            _ => None,
        }
    }
}

/// Interface scale in percent; always one of [`DENSITIES`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Density(u16);

impl Default for Density {
    fn default() -> Self {
        Density(100)
    }
}

impl Density {
    pub fn new(percent: u16) -> Option<Self> {
        DENSITIES.contains(&percent).then_some(Density(percent))
    }

    /// Only JSON integers qualify: `125.0`, `"125"` and `true` are not densities.
    pub fn parse(value: &Value) -> Option<Self> {
        Self::new(u16::try_from(value.as_u64()?).ok()?)
    }

    pub fn percent(self) -> u16 {
        self.0
    }
}

/// The complete, valid preferences returned to the renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AppearancePreferences {
    pub theme: Theme,
    pub density: Density,
}

/// A validated, non-empty update. Absent fields stay unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppearancePatch {
    theme: Option<Theme>,
    density: Option<Density>,
}

impl AppearancePatch {
    pub fn theme(&self) -> Option<Theme> {
        self.theme
    }

    pub fn density(&self) -> Option<Density> {
        self.density
    }

    /// Validate a renderer patch. `null` counts as absent. Rejections:
    /// 1005001 unknown theme, 1005006 unknown density, 1005002 not an
    /// object, an unknown key, or no field at all. Error text is fixed and
    /// never echoes the input.
    pub fn parse(raw: &Value) -> Result<Self, DgosError> {
        let object: &Map<String, Value> =
            raw.as_object().ok_or(DgosError::InvalidAppearancePatch)?;
        if object.keys().any(|key| key != "theme" && key != "density") {
            return Err(DgosError::InvalidAppearancePatch);
        }
        let theme = match object.get("theme") {
            None | Some(Value::Null) => None,
            Some(value) => Some(Theme::parse(value).ok_or(DgosError::ThemeNotFound)?),
        };
        let density = match object.get("density") {
            None | Some(Value::Null) => None,
            Some(value) => Some(Density::parse(value).ok_or(DgosError::InvalidDensity)?),
        };
        if theme.is_none() && density.is_none() {
            return Err(DgosError::InvalidAppearancePatch);
        }
        Ok(Self { theme, density })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn code(raw: Value) -> u32 {
        AppearancePatch::parse(&raw).unwrap_err().error_code()
    }

    #[test]
    fn accepts_exactly_the_closed_sets() {
        for theme in ["light", "dark", "auto"] {
            assert!(AppearancePatch::parse(&json!({ "theme": theme })).is_ok());
        }
        for density in DENSITIES {
            assert!(AppearancePatch::parse(&json!({ "density": density })).is_ok());
        }
    }

    #[test]
    fn rejects_values_outside_the_sets_with_their_own_codes() {
        for theme in [
            json!("Dark"),
            json!("sepia"),
            json!(""),
            json!(1),
            json!(true),
            json!([]),
        ] {
            assert_eq!(code(json!({ "theme": theme })), 1005001);
        }
        for density in [
            json!(0),
            json!(80),
            json!(200),
            json!(-100),
            json!(100.5),
            json!(125.0),
            json!("125"),
            json!(true),
            json!(70000),
        ] {
            assert_eq!(code(json!({ "density": density })), 1005006, "{density}");
        }
    }

    #[test]
    fn rejects_empty_unknown_and_non_object_patches() {
        for raw in [
            json!({}),
            json!({ "theme": null }),
            json!({ "theme": null, "density": null }),
            json!({ "theme": "dark", "font_size": 14 }),
            json!({ "setting_key": "appearance.theme", "theme": "dark" }),
            json!("dark"),
            json!(null),
            json!([{ "theme": "dark" }]),
        ] {
            assert_eq!(code(raw.clone()), 1005002, "{raw}");
        }
    }

    #[test]
    fn a_bad_field_rejects_the_whole_pair() {
        assert_eq!(code(json!({ "theme": "dark", "density": 90 })), 1005006);
        assert_eq!(code(json!({ "theme": "neon", "density": 100 })), 1005001);
    }

    #[test]
    fn error_text_never_echoes_input() {
        let message = AppearancePatch::parse(&json!({ "theme": "<script>x</script>" }))
            .unwrap_err()
            .to_string();
        assert!(!message.contains("script"), "{message}");
    }

    #[test]
    fn serializes_as_the_wire_contract() {
        let value = serde_json::to_value(AppearancePreferences::default()).unwrap();
        assert_eq!(value, json!({ "theme": "light", "density": 100 }));
        let dark = AppearancePreferences {
            theme: Theme::Dark,
            density: Density::new(175).unwrap(),
        };
        assert_eq!(
            serde_json::to_value(dark).unwrap(),
            json!({ "theme": "dark", "density": 175 })
        );
    }
}
