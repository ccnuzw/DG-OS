//! FR-006 stage 5 (AC22): MCP tools inside a chat session. Per-send selection, one proposal at a
//! time shown with its exact arguments, one-shot approval, schema drift, expiry/cancel while
//! parked, ownership, and the rule that tool round trips never enter chat storage or later
//! context (D022/D075). Uses the stdio MCP fixture from `dgos-mcp` and mocked Provider rounds;
//! it is not evidence about real Providers or real MCP servers.

use async_trait::async_trait;
use dgos_db::{
    migrations::Migrator,
    sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement},
    sea_orm_migration::MigratorTrait,
    ModelRepository,
};
use dgos_mcp::{
    CreateMcpServerRequest, McpCallPolicy, McpManager, McpTool, StdioConfig, TransportKind,
};
use dgos_providers::{
    test_support::{seed_provider, MemoryCredentialVault},
    ProviderManager,
};
use dgos_skills::SkillsManager;
use dgos_types::{Model, ProviderConfig};
use dgos_workbench::{
    AdvanceChatToolTurnInput, ChatError, ChatEvent, ChatManager, ChatSink, ChatTargetInput,
    CreateChatSessionInput, ListChatMessagesInput, SendChatInput, Sent, ToolAction, ToolSelection,
};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

const ALICE: &str = "alice";
const BOB: &str = "bob";
const ARGUMENT_MARKER: &str = "SENTINEL-TOOL-ARGUMENT-4e2b";
const FINAL_ANSWER: &str = "final answer after the tool";

struct Allow;
#[async_trait]
impl McpCallPolicy for Allow {
    async fn allows_call(&self, user: &str, _: &str, _: &str) -> bool {
        user == ALICE
    }
}

#[derive(Default)]
struct Collect(Mutex<Vec<ChatEvent>>);

impl Collect {
    fn take(&self) -> Vec<ChatEvent> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
    fn proposal(&self) -> Option<dgos_workbench::ChatToolProposal> {
        self.0.lock().unwrap().iter().find_map(|event| match event {
            ChatEvent::ToolProposal { proposal, .. } => Some(proposal.clone()),
            _ => None,
        })
    }
}

impl ChatSink for Collect {
    fn send(&self, event: ChatEvent) -> bool {
        self.0.lock().unwrap().push(event);
        true
    }
}

struct Fx {
    db: DatabaseConnection,
    chat: ChatManager,
    providers: ProviderManager,
    skills: SkillsManager,
    mcp: McpManager,
    tool: McpTool,
    provider_id: String,
    server: mockito::ServerGuard,
    _dir: tempfile::TempDir,
}

async fn fx() -> Fx {
    let server = mockito::Server::new_async().await;
    let dir = tempfile::tempdir().unwrap();
    let db_url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("fixture.db").display()
    );
    let db = dgos_db::establish_connection(&db_url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let vault = Arc::new(MemoryCredentialVault::default());
    let provider = seed_provider(
        db.clone(),
        &vault,
        ProviderConfig {
            name: "fixture".into(),
            provider_type: "openai_compatible".into(),
            base_url: server.url(),
            api_key: "synthetic-secret".into(),
            enabled: true,
            config: None,
        },
        ALICE,
    )
    .await
    .unwrap();
    ModelRepository::new(db.clone())
        .sync_models(
            &provider.id,
            vec![Model {
                id: uuid::Uuid::new_v4().to_string(),
                provider_id: provider.id.clone(),
                model_id: "chat".into(),
                model_name: "chat".into(),
                model_type: "chat".into(),
                context_window: Some(65536),
                max_output_tokens: Some(2048),
                supports_streaming: false,
                supports_vision: false,
                cached_at: String::new(),
                ..Default::default()
            }],
        )
        .await
        .unwrap();
    let mcp = McpManager::with_policy(db.clone(), Arc::new(Allow));
    let registered = mcp
        .create_server(
            ALICE,
            CreateMcpServerRequest {
                name: "fixture-server".into(),
                transport: TransportKind::Stdio,
                stdio: Some(StdioConfig {
                    executable: "node".into(),
                    args: vec![
                        format!(
                            "{}/../dgos-mcp/tests/fixtures/mcp-server.mjs",
                            env!("CARGO_MANIFEST_DIR")
                        ),
                        "normal".into(),
                    ],
                    cwd: None,
                }),
                http: None,
                enabled: Some(true),
                auto_connect: None,
            },
        )
        .await
        .unwrap();
    mcp.connect(ALICE, &registered.id).await.unwrap();
    let tool = mcp
        .list_tools(ALICE, &registered.id)
        .await
        .unwrap()
        .remove(0);
    Fx {
        chat: ChatManager::new(db.clone()),
        skills: SkillsManager::new(db.clone(), dir.path().join("skills")),
        providers: ProviderManager::with_vault(&db_url, vault).await.unwrap(),
        provider_id: provider.id,
        db,
        mcp,
        tool,
        server,
        _dir: dir,
    }
}

fn selection(fx: &Fx) -> ToolSelection {
    ToolSelection {
        server_id: fx.tool.server_id.clone(),
        tool_name: fx.tool.tool_name.clone(),
        schema_hash: fx.tool.schema_hash.clone(),
    }
}

fn is_followup(body: &Value) -> bool {
    body["messages"]
        .as_array()
        .and_then(|m| m.last())
        .is_some_and(|last| last["role"] == "tool")
}

/// First round proposes the call; the follow-up (ending in the tool result) answers. `answer:
/// false` means the provider must not be called a second time.
async fn script_rounds(fx: &mut Fx, answer: bool) -> (mockito::Mock, mockito::Mock) {
    script_rounds_answering(fx, answer.then_some(FINAL_ANSWER)).await
}

/// As [`script_rounds`], with the follow-up's final text chosen by the caller.
async fn script_rounds_answering(
    fx: &mut Fx,
    answer: Option<&str>,
) -> (mockito::Mock, mockito::Mock) {
    let proposal = json!({"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{
        "id":"call_1","type":"function","function":{"name":"dgos_tool_0","arguments":json!({"text":ARGUMENT_MARKER}).to_string()}
    }]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}});
    let first = fx
        .server
        .mock("POST", "/chat/completions")
        .match_request(|r| {
            !is_followup(&serde_json::from_slice::<Value>(r.body().unwrap()).unwrap())
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(proposal.to_string())
        .expect(1)
        .create_async()
        .await;
    let (status, body, hits) = if let Some(answer) = answer {
        (
            200,
            json!({"choices":[{"message":{"role":"assistant","content":answer},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":8,"completion_tokens":3,"total_tokens":11}}),
            1,
        )
    } else {
        (500, json!({"error":"unexpected second round"}), 0)
    };
    let second = fx
        .server
        .mock("POST", "/chat/completions")
        .match_request(|r| {
            is_followup(&serde_json::from_slice::<Value>(r.body().unwrap()).unwrap())
        })
        .with_status(status)
        .with_header("content-type", "application/json")
        .with_body(body.to_string())
        .expect(hits)
        .create_async()
        .await;
    (first, second)
}

async fn session(fx: &Fx) -> String {
    fx.chat
        .create_session(
            &fx.providers,
            &fx.skills,
            ALICE,
            CreateChatSessionInput {
                target: Some(ChatTargetInput {
                    provider_id: Some(fx.provider_id.clone()),
                    model_id: Some("chat".into()),
                    skill_id: None,
                    no_skill: true,
                }),
            },
        )
        .await
        .unwrap()
        .id
}

fn input(session: &str, text: &str, tools: Vec<ToolSelection>) -> SendChatInput {
    SendChatInput {
        session_id: session.into(),
        content: text.into(),
        client_request_id: uuid::Uuid::new_v4().to_string(),
        accept_destination: None,
        tools,
    }
}

async fn send(fx: &Fx, user: &str, input: SendChatInput) -> Result<Sent, ChatError> {
    fx.chat
        .send_with_mcp(
            &fx.providers,
            &fx.skills,
            Some(&fx.mcp),
            user,
            "main",
            input,
        )
        .await
}

/// Sends with the fixture tool and runs the turn up to its first stop (a parked proposal).
async fn start_turn(fx: &Fx, session: &str, text: &str) -> (String, Arc<Collect>) {
    let sent = send(fx, ALICE, input(session, text, vec![selection(fx)]))
        .await
        .unwrap();
    let job = sent.job.expect("a new turn");
    assert!(job.is_tool_turn());
    let sink = Arc::new(Collect::default());
    fx.chat
        .run_tool_turn(&fx.providers, &fx.mcp, job, sink.clone())
        .await;
    (sent.ack.task_id, sink)
}

async fn decide(fx: &Fx, user: &str, task_id: &str, action: ToolAction) -> Result<(), ChatError> {
    let job = fx
        .chat
        .begin_tool_action(
            user,
            AdvanceChatToolTurnInput {
                task_id: task_id.into(),
                action,
            },
        )
        .await?;
    fx.chat.run_tool_action(&fx.providers, &fx.mcp, job).await;
    Ok(())
}

async fn until(mut condition: impl FnMut() -> bool) {
    for _ in 0..300 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached in time");
}

async fn scalar(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index::<i64>(0)
        .unwrap()
}

/// Every cell of every table, as text.
async fn dump(db: &DatabaseConnection) -> String {
    let mut out = String::new();
    let tables = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
                .to_owned(),
        ))
        .await
        .unwrap();
    for table in tables {
        let table: String = table.try_get("", "name").unwrap();
        let columns = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT name FROM pragma_table_info('{table}')"),
            ))
            .await
            .unwrap();
        let names: Vec<String> = columns
            .iter()
            .map(|c| c.try_get("", "name").unwrap())
            .collect();
        let select = names
            .iter()
            .map(|n| format!("CAST(\"{n}\" AS TEXT)"))
            .collect::<Vec<_>>()
            .join(" || ' ' || ");
        let rows = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT {select} AS line FROM \"{table}\""),
            ))
            .await
            .unwrap();
        for row in rows {
            out.push_str(
                &row.try_get::<Option<String>>("", "line")
                    .unwrap()
                    .unwrap_or_default(),
            );
            out.push('\n');
        }
    }
    out
}

async fn messages(fx: &Fx, session: &str) -> Vec<dgos_db::ChatMessage> {
    fx.chat
        .list_messages(
            ALICE,
            ListChatMessagesInput {
                session_id: session.into(),
                before_seq: None,
                limit: Some(100),
            },
        )
        .await
        .unwrap()
        .items
}

fn terminal(events: &[ChatEvent]) -> &ChatEvent {
    let last = events.last().expect("events");
    assert!(
        matches!(last, ChatEvent::Done { .. } | ChatEvent::Error { .. }),
        "ends with a terminal event, got {last:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ChatEvent::Done { .. } | ChatEvent::Error { .. }))
            .count(),
        1
    );
    last
}

#[tokio::test]
async fn an_approved_tool_turn_saves_only_the_final_answer_and_never_enters_later_context() {
    let mut fx = fx().await;
    let s = session(&fx).await;
    let (first, second) = script_rounds(&mut fx, true).await;
    let (task, sink) = start_turn(&fx, &s, "please use the tool").await;

    let proposal = sink.proposal().expect("the model's proposal is shown");
    assert_eq!(proposal.tool_name, fx.tool.tool_name);
    assert_eq!(
        proposal.arguments["text"], ARGUMENT_MARKER,
        "the user sees the exact arguments"
    );
    assert!(
        !sink
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, ChatEvent::Delta { .. })),
        "tool turns do not stream"
    );
    assert_eq!(
        messages(&fx, &s).await[1].status,
        "generating",
        "the reply waits for the decision"
    );

    // Only the proposal's own identity is accepted.
    let wrong = decide(
        &fx,
        ALICE,
        &task,
        ToolAction::Approve {
            proposal_id: "not-the-proposal".into(),
        },
    )
    .await;
    assert_eq!(wrong.err(), Some(ChatError::ToolTurnInvalid));
    assert_eq!(
        decide(&fx, ALICE, &task, ToolAction::RunModel).await.err(),
        Some(ChatError::ToolTurnInvalid)
    );
    assert_eq!(
        decide(
            &fx,
            BOB,
            &task,
            ToolAction::Approve {
                proposal_id: proposal.id.clone()
            }
        )
        .await
        .err(),
        Some(ChatError::ToolTurnInvalid),
        "a stranger cannot decide"
    );

    decide(
        &fx,
        ALICE,
        &task,
        ToolAction::Approve {
            proposal_id: proposal.id.clone(),
        },
    )
    .await
    .unwrap();
    let events = sink.take();
    let ChatEvent::Done { message } = terminal(&events) else {
        panic!("done")
    };
    assert_eq!(
        (message.content.as_str(), message.status.as_str()),
        (FINAL_ANSWER, "completed")
    );
    assert_eq!(
        decide(
            &fx,
            ALICE,
            &task,
            ToolAction::Approve {
                proposal_id: proposal.id
            }
        )
        .await
        .err(),
        Some(ChatError::ToolTurnInvalid),
        "an approval is consumed by use"
    );
    first.assert_async().await;
    second.assert_async().await;

    let stored = messages(&fx, &s).await;
    assert_eq!(
        stored
            .iter()
            .map(|m| (m.role.as_str(), m.content.as_str()))
            .collect::<Vec<_>>(),
        [("user", "please use the tool"), ("assistant", FINAL_ANSWER)]
    );
    assert!(
        !dump(&fx.db).await.contains(ARGUMENT_MARKER),
        "tool arguments are stored nowhere"
    );
    assert_eq!(
        scalar(
            &fx.db,
            &format!("SELECT COUNT(*) FROM workbench_task_tools WHERE task_id = '{task}'")
        )
        .await,
        1,
        "the audit keeps the server/tool/schema reference"
    );
    assert_eq!(scalar(&fx.db, &format!("SELECT COUNT(*) FROM workbench_tasks WHERE id = '{task}' AND kind = 'chat' AND status = 'succeeded'")).await, 1);

    // The next plain turn carries the question and answer, not the tool round trip.
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let recorder = seen.clone();
    let _stream = fx
        .server
        .mock("POST", "/chat/completions")
        .match_request(move |r| {
            recorder.lock().unwrap().push(serde_json::from_slice(r.body().unwrap()).unwrap());
            true
        })
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body("data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
        .create_async()
        .await;
    let plain = send(&fx, ALICE, input(&s, "plain follow-up", vec![]))
        .await
        .unwrap();
    let plain_sink = Collect::default();
    fx.chat
        .run_generation(&fx.providers, plain.job.unwrap(), &plain_sink)
        .await;
    assert!(matches!(
        terminal(&plain_sink.take()),
        ChatEvent::Done { .. }
    ));
    let bodies = seen.lock().unwrap();
    let body = bodies.last().expect("the plain turn reached the provider");
    let text = body["messages"].to_string();
    assert!(
        text.contains(FINAL_ANSWER) && text.contains("please use the tool"),
        "the earlier question and answer are context"
    );
    assert!(
        !text.contains(ARGUMENT_MARKER) && !text.contains("\"role\":\"tool\""),
        "no tool round trip in later context"
    );
    assert!(
        body.get("tools").is_none(),
        "tools are not carried over to the next send"
    );
}

#[tokio::test]
async fn denial_ends_the_turn_without_calling_the_model_again() {
    let mut fx = fx().await;
    let s = session(&fx).await;
    let (_, second) = script_rounds(&mut fx, false).await;
    let (task, sink) = start_turn(&fx, &s, "use the tool").await;
    let proposal = sink.proposal().unwrap();
    decide(
        &fx,
        ALICE,
        &task,
        ToolAction::Deny {
            proposal_id: proposal.id,
        },
    )
    .await
    .unwrap();
    let events = sink.take();
    let ChatEvent::Error {
        error_code,
        message,
        ..
    } = terminal(&events)
    else {
        panic!("error")
    };
    assert_eq!(error_code, "tool_denied");
    assert_eq!(message.as_ref().unwrap().status, "failed");
    second.assert_async().await;
    assert_eq!(scalar(&fx.db, &format!("SELECT COUNT(*) FROM workbench_tasks WHERE id = '{task}' AND status = 'failed' AND error_code = 'tool_denied'")).await, 1);
    assert!(!dump(&fx.db).await.contains(ARGUMENT_MARKER));
    // The user can start again right away.
    let again = send(&fx, ALICE, input(&s, "plain follow-up", vec![])).await;
    assert!(again.is_ok(), "the denied turn released the user's slot");
}

#[tokio::test]
async fn a_final_answer_over_the_reply_limit_is_cut_like_a_stream() {
    // D079: the 64 KiB reply limit (D074, B06-31) also bounds a tool turn's final answer. The
    // answer is multibyte so the cut must land on a character boundary below the limit.
    const LIMIT: usize = 64 * 1024;
    let oversized = "中".repeat(LIMIT / 3 + 100);
    assert!(oversized.len() > LIMIT);
    let mut fx = fx().await;
    let s = session(&fx).await;
    let (_, second) = script_rounds_answering(&mut fx, Some(&oversized)).await;
    let (task, sink) = start_turn(&fx, &s, "use the tool").await;
    let proposal = sink.proposal().unwrap();
    decide(
        &fx,
        ALICE,
        &task,
        ToolAction::Approve {
            proposal_id: proposal.id,
        },
    )
    .await
    .unwrap();
    second.assert_async().await;
    let events = sink.take();
    let ChatEvent::Error {
        error_code,
        message,
        ..
    } = terminal(&events)
    else {
        panic!("an oversized answer ends as a limit, not as done")
    };
    assert_eq!(error_code, "generation_limit");
    let message = message.as_ref().unwrap();
    assert_eq!(message.status, "interrupted");
    assert!(message.content.len() <= LIMIT, "{}", message.content.len());
    assert!(
        message.content.len() > LIMIT - 3,
        "cut at the boundary nearest the limit"
    );
    assert!(oversized.starts_with(message.content.as_str()));
    let stored = &messages(&fx, &s).await[1];
    assert_eq!(
        (stored.status.as_str(), stored.content.len()),
        ("interrupted", message.content.len())
    );
    assert_eq!(scalar(&fx.db, &format!("SELECT COUNT(*) FROM workbench_tasks WHERE id = '{task}' AND status = 'interrupted' AND error_code = 'generation_limit'")).await, 1);
}

#[tokio::test]
async fn stopping_a_parked_turn_finishes_it_once_and_revokes_the_proposal() {
    let mut fx = fx().await;
    let s = session(&fx).await;
    let (_, second) = script_rounds(&mut fx, false).await;
    let (task, sink) = start_turn(&fx, &s, "use the tool").await;
    let proposal = sink.proposal().unwrap();
    assert_eq!(
        send(&fx, ALICE, input(&s, "second", vec![])).await.err(),
        Some(ChatError::GenerationInProgress),
        "a parked proposal still holds the user's single generation"
    );
    assert!(fx.chat.cancel(ALICE, &task));
    assert!(!fx.chat.cancel(BOB, &task), "a stranger cannot stop it");
    until(|| !fx.chat.has_active_session(ALICE, &s)).await;
    let events = sink.take();
    let ChatEvent::Error {
        error_code,
        message,
        ..
    } = terminal(&events)
    else {
        panic!("error")
    };
    assert_eq!(error_code, "task_cancelled");
    assert_eq!(message.as_ref().unwrap().status, "failed");
    assert_eq!(
        decide(
            &fx,
            ALICE,
            &task,
            ToolAction::Approve {
                proposal_id: proposal.id
            }
        )
        .await
        .err(),
        Some(ChatError::ToolTurnInvalid),
        "a revoked proposal cannot be approved"
    );
    second.assert_async().await;
    assert_eq!(scalar(&fx.db, &format!("SELECT COUNT(*) FROM workbench_tasks WHERE id = '{task}' AND status = 'interrupted' AND error_code = 'task_cancelled'")).await, 1);
    assert!(
        !fx.chat.cancel(ALICE, &task),
        "a finished task answers false"
    );
}

#[tokio::test]
async fn destroying_the_window_or_shutting_down_ends_a_parked_turn() {
    for shutdown in [false, true] {
        let mut fx = fx().await;
        let s = session(&fx).await;
        let (_, second) = script_rounds(&mut fx, false).await;
        let (task, sink) = start_turn(&fx, &s, "use the tool").await;
        if shutdown {
            assert!(
                fx.chat.shutdown().await,
                "the terminal transaction committed inside the exit window"
            );
        } else {
            assert_eq!(fx.chat.cancel_window("main"), 1);
            until(|| !fx.chat.has_active_session(ALICE, &s)).await;
        }
        let events = sink.take();
        let ChatEvent::Error { error_code, .. } = terminal(&events) else {
            panic!("error")
        };
        assert_eq!(
            error_code,
            if shutdown {
                "process_interrupted"
            } else {
                "task_cancelled"
            }
        );
        second.assert_async().await;
        assert_eq!(scalar(&fx.db, &format!("SELECT COUNT(*) FROM workbench_tasks WHERE id = '{task}' AND status = 'interrupted'")).await, 1);
    }
}

#[tokio::test]
async fn selections_are_checked_on_every_send_before_anything_is_written() {
    let fx = fx().await;
    let s = session(&fx).await;
    let drifted = ToolSelection {
        schema_hash: "0".repeat(64),
        ..selection(&fx)
    };
    assert_eq!(
        send(&fx, ALICE, input(&s, "x", vec![drifted])).await.err(),
        Some(ChatError::ToolTurnInvalid),
        "schema drift is refused"
    );
    let unknown = ToolSelection {
        tool_name: "no_such_tool".into(),
        ..selection(&fx)
    };
    assert_eq!(
        send(&fx, ALICE, input(&s, "x", vec![unknown])).await.err(),
        Some(ChatError::ToolTurnInvalid)
    );
    let duplicate = vec![selection(&fx), selection(&fx)];
    assert_eq!(
        send(&fx, ALICE, input(&s, "x", duplicate)).await.err(),
        Some(ChatError::ToolTurnInvalid)
    );
    let nine: Vec<_> = (0..9).map(|_| selection(&fx)).collect();
    assert_eq!(
        send(&fx, ALICE, input(&s, "x", nine)).await.err(),
        Some(ChatError::ToolTurnInvalid),
        "at most eight tools"
    );
    // Without an MCP manager the host cannot honour a tool request, and says so.
    let no_mcp = fx
        .chat
        .send(
            &fx.providers,
            &fx.skills,
            ALICE,
            "main",
            input(&s, "x", vec![selection(&fx)]),
        )
        .await;
    assert_eq!(no_mcp.err(), Some(ChatError::ToolTurnInvalid));
    // Another user cannot use this user's server.
    let bob_session = fx
        .chat
        .create_session(
            &fx.providers,
            &fx.skills,
            BOB,
            CreateChatSessionInput::default(),
        )
        .await
        .unwrap();
    assert!(
        send(&fx, BOB, input(&bob_session.id, "x", vec![selection(&fx)]))
            .await
            .is_err()
    );
    assert_eq!(
        scalar(
            &fx.db,
            "SELECT COUNT(*) FROM workbench_tasks WHERE kind = 'chat'"
        )
        .await,
        0,
        "nothing was written"
    );
    assert!(messages(&fx, &s).await.is_empty());
}

#[tokio::test]
async fn a_plain_send_after_a_tool_turn_needs_no_reselection_and_carries_no_authorization() {
    let mut fx = fx().await;
    let s = session(&fx).await;
    let (_, _second) = script_rounds(&mut fx, true).await;
    let (task, sink) = start_turn(&fx, &s, "use the tool").await;
    let id = sink.proposal().unwrap().id;
    decide(&fx, ALICE, &task, ToolAction::Approve { proposal_id: id })
        .await
        .unwrap();
    let sent = send(&fx, ALICE, input(&s, "plain", vec![])).await.unwrap();
    let job = sent.job.unwrap();
    assert!(
        !job.is_tool_turn(),
        "the previous selection is not remembered"
    );
    assert_eq!(
        scalar(
            &fx.db,
            &format!(
                "SELECT COUNT(*) FROM workbench_task_tools WHERE task_id = '{}'",
                sent.ack.task_id
            )
        )
        .await,
        0
    );
}
