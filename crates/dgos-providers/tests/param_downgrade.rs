//! AC36: a 400 that names a whitelisted parameter is retried once without it. At most one
//! downgrade per call, none persisted, no upstream text in errors.
mod common;

use common::*;
use dgos_providers::ProviderAdapter;
use dgos_types::{ToolChatRequest, ToolMessage};
use serde_json::{json, Value};

fn reject(message: &str, param: &str) -> Vec<Step> {
    response(
        400,
        &[],
        &json!({"error":{"message":message,"param":param,"type":"invalid_request_error"}})
            .to_string(),
    )
}

fn body_of(stub: &Stub, index: usize) -> Value {
    serde_json::from_str(&stub.requests()[index].body).unwrap()
}

fn tool_request() -> ToolChatRequest {
    ToolChatRequest {
        messages: vec![ToolMessage {
            role: "user".into(),
            content: Some("hi".into()),
            tool_call_id: None,
            tool_calls: vec![],
            reasoning: None,
        }],
        tools: vec![],
        max_tokens: 64,
    }
}

fn tool_ok() -> String {
    json!({"choices":[{"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})
    .to_string()
}

#[tokio::test]
async fn stream_options_rejection_is_dropped_and_the_stream_succeeds() {
    let stub = serve(vec![
        reject("Unknown parameter: 'stream_options'.", "stream_options"),
        sse_script(&[delta("hi"), finish("stop"), DONE.into()]),
    ])
    .await;
    // Zero retries configured: the downgrade is not a retry.
    let items = drain(
        adapter(&stub.url, 0, 1)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text_of(&items), "hi");
    assert_eq!(stub.hits(), 2);
    assert!(body_of(&stub, 0).get("stream_options").is_some());
    assert!(body_of(&stub, 1).get("stream_options").is_none());
    assert_eq!(body_of(&stub, 1)["stream"], true);
}

#[tokio::test]
async fn a_downgrade_is_not_remembered_by_the_next_call() {
    let stub = serve(vec![
        reject("Unknown parameter: 'stream_options'.", "stream_options"),
        sse_script(&[delta("a"), finish("stop"), DONE.into()]),
        sse_script(&[delta("b"), finish("stop"), DONE.into()]),
    ])
    .await;
    let adapter = adapter(&stub.url, 0, 1);
    drain(adapter.stream("m", request()).await.unwrap()).await;
    drain(adapter.stream("m", request()).await.unwrap()).await;
    assert_eq!(stub.hits(), 3);
    assert!(
        body_of(&stub, 2).get("stream_options").is_some(),
        "sent again on a new call"
    );
}

#[tokio::test]
async fn tool_choice_rejection_is_dropped_and_the_tool_call_succeeds() {
    let stub = serve(vec![
        reject("tool_choice is not supported by this model", "tool_choice"),
        response(200, &[], &tool_ok()),
    ])
    .await;
    let reply = adapter(&stub.url, 0, 1)
        .call_tools("m", tool_request())
        .await
        .unwrap();
    assert_eq!(reply.message.content.as_deref(), Some("done"));
    assert_eq!(stub.hits(), 2);
    assert_eq!(body_of(&stub, 0)["tool_choice"], "auto");
    assert!(body_of(&stub, 1).get("tool_choice").is_none());
    assert_eq!(body_of(&stub, 1)["parallel_tool_calls"], false);
}

#[tokio::test]
async fn a_call_is_downgraded_at_most_once() {
    let stub = serve(vec![
        reject("Unknown parameter: 'tool_choice'.", "tool_choice"),
        reject(
            "Unknown parameter: 'parallel_tool_calls'.",
            "parallel_tool_calls",
        ),
        response(200, &[], &tool_ok()),
    ])
    .await;
    let error = adapter(&stub.url, 0, 1)
        .call_tools("m", tool_request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001017);
    assert_eq!(
        stub.hits(),
        2,
        "second rejection is final; the success script is never reached"
    );
}

#[tokio::test]
async fn a_parameter_outside_the_whitelist_is_rejected_at_once_without_a_retry() {
    let stub = serve(vec![
        reject(
            "response_format 'json_schema' is invalid. secret-detail",
            "response_format",
        ),
        response(200, &[], &ok_chat()),
    ])
    .await;
    let error = adapter(&stub.url, 3, 1)
        .stream("m", request())
        .await
        .err()
        .unwrap();
    assert_eq!(error.error_code(), 1001017);
    assert_eq!(stub.hits(), 1);
    assert!(!format!("{error} {error:?}").contains("secret-detail"));
}

#[tokio::test]
async fn a_whitelisted_name_the_request_does_not_carry_does_not_trigger_a_downgrade() {
    // A plain call has no stream_options, so naming it cannot be acted on.
    let stub = serve(vec![
        reject("stream_options is not allowed", "stream_options"),
        response(200, &[], &ok_chat()),
    ])
    .await;
    let error = adapter(&stub.url, 3, 1)
        .call("m", request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001017);
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn max_tokens_and_max_completion_tokens_trade_places() {
    let stub = serve(vec![
        reject(
            "Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens' instead.",
            "max_tokens",
        ),
        response(200, &[], &ok_chat()),
    ])
    .await;
    let reply = adapter(&stub.url, 0, 1).call("m", request()).await.unwrap();
    assert_eq!(reply.content, "ok");
    let (first, second) = (body_of(&stub, 0), body_of(&stub, 1));
    assert_eq!(first["max_tokens"], 16);
    assert!(first.get("max_completion_tokens").is_none());
    assert_eq!(second["max_completion_tokens"], 16);
    assert!(second.get("max_tokens").is_none());
}

#[tokio::test]
async fn an_unnamed_400_is_rejected_without_a_downgrade() {
    let stub = serve(vec![response(400, &[], "")]).await;
    let error = adapter(&stub.url, 3, 1)
        .call_tools("m", tool_request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001017);
    assert_eq!(stub.hits(), 1);
}

// ---------------------------------------------------------------------------------------------
// AC36 leftovers: `parallel_tool_calls` named alone on openai_compatible.
// ---------------------------------------------------------------------------------------------

const MARKER: &str = "upstream-marker-downgrade-64";

#[tokio::test]
async fn parallel_tool_calls_named_alone_is_dropped_keeping_tool_choice_and_is_not_persisted() {
    let stub = serve(vec![
        reject(
            &format!("Unknown parameter: 'parallel_tool_calls'. {MARKER}"),
            "parallel_tool_calls",
        ),
        response(200, &[], &tool_ok()),
        response(200, &[], &tool_ok()),
    ])
    .await;
    let adapter = adapter(&stub.url, 0, 1);
    let reply = adapter.call_tools("m", tool_request()).await.unwrap();
    assert_eq!(reply.message.content.as_deref(), Some("done"));
    assert_eq!(stub.hits(), 2, "exactly one downgrade retry");
    let (first, second) = (body_of(&stub, 0), body_of(&stub, 1));
    assert_eq!(first["parallel_tool_calls"], false);
    assert_eq!(first["tool_choice"], "auto");
    assert!(second.get("parallel_tool_calls").is_none());
    assert_eq!(
        second["tool_choice"], "auto",
        "only the named key is removed"
    );
    // Everything else is unchanged between the two attempts.
    let mut expected = first.clone();
    expected
        .as_object_mut()
        .unwrap()
        .remove("parallel_tool_calls");
    assert_eq!(second, expected);

    // The next call starts from the full body again.
    adapter.call_tools("m", tool_request()).await.unwrap();
    assert_eq!(stub.hits(), 3);
    assert_eq!(body_of(&stub, 2), first, "the downgrade is not persisted");
}

#[tokio::test]
async fn parallel_tool_calls_rejected_in_the_message_only_is_found_and_errors_stay_clean() {
    // The name appears in the message, not in `param`: still named.
    let stub =
        serve(vec![
        response(
            400,
            &[],
            &json!({"error":{"message":format!("parallel_tool_calls is not supported {MARKER}")}})
                .to_string(),
        ),
        // The second rejection is final, whatever it names.
        reject(&format!("Unknown parameter: 'tool_choice'. {MARKER}"), "tool_choice"),
        response(200, &[], &tool_ok()),
    ])
        .await;
    let error = adapter(&stub.url, 3, 1)
        .call_tools("m", tool_request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001017);
    assert_eq!(
        stub.hits(),
        2,
        "one downgrade, then the second 400 is final"
    );
    assert!(body_of(&stub, 1).get("parallel_tool_calls").is_none());
    assert_eq!(body_of(&stub, 1)["tool_choice"], "auto");
    let shown = format!("{error} {error:?}");
    assert!(
        !shown.contains(MARKER) && !shown.contains(API_KEY),
        "{shown}"
    );
}

// ---------------------------------------------------------------------------------------------
// Anthropic Messages: only `tool_choice` may be dropped.
// ---------------------------------------------------------------------------------------------

fn anthropic_reject(message: &str) -> Vec<Step> {
    response(
        400,
        &[],
        &json!({"type":"error","error":{"type":"invalid_request_error","message":message}})
            .to_string(),
    )
}

fn anthropic_tool_ok() -> String {
    json!({"content":[{"type":"text","text":"done"}],"stop_reason":"end_turn",
        "usage":{"input_tokens":1,"output_tokens":1}})
    .to_string()
}

#[tokio::test]
async fn anthropic_tool_choice_rejection_is_dropped_once_and_the_call_succeeds() {
    let stub = serve(vec![
        anthropic_reject(&format!(
            "tool_choice: Extra inputs are not permitted {MARKER}"
        )),
        response(200, &[], &anthropic_tool_ok()),
        response(200, &[], &anthropic_tool_ok()),
    ])
    .await;
    // Zero retries configured: the downgrade is not a retry.
    let adapter = anthropic(&stub.url, 0, 1);
    let reply = adapter.call_tools("m", tool_request()).await.unwrap();
    assert_eq!(reply.message.content.as_deref(), Some("done"));
    assert_eq!(stub.hits(), 2);
    let (first, second) = (body_of(&stub, 0), body_of(&stub, 1));
    assert_eq!(first["tool_choice"]["type"], "auto");
    assert_eq!(first["tool_choice"]["disable_parallel_tool_use"], true);
    assert!(second.get("tool_choice").is_none());
    let mut expected = first.clone();
    expected.as_object_mut().unwrap().remove("tool_choice");
    assert_eq!(second, expected, "only tool_choice is removed");
    assert_eq!(second["max_tokens"], 64);

    // Not persisted: the next call sends tool_choice again.
    adapter.call_tools("m", tool_request()).await.unwrap();
    assert_eq!(body_of(&stub, 2), first);
}

#[tokio::test]
async fn anthropic_tool_choice_downgrade_also_works_under_the_official_profile() {
    let stub = serve(vec![
        anthropic_reject("tool_choice is not supported by this model"),
        response(200, &[], &anthropic_tool_ok()),
    ])
    .await;
    let reply = anthropic_official(&stub.url, Default::default())
        .call_tools("m", tool_request())
        .await
        .unwrap();
    assert_eq!(reply.message.content.as_deref(), Some("done"));
    assert_eq!(stub.hits(), 2);
    assert!(body_of(&stub, 0).get("tool_choice").is_some());
    assert!(body_of(&stub, 1).get("tool_choice").is_none());
}

#[tokio::test]
async fn anthropic_a_second_rejection_is_final_and_leaks_nothing() {
    let stub = serve(vec![
        anthropic_reject("tool_choice: Extra inputs are not permitted"),
        anthropic_reject(&format!("tools: invalid schema {MARKER}")),
        response(200, &[], &anthropic_tool_ok()),
    ])
    .await;
    let error = anthropic(&stub.url, 3, 1)
        .call_tools("m", tool_request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001017);
    assert_eq!(stub.hits(), 2, "one downgrade, then final");
    let shown = format!("{error} {error:?}");
    assert!(
        !shown.contains(MARKER) && !shown.contains(API_KEY),
        "{shown}"
    );
}

#[tokio::test]
async fn anthropic_max_tokens_is_required_and_never_downgraded() {
    // Tool call.
    let stub = serve(vec![
        anthropic_reject(&format!("max_tokens: Field required {MARKER}")),
        response(200, &[], &anthropic_tool_ok()),
    ])
    .await;
    let error = anthropic(&stub.url, 3, 1)
        .call_tools("m", tool_request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001017);
    assert_eq!(stub.hits(), 1);
    assert_eq!(body_of(&stub, 0)["max_tokens"], 64);
    let shown = format!("{error} {error:?}");
    assert!(
        !shown.contains(MARKER) && !shown.contains(API_KEY),
        "{shown}"
    );

    // Plain call, and the `max_completion_tokens` swap of openai_compatible does not exist here.
    let stub = serve(vec![
        anthropic_reject("max_tokens: Field required"),
        response(200, &[], &ok_message()),
    ])
    .await;
    let error = anthropic(&stub.url, 3, 1)
        .call("m", request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001017);
    assert_eq!(stub.hits(), 1);
    let body = body_of(&stub, 0);
    assert_eq!(body["max_tokens"], 16);
    assert!(body.get("max_completion_tokens").is_none());
}

#[tokio::test]
async fn anthropic_stream_never_sends_stream_options_and_a_400_naming_it_is_not_downgraded() {
    let stub = serve(vec![
        anthropic_reject(&format!(
            "stream_options: Extra inputs are not permitted {MARKER}"
        )),
        sse_script(&[
            message_start(1),
            text_delta("hi"),
            message_delta("end_turn", 1),
            message_stop(),
        ]),
    ])
    .await;
    let error = anthropic(&stub.url, 3, 1)
        .stream("m", request())
        .await
        .err()
        .unwrap();
    assert_eq!(error.error_code(), 1001017);
    assert_eq!(stub.hits(), 1, "no downgrade, no retry");
    let body = body_of(&stub, 0);
    assert!(
        body.get("stream_options").is_none(),
        "the key is never sent"
    );
    assert_eq!(body["stream"], true);
    let shown = format!("{error} {error:?}");
    assert!(
        !shown.contains(MARKER) && !shown.contains(API_KEY),
        "{shown}"
    );

    // A successful stream carries no stream_options and no tool_choice either.
    let stub = serve(vec![sse_script(&[
        message_start(1),
        text_delta("hi"),
        message_delta("end_turn", 1),
        message_stop(),
    ])])
    .await;
    let items = drain(
        anthropic(&stub.url, 0, 1)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text_of(&items), "hi");
    let body = body_of(&stub, 0);
    assert!(body.get("stream_options").is_none());
    assert!(body.get("tool_choice").is_none());
}

#[tokio::test]
async fn anthropic_a_tool_choice_name_on_a_request_without_it_does_not_trigger_a_downgrade() {
    // A plain call carries no `tool_choice`, so naming it cannot be acted on.
    let stub = serve(vec![
        anthropic_reject("tool_choice is not allowed"),
        response(200, &[], &ok_message()),
    ])
    .await;
    let error = anthropic(&stub.url, 3, 1)
        .call("m", request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001017);
    assert_eq!(stub.hits(), 1);
    assert!(body_of(&stub, 0).get("tool_choice").is_none());
}
