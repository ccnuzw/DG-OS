//! Full appearance snapshots (D126). Identity and revision are host-owned.
use crate::{DgosError, DgosResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const SNAPSHOT_KEY: &str = "appearance.snapshot.v2";
pub const MAX_REVISION: u64 = 9_007_199_254_740_991;
pub const DENSITIES: [u16; 10] = [60, 70, 80, 90, 100, 110, 120, 130, 140, 150];
pub const PRESETS: [&str; 5] = ["precision", "studio", "paper", "forest", "slate"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppearanceValues {
    pub schema_version: u32,
    pub mode: String,
    pub variants: Variants,
    pub typography: Typography,
    pub density: u16,
    pub effects: Effects,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variants { pub light: Variant, pub dark: Variant }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Variant { pub preset_id: String, pub overrides: Overrides }
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overrides { pub interface: InterfaceColors, pub reading: ContentColors, pub code: ContentColors }
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterfaceColors {
    #[serde(skip_serializing_if = "Option::is_none")] pub accent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub foreground: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentColors {
    #[serde(skip_serializing_if = "Option::is_none")] pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub foreground: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum FontSelection {
    SystemUi,
    SystemMono,
    Local { family: String },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Typography {
    pub ui_font: FontSelection, pub reading_font: FontSelection, pub code_font: FontSelection,
    pub ui_size: u16, pub reading_size: u16, pub code_size: u16, pub line_height: f64,
    pub reading_follow_ui: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Effects { pub sidebar_treatment: String, pub reduced_motion: String }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppearanceSnapshot {
    pub revision: u64,
    #[serde(flatten)] pub values: AppearanceValues,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppearanceRead {
    #[serde(rename_all = "camelCase")]
    Ready { snapshot: AppearanceSnapshot, repair_warnings: Vec<String> },
    #[serde(rename_all = "camelCase")]
    Corrupt { safe_defaults: AppearanceValues, recovery_token: String },
    #[serde(rename_all = "camelCase")]
    UnsupportedSchema { safe_defaults: AppearanceValues, schema_version: u64 },
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppearanceSave {
    Saved { snapshot: AppearanceSnapshot },
    Conflict { code: u32, snapshot: AppearanceSnapshot },
}
impl Default for AppearanceValues {
    fn default() -> Self {
        serde_json::from_value(json!({
            "schemaVersion":6,"mode":"light","density":100,
            "variants":{"light":{"presetId":"studio","overrides":{"interface":{},"reading":{},"code":{}}},"dark":{"presetId":"studio","overrides":{"interface":{},"reading":{},"code":{}}}},
            "typography":{"uiFont":{"kind":"system-ui"},"readingFont":{"kind":"system-ui"},"codeFont":{"kind":"system-mono"},"uiSize":12,"readingSize":12,"codeSize":12,"lineHeight":1.75,"readingFollowUi":true},
            "effects":{"sidebarTreatment":"solid","reducedMotion":"system"}
        })).expect("valid built-in appearance")
    }
}
impl AppearanceValues {
    pub fn parse(raw: &Value) -> DgosResult<Self> {
        if raw.get("schemaVersion").and_then(Value::as_u64).is_some_and(|v| v > 6) { return Err(DgosError::AppearanceSchemaUnsupported); }
        let mut values: Self = serde_json::from_value(raw.clone()).map_err(|_| DgosError::InvalidAppearancePatch)?;
        if values.schema_version != 6 { return Err(DgosError::InvalidAppearancePatch); }
        if !["light", "dark", "auto"].contains(&values.mode.as_str()) { return Err(DgosError::ThemeNotFound); }
        if !DENSITIES.contains(&values.density) { return Err(DgosError::InvalidDensity); }
        let t = &values.typography;
        if !valid_font(&t.ui_font, false) || !valid_font(&t.reading_font, false) || !valid_font(&t.code_font, true)
            || !(12..=20).contains(&t.ui_size) || !(12..=24).contains(&t.reading_size) || !(12..=24).contains(&t.code_size) || ![1.5,1.75,2.0].contains(&t.line_height)
            || !["solid","soft"].contains(&values.effects.sidebar_treatment.as_str()) || !["system","reduce"].contains(&values.effects.reduced_motion.as_str()) { return Err(DgosError::InvalidAppearancePatch); }
        for variant in [&mut values.variants.light, &mut values.variants.dark] {
            if !PRESETS.contains(&variant.preset_id.as_str()) { return Err(DgosError::ThemeNotFound); }
            for color in [&mut variant.overrides.interface.accent, &mut variant.overrides.interface.background, &mut variant.overrides.interface.foreground, &mut variant.overrides.reading.background, &mut variant.overrides.reading.foreground, &mut variant.overrides.code.background, &mut variant.overrides.code.foreground].into_iter().flatten() {
                if color.len() != 7 || !color.starts_with('#') || !color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit) { return Err(DgosError::InvalidAppearancePatch); }
                color.make_ascii_lowercase();
            }
        }
        for (mode, variant) in [("light", &values.variants.light), ("dark", &values.variants.dark)] { validate_colors(mode, variant)?; }
        Ok(values)
    }
}
fn valid_font(font: &FontSelection, code: bool) -> bool {
    match font {
        FontSelection::SystemMono => code,
        FontSelection::SystemUi => !code,
        FontSelection::Local { family } => {
            let trimmed = family.trim();
            (1..=200).contains(&trimmed.chars().count()) && family == trimmed && !family.chars().any(char::is_control)
        }
    }
}
impl AppearanceSnapshot {
    pub fn parse(raw: &Value) -> DgosResult<Self> {
        let mut object = raw.as_object().cloned().ok_or(DgosError::InvalidAppearancePatch)?;
        let revision = object.remove("revision").and_then(|v| v.as_u64()).filter(|v| *v <= MAX_REVISION).ok_or(DgosError::InvalidAppearancePatch)?;
        Ok(Self { revision, values: AppearanceValues::parse(&Value::Object(object))? })
    }
}

fn validate_colors(mode: &str, variant: &Variant) -> DgosResult<()> {
    static SEEDS: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| serde_json::from_str(include_str!("../../../src/ui/tokens/presets.json")).expect("built-in seeds"));
    let seed = &SEEDS[&variant.preset_id][mode];
    let color = |value: &str| -> [f64; 3] {
        [1, 3, 5].map(|offset| u8::from_str_radix(&value[offset..offset+2], 16).expect("validated color") as f64)
    };
    let background = color(variant.overrides.interface.background.as_deref().unwrap_or(seed["background"].as_str().unwrap()));
    let foreground = color(variant.overrides.interface.foreground.as_deref().unwrap_or(seed["foreground"].as_str().unwrap()));
    let accent = color(variant.overrides.interface.accent.as_deref().unwrap_or(seed["accent"].as_str().unwrap()));
    let luminance = |rgb: [f64; 3]| {
        let c = rgb.map(|n| { let n=n/255.0; if n<=0.04045 {n/12.92} else {((n+0.055)/1.055).powf(2.4)} });
        c[0]*0.2126+c[1]*0.7152+c[2]*0.0722
    };
    let contrast = |a: [f64; 3], b: [f64; 3]| { let a=luminance(a); let b=luminance(b); (a.max(b)+0.05)/(a.min(b)+0.05) };
    for amount in [0.0, 0.035, 0.055, 0.075] {
        let surface = std::array::from_fn(|i| (background[i]+(foreground[i]-background[i])*amount).round());
        if contrast(foreground,surface)<4.5 || contrast(accent,surface)<3.0 { return Err(DgosError::InvalidAppearancePatch); }
    }
    for scope in [&variant.overrides.reading, &variant.overrides.code] {
        let bg=scope.background.as_deref().map(color).unwrap_or(background);
        let fg=scope.foreground.as_deref().map(color).unwrap_or(foreground);
        if contrast(bg,fg)<4.5 { return Err(DgosError::InvalidAppearancePatch); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_fields_and_css_injection_and_incomplete_values() {
        let mut value = serde_json::to_value(AppearanceValues::default()).unwrap();
        value["variants"]["light"]["overrides"]["interface"]["accent"] = json!("url(secret)");
        assert!(AppearanceValues::parse(&value).is_err());
        value["variants"]["light"]["overrides"]["interface"] = json!({});
        value["userId"] = json!("other");
        assert!(AppearanceValues::parse(&value).is_err());
        assert!(AppearanceValues::parse(&json!({"mode":"dark"})).is_err());
    }
    #[test]
    fn validates_typography_and_preserves_every_density() {
        assert_eq!(DENSITIES, [60, 70, 80, 90, 100, 110, 120, 130, 140, 150]);
        for density in DENSITIES {
            let mut value = serde_json::to_value(AppearanceValues::default()).unwrap();
            value["density"] = json!(density);
            assert_eq!(AppearanceValues::parse(&value).unwrap().density, density);
            value["typography"]["codeSize"] = json!(10);
            assert!(AppearanceValues::parse(&value).is_err());
        }
    }
    #[test]
    fn requires_complete_schema6_typography_and_integer_ranges() {
        let mut value = serde_json::to_value(AppearanceValues::default()).unwrap();
        value["schemaVersion"] = json!(3);
        assert!(AppearanceValues::parse(&value).is_err());
        value["schemaVersion"] = json!(6);
        value["typography"].as_object_mut().unwrap().remove("readingFollowUi");
        assert!(AppearanceValues::parse(&value).is_err());
        value["typography"]["readingFollowUi"] = json!(true);
        value["typography"]["uiSize"] = json!(12.5);
        assert!(AppearanceValues::parse(&value).is_err());
        value["typography"]["uiSize"] = json!(21);
        assert!(AppearanceValues::parse(&value).is_err());
        value["typography"]["uiSize"] = json!(12);
        value["typography"]["unknown"] = json!(1);
        assert!(AppearanceValues::parse(&value).is_err());
    }
    #[test]
    fn validates_font_role_and_local_family_constraints() {
        let mut value = serde_json::to_value(AppearanceValues::default()).unwrap();
        value["typography"]["uiFont"] = json!({"kind":"system-mono"});
        assert!(AppearanceValues::parse(&value).is_err());
        value["typography"]["uiFont"] = json!({"kind":"local","family":"  Inter"});
        assert!(AppearanceValues::parse(&value).is_err());
        value["typography"]["uiFont"] = json!({"kind":"local","family":"Inter\n"});
        assert!(AppearanceValues::parse(&value).is_err());
        value["typography"]["uiFont"] = json!({"kind":"local","family":"x".repeat(201)});
        assert!(AppearanceValues::parse(&value).is_err());
        value["typography"]["uiFont"] = json!({"kind":"local","family":"Noto Sans 日本語"});
        assert!(AppearanceValues::parse(&value).is_ok());
        value["typography"]["codeFont"] = json!({"kind":"system-ui"});
        assert!(AppearanceValues::parse(&value).is_err());
    }
    #[test]
    fn rejects_unknown_font_selection_variants_and_fields() {
        let mut value = serde_json::to_value(AppearanceValues::default()).unwrap();
        value["typography"]["uiFont"] = json!({"kind":"local","family":"Inter","extra":true});
        assert!(AppearanceValues::parse(&value).is_err());
        value["typography"]["uiFont"] = json!({"kind":"bundled"});
        assert!(AppearanceValues::parse(&value).is_err());
    }
    #[test]
    fn strict_writes_reject_legacy_schemas_and_density_values() {
        for schema in [2, 3, 4, 5] {
            let mut value = serde_json::to_value(AppearanceValues::default()).unwrap();
            value["schemaVersion"] = json!(schema);
            assert!(AppearanceValues::parse(&value).is_err());
        }
        for density in [0, 59, 61, 75, 95, 115, 125, 151, 175] {
            let mut value = serde_json::to_value(AppearanceValues::default()).unwrap();
            value["density"] = json!(density);
            assert_eq!(AppearanceValues::parse(&value).unwrap_err().error_code(), DgosError::InvalidDensity.error_code());
        }
        for density in [json!(60.5), json!("100"), Value::Null] {
            let mut value = serde_json::to_value(AppearanceValues::default()).unwrap();
            value["density"] = density;
            assert!(AppearanceValues::parse(&value).is_err());
        }
    }
    #[test]
    fn revision_is_host_safe_integer_and_future_schema_is_protected() {
        let mut value = serde_json::to_value(AppearanceSnapshot { revision:0, values:AppearanceValues::default() }).unwrap();
        value["revision"] = json!(MAX_REVISION+1);
        assert!(AppearanceSnapshot::parse(&value).is_err());
        value["revision"] = json!(0);
        value["schemaVersion"] = json!(7);
        assert_eq!(AppearanceSnapshot::parse(&value).unwrap_err().error_code(),1005009);
    }
}
