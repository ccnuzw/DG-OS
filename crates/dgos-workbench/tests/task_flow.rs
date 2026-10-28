use dgos_db::{
    migrations::Migrator,
    sea_orm::{ActiveModelTrait, Database, DatabaseConnection, EntityTrait, Set},
    sea_orm_migration::MigratorTrait,
    ModelRepository,
};
use dgos_providers::{
    test_support::{seed_provider, MemoryCredentialVault},
    ProviderManager,
};
use dgos_skills::SkillsManager;
use dgos_types::{Model, ProviderConfig};
use dgos_workbench::{TaskInput, WorkbenchManager};
use std::path::PathBuf;

const SKILL: &str = "dgos.official.writing.summarize";
const PROMPT: &str =
    "Private task material: Team decided to ship on Friday. Summarize in one sentence.";
struct Fixture {
    db: DatabaseConnection,
    skills: SkillsManager,
    workbench: WorkbenchManager,
    providers: ProviderManager,
    input: TaskInput,
    _dir: tempfile::TempDir,
}
async fn setup(base_url: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("test.db").display());
    let db = Database::connect(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let skills = SkillsManager::new(db.clone(), dir.path().join("skills"));
    skills
        .register_bundled_catalog(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills"))
        .await
        .unwrap();
    skills.install_skill(SKILL, "alice").await.unwrap();
    let vault = std::sync::Arc::new(MemoryCredentialVault::default());
    let provider = seed_provider(
        db.clone(),
        &vault,
        ProviderConfig {
            name: "Test provider".into(),
            provider_type: "openai_compatible".into(),
            base_url: base_url.into(),
            api_key: "test-only-secret".into(),
            enabled: true,
            config: None,
        },
        "alice",
    )
    .await
    .unwrap();
    ModelRepository::new(db.clone())
        .sync_models(
            &provider.id,
            vec![Model {
                id: uuid::Uuid::new_v4().to_string(),
                provider_id: provider.id.clone(),
                model_id: "test-chat".into(),
                model_name: "Test chat".into(),
                model_type: "chat".into(),
                context_window: Some(32768),
                max_output_tokens: None,
                supports_streaming: false,
                supports_vision: false,
                cached_at: String::new(),
            }],
        )
        .await
        .unwrap();
    Fixture {
        workbench: WorkbenchManager::new(db.clone()),
        providers: ProviderManager::with_vault(&url, vault).await.unwrap(),
        db,
        skills,
        input: TaskInput {
            provider_id: provider.id,
            model_id: "test-chat".into(),
            prompt: PROMPT.into(),
            skill_id: Some(SKILL.into()),
            allow_without_skill: false,
            client_task_id: None,
        },
        _dir: dir,
    }
}

#[tokio::test]
async fn ac20_selected_version_is_bound_and_never_elevated_to_system() {
    let f = setup("http://unused.invalid").await;
    let prepared = f
        .workbench
        .prepare(&f.skills, &f.input, "alice", Some(32768))
        .await
        .unwrap();
    assert_eq!(prepared.record.skills.len(), 1);
    assert_eq!(
        prepared.record.skills[0].version_id,
        format!("{SKILL}@1.0.0")
    );
    assert_eq!(prepared.request.messages.len(), 3);
    assert_eq!(prepared.request.messages[0].role, "system");
    assert!(!prepared.request.messages[0].content.contains("执行步骤"));
    assert_eq!(prepared.request.messages[1].role, "user");
    assert!(prepared.request.messages[1]
        .content
        .contains("name: summarize"));
    assert!(!prepared.request.messages[1]
        .content
        .contains("name: rewrite"));
    assert_eq!(prepared.request.messages[2].content, PROMPT);
    let audit = serde_json::to_string(&prepared.record).unwrap();
    assert!(!audit.contains(PROMPT));
    assert!(!audit.contains("test-only-secret"));
}

#[tokio::test]
async fn ac22_disabled_missing_and_tampered_skills_fail_without_task_or_fallback() {
    let f = setup("http://unused.invalid").await;
    assert!(f
        .workbench
        .prepare(&f.skills, &f.input, "bob", None)
        .await
        .is_err());
    f.skills.toggle_skill(SKILL, "alice", false).await.unwrap();
    assert!(f
        .workbench
        .prepare(&f.skills, &f.input, "alice", None)
        .await
        .is_err());
    f.skills.toggle_skill(SKILL, "alice", true).await.unwrap();
    let installed = f
        .skills
        .list_installed_skills("alice")
        .await
        .unwrap()
        .remove(0);
    std::fs::write(
        PathBuf::from(installed.install_path).join("SKILL.md"),
        "tampered",
    )
    .unwrap();
    assert!(f
        .workbench
        .prepare(&f.skills, &f.input, "alice", None)
        .await
        .is_err());
    assert!(f.workbench.list("alice").await.unwrap().is_empty());
}

#[tokio::test]
async fn ac22_budget_and_explicit_no_skill_are_enforced_without_truncation() {
    let mut f = setup("http://unused.invalid").await;
    assert!(f
        .workbench
        .prepare(&f.skills, &f.input, "alice", Some(1000))
        .await
        .is_err());
    f.input.prompt = "中".repeat(6000);
    assert!(f
        .workbench
        .prepare(&f.skills, &f.input, "alice", None)
        .await
        .is_err());
    f.input.prompt = PROMPT.into();
    f.input.skill_id = None;
    assert!(f
        .workbench
        .prepare(&f.skills, &f.input, "alice", None)
        .await
        .is_err());
    f.input.allow_without_skill = true;
    let prepared = f
        .workbench
        .prepare(&f.skills, &f.input, "alice", None)
        .await
        .unwrap();
    assert!(prepared.record.skills.is_empty());
    assert_eq!(prepared.request.messages.len(), 2);
    assert_eq!(prepared.request.messages[1].content, PROMPT);
}

#[tokio::test]
async fn ac21_catalog_latest_does_not_change_the_users_selected_version() {
    use dgos_db::entities::skill_catalog;
    let f = setup("http://unused.invalid").await;
    let mut catalog: skill_catalog::ActiveModel = skill_catalog::Entity::find_by_id(SKILL)
        .one(&f.db)
        .await
        .unwrap()
        .unwrap()
        .into();
    catalog.latest_version = Set("2.0.0".into());
    catalog.update(&f.db).await.unwrap();
    let prepared = f
        .workbench
        .prepare(&f.skills, &f.input, "alice", None)
        .await
        .unwrap();
    assert_eq!(
        prepared.record.skills[0].version_id,
        format!("{SKILL}@1.0.0")
    );
}

#[tokio::test]
async fn ac24_35_recovery_retains_version_references_and_user_isolation() {
    let f = setup("http://unused.invalid").await;
    let prepared = f
        .workbench
        .prepare(&f.skills, &f.input, "alice", None)
        .await
        .unwrap();
    f.skills.toggle_skill(SKILL, "alice", false).await.unwrap();
    f.workbench.recover_interrupted().await.unwrap();
    f.workbench.recover_interrupted().await.unwrap();
    let audit = f.workbench.list("alice").await.unwrap();
    assert_eq!(audit[0].task.status, "interrupted");
    assert_eq!(
        audit[0].skills[0].content_hash,
        prepared.record.skills[0].content_hash
    );
    assert!(f.workbench.list("bob").await.unwrap().is_empty());
}

#[tokio::test]
async fn ac21_35_http_mock_runs_exact_selected_context_and_saves_only_safe_audit() {
    let mut server = mockito::Server::new_async().await;
    let f = setup(&format!("{}/v1", server.url())).await;
    let http = server.mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-only-secret")
        .match_body(mockito::Matcher::JsonString(serde_json::to_string(&serde_json::json!({
            "model": "test-chat", "messages": f.workbench.prepare(&f.skills, &f.input, "alice", None).await.unwrap().request.messages,
            "max_tokens": 1024, "stream": false
        })).unwrap()))
        .with_status(200).with_body(r#"{"choices":[{"message":{"content":"Ship on Friday."}}],"usage":{"prompt_tokens":60,"completion_tokens":5,"total_tokens":65}}"#)
        .expect(1).create_async().await;
    let result = f
        .workbench
        .run(&f.skills, &f.providers, f.input.clone(), "alice")
        .await
        .unwrap();
    http.assert_async().await;
    assert_eq!(result.content.as_deref(), Some("Ship on Friday."));
    assert_eq!(result.record.task.status, "succeeded");
    assert_eq!(result.record.task.total_tokens, Some(65));
    f.workbench.recover_interrupted().await.unwrap();
    let audit = f.workbench.list("alice").await.unwrap();
    let success = audit
        .iter()
        .find(|r| r.task.id == result.record.task.id)
        .unwrap();
    assert_eq!(success.task.status, "succeeded");
    let serialized = serde_json::to_string(&audit).unwrap();
    assert!(!serialized.contains(PROMPT));
    assert!(!serialized.contains("Ship on Friday."));
    assert!(!serialized.contains("test-only-secret"));
    let skill_usage = f.skills.list_usage_logs("alice", None).await.unwrap();
    let skill_audit = skill_usage
        .iter()
        .find(|log| log.task_ref.as_deref() == Some(result.record.task.id.as_str()))
        .expect("Workbench skill preparation should retain host provenance");
    assert_eq!(skill_audit.caller.as_deref(), Some("workbench"));
    assert_eq!(
        skill_audit.request_id.as_deref(),
        Some(result.record.task.id.as_str())
    );
    assert_eq!(skill_audit.success, true);
    assert_eq!(
        skill_audit.input_params,
        serde_json::json!({"operation": "prepare_content_skill"})
    );
}

#[tokio::test]
async fn shutdown_interrupts_text_task_without_dispatch_or_replay() {
    let mut server = mockito::Server::new_async().await;
    let f = setup(&server.url()).await;
    let http = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(r#"{"choices":[{"message":{"content":"must not dispatch"}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#)
        .expect(0)
        .create_async()
        .await;
    f.workbench.shutdown().await.unwrap();
    let result = f
        .workbench
        .run(&f.skills, &f.providers, f.input.clone(), "alice")
        .await
        .unwrap();
    assert_eq!(result.record.task.status, "interrupted");
    assert_eq!(
        result.record.task.error_code.as_deref(),
        Some("process_interrupted")
    );
    assert!(result.content.is_none());
    http.assert_async().await;
}

#[tokio::test]
async fn cancel_text_task_interrupts_inflight_provider_without_replay() {
    let mut server = mockito::Server::new_async().await;
    let f = setup(&server.url()).await;
    let http = server
        .mock("POST", "/chat/completions")
        .with_chunked_body(|writer| {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let _ = std::io::Write::write_all(
                writer,
                br#"{"choices":[{"message":{"content":"late"}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
            );
            Ok(())
        })
        .expect(1)
        .create_async()
        .await;
    let task_id = uuid::Uuid::new_v4().to_string();
    let mut input = f.input.clone();
    input.client_task_id = Some(task_id.clone());
    let workbench = f.workbench.clone();
    let skills = f.skills;
    let providers = f.providers;
    let run = tokio::spawn(async move { workbench.run(&skills, &providers, input, "alice").await });
    let mut dispatched = false;
    for _ in 0..50 {
        if http.matched_async().await {
            dispatched = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        dispatched,
        "provider request should be observed before cancellation"
    );
    assert!(f
        .workbench
        .cancel_text_task(&task_id, "alice")
        .await
        .unwrap());
    let result = run.await.unwrap().unwrap();
    assert_eq!(result.record.task.status, "interrupted");
    assert_eq!(
        result.record.task.error_code.as_deref(),
        Some("task_cancelled")
    );
    assert!(result.content.is_none());
    http.assert_async().await;
}

#[tokio::test]
async fn ac35_provider_failure_has_a_task_id_and_redacted_error_not_success() {
    let mut server = mockito::Server::new_async().await;
    let f = setup(&server.url()).await;
    let http = server
        .mock("POST", "/chat/completions")
        .with_status(401)
        .with_body("secret upstream detail")
        .expect(1)
        .create_async()
        .await;
    let result = f
        .workbench
        .run(&f.skills, &f.providers, f.input.clone(), "alice")
        .await
        .unwrap();
    http.assert_async().await;
    assert_eq!(result.record.task.status, "failed");
    assert!(result.content.is_none());
    assert_eq!(
        result.record.task.error_code.as_deref(),
        Some("provider_1001002")
    );
    assert!(!serde_json::to_string(&result.record)
        .unwrap()
        .contains("secret upstream"));
}

#[tokio::test]
async fn ac24_foreign_provider_and_unknown_model_are_rejected_before_sending() {
    let mut f = setup("http://unused.invalid").await;
    assert!(f
        .workbench
        .run(&f.skills, &f.providers, f.input.clone(), "bob")
        .await
        .is_err());
    f.input.model_id = "not-in-cache".into();
    assert!(f
        .workbench
        .run(&f.skills, &f.providers, f.input, "alice")
        .await
        .is_err());
    assert!(f.workbench.list("alice").await.unwrap().is_empty());
}

#[tokio::test]
async fn provider_log_alignment_preserves_and_reads_historical_usage_rows() {
    use dgos_db::sea_orm::{ConnectionTrait, DbBackend, Statement};
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, Some(6)).await.unwrap();
    // Seed the historical schema directly; the current repository requires
    // the later credential-reference migration.
    db.execute_unprepared("INSERT INTO providers (id, name, provider_type, base_url, api_key, enabled, sort_order, user_id, created_at, updated_at) VALUES ('old-provider', 'Old provider', 'openai_compatible', 'http://unused.invalid', '', 1, 0, 'alice', '2026-10-27T00:00:00+00:00', '2026-10-27T00:00:00+00:00');")
        .await.unwrap();
    db.execute_unprepared("INSERT INTO usage_logs (id, provider_id, model_id, user_id, request_tokens, response_tokens, total_tokens, latency_ms, success, created_at) SELECT 'old-log', id, 'old-model', 'alice', 2, 3, 5, 10, 1, '2026-10-27T00:00:00+00:00' FROM providers;").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let logs = dgos_db::UsageLogRepository::new(db.clone())
        .list("alice", None)
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].id, "old-log");
    assert_eq!(logs[0].total_tokens, 5);
    let source = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) AS n FROM usage_logs",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source.try_get::<i64>("", "n").unwrap(), 1);
}
