//! AC25: structured output (non-streaming). The schema goes upstream in the form each profile
//! supports, the answer is validated on the host, and a mismatch is final: one request, no retry.
mod common;

use common::*;
use dgos_providers::{AdapterOptions, ProviderAdapter};
use dgos_types::{ChatMessage, ChatRequest, DgosError, DgosResult, ResponseFormat};
use serde_json::{json, Value};
use std::time::Duration;

const MARKER: &str = "ANSWER-MARKER-7731";

// ---------------------------------------------------------------------------------------------
// Helpers local to this file
// ---------------------------------------------------------------------------------------------

fn options() -> AdapterOptions {
    AdapterOptions {
        request_timeout: Duration::from_secs(5),
        stream_idle_timeout: Duration::from_secs(2),
        ..Default::default()
    }
}

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {"n": {"type": "integer"}},
        "required": ["n"],
        "additionalProperties": false
    })
}

fn format_with(strict: Option<bool>) -> ResponseFormat {
    ResponseFormat {
        kind: "json_schema".into(),
        name: "answer".into(),
        schema: schema(),
        strict,
    }
}

fn structured_request(format: ResponseFormat) -> ChatRequest {
    ChatRequest {
        response_format: Some(format),
        max_tokens: Some(256),
        ..request()
    }
}

fn body_of(stub: &Stub, index: usize) -> Value {
    serde_json::from_str(&stub.requests()[index].body).unwrap()
}

fn error_of<T>(result: DgosResult<T>) -> DgosError {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(error) => error,
    }
}

fn openai_reply(content: &str, finish_reason: &str) -> String {
    json!({"choices": [{"message": {"role": "assistant", "content": content},
        "finish_reason": finish_reason}],
        "usage": {"prompt_tokens": 5, "completion_tokens": 7, "total_tokens": 12}})
    .to_string()
}

fn anthropic_reply(content: &str, stop_reason: &str) -> String {
    json!({"id": "msg_1", "type": "message", "role": "assistant", "model": "m",
        "content": [{"type": "text", "text": content}], "stop_reason": stop_reason,
        "usage": {"input_tokens": 5, "output_tokens": 7}})
    .to_string()
}

/// Which of the four profile/protocol combinations a case runs on.
#[derive(Clone, Copy, Debug)]
enum Kind {
    OpenAiGeneric,
    OpenAiOfficial,
    AnthropicGeneric,
    AnthropicOfficial,
}

const ALL: [Kind; 4] = [
    Kind::OpenAiGeneric,
    Kind::OpenAiOfficial,
    Kind::AnthropicGeneric,
    Kind::AnthropicOfficial,
];

impl Kind {
    fn is_openai(self) -> bool {
        matches!(self, Kind::OpenAiGeneric | Kind::OpenAiOfficial)
    }

    /// An adapter configured with `retries` automatic retries where the profile helper allows it;
    /// the official-profile helpers always use zero.
    fn adapter(self, url: &str, retries: u32) -> Box<dyn ProviderAdapter> {
        match self {
            Kind::OpenAiGeneric => Box::new(adapter_with(url, retries, 1, options())),
            Kind::OpenAiOfficial => Box::new(openai_official(url, options())),
            Kind::AnthropicGeneric => Box::new(anthropic_with(url, retries, 1, options())),
            Kind::AnthropicOfficial => Box::new(anthropic_official(url, options())),
        }
    }

    /// A 200 reply with `content` as the whole answer.
    fn reply(self, content: &str, finish: &str) -> Vec<Step> {
        if self.is_openai() {
            response(200, &[], &openai_reply(content, finish))
        } else {
            let stop = if finish == "length" {
                "max_tokens"
            } else {
                "end_turn"
            };
            response(200, &[], &anthropic_reply(content, stop))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Request mapping
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn openai_sends_response_format_json_schema_and_strict_only_when_set() {
    for kind in [Kind::OpenAiGeneric, Kind::OpenAiOfficial] {
        let stub = serve(vec![kind.reply(r#"{"n":1}"#, "stop")]).await;
        let adapter = kind.adapter(&stub.url, 0);
        adapter
            .call("m", structured_request(format_with(None)))
            .await
            .unwrap();
        adapter
            .call("m", structured_request(format_with(Some(true))))
            .await
            .unwrap();
        adapter
            .call("m", structured_request(format_with(Some(false))))
            .await
            .unwrap();
        assert_eq!(stub.hits(), 3, "{kind:?}");

        let unset = body_of(&stub, 0);
        assert_eq!(
            unset["response_format"],
            json!({"type": "json_schema",
                "json_schema": {"name": "answer", "schema": schema()}}),
            "{kind:?}"
        );
        assert!(
            unset["response_format"]["json_schema"]
                .get("strict")
                .is_none(),
            "{kind:?}: strict must be absent when unset"
        );
        assert_eq!(
            body_of(&stub, 1)["response_format"]["json_schema"]["strict"],
            true,
            "{kind:?}"
        );
        assert_eq!(
            body_of(&stub, 2)["response_format"]["json_schema"]["strict"],
            false,
            "{kind:?}"
        );
        assert!(unset.get("output_config").is_none());
        assert_eq!(
            unset["messages"],
            json!([{"role": "user", "content": "hello"}])
        );
    }
}

#[tokio::test]
async fn requests_without_response_format_do_not_carry_one() {
    for kind in ALL {
        let stub = serve(vec![kind.reply("plain text", "stop")]).await;
        let response = kind
            .adapter(&stub.url, 0)
            .call("m", request())
            .await
            .unwrap();
        let body = body_of(&stub, 0);
        assert!(body.get("response_format").is_none(), "{kind:?}");
        assert!(body.get("output_config").is_none(), "{kind:?}");
        assert!(
            body.get("system").is_none(),
            "{kind:?}: no instruction without a format"
        );
        // Plain text is not validated and not parsed.
        assert_eq!(response.content, "plain text");
        assert_eq!(response.structured, None);
    }
}

#[tokio::test]
async fn official_anthropic_uses_output_config_format_and_adds_no_system_text() {
    let stub = serve(vec![anthropic_official_reply()]).await;
    let adapter = anthropic_official(&stub.url, options());
    adapter
        .call("m", structured_request(format_with(Some(true))))
        .await
        .unwrap();
    let body = body_of(&stub, 0);
    assert_eq!(
        body["output_config"],
        json!({"format": {"type": "json_schema", "schema": schema()}})
    );
    assert!(
        body.get("system").is_none(),
        "native mode adds no system text"
    );
    assert!(body.get("response_format").is_none());
    assert_eq!(
        body["messages"],
        json!([{"role": "user", "content": [
        {"type": "text", "text": "hello"}]}])
    );
}

fn anthropic_official_reply() -> Vec<Step> {
    Kind::AnthropicOfficial.reply(r#"{"n":1}"#, "stop")
}

#[tokio::test]
async fn official_anthropic_keeps_existing_system_text_untouched() {
    let stub = serve(vec![anthropic_official_reply()]).await;
    let adapter = anthropic_official(&stub.url, options());
    let mut request = structured_request(format_with(None));
    request.messages.insert(
        0,
        ChatMessage {
            role: "system".into(),
            content: "Be terse.".into(),
        },
    );
    adapter.call("m", request).await.unwrap();
    let body = body_of(&stub, 0);
    assert_eq!(body["system"], "Be terse.");
    assert_eq!(body["output_config"]["format"]["schema"], schema());
}

#[tokio::test]
async fn official_anthropic_merges_format_with_reasoning_effort_in_output_config() {
    let stub = serve(vec![anthropic_official_reply()]).await;
    let adapter = anthropic_official(&stub.url, options());
    let mut request = structured_request(format_with(None));
    request.reasoning = Some(dgos_types::ReasoningConfig {
        effort: Some("high".into()),
        budget_tokens: None,
    });
    adapter.call("m", request).await.unwrap();
    let body = body_of(&stub, 0);
    assert_eq!(
        body["output_config"],
        json!({"effort": "high",
            "format": {"type": "json_schema", "schema": schema()}})
    );
    assert_eq!(
        body["thinking"],
        json!({"type": "adaptive", "display": "summarized"})
    );
}

#[tokio::test]
async fn generic_anthropic_puts_the_schema_and_json_in_the_system_string() {
    let stub = serve(vec![Kind::AnthropicGeneric.reply(r#"{"n":1}"#, "stop")]).await;
    let adapter = anthropic_with(&stub.url, 0, 1, options());
    let mut request = structured_request(format_with(Some(true)));
    request.messages.insert(
        0,
        ChatMessage {
            role: "system".into(),
            content: "Be terse.".into(),
        },
    );
    adapter.call("m", request).await.unwrap();
    let body = body_of(&stub, 0);
    assert!(
        body.get("output_config").is_none(),
        "no native mode on the generic profile"
    );
    assert!(body.get("response_format").is_none());
    let system = body["system"].as_str().expect("system is a string");
    assert!(
        system.starts_with("Be terse."),
        "existing system text stays first: {system}"
    );
    assert!(
        system.contains("JSON"),
        "the instruction names JSON: {system}"
    );
    assert!(
        system.contains(&schema().to_string()),
        "the instruction carries the schema: {system}"
    );
    assert!(system.len() > "Be terse.".len());
    // The conversation itself is untouched.
    assert_eq!(
        body["messages"],
        json!([{"role": "user", "content": [
        {"type": "text", "text": "hello"}]}])
    );
}

#[tokio::test]
async fn generic_anthropic_without_a_system_message_gets_the_instruction_alone() {
    let stub = serve(vec![Kind::AnthropicGeneric.reply(r#"{"n":1}"#, "stop")]).await;
    anthropic_with(&stub.url, 0, 1, options())
        .call("m", structured_request(format_with(None)))
        .await
        .unwrap();
    let system = body_of(&stub, 0)["system"].as_str().unwrap().to_owned();
    assert!(system.contains("JSON") && system.contains(&schema().to_string()));
}

// ---------------------------------------------------------------------------------------------
// Validation of the answer
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn conforming_answer_is_parsed_and_the_raw_text_stays_in_content() {
    for kind in ALL {
        let stub = serve(vec![kind.reply(r#"{"n": 42}"#, "stop")]).await;
        let response = kind
            .adapter(&stub.url, 0)
            .call("m", structured_request(format_with(None)))
            .await
            .unwrap();
        assert_eq!(response.structured, Some(json!({"n": 42})), "{kind:?}");
        assert_eq!(response.content, r#"{"n": 42}"#, "{kind:?}");
        assert_eq!(response.finish_reason.as_deref(), Some("stop"));
        assert_eq!(response.usage.total_tokens, 12);
        assert_eq!(stub.hits(), 1);
    }
}

#[tokio::test]
async fn conforming_answer_with_surrounding_whitespace_is_accepted() {
    for kind in ALL {
        let stub = serve(vec![kind.reply("\n  {\"n\":3}\n", "stop")]).await;
        let response = kind
            .adapter(&stub.url, 0)
            .call("m", structured_request(format_with(None)))
            .await
            .unwrap();
        assert_eq!(response.structured, Some(json!({"n": 3})), "{kind:?}");
    }
}

/// Contents that must be refused; each carries the marker so leaks into errors are visible.
fn invalid_answers() -> Vec<(&'static str, String, &'static str)> {
    vec![
        ("wrong type", format!(r#"{{"n":"{MARKER}"}}"#), "stop"),
        ("float instead of integer", r#"{"n":1.5}"#.into(), "stop"),
        (
            "missing required",
            format!(r#"{{"other":"{MARKER}"}}"#),
            "stop",
        ),
        (
            "extra property",
            format!(r#"{{"n":1,"extra":"{MARKER}"}}"#),
            "stop",
        ),
        ("not json", format!("The answer is {MARKER}"), "stop"),
        ("empty content", String::new(), "stop"),
        ("whitespace only", "  \n ".into(), "stop"),
        (
            "code fenced json",
            format!("```json\n{{\"n\":1,\"note\":\"{MARKER}\"}}\n```"),
            "stop",
        ),
        (
            "array instead of object",
            format!(r#"[{{"n":1,"x":"{MARKER}"}}]"#),
            "stop",
        ),
        ("bare string", format!(r#""{MARKER}""#), "stop"),
        (
            "truncated by length",
            format!(r#"{{"n":1,"note":"{MARKER}"#),
            "length",
        ),
    ]
}

#[tokio::test]
async fn non_conforming_answers_fail_with_structured_output_invalid_and_one_request() {
    for kind in ALL {
        for (label, content, finish_reason) in invalid_answers() {
            let stub = serve(vec![kind.reply(&content, finish_reason)]).await;
            // Three automatic retries are configured (where the profile helper allows it): a
            // mismatch is still never retried.
            let error = error_of(
                kind.adapter(&stub.url, 3)
                    .call("m", structured_request(format_with(None)))
                    .await,
            );
            assert_eq!(error.error_code(), 1001021, "{kind:?} / {label}: {error}");
            assert!(matches!(error, DgosError::StructuredOutputInvalid));
            assert_eq!(stub.hits(), 1, "{kind:?} / {label}: no automatic retry");
            let message = error.to_string();
            assert!(!message.contains(MARKER), "{kind:?} / {label}: {message}");
            assert!(!message.contains(API_KEY), "{kind:?} / {label}");
            if !content.trim().is_empty() {
                assert!(!message.contains(content.trim()), "{kind:?} / {label}");
            }
            let debug = format!("{error:?}");
            assert!(!debug.contains(MARKER), "{kind:?} / {label}: {debug}");
        }
    }
}

#[tokio::test]
async fn invalid_answer_is_final_while_transport_failures_still_retry() {
    // A 500 is retried and the following conforming answer is accepted ...
    let stub = serve(vec![
        response(500, &[], "{}"),
        response(200, &[], &openai_reply(r#"{"n":5}"#, "stop")),
    ])
    .await;
    let answer = adapter_with(&stub.url, 3, 1, options())
        .call("m", structured_request(format_with(None)))
        .await
        .unwrap();
    assert_eq!(answer.structured, Some(json!({"n": 5})));
    assert_eq!(stub.hits(), 2);

    // ... but a valid HTTP answer that violates the schema is not, even though the next
    // scripted answer would have conformed.
    let stub = serve(vec![
        response(200, &[], &openai_reply(r#"{"n":"x"}"#, "stop")),
        response(200, &[], &openai_reply(r#"{"n":5}"#, "stop")),
    ])
    .await;
    let error = error_of(
        adapter_with(&stub.url, 3, 1, options())
            .call("m", structured_request(format_with(None)))
            .await,
    );
    assert_eq!(error.error_code(), 1001021);
    assert_eq!(stub.hits(), 1);
}

// ---------------------------------------------------------------------------------------------
// Upstream rejection
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn upstream_400_on_the_format_is_rejected_after_exactly_one_request() {
    let openai_bodies = [
        json!({"error": {"message": format!("Invalid schema {MARKER}"),
            "type": "invalid_request_error", "code": "invalid_json_schema"}}),
        // Naming `response_format` as the parameter does not make it downgradable.
        json!({"error": {"message": format!("Unsupported {MARKER}"),
            "type": "invalid_request_error", "param": "response_format"}}),
    ];
    for body in openai_bodies {
        for kind in [Kind::OpenAiGeneric, Kind::OpenAiOfficial] {
            let stub = serve(vec![response(400, &[], &body.to_string())]).await;
            let error = error_of(
                kind.adapter(&stub.url, 3)
                    .call("m", structured_request(format_with(Some(true))))
                    .await,
            );
            assert_eq!(error.error_code(), 1001017, "{kind:?}: {error}");
            assert_eq!(stub.hits(), 1, "{kind:?}: no downgrade, no retry");
            assert!(
                body_of(&stub, 0).get("response_format").is_some(),
                "{kind:?}: the host does not drop response_format"
            );
            let message = error.to_string();
            assert!(
                !message.contains(MARKER) && !message.contains(API_KEY),
                "{message}"
            );
        }
    }

    let anthropic_body = json!({"type": "error", "error": {"type": "invalid_request_error",
        "message": format!("output_config.format is not supported {MARKER}")}});
    for kind in [Kind::AnthropicGeneric, Kind::AnthropicOfficial] {
        let stub = serve(vec![response(400, &[], &anthropic_body.to_string())]).await;
        let error = error_of(
            kind.adapter(&stub.url, 3)
                .call("m", structured_request(format_with(None)))
                .await,
        );
        assert_eq!(error.error_code(), 1001017, "{kind:?}: {error}");
        assert_eq!(stub.hits(), 1, "{kind:?}");
        let message = error.to_string();
        assert!(
            !message.contains(MARKER) && !message.contains(API_KEY),
            "{message}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Local refusals
// ---------------------------------------------------------------------------------------------

fn assert_not_supported(error: &DgosError, label: &str) {
    assert_eq!(error.error_code(), 1001020, "{label}: {error}");
    assert!(matches!(error, DgosError::RequestNotSupported), "{label}");
    assert!(!error.to_string().contains(API_KEY), "{label}");
}

fn bad_formats() -> Vec<(&'static str, ResponseFormat)> {
    let with = |change: &dyn Fn(&mut ResponseFormat)| {
        let mut format = format_with(None);
        change(&mut format);
        format
    };
    vec![
        (
            "type is not json_schema",
            with(&|f| f.kind = "json_object".into()),
        ),
        ("empty type", with(&|f| f.kind = String::new())),
        ("name with a space", with(&|f| f.name = "my answer".into())),
        ("empty name", with(&|f| f.name = String::new())),
        ("name of 65 characters", with(&|f| f.name = "a".repeat(65))),
        ("name with a slash", with(&|f| f.name = "a/b".into())),
        ("schema is a string", with(&|f| f.schema = json!("object"))),
        (
            "schema is an array",
            with(&|f| f.schema = json!([{"type": "object"}])),
        ),
        ("schema is null", with(&|f| f.schema = Value::Null)),
        (
            "schema does not compile",
            with(&|f| f.schema = json!({"type": "no-such-type"})),
        ),
    ]
}

#[tokio::test]
async fn unusable_formats_are_refused_locally_without_a_request() {
    for kind in ALL {
        let stub = serve(vec![kind.reply(r#"{"n":1}"#, "stop")]).await;
        let adapter = kind.adapter(&stub.url, 3);
        for (label, format) in bad_formats() {
            let error = error_of(adapter.call("m", structured_request(format)).await);
            assert_not_supported(&error, &format!("{kind:?} / {label}"));
        }
        assert_eq!(stub.hits(), 0, "{kind:?}: nothing may reach the upstream");
    }
}

#[tokio::test]
async fn name_of_exactly_64_characters_is_accepted() {
    for kind in ALL {
        let stub = serve(vec![kind.reply(r#"{"n":1}"#, "stop")]).await;
        let mut format = format_with(None);
        format.name = "a-B_9".repeat(12) + "wxyz";
        assert_eq!(format.name.len(), 64);
        let response = kind
            .adapter(&stub.url, 0)
            .call("m", structured_request(format.clone()))
            .await
            .unwrap();
        assert_eq!(response.structured, Some(json!({"n": 1})), "{kind:?}");
        if kind.is_openai() {
            assert_eq!(
                body_of(&stub, 0)["response_format"]["json_schema"]["name"],
                format.name.as_str()
            );
        }
    }
}

#[tokio::test]
async fn streaming_with_a_response_format_is_refused_without_a_request() {
    for kind in ALL {
        let stub = serve(vec![sse_script(&[
            delta("{}"),
            finish("stop"),
            DONE.into(),
        ])])
        .await;
        let error = error_of(
            kind.adapter(&stub.url, 3)
                .stream("m", structured_request(format_with(Some(true))))
                .await,
        );
        assert_not_supported(&error, &format!("{kind:?} stream"));
        assert_eq!(stub.hits(), 0, "{kind:?}");
    }
}
