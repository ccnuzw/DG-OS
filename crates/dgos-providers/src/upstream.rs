//! Shared HTTP plumbing for upstream calls: size-bounded reads, error classification (B01-54) and
//! the one-shot parameter downgrade (B01-71).
//!
//! Upstream error bodies are read only to pick a category. Nothing from them is copied into an
//! error value, a log line or a usage record.

use crate::retry::AttemptError;
use dgos_types::{DgosError, DgosResult};
use reqwest::header::HeaderMap;
use serde_json::Value;
use std::time::Duration;

/// §2.4.6 #3: model lists and synchronous responses.
pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
/// §2.4.6 #3: one SSE event.
pub const MAX_EVENT_BYTES: usize = 1024 * 1024;
/// Tool-call responses keep the limit they had before this change.
pub const MAX_TOOL_BODY_BYTES: usize = 1024 * 1024;
/// Only the category fields of an error body are looked at.
const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;

/// Seconds form of `retry-after`; the HTTP-date form is ignored.
pub fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// Reads a whole body, failing with `ProviderResponseInvalid` once it exceeds `limit`.
pub async fn read_limited(mut response: reqwest::Response, limit: usize) -> DgosResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(DgosError::ProviderResponseInvalid);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > limit {
            return Err(DgosError::ProviderResponseInvalid);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Best-effort read of an error body; a failure just yields what was read so far.
pub async fn read_error_body(mut response: reqwest::Response) -> Vec<u8> {
    let mut body = Vec::new();
    while body.len() < MAX_ERROR_BODY_BYTES {
        match response.chunk().await {
            Ok(Some(chunk)) => body.extend_from_slice(&chunk),
            _ => break,
        }
    }
    body.truncate(MAX_ERROR_BODY_BYTES);
    body
}

#[derive(Debug, Default)]
pub struct ErrorInfo {
    pub code: String,
    pub kind: String,
    pub param: String,
    pub message: String,
}

/// Accepts `{"error": {...}}` and a bare `{...}` error object. Values are lower-cased for matching.
pub fn parse_error_info(body: &[u8]) -> ErrorInfo {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return ErrorInfo::default();
    };
    let object = match value.get("error") {
        Some(inner) if inner.is_object() => inner,
        _ => &value,
    };
    let text = |key: &str| match object.get(key) {
        Some(Value::String(text)) => text.to_lowercase(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    };
    ErrorInfo {
        code: text("code"),
        kind: text("type"),
        param: text("param"),
        message: text("message"),
    }
}

/// OpenAI `insufficient_quota`, `credit_balance_exhausted`, `*_spend_limit_exceeded`, and
/// Anthropic `enforced_spend_limit_reached` (§8.1).
fn is_quota_exhausted(info: &ErrorInfo) -> bool {
    [&info.code, &info.kind].into_iter().any(|name| {
        matches!(
            name.as_str(),
            "insufficient_quota"
                | "credit_balance_exhausted"
                | "enforced_spend_limit_reached"
                | "billing_hard_limit_reached"
        ) || name.ends_with("_spend_limit_exceeded")
    })
}

/// Heuristic (technical design B01-54): the exact OpenAI spelling is unconfirmed, so both the
/// documented code and the common message wording are accepted. Anthropic reports it as an
/// `invalid_request_error` whose message starts with "prompt is too long".
fn is_context_length_exceeded(info: &ErrorInfo) -> bool {
    info.code == "context_length_exceeded"
        || info.kind == "context_length_exceeded"
        || info.message.contains("maximum context length")
        || info.message.contains("context length")
        || info.message.contains("context window")
        || info.message.starts_with("prompt is too long")
}

/// B01-54 mapping from status to error class. `model_id` is the caller's own request value.
pub fn classify_status(
    status: u16,
    body: &[u8],
    model_id: &str,
    retry_after: Option<Duration>,
) -> AttemptError {
    let info = parse_error_info(body);
    let error = match status {
        401 => DgosError::InvalidApiKey,
        402 => DgosError::ProviderQuotaExhausted,
        403 => DgosError::ProviderPermissionDenied,
        404 => DgosError::ModelNotFound(model_id.to_owned()),
        400 | 413 if is_context_length_exceeded(&info) => DgosError::ContextLengthExceeded,
        400 | 409 | 413 | 422 => DgosError::ProviderRequestRejected,
        408 | 504 => DgosError::Timeout,
        429 if is_quota_exhausted(&info) => DgosError::ProviderQuotaExhausted,
        429 => DgosError::RateLimitExceeded,
        500..=503 | 505..=599 => DgosError::UpstreamServerError,
        300..=399 => DgosError::ProviderResponseInvalid,
        other => DgosError::HttpError(other),
    };
    AttemptError::new(error).with_retry_after(retry_after)
}

/// Parameters `openai_compatible` may drop once when the upstream rejects them with a 400.
pub const OPENAI_DOWNGRADABLE: &[&str] = &[
    "stream_options",
    "parallel_tool_calls",
    "tool_choice",
    "max_tokens",
    "max_completion_tokens",
];

/// `anthropic` has no `stream_options`; `max_tokens` is required, so it is never dropped or
/// swapped. Parallelism is part of `tool_choice`.
pub const ANTHROPIC_DOWNGRADABLE: &[&str] = &["tool_choice"];

/// The allowed parameter that a 400 error names *and* that the request actually carries.
pub fn named_downgrade(
    error_body: &[u8],
    request: &Value,
    allowed: &[&'static str],
) -> Option<&'static str> {
    let info = parse_error_info(error_body);
    let haystack = format!("{} {} {}", info.param, info.code, info.message);
    allowed
        .iter()
        .copied()
        .find(|name| request.get(*name).is_some() && haystack.contains(name))
}

/// `max_tokens` and `max_completion_tokens` trade places; every other parameter is removed.
pub fn apply_downgrade(request: &mut Value, param: &str) {
    let Some(object) = request.as_object_mut() else {
        return;
    };
    let Some(value) = object.remove(param) else {
        return;
    };
    match param {
        "max_tokens" => {
            object.insert("max_completion_tokens".into(), value);
        }
        "max_completion_tokens" => {
            object.insert("max_tokens".into(), value);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn code(status: u16, body: &str) -> u32 {
        classify_status(status, body.as_bytes(), "m", None)
            .error
            .error_code()
    }

    #[test]
    fn classifies_statuses_into_the_registered_codes() {
        assert_eq!(code(401, ""), 1001002);
        assert_eq!(code(402, ""), 1001024);
        assert_eq!(code(403, ""), 1001019);
        assert_eq!(code(404, ""), 1001004);
        assert_eq!(code(400, r#"{"error":{"message":"bad"}}"#), 1001017);
        assert_eq!(code(413, ""), 1001017);
        assert_eq!(code(409, ""), 1001017);
        assert_eq!(code(422, ""), 1001017);
        assert_eq!(code(408, ""), 1001006);
        assert_eq!(code(504, ""), 1001006);
        assert_eq!(code(429, ""), 1001005);
        for status in [500, 502, 503, 505, 529, 599] {
            assert_eq!(code(status, ""), 1001014, "{status}");
        }
        for status in [301, 302, 307, 308] {
            assert_eq!(code(status, ""), 1001025, "{status}");
        }
        assert_eq!(code(418, ""), 1000000);
    }

    #[test]
    fn recognises_context_length_by_code_type_or_wording() {
        for body in [
            r#"{"error":{"code":"context_length_exceeded","message":"x"}}"#,
            r#"{"error":{"type":"context_length_exceeded"}}"#,
            r#"{"error":{"message":"This model's maximum context length is 8192 tokens."}}"#,
        ] {
            assert_eq!(code(400, body), 1001018, "{body}");
            assert_eq!(code(413, body), 1001018, "{body}");
        }
        assert_eq!(
            code(400, r#"{"error":{"message":"unknown field"}}"#),
            1001017
        );
        // Not JSON, wrong shape, or a status that never means context length.
        assert_eq!(code(400, "<html>context length</html>"), 1001017);
        assert_eq!(
            code(500, r#"{"error":{"code":"context_length_exceeded"}}"#),
            1001014
        );
    }

    #[test]
    fn splits_429_into_quota_exhaustion_and_rate_limiting() {
        for body in [
            r#"{"error":{"code":"insufficient_quota"}}"#,
            r#"{"error":{"type":"credit_balance_exhausted"}}"#,
            r#"{"error":{"code":"monthly_spend_limit_exceeded"}}"#,
            r#"{"type":"error","error":{"type":"enforced_spend_limit_reached"}}"#,
        ] {
            assert_eq!(code(429, body), 1001024, "{body}");
        }
        assert_eq!(
            code(429, r#"{"error":{"code":"rate_limit_exceeded"}}"#),
            1001005
        );
        assert_eq!(code(429, "not json"), 1001005);
    }

    #[test]
    fn carries_retry_after_without_changing_the_class() {
        let failure = classify_status(429, b"", "m", Some(Duration::from_secs(7)));
        assert_eq!(failure.retry_after, Some(Duration::from_secs(7)));
        assert!(!failure.terminal);
    }

    #[test]
    fn parses_retry_after_seconds_only() {
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, " 12 ".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_secs(12)));
        headers.insert(
            reqwest::header::RETRY_AFTER,
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after(&headers), None);
        assert_eq!(retry_after(&HeaderMap::new()), None);
    }

    #[test]
    fn names_a_downgrade_only_for_allowed_parameters_the_request_carries() {
        let request = json!({"model": "m", "stream": true, "stream_options": {"include_usage": true},
            "tool_choice": "auto", "max_tokens": 16});
        let named = |body: &str| named_downgrade(body.as_bytes(), &request, OPENAI_DOWNGRADABLE);
        assert_eq!(
            named(
                r#"{"error":{"message":"Unknown parameter: 'stream_options'.","param":"stream_options"}}"#
            ),
            Some("stream_options")
        );
        assert_eq!(
            named(r#"{"error":{"message":"tool_choice is not supported"}}"#),
            Some("tool_choice")
        );
        assert_eq!(
            named(
                r#"{"error":{"message":"Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens' instead."}}"#
            ),
            Some("max_tokens")
        );
        // Named by the upstream but not in the whitelist, or not part of this request.
        assert_eq!(
            named(r#"{"error":{"message":"response_format is invalid"}}"#),
            None
        );
        assert_eq!(
            named(r#"{"error":{"message":"parallel_tool_calls is invalid"}}"#),
            None
        );
        assert_eq!(named("not json"), None);
        // Anthropic: max_tokens is required and is never named for removal.
        assert_eq!(
            named_downgrade(
                br#"{"error":{"message":"max_tokens: Field required"}}"#,
                &request,
                ANTHROPIC_DOWNGRADABLE
            ),
            None
        );
        assert_eq!(
            named_downgrade(
                br#"{"type":"error","error":{"type":"invalid_request_error","message":"tool_choice is not supported"}}"#,
                &request,
                ANTHROPIC_DOWNGRADABLE
            ),
            Some("tool_choice")
        );
    }

    #[test]
    fn downgrades_remove_a_parameter_or_swap_the_token_limit_name() {
        let mut request = json!({"stream_options": {}, "max_tokens": 16, "model": "m"});
        apply_downgrade(&mut request, "stream_options");
        assert_eq!(request, json!({"max_tokens": 16, "model": "m"}));
        apply_downgrade(&mut request, "max_tokens");
        assert_eq!(request, json!({"max_completion_tokens": 16, "model": "m"}));
        apply_downgrade(&mut request, "max_completion_tokens");
        assert_eq!(request, json!({"max_tokens": 16, "model": "m"}));
    }
}
