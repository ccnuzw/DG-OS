//! Vendor table and compatibility profiles (V1-FR-001 §2.4.2, technical design "厂商表与兼容画像").
//!
//! A vendor is built-in data with one or more endpoints; an endpoint names the adapter
//! (`provider_type`), a default address, the official hosts and the official profile. A profile
//! describes only the *shape* of requests. It never carries a host, never relaxes the HTTPS policy
//! and never sees a credential.
//!
//! Profile facts below come from the vendors' public documentation pages read on 2026-10-29
//! (technical design "协议映射"、"推理映射"). No call to a real service was made, so every value
//! marked "inferred" still needs a real smoke run.

use crate::endpoint::check_endpoint;
use dgos_types::{
    DgosError, DgosResult, ProviderVendor, ProviderVendorEndpoint, ReasoningConfig,
    PROTOCOL_ANTHROPIC, PROTOCOL_OPENAI_COMPATIBLE,
};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStyle {
    Bearer,
    XApiKey,
    Both,
}

impl AuthStyle {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "bearer" => AuthStyle::Bearer,
            "x-api-key" => AuthStyle::XApiKey,
            "both" => AuthStyle::Both,
            _ => return None,
        })
    }

    pub fn sends_bearer(self) -> bool {
        matches!(self, AuthStyle::Bearer | AuthStyle::Both)
    }

    pub fn sends_x_api_key(self) -> bool {
        matches!(self, AuthStyle::XApiKey | AuthStyle::Both)
    }
}

/// Which sampling parameters the profile refuses before anything is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingRule {
    /// Sent as given; the upstream decides.
    Allowed,
    /// Refused while a reasoning level other than `none` is requested (official OpenAI).
    RejectedWithReasoning,
    /// Refused always (official Anthropic, `anthropic-version: 2026-10-21`).
    Rejected,
}

/// How a reasoning request is written into the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningMapping {
    /// `reasoning_effort: <effort>`, handed over unchanged (`openai_compatible`).
    OpenAiEffort,
    /// `thinking: {type: adaptive, display: summarized}` plus `output_config.effort`.
    AnthropicAdaptive,
    /// `thinking: {type: enabled, budget_tokens}`; `low`/`medium`/`high` map to fixed budgets.
    AnthropicBudget,
}

/// How a JSON Schema request is written into the body. The host validates the answer in every
/// case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredOutput {
    /// `response_format: {type: json_schema, json_schema: {...}}`.
    OpenAiJsonSchema,
    /// `output_config.format: {type: json_schema, schema}`.
    AnthropicNative,
    /// A system instruction carrying the schema; nothing in the request format changes.
    AnthropicInstruction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelList {
    /// `GET {base}/models`, one response.
    Single,
    /// `GET {base}/v1/models?limit&after_id`, `has_more` / `last_id`.
    AfterId,
}

/// Budgets for `AnthropicBudget` (inferred, not from the vendor documentation).
pub const BUDGET_LOW: u32 = 2048;
pub const BUDGET_MEDIUM: u32 = 8192;
pub const BUDGET_HIGH: u32 = 16384;

/// Used when neither the request nor the model row gives `max_tokens` (inferred; Anthropic
/// requires the field).
pub const ANTHROPIC_DEFAULT_MAX_TOKENS: u32 = 4096;

#[derive(Debug, PartialEq, Eq)]
pub struct Profile {
    pub name: &'static str,
    pub official: bool,
    pub provider_type: &'static str,
    pub auth: AuthStyle,
    /// `anthropic-version`; `None` for `openai_compatible`.
    pub version_header: Option<&'static str>,
    pub models: ModelList,
    /// `max_tokens` or `max_completion_tokens`.
    pub max_tokens_field: &'static str,
    pub send_stream_options: bool,
    pub send_tool_choice: bool,
    pub send_parallel_tool_calls: bool,
    pub sampling: SamplingRule,
    pub reasoning: ReasoningMapping,
    pub structured: StructuredOutput,
    /// Where the facts of an official profile were read, and when.
    pub source: &'static str,
}

const SOURCE_DESIGN: &str =
    "FR-001 技术设计「厂商表与兼容画像」，官方文档页面，2026-10-29 核对，未做真实调用";

pub static OPENAI_GENERIC: Profile = Profile {
    name: "openai_compatible/generic",
    official: false,
    provider_type: PROTOCOL_OPENAI_COMPATIBLE,
    auth: AuthStyle::Bearer,
    version_header: None,
    models: ModelList::Single,
    max_tokens_field: "max_tokens",
    send_stream_options: true,
    send_tool_choice: true,
    send_parallel_tool_calls: true,
    sampling: SamplingRule::Allowed,
    reasoning: ReasoningMapping::OpenAiEffort,
    structured: StructuredOutput::OpenAiJsonSchema,
    source: "通用画像：只发送协议标准字段",
};

pub static OPENAI_OFFICIAL: Profile = Profile {
    name: "openai_compatible/openai-official",
    official: true,
    max_tokens_field: "max_completion_tokens",
    sampling: SamplingRule::RejectedWithReasoning,
    source: SOURCE_DESIGN,
    ..OPENAI_GENERIC
};

pub static ANTHROPIC_GENERIC: Profile = Profile {
    name: "anthropic/generic",
    official: false,
    provider_type: PROTOCOL_ANTHROPIC,
    // Third-party gateways accept the two headers inconsistently; sending both covers the most
    // services (inferred, §2.4.6 #4).
    auth: AuthStyle::Both,
    version_header: Some("2023-06-01"),
    models: ModelList::AfterId,
    max_tokens_field: "max_tokens",
    send_stream_options: false,
    send_tool_choice: true,
    send_parallel_tool_calls: false,
    sampling: SamplingRule::Allowed,
    reasoning: ReasoningMapping::AnthropicBudget,
    structured: StructuredOutput::AnthropicInstruction,
    source: "通用画像：只发送协议标准字段（版本头 2023-06-01 为推断值）",
};

pub static ANTHROPIC_OFFICIAL: Profile = Profile {
    name: "anthropic/anthropic-official",
    official: true,
    auth: AuthStyle::Bearer,
    version_header: Some("2026-10-21"),
    sampling: SamplingRule::Rejected,
    reasoning: ReasoningMapping::AnthropicAdaptive,
    structured: StructuredOutput::AnthropicNative,
    source: SOURCE_DESIGN,
    ..ANTHROPIC_GENERIC
};

impl Profile {
    /// Local refusal of sampling parameters the profile declares unusable (B01-46).
    pub fn check_sampling(
        &self,
        temperature: Option<f32>,
        reasoning: Option<&ReasoningConfig>,
    ) -> DgosResult<()> {
        if temperature.is_none() {
            return Ok(());
        }
        match self.sampling {
            SamplingRule::Allowed => Ok(()),
            SamplingRule::Rejected => Err(DgosError::RequestNotSupported),
            SamplingRule::RejectedWithReasoning => {
                let active = reasoning
                    .and_then(|reasoning| reasoning.effort.as_deref())
                    .is_some_and(|effort| effort != "none");
                if active {
                    Err(DgosError::RequestNotSupported)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// The body fragment for a reasoning request, or `RequestNotSupported` when this profile has
    /// no mapping for it. Levels are never replaced by a nearby one.
    pub fn reasoning_params(&self, reasoning: &ReasoningConfig) -> DgosResult<ReasoningParams> {
        let effort = reasoning
            .effort
            .as_deref()
            .map(str::trim)
            .filter(|effort| !effort.is_empty());
        match self.reasoning {
            ReasoningMapping::OpenAiEffort => match effort {
                Some(effort) => Ok(ReasoningParams::OpenAiEffort(effort.to_owned())),
                None => Err(DgosError::RequestNotSupported),
            },
            ReasoningMapping::AnthropicAdaptive => match effort {
                Some(effort @ ("low" | "medium" | "high" | "xhigh" | "max")) => {
                    Ok(ReasoningParams::AnthropicAdaptive {
                        effort: effort.to_owned(),
                    })
                }
                _ => Err(DgosError::RequestNotSupported),
            },
            ReasoningMapping::AnthropicBudget => {
                let budget = match (reasoning.budget_tokens, effort) {
                    (Some(0), _) => return Err(DgosError::RequestNotSupported),
                    (Some(budget), _) => budget,
                    (None, Some("low")) => BUDGET_LOW,
                    (None, Some("medium")) => BUDGET_MEDIUM,
                    (None, Some("high")) => BUDGET_HIGH,
                    _ => return Err(DgosError::RequestNotSupported),
                };
                Ok(ReasoningParams::AnthropicBudget { budget })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasoningParams {
    OpenAiEffort(String),
    AnthropicAdaptive { effort: String },
    AnthropicBudget { budget: u32 },
}

impl ReasoningParams {
    /// The `thinking` object for the Anthropic variants.
    pub fn anthropic_thinking(&self) -> Option<Value> {
        match self {
            ReasoningParams::AnthropicAdaptive { .. } => {
                Some(json!({"type": "adaptive", "display": "summarized"}))
            }
            ReasoningParams::AnthropicBudget { budget } => {
                Some(json!({"type": "enabled", "budget_tokens": budget}))
            }
            ReasoningParams::OpenAiEffort(_) => None,
        }
    }
}

pub struct EndpointSpec {
    pub provider_type: &'static str,
    pub display_name: &'static str,
    pub default_base_url: Option<&'static str>,
    pub official_hosts: &'static [&'static str],
    pub official_profile: Option<&'static Profile>,
}

pub struct VendorSpec {
    pub vendor_id: &'static str,
    pub display_name: &'static str,
    pub endpoints: &'static [EndpointSpec],
}

pub const VENDOR_CUSTOM: &str = "custom";

/// Built-in vendors. A new vendor is a new row here; an adapter is added only for a new
/// transport protocol (ADR-0003).
pub static VENDORS: &[VendorSpec] = &[
    VendorSpec {
        vendor_id: "openai",
        display_name: "OpenAI",
        endpoints: &[EndpointSpec {
            provider_type: PROTOCOL_OPENAI_COMPATIBLE,
            display_name: "OpenAI 兼容",
            default_base_url: Some("https://api.openai.com/v1"),
            official_hosts: &["api.openai.com"],
            official_profile: Some(&OPENAI_OFFICIAL),
        }],
    },
    VendorSpec {
        vendor_id: "anthropic",
        display_name: "Anthropic",
        endpoints: &[EndpointSpec {
            provider_type: PROTOCOL_ANTHROPIC,
            display_name: "Anthropic 兼容",
            default_base_url: Some("https://api.anthropic.com"),
            official_hosts: &["api.anthropic.com"],
            official_profile: Some(&ANTHROPIC_OFFICIAL),
        }],
    },
    VendorSpec {
        vendor_id: VENDOR_CUSTOM,
        display_name: "自定义",
        endpoints: &[
            EndpointSpec {
                provider_type: PROTOCOL_OPENAI_COMPATIBLE,
                display_name: "OpenAI 兼容",
                default_base_url: None,
                official_hosts: &[],
                official_profile: None,
            },
            EndpointSpec {
                provider_type: PROTOCOL_ANTHROPIC,
                display_name: "Anthropic 兼容",
                default_base_url: None,
                official_hosts: &[],
                official_profile: None,
            },
        ],
    },
];

impl VendorSpec {
    pub fn endpoint(&self, provider_type: &str) -> Option<&EndpointSpec> {
        self.endpoints
            .iter()
            .find(|endpoint| endpoint.provider_type == provider_type)
    }
}

pub fn vendor(vendor_id: &str) -> Option<&'static VendorSpec> {
    VENDORS.iter().find(|vendor| vendor.vendor_id == vendor_id)
}

/// Public fields only; the profiles stay inside the host.
pub fn list_vendors() -> Vec<ProviderVendor> {
    VENDORS
        .iter()
        .map(|vendor| ProviderVendor {
            vendor_id: vendor.vendor_id.to_owned(),
            display_name: vendor.display_name.to_owned(),
            endpoints: vendor
                .endpoints
                .iter()
                .map(|endpoint| ProviderVendorEndpoint {
                    provider_type: endpoint.provider_type.to_owned(),
                    display_name: endpoint.display_name.to_owned(),
                    default_base_url: endpoint.default_base_url.map(str::to_owned),
                })
                .collect(),
        })
        .collect()
}

pub fn generic_profile(provider_type: &str) -> Option<&'static Profile> {
    match provider_type {
        PROTOCOL_OPENAI_COMPATIBLE => Some(&OPENAI_GENERIC),
        PROTOCOL_ANTHROPIC => Some(&ANTHROPIC_GENERIC),
        _ => None,
    }
}

/// The host of `base_url`, lower-cased, or `None` when it does not parse.
fn host_of(base_url: &str) -> Option<String> {
    reqwest::Url::parse(base_url.trim())
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

/// Whole-label match: the host is the official host, or a subdomain of it. The dot boundary keeps
/// `api.openai.com.evil.example` and `evilapi.openai.com.example` out.
fn is_official_host(host: &str, official: &str) -> bool {
    host == official
        || host
            .strip_suffix(official)
            .is_some_and(|rest| rest.ends_with('.'))
}

/// Vendor for a Provider saved before `vendor_id` existed: the vendor whose endpoint for this
/// adapter owns the host, otherwise `custom` (§2.4.2).
pub fn infer_vendor(provider_type: &str, base_url: &str) -> &'static str {
    let Some(host) = host_of(base_url) else {
        return VENDOR_CUSTOM;
    };
    VENDORS
        .iter()
        .find(|vendor| {
            vendor.endpoint(provider_type).is_some_and(|endpoint| {
                endpoint
                    .official_hosts
                    .iter()
                    .any(|official| is_official_host(&host, official))
            })
        })
        .map_or(VENDOR_CUSTOM, |vendor| vendor.vendor_id)
}

/// Official profile when the host belongs to the endpoint, otherwise the generic profile of the
/// adapter. `vendor_id` and `provider_type` must have been validated together.
pub fn resolve_profile(
    vendor_id: &str,
    provider_type: &str,
    base_url: &str,
) -> DgosResult<&'static Profile> {
    let endpoint = vendor(vendor_id)
        .and_then(|vendor| vendor.endpoint(provider_type))
        .ok_or(DgosError::UnsupportedProviderType)?;
    if let (Some(profile), Some(host)) = (endpoint.official_profile, host_of(base_url)) {
        if endpoint
            .official_hosts
            .iter()
            .any(|official| is_official_host(&host, official))
        {
            return Ok(profile);
        }
    }
    generic_profile(provider_type).ok_or(DgosError::UnsupportedProviderType)
}

/// A saved or submitted configuration after every save-time check.
#[derive(Debug, Clone)]
pub struct NormalizedConfig {
    pub vendor_id: &'static str,
    pub provider_type: &'static str,
    pub base_url: String,
    /// The user's override, when set; otherwise the profile decides.
    pub auth_override: Option<AuthStyle>,
    pub allow_insecure_http: bool,
    /// Sent only under an official Anthropic profile.
    pub workspace_id: Option<String>,
    pub profile: &'static Profile,
}

/// Anthropic base URLs are written without `/v1` (§2.4.6 #2); a pasted `/v1` suffix is removed.
pub fn normalize_anthropic_base(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_owned()
}

fn option_str<'a>(config: Option<&'a Value>, key: &str) -> DgosResult<Option<&'a str>> {
    match config.and_then(|config| config.get(key)) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(DgosError::RequestNotSupported),
    }
}

/// Save-time and call-time validation (B01-36, B01-37, B01-38). Order of failures:
/// unknown adapter / vendor / missing endpoint -> `unsupported_provider_type`; bad option value ->
/// `request_not_supported`; bad address -> `insecure_endpoint`. Keys of `config` that are not
/// advanced options (the retry settings) are left alone.
pub fn validate_provider_config(
    provider_type: &str,
    base_url: &str,
    config: Option<&Value>,
) -> DgosResult<NormalizedConfig> {
    let provider_type = match provider_type {
        PROTOCOL_OPENAI_COMPATIBLE => PROTOCOL_OPENAI_COMPATIBLE,
        PROTOCOL_ANTHROPIC => PROTOCOL_ANTHROPIC,
        _ => return Err(DgosError::UnsupportedProviderType),
    };
    let vendor_id = match config.and_then(|config| config.get("vendor_id")) {
        None | Some(Value::Null) => infer_vendor(provider_type, base_url),
        Some(Value::String(id)) => {
            let vendor = vendor(id).ok_or(DgosError::UnsupportedProviderType)?;
            vendor
                .endpoint(provider_type)
                .ok_or(DgosError::UnsupportedProviderType)?;
            vendor.vendor_id
        }
        Some(_) => return Err(DgosError::UnsupportedProviderType),
    };
    let auth_override = match option_str(config, "auth_style")? {
        None => None,
        Some(style) => Some(AuthStyle::parse(style).ok_or(DgosError::RequestNotSupported)?),
    };
    let allow_insecure_http = match config.and_then(|config| config.get("allow_insecure_http")) {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err(DgosError::RequestNotSupported),
    };
    let workspace_id = match option_str(config, "workspace_id")? {
        None | Some("") => None,
        Some(id) if id.len() <= 128 && id.bytes().all(|b| b.is_ascii_graphic()) => {
            Some(id.to_owned())
        }
        Some(_) => return Err(DgosError::RequestNotSupported),
    };
    let mut base_url = check_endpoint(base_url, allow_insecure_http)?;
    if provider_type == PROTOCOL_ANTHROPIC {
        base_url = normalize_anthropic_base(&base_url);
    }
    let profile = resolve_profile(vendor_id, provider_type, &base_url)?;
    Ok(NormalizedConfig {
        vendor_id,
        provider_type,
        base_url,
        auth_override,
        allow_insecure_http,
        workspace_id,
        profile,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(value: Value) -> Option<Value> {
        Some(value)
    }

    #[test]
    fn vendor_list_exposes_public_fields_only() {
        let vendors = list_vendors();
        let ids: Vec<_> = vendors.iter().map(|v| v.vendor_id.as_str()).collect();
        assert_eq!(ids, ["openai", "anthropic", "custom"]);
        let custom = &vendors[2];
        assert_eq!(custom.display_name, "自定义");
        assert_eq!(custom.endpoints.len(), 2);
        assert!(custom
            .endpoints
            .iter()
            .all(|e| e.default_base_url.is_none()));
        assert_eq!(
            vendors[0].endpoints[0].default_base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(
            vendors[1].endpoints[0].default_base_url.as_deref(),
            Some("https://api.anthropic.com")
        );
        // Profiles and official hosts are not part of the public shape.
        let json = serde_json::to_string(&vendors).unwrap();
        assert!(!json.contains("api.openai.com\""));
        assert!(!json.contains("official"));
    }

    #[test]
    fn profile_follows_the_host_by_whole_labels() {
        let profile =
            |vendor: &str, kind: &str, url: &str| resolve_profile(vendor, kind, url).unwrap().name;
        assert_eq!(
            profile("openai", "openai_compatible", "https://api.openai.com/v1"),
            "openai_compatible/openai-official"
        );
        assert_eq!(
            profile("openai", "openai_compatible", "https://API.OpenAI.com/v1"),
            "openai_compatible/openai-official"
        );
        assert_eq!(
            profile(
                "openai",
                "openai_compatible",
                "https://eu.api.openai.com/v1"
            ),
            "openai_compatible/openai-official"
        );
        for url in [
            "https://api.openai.com.evil.example/v1",
            "https://evilapi.openai.com/v1",
            "https://notapi.openai.com.evil.example/v1",
            "https://gateway.example/v1",
            "https://example.com/api.openai.com",
            "https://api.openai.com@evil.example/v1",
        ] {
            assert_eq!(
                profile("openai", "openai_compatible", url),
                "openai_compatible/generic",
                "{url}"
            );
        }
        assert_eq!(
            profile("anthropic", "anthropic", "https://api.anthropic.com"),
            "anthropic/anthropic-official"
        );
        assert_eq!(
            profile("anthropic", "anthropic", "https://proxy.example"),
            "anthropic/generic"
        );
        // `custom` has no official host, even when the address is one.
        assert_eq!(
            profile("custom", "anthropic", "https://api.anthropic.com"),
            "anthropic/generic"
        );
        assert_eq!(
            profile("custom", "openai_compatible", "https://api.openai.com/v1"),
            "openai_compatible/generic"
        );
        assert!(resolve_profile("anthropic", "openai_compatible", "https://x.example").is_err());
    }

    #[test]
    fn profile_does_not_choose_the_target_host() {
        // The profile is a pure function of the address; it cannot hand back another one.
        let normalized = validate_provider_config(
            "openai_compatible",
            "https://api.openai.com.evil.example/v1/",
            None,
        )
        .unwrap();
        assert_eq!(
            normalized.base_url,
            "https://api.openai.com.evil.example/v1"
        );
        assert!(!normalized.profile.official);
    }

    #[test]
    fn legacy_providers_without_vendor_id_are_inferred_from_the_host() {
        assert_eq!(
            infer_vendor("openai_compatible", "https://api.openai.com/v1"),
            "openai"
        );
        assert_eq!(
            infer_vendor("openai_compatible", "https://gateway.example/v1"),
            "custom"
        );
        assert_eq!(
            infer_vendor("anthropic", "https://api.anthropic.com"),
            "anthropic"
        );
        // Right host, wrong adapter: not that vendor's endpoint.
        assert_eq!(
            infer_vendor("anthropic", "https://api.openai.com/v1"),
            "custom"
        );
        assert_eq!(infer_vendor("openai_compatible", "not a url"), "custom");
        let legacy =
            validate_provider_config("openai_compatible", "https://api.openai.com/v1", None)
                .unwrap();
        assert_eq!(legacy.vendor_id, "openai");
        assert!(legacy.profile.official);
        let third_party =
            validate_provider_config("openai_compatible", "https://gateway.example/v1", None)
                .unwrap();
        assert_eq!(third_party.vendor_id, "custom");
        assert!(!third_party.profile.official);
    }

    #[test]
    fn vendor_and_option_validation_rejects_with_the_registered_classes() {
        let code = |provider_type: &str, url: &str, config: Option<Value>| {
            validate_provider_config(provider_type, url, config.as_ref())
                .map(|_| 0)
                .unwrap_or_else(|e| e.error_code())
        };
        let ok = "https://gateway.example/v1";
        assert_eq!(code("openai_compatible", ok, None), 0);
        assert_eq!(code("custom_proto", ok, None), 1001022);
        assert_eq!(
            code("openai_compatible", ok, cfg(json!({"vendor_id": "nope"}))),
            1001022
        );
        // The vendor exists but has no endpoint for this adapter.
        assert_eq!(
            code(
                "openai_compatible",
                ok,
                cfg(json!({"vendor_id": "anthropic"}))
            ),
            1001022
        );
        assert_eq!(
            code("anthropic", ok, cfg(json!({"vendor_id": "openai"}))),
            1001022
        );
        assert_eq!(
            code("openai_compatible", ok, cfg(json!({"vendor_id": 7}))),
            1001022
        );
        assert_eq!(
            code("anthropic", ok, cfg(json!({"vendor_id": "custom"}))),
            0
        );
        for bad in [
            json!({"auth_style": "basic"}),
            json!({"auth_style": 1}),
            json!({"allow_insecure_http": "yes"}),
            json!({"allow_insecure_http": 1}),
            json!({"workspace_id": "has space"}),
            json!({"workspace_id": 3}),
        ] {
            assert_eq!(
                code("openai_compatible", ok, cfg(bad.clone())),
                1001020,
                "{bad}"
            );
        }
        for good in [
            json!({"auth_style": "bearer"}),
            json!({"auth_style": "x-api-key"}),
            json!({"auth_style": "both"}),
            json!({"allow_insecure_http": false}),
            json!({"max_retries": 2, "initial_delay_ms": 5, "max_delay_ms": 9}),
        ] {
            assert_eq!(
                code("openai_compatible", ok, cfg(good.clone())),
                0,
                "{good}"
            );
        }
        assert_eq!(
            code("openai_compatible", "file:///etc/passwd", None),
            1001023
        );
        assert_eq!(
            code("openai_compatible", "http://gateway.example/v1", None),
            1001023
        );
        assert_eq!(
            code(
                "openai_compatible",
                "http://gateway.example/v1",
                cfg(json!({"allow_insecure_http": true}))
            ),
            0
        );
        // Unknown vendor is reported before the option check.
        assert_eq!(
            code(
                "openai_compatible",
                ok,
                cfg(json!({"vendor_id": "nope", "auth_style": "basic"}))
            ),
            1001022
        );
    }

    #[test]
    fn anthropic_base_url_loses_a_pasted_v1_suffix() {
        for (input, expected) in [
            ("https://api.anthropic.com/v1", "https://api.anthropic.com"),
            ("https://api.anthropic.com/v1/", "https://api.anthropic.com"),
            ("https://api.anthropic.com", "https://api.anthropic.com"),
            (
                "https://gw.example/anthropic/v1",
                "https://gw.example/anthropic",
            ),
            ("https://gw.example/v10", "https://gw.example/v10"),
        ] {
            let normalized = validate_provider_config("anthropic", input, None).unwrap();
            assert_eq!(normalized.base_url, expected, "{input}");
        }
        // `openai_compatible` keeps its /v1.
        let openai =
            validate_provider_config("openai_compatible", "https://gw.example/v1", None).unwrap();
        assert_eq!(openai.base_url, "https://gw.example/v1");
    }

    #[test]
    fn sampling_rules_follow_the_profile() {
        let reasoning = |effort: &str| ReasoningConfig {
            effort: Some(effort.into()),
            budget_tokens: None,
        };
        assert!(ANTHROPIC_OFFICIAL.check_sampling(Some(0.5), None).is_err());
        assert!(ANTHROPIC_OFFICIAL.check_sampling(None, None).is_ok());
        assert!(ANTHROPIC_GENERIC.check_sampling(Some(0.5), None).is_ok());
        assert!(OPENAI_GENERIC
            .check_sampling(Some(0.5), Some(&reasoning("high")))
            .is_ok());
        assert!(OPENAI_OFFICIAL.check_sampling(Some(0.5), None).is_ok());
        assert!(OPENAI_OFFICIAL
            .check_sampling(Some(0.5), Some(&reasoning("none")))
            .is_ok());
        assert!(OPENAI_OFFICIAL
            .check_sampling(Some(0.5), Some(&reasoning("low")))
            .is_err());
        assert!(OPENAI_OFFICIAL
            .check_sampling(None, Some(&reasoning("low")))
            .is_ok());
    }

    #[test]
    fn reasoning_mapping_per_profile() {
        let effort = |level: &str| ReasoningConfig {
            effort: Some(level.into()),
            budget_tokens: None,
        };
        assert_eq!(
            OPENAI_GENERIC.reasoning_params(&effort("xhigh")).unwrap(),
            ReasoningParams::OpenAiEffort("xhigh".into())
        );
        assert!(OPENAI_GENERIC
            .reasoning_params(&ReasoningConfig::default())
            .is_err());
        for (level, budget) in [("low", 2048), ("medium", 8192), ("high", 16384)] {
            assert_eq!(
                ANTHROPIC_GENERIC.reasoning_params(&effort(level)).unwrap(),
                ReasoningParams::AnthropicBudget { budget }
            );
        }
        for level in ["none", "minimal", "xhigh", "max", "bogus"] {
            assert!(
                ANTHROPIC_GENERIC.reasoning_params(&effort(level)).is_err(),
                "{level}"
            );
        }
        let explicit = ReasoningConfig {
            effort: Some("low".into()),
            budget_tokens: Some(3000),
        };
        assert_eq!(
            ANTHROPIC_GENERIC.reasoning_params(&explicit).unwrap(),
            ReasoningParams::AnthropicBudget { budget: 3000 }
        );
        for level in ["low", "medium", "high", "xhigh", "max"] {
            assert_eq!(
                ANTHROPIC_OFFICIAL.reasoning_params(&effort(level)).unwrap(),
                ReasoningParams::AnthropicAdaptive {
                    effort: level.into()
                }
            );
        }
        for level in ["none", "minimal", ""] {
            assert!(
                ANTHROPIC_OFFICIAL.reasoning_params(&effort(level)).is_err(),
                "{level:?}"
            );
        }
        // Official profile does not use a budget at all.
        assert!(ANTHROPIC_OFFICIAL
            .reasoning_params(&ReasoningConfig {
                effort: None,
                budget_tokens: Some(4096)
            })
            .is_err());
    }
}
