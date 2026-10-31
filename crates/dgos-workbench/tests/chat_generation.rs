//! FR-006 stage 2 backend: streaming, cancellation, terminal states, idempotency, concurrency,
//! snapshots, context budget, regeneration, recovery, isolation and redaction.
//!
//! The upstream is a local TCP server speaking just enough HTTP/SSE for scripted scenarios. It
//! proves the host's behaviour against these scripts; it is not evidence about real Providers.

use dgos_db::{
    migrations::Migrator,
    sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement},
    sea_orm_migration::MigratorTrait,
    BeginGeneration, ChatMessage, ChatRepository, ChatSession, FinalizeGeneration,
};
use dgos_providers::{
    test_support::{seed_provider, MemoryCredentialVault},
    ProviderManager,
};
use dgos_skills::SkillsManager;
use dgos_types::ProviderConfig;
use dgos_workbench::{
    ChatError, ChatEvent, ChatManager, ChatSink, ChatTargetInput, CreateChatSessionInput,
    DeleteChatSessionInput, ListChatMessagesInput, RegenerateChatInput, RenameChatSessionInput,
    SearchChatSessionsInput, SendAck, SendChatInput, Sent, UpdateChatSessionTargetInput,
    WorkbenchManager,
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

// ---------------------------------------------------------------------------------------------
// Scripted upstream

#[derive(Clone)]
enum Step {
    Send(String),
    Hold,
    Close,
}

fn sse(value: Value) -> Step {
    Step::Send(format!("data: {value}\n\n"))
}
fn delta(text: &str) -> Step {
    sse(json!({"choices":[{"delta":{"content":text},"finish_reason":null}]}))
}
fn reasoning(text: &str) -> Step {
    sse(json!({"choices":[{"delta":{"reasoning_content":text},"finish_reason":null}]}))
}
fn finish() -> Step {
    sse(json!({"choices":[{"delta":{},"finish_reason":"stop"}]}))
}
fn usage() -> Step {
    sse(json!({"choices":[],"usage":{"prompt_tokens":11,"completion_tokens":7,"total_tokens":18}}))
}
fn done() -> Step {
    Step::Send("data: [DONE]\n\n".into())
}
fn reply(text: &str) -> Vec<Step> {
    vec![delta(text), finish(), usage(), done()]
}

struct Upstream {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<Value>>>,
    scripts: Arc<Mutex<VecDeque<Vec<Step>>>>,
    open: Arc<AtomicUsize>,
}

impl Upstream {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let scripts: Arc<Mutex<VecDeque<Vec<Step>>>> = Arc::new(Mutex::new(VecDeque::new()));
        let open = Arc::new(AtomicUsize::new(0));
        let (r, s, o) = (requests.clone(), scripts.clone(), open.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let (r, s, o) = (r.clone(), s.clone(), o.clone());
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let header_end = loop {
                        let n = socket.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..n]);
                        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                            break at + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buffer[..header_end]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    while buffer.len() < header_end + length {
                        let n = socket.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..n]);
                    }
                    let body = serde_json::from_slice(&buffer[header_end..header_end + length])
                        .unwrap_or(Value::Null);
                    r.lock().unwrap().push(body);
                    let steps = s
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or_else(|| reply("default reply"));
                    o.fetch_add(1, Ordering::SeqCst);
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
                        )
                        .await;
                    for step in steps {
                        match step {
                            Step::Send(text) => {
                                if socket.write_all(text.as_bytes()).await.is_err() {
                                    break;
                                }
                                let _ = socket.flush().await;
                            }
                            Step::Hold => {
                                // Returns when the client drops the connection.
                                while matches!(socket.read(&mut chunk).await, Ok(n) if n > 0) {}
                                break;
                            }
                            Step::Close => break,
                        }
                    }
                    o.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        Self {
            addr,
            requests,
            scripts,
            open,
        }
    }

    fn script(&self, steps: Vec<Step>) {
        self.scripts.lock().unwrap().push_back(steps);
    }
    fn hits(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    fn body(&self, index: usize) -> Value {
        self.requests.lock().unwrap()[index].clone()
    }
    fn open(&self) -> usize {
        self.open.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------------------------
// Fixture

struct Fx {
    db: DatabaseConnection,
    chat: ChatManager,
    providers: ProviderManager,
    skills: SkillsManager,
    workbench: WorkbenchManager,
    up: Upstream,
    provider_id: String,
    vault: Arc<MemoryCredentialVault>,
    db_path: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

const ALICE: &str = "alice";
const BOB: &str = "bob";

async fn fx() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let db = dgos_db::establish_connection(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let up = Upstream::start().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let provider = seed_provider(
        db.clone(),
        &vault,
        ProviderConfig {
            name: "Loopback".into(),
            provider_type: "openai_compatible".into(),
            base_url: format!("http://{}/v1", up.addr),
            api_key: "test-only-secret".into(),
            enabled: true,
            config: None,
        },
        ALICE,
    )
    .await
    .unwrap();
    let providers = ProviderManager::with_vault(&url, vault.clone())
        .await
        .unwrap();
    providers
        .add_manual_model(&provider.id, ALICE, "chat-model", Some("text-generation"))
        .await
        .unwrap();
    Fx {
        chat: ChatManager::new(db.clone()),
        workbench: WorkbenchManager::new(db.clone()),
        skills: SkillsManager::new(db.clone(), dir.path().join("skills")),
        providers,
        up,
        provider_id: provider.id,
        vault,
        db,
        db_path,
        _dir: dir,
    }
}

#[derive(Default)]
struct Collect {
    events: Mutex<Vec<ChatEvent>>,
    close_after_delta: bool,
}

impl Collect {
    fn closing() -> Self {
        Self {
            events: Mutex::default(),
            close_after_delta: true,
        }
    }
    fn deltas(&self) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, ChatEvent::Delta { .. }))
            .count()
    }
    fn take(&self) -> Vec<ChatEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
}

impl ChatSink for Collect {
    fn send(&self, event: ChatEvent) -> bool {
        let is_delta = matches!(event, ChatEvent::Delta { .. });
        self.events.lock().unwrap().push(event);
        !(self.close_after_delta && is_delta)
    }
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

fn target(fx: &Fx) -> ChatTargetInput {
    ChatTargetInput {
        provider_id: Some(fx.provider_id.clone()),
        model_id: Some("chat-model".into()),
        skill_id: None,
        no_skill: true,
    }
}

async fn session(fx: &Fx, user: &str) -> ChatSession {
    fx.chat
        .create_session(
            &fx.providers,
            &fx.skills,
            user,
            CreateChatSessionInput {
                target: Some(target(fx)),
            },
        )
        .await
        .unwrap()
}

fn send_input(session: &str, content: &str) -> SendChatInput {
    SendChatInput {
        session_id: session.into(),
        content: content.into(),
        client_request_id: uuid::Uuid::new_v4().to_string(),
        accept_destination: None,
        tools: vec![],
    }
}

async fn start(fx: &Fx, user: &str, input: SendChatInput) -> Result<Sent, ChatError> {
    fx.chat
        .send(&fx.providers, &fx.skills, user, "main", input)
        .await
}

async fn turn(fx: &Fx, user: &str, session: &str, text: &str) -> (SendAck, Vec<ChatEvent>) {
    let sent = start(fx, user, send_input(session, text)).await.unwrap();
    let sink = Collect::default();
    if let Some(job) = sent.job {
        fx.chat.run_generation(&fx.providers, job, &sink).await;
    }
    (sent.ack, sink.take())
}

async fn regenerate(
    fx: &Fx,
    user: &str,
    message_id: &str,
    request_id: Option<String>,
) -> Result<(SendAck, Vec<ChatEvent>), ChatError> {
    let sent = fx
        .chat
        .regenerate(
            &fx.providers,
            &fx.skills,
            user,
            "main",
            RegenerateChatInput {
                message_id: message_id.into(),
                client_request_id: request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                accept_destination: None,
            },
        )
        .await?;
    let sink = Collect::default();
    if let Some(job) = sent.job {
        fx.chat.run_generation(&fx.providers, job, &sink).await;
    }
    Ok((sent.ack, sink.take()))
}

async fn messages(fx: &Fx, user: &str, session: &str) -> Vec<ChatMessage> {
    fx.chat
        .list_messages(
            user,
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

async fn scalar(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index::<i64>(0)
        .unwrap()
}

/// Every `table.column` whose text contains `needle`.
async fn leaks(db: &DatabaseConnection, needle: &str, include_chat: bool) -> Vec<String> {
    let mut found = Vec::new();
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
        if !include_chat && table.starts_with("chat_") {
            continue;
        }
        let columns = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT name FROM pragma_table_info('{table}')"),
            ))
            .await
            .unwrap();
        for column in columns {
            let column: String = column.try_get("", "name").unwrap();
            let hits = scalar(
                db,
                &format!("SELECT COUNT(*) FROM \"{table}\" WHERE CAST(\"{column}\" AS TEXT) LIKE '%{needle}%'"),
            )
            .await;
            if hits > 0 {
                found.push(format!("{table}.{column}"));
            }
        }
    }
    found
}

fn terminal(events: &[ChatEvent]) -> &ChatEvent {
    let last = events.last().expect("events");
    assert!(
        matches!(last, ChatEvent::Done { .. } | ChatEvent::Error { .. }),
        "stream must end with a terminal event"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ChatEvent::Done { .. } | ChatEvent::Error { .. }))
            .count(),
        1,
        "exactly one terminal event"
    );
    last
}

fn expect_error(event: &ChatEvent) -> (&Option<ChatMessage>, &str, bool, bool) {
    match event {
        ChatEvent::Error {
            message,
            error_code,
            result_unknown,
            regeneration_discarded,
            ..
        } => (
            message,
            error_code.as_str(),
            *result_unknown,
            *regeneration_discarded,
        ),
        other => panic!("expected an error terminal event, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// AC03/AC04/AC05/AC07: streaming, persistence, context

#[tokio::test]
async fn a_streamed_reply_is_saved_with_the_snapshot_and_feeds_the_next_turn() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    fx.up.script(vec![
        reasoning("THINKING-NEVER-STORED"),
        delta("Hello "),
        delta("world"),
        finish(),
        usage(),
        done(),
    ]);
    let (ack, events) = turn(&fx, ALICE, &s.id, "first question").await;
    assert!(!ack.replayed);
    assert!(matches!(&events[0], ChatEvent::Started { task_id, .. } if *task_id == ack.task_id));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ChatEvent::ReasoningDelta { .. })),
        "reasoning is shown live"
    );
    let joined: String = events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::Delta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(joined, "Hello world");
    let ChatEvent::Done { message } = terminal(&events) else {
        panic!("done")
    };
    assert_eq!(
        (message.content.as_str(), message.status.as_str()),
        ("Hello world", "completed")
    );
    assert_eq!(
        (message.total_tokens, message.id.as_str()),
        (Some(18), ack.assistant_message_id.as_str())
    );

    let stored = messages(&fx, ALICE, &s.id).await;
    assert_eq!(stored.len(), 2);
    assert_eq!(
        (stored[0].role.as_str(), stored[0].content.as_str()),
        ("user", "first question")
    );
    let first = fx.up.body(0);
    assert_eq!(first["stream"], true);
    assert_eq!(first["max_tokens"], 1024);
    let roles: Vec<_> = first["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        roles,
        ["system", "user"],
        "only the host supplies the system role"
    );

    // Second turn carries the completed turn, never the reasoning.
    fx.up.script(reply("second answer"));
    turn(&fx, ALICE, &s.id, "second question").await;
    let second = fx.up.body(1)["messages"].to_string();
    assert!(second.contains("first question") && second.contains("Hello world"));
    assert!(!second.contains("THINKING-NEVER-STORED"));
    assert_eq!(fx.up.body(1)["messages"].as_array().unwrap().len(), 4);

    let task = scalar(&fx.db, &format!("SELECT COUNT(*) FROM workbench_tasks WHERE id = '{}' AND kind = 'chat' AND status = 'succeeded' AND total_tokens = 18 AND session_id = '{}'", ack.task_id, s.id)).await;
    assert_eq!(task, 1);
    assert!(leaks(&fx.db, "THINKING-NEVER-STORED", true)
        .await
        .is_empty());
    let file = std::fs::read(&fx.db_path).unwrap();
    assert_eq!(
        file.windows(21)
            .filter(|w| *w == b"THINKING-NEVER-STORED")
            .count(),
        0,
        "reasoning is not in the database file"
    );
}

// ---------------------------------------------------------------------------------------------
// AC06/AC17: cancel, cut connections, limits, closed sinks, windows, shutdown

#[tokio::test]
async fn stopping_keeps_the_received_part_and_closes_the_upstream_stream() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    fx.up.script(vec![delta("par"), Step::Hold]);
    let sent = start(&fx, ALICE, send_input(&s.id, "question"))
        .await
        .unwrap();
    let task = sent.ack.task_id.clone();
    let sink = Collect::default();
    let (_, cancelled) = tokio::join!(
        fx.chat
            .run_generation(&fx.providers, sent.job.unwrap(), &sink),
        async {
            until(|| sink.deltas() > 0).await;
            let first = fx.chat.cancel(ALICE, &task);
            let _ = fx.chat.cancel(ALICE, &task);
            first
        }
    );
    assert!(
        cancelled,
        "the owner can cancel from the moment the ack was returned"
    );
    let events = sink.take();
    let (message, code, unknown, discarded) = expect_error(terminal(&events));
    let message = message.as_ref().unwrap();
    assert_eq!(
        (message.status.as_str(), message.content.as_str()),
        ("interrupted", "par")
    );
    assert_eq!((code, unknown, discarded), ("task_cancelled", true, false));
    until(|| fx.up.open() == 0).await;
    let task_row = scalar(&fx.db, &format!("SELECT COUNT(*) FROM workbench_tasks WHERE id = '{task}' AND status = 'interrupted' AND error_code = 'task_cancelled' AND total_tokens IS NULL")).await;
    assert_eq!(task_row, 1);
    assert!(
        !fx.chat.cancel(ALICE, &task),
        "a finished task answers false"
    );
    turn(&fx, ALICE, &s.id, "can send again immediately").await;
}

#[tokio::test]
async fn a_cut_connection_is_interrupted_with_content_and_failed_without() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    fx.up.script(vec![delta("some text"), Step::Close]);
    let (_, events) = turn(&fx, ALICE, &s.id, "q1").await;
    let (message, code, unknown, _) = expect_error(terminal(&events));
    let message = message.as_ref().unwrap();
    assert_eq!(
        (message.status.as_str(), message.content.as_str()),
        ("interrupted", "some text")
    );
    assert_eq!(code, "provider_1001007");
    assert!(
        unknown && message.total_tokens.is_none(),
        "unknown usage stays NULL and is flagged"
    );

    fx.up.script(vec![Step::Close]);
    let (_, events) = turn(&fx, ALICE, &s.id, "q2").await;
    let (message, code, ..) = expect_error(terminal(&events));
    assert_eq!(message.as_ref().unwrap().status, "failed");
    assert_eq!(code, "provider_1001007");
    // Neither failed turn enters later context.
    fx.up.script(reply("fine"));
    turn(&fx, ALICE, &s.id, "q3").await;
    let body = fx.up.body(2)["messages"].to_string();
    assert!(!body.contains("some text") && !body.contains("\"q1\"") && !body.contains("\"q2\""));
}

#[tokio::test]
async fn a_reply_over_the_size_limit_stops_with_generation_limit() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    let block = "x".repeat(20_000);
    fx.up.script(vec![
        delta(&block),
        delta(&block),
        delta(&block),
        delta(&block),
        Step::Hold,
    ]);
    let (_, events) = turn(&fx, ALICE, &s.id, "long please").await;
    let (message, code, ..) = expect_error(terminal(&events));
    let message = message.as_ref().unwrap();
    assert_eq!(
        (message.status.as_str(), code),
        ("interrupted", "generation_limit")
    );
    assert_eq!(
        message.content.len(),
        64 * 1024,
        "the reply is cut at the limit, not stored longer"
    );
    until(|| fx.up.open() == 0).await;
}

#[tokio::test]
async fn a_closed_sink_stops_the_generation_and_keeps_what_arrived() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    fx.up.script(vec![delta("a"), delta("b"), Step::Hold]);
    let sent = start(&fx, ALICE, send_input(&s.id, "q")).await.unwrap();
    let sink = Collect::closing();
    fx.chat
        .run_generation(&fx.providers, sent.job.unwrap(), &sink)
        .await;
    let stored = messages(&fx, ALICE, &s.id).await;
    let reply = stored.last().unwrap();
    assert_eq!(reply.status, "interrupted");
    assert!(reply.content.starts_with('a'));
    assert_eq!(reply.error_code.as_deref(), Some("task_cancelled"));
    until(|| fx.up.open() == 0).await;
}

#[tokio::test]
async fn destroying_the_window_cancels_only_its_generation() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    fx.up.script(vec![delta("partial"), Step::Hold]);
    let sent = start(&fx, ALICE, send_input(&s.id, "q")).await.unwrap();
    let sink = Collect::default();
    tokio::join!(
        fx.chat
            .run_generation(&fx.providers, sent.job.unwrap(), &sink),
        async {
            until(|| sink.deltas() > 0).await;
            assert_eq!(fx.chat.cancel_window("some-other-window"), 0);
            assert_eq!(fx.chat.cancel_window("main"), 1);
        }
    );
    let events = sink.take();
    let (message, code, ..) = expect_error(terminal(&events));
    assert_eq!(message.as_ref().unwrap().content, "partial");
    assert_eq!(code, "task_cancelled");
}

#[tokio::test]
async fn graceful_shutdown_ends_the_generation_as_process_interrupted() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    fx.up.script(vec![delta("before exit"), Step::Hold]);
    let sent = start(&fx, ALICE, send_input(&s.id, "q")).await.unwrap();
    let sink = Collect::default();
    let (_, drained) = tokio::join!(
        fx.chat
            .run_generation(&fx.providers, sent.job.unwrap(), &sink),
        async {
            until(|| sink.deltas() > 0).await;
            fx.chat.shutdown().await
        }
    );
    assert!(
        drained,
        "the terminal transaction committed inside the exit window"
    );
    let stored = messages(&fx, ALICE, &s.id).await;
    assert_eq!(
        (stored[1].status.as_str(), stored[1].error_code.as_deref()),
        ("interrupted", Some("process_interrupted"))
    );
    assert_eq!(stored[1].content, "before exit");
    assert!(
        start(&fx, ALICE, send_input(&s.id, "after")).await.is_err(),
        "no new generation during shutdown"
    );
}

// ---------------------------------------------------------------------------------------------
// AC19: duplicates and concurrency

#[tokio::test]
async fn repeats_and_concurrent_sends_never_reach_the_upstream_twice() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    let other = session(&fx, ALICE).await;
    fx.up.script(vec![delta("working"), Step::Hold]);
    let input = send_input(&s.id, "same text");
    let first = start(&fx, ALICE, input.clone()).await.unwrap();
    let job = first.job.unwrap();

    let replay = start(&fx, ALICE, input.clone()).await.unwrap();
    assert!(replay.ack.replayed && replay.job.is_none());
    assert_eq!(
        replay.ack.assistant_message_id,
        first.ack.assistant_message_id
    );
    assert_eq!(replay.ack.user_message_id, first.ack.user_message_id);
    let conflict = start(
        &fx,
        ALICE,
        SendChatInput {
            content: "different".into(),
            ..input.clone()
        },
    )
    .await;
    assert_eq!(conflict.err(), Some(ChatError::RequestIdConflict));
    for target in [&s.id, &other.id] {
        let busy = start(&fx, ALICE, send_input(target, "again")).await;
        assert_eq!(
            busy.err(),
            Some(ChatError::GenerationInProgress),
            "one active generation per user, any session"
        );
    }
    for bad in ["", "   \n\t ", &"a".repeat(16 * 1024 + 1)] {
        let invalid = start(&fx, ALICE, send_input(&s.id, bad)).await;
        assert_eq!(invalid.err(), Some(ChatError::InvalidMessageContent));
    }
    let not_a_uuid = start(
        &fx,
        ALICE,
        SendChatInput {
            client_request_id: "nope".into(),
            ..send_input(&s.id, "x")
        },
    )
    .await;
    assert_eq!(not_a_uuid.err(), Some(ChatError::InvalidMessageContent));
    assert_eq!(
        fx.chat
            .delete_session(
                ALICE,
                DeleteChatSessionInput {
                    session_id: s.id.clone()
                }
            )
            .await
            .err(),
        Some(ChatError::GenerationInProgress),
        "a session with an active generation is not deleted"
    );
    fx.chat
        .delete_session(
            ALICE,
            DeleteChatSessionInput {
                session_id: other.id.clone(),
            },
        )
        .await
        .unwrap();

    let sink = Collect::default();
    let task = first.ack.task_id.clone();
    tokio::join!(fx.chat.run_generation(&fx.providers, job, &sink), async {
        until(|| sink.deltas() > 0).await;
        assert!(
            fx.chat.cancel(ALICE, &task),
            "the original generation is still cancellable after the duplicates"
        );
    });
    assert_eq!(fx.up.hits(), 1, "no duplicate upstream call");
    assert_eq!(
        scalar(
            &fx.db,
            "SELECT COUNT(*) FROM workbench_tasks WHERE kind = 'chat'"
        )
        .await,
        1
    );
    // After the terminal state the id replays and a fresh id works.
    let again = start(&fx, ALICE, input).await.unwrap();
    assert!(again.ack.replayed);
    turn(&fx, ALICE, &s.id, "next").await;
}

// ---------------------------------------------------------------------------------------------
// AC20: provider and model snapshot

#[tokio::test]
async fn each_send_rereads_the_provider_and_blocks_a_changed_destination() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    turn(&fx, ALICE, &s.id, "first").await;

    // Rename-only and refresh-style changes do not block.
    fx.db
        .execute_unprepared("UPDATE providers SET name = 'Renamed'")
        .await
        .unwrap();
    turn(&fx, ALICE, &s.id, "after rename").await;

    // A second provider on a different host name is a different destination.
    let moved = seed_provider(
        fx.db.clone(),
        &fx.vault,
        ProviderConfig {
            name: "Moved".into(),
            provider_type: "openai_compatible".into(),
            base_url: format!("http://localhost:{}/v1", fx.up.addr.port()),
            api_key: "test-only-secret".into(),
            enabled: true,
            config: None,
        },
        ALICE,
    )
    .await
    .unwrap();
    fx.providers
        .add_manual_model(&moved.id, ALICE, "chat-model", Some("text-generation"))
        .await
        .unwrap();
    fx.chat
        .update_target(
            &fx.providers,
            &fx.skills,
            ALICE,
            UpdateChatSessionTargetInput {
                session_id: s.id.clone(),
                provider_id: Some(moved.id.clone()),
                model_id: Some("chat-model".into()),
                skill_id: None,
                no_skill: true,
            },
        )
        .await
        .unwrap();
    let hits_before = fx.up.hits();
    let blocked = start(&fx, ALICE, send_input(&s.id, "to the new host")).await;
    let Err(ChatError::DestinationChanged { fingerprint, host }) = blocked else {
        panic!("blocked")
    };
    assert_eq!(host, "localhost");
    assert_eq!(
        fx.up.hits(),
        hits_before,
        "nothing was sent before the user confirmed"
    );
    assert_eq!(
        scalar(
            &fx.db,
            "SELECT COUNT(*) FROM workbench_tasks WHERE kind = 'chat'"
        )
        .await,
        2,
        "a blocked send leaves no task row"
    );

    let confirmed = start(
        &fx,
        ALICE,
        SendChatInput {
            accept_destination: Some(fingerprint.clone()),
            ..send_input(&s.id, "to the new host")
        },
    )
    .await
    .unwrap();
    let sink = Collect::default();
    fx.chat
        .run_generation(&fx.providers, confirmed.job.unwrap(), &sink)
        .await;
    assert!(matches!(terminal(&sink.take()), ChatEvent::Done { .. }));
    let stored = scalar(
        &fx.db,
        &format!(
            "SELECT COUNT(*) FROM workbench_tasks WHERE endpoint_fingerprint = '{fingerprint}'"
        ),
    )
    .await;
    assert_eq!(stored, 1);
    let audit_hits = scalar(
        &fx.db,
        "SELECT COUNT(*) FROM workbench_tasks WHERE COALESCE(endpoint_fingerprint,'') || COALESCE(provider_id,'') || COALESCE(error_code,'') LIKE '%localhost%' \
         OR COALESCE(endpoint_fingerprint,'') LIKE '%http%'",
    )
    .await;
    assert_eq!(
        audit_hits, 0,
        "the raw host and URL never reach the task audit"
    );
    // Once a reply exists on the new host the next send is no longer blocked.
    turn(&fx, ALICE, &s.id, "and again").await;
}

#[tokio::test]
async fn unusable_targets_are_refused_without_choosing_another_provider() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    let blank = fx
        .chat
        .create_session(
            &fx.providers,
            &fx.skills,
            ALICE,
            CreateChatSessionInput::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        start(&fx, ALICE, send_input(&blank.id, "x")).await.err(),
        Some(ChatError::ProviderNotSelected)
    );
    let nobody = fx
        .chat
        .create_session(
            &fx.providers,
            &fx.skills,
            BOB,
            CreateChatSessionInput::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        start(&fx, BOB, send_input(&nobody.id, "x")).await.err(),
        Some(ChatError::NoProviderConfigured)
    );

    // A non-text model is refused at selection and, if it changes later, at send.
    fx.providers
        .add_manual_model(&fx.provider_id, ALICE, "embed-model", Some("embedding"))
        .await
        .unwrap();
    let refused = fx
        .chat
        .create_session(
            &fx.providers,
            &fx.skills,
            ALICE,
            CreateChatSessionInput {
                target: Some(ChatTargetInput {
                    model_id: Some("embed-model".into()),
                    ..target(&fx)
                }),
            },
        )
        .await;
    assert_eq!(refused.err(), Some(ChatError::Provider(1001020)));
    fx.db
        .execute_unprepared("UPDATE models SET kind = 'embedding' WHERE model_id = 'chat-model'")
        .await
        .unwrap();
    assert_eq!(
        start(&fx, ALICE, send_input(&s.id, "x")).await.err(),
        Some(ChatError::Provider(1001020))
    );
    fx.db
        .execute_unprepared(
            "UPDATE models SET kind = 'text-generation' WHERE model_id = 'chat-model'",
        )
        .await
        .unwrap();

    fx.providers
        .toggle_provider(&fx.provider_id, false, ALICE)
        .await
        .unwrap();
    assert_eq!(
        start(&fx, ALICE, send_input(&s.id, "x")).await.err(),
        Some(ChatError::Provider(1001003))
    );
    fx.providers
        .toggle_provider(&fx.provider_id, true, ALICE)
        .await
        .unwrap();
    fx.providers
        .delete_provider(&fx.provider_id, ALICE)
        .await
        .unwrap();
    assert_eq!(
        start(&fx, ALICE, send_input(&s.id, "x")).await.err(),
        Some(ChatError::ProviderNotSelected)
    );
    assert_eq!(fx.up.hits(), 0, "no refusal reached the upstream");
    assert_eq!(
        scalar(
            &fx.db,
            "SELECT COUNT(*) FROM workbench_tasks WHERE kind = 'chat'"
        )
        .await,
        0
    );
    // The history stays readable although the Provider is gone.
    assert!(messages(&fx, ALICE, &s.id).await.is_empty());
}

// ---------------------------------------------------------------------------------------------
// AC23: context budget

#[tokio::test]
async fn history_over_budget_drops_whole_oldest_turns_and_reports_them() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    let repo = ChatRepository::new(fx.db.clone());
    for i in 1..=4 {
        let (task, user, assistant) = (format!("seed-{i}"), format!("su{i}"), format!("sa{i}"));
        repo.begin_generation(&BeginGeneration {
            user: ALICE,
            session_id: &s.id,
            task_id: &task,
            user_message_id: &user,
            assistant_message_id: &assistant,
            user_content: Some(&format!("question-{i}")),
            regenerate_of: None,
            provider_id: "p",
            model_id: "m",
            provider_config_revision: None,
            endpoint_fingerprint: "seed",
            skill: None,
            tools: &[],
        })
        .await
        .unwrap();
        repo.finalize_generation(&FinalizeGeneration {
            user: ALICE,
            assistant_message_id: &assistant,
            task_id: &task,
            content: &format!("reply-{i}-{}", "r".repeat(7000)),
            message_status: "completed",
            finish_reason: Some("stop"),
            message_error: None,
            task_status: "succeeded",
            task_error: None,
            total_tokens: Some(1),
        })
        .await
        .unwrap();
    }
    // The seeded turns carry a placeholder fingerprint; this test is about the budget, not the
    // destination check.
    fx.db
        .execute_unprepared("UPDATE workbench_tasks SET endpoint_fingerprint = NULL")
        .await
        .unwrap();
    let (ack, events) = turn(&fx, ALICE, &s.id, "now").await;
    assert_eq!(ack.dropped_turns, 1);
    assert!(matches!(terminal(&events), ChatEvent::Done { .. }));
    let sent = fx.up.body(0)["messages"].clone();
    let contents: Vec<String> = sent
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        contents.len(),
        8,
        "system + three whole turns + the new message"
    );
    assert!(
        !contents
            .iter()
            .any(|c| c.contains("question-1") || c.contains("reply-1-")),
        "the oldest turn is gone whole"
    );
    assert!(contents.iter().any(|c| c.starts_with("reply-2-")) && contents[7] == "now");
    assert_eq!(
        messages(&fx, ALICE, &s.id).await.len(),
        10,
        "dropped turns stay in the stored history"
    );
}

// ---------------------------------------------------------------------------------------------
// AC27: regeneration

#[tokio::test]
async fn regeneration_replaces_the_reply_only_when_the_new_one_succeeds() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    fx.up.script(reply("old answer"));
    let (ack, _) = turn(&fx, ALICE, &s.id, "the question").await;

    fx.up.script(vec![Step::Close]);
    let (failed_ack, events) = regenerate(&fx, ALICE, &ack.assistant_message_id, None)
        .await
        .unwrap();
    assert_eq!(failed_ack.user_message_id, ack.user_message_id);
    let (kept, _, _, discarded) = expect_error(terminal(&events));
    assert!(discarded);
    assert_eq!(
        kept.as_ref().unwrap().content,
        "old answer",
        "the failure keeps the old reply"
    );
    let listed = messages(&fx, ALICE, &s.id).await;
    assert_eq!(
        listed
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        ["the question", "old answer"]
    );

    fx.up.script(reply("new answer"));
    let request = uuid::Uuid::new_v4().to_string();
    let (new_ack, events) =
        regenerate(&fx, ALICE, &ack.assistant_message_id, Some(request.clone()))
            .await
            .unwrap();
    let ChatEvent::Done { message } = terminal(&events) else {
        panic!("done")
    };
    assert_eq!(message.content, "new answer");
    let listed = messages(&fx, ALICE, &s.id).await;
    assert_eq!(
        listed
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        ["the question", "new answer"]
    );
    let body = fx.up.body(2)["messages"].to_string();
    assert!(
        !body.contains("old answer"),
        "the replaced reply is not part of the context"
    );
    assert_eq!(fx.up.body(2)["messages"].as_array().unwrap().len(), 2);
    assert_eq!(
        scalar(
            &fx.db,
            "SELECT COUNT(*) FROM workbench_tasks WHERE kind = 'chat'"
        )
        .await,
        3,
        "every regeneration is its own billed task"
    );

    let (replayed, events) = regenerate(&fx, ALICE, &ack.assistant_message_id, Some(request))
        .await
        .unwrap();
    assert!(replayed.replayed && events.is_empty());
    assert_eq!(replayed.assistant_message_id, new_ack.assistant_message_id);
    assert_eq!(fx.up.hits(), 3, "the replay made no upstream call");

    // Only the last assistant message; others' messages look the same as missing ones.
    let (_, _) = turn(&fx, ALICE, &s.id, "later question").await;
    assert_eq!(
        regenerate(&fx, ALICE, &new_ack.assistant_message_id, None)
            .await
            .err(),
        Some(ChatError::MessageNotRegenerable)
    );
    assert_eq!(
        regenerate(&fx, ALICE, &listed[0].id, None).await.err(),
        Some(ChatError::MessageNotRegenerable),
        "a user message"
    );
    let last = messages(&fx, ALICE, &s.id).await.pop().unwrap();
    assert_eq!(
        regenerate(&fx, BOB, &last.id, None).await.err(),
        Some(ChatError::MessageNotRegenerable)
    );
    assert_eq!(
        regenerate(&fx, ALICE, "missing", None).await.err(),
        Some(ChatError::MessageNotRegenerable)
    );
}

// ---------------------------------------------------------------------------------------------
// AC18: restart recovery

#[tokio::test]
async fn restart_recovery_finalizes_unfinished_generations_without_resending() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    fx.up.script(reply("kept"));
    let (ack, _) = turn(&fx, ALICE, &s.id, "q1").await;
    // A regeneration and a plain send that never finish (the process "dies" before running them).
    let regen = fx
        .chat
        .regenerate(
            &fx.providers,
            &fx.skills,
            ALICE,
            "main",
            RegenerateChatInput {
                message_id: ack.assistant_message_id.clone(),
                client_request_id: uuid::Uuid::new_v4().to_string(),
                accept_destination: None,
            },
        )
        .await
        .unwrap();
    let draft_id = regen.ack.assistant_message_id.clone();
    drop(regen);
    let hits = fx.up.hits();

    let restarted = ChatManager::new(fx.db.clone());
    assert_eq!(
        restarted.recover_interrupted().await.unwrap(),
        0,
        "the draft is dropped, nothing else was generating"
    );
    fx.workbench.recover_interrupted().await.unwrap();
    let stored = messages(&fx, ALICE, &s.id).await;
    assert_eq!(
        stored
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        ["q1", "kept"],
        "the old reply survives an unfinished regeneration"
    );
    assert!(stored.iter().all(|m| m.id != draft_id));
    assert_eq!(scalar(&fx.db, "SELECT COUNT(*) FROM workbench_tasks WHERE status = 'interrupted' AND error_code = 'process_interrupted'").await, 1);

    let sent = restarted
        .send(
            &fx.providers,
            &fx.skills,
            ALICE,
            "main",
            send_input(&s.id, "q2"),
        )
        .await
        .unwrap();
    fx.db
        .execute_unprepared("UPDATE chat_messages SET content = 'half' WHERE status = 'generating'")
        .await
        .unwrap();
    drop(sent);
    let after = ChatManager::new(fx.db.clone());
    assert_eq!(after.recover_interrupted().await.unwrap(), 1);
    fx.workbench.recover_interrupted().await.unwrap();
    let stored = messages(&fx, ALICE, &s.id).await;
    let last = stored.last().unwrap();
    assert_eq!(
        (
            last.status.as_str(),
            last.content.as_str(),
            last.error_code.as_deref()
        ),
        ("interrupted", "half", Some("process_interrupted"))
    );
    assert_eq!(fx.up.hits(), hits, "recovery never resends");
    assert_eq!(after.recover_interrupted().await.unwrap(), 0, "repeatable");
    let (_, events) = {
        fx.up.script(reply("works again"));
        let s2 = after
            .send(
                &fx.providers,
                &fx.skills,
                ALICE,
                "main",
                send_input(&s.id, "q3"),
            )
            .await
            .unwrap();
        let sink = Collect::default();
        after
            .run_generation(&fx.providers, s2.job.unwrap(), &sink)
            .await;
        (s2.ack, sink.take())
    };
    assert!(matches!(terminal(&events), ChatEvent::Done { .. }));
}

// ---------------------------------------------------------------------------------------------
// AC16: isolation

#[tokio::test]
async fn another_user_cannot_touch_a_session_and_gets_the_missing_answer() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    let (ack, _) = turn(&fx, ALICE, &s.id, "alice only").await;
    let missing = "00000000-0000-4000-8000-000000000000";
    for id in [s.id.as_str(), missing] {
        assert_eq!(
            fx.chat
                .list_messages(
                    BOB,
                    ListChatMessagesInput {
                        session_id: id.into(),
                        before_seq: None,
                        limit: None
                    }
                )
                .await
                .err(),
            Some(ChatError::SessionNotFound)
        );
        assert_eq!(
            fx.chat
                .rename_session(
                    BOB,
                    RenameChatSessionInput {
                        session_id: id.into(),
                        title: "x".into()
                    }
                )
                .await
                .err(),
            Some(ChatError::SessionNotFound)
        );
        assert_eq!(
            fx.chat
                .delete_session(
                    BOB,
                    DeleteChatSessionInput {
                        session_id: id.into()
                    }
                )
                .await
                .err(),
            Some(ChatError::SessionNotFound)
        );
        assert_eq!(
            fx.chat
                .update_target(
                    &fx.providers,
                    &fx.skills,
                    BOB,
                    UpdateChatSessionTargetInput {
                        session_id: id.into(),
                        provider_id: None,
                        model_id: None,
                        skill_id: None,
                        no_skill: false
                    }
                )
                .await
                .err(),
            Some(ChatError::SessionNotFound)
        );
        assert_eq!(
            start(&fx, BOB, send_input(id, "hi")).await.err(),
            Some(ChatError::SessionNotFound)
        );
    }
    assert!(!fx.chat.cancel(BOB, &ack.task_id));
    let search = fx
        .chat
        .search_sessions(
            BOB,
            SearchChatSessionsInput {
                query: "alice".into(),
                cursor: None,
                limit: None,
            },
        )
        .await
        .unwrap();
    assert!(search.items.is_empty());
    assert_eq!(
        fx.chat
            .list_messages(
                ALICE,
                ListChatMessagesInput {
                    session_id: s.id.clone(),
                    before_seq: None,
                    limit: None
                }
            )
            .await
            .unwrap()
            .items
            .len(),
        2
    );
    assert_eq!(fx.up.hits(), 1);
    assert!(fx.chat.purge_user(BOB).await.is_ok());
    assert_eq!(
        messages(&fx, ALICE, &s.id).await.len(),
        2,
        "purging another user leaves this one alone"
    );
}

// ---------------------------------------------------------------------------------------------
// AC24/AC25: redaction and deletion

#[tokio::test]
async fn content_stays_in_chat_tables_and_survives_neither_deletion_nor_audit() {
    const USER: &str = "SENTINEL-USER-3c1f";
    const REPLY: &str = "SENTINEL-REPLY-8b2e";
    const TITLE: &str = "SENTINEL-TITLE-5d7a";
    const QUERY: &str = "SENTINEL-QUERY-9e44";
    const THOUGHT: &str = "SENTINEL-THOUGHT-71c0";
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    fx.chat
        .rename_session(
            ALICE,
            RenameChatSessionInput {
                session_id: s.id.clone(),
                title: TITLE.into(),
            },
        )
        .await
        .unwrap();
    fx.up.script(vec![
        reasoning(THOUGHT),
        delta(REPLY),
        finish(),
        usage(),
        done(),
    ]);
    turn(&fx, ALICE, &s.id, &format!("{USER} please")).await;
    fx.up.script(vec![delta(REPLY), Step::Close]);
    turn(&fx, ALICE, &s.id, &format!("{USER} failing")).await;
    fx.up.script(vec![delta(REPLY), Step::Hold]);
    let sent = start(&fx, ALICE, send_input(&s.id, &format!("{USER} cancelled")))
        .await
        .unwrap();
    let task = sent.ack.task_id.clone();
    let sink = Collect::default();
    tokio::join!(
        fx.chat
            .run_generation(&fx.providers, sent.job.unwrap(), &sink),
        async {
            until(|| sink.deltas() > 0).await;
            fx.chat.cancel(ALICE, &task);
        }
    );
    fx.chat
        .search_sessions(
            ALICE,
            SearchChatSessionsInput {
                query: QUERY.into(),
                cursor: None,
                limit: None,
            },
        )
        .await
        .unwrap();
    let found = fx
        .chat
        .search_sessions(
            ALICE,
            SearchChatSessionsInput {
                query: USER.into(),
                cursor: None,
                limit: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(found.items.len(), 1);

    for sentinel in [USER, REPLY, TITLE, QUERY, THOUGHT] {
        assert!(
            leaks(&fx.db, sentinel, false).await.is_empty(),
            "{sentinel} leaked outside chat_*: {:?}",
            leaks(&fx.db, sentinel, false).await
        );
    }
    assert!(
        !leaks(&fx.db, USER, true).await.is_empty() && !leaks(&fx.db, TITLE, true).await.is_empty(),
        "the content is where it should be"
    );
    assert!(
        leaks(&fx.db, THOUGHT, true).await.is_empty(),
        "reasoning is nowhere"
    );

    fx.chat
        .delete_session(
            ALICE,
            DeleteChatSessionInput {
                session_id: s.id.clone(),
            },
        )
        .await
        .unwrap();
    for sentinel in [USER, REPLY, TITLE] {
        assert!(
            leaks(&fx.db, sentinel, true).await.is_empty(),
            "{sentinel} remained after deleting the session"
        );
    }
    let file = std::fs::read(&fx.db_path).unwrap();
    for sentinel in [USER, REPLY, TITLE, THOUGHT] {
        assert_eq!(
            file.windows(sentinel.len())
                .filter(|w| *w == sentinel.as_bytes())
                .count(),
            0,
            "{sentinel} is still in the main database file"
        );
    }
    assert_eq!(
        scalar(
            &fx.db,
            "SELECT COUNT(*) FROM workbench_tasks WHERE kind = 'chat' AND session_id IS NULL"
        )
        .await,
        3,
        "the content-free audit rows stay"
    );
}

// ---------------------------------------------------------------------------------------------
// A job that is dropped without running must not lock the user out

#[tokio::test]
async fn an_abandoned_job_frees_the_user_and_its_orphan_is_closed_without_resending() {
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    let abandoned = start(&fx, ALICE, send_input(&s.id, "never driven"))
        .await
        .unwrap();
    let orphan_task = abandoned.ack.task_id.clone();
    drop(abandoned);
    assert!(
        !fx.chat.cancel(ALICE, &orphan_task),
        "dropping the job released its registry slot"
    );
    assert_eq!(fx.up.hits(), 0);

    fx.up.script(reply("fresh answer"));
    let (_, events) = turn(&fx, ALICE, &s.id, "next question").await;
    assert!(matches!(terminal(&events), ChatEvent::Done { .. }));
    let stored = messages(&fx, ALICE, &s.id).await;
    let orphan = &stored[1];
    assert_eq!(
        (orphan.status.as_str(), orphan.error_code.as_deref()),
        ("failed", Some("process_interrupted"))
    );
    assert_eq!(
        scalar(&fx.db, &format!("SELECT COUNT(*) FROM workbench_tasks WHERE id = '{orphan_task}' AND status = 'interrupted'")).await,
        1
    );
    assert_eq!(fx.up.hits(), 1, "only the new send reached the upstream");
    let context = fx.up.body(0)["messages"].to_string();
    assert!(
        !context.contains("never driven"),
        "the orphaned question is not part of later context"
    );
}

#[tokio::test]
async fn chat_generations_stay_out_of_the_single_task_history() {
    // D077/D078: the single-task tab's history (`list_workbench_tasks`) returns only
    // `kind = 'single'`; chat sends must not push single tasks out of its 50-row page.
    let fx = fx().await;
    let s = session(&fx, ALICE).await;
    fx.up.script(reply("answer"));
    let (ack, _) = turn(&fx, ALICE, &s.id, "question").await;
    assert_eq!(
        scalar(
            &fx.db,
            &format!(
                "SELECT COUNT(*) FROM workbench_tasks WHERE id = '{}' AND kind = 'chat'",
                ack.task_id
            )
        )
        .await,
        1,
        "the chat send wrote its audit row"
    );
    let history = fx.workbench.list(ALICE).await.unwrap();
    assert!(
        history.iter().all(|record| record.task.kind == "single"),
        "chat audit rows leaked into the single-task history"
    );
    assert!(history.is_empty());
}
