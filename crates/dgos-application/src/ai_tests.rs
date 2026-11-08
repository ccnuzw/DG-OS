//! Local protocol fixtures only. These tests do not call a real supplier.
use crate::ai::*;
use crate::{HostCapabilityService, McpConfirmationDeliveryError, McpConfirmationNotifier, McpConfirmationPrompt, ModuleMcpConfirmations, ModuleReleaseService};
use dgos_mcp::{CallMcpToolRequest, CreateMcpServerRequest, McpCallPolicy, McpTool, StdioConfig, TransportKind};
use dgos_db::{migrations::Migrator, sea_orm_migration::MigratorTrait, AiRunRepository, MaintenanceGate, ModulePermission, ModulePermissionRepository, NewUser, UserRepository};
use dgos_modules::{ModuleCallRouter, ModulePageTransport, ModulePublisher, ModuleRuntimeSupervisor, PageInstruction, PageLoaderConfig, PageModuleLoader, PageReceipt, PageSendError, PinnedModuleAdmission};
use dgos_providers::test_support::{seed_provider, MemoryCredentialVault};
use dgos_providers::ProviderManager;
use dgos_skills::SkillsManager;
use dgos_types::*;
use std::{sync::{Arc, Mutex}, time::Duration};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use serde_json::json;
use tempfile::{tempdir, TempDir};
use tokio::sync::mpsc;
#[test]
fn cancellation_and_terminal_seal_have_one_registry_winner() {
    fn registry(token: CancellationToken) -> std::collections::HashMap<String, ActiveRun> {
        [ ("run".to_owned(), ActiveRun { owner: "alice".into(), module: "module".into(), runtime: "runtime".into(), cancel: token, settling: false }) ].into_iter().collect()
    }
    let token = CancellationToken::new();
    let mut cancel_first = registry(token.clone());
    assert!(cancel_active_run(&cancel_first, "run", "alice", "module", "runtime"));
    assert!(seal_active_run(&mut cancel_first, "run"));
    assert!(!cancel_active_run(&cancel_first, "run", "alice", "module", "runtime"));

    let mut seal_first = registry(CancellationToken::new());
    assert!(!seal_active_run(&mut seal_first, "run"));
    assert!(!cancel_active_run(&seal_first, "run", "alice", "module", "runtime"));
}
#[test]
fn ai_failure_categories_keep_domain_and_registered_code() {
    let provider = failure("provider_1001012", false);
    assert_eq!((provider.domain.as_str(), provider.code), ("provider", 1001012));
    let skill = failure("skill_1002004", false);
    assert_eq!((skill.domain.as_str(), skill.code), ("skill", 1002004));
    let uncertain_mcp = failure("mcp_1003006", true);
    assert_eq!((uncertain_mcp.domain.as_str(), uncertain_mcp.code, uncertain_mcp.result_unknown), ("mcp", 1003006, true));
    let context = failure("context_budget_exceeded", false);
    assert_eq!((context.domain.as_str(), context.code, context.reason.as_str()), ("execution", AiExecutionFailure::ContextTooLong.code(), "context_too_long"));
}


struct Allow;
#[async_trait::async_trait]
impl McpCallPolicy for Allow {
    async fn allows_call(&self, user: &str, _: &str, _: &str) -> bool { user == "alice" }
}

struct Confirm(mpsc::UnboundedSender<String>);
impl McpConfirmationNotifier for Confirm {
    fn open(&self, _: &str, prompt: McpConfirmationPrompt<'_>) -> Result<(), McpConfirmationDeliveryError> {
        self.0.send(prompt.confirmation_id.to_owned()).map_err(|_| McpConfirmationDeliveryError)
    }
    fn close(&self, _: &str, _: &str) {}
}

struct Page(mpsc::UnboundedSender<(String, PageInstruction)>);
impl ModulePageTransport for Page {
    fn send(&self, attachment: &str, instruction: PageInstruction) -> Result<(), PageSendError> {
        self.0.send((attachment.into(), instruction)).map_err(|_| PageSendError)
    }
}
struct Fixture {
    _dir: TempDir,
    db: dgos_db::sea_orm::DatabaseConnection,
    service: AiService,
    mcp: Arc<dgos_mcp::McpManager>,
    confirmations: Arc<ModuleMcpConfirmations>,
    prompts: Option<mpsc::UnboundedReceiver<String>>,
    runtime: dgos_modules::ModuleRuntimeHandle,
    permissions: Arc<ModulePermissionRepository>,
    repository: AiRunRepository,
    router: ModuleCallRouter,
    route: String,
    attachment: String,
    account: CancellationToken,
    page: CancellationToken,
    input: AiRunInput,
    _supervisor: ModuleRuntimeSupervisor,
    driver: tokio::task::JoinHandle<()>,
    _lost: mpsc::UnboundedReceiver<dgos_modules::RuntimeLost>,
}
impl Drop for Fixture { fn drop(&mut self) { self.driver.abort(); } }
impl Fixture {
    async fn new(base_url: &str) -> Self {
        let dir = tempdir().unwrap();
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("db.sqlite").display());
        let db = dgos_db::establish_connection(&url).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        UserRepository::new(db.clone()).insert(NewUser { id: "alice", username: "Alice", username_key: "alice", password_hash: "fixture", now: 1 }).await.unwrap();
        let package = dir.path().join("package");
        std::fs::create_dir(&package).unwrap();
        std::fs::write(package.join("entry.js"), "export default () => ({Component: () => null});").unwrap();
        let manifest = |hash: &str| json!({"id":"ai-consumer","name":"AI consumer","version":"1.0.0","entry":"entry.js","artifact":{"version":"1.0.0","sha256":hash},"permissions":["providers.call","skills.execute","mcp.call"]}).to_string();
        std::fs::write(package.join("manifest.json"), manifest(&"0".repeat(64))).unwrap();
        let digest = dgos_modules::package_integrity(&package).unwrap();
        std::fs::write(package.join("manifest.json"), manifest(digest.strip_prefix("v2:").unwrap())).unwrap();
        let publisher = Arc::new(ModulePublisher::new(dir.path().join("modules"), db.clone()));
        let release = ModuleReleaseService::new(publisher.clone()).publish(&package).await.unwrap();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let (loader, lost) = PageModuleLoader::new(Arc::new(Page(sender)), PageLoaderConfig::default());
        let attachment = loader.attach("main").attachment_id;
        let router = ModuleCallRouter::new(loader.clone());
        let responder = loader.clone();
        let driver = tokio::spawn(async move {
            while let Some((attachment, instruction)) = receiver.recv().await {
                let (ticket, receipt) = match instruction {
                    PageInstruction::Load { ticket, .. } => (ticket, PageReceipt::Loaded),
                    PageInstruction::Unload { ticket, .. } => (ticket, PageReceipt::Unloaded { hook_failed: false }),
                };
                responder.receive(&attachment, &ticket, receipt);
            }
        });
        let supervisor = ModuleRuntimeSupervisor::new(publisher.clone(), Arc::new(loader))
            .with_admission(Arc::new(PinnedModuleAdmission::new([("ai-consumer", "1.0.0", release.integrity.as_str())])));
        let handle = supervisor.start(&HostSession::for_user("alice"), "ai-consumer").await.unwrap();
        let route = router.issue(&handle).unwrap().as_str().to_owned();
        let permissions = Arc::new(ModulePermissionRepository::new(db.clone()));
        permissions.grant("alice", "ai-consumer", ModulePermission::ProvidersCall).await.unwrap();
        let vault = Arc::new(MemoryCredentialVault::default());
        let provider = seed_provider(db.clone(), &vault, ProviderConfig { name: "Fixture".into(), provider_type: "openai_compatible".into(), base_url: base_url.into(), api_key: "test-only-key".into(), enabled: true, config: None }, "alice").await.unwrap();
        let providers = Arc::new(ProviderManager::with_vault(&url, vault).await.unwrap());
        providers.add_manual_model(&provider.id, "alice", "fixture-model", None).await.unwrap();
        let revision = providers.get_provider(&provider.id, "alice").await.unwrap().unwrap().config_revision;
        let skills = Arc::new(RwLock::new(SkillsManager::new(db.clone(), dir.path().join("skills"))));
        let mcp = Arc::new(dgos_mcp::McpManager::with_policy(db.clone(), Arc::new(Allow)));
        let repository = AiRunRepository::new(db.clone());
        let (sender, receiver) = mpsc::unbounded_channel();
        let confirmations = Arc::new(ModuleMcpConfirmations::new(Arc::new(Confirm(sender))));
        let service = AiService::new(HostCapabilityService::with_publisher(permissions.clone(), publisher), repository.clone(), providers, skills, mcp.clone(), confirmations.clone(), MaintenanceGate::new());
        Self { _dir: dir, db, service, mcp, confirmations, prompts: Some(receiver), runtime: handle, permissions, repository, router, route, attachment, account: CancellationToken::new(), page: CancellationToken::new(),
            input: AiRunInput { provider_id: provider.id, model_id: "fixture-model".into(), provider_revision: revision, prompt: "private source".into(), skill_refs: vec![], tools: vec![], generation: GenerationParameters::default() },
            _supervisor: supervisor, driver, _lost: lost }
    }
    fn invoke(&self) -> AiInvocation {
        let call = self.router.admit_once(&self.route, &self.attachment, &uuid::Uuid::new_v4().to_string()).unwrap();
        AiInvocation::bind(call, &HostSession::for_user("alice"), self.account.clone(), self.page.clone()).unwrap()
    }
    async fn tool(&mut self) -> (McpTool, mpsc::UnboundedReceiver<String>) {
        self.permissions.grant("alice", "ai-consumer", ModulePermission::McpCall).await.unwrap();
        let server = self.mcp.create_server("alice", CreateMcpServerRequest {
            name: "AI local fixture".into(), transport: TransportKind::Stdio,
            stdio: Some(StdioConfig { executable: "node".into(), args: vec![format!("{}/../dgos-mcp/tests/fixtures/mcp-server.mjs", env!("CARGO_MANIFEST_DIR")), "cancellation-observable".into()], cwd: None }),
            http: None, enabled: Some(true), auto_connect: None,
        }).await.unwrap();
        self.mcp.connect("alice", &server.id).await.unwrap();
        let tool = self.mcp.list_tools("alice", &server.id).await.unwrap().into_iter().find(|t| t.tool_name == "echo").unwrap();
        self.input.tools = vec![AiToolSelection { server_id: tool.server_id.clone(), tool_name: tool.tool_name.clone(), schema_hash: tool.schema_hash.clone() }];
        (tool, self.prompts.take().unwrap())
    }

    async fn wait_tool_started(&self, tool: &McpTool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let mut request = CallMcpToolRequest { server_id: tool.server_id.clone(), tool_name: tool.tool_name.clone(), arguments: json!({"text":"fixture-counts"}),
                    call_id: None, task_ref: None, timeout_ms: Some(1000), confirmation_token: None };
                request.confirmation_token = Some(self.mcp.confirm_call("alice", &request).await.unwrap());
                let result = self.mcp.call_tool("alice", request).await.unwrap();
                let counts: serde_json::Value = serde_json::from_str(result.content[0]["text"].as_str().unwrap()).unwrap();
                if counts["started"].as_u64().unwrap() > 0 { break; }
                tokio::task::yield_now().await;
            }
        }).await.expect("fixture tool actually started");
    }
}

fn sse() -> String {
    format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"delta":{"content":"actual answer"},"finish_reason":null}]}),
        json!({"choices":[{"delta":{},"finish_reason":"stop"}]}))
}

#[tokio::test]
async fn module_stream_finishes_once_and_is_found_without_started() {
    let mut server = mockito::Server::new_async().await;
    let upstream = server.mock("POST", "/chat/completions").with_status(200).with_header("content-type", "text/event-stream").with_body(sse()).expect(1).create_async().await;
    let f = Fixture::new(&server.url()).await;
    let received = Arc::new(Mutex::new(Vec::new()));
    let output = received.clone();
    let sink: Arc<dyn AiEventSink> = Arc::new(move |event: AiEvent| { output.lock().unwrap().push(event); true });
    let terminal = f.service.run(f.invoke(), f.input.clone(), sink).await.unwrap();
    assert_eq!(terminal.status, AiRunStatus::Succeeded);
    assert_eq!(terminal.content.as_deref(), Some("actual answer"));
    let events = received.lock().unwrap();
    assert_eq!(events.iter().filter(|e| matches!(&e.payload, AiEventPayload::Terminal { .. })).count(), 1);
    assert!(events.iter().any(|e| matches!(&e.payload, AiEventPayload::Delta { text } if text == "actual answer")));
    drop(events);
    // The caller deliberately ignores Started and discovers the record by a fresh bound query.
    let page = f.service.list_runs(f.invoke(), AiRunListInput::default()).await.unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].status, AiRunStatus::Succeeded);
    assert_eq!(f.service.get_run(f.invoke(), &page.items[0].run_id).await.unwrap().unwrap().status, AiRunStatus::Succeeded);
    assert!(f.repository.list("bob", "ai-consumer", AiRunListInput::default()).await.unwrap().items.is_empty());
    assert!(f.repository.list("alice", "different-module", AiRunListInput::default()).await.unwrap().items.is_empty());
    upstream.assert_async().await;
}
#[tokio::test]
async fn provider_stream_audit_failure_reaches_ai_promise_and_terminal_event() {
    use dgos_db::sea_orm::{ConnectionTrait, DbBackend, Statement};
    let mut server = mockito::Server::new_async().await;
    let upstream = server.mock("POST", "/chat/completions").with_status(200).with_header("content-type", "text/event-stream").with_body(sse()).expect(1).create_async().await;
    let f = Fixture::new(&server.url()).await;
    f.db.execute(Statement::from_string(DbBackend::Sqlite, "DROP TABLE provider_usage_logs".to_owned())).await.unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let output = received.clone();
    let sink: Arc<dyn AiEventSink> = Arc::new(move |event: AiEvent| { output.lock().unwrap().push(event); true });
    let terminal = f.service.run(f.invoke(), f.input.clone(), sink).await.unwrap();
    assert_eq!(terminal.status, AiRunStatus::Succeeded);
    assert_eq!(terminal.content.as_deref(), Some("actual answer"));
    assert!(terminal.audit_warning);
    let events = received.lock().unwrap();
    let terminals = events.iter().filter_map(|event| match &event.payload { AiEventPayload::Terminal { terminal } => Some(terminal), _ => None }).collect::<Vec<_>>();
    assert_eq!(terminals.len(), 1);
    assert!(terminals[0].audit_warning);
    upstream.assert_async().await;
}

#[tokio::test]
async fn lost_started_cancels_without_dispatch_and_fresh_query_can_locate_the_run() {
    let mut server = mockito::Server::new_async().await;
    let upstream = server.mock("POST", "/chat/completions").expect(0).create_async().await;
    let f = Fixture::new(&server.url()).await;
    let terminal = f.service.run(f.invoke(), f.input.clone(), Arc::new(|_: AiEvent| false)).await.unwrap();
    assert_eq!(terminal.status, AiRunStatus::Cancelled);
    assert!(!terminal.result_unknown);
    let page = f.service.list_runs(f.invoke(), AiRunListInput::default()).await.unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].status, AiRunStatus::Cancelled);
    upstream.assert_async().await;
}


#[tokio::test]
async fn revoked_provider_grant_is_not_borrowed_from_a_builtin_caller() {
    let mut server = mockito::Server::new_async().await;
    let upstream = server.mock("POST", "/chat/completions").expect(0).create_async().await;
    let f = Fixture::new(&server.url()).await;
    f.permissions.revoke("alice", "ai-consumer", ModulePermission::ProvidersCall).await.unwrap();
    let result = f.service.run(f.invoke(), f.input.clone(), Arc::new(|_: AiEvent| true)).await;
    assert!(matches!(result, Err(DgosError::PermissionDenied)));
    assert!(f.service.list_runs(f.invoke(), AiRunListInput::default()).await.unwrap().items.is_empty());
    upstream.assert_async().await;
}

#[tokio::test]
async fn invocation_cannot_bind_a_foreign_account_or_revoked_lease() {
    let f = Fixture::new("http://127.0.0.1:1").await;
    let call = f.router.admit_once(&f.route, &f.attachment, "foreign").unwrap();
    assert!(matches!(AiInvocation::bind(call, &HostSession::for_user("bob"), CancellationToken::new(), CancellationToken::new()), Err(DgosError::SessionInvalid)));
    let lease = CancellationToken::new(); lease.cancel();
    let call = f.router.admit_once(&f.route, &f.attachment, "revoked").unwrap();
    assert!(matches!(AiInvocation::bind(call, &HostSession::for_user("alice"), lease, CancellationToken::new()), Err(DgosError::SessionInvalid)));
}

fn tool_proposal(delay: u64) -> String {
    json!({"choices":[{"message":{"role":"assistant","content":null,"reasoning_content":"private reasoning","tool_calls":[{
        "id":"ai-tool-1","type":"function","function":{"name":"dgos_tool_0","arguments":json!({"text":"AI fixture","delay":delay}).to_string()}
    }]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}).to_string()
}

#[tokio::test]
async fn approved_module_tool_round_preserves_reasoning_usage_and_module_provenance() {
    let mut server = mockito::Server::new_async().await;
    let first = server.mock("POST", "/chat/completions")
        .match_request(|r| !r.body().unwrap().windows(b"tool_call_id".len()).any(|w| w == b"tool_call_id"))
        .with_header("content-type", "application/json").with_body(tool_proposal(0)).expect(1).create_async().await;
    let second = server.mock("POST", "/chat/completions").match_request(|r| {
        let body: serde_json::Value = serde_json::from_slice(r.body().unwrap()).unwrap();
        let messages = body["messages"].as_array().unwrap();
        messages.last().is_some_and(|m| m["role"] == "tool" && m["tool_call_id"] == "ai-tool-1")
            && messages.iter().any(|m| m["reasoning_content"] == "private reasoning")
    }).with_header("content-type", "application/json").with_body(json!({"choices":[{"message":{"role":"assistant","content":"tool completed"},"finish_reason":"stop"}],"usage":{"prompt_tokens":8,"completion_tokens":2,"total_tokens":10}}).to_string()).expect(1).create_async().await;
    let mut f = Fixture::new(&server.url()).await;
    let (tool, mut prompts) = f.tool().await;
    let received = Arc::new(Mutex::new(Vec::new()));
    let output = received.clone();
    let run = f.service.run(f.invoke(), f.input.clone(), Arc::new(move |event: AiEvent| { output.lock().unwrap().push(event); true }));
    let approve = async {
        let id = prompts.recv().await.unwrap();
        f.confirmations.resolve("alice", &id, true).unwrap();
    };
    let (terminal, ()) = tokio::join!(run, approve);
    let terminal = terminal.unwrap();
    assert_eq!(terminal.status, AiRunStatus::Succeeded);
    assert_eq!(terminal.content.as_deref(), Some("tool completed"));
    assert_eq!(terminal.total_tokens, Some(17));
    assert!(!terminal.result_unknown);
    let events = received.lock().unwrap();
    assert_eq!(events.iter().filter(|e| matches!(e.payload, AiEventPayload::Terminal { .. })).count(), 1);
    let id = events[0].run_id.clone();
    drop(events);
    let logs = f.mcp.list_usage_logs("alice", Some(&tool.server_id), None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].caller.as_deref(), Some("module"));
    assert_eq!(logs[0].task_ref.as_deref(), Some(id.as_str()));
    assert_eq!(logs[0].outcome, "success");
    f.mcp.shutdown().await.unwrap();
    first.assert_async().await;
    second.assert_async().await;
}

#[tokio::test]
async fn module_tool_inflight_invalidation_keeps_unknown_audit_and_usage() {
    use dgos_db::sea_orm::{ConnectionTrait, DbBackend, Statement};
    for reason in ["account", "attachment", "permission", "deadline"] {
        let mut server = mockito::Server::new_async().await;
        let model = server.mock("POST", "/chat/completions").with_header("content-type", "application/json")
            .with_body(tool_proposal(5000)).expect(1).create_async().await;
        let mut f = Fixture::new(&server.url()).await;
        let (tool, mut prompts) = f.tool().await;
        f.db.execute(Statement::from_string(DbBackend::Sqlite,
            "CREATE TRIGGER fail_mcp_terminal BEFORE UPDATE OF outcome ON mcp_usage_logs WHEN OLD.caller='module' BEGIN SELECT RAISE(ABORT,'fixture audit failure'); END".to_owned())).await.unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let output = received.clone();
        let invocation = f.invoke();
        let runtime = f.runtime.clone();
        let deadline = tokio::time::Instant::now() + if reason == "deadline" { Duration::from_secs(2) } else { Duration::from_secs(20) };
        let run = f.service.run_until(invocation, f.input.clone(), Arc::new(move |event: AiEvent| { output.lock().unwrap().push(event); true }), deadline);
        let revoke = async {
            let id = prompts.recv().await.unwrap();
            f.confirmations.resolve("alice", &id, true).unwrap();
            f.wait_tool_started(&tool).await;
            match reason {
                "account" => f.account.cancel(),
                "attachment" => f.page.cancel(),
                "permission" => { f.permissions.revoke("alice", "ai-consumer", ModulePermission::McpCall).await.unwrap(); },
                _ => {},
            }
        };
        let (terminal, ()) = tokio::join!(run, revoke);
        let terminal = terminal.unwrap();
        assert_eq!(terminal.status, AiRunStatus::Interrupted, "{reason}");
        assert!(terminal.result_unknown && terminal.audit_warning, "{reason}");
        assert!(terminal.error.as_ref().unwrap().result_unknown);
        assert_eq!(terminal.total_tokens, Some(7));
        assert!(terminal.content.is_none());
        assert_eq!(runtime.request_stats().in_flight, 0);
        let summaries = f.repository.list("alice", "ai-consumer", AiRunListInput::default()).await.unwrap();
        assert_eq!(summaries.items.len(), 1);
        assert_eq!(summaries.items[0].status, AiRunStatus::Interrupted);
        assert!(summaries.items[0].result_unknown);
        let events = received.lock().unwrap();
        assert!(!events.iter().any(|e| matches!(e.payload, AiEventPayload::Delta { .. })));
        if matches!(reason, "account" | "attachment") {
            assert!(!events.iter().any(|e| matches!(e.payload, AiEventPayload::Terminal { .. })));
        } else {
            assert_eq!(events.iter().filter(|e| matches!(e.payload, AiEventPayload::Terminal { .. })).count(), 1);
        }
        drop(events);
        f.db.execute(Statement::from_string(DbBackend::Sqlite,
            "DROP TRIGGER fail_mcp_terminal".to_owned())).await.unwrap();
        f.mcp.shutdown().await.unwrap();
        model.assert_async().await;
    }
}

#[tokio::test]
async fn revoked_then_regranted_proposal_cannot_dispatch() {
    let mut server = mockito::Server::new_async().await;
    let model = server.mock("POST", "/chat/completions").with_header("content-type", "application/json").with_body(tool_proposal(0)).expect(1).create_async().await;
    let mut f = Fixture::new(&server.url()).await;
    let (tool, mut prompts) = f.tool().await;
    let run = f.service.run(f.invoke(), f.input.clone(), Arc::new(|_: AiEvent| true));
    let revoke = async {
        let id = prompts.recv().await.unwrap();
        f.permissions.revoke("alice", "ai-consumer", ModulePermission::McpCall).await.unwrap();
        f.permissions.grant("alice", "ai-consumer", ModulePermission::McpCall).await.unwrap();
        let _ = f.confirmations.resolve("alice", &id, true);
    };
    let (terminal, ()) = tokio::join!(run, revoke);
    let terminal = terminal.unwrap();
    assert_eq!(terminal.status, AiRunStatus::Failed);
    assert!(!terminal.result_unknown);
    assert_eq!(terminal.error.unwrap().reason, "permission_denied");
    assert!(f.mcp.list_usage_logs("alice", Some(&tool.server_id), None).await.unwrap().is_empty());
    f.mcp.shutdown().await.unwrap();
    model.assert_async().await;
}
