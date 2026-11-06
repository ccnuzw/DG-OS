use crate::{
    diagnostics::ProcessDiagnostic,
    session::{PendingRequest, SessionOptions, SessionView},
    transport::Transport,
};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tokio::{process::Command, sync::Mutex};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct ClaudeSession {
    wire: Transport,
    view: Arc<Mutex<SessionView>>,
}
impl ClaudeSession {
    /// Keeps credentials on the bridge's private environment and stdio channel.
    pub(crate) async fn open_with_capabilities(options: SessionOptions, lease: CancellationToken, launch: crate::capabilities::CapabilityLaunch) -> Result<Self, String> {
        let diagnostic = ProcessDiagnostic::new("agent_start", "claude");
        let discovery = diagnostic.phase("discovery", None);
        let executable = crate::session::resolve_executable("claude", options.executable.as_deref());
        discovery.finish(executable.as_ref().err().map(String::as_str));
        let executable = executable?;
        let discovery = diagnostic.phase("bridge_discovery", None);
        let bridge = bridge_path();
        discovery.finish(bridge.as_ref().err().map(String::as_str));
        let bridge = bridge?;
        let mut command = Command::new(bridge);
        command.current_dir(&options.workspace).envs(&launch.env);
        let (wire, mut events) = Transport::spawn(command, lease, diagnostic)?;
        let session = Self {
            wire,
            view: Arc::new(Mutex::new(SessionView::new(options.workspace.clone()))),
        };
        let reader = session.clone();
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                let method = event.get("method").and_then(Value::as_str).unwrap_or("");
                let params = event.get("params").cloned().unwrap_or(Value::Null);
                let mut view = reader.view.lock().await;
                match method {
                    "ready" => {
                        view.native_id = params
                            .get("session_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        view.push(method, None, params);
                    }
                    "permission" => {
                        if let Some(id) = params.get("id").and_then(Value::as_str) {
                            view.request(PendingRequest {
                                id: id.into(),
                                kind: "claude/toolPermission".into(),
                                title: params
                                    .get("tool")
                                    .and_then(Value::as_str)
                                    .unwrap_or("Tool")
                                    .into(),
                                data: params,
                            });
                        }
                    }
                    "permission_resolved" => {
                        if let Some(id) = params.get("id").and_then(Value::as_str) {
                            view.pending.retain(|p| p.id != id);
                        }
                        if view.pending.is_empty() {
                            view.status = "running".into();
                        }
                    }
                    "result" => {
                        let status = run_status(&params);
                        view.status = if status == "failed" { "error" } else { "idle" }.into();
                        let message = result_text(&params);
                        view.error = (status == "failed").then(|| message.clone().unwrap_or_else(|| "engine_failed".into()));
                        view.pending.clear();
                        view.push(
                            method,
                            message,
                            terminal_data(params),
                        );
                        view.active_run = None;
                    }
                    "message" | "tool_progress" => view.push(method, None, params),
                    "capabilities/initialized" => view.push(method, None, params),
                    "text_delta" => view.push(
                        method,
                        params
                            .get("text")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        Value::Null,
                    ),
                    "failure" => {
                        view.status = "error".into();
                        let message = params.get("message").and_then(Value::as_str).unwrap_or("engine_failed").to_owned();
                        view.error = Some(message.clone());
                        view.pending.clear();
                        view.push(method, Some(message), json!({"run_status":"failed","native_status":"failure","error":params}));
                        view.active_run = None;
                    }
                    "host/closed" | "closed" => {
                        if view.status != "closed" {
                            view.status = "unknown".into();
                        }
                        view.pending.clear();
                    }
                    _ => {}
                }
            }
        });
        let mut params = json!({"workspace":options.workspace,"resume_id":options.resume_id,"model":launch.model.or(options.model),"executable":executable});
        if launch.managed || launch.claude_plugin.is_some() || launch.mcp_url.is_some() {
            params["capabilities"] = json!({"managed":launch.managed,"plugin_path":launch.claude_plugin,"mcp_url":launch.mcp_url,"mcp_bearer":launch.mcp_bearer,"provider_env_keys":launch.env.keys().filter(|key| key.starts_with("ANTHROPIC_") || key.starts_with("CLAUDE_CODE_USE_")).collect::<Vec<_>>()});
        }
        let result = session.wire.startup_request("handshake_open", "open", params).await;
        session.wire.startup_result(result.as_ref().err().map(String::as_str).or_else(|| result.as_ref().ok().and_then(|value| value.get("capability_error").and_then(Value::as_str))));
        match result {
            Ok(result) if matches!(result.get("capability_error").and_then(Value::as_str), Some("skill_load_failed" | "provider_load_failed" | "provider_unavailable" | "mcp_unavailable" | "capability_load_failed")) => {
                let error = result["capability_error"].as_str().unwrap().to_owned();
                session.close().await;
                return Err(error);
            }
            Ok(result) => {
                let mut view = session.view.lock().await;
                view.native_id = result
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                view.status = "idle".into();
            }
            Err(error) => {
                session.close().await;
                return Err(error);
            }
        }
        Ok(session)
    }
    pub async fn send(&self, request_id: String, text: String) -> Result<(), String> {
        {
            let mut view = self.view.lock().await;
            if view.status != "idle" {
                return Err("busy".into());
            }
            view.status = "running".into();
            view.active_run = Some(request_id.clone());
            view.error = None;
            view.push("userMessage", Some(text.clone()), Value::Null);
        }
        if let Err(error) = self
            .wire
            .request("send", json!({"request_id":request_id,"text":text}))
            .await
        {
            let mut view = self.view.lock().await;
            view.status = "unknown".into();
            view.error = Some(error.clone());
            view.push("failure", Some(error.clone()), json!({"run_status":"unknown","native_status":"send_failed"}));
            view.active_run = None;
            return Err(error);
        }
        Ok(())
    }
    pub async fn respond(&self, request_id: String, response: Value) -> Result<(), String> {
        if !self
            .view
            .lock()
            .await
            .pending
            .iter()
            .any(|p| p.id == request_id)
        {
            return Err("request_expired".into());
        }
        self.wire
            .request(
                "respond",
                json!({"request_id":request_id,"response":response}),
            )
            .await
            .map(|_| ())
    }
    pub async fn interrupt(&self) -> Result<(), String> {
        self.wire.request("interrupt", json!({})).await.map(|_| ())
    }
    pub async fn snapshot(&self) -> SessionView {
        self.view.lock().await.clone()
    }
    pub async fn close(&self) -> bool {
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.wire.request("close", json!({})),
        )
        .await;
        let closed = self.wire.close().await;
        if closed { self.view.lock().await.status = "closed".into(); }
        closed
    }
    pub async fn wait_stopped(&self) -> bool {
        self.wire.wait_stopped().await
    }
}

fn result_text(params: &Value) -> Option<String> {
    params.get("result").and_then(Value::as_str).map(str::to_owned).or_else(|| {
        let errors = params.get("errors")?.as_array()?;
        let text = errors.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
        (!text.is_empty()).then_some(text)
    })
}

fn terminal_data(mut params: Value) -> Value {
    params["run_status"] = json!(run_status(&params));
    params["native_status"] = params.get("subtype").cloned().unwrap_or(Value::Null);
    params
}

// Native aborts end a turn, not the persistent session. Unknown reasons fail closed.
fn result_failed(params: &Value) -> bool {
    if matches!(params.get("terminal_reason").and_then(Value::as_str), Some("aborted_tools" | "aborted_streaming")) {
        return false;
    }
    params.get("is_error").and_then(Value::as_bool).unwrap_or(true)
}

// Maps a native result to the run terminal status. Aborts end only the run.
fn run_status(params: &Value) -> &'static str {
    if result_failed(params) {
        "failed"
    } else if matches!(params.get("terminal_reason").and_then(Value::as_str), Some("aborted_tools" | "aborted_streaming")) {
        "cancelled"
    } else {
        "completed"
    }
}

fn bridge_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("DGOS_CLAUDE_BRIDGE") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }
    let executable = std::env::current_exe().map_err(|_| "bridge_unavailable")?;
    if let Some(parent) = executable.parent() {
        for path in [
            parent.join("../Resources/agent-bridge/dgos-claude-bridge"),
            parent.join("agent-bridge/dgos-claude-bridge"),
        ] {
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    #[cfg(debug_assertions)]
    {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/agent-bridge/bin/dgos-claude-bridge");
        if path.is_file() {
            return Ok(path);
        }
    }
    Err("bridge_unavailable".into())
}
#[cfg(test)]
mod tests {
    use super::{result_failed, result_text, terminal_data};
    use serde_json::json;

    #[test]
    fn native_abort_allows_next_turn_without_hiding_engine_failures() {
        for reason in ["aborted_tools", "aborted_streaming"] {
            assert!(!result_failed(&json!({"is_error":true,"terminal_reason":reason})));
        }
        for reason in ["api_error", "model_error", "turn_setup_failed", "future_reason"] {
            assert!(result_failed(&json!({"is_error":true,"terminal_reason":reason})));
        }
        assert!(result_failed(&json!({"is_error":true})));
        assert!(result_failed(&json!({})));
        assert!(!result_failed(&json!({"is_error":false,"terminal_reason":"completed"})));
    }

    #[test]
    fn failed_result_retains_native_details_and_actual_errors() {
        let native = json!({"subtype":"error_during_execution","is_error":true,"terminal_reason":"api_error","errors":["native failure","request failed"]});
        assert_eq!(result_text(&native).as_deref(), Some("native failure\nrequest failed"));
        let data = terminal_data(native.clone());
        assert_eq!(data["run_status"], "failed");
        assert_eq!(data["native_status"], "error_during_execution");
        assert_eq!(data["terminal_reason"], native["terminal_reason"]);
        assert_eq!(data["errors"], native["errors"]);
    }
}
