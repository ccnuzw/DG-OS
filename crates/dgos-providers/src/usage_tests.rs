use super::*;
use dgos_db::{BeginGeneration, ChatRepository, ChatTarget, ChatUsageRepository};
use dgos_types::{ToolChatRequest, ToolMessage, ToolStreamEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

async fn owned_run(db: &DatabaseConnection, provider: &dgos_types::Provider) -> (String, ExecutionProvenance) {
    let chats = ChatRepository::new(db.clone());
    let session = chats.create_session("local", &ChatTarget {
        provider_id: Some(provider.id.clone()), model_id: Some("m".into()), no_skill: true,
        ..Default::default()
    }).await.unwrap();
    let run = uuid::Uuid::new_v4().to_string();
    chats.begin_generation(&BeginGeneration {
        user: "local", session_id: &session.id, task_id: &run,
        user_message_id: &uuid::Uuid::new_v4().to_string(),
        assistant_message_id: &uuid::Uuid::new_v4().to_string(), user_content: Some("hello"),
        regenerate_of: None, provider_id: &provider.id, model_id: "m",
        provider_config_revision: Some(provider.config_revision), endpoint_fingerprint: "fixture",
        skills: &[], generation: &Default::default(), tools: &[], assets: &[],
    }).await.unwrap();
    (session.id, ExecutionProvenance { caller: BuiltinCaller::Workbench, request_id: run })
}

fn tool_request() -> ToolChatRequest {
    ToolChatRequest { messages: vec![ToolMessage { role: "user".into(), content: Some("hello".into()),
        tool_call_id: None, tool_calls: vec![], reasoning: None, images: vec![] }],
        tools: vec![], max_tokens: 32, temperature: None, reasoning: None }
}

async fn provider_fixture(kind: &str, server: &mockito::ServerGuard) ->
    (tempfile::TempDir, DatabaseConnection, ProviderManager, dgos_types::Provider) {
    let (dir, db, vault, manager) = fixture().await;
    let provider = crate::test_support::seed_provider(db.clone(), &vault,
        typed(kind, &server.url(), SECRET, Some(json!({"max_retries":0}))), "local").await.unwrap();
    (dir, db, manager, provider)
}

#[tokio::test]
async fn all_protocols_measure_normal_tool_and_both_stream_paths_without_double_counting_subsets() {
    for kind in KINDS {
        let mut server = mockito::Server::new_async().await;
        let (_dir, db, manager, provider) = provider_fixture(kind, &server).await;
        let (session, provenance) = owned_run(&db, &provider).await;
        let path = if kind == "anthropic" { "/v1/messages" } else { "/chat/completions" };
        let usage = if kind == "anthropic" {
            json!({"input_tokens":10,"output_tokens":4,"cache_read_input_tokens":3,"cache_creation_input_tokens":2})
        } else {
            json!({"prompt_tokens":15,"completion_tokens":4,"total_tokens":19,
                "prompt_tokens_details":{"cached_tokens":3,"cache_creation_tokens":2},
                "completion_tokens_details":{"reasoning_tokens":1}})
        };
        let reply = if kind == "anthropic" {
            json!({"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":usage})
        } else {
            json!({"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":usage})
        };
        let sync = server.mock("POST", path).with_status(200).with_body(reply.to_string())
            .expect(2).create_async().await;
        let response = manager.call_provider_bound_with_provenance(&provider.id,"m",chat(Some(32),false),
            "local",None,Some(&provenance)).await.unwrap();
        assert!(response.usage.total_tokens_reported && !response.audit_warning);
        let response = manager.call_provider_tools_bound_with_provenance(&provider.id,"m",tool_request(),
            "local",None,Some(&provenance)).await.unwrap();
        assert_eq!(response.usage.total_tokens,19);
        sync.assert_async().await;
        sync.remove_async().await;
        let body = if kind == "anthropic" {
            sse(&[
                ("message_start",json!({"type":"message_start","message":{"id":"m","type":"message",
                    "role":"assistant","content":[],"usage":{"input_tokens":10,"output_tokens":1,
                    "cache_read_input_tokens":3,"cache_creation_input_tokens":2}}})),
                ("content_block_start",json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}})),
                ("content_block_delta",json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}})),
                ("content_block_stop",json!({"type":"content_block_stop","index":0})),
                ("message_delta",json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":4}})),
                ("message_stop",json!({"type":"message_stop"})),
            ])
        } else {
            format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}),
                json!({"choices":[],"usage":usage}))
        };
        let streams = server.mock("POST",path).with_status(200).with_header("content-type","text/event-stream")
            .with_body(body).expect(2).create_async().await;
        let mut stream = manager.stream_provider_bound_with_provenance(&provider.id,"m",chat(Some(32),true),
            "local",None,Some(&provenance)).await.unwrap();
        while let Some(chunk) = stream.next().await { chunk.unwrap(); }
        let mut stream = manager.stream_provider_tools_bound_cancellable_controlled_with_provenance(
            &provider.id,"m",tool_request(),"local",None,Some(&provenance),CancellationToken::new(),
            tokio::time::Instant::now()+Duration::from_secs(5),Arc::new(AtomicBool::new(false))).await.unwrap();
        let mut complete = false;
        while let Some(event) = stream.next().await {
            if let ToolStreamEvent::Complete { response,usage_unknown } = event.unwrap() {
                assert!(!usage_unknown && !response.audit_warning);
                assert_eq!(response.usage.total_tokens,19);
                complete = true;
            }
        }
        assert!(complete);
        streams.assert_async().await;
        let summary = ChatUsageRepository::new(db).summary("local",&session).await.unwrap();
        assert_eq!((summary.call_count,summary.reported_count,summary.pending_count),(4,4,0));
        assert_eq!((summary.total_tokens,summary.input_tokens,summary.output_tokens),(Some(76),Some(60),Some(16)));
        assert_eq!((summary.cache_read_tokens,summary.cache_write_tokens),(Some(12),Some(8)));
        assert_eq!(summary.cache_hit_rate,Some(0.2));
        assert_eq!(manager.list_usage_logs("local",None).await.unwrap().len(),4);
    }
}

async fn wait_for_queued_usage_delete(gate: &tokio::sync::RwLock<()>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while gate.try_read().is_ok() { tokio::task::yield_now().await; }
    }).await.expect("provider deletion must queue its exclusive admission guard");
}

#[tokio::test]
async fn provider_deletion_waits_for_inflight_http_usage_without_blocking_other_providers() {
    for mode in ["call", "tools", "stream", "tool_stream"] {
        let mut server = mockito::Server::new_async().await;
        let mut other_server = mockito::Server::new_async().await;
        let (_dir, db, vault, manager) = fixture().await;
        let provider = crate::test_support::seed_provider(db.clone(), &vault,
            typed("openai_compatible", &server.url(), SECRET, Some(json!({"max_retries":0}))),
            "local").await.unwrap();
        let other = crate::test_support::seed_provider(db.clone(), &vault,
            typed("openai_compatible", &other_server.url(), SECRET, Some(json!({"max_retries":0}))),
            "local").await.unwrap();
        let (session, provenance) = owned_run(&db, &provider).await;
        let streamed = matches!(mode, "stream" | "tool_stream");
        let body = if streamed {
            format!("data: {}\n\ndata: [DONE]\n\n", json!({
                "choices":[{"index":0,"delta":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":4,"completion_tokens":3,"total_tokens":7}
            }))
        } else { json!({"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":4,"completion_tokens":3,"total_tokens":7}}).to_string() };
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started_tx = std::sync::Mutex::new(Some(started_tx));
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = std::sync::Mutex::new(release_rx);
        let response = server.mock("POST", "/chat/completions").with_status(200)
            .with_header("content-type", if streamed { "text/event-stream" } else { "application/json" })
            .with_chunked_body(move |writer| {
                if let Some(tx) = started_tx.lock().unwrap().take() { let _ = tx.send(()); }
                release_rx.lock().unwrap().recv_timeout(Duration::from_secs(10))
                    .map_err(std::io::Error::other)?;
                std::io::Write::write_all(writer, body.as_bytes())
            }).expect(1).create_async().await;
        let other_response = other_server.mock("POST", "/chat/completions").with_status(200)
            .with_body(openai_reply("unrelated", 2, 1)).expect(1).create_async().await;
        let usage_gate = manager.provider_usage_gate_for_test("local", &provider.id);
        let running = {
            let manager = manager.clone();
            let provider = provider.clone();
            let provenance = provenance.clone();
            tokio::spawn(async move {
                match mode {
                    "call" => manager.call_provider_bound_with_provenance(&provider.id, "m",
                        chat(Some(32), false), "local", None, Some(&provenance)).await
                        .map(|response| (response.usage.total_tokens, response.audit_warning)),
                    "tools" => manager.call_provider_tools_bound_with_provenance(&provider.id, "m",
                        tool_request(), "local", None, Some(&provenance)).await
                        .map(|response| (response.usage.total_tokens, response.audit_warning)),
                    "stream" => {
                        let mut stream = manager.stream_provider_bound_with_provenance(&provider.id, "m",
                            chat(Some(32), true), "local", None, Some(&provenance)).await?;
                        let mut terminal = None;
                        while let Some(chunk) = stream.next().await {
                            let chunk = chunk?;
                            if chunk.finish_reason.is_some() {
                                terminal = Some((chunk.usage.unwrap().total_tokens, chunk.audit_warning));
                            }
                        }
                        Ok(terminal.expect("text stream must finish"))
                    }
                    "tool_stream" => {
                        let mut stream = manager.stream_provider_tools_bound_cancellable_controlled_with_provenance(
                            &provider.id, "m", tool_request(), "local", None, Some(&provenance),
                            CancellationToken::new(), tokio::time::Instant::now()+Duration::from_secs(5),
                            Arc::new(AtomicBool::new(false))).await?;
                        let mut terminal = None;
                        while let Some(event) = stream.next().await {
                            if let ToolStreamEvent::Complete { response, .. } = event? {
                                terminal = Some((response.usage.total_tokens, response.audit_warning));
                            }
                        }
                        Ok(terminal.expect("tool stream must finish"))
                    }
                    _ => unreachable!(),
                }
            })
        };
        tokio::time::timeout(Duration::from_secs(5), started_rx).await.unwrap().unwrap();
        let summary = ChatUsageRepository::new(db.clone()).summary("local", &session).await.unwrap();
        assert_eq!((summary.call_count, summary.pending_count), (1, 1), "{mode}");
        let deleting = {
            let manager = manager.clone();
            let id = provider.id.clone();
            tokio::spawn(async move { manager.delete_provider(&id, "local").await })
        };
        wait_for_queued_usage_delete(&usage_gate).await;
        assert!(!deleting.is_finished(), "deletion must wait for {mode}'s terminal transaction");
        assert!(manager.get_provider(&provider.id, "local").await.unwrap().is_some());
        let unrelated = tokio::time::timeout(Duration::from_secs(5), manager.call_provider(
            &other.id, "m", chat(Some(32), false), "local")).await.unwrap().unwrap();
        assert!(!unrelated.audit_warning, "deletion must not hold the global credentials lock while waiting");
        // The accepted mutation must also finish if its RPC waiter disappears while queued.
        if mode == "tools" { deleting.abort(); }
        release_tx.send(()).unwrap();
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), running).await.unwrap().unwrap().unwrap(),
            (7, false), "{mode}");
        if mode == "tools" {
            assert!(deleting.await.unwrap_err().is_cancelled());
            tokio::time::timeout(Duration::from_secs(5), manager.shutdown_provider_mutations()).await.unwrap().unwrap();
        } else {
            tokio::time::timeout(Duration::from_secs(5), deleting).await.unwrap().unwrap().unwrap();
        }
        assert!(manager.settle_chat_usage("local", &provenance.request_id).await);
        let summary = ChatUsageRepository::new(db).summary("local", &session).await.unwrap();
        assert_eq!((summary.total_tokens, summary.call_count, summary.reported_count, summary.pending_count),
            (Some(7), 1, 1, 0), "{mode}");
        assert!(manager.get_provider(&provider.id, "local").await.unwrap().is_none());
        let logs = manager.list_usage_logs("local", None).await.unwrap();
        assert_eq!(logs.len(), 1, "deleted Provider audit must be removed, not the chat fact");
        assert_eq!(logs[0].provider_id, other.id);
        response.assert_async().await;
        other_response.assert_async().await;
    }
}

#[tokio::test]
async fn terminal_audit_failure_rolls_back_fact_and_leaves_durable_unknown() {
    let mut server = mockito::Server::new_async().await;
    let (_dir, db, manager, provider) = provider_fixture("openai_compatible",&server).await;
    let (session, provenance) = owned_run(&db,&provider).await;
    db.execute(Statement::from_string(DbBackend::Sqlite,
        "CREATE TRIGGER reject_usage BEFORE INSERT ON provider_usage_logs BEGIN SELECT RAISE(ABORT,'fixture'); END".to_owned())).await.unwrap();
    let call = server.mock("POST","/chat/completions").with_status(200)
        .with_body(openai_reply("ok",4,3)).expect(1).create_async().await;
    let response = manager.call_provider_bound_with_provenance(&provider.id,"m",chat(Some(32),false),
        "local",None,Some(&provenance)).await.unwrap();
    assert!(response.audit_warning);
    let summary = ChatUsageRepository::new(db).summary("local",&session).await.unwrap();
    assert_eq!((summary.call_count,summary.reported_count,summary.pending_count),(1,0,1));
    assert_eq!(summary.total_tokens,None);
    assert!(manager.list_usage_logs("local",None).await.unwrap().is_empty());
    call.assert_async().await;
}

#[tokio::test]
async fn begin_is_durable_before_network_and_nonchat_provenance_cannot_claim_chat_run() {
    let mut server = mockito::Server::new_async().await;
    let (_dir,db,manager,provider) = provider_fixture("openai_compatible",&server).await;
    let (session,provenance) = owned_run(&db,&provider).await;
    let (tx,rx) = tokio::sync::oneshot::channel();
    let tx = std::sync::Mutex::new(Some(tx));
    let call = server.mock("POST","/chat/completions").with_chunked_body(move |writer| {
        if let Some(tx) = tx.lock().unwrap().take() { let _ = tx.send(()); }
        std::thread::sleep(Duration::from_millis(300));
        std::io::Write::write_all(writer,br#"{"choices":[],"usage":{"total_tokens":0}}"#)
    }).expect(2).create_async().await;
    let cancellation = CancellationToken::new();
    let dispatched = Arc::new(AtomicBool::new(false));
    let future = manager.call_provider_bound_cancellable_with_provenance(&provider.id,"m",chat(Some(32),false),
        "local",None,Some(&provenance),cancellation.clone(),tokio::time::Instant::now()+Duration::from_secs(5),dispatched.clone());
    tokio::pin!(future);
    tokio::select! { _ = rx => {}, result = &mut future => panic!("premature result: {result:?}") }
    assert!(dispatched.load(Ordering::Acquire));
    let repo = ChatUsageRepository::new(db.clone());
    let pending = repo.summary("local",&session).await.unwrap();
    assert_eq!((pending.call_count,pending.pending_count,pending.total_tokens),(1,1,None));
    cancellation.cancel();
    assert!(matches!(future.await,Err(DgosError::StreamInterrupted)));
    usage_rows(&manager).await;
    let settled = repo.summary("local",&session).await.unwrap();
    assert_eq!((settled.call_count,settled.reported_count,settled.pending_count),(1,0,0));
    let foreign = ExecutionProvenance { caller:BuiltinCaller::Module,request_id:provenance.request_id.clone() };
    manager.call_provider_bound_with_provenance(&provider.id,"m",chat(Some(32),false),
        "local",None,Some(&foreign)).await.unwrap();
    assert_eq!(repo.summary("local",&session).await.unwrap().call_count,1);
    assert_eq!(manager.list_usage_logs("local",None).await.unwrap().len(),2);
    call.assert_async().await;
}

#[tokio::test]
async fn failed_tool_response_keeps_reported_partial_facts_and_explicit_zero_is_known() {
    let mut server = mockito::Server::new_async().await;
    let (_dir,db,manager,provider) = provider_fixture("openai_compatible",&server).await;
    let (session,provenance) = owned_run(&db,&provider).await;
    let bad = server.mock("POST","/chat/completions").with_status(200)
        .with_body(json!({"choices":[],"usage":{"prompt_tokens":9,
            "prompt_tokens_details":{"cached_tokens":0}}}).to_string()).expect(1).create_async().await;
    assert!(manager.call_provider_tools_bound_with_provenance(&provider.id,"m",tool_request(),"local",
        None,Some(&provenance)).await.is_err());
    bad.assert_async().await;
    bad.remove_async().await;
    let zero = server.mock("POST","/chat/completions").with_status(200)
        .with_body(openai_reply("ok",0,0)).expect(1).create_async().await;
    manager.call_provider_bound_with_provenance(&provider.id,"m",chat(Some(32),false),"local",None,
        Some(&provenance)).await.unwrap();
    let summary = ChatUsageRepository::new(db.clone()).summary("local",&session).await.unwrap();
    assert_eq!((summary.call_count,summary.reported_count),(2,2));
    assert_eq!((summary.input_tokens,summary.output_tokens,summary.total_tokens),(Some(9),Some(0),Some(0)));
    assert_eq!((summary.coverage.input,summary.coverage.output,summary.coverage.total),(2,1,1));
    assert!(summary.partial);
    let rows = manager.list_usage_logs("local",None).await.unwrap();
    assert!(rows.iter().all(|row| row.usage_reported == Some(true)));
    zero.assert_async().await;
    zero.remove_async().await;
    let body = format!("data: {}\n\ndata: {}\n\n",
        json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"partial"},"finish_reason":null}],
            "usage":{"prompt_tokens":5,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":1}}}),
        json!({"error":{"type":"server_error"}}));
    let interrupted = server.mock("POST","/chat/completions").with_status(200)
        .with_header("content-type","text/event-stream").with_body(body).expect(2).create_async().await;
    let mut stream = manager.stream_provider_bound_with_provenance(&provider.id,"m",chat(Some(32),true),
        "local",None,Some(&provenance)).await.unwrap();
    assert!(stream.next().await.unwrap().is_ok());
    assert!(matches!(stream.next().await,Some(Err(DgosError::StreamInterrupted))));
    let mut stream = manager.stream_provider_tools_bound_cancellable_controlled_with_provenance(
        &provider.id,"m",tool_request(),"local",None,Some(&provenance),CancellationToken::new(),
        tokio::time::Instant::now()+Duration::from_secs(5),Arc::new(AtomicBool::new(false))).await.unwrap();
    assert!(matches!(stream.next().await,Some(Ok(ToolStreamEvent::Delta(_)))));
    assert!(matches!(stream.next().await,Some(Err(DgosError::StreamInterrupted))));
    let summary = ChatUsageRepository::new(db).summary("local",&session).await.unwrap();
    assert_eq!((summary.call_count,summary.reported_count),(4,4));
    assert_eq!((summary.input_tokens,summary.output_tokens,summary.total_tokens),(Some(19),Some(4),Some(14)));
    assert_eq!(summary.cache_read_tokens,Some(2));
    interrupted.assert_async().await;
}

#[tokio::test]
async fn retries_and_parameter_downgrades_create_independent_durable_attempts() {
    let mut server = mockito::Server::new_async().await;
    let (dir,db,vault,manager) = fixture().await;
    let provider = crate::test_support::seed_provider(db.clone(),&vault,typed("openai_compatible",
        &server.url(),SECRET,Some(json!({"max_retries":2,"initial_delay_ms":1,"max_delay_ms":2}))),"local").await.unwrap();
    let (session,provenance) = owned_run(&db,&provider).await;
    let failures = server.mock("POST","/chat/completions").with_status(503)
        .with_body(json!({"error":{"type":"server_error"},"usage":{"completion_tokens":2}}).to_string())
        .expect(3).create_async().await;
    assert!(matches!(manager.call_provider_bound_with_provenance(&provider.id,"m",chat(Some(32),false),
        "local",None,Some(&provenance)).await,Err(DgosError::MaxRetriesExceeded)));
    failures.assert_async().await;
    failures.remove_async().await;
    let summary = ChatUsageRepository::new(db.clone()).summary("local",&session).await.unwrap();
    assert_eq!((summary.call_count,summary.reported_count,summary.output_tokens),(3,3,Some(6)));
    assert_eq!((summary.total_tokens,summary.input_tokens),(None,None));
    let rejected = server.mock("POST","/chat/completions")
        .match_body(mockito::Matcher::PartialJson(json!({"max_tokens":32}))).with_status(400)
        .with_body(r#"{"error":{"param":"max_tokens","message":"unsupported max_tokens"}}"#)
        .expect(1).create_async().await;
    let success = server.mock("POST","/chat/completions")
        .match_body(mockito::Matcher::PartialJson(json!({"max_completion_tokens":32})))
        .with_status(200).with_body(openai_reply("ok",3,2)).expect(1).create_async().await;
    manager.call_provider_bound_with_provenance(&provider.id,"m",chat(Some(32),false),
        "local",None,Some(&provenance)).await.unwrap();
    let summary = ChatUsageRepository::new(db).summary("local",&session).await.unwrap();
    assert_eq!((summary.call_count,summary.reported_count,summary.pending_count),(5,4,0));
    assert_eq!((summary.total_tokens,summary.output_tokens),(Some(5),Some(8)));
    assert_eq!(manager.list_usage_logs("local",None).await.unwrap().len(),5);
    rejected.assert_async().await;
    success.assert_async().await;
    drop(dir);
}

#[tokio::test]
async fn failed_durable_begin_never_dispatches_or_creates_a_usage_audit() {
    let mut server = mockito::Server::new_async().await;
    let (_dir,db,manager,provider) = provider_fixture("openai_compatible",&server).await;
    let (session,provenance) = owned_run(&db,&provider).await;
    db.execute(Statement::from_string(DbBackend::Sqlite,
        "CREATE TRIGGER reject_attempt BEFORE INSERT ON chat_usage_attempts BEGIN SELECT RAISE(ABORT,'fixture'); END".to_owned())).await.unwrap();
    let call = server.mock("POST","/chat/completions").with_status(200)
        .with_body(openai_reply("must not dispatch",1,1)).expect(0).create_async().await;
    let dispatched = Arc::new(AtomicBool::new(false));
    let result = manager.call_provider_bound_cancellable_with_provenance(&provider.id,"m",chat(Some(32),false),
        "local",None,Some(&provenance),CancellationToken::new(),
        tokio::time::Instant::now()+Duration::from_secs(5),dispatched.clone()).await;
    assert!(matches!(result,Err(DgosError::DatabaseError(_))));
    assert!(!dispatched.load(Ordering::Acquire));
    assert_eq!(ChatUsageRepository::new(db).summary("local",&session).await.unwrap().call_count,0);
    assert!(manager.list_usage_logs("local",None).await.unwrap().is_empty());
    call.assert_async().await;
}

#[tokio::test]
async fn anthropic_incomplete_terminal_usage_keeps_durable_subtotals_without_claiming_a_total() {
    for (input_present,final_output_present) in [(true,false),(false,true)] {
        let mut server = mockito::Server::new_async().await;
        let (_dir,db,manager,provider) = provider_fixture("anthropic",&server).await;
        let (session,provenance) = owned_run(&db,&provider).await;
        let mut initial = json!({"output_tokens":0,"cache_read_input_tokens":3});
        if input_present { initial["input_tokens"] = json!(10); }
        let mut delta = json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}});
        if final_output_present { delta["usage"] = json!({"output_tokens":2}); }
        let body = sse(&[
            ("message_start",json!({"type":"message_start","message":{"id":"m","type":"message",
                "role":"assistant","content":[],"usage":initial}})),
            ("content_block_start",json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"ok"}})),
            ("content_block_stop",json!({"type":"content_block_stop","index":0})),
            ("message_delta",delta), ("message_stop",json!({"type":"message_stop"})),
        ]);
        let calls = server.mock("POST","/v1/messages").with_status(200)
            .with_header("content-type","text/event-stream").with_body(body).expect(2).create_async().await;
        let mut plain = manager.stream_provider_bound_with_provenance(&provider.id,"m",chat(Some(32),true),
            "local",None,Some(&provenance)).await.unwrap();
        let mut terminal = false;
        while let Some(chunk) = plain.next().await {
            let chunk = chunk.unwrap();
            if chunk.finish_reason.is_some() {
                let usage = chunk.usage.unwrap();
                assert!(!usage.total_tokens_reported && !chunk.audit_warning);
                assert_eq!(usage.prompt_tokens_reported,input_present);
                assert_eq!(usage.completion_tokens_reported,final_output_present);
                terminal = true;
            }
        }
        assert!(terminal);
        let mut tools = manager.stream_provider_tools_bound_cancellable_controlled_with_provenance(
            &provider.id,"m",tool_request(),"local",None,Some(&provenance),CancellationToken::new(),
            tokio::time::Instant::now()+Duration::from_secs(5),Arc::new(AtomicBool::new(false))).await.unwrap();
        let mut terminal = false;
        while let Some(event) = tools.next().await {
            if let ToolStreamEvent::Complete { response,usage_unknown } = event.unwrap() {
                assert!(usage_unknown && !response.usage.total_tokens_reported && !response.audit_warning);
                assert_eq!(response.usage.prompt_tokens_reported,input_present);
                assert_eq!(response.usage.completion_tokens_reported,final_output_present);
                terminal = true;
            }
        }
        assert!(terminal);
        assert!(manager.settle_chat_usage("local",&provenance.request_id).await);
        let summary = ChatUsageRepository::new(db).summary("local",&session).await.unwrap();
        assert_eq!((summary.call_count,summary.reported_count,summary.pending_count),(2,2,0));
        assert_eq!(summary.total_tokens,None);
        assert_eq!(summary.input_tokens,input_present.then_some(26));
        assert_eq!(summary.output_tokens,final_output_present.then_some(4));
        assert_eq!(summary.cache_read_tokens,Some(6));
        assert_eq!(summary.coverage.total,0);
        assert_eq!(summary.coverage.output,if final_output_present {2} else {0});
        assert!(summary.partial);
        calls.assert_async().await;
    }
}
