//! Chat turns that carry MCP tools (V1-FR-006 stage 5, D022/D075).
//!
//! Genuine model streaming and host-local skill reads use the shared bounded execution loop.
//! Exact MCP proposals park for one explicit approval; dispatched receipts and batched partial
//! content are retained in account-owned chat storage, never general task audit.

use super::*;
use crate::tools::{SelectedTool, ToolAction, ToolContext, ToolSession};
use crate::TaskRecord;
use dgos_db::sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use dgos_types::ToolMessage;

const PROPOSAL_WINDOW: Duration = Duration::from_secs(120);
const TURN_WINDOW: Duration = MAX_DURATION;


/// Everything a tool turn needs beyond the plain job, fixed at send time.
pub(super) struct ToolPlan {
    pub(super) messages: Vec<WireMessage>,
    pub(super) tools: Vec<SelectedTool>,
    pub(super) window: Option<i32>,
    pub(super) generation: dgos_types::GenerationParameters,
}

/// A running tool turn, parked between a proposal and the user's decision.
pub(super) struct ToolTurn {
    user: String,
    task_id: String,
    assistant_message_id: String,
    active: Arc<Active>,
    session: ToolSession,
    sink: Arc<dyn ChatSink>,
    _guard: ReleaseGuard,
    permit: Mutex<Option<dgos_db::ActivityPermit>>,
    provider_id: String,
    providers: ProviderManager,
    model_id: String,
    revision: Option<i64>,
    pins: Vec<PinnedSkill>,
    blocked_effects: Vec<String>,
    runtime: tokio::sync::Mutex<Option<crate::skill_runtime::SkillRuntime>>,
    web_runtime: tokio::sync::Mutex<Option<crate::web::WebRuntime>>,
    contexts: Vec<crate::foundation::ResolvedContext>,
    output: dgos_types::AiOutputContract,
    foundation_receipts: Mutex<crate::foundation::FoundationReceipts>,
    source_runs: Vec<String>,
    skill_receipts_written: Mutex<usize>,
    context_receipts_written: Mutex<usize>,
    checkpoint: CheckpointWriter,
    evidence: Mutex<TerminalEvidence>,
    deadline: tokio::time::Instant,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvanceChatToolTurnInput {
    pub task_id: String,
    pub action: ToolAction,
}

/// An accepted decision whose context has been taken; hand it to
/// [`ChatManager::run_tool_action`] right away.
pub struct ToolActionJob {
    turn: Arc<ToolTurn>,
    context: ToolContext,
    action: ToolAction,
}

/// Only fixed category strings reach the task row and the renderer.
fn category(error: String) -> String {
    if !error.is_empty()
        && error.len() <= 64
        && error
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        error
    } else {
        "tool_failed".to_owned()
    }
}

impl ChatManager {
    /// Drives a started tool turn up to its first stop: a final answer, a proposal waiting for the
    /// user, or a failure. Returns once the turn is parked or finished.
    pub async fn run_tool_turn(
        &self,
        providers: &ProviderManager,
        mcp: &McpManager,
        skills: Arc<tokio::sync::RwLock<SkillsManager>>,
        job: GenerationJob,
        sink: Arc<dyn ChatSink>,
    ) {
        let sink: Arc<dyn ChatSink> = Arc::new(OrderedChatSink::new(sink, job.active.stream.clone()));
        let GenerationJob {
            task_id,
            user,
            assistant_message_id,
            plan,
            provider_id, model_id, revision, pins, skill_runtime, mut web_runtime, blocked_effects, contexts, output, inline_receipts, source_runs,
            active,
            _guard,
            _permit,
            ..
        } = job;
        let Some(plan) = plan else {
            self.finish(
                providers,
                &user,
                &task_id,
                &assistant_message_id,
                String::new(),
                End::Tool("tool_turn_misrouted".into()),
                &*sink,
            )
            .await;
            return;
        };
        if !sink.send(ChatEvent::Started { seq: 0, task_id: task_id.clone(),
        assistant_message_id: assistant_message_id.clone(), }) {
            active.stop("task_cancelled");
        }
        if let Some(web) = &mut web_runtime {
            web_runtime::attach_activities(web, self.repo.clone(), user.clone(), task_id.clone(),
                assistant_message_id.clone(), active.clone(), sink.clone());
        }
        let record = match self.task_record(&user, &task_id).await {
            Some(record) => record,
            None => {
                self.finish(
                    providers,
                    &user,
                    &task_id,
                    &assistant_message_id,
                    String::new(),
                    End::Tool("persistence_failed".into()),
                    &*sink,
                )
                .await;
                return;
            }
        };
        let messages = plan
            .messages
            .into_iter()
            .map(|message| ToolMessage {
                role: message.role,
                content: Some(message.content),
                tool_call_id: None,
                tool_calls: vec![],
                reasoning: None,
                images: message.images,
            })
            .collect();
        let context = ToolContext::new(record, messages, plan.tools, plan.window, plan.generation);
        let turn = Arc::new(ToolTurn {
            user: user.clone(),
            task_id: task_id.clone(),
            assistant_message_id,
            session: ToolSession::new(&user, active.token.clone(), context),
            active,
            sink,
            _guard,
            permit: Mutex::new(Some(_permit)),
            provider_id, model_id, revision, pins, blocked_effects,
            providers: providers.clone(),
            runtime: tokio::sync::Mutex::new(skill_runtime),
            web_runtime: tokio::sync::Mutex::new(web_runtime),
            contexts, output, foundation_receipts: Mutex::new(crate::foundation::FoundationReceipts {
                pending_context_receipts: inline_receipts, ..Default::default()
            }),
            source_runs,
            skill_receipts_written: Mutex::new(0),
            context_receipts_written: Mutex::new(0),
            checkpoint: CheckpointWriter::new(self.repo.clone(),user.clone(),task_id.clone()),
            evidence: Mutex::new(TerminalEvidence::default()),
            deadline: tokio::time::Instant::now()+TURN_WINDOW,
        });
        self.tool_turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(task_id, turn.clone());
        self.spawn_watchers(&turn);
        let taken = {
            let mut slot = turn.session.context.lock().await;
            slot.take()
        };
        if let Some(context) = taken {
            self.tool_step(providers, mcp, skills, turn, context, ToolAction::RunModel)
                .await;
        }
    }

    /// Validates the decision and takes the parked context. The take is the linearization point:
    /// a repeated, stale, expired or foreign decision finds nothing to take.
    pub async fn begin_tool_action(
        &self,
        user: &str,
        input: AdvanceChatToolTurnInput,
    ) -> Result<ToolActionJob> {
        self.check_lease()?;
        let turn = {
            let turns = self.tool_turns.lock().unwrap_or_else(|e| e.into_inner());
            turns.get(&input.task_id).cloned()
        };
        let turn = turn
            .filter(|turn| turn.user == user)
            .ok_or(ChatError::ToolTurnInvalid)?;
        if turn.active.token.is_cancelled() || tokio::time::Instant::now() >= turn.deadline {
            return Err(ChatError::ToolTurnInvalid);
        }
        let mut slot = turn.session.context.lock().await;
        if turn.active.token.is_cancelled() || tokio::time::Instant::now() >= turn.deadline {
            return Err(ChatError::ToolTurnInvalid);
        }
        if slot.as_ref().and_then(|context| context.pending.as_ref()).is_some_and(|pending| pending.expires <= tokio::time::Instant::now()) {
            return Err(ChatError::ToolTurnInvalid);
        }
        if !slot
            .as_ref()
            .is_some_and(|context| context.accepts(&input.action))
        {
            return Err(ChatError::ToolTurnInvalid);
        }
        let context = slot.take().ok_or(ChatError::ToolTurnInvalid)?;
        drop(slot);
        Ok(ToolActionJob {
            turn,
            context,
            action: input.action,
        })
    }

    pub async fn run_tool_action(
        &self,
        providers: &ProviderManager,
        mcp: &McpManager,
        skills: Arc<tokio::sync::RwLock<SkillsManager>>,
        job: ToolActionJob,
    ) {
        let ToolActionJob {
            turn,
            context,
            action,
        } = job;
        self.tool_step(providers, mcp, skills, turn, context, action).await;
    }

    async fn task_record(&self, user: &str, task_id: &str) -> Option<TaskRecord> {
        use dgos_db::entities::workbench_tasks as tasks;
        let task = tasks::Entity::find_by_id(task_id)
            .filter(tasks::Column::UserId.eq(user))
            .one(&self.db)
            .await
            .ok()??;
        Some(TaskRecord {
            task,
            skills: vec![],
            tools: vec![],
        })
    }

    async fn tool_step(
        &self,
        providers: &ProviderManager,
        mcp: &McpManager,
        skills: Arc<tokio::sync::RwLock<SkillsManager>>,
        turn: Arc<ToolTurn>,
        mut context: ToolContext,
        action: ToolAction,
    ) {
        let guard=ChatStepGuard{manager:self,providers,user:&turn.user,task:&turn.task_id,assistant:&turn.assistant_message_id,
            provider:&turn.provider_id,model:&turn.model_id,revision:turn.revision,active:&turn.active,pins:&turn.pins,blocked_effects:&turn.blocked_effects,contexts:&turn.contexts,source_runs:&turn.source_runs,sink:&*turn.sink};
        let forward=|event| {
            let sent=match event {
                crate::execution::CoreEvent::Delta(text)=>{turn.checkpoint.push(&text);turn.sink.send(ChatEvent::Delta { seq: 0, assistant_message_id:turn.assistant_message_id.clone(),text })},
                crate::execution::CoreEvent::ReasoningDelta(text)=>turn.sink.send(ChatEvent::ReasoningDelta { seq: 0, assistant_message_id:turn.assistant_message_id.clone(),text }),
                crate::execution::CoreEvent::Phase(phase)=>turn.sink.send(ChatEvent::Phase { seq: 0, task_id:turn.task_id.clone(),assistant_message_id:turn.assistant_message_id.clone(),phase }),
                crate::execution::CoreEvent::ModelSegment(round) => {
                    turn.checkpoint.begin_segment();
                    turn.sink.send(ChatEvent::ModelSegment { seq: 0, task_id: turn.task_id.clone(), assistant_message_id: turn.assistant_message_id.clone(), round })
                },
            };if !sent{turn.active.stop("task_cancelled")}sent
        };
        let mut runtime=turn.runtime.lock().await;
        let mut tools=match crate::foundation::FoundationTools::new_shared_contexts(skills,runtime.take(),&turn.contexts,&guard) {
            Ok(tools)=>tools,
            Err(error)=>{drop(runtime);self.finish_tool(&turn,turn.checkpoint.content(),End::Tool(error)).await;return;}
        };
        *tools.receipts().lock().unwrap_or_else(|e|e.into_inner())=turn.foundation_receipts.lock().unwrap_or_else(|e|e.into_inner()).clone();
        let mut web = turn.web_runtime.lock().await;
        let result = {
            let mut local = web_runtime::ChatLocalTools { foundation: &mut tools, web: web.as_mut() };
            turn.session.advance_streaming(providers,mcp,&mut context,action,&guard,&forward,turn.deadline,Some(&mut local)).await
        };
        if let Some(web) = &mut *web { web.cancel_pending().await; }
        drop(web);
        let context_receipts=tools.context_receipts();
        let skill_receipts=tools.skill_receipts();
        *turn.foundation_receipts.lock().unwrap_or_else(|e|e.into_inner())=tools.receipts().lock().unwrap_or_else(|e|e.into_inner()).clone();
        *runtime=tools.into_runtime();
        drop(runtime);
        {let mut evidence=turn.evidence.lock().unwrap_or_else(|e|e.into_inner());
            evidence.dispatched=context.dispatched();evidence.result_unknown=context.result_unknown();
            evidence.audit_warning=context.audit_warning().then(||"execution_audit_warning".into());
            evidence.tokens=(context.dispatched()&&!context.usage_unknown()).then(||context.tokens());}
        if self.persist_tool_coverage(&turn,&context_receipts,&skill_receipts).await.is_err() {
            turn.evidence.lock().unwrap_or_else(|e|e.into_inner()).audit_warning=Some("context_receipt_persistence_failed".into());
            self.finish_tool(&turn,turn.checkpoint.content(),End::Tool("persistence_failed".into())).await;return;
        }
        self.after_step(turn, context, result).await;
    }

    async fn persist_tool_coverage(&self,turn:&ToolTurn,contexts:&[dgos_types::AiContextReceipt],skills:&Value)->Result<()> {
        let context_count=*turn.context_receipts_written.lock().unwrap_or_else(|e|e.into_inner());
        self.record_inline_context(&turn.user,&turn.task_id,&contexts[context_count.min(contexts.len())..]).await?;
        *turn.context_receipts_written.lock().unwrap_or_else(|e|e.into_inner())=contexts.len();
        let count=*turn.skill_receipts_written.lock().unwrap_or_else(|e|e.into_inner());
        if let Some(receipts)=skills.as_array() {
            for receipt in &receipts[count.min(receipts.len())..] {
                let step=self.repo.record_step(&turn.user,&turn.task_id,"skill","processing_result","supplied",false,false,Some(receipt.clone())).await?;
                if !turn.sink.send(ChatEvent::Activity { seq: 0, task_id:turn.task_id.clone(),assistant_message_id:turn.assistant_message_id.clone(),step }) {turn.active.stop("task_cancelled");return Err(ChatError::ToolTurnInvalid)}
            }
            *turn.skill_receipts_written.lock().unwrap_or_else(|e|e.into_inner())=receipts.len();
        }
        Ok(())
    }

    async fn after_step(
        &self,
        turn: Arc<ToolTurn>,
        context: ToolContext,
        result: std::result::Result<Option<String>, String>,
    ) {
        let cancelled = turn.active.token.is_cancelled();
        match result {
            Ok(Some(mut content)) => {
                // D079: the reply limit of a stream (B06-31) also bounds the final answer.
                let (kept, over) = split_at_boundary(&content, MAX_REPLY_BYTES);
                let kept = kept.len();
                let end = if over {
                    content.truncate(kept);
                    End::Stopped("generation_limit")
                } else {
                    End::Completed {
                        finish_reason: "stop".into(),
                        tokens: (context.dispatched()&&!context.usage_unknown()).then(||context.tokens()),
                    }
                };
                self.finish_tool(&turn, content, end).await;
            }
            Ok(None) if !cancelled => {
                let Some(proposal) = context.proposal().cloned() else {
                    self.finish_tool(&turn, turn.checkpoint.content(), End::Tool("tool_turn_invalid".into()))
                        .await;
                    return;
                };
                // Park before announcing, so a fast approval already finds the context. A
                // cancellation that raced in finishes the turn here instead of the watcher.
                {
                    let mut slot = turn.session.context.lock().await;
                    if turn.active.token.is_cancelled() {
                        drop(slot);
                        self.finish_tool(&turn, turn.checkpoint.content(), End::Stopped(turn.active.reason()))
                            .await;
                        return;
                    }
                    *slot = Some(context);
                    if !turn.sink.send(ChatEvent::ToolProposal { seq: 0, assistant_message_id: turn.assistant_message_id.clone(),
                    proposal: proposal.clone(), }) {
                        turn.active.stop("task_cancelled");
                    }
                }
                self.arm_proposal_timer(&turn, proposal.id.clone());
            }
            Ok(None) => {
                self.finish_tool(&turn, turn.checkpoint.content(), End::Stopped(turn.active.reason()))
                    .await;
            }
            Err(error) => {
                let end = if cancelled {
                    End::Stopped(turn.active.reason())
                } else if error == "generation_limit" {
                    End::Stopped("generation_limit")
                } else {
                    End::Tool(category(error))
                };
                self.finish_tool(&turn, turn.checkpoint.content(), end).await;
            }
        }
    }

    /// The only place a tool turn ends. It leaves the table first, so a decision arriving while
    /// the terminal transaction runs is refused rather than lost.
    async fn finish_tool(&self, turn: &Arc<ToolTurn>, buffer: String, mut end: End) {
        if matches!(end, End::Completed { .. }) && crate::foundation::validate_output(&turn.output, &buffer).is_err() {
            end=End::Tool("output_validation_failed".into());
        }
        self.tool_turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&turn.task_id);
        let mut evidence=turn.evidence.lock().unwrap_or_else(|e|e.into_inner()).clone();
        if turn.checkpoint.flush().await.is_err(){evidence.audit_warning=Some("checkpoint_persistence_failed".into());end=End::Tool("persistence_failed".into());}
        if evidence.audit_warning.is_some() && matches!(end,End::Completed{..}) {end=End::Tool("execution_audit_warning".into());}
        self.finish_evidenced(
            &turn.providers,
            &turn.user,
            &turn.task_id,
            &turn.assistant_message_id,
            buffer,
            end,
            evidence,
            &*turn.sink,
        )
        .await;
        // Timers can retain the turn after completion; release admission at the terminal write.
        turn.permit.lock().unwrap_or_else(|e| e.into_inner()).take();
        // Wakes the watcher and the timers so they exit.
        turn.active.token.cancel();
    }

    /// Finishes a parked turn when it is cancelled (user stop, window destroyed, shutdown,
    /// expiry). When a step is running instead, that step sees the cancellation and finishes.
    fn spawn_watchers(&self, turn: &Arc<ToolTurn>) {
        let manager = self.clone();
        let watched = turn.clone();
        tokio::spawn(async move {
            watched.active.token.cancelled().await;
            let parked = watched.session.context.lock().await.take();
            if parked.is_some() {
                let reason = watched.active.reason();
                manager
                    .finish_tool(&watched, watched.checkpoint.content(), End::Stopped(reason))
                    .await;
            }
        });
        let overall = turn.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = overall.active.token.cancelled() => {}
                _ = tokio::time::sleep(TURN_WINDOW) => overall.active.stop("task_expired"),
            }
        });
    }

    fn arm_proposal_timer(&self, turn: &Arc<ToolTurn>, proposal_id: String) {
        let turn = turn.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = turn.active.token.cancelled() => return,
                _ = tokio::time::sleep(PROPOSAL_WINDOW) => {}
            }
            // Revoke while holding the slot so an approval cannot consume an expired proposal
            // between the check and the stop.
            let slot = turn.session.context.lock().await;
            if slot
                .as_ref()
                .and_then(|context| context.proposal())
                .is_some_and(|proposal| proposal.id == proposal_id)
            {
                turn.active.stop("confirmation_expired");
            }
        });
    }
}
