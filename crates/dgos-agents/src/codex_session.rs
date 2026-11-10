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
#[path = "codex_operations_session.rs"]
mod operations;

type SendReceipt = ([u8; 32], Option<Result<(), String>>);
type SteerReceipt = ([u8; 32], Result<crate::session::SteeringReceipt, String>);

fn steering_fingerprint(run_id: &str, text: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update((run_id.len() as u64).to_le_bytes());
    hash.update(run_id.as_bytes());
    hash.update(text.as_bytes());
    hash.finalize().into()
}

fn steering_result(request_id: String, run_id: String, expected: &str, reply: Result<Value, String>) -> Result<crate::session::SteeringReceipt, String> {
    use crate::session::{SteeringReceipt, SteeringState};
    let state = match reply {
        Ok(value) if value["turnId"].as_str() == Some(expected) => SteeringState::Applied,
        Err(error) if error == "method_unsupported" => return Err("steering_unsupported".into()),
        Err(error) if error == "engine_rejected" => return Err(error),
        _ => SteeringState::Unknown,
    };
    Ok(SteeringReceipt { request_id, run_id, state })
}

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
    steering_echoes: usize,
    deferred_bytes: usize,
    requests: HashMap<String, NativeRequest>,
    operations: operations::OperationsState,
    operation_binding: bool,
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
    steering_receipts: Arc<Mutex<HashMap<String, SteerReceipt>>>,
    pending_steering: Arc<std::sync::atomic::AtomicUsize>,
    turn: Arc<Mutex<TurnState>>,
    unsupported_observations: Arc<std::sync::atomic::AtomicU8>,
    auth_launch: Option<Arc<crate::manager::native_auth::LaunchContext>>,
}
impl CodexSession {
    async fn event(&self, mut event: Value) {
        crate::images::redact_image_inputs(&mut event);
        let mut view = self.view.lock().await;
        let mut turn = self.turn.lock().await;
        // Startup must fail promptly, but a turn ACK already received by the
        // transport must bind queued output before closure invalidates the turn.
        if event.get("method").and_then(Value::as_str) == Some("host/closed") {
            if turn.starting {
                turn.deferred.push(event);
                return;
            }
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
        if let Some(root) = view.native_id.as_deref() { turn.operations.event(root, &event); }
        // Compact has an empty ACK; its first native started event supplies the
        // identity. Never bind an arbitrary item or a terminal-only stale event.
        if turn.operation_binding && turn.id.is_none() && method == "turn/started" && Self::matches_thread(view, &params) {
            if let Some(id) = params.pointer("/turn/id").and_then(Value::as_str).filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control) && !turn.operations.has_seen_turn(id)) {
                turn.id = Some(id.into());
                turn.operation_binding = false;
            }
        }
        if method == "turn/started" && Self::matches_thread(view, &params) { operations::bind_automatic(view, turn, &params); }
        if let Some(id) = event.get("id") {
            if !matches!(method, "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" | "item/permissions/requestApproval" | "item/tool/requestUserInput" | "mcpServer/elicitation/request") {
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
                // Session-scoped MCP elicitation is not owned by this turn. Its
                // request remains answerable until native resolution or closure.
                view.pending.retain(|pending| session != "unknown" && pending.kind == "mcpServer/elicitation/request" && pending.data.get("turnId").is_none_or(Value::is_null));
                view.status = if session == "idle" && !view.pending.is_empty() { "waiting" } else { session }.into();
                view.error = error;
                let mut terminal = params.clone();
                if let Some(items) = terminal.pointer_mut("/turn/items").and_then(Value::as_array_mut) {
                    for item in items.iter_mut().filter(|item| item["type"] == "reasoning") {
                        *item = json!({"type":"reasoning","id":item.get("id"),"summary":item.get("summary")});
                    }
                }
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
            "item/reasoning/summaryTextDelta" | "item/reasoning/summaryPartAdded" => {
                view.push(method, params.get("delta").and_then(Value::as_str).map(str::to_owned), params.clone());
            }
            "item/started" => {
                if let Some(item) = params.get("item").filter(|item| !matches!(item.get("type").and_then(Value::as_str), Some("userMessage" | "agentMessage"))) {
                    let data = if item["type"] == "reasoning" { json!({"type":"reasoning","id":item.get("id"),"summary":item.get("summary")}) } else { item.clone() };
                    view.push(method, None, data);
                }
            }
            "item/commandExecution/outputDelta" | "item/fileChange/outputDelta" => {
                view.push(method, params.get("delta").and_then(Value::as_str).map(str::to_owned), params.clone());
            }
            "item/completed" => {
                if let Some(item) = params.get("item") {
                    if item["type"] == "userMessage" && !turn.user_echo_pending && turn.steering_echoes > 0 {
                        turn.steering_echoes -= 1;
                    } else {
                        Self::completed_item(view, item, &mut turn.user_echo_pending);
                    }
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
            "thread/tokenUsage/updated" if Self::matches_thread(view, &params) => {
                view.push(method, None, params);
            }
            "host/closed" => {
                view.status = if view.status == "closed" {
                    "closed"
                } else {
                    "unknown"
                }
                .into();
                if let Some(controls) = &mut view.controls { controls.uncertain(); }
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
    pub async fn send(&self, request_id: String, text: String, settings: Option<crate::session::AgentTurnSettings>, skills: Vec<crate::session::AgentSkillInput>, images: Vec<crate::images::ValidatedImage>) -> Result<(), String> {
        let settings = settings.filter(|settings| !settings.is_empty());
        if (text.trim().is_empty() && images.is_empty()) || text.len() > 65536 {
            return Err("invalid_prompt".into());
        }
        crate::session::validate_skill_inputs(&skills)?;
        let mut hash = Sha256::new();
        // Prompt bytes may contain the delimiter and settings JSON verbatim.
        hash.update((text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
        if let Some(settings) = &settings { hash.update([0]); hash.update(serde_json::to_vec(settings).map_err(|_| "invalid_turn_settings")?); }
        if !skills.is_empty() { hash.update(b"\0DG-OS skills\0"); hash.update(serde_json::to_vec(&skills).map_err(|_| "agent_skill_invalid")?); }
        for image in &images { hash.update(image.fingerprint()); }
        let fingerprint: [u8; 32] = hash.finalize().into();
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
        let (native, overrides) = {
            let mut view = self.view.lock().await;
            if view.status != "idle" || !self.wire.native_available() { return Err("busy".into()); }
            let native = view.native_id.clone().ok_or("closed")?;
            let overrides = turn_overrides(view.controls.as_ref(), settings.as_ref())?;
            let mut turn = self.turn.lock().await;
            turn.id = None;
            turn.starting = true;
            turn.deferred.clear();
            turn.deferred_bytes = 0;
            turn.requests.retain(|_, request| request.kind == "mcpServer/elicitation/request" && request.params.get("turnId").is_none_or(Value::is_null));
            turn.user_echo_pending = true;
            turn.steering_echoes = 0;
            view.status = "running".into();
            view.active_run = Some(request_id.clone());
            view.error = None;
            let mut data = serde_json::Map::new();
            if !skills.is_empty() { data.insert("skills".into(), json!(skills)); }
            if !images.is_empty() { data.insert("images".into(), json!(images.iter().map(crate::images::ValidatedImage::metadata).collect::<Vec<_>>())); }
            view.push("userMessage", Some(text.clone()), if data.is_empty() { Value::Null } else { Value::Object(data) });
            (native, overrides)
        };
        receipts.insert(request_id.clone(), (fingerprint, None));
        drop(receipts);
        let mut input = Vec::with_capacity(1 + skills.len() + images.len());
        if !text.trim().is_empty() { input.push(json!({"type":"text","text":text})); }
        input.extend(images.iter().map(|image| image.native_input()));
        input.extend(skills.iter().map(|skill| json!({"type":"skill","name":skill.name,"path":skill.path})));
        let mut params = json!({"threadId":native,"input":input});
        if let Some(overrides) = overrides { params.as_object_mut().unwrap().extend(overrides); }
        let result = self.wire.request("turn/start", params).await
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
                if let (Some(controls), Some(settings)) = (&mut view.controls, &settings) {
                    if let Some(effort) = &settings.reasoning_effort {
                        controls.reasoning.current = effort.clone().or_else(|| controls.default_effort.clone());
                        controls.reasoning.source = "codex/turn_start".into();
                        controls.reasoning.reason = None;
                    }
                    if let Some(permission) = settings.permission_mode {
                        controls.permission.current = Some(permission);
                        controls.permission.source = "codex/turn_start".into();
                        controls.permission.reason = None;
                    }
                    if let (Some(mode), Some(control)) = (settings.collaboration_mode, &mut controls.collaboration) {
                        control.current = Some(mode.as_str().into());
                        control.source = "codex/turn_start".into();
                        control.reason = None;
                    }
                    if let Some(preferences) = &settings.response_preferences { controls.response_preferences = Some(preferences.clone()); }
                }
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
                if let Some(controls) = &mut view.controls { controls.uncertain(); }
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
        let planning = view.controls.as_ref().and_then(|controls| controls.collaboration.as_ref()).and_then(|control| control.current.as_deref()).is_some_and(|mode| matches!(mode, "plan" | "deep"));
        if (planning || view.controls.as_ref().and_then(|controls| controls.permission.current) == Some(crate::session::PermissionMode::ReadOnly))
            && !read_only_response(pending, &response) { return Err("invalid_response".into()); }
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
            "item/tool/requestUserInput" => response.as_object().is_some_and(|object| object.len() == 1) && response.get("answers").and_then(Value::as_object).is_some_and(|answers| {
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
    pub(crate) async fn steer(&self, run_id: String, request_id: String, text: String) -> Result<crate::session::SteeringReceipt, String> {
        use crate::session::{SteeringReceipt, SteeringState};
        if text.trim().is_empty() || text.len() > 65536 { return Err("invalid_prompt".into()); }
        let fingerprint = steering_fingerprint(&run_id, &text);
        let mut receipts = self.steering_receipts.lock().await;
        if let Some((old, receipt)) = receipts.get(&request_id) {
            return if old == &fingerprint { receipt.clone() } else { Err("request_conflict".into()) };
        }
        if receipts.len() >= 256 { return Err("request_capacity".into()); }
        let view = self.view.lock().await;
        if view.controls.as_ref().is_none_or(|controls| !controls.steering) || !self.wire.native_available() { return Err("steering_unsupported".into()); }
        let mut turn = self.turn.lock().await;
        if view.active_run.as_deref() != Some(run_id.as_str()) || !matches!(view.status.as_str(), "running" | "waiting") || turn.starting {
            return Err("not_running".into());
        }
        let native = view.native_id.clone().ok_or("session_closed")?;
        let expected = turn.id.clone().ok_or("not_running")?;
        // Reserve the only dispatch before IO; pending/unknown receipts are never replayed.
        receipts.insert(request_id.clone(), (fingerprint, Ok(SteeringReceipt { request_id: request_id.clone(), run_id: run_id.clone(), state: SteeringState::Unknown })));
        turn.steering_echoes += 1;
        self.pending_steering.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        drop(turn);
        drop(view);
        drop(receipts);
        let reply = self.wire.request("turn/steer", json!({"threadId":native,"expectedTurnId":expected,"input":[{"type":"text","text":text,"text_elements":[]}]})).await;
        let result = steering_result(request_id.clone(), run_id.clone(), &expected, reply);
        let mut view = self.view.lock().await;
        if result.as_ref().err().is_some_and(|error| error == "steering_unsupported") {
            if let Some(controls) = &mut view.controls { controls.steering = false; }
        }
        if let Ok(receipt) = &result {
            if receipt.state == SteeringState::Applied {
                view.push_run("userSteering", Some(text), json!({"requestId":request_id,"run":run_id}), Some(run_id.clone()));
            }
            view.push_run("steeringReceipt", None, json!({"state":receipt.state,"requestId":request_id,"run":run_id}), Some(run_id.clone()));
        }
        drop(view);
        self.steering_receipts.lock().await.insert(request_id, (fingerprint, result.clone()));
        self.pending_steering.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        result
    }
    pub async fn interrupt(&self, run_id: &str) -> Result<(), String> {
        let view = self.view.lock().await;
        if view.active_run.as_deref() != Some(run_id) || !matches!(view.status.as_str(), "running" | "waiting") { return Err("not_running".into()); }
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
    pub(crate) async fn snapshot_since(&self, after: Option<u64>) -> (SessionView, crate::session::EventSync) {
        self.view.lock().await.snapshot_since(after)
    }
    pub(crate) fn native_available(&self) -> bool { self.wire.native_available() }
    pub(crate) fn auth_launch(&self) -> Option<Arc<crate::manager::native_auth::LaunchContext>> {
        self.native_available().then(|| self.auth_launch.clone()).flatten()
    }
    pub(crate) async fn steering_settled(&self, after: u64) -> bool {
        let view = self.view.lock().await;
        self.pending_steering.load(std::sync::atomic::Ordering::Acquire) == 0
            && view.events.last().is_none_or(|event| event.seq <= after)
    }
    pub(crate) fn native_capabilities(&self) -> crate::native_session::NativeSessionCapabilities {
        let mut capabilities = crate::native_session::NativeSessionCapabilities {
            native_auth_guidance: self.auth_launch().is_none().then(|| crate::native_session::NativeAuthGuidance::installed(crate::Engine::Codex)),
            native_login: cfg!(unix) && self.auth_launch().is_some(),
            native_logout: cfg!(unix) && self.auth_launch().is_some(),
            mcp_control_guidance: Some(crate::native_session::NativeMcpControlGuidance::codex()),
            ..Default::default()
        };
        if self.native_available() {
            let unsupported = self.unsupported_observations.load(std::sync::atomic::Ordering::Acquire);
            capabilities.observe_skills = unsupported & 1 == 0;
            capabilities.observe_mcp = unsupported & 2 == 0;
            capabilities.reload_skills = capabilities.observe_skills;
            capabilities.native_login &= capabilities.observe_mcp;
            capabilities.native_logout &= capabilities.observe_mcp;
        }
        capabilities
    }
    async fn observe_request(&self, method: &str, params: Value, capability: u8) -> Result<Value, String> {
        if self.unsupported_observations.load(std::sync::atomic::Ordering::Acquire) & capability != 0 {
            return Err("native_extension_Unsupported".into());
        }
        self.wire.request(method, params).await.map_err(|error| {
            if error == "method_unsupported" {
                self.unsupported_observations.fetch_or(capability, std::sync::atomic::Ordering::AcqRel);
                "native_extension_Unsupported".into()
            } else { error }
        })
    }
    pub(crate) async fn observe_native(&self, domain: crate::native_session::SessionDomain) -> Result<crate::native_session::Projection, String> {
        use crate::native_session::{self as native, SessionDomain};
        if !self.native_available() { return Err("native_extension_Unsupported".into()); }
        let view = self.snapshot().await;
        let mut out = native::Projection::default();
        match domain {
            SessionDomain::Skills => {
                let result = self.observe_request("skills/list", json!({"cwds":[view.workspace],"forceReload":false}), 1).await?;
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
                let mut finished = false;
                for _ in 0..4 {
                    let result = self.observe_request("mcpServerStatus/list", json!({"threadId":thread,"detail":"toolsAndAuthOnly","limit":64,"cursor":cursor}), 2).await?;
                    native::mcp(result["data"].as_array().ok_or("native_extension_InvalidFormat")?, true, &mut out);
                    if result.get("nextCursor").is_some_and(Value::is_null) { finished = true; break; }
                    let Some(next) = result["nextCursor"].as_str() else { out.incomplete = true; break; };
                    if next.is_empty() || next.len() > 4096 || !cursors.insert(next.to_owned()) { out.incomplete = true; break; }
                    cursor = json!(next);
                }
                out.incomplete |= !finished;
                native::mcp_call_evidence(&self.snapshot().await.events, &mut out);
            }
        }
        Ok(out)
    }
    pub(crate) async fn native_action(&self, action: &crate::native_session::NativeSessionAction) -> Result<crate::native_session::NativeActionState, String> {
        if !self.native_available() { return Err("native_extension_Unsupported".into()); }
        if !matches!(action, crate::native_session::NativeSessionAction::ReloadSkills {}) { return Err("native_extension_Unsupported".into()); }
        let workspace = self.snapshot().await.workspace;
        let result = self.observe_request("skills/list", json!({"cwds":[workspace],"forceReload":true}), 1).await?;
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
        if let Some(error) = probe.startup_error() { return Err(error.into()); }
        let executable = probe.path.as_deref().ok_or("cli_not_installed")?;
        if launch.env.get("DGOS_PROVIDER_KEY").is_none_or(|key| key.is_empty()) { return Err("provider_incompatible".into()); }
        let mut command = Command::new(executable);
        command.args(["app-server", "--stdio"]).current_dir(&options.workspace);
        for name in ["OPENAI_API_KEY", "OPENAI_BASE_URL", "OPENAI_ORGANIZATION", "OPENAI_PROJECT", "AZURE_OPENAI_API_KEY", "CODEX_API_KEY"] { command.env_remove(name); }
        command.envs(&launch.env);
        for value in &launch.codex_overrides { command.arg("-c").arg(value); }
        let mut auth_env: std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString> = std::env::vars_os().filter(|(key, _)| {
            let key = key.to_string_lossy();
            !key.starts_with("DGOS_PROVIDER_") && !key.starts_with("OPENAI_") && !key.starts_with("AZURE_OPENAI_") && key != "CODEX_API_KEY"
        }).collect();
        // MCP auth CLI uses the trusted executable/workspace and ambient config,
        // never the project's injected model-provider credential environment.
        auth_env.insert("NO_COLOR".into(), "1".into());
        let auth_launch = Arc::new(crate::manager::native_auth::LaunchContext { executable: executable.into(), workspace: options.workspace.clone(), env: auth_env });
        let process_lease = launch.lease.clone();
        let (wire, mut events) = providers.with_cli_spawn(&launch.lease, &launch.launch, |_| {
            Ok(Transport::spawn(command, process_lease.cancellation_token(), diagnostic, process_lease.clone()))
        }).await.map_err(|_| "provider_revision_changed")??;
        let session = Self {
            wire,
            view: Arc::new(Mutex::new({
                let mut view = SessionView::new(options.workspace.clone());
                view.cli_version = probe.version.clone();
                view
            })),
            receipts: Arc::default(),
            steering_receipts: Arc::default(),
            pending_steering: Arc::default(),
            turn: Arc::new(Mutex::new(TurnState { opening: true, ..Default::default() })),
            unsupported_observations: Arc::default(),
            auth_launch: Some(auth_launch),
        };
        let reader = session.clone();
        tokio::spawn(async move {
            // Transport drops the sender before reaping. Drain accepted events
            // in order; cancellation must not overtake output already received.
            while let Some(event) = events.recv().await {
                reader.event(event).await;
            }
            reader.event(json!({"method":"host/closed"})).await;
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
                params["initialTurnsPage"] = json!({"limit":128,"sortDirection":"desc","itemsView":"notLoaded"});
            }
            let mut result = session.wire.startup_request("handshake_thread", method, params).await?;
            crate::images::redact_image_inputs(&mut result);
            if result.get("model").and_then(Value::as_str) != Some(launch.model.as_str()) || result.get("modelProvider").and_then(Value::as_str) != Some("dgos_project") { return Err("provider_incompatible".into()); }
            providers.validate_cli_lease(&launch.lease).await.map_err(|_| "provider_revision_changed")?;
            let id = result.pointer("/thread/id").and_then(Value::as_str).ok_or("invalid_output")?;
            if id.is_empty() || options.resume_id.as_deref().is_some_and(|expected| expected != id) { return Err("invalid_output".into()); }
            let cwd = result.get("cwd").and_then(Value::as_str).ok_or("provider_binding_unconfirmed")?;
            if tokio::fs::canonicalize(cwd).await.map_err(|_| "provider_binding_unconfirmed")? != tokio::fs::canonicalize(&options.workspace).await.map_err(|_| "provider_binding_unconfirmed")? { return Err("provider_binding_unconfirmed".into()); }
            // Model metadata is control-only: never start a prompt to discover capabilities.
            let mut model = None;
            let mut cursor = Value::Null;
            for _ in 0..8 {
                let listed = session.wire.startup_request("handshake_models", "model/list", json!({"limit":100,"includeHidden":true,"cursor":cursor})).await;
                let Ok(listed) = listed else { break; };
                model = listed["data"].as_array().and_then(|rows| rows.iter().find(|row| row["model"].as_str() == Some(launch.model.as_str()))).cloned();
                if model.is_some() { break; }
                cursor = listed.get("nextCursor").cloned().unwrap_or(Value::Null);
                if cursor.is_null() { break; }
            }
            // Optional experimental metadata must never prevent a usable session.
            let modes = tokio::time::timeout(std::time::Duration::from_secs(2), session.wire.startup_request("handshake_collaboration", "collaborationMode/list", json!({}))).await.ok().and_then(Result::ok);
            {
                let mut view = session.view.lock().await;
                if view.status != "starting" || !session.wire.native_available() { return Err("result_unknown".into()); }
                view.native_id = Some(id.into());
                let mut controls = codex_controls(&result, model.as_ref());
                controls.collaboration = collaboration_control(modes.as_ref());
                view.controls = Some(controls);
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
                Ok(Ok(mut result)) => {
                    crate::images::redact_image_inputs(&mut result);
                    history.page(&mut view, &page, &result);
                },
                _ => view.history_truncated = true,
            }
        }
        let mut view = self.view.lock().await;
        history.finish(&mut view);
        let mut turn = self.turn.lock().await;
        for event in &view.events { turn.operations.item(thread, thread, &event.data); }
    }
    fn completed_item(view: &mut SessionView, item: &Value, echo: &mut bool) {
        if item.get("type").and_then(Value::as_str) != Some("userMessage") || !std::mem::take(echo) {
            Self::item(view, item);
        }
    }
    fn item(view: &mut SessionView, item: &Value) {
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("item");
        let text = (kind != "reasoning").then(|| item.get("text").and_then(Value::as_str).map(str::to_owned)).flatten();
        let data = if kind == "reasoning" { json!({"type":"reasoning","id":item.get("id"),"summary":item.get("summary")}) } else { item.clone() };
        view.push(kind, text, data);
    }
}

fn codex_controls(result: &Value, model: Option<&Value>) -> crate::session::SessionControls {
    use crate::session::{PermissionMode, SessionControl, SessionControls};
    let options: Vec<String> = model.and_then(|row| row["supportedReasoningEfforts"].as_array()).into_iter().flatten()
        .filter_map(|option| option["reasoningEffort"].as_str())
        .filter(|value| !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control))
        .map(str::to_owned).collect();
    let default_effort = model.and_then(|row| row["defaultReasoningEffort"].as_str()).filter(|value| options.iter().any(|option| option == value)).map(str::to_owned);
    let workspace_sandbox = result.pointer("/sandbox/type").and_then(Value::as_str) == Some("workspaceWrite");
    let read_only_sandbox = result.pointer("/sandbox/type").and_then(Value::as_str) == Some("readOnly");
    let permission_available = (result["approvalPolicy"] == "on-request" || result["approvalPolicy"] == "never") && (workspace_sandbox || read_only_sandbox)
        && result.get("approvalsReviewer").is_none_or(|value| value == "user");
    SessionControls {
        reasoning: SessionControl {
            available: !options.is_empty() && default_effort.is_some(), options,
            current: result["reasoningEffort"].as_str().map(str::to_owned), source: "codex/model_list+thread".into(),
            reason: default_effort.is_none().then(|| "bound_model_metadata_unavailable".into()),
        },
        permission: SessionControl {
            available: permission_available,
            options: if permission_available && workspace_sandbox { vec![PermissionMode::Default, PermissionMode::ReadOnly, PermissionMode::FullAuto] } else if permission_available { vec![PermissionMode::ReadOnly] } else { vec![] },
            current: permission_available.then(|| if result["approvalPolicy"] == "never" && workspace_sandbox { PermissionMode::FullAuto } else if read_only_sandbox { PermissionMode::ReadOnly } else { PermissionMode::Default }), source: "codex/thread".into(),
            reason: (!permission_available).then(|| "native_policy_unavailable".into()),
        },
        collaboration: None,
        steering: true,
        bound_model: result["model"].as_str().map(str::to_owned),
        default_effort,
        default_sandbox: workspace_sandbox.then(|| result["sandbox"].clone()),
        response_preferences: None,
    }
}

fn collaboration_control(listed: Option<&Value>) -> Option<crate::session::SessionControl<String>> {
    let rows = listed?.get("data")?.as_array()?;
    let options: Vec<String> = ["default", "plan", "deep"].into_iter()
        .filter(|mode| rows.iter().any(|row| row["mode"].as_str() == Some(if *mode == "deep" { "plan" } else { *mode })))
        .map(str::to_owned).collect();
    if options.is_empty() { return None; }
    Some(crate::session::SessionControl {
        // A preset catalog does not report the active mode of a resumed thread.
        available: true, current: None,
        options, source: "codex/collaboration_mode_list".into(), reason: None,
    })
}

fn turn_overrides(controls: Option<&crate::session::SessionControls>, settings: Option<&crate::session::AgentTurnSettings>) -> Result<Option<serde_json::Map<String, Value>>, String> {
    use crate::session::{CollaborationMode, PermissionMode};
    let Some(settings) = settings.filter(|settings| !settings.is_empty()) else { return Ok(None); };
    let controls = controls.ok_or("turn_controls_unavailable")?;
    controls.validate(settings)?;
    let mut params = serde_json::Map::new();
    if let Some(effort) = &settings.reasoning_effort {
        // Native null preserves sticky effort. Resolve UI default from the exact model row instead.
        let effort = effort.as_ref().or(controls.default_effort.as_ref()).ok_or("unsupported_reasoning_effort")?;
        params.insert("effort".into(), json!(effort));
    }
    let mode = settings.collaboration_mode.or_else(|| controls.collaboration.as_ref()?.current.as_deref().and_then(|mode| match mode {
        "default" => Some(CollaborationMode::Default), "plan" => Some(CollaborationMode::Plan), "deep" => Some(CollaborationMode::Deep), _ => None,
    }));
    if settings.permission_mode.is_some() || settings.collaboration_mode.is_some() || mode.is_some_and(CollaborationMode::planning) {
        let permission = settings.permission_mode.or(controls.permission.current).unwrap_or(PermissionMode::Default);
        let policy = permissions::turn_policy(permission, mode.is_some_and(CollaborationMode::planning), controls.default_sandbox.as_ref())?;
        params.extend(policy.as_object().unwrap().clone());
    }
    if mode.is_some() || settings.response_preferences.is_some() {
        let mode = mode.unwrap_or(CollaborationMode::Default);
        let effort = match &settings.reasoning_effort {
            Some(effort) => effort.as_ref().or(controls.default_effort.as_ref()),
            None => controls.reasoning.current.as_ref().or(controls.default_effort.as_ref()),
        };
        let mut instructions = Vec::new();
        if mode == CollaborationMode::Deep {
            instructions.push("Planning workflow: clarify the user's goals and unresolved constraints first; compare viable approaches and their tradeoffs; then produce a concrete plan and wait for the user to choose execution mode before implementation.".to_owned());
        }
        if let Some(preferences) = settings.response_preferences.as_ref().or(controls.response_preferences.as_ref()) { instructions.push(preferences.instructions()); }
        let instructions = (!instructions.is_empty()).then(|| instructions.join("\n"));
        params.insert("collaborationMode".into(), json!({"mode":if mode.planning() { "plan" } else { "default" },"settings":{
            "model":controls.bound_model.as_deref().ok_or("unsupported_collaboration_mode")?,
            "reasoning_effort":effort,"developer_instructions":instructions
        }}));
    }
    Ok(Some(params))
}

fn read_only_response(pending: &PendingRequest, response: &Value) -> bool {
    match pending.kind.as_str() {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => matches!(response["decision"].as_str(), Some("decline" | "cancel")),
        "item/permissions/requestApproval" => response["scope"] == "turn" && response["permissions"].as_object().is_some_and(|permissions| permissions.is_empty()),
        _ => true,
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
    fn turn_settings_preserve_defaults_and_never_broaden_permissions() {
        use crate::session::{AgentTurnSettings, PermissionMode};
        let original = json!({"approvalPolicy":"on-request","approvalsReviewer":"user","sandbox":{"type":"workspaceWrite","networkAccess":false,"writableRoots":["/fixture"]},"reasoningEffort":"medium"});
        let model = json!({"defaultReasoningEffort":"medium","supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"medium"}]});
        let controls = codex_controls(&original, Some(&model));
        assert!(turn_overrides(Some(&controls), None).unwrap().is_none());
        let empty = AgentTurnSettings { reasoning_effort: None, permission_mode: None, collaboration_mode: None, response_preferences: None };
        assert!(turn_overrides(None, Some(&empty)).unwrap().is_none());
        let permission_only = turn_overrides(Some(&controls), Some(&AgentTurnSettings { reasoning_effort: None, permission_mode: Some(PermissionMode::ReadOnly), collaboration_mode: None, response_preferences: None })).unwrap().unwrap();
        assert!(!permission_only.contains_key("effort"));
        let settings = AgentTurnSettings { reasoning_effort: Some(None), permission_mode: Some(PermissionMode::Default), collaboration_mode: None, response_preferences: None };
        let params = turn_overrides(Some(&controls), Some(&settings)).unwrap().unwrap();
        assert_eq!(params["effort"], "medium");
        assert_eq!(params["sandboxPolicy"], original["sandbox"]);
        assert_eq!(params["approvalPolicy"], "on-request");
        for settings in [
            AgentTurnSettings { reasoning_effort: Some(Some("xhigh".into())), permission_mode: Some(PermissionMode::Default), collaboration_mode: None, response_preferences: None },
            AgentTurnSettings { reasoning_effort: None, permission_mode: Some(PermissionMode::AcceptEdits), collaboration_mode: None, response_preferences: None },
        ] { assert!(turn_overrides(Some(&controls), Some(&settings)).is_err()); }
        let unavailable = codex_controls(&original, None);
        assert!(!unavailable.reasoning.available);
        assert!(turn_overrides(Some(&unavailable), Some(&AgentTurnSettings { reasoning_effort: Some(Some("low".into())), permission_mode: None, collaboration_mode: None, response_preferences: None })).is_err());
        let unsafe_policy = codex_controls(&json!({"approvalPolicy":"never","sandbox":{"type":"dangerFullAccess"}}), Some(&model));
        assert!(!unsafe_policy.permission.available);
        let effort_only = turn_overrides(Some(&unsafe_policy), Some(&AgentTurnSettings { reasoning_effort: Some(Some("low".into())), permission_mode: None, collaboration_mode: None, response_preferences: None })).unwrap().unwrap();
        assert!(!effort_only.contains_key("approvalPolicy"));
        assert!(!effort_only.contains_key("sandboxPolicy"));
    }

    #[test]
    fn collaboration_modes_use_bound_model_builtin_instructions_and_selected_effort() {
        use crate::session::{AgentTurnSettings, CollaborationMode};
        let native = json!({"model":"bound-model","reasoningEffort":"medium","approvalPolicy":"on-request","sandbox":{"type":"workspaceWrite"}});
        let model = json!({"defaultReasoningEffort":"medium","supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"medium"}]});
        let mut controls = codex_controls(&native, Some(&model));
        assert!(collaboration_control(None).is_none());
        assert!(collaboration_control(Some(&json!({"data":[]}))).is_none());
        let mut settings = AgentTurnSettings { reasoning_effort: None, permission_mode: None, collaboration_mode: Some(CollaborationMode::Plan), response_preferences: None };
        assert_eq!(turn_overrides(Some(&controls), Some(&settings)).unwrap_err(), "unsupported_collaboration_mode");
        controls.collaboration = collaboration_control(Some(&json!({"data":[{"mode":"default","model":"injected"},{"mode":"plan","model":"other-model","reasoning_effort":"high"}]})));
        assert!(controls.collaboration.as_ref().unwrap().current.is_none());
        let params = turn_overrides(Some(&controls), Some(&settings)).unwrap().unwrap();
        assert_eq!(params["collaborationMode"], json!({"mode":"plan","settings":{"model":"bound-model","reasoning_effort":"medium","developer_instructions":null}}));
        assert_eq!(params["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(params["approvalPolicy"], "never");
        settings.reasoning_effort = Some(Some("low".into()));
        assert_eq!(turn_overrides(Some(&controls), Some(&settings)).unwrap().unwrap()["collaborationMode"]["settings"]["reasoning_effort"], "low");
        settings.reasoning_effort = Some(None);
        assert_eq!(turn_overrides(Some(&controls), Some(&settings)).unwrap().unwrap()["collaborationMode"]["settings"]["reasoning_effort"], "medium");
        controls.permission.current = Some(crate::session::PermissionMode::FullAuto);
        controls.collaboration.as_mut().unwrap().current = Some("deep".into());
        settings.collaboration_mode = Some(CollaborationMode::Deep);
        let deep = turn_overrides(Some(&controls), Some(&settings)).unwrap().unwrap();
        assert_eq!(deep["collaborationMode"]["mode"], "plan");
        assert!(deep["collaborationMode"]["settings"]["developer_instructions"].as_str().unwrap().contains("compare"));
        assert_eq!(deep["sandboxPolicy"]["type"], "readOnly");
        settings.collaboration_mode = Some(CollaborationMode::Default);
        let execute = turn_overrides(Some(&controls), Some(&settings)).unwrap().unwrap();
        assert_eq!(execute["sandboxPolicy"], native["sandbox"]);
        assert_eq!(execute["approvalPolicy"], "never");
    }

    #[test]
    fn steering_results_never_turn_ambiguity_into_success_or_run_failure() {
        use crate::session::SteeringState;
        assert_eq!(steering_result("request".into(), "run".into(), "turn", Ok(json!({"turnId":"turn"}))).unwrap().state, SteeringState::Applied);
        for reply in [Ok(json!({"turnId":"different"})), Ok(json!({})), Err("result_unknown".into()), Err("connection_lost".into())] {
            assert_eq!(steering_result("request".into(), "run".into(), "turn", reply).unwrap().state, SteeringState::Unknown);
        }
        assert_eq!(steering_result("request".into(), "run".into(), "turn", Err("engine_rejected".into())).unwrap_err(), "engine_rejected");
        assert_eq!(steering_result("request".into(), "run".into(), "turn", Err("method_unsupported".into())).unwrap_err(), "steering_unsupported");
        assert_ne!(steering_fingerprint("run-a", "text"), steering_fingerprint("run-b", "text"));
        assert_ne!(steering_fingerprint("run", "text"), steering_fingerprint("run", "changed"));
    }

    #[test]
    fn live_reasoning_forwards_summary_only_and_rejects_stale_turns() {
        let (mut view, mut turn) = running();
        for method in ["item/reasoning/summaryPartAdded", "item/reasoning/summaryTextDelta", "item/reasoning/textDelta"] {
            CodexSession::apply_event(&mut view, &mut turn, json!({"method":method,"params":{"threadId":"thread","turnId":"second-turn","itemId":"thought","summaryIndex":0,"delta":"summary"}}));
        }
        assert_eq!(view.events.len(), 2);
        CodexSession::apply_event(&mut view, &mut turn, json!({"method":"item/reasoning/summaryTextDelta","params":{"threadId":"thread","turnId":"old-turn","delta":"stale"}}));
        assert_eq!(view.events.len(), 2);
        CodexSession::apply_event(&mut view, &mut turn, json!({"method":"item/completed","params":{"threadId":"thread","turnId":"second-turn","item":{"id":"thought","type":"reasoning","summary":["safe summary"],"content":["private raw"]}}}));
        assert_eq!(view.events.last().unwrap().data, json!({"id":"thought","type":"reasoning","summary":["safe summary"]}));
        CodexSession::apply_event(&mut view, &mut turn, json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"second-turn","status":"completed","items":[{"id":"thought","type":"reasoning","summary":["safe summary"],"content":["private raw"]}]}}}));
        assert_eq!(view.events.last().unwrap().data["turn"]["items"][0], json!({"id":"thought","type":"reasoning","summary":["safe summary"]}));
    }

    #[test]
    fn read_only_turn_cannot_accept_a_native_sandbox_escape() {
        for kind in ["item/commandExecution/requestApproval", "item/fileChange/requestApproval"] {
            let pending = PendingRequest { id: "approval".into(), kind: kind.into(), title: kind.into(), data: Value::Null };
            for decision in ["accept", "acceptForSession"] { assert!(!read_only_response(&pending, &json!({"decision":decision}))); }
            for decision in ["decline", "cancel"] { assert!(read_only_response(&pending, &json!({"decision":decision}))); }
        }
        let pending = PendingRequest { id: "permissions".into(), kind: "item/permissions/requestApproval".into(), title: "permissions".into(), data: Value::Null };
        assert!(!read_only_response(&pending, &json!({"scope":"turn","permissions":{"fileSystem":{"write":["/"]}}})));
        assert!(read_only_response(&pending, &json!({"scope":"turn","permissions":{}})));
    }

    #[test]
    fn latest_native_token_usage_is_retained_without_replacing_last_with_total() {
        let (mut view, mut turn) = running();
        let params = json!({"threadId":"thread","turnId":"second-turn","tokenUsage":{"last":{"inputTokens":100,"outputTokens":20},"total":{"inputTokens":90000,"outputTokens":5000},"modelContextWindow":200000}});
        CodexSession::apply_event(&mut view, &mut turn, json!({"method":"thread/tokenUsage/updated","params":params}));
        assert_eq!(view.events.last().unwrap().data, params);
        CodexSession::apply_event(&mut view, &mut turn, json!({"method":"thread/tokenUsage/updated","params":{"threadId":"foreign","tokenUsage":{}}}));
        assert_eq!(view.events.len(), 1);
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
    fn native_question_enters_pending_and_expires_with_its_turn() {
        let (mut view, mut turn) = running();
        let request = json!({"id":71,"method":"item/tool/requestUserInput","params":{"threadId":"thread","turnId":"second-turn","itemId":"question","isBlocking":true,"questions":[{"id":"choice","header":"Scope","question":"Which module?","options":[{"label":"Core","description":"Core module"}]}]}});
        assert!(CodexSession::apply_event(&mut view, &mut turn, request).is_none());
        assert_eq!(view.pending[0].kind, "item/tool/requestUserInput");
        assert_eq!(view.status, "waiting");
        assert!(CodexSession::valid_response(&view.pending[0], &json!({"answers":{"choice":{"answers":["Core"]}}})));
        CodexSession::apply_event(&mut view, &mut turn, json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"second-turn","status":"completed"}}}));
        assert!(view.pending.is_empty());
        let stale = CodexSession::apply_event(&mut view, &mut turn, json!({"id":72,"method":"item/tool/requestUserInput","params":{"threadId":"thread","turnId":"second-turn","questions":[]}}));
        assert_eq!(stale.unwrap()["error"]["code"], -32600);
    }

    #[test]
    fn turn_completion_does_not_expire_session_scoped_mcp_elicitation() {
        let (mut view, mut turn) = running();
        for (id, method, params) in [
            (71, "item/tool/requestUserInput", json!({"threadId":"thread","turnId":"second-turn","questions":[]})),
            (72, "mcpServer/elicitation/request", json!({"threadId":"thread","serverName":"fixture","turnId":null})),
        ] {
            assert!(CodexSession::apply_event(&mut view, &mut turn, json!({"id":id,"method":method,"params":params})).is_none());
        }
        let elicitation = view.pending[1].id.clone();
        CodexSession::apply_event(&mut view, &mut turn, json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"second-turn","status":"completed"}}}));
        assert_eq!(view.pending.len(), 1);
        assert_eq!(view.pending[0].id, elicitation);
        assert_eq!(view.status, "waiting");
        assert!(view.active_run.is_none());
        assert_eq!(view.terminal_run.as_ref().unwrap().status, "completed");
        CodexSession::apply_event(&mut view, &mut turn, json!({"method":"serverRequest/resolved","params":{"threadId":"thread","requestId":72}}));
        assert!(view.pending.is_empty());
        assert_eq!(view.status, "idle");
    }

    #[test]
    fn user_input_preserves_partial_or_cancel_answers_and_rejects_replacement_fields() {
        let pending = PendingRequest { id: "request".into(), kind: "item/tool/requestUserInput".into(), title: String::new(), data: json!({"questions":[{"id":"a"},{"id":"b"}]}) };
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
