use super::{CapabilitySnapshot, ProjectCapabilities};
use axum::{extract::{DefaultBodyLimit, State}, http::{HeaderMap, StatusCode}, response::{IntoResponse, Response}, routing::post, Json, Router};
use dgos_mcp::CallMcpToolRequest;
use dgos_types::{BuiltinCaller, ExecutionProvenance};
use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc, time::{Duration, Instant}};
use subtle::ConstantTimeEq;
use tokio::sync::{oneshot, Mutex};
use tokio_util::sync::CancellationToken;

pub(super) struct Binding {
    pub owner: String,
    pub session_id: String,
    pub task_id: String,
    pub project_id: String,
    pub snapshot: CapabilitySnapshot,
    pub stop: CancellationToken,
    lease: CancellationToken,
    pub run: Mutex<Option<(String, CancellationToken)>>,
    calls: Mutex<BTreeMap<String, CancellationToken>>,
    created: Instant,
}
impl Binding {
    pub fn new(owner: &str, session: &str, task: &str, project: &str, snapshot: CapabilitySnapshot, lease: CancellationToken) -> Self {
        Self { owner: owner.into(), session_id: session.into(), task_id: task.into(), project_id: project.into(), snapshot, stop: CancellationToken::new(), lease, run: Mutex::new(None), calls: Mutex::new(BTreeMap::new()), created: Instant::now() }
    }
    pub async fn authorize(&self, service: &ProjectCapabilities) -> Result<(), String> {
        if self.stop.is_cancelled() || self.lease.is_cancelled() || self.created.elapsed() > Duration::from_secs(86400) { return Err("capability_revoked".into()); }
        if service.repository.capability_epoch(&self.owner, &self.project_id).await? != self.snapshot.permission_epoch { return Err("capability_revoked".into()); }
        let project = service.repository.project(&self.owner, &self.project_id).await?;
        let task = service.repository.task(&self.owner, &self.task_id).await?;
        if project.archived || task.project_id != project.id { return Err("capability_revoked".into()); }
        Ok(())
    }
    async fn current_run(&self, service: &ProjectCapabilities) -> Result<(String, CancellationToken), String> {
        self.authorize(service).await?;
        let (id, cancellation) = self.run.lock().await.clone().ok_or("run_not_active")?;
        let run = service.repository.run(&self.owner, &id).await?.ok_or("run_not_active")?;
        if run.session_id.as_deref() != Some(&self.session_id) || run.project_id != self.project_id || run.task_id != self.task_id || run.source != "native_agent" || !matches!(run.status.as_str(), "running" | "waiting") || cancellation.is_cancelled() { return Err("run_not_active".into()); }
        Ok((id, cancellation))
    }
}
#[derive(Clone, Serialize)]
pub struct CapabilityConfirmation {
    pub id: String,
    pub project_id: String,
    pub task_id: String,
    pub session_id: String,
    pub run_id: String,
    pub server_id: String,
    pub tool_name: String,
    pub schema_hash: String,
    pub arguments: Value,
}
pub(super) struct PendingConfirmation {
    pub owner: String,
    pub prompt: CapabilityConfirmation,
    pub(super) answer: oneshot::Sender<bool>,
}
#[derive(Clone)]
struct Endpoint { service: ProjectCapabilities, binding: Arc<Binding>, bearer: Arc<String> }
impl ProjectCapabilities {
    pub(super) async fn start_gateway(&self, binding: Arc<Binding>) -> Result<(String, String), String> {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.map_err(|_| "mcp_unavailable")?;
        let address = listener.local_addr().map_err(|_| "mcp_unavailable")?;
        let bearer = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let state = Endpoint { service: self.clone(), binding: binding.clone(), bearer: Arc::new(bearer.clone()) };
        let router = Router::new().route("/mcp", post(endpoint)).layer(DefaultBodyLimit::max(1024 * 1024)).with_state(state);
        let stopped = binding.stop.clone();
        let lease = binding.lease.clone();
        tokio::spawn(async move {
            let shutdown = async move { tokio::select! { _ = stopped.cancelled() => {}, _ = lease.cancelled() => {}, _ = tokio::time::sleep(Duration::from_secs(86400)) => {} } };
            let _ = axum::serve(listener, router).with_graceful_shutdown(shutdown).await;
            binding.stop.cancel();
        });
        Ok((format!("http://{address}/mcp"), bearer))
    }
    pub async fn pending_confirmations(&self, owner: &str, session: &str) -> Result<Vec<CapabilityConfirmation>, String> {
        let binding = self.bindings.lock().await.get(session).cloned().ok_or("session_not_found")?;
        if binding.owner != owner { return Err("session_not_found".into()); }
        binding.authorize(self).await?;
        Ok(self.confirmations.lock().await.values().filter(|p| p.owner == owner && p.prompt.session_id == session && !p.answer.is_closed()).map(|p| p.prompt.clone()).collect())
    }
    pub async fn respond_confirmation(&self, owner: &str, session: &str, id: &str, approved: bool) -> Result<(), String> {
        let binding = self.bindings.lock().await.get(session).cloned().ok_or("session_not_found")?;
        if binding.owner != owner { return Err("session_not_found".into()); }
        let (run, _) = binding.current_run(self).await?;
        let mut pending = self.confirmations.lock().await;
        if !pending.get(id).is_some_and(|p| p.owner == owner && p.prompt.session_id == session && p.prompt.run_id == run && !p.answer.is_closed()) { return Err("request_expired".into()); }
        pending.remove(id).ok_or("request_expired")?.answer.send(approved).map_err(|_| "request_expired".into())
    }
}
async fn endpoint(State(endpoint): State<Endpoint>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    // Authentication precedes JSON parsing and all capability reads. No cookies, Origin or URL token fallback.
    let supplied = headers.get("authorization").and_then(|h| h.to_str().ok()).and_then(|s| s.strip_prefix("Bearer ")).unwrap_or("");
    if supplied.as_bytes().ct_eq(endpoint.bearer.as_bytes()).unwrap_u8() != 1 || headers.contains_key("origin") {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    if endpoint.binding.authorize(&endpoint.service).await.is_err() { return (StatusCode::FORBIDDEN, "capability_revoked").into_response(); }
    let request: Value = match serde_json::from_slice(&body) { Ok(value) => value, Err(_) => return StatusCode::BAD_REQUEST.into_response() };
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") { return StatusCode::BAD_REQUEST.into_response(); }
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    if id.is_none() {
        if method == "notifications/cancelled" {
            if let Some(id) = request.pointer("/params/requestId") { if let Some(token) = endpoint.binding.calls.lock().await.get(&id.to_string()) { token.cancel(); } }
        }
        return StatusCode::ACCEPTED.into_response();
    }
    let id = id.unwrap_or(Value::Null);
    if !id.is_string() && !id.is_number() { return StatusCode::BAD_REQUEST.into_response(); }
    let result = match method {
        "initialize" => Ok(json!({"protocolVersion":request.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or("2025-03-26"),"capabilities":{"tools":{}},"serverInfo":{"name":"dgos-project","version":"1.0.0"},"instructions":"DG-OS project tools require separate host approval on every call. Native Shell is outside this MCP policy."})),
        "ping" => Ok(json!({})),
        "tools/list" => list_tools(&endpoint).await,
        "tools/call" => {
            let cancellation = CancellationToken::new();
            let key = id.to_string();
            let duplicate = {
                let mut calls = endpoint.binding.calls.lock().await;
                if calls.contains_key(&key) { true } else { calls.insert(key.clone(), cancellation.clone()); false }
            };
            if duplicate { Err("request_conflict".into()) } else {
                let call = call_tool(&endpoint, request.get("params").unwrap_or(&Value::Null), &cancellation);
                tokio::pin!(call);
                let outcome = tokio::select! {
                    result = &mut call => result,
                    _ = endpoint.binding.stop.cancelled() => { cancellation.cancel(); let _ = call.await; Err("capability_revoked_result_unknown".into()) },
                    _ = endpoint.binding.lease.cancelled() => { cancellation.cancel(); let _ = call.await; Err("capability_revoked_result_unknown".into()) },
                    _ = cancellation.cancelled() => { let _ = call.await; Err("cancelled_result_unknown".into()) },
                };
                endpoint.binding.calls.lock().await.remove(&key);
                outcome
            }
        }
        _ => Err("method_not_supported".into()),
    };
    let response = match result { Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}), Err(reason) => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":reason}}) };
    Json(response).into_response()
}
fn exposed_name(index: usize) -> String { format!("project_tool_{index}") }
async fn list_tools(endpoint: &Endpoint) -> Result<Value, String> {
    let mut tools = Vec::new();
    for (index, selected) in endpoint.binding.snapshot.configuration.mcp.iter().enumerate() {
        let current = endpoint.service.mcp.list_tools(&endpoint.binding.owner, &selected.server_id).await.map_err(|_| "mcp_disconnected")?;
        let tool = current.into_iter().find(|t| t.tool_name == selected.tool_name && t.schema_hash == selected.schema_hash).ok_or("schema_changed")?;
        tools.push(json!({"name":exposed_name(index),"description":format!("{} / {}. {}", selected.server_id, selected.tool_name, tool.description.unwrap_or_default()),"inputSchema":tool.input_schema}));
    }
    Ok(json!({"tools":tools}))
}
struct ConfirmationGuard { service: ProjectCapabilities, id: String }
impl Drop for ConfirmationGuard {
    fn drop(&mut self) {
        let service = self.service.clone(); let id = self.id.clone();
        tokio::spawn(async move { service.confirmations.lock().await.remove(&id); });
    }
}
async fn call_tool(endpoint: &Endpoint, params: &Value, cancellation: &CancellationToken) -> Result<Value, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    let binding = &endpoint.binding;
    let service = &endpoint.service;
    let (run, run_cancel) = binding.current_run(service).await?;
    let name = params.get("name").and_then(Value::as_str).ok_or("tool_not_selected")?;
    let selected = binding.snapshot.configuration.mcp.iter().enumerate().find(|(i, _)| exposed_name(*i) == name).map(|(_, t)| t).ok_or("tool_not_selected")?;
    if params.as_object().is_none_or(|p| p.keys().any(|k| !matches!(k.as_str(), "name" | "arguments" | "_meta"))) { return Err("invalid_tool_request".into()); }
    let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    if !arguments.is_object() { return Err("invalid_tool_request".into()); }
    let current = service.mcp.list_tools(&binding.owner, &selected.server_id).await.map_err(|_| "mcp_disconnected")?;
    if !current.iter().any(|t| t.tool_name == selected.tool_name && t.schema_hash == selected.schema_hash) { return Err("schema_changed".into()); }
    let confirmation_id = uuid::Uuid::new_v4().to_string();
    let (answer, answered) = oneshot::channel();
    service.confirmations.lock().await.insert(confirmation_id.clone(), PendingConfirmation { owner: binding.owner.clone(), prompt: CapabilityConfirmation { id: confirmation_id.clone(), project_id: binding.project_id.clone(), task_id: binding.task_id.clone(), session_id: binding.session_id.clone(), run_id: run.clone(), server_id: selected.server_id.clone(), tool_name: selected.tool_name.clone(), schema_hash: selected.schema_hash.clone(), arguments: arguments.clone() }, answer });
    let _guard = ConfirmationGuard { service: service.clone(), id: confirmation_id };
    let approved = tokio::select! {
        result = tokio::time::timeout_at(deadline, answered) => matches!(result, Ok(Ok(true))),
        _ = run_cancel.cancelled() => false,
        _ = cancellation.cancelled() => false,
    };
    if !approved { return Err("confirmation_refused".into()); }
    let (current_run, _) = binding.current_run(service).await?;
    if current_run != run { return Err("run_not_active".into()); }
    let current = service.mcp.list_tools(&binding.owner, &selected.server_id).await.map_err(|_| "mcp_disconnected")?;
    if !current.iter().any(|t| t.tool_name == selected.tool_name && t.schema_hash == selected.schema_hash) { return Err("schema_changed".into()); }
    let mut input = CallMcpToolRequest { server_id: selected.server_id.clone(), tool_name: selected.tool_name.clone(), arguments, call_id: Some(uuid::Uuid::new_v4().to_string()), task_ref: Some(binding.task_id.clone()), timeout_ms: Some(deadline.saturating_duration_since(tokio::time::Instant::now()).as_millis().max(1) as u64), confirmation_token: None };
    input.confirmation_token = Some(service.mcp.confirm_call(&binding.owner, &input).await.map_err(|_| "confirmation_refused")?);
    // McpManager holds its schema/lifecycle locks across authorization and external dispatch.
    let provenance = ExecutionProvenance { caller: BuiltinCaller::Workbench, request_id: run };
    let call = service.mcp.call_tool_at_schema_cancellable_with_provenance(&binding.owner, input, &selected.schema_hash, cancellation, &provenance);
    tokio::pin!(call);
    let output = tokio::select! {
        result = &mut call => result.map_err(|error| match error { dgos_mcp::McpError::ConfirmationRequired => "schema_changed_or_confirmation_expired", dgos_mcp::McpError::PermissionDenied => "capability_revoked", _ => "mcp_failed_or_disconnected" })?,
        _ = run_cancel.cancelled() => { cancellation.cancel(); let _ = call.await; return Err("cancelled_result_unknown".into()); },
        _ = tokio::time::sleep_until(deadline) => { cancellation.cancel(); let _ = call.await; return Err("mcp_result_unknown".into()); },
    };
    if matches!(output.outcome.as_str(), "cancelled" | "timeout") { return Err("mcp_result_unknown".into()); }
    let mut result = json!({"isError":output.is_error,"content":output.content});
    if let Some(structured) = output.structured_content { result["structuredContent"] = structured; }
    Ok(result)
}
