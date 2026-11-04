//! Local fixture evidence: real HTTP gateway, SQLite authorization, and MCP stdio dispatch.
//! These tests do not establish compatibility with either installed native CLI.
use super::*;
use async_trait::async_trait;
use dgos_db::development_repository::{DevelopmentTask, StoredAgentSession, StoredRun};
use dgos_mcp::{CallMcpToolRequest, CreateMcpServerRequest, McpCallPolicy, StdioConfig, TransportKind};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream, task::JoinHandle};

struct FixturePolicy;
#[async_trait]
impl McpCallPolicy for FixturePolicy {
    async fn allows_call(&self, owner: &str, _: &str, _: &str) -> bool { owner == "alice" }
}

struct Chain { project: Project, task: DevelopmentTask, session: StoredAgentSession, run: StoredRun }
async fn chain(repository: &DevelopmentRepository, owner: &str, root: &Path) -> Chain {
    let path = root.join(uuid::Uuid::new_v4().to_string()).to_string_lossy().into_owned();
    let project = Project { id: uuid::Uuid::new_v4().to_string(), owner: owner.into(), name: "Gateway fixture".into(), path: path.clone(), directory_identity: path.clone(), archived: false, created_at: 1, last_opened_at: 1 };
    repository.insert_project(&project).await.unwrap();
    let task = DevelopmentTask { id: uuid::Uuid::new_v4().to_string(), owner: owner.into(), project_id: project.id.clone(), goal: "Exercise gateway authority".into(), requirements: "Explicit host consent per dispatch".into(), status: "todo".into(), created_at: 1 };
    repository.save_task(&task).await.unwrap();
    let session = StoredAgentSession { id: uuid::Uuid::new_v4().to_string(), owner: owner.into(), engine: "codex".into(), native_id: None, workspace: path, executable: None, model: None, project_id: Some(project.id.clone()), task_id: Some(task.id.clone()) };
    repository.save_session(&session).await.unwrap();
    let run = StoredRun { id: uuid::Uuid::new_v4().to_string(), owner: owner.into(), session_id: Some(session.id.clone()), task_id: task.id.clone(), project_id: project.id.clone(), engine: "codex".into(), status: "running".into(), started_at: 1, finished_at: None, summary: String::new(), reason: None, events: "[]".into(), fingerprint: "fixture".into(), start_fingerprint: "fixture".into(), truncated: false, request_fingerprint: uuid::Uuid::new_v4().to_string(), source: "native_agent".into(), runtime: None };
    repository.start_run(&run).await.unwrap();
    Chain { project, task, session, run }
}

#[derive(Clone)]
struct HttpGateway { address: String, bearer: String }
struct HttpResponse { status: u16, body: Vec<u8> }
impl HttpResponse {
    fn json(&self) -> Value { serde_json::from_slice(&self.body).unwrap() }
    fn error(&self) -> String { self.json()["error"]["message"].as_str().expect("expected JSON-RPC error").into() }
}
impl HttpGateway {
    async fn connect(&self, headers: &[(&str, &str)], body: &[u8]) -> std::io::Result<TcpStream> {
        let mut stream = TcpStream::connect(&self.address).await?;
        let mut request = format!("POST /mcp HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n", self.address, body.len());
        for (name, value) in headers { request.push_str(&format!("{name}: {value}\r\n")); }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).await?;
        stream.write_all(body).await?;
        Ok(stream)
    }
    async fn raw(&self, headers: &[(&str, &str)], body: &[u8]) -> HttpResponse {
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut stream = self.connect(headers, body).await.unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).await.unwrap();
            let split = bytes.windows(4).position(|w| w == b"\r\n\r\n").expect("HTTP response headers");
            let header = std::str::from_utf8(&bytes[..split]).unwrap();
            let status = header.split_whitespace().nth(1).unwrap().parse().unwrap();
            HttpResponse { status, body: bytes[split + 4..].to_vec() }
        }).await.expect("gateway HTTP request timed out")
    }
    async fn rpc(&self, value: Value) -> HttpResponse {
        let authorization = format!("Bearer {}", self.bearer);
        self.raw(&[("Authorization", &authorization)], &serde_json::to_vec(&value).unwrap()).await
    }
    fn call(&self, id: &str, name: &str, arguments: Value) -> JoinHandle<HttpResponse> {
        let gateway = self.clone();
        let value = json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}});
        tokio::spawn(async move { gateway.rpc(value).await })
    }
    async fn list(&self) -> HttpResponse {
        self.rpc(json!({"jsonrpc":"2.0","id":"list","method":"tools/list"})).await
    }
}

struct Fixture {
    root: PathBuf,
    service: ProjectCapabilities,
    chain: Chain,
    server: String,
    gateway: HttpGateway,
    binding: Arc<gateway::Binding>,
    lease: CancellationToken,
}
impl Fixture {
    async fn new(mode: &str) -> Self {
        let root = std::env::temp_dir().join(format!("dgos-gateway-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let repository = crate::test_support::repository(&root).await;
        let url = format!("sqlite://{}?mode=rwc", root.join("metadata.sqlite").display());
        let db = dgos_db::establish_connection(&url).await.unwrap();
        let providers = ProviderManager::new(&url).await.unwrap();
        let skills = Arc::new(RwLock::new(SkillsManager::new(db.clone(), root.join("skills"))));
        let mcp = Arc::new(McpManager::with_policy(db, Arc::new(FixturePolicy)));
        let server = mcp.create_server("alice", CreateMcpServerRequest {
            name: "Controlled gateway fixture".into(), transport: TransportKind::Stdio,
            stdio: Some(StdioConfig { executable: "node".into(), args: vec![format!("{}/../dgos-mcp/tests/fixtures/mcp-server.mjs", env!("CARGO_MANIFEST_DIR")), mode.into()], cwd: None }),
            http: None, enabled: Some(true), auto_connect: Some(false),
        }).await.unwrap();
        mcp.connect("alice", &server.id).await.unwrap();
        let tool = mcp.list_tools("alice", &server.id).await.unwrap().into_iter().find(|t| t.tool_name == "echo").unwrap();
        let service = ProjectCapabilities::new(repository, providers, skills, mcp, root.join("launches"));
        let chain = chain(&service.repository, "alice", &root).await;
        let configuration = CapabilityConfiguration { engine: Engine::Codex, provider: ProjectProvider::Native, skills: Vec::new(), mcp: vec![SelectedMcpTool { server_id: server.id.clone(), tool_name: tool.tool_name, schema_hash: tool.schema_hash }] };
        let revision = service.save("alice", SaveCapabilitiesInput { project_id: chain.project.id.clone(), expected_revision: 0, configuration }).await.unwrap();
        let snapshot = CapabilitySnapshot { revision: revision.revision, permission_epoch: revision.permission_epoch, engine: Engine::Codex, cli_version: "controlled-fixture".into(), configuration: revision.configuration, provider_status: "fixture".into(), skills_status: "fixture".into(), mcp_status: "fixture".into(), native_source: "fixture".into() };
        service.repository.save_session_capabilities("alice", &chain.session.id, &chain.project.id, snapshot.revision, snapshot.permission_epoch, &serde_json::to_string(&snapshot).unwrap()).await.unwrap();
        let lease = CancellationToken::new();
        let binding = Arc::new(gateway::Binding::new("alice", &chain.session.id, &chain.task.id, &chain.project.id, snapshot, lease.clone()));
        let gateway = start(&service, binding.clone()).await;
        service.bindings.lock().await.insert(chain.session.id.clone(), binding.clone());
        service.begin_run("alice", &chain.session.id, &chain.run.id).await.unwrap();
        Self { root, service, chain, server: server.id, gateway, binding, lease }
    }
    async fn prompt(&self) -> CapabilityConfirmation {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let prompts = self.service.pending_confirmations("alice", &self.chain.session.id).await.unwrap();
                if let Some(prompt) = prompts.into_iter().next() { return prompt; }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("host confirmation did not appear")
    }
    async fn answer(&self, prompt: &CapabilityConfirmation, approved: bool) {
        self.service.respond_confirmation("alice", &self.chain.session.id, &prompt.id, approved).await.unwrap();
    }
    async fn counts(&self) -> Value {
        let mut input = CallMcpToolRequest { server_id: self.server.clone(), tool_name: "echo".into(), arguments: json!({"text":"fixture-counts"}), call_id: Some(uuid::Uuid::new_v4().to_string()), task_ref: None, timeout_ms: Some(1000), confirmation_token: None };
        input.confirmation_token = Some(self.service.mcp.confirm_call("alice", &input).await.unwrap());
        let result = self.service.mcp.call_tool("alice", input).await.unwrap();
        serde_json::from_str(result.content[0]["text"].as_str().unwrap()).unwrap()
    }
    async fn wait_started(&self, expected: u64) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if self.counts().await["started"].as_u64().unwrap() >= expected { break; }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("fixture dispatch did not start")
    }
    async fn finish(&self) {
        self.service.close_session(&self.chain.session.id).await;
        self.service.mcp.shutdown().await.unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.binding.stop.cancel();
        self.lease.cancel();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
async fn start(service: &ProjectCapabilities, binding: Arc<gateway::Binding>) -> HttpGateway {
    let (url, bearer) = service.start_gateway(binding).await.unwrap();
    HttpGateway { address: url.strip_prefix("http://").unwrap().strip_suffix("/mcp").unwrap().into(), bearer }
}

#[tokio::test]
async fn http_authentication_precedes_parsing_and_unselected_tools_never_dispatch() {
    let f = Fixture::new("normal").await;
    assert_eq!(f.gateway.raw(&[], b"not-json").await.status, 401);
    assert_eq!(f.gateway.raw(&[("Authorization", "Bearer wrong")], b"not-json").await.status, 401);
    let auth = format!("Bearer {}", f.gateway.bearer);
    assert_eq!(f.gateway.raw(&[("Authorization", &auth), ("Origin", "http://localhost")], b"not-json").await.status, 401);
    assert_eq!(f.gateway.raw(&[("Authorization", &auth)], b"not-json").await.status, 400);
    let list = f.gateway.list().await;
    assert_eq!(list.status, 200);
    assert_eq!(list.json()["result"]["tools"].as_array().unwrap().len(), 1);
    assert_eq!(list.json()["result"]["tools"][0]["name"], "project_tool_0");
    for name in ["echo", "echo_second", "project_tool_1"] {
        assert_eq!(f.gateway.call(name, name, json!({"text":"denied"})).await.unwrap().error(), "tool_not_selected");
    }
    assert!(f.service.pending_confirmations("alice", &f.chain.session.id).await.unwrap().is_empty());
    assert!(f.service.mcp.list_usage_logs("alice", Some(&f.server), None).await.unwrap().is_empty());
    f.finish().await;
}

#[tokio::test]
async fn each_http_call_requires_fresh_host_consent_and_preserves_provenance() {
    let f = Fixture::new("normal").await;
    let refused = f.gateway.call("refused", "project_tool_0", json!({"text":"refused"}));
    let first = f.prompt().await;
    assert_eq!(first.project_id, f.chain.project.id);
    assert_eq!(first.task_id, f.chain.task.id);
    assert_eq!(first.session_id, f.chain.session.id);
    assert_eq!(first.run_id, f.chain.run.id);
    assert_eq!(first.arguments, json!({"text":"refused"}));
    assert!(!refused.is_finished());
    f.answer(&first, false).await;
    assert_eq!(refused.await.unwrap().error(), "confirmation_refused");
    assert!(f.service.mcp.list_usage_logs("alice", Some(&f.server), None).await.unwrap().is_empty());
    let approved = f.gateway.call("approved", "project_tool_0", json!({"text":"approved"}));
    let second = f.prompt().await;
    assert_ne!(first.id, second.id);
    f.answer(&second, true).await;
    let response = approved.await.unwrap().json();
    assert_eq!(response["result"]["content"][0]["text"], "approved");
    assert!(response["result"].get("structuredContent").is_none(), "MCP clients reject a null structuredContent; omit absent output");
    assert_eq!(f.service.respond_confirmation("alice", &f.chain.session.id, &second.id, true).await.unwrap_err(), "request_expired");
    let next = f.gateway.call("next", "project_tool_0", json!({"text":"new permission"}));
    let third = f.prompt().await;
    assert_ne!(second.id, third.id);
    assert!(!next.is_finished());
    f.answer(&third, false).await;
    assert_eq!(next.await.unwrap().error(), "confirmation_refused");
    let logs = f.service.mcp.list_usage_logs("alice", Some(&f.server), None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].outcome, "success");
    assert_eq!(logs[0].caller.as_deref(), Some("workbench"));
    assert_eq!(logs[0].task_ref.as_deref(), Some(f.chain.task.id.as_str()));
    assert!(!format!("{logs:?}").contains("new permission"));
    f.finish().await;
}

#[tokio::test]
async fn schema_refresh_invalidates_the_prompt_and_pinned_tool_before_dispatch() {
    let f = Fixture::new("output-change").await;
    let request = f.gateway.call("old-schema", "project_tool_0", json!({"text":"old schema"}));
    let prompt = f.prompt().await;
    let refreshed = f.service.mcp.refresh_tools("alice", &f.server).await.unwrap();
    assert_ne!(refreshed.iter().find(|t| t.tool_name == "echo").unwrap().schema_hash, prompt.schema_hash);
    f.answer(&prompt, true).await;
    let response = request.await.unwrap();
    assert_eq!(response.error(), "schema_changed", "old prompt must not authorize the changed schema");
    assert_eq!(f.gateway.list().await.error(), "schema_changed");
    assert_eq!(f.gateway.call("after-refresh", "project_tool_0", json!({"text":"changed"})).await.unwrap().error(), "schema_changed");
    assert!(f.service.pending_confirmations("alice", &f.chain.session.id).await.unwrap().is_empty());
    assert!(f.service.mcp.list_usage_logs("alice", Some(&f.server), None).await.unwrap().iter().all(|row| row.outcome != "success" && row.outcome != "pending"));
    f.finish().await;
}

#[tokio::test]
async fn confirmations_and_bearers_cannot_cross_account_project_session_or_run() {
    let f = Fixture::new("normal").await;
    let other = chain(&f.service.repository, "alice", &f.root).await;
    let foreign = chain(&f.service.repository, "bob", &f.root).await;
    let request = f.gateway.call("scoped", "project_tool_0", json!({"text":"scoped"}));
    let prompt = f.prompt().await;
    assert_eq!(f.service.pending_confirmations("bob", &f.chain.session.id).await.err().unwrap(), "session_not_found");
    assert_eq!(f.service.respond_confirmation("bob", &f.chain.session.id, &prompt.id, true).await.unwrap_err(), "session_not_found");
    let other_binding = Arc::new(gateway::Binding::new("alice", &other.session.id, &other.task.id, &other.project.id, f.binding.snapshot.clone(), CancellationToken::new()));
    f.service.bindings.lock().await.insert(other.session.id.clone(), other_binding.clone());
    f.service.begin_run("alice", &other.session.id, &other.run.id).await.unwrap();
    let other_gateway = start(&f.service, other_binding.clone()).await;
    let wrong_bearer = format!("Bearer {}", f.gateway.bearer);
    assert_eq!(other_gateway.raw(&[("Authorization", &wrong_bearer)], b"{}").await.status, 401);
    assert_eq!(f.service.respond_confirmation("alice", &other.session.id, &prompt.id, true).await.unwrap_err(), "request_expired");
    assert!(!request.is_finished());
    // A stored run from a different session/project/account cannot become current authority.
    for run in [&other.run, &foreign.run] {
        *f.binding.run.lock().await = Some((run.id.clone(), CancellationToken::new()));
        assert_eq!(f.gateway.call(&run.id, "project_tool_0", json!({"text":"foreign run"})).await.unwrap().error(), "run_not_active");
        assert!(f.service.respond_confirmation("alice", &f.chain.session.id, &prompt.id, true).await.is_err());
    }
    *f.binding.run.lock().await = Some((f.chain.run.id.clone(), CancellationToken::new()));
    f.answer(&prompt, false).await;
    assert_eq!(request.await.unwrap().error(), "confirmation_refused");
    let invalid_project = Arc::new(gateway::Binding::new("alice", &f.chain.session.id, &other.task.id, &f.chain.project.id, f.binding.snapshot.clone(), CancellationToken::new()));
    let invalid_gateway = start(&f.service, invalid_project.clone()).await;
    assert_eq!(invalid_gateway.list().await.status, 403);
    invalid_project.stop.cancel();
    other_binding.stop.cancel();
    assert!(f.service.mcp.list_usage_logs("alice", Some(&f.server), None).await.unwrap().is_empty());
    f.finish().await;
}

#[tokio::test]
async fn ending_a_run_expires_pending_host_consent_without_external_dispatch() {
    let f = Fixture::new("normal").await;
    let request = f.gateway.call("ending", "project_tool_0", json!({"text":"must not dispatch"}));
    let prompt = f.prompt().await;
    f.service.end_run(&f.chain.session.id).await;
    assert!(matches!(request.await.unwrap().error().as_str(), "confirmation_refused" | "run_not_active"));
    assert_eq!(f.service.respond_confirmation("alice", &f.chain.session.id, &prompt.id, true).await.unwrap_err(), "run_not_active");
    assert!(f.service.pending_confirmations("alice", &f.chain.session.id).await.unwrap().is_empty());
    assert!(f.service.mcp.list_usage_logs("alice", Some(&f.server), None).await.unwrap().is_empty());
    f.finish().await;
}

#[tokio::test]
async fn previous_run_confirmation_cannot_authorize_the_next_run_in_the_same_session() {
    let f = Fixture::new("normal").await;
    let previous = f.gateway.call("previous-run", "project_tool_0", json!({"text":"previous arguments"}));
    let old_prompt = f.prompt().await;
    f.service.end_run(&f.chain.session.id).await;
    assert!(matches!(previous.await.unwrap().error().as_str(), "confirmation_refused" | "run_not_active"));
    let mut completed = f.chain.run.clone();
    completed.status = "completed".into();
    completed.finished_at = Some(2);
    f.service.repository.finish_run("alice", &completed, None).await.unwrap();
    let mut next_run = f.chain.run.clone();
    next_run.id = uuid::Uuid::new_v4().to_string();
    next_run.started_at = 3;
    f.service.repository.start_run(&next_run).await.unwrap();
    f.service.begin_run("alice", &f.chain.session.id, &next_run.id).await.unwrap();
    let next = f.gateway.call("next-run", "project_tool_0", json!({"text":"next arguments"}));
    let prompt = f.prompt().await;
    assert_eq!(prompt.run_id, next_run.id);
    assert_ne!(prompt.id, old_prompt.id);
    assert_eq!(f.service.respond_confirmation("alice", &f.chain.session.id, &old_prompt.id, true).await.unwrap_err(), "request_expired");
    assert!(!next.is_finished());
    f.answer(&prompt, false).await;
    assert_eq!(next.await.unwrap().error(), "confirmation_refused");
    assert!(f.service.mcp.list_usage_logs("alice", Some(&f.server), None).await.unwrap().is_empty());
    f.finish().await;
}

#[tokio::test]
async fn epoch_revocation_is_live_and_lease_cancellation_stops_pending_authority() {
    for revoke_epoch in [true, false] {
        let f = Fixture::new("normal").await;
        let request = f.gateway.call("pending-revocation", "project_tool_0", json!({"text":"must not dispatch"}));
        let prompt = f.prompt().await;
        if revoke_epoch {
            // Exercise the database epoch check independently from service.revoke's stop token.
            assert_eq!(f.service.repository.revoke_project_capabilities("alice", &f.chain.project.id).await.unwrap(), 1);
            assert_eq!(f.gateway.list().await.status, 403);
            assert_eq!(f.service.respond_confirmation("alice", &f.chain.session.id, &prompt.id, true).await.unwrap_err(), "capability_revoked");
            f.service.revoke("alice", &f.chain.project.id).await.unwrap();
        } else {
            f.lease.cancel();
            assert_eq!(f.service.respond_confirmation("alice", &f.chain.session.id, &prompt.id, true).await.unwrap_err(), "capability_revoked");
        }
        assert!(matches!(request.await.unwrap().error().as_str(), "capability_revoked" | "capability_revoked_result_unknown" | "confirmation_refused"));
        assert!(f.service.mcp.list_usage_logs("alice", Some(&f.server), None).await.unwrap().is_empty());
        f.finish().await;
    }
}

#[tokio::test]
async fn dispatched_request_cancellation_is_unknown_and_never_automatically_replayed() {
    let f = Fixture::new("cancellation-observable").await;
    let request = f.gateway.call("cancel-dispatched", "project_tool_0", json!({"text":"late result", "delay":400}));
    f.answer(&f.prompt().await, true).await;
    f.wait_started(1).await;
    let notification = f.gateway.rpc(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"cancel-dispatched"}})).await;
    assert_eq!(notification.status, 202);
    let response = request.await.unwrap();
    assert!(response.json().get("error").is_some() || response.json()["result"]["isError"] == true);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let counts = f.counts().await;
            assert_eq!(counts["started"], 1, "gateway must never retry an uncertain external side effect");
            if counts["cancellations"] == 1 { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("cancellation must reach the MCP transport");
    tokio::time::sleep(Duration::from_millis(450)).await;
    assert_eq!(f.counts().await["started"], 1);
    let logs = f.service.mcp.list_usage_logs("alice", Some(&f.server), None).await.unwrap();
    assert_eq!(logs.iter().filter(|row| row.task_ref.as_deref() == Some(f.chain.task.id.as_str())).count(), 1);
    assert!(logs.iter().filter(|row| row.task_ref.as_deref() == Some(f.chain.task.id.as_str())).all(|row| row.outcome != "pending" && row.outcome != "success"));
    assert!(f.service.pending_confirmations("alice", &f.chain.session.id).await.unwrap().is_empty());
    f.finish().await;
}

#[tokio::test]
async fn ending_a_dispatched_run_drains_remote_cancellation_and_retains_one_audit() {
    let f = Fixture::new("cancellation-observable").await;
    let request = f.gateway.call("run-cancel", "project_tool_0", json!({"text":"uncertain side effect", "delay":400}));
    f.answer(&f.prompt().await, true).await;
    f.wait_started(1).await;
    f.service.end_run(&f.chain.session.id).await;
    assert_eq!(request.await.unwrap().error(), "cancelled_result_unknown");
    tokio::time::sleep(Duration::from_millis(450)).await;
    let counts = f.counts().await;
    assert_eq!(counts["started"], 1);
    assert_eq!(counts["cancellations"], 1);
    let logs = f.service.mcp.list_usage_logs("alice", Some(&f.server), None).await.unwrap();
    let run_logs: Vec<_> = logs.iter().filter(|row| row.task_ref.as_deref() == Some(f.chain.task.id.as_str())).collect();
    assert_eq!(run_logs.len(), 1);
    assert_eq!(run_logs[0].outcome, "cancelled");
    assert_eq!(f.gateway.call("after-run", "project_tool_0", json!({"text":"must not retry"})).await.unwrap().error(), "run_not_active");
    f.finish().await;
}

#[tokio::test]
async fn revoking_or_cancelling_a_dispatched_lease_drains_once_without_replay() {
    for revoke in [true, false] {
        let f = Fixture::new("cancellation-observable").await;
        let request = f.gateway.call("lease-cancel", "project_tool_0", json!({"text":"uncertain lease effect", "delay":400}));
        f.answer(&f.prompt().await, true).await;
        f.wait_started(1).await;
        if revoke {
            assert_eq!(f.service.revoke("alice", &f.chain.project.id).await.unwrap(), 1);
        } else {
            f.lease.cancel();
        }
        let response = request.await.unwrap();
        assert!(response.json().get("error").is_some() || response.json()["result"]["isError"] == true);
        tokio::time::sleep(Duration::from_millis(450)).await;
        let counts = f.counts().await;
        assert_eq!(counts["started"], 1);
        assert_eq!(counts["cancellations"], 1);
        let logs = f.service.mcp.list_usage_logs("alice", Some(&f.server), None).await.unwrap();
        let run_logs: Vec<_> = logs.iter().filter(|row| row.task_ref.as_deref() == Some(f.chain.task.id.as_str())).collect();
        assert_eq!(run_logs.len(), 1);
        assert_eq!(run_logs[0].outcome, "cancelled");
        assert_eq!(f.service.pending_confirmations("alice", &f.chain.session.id).await.err().unwrap(), "capability_revoked");
        f.finish().await;
    }
}

#[tokio::test]
async fn mcp_disconnect_cancels_once_and_does_not_reconnect_or_replay() {
    let f = Fixture::new("cancellation-observable").await;
    let request = f.gateway.call("mcp-disconnect", "project_tool_0", json!({"text":"uncertain effect", "delay":1000}));
    f.answer(&f.prompt().await, true).await;
    f.wait_started(1).await;
    assert_eq!(f.service.mcp.disconnect_user("alice").await.unwrap(), 1);
    let response = request.await.unwrap().json();
    assert!(response.get("error").is_some() || response["result"]["isError"] == true);
    let logs = f.service.mcp.list_usage_logs("alice", Some(&f.server), None).await.unwrap();
    let run_logs: Vec<_> = logs.iter().filter(|row| row.task_ref.as_deref() == Some(f.chain.task.id.as_str())).collect();
    assert_eq!(run_logs.len(), 1);
    assert_eq!(run_logs[0].outcome, "cancelled");
    assert_eq!(f.service.mcp.list_servers("alice").await.unwrap()[0].status, "configured");
    assert!(f.service.pending_confirmations("alice", &f.chain.session.id).await.unwrap().is_empty());
    assert_eq!(f.service.mcp.list_connection_logs("alice", Some(&f.server), None).await.unwrap().iter().filter(|row| row.event == "connected").count(), 1);
    f.finish().await;
}

#[tokio::test]
async fn duplicate_http_request_ids_cannot_create_two_host_prompts_or_dispatches() {
    let f = Fixture::new("normal").await;
    let first = f.gateway.call("same-id", "project_tool_0", json!({"text":"first"}));
    let prompt = f.prompt().await;
    let duplicate = f.gateway.call("same-id", "project_tool_0", json!({"text":"duplicate"}));
    assert_eq!(duplicate.await.unwrap().error(), "request_conflict");
    assert_eq!(f.service.pending_confirmations("alice", &f.chain.session.id).await.unwrap().len(), 1);
    f.answer(&prompt, true).await;
    assert_eq!(first.await.unwrap().json()["result"]["content"][0]["text"], "first");
    assert_eq!(f.service.mcp.list_usage_logs("alice", Some(&f.server), None).await.unwrap().len(), 1);
    f.finish().await;
}

#[tokio::test]
async fn http_disconnect_does_not_replay_a_dispatched_side_effect() {
    let f = Fixture::new("cancellation-observable").await;
    let auth = format!("Bearer {}", f.gateway.bearer);
    let bytes = serde_json::to_vec(&json!({"jsonrpc":"2.0","id":"lost-http","method":"tools/call","params":{"name":"project_tool_0","arguments":{"text":"lost result","delay":150}}})).unwrap();
    let connection = f.gateway.connect(&[("Authorization", &auth)], &bytes).await.unwrap();
    f.answer(&f.prompt().await, true).await;
    f.wait_started(1).await;
    drop(connection);
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(f.counts().await["started"], 1);
    assert!(f.service.pending_confirmations("alice", &f.chain.session.id).await.unwrap().is_empty());
    // A separately initiated request after transport loss requires a new host decision.
    let next = f.gateway.call("explicit-next", "project_tool_0", json!({"text":"explicit next"}));
    let prompt = f.prompt().await;
    assert!(!next.is_finished());
    f.answer(&prompt, false).await;
    assert_eq!(next.await.unwrap().error(), "confirmation_refused");
    assert_eq!(f.counts().await["started"], 1);
    f.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn saved_configuration_revisions_do_not_rewrite_existing_session_or_run_snapshots() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("normal").await;
    let executable = f.root.join("version-only-cli");
    std::fs::write(&executable, "#!/bin/sh\nprintf 'controlled-fixture\\n'\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let options = SessionOptions { workspace: f.chain.project.path.clone(), resume_id: None, executable: Some(executable.to_string_lossy().into_owned()), model: None, trust_workspace: true };
    let latest = f.service.save("alice", SaveCapabilitiesInput { project_id: f.chain.project.id.clone(), expected_revision: 1, configuration: CapabilityConfiguration { engine: Engine::Codex, provider: ProjectProvider::Native, skills: Vec::new(), mcp: Vec::new() } }).await.unwrap();
    assert_eq!(latest.revision, 2);
    assert_eq!(f.gateway.list().await.json()["result"]["tools"].as_array().unwrap().len(), 1);
    assert_eq!(f.service.session_snapshot("alice", &f.chain.session.id).await.unwrap().unwrap().revision, 1);
    let run: CapabilitySnapshot = serde_json::from_str(&f.service.repository.run_capabilities("alice", &f.chain.run.id).await.unwrap().unwrap()).unwrap();
    assert_eq!(run.revision, 1);
    assert_eq!(run.configuration.mcp.len(), 1);
    let conflict = f.service.prepare("alice", &f.chain.session.id, &f.chain.task.id, &f.chain.project, Engine::Codex, &options, Some(2), true, CancellationToken::new()).await;
    assert!(matches!(&conflict, Err(reason) if reason == "capability_conflict"));
    let (restored, launch) = f.service.prepare("alice", &f.chain.session.id, &f.chain.task.id, &f.chain.project, Engine::Codex, &options, None, true, CancellationToken::new()).await.unwrap();
    assert_eq!(restored.revision, 1);
    assert_eq!(restored.configuration.mcp.len(), 1);
    let restored_gateway = HttpGateway { address: launch.mcp_url.unwrap().strip_prefix("http://").unwrap().strip_suffix("/mcp").unwrap().into(), bearer: launch.mcp_bearer.unwrap() };
    assert_eq!(restored_gateway.list().await.json()["result"]["tools"].as_array().unwrap().len(), 1);
    // A separate session selects the new revision without rewriting the old session chain.
    let mut new_session = f.chain.session.clone();
    new_session.id = uuid::Uuid::new_v4().to_string();
    f.service.repository.save_session(&new_session).await.unwrap();
    let forbidden = f.service.prepare("alice", &new_session.id, &f.chain.task.id, &f.chain.project, Engine::Codex, &options, Some(2), true, CancellationToken::new()).await;
    assert!(matches!(&forbidden, Err(reason) if reason == "capability_conflict"), "restoring an unpinned native session must not gain managed capabilities");
    let (fresh, _) = f.service.prepare("alice", &new_session.id, &f.chain.task.id, &f.chain.project, Engine::Codex, &options, Some(2), false, CancellationToken::new()).await.unwrap();
    assert_eq!(fresh.revision, 2);
    assert!(fresh.configuration.mcp.is_empty());
    f.service.close_session(&new_session.id).await;
    f.finish().await;
}
