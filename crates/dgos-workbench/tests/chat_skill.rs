//! FR-006 stage 5 (AC21): sessions may omit skills; selected versions are pinned, every
//! send revalidates them, and a failure blocks instead of dropping or swapping a skill (D075).
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
    RegenerateChatInput, SendChatInput, UpdateChatSessionTargetInput,
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
        .with_body("data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"summary\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
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
        chat: ChatManager::new(db.clone(), dgos_db::MaintenanceGate::new()),
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
        skill_ids: skill.map(str::to_owned).into_iter().collect(),
        generation: Default::default(),
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
            SendChatInput { skill_mode: Default::default(), authorized_skill_refs: Vec::new(), session_id: session.into(),
            foundation: Default::default(),
            content: text.into(),
            client_request_id: uuid::Uuid::new_v4().to_string(),
            accept_destination: None,
            tools: vec![],
            asset_ids: vec![], },
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
    assert_eq!(pinned.skills[0].skill_id, SKILL);
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

    let pinned = fx.chat.list_sessions(ALICE, Default::default()).await.unwrap()
        .items.remove(0).skills.remove(0);
    fx.db
        .execute_unprepared("UPDATE chat_sessions SET skill_refs = json_set(skill_refs, '$[0].version_id', 'missing-version')")
        .await
        .unwrap();
    assert_eq!(
        turn(&fx, &s, "x").await.err(),
        Some(ChatError::SkillUnavailable),
        "a missing pinned version is never accepted silently"
    );
    fx.db
        .execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE chat_sessions SET skill_refs = json_set(skill_refs, '$[0].version_id', ?, '$[0].content_hash', 'not-the-pinned-hash')",
            [pinned.version_id.into()],
        ))
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

    // A retained id cannot repair or replace a pin. Remove, then explicitly select it again.
    let reselect = UpdateChatSessionTargetInput { session_id: s.clone(), provider_id: Some(fx.provider_id.clone()), model_id: Some("chat".into()), skill_ids: vec![SKILL.into()], generation: Default::default(), no_skill: false };
    assert_eq!(fx.chat.update_target(&fx.providers, &fx.skills, ALICE, reselect.clone()).await.err(),
        Some(ChatError::SkillUnavailable));
    fx.chat.update_target(&fx.providers, &fx.skills, ALICE,
        UpdateChatSessionTargetInput { skill_ids: vec![], ..reselect }).await.unwrap();
    fx.chat
        .update_target(
            &fx.providers,
            &fx.skills,
            ALICE,
            UpdateChatSessionTargetInput { session_id: s.clone(), provider_id: Some(fx.provider_id.clone()), model_id: Some("chat".into()), skill_ids: Some(SKILL.into()).into_iter().collect(), generation: Default::default(), no_skill: false },
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
            UpdateChatSessionTargetInput { session_id: s.clone(),
            provider_id: Some(fx.provider_id.clone()),
            model_id: Some("chat".into()), skill_ids: vec![], generation: Default::default(), no_skill: false, },
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
async fn empty_skills_send_without_confirmation_in_new_and_reopened_sessions() {
    let mut fx = fx().await;
    let chosen: ChatTargetInput = serde_json::from_value(serde_json::json!({
        "provider_id": fx.provider_id,
        "model_id": "chat"
    })).unwrap();
    assert!(chosen.skill_ids.is_empty() && !chosen.no_skill);
    let created = fx.chat.create_session(
        &fx.providers, &fx.skills, ALICE,
        CreateChatSessionInput { target: Some(chosen) },
    ).await.unwrap();
    assert!(created.skills.is_empty() && !created.no_skill_confirmed);
    assert!(matches!(
        turn(&fx, &created.id, "default empty selection").await.unwrap(),
        ChatEvent::Done { .. }
    ));

    // The pre-existing false value must work after reopening, without a migration or target edit.
    let legacy = session(&fx, None, false).await;
    fx.chat = ChatManager::new(fx.db.clone(), dgos_db::MaintenanceGate::new());
    let saved = fx.chat.list_sessions(ALICE, Default::default()).await.unwrap().items
        .into_iter().find(|s| s.id == legacy).unwrap();
    assert!(saved.skills.is_empty() && !saved.no_skill_confirmed);
    let ChatEvent::Done { message, .. } = turn(&fx, &legacy, "legacy empty selection").await.unwrap() else { panic!("empty-skill legacy send did not complete"); };
    let regenerated = fx.chat.regenerate(
        &fx.providers, &fx.skills, ALICE, "main",
        RegenerateChatInput {
            skill_mode: Default::default(), authorized_skill_refs: Vec::new(),
            foundation: Default::default(),
            message_id: message.id,
            client_request_id: uuid::Uuid::new_v4().to_string(),
            accept_destination: None,
        },
    ).await.unwrap();
    let sink = Collect::default();
    fx.chat.run_generation(&fx.providers, regenerated.job.unwrap(), &sink).await;
    assert!(matches!(sink.0.lock().unwrap().last(), Some(ChatEvent::Done { .. })));
    let none = session(&fx, None, true).await;
    assert!(matches!(
        turn(&fx, &none, "legacy explicit flag").await.unwrap(),
        ChatEvent::Done { .. }
    ));
    assert_eq!(fx.seen.lock().unwrap().len(), 4);
    assert!(fx.seen.lock().unwrap().iter().all(|body| {
        body["messages"].as_array().unwrap().iter().all(|m| {
            !m["content"].as_str().unwrap_or_default().starts_with("Selected skill guidance")
        })
    }));
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
                target: Some(ChatTargetInput { provider_id: None, model_id: None, skill_ids: Some(SKILL.into()).into_iter().collect(), generation: Default::default(), no_skill: false }),
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
                    skill_ids: vec![SKILL.into()],
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

#[tokio::test]
async fn ordered_pins_and_controls_survive_session_read_and_reach_stream_request() {
    let fx = fx().await;
    let second = "dgos.official.writing.rewrite";
    fx.skills.install_skill(second, ALICE).await.unwrap();
    let mut chosen = target(&fx, Some(SKILL), false);
    chosen.skill_ids = vec![second.into(), SKILL.into()];
    chosen.generation = dgos_types::GenerationParameters { max_tokens: Some(4096), temperature: Some(0.4), reasoning: Some(dgos_types::ReasoningConfig { effort: Some("high".into()), budget_tokens: None }), instruction: "session guidance marker".into() };
    let created = fx.chat.create_session(&fx.providers, &fx.skills, ALICE, CreateChatSessionInput { target: Some(chosen.clone()) }).await.unwrap();
    let saved = fx.chat.list_sessions(ALICE, Default::default()).await.unwrap().items.remove(0);
    assert_eq!(saved.generation, chosen.generation);
    assert_eq!(saved.skills.iter().map(|s| s.skill_id.as_str()).collect::<Vec<_>>(), vec![second, SKILL]);
    assert!(matches!(turn(&fx, &created.id, "summarize").await.unwrap(), ChatEvent::Done { .. }));
    {
        let bodies = fx.seen.lock().unwrap();
        let wire = bodies.last().unwrap();
        assert_eq!(wire["max_tokens"], 4096);
        assert_eq!(wire["reasoning_effort"], "high");
        assert!(wire["messages"][1]["content"].as_str().unwrap().contains(second));
        assert!(wire["messages"][2]["content"].as_str().unwrap().contains(SKILL));
        assert!(wire["messages"][3]["content"].as_str().unwrap().contains("session guidance marker"));
    }
    chosen.skill_ids.push(SKILL.into());
    assert!(matches!(fx.chat.create_session(&fx.providers, &fx.skills, ALICE, CreateChatSessionInput { target: Some(chosen) }).await, Err(ChatError::InvalidMessageContent)));
}

#[tokio::test]
async fn unsupported_budget_and_output_limit_refuse_before_session_commit_or_dispatch() {
    let fx = fx().await;
    let mut chosen = target(&fx, None, true);
    chosen.generation.reasoning = Some(dgos_types::ReasoningConfig { effort: Some("high".into()), budget_tokens: Some(2048) });
    assert!(fx.chat.create_session(&fx.providers, &fx.skills, ALICE, CreateChatSessionInput { target: Some(chosen.clone()) }).await.is_err());
    chosen.generation.reasoning = None;
    chosen.generation.max_tokens = Some(65537);
    assert!(fx.chat.create_session(&fx.providers, &fx.skills, ALICE, CreateChatSessionInput { target: Some(chosen) }).await.is_err());
    assert!(fx.chat.list_sessions(ALICE, Default::default()).await.unwrap().items.is_empty());
    assert!(fx.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn regeneration_honors_current_on_demand_selection_and_rejects_manual_pins() {
    let fx=fx().await;let s=session(&fx,None,true).await;
    let ChatEvent::Done{message,..}=turn(&fx,&s,"first question").await.unwrap() else {panic!("initial reply")};
    let prepared=fx.skills.prepare_content_skill(SKILL,ALICE).await.unwrap();
    let pin=dgos_types::AiSkillRef{skill_id:prepared.skill_id,version:prepared.version,content_hash:prepared.content_hash};
    let manual=fx.chat.regenerate(&fx.providers,&fx.skills,ALICE,"main",RegenerateChatInput{message_id:message.id.clone(),client_request_id:uuid::Uuid::new_v4().to_string(),accept_destination:None,foundation:Default::default(),skill_mode:dgos_workbench::ChatSkillMode::Manual,authorized_skill_refs:vec![pin.clone()]}).await;
    assert!(matches!(manual,Err(ChatError::SkillUnavailable)));
    let sent=fx.chat.regenerate(&fx.providers,&fx.skills,ALICE,"main",RegenerateChatInput{message_id:message.id,client_request_id:uuid::Uuid::new_v4().to_string(),accept_destination:None,foundation:Default::default(),skill_mode:dgos_workbench::ChatSkillMode::OnDemand,authorized_skill_refs:vec![pin]}).await.unwrap();
    assert!(sent.job.as_ref().unwrap().is_tool_turn(),"current explicit pins select actual local runtime");
    let job=sent.job.unwrap();let task=sent.ack.task_id;
    assert_eq!(count(&fx.db, &format!("SELECT COUNT(*) FROM workbench_task_skills WHERE task_id='{task}' AND skill_id='{SKILL}'")).await, 1, "regeneration audit pins the actual on-demand runtime selection");
    let skills=Arc::new(tokio::sync::RwLock::new(fx.skills));let mcp=dgos_mcp::McpManager::new(fx.db.clone());let sink=Arc::new(Collect::default());
    fx.chat.run_tool_turn(&fx.providers,&mcp,skills.clone(),job,sink.clone()).await;
    {
        let events = sink.0.lock().unwrap();
        assert!(matches!(events.last(), Some(ChatEvent::Done { .. })), "regeneration events: {events:?}");
    }
    let run=fx.chat.get_run(ALICE,dgos_workbench::GetChatRunInput{task_id:task}).await.unwrap();assert_eq!(run.status,"completed");
    let wire=fx.seen.lock().unwrap();let request=wire.last().unwrap();
    assert!(request["tools"].as_array().unwrap().iter().any(|tool|tool["function"]["name"].as_str().is_some_and(|name|name.starts_with("dgos_skill_"))));
    assert!(skills.try_write().is_ok());
}
