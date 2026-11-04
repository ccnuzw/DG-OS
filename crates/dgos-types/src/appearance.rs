//! Full appearance snapshots (D126). Identity and revision are host-owned.
use crate::{DgosError, DgosResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const SNAPSHOT_KEY: &str = "appearance.snapshot.v2";
pub const MAX_REVISION: u64 = 9_007_199_254_740_991;
pub const DENSITIES: [u16; 5] = [75, 100, 125, 150, 175];
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Typography {
    pub ui_font_id: String, pub reading_font_id: String, pub code_font_id: String,
    pub ui_size: u16, pub reading_size: u16, pub code_size: u16, pub line_height: f64,
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
            "schemaVersion":2,"mode":"light","density":100,
            "variants":{"light":{"presetId":"studio","overrides":{"interface":{},"reading":{},"code":{}}},"dark":{"presetId":"studio","overrides":{"interface":{},"reading":{},"code":{}}}},
            "typography":{"uiFontId":"system","readingFontId":"system","codeFontId":"mono","uiSize":12,"readingSize":13,"codeSize":12,"lineHeight":1.75},
            "effects":{"sidebarTreatment":"solid","reducedMotion":"system"}
        })).expect("valid built-in appearance")
    }
}
impl AppearanceValues {
    pub fn parse(raw: &Value) -> DgosResult<Self> {
        if raw.get("schemaVersion").and_then(Value::as_u64).is_some_and(|v| v > 2) { return Err(DgosError::AppearanceSchemaUnsupported); }
        let mut values: Self = serde_json::from_value(raw.clone()).map_err(|_| DgosError::InvalidAppearancePatch)?;
        if values.schema_version != 2 { return Err(DgosError::InvalidAppearancePatch); }
        if !["light", "dark", "auto"].contains(&values.mode.as_str()) { return Err(DgosError::ThemeNotFound); }
        if !DENSITIES.contains(&values.density) { return Err(DgosError::InvalidDensity); }
        let t = &values.typography;
        if !["system", "humanist"].contains(&t.ui_font_id.as_str()) || !["system", "humanist", "serif"].contains(&t.reading_font_id.as_str()) || !["mono", "system"].contains(&t.code_font_id.as_str())
            || ![12,13,14,16].contains(&t.ui_size) || ![13,14,16,18,20].contains(&t.reading_size) || ![12,13,14,16,18].contains(&t.code_size) || ![1.5,1.75,2.0].contains(&t.line_height)
            || !["solid","soft"].contains(&values.effects.sidebar_treatment.as_str()) || !["system","reduce"].contains(&values.effects.reduced_motion.as_str()) { return Err(DgosError::InvalidAppearancePatch); }
        for variant in [&mut values.variants.light, &mut values.variants.dark] {
            if !PRESETS.contains(&variant.preset_id.as_str()) { return Err(DgosError::ThemeNotFound); }
            for color in [&mut variant.overrides.interface.accent, &mut variant.overrides.interface.background, &mut variant.overrides.interface.foreground, &mut variant.overrides.reading.background, &mut variant.overrides.reading.foreground, &mut variant.overrides.code.background, &mut variant.overrides.code.foreground].into_iter().flatten() {
                if color.len() != 7 || !color.starts_with('#') || !color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit) { return Err(DgosError::InvalidAppearancePatch); }
                color.make_ascii_lowercase();
            }
        }
        for (mode, variant) in [("light", &values.variants.light), ("dark", &values.variants.dark)] {
            validate_colors(mode, variant)?;
        }
        Ok(values)
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
        for density in DENSITIES {
            let mut value = serde_json::to_value(AppearanceValues::default()).unwrap();
            value["density"] = json!(density);
            assert_eq!(AppearanceValues::parse(&value).unwrap().density, density);
            value["typography"]["codeSize"] = json!(10);
            assert!(AppearanceValues::parse(&value).is_err());
        }
    }
    #[test]
    fn revision_is_host_safe_integer_and_future_schema_is_protected() {
        let mut value = serde_json::to_value(AppearanceSnapshot { revision:0, values:AppearanceValues::default() }).unwrap();
        value["revision"] = json!(MAX_REVISION+1);
        assert!(AppearanceSnapshot::parse(&value).is_err());
        value["revision"] = json!(0);
        value["schemaVersion"] = json!(3);
        assert_eq!(AppearanceSnapshot::parse(&value).unwrap_err().error_code(),1005009);
    }
}
