//! FR-006 stage 5 (AC21): a session pins one skill version (or an explicit "no skill"), every
//! send revalidates it, and a failure blocks instead of dropping or swapping the skill (D075).
//! Uses the bundled skill catalog and a mocked Provider; not evidence about real Providers.

use dgos_db::{
    migrations::Migrator,
    sea_orm::{
        ActiveModelTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, Set,
        Statement,
    },
    sea_orm_migration::MigratorTrait,
    ModelRepository,
};
use dgos_providers::{
    test_support::{seed_provider, MemoryCredentialVault},
    ProviderManager,
};
use dgos_skills::SkillsManager;
use dgos_types::{Model, ProviderConfig};
use dgos_workbench::{
    ChatError, ChatEvent, ChatManager, ChatSink, ChatTargetInput, CreateChatSessionInput,
    SendChatInput, UpdateChatSessionTargetInput,
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

const SKILL: &str = "dgos.official.writing.summarize";
const ALICE: &str = "alice";
const BOB: &str = "bob";

struct Fx {
    db: DatabaseConnection,
    chat: ChatManager,
    providers: ProviderManager,
    skills: SkillsManager,
    provider_id: String,
    server: mockito::ServerGuard,
    seen: Arc<Mutex<Vec<Value>>>,
    _mock: mockito::Mock,
    _dir: tempfile::TempDir,
}

async fn fx() -> Fx {
    let mut server = mockito::Server::new_async().await;
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let recorder = seen.clone();
    let mock = server
        .mock("POST", "/chat/completions")
        .match_request(move |r| {
            recorder.lock().unwrap().push(serde_json::from_slice(r.body().unwrap()).unwrap());
            true
        })
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body("data: {\"choices\":[{\"delta\":{\"content\":\"summary\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
        .expect_at_least(0)
        .create_async()
        .await;
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("t.db").display());
    let db = dgos_db::establish_connection(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let skills = SkillsManager::new(db.clone(), dir.path().join("skills"));
    skills
        .register_bundled_catalog(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills"))
        .await
        .unwrap();
    skills.install_skill(SKILL, ALICE).await.unwrap();
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
                max_output_tokens: None,
                supports_streaming: true,
                supports_vision: false,
                cached_at: String::new(),
                ..Default::default()
            }],
        )
        .await
        .unwrap();
    Fx {
        chat: ChatManager::new(db.clone()),
        providers: ProviderManager::with_vault(&url, vault).await.unwrap(),
        provider_id: provider.id,
        db,
        skills,
        server,
        seen,
        _mock: mock,
        _dir: dir,
    }
}

#[derive(Default)]
struct Collect(Mutex<Vec<ChatEvent>>);
impl ChatSink for Collect {
    fn send(&self, event: ChatEvent) -> bool {
        self.0.lock().unwrap().push(event);
        true
    }
}

fn target(fx: &Fx, skill: Option<&str>, no_skill: bool) -> ChatTargetInput {
    ChatTargetInput {
        provider_id: Some(fx.provider_id.clone()),
        model_id: Some("chat".into()),
        skill_id: skill.map(str::to_owned),
        no_skill,
    }
}

async fn session(fx: &Fx, skill: Option<&str>, no_skill: bool) -> String {
    fx.chat
        .create_session(
            &fx.providers,
            &fx.skills,
            ALICE,
            CreateChatSessionInput {
                target: Some(target(fx, skill, no_skill)),
            },
        )
        .await
        .unwrap()
        .id
}

/// Sends and, when accepted, runs the generation to its end. `Ok` carries the terminal event.
async fn turn(fx: &Fx, session: &str, text: &str) -> Result<ChatEvent, ChatError> {
    let sent = fx
        .chat
        .send(
            &fx.providers,
            &fx.skills,
            ALICE,
            "main",
            SendChatInput {
                session_id: session.into(),
                content: text.into(),
                client_request_id: uuid::Uuid::new_v4().to_string(),
                accept_destination: None,
                tools: vec![],
            },
        )
        .await?;
    let sink = Collect::default();
    fx.chat
        .run_generation(&fx.providers, sent.job.unwrap(), &sink)
        .await;
    let last = sink.0.lock().unwrap().pop().unwrap();
    Ok(last)
}

async fn count(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index::<i64>(0)
        .unwrap()
}

#[tokio::test]
async fn the_pinned_version_is_sent_as_its_own_user_message_and_recorded_with_its_hash() {
    let fx = fx().await;
    let s = session(&fx, Some(SKILL), false).await;
    let pinned = fx
        .chat
        .list_sessions(ALICE, Default::default())
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(pinned.skill_id.as_deref(), Some(SKILL));
    assert!(matches!(
        turn(&fx, &s, "summarize this").await.unwrap(),
        ChatEvent::Done { .. }
    ));

    let body = fx.seen.lock().unwrap()[0].clone();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    let system = messages[0]["content"].as_str().unwrap();
    let guidance = messages[1]["content"].as_str().unwrap();
    assert!(
        !system.contains(SKILL),
        "the skill's identity never enters the system role"
    );
    assert!(
        guidance.len() > system.len() / 4 && guidance.contains(SKILL),
        "the skill text is its own user message"
    );
    assert_eq!(messages[1]["role"], "user");
    assert!(messages[1]["content"]
        .as_str()
        .unwrap()
        .starts_with(&format!("Selected skill guidance ({SKILL}@")));
    assert_eq!(messages[2]["content"], "summarize this");

    let row = count(&fx.db, &format!("SELECT COUNT(*) FROM workbench_task_skills WHERE skill_id = '{SKILL}' AND content_hash <> ''")).await;
    assert_eq!(
        row, 1,
        "every send records the exact version and hash it used"
    );
    // Pinned, so a newer catalog version is not adopted on its own.
    use dgos_db::entities::skill_catalog;
    let mut catalog: skill_catalog::ActiveModel = skill_catalog::Entity::find_by_id(SKILL)
        .one(&fx.db)
        .await
        .unwrap()
        .unwrap()
        .into();
    catalog.latest_version = Set("2.0.0".into());
    catalog.update(&fx.db).await.unwrap();
    assert!(matches!(
        turn(&fx, &s, "again").await.unwrap(),
        ChatEvent::Done { .. }
    ));
    let versions = count(
        &fx.db,
        "SELECT COUNT(DISTINCT version_id) FROM workbench_task_skills",
    )
    .await;
    assert_eq!(versions, 1, "later sends still use the pinned version");
}

#[tokio::test]
async fn a_disabled_tampered_or_changed_skill_blocks_the_send_until_the_user_chooses_again() {
    let fx = fx().await;
    let s = session(&fx, Some(SKILL), false).await;
    turn(&fx, &s, "first").await.unwrap();
    let upstream_calls = fx.seen.lock().unwrap().len();
    let tasks = count(
        &fx.db,
        "SELECT COUNT(*) FROM workbench_tasks WHERE kind = 'chat'",
    )
    .await;

    fx.skills.toggle_skill(SKILL, ALICE, false).await.unwrap();
    assert_eq!(
        turn(&fx, &s, "x").await.err(),
        Some(ChatError::SkillUnavailable),
        "disabled"
    );
    fx.skills.toggle_skill(SKILL, ALICE, true).await.unwrap();

    let installed = fx
        .skills
        .list_installed_skills(ALICE)
        .await
        .unwrap()
        .remove(0);
    let skill_file = PathBuf::from(&installed.install_path).join("SKILL.md");
    let original = std::fs::read(&skill_file).unwrap();
    std::fs::write(&skill_file, "tampered").unwrap();
    assert_eq!(
        turn(&fx, &s, "x").await.err(),
        Some(ChatError::SkillUnavailable),
        "tampered"
    );
    std::fs::write(&skill_file, original).unwrap();

    fx.db
        .execute_unprepared("UPDATE chat_sessions SET skill_content_hash = 'not-the-pinned-hash'")
        .await
        .unwrap();
    assert_eq!(
        turn(&fx, &s, "x").await.err(),
        Some(ChatError::SkillUnavailable),
        "a different content hash is never accepted silently"
    );

    assert_eq!(
        fx.seen.lock().unwrap().len(),
        upstream_calls,
        "no blocked send reached the upstream"
    );
    assert_eq!(
        count(
            &fx.db,
            "SELECT COUNT(*) FROM workbench_tasks WHERE kind = 'chat'"
        )
        .await,
        tasks,
        "no task row, no silent fallback to no skill"
    );
    let user_messages = count(
        &fx.db,
        "SELECT COUNT(*) FROM chat_messages WHERE role = 'user'",
    )
    .await;
    assert_eq!(user_messages, 1, "a blocked send stores no message");

    // Only an explicit choice repairs the session: re-pin, or run without a skill.
    fx.chat
        .update_target(
            &fx.providers,
            &fx.skills,
            ALICE,
            UpdateChatSessionTargetInput {
                session_id: s.clone(),
                provider_id: Some(fx.provider_id.clone()),
                model_id: Some("chat".into()),
                skill_id: Some(SKILL.into()),
                no_skill: false,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        turn(&fx, &s, "after re-pin").await.unwrap(),
        ChatEvent::Done { .. }
    ));
    fx.chat
        .update_target(
            &fx.providers,
            &fx.skills,
            ALICE,
            UpdateChatSessionTargetInput {
                session_id: s.clone(),
                provider_id: Some(fx.provider_id.clone()),
                model_id: Some("chat".into()),
                skill_id: None,
                no_skill: true,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        turn(&fx, &s, "no skill now").await.unwrap(),
        ChatEvent::Done { .. }
    ));
    let last = fx.seen.lock().unwrap().last().unwrap().clone();
    assert_eq!(
        last["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["content"]
                .as_str()
                .is_some_and(|c| c.starts_with("Selected skill guidance")))
            .count(),
        0
    );
    let _ = &fx.server;
}

#[tokio::test]
async fn a_session_must_choose_a_skill_or_no_skill_before_the_first_send() {
    let fx = fx().await;
    let undecided = session(&fx, None, false).await;
    assert_eq!(
        turn(&fx, &undecided, "x").await.err(),
        Some(ChatError::SkillUnavailable)
    );
    assert!(fx.seen.lock().unwrap().is_empty());
    let none = session(&fx, None, true).await;
    assert!(matches!(
        turn(&fx, &none, "x").await.unwrap(),
        ChatEvent::Done { .. }
    ));
    assert_eq!(
        count(&fx.db, "SELECT COUNT(*) FROM workbench_task_skills").await,
        0,
        "no skill, no reference row"
    );
}

#[tokio::test]
async fn a_skill_the_user_has_not_added_cannot_be_pinned() {
    let fx = fx().await;
    let refused = fx
        .chat
        .create_session(
            &fx.providers,
            &fx.skills,
            BOB,
            CreateChatSessionInput {
                target: Some(ChatTargetInput {
                    provider_id: None,
                    model_id: None,
                    skill_id: Some(SKILL.into()),
                    no_skill: false,
                }),
            },
        )
        .await;
    assert_eq!(
        refused.err(),
        Some(ChatError::SkillUnavailable),
        "alice's installed skill is not bob's"
    );
    let both = fx
        .chat
        .create_session(
            &fx.providers,
            &fx.skills,
            ALICE,
            CreateChatSessionInput {
                target: Some(ChatTargetInput {
                    skill_id: Some(SKILL.into()),
                    no_skill: true,
                    ..target(&fx, None, false)
                }),
            },
        )
        .await;
    assert_eq!(
        both.err(),
        Some(ChatError::InvalidMessageContent),
        "a skill and no-skill are mutually exclusive"
    );
}
