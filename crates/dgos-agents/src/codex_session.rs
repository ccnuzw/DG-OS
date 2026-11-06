use crate::{
    diagnostics::ProcessDiagnostic,
    session::{PendingRequest, SessionOptions, SessionView},
    transport::Transport,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::{HashMap, HashSet}, sync::Arc, time::Duration};
use tokio::{process::Command, sync::Mutex};
use tokio_util::sync::CancellationToken;

type SendReceipt = ([u8; 32], Option<Result<(), String>>);

#[derive(Clone)]
pub struct CodexSession {
    wire: Transport,
    view: Arc<Mutex<SessionView>>,
    receipts: Arc<Mutex<HashMap<String, SendReceipt>>>,
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
                    let request_id = id.to_string();
                    view.pending.retain(|p| p.id != request_id);
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
        let (wire, mut events) = Transport::spawn(command, lease.clone(), diagnostic)?;
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
                // Plugin MCP servers are absent from config/read's mcp_servers table.
                // Discover their names before starting a thread, without disabling the
                // plugins themselves (which also supply native Skills and hooks).
                let inventory = session.wire.startup_request("handshake_mcp_inventory", "mcpServerStatus/list", json!({"detail":"toolsAndAuthOnly","limit":1000})).await.map_err(|_| "mcp_unavailable")?;
                thread_config = Some(managed_mcp_config(&read, &inventory)?);
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
                session.wait_managed_mcp(thread_id, &lease).await?;
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
    async fn wait_managed_mcp(&self, thread_id: &str, lease: &CancellationToken) -> Result<(), String> {
        let ready = async {
            loop {
                let status = self.wire.startup_request("handshake_mcp_status", "mcpServerStatus/list", json!({"threadId":thread_id,"detail":"toolsAndAuthOnly","limit":1000})).await.map_err(|_| "mcp_unavailable")?;
                if managed_mcp_ready(&status)? {
                    let servers = status["data"].as_array().ok_or("mcp_unavailable")?;
                    self.view.lock().await.push("mcp/isolated", None, json!({"servers":servers.iter().map(|server| json!({"name":server["name"],"status":server["runtimeStatus"]})).collect::<Vec<_>>()}));
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        tokio::select! {
            biased;
            _ = lease.cancelled() => Err("closed".into()),
            result = tokio::time::timeout(Duration::from_secs(20), ready) => result.unwrap_or_else(|_| Err("mcp_unavailable".into())),
        }
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

// Thread config keys are protocol dotted paths, not TOML expressions: quoting a
// segment creates a different literal server name. Reject ambiguous segments.
fn mcp_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn managed_mcp_config(read: &Value, inventory: &Value) -> Result<Value, String> {
    let servers = read.pointer("/config/mcp_servers").and_then(Value::as_object).ok_or("mcp_unavailable")?;
    let gateway_url = servers.get("dgos_project").and_then(|server| server.get("url")).and_then(Value::as_str).filter(|url| !url.is_empty()).ok_or("mcp_unavailable")?;
    let mut config = serde_json::Map::with_capacity(servers.len());
    for (name, server) in servers {
        if !mcp_name(name) || !server.is_object() { return Err("mcp_unavailable".into()); }
        config.insert(format!("mcp_servers.{name}.enabled"), json!(name == "dgos_project"));
    }
    if inventory.get("nextCursor").is_some_and(|cursor| !cursor.is_null()) { return Err("mcp_unavailable".into()); }
    let entries = inventory.get("data").and_then(Value::as_array).ok_or("mcp_unavailable")?;
    let mut names = HashSet::with_capacity(entries.len());
    for entry in entries {
        let name = entry.get("name").and_then(Value::as_str).filter(|name| mcp_name(name)).ok_or("mcp_unavailable")?;
        if !names.insert(name) { return Err("mcp_unavailable".into()); }
        if !servers.contains_key(name) {
            if entry.get("pluginId").and_then(Value::as_str).filter(|id| !id.is_empty()).is_none() { return Err("mcp_unavailable".into()); }
            // An enabled=false entry still needs a valid transport in Codex.
            // A disabled host URL shadows only this plugin's MCP server; no
            // native plugin, Skill, hook, or global configuration is changed.
            config.insert(format!("mcp_servers.{name}"), json!({"url":gateway_url,"enabled":false}));
        }
    }
    Ok(Value::Object(config))
}

fn managed_mcp_ready(status: &Value) -> Result<bool, String> {
    if status.get("nextCursor").is_some_and(|cursor| !cursor.is_null()) { return Err("mcp_unavailable".into()); }
    let servers = status.get("data").and_then(Value::as_array).ok_or("mcp_unavailable")?;
    let mut names = HashSet::with_capacity(servers.len());
    let mut gateway = None;
    for server in servers {
        let name = server.get("name").and_then(Value::as_str).filter(|name| mcp_name(name)).ok_or("mcp_unavailable")?;
        if !names.insert(name) { return Err("mcp_unavailable".into()); }
        let state = server.get("runtimeStatus").and_then(Value::as_str).ok_or("mcp_unavailable")?;
        if name == "dgos_project" {
            gateway = Some(match state {
                "connected" => true,
                "starting" => false,
                _ => return Err("mcp_unavailable".into()),
            });
        } else if state != "disabled" {
            return Err("mcp_unavailable".into());
        }
    }
    gateway.ok_or_else(|| "mcp_unavailable".into())
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

    #[test]
    fn managed_mcp_rejects_unknown_terminal_and_unselected_states() {
        for state in ["notStarted", "failed", "cancelled", "disabled", "authenticationRequired", "unknown"] {
            assert_eq!(managed_mcp_ready(&json!({"data":[{"name":"dgos_project","runtimeStatus":state}]})), Err("mcp_unavailable".into()));
        }
        for state in ["connected", "starting", "failed", "unknown"] {
            assert_eq!(managed_mcp_ready(&json!({"data":[{"name":"dgos_project","runtimeStatus":"starting"},{"name":"native","runtimeStatus":state}]})), Err("mcp_unavailable".into()));
        }
        for invalid in [
            json!({"data":[]}),
            json!({"data":[{"name":"dgos_project","runtimeStatus":null}]}),
            json!({"data":[{"name":"dgos_project","runtimeStatus":"connected"}],"nextCursor":"more"}),
            json!({"data":[{"name":"dgos_project","runtimeStatus":"connected"},{"name":"dgos_project","runtimeStatus":"starting"}]}),
        ] {
            assert_eq!(managed_mcp_ready(&invalid), Err("mcp_unavailable".into()));
        }
    }

    #[cfg(unix)]
    async fn managed_fixture(root: &std::path::Path, state: &str) -> SessionOptions {
        std::fs::create_dir_all(root).unwrap();
        let executable = root.join("codex");
        let script = r##"count=0
state=STATE
while IFS= read -r line; do
case "$line" in
*'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"config/read"'*) printf '%s\n' '{"id":2,"result":{"config":{"mcp_servers":{"dgos_project":{"url":"http://127.0.0.1:1/mcp"},"node_repl":{"command":"native"}},"plugins":{"native@bundled":{"enabled":true}},"skills":{"native":true},"hooks":{"native":true}}}}';;
*'"method":"mcpServerStatus/list"'*)
case "$line" in
*'"threadId"'*)
count=$((count + 1))
if [ "$count" -eq 1 ]; then
    printf '%s\n' '{"id":5,"result":{"data":[{"name":"dgos_project","runtimeStatus":"starting"},{"name":"cua_repl","runtimeStatus":"disabled"},{"name":"node_repl","runtimeStatus":"disabled"}],"nextCursor":null}}'
    printf '%s' starting > "starting"
elif [ "$state" = connected ]; then
    printf '%s\n' '{"id":6,"result":{"data":[{"name":"dgos_project","runtimeStatus":"connected"},{"name":"cua_repl","runtimeStatus":"disabled"},{"name":"node_repl","runtimeStatus":"disabled"}],"nextCursor":null}}'
else
    : # Leave the next readiness request pending to exercise lease cancellation.
fi;;
*) printf '%s\n' '{"id":3,"result":{"data":[{"name":"dgos_project","pluginId":null},{"name":"node_repl","pluginId":null},{"name":"cua_repl","pluginId":"native@bundled"}],"nextCursor":null}}';;
esac;;
*'"method":"thread/start"'*)
case "$line" in
*'"mcp_servers.cua_repl":{"enabled":false,"url":"http://127.0.0.1:1/mcp"}'*'"mcp_servers.dgos_project.enabled":true'*'"mcp_servers.node_repl.enabled":false'*)
    case "$line" in
    *'"plugins.'*|*'"skills.'*|*'"hooks.'*) exit 1;;
    esac
    printf '%s\n' '{"id":4,"result":{"thread":{"id":"isolated","turns":[]}}}';;
*) printf '%s\n' '{"id":4,"error":{"code":-32600,"message":"native MCP remained enabled"}}';;
esac;;
esac
done
"##.replace("state=STATE", &format!("state={state}"));
        crate::test_support::prepare_shell_fixture(&executable, &script).await;
        SessionOptions {
            workspace: root.to_string_lossy().into_owned(), resume_id: None,
            executable: Some(executable.to_string_lossy().into_owned()), model: None, trust_workspace: true,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_plugin_mcp_is_disabled_before_gateway_becomes_ready() {
        let root = std::env::temp_dir().join(format!("dgos-codex-isolation-{}", uuid::Uuid::new_v4()));
        let options = managed_fixture(&root, "connected").await;
        let session = tokio::time::timeout(Duration::from_secs(3), CodexSession::open_with_capabilities(options, CancellationToken::new(), crate::capabilities::CapabilityLaunch { managed: true, ..Default::default() })).await.unwrap().unwrap();
        let view = session.snapshot().await;
        assert_eq!(view.status, "idle");
        assert_eq!(view.native_id.as_deref(), Some("isolated"));
        let isolated = view.events.iter().find(|event| event.kind == "mcp/isolated").unwrap();
        assert!(isolated.data["servers"].as_array().unwrap().iter().any(|server| server["name"] == "dgos_project" && server["status"] == "connected"));
        assert!(session.close().await);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_gateway_readiness_wait_is_cancelled_without_a_reply() {
        let root = std::env::temp_dir().join(format!("dgos-codex-cancel-{}", uuid::Uuid::new_v4()));
        let lease = CancellationToken::new();
        let options = managed_fixture(&root, "pending").await;
        let task_lease = lease.clone();
        let opening = tokio::spawn(async move { CodexSession::open_with_capabilities(options, task_lease, crate::capabilities::CapabilityLaunch { managed: true, ..Default::default() }).await });
        tokio::time::timeout(Duration::from_secs(3), async {
            while !root.join("starting").exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
        }).await.unwrap();
        lease.cancel();
        let result = tokio::time::timeout(Duration::from_secs(3), opening).await.unwrap().unwrap();
        assert!(matches!(result, Err(reason) if reason == "closed"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
