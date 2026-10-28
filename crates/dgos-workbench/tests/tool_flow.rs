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
            provider_type: "openai_compatible".into(),
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
