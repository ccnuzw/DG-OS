//! Explicit, unverified CLI protocol routes, independent of the ordinary Chat adapter.
use crate::endpoint::check_endpoint;
use crate::vendor::{
    infer_vendor, resolve_profile, validate_provider_config,
    NormalizedConfig, ANTHROPIC_GENERIC, OPENAI_GENERIC,
};
use crate::{AuthStyle, CliProviderProtocol};
use dgos_types::{DgosError, DgosResult, PROTOCOL_ANTHROPIC, PROTOCOL_OPENAI_COMPATIBLE};
use serde_json::{Map, Value};

/// Non-secret effective CLI settings. A declaration is user authorization, not a protocol test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliProviderRoute {
    pub base_url: String,
    pub auth_style: AuthStyle,
    pub allow_insecure_http: bool,
    pub anthropic_version: Option<&'static str>,
    pub workspace_id: Option<String>,
    pub declared: bool,
}

impl CliProviderRoute {
    pub(crate) fn same_execution_settings(&self, other: &Self) -> bool {
        self.base_url == other.base_url && self.auth_style == other.auth_style
            && self.allow_insecure_http == other.allow_insecure_http
            && self.anthropic_version == other.anthropic_version
            && self.workspace_id == other.workspace_id
    }
}

#[derive(Clone, Copy)]
struct RouteDeclaration<'a> {
    enabled: bool,
    base_url: Option<&'a str>,
    auth_style: Option<AuthStyle>,
}

fn declarations(config: Option<&Value>) -> DgosResult<[Option<RouteDeclaration<'_>>; 2]> {
    let Some(routes) = config.and_then(|config| config.get("cli_routes")) else {
        return Ok([None, None]);
    };
    let routes = routes.as_object().ok_or(DgosError::CliProviderRouteInvalid)?;
    if routes.keys().any(|key| key != "responses" && key != "messages") {
        return Err(DgosError::CliProviderRouteInvalid);
    }
    Ok([declaration(routes, "responses")?, declaration(routes, "messages")?])
}

fn declaration<'a>(routes: &'a Map<String, Value>, key: &str) -> DgosResult<Option<RouteDeclaration<'a>>> {
    let Some(route) = routes.get(key) else { return Ok(None); };
    let route = route.as_object().ok_or(DgosError::CliProviderRouteInvalid)?;
    if route.keys().any(|key| !matches!(key.as_str(), "enabled" | "base_url" | "auth_style")) {
        return Err(DgosError::CliProviderRouteInvalid);
    }
    let enabled = route.get("enabled").and_then(Value::as_bool)
        .ok_or(DgosError::CliProviderRouteInvalid)?;
    let base_url = match route.get("base_url") {
        None => None,
        Some(Value::String(base)) => Some(base.as_str()),
        Some(_) => return Err(DgosError::CliProviderRouteInvalid),
    };
    let auth_style = match route.get("auth_style") {
        None => None,
        Some(Value::String(style)) => Some(AuthStyle::parse(style).ok_or(DgosError::CliProviderAuthUnsupported)?),
        Some(_) => return Err(DgosError::CliProviderRouteInvalid),
    };
    Ok(Some(RouteDeclaration { enabled, base_url, auth_style }))
}

fn protocol_index(protocol: CliProviderProtocol) -> usize {
    match protocol {
        CliProviderProtocol::OpenAiResponses => 0,
        CliProviderProtocol::AnthropicMessages => 1,
    }
}

fn protocol_provider_type(protocol: CliProviderProtocol) -> &'static str {
    match protocol {
        CliProviderProtocol::OpenAiResponses => PROTOCOL_OPENAI_COMPATIBLE,
        CliProviderProtocol::AnthropicMessages => PROTOCOL_ANTHROPIC,
    }
}

fn explicit_base(base: &str, allow_insecure_http: bool) -> DgosResult<String> {
    let url = reqwest::Url::parse(base.trim()).map_err(|_| DgosError::CliProviderRouteInvalid)?;
    let path = url.path().trim_end_matches('/');
    let authority = base.trim().split_once("://").map(|(_, rest)| {
        rest.trim_start_matches(['/', '\\']).split(['/', '\\', '?', '#']).next().unwrap_or_default()
    });
    if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty()
        || url.password().is_some() || authority.is_some_and(|authority| authority.contains('@'))
        || ["/responses", "/chat/completions", "/messages"].iter().any(|operation| {
            path.get(path.len().saturating_sub(operation.len())..).is_some_and(|suffix| suffix.eq_ignore_ascii_case(operation))
        })
    {
        return Err(DgosError::CliProviderRouteInvalid);
    }
    check_endpoint(url.as_str(), allow_insecure_http).map_err(|_| DgosError::CliProviderRouteInvalid)
}

fn resolve_route(
    ordinary: &NormalizedConfig,
    declaration: Option<RouteDeclaration<'_>>,
    protocol: CliProviderProtocol,
) -> DgosResult<CliProviderRoute> {
    let same_family = ordinary.provider_type == protocol_provider_type(protocol);
    match declaration {
        Some(route) if !route.enabled => return Err(DgosError::CliProviderProtocolUnsupported),
        None if !same_family => return Err(DgosError::CliProviderProtocolUnsupported),
        _ => {}
    }
    let override_base = declaration.and_then(|route| route.base_url).map(str::trim).filter(|base| !base.is_empty());
    let base = override_base.unwrap_or(&ordinary.base_url);
    let mut base_url = if declaration.is_some() {
        explicit_base(base, ordinary.allow_insecure_http)?
    } else {
        let url = reqwest::Url::parse(base).map_err(|_| DgosError::InsecureEndpoint)?;
        url.as_str().trim_end_matches('/').to_owned()
    };
    if protocol == CliProviderProtocol::AnthropicMessages && base_url.ends_with("/v1") {
        base_url.truncate(base_url.len() - 3);
    }
    let inherits_profile = same_family && (override_base.is_none() || base_url == ordinary.base_url
        || reqwest::Url::parse(&ordinary.base_url).is_ok_and(|url| url.as_str().trim_end_matches('/') == base_url));
    let profile = if inherits_profile {
        ordinary.profile
    } else {
        let provider_type = protocol_provider_type(protocol);
        resolve_profile(infer_vendor(provider_type, &base_url), provider_type, &base_url)?
    };
    let auth_style = declaration.and_then(|route| route.auth_style).unwrap_or_else(|| {
        if same_family {
            ordinary.auth_override.unwrap_or(ordinary.profile.auth)
        } else {
            // Opposite-family routes never inherit the ordinary adapter's auth convention.
            match protocol {
                CliProviderProtocol::OpenAiResponses => OPENAI_GENERIC.auth,
                CliProviderProtocol::AnthropicMessages => ANTHROPIC_GENERIC.auth,
            }
        }
    });
    if protocol == CliProviderProtocol::OpenAiResponses && auth_style != AuthStyle::Bearer {
        return Err(DgosError::CliProviderAuthUnsupported);
    }
    Ok(CliProviderRoute {
        base_url,
        auth_style,
        allow_insecure_http: ordinary.allow_insecure_http,
        anthropic_version: profile.version_header,
        workspace_id: if profile.official && protocol == CliProviderProtocol::AnthropicMessages {
            ordinary.workspace_id.clone()
        } else { None },
        declared: declaration.is_some(),
    })
}

/// Resolve the exact requested protocol without network probes or fallback to another route.
/// Absent declarations retain only the ordinary adapter's legacy, unverified candidate.
/// Explicit endpoint overrides authorize the same Provider credential at that endpoint; they
/// never change the Provider's credential-vault scope or ordinary Chat configuration.
pub fn resolve_cli_provider_route(
    provider_type: &str,
    base_url: &str,
    config: Option<&Value>,
    protocol: CliProviderProtocol,
) -> DgosResult<CliProviderRoute> {
    let ordinary = validate_provider_config(provider_type, base_url, config)?;
    let declarations = declarations(config)?;
    // Reject invalid enabled declarations consistently at save, catalog and launch time.
    for candidate in [CliProviderProtocol::OpenAiResponses, CliProviderProtocol::AnthropicMessages] {
        if candidate != protocol && declarations[protocol_index(candidate)].is_some_and(|route| route.enabled) {
            resolve_route(&ordinary, declarations[protocol_index(candidate)], candidate)?;
        }
    }
    resolve_route(&ordinary, declarations[protocol_index(protocol)], protocol)
}

/// Validate only explicit routes at save time. Legacy Chat auth settings remain valid even
/// when the corresponding CLI cannot use them. Disabled opposite-family routes are valid.
pub(crate) fn validate_cli_provider_routes(ordinary: &NormalizedConfig, config: Option<&Value>) -> DgosResult<()> {
    let declarations = declarations(config)?;
    for protocol in [CliProviderProtocol::OpenAiResponses, CliProviderProtocol::AnthropicMessages] {
        if declarations[protocol_index(protocol)].is_some_and(|route| route.enabled) {
            resolve_route(ordinary, declarations[protocol_index(protocol)], protocol)?;
        }
    }
    Ok(())
}

pub(crate) fn execution_settings_changed(
    before: &NormalizedConfig,
    before_config: Option<&Value>,
    after: &NormalizedConfig,
    after_config: Option<&Value>,
) -> bool {
    let Ok(before_routes) = declarations(before_config) else { return true; };
    let Ok(after_routes) = declarations(after_config) else { return true; };
    [CliProviderProtocol::OpenAiResponses, CliProviderProtocol::AnthropicMessages].into_iter().any(|protocol| {
        match (resolve_route(before, before_routes[protocol_index(protocol)], protocol),
            resolve_route(after, after_routes[protocol_index(protocol)], protocol)) {
            (Ok(before), Ok(after)) => !before.same_execution_settings(&after),
            (Err(DgosError::CliProviderProtocolUnsupported), Err(DgosError::CliProviderProtocolUnsupported))
                | (Err(DgosError::CliProviderAuthUnsupported), Err(DgosError::CliProviderAuthUnsupported)) => false,
            _ => true,
        }
    })
}

#[cfg(test)]
#[path = "cli_route_tests.rs"]
mod tests;
