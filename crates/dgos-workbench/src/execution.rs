//! Host-only model execution. Account identity, approval and lifecycle are supplied by the host.
use async_trait::async_trait;
use dgos_mcp::{CallMcpToolRequest, McpManager};
use dgos_providers::ProviderManager;
use dgos_types::{AiPhase, AiToolSelection, ChatMessage, ChatRequest, DgosError, ExecutionProvenance, GenerationParameters, Provider, ToolChatRequest, ToolMessage};
use futures::StreamExt;
use serde_json::{json, Value};
use std::{collections::HashSet, future::Future, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use crate::tools::{resolve_selected_tools, validate_budget, PendingProposal, ToolAction, ToolContext, ToolProposal};

pub struct ExecutionRequest {
    pub user: String,
    pub provider_id: String,
    pub model_id: String,
    pub revision: Option<i64>,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<AiToolSelection>,
    pub generation: GenerationParameters,
    pub context_window: Option<i32>,
    pub provenance: ExecutionProvenance,
    pub cancellation: CancellationToken,
    pub deadline: Instant,
}

#[derive(Default)]
pub struct ExecutionResult {
    pub content: Option<String>,
    pub total_tokens: Option<i64>,
    pub audit_warning: bool,
    pub error: Option<String>,
    pub result_unknown: bool,
}

pub enum CoreEvent { Delta(String), ReasoningDelta(String), Phase(AiPhase) }
pub trait ExecutionSink: Send + Sync { fn send(&self, event: CoreEvent) -> bool; }
impl<F: Fn(CoreEvent) -> bool + Send + Sync> ExecutionSink for F {
    fn send(&self, event: CoreEvent) -> bool { self(event) }
}
#[async_trait]
pub trait ExecutionGuard: Send + Sync {
    async fn before_model(&self) -> Result<(), String>;
    async fn before_tool(&self) -> Result<(), String>;
    async fn approve(&self, proposal: &ToolProposal) -> Result<bool, String>;
}

pub fn host_policy(has_tools: bool) -> &'static str {
    if has_tools { crate::tools::TOOL_HOST_POLICY } else { crate::HOST_POLICY }
}

pub fn validate_generation(provider: &Provider, generation: &GenerationParameters) -> Result<(), DgosError> {
    let vendor = provider.config.as_ref().and_then(|v| v.get("vendor_id")).and_then(Value::as_str)
        .unwrap_or_else(|| dgos_providers::vendor::infer_vendor(&provider.provider_type, &provider.base_url));
    let profile = dgos_providers::vendor::resolve_profile(vendor, &provider.provider_type, &provider.base_url)?;
    profile.check_sampling(generation.temperature, generation.reasoning.as_ref())?;
    if let Some(reasoning) = &generation.reasoning {
        let mapped = profile.reasoning_params(reasoning)?;
        if reasoning.budget_tokens.is_some() && !matches!(mapped, dgos_providers::vendor::ReasoningParams::AnthropicBudget { .. }) {
            return Err(DgosError::RequestNotSupported);
        }
        if matches!(mapped, dgos_providers::vendor::ReasoningParams::AnthropicBudget { budget } if budget >= generation.output_tokens()) {
            return Err(DgosError::RequestNotSupported);
        }
    }
    Ok(())
}

pub(crate) struct StepBinding<'a> {
    pub user: &'a str,
    pub provider_id: &'a str,
    pub model_id: &'a str,
    pub revision: Option<i64>,
    pub provenance: &'a ExecutionProvenance,
    pub cancellation: &'a CancellationToken,
    pub deadline: Instant,
    pub guard: &'a dyn ExecutionGuard,
    pub sink: &'a dyn ExecutionSink,
}

impl StepBinding<'_> {
    async fn bounded<T>(&self, future: impl Future<Output = Result<T, String>>) -> Result<T, String> {
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err("task_cancelled".into()),
            _ = tokio::time::sleep_until(self.deadline) => Err("generation_limit".into()),
            result = future => result,
        }
    }
    fn emit(&self, event: CoreEvent) -> Result<(), String> {
        if self.sink.send(event) { Ok(()) } else { self.cancellation.cancel(); Err("task_cancelled".into()) }
    }
}

pub struct AiExecutor;
impl AiExecutor {
    pub async fn run(providers: &ProviderManager, mcp: &McpManager, request: ExecutionRequest, guard: &dyn ExecutionGuard, sink: &dyn ExecutionSink) -> ExecutionResult {
        let mut result = ExecutionResult::default();
        let binding = StepBinding { user: &request.user, provider_id: &request.provider_id, model_id: &request.model_id, revision: request.revision,
            provenance: &request.provenance, cancellation: &request.cancellation, deadline: request.deadline, guard, sink };
        if request.tools.is_empty() {
            let bytes = request.messages.iter().map(|m| m.content.len()).sum::<usize>();
            if bytes > 24 * 1024 || request.context_window.is_some_and(|window| window <= 0 || bytes + 1024 + request.generation.output_tokens() as usize > window as usize) {
                result.error = Some("context_budget_exceeded".into());
                return result;
            }
            let admission = binding.bounded(guard.before_model()).await.and_then(|_| binding.emit(CoreEvent::Phase(AiPhase::Model)));
            if let Err(error) = admission { result.error = Some(error); return result; }
            let mut wire = ChatRequest { messages: request.messages, stream: Some(true), ..Default::default() };
            request.generation.apply(&mut wire);
            let drained = drain_stream(providers, &binding, wire).await;
            result.content = Some(drained.content).filter(|content| !content.is_empty());
            result.audit_warning = drained.audit_warning;
            match drained.end {
                StreamEnd::Completed { tokens, .. } => {
                    result.total_tokens = tokens;
                    if result.content.as_ref().map_or(true, |s| s.trim().is_empty()) { result.error = Some("empty_response".into()); }
                }
                StreamEnd::Stopped(error) => { result.error = Some(error.into()); result.result_unknown = drained.dispatched; }
                StreamEnd::Failed(code) => { result.error = Some(format!("provider_{code}")); result.result_unknown = drained.dispatched && !provider_rejection(code); }
            }
            return result;
        }
        if request.tools.len() > 8 { result.error = Some("tool_call_limit".into()); return result; }
        let tools = match binding.bounded(resolve_selected_tools(mcp, &request.user, request.tools)).await {
            Ok(tools) => tools,
            Err(error) => { result.error = Some(error); return result; }
        };
        let mut context = ToolContext { record: None, messages: request.messages.into_iter().map(|m| ToolMessage {
            role: m.role, content: Some(m.content), tool_call_id: None, tool_calls: vec![], reasoning: None, images: m.images,
        }).collect(), tools, window: request.context_window, generation: request.generation, pending: None, calls: HashSet::new(), tokens: 0 };
        let mut action = ToolAction::RunModel;
        let mut audit_warning = false;
        let mut unknown = false;
        let terminal = loop {
            match advance_tool_context(providers, mcp, &binding, &mut context, action, &mut audit_warning, &mut unknown).await {
                Ok(Some(content)) => break Ok(content),
                Err(error) => break Err(error),
                Ok(None) => {
                    let proposal = context.proposal().expect("step parks one proposal");
                    if let Err(error) = binding.emit(CoreEvent::Phase(AiPhase::AwaitingConfirmation)) { break Err(error); }
                    let approval_deadline = context.pending.as_ref().unwrap().expires.min(binding.deadline);
                    let decision = tokio::select! {
                        biased;
                        _ = binding.cancellation.cancelled() => Err("task_cancelled".into()),
                        _ = tokio::time::sleep_until(approval_deadline) => Err("confirmation_expired".into()),
                        answer = guard.approve(proposal) => answer,
                    };
                    match decision {
                        Ok(true) => action = ToolAction::Approve { proposal_id: proposal.id.clone() },
                        Ok(false) => break Err("tool_denied".into()),
                        Err(error) => break Err(error),
                    }
                }
            }
        };
        result.total_tokens = Some(context.tokens).filter(|n| *n > 0);
        result.audit_warning = audit_warning;
        result.result_unknown = unknown;
        match terminal { Ok(content) => result.content = Some(content), Err(error) => result.error = Some(error) }
        result
    }
}

/// One model step, optionally consuming the exact pending approval first. Both parked workbench
/// APIs and the callback executor use this function; the original assistant reasoning stays in memory.
pub(crate) async fn advance_tool_context(
    providers: &ProviderManager, mcp: &McpManager, binding: &StepBinding<'_>, context: &mut ToolContext,
    action: ToolAction, audit_warning: &mut bool, unknown: &mut bool,
) -> Result<Option<String>, String> {
    if !context.accepts(&action) { return Err("invalid_tool_action".into()); }
    if let Some(revision) = binding.revision {
        binding.bounded(async { providers.ensure_revision(binding.provider_id, binding.user, revision).await.map_err(|e| format!("provider_{}", e.error_code())) }).await?;
    }
    if let Some(pending) = context.pending.take() {
        if pending.expires <= Instant::now() { return Err("confirmation_expired".into()); }
        if matches!(action, ToolAction::Deny { .. }) { return Err("tool_denied".into()); }
        binding.bounded(binding.guard.before_tool()).await?;
        binding.emit(CoreEvent::Phase(AiPhase::Tool))?;
        let selected = &context.tools[pending.selected];
        let mut request = CallMcpToolRequest { server_id: selected.tool.server_id.clone(), tool_name: selected.tool.tool_name.clone(),
            arguments: pending.public.arguments, call_id: Some(uuid::Uuid::new_v4().to_string()), task_ref: Some(binding.provenance.request_id.clone()),
            timeout_ms: Some(30_000), confirmation_token: None };
        request.confirmation_token = Some(binding.bounded(async { mcp.confirm_call(binding.user, &request).await.map_err(|e| format!("mcp_{}", e.error_code())) }).await?);
        *unknown = true;
        let call = async { mcp.call_tool_at_schema_cancellable_with_provenance(binding.user, request, &selected.tool.schema_hash, binding.cancellation, binding.provenance).await.map_err(|e| format!("mcp_{}", e.error_code())) };
        tokio::pin!(call);
        let result = tokio::select! {
            biased;
            _ = tokio::time::sleep_until(binding.deadline) => {
                binding.cancellation.cancel();
                None
            }
            result = &mut call => Some(result),
        };
        let result = match result {
            Some(result) => result,
            None => call.await,
        };
        let result = match result {
            Ok(result) => result,
            Err(error) => { *unknown = false; return Err(error); }
        };
        *audit_warning |= result.audit_warning;
        *unknown = !matches!(result.outcome.as_str(), "success" | "error") || (result.outcome == "error" && !result.content.is_array());
        if *unknown { return Err(format!("mcp_{}", match result.outcome.as_str() { "timeout" => 1003006, "cancelled" => 1003016, _ => 1003005 })); }
        if result.audit_warning { return Err("mcp_audit_warning".into()); }
        if binding.cancellation.is_cancelled() { return Err("task_cancelled".into()); }
        context.messages.push(pending.assistant);
        context.messages.push(ToolMessage { role: "tool".into(), content: Some(json!({"isError": result.is_error, "content": result.content, "structuredContent": result.structured_content}).to_string()),
            tool_call_id: Some(pending.call.id), tool_calls: vec![], reasoning: None, images: vec![] });
    }
    let request = ToolChatRequest { messages: context.messages.clone(), tools: context.tools.iter().map(|t| t.definition.clone()).collect(),
        max_tokens: context.generation.output_tokens(), temperature: context.generation.temperature, reasoning: context.generation.reasoning.clone() };
    validate_budget(&request, context.window)?;
    let (response, stopped) = settle_model(providers, binding, ModelRequest::Tools(request), unknown).await;
    let response = match response? {
        ModelResponse::Tools(response) => response,
        ModelResponse::Text(_) => unreachable!("tool request returns tool response"),
    };
    *unknown = false;
    context.tokens = context.tokens.saturating_add(i64::from(response.usage.total_tokens));
    *audit_warning |= response.audit_warning;
    if let Some(reason) = stopped { return Err(reason.into()); }
    if response.message.tool_calls.is_empty() {
        return response.message.content.filter(|s| !s.trim().is_empty()).map(Some).ok_or_else(|| "empty_response".into());
    }
    if response.message.tool_calls.len() != 1 || context.calls.len() >= 8 { return Err("tool_call_limit".into()); }
    let assistant = response.message;
    let call = assistant.tool_calls[0].clone();
    if call.kind != "function" || call.id.is_empty() || call.id.len() > 128 || !call.id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') || !context.calls.insert(call.id.clone()) {
        return Err("invalid_tool_call".into());
    }
    let selected = context.tools.iter().position(|t| t.definition.function.name == call.function.name).ok_or("unselected_tool")?;
    if call.function.arguments.len() > 16 * 1024 { return Err("tool_arguments_limit".into()); }
    let arguments: Value = serde_json::from_str(&call.function.arguments).map_err(|_| "invalid_tool_arguments")?;
    let tool = &context.tools[selected];
    if !arguments.is_object() || !jsonschema::validator_for(&tool.tool.input_schema).map_err(|_| "invalid_tool_schema")?.is_valid(&arguments) { return Err("invalid_tool_arguments".into()); }
    context.pending = Some(PendingProposal { public: ToolProposal { id: uuid::Uuid::new_v4().to_string(), server_id: tool.tool.server_id.clone(), server_name: tool.server_name.clone(),
        tool_name: tool.tool.tool_name.clone(), schema_hash: tool.tool.schema_hash.clone(), arguments, expires_at: chrono::Utc::now().timestamp() + 120 }, selected, call, assistant,
        expires: Instant::now() + Duration::from_secs(120) });
    Ok(None)
}

pub(crate) enum ModelRequest { Text(ChatRequest), Tools(ToolChatRequest) }
pub(crate) enum ModelResponse { Text(dgos_types::ChatResponse), Tools(dgos_types::ToolChatResponse) }

pub(crate) async fn call_model(providers: &ProviderManager, binding: &StepBinding<'_>, request: ModelRequest, dispatched: &mut bool) -> Result<ModelResponse, String> {
    let (result, stopped) = settle_model(providers, binding, request, dispatched).await;
    match stopped { Some(reason) => Err(reason.into()), None => result }
}

async fn settle_model(providers: &ProviderManager, binding: &StepBinding<'_>, request: ModelRequest, dispatched: &mut bool) -> (Result<ModelResponse, String>, Option<&'static str>) {
    if let Err(error) = binding.bounded(binding.guard.before_model()).await.and_then(|_| binding.emit(CoreEvent::Phase(AiPhase::Model))) { return (Err(error), None); }
    let dispatch_state = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let call = async {
        match request {
            ModelRequest::Text(request) => providers.call_provider_bound_cancellable_with_provenance(binding.provider_id, binding.model_id, request, binding.user, binding.revision, Some(binding.provenance), binding.cancellation.clone(), binding.deadline, dispatch_state.clone()).await.map(ModelResponse::Text).map_err(|e| format!("provider_{}", e.error_code())),
            ModelRequest::Tools(request) => providers.call_provider_tools_bound_cancellable_with_provenance(binding.provider_id, binding.model_id, request, binding.user, binding.revision, Some(binding.provenance), binding.cancellation.clone(), binding.deadline, dispatch_state.clone()).await.map(ModelResponse::Tools).map_err(|e| format!("provider_{}", e.error_code())),
        }
    };
    tokio::pin!(call);
    let stopped = tokio::select! {
        biased;
        _ = binding.cancellation.cancelled() => Some("task_cancelled"),
        _ = tokio::time::sleep_until(binding.deadline) => Some("generation_limit"),
        result = &mut call => {
            let rejected = result.as_ref().err().is_some_and(|error| error.strip_prefix("provider_").and_then(|code| code.parse().ok()).is_some_and(provider_rejection));
            *dispatched = dispatch_state.load(std::sync::atomic::Ordering::Acquire) && !rejected;
            return (result, None);
        },
    };
    *dispatched = dispatch_state.load(std::sync::atomic::Ordering::Acquire);
    (Err(stopped.unwrap_or("task_cancelled").into()), stopped)
}

// These codes describe local admission failures or explicit upstream rejection.
// Transport, timeout, invalid response and unknown failures retain uncertainty.
fn provider_rejection(code: u32) -> bool {
    matches!(code, 1001001 | 1001002 | 1001003 | 1001004 | 1001005 | 1001012 |
        1001015 | 1001016 | 1001017 | 1001018 | 1001019 | 1001020 | 1001022 |
        1001023 | 1001024)
}

pub(crate) enum StreamEnd { Completed { finish_reason: String, tokens: Option<i64> }, Stopped(&'static str), Failed(u32) }
pub(crate) struct StreamOutcome { pub content: String, pub end: StreamEnd, pub dispatched: bool, pub audit_warning: bool }

pub(crate) async fn drain_stream(providers: &ProviderManager, binding: &StepBinding<'_>, request: ChatRequest) -> StreamOutcome {
    let mut content = String::new();
    let mut audit_warning = false;
    let mut dispatched = false;
    let end = 'drain: {
        if binding.cancellation.is_cancelled() { break 'drain StreamEnd::Stopped("task_cancelled"); }
        if Instant::now() >= binding.deadline { break 'drain StreamEnd::Stopped("generation_limit"); }
        let dispatch_state = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let opening = providers.stream_provider_bound_cancellable_controlled_with_provenance(binding.provider_id, binding.model_id, request, binding.user, binding.revision, Some(binding.provenance), binding.cancellation.clone(), binding.deadline, dispatch_state.clone());
        tokio::pin!(opening);
        let opened = tokio::select! {
            biased;
            _ = binding.cancellation.cancelled() => None,
            _ = tokio::time::sleep_until(binding.deadline) => { binding.cancellation.cancel(); None },
            opened = &mut opening => Some(opened),
        };
        dispatched = dispatch_state.load(std::sync::atomic::Ordering::Acquire);
        let opened = match opened {
            Some(opened) => opened,
            None => break 'drain StreamEnd::Stopped(if Instant::now() >= binding.deadline { "generation_limit" } else { "task_cancelled" }),
        };
        let mut stream = match opened { Ok(stream) => stream, Err(e) => break 'drain StreamEnd::Failed(e.error_code()) };
        loop {
            let item = tokio::select! {
                biased;
                _ = binding.cancellation.cancelled() => break 'drain StreamEnd::Stopped("task_cancelled"),
                _ = tokio::time::sleep_until(binding.deadline) => {
                    binding.cancellation.cancel();
                    break 'drain StreamEnd::Stopped("generation_limit");
                }
                item = stream.next() => item,
            };
            match item {
                None => break 'drain StreamEnd::Failed(DgosError::StreamInterrupted.error_code()),
                Some(Err(error)) => break 'drain if binding.cancellation.is_cancelled() { StreamEnd::Stopped("task_cancelled") } else { StreamEnd::Failed(error.error_code()) },
                Some(Ok(chunk)) => {
                    audit_warning |= chunk.audit_warning;
                    if let Some(text) = chunk.reasoning_delta.filter(|s| !s.is_empty()) {
                        if binding.emit(CoreEvent::ReasoningDelta(text)).is_err() { break 'drain StreamEnd::Stopped("task_cancelled"); }
                    }
                    if !chunk.delta.is_empty() {
                        let room = 64 * 1024 - content.len();
                        let mut end = chunk.delta.len().min(room);
                        while !chunk.delta.is_char_boundary(end) { end -= 1; }
                        let piece = &chunk.delta[..end];
                        content.push_str(piece);
                        if !piece.is_empty() && binding.emit(CoreEvent::Delta(piece.to_owned())).is_err() { break 'drain StreamEnd::Stopped("task_cancelled"); }
                        if end < chunk.delta.len() { break 'drain StreamEnd::Stopped("generation_limit"); }
                    }
                    if let Some(finish_reason) = chunk.finish_reason {
                        break 'drain StreamEnd::Completed { finish_reason, tokens: chunk.usage.map(|u| i64::from(u.total_tokens)).filter(|n| *n > 0) };
                    }
                }
            }
        }
    };
    StreamOutcome { content, end, dispatched, audit_warning }
}

pub(crate) struct LegacyGuard;
#[async_trait]
impl ExecutionGuard for LegacyGuard {
    async fn before_model(&self) -> Result<(), String> { Ok(()) }
    async fn before_tool(&self) -> Result<(), String> { Ok(()) }
    async fn approve(&self, _: &ToolProposal) -> Result<bool, String> { Err("approval_requires_parked_action".into()) }
}
