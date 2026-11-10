//! Actual public AiService/API smoke, with a published pinned module and temporary SQLite.
//! Default: loopback protocol evidence only, never a real supplier or model-quality claim.
//! Run: cargo run --offline -p dgos-application --example ai_foundation_smoke
//! Optional `--real` is gated by an explicit fixed-sample authorization and endpoint/model/key.
//! No real credentials, user database, OS credential vault or user files are read by default.
#[path = "ai_foundation_smoke/fixture.rs"]
mod fixture;
use fixture::*;
use dgos_application::{AiEventSink, AiInvocation};
use dgos_db::{sea_orm::{ConnectionTrait, DbBackend, Statement}, AiContentRepository, ModulePermission};
use dgos_types::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::Path, sync::{Arc, Mutex}, time::{Duration, Instant}};
use tokio_util::sync::CancellationToken;

const BASELINE: &str = include_str!("ai_foundation_smoke/baseline.json");
const AUTHORIZATION: &str = "I_AUTHORIZE_FIXED_FOUNDATION_SAMPLE_2_HTTP_ATTEMPTS_128_OUTPUT_TOKENS";
const PARTIAL: &str = "Release Friday; budget still pending (interrupted partial).";
#[derive(Deserialize)]
struct Baseline { id: String, source: String, unicode_source: String, expected_facts: Value, schema: Value, cases: Vec<Case> }
#[derive(Deserialize)]
struct Case { id: String, group: String, path: String, expected_facts: Vec<String>, expected_actions: Vec<String> }

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--crash-child") {
        if args.len() != 3 { return Err("internal child requires temporary root and loopback URL".into()); }
        let url = url::Url::parse(&args[2])?;
        if !url.host_str().is_some_and(|host| host == "127.0.0.1" || host == "localhost" || host == "::1") {
            return Err("crash child only accepts loopback".into());
        }
        return crash_child(Path::new(&args[1]), &args[2]).await;
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("cargo run --offline -p dgos-application --example ai_foundation_smoke [-- --real]\n\
Default runs all 18 local fixed cases. --real additionally runs one explicitly authorized fixed factual JSON sample.\n\
Real gate: DGOS_FOUNDATION_REAL_AUTHORIZATION={AUTHORIZATION}\n\
Also required: DGOS_FOUNDATION_REAL_BASE_URL, DGOS_FOUNDATION_REAL_MODEL, DGOS_FOUNDATION_REAL_API_KEY.\n\
Authorization covers one logical run, at most two HTTP attempts (existing parameter negotiation), 128 output tokens maximum.
No MCP/Skill/production-data calls or transient retries. Monetary cost is unknown, not zero.\n\
Missing authorization/configuration prints NOT RUN and is excluded from PASS counts.");
        return Ok(());
    }
    if args.iter().any(|arg| arg != "--real") { return Err("unknown argument; use --help".into()); }
    let baseline: Baseline = serde_json::from_str(BASELINE)?;
    validate_baseline(&baseline);
    let root = tempfile::tempdir()?;
    let started = Instant::now();
    let mut passed = 0;
    for case in &baseline.cases {
        let case_started = Instant::now();
        let mut server = mockito::Server::new_async().await;
        // Any unplanned dispatch is observed explicitly, even on expected rejection paths.
        let unexpected = server.mock("POST", "/chat/completions").expect(0).create_async().await;
        let path = root.path().join(&case.id);
        if case.id == "lifecycle-failure" {
            process_recovery(&path, &mut server).await?;
        } else {
            let mut f = Fixture::open(&path, &server.url(), "foundation-fixture-model", "synthetic-foundation-key").await?;
            exercise(case, &baseline, &mut f, &mut server).await?;
            f.mcp.shutdown().await?;
        }
        unexpected.assert_async().await;
        passed += 1;
        println!("{}", json!({"baseline":baseline.id,"case":case.id,"group":case.group,"path":case.path,
            "evidence":"LOCAL_PROTOCOL_AND_PRODUCTION_API","status":"PASS","elapsed_ms":case_started.elapsed().as_millis(),
            "expected_facts":case.expected_facts,"expected_actions":case.expected_actions,
            "model_quality":"NOT_EVALUATED_BY_SCRIPTED_PROVIDER","billed_cost":"not_applicable_local"}));
    }
    println!("{}", json!({"evidence":"LOCAL_PROTOCOL_AND_PRODUCTION_API","status":"PASS","passed":passed,
        "total":baseline.cases.len(),"elapsed_ms":started.elapsed().as_millis(),
        "webview":"NOT RUN: page transport is a host receipt fixture, not a browser",
        "platform":std::env::consts::OS,"architecture":std::env::consts::ARCH}));
    if args.iter().any(|arg| arg == "--real") { real_sample(&baseline).await?; }
    else { not_run("real Provider/MCP quality: no explicit --real authorization; local PASS never counts as real PASS"); }
    Ok(())
}
fn validate_baseline(b: &Baseline) {
    let groups = ["context", "tools", "skills", "output", "cancel_recovery", "isolation"];
    assert_eq!(b.cases.len(), 18);
    let mut ids = BTreeSet::new();
    for group in groups {
        let paths: BTreeSet<_> = b.cases.iter().filter(|case| case.group == group).map(|case| case.path.as_str()).collect();
        assert_eq!(paths, BTreeSet::from(["success", "boundary", "failure"]));
    }
    for case in &b.cases {
        assert!(ids.insert(&case.id));
        assert!(!case.expected_facts.is_empty() && !case.expected_actions.is_empty());
    }
}
fn success(run: &Run, text: &str) {
    assert_eq!(run.terminal.status, AiRunStatus::Succeeded);
    assert_eq!(run.terminal.content.as_deref(), Some(text));
    assert!(!run.terminal.audit_warning && !run.terminal.result_unknown);
    assert!(run.terminal.error.is_none());
    assert!(run.events.iter().any(|event| matches!(event.payload, AiEventPayload::Delta { .. })));
}
async fn result(f: &Fixture, id: &str) -> Result<AiRunContent> {
    Ok(f.service.get_result(f.invoke(true), id).await?.ok_or("missing content metadata")?)
}
async fn denied_before_dispatch(f: &Fixture, input: AiRunInput) {
    assert!(f.service.inspect_context(f.invoke(false), input.clone()).await.is_err());
    assert!(f.service.run(f.invoke(false), input, Arc::new(|_: AiEvent| true)).await.is_err());
}
async fn approval(f: &mut Fixture, input: AiRunInput, approved: bool) -> Result<Run> {
    let invocation = f.invoke(false);
    let service = f.service.clone();
    let account = f.account.clone();
    let selected = input.tools[0].clone();
    let attachment = f.attachment.clone();
    let execute = run_service(&service, invocation, input);
    let decide = async {
        let prompt = tokio::time::timeout(Duration::from_secs(10), f.prompts.recv()).await?
            .ok_or("missing exact MCP confirmation")?;
        assert_eq!(prompt.attachment, attachment);
        assert_eq!(prompt.module, MODULE);
        assert_eq!(prompt.server, selected.server_id);
        assert_eq!(prompt.tool, selected.tool_name);
        assert_eq!(prompt.args, json!({"text":"foundation-fixed-effect-once","delay":0}));
        service.resolve_confirmation(&HostSession::for_user(USER), &account, &prompt.id, approved)?;
        Ok::<_, Error>(())
    };
    let (run, decision) = tokio::join!(execute, decide);
    decision?;
    run
}
async fn exercise(case: &Case, b: &Baseline, f: &mut Fixture, server: &mut mockito::ServerGuard) -> Result<()> {
    let marker = case.id.as_str();
    match marker {
        "context-success" => {
            let resource = f.import("fixed-release.txt", &b.source).await?;
            assert_eq!(resource.bytes_len, b.source.len() as u64);
            assert_eq!(resource.sha256.len(), 64);
            let mut input = f.input(marker);
            input.foundation.retain_result = true;
            input.foundation.context_refs = vec![reference(&resource, AiContextMode::Inline, 0, 8192)];
            let budget = f.service.inspect_context(f.invoke(false), input.clone()).await?;
            assert!(budget.can_send && budget.text_bytes >= b.source.len());
            let expected = b.expected_facts.to_string();
            let (mock, requests) = response(server, marker, None, text_sse(&expected)).await;
            let run = run(f, input).await?;
            success(&run, &expected);
            assert!(request(&requests).to_string().contains(&b.source));
            let stored = result(f, &run.id).await?;
            assert_eq!(stored.context_receipts.len(), 1);
            let receipt = &stored.context_receipts[0];
            assert_eq!(receipt.resource_id, resource.id);
            assert_eq!(receipt.sha256, resource.sha256);
            assert_eq!(receipt.start, 0);
            assert_eq!(receipt.bytes as usize, b.source.len());
            mock.assert_async().await;
        }
        "context-boundary" => {
            let resource = f.import("utf8-boundary.txt", &b.unicode_source).await?;
            let mut input = f.input(marker);
            input.foundation.retain_result = true;
            input.foundation.context_refs = vec![reference(&resource, AiContextMode::OnDemand, 0, 4)];
            assert!(f.service.inspect_context(f.invoke(false), input.clone()).await?.can_send);
            let (first, initial) = response(server, marker, None,
                tool_sse("dgos_resource_read", "read-utf8", json!({"resource_id":resource.id,"start":0,"max_bytes":4}))).await;
            let (second, delivered) = response(server, marker, Some("read-utf8"), text_sse("Only 甲 was supplied; the remaining source is unread.")).await;
            let run = run(f, input).await?;
            success(&run, "Only 甲 was supplied; the remaining source is unread.");
            assert!(!request(&initial).to_string().contains(&b.unicode_source));
            let actual = tool_result(&request(&delivered), "read-utf8");
            assert_eq!(actual["content"], "甲");
            assert_eq!(actual["receipt"]["bytes"], 3);
            assert_eq!(actual["receipt"]["start"], 0);
            assert_eq!(result(f, &run.id).await?.context_receipts[0].bytes, 3);
            first.assert_async().await;
            second.assert_async().await;
            let large = f.import("budget-limit.txt", &"x".repeat(23 * 1024)).await?;
            let mut oversized = f.input(&format!("context-too-long {}", "p".repeat(2048)));
            oversized.foundation.context_refs = vec![reference(&large, AiContextMode::Inline, 0, 24 * 1024)];
            let budget = f.service.inspect_context(f.invoke(false), oversized.clone()).await?;
            assert!(!budget.can_send && budget.rejection.is_some());
            assert!(f.service.run(f.invoke(false), oversized, Arc::new(|_: AiEvent| true)).await.is_err());
        }
        "context-failure" => {
            let deleted = f.import("deleted.txt", &b.source).await?;
            let mut input = f.input(marker);
            input.foundation.context_refs = vec![reference(&deleted, AiContextMode::Inline, 0, 8192)];
            assert!(f.service.delete_resource(f.invoke(false), &deleted.id).await?);
            denied_before_dispatch(f, input).await;
            let live = f.import("revoked-mid-step.txt", &b.source).await?;
            let mut input = f.input(marker);
            input.foundation.retain_result = true;
            input.foundation.context_refs = vec![reference(&live, AiContextMode::OnDemand, 0, 8192)];
            let mut wrong_hash = input.clone();
            wrong_hash.foundation.context_refs[0].sha256 = "0".repeat(64);
            denied_before_dispatch(f, wrong_hash).await;
            let entered = Arc::new(tokio::sync::Notify::new());
            let signal = entered.clone();
            let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
            let requests = captured.clone();
            let response_body = tool_sse("dgos_resource_read", "deleted-read", json!({"resource_id":live.id,"start":0,"max_bytes":8192}));
            let mock = server.mock("POST", "/chat/completions").match_request(move |r| {
                requests.lock().unwrap().push(serde_json::from_slice(r.body().unwrap()).unwrap());
                signal.notify_one(); true
            }).with_header("content-type", "text/event-stream").with_chunked_body(move |writer| {
                std::thread::sleep(Duration::from_millis(300));
                std::io::Write::write_all(writer, response_body.as_bytes())
            }).expect(1).create_async().await;
            let delete = async {
                tokio::time::timeout(Duration::from_secs(5), entered.notified()).await?;
                assert!(f.service.delete_resource(f.invoke(false), &live.id).await?);
                Ok::<_, Error>(())
            };
            let (run, deletion) = tokio::join!(run(f, input), delete);
            deletion?;
            let run = run?;
            assert_ne!(run.terminal.status, AiRunStatus::Succeeded);
            assert!(result(f, &run.id).await?.context_receipts.is_empty());
            assert!(!request(&captured).to_string().contains(&b.source));
            mock.assert_async().await;
        }
        "tools-success" | "tools-boundary" | "tools-failure" => {
            let tool = f.tool().await?;
            let mut input = f.input(marker);
            input.tools = vec![tool.clone()];
            input.foundation.retain_result = true;
            let args = json!({"text":"foundation-fixed-effect-once","delay":0});
            let (first, initial) = response(server, marker, None, tool_sse("dgos_tool_0", "effect-original-id", args.clone())).await;
            if marker == "tools-failure" {
                let run = approval(f, input, false).await?;
                assert_ne!(run.terminal.status, AiRunStatus::Succeeded);
                assert!(f.mcp.list_usage_logs(USER, Some(&tool.server_id), None).await?.is_empty());
                assert!(result(f, &run.id).await?.tool_receipts.as_array().unwrap().is_empty());
            } else {
                let (second, delivered) = response(server, marker, Some("effect-original-id"), text_sse("Observed approved fixture effect once.")).await;
                let source = approval(f, input.clone(), true).await?;
                success(&source, "Observed approved fixture effect once.");
                let actual = tool_result(&request(&delivered), "effect-original-id");
                assert_eq!(actual["isError"], false);
                assert_eq!(actual["content"][0]["text"], "foundation-fixed-effect-once");
                let logs = f.mcp.list_usage_logs(USER, Some(&tool.server_id), None).await?;
                assert_eq!(logs.len(), 1);
                assert_eq!(logs[0].caller.as_deref(), Some("module"));
                assert_eq!(logs[0].task_ref.as_deref(), Some(source.id.as_str()));
                assert_eq!(logs[0].outcome, "success");
                let content = result(f, &source.id).await?;
                assert_eq!(content.tool_receipts.as_array().unwrap().len(), 1);
                assert_eq!(content.tool_receipts[0]["outcome"], "succeeded");
                assert_eq!(content.tool_receipts[0]["unknown"], false);
                assert_eq!(content.tool_receipts[0]["fingerprint"].as_str().unwrap().len(), 64);
                second.assert_async().await;
                if marker == "tools-boundary" {
                    let continuation_marker = "tools-boundary-continuation";
                    input.prompt = continuation_marker.into();
                    input.foundation.source_run_id = Some(source.id.clone());
                    let (replay, continuation_request) = response(server, continuation_marker, None,
                        tool_sse("dgos_tool_0", "different-provider-call-id", args)).await;
                    let continuation = run(f, input).await?;
                    assert_ne!(continuation.terminal.status, AiRunStatus::Succeeded);
                    assert_eq!(continuation.terminal.error.as_ref().map(|error| error.reason.as_str()), Some("completed_effect_replay"));
                    assert_ne!(continuation.id, source.id);
                    assert_eq!(result(f, &continuation.id).await?.source_run_id.as_deref(), Some(source.id.as_str()));
                    assert!(request(&continuation_request).to_string().contains("Observed approved fixture effect once."));
                    assert_eq!(f.mcp.list_usage_logs(USER, Some(&tool.server_id), None).await?.len(), 1);
                    assert!(f.prompts.try_recv().is_err(), "inherited fingerprint must block before a new approval");
                    replay.assert_async().await;
                }
            }
            assert!(request(&initial)["tools"].as_array().unwrap().iter().any(|tool| tool["function"]["name"] == "dgos_tool_0"));
            first.assert_async().await;
        }
        "skills-success" | "skills-boundary" | "skills-failure" => {
            let pin = f.pin().await?;
            let mut input = f.input(marker);
            input.skill_refs = vec![pin.clone()];
            input.foundation.skill_mode = AiSkillMode::OnDemand;
            input.foundation.retain_result = true;
            if marker == "skills-failure" {
                f.skills.read().await.toggle_skill(SKILL, USER, false).await?;
                denied_before_dispatch(f, input).await;
            } else if marker == "skills-boundary" {
                let (first, _) = response(server, marker, None, tool_sse("dgos_skill_read", "traversal",
                    json!({"skill_id":SKILL,"path":"../SKILL.md"}))).await;
                let run = run(f, input).await?;
                assert_ne!(run.terminal.status, AiRunStatus::Succeeded);
                assert!(result(f, &run.id).await?.skill_receipts.as_array().unwrap().is_empty());
                assert!(f.prompts.try_recv().is_err());
                first.assert_async().await;
            } else {
                let (first, initial) = response(server, marker, None,
                    tool_sse("dgos_skill_load", "load-selected-skill", json!({"skill_id":SKILL}))).await;
                let (second, delivered) = response(server, marker, Some("load-selected-skill"), text_sse("Pinned summary guidance read; approval remains pending.")).await;
                let run = run(f, input).await?;
                success(&run, "Pinned summary guidance read; approval remains pending.");
                let actual = tool_result(&request(&delivered), "load-selected-skill");
                let expected = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/official/writing/summarize/SKILL.md"))?;
                assert_eq!(actual["content"], expected);
                assert!(!request(&initial)["messages"].as_array().unwrap().iter().any(|message|
                    message["content"].as_str().is_some_and(|text| text.contains(&expected))));
                let stored = result(f, &run.id).await?;
                assert_eq!(stored.skill_receipts.as_array().unwrap().len(), 1);
                assert_eq!(stored.skill_receipts[0]["pin"], serde_json::to_value(&pin)?);
                assert_eq!(stored.skill_receipts[0]["path"], "SKILL.md");
                assert_eq!(stored.skill_receipts[0]["bytes"], expected.len());
                assert!(f.prompts.try_recv().is_err());
                first.assert_async().await;
                second.assert_async().await;
            }
        }
        "output-success" | "output-boundary" | "output-failure" => {
            let source = f.import("factual-source.txt", &b.source).await?;
            let mut input = f.input(marker);
            input.foundation.context_refs = vec![reference(&source, AiContextMode::Inline, 0, 8192)];
            input.foundation.output = AiOutputContract::JsonSchema { schema: b.schema.clone() };
            input.foundation.retain_result = marker != "output-boundary";
            let expected = b.expected_facts.to_string();
            let text = if marker == "output-failure" { json!({"release":"Friday","owner":"Lin","budget":"approved"}).to_string() } else { expected.clone() };
            let (mock, captured) = response(server, marker, None, text_sse(&text)).await;
            let original = run(f, input.clone()).await?;
            let content = result(f, &original.id).await?;
            if marker == "output-failure" {
                assert_ne!(original.terminal.status, AiRunStatus::Succeeded);
                assert!(!content.output_valid);
                let mut remote_schema = input;
                remote_schema.prompt = "remote-schema-forbidden".into();
                remote_schema.foundation.output = AiOutputContract::JsonSchema { schema: json!({"$ref":"https://example.invalid/schema"}) };
                denied_before_dispatch(f, remote_schema).await;
            } else {
                success(&original, &expected);
                assert!(content.output_valid);
                assert!(f.service.acknowledge_result(f.invoke(true), &original.id).await?);
                let acknowledged = result(f, &original.id).await?.acknowledged_at;
                assert!(acknowledged.is_some());
                assert!(f.service.acknowledge_result(f.invoke(true), &original.id).await?);
                assert_eq!(result(f, &original.id).await?.acknowledged_at, acknowledged);
                if marker == "output-boundary" {
                    assert!(content.content.is_none());
                    input.prompt = "unretained-continuation-forbidden".into();
                    input.foundation.source_run_id = Some(original.id.clone());
                    denied_before_dispatch(f, input).await;
                } else {
                    assert_eq!(content.content.as_deref(), Some(expected.as_str()));
                    assert_eq!(result(f, &original.id).await?.content, content.content);
                    explicit_continue(f, server, &original.id, &expected, "output-success-continuation").await?;
                }
            }
            assert!(request(&captured).to_string().contains(&b.source));
            mock.assert_async().await;
        }
        "lifecycle-success" => {
            let (mock, _) = response(server, marker, None, text_sse("Confirmed Friday release; budget unresolved.")).await;
            let mut input = f.input(marker);
            input.foundation.retain_result = true;
            let original = run(f, input).await?;
            success(&original, "Confirmed Friday release; budget unresolved.");
            explicit_continue(f, server, &original.id, "Confirmed Friday release; budget unresolved.", "lifecycle-success-continuation").await?;
            mock.assert_async().await;
        }
        "lifecycle-boundary" => {
            let (mock, _) = response(server, marker, None, text_sse("Saturation source.")).await;
            let mut input = f.input(marker);
            input.foundation.retain_result = true;
            let original = run(f, input).await?;
            success(&original, "Saturation source.");
            let entered = Arc::new(tokio::sync::Notify::new());
            let signal = entered.clone();
            let late = text_sse("Must not complete after explicit cancellation.");
            let active_mock = server.mock("POST", "/chat/completions").match_request(move |_| { signal.notify_one(); true })
                .with_header("content-type", "text/event-stream").with_chunked_body(move |writer| {
                    std::thread::sleep(Duration::from_secs(2));
                    std::io::Write::write_all(writer, late.as_bytes())
                }).expect(1).create_async().await;
            let call = f.router.admit_once(&f.route, &f.attachment, "active-saturated-run").unwrap();
            let active_id = call.context().request_id().to_owned();
            let invocation = AiInvocation::bind(call, &HostSession::for_user(USER), f.account.clone(), f.page.clone())?;
            let control = async {
                tokio::time::timeout(Duration::from_secs(5), entered.notified()).await?;
                let held: Vec<_> = (0..7).map(|_| f.invoke(false)).collect();
                assert!(f.router.admit_once(&f.route, &f.attachment, "ninth-execution").is_err());
                assert_eq!(result(f, &original.id).await?.content.as_deref(), Some("Saturation source."));
                let first = f.invoke(true);
                let second = f.invoke(true);
                assert!(f.router.admit_control_once(&f.route, &f.attachment, "third-control").is_err());
                drop(first);
                let reopened = f.invoke(true);
                drop((second, reopened));
                assert!(f.service.cancel(f.invoke(true), &active_id).await?);
                let id = "remember-this-control-id";
                let call = f.router.admit_control_once(&f.route, &f.attachment, id).unwrap();
                drop(call);
                assert!(f.router.admit_control_once(&f.route, &f.attachment, id).is_err());
                assert!(f.router.admit_once(&f.route, &f.attachment, id).is_err());
                drop(held);
                Ok::<_, Error>(())
            };
            let (cancelled, controlled) = tokio::join!(run_service(&f.service, invocation, f.input("lifecycle-boundary-active")), control);
            controlled?;
            let cancelled = cancelled?;
            assert_ne!(cancelled.terminal.status, AiRunStatus::Succeeded);
            assert_eq!(cancelled.terminal.error.as_ref().map(|error| error.reason.as_str()), Some("task_cancelled"));
            active_mock.assert_async().await;
            mock.assert_async().await;
        }
        "isolation-success" => {
            assert_eq!(chat_count(f).await?, 0);
            let resource = f.import("module-only.txt", &b.source).await?;
            let mut input = f.input(marker);
            input.foundation.retain_result = true;
            input.foundation.context_refs = vec![reference(&resource, AiContextMode::Inline, 0, 8192)];
            let (mock, _) = response(server, marker, None, text_sse("Module-owned fixed source only.")).await;
            let original = run(f, input).await?;
            success(&original, "Module-owned fixed source only.");
            assert_eq!(result(f, &original.id).await?.content.as_deref(), Some("Module-owned fixed source only."));
            assert!(f.repository.content_repository().get_run("foundation-other-user", &format!("module:{MODULE}"), &original.id).await?.is_none());
            assert!(f.repository.content_repository().resource("foundation-other-user", &format!("module:{MODULE}"), &resource.id).await?.is_none());
            assert!(f.repository.content_repository().get_run(USER, "module:other-module", &original.id).await?.is_none());
            assert_eq!(chat_count(f).await?, 0);
            mock.assert_async().await;
        }
        "isolation-boundary" => {
            assert!(f.router.admit_once(&f.route, "wrong-attachment", "foreign-attachment").is_err());
            let call = f.router.admit_once(&f.route, &f.attachment, "foreign-account").unwrap();
            assert!(AiInvocation::bind(call, &HostSession::for_user("foundation-other-user"), f.account.clone(), f.page.clone()).is_err());
            let call = f.router.admit_once(&f.route, &f.attachment, "dead-account").unwrap();
            let dead = CancellationToken::new();
            dead.cancel();
            assert!(AiInvocation::bind(call, &HostSession::for_user(USER), dead, f.page.clone()).is_err());
            let call = f.router.admit_once(&f.route, &f.attachment, "dead-page").unwrap();
            let dead = CancellationToken::new();
            dead.cancel();
            assert!(AiInvocation::bind(call, &HostSession::for_user(USER), f.account.clone(), dead).is_err());
        }
        "isolation-failure" => {
            let (mock, _) = response(server, marker, None, text_sse("Retained before revocation.")).await;
            let mut input = f.input(marker);
            input.foundation.retain_result = true;
            let original = run(f, input.clone()).await?;
            success(&original, "Retained before revocation.");
            f.permissions.revoke(USER, MODULE, ModulePermission::ProvidersCall).await?;
            assert!(f.service.get_result(f.invoke(true), &original.id).await.is_err());
            assert!(f.import("revoked.txt", &b.source).await.is_err());
            denied_before_dispatch(f, input).await;
            mock.assert_async().await;
        }
        _ => return Err(format!("unimplemented fixed baseline case: {marker}").into()),
    }
    Ok(())
}
async fn explicit_continue(f: &Fixture, server: &mut mockito::ServerGuard, source: &str, original: &str, marker: &str) -> Result<()> {
    let mut input = f.input(marker);
    input.foundation.retain_result = true;
    input.foundation.source_run_id = Some(source.into());
    assert!(f.service.inspect_context(f.invoke(false), input.clone()).await?.can_send);
    let (mock, captured) = response(server, marker, None, text_sse("Continued explicitly; budget remains pending.")).await;
    let continuation = run(f, input).await?;
    success(&continuation, "Continued explicitly; budget remains pending.");
    assert_ne!(continuation.id, source);
    assert!(request(&captured)["messages"].as_array().unwrap().iter().any(|message|
        message["content"].as_str().is_some_and(|text| text.contains(original))));
    let stored = result(f, &continuation.id).await?;
    assert_eq!(stored.source_run_id.as_deref(), Some(source));
    assert_eq!(result(f, source).await?.content.as_deref(), Some(original));
    mock.assert_async().await;
    Ok(())
}
async fn chat_count(f: &Fixture) -> Result<i64> {
    let row = f.db.query_one(Statement::from_string(DbBackend::Sqlite, "SELECT COUNT(*) AS n FROM chat_sessions".to_owned())).await?.ok_or("missing count row")?;
    Ok(row.try_get("", "n")?)
}

async fn crash_child(path: &Path, base_url: &str) -> Result<()> {
    let f = Fixture::open(path, base_url, "foundation-fixture-model", "synthetic-foundation-key").await?;
    let mut input = f.input("lifecycle-failure");
    input.foundation.retain_result = true;
    let sink: Arc<dyn AiEventSink> = Arc::new(|event: AiEvent| {
        if matches!(event.payload, AiEventPayload::Delta { .. }) {
            use std::io::Write;
            println!("CHECKPOINT_CANDIDATE {}", event.run_id);
            std::io::stdout().flush().expect("child control stdout");
        }
        true
    });
    f.service.run(f.invoke(false), input, sink).await?;
    Err("crash child unexpectedly completed before parent killed it".into())
}
struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}
async fn process_recovery(path: &Path, server: &mut mockito::ServerGuard) -> Result<()> {
    let first = format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":{"content":PARTIAL},"finish_reason":null}]}));
    let last = text_sse("This final text must not be observed as succeeded.");
    let mock = server.mock("POST", "/chat/completions").with_header("content-type", "text/event-stream")
        .with_chunked_body(move |writer| {
            std::io::Write::write_all(writer, first.as_bytes())?;
            std::io::Write::flush(writer)?;
            std::thread::sleep(Duration::from_secs(4));
            std::io::Write::write_all(writer, last.as_bytes())
        }).expect(1).create_async().await;
    let child = std::process::Command::new(std::env::current_exe()?).arg("--crash-child").arg(path).arg(server.url())
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::inherit()).spawn()?;
    let mut child = Child(child);
    // Blocking read has a bounded producer: the production run has its normal 120s deadline.
    // A separate reader task keeps the main runtime available for SQLite and the loopback stream.
    let stdout = child.0.stdout.take().ok_or("missing child stdout")?;
    let line = tokio::time::timeout(Duration::from_secs(20), tokio::task::spawn_blocking(move || {
        use std::io::BufRead;
        let mut line = String::new();
        std::io::BufReader::new(stdout).read_line(&mut line).map(|_| line)
    })).await???;
    let id = line.trim().strip_prefix("CHECKPOINT_CANDIDATE ").ok_or("child did not stream candidate")?.to_owned();
    let url = format!("sqlite://{}?mode=rw", path.join("smoke.sqlite").display());
    let db = dgos_db::establish_connection(&url).await?;
    let content = AiContentRepository::new(db.clone());
    let observed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(run) = content.get_run(USER, &format!("module:{MODULE}"), &id).await? {
                if run.content.as_deref() == Some(PARTIAL) {
                    assert_eq!(run.status, "running");
                    return Ok::<_, Error>(run);
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await??;
    assert_eq!(observed.run_id, id);
    child.0.kill()?;
    let killed = child.0.wait()?;
    assert!(!killed.success());
    drop(child);
    drop(content);
    db.close().await?;
    let f = Fixture::open(path, &server.url(), "foundation-fixture-model", "synthetic-foundation-key").await?;
    assert_eq!(f.service.recover_interrupted().await?, 1);
    assert_eq!(f.service.recover_interrupted().await?, 0, "recovery is idempotent");
    let recovered = result(&f, &id).await?;
    assert_eq!(recovered.status, "interrupted");
    assert_eq!(recovered.content.as_deref(), Some(PARTIAL));
    assert!(!recovered.output_valid);
    assert_eq!(f.service.get_run(f.invoke(true), &id).await?.ok_or("missing original audit")?.status, AiRunStatus::Interrupted);
    assert!(f.service.acknowledge_result(f.invoke(true), &id).await?);
    f.mcp.shutdown().await?;
    mock.assert_async().await;
    Ok(())
}
fn not_run(reason: &str) {
    println!("{}", json!({"evidence":"REAL_UPSTREAM","status":"NOT RUN","counted_as_pass":false,"reason":reason,"cost":"unknown"}));
}
async fn real_sample(b: &Baseline) -> Result<()> {
    let authorization = std::env::var("DGOS_FOUNDATION_REAL_AUTHORIZATION").unwrap_or_default();
    let endpoint = std::env::var("DGOS_FOUNDATION_REAL_BASE_URL").unwrap_or_default();
    let model = std::env::var("DGOS_FOUNDATION_REAL_MODEL").unwrap_or_default();
    let key = std::env::var("DGOS_FOUNDATION_REAL_API_KEY").unwrap_or_default();
    if authorization != AUTHORIZATION || endpoint.trim().is_empty() || model.trim().is_empty() || key.trim().is_empty() {
        not_run("missing explicit fixed-data/two-HTTP-attempt/128-output-token authorization, endpoint, model, or key; no real dispatch");
        return Ok(());
    }
    let url = url::Url::parse(&endpoint)?;
    if !matches!(url.scheme(), "https" | "http") || !url.username().is_empty() || url.password().is_some() {
        return Err("real endpoint must be HTTP(S), with no URL credentials".into());
    }
    // The production OpenAI-compatible adapter can negotiate a rejected parameter once.
    // Transient retries are explicitly disabled; authorization includes both HTTP attempts.
    let dir = tempfile::tempdir()?;
    let f = Fixture::open(dir.path(), &endpoint, &model, &key).await?;
    let resource = f.import("authorized-fixed-sample.txt", &b.source).await?;
    let mut input = f.input("Return only one JSON object with release, owner and budget. Use the supplied fixed source exactly: budget is pending, not approved. No extra facts.");
    input.generation.max_tokens = Some(128);
    input.foundation.context_refs = vec![reference(&resource, AiContextMode::Inline, 0, 8192)];
    input.foundation.output = AiOutputContract::JsonSchema { schema: b.schema.clone() };
    input.foundation.retain_result = true;
    let started = Instant::now();
    let sample = run(&f, input).await?;
    let actual = sample.terminal.content.as_deref().and_then(|text| serde_json::from_str::<Value>(text).ok());
    let correct = sample.terminal.status == AiRunStatus::Succeeded && actual.as_ref() == Some(&b.expected_facts)
        && result(&f, &sample.id).await?.output_valid;
    println!("{}", json!({"baseline":b.id,"evidence":"REAL_UPSTREAM","protocol":"openai_compatible",
        "coverage":"one context+structured-output fixed factual sample; no MCP or Skills",
        "status":if correct {"PASS"} else {"FAIL"},"counted_as_pass":correct,"max_logical_runs":1,"max_http_attempts":2,"max_output_tokens":128,
        "total_tokens":sample.terminal.total_tokens,"cost":"unknown","elapsed_ms":started.elapsed().as_millis(),
        "terminal_status":sample.terminal.status,"error_code":sample.terminal.error.as_ref().map(|error| error.code)}));
    f.mcp.shutdown().await?;
    not_run("remaining real baseline groups, Anthropic compatibility, browser/WebView, MCP effects and human quality comparison were not exercised");
    if !correct { return Err("real fixed factual JSON sample failed; raw prompt/reply/credentials are intentionally not printed".into()); }
    Ok(())
}
