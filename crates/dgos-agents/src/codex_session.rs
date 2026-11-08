use crate::{
    diagnostics::ProcessDiagnostic,
    session::{PendingRequest, SessionOptions, SessionView},
    transport::Transport,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
use tokio::{process::Command, sync::Mutex};

#[path = "codex_history.rs"]
mod history;
#[path = "codex_permissions.rs"]
mod permissions;

type SendReceipt = ([u8; 32], Option<Result<(), String>>);

// Codex rust-v0.167.0 App Server v2: turn/start returns the turn identity;
// notifications may precede that reply. Approval IDs exposed to callers are
// host-owned so native RPC ID reuse after resume cannot accept an old answer.

#[derive(Default)]
struct TurnState {
    id: Option<String>,
    starting: bool,
    opening: bool,
    deferred: Vec<Value>,
    user_echo_pending: bool,
    deferred_bytes: usize,
    requests: HashMap<String, NativeRequest>,
}

struct NativeRequest {
    public_id: String,
    kind: String,
    params: Value,
    answered: bool,
}

#[derive(Clone)]
pub struct CodexSession {
    wire: Transport,
    view: Arc<Mutex<SessionView>>,
    receipts: Arc<Mutex<HashMap<String, SendReceipt>>>,
    turn: Arc<Mutex<TurnState>>,
    native_protocol_supported: bool,
}
impl CodexSession {
    async fn event(&self, event: Value) {
        let mut view = self.view.lock().await;
        let mut turn = self.turn.lock().await;
        // Closure must invalidate an opening immediately, not wait behind its
        // history replies; a late resume reply cannot revive a closed transport.
        if event.get("method").and_then(Value::as_str) == Some("host/closed") {
            Self::apply_event(&mut view, &mut turn, event);
            return;
        }
        if turn.starting || turn.opening {
            let bytes = event_size(&event);
            if turn.deferred.len() >= 128 || turn.deferred_bytes + bytes > 16 * 1024 * 1024 {
                turn.starting = false;
                turn.opening = false;
                turn.deferred.clear();
                turn.deferred_bytes = 0;
                view.status = "unknown".into();
                view.error = Some("result_unknown".into());
            } else {
                turn.deferred_bytes += bytes;
                turn.deferred.push(event);
            }
            return;
        }
        let reply = Self::apply_event(&mut view, &mut turn, event);
        drop(turn);
        drop(view);
        if let Some(reply) = reply { let _ = self.wire.write(reply).await; }
    }
    fn apply_event(view: &mut SessionView, turn: &mut TurnState, event: Value) -> Option<Value> {
        let method = event.get("method").and_then(Value::as_str).unwrap_or("");
        let params = event.get("params").cloned().unwrap_or(Value::Null);
        if let Some(id) = event.get("id") {
            if !matches!(method, "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" | "item/permissions/requestApproval" | "tool/requestUserInput" | "mcpServer/elicitation/request") {
                return Some(json!({"id":id,"error":{"code":-32601,"message":"Unsupported client request"}}));
            }
            let standalone = method == "mcpServer/elicitation/request" && params.get("turnId").is_none_or(Value::is_null);
            if !Self::matches_thread(view, &params) || (!standalone && !Self::matches_turn(view, turn, &params)) || matches!(view.status.as_str(), "closed" | "unknown" | "starting") {
                return Some(json!({"id":id,"error":{"code":-32600,"message":"Request expired"}}));
            }
            let native_id = id.to_string();
            if let Some(previous) = turn.requests.get(&native_id) {
                if previous.kind != method || previous.params != params {
                    view.pending.retain(|pending| pending.id != previous.public_id);
                    view.status = "unknown".into();
                    view.error = Some("request_conflict".into());
                }
                return None;
            }
            if turn.requests.len() >= 1024 {
                view.status = "unknown".into();
                view.error = Some("request_capacity".into());
                return Some(json!({"id":id,"error":{"code":-32600,"message":"Request capacity exceeded"}}));
            }
            if method == "item/permissions/requestApproval" && !permissions::valid_request(&params) {
                return Some(json!({"id":id,"error":{"code":-32602,"message":"Unsupported permission request"}}));
            }
            let public_id = uuid::Uuid::new_v4().to_string();
            view.request(PendingRequest { id: public_id.clone(), kind: method.into(), title: method.into(), data: params.clone() });
            turn.requests.insert(native_id, NativeRequest { public_id, kind: method.into(), params, answered: false });
            return None;
        }
        let turn_event = method.starts_with("turn/") || method.starts_with("item/") || method == "error";
        if turn_event && (matches!(view.status.as_str(), "closed" | "unknown") || !Self::matches_turn(view, turn, &params)) { return None; }
        match method {
            "turn/started" => {
                view.status = if view.pending.is_empty() { "running" } else { "waiting" }.into();
            }
            "turn/completed" => {
                let status = params.pointer("/turn/status").and_then(Value::as_str).unwrap_or("unknown").to_owned();
                turn.id = None;
                let (session, error) = match status.as_str() {
                    "completed" => ("idle", None),
                    "interrupted" => ("idle", None),
                    "failed" => ("error", Some(native_error(&params))),
                    _ => ("unknown", Some("engine_unknown".to_owned())),
                };
                let run = match status.as_str() {
                    "completed" => "completed",
                    "interrupted" => "cancelled",
                    "failed" => "failed",
                    _ => "unknown",
                };
                view.status = session.into();
                view.error = error;
                view.pending.clear();
                let mut terminal = params.clone();
                terminal["run_status"] = json!(run);
                terminal["native_status"] = json!(status);
                view.push(method, (run == "failed").then(|| native_error(&params)), terminal);
                turn.user_echo_pending = false;
                view.active_run = None;
            }
            "item/agentMessage/delta" => view.push(
                method,
                params
                    .get("delta")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                json!({"itemId":params.get("itemId")}),
            ),
            "item/started" => {
                if let Some(item) = params.get("item").filter(|item| !matches!(item.get("type").and_then(Value::as_str), Some("userMessage" | "agentMessage"))) {
                    view.push(method, None, item.clone());
                }
            }
            "item/commandExecution/outputDelta" | "item/fileChange/outputDelta" => {
                view.push(method, params.get("delta").and_then(Value::as_str).map(str::to_owned), params.clone());
            }
            "item/completed" => {
                if let Some(item) = params.get("item") {
                    Self::completed_item(view, item, &mut turn.user_echo_pending);
                }
            }
            "turn/diff/updated" => view.push(
                method,
                params.get("diff").and_then(Value::as_str).map(str::to_owned),
                json!({"turnId":params.get("turnId")}),
            ),
            "turn/plan/updated" => view.push(method, None, json!({"plan":params.get("plan"),"explanation":params.get("explanation"),"turnId":params.get("turnId")})),
            "serverRequest/resolved" => {
                if Self::matches_thread(view, &params) {
                    if let Some(request) = params.get("requestId").and_then(|id| turn.requests.get_mut(&id.to_string())) {
                        request.answered = true;
                        view.pending.retain(|pending| pending.id != request.public_id);
                    }
                }
                if view.pending.is_empty() && view.status == "waiting" {
                    view.status = if view.active_run.is_some() { "running" } else { "idle" }.into();
                }
            }
            "host/closed" => {
                view.status = if view.status == "closed" {
                    "closed"
                } else {
                    "unknown"
                }
                .into();
                view.pending.clear();
                turn.id = None;
                turn.starting = false;
                turn.opening = false;
                turn.deferred.clear();
                turn.deferred_bytes = 0;
                turn.user_echo_pending = false;
            }
            "error" => {
                let message = native_error(&params);
                view.error = Some(message.clone());
                view.push(method, Some(message), params);
            }
            _ => {}
        }
        None
    }
    fn matches_thread(view: &SessionView, params: &Value) -> bool {
        view.native_id.as_deref().is_some_and(|id| params.get("threadId").and_then(Value::as_str) == Some(id))
    }
    fn matches_turn(view: &SessionView, turn: &TurnState, params: &Value) -> bool {
        Self::matches_thread(view, params) && view.active_run.is_some() && turn.id.as_deref().is_some_and(|id| {
            params.get("turnId").or_else(|| params.pointer("/turn/id")).and_then(Value::as_str) == Some(id)
        })
    }
    pub async fn send(&self, request_id: String, text: String) -> Result<(), String> {
        if text.trim().is_empty() || text.len() > 65536 {
            return Err("invalid_prompt".into());
        }
        let fingerprint: [u8; 32] = Sha256::digest(text.as_bytes()).into();
        let mut receipts = self.receipts.lock().await;
        if let Some(old) = receipts.get(&request_id) {
            return if old.0 == fingerprint {
                old.1
                    .clone()
                    .unwrap_or_else(|| Err("request_pending".into()))
            } else {
                Err("request_conflict".into())
            };
        }
        if receipts.len() >= 1024 {
            return Err("request_capacity".into());
        }
        let native = {
            let mut view = self.view.lock().await;
            if view.status != "idle" || !self.wire.native_available() { return Err("busy".into()); }
            let native = view.native_id.clone().ok_or("closed")?;
            let mut turn = self.turn.lock().await;
            turn.id = None;
            turn.starting = true;
            turn.deferred.clear();
            turn.deferred_bytes = 0;
            turn.requests.retain(|_, request| request.kind == "mcpServer/elicitation/request" && request.params.get("turnId").is_none_or(Value::is_null));
            turn.user_echo_pending = true;
            view.status = "running".into();
            view.active_run = Some(request_id.clone());
            view.error = None;
            view.push("userMessage", Some(text.clone()), Value::Null);
            native
        };
        receipts.insert(request_id.clone(), (fingerprint, None));
        drop(receipts);
        let result = self
            .wire
            .request(
                "turn/start",
                json!({"threadId":native,"input":[{"type":"text","text":text}]}),
            )
            .await
            .and_then(|reply| reply.pointer("/turn/id").and_then(Value::as_str).filter(|id| !id.is_empty()).map(str::to_owned).ok_or_else(|| "invalid_output".into()));
        let mut view = self.view.lock().await;
        let mut turn = self.turn.lock().await;
        turn.starting = false;
        turn.deferred_bytes = 0;
        let result = if view.status == "unknown" || view.status == "closed" { Err("result_unknown".into()) } else { result };
        let mut replies = Vec::new();
        match &result {
            Ok(id) => {
                turn.id = Some(id.clone());
                for event in std::mem::take(&mut turn.deferred) {
                    if let Some(reply) = Self::apply_event(&mut view, &mut turn, event) { replies.push(reply); }
                }
            }
            Err(error) => {
                turn.id = None;
                turn.deferred.clear();
                turn.user_echo_pending = false;
                view.status = "unknown".into();
                view.error = Some(error.clone());
                view.pending.clear();
                view.push("error", Some(error.clone()), json!({"run_status":"unknown","native_status":"send_failed"}));
                view.active_run = None;
            }
        }
        drop(turn);
        drop(view);
        let result = result.map(|_| ());
        self.receipts.lock().await.insert(request_id, (fingerprint, Some(result.clone())));
        for reply in replies { let _ = self.wire.write(reply).await; }
        result
    }
    pub async fn respond(&self, request_id: String, response: Value) -> Result<(), String> {
        let mut view = self.view.lock().await;
        let mut turn = self.turn.lock().await;
        if matches!(view.status.as_str(), "unknown" | "closed" | "starting") { return Err("request_expired".into()); }
        let pending = view.pending.iter().find(|pending| pending.id == request_id).ok_or("request_expired")?;
        let standalone = pending.kind == "mcpServer/elicitation/request" && pending.data.get("turnId").is_none_or(Value::is_null);
        if !Self::matches_thread(&view, &pending.data) || (!standalone && !Self::matches_turn(&view, &turn, &pending.data)) { return Err("request_expired".into()); }
        if !Self::valid_response(pending, &response) { return Err("invalid_response".into()); }
        let (native_id, request) = turn.requests.iter_mut().find(|(_, request)| request.public_id == request_id && !request.answered).ok_or("request_expired")?;
        let id: Value = serde_json::from_str(native_id).map_err(|_| "invalid_response")?;
        // Reserve before dispatch: an uncertain write must never authorize a replacement action.
        request.answered = true;
        view.pending.retain(|pending| pending.id != request_id);
        let result = self.wire.write(json!({"id":id,"result":response})).await;
        if result.is_err() {
            view.status = "unknown".into();
            view.error = Some("result_unknown".into());
        } else if view.pending.is_empty() {
            view.status = if view.active_run.is_some() { "running" } else { "idle" }.into();
        }
        result
    }
    fn valid_response(pending: &PendingRequest, response: &Value) -> bool {
        let valid = match pending.kind.as_str() {
            "item/permissions/requestApproval" => permissions::valid_response(&pending.data, response),
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                let decision = response.get("decision");
                decision.is_some_and(|decision| {
                    decision.as_str().is_some_and(|d| matches!(d, "accept" | "acceptForSession" | "decline" | "cancel"))
                        || (pending.kind == "item/commandExecution/requestApproval" && decision.is_object() && pending.data.get("availableDecisions").and_then(Value::as_array).is_some_and(|allowed| allowed.contains(decision)))
                }) && pending.data.get("availableDecisions").and_then(Value::as_array).is_none_or(|allowed| allowed.contains(decision.unwrap_or(&Value::Null)))
                    && response.as_object().is_some_and(|object| object.len() == 1)
            }
            "tool/requestUserInput" => response.as_object().is_some_and(|object| object.len() == 1) && response.get("answers").and_then(Value::as_object).is_some_and(|answers| {
                pending.data.get("questions").and_then(Value::as_array).is_some_and(|questions| {
                    answers.iter().all(|(id, value)| {
                        questions.iter().any(|question| question.get("id").and_then(Value::as_str) == Some(id.as_str()))
                            && value.as_object().is_some_and(|object| object.len() == 1)
                            && value.get("answers").and_then(Value::as_array).is_some_and(|answers| answers.iter().all(Value::is_string))
                    })
                })
            }),
            "mcpServer/elicitation/request" => response.as_object().is_some_and(|object| object.keys().all(|key| matches!(key.as_str(), "action" | "content" | "_meta")))
                && response.get("action").and_then(Value::as_str).is_some_and(|action| matches!(action, "accept" | "decline" | "cancel"))
                && response.get("content").is_none_or(|content| content.is_null() || content.is_object())
                && response.get("_meta").is_none_or(|meta| meta.is_null() || meta.is_object()),
            _ => false,
        };
        valid
    }
    pub async fn interrupt(&self) -> Result<(), String> {
        let view = self.view.lock().await;
        let native = view.native_id.clone().ok_or("closed")?;
        let turn = self.turn.lock().await.id.clone().ok_or("not_running")?;
        drop(view);
        self.wire
            .request("turn/interrupt", json!({"threadId":native,"turnId":turn}))
            .await
            .map(|_| ())
    }
    pub async fn snapshot(&self) -> SessionView {
        self.view.lock().await.clone()
    }
    pub(crate) fn native_available(&self) -> bool { self.wire.native_available() }
    pub(crate) fn native_capabilities(&self) -> crate::native_session::NativeSessionCapabilities {
        if !self.native_protocol_supported || !self.native_available() { return crate::native_session::NativeSessionCapabilities { native_auth_guidance: Some(crate::native_session::NativeAuthGuidance::installed(crate::Engine::Codex)), ..Default::default() }; }
        crate::native_session::NativeSessionCapabilities { observe_skills: true, observe_mcp: true, reload_skills: true, native_auth_guidance: Some(crate::native_session::NativeAuthGuidance::installed(crate::Engine::Codex)), ..Default::default() }
    }
    pub(crate) async fn observe_native(&self, domain: crate::native_session::SessionDomain) -> Result<crate::native_session::Projection, String> {
        use crate::native_session::{self as native, SessionDomain};
        if !self.native_protocol_supported || !self.native_available() { return Err("native_extension_Unsupported".into()); }
        let view = self.snapshot().await;
        let mut out = native::Projection::default();
        match domain {
            SessionDomain::Skills => {
                let result = self.wire.request("skills/list", json!({"cwds":[view.workspace],"forceReload":false})).await?;
                let entries = result["data"].as_array().ok_or("native_extension_InvalidFormat")?;
                for entry in entries.iter().take(16) {
                    native::skills(entry["skills"].as_array().ok_or("native_extension_InvalidFormat")?, true, &mut out);
                    out.incomplete |= entry["errors"].as_array().is_none_or(|errors| !errors.is_empty());
                }
                out.incomplete |= entries.len() != 1;
            }
            SessionDomain::Mcp => {
                let thread = view.native_id.ok_or("session_closed")?;
                let mut cursor = Value::Null;
                let mut cursors = std::collections::HashSet::new();
                for _ in 0..4 {
                    let result = self.wire.request("mcpServerStatus/list", json!({"threadId":thread,"detail":"toolsAndAuthOnly","limit":64,"cursor":cursor})).await?;
                    native::mcp(result["data"].as_array().ok_or("native_extension_InvalidFormat")?, true, &mut out);
                    if result.get("nextCursor").is_some_and(Value::is_null) { return Ok(out); }
                    let Some(next) = result["nextCursor"].as_str() else { out.incomplete = true; return Ok(out); };
                    if next.is_empty() || next.len() > 4096 || !cursors.insert(next.to_owned()) { out.incomplete = true; return Ok(out); }
                    cursor = json!(next);
                }
                out.incomplete = true;
            }
        }
        Ok(out)
    }
    pub(crate) async fn native_action(&self, action: &crate::native_session::NativeSessionAction) -> Result<crate::native_session::NativeActionState, String> {
        if !self.native_protocol_supported || !self.native_available() { return Err("native_extension_Unsupported".into()); }
        if !matches!(action, crate::native_session::NativeSessionAction::ReloadSkills {}) { return Err("native_extension_Unsupported".into()); }
        let workspace = self.snapshot().await.workspace;
        let result = self.wire.request("skills/list", json!({"cwds":[workspace],"forceReload":true})).await?;
        let entries = result["data"].as_array().ok_or("result_unknown")?;
        if entries.len() != 1 || entries.iter().any(|entry| !entry["skills"].is_array() || entry["errors"].as_array().is_none_or(|errors| !errors.is_empty())) { return Err("result_unknown".into()); }
        Ok(crate::native_session::NativeActionState::Applied)
    }
    pub async fn close(&self) -> bool {
        let closed = self.wire.close().await;
        if closed { self.view.lock().await.status = "closed".into(); }
        closed
    }
    pub async fn wait_stopped(&self) -> bool {
        self.wire.wait_stopped().await
    }
    pub(crate) async fn open_provider(options: SessionOptions, providers: &dgos_providers::ProviderManager, launch: &crate::capabilities::ProviderLaunch) -> Result<Self, crate::transport::StartupError> {
        let diagnostic = ProcessDiagnostic::new("agent_start", "codex");
        let probe = crate::cli::probe(crate::cli::CliTool::Codex, options.executable.as_deref()).await;
        let executable = probe.path.filter(|_| probe.status == "available").ok_or("cli_incompatible")?;
        if probe.version.as_deref() != Some("codex-cli 0.167.0") { return Err("cli_incompatible".into()); }
        if launch.env.get("DGOS_PROVIDER_KEY").is_none_or(|key| key.is_empty()) { return Err("provider_incompatible".into()); }
        let mut command = Command::new(executable);
        command.args(["app-server", "--stdio"]).current_dir(&options.workspace);
        for name in ["OPENAI_API_KEY", "OPENAI_BASE_URL", "OPENAI_ORGANIZATION", "OPENAI_PROJECT", "AZURE_OPENAI_API_KEY", "CODEX_API_KEY"] { command.env_remove(name); }
        command.envs(&launch.env);
        for value in &launch.codex_overrides { command.arg("-c").arg(value); }
        let process_lease = launch.lease.clone();
        let (wire, mut events) = providers.with_cli_spawn(&launch.lease, &launch.launch, |_| {
            Transport::spawn(command, process_lease.cancellation_token(), diagnostic, process_lease.clone()).map_err(|_| dgos_types::DgosError::ProviderRequestRejected)
        }).await.map_err(|_| "provider_revision_changed")?;
        let session = Self {
            wire,
            view: Arc::new(Mutex::new(SessionView::new(options.workspace.clone()))),
            receipts: Arc::default(),
            turn: Arc::new(Mutex::new(TurnState { opening: true, ..Default::default() })),
            native_protocol_supported: probe.version.as_deref() == Some("codex-cli 0.167.0"),
        };
        let reader = session.clone();
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                reader.event(event).await;
            }
        });
        let setup = async {
            session.wire.startup_request("handshake_initialize", "initialize", json!({"clientInfo":{"name":"dgos","version":"0.1.0"},"capabilities":{"experimentalApi":true}})).await?;
            session.wire.startup_write(json!({"method":"initialized","params":{}})).await?;
            let read = session.wire.startup_request("handshake_config_read", "config/read", json!({"includeLayers":false,"cwd":options.workspace})).await?;
            verify_provider_config(&read, &launch.launch.metadata().base_url)?;
            let method = if options.resume_id.is_some() { "thread/resume" } else { "thread/start" };
            let mut params = json!({"cwd":options.workspace,"approvalPolicy":"on-request","sandbox":"workspace-write","model":launch.model,"modelProvider":"dgos_project"});
            if let Some(id) = &options.resume_id {
                params["threadId"] = json!(id);
                params["excludeTurns"] = json!(true);
                params["initialTurnsPage"] = json!({"limit":128,"sortDirection":"desc","itemsView":"full"});
            }
            let result = session.wire.startup_request("handshake_thread", method, params).await?;
            if result.get("model").and_then(Value::as_str) != Some(launch.model.as_str()) || result.get("modelProvider").and_then(Value::as_str) != Some("dgos_project") { return Err("provider_incompatible".into()); }
            providers.validate_cli_lease(&launch.lease).await.map_err(|_| "provider_revision_changed")?;
            let id = result.pointer("/thread/id").and_then(Value::as_str).ok_or("invalid_output")?;
            if id.is_empty() || options.resume_id.as_deref().is_some_and(|expected| expected != id) { return Err("invalid_output".into()); }
            let cwd = result.get("cwd").and_then(Value::as_str).ok_or("provider_binding_unconfirmed")?;
            if tokio::fs::canonicalize(cwd).await.map_err(|_| "provider_binding_unconfirmed")? != tokio::fs::canonicalize(&options.workspace).await.map_err(|_| "provider_binding_unconfirmed")? { return Err("provider_binding_unconfirmed".into()); }
            {
                let mut view = session.view.lock().await;
                if view.status != "starting" || !session.wire.native_available() { return Err("result_unknown".into()); }
                view.native_id = Some(id.into());
            }
            session.restore_history(id, &result).await;
            // Hydration can await multiple replies. Reconfirm the exact lease
            // after them, before publishing this backend to the manager generation.
            providers.validate_cli_lease(&launch.lease).await.map_err(|_| "provider_revision_changed")?;
            let mut view = session.view.lock().await;
            if view.status != "starting" || !session.wire.native_available() { return Err("result_unknown".into()); }
            view.status = "idle".into();
            let mut turn = session.turn.lock().await;
            turn.opening = false;
            turn.deferred_bytes = 0;
            let replies: Vec<_> = std::mem::take(&mut turn.deferred).into_iter().filter_map(|event| Self::apply_event(&mut view, &mut turn, event)).collect();
            if view.status == "unknown" || view.status == "closed" { return Err("result_unknown".into()); }
            drop(turn);
            drop(view);
            for reply in replies { session.wire.startup_write(reply).await?; }
            Ok::<(),String>(())
        }.await;
        session.wire.startup_result(setup.as_ref().err().map(String::as_str));
        if let Err(error) = setup {
            return Err(session.wire.startup_failure(error).await);
        }
        Ok(session)
    }
    async fn restore_history(&self, thread: &str, result: &Value) {
        self.restore_history_until(thread, result, tokio::time::Instant::now() + history::TIME_BUDGET).await;
    }
    async fn restore_history_until(&self, thread: &str, result: &Value, deadline: tokio::time::Instant) {
        let mut history = history::History::default();
        history.seed(&mut *self.view.lock().await, result);
        let mut count = 0;
        while let Some(page) = history.pages.pop_front() {
            if history.exhausted || count >= history::MAX_PAGES || tokio::time::Instant::now() >= deadline {
                self.view.lock().await.history_truncated = true;
                break;
            }
            {
                let view = self.view.lock().await;
                if view.status != "starting" || view.native_id.as_deref() != Some(thread) || !self.wire.native_available() {
                    drop(view);
                    self.view.lock().await.history_truncated = true;
                    break;
                }
            }
            count += 1;
            let reply = tokio::time::timeout_at(deadline, self.wire.startup_request("handshake_history", page.method, page.params(thread))).await;
            let mut view = self.view.lock().await;
            if view.status != "starting" || view.native_id.as_deref() != Some(thread) || !self.wire.native_available() {
                view.history_truncated = true;
                break;
            }
            match reply {
                Ok(Ok(result)) => history.page(&mut view, &page, &result),
                _ => view.history_truncated = true,
            }
        }
        history.finish(&mut *self.view.lock().await);
    }
    fn completed_item(view: &mut SessionView, item: &Value, echo: &mut bool) {
        if item.get("type").and_then(Value::as_str) != Some("userMessage") || !std::mem::take(echo) {
            Self::item(view, item);
        }
    }
    fn item(view: &mut SessionView, item: &Value) {
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("item");
        let text = item.get("text").and_then(Value::as_str).map(str::to_owned);
        let data = item.clone();
        view.push(kind, text, data);
    }
}


fn native_error(_: &Value) -> String { "engine_failed".into() }

fn event_size(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Array(values) => values.iter().map(event_size).sum(),
        Value::Object(values) => values.iter().map(|(key, value)| key.len() + event_size(value)).sum(),
        _ => 16,
    }
}

fn verify_provider_config(read: &Value, base_url: &str) -> Result<(), String> {
    let config = read.get("config").ok_or("provider_incompatible")?;
    let provider = &config["model_providers"]["dgos_project"];
    if config.get("model_provider").and_then(Value::as_str) != Some("dgos_project")
        || provider.get("base_url").and_then(Value::as_str) != Some(base_url)
        || provider.get("wire_api").and_then(Value::as_str) != Some("responses")
        || provider.get("env_key").and_then(Value::as_str) != Some("DGOS_PROVIDER_KEY")
        || provider.get("requires_openai_auth").and_then(Value::as_bool) != Some(false)
        || ["http_headers", "env_http_headers", "experimental_bearer_token", "experimental_bearer_token_command"].iter().any(|key| provider.get(key).is_some_and(|v| !v.is_null() && v.as_object().is_none_or(|o| !o.is_empty())))
    { return Err("provider_incompatible".into()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running() -> (SessionView, TurnState) {
        let mut view = SessionView::new("/tmp".into());
        view.native_id = Some("thread".into());
        view.status = "running".into();
        view.active_run = Some("second-run".into());
        (view, TurnState { id: Some("second-turn".into()), ..Default::default() })
    }

    #[test]
    fn late_or_foreign_events_cannot_finish_or_write_current_run() {
        let (mut view, mut turn) = running();
        for (thread, turn_id) in [("thread", "old-turn"), ("other-thread", "second-turn")] {
            for event in [
                json!({"method":"turn/started","params":{"threadId":thread,"turn":{"id":turn_id}}}),
                json!({"method":"turn/completed","params":{"threadId":thread,"turn":{"id":turn_id,"status":"completed"}}}),
                json!({"method":"item/agentMessage/delta","params":{"threadId":thread,"turnId":turn_id,"itemId":"message","delta":"late"}}),
                json!({"method":"item/completed","params":{"threadId":thread,"turnId":turn_id,"item":{"type":"userMessage","content":[]}}}),
            ] { CodexSession::apply_event(&mut view, &mut turn, event); }
        }
        assert_eq!(view.status, "running");
        assert_eq!(turn.id.as_deref(), Some("second-turn"));
        assert!(view.events.is_empty());
        CodexSession::apply_event(&mut view, &mut turn, json!({"method":"item/agentMessage/delta","params":{"threadId":"thread","turnId":"second-turn","itemId":"message","delta":"current"}}));
        CodexSession::apply_event(&mut view, &mut turn, json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"second-turn","status":"completed"}}}));
        assert_eq!(view.status, "idle");
        assert_eq!(view.events.len(), 2);
        assert!(view.events.iter().all(|event| event.run.as_deref() == Some("second-run")));
        assert!(view.active_run.is_none());
    }

    #[test]
    fn approval_identity_preserves_original_request_and_rejects_replacement() {
        let (mut view, mut turn) = running();
        let event = json!({"id":7,"method":"item/commandExecution/requestApproval","params":{"threadId":"thread","turnId":"second-turn","itemId":"command","command":"original","availableDecisions":["accept","decline"]}});
        CodexSession::apply_event(&mut view, &mut turn, event.clone());
        let public_id = view.pending[0].id.clone();
        assert!(uuid::Uuid::parse_str(&public_id).is_ok());
        CodexSession::apply_event(&mut view, &mut turn, event.clone());
        assert_eq!(view.pending.len(), 1);
        assert_eq!(view.pending[0].id, public_id);
        assert!(!CodexSession::valid_response(&view.pending[0], &json!({"decision":"accept","command":"replacement"})));
        turn.requests.get_mut("7").unwrap().answered = true;
        view.pending.clear();
        CodexSession::apply_event(&mut view, &mut turn, event.clone());
        assert!(view.pending.is_empty());
        let mut replacement = event;
        replacement["params"]["command"] = json!("replacement");
        CodexSession::apply_event(&mut view, &mut turn, replacement);
        assert_eq!(view.status, "unknown");
        assert_eq!(turn.requests["7"].params["command"], "original");
        CodexSession::apply_event(&mut view, &mut turn, json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"second-turn","status":"completed"}}}));
        assert_eq!(view.status, "unknown", "a terminal event must not undo an approval identity conflict");
        let (mut restored, mut restored_turn) = running();
        CodexSession::apply_event(&mut restored, &mut restored_turn, json!({"id":7,"method":"item/commandExecution/requestApproval","params":{"threadId":"thread","turnId":"second-turn","command":"original"}}));
        assert_ne!(restored.pending[0].id, public_id);
    }

    #[test]
    fn structured_approval_must_equal_a_native_offered_decision() {
        let offered = json!({"acceptWithExecpolicyAmendment":{"execpolicyAmendment":["echo","allowed"]}});
        let pending = PendingRequest { id: "request".into(), kind: "item/commandExecution/requestApproval".into(), title: String::new(), data: json!({"availableDecisions":[offered.clone(),"decline"]}) };
        assert!(CodexSession::valid_response(&pending, &json!({"decision":offered})));
        assert!(!CodexSession::valid_response(&pending, &json!({"decision":{"acceptWithExecpolicyAmendment":{"execpolicyAmendment":["different"]}}})));
        assert!(!CodexSession::valid_response(&pending, &json!({"decision":"accept"})));
    }

    #[test]
    fn user_input_preserves_partial_or_cancel_answers_and_rejects_replacement_fields() {
        let pending = PendingRequest { id: "request".into(), kind: "tool/requestUserInput".into(), title: String::new(), data: json!({"questions":[{"id":"a"},{"id":"b"}]}) };
        assert!(CodexSession::valid_response(&pending, &json!({"answers":{}})));
        assert!(!CodexSession::valid_response(&pending, &json!({"answers":{"a":{"answers":["yes"]},"other":{"answers":["no"]}}})));
        assert!(!CodexSession::valid_response(&pending, &json!({"answers":{"a":{"answers":["yes"],"command":"replacement"},"b":{"answers":[]}}})));
        assert!(CodexSession::valid_response(&pending, &json!({"answers":{"a":{"answers":["yes"]},"b":{"answers":[]}}})));
    }

    #[test]
    fn suppresses_only_current_echo_and_preserves_repeated_history() {
        let mut view = SessionView::new("/tmp".into());
        let item = json!({"type":"userMessage","content":[{"type":"text","text":"same prompt"}]});
        CodexSession::item(&mut view, &item);
        CodexSession::item(&mut view, &item);
        for run in ["first", "second"] {
            view.active_run = Some(run.into());
            view.push("userMessage", Some("same prompt".into()), Value::Null);
            let mut echo = true;
            CodexSession::completed_item(&mut view, &item, &mut echo);
            assert!(!echo);
        }
        assert_eq!(view.events.len(), 4);
        assert_eq!(view.events[2].run.as_deref(), Some("first"));
        assert_eq!(view.events[3].run.as_deref(), Some("second"));
    }

    #[test]
    fn preserves_command_failure_and_native_error() {
        let mut view = SessionView::new("/tmp".into());
        let item = json!({"type":"commandExecution","status":"failed","command":"false","exitCode":1,"aggregatedOutput":"failure output","error":{"message":"command failed"}});
        CodexSession::item(&mut view, &item);
        assert_eq!(view.events[0].data, item);
    }

}

#[cfg(test)]
#[path = "codex_history_tests.rs"]
mod history_tests;
