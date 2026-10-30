use async_trait::async_trait;
use dgos_db::{
    migrations::Migrator,
    sea_orm::{
        ActiveModelTrait, ColumnTrait, Database, DatabaseConnection, EntityTrait, QueryFilter, Set,
    },
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
    TaskInput, ToolAction, ToolSelection, ToolTaskInput, ToolTaskUpdate, WorkbenchManager,
};
use serde_json::{json, Value};
use std::sync::Arc;

struct Allow;
#[async_trait]
impl McpCallPolicy for Allow {
    async fn allows_call(&self, user: &str, _: &str, _: &str) -> bool {
        user == "alice"
    }
}
struct Fixture {
    db: DatabaseConnection,
    workbench: WorkbenchManager,
    providers: ProviderManager,
    skills: SkillsManager,
    mcp: McpManager,
    tool: McpTool,
    input: TaskInput,
    _dir: tempfile::TempDir,
}
impl Fixture {
    fn tool_input(&self) -> ToolTaskInput {
        ToolTaskInput {
            task: self.input.clone(),
            tools: vec![ToolSelection {
                server_id: self.tool.server_id.clone(),
                tool_name: self.tool.tool_name.clone(),
                schema_hash: self.tool.schema_hash.clone(),
            }],
        }
    }
    async fn start(&self) -> ToolTaskUpdate {
        self.workbench
            .start_tool_task(
                &self.skills,
                &self.providers,
                &self.mcp,
                self.tool_input(),
                "alice",
            )
            .await
            .unwrap()
    }
    async fn propose(&self) -> ToolTaskUpdate {
        let started = self.start().await;
        self.workbench
            .advance_tool_task(
                &self.providers,
                &self.mcp,
                &started.record.task.id,
                "alice",
                ToolAction::RunModel,
            )
            .await
            .unwrap()
    }
    async fn stop(&self) {
        self.mcp
            .disconnect("alice", &self.tool.server_id)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn graceful_shutdown_revokes_proposal_and_records_interruption_without_dispatch() {
    let mut server = mockito::Server::new_async().await;
    let model = proposal_mock(
        &mut server,
        proposed("dgos_tool_0", json!({"text":"synthetic-shutdown"})),
    )
    .await;
    let fixture = setup(&server.url()).await;
    let started = fixture.propose().await;
    let proposal = started.proposal.unwrap();
    fixture.workbench.shutdown().await.unwrap();
    let record = fixture.workbench.list("alice").await.unwrap().remove(0);
    assert_eq!(record.task.id, started.record.task.id);
    assert_eq!(record.task.status, "interrupted");
    assert_eq!(
        record.task.error_code.as_deref(),
        Some("process_interrupted")
    );
    assert!(fixture
        .mcp
        .list_usage_logs("alice", None, None)
        .await
        .unwrap()
        .is_empty());
    assert!(fixture
        .workbench
        .advance_tool_task(
            &fixture.providers,
            &fixture.mcp,
            &started.record.task.id,
            "alice",
            ToolAction::Approve {
                proposal_id: proposal.id
            }
        )
        .await
        .is_err());
    assert!(fixture
        .workbench
        .start_tool_task(
            &fixture.skills,
            &fixture.providers,
            &fixture.mcp,
            fixture.tool_input(),
            "alice"
        )
        .await
        .is_err());
    fixture.workbench.shutdown().await.unwrap();
    assert_eq!(
        fixture.workbench.list("alice").await.unwrap()[0]
            .task
            .status,
        "interrupted"
    );
    fixture.stop().await;
    model.assert_async().await;
}
async fn setup(url: &str) -> Fixture {
    setup_mode(url, "normal").await
}

async fn setup_mode(url: &str, mode: &str) -> Fixture {
    setup_provider(url, mode, "openai_compatible").await
}

async fn setup_provider(url: &str, mode: &str, provider_type: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db_url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("fixture.db").display()
    );
    let db = Database::connect(&db_url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let vault = Arc::new(MemoryCredentialVault::default());
    let provider = seed_provider(
        db.clone(),
        &vault,
        ProviderConfig {
            name: "fixture".into(),
            provider_type: provider_type.into(),
            base_url: url.into(),
            api_key: "synthetic-secret".into(),
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
    let server = mcp
        .create_server(
            "alice",
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
                        mode.into(),
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
    mcp.connect("alice", &server.id).await.unwrap();
    let tool = mcp.list_tools("alice", &server.id).await.unwrap().remove(0);
    Fixture {
        skills: SkillsManager::new(db.clone(), dir.path().join("skills")),
        workbench: WorkbenchManager::new(db.clone()),
        providers: ProviderManager::with_vault(&db_url, vault).await.unwrap(),
        db,
        mcp,
        tool,
        input: TaskInput {
            provider_id: provider.id,
            model_id: "chat".into(),
            prompt: "private task input".into(),
            skill_id: None,
            allow_without_skill: true,
            client_task_id: None,
        },
        _dir: dir,
    }
}
fn proposed(name: &str, arguments: Value) -> Value {
    json!({"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{
        "id":"call_1","type":"function","function":{"name":name,"arguments":arguments.to_string()}
    }]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}})
}
async fn proposal_mock(server: &mut mockito::ServerGuard, body: Value) -> mockito::Mock {
    server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Regex("\"role\":\"user\"".into()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(body.to_string())
        .expect(1)
        .create_async()
        .await
}

#[tokio::test]
async fn normal_vault_backed_workbench_tool_task_completes() {
    let mut server = mockito::Server::new_async().await;
    let first = proposal_mock(
        &mut server,
        proposed("dgos_tool_0", json!({"text":"synthetic-vault-flow"})),
    )
    .await;
    let second = server.mock("POST", "/chat/completions")
        .match_request(|request| {
            let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
            body["messages"].as_array().is_some_and(|messages| messages.last()
                .is_some_and(|message| message["role"] == "tool" && message["tool_call_id"] == "call_1"))
        })
        .with_status(200).with_header("content-type", "application/json")
        .with_body(json!({"choices":[{"message":{"role":"assistant","content":"Completed"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":8,"completion_tokens":2,"total_tokens":10}}).to_string())
        .expect(1).create_async().await;
    let fixture = setup(&server.url()).await;
    let update = fixture.propose().await;
    assert_eq!(update.record.task.provider_config_revision, Some(0));
    let proposal = update.proposal.unwrap();
    let done = fixture
        .workbench
        .advance_tool_task(
            &fixture.providers,
            &fixture.mcp,
            &update.record.task.id,
            "alice",
            ToolAction::Approve {
                proposal_id: proposal.id,
            },
        )
        .await
        .unwrap();
    assert_eq!(done.record.task.status, "succeeded");
    assert_eq!(done.content.as_deref(), Some("Completed"));
    assert_eq!(done.record.task.provider_config_revision, Some(0));
    let usage = fixture
        .mcp
        .list_usage_logs("alice", None, None)
        .await
        .unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(
        usage[0].task_ref.as_deref(),
        Some(done.record.task.id.as_str())
    );
    assert_eq!(usage[0].caller.as_deref(), Some("workbench"));
    first.assert_async().await;
    second.assert_async().await;
    fixture.stop().await;
}

#[tokio::test]
async fn provider_configuration_change_ends_bound_task_before_next_dispatch() {
    let mut server = mockito::Server::new_async().await;
    let model = proposal_mock(
        &mut server,
        proposed("dgos_tool_0", json!({"text":"synthetic-config-lifecycle"})),
    )
    .await;
    let fixture = setup(&server.url()).await;
    let started = fixture.propose().await;
    assert_eq!(started.record.task.provider_config_revision, Some(0));
    let updated = fixture
        .providers
        .update_provider(
            &fixture.input.provider_id,
            ProviderConfig {
                name: "Updated metadata".into(),
                provider_type: "openai_compatible".into(),
                base_url: server.url(),
                api_key: String::new(),
                enabled: true,
                config: None,
            },
            "alice",
        )
        .await
        .unwrap();
    assert_eq!(updated.config_revision, 1);
    let done = fixture
        .workbench
        .advance_tool_task(
            &fixture.providers,
            &fixture.mcp,
            &started.record.task.id,
            "alice",
            ToolAction::Approve {
                proposal_id: started.proposal.unwrap().id,
            },
        )
        .await
        .unwrap();
    assert_eq!(done.record.task.status, "failed");
    assert_eq!(
        done.record.task.error_code.as_deref(),
        Some("provider_1001012")
    );
    assert_eq!(done.record.task.provider_config_revision, Some(0));
    assert!(fixture
        .mcp
        .list_usage_logs("alice", None, None)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        fixture
            .providers
            .list_usage_logs("alice", None)
            .await
            .unwrap()
            .len(),
        1
    );
    model.assert_async().await;
    fixture.stop().await;
}

#[tokio::test]
async fn model_proposal_waits_for_exact_approval_then_continues_with_correlated_result() {
    let mut server = mockito::Server::new_async().await;
    let first = proposal_mock(
        &mut server,
        proposed("dgos_tool_0", json!({"text":"private tool data"})),
    )
    .await;
    let second = server.mock("POST", "/chat/completions")
        .match_request(|request| {
            let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
            body["messages"].as_array().is_some_and(|messages| messages.last().is_some_and(|message|
                message["role"] == "tool" && message["tool_call_id"] == "call_1"
                && message["content"].as_str().is_some_and(|content| content.contains("private tool data"))))
        })
        .match_body(mockito::Matcher::PartialJson(json!({"parallel_tool_calls":false,"tools":[{"type":"function","function":{
            "name":"dgos_tool_0","description":"fixture-server / echo: Untrusted external tool","parameters":{
                "type":"object","required":["text"],"additionalProperties":false,"properties":{
                    "text":{"type":"string","minLength":1},"delay":{"type":"integer","minimum":0},"fail":{"type":"boolean"}
                }}
        }}]})))
        .with_status(200).with_header("content-type", "application/json")
        .with_body(json!({"choices":[{"message":{"role":"assistant","content":"Final answer"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":3,"total_tokens":13}}).to_string()).expect(1).create_async().await;
    let f = setup(&server.url()).await;
    let update = f.propose().await;
    assert_eq!(update.phase, "awaiting_confirmation");
    assert!(f
        .mcp
        .list_usage_logs("alice", None, None)
        .await
        .unwrap()
        .is_empty());
    let id = &update.record.task.id;
    assert_eq!(update.record.tools[0].schema_hash, f.tool.schema_hash);
    assert!(f
        .workbench
        .advance_tool_task(
            &f.providers,
            &f.mcp,
            id,
            "alice",
            ToolAction::Approve {
                proposal_id: "stale".into()
            }
        )
        .await
        .is_err());
    let proposal = update.proposal.unwrap();
    let done = f
        .workbench
        .advance_tool_task(
            &f.providers,
            &f.mcp,
            id,
            "alice",
            ToolAction::Approve {
                proposal_id: proposal.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(done.record.task.status, "succeeded");
    assert_eq!(done.content.as_deref(), Some("Final answer"));
    assert_eq!(done.record.task.total_tokens, Some(20));
    let logs = f.mcp.list_usage_logs("alice", None, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].task_ref.as_deref(), Some(id.as_str()));
    assert!(f
        .workbench
        .advance_tool_task(
            &f.providers,
            &f.mcp,
            id,
            "alice",
            ToolAction::Approve {
                proposal_id: proposal.id
            }
        )
        .await
        .is_err());
    let audit = serde_json::to_string(&f.workbench.list("alice").await.unwrap()).unwrap();
    for private in [
        "private task input",
        "private tool data",
        "synthetic-secret",
        "call_1",
        "Final answer",
    ] {
        assert!(!audit.contains(private));
    }
    first.assert_async().await;
    second.assert_async().await;
    f.stop().await;
}

#[tokio::test]
async fn rejection_stops_without_dispatch_or_followup() {
    let mut server = mockito::Server::new_async().await;
    let first = proposal_mock(
        &mut server,
        proposed("dgos_tool_0", json!({"text":"private"})),
    )
    .await;
    let f = setup(&server.url()).await;
    let update = f.propose().await;
    let done = f
        .workbench
        .advance_tool_task(
            &f.providers,
            &f.mcp,
            &update.record.task.id,
            "alice",
            ToolAction::Deny {
                proposal_id: update.proposal.unwrap().id,
            },
        )
        .await
        .unwrap();
    assert_eq!(done.record.task.error_code.as_deref(), Some("tool_denied"));
    assert!(f
        .mcp
        .list_usage_logs("alice", None, None)
        .await
        .unwrap()
        .is_empty());
    first.assert_async().await;
    f.stop().await;
}

#[tokio::test]
async fn schema_pinning_rejects_changed_contract_before_dispatch() {
    use dgos_db::entities::mcp_server_tools;
    let mut server = mockito::Server::new_async().await;
    let first = proposal_mock(
        &mut server,
        proposed("dgos_tool_0", json!({"text":"private"})),
    )
    .await;
    let f = setup(&server.url()).await;
    let update = f.propose().await;
    let row = mcp_server_tools::Entity::find()
        .filter(mcp_server_tools::Column::ServerId.eq(&f.tool.server_id))
        .filter(mcp_server_tools::Column::ToolName.eq(&f.tool.tool_name))
        .one(&f.db)
        .await
        .unwrap()
        .unwrap();
    let mut active: mcp_server_tools::ActiveModel = row.into();
    active.schema_hash = Set("changed".into());
    active.update(&f.db).await.unwrap();
    let done = f
        .workbench
        .advance_tool_task(
            &f.providers,
            &f.mcp,
            &update.record.task.id,
            "alice",
            ToolAction::Approve {
                proposal_id: update.proposal.unwrap().id,
            },
        )
        .await
        .unwrap();
    assert_eq!(done.record.task.status, "failed");
    assert_eq!(
        done.record.task.error_code.as_deref(),
        Some("mcp_tool_input_invalid")
    );
    assert!(f
        .mcp
        .list_usage_logs("alice", None, None)
        .await
        .unwrap()
        .iter()
        .all(|log| log.outcome == "denied"));
    first.assert_async().await;
    f.stop().await;
}

#[tokio::test]
async fn output_only_contract_refresh_revokes_pending_workbench_proposal() {
    let mut server = mockito::Server::new_async().await;
    let first = proposal_mock(
        &mut server,
        proposed("dgos_tool_0", json!({"text":"private-output-contract"})),
    )
    .await;
    let f = setup_mode(&server.url(), "output-change").await;
    let update = f.propose().await;
    let refreshed = f
        .mcp
        .refresh_tools("alice", &f.tool.server_id)
        .await
        .unwrap();
    let tool = refreshed
        .iter()
        .find(|tool| tool.tool_name == f.tool.tool_name)
        .unwrap();
    assert_eq!(tool.input_schema, f.tool.input_schema);
    assert_ne!(tool.output_schema, f.tool.output_schema);
    assert_ne!(tool.schema_hash, f.tool.schema_hash);
    let done = f
        .workbench
        .advance_tool_task(
            &f.providers,
            &f.mcp,
            &update.record.task.id,
            "alice",
            ToolAction::Approve {
                proposal_id: update.proposal.unwrap().id,
            },
        )
        .await
        .unwrap();
    assert_eq!(done.record.task.status, "failed");
    assert_eq!(
        done.record.task.error_code.as_deref(),
        Some("mcp_tool_input_invalid")
    );
    let logs = f.mcp.list_usage_logs("alice", None, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].outcome, "denied");
    first.assert_async().await;
    f.stop().await;
}

#[tokio::test]
async fn invalid_tool_results_stop_before_model_followup_and_keep_redacted_audit() {
    for mode in ["output-missing", "output-wrong", "malformed-result"] {
        let mut server = mockito::Server::new_async().await;
        let first = proposal_mock(
            &mut server,
            proposed("dgos_tool_0", json!({"text":"private-invalid-output"})),
        )
        .await;
        let f = setup_mode(&server.url(), mode).await;
        let update = f.propose().await;
        let done = f
            .workbench
            .advance_tool_task(
                &f.providers,
                &f.mcp,
                &update.record.task.id,
                "alice",
                ToolAction::Approve {
                    proposal_id: update.proposal.unwrap().id,
                },
            )
            .await
            .unwrap();
        assert_eq!(done.record.task.status, "failed", "{mode}");
        assert_eq!(
            done.record.task.error_code.as_deref(),
            Some("mcp_unknown_result")
        );
        assert!(done.content.is_none());
        let logs = f.mcp.list_usage_logs("alice", None, None).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].outcome, "error");
        assert_eq!(logs[0].error_class.as_deref(), Some("mcp_protocol_error"));
        let audit =
            serde_json::to_string(&(logs, f.workbench.list("alice").await.unwrap())).unwrap();
        assert!(!audit.contains("private-invalid-output"));
        first.assert_async().await;
        f.stop().await;
    }
}

#[tokio::test]
async fn task_is_user_scoped_single_active_and_cancelled_without_replay() {
    let f = setup("http://unused.invalid").await;
    let started = f.start().await;
    assert!(f
        .workbench
        .start_tool_task(&f.skills, &f.providers, &f.mcp, f.tool_input(), "alice")
        .await
        .is_err());
    assert!(f
        .workbench
        .cancel_tool_task(&started.record.task.id, "bob")
        .await
        .is_err());
    let done = f
        .workbench
        .cancel_tool_task(&started.record.task.id, "alice")
        .await
        .unwrap();
    assert_eq!(
        done.record.task.error_code.as_deref(),
        Some("task_cancelled")
    );
    let next = f.start().await;
    f.workbench
        .cancel_tool_task(&next.record.task.id, "alice")
        .await
        .unwrap();
    assert!(f
        .mcp
        .list_usage_logs("alice", None, None)
        .await
        .unwrap()
        .is_empty());
    f.stop().await;
}

#[tokio::test]
async fn unselected_tool_and_invalid_arguments_stop_before_confirmation() {
    for (name, arguments, code) in [
        ("unselected", json!({"text":"private"}), "unselected_tool"),
        ("dgos_tool_0", json!({"text":42}), "invalid_tool_arguments"),
    ] {
        let mut server = mockito::Server::new_async().await;
        let wire = proposal_mock(&mut server, proposed(name, arguments)).await;
        let f = setup(&server.url()).await;
        let done = f.propose().await;
        assert_eq!(done.record.task.error_code.as_deref(), Some(code));
        assert!(done.proposal.is_none());
        assert!(f
            .mcp
            .list_usage_logs("alice", None, None)
            .await
            .unwrap()
            .is_empty());
        wire.assert_async().await;
        f.stop().await;
    }
}

#[tokio::test]
async fn tools_selection_validation_precedes_task_audit() {
    let f = setup("http://unused.invalid").await;
    let mut input = f.tool_input();
    input.tools[0].schema_hash = "stale".into();
    assert!(f
        .workbench
        .start_tool_task(&f.skills, &f.providers, &f.mcp, input, "alice")
        .await
        .is_err());
    let mut input = f.tool_input();
    input.tools.push(input.tools[0].clone());
    assert!(f
        .workbench
        .start_tool_task(&f.skills, &f.providers, &f.mcp, input, "alice")
        .await
        .is_err());
    assert!(f.workbench.list("alice").await.unwrap().is_empty());
    f.stop().await;
}

#[tokio::test]
async fn duplicate_approval_is_rejected_and_inflight_cancel_stops_without_model_followup() {
    let mut server = mockito::Server::new_async().await;
    let first = proposal_mock(
        &mut server,
        proposed("dgos_tool_0", json!({"text":"private","delay":5000})),
    )
    .await;
    let f = setup(&server.url()).await;
    let update = f.propose().await;
    let id = update.record.task.id;
    let proposal_id = update.proposal.unwrap().id;
    let approval = f.workbench.advance_tool_task(
        &f.providers,
        &f.mcp,
        &id,
        "alice",
        ToolAction::Approve {
            proposal_id: proposal_id.clone(),
        },
    );
    let cancel = async {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if f.mcp
                    .list_usage_logs("alice", None, None)
                    .await
                    .unwrap()
                    .iter()
                    .any(|log| log.outcome == "pending")
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(f
            .workbench
            .advance_tool_task(
                &f.providers,
                &f.mcp,
                &id,
                "alice",
                ToolAction::Approve { proposal_id }
            )
            .await
            .is_err());
        f.workbench.cancel_tool_task(&id, "alice").await.unwrap()
    };
    let (done, cancelled) = tokio::join!(approval, cancel);
    assert_eq!(done.unwrap().record.task.status, "failed");
    assert_eq!(
        cancelled.record.task.error_code.as_deref(),
        Some("task_cancelled")
    );
    let logs = f.mcp.list_usage_logs("alice", None, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].outcome, "cancelled");
    first.assert_async().await;
    f.stop().await;
}

#[tokio::test]
async fn oversized_result_context_ends_task_without_resending_or_truncating() {
    let mut server = mockito::Server::new_async().await;
    let first = proposal_mock(
        &mut server,
        proposed("dgos_tool_0", json!({"text":"x".repeat(12000)})),
    )
    .await;
    let mut f = setup(&server.url()).await;
    f.input.prompt = "p".repeat(12000);
    let update = f.propose().await;
    let done = f
        .workbench
        .advance_tool_task(
            &f.providers,
            &f.mcp,
            &update.record.task.id,
            "alice",
            ToolAction::Approve {
                proposal_id: update.proposal.unwrap().id,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        done.record.task.error_code.as_deref(),
        Some("context_budget_exceeded")
    );
    let logs = f.mcp.list_usage_logs("alice", None, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].outcome, "success");
    assert!(done.content.is_none());
    first.assert_async().await;
    f.stop().await;
}

// ---------------------------------------------------------------------------------------------
// Provider protocol parity (AC22) and reasoning round trip (AC24)
// ---------------------------------------------------------------------------------------------

use dgos_db::sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::sync::Mutex;

const THOUGHT: &str = "PRIVATE-THOUGHT-7f3a";
const SIGNATURE: &str = "PRIVATE-SIGNATURE-7f3a";
const OPAQUE: &str = "PRIVATE-OPAQUE-7f3a";
const UPSTREAM_MARKER: &str = "UPSTREAM-MARKER-91c2";

/// A follow-up round ends with the tool result (OpenAI `tool` role, Anthropic `tool_result` block).
fn is_followup(body: &Value) -> bool {
    body["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .is_some_and(|last| {
            last["role"] == "tool"
                || last["content"]
                    .as_array()
                    .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "tool_result"))
        })
}

type Seen = Arc<Mutex<[Vec<Value>; 2]>>;

fn recorder(
    round: usize,
    seen: Seen,
) -> impl Fn(&mockito::Request) -> bool + Send + Sync + 'static {
    move |request| {
        let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
        let matches = is_followup(&body) == (round == 1);
        if matches {
            seen.lock().unwrap()[round].push(body);
        }
        matches
    }
}

/// Two sequential model rounds on one path. The request bodies the upstream received are kept.
/// `second: None` means the provider must not be called a second time.
struct Rounds {
    first: mockito::Mock,
    second: mockito::Mock,
    seen: Seen,
}

impl Rounds {
    fn body(&self, round: usize) -> Value {
        self.seen.lock().unwrap()[round]
            .last()
            .cloned()
            .unwrap_or_else(|| panic!("round {round} was never requested"))
    }
    async fn assert(&self) {
        self.first.assert_async().await;
        self.second.assert_async().await;
    }
}

async fn mock_rounds(
    server: &mut mockito::ServerGuard,
    path: &str,
    first: Value,
    second: Option<Value>,
) -> Rounds {
    let seen: Seen = Arc::new(Mutex::new([Vec::new(), Vec::new()]));
    let first = server
        .mock("POST", path)
        .match_request(recorder(0, seen.clone()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(first.to_string())
        .expect(1)
        .create_async()
        .await;
    let (status, body, hits) = match second {
        Some(body) => (200, body, 1),
        None => (500, json!({"error": "unexpected second round"}), 0),
    };
    let second = server
        .mock("POST", path)
        .match_request(recorder(1, seen.clone()))
        .with_status(status)
        .with_header("content-type", "application/json")
        .with_body(body.to_string())
        .expect(hits)
        .create_async()
        .await;
    Rounds {
        first,
        second,
        seen,
    }
}

fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({"type": "tool_use", "id": id, "name": name, "input": input})
}

fn anthropic_text(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn anthropic_message(content: Vec<Value>, stop_reason: &str, input: u32, output: u32) -> Value {
    json!({"id": "msg_1", "type": "message", "role": "assistant", "model": "chat",
        "content": content, "stop_reason": stop_reason,
        "usage": {"input_tokens": input, "output_tokens": output}})
}

fn openai_final(text: &str) -> Value {
    json!({"choices":[{"message":{"role":"assistant","content":text},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":8,"completion_tokens":3,"total_tokens":11}})
}

/// start -> model round -> (approve | deny when a proposal exists). Every update is kept.
async fn drive(f: &Fixture, approve: bool) -> Vec<ToolTaskUpdate> {
    let mut updates = vec![f.start().await];
    let id = updates[0].record.task.id.clone();
    let proposed = f
        .workbench
        .advance_tool_task(&f.providers, &f.mcp, &id, "alice", ToolAction::RunModel)
        .await
        .unwrap();
    let proposal = proposed.proposal.as_ref().map(|p| p.id.clone());
    updates.push(proposed);
    if let Some(proposal_id) = proposal {
        let action = if approve {
            ToolAction::Approve { proposal_id }
        } else {
            ToolAction::Deny { proposal_id }
        };
        updates.push(
            f.workbench
                .advance_tool_task(&f.providers, &f.mcp, &id, "alice", action)
                .await
                .unwrap(),
        );
    }
    updates
}

/// Every row of every table, as text.
async fn dump_database(f: &Fixture) -> String {
    let sql = |text: String| Statement::from_string(DbBackend::Sqlite, text);
    let tables =
        f.db.query_all(sql(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'".into(),
        ))
        .await
        .unwrap();
    let mut out = String::new();
    for table in tables {
        let name: String = table.try_get("", "name").unwrap();
        let columns =
            f.db.query_all(sql(format!("SELECT name FROM pragma_table_info('{name}')")))
                .await
                .unwrap();
        let columns: Vec<String> = columns
            .iter()
            .map(|column| {
                let column: String = column.try_get("", "name").unwrap();
                format!("COALESCE(CAST(\"{column}\" AS TEXT),'')")
            })
            .collect();
        if columns.is_empty() {
            continue;
        }
        let rows =
            f.db.query_all(sql(format!(
                "SELECT {} AS line FROM \"{name}\"",
                columns.join(" || '|' || ")
            )))
            .await
            .unwrap();
        for row in rows {
            out.push_str(&row.try_get::<String>("", "line").unwrap());
            out.push('\n');
        }
    }
    out
}

/// The markers must be nowhere: SQLite rows and files, provider and MCP usage logs, the task
/// audit list and every update returned along the way (proposals included).
async fn assert_private(f: &Fixture, updates: &[ToolTaskUpdate], markers: &[&str]) {
    let mut corpus = dump_database(f).await;
    assert!(
        corpus.contains("fixture-server"),
        "the database dump must see stored rows"
    );
    for entry in std::fs::read_dir(f._dir.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            corpus.push_str(&String::from_utf8_lossy(&std::fs::read(path).unwrap()));
        }
    }
    let provider_logs = f.providers.list_usage_logs("alice", None).await.unwrap();
    assert!(!provider_logs.is_empty());
    corpus.push_str(&serde_json::to_string(&provider_logs).unwrap());
    corpus.push_str(
        &serde_json::to_string(&f.mcp.list_usage_logs("alice", None, None).await.unwrap()).unwrap(),
    );
    corpus.push_str(&serde_json::to_string(&f.workbench.list("alice").await.unwrap()).unwrap());
    for update in updates {
        corpus.push_str(&serde_json::to_string(update).unwrap());
    }
    for marker in markers {
        assert!(!corpus.contains(marker), "{marker} leaked");
    }
}

/// Round two of an Anthropic tool loop: same request, plus the assistant turn kept whole and
/// the user turn answering it with a `tool_result`.
fn assert_anthropic_followup(
    first: &Value,
    second: &Value,
    assistant_content: &[Value],
    result_marker: &str,
) {
    for key in ["system", "tools"] {
        assert!(!first[key].is_null(), "{key} missing from round one");
    }
    for key in ["model", "max_tokens", "system", "tools", "tool_choice"] {
        assert_eq!(first[key], second[key], "{key} changed between rounds");
    }
    assert!(first.get("thinking").is_none() && second.get("thinking").is_none());
    let a = first["messages"].as_array().unwrap();
    let b = second["messages"].as_array().unwrap();
    assert_eq!(a.last().unwrap()["role"], "user");
    assert_eq!(b.len(), a.len() + 2);
    assert_eq!(&b[..a.len()], &a[..]);
    let assistant = &b[a.len()];
    assert_eq!(assistant["role"], "assistant");
    assert_eq!(assistant["content"], json!(assistant_content));
    let tool_use = assistant_content.last().unwrap();
    assert_eq!(tool_use["type"], "tool_use");
    let result = &b[a.len() + 1];
    assert_eq!(result["role"], "user");
    let block = &result["content"][0];
    assert_eq!(block["type"], "tool_result");
    assert_eq!(block["tool_use_id"], tool_use["id"]);
    assert!(block["content"].as_str().unwrap().contains(result_marker));
}

#[tokio::test]
async fn anthropic_approved_tool_round_keeps_tool_use_then_tool_result_and_pins_system_and_tools() {
    let mut server = mockito::Server::new_async().await;
    let content = vec![
        anthropic_text("Calling the tool"),
        tool_use(
            "toolu_01A",
            "dgos_tool_0",
            json!({"text": "synthetic-anthropic-flow"}),
        ),
    ];
    let rounds = mock_rounds(
        &mut server,
        "/v1/messages",
        anthropic_message(content.clone(), "tool_use", 5, 2),
        Some(anthropic_message(
            vec![anthropic_text("Anthropic final")],
            "end_turn",
            8,
            3,
        )),
    )
    .await;
    let f = setup_provider(&server.url(), "normal", "anthropic").await;
    let updates = drive(&f, true).await;
    assert_eq!(updates.len(), 3);
    assert_eq!(updates[1].phase, "awaiting_confirmation");
    let proposal = updates[1].proposal.as_ref().unwrap();
    assert_eq!(
        proposal.arguments,
        json!({"text": "synthetic-anthropic-flow"})
    );
    assert_eq!(proposal.tool_name, f.tool.tool_name);
    let done = &updates[2];
    assert_eq!(done.record.task.status, "succeeded");
    assert_eq!(done.content.as_deref(), Some("Anthropic final"));
    assert_eq!(done.record.task.total_tokens, Some(18));
    let logs = f.mcp.list_usage_logs("alice", None, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].caller.as_deref(), Some("workbench"));
    let (first, second) = (rounds.body(0), rounds.body(1));
    assert_eq!(first["tools"][0]["name"], "dgos_tool_0");
    assert_eq!(
        first["tools"][0]["input_schema"]["required"],
        json!(["text"])
    );
    assert_anthropic_followup(&first, &second, &content, "synthetic-anthropic-flow");
    rounds.assert().await;
    f.stop().await;
}

#[tokio::test]
async fn anthropic_denied_proposal_fails_with_tool_denied_and_never_calls_the_provider_again() {
    let mut server = mockito::Server::new_async().await;
    let rounds = mock_rounds(
        &mut server,
        "/v1/messages",
        anthropic_message(
            vec![tool_use(
                "toolu_01A",
                "dgos_tool_0",
                json!({"text": "private"}),
            )],
            "tool_use",
            5,
            2,
        ),
        None,
    )
    .await;
    let f = setup_provider(&server.url(), "normal", "anthropic").await;
    let updates = drive(&f, false).await;
    assert_eq!(updates.len(), 3);
    assert!(updates[1].proposal.is_some());
    let done = &updates[2];
    assert_eq!(done.record.task.status, "failed");
    assert_eq!(done.record.task.error_code.as_deref(), Some("tool_denied"));
    assert!(done.content.is_none());
    assert!(f
        .mcp
        .list_usage_logs("alice", None, None)
        .await
        .unwrap()
        .is_empty());
    rounds.assert().await;
    f.stop().await;
}

#[tokio::test]
async fn anthropic_invalid_tool_responses_stop_before_confirmation_with_the_same_codes_as_openai() {
    let text = || anthropic_text(UPSTREAM_MARKER);
    let cases: Vec<(&str, Vec<Value>, &str, &str)> = vec![
        (
            "unselected tool",
            vec![
                text(),
                tool_use("toolu_1", "unselected", json!({"text": "x"})),
            ],
            "tool_use",
            "unselected_tool",
        ),
        (
            "wrong argument type",
            vec![
                text(),
                tool_use("toolu_1", "dgos_tool_0", json!({"text": 42})),
            ],
            "tool_use",
            "invalid_tool_arguments",
        ),
        (
            "non-object input",
            vec![
                text(),
                tool_use("toolu_1", "dgos_tool_0", json!("not-an-object")),
            ],
            "tool_use",
            "invalid_tool_arguments",
        ),
        (
            "two tool_use blocks",
            vec![
                text(),
                tool_use("toolu_1", "dgos_tool_0", json!({"text": "a"})),
                tool_use("toolu_2", "dgos_tool_0", json!({"text": "b"})),
            ],
            "tool_use",
            "tool_call_limit",
        ),
        (
            "tool_use block with end_turn",
            vec![
                text(),
                tool_use("toolu_1", "dgos_tool_0", json!({"text": "a"})),
            ],
            "end_turn",
            "provider_",
        ),
        (
            "tool_use block with max_tokens",
            vec![
                text(),
                tool_use("toolu_1", "dgos_tool_0", json!({"text": "a"})),
            ],
            "max_tokens",
            "provider_",
        ),
        (
            "tool_use stop_reason without a block",
            vec![text()],
            "tool_use",
            "provider_",
        ),
    ];
    for (label, content, stop_reason, code) in cases {
        let mut server = mockito::Server::new_async().await;
        let rounds = mock_rounds(
            &mut server,
            "/v1/messages",
            anthropic_message(content, stop_reason, 5, 2),
            None,
        )
        .await;
        let f = setup_provider(&server.url(), "normal", "anthropic").await;
        let updates = drive(&f, true).await;
        assert_eq!(updates.len(), 2, "{label}");
        let done = &updates[1];
        assert!(done.proposal.is_none(), "{label}");
        assert_eq!(done.record.task.status, "failed", "{label}");
        let error = done.record.task.error_code.as_deref().unwrap();
        if code == "provider_" {
            assert!(error.starts_with(code), "{label}: {error}");
        } else {
            assert_eq!(error, code, "{label}");
        }
        assert!(f
            .mcp
            .list_usage_logs("alice", None, None)
            .await
            .unwrap()
            .is_empty());
        assert_private(&f, &updates, &[UPSTREAM_MARKER, "synthetic-secret"]).await;
        rounds.assert().await;
        f.stop().await;
    }
}

#[tokio::test]
async fn reasoning_round_trip_anthropic_resends_thinking_blocks_first_and_leaves_no_trace() {
    for with_text in [true, false] {
        let mut server = mockito::Server::new_async().await;
        let thinking = json!({"type": "thinking", "thinking": format!("{THOUGHT} first plan"),
            "signature": SIGNATURE});
        let redacted = json!({"type": "redacted_thinking", "data": OPAQUE});
        let mut content = vec![thinking.clone(), redacted.clone()];
        if with_text {
            content.push(anthropic_text("Let me call the tool"));
        }
        content.push(tool_use(
            "toolu_01A",
            "dgos_tool_0",
            json!({"text": "synthetic-thinking-flow"}),
        ));
        let rounds = mock_rounds(
            &mut server,
            "/v1/messages",
            anthropic_message(content.clone(), "tool_use", 5, 2),
            Some(anthropic_message(
                vec![anthropic_text("Thinking final")],
                "end_turn",
                8,
                3,
            )),
        )
        .await;
        let f = setup_provider(&server.url(), "normal", "anthropic").await;
        let updates = drive(&f, true).await;
        let done = updates.last().unwrap();
        assert_eq!(
            done.record.task.status, "succeeded",
            "with_text={with_text}"
        );
        assert_eq!(done.content.as_deref(), Some("Thinking final"));
        let (first, second) = (rounds.body(0), rounds.body(1));
        // The caller never asked for reasoning, yet the blocks the model sent come back.
        assert!(first.get("thinking").is_none());
        assert!(!first.to_string().contains(THOUGHT));
        let messages = second["messages"].as_array().unwrap();
        let assistant = &messages[messages.len() - 2];
        assert_eq!(assistant["content"][0], thinking);
        assert_eq!(assistant["content"][1], redacted);
        assert!(second.to_string().contains(SIGNATURE));
        assert_anthropic_followup(&first, &second, &content, "synthetic-thinking-flow");
        assert_private(&f, &updates, &[THOUGHT, SIGNATURE, OPAQUE]).await;
        rounds.assert().await;
        f.stop().await;
    }
}

fn openai_proposal(content: Option<&str>, extra: Option<(&str, Value)>) -> Value {
    let mut message = json!({"role": "assistant", "content": content, "tool_calls": [{
        "id": "call_1", "type": "function",
        "function": {"name": "dgos_tool_0",
            "arguments": json!({"text": "synthetic-openai-reasoning"}).to_string()}}]});
    if let Some((name, value)) = extra {
        message[name] = value;
    }
    json!({"choices": [{"message": message, "finish_reason": "tool_calls"}],
        "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}})
}

async fn openai_reasoning_case(content: Option<&str>, extra: Option<(&'static str, Value)>) {
    let mut server = mockito::Server::new_async().await;
    let rounds = mock_rounds(
        &mut server,
        "/chat/completions",
        openai_proposal(content, extra.clone()),
        Some(openai_final("Completed")),
    )
    .await;
    let f = setup(&server.url()).await;
    let updates = drive(&f, true).await;
    let done = updates.last().unwrap();
    assert_eq!(done.record.task.status, "succeeded");
    assert_eq!(done.content.as_deref(), Some("Completed"));
    let (first, second) = (rounds.body(0), rounds.body(1));
    let a = first["messages"].as_array().unwrap();
    let b = second["messages"].as_array().unwrap();
    assert_eq!(b.len(), a.len() + 2);
    assert_eq!(&b[..a.len()], &a[..]);
    assert_eq!(first["tools"], second["tools"]);
    assert_eq!(first["parallel_tool_calls"], second["parallel_tool_calls"]);
    let assistant = &b[a.len()];
    assert_eq!(assistant["role"], "assistant");
    assert_eq!(
        assistant.get("content").cloned().unwrap_or(Value::Null),
        json!(content)
    );
    assert_eq!(assistant["tool_calls"][0]["id"], "call_1");
    assert_eq!(
        serde_json::from_str::<Value>(
            assistant["tool_calls"][0]["function"]["arguments"]
                .as_str()
                .unwrap()
        )
        .unwrap(),
        json!({"text": "synthetic-openai-reasoning"})
    );
    for name in ["reasoning_content", "reasoning", "reasoning_details"] {
        match &extra {
            Some((kept, value)) if *kept == name => {
                assert_eq!(&assistant[name], value, "{name} must come back unchanged")
            }
            _ => assert!(assistant.get(name).is_none(), "{name} must not be added"),
        }
    }
    assert_eq!(b[a.len() + 1]["role"], "tool");
    assert_eq!(b[a.len() + 1]["tool_call_id"], "call_1");
    assert_private(&f, &updates, &[THOUGHT, OPAQUE, SIGNATURE]).await;
    rounds.assert().await;
    f.stop().await;
}

#[tokio::test]
async fn reasoning_round_trip_openai_compatible_reasoning_content_comes_back_under_the_same_name() {
    openai_reasoning_case(
        Some("Let me call the tool"),
        Some(("reasoning_content", json!(format!("{THOUGHT} plan")))),
    )
    .await;
}

#[tokio::test]
async fn reasoning_round_trip_openai_compatible_reasoning_comes_back_under_the_same_name() {
    openai_reasoning_case(None, Some(("reasoning", json!(format!("{THOUGHT} plan"))))).await;
}

#[tokio::test]
async fn reasoning_round_trip_openai_compatible_reasoning_details_come_back_under_the_same_name() {
    openai_reasoning_case(
        Some("Let me call the tool"),
        Some((
            "reasoning_details",
            json!([{"type": "reasoning.text", "text": format!("{THOUGHT} detail")},
                {"type": "reasoning.encrypted", "data": OPAQUE}]),
        )),
    )
    .await;
}

#[tokio::test]
async fn reasoning_round_trip_openai_compatible_adds_no_reasoning_field_when_upstream_sent_none() {
    openai_reasoning_case(Some("Let me call the tool"), None).await;
}

#[tokio::test]
async fn reasoning_round_trip_counts_toward_the_context_budget() {
    for provider in ["anthropic", "openai_compatible"] {
        for large in [true, false] {
            let filler = "x".repeat(if large { 30 * 1024 } else { 16 });
            let mut server = mockito::Server::new_async().await;
            let (path, first, second) = if provider == "anthropic" {
                (
                    "/v1/messages",
                    anthropic_message(
                        vec![
                            json!({"type": "thinking", "thinking": THOUGHT,
                                "signature": format!("{SIGNATURE}{filler}")}),
                            tool_use(
                                "toolu_01A",
                                "dgos_tool_0",
                                json!({"text": "synthetic-budget"}),
                            ),
                        ],
                        "tool_use",
                        5,
                        2,
                    ),
                    anthropic_message(vec![anthropic_text("Budget final")], "end_turn", 8, 3),
                )
            } else {
                (
                    "/chat/completions",
                    openai_proposal(
                        None,
                        Some(("reasoning_content", json!(format!("{THOUGHT}{filler}")))),
                    ),
                    openai_final("Budget final"),
                )
            };
            let rounds = mock_rounds(&mut server, path, first, (!large).then_some(second)).await;
            let f = setup_provider(&server.url(), "normal", provider).await;
            let updates = drive(&f, true).await;
            let done = updates.last().unwrap();
            let label = format!("{provider} large={large}");
            let logs = f.mcp.list_usage_logs("alice", None, None).await.unwrap();
            assert_eq!(logs.len(), 1, "{label}");
            assert_eq!(logs[0].outcome, "success", "{label}");
            if large {
                assert_eq!(done.record.task.status, "failed", "{label}");
                assert_eq!(
                    done.record.task.error_code.as_deref(),
                    Some("context_budget_exceeded"),
                    "{label}"
                );
                assert!(done.content.is_none(), "{label}");
            } else {
                assert_eq!(done.record.task.status, "succeeded", "{label}");
                assert_eq!(done.content.as_deref(), Some("Budget final"), "{label}");
            }
            assert_private(&f, &updates, &[THOUGHT, SIGNATURE]).await;
            rounds.assert().await;
            f.stop().await;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// AC24: reasoning text never reaches the log.
//
// One process-wide subscriber at TRACE writes into a shared buffer. A global default is used on
// purpose: `set_default` is per thread, and the flow also logs from sqlx worker threads, the
// mockito server thread and spawned tasks, which a thread-local subscriber would miss. The
// subscriber is installed once (`OnceLock`) and never removed; each test uses markers that no
// other test logs, so parallel tests sharing the buffer cannot disturb each other's assertions.
// ---------------------------------------------------------------------------------------------

use std::sync::OnceLock;
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

impl LogBuffer {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

fn captured_log() -> LogBuffer {
    static LOG: OnceLock<LogBuffer> = OnceLock::new();
    LOG.get_or_init(|| {
        let buffer = LogBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(buffer.clone())
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("no other subscriber may be installed in this test binary");
        buffer
    })
    .clone()
}

#[tokio::test]
async fn reasoning_round_trip_log_capture_sees_events_from_this_and_other_threads() {
    // Positive control for the two tests below: the same subscriber must see a marker logged
    // from the test task, from a spawned task and from a plain OS thread.
    let log = captured_log();
    tracing::info!(marker = "LOG-CONTROL-task-5d1e", "control event");
    tokio::spawn(async { tracing::debug!(marker = "LOG-CONTROL-spawned-5d1e", "control event") })
        .await
        .unwrap();
    std::thread::spawn(|| tracing::trace!(marker = "LOG-CONTROL-thread-5d1e", "control event"))
        .join()
        .unwrap();
    let text = log.text();
    for marker in [
        "LOG-CONTROL-task-5d1e",
        "LOG-CONTROL-spawned-5d1e",
        "LOG-CONTROL-thread-5d1e",
    ] {
        assert!(text.contains(marker), "{marker} missing from the capture");
    }
}

#[tokio::test]
async fn reasoning_round_trip_anthropic_thinking_never_reaches_the_log() {
    const LOG_THOUGHT: &str = "LOG-THOUGHT-anthropic-2b9c";
    const LOG_SIGNATURE: &str = "LOG-SIGNATURE-anthropic-2b9c";
    const LOG_OPAQUE: &str = "LOG-OPAQUE-anthropic-2b9c";
    let log = captured_log();
    let mut server = mockito::Server::new_async().await;
    let content = vec![
        json!({"type": "thinking", "thinking": format!("{LOG_THOUGHT} plan"),
            "signature": LOG_SIGNATURE}),
        json!({"type": "redacted_thinking", "data": LOG_OPAQUE}),
        anthropic_text("Let me call the tool"),
        tool_use(
            "toolu_01A",
            "dgos_tool_0",
            json!({"text": "synthetic-log-flow"}),
        ),
    ];
    let rounds = mock_rounds(
        &mut server,
        "/v1/messages",
        anthropic_message(content, "tool_use", 5, 2),
        Some(anthropic_message(
            vec![anthropic_text("Log final")],
            "end_turn",
            8,
            3,
        )),
    )
    .await;
    let f = setup_provider(&server.url(), "normal", "anthropic").await;
    let updates = drive(&f, true).await;
    assert_eq!(updates.last().unwrap().record.task.status, "succeeded");
    // The reasoning really travelled through the code under test: it went back upstream.
    let second = rounds.body(1).to_string();
    assert!(second.contains(LOG_THOUGHT) && second.contains(LOG_SIGNATURE));
    assert!(second.contains(LOG_OPAQUE));
    rounds.assert().await;
    f.stop().await;
    let text = log.text();
    for marker in [LOG_THOUGHT, LOG_SIGNATURE, LOG_OPAQUE] {
        assert!(!text.contains(marker), "{marker} reached the log");
    }
}

#[tokio::test]
async fn reasoning_round_trip_openai_compatible_reasoning_content_never_reaches_the_log() {
    const LOG_THOUGHT: &str = "LOG-THOUGHT-openai-8e44";
    let log = captured_log();
    let mut server = mockito::Server::new_async().await;
    let rounds = mock_rounds(
        &mut server,
        "/chat/completions",
        openai_proposal(
            Some("Let me call the tool"),
            Some(("reasoning_content", json!(format!("{LOG_THOUGHT} plan")))),
        ),
        Some(openai_final("Completed")),
    )
    .await;
    let f = setup(&server.url()).await;
    let updates = drive(&f, true).await;
    assert_eq!(updates.last().unwrap().record.task.status, "succeeded");
    let second = rounds.body(1);
    let messages = second["messages"].as_array().unwrap();
    assert!(
        messages
            .iter()
            .any(|message| message["reasoning_content"] == json!(format!("{LOG_THOUGHT} plan"))),
        "the reasoning must have come back upstream"
    );
    rounds.assert().await;
    f.stop().await;
    let text = log.text();
    assert!(!text.contains(LOG_THOUGHT), "{LOG_THOUGHT} reached the log");
}
