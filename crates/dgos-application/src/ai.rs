//! D136 public AI application service. A module never borrows chat identity or history.
use crate::{HostCapabilityService, McpConfirmationRequest, McpVerdict, ModuleMcpConfirmations};
use dgos_db::{AiRunRepository, MaintenanceGate, ModulePermission, NewAiRun};
use dgos_modules::{BoundModuleCall, CallControl, ModuleRequestOutcome, ModuleRuntimeHandle, ModuleRuntimeState};
use dgos_providers::ProviderManager;
use dgos_skills::SkillsManager;
use dgos_types::*;
use dgos_workbench::execution::{host_policy, validate_generation, AiExecutor, CoreEvent, ExecutionGuard, ExecutionRequest, ExecutionSink};
use dgos_workbench::ToolProposal;
use std::{collections::{HashMap, HashSet}, sync::{Arc, Mutex}, time::Duration};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

pub const AI_RUN_DEADLINE: Duration = Duration::from_secs(120);
pub const AI_CATALOG_DEADLINE: Duration = Duration::from_secs(10);
const INVALIDATION_INTERVAL: Duration = Duration::from_millis(50);

/// Host-only envelope. The adapter obtains both leases from host state, never IPC fields.
/// The attachment lease must be cancelled on route replacement/page loss; an account lease
/// alone cannot detect a reload of the same account.
pub struct AiInvocation {
    call: BoundModuleCall,
    account_lease: CancellationToken,
    attachment_lease: CancellationToken,
}
impl AiInvocation {
    pub fn bind(call: BoundModuleCall, session: &HostSession, account_lease: CancellationToken, attachment_lease: CancellationToken) -> DgosResult<Self> {
        if session.user_id() != call.context().user_id() || call.context().module_identity().is_none()
            || account_lease.is_cancelled() || attachment_lease.is_cancelled()
        {
            call.finish(ModuleRequestOutcome::Denied);
            return Err(DgosError::SessionInvalid);
        }
        Ok(Self { call, account_lease, attachment_lease })
    }
    fn valid(&self) -> DgosResult<()> {
        if self.account_lease.is_cancelled() { return Err(DgosError::SessionInvalid); }
        if self.attachment_lease.is_cancelled() || self.call.control().is_cancel_requested()
            || self.call.runtime().state() != ModuleRuntimeState::Ready
        { return Err(DgosError::ModuleCallCancelled); }
        Ok(())
    }
}

pub trait AiEventSink: Send + Sync { fn send(&self, event: AiEvent) -> bool; }
impl<F: Fn(AiEvent) -> bool + Send + Sync> AiEventSink for F { fn send(&self, event: AiEvent) -> bool { self(event) } }

struct EventState { seq: u64, terminal: bool }
struct Events {
    id: String,
    state: Mutex<EventState>,
    sink: Arc<dyn AiEventSink>,
    cancel: CancellationToken,
    account: CancellationToken,
    attachment: CancellationToken,
}
impl Events {
    fn emit(&self, payload: AiEventPayload) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.terminal || self.account.is_cancelled() || self.attachment.is_cancelled() { return false; }
        if matches!(&payload, AiEventPayload::Terminal { .. }) { state.terminal = true; }
        state.seq += 1;
        let delivered = self.sink.send(AiEvent { run_id: self.id.clone(), request_id: self.id.clone(), seq: state.seq, payload });
        if !delivered { self.cancel.cancel(); }
        delivered
    }
}
impl ExecutionSink for Events {
    fn send(&self, event: CoreEvent) -> bool {
        self.emit(match event {
            CoreEvent::Delta(text) => AiEventPayload::Delta { text },
            CoreEvent::ReasoningDelta(text) => AiEventPayload::ReasoningDelta { text },
            CoreEvent::Phase(phase) => AiEventPayload::Phase { phase },
        })
    }
}

struct ActiveRun { owner: String, module: String, runtime: String, cancel: CancellationToken, settling: bool }
struct ActiveGuard { registry: Arc<Mutex<HashMap<String, ActiveRun>>>, id: String }
impl Drop for ActiveGuard {
    fn drop(&mut self) { self.registry.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id); }
}

fn seal_active_run(registry: &mut HashMap<String, ActiveRun>, id: &str) -> bool {
    let Some(run) = registry.get_mut(id) else { return false; };
    let cancelled = run.cancel.is_cancelled();
    run.settling = true;
    cancelled
}

fn cancel_active_run(registry: &HashMap<String, ActiveRun>, id: &str, owner: &str, module: &str, runtime: &str) -> bool {
    let Some(run) = registry.get(id).filter(|run| !run.settling && !run.cancel.is_cancelled()
        && run.owner == owner && run.module == module && run.runtime == runtime) else { return false; };
    run.cancel.cancel();
    true
}

#[derive(Clone)]
pub struct AiService {
    capabilities: HostCapabilityService,
    repository: AiRunRepository,
    providers: Arc<ProviderManager>,
    skills: Arc<RwLock<SkillsManager>>,
    mcp: Arc<dgos_mcp::McpManager>,
    confirmations: Arc<ModuleMcpConfirmations>,
    maintenance: MaintenanceGate,
    active: Arc<Mutex<HashMap<String, ActiveRun>>>,
    stopping: CancellationToken,
}
impl AiService {
    pub fn new(capabilities: HostCapabilityService, repository: AiRunRepository, providers: Arc<ProviderManager>, skills: Arc<RwLock<SkillsManager>>, mcp: Arc<dgos_mcp::McpManager>, confirmations: Arc<ModuleMcpConfirmations>, maintenance: MaintenanceGate) -> Self {
        Self { capabilities, repository, providers, skills, mcp, confirmations, maintenance, active: Arc::new(Mutex::new(HashMap::new())), stopping: CancellationToken::new() }
    }
    /// Host-only composition entrypoint. Chat/workbench currently consume the shared
    /// executor directly and retain their domain persistence; they do not call this method.
    pub async fn run_builtin(&self, session: &HostSession, account_lease: &CancellationToken, request: ExecutionRequest, guard: &dyn ExecutionGuard, sink: &dyn ExecutionSink) -> DgosResult<dgos_workbench::execution::ExecutionResult> {
        if account_lease.is_cancelled() || self.stopping.is_cancelled()
            || request.user != session.user_id() || matches!(request.provenance.caller, BuiltinCaller::Module)
        { return Err(DgosError::SessionInvalid); }
        let _permit = self.maintenance.admit().map_err(|_| DgosError::ProviderRequestRejected)?;
        let bound = BuiltinGuard { account: account_lease, stopping: &self.stopping, inner: guard };
        let cancellation = request.cancellation.clone();
        let execution = AiExecutor::run(&self.providers, &self.mcp, request, &bound, sink);
        tokio::pin!(execution);
        let reason = tokio::select! {
            biased;
            _ = account_lease.cancelled() => Some("session_invalid"),
            _ = self.stopping.cancelled() => Some("process_interrupted"),
            result = &mut execution => return Ok(result),
        };
        cancellation.cancel();
        let mut result = execution.await;
        result.error = Some(reason.unwrap_or("process_interrupted").to_owned());
        result.content = None;
        Ok(result)
    }

    /// The future owns its BoundModuleCall until finalization. The host must drive it to completion;
    /// dropping it on navigation leaves a recoverable running audit, never permission to replay.
    pub async fn run(&self, invocation: AiInvocation, input: AiRunInput, sink: Arc<dyn AiEventSink>) -> DgosResult<AiTerminal> {
        self.run_until(invocation, input, sink, tokio::time::Instant::now() + AI_RUN_DEADLINE).await
    }

    pub(crate) async fn run_until(&self, invocation: AiInvocation, input: AiRunInput, sink: Arc<dyn AiEventSink>, deadline: tokio::time::Instant) -> DgosResult<AiTerminal> {
        let permit = match self.maintenance.admit() {
            Ok(permit) => permit,
            Err(_) => { invocation.call.finish(ModuleRequestOutcome::Denied); return Err(DgosError::ProviderRequestRejected); }
        };
        let prepared = tokio::select! {
            biased;
            _ = invocation.account_lease.cancelled() => Err(DgosError::SessionInvalid),
            _ = invocation.attachment_lease.cancelled() => Err(DgosError::ModuleCallCancelled),
            result = tokio::time::timeout_at(deadline, self.prepare(&invocation, &input)) => result.unwrap_or(Err(DgosError::Timeout)),
        };
        let (epoch, messages, context_window) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => { invocation.call.finish(ModuleRequestOutcome::Denied); return Err(error); }
        };
        if let Err(error) = invocation.valid() { return crate::settle_bound(invocation.call, Err(error)); }
        let context = invocation.call.context().clone();
        let (module, version, integrity) = match context.module_identity() {
            Some(identity) => identity,
            None => return crate::settle_bound(invocation.call, Err(DgosError::PermissionDenied)),
        };
        let run_id = context.request_id().to_owned();
        let begun = self.repository.begin(&NewAiRun {
            run_id: run_id.clone(), user_id: context.user_id().into(), module_id: module.into(),
            module_version: version.into(), module_integrity: integrity.into(),
            runtime_id: invocation.call.runtime().runtime_id().into(), request_id: run_id.clone(), authorization_epoch: epoch,
            provider_id: input.provider_id.clone(), model_id: input.model_id.clone(), provider_revision: input.provider_revision,
            skill_refs: input.skill_refs.clone(), tools: input.tools.clone(), generation: input.generation.clone(),
        }).await;
        if let Err(error) = begun { return crate::settle_bound(invocation.call, Err(error)); }
        let cancellation = CancellationToken::new();
        {
            let mut registry = self.active.lock().unwrap_or_else(|e| e.into_inner());
            if registry.contains_key(&run_id) {
                drop(registry);
                return crate::settle_bound(invocation.call, Err(DgosError::PermissionDenied));
            }
            registry.insert(run_id.clone(), ActiveRun { owner: context.user_id().into(), module: module.into(), runtime: invocation.call.runtime().runtime_id().into(), cancel: cancellation.clone(), settling: false });
        }
        let _active = ActiveGuard { registry: self.active.clone(), id: run_id.clone() };
        let events = Events { id: run_id.clone(), state: Mutex::new(EventState { seq: 0, terminal: false }), sink,
            cancel: cancellation.clone(), account: invocation.account_lease.clone(), attachment: invocation.attachment_lease.clone() };
        let guard = RunGuard {
            service: self, context: &context, runtime: invocation.call.runtime().clone(), control: invocation.call.control(),
            account: invocation.account_lease.clone(), attachment: invocation.attachment_lease.clone(), attachment_id: invocation.call.attachment_id().into(),
            cancellation: cancellation.clone(), epoch, input: &input, deadline,
        };
        events.emit(AiEventPayload::Started { provider_revision: input.provider_revision, skill_refs: input.skill_refs.clone(), tools: input.tools.clone() });
        let request = ExecutionRequest {
            user: context.user_id().into(), provider_id: input.provider_id.clone(), model_id: input.model_id.clone(),
            revision: Some(input.provider_revision), messages, tools: input.tools.clone(), generation: input.generation.clone(),
            context_window, provenance: context.provenance(), cancellation: cancellation.clone(), deadline,
        };
        let execution = AiExecutor::run(&self.providers, &self.mcp, request, &guard, &events);
        tokio::pin!(execution);
        let interrupted_reason = tokio::select! {
            biased;
            reason = guard.invalidated() => Some(reason),
            _ = tokio::time::sleep_until(deadline) => Some("generation_limit"),
            result = &mut execution => {
                let error = result.error.as_deref().map(|reason| failure(reason, result.result_unknown));
                let terminal = AiTerminal { status: if error.is_some() { if result.result_unknown { AiRunStatus::Interrupted } else if result.error.as_deref() == Some("task_cancelled") { AiRunStatus::Cancelled } else { AiRunStatus::Failed } } else { AiRunStatus::Succeeded },
                    content: result.content, error, result_unknown: result.result_unknown, audit_warning: result.audit_warning, total_tokens: result.total_tokens };
                let account = invocation.account_lease.clone();
                let attachment = invocation.attachment_lease.clone();
                return self.finish_run(invocation.call, &account, &attachment, &context, module, &run_id, &events, permit, terminal).await;
            }
        };
        cancellation.cancel();
        let settled = execution.await;
        let mut terminal = interrupted(interrupted_reason.unwrap_or("process_interrupted"), settled.result_unknown);
        terminal.audit_warning = settled.audit_warning;
        terminal.total_tokens = settled.total_tokens;
        let account = invocation.account_lease.clone();
        let attachment = invocation.attachment_lease.clone();
        self.finish_run(invocation.call, &account, &attachment, &context, module, &run_id, &events, permit, terminal).await
    }

    async fn finish_run(&self, mut call: BoundModuleCall, account: &CancellationToken, attachment: &CancellationToken, context: &HostExecutionContext, module: &str, run_id: &str, events: &Events, permit: dgos_db::ActivityPermit, mut terminal: AiTerminal) -> DgosResult<AiTerminal> {
        {
            let mut registry = self.active.lock().unwrap_or_else(|e| e.into_inner());
            if seal_active_run(&mut registry, run_id) {
                invalidate_terminal(&mut terminal, "task_cancelled");
            }
            if !call.seal_settlement() {
                invalidate_terminal(&mut terminal, "task_cancelled");
            }
        }
        if account.is_cancelled() { invalidate_terminal(&mut terminal, "session_invalid"); }
        else if attachment.is_cancelled() { invalidate_terminal(&mut terminal, "task_cancelled"); }
        let saved = self.repository.finish(context.user_id(), module, run_id, &terminal).await;
        if saved.is_err() {
            terminal.status = AiRunStatus::Interrupted;
            terminal.audit_warning = true;
            terminal.error = Some(failure("persistence_failed", terminal.result_unknown));
        } else if matches!(saved, Ok(false)) {
            terminal.status = AiRunStatus::Interrupted;
            terminal.content = None;
            terminal.error = Some(failure("process_interrupted", terminal.result_unknown));
        }
        // Settlement was sealed before storage; the persisted terminal is immutable thereafter.
        let outcome = match terminal.status {
            AiRunStatus::Succeeded => ModuleRequestOutcome::Succeeded,
            AiRunStatus::Interrupted => ModuleRequestOutcome::Interrupted,
            AiRunStatus::Cancelled => ModuleRequestOutcome::Cancelled,
            _ => ModuleRequestOutcome::Failed,
        };
        let _ = call.finish(outcome);
        events.emit(AiEventPayload::Terminal { terminal: terminal.clone() });
        drop(permit);
        Ok(terminal)
    }
    async fn prepare(&self, invocation: &AiInvocation, input: &AiRunInput) -> DgosResult<(i64, Vec<ChatMessage>, Option<i32>)> {
        invocation.valid()?;
        if self.stopping.is_cancelled() { return Err(DgosError::SessionInvalid); }
        if input.prompt.trim().is_empty() || input.prompt.len() > 16 * 1024 || input.tools.len() > 8 {
            return Err(DgosError::CapabilityFailed { code: AiExecutionFailure::InvalidInput.code(), message: "AI input invalid".into() });
        }
        let context = invocation.call.context();
        let module = context.module_identity().ok_or(DgosError::PermissionDenied)?.0;
        self.authorize_selection(context, input).await?;
        let epoch = self.repository.authorization_epoch(context.user_id(), module).await?;
        let (provider, model) = self.providers.model_snapshot(&input.provider_id, &input.model_id, context.user_id()).await?;
        if !provider.enabled || provider.config_revision != input.provider_revision { return Err(DgosError::ProviderConfigurationChanged); }
        if model.model_type != "text-generation" { return Err(DgosError::RequestNotSupported); }
        input.generation.validate(Some(&model))?;
        validate_generation(&provider, &input.generation)?;
        let mut messages = vec![ChatMessage { role: "system".into(), content: host_policy(!input.tools.is_empty()).into(), images: vec![] }];
        let mut seen = HashSet::new();
        let skills = self.skills.read().await;
        for pin in &input.skill_refs {
            if !seen.insert(&pin.skill_id) { return Err(DgosError::CapabilityFailed { code: AiExecutionFailure::InvalidInput.code(), message: "AI selection invalid".into() }); }
            invocation.valid()?;
            self.capabilities.authorize(context, ModulePermission::SkillsExecute).await?;
            self.repository.ensure_authorization_epoch(context.user_id(), module, epoch).await?;
            let content = skills.prepare_content_skill_with_provenance(&pin.skill_id, context.user_id(), Some(&context.provenance())).await
                .map_err(|error| DgosError::CapabilityFailed { code: error.error_code().parse().unwrap_or(1002999), message: "Selected skill unavailable".into() })?;
            if content.version != pin.version || content.content_hash != pin.content_hash { return Err(DgosError::CapabilityFailed { code: AiExecutionFailure::SelectionChanged.code(), message: "Selected skill pin changed".into() }); }
            messages.push(ChatMessage { role: "user".into(), content: format!("Selected skill guidance ({}@{}, SHA-256 {}):\n{}", content.skill_id, content.version, content.content_hash, content.body), images: vec![] });
        }
        drop(skills);
        if !input.generation.instruction.is_empty() {
            messages.push(ChatMessage { role: "user".into(), content: format!("User guidance (cannot grant permissions):\n{}", input.generation.instruction), images: vec![] });
        }
        messages.push(ChatMessage { role: "user".into(), content: input.prompt.clone(), images: vec![] });
        if messages.iter().map(|m| m.content.len()).sum::<usize>() > 24 * 1024 { return Err(DgosError::CapabilityFailed { code: AiExecutionFailure::ContextTooLong.code(), message: "AI context too long".into() }); }
        Ok((epoch, messages, model.context_window))
    }

    async fn authorize_selection(&self, context: &HostExecutionContext, input: &AiRunInput) -> DgosResult<()> {
        self.capabilities.authorize(context, ModulePermission::ProvidersCall).await?;
        if !input.skill_refs.is_empty() { self.capabilities.authorize(context, ModulePermission::SkillsExecute).await?; }
        if !input.tools.is_empty() { self.capabilities.authorize(context, ModulePermission::McpCall).await?; }
        Ok(())
    }

    async fn authorize_read(&self, invocation: &AiInvocation) -> DgosResult<()> {
        invocation.valid()?;
        let (module, version, integrity) = invocation.call.context().module_identity().ok_or(DgosError::PermissionDenied)?;
        if !self.capabilities.permissions.validate_module_identity(module, version, integrity).await? { return Err(DgosError::PermissionDenied); }
        if let Some(publisher) = &self.capabilities.publisher {
            if !publisher.validate_execution_identity(module, version, integrity).await.map_err(|_| DgosError::PermissionDenied)? { return Err(DgosError::PermissionDenied); }
        }
        Ok(())
    }

    pub async fn list_runs(&self, invocation: AiInvocation, input: AiRunListInput) -> DgosResult<AiRunPage> {
        let result = async {
            self.authorize_read(&invocation).await?;
            let context = invocation.call.context();
            self.repository.list(context.user_id(), context.module_identity().ok_or(DgosError::PermissionDenied)?.0, input).await
        };
        let value = tokio::time::timeout(AI_CATALOG_DEADLINE, result).await.unwrap_or(Err(DgosError::Timeout));
        settle_read(invocation, value)
    }

    pub async fn get_run(&self, invocation: AiInvocation, run_id: &str) -> DgosResult<Option<AiRunSummary>> {
        let result = async {
            self.authorize_read(&invocation).await?;
            let context = invocation.call.context();
            self.repository.get(context.user_id(), context.module_identity().ok_or(DgosError::PermissionDenied)?.0, run_id).await
        };
        let value = tokio::time::timeout(AI_CATALOG_DEADLINE, result).await.unwrap_or(Err(DgosError::Timeout));
        settle_read(invocation, value)
    }

    /// A fresh bound call may cancel only this runtime's own run. New runtime history is read-only.
    pub async fn cancel(&self, invocation: AiInvocation, run_id: &str) -> DgosResult<bool> {
        let result = async {
            self.authorize_read(&invocation).await?;
            let context = invocation.call.context();
            let module = context.module_identity().ok_or(DgosError::PermissionDenied)?.0;
            let registry = self.active.lock().unwrap_or_else(|e| e.into_inner());
            Ok(cancel_active_run(&registry, run_id, context.user_id(), module, invocation.call.runtime().runtime_id()))
        }.await;
        settle_read(invocation, result)
    }

    pub async fn list_providers(&self, invocation: AiInvocation) -> DgosResult<Vec<AiProviderEntry>> {
        let result = async {
            invocation.valid()?;
            let grant = self.capabilities.authorize(invocation.call.context(), ModulePermission::ProvidersCall).await?;
            let mut entries = Vec::new();
            for provider in self.providers.list_providers(&grant.user_id).await?.into_iter().filter(|provider| provider.enabled) {
                let revision = provider.config_revision;
                let models = self.providers.list_models(&provider.id, &grant.user_id).await?.into_iter().map(|model| AiModelEntry { model_id: model.model_id, display_name: model.model_name, kind: model.model_type }).collect();
                self.providers.ensure_revision(&provider.id, &grant.user_id, revision).await?;
                entries.push(AiProviderEntry { provider_id: provider.id, display_name: provider.name, config_revision: revision, models });
            }
            Ok(entries)
        };
        let value = tokio::time::timeout(AI_CATALOG_DEADLINE, result).await.unwrap_or(Err(DgosError::Timeout));
        settle_read(invocation, value)
    }

    pub async fn list_skills(&self, invocation: AiInvocation) -> DgosResult<Vec<AiSkillEntry>> {
        let result = async {
            invocation.valid()?;
            let grant = self.capabilities.authorize(invocation.call.context(), ModulePermission::SkillsExecute).await?;
            self.skills.read().await.ai_content_catalog(&grant.user_id).await.map_err(|e| DgosError::CapabilityFailed { code: e.error_code().parse().unwrap_or(1002999), message: "Skill catalogue unavailable".into() })
        };
        let value = tokio::time::timeout(AI_CATALOG_DEADLINE, result).await.unwrap_or(Err(DgosError::Timeout));
        settle_read(invocation, value)
    }

    pub async fn list_tools(&self, invocation: AiInvocation) -> DgosResult<Vec<AiToolEntry>> {
        let result = async {
            invocation.valid()?;
            let grant = self.capabilities.authorize(invocation.call.context(), ModulePermission::McpCall).await?;
            let mut entries = Vec::new();
            let servers = self.mcp.list_servers(&grant.user_id).await.map_err(mcp_error)?;
            for server in servers.into_iter().filter(|s| s.enabled && s.status == "ready") {
                for tool in self.mcp.list_tools(&grant.user_id, &server.id).await.map_err(mcp_error)? {
                    entries.push(AiToolEntry { tool: AiToolSelection { server_id: tool.server_id, tool_name: tool.tool_name, schema_hash: tool.schema_hash }, server_name: server.name.clone(), description: tool.description, input_schema: tool.input_schema });
                }
            }
            Ok(entries)
        };
        let value = tokio::time::timeout(AI_CATALOG_DEADLINE, result).await.unwrap_or(Err(DgosError::Timeout));
        settle_read(invocation, value)
    }

    /// Called by the trusted host confirmation adapter, never exposed as module approval.
    pub fn resolve_confirmation(&self, session: &HostSession, account_lease: &CancellationToken, confirmation_id: &str, approved: bool) -> DgosResult<()> {
        if account_lease.is_cancelled() { return Err(DgosError::SessionInvalid); }
        self.confirmations.resolve(session.user_id(), confirmation_id, approved)
    }

    pub async fn cancel_user(&self, user: &str) -> bool {
        for run in self.active.lock().unwrap_or_else(|e| e.into_inner()).values().filter(|run| run.owner == user) { run.cancel.cancel(); }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !self.active.lock().unwrap_or_else(|e| e.into_inner()).values().any(|run| run.owner == user) { break; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.is_ok()
    }
    pub async fn shutdown(&self) -> bool {
        self.stopping.cancel();
        for run in self.active.lock().unwrap_or_else(|e| e.into_inner()).values() { run.cancel.cancel(); }
        tokio::time::timeout(Duration::from_secs(5), async {
            while !self.active.lock().unwrap_or_else(|e| e.into_inner()).is_empty() { tokio::time::sleep(Duration::from_millis(20)).await; }
        }).await.is_ok()
    }
    pub async fn recover_interrupted(&self) -> DgosResult<u64> { self.repository.recover_interrupted().await }
}

struct BuiltinGuard<'a> {
    account: &'a CancellationToken,
    stopping: &'a CancellationToken,
    inner: &'a dyn ExecutionGuard,
}
impl BuiltinGuard<'_> {
    fn valid(&self) -> Result<(), String> {
        if self.account.is_cancelled() || self.stopping.is_cancelled() { Err("session_invalid".into()) } else { Ok(()) }
    }
}
#[async_trait::async_trait]
impl ExecutionGuard for BuiltinGuard<'_> {
    async fn before_model(&self) -> Result<(), String> { self.valid()?; self.inner.before_model().await?; self.valid() }
    async fn before_tool(&self) -> Result<(), String> { self.valid()?; self.inner.before_tool().await?; self.valid() }
    async fn approve(&self, proposal: &ToolProposal) -> Result<bool, String> { self.valid()?; let answer = self.inner.approve(proposal).await?; self.valid()?; Ok(answer) }
}

struct RunGuard<'a> {
    service: &'a AiService, context: &'a HostExecutionContext, runtime: ModuleRuntimeHandle, control: CallControl,
    account: CancellationToken, attachment: CancellationToken, attachment_id: String, cancellation: CancellationToken,
    epoch: i64, input: &'a AiRunInput, deadline: tokio::time::Instant,
}
impl RunGuard<'_> {
    fn valid(&self) -> Result<(), String> {
        if self.account.is_cancelled() || self.service.stopping.is_cancelled() { return Err("session_invalid".into()); }
        if self.attachment.is_cancelled() || self.cancellation.is_cancelled() || self.control.is_cancel_requested() || self.runtime.state() != ModuleRuntimeState::Ready { return Err("task_cancelled".into()); }
        Ok(())
    }
    async fn check(&self) -> Result<(), String> {
        self.valid()?;
        self.service.authorize_selection(self.context, self.input).await.map_err(|_| "permission_denied".to_owned())?;
        let module = self.context.module_identity().ok_or("permission_denied")?.0;
        self.service.repository.ensure_authorization_epoch(self.context.user_id(), module, self.epoch).await.map_err(|_| "permission_denied".to_owned())?;
        self.service.providers.ensure_revision(&self.input.provider_id, self.context.user_id(), self.input.provider_revision).await.map_err(|e| format!("provider_{}", e.error_code()))?;
        let skills = self.service.skills.read().await;
        for pin in &self.input.skill_refs { skills.validate_ai_pin(self.context.user_id(), pin).await.map_err(|error| format!("skill_{}", error.error_code()))?; }
        self.valid()
    }
    async fn admit(&self) -> Result<(), String> {
        self.check().await?;
        let module = self.context.module_identity().ok_or("permission_denied")?.0;
        self.service.repository.admit_step(self.context.user_id(), module, self.epoch, self.context.request_id()).await.map_err(|_| "permission_denied".to_owned())?;
        self.valid()?;
        self.control.mark_started();
        Ok(())
    }
    async fn invalidated(&self) -> &'static str {
        loop {
            tokio::select! {
                biased;
                _ = self.account.cancelled() => return "session_invalid",
                _ = self.attachment.cancelled() => return "task_cancelled",
                _ = self.cancellation.cancelled() => return "task_cancelled",
                _ = self.control.cancelled() => return "task_cancelled",
                _ = self.service.stopping.cancelled() => return "process_interrupted",
                _ = tokio::time::sleep(INVALIDATION_INTERVAL) => {
                    if self.runtime.state() != ModuleRuntimeState::Ready { return "task_cancelled"; }
                    let Some((module, _, _)) = self.context.module_identity() else { return "permission_denied"; };
                    if self.service.repository.ensure_authorization_epoch(self.context.user_id(), module, self.epoch).await.is_err() { return "permission_denied"; }
                }
            }
        }
    }
}
#[async_trait::async_trait]
impl ExecutionGuard for RunGuard<'_> {
    async fn before_model(&self) -> Result<(), String> { self.admit().await }
    async fn before_tool(&self) -> Result<(), String> { self.admit().await }
    async fn approve(&self, proposal: &ToolProposal) -> Result<bool, String> {
        self.check().await?;
        let module = self.context.module_identity().ok_or("permission_denied")?.0;
        let wait = self.deadline.saturating_duration_since(tokio::time::Instant::now()).min(Duration::from_secs(120));
        let verdict = self.service.confirmations.ask(McpConfirmationRequest {
            user_id: self.context.user_id(), attachment_id: &self.attachment_id, module_name: module,
            server_id: &proposal.server_id, tool_name: &proposal.tool_name, arguments: &proposal.arguments,
        }, wait).await;
        self.check().await?;
        match verdict { McpVerdict::Approved => Ok(true), McpVerdict::Rejected => Ok(false), _ => Err("confirmation_expired".into()) }
    }
}

fn settle_read<T>(invocation: AiInvocation, result: DgosResult<T>) -> DgosResult<T> {
    let result = invocation.valid().and(result);
    crate::settle_bound(invocation.call, result)
}
fn mcp_error(error: dgos_mcp::McpError) -> DgosError {
    DgosError::CapabilityFailed { code: error.error_code(), message: "MCP capability unavailable".into() }
}
fn invalidate_terminal(terminal: &mut AiTerminal, reason: &str) {
    let invalidated = interrupted(reason, terminal.result_unknown);
    terminal.status = invalidated.status;
    terminal.error = invalidated.error;
    terminal.content = None;
}
fn interrupted(reason: &str, started: bool) -> AiTerminal {
    AiTerminal { status: if started { AiRunStatus::Interrupted } else if reason == "task_cancelled" { AiRunStatus::Cancelled } else { AiRunStatus::Failed }, content: None,
        error: Some(failure(reason, started)), result_unknown: started, audit_warning: false, total_tokens: None }
}
fn failure(reason: &str, unknown: bool) -> AiFailure {
    if let Some(code) = reason.strip_prefix("provider_").and_then(|s| s.parse::<u32>().ok()) {
        return AiFailure::new("provider", code, "provider_error", unknown);
    }
    if let Some(code) = reason.strip_prefix("mcp_").and_then(|s| s.parse::<u32>().ok()) {
        let safe = if code == 1003019 { "tool_not_confirmed" } else { "mcp_error" };
        return AiFailure::new("mcp", code, safe, unknown);
    }
    if let Some(code) = reason.strip_prefix("skill_").and_then(|s| s.parse::<u32>().ok()) {
        return AiFailure::new("skill", code, "skill_error", unknown);
    }
    if reason == "mcp_audit_warning" { return AiFailure::new("storage", 1000000, "audit_write_failed", unknown); }
    if reason == "selection_changed" { return AiFailure::new("skill", AiExecutionFailure::SelectionChanged.code(), AiExecutionFailure::SelectionChanged.reason(), unknown); }
    let failure = match reason {
        "permission_denied" => return AiFailure::new("admission", 1004004, "permission_denied", unknown),
        "session_invalid" => return AiFailure::new("admission", 1008007, "session_invalid", unknown),
        "task_cancelled" => return AiFailure::new("execution", 1004028, "task_cancelled", unknown),
        "confirmation_expired" | "tool_denied" => return AiFailure::new("mcp", 1003019, "tool_not_confirmed", unknown),
        "context_budget_exceeded" => AiExecutionFailure::ContextTooLong,
        "tool_call_limit" => AiExecutionFailure::ToolCallLimit,
        "generation_limit" | "provider_timeout" => AiExecutionFailure::GenerationLimit,
        "empty_response" => AiExecutionFailure::EmptyResponse,
        "Duplicate tool selection" => AiExecutionFailure::InvalidInput,
        "Select tools from enabled, connected MCP servers" | "Selected tool changed; refresh configuration and select again" => AiExecutionFailure::SelectionChanged,
        "Unable to read MCP servers" | "Unable to read tools" => return AiFailure::new("mcp", 1003005, "mcp_error", unknown),
        "invalid_tool_action" | "invalid_tool_call" | "invalid_tool_arguments" | "invalid_tool_schema" | "tool_arguments_limit" | "unselected_tool" => AiExecutionFailure::InvalidToolProposal,
        "persistence_failed" => return AiFailure::new("storage", DgosError::DatabaseError(String::new()).error_code(), "persistence_failed", unknown),
        "process_interrupted" => return AiFailure::new("execution", DgosError::StreamInterrupted.error_code(), "process_interrupted", unknown),
        _ => AiExecutionFailure::ExecutionFailed,
    };
    AiFailure::new("execution", failure.code(), failure.reason(), unknown)
}

#[cfg(test)]
#[path = "ai_tests.rs"]
mod tests;
