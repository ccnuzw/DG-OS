use super::*;
use crate::CliProviderProtocol::{AnthropicMessages, OpenAiResponses};
use serde_json::json;

const BASE: &str = "https://chat.example.invalid/v1";

#[test]
fn absent_routes_preserve_only_legacy_unverified_candidates() {
    for (provider_type, protocol, expected_base, auth) in [
        (PROTOCOL_OPENAI_COMPATIBLE, OpenAiResponses, BASE, AuthStyle::Bearer),
        (PROTOCOL_ANTHROPIC, AnthropicMessages, "https://chat.example.invalid", AuthStyle::Both),
    ] {
        let route = resolve_cli_provider_route(provider_type, BASE, None, protocol).unwrap();
        assert_eq!(route.base_url, expected_base);
        assert_eq!(route.auth_style, auth);
        assert!(!route.declared);
        let opposite = if protocol == OpenAiResponses { AnthropicMessages } else { OpenAiResponses };
        assert!(matches!(resolve_cli_provider_route(provider_type, BASE, None, opposite), Err(DgosError::CliProviderProtocolUnsupported)));
    }
}

#[test]
fn dual_routes_are_independent_of_chat_adapter_and_use_protocol_auth_defaults() {
    let config = json!({"cli_routes": {
        "responses": {"enabled": true, "base_url": "https://responses.example.invalid/v1/"},
        "messages": {"enabled": true, "base_url": "https://messages.example.invalid/v1/"}
    }});
    for provider_type in [PROTOCOL_OPENAI_COMPATIBLE, PROTOCOL_ANTHROPIC] {
        let responses = resolve_cli_provider_route(provider_type, BASE, Some(&config), OpenAiResponses).unwrap();
        let messages = resolve_cli_provider_route(provider_type, BASE, Some(&config), AnthropicMessages).unwrap();
        assert_eq!(responses.base_url, "https://responses.example.invalid/v1");
        assert_eq!(responses.auth_style, AuthStyle::Bearer);
        assert_eq!(responses.anthropic_version, None);
        assert_eq!(messages.base_url, "https://messages.example.invalid");
        assert_eq!(messages.auth_style, AuthStyle::Both);
        assert_eq!(messages.anthropic_version, ANTHROPIC_GENERIC.version_header);
        assert!(responses.declared && messages.declared);
    }
    let ordinary_override = json!({"auth_style": "x-api-key", "cli_routes": {
        "responses": {"enabled": true}, "messages": {"enabled": true}
    }});
    let route = resolve_cli_provider_route(PROTOCOL_ANTHROPIC, BASE, Some(&ordinary_override), OpenAiResponses).unwrap();
    assert_eq!(route.auth_style, AuthStyle::Bearer);
    let route = resolve_cli_provider_route(PROTOCOL_ANTHROPIC, BASE, Some(&ordinary_override), AnthropicMessages).unwrap();
    assert_eq!(route.auth_style, AuthStyle::XApiKey);
}

#[test]
fn disabled_routes_fail_closed_without_requiring_opposite_family_support() {
    let config = json!({"auth_style": "both", "cli_routes": {
        "responses": {"enabled": false}, "messages": {"enabled": false}
    }});
    let ordinary = validate_provider_config(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config)).unwrap();
    validate_cli_provider_routes(&ordinary, Some(&config)).unwrap();
    for protocol in [OpenAiResponses, AnthropicMessages] {
        assert!(matches!(resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config), protocol), Err(DgosError::CliProviderProtocolUnsupported)));
    }
}

#[test]
fn route_keys_and_shapes_are_strict_but_unrelated_config_is_preserved() {
    for routes in [
        Value::Null, json!([]), json!(true), json!({"chat": {"enabled": true}}),
        json!({"responses": null}), json!({"responses": {}}),
        json!({"responses": {"enabled": "true"}}),
        json!({"responses": {"enabled": true, "api_key": "not-allowed"}}),
        json!({"messages": {"enabled": false, "base_url": null}}),
        json!({"messages": {"enabled": true, "auth_style": false}}),
    ] {
        let config = json!({"cli_routes": routes});
        let error = resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config), OpenAiResponses).unwrap_err();
        assert!(matches!(error, DgosError::CliProviderRouteInvalid), "{config}");
    }
    let config = json!({"display_note": {"arbitrary": true}, "max_retries": 2, "cli_routes": {}});
    let original = config.clone();
    assert!(!resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config), OpenAiResponses).unwrap().declared);
    assert_eq!(config, original);
}

#[test]
fn responses_rejects_non_bearer_without_changing_ordinary_chat_auth() {
    for style in ["x-api-key", "both", "invalid"] {
        let config = json!({"cli_routes": {"responses": {"enabled": true, "auth_style": style}}});
        assert!(matches!(resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config), OpenAiResponses), Err(DgosError::CliProviderAuthUnsupported)));
    }
    let config = json!({"auth_style": "both"});
    let ordinary = validate_provider_config(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config)).unwrap();
    validate_cli_provider_routes(&ordinary, Some(&config)).unwrap();
    assert_eq!(ordinary.auth_override, Some(AuthStyle::Both));
    assert!(matches!(resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config), OpenAiResponses), Err(DgosError::CliProviderAuthUnsupported)));
    let config = json!({"auth_style": "both", "cli_routes": {"responses": {"enabled": true, "auth_style": "bearer"}}});
    assert_eq!(resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config), OpenAiResponses).unwrap().auth_style, AuthStyle::Bearer);
    for (style, expected) in [("bearer", AuthStyle::Bearer), ("x-api-key", AuthStyle::XApiKey), ("both", AuthStyle::Both)] {
        let config = json!({"cli_routes": {"messages": {"enabled": true, "auth_style": style}}});
        assert_eq!(resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config), AnthropicMessages).unwrap().auth_style, expected);
    }
}

#[test]
fn explicit_bases_reject_credential_routing_ambiguity_and_operation_urls() {
    for base in [
        "not a URL", "file:///tmp/route", "https://example.invalid/v1?token=synthetic",
        "https://example.invalid/v1#fragment", "https://user@example.invalid/v1",
        "https://user:synthetic@example.invalid/v1", "https://@example.invalid/v1",
        "https://example.invalid/v1/responses", "https://example.invalid/v1/responses/",
        "https://example.invalid/v1/chat/completions", "https://example.invalid/v1/messages",
        "https://example.invalid/v1/MESSAGES/", "http://public.example.invalid/v1",
    ] {
        for protocol in [OpenAiResponses, AnthropicMessages] {
            let key = if protocol == OpenAiResponses { "responses" } else { "messages" };
            let mut routes = Map::new();
            routes.insert(key.into(), json!({"enabled": true, "base_url": base}));
            let config = json!({"cli_routes": routes});
            assert!(matches!(resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config), protocol), Err(DgosError::CliProviderRouteInvalid)), "{base}");
        }
    }
}

#[test]
fn route_http_policy_reuses_provider_opt_in_and_local_address_rules() {
    for base in ["http://127.0.0.1:18888/v1/", "http://localhost:18888/v1/", "http://192.168.1.2/v1/"] {
        let config = json!({"cli_routes": {"responses": {"enabled": true, "base_url": base}}});
        let route = resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config), OpenAiResponses).unwrap();
        assert_eq!(route.base_url, base.trim_end_matches('/'));
        assert!(!route.allow_insecure_http);
    }
    let config = json!({"allow_insecure_http": true, "cli_routes": {"responses": {
        "enabled": true, "base_url": "http://public.example.invalid/v1/"
    }}});
    let route = resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config), OpenAiResponses).unwrap();
    assert_eq!(route.base_url, "http://public.example.invalid/v1");
    assert!(route.allow_insecure_http);
}

#[test]
fn inherited_endpoints_and_equivalent_declarations_have_identical_execution_settings() {
    let automatic = resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, None, OpenAiResponses).unwrap();
    for declaration in [json!({"enabled": true}), json!({"enabled": true, "base_url": "  "}),
        json!({"enabled": true, "base_url": " https://chat.example.invalid/v1/ ", "auth_style": "bearer"}),
        json!({"enabled": true, "base_url": "https://CHAT.EXAMPLE.INVALID:443/v1/"})] {
        let config = json!({"cli_routes": {"responses": declaration}});
        let explicit = resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config), OpenAiResponses).unwrap();
        assert!(explicit.declared);
        assert!(automatic.same_execution_settings(&explicit));
    }
    let config = json!({"cli_routes": {"messages": {"enabled": true, "base_url": "https://messages.example.invalid/v1/"}}});
    let with_v1 = resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config), AnthropicMessages).unwrap();
    let config = json!({"cli_routes": {"messages": {"enabled": true, "base_url": "https://messages.example.invalid/"}}});
    let without_v1 = resolve_cli_provider_route(PROTOCOL_OPENAI_COMPATIBLE, BASE, Some(&config), AnthropicMessages).unwrap();
    assert!(with_v1.same_execution_settings(&without_v1));
}

#[test]
fn official_workspace_metadata_does_not_leak_to_custom_cli_endpoints() {
    let config = json!({"workspace_id": "workspace-fixture", "cli_routes": {"messages": {
        "enabled": true, "base_url": "https://gateway.example.invalid/v1", "auth_style": "bearer"
    }}});
    let route = resolve_cli_provider_route(PROTOCOL_ANTHROPIC, "https://api.anthropic.com", Some(&config), AnthropicMessages).unwrap();
    assert_eq!(route.workspace_id, None);
    assert_eq!(route.anthropic_version, ANTHROPIC_GENERIC.version_header);
    let config = json!({"workspace_id": "workspace-fixture"});
    let route = resolve_cli_provider_route(PROTOCOL_ANTHROPIC, "https://api.anthropic.com", Some(&config), AnthropicMessages).unwrap();
    assert_eq!(route.workspace_id.as_deref(), Some("workspace-fixture"));
    assert_eq!(route.anthropic_version, crate::vendor::ANTHROPIC_OFFICIAL.version_header);
}
