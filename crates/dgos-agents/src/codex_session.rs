use crate::{
    diagnostics::ProcessDiagnostic,
    session::{PendingRequest, SessionOptions, SessionView},
    transport::Transport,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
use tokio::{process::Command, sync::Mutex};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct CodexSession {
    wire: Transport,
    view: Arc<Mutex<SessionView>>,
    receipts: Arc<Mutex<HashMap<String, ([u8; 32], Option<Result<(), String>>)>>>,
    turn: Arc<Mutex<Option<String>>>,
    user_echo_pending: Arc<Mutex<bool>>,
}
impl CodexSession {
    async fn event(&self, event: Value) {
        let method = event.get("method").and_then(Value::as_str).unwrap_or("");
        let params = event.get("params").cloned().unwrap_or(Value::Null);
        if let Some(id) = event.get("id") {
            if matches!(
                method,
                "item/commandExecution/requestApproval"
                    | "item/fileChange/requestApproval"
                    | "tool/requestUserInput"
                    | "mcpServer/elicitation/request"
            ) {
                let mut view = self.view.lock().await;
                view.request(PendingRequest {
                    id: id.to_string(),
                    kind: method.into(),
                    title: method.into(),
                    data: params,
                });
            } else {
                let _ = self.wire.write(json!({"id":id,"error":{"code":-32601,"message":"Unsupported client request"}})).await;
            }
            return;
        }
        let mut view = self.view.lock().await;
        match method {
            "turn/started" => {
                *self.turn.lock().await = params.pointer("/turn/id").and_then(Value::as_str).map(str::to_owned);
                view.status = "running".into();
            }
            "turn/completed" => {
                let status = params.pointer("/turn/status").and_then(Value::as_str).unwrap_or("unknown").to_owned();
                *self.turn.lock().await = None;
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
                *self.user_echo_pending.lock().await = false;
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
                    let mut echo = self.user_echo_pending.lock().await;
                    Self::completed_item(&mut view, item, &mut echo);
                }
            }
            "turn/diff/updated" => view.push(
                method,
                params.get("diff").and_then(Value::as_str).map(str::to_owned),
                json!({"turnId":params.get("turnId")}),
            ),
            "turn/plan/updated" => view.push(method, None, json!({"plan":params.get("plan"),"explanation":params.get("explanation"),"turnId":params.get("turnId")})),
            "serverRequest/resolved" => {
                if let Some(id) = params.get("requestId") {
                    view.pending.retain(|p| p.id != id.to_string());
                }
                if view.pending.is_empty() && view.status == "waiting" {
                    view.status = "running".into();
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
            }
            "error" => {
                let message = native_error(&params);
                view.error = Some(message.clone());
                view.push(method, Some(message), params);
            }
            _ => {}
        }
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
            if view.status != "idle" {
                return Err("busy".into());
            }
            view.status = "running".into();
            view.active_run = Some(request_id.clone());
            *self.user_echo_pending.lock().await = true;
            view.error = None;
            view.push("userMessage", Some(text.clone()), Value::Null);
            view.native_id.clone().ok_or("closed")?
        };
        receipts.insert(request_id.clone(), (fingerprint, None));
        drop(receipts);
        let result = self
            .wire
            .request(
                "turn/start",
                json!({"threadId":native,"input":[{"type":"text","text":text}]}),
            )
            .await;
        self.receipts.lock().await.insert(
            request_id,
            (
                fingerprint,
                Some(result.as_ref().map(|_| ()).map_err(Clone::clone)),
            ),
        );
        if let Err(error) = result {
            let mut view = self.view.lock().await;
            view.status = "unknown".into();
            view.error = Some(error.clone());
            *self.user_echo_pending.lock().await = false;
            view.push("error", Some(error.clone()), json!({"run_status":"unknown","native_status":"send_failed"}));
            view.active_run = None;
            return Err(error);
        }
        Ok(())
    }
    pub async fn respond(&self, request_id: String, response: Value) -> Result<(), String> {
        let mut view = self.view.lock().await;
        let pending = view
            .pending
            .iter()
            .find(|p| p.id == request_id)
            .ok_or("request_expired")?;
        let valid = match pending.kind.as_str() {
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                let decision = response.get("decision");
                decision.and_then(Value::as_str).is_some_and(|d| {
                    matches!(d, "accept" | "acceptForSession" | "decline" | "cancel")
                }) && pending
                    .data
                    .get("availableDecisions")
                    .and_then(Value::as_array)
                    .is_none_or(|allowed| allowed.contains(decision.unwrap_or(&Value::Null)))
                    && response.as_object().is_some_and(|o| o.len() == 1)
            }
            "tool/requestUserInput" => response
                .get("answers")
                .and_then(Value::as_object)
                .is_some_and(|answers| {
                    answers.iter().all(|(id, value)| {
                        pending
                            .data
                            .get("questions")
                            .and_then(Value::as_array)
                            .is_some_and(|questions| {
                                questions
                                    .iter()
                                    .any(|q| q.get("id").and_then(Value::as_str) == Some(id))
                            })
                            && value
                                .get("answers")
                                .and_then(Value::as_array)
                                .is_some_and(|a| a.iter().all(Value::is_string))
                    })
                }),
            "mcpServer/elicitation/request" => {
                response
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|d| matches!(d, "accept" | "decline" | "cancel"))
                    && response
                        .get("content")
                        .is_some_and(|v| v.is_null() || v.is_object())
            }
            _ => false,
        };
        if !valid {
            return Err("invalid_response".into());
        }
        let id: Value = serde_json::from_str(&request_id).map_err(|_| "invalid_response")?;
        self.wire.write(json!({"id":id,"result":response})).await?;
        view.pending.retain(|p| p.id != request_id);
        if view.pending.is_empty() {
            view.status = "running".into();
        }
        Ok(())
    }
    pub async fn interrupt(&self) -> Result<(), String> {
        let native = self.view.lock().await.native_id.clone().ok_or("closed")?;
        let turn = self.turn.lock().await.clone().ok_or("not_running")?;
        self.wire
            .request("turn/interrupt", json!({"threadId":native,"turnId":turn}))
            .await
            .map(|_| ())
    }
    pub async fn snapshot(&self) -> SessionView {
        self.view.lock().await.clone()
    }
    pub async fn close(&self) -> bool {
        let closed = self.wire.close().await;
        if closed { self.view.lock().await.status = "closed".into(); }
        closed
    }
    pub async fn wait_stopped(&self) -> bool {
        self.wire.wait_stopped().await
    }
    #[cfg(test)]
    pub async fn open(options: SessionOptions, lease: CancellationToken) -> Result<Self, String> {
        Self::open_with_capabilities(options, lease, Default::default()).await
    }
    /// Applies host-owned capability material only to this CLI process.
    pub(crate) async fn open_with_capabilities(options: SessionOptions, lease: CancellationToken, launch: crate::capabilities::CapabilityLaunch) -> Result<Self, String> {
        let diagnostic = ProcessDiagnostic::new("agent_start", "codex");
        let discovery = diagnostic.phase("discovery", None);
        let executable = crate::session::resolve_executable("codex", options.executable.as_deref());
        discovery.finish(executable.as_ref().err().map(String::as_str));
        let executable = executable?;
        let mut command = Command::new(executable);
        command
            .args(["app-server", "--stdio"])
            .current_dir(&options.workspace);
        command.envs(&launch.env);
        for override_value in &launch.codex_overrides {
            command.arg("-c").arg(override_value);
        }
        let (wire, mut events) = Transport::spawn(command, lease, diagnostic)?;
        let session = Self {
            wire,
            view: Arc::new(Mutex::new(SessionView::new(options.workspace.clone()))),
            receipts: Arc::default(),
            turn: Arc::default(),
            user_echo_pending: Arc::default(),
        };
        let reader = session.clone();
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                reader.event(event).await;
            }
        });
        let setup = async {
            session.wire.startup_request("handshake_initialize", "initialize", json!({"clientInfo":{"name":"dgos","version":"0.1.0"},"capabilities":{"experimentalApi":launch.managed || launch.skill_root.is_some()}})).await?;
            session.wire.startup_write(json!({"method":"initialized","params":{}})).await?;
            if let Some(root) = &launch.skill_root {
                session.register_skills(root, &options.workspace).await?;
            }
            let mut thread_config = None;
            if launch.managed {
                let read = session.wire.startup_request("handshake_config_read", "config/read", json!({"includeLayers":false,"cwd":options.workspace})).await.map_err(|_| "mcp_unavailable")?;
                let servers = read.pointer("/config/mcp_servers").and_then(Value::as_object).ok_or("mcp_unavailable")?;
                let mut config = serde_json::Map::with_capacity(servers.len());
                for (name, server) in servers {
                    if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-') || !server.is_object() { return Err("mcp_unavailable".into()); }
                    config.insert(format!("mcp_servers.{name}.enabled"), json!(name == "dgos_project"));
                }
                if !servers.contains_key("dgos_project") { return Err("mcp_unavailable".into()); }
                thread_config = Some(Value::Object(config));
            }
            let method = if options.resume_id.is_some() { "thread/resume" } else { "thread/start" };
            let mut params = json!({"cwd":options.workspace,"approvalPolicy":"on-request","sandbox":"workspace-write"});
            let stage = if options.resume_id.is_some() { "handshake_thread_resume" } else { "handshake_thread_start" };
            if let Some(id) = options.resume_id { params["threadId"] = json!(id); }
            if let Some(model) = launch.model.or(options.model).filter(|s| !s.is_empty()) { params["model"] = json!(model); }
            if let Some(config) = thread_config { params["config"] = config; }
            if launch.managed && launch.codex_overrides.iter().any(|value| value.split_once('=').is_some_and(|(key, provider)| key.trim() == "model_provider" && provider.trim() == "\"dgos_project\"")) {
                params["modelProvider"] = json!("dgos_project");
            }
            let result = session.wire.startup_request(stage, method, params).await?;
            if launch.managed {
                let thread_id = result.pointer("/thread/id").and_then(Value::as_str).ok_or("mcp_unavailable")?;
                let status = session.wire.startup_request("handshake_mcp_status", "mcpServerStatus/list", json!({"threadId":thread_id,"detail":"toolsAndAuthOnly","limit":1000})).await.map_err(|_| "mcp_unavailable")?;
                let servers = status.get("data").and_then(Value::as_array).ok_or("mcp_unavailable")?;
                session.view.lock().await.push("mcp/isolated", None, json!({"servers":servers.iter().filter_map(|server| Some(json!({"name":server.get("name")?.as_str()? ,"status":server.get("runtimeStatus").cloned().unwrap_or(Value::Null)}))).collect::<Vec<_>>()}));
                let gateway_connected = servers.iter().any(|server| server.get("name").and_then(Value::as_str) == Some("dgos_project") && server.get("runtimeStatus").and_then(Value::as_str) == Some("connected"));
                let others_disabled = servers.iter().filter(|server| server.get("name").and_then(Value::as_str) != Some("dgos_project")).all(|server| server.get("runtimeStatus").and_then(Value::as_str) == Some("disabled"));
                if status.get("nextCursor").is_some_and(|cursor| !cursor.is_null()) || !gateway_connected || !others_disabled {
                    return Err("mcp_unavailable".into());
                }
            }
            let id = result.pointer("/thread/id").and_then(Value::as_str).ok_or("invalid_output")?;
            let mut view = session.view.lock().await;
            view.native_id = Some(id.into()); view.status = "idle".into();
            if let Some(turns) = result.pointer("/thread/turns").and_then(Value::as_array) {
                for turn in turns { if let Some(items) = turn.get("items").and_then(Value::as_array) {
                    for item in items { Self::item(&mut view, item); }
                }}
            }
            Ok::<(),String>(())
        }.await;
        session.wire.startup_result(setup.as_ref().err().map(String::as_str));
        if let Err(error) = setup {
            session.close().await;
            return Err(error);
        }
        Ok(session)
    }
    async fn register_skills(&self, root: &str, workspace: &str) -> Result<(), String> {
        let registered = async {
            self.wire.startup_request("handshake_skill_roots", "skills/extraRoots/set", json!({"extraRoots":[root]})).await?;
            let result = self.wire.startup_request("handshake_skills_list", "skills/list", json!({"cwds":[workspace],"forceReload":true})).await?;
            let entries = result.get("data").and_then(Value::as_array).ok_or("skill_load_failed")?;
            let mut directories = tokio::fs::read_dir(root).await.map_err(|_| "skill_load_failed")?;
            let mut paths = Vec::new();
            while let Some(directory) = directories.next_entry().await.map_err(|_| "skill_load_failed")? {
                if !directory.file_type().await.map_err(|_| "skill_load_failed")?.is_dir() { return Err("skill_load_failed".into()); }
                let expected = tokio::fs::canonicalize(directory.path().join("SKILL.md")).await.map_err(|_| "skill_load_failed")?;
                let mut found = false;
                for entry in entries {
                    let Some(skills) = entry.get("skills").and_then(Value::as_array) else { continue; };
                    for skill in skills {
                        if skill.get("enabled").and_then(Value::as_bool) != Some(true) { continue; }
                        if let Some(path) = skill.get("path").and_then(Value::as_str) {
                            if tokio::fs::canonicalize(path).await.is_ok_and(|path| path == expected) { found = true; break; }
                        }
                    }
                    if found { break; }
                }
                if !found { return Err("skill_load_failed".into()); }
                paths.push(expected.to_string_lossy().into_owned());
            }
            if paths.is_empty() { return Err("skill_load_failed".into()); }
            self.view.lock().await.push("skills/registered", None, json!({"paths":paths,"status":"registered_not_executed"}));
            Ok::<(), String>(())
        }.await;
        registered.map_err(|_| "skill_load_failed".into())
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

fn native_error(params: &Value) -> String {
    params.pointer("/turn/error/message")
        .or_else(|| params.pointer("/error/message"))
        .or_else(|| params.get("message"))
        .and_then(Value::as_str).unwrap_or("engine_failed").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(native_error(&json!({"turn":{"error":{"message":"native failure"}}})), "native failure");
    }
}
