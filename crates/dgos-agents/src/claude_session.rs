use crate::{
    diagnostics::ProcessDiagnostic,
    session::{PendingRequest, SessionOptions, SessionView},
    transport::Transport,
};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tokio::{process::Command, sync::Mutex};

#[derive(Clone)]
pub struct ClaudeSession {
    wire: Transport,
    view: Arc<Mutex<SessionView>>,
    native_protocol_supported: Arc<std::sync::atomic::AtomicBool>,
}
impl ClaudeSession {
    /// Provider material travels only over the private bridge environment and stdio.
    pub(crate) async fn open_provider(options: SessionOptions, providers: &dgos_providers::ProviderManager, launch: &crate::capabilities::ProviderLaunch) -> Result<Self, crate::transport::StartupError> {
        let diagnostic = ProcessDiagnostic::new("agent_start", "claude");
        let probe = crate::cli::probe(crate::cli::CliTool::Claude, options.executable.as_deref()).await;
        let executable = probe.path.filter(|_| probe.status == "available").ok_or("cli_incompatible")?;
        let discovery = diagnostic.phase("bridge_discovery", None);
        let bridge = bridge_path();
        discovery.finish(bridge.as_ref().err().map(String::as_str));
        let bridge = bridge?;
        let auth_style = match (launch.launch.metadata().auth_style.sends_bearer(), launch.launch.metadata().auth_style.sends_x_api_key()) { (true, true) => "both", (true, false) => "bearer", (false, true) => "x_api_key", _ => return Err("provider_incompatible".into()) };
        let mut command = Command::new(bridge);
        command.current_dir(&options.workspace);
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if name.starts_with("ANTHROPIC_") || name.starts_with("CLAUDE_CODE_USE_") || matches!(name.as_ref(), "AWS_BEARER_TOKEN_BEDROCK" | "GOOGLE_API_KEY" | "GOOGLE_APPLICATION_CREDENTIALS") { command.env_remove(key); }
        }
        command.envs(&launch.env);
        let process_lease = launch.lease.clone();
        let (wire, mut events) = providers.with_cli_spawn(&launch.lease, &launch.launch, |_| {
            Transport::spawn(command, process_lease.cancellation_token(), diagnostic, process_lease.clone()).map_err(|_| dgos_types::DgosError::ProviderRequestRejected)
        }).await.map_err(|_| "provider_revision_changed")?;
        let session = Self {
            wire,
            view: Arc::new(Mutex::new(SessionView::new(options.workspace.clone()))),
            native_protocol_supported: Arc::default(),
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
                        if event_run(&view, &params).is_none() || matches!(view.status.as_str(), "closed" | "unknown" | "error") { continue; }
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
                        if view.pending.is_empty() && view.active_run.is_some() && view.status == "waiting" {
                            view.status = "running".into();
                        }
                    }
                    "result" => {
                        if event_run(&view, &params).is_none() { continue; }
                        let status = run_status(&params);
                        view.status = match status { "failed" => "error", "unknown" => "unknown", _ => "idle" }.into();
                        let message = result_text(&params);
                        view.error = matches!(status, "failed" | "unknown").then(|| message.clone().unwrap_or_else(|| if status == "unknown" { "result_unknown" } else { "engine_failed" }.into()));
                        view.pending.clear();
                        view.push(
                            method,
                            message,
                            terminal_data(params),
                        );
                        view.active_run = None;
                    }
                    "message" | "tool_progress" => {
                        let run = event_run(&view, &params);
                        // send() already records the human prompt with this native UUID.
                        if method == "message" && params["type"] == "user" && run.as_deref().is_some_and(|run| params["uuid"].as_str() == Some(run)) { continue; }
                        view.push_run(method, None, params, run);
                    }
                    "history" => { view.history_truncated |= params["truncated"] == true; }
                    "controls" => {
                        if params.get("session_id").and_then(Value::as_str) != view.native_id.as_deref()
                            || params.get("request_id").and_then(Value::as_str).is_some_and(|id| view.active_run.as_deref() != Some(id)) { continue; }
                        if let Ok(controls) = serde_json::from_value::<crate::session::SessionControls>(params["controls"].clone()) { view.controls = Some(controls); }
                    }
                    "native/initialized" => view.push(method, None, params),
                    "text_delta" => {
                        let run = event_run(&view, &params);
                        let text = params.get("text").and_then(Value::as_str).map(str::to_owned);
                        view.push_run(method, text, params, run);
                    }
                    "failure" => {
                        if params.get("request_id").and_then(Value::as_str).is_some_and(|id| view.active_run.as_deref() != Some(id)) { continue; }
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
                        if let Some(controls) = &mut view.controls { controls.uncertain(); }
                        view.pending.clear();
                    }
                    _ => {}
                }
            }
        });
        let params = json!({"workspace":options.workspace,"resume_id":options.resume_id,"executable":executable,"provider":{"model":launch.model,"base_url":launch.launch.metadata().base_url,"auth_style":auth_style,"env":launch.env}});
        let result = session.wire.startup_request("handshake_open", "open", params).await;
        session.wire.startup_result(result.as_ref().err().map(String::as_str));
        match result {
            Ok(result) => {
                let evidence = &result["provider_evidence"];
                if evidence["source"] != "cli_control" || evidence["model"] != launch.model || evidence["base_url"] != launch.launch.metadata().base_url || evidence["auth_style"] != auth_style || evidence["api_provider"] != "firstParty" || evidence["flag_settings_applied"] != true || evidence["route_env_verified"] != true || evidence["auth_helper_disabled"] != true {
                    return Err(session.wire.startup_failure("provider_incompatible".into()).await);
                }
                if providers.validate_cli_lease(&launch.lease).await.is_err() {
                    return Err(session.wire.startup_failure("provider_revision_changed".into()).await);
                }
                if result.get("session_id").and_then(Value::as_str).is_none_or(str::is_empty) {
                    return Err(session.wire.startup_failure("invalid_output".into()).await);
                }
                if options.resume_id.as_deref().is_some_and(|expected| result["session_id"].as_str() != Some(expected)) {
                    return Err(session.wire.startup_failure("resume_identity_mismatch".into()).await);
                }
                session.native_protocol_supported.store(result["native_protocol_supported"] == true, std::sync::atomic::Ordering::Release);
                let mut view = session.view.lock().await;
                if view.status != "starting" || !session.wire.native_available() {
                    drop(view);
                    return Err(session.wire.startup_failure("result_unknown".into()).await);
                }
                view.native_id = result
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                view.status = "idle".into();
                view.controls = result.get("controls").cloned().and_then(|controls| serde_json::from_value(controls).ok());
            }
            Err(error) => {
                return Err(session.wire.startup_failure(error).await);
            }
        }
        Ok(session)
    }
    pub async fn send(&self, request_id: String, text: String, settings: Option<crate::session::AgentTurnSettings>) -> Result<(), String> {
        let settings = settings.filter(|settings| !settings.is_empty());
        {
            let mut view = self.view.lock().await;
            if view.status != "idle" {
                return Err("busy".into());
            }
            if let Some(settings) = &settings {
                view.controls.as_ref().ok_or("turn_controls_unavailable")?.validate(settings)?;
            }
            view.status = "running".into();
            view.active_run = Some(request_id.clone());
            view.error = None;
            view.push("userMessage", Some(text.clone()), json!({"uuid":request_id,"request_id":request_id}));
        }
        let mut params = json!({"request_id":request_id,"text":text});
        if let Some(settings) = settings { params["settings"] = serde_json::to_value(settings).map_err(|_| "invalid_turn_settings")?; }
        if let Err(error) = self.wire.request("send", params).await
        {
            let mut view = self.view.lock().await;
            if view.active_run.as_deref() == Some(request_id.as_str()) {
                view.status = "unknown".into();
                view.error = Some(error.clone());
                if let Some(controls) = &mut view.controls { controls.uncertain(); }
                view.push("failure", Some(error.clone()), json!({"run_status":"unknown","native_status":"send_failed"}));
                view.active_run = None;
            }
            return Err(error);
        }
        Ok(())
    }
    pub async fn respond(&self, request_id: String, response: Value) -> Result<(), String> {
        {
            let mut view = self.view.lock().await;
            if view.status != "waiting" || !view.pending.iter().any(|pending| pending.id == request_id) {
                return Err("request_expired".into());
            }
            if !matches!(response.get("behavior").and_then(Value::as_str), Some("allow" | "deny")) {
                return Err("invalid_response".into());
            }
            // Reserve before IO. A lost acknowledgement never makes approval safe to repeat.
            view.pending.retain(|pending| pending.id != request_id);
        }
        let result = self.wire.request("respond", json!({"request_id":request_id,"response":response})).await;
        if result.is_err() {
            let mut view = self.view.lock().await;
            if view.active_run.is_some() && !matches!(view.status.as_str(), "closed" | "unknown") {
                view.status = "unknown".into();
                view.error = Some("result_unknown".into());
                view.pending.clear();
            }
        }
        result.map(|_| ())
    }
    pub async fn interrupt(&self) -> Result<(), String> {
        self.wire.request("interrupt", json!({})).await.map(|_| ())
    }
    pub async fn snapshot(&self) -> SessionView {
        self.view.lock().await.clone()
    }
    pub(crate) fn native_available(&self) -> bool { self.wire.native_available() }
    pub(crate) fn native_capabilities(&self) -> crate::native_session::NativeSessionCapabilities {
        if !self.native_protocol_supported.load(std::sync::atomic::Ordering::Acquire) { return crate::native_session::NativeSessionCapabilities::default(); }
        if !self.native_available() { return crate::native_session::NativeSessionCapabilities { native_auth_guidance: Some(crate::native_session::NativeAuthGuidance::installed(crate::Engine::Claude)), ..Default::default() }; }
        crate::native_session::NativeSessionCapabilities { observe_skills: true, observe_mcp: true, reload_skills: true, reload_plugins: true, reconnect_mcp: true, toggle_mcp: true, native_auth_guidance: Some(crate::native_session::NativeAuthGuidance::installed(crate::Engine::Claude)), ..Default::default() }
    }
    pub(crate) async fn observe_native(&self, domain: crate::native_session::SessionDomain) -> Result<crate::native_session::Projection, String> {
        if !self.native_capabilities().observe_skills { return Err("native_extension_Unsupported".into()); }
        let result = self.wire.request("native/observe", json!({"domain":domain})).await?;
        let mut out = crate::native_session::Projection::default();
        match domain {
            crate::native_session::SessionDomain::Skills => crate::native_session::skills(result["skills"].as_array().ok_or("native_extension_InvalidFormat")?, false, &mut out),
            crate::native_session::SessionDomain::Mcp => crate::native_session::mcp(result["servers"].as_array().ok_or("native_extension_InvalidFormat")?, false, &mut out),
        }
        out.incomplete |= result["incomplete"].as_bool().unwrap_or(true);
        Ok(out)
    }
    pub(crate) async fn admit_native_action(&self, action: &crate::native_session::NativeSessionAction) -> Result<(), String> {
        use crate::native_session::NativeSessionAction;
        let name = match action { NativeSessionAction::ReconnectMcp { server_name } | NativeSessionAction::ToggleMcp { server_name, .. } => server_name, _ => return Ok(()) };
        let observed = self.observe_native(crate::native_session::SessionDomain::Mcp).await.map_err(|_| "native_extension_Unavailable")?;
        if observed.incomplete { return Err("native_extension_Unsupported".into()); }
        let mut matches = observed.mcp_servers.iter().filter(|server| &server.name == name);
        let server = matches.next().ok_or("native_extension_Unsupported")?;
        if matches.next().is_some() || !matches!(server.source.as_deref(), Some("user" | "project" | "local")) { return Err("native_extension_Unsupported".into()); }
        Ok(())
    }
    pub(crate) async fn native_action(&self, action: &crate::native_session::NativeSessionAction) -> Result<crate::native_session::NativeActionState, String> {
        if !self.native_capabilities().reload_skills || matches!(action, crate::native_session::NativeSessionAction::NativeLogin { .. } | crate::native_session::NativeSessionAction::NativeLogout { .. }) { return Err("native_extension_Unsupported".into()); }
        let result = self.wire.request("native/action", json!({"action":action})).await?;
        serde_json::from_value(result["state"].clone()).map_err(|_| "result_unknown".into())
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

fn event_run(view: &SessionView, params: &Value) -> Option<String> {
    if params["history"] == true { return None; }
    if let (Some(expected), Some(actual)) = (view.native_id.as_deref(), params.get("session_id").and_then(Value::as_str)) {
        if expected != actual { return None; }
    }
    let id = params.get("request_id")?.as_str()?;
    view.active_run.as_ref().filter(|active| active.as_str() == id).cloned()
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

// Known native aborts end only the run. Future terminal reasons remain unknown.

fn run_status(params: &Value) -> &'static str {
    let reason = params.get("terminal_reason").and_then(Value::as_str);
    if matches!(reason, Some("aborted_tools" | "aborted_streaming")) { return "cancelled"; }
    if reason.is_some_and(|reason| !matches!(reason,
        "blocking_limit" | "rapid_refill_breaker" | "prompt_too_long" | "image_error" |
        "model_error" | "api_error" | "malformed_tool_use_exhausted" | "stop_hook_prevented" |
        "hook_stopped" | "tool_deferred" | "max_turns" | "background_requested" |
        "completed" | "budget_exhausted" | "structured_output_retry_exhausted" |
        "tool_deferred_unavailable" | "turn_setup_failed")) { return "unknown"; }
    match params.get("is_error").and_then(Value::as_bool) {
        Some(true) => "failed",
        Some(false) if params["subtype"] == "success" || reason == Some("completed") => "completed",
        _ => "unknown",
    }
}

fn bridge_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("DGOS_CLAUDE_BRIDGE") {
        let path = PathBuf::from(path);
        return if is_executable_file(&path) {
            Ok(path)
        } else {
            Err("bridge_unavailable".into())
        };
    }
    let executable = std::env::current_exe().map_err(|_| "bridge_unavailable")?;
    if let Some(parent) = executable.parent() {
        for path in [
            parent.join("../Resources/agent-bridge/dgos-claude-bridge"),
            parent.join("agent-bridge/dgos-claude-bridge"),
        ] {
            if is_executable_file(&path) {
                return Ok(path);
            }
        }
    }
    #[cfg(debug_assertions)]
    {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/agent-bridge/bin/dgos-claude-bridge");
        if is_executable_file(&path) {
            return Ok(path);
        }
    }
    Err("bridge_unavailable".into())
}

fn is_executable_file(path: &std::path::Path) -> bool {
    let Ok(metadata) = path.metadata() else { return false; };
    if !metadata.is_file() { return false; }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    { true }
}
#[cfg(test)]
mod tests {
    use super::{event_run, run_status, result_text, terminal_data};
    use serde_json::json;

    #[test]
    fn native_abort_allows_next_turn_without_hiding_engine_failures() {
        for reason in ["aborted_tools", "aborted_streaming"] {
            assert_eq!(run_status(&json!({"is_error":true,"terminal_reason":reason})), "cancelled");
        }
        for reason in ["api_error", "model_error", "turn_setup_failed"] {
            assert_eq!(run_status(&json!({"is_error":true,"terminal_reason":reason})), "failed");
        }
        assert_eq!(run_status(&json!({"is_error":true})), "failed");
        assert_eq!(run_status(&json!({})), "unknown");
        for error in [false, true] {
            assert_eq!(run_status(&json!({"is_error":error,"terminal_reason":"future_reason"})), "unknown");
        }
        assert_eq!(run_status(&json!({"is_error":false,"terminal_reason":"completed"})), "completed");
        assert_eq!(run_status(&json!({"is_error":false,"subtype":"success"})), "completed");
        assert_eq!(run_status(&json!({"is_error":false,"subtype":"future_subtype"})), "unknown");
    }

    #[test]
    fn historical_and_late_events_cannot_end_the_current_send() {
        let mut view = crate::session::SessionView::new("/workspace".into());
        view.native_id = Some("native-session".into());
        view.active_run = Some("second".into());
        assert_eq!(event_run(&view, &json!({"request_id":"second"})).as_deref(), Some("second"));
        for params in [json!({"request_id":"first"}), json!({"request_id":"second","history":true}), json!({"request_id":"second","session_id":"other-session"}), json!({})] {
            assert!(event_run(&view, &params).is_none());
        }
        view.active_run = None;
        assert!(event_run(&view, &json!({"request_id":"second"})).is_none());
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

    #[test]
    fn unknown_terminal_keeps_identity_and_cannot_report_success() {
        for native in [
            json!({"request_id":"send","uuid":"result","session_id":"session","is_error":false,"subtype":"success","terminal_reason":"future_reason"}),
            json!({"request_id":"send","uuid":"result","session_id":"session","terminal_reason":"result_identity_unknown"}),
        ] {
            let data = terminal_data(native.clone());
            assert_eq!(data["run_status"], "unknown");
            assert_eq!(data["request_id"], native["request_id"]);
            assert_eq!(data["uuid"], native["uuid"]);
            assert_eq!(data["session_id"], native["session_id"]);
            assert_eq!(data["terminal_reason"], native["terminal_reason"]);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn native_held_receipt_is_preserved_and_managed_mcp_is_read_only() {
        use super::*;
        use crate::native_session::{NativeSessionAction, NativeActionState};
        let root = std::env::temp_dir().join(format!("dgos-native-claude-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let manager = crate::test_support::manager(&root).await;
        let task_id = crate::test_support::task(&manager, &root).await;
        let options = SessionOptions { workspace: root.to_string_lossy().into(), executable: None, resume_id: None, model: None, trust_workspace: true };
        let create = crate::test_support::create(&manager, "alice", &task_id, options.clone()).await;
        let project_id = manager.development_workspace("alice").await.unwrap().projects[0].id.clone();
        let project = manager.inner.repository.project("alice", &project_id).await.unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let lease = tokio_util::sync::CancellationToken::new();
        let launch = manager.inner.providers.prepare("alice", &id, &task_id, &project, crate::Engine::Codex, &options, create.project_revision, create.provider, &lease).await.unwrap();
        let executable = root.join("bridge");
        crate::test_support::prepare_shell_fixture(&executable, r#"
while IFS= read -r line; do
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
*'"method":"native/action"'*) printf '{"id":%s,"result":{"state":"held"}}\n' "$request";;
*'"method":"native/observe"'*'"domain":"skills"'*) printf '{"id":%s,"result":{"skills":[{"name":"help","builtin":true},{"name":"help"},{"name":"mcp__fixture__tool","builtin":false}],"incomplete":false}}\n' "$request";;
*'"method":"native/observe"'*) printf '{"id":%s,"result":{"servers":[{"name":"managed","status":"connected","source":"managed","config":{"env":{"KEY":"secret"}}}],"incomplete":false}}\n' "$request";;
esac
done
"#).await;
        let mut command = Command::new(executable);
        command.current_dir(&root);
        let (wire, _events) = Transport::spawn(command, launch.lease.cancellation_token(), ProcessDiagnostic::new("agent_start", "claude"), launch.lease.clone()).unwrap();
        let session = ClaudeSession { wire, view: Arc::new(Mutex::new(SessionView::new(root.to_string_lossy().into()))), native_protocol_supported: Arc::new(std::sync::atomic::AtomicBool::new(true)) };
        let commands = session.observe_native(crate::native_session::SessionDomain::Skills).await.unwrap();
        assert_eq!(commands.skills[0].classification, crate::native_session::NativeSkillClassification::BuiltinCommand);
        assert_eq!(commands.skills[1].classification, crate::native_session::NativeSkillClassification::Unknown);
        assert_eq!(commands.skills[2].classification, crate::native_session::NativeSkillClassification::Unknown);
        assert_eq!(commands.skills[2].builtin, Some(false));
        assert!(!commands.incomplete);
        assert_eq!(session.native_action(&NativeSessionAction::ReloadPlugins { hold_on_cache_impact: true }).await.unwrap(), NativeActionState::Held);
        assert_eq!(session.admit_native_action(&NativeSessionAction::ReconnectMcp { server_name: "managed".into() }).await.unwrap_err(), "native_extension_Unsupported");
        assert!(!session.native_capabilities().native_login);
        assert_eq!(session.native_capabilities().native_auth_guidance.unwrap().verified_version, "2.1.319");
        session.native_protocol_supported.store(false, std::sync::atomic::Ordering::Release);
        let unsupported = session.native_capabilities();
        assert!(!unsupported.observe_skills);
        assert!(unsupported.native_auth_guidance.is_none());
        assert!(!unsupported.native_login);
        assert_eq!(session.native_action(&NativeSessionAction::ReloadSkills {}).await.unwrap_err(), "native_extension_Unsupported");
        session.native_protocol_supported.store(true, std::sync::atomic::Ordering::Release);
        assert!(session.wire.close().await);
        manager.inner.providers.cleanup("alice", &id, &launch.binding, true, false).await.unwrap();
        assert!(manager.shutdown().await);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn resolved_host_references_reach_claude_as_text_without_settings_overrides() {
        use super::*;
        use crate::references::{ConversationReference, ReferenceKind};
        let root = std::env::temp_dir().join(format!("dgos-claude-references-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("code")).unwrap();
        std::fs::write(root.join("code/source.txt"), "host-owned reference bytes\n").unwrap();
        let manager = crate::test_support::manager(&root).await;
        let task_id = crate::test_support::task(&manager, &root.join("code")).await;
        let project = manager.task_project("alice", &task_id).await.unwrap();
        let options = SessionOptions { workspace: project.path.clone(), executable: None, resume_id: None, model: None, trust_workspace: true };
        let create = crate::test_support::create(&manager, "alice", &task_id, options.clone()).await;
        let id = uuid::Uuid::new_v4().to_string();
        let lease = tokio_util::sync::CancellationToken::new();
        let launch = manager.inner.providers.prepare("alice", &id, &task_id, &project, crate::Engine::Codex, &options, create.project_revision, create.provider, &lease).await.unwrap();
        let record = crate::SessionRecord { id: id.clone(), owner: "alice".into(), engine: crate::Engine::Claude, native_id: None, workspace: project.path.clone(), executable: None, model: None, project_id: Some(project.id.clone()), task_id: Some(task_id.clone()) };
        let file = manager.read_development_file("alice", crate::files::ReadFileInput { project_id: project.id.clone(), task_id: Some(task_id.clone()), path: "source.txt".into() }).await.unwrap();
        let reference = ConversationReference { project_id: project.id, task_id: Some(task_id), path: "source.txt".into(), kind: ReferenceKind::File, diff_source: None, expected_hash: file.hash, start_line: None, end_line: None };
        let resolved = manager.resolve_conversation_references("alice", &record, "Review this file".into(), vec![reference]).await.unwrap();
        let executable = root.join("bridge");
        crate::test_support::prepare_shell_fixture(&executable, r#"
while IFS= read -r line; do
printf '%s\n' "$line" >> protocol
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
*'"method":"send"'*) printf '{"id":%s,"result":{}}\n' "$request";;
esac
done
"#).await;
        let mut command = Command::new(executable); command.current_dir(&root);
        let (wire, _events) = Transport::spawn(command, launch.lease.cancellation_token(), ProcessDiagnostic::new("agent_start", "claude"), launch.lease.clone()).unwrap();
        let mut view = SessionView::new(project.path); view.status = "idle".into();
        let session = ClaudeSession { wire, view: Arc::new(Mutex::new(view)), native_protocol_supported: Arc::new(std::sync::atomic::AtomicBool::new(true)) };
        session.send(uuid::Uuid::new_v4().to_string(), resolved.clone(), None).await.unwrap();
        let protocol: serde_json::Value = serde_json::from_str(std::fs::read_to_string(root.join("protocol")).unwrap().lines().next().unwrap()).unwrap();
        assert_eq!(protocol["method"], "send");
        assert_eq!(protocol["params"]["text"], resolved);
        assert!(protocol["params"].get("settings").is_none());
        assert!(protocol["params"]["text"].as_str().unwrap().ends_with("host-owned reference bytes\n"));
        assert!(session.wire.close().await);
        manager.inner.providers.cleanup("alice", &id, &launch.binding, true, false).await.unwrap();
        assert!(manager.shutdown().await);
        std::fs::remove_dir_all(root).unwrap();
    }
}
