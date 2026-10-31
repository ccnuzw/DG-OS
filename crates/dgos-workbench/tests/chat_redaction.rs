//! FR-006 AC24 (logs): chat text, titles, search queries, reasoning and upstream error bodies
//! never reach the process log. A positive control proves the capture sees events from the test
//! task, spawned tasks and plain threads. The database side of AC24 is in `chat_generation.rs`.

use dgos_db::{migrations::Migrator, sea_orm_migration::MigratorTrait};
use dgos_providers::{
    test_support::{seed_provider, MemoryCredentialVault},
    ProviderManager,
};
use dgos_skills::SkillsManager;
use dgos_types::{Model, ProviderConfig};
use dgos_workbench::{
    ChatEvent, ChatManager, ChatSink, ChatTargetInput, CreateChatSessionInput,
    RenameChatSessionInput, SearchChatSessionsInput, SendChatInput,
};
use std::sync::{Arc, LazyLock, Mutex};
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;
    fn make_writer(&'a self) -> LogBuffer {
        self.clone()
    }
}

/// Worst-case verbosity with the same database cap the host applies (`src-tauri/src/logging.rs`):
/// a separate `Targets` filter AND-ed with the user's spec. The spec names the database targets
/// more specifically than the cap, which is what lifted the earlier directive-based cap. Without
/// the cap this test fails: sea-orm's trace spans carry the bound statement values.
static LOG: LazyLock<LogBuffer> = LazyLock::new(|| {
    use tracing_subscriber::filter::{EnvFilter, FilterExt, LevelFilter, Targets};
    use tracing_subscriber::layer::{Layer, SubscriberExt};
    let buffer = LogBuffer::default();
    let cap = dgos_db::QUIET_LOG_TARGETS.iter().fold(
        Targets::new().with_default(LevelFilter::TRACE),
        |cap, target| cap.with_target(*target, LevelFilter::WARN),
    );
    let verbosity = EnvFilter::new("sea_orm::driver=trace,sqlx::query=trace,trace");
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(buffer.clone())
        .with_filter(verbosity.and(cap));
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::set_global_default(subscriber).expect("only subscriber in this binary");
    buffer
});

fn captured_log() -> LogBuffer {
    LOG.clone()
}

#[derive(Default)]
struct Collect(Mutex<Vec<ChatEvent>>);
impl ChatSink for Collect {
    fn send(&self, event: ChatEvent) -> bool {
        self.0.lock().unwrap().push(event);
        true
    }
}

const USER_TEXT: &str = "LOG-SENTINEL-USER-7a41";
const REPLY: &str = "LOG-SENTINEL-REPLY-7a41";
const THOUGHT: &str = "LOG-SENTINEL-THOUGHT-7a41";
const TITLE: &str = "LOG-SENTINEL-TITLE-7a41";
const QUERY: &str = "LOG-SENTINEL-QUERY-7a41";
const UPSTREAM_ERROR: &str = "LOG-SENTINEL-UPSTREAM-ERROR-7a41";

#[tokio::test]
async fn the_log_capture_sees_events_from_every_kind_of_caller() {
    let log = captured_log();
    tracing::info!(marker = "CHAT-CONTROL-task-91", "control");
    tokio::spawn(async { tracing::debug!(marker = "CHAT-CONTROL-spawned-91", "control") })
        .await
        .unwrap();
    std::thread::spawn(|| tracing::trace!(marker = "CHAT-CONTROL-thread-91", "control"))
        .join()
        .unwrap();
    let text = String::from_utf8_lossy(&log.0.lock().unwrap()).into_owned();
    for marker in [
        "CHAT-CONTROL-task-91",
        "CHAT-CONTROL-spawned-91",
        "CHAT-CONTROL-thread-91",
    ] {
        assert!(
            text.contains(marker),
            "{marker} missing: the redaction test below would prove nothing"
        );
    }
}

#[tokio::test]
async fn chat_content_and_upstream_error_text_never_reach_the_log() {
    let log = captured_log();
    let mut server = mockito::Server::new_async().await;
    let ok = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        serde_json::json!({"choices":[{"delta":{"reasoning_content":THOUGHT},"finish_reason":null}]}),
        serde_json::json!({"choices":[{"delta":{"content":REPLY},"finish_reason":"stop"}]}),
    );
    let _first = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Regex("LOG-SENTINEL-USER-7a41 ok".into()))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(ok)
        .create_async()
        .await;
    let _second = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Regex(
            "LOG-SENTINEL-USER-7a41 fails".into(),
        ))
        .with_status(500)
        .with_header("content-type", "application/json")
        .with_body(serde_json::json!({"error": {"message": UPSTREAM_ERROR}}).to_string())
        .create_async()
        .await;

    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("t.db").display());
    let db = dgos_db::establish_connection(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let vault = Arc::new(MemoryCredentialVault::default());
    let provider = seed_provider(
        db.clone(),
        &vault,
        ProviderConfig {
            name: "mock".into(),
            provider_type: "openai_compatible".into(),
            base_url: server.url(),
            api_key: "synthetic".into(),
            enabled: true,
            config: None,
        },
        "alice",
    )
    .await
    .unwrap();
    dgos_db::ModelRepository::new(db.clone())
        .sync_models(
            &provider.id,
            vec![Model {
                id: uuid::Uuid::new_v4().to_string(),
                provider_id: provider.id.clone(),
                model_id: "chat".into(),
                model_name: "chat".into(),
                model_type: "chat".into(),
                context_window: Some(65536),
                supports_streaming: true,
                cached_at: String::new(),
                ..Default::default()
            }],
        )
        .await
        .unwrap();
    let providers = ProviderManager::with_vault(&url, vault).await.unwrap();
    let skills = SkillsManager::new(db.clone(), dir.path().join("skills"));
    let chat = ChatManager::new(db.clone());
    let session = chat
        .create_session(
            &providers,
            &skills,
            "alice",
            CreateChatSessionInput {
                target: Some(ChatTargetInput {
                    provider_id: Some(provider.id.clone()),
                    model_id: Some("chat".into()),
                    skill_id: None,
                    no_skill: true,
                }),
            },
        )
        .await
        .unwrap();
    chat.rename_session(
        "alice",
        RenameChatSessionInput {
            session_id: session.id.clone(),
            title: TITLE.into(),
        },
    )
    .await
    .unwrap();

    let mut outcomes = Vec::new();
    for text in [format!("{USER_TEXT} ok"), format!("{USER_TEXT} fails")] {
        let sent = chat
            .send(
                &providers,
                &skills,
                "alice",
                "main",
                SendChatInput {
                    session_id: session.id.clone(),
                    content: text,
                    client_request_id: uuid::Uuid::new_v4().to_string(),
                    accept_destination: None,
                    tools: vec![],
                },
            )
            .await
            .unwrap();
        let sink = Collect::default();
        chat.run_generation(&providers, sent.job.unwrap(), &sink)
            .await;
        outcomes.push(sink.0.lock().unwrap().pop().unwrap());
    }
    assert!(matches!(outcomes[0], ChatEvent::Done { .. }));
    let ChatEvent::Error {
        error_code,
        message,
        ..
    } = &outcomes[1]
    else {
        panic!("the second turn fails")
    };
    assert!(error_code.starts_with("provider_"), "{error_code}");
    let rendered = format!("{error_code} {:?}", message.as_ref().map(|m| &m.error_code));
    assert!(
        !rendered.contains(UPSTREAM_ERROR),
        "the upstream's text is not carried into the result"
    );
    chat.search_sessions(
        "alice",
        SearchChatSessionsInput {
            query: QUERY.into(),
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap();
    chat.search_sessions(
        "alice",
        SearchChatSessionsInput {
            query: USER_TEXT.into(),
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap();

    let text = String::from_utf8_lossy(&log.0.lock().unwrap()).into_owned();
    for sentinel in [USER_TEXT, REPLY, THOUGHT, TITLE, QUERY, UPSTREAM_ERROR] {
        assert!(
            !text.contains(sentinel),
            "{sentinel} appears in the process log"
        );
    }
}
