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

pub enum CoreEvent { Delta(String), ReasoningDelta(String), Phase(AiPhase), ModelSegment(u32) }
pub trait ExecutionSink: Send + Sync { fn send(&self, event: CoreEvent) -> bool; }
impl<F: Fn(CoreEvent) -> bool + Send + Sync> ExecutionSink for F {
    fn send(&self, event: CoreEvent) -> bool { self(event) }
}

/// Host-only capabilities. Definitions never grant MCP authority; the host supplies the
/// explicitly authorized immutable skill collection for this run.
#[async_trait]
pub trait LocalTools: Send {
    fn definitions(&self) -> Vec<dgos_types::ToolDefinition>;
    /// Called only after the Provider reports a real outbound dispatch, never preflight rejection.
    fn model_dispatched(&mut self) {}
    /// Host authorization/configuration and immutable resource leases, checked each model step.
    async fn revalidate(&mut self) -> Result<(), String> { Ok(()) }
    async fn dispatch(&mut self, name: &str, args: Value, remaining_context_bytes: usize) -> Result<Value, String>;
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ToolReceipt {
    pub call_id: String,
    pub outcome: String,
    pub result: Option<Value>,
    pub audit_warning: bool,
    pub result_unknown: bool,
}
#[async_trait]
pub trait ExecutionGuard: Send + Sync {
    async fn before_model(&self) -> Result<(), String>;
    async fn before_tool(&self) -> Result<(), String>;
    async fn approve(&self, proposal: &ToolProposal) -> Result<bool, String>;
    /// Persist the dispatch boundary before any external MCP await. Failure blocks dispatch.
    async fn tool_dispatched(&self, _: &ToolProposal) -> Result<(), String> { Ok(()) }
    /// Persist the observed outcome before permitting the next model/tool round.
    async fn tool_settled(&self, _: &ToolProposal, _: &ToolReceipt) -> Result<(), String> { Ok(()) }
    /// Persist only bounded displayable intermediate text, never model reasoning.
    async fn model_intermediate(&self, _: u32, _: &str, _: bool) -> Result<(), String> { Ok(()) }
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
        Self::run_with_local(providers, mcp, request, guard, sink, None).await
    }

    pub async fn run_with_local(providers: &ProviderManager, mcp: &McpManager, request: ExecutionRequest, guard: &dyn ExecutionGuard, sink: &dyn ExecutionSink, mut local: Option<&mut dyn LocalTools>) -> ExecutionResult {
        let mut result = ExecutionResult::default();
        let binding = StepBinding { user: &request.user, provider_id: &request.provider_id, model_id: &request.model_id, revision: request.revision,
            provenance: &request.provenance, cancellation: &request.cancellation, deadline: request.deadline, guard, sink };
        if request.tools.is_empty() && local.as_ref().is_none_or(|host| host.definitions().is_empty()) {
            let bytes = request.messages.iter().map(|m| m.content.len()).sum::<usize>();
            let images = request.messages.iter().map(|m| m.images.len()).sum();
            if !crate::foundation::context_budget(bytes, images, 0, request.context_window, request.generation.output_tokens()).can_send {
                result.error = Some("context_budget_exceeded".into());
                return result;
            }
            if let Some(host) = local.as_mut() {
                if let Err(error) = binding.bounded(host.revalidate()).await { result.error = Some(error); return result; }
            }
            let admission = binding.bounded(guard.before_model()).await.and_then(|_| binding.emit(CoreEvent::Phase(AiPhase::Model)));
            if let Err(error) = admission { result.error = Some(error); return result; }
            let mut wire = ChatRequest { messages: request.messages, stream: Some(true), ..Default::default() };
            request.generation.apply(&mut wire);
            let drained = drain_stream(providers, &binding, wire).await;
            if drained.dispatched { if let Some(host) = local.as_mut() { host.model_dispatched(); } }
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
            if let Some(host) = local.as_mut() {
                if let Err(error) = binding.bounded(host.revalidate()).await { result.error = Some(error); }
            }
            return result;
        }
        if request.tools.len() > 8 { result.error = Some("tool_call_limit".into()); return result; }
        let tools = if request.tools.is_empty() { vec![] } else {
            match binding.bounded(resolve_selected_tools(mcp, &request.user, request.tools)).await {
                Ok(tools) => tools,
                Err(error) => { result.error = Some(error); return result; }
            }
        };
        let partial = std::sync::Mutex::new(String::new());
        let tool_sink = |event: CoreEvent| {
            match &event {
                CoreEvent::ModelSegment(_) => partial.lock().unwrap_or_else(|error| error.into_inner()).clear(),
                CoreEvent::Delta(text) => {
                    let mut output = partial.lock().unwrap_or_else(|error| error.into_inner());
                    let room = (64usize * 1024).saturating_sub(output.len());
                    output.push_str(prefix(text, room));
                }
                _ => {}
            }
            sink.send(event)
        };
        let binding = StepBinding { sink: &tool_sink, ..binding };
        let mut context = ToolContext { record: None, messages: request.messages.into_iter().map(|m| ToolMessage {
            role: m.role, content: Some(m.content), tool_call_id: None, tool_calls: vec![], reasoning: None, images: m.images,
        }).collect(), tools, window: request.context_window, generation: request.generation, pending: None, calls: HashSet::new(), tokens: 0,
            generated_bytes: 0, model_round: 0, retained_intermediate_bytes: 0,
            receipts: vec![], retained_results: vec![], dispatched: false, result_unknown: false, audit_warning: false, usage_unknown: false };
        let mut action = ToolAction::RunModel;
        let mut audit_warning = false;
        let mut unknown = false;
        let terminal = loop {
            let step = match local.as_mut() {
                Some(host) => advance_tool_context_with_local(providers, mcp, &binding, &mut context, action, &mut audit_warning, &mut unknown, Some(&mut **host)).await,
                None => advance_tool_context(providers, mcp, &binding, &mut context, action, &mut audit_warning, &mut unknown).await,
            };
            match step {
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
        result.total_tokens = (context.dispatched && !context.usage_unknown).then_some(context.tokens);
        result.audit_warning = audit_warning;
        result.result_unknown = unknown;
        match terminal {
            Ok(content) => result.content = Some(content),
            Err(error) => {
                result.error = Some(error);
                result.content = Some(std::mem::take(&mut *partial.lock().unwrap_or_else(|error| error.into_inner()))).filter(|text| !text.is_empty());
            }
        }
        result
    }
}

/// One model step, optionally consuming the exact pending approval first. Both parked workbench
/// APIs and the callback executor use this function; the original assistant reasoning stays in memory.
pub(crate) async fn advance_tool_context(
    providers: &ProviderManager, mcp: &McpManager, binding: &StepBinding<'_>, context: &mut ToolContext,
    action: ToolAction, audit_warning: &mut bool, unknown: &mut bool,
) -> Result<Option<String>, String> {
    advance_tool_context_with_local(providers, mcp, binding, context, action, audit_warning, unknown, None).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn advance_tool_context_with_local(
    providers: &ProviderManager, mcp: &McpManager, binding: &StepBinding<'_>, context: &mut ToolContext,
    action: ToolAction, audit_warning: &mut bool, unknown: &mut bool, mut local: Option<&mut dyn LocalTools>,
) -> Result<Option<String>, String> {
    if *unknown || context.result_unknown { return Err("tool_result_unknown".into()); }
    if *audit_warning { return Err("tool_audit_warning".into()); }
    if !context.accepts(&action) { return Err("invalid_tool_action".into()); }
    if let Some(host) = local.as_mut() { binding.bounded(host.revalidate()).await?; }
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
            arguments: pending.public.arguments.clone(), call_id: Some(pending.public.id.clone()), task_ref: Some(binding.provenance.request_id.clone()),
            timeout_ms: Some(30_000), confirmation_token: None };
        request.confirmation_token = Some(binding.bounded(async { mcp.confirm_call(binding.user, &request).await.map_err(|e| format!("mcp_{}", e.error_code())) }).await?);
        binding.bounded(binding.guard.before_tool()).await?;
        binding.bounded(binding.guard.tool_dispatched(&pending.public)).await?;
        *unknown = true;
        context.result_unknown = true;
        context.dispatched = true;
        let result = {
            let call = async { mcp.call_tool_at_schema_cancellable_with_provenance(binding.user, request, &selected.tool.schema_hash, binding.cancellation, binding.provenance).await.map_err(|e| format!("mcp_{}", e.error_code())) };
            tokio::pin!(call);
            let result = tokio::select! {
                biased;
                _ = tokio::time::sleep_until(binding.deadline) => { binding.cancellation.cancel(); None }
                result = &mut call => Some(result),
            };
            match result { Some(result) => result, None => call.await }
        };
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                let receipt = ToolReceipt { call_id: pending.public.id.clone(), outcome: "unknown".into(), result: None, audit_warning: false, result_unknown: true };
                context.receipts.push(receipt.clone());
                if binding.guard.tool_settled(&pending.public, &receipt).await.is_err() { *audit_warning = true; }
                return Err(error);
            }
        };
        *audit_warning |= result.audit_warning;
        *unknown = !matches!(result.outcome.as_str(), "success" | "error") || (result.outcome == "error" && !result.content.is_array());
        context.result_unknown = *unknown;
        let raw = json!({"isError": result.is_error, "content": result.content, "structuredContent": result.structured_content}).to_string();
        let view = retain_tool_result(context, raw);
        let receipt = ToolReceipt { call_id: pending.public.id.clone(), outcome: result.outcome.clone(), result: Some(view.clone()), audit_warning: result.audit_warning, result_unknown: *unknown };
        context.receipts.push(receipt.clone());
        if binding.guard.tool_settled(&pending.public, &receipt).await.is_err() { *audit_warning = true; return Err("tool_journal_failed".into()); }
        if *unknown { return Err(format!("mcp_{}", match result.outcome.as_str() { "timeout" => 1003006, "cancelled" => 1003016, _ => 1003005 })); }
        if result.audit_warning { return Err("mcp_audit_warning".into()); }
        if binding.cancellation.is_cancelled() { return Err("task_cancelled".into()); }
        binding.emit(CoreEvent::Phase(AiPhase::ProcessingResult))?;
        context.messages.push(pending.assistant);
        context.messages.push(tool_result_message(pending.call.id, view));
    }
    loop {
        let mut definitions: Vec<_> = context.tools.iter().map(|tool| tool.definition.clone()).collect();
        if let Some(host) = local.as_ref() { definitions.extend(host.definitions()); }
        if !context.retained_results.is_empty() { definitions.push(result_read_definition()); }
        let mut names = HashSet::new();
        if definitions.iter().any(|definition| !names.insert(definition.function.name.clone())) { return Err("duplicate_tool_definition".into()); }
        let request = ToolChatRequest { messages: context.messages.clone(), tools: definitions.clone(), max_tokens: context.generation.output_tokens(),
            temperature: context.generation.temperature, reasoning: context.generation.reasoning.clone() };
        validate_budget(&request, context.window)?;
        if let Some(host) = local.as_mut() { binding.bounded(host.revalidate()).await?; }
        context.model_round = context.model_round.saturating_add(1);
        binding.emit(CoreEvent::ModelSegment(context.model_round))?;
        let mut round_dispatched = false;
        let streamed = drain_tool_stream(providers, binding, request, unknown, &mut round_dispatched, audit_warning, &mut context.generated_bytes).await;
        context.dispatched |= round_dispatched;
        if round_dispatched { if let Some(host) = local.as_mut() { host.model_dispatched(); } }
        let (response, usage_unknown) = match streamed {
            Ok(response) => response,
            // An explicit rejection resolves execution uncertainty, not usage presence.
            // No complete usage receipt was returned for any dispatched failed round.
            Err(error) => { context.usage_unknown |= round_dispatched; return Err(error); }
        };
        *unknown = false;
        context.usage_unknown |= usage_unknown || !response.usage.total_tokens_reported;
        context.tokens = context.tokens.saturating_add(i64::from(response.usage.total_tokens));
        *audit_warning |= response.audit_warning;
        if let Some(host) = local.as_mut() { binding.bounded(host.revalidate()).await?; }
        if response.audit_warning { return Err("provider_audit_warning".into()); }
        if response.message.tool_calls.is_empty() {
            binding.emit(CoreEvent::Phase(AiPhase::FinalAnswer))?;
            return response.message.content.filter(|s| !s.trim().is_empty()).map(Some).ok_or_else(|| "empty_response".into());
        }
        if response.message.tool_calls.len() != 1 || context.calls.len() >= 8 { return Err("tool_call_limit".into()); }
        let intermediate = response.message.content.as_deref().unwrap_or("");
        let retained = prefix(intermediate, (8usize * 1024).saturating_sub(context.retained_intermediate_bytes));
        context.retained_intermediate_bytes += retained.len();
        binding.guard.model_intermediate(context.model_round, retained, retained.len() < intermediate.len()).await?;
        let assistant = response.message;
        let call = assistant.tool_calls[0].clone();
        if call.kind != "function" || call.id.is_empty() || call.id.len() > 128 || !call.id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') || !context.calls.insert(call.id.clone()) {
            return Err("invalid_tool_call".into());
        }
        if call.function.arguments.len() > 16 * 1024 { return Err("tool_arguments_limit".into()); }
        let arguments: Value = serde_json::from_str(&call.function.arguments).map_err(|_| "invalid_tool_arguments")?;
        let definition = definitions.iter().find(|definition| definition.function.name == call.function.name).ok_or("unselected_tool")?;
        if !arguments.is_object() || !jsonschema::validator_for(&definition.function.parameters).map_err(|_| "invalid_tool_schema")?.is_valid(&arguments) { return Err("invalid_tool_arguments".into()); }
        if let Some(selected) = context.tools.iter().position(|tool| tool.definition.function.name == call.function.name) {
            let tool = &context.tools[selected];
            context.pending = Some(PendingProposal { public: ToolProposal { id: uuid::Uuid::new_v4().to_string(), server_id: tool.tool.server_id.clone(), server_name: tool.server_name.clone(),
                tool_name: tool.tool.tool_name.clone(), schema_hash: tool.tool.schema_hash.clone(), arguments, expires_at: chrono::Utc::now().timestamp() + 120 }, selected, call, assistant,
                expires: Instant::now() + Duration::from_secs(120) });
            return Ok(None);
        }
        binding.bounded(binding.guard.before_model()).await?;
        binding.emit(CoreEvent::Phase(AiPhase::ProcessingResult))?;
        context.messages.push(assistant);
        let remaining = remaining_tool_bytes(context, &definitions);
        let value = if call.function.name == "dgos_tool_result_read" {
            read_tool_result(context, &arguments)?
        } else {
            let host = local.as_mut().ok_or("unselected_tool")?;
            binding.bounded(host.dispatch(&call.function.name, arguments, remaining)).await?
        };
        binding.bounded(binding.guard.before_model()).await?;
        if let Some(host) = local.as_mut() { binding.bounded(host.revalidate()).await?; }
        if crate::foundation::tool_result_content_bytes(&value)? > remaining { return Err("context_budget_exceeded".into()); }
        context.messages.push(tool_result_message(call.id.clone(), value.clone()));
        context.receipts.push(ToolReceipt { call_id: call.id, outcome: "host_read".into(), result: Some(value), audit_warning: false, result_unknown: false });
    }
}

fn tool_result_message(id: String, value: Value) -> ToolMessage {
    ToolMessage { role: "tool".into(), content: Some(value.to_string()), tool_call_id: Some(id), tool_calls: vec![], reasoning: None, images: vec![] }
}

fn remaining_tool_bytes(context: &ToolContext, definitions: &[dgos_types::ToolDefinition]) -> usize {
    let request = ToolChatRequest { messages: context.messages.clone(), tools: definitions.to_vec(), max_tokens: context.generation.output_tokens(),
        temperature: context.generation.temperature, reasoning: context.generation.reasoning.clone() };
    let used = serde_json::to_vec(&request).map_or(usize::MAX, |bytes| bytes.len())
        .saturating_add(context.messages.iter().filter_map(|message| message.reasoning.as_ref()).map(|reasoning| reasoning.encoded_len()).sum::<usize>());
    let actual_schema = serde_json::to_vec(definitions).map_or(usize::MAX, |bytes| bytes.len().saturating_sub(2));
    let reserved_schema = crate::foundation::tool_schema_bytes(definitions).unwrap_or(usize::MAX);
    let used = used.saturating_add(reserved_schema.saturating_sub(actual_schema));
    let images = context.messages.iter().map(|message| message.images.len()).sum();
    crate::context_text_ceiling(context.window, context.generation.output_tokens(), images).unwrap_or(0).saturating_sub(used).saturating_sub(1024)
}

fn prefix(text: &str, limit: usize) -> &str {
    let mut end = limit.min(text.len());
    while !text.is_char_boundary(end) { end -= 1; }
    &text[..end]
}

fn retain_tool_result(context: &mut ToolContext, raw: String) -> Value {
    if raw.len() <= 4096 { return serde_json::from_str(&raw).expect("host encoded tool result"); }
    let retained = context.retained_results.iter().map(String::len).sum::<usize>();
    let stored = prefix(&raw, (256usize * 1024).saturating_sub(retained)).to_owned();
    let index = context.retained_results.len();
    let retained_bytes = stored.len();
    if retained_bytes > 0 { context.retained_results.push(stored); }
    json!({"omitted":true,"total_bytes":raw.len(),"preview":prefix(&raw,4096),"preview_bytes":prefix(&raw,4096).len(),
        "retained_bytes":retained_bytes,"unavailable_bytes":raw.len()-retained_bytes,"result_index":(retained_bytes>0).then_some(index),
        "retrieval":"Only the preview has been consumed. Use dgos_tool_result_read with result_index and byte offset for retained portions; unavailable bytes cannot be read. In-memory results expire with this run."})
}

pub(crate) fn result_read_definition() -> dgos_types::ToolDefinition {
    dgos_types::ToolDefinition { kind: "function".into(), function: dgos_types::ToolFunction { name: "dgos_tool_result_read".into(),
        description: "Read at most 4096 bytes from an explicitly omitted tool result retained in this run. Does not fetch external resources.".into(),
        parameters: json!({"type":"object","properties":{"result_index":{"type":"integer","minimum":0},"offset":{"type":"integer","minimum":0}},"required":["result_index","offset"],"additionalProperties":false}) } }
}

fn read_tool_result(context: &ToolContext, arguments: &Value) -> Result<Value, String> {
    let index = arguments["result_index"].as_u64().and_then(|value| usize::try_from(value).ok()).ok_or("invalid_tool_arguments")?;
    let offset = arguments["offset"].as_u64().and_then(|value| usize::try_from(value).ok()).ok_or("invalid_tool_arguments")?;
    let text = context.retained_results.get(index).ok_or("tool_result_unavailable")?;
    if offset > text.len() || !text.is_char_boundary(offset) { return Err("invalid_tool_result_offset".into()); }
    let piece = prefix(&text[offset..], 4096);
    Ok(json!({"result_index":index,"offset":offset,"content":piece,"next_offset":offset+piece.len(),"retained_bytes":text.len(),"more_retained":offset+piece.len()<text.len()}))
}

fn tool_stream_error(error: DgosError) -> String {
    match error {
        DgosError::ProviderOutputLimit => "generation_limit".into(),
        DgosError::ProviderToolArgumentsInvalid => "invalid_tool_arguments".into(),
        DgosError::ProviderToolCallLimit => "tool_call_limit".into(),
        error => format!("provider_{}", error.error_code()),
    }
}

async fn drain_tool_stream(providers: &ProviderManager, binding: &StepBinding<'_>, request: ToolChatRequest,
    unknown: &mut bool, any_dispatched: &mut bool, audit_warning: &mut bool, generated_bytes: &mut usize) -> Result<(dgos_types::ToolChatResponse, bool), String> {
    binding.bounded(binding.guard.before_model()).await?;
    binding.emit(CoreEvent::Phase(AiPhase::Model))?;
    let dispatch = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let opening = providers.stream_provider_tools_bound_cancellable_controlled_with_provenance(binding.provider_id, binding.model_id, request,
        binding.user, binding.revision, Some(binding.provenance), binding.cancellation.clone(), binding.deadline, dispatch.clone());
    let opened = binding.bounded(async { opening.await.map_err(tool_stream_error) }).await;
    let sent = dispatch.load(std::sync::atomic::Ordering::Acquire);
    *any_dispatched |= sent;
    *unknown = sent;
    let mut stream = match opened {
        Ok(stream) => stream,
        Err(error) => {
            if error.strip_prefix("provider_").and_then(|code| code.parse().ok()).is_some_and(provider_rejection) { *unknown = false; }
            return Err(error);
        }
    };
    loop {
        let next = binding.bounded(async { stream.next().await.ok_or_else(|| "provider_stream_interrupted".to_string())?
            .map_err(tool_stream_error) }).await;
        let event = match next {
            Ok(event) => event,
            Err(error) => {
                if error.strip_prefix("provider_").and_then(|code| code.parse().ok()).is_some_and(provider_rejection) { *unknown = false; }
                return Err(error);
            }
        };
        match event {
            dgos_types::ToolStreamEvent::AuditWarning => { *audit_warning = true; }
            dgos_types::ToolStreamEvent::Delta(text) => {
                let (text, over) = bound_generated(text, generated_bytes);
                if !text.is_empty() { binding.emit(CoreEvent::Delta(text))?; }
                if over { return Err("generation_limit".into()); }
            }
            dgos_types::ToolStreamEvent::ReasoningDelta(text) => {
                let (text, over) = bound_generated(text, generated_bytes);
                if !text.is_empty() { binding.emit(CoreEvent::ReasoningDelta(text))?; }
                if over { return Err("generation_limit".into()); }
            }
            dgos_types::ToolStreamEvent::Complete { response, usage_unknown } => {
                *unknown = false;
                return Ok((response, usage_unknown));
            }
        }
    }
}

pub(crate) enum ModelRequest { Text(ChatRequest) }
pub(crate) enum ModelResponse { Text(dgos_types::ChatResponse) }

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


fn bound_generated(mut text: String, used: &mut usize) -> (String, bool) {
    let room = (64usize * 1024).saturating_sub(*used);
    *used = used.saturating_add(text.len());
    let over = text.len() > room;
    if over { text.truncate(prefix(&text, room).len()); }
    (text, over)
}
pub(crate) async fn drain_stream(providers: &ProviderManager, binding: &StepBinding<'_>, request: ChatRequest) -> StreamOutcome {
    let mut content = String::new();
    let mut audit_warning = false;
    let mut dispatched = false;
    let mut generated_bytes = 0usize;
    if binding.emit(CoreEvent::ModelSegment(1)).is_err() {
        return StreamOutcome { content, end: StreamEnd::Stopped("task_cancelled"), dispatched, audit_warning };
    }
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
                        let (text, over) = bound_generated(text, &mut generated_bytes);
                        if !text.is_empty() && binding.emit(CoreEvent::ReasoningDelta(text)).is_err() { break 'drain StreamEnd::Stopped("task_cancelled"); }
                        if over { break 'drain StreamEnd::Stopped("generation_limit"); }
                    }
                    if !chunk.delta.is_empty() {
                        let (text, over) = bound_generated(chunk.delta, &mut generated_bytes);
                        content.push_str(&text);
                        if !text.is_empty() && binding.emit(CoreEvent::Delta(text)).is_err() { break 'drain StreamEnd::Stopped("task_cancelled"); }
                        if over { break 'drain StreamEnd::Stopped("generation_limit"); }
                    }
                    if let Some(finish_reason) = chunk.finish_reason {
                        break 'drain StreamEnd::Completed { finish_reason, tokens: chunk.usage.filter(|usage| usage.total_tokens_reported).map(|usage| i64::from(usage.total_tokens)) };
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

#[cfg(test)]
mod result_tests {
    use super::*;

    fn new_context() -> ToolContext {
        ToolContext { record: None, messages: vec![], tools: vec![], window: None, generation: Default::default(),
            pending: None, calls: HashSet::new(), tokens: 0, receipts: vec![], retained_results: vec![],
            generated_bytes: 0, model_round: 0, retained_intermediate_bytes: 0,
            dispatched: false, result_unknown: false, audit_warning: false, usage_unknown: false }
    }

    #[test]
    fn generated_budget_is_shared_across_reasoning_text_and_model_rounds() {
        let mut used = 0;
        assert_eq!(bound_generated("r".repeat(32 * 1024), &mut used).1, false);
        assert_eq!(bound_generated("x".repeat(32 * 1024 - 2), &mut used).1, false);
        let (piece, over) = bound_generated("界next".into(), &mut used);
        assert!(piece.is_empty(), "remaining bytes do not split a UTF-8 character");
        assert!(over);
        assert!(bound_generated("another round".into(), &mut used).0.is_empty());
    }

    #[test]
    fn large_results_disclose_omission_and_only_explicit_reads_consume_more() {
        let mut context = new_context();
        let raw = json!({"content":"界".repeat(4000)}).to_string();
        let view = retain_tool_result(&mut context, raw.clone());
        assert_eq!(view["omitted"], true);
        assert_eq!(view["total_bytes"], raw.len());
        assert!(view["preview"].as_str().unwrap().len() <= 4096);
        assert!(!view["preview"].as_str().unwrap().contains(&"界".repeat(2000)));
        let offset = view["preview_bytes"].as_u64().unwrap();
        let next = read_tool_result(&context, &json!({"result_index":0,"offset":offset})).unwrap();
        assert_eq!(next["offset"], offset);
        assert!(next["content"].as_str().unwrap().len() <= 4096);
        assert!(read_tool_result(&context, &json!({"result_index":1,"offset":0})).is_err());
        assert!(read_tool_result(&context, &json!({"result_index":0,"offset":raw.len()+1})).is_err());
    }

    #[test]
    fn retained_results_have_a_run_wide_memory_ceiling_and_no_cross_run_lookup() {
        let mut context = new_context();
        let view = retain_tool_result(&mut context, json!({"content":"x".repeat(300 * 1024)}).to_string());
        assert_eq!(view["retained_bytes"], 256 * 1024);
        assert!(view["unavailable_bytes"].as_u64().unwrap() > 0);
        let second = retain_tool_result(&mut context, json!({"content":"y".repeat(8192)}).to_string());
        assert_eq!(second["retained_bytes"], 0);
        assert!(second["result_index"].is_null());
        assert_eq!(context.retained_results.iter().map(String::len).sum::<usize>(), 256 * 1024);
        assert!(read_tool_result(&new_context(), &json!({"result_index":0,"offset":0})).is_err());
    }
}

#[cfg(test)]
mod tool_stream_error_tests {
    use super::*;

    #[test]
    fn semantic_stream_rejections_preserve_existing_host_codes() {
        assert_eq!(tool_stream_error(DgosError::ProviderOutputLimit), "generation_limit");
        assert_eq!(tool_stream_error(DgosError::ProviderToolArgumentsInvalid), "invalid_tool_arguments");
        assert_eq!(tool_stream_error(DgosError::ProviderToolCallLimit), "tool_call_limit");
        assert_eq!(tool_stream_error(DgosError::ProviderResponseInvalid), "provider_1001025");
        for error in [DgosError::ProviderOutputLimit, DgosError::ProviderToolArgumentsInvalid, DgosError::ProviderToolCallLimit] {
            assert_eq!(error.error_code(), 1001025, "public provider error codes remain unchanged");
        }
    }
}
