//! Chat turns that carry MCP tools (V1-FR-006 stage 5, D022/D075).
//!
//! A tool turn is not streamed. The model proposes one call; the renderer shows the exact server,
//! tool and JSON arguments; `advance_chat_tool_turn` approves or denies that single proposal
//! (consumed on use, 120 s limit); the host re-checks Provider revision, permission, connection
//! and schema hash before the one MCP call. The model/MCP mechanics are the single-task code in
//! `tools.rs` (`advance_tool_context`); this module only adds the chat lifecycle around it:
//! registry slot, parked state, expiry, cancellation and the one terminal transaction.
//!
//! Arguments, results and proposals live in memory only. The chat database receives the final
//! assistant text (or an error category), never a tool round trip, so later turns do not see one.

use super::*;
use crate::tools::{SelectedTool, ToolAction, ToolContext, ToolSession};
use crate::{TaskRecord, WorkbenchManager};
use dgos_db::sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use dgos_types::ToolMessage;

const PROPOSAL_WINDOW: Duration = Duration::from_secs(120);
const TURN_WINDOW: Duration = Duration::from_secs(15 * 60);

/// Everything a tool turn needs beyond the plain job, fixed at send time.
pub(super) struct ToolPlan {
    pub(super) messages: Vec<WireMessage>,
    pub(super) tools: Vec<SelectedTool>,
    pub(super) window: Option<i32>,
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
        job: GenerationJob,
        sink: Arc<dyn ChatSink>,
    ) {
        let GenerationJob {
            task_id,
            user,
            assistant_message_id,
            plan,
            active,
            _guard,
            ..
        } = job;
        let Some(plan) = plan else {
            self.finish(
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
        if !sink.send(ChatEvent::Started {
            task_id: task_id.clone(),
            assistant_message_id: assistant_message_id.clone(),
        }) {
            active.stop("task_cancelled");
        }
        let record = match self.task_record(&user, &task_id).await {
            Some(record) => record,
            None => {
                self.finish(
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
            })
            .collect();
        let context = ToolContext::new(record, messages, plan.tools, plan.window);
        let turn = Arc::new(ToolTurn {
            user: user.clone(),
            task_id: task_id.clone(),
            assistant_message_id,
            session: ToolSession::new(&user, active.token.clone(), context),
            active,
            sink,
            _guard,
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
            self.tool_step(providers, mcp, turn, context, ToolAction::RunModel)
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
        let turn = {
            let turns = self.tool_turns.lock().unwrap_or_else(|e| e.into_inner());
            turns.get(&input.task_id).cloned()
        };
        let turn = turn
            .filter(|turn| turn.user == user)
            .ok_or(ChatError::ToolTurnInvalid)?;
        let mut slot = turn.session.context.lock().await;
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
        job: ToolActionJob,
    ) {
        let ToolActionJob {
            turn,
            context,
            action,
        } = job;
        self.tool_step(providers, mcp, turn, context, action).await;
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
        turn: Arc<ToolTurn>,
        mut context: ToolContext,
        action: ToolAction,
    ) {
        let result = WorkbenchManager::advance_tool_context(
            providers,
            mcp,
            &turn.session,
            &mut context,
            action,
        )
        .await;
        self.after_step(turn, context, result).await;
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
                        tokens: Some(context.tokens()).filter(|tokens| *tokens > 0),
                    }
                };
                self.finish_tool(&turn, content, end).await;
            }
            Ok(None) if !cancelled => {
                let Some(proposal) = context.proposal().cloned() else {
                    self.finish_tool(&turn, String::new(), End::Tool("tool_turn_invalid".into()))
                        .await;
                    return;
                };
                // Park before announcing, so a fast approval already finds the context. A
                // cancellation that raced in finishes the turn here instead of the watcher.
                {
                    let mut slot = turn.session.context.lock().await;
                    if turn.active.token.is_cancelled() {
                        drop(slot);
                        self.finish_tool(&turn, String::new(), End::Stopped(turn.active.reason()))
                            .await;
                        return;
                    }
                    *slot = Some(context);
                }
                self.arm_proposal_timer(&turn, proposal.id.clone());
                if !turn.sink.send(ChatEvent::ToolProposal {
                    assistant_message_id: turn.assistant_message_id.clone(),
                    proposal,
                }) {
                    turn.active.stop("task_cancelled");
                }
            }
            Ok(None) => {
                self.finish_tool(&turn, String::new(), End::Stopped(turn.active.reason()))
                    .await;
            }
            Err(error) => {
                let end = if cancelled {
                    End::Stopped(turn.active.reason())
                } else {
                    End::Tool(category(error))
                };
                self.finish_tool(&turn, String::new(), end).await;
            }
        }
    }

    /// The only place a tool turn ends. It leaves the table first, so a decision arriving while
    /// the terminal transaction runs is refused rather than lost.
    async fn finish_tool(&self, turn: &Arc<ToolTurn>, buffer: String, end: End) {
        self.tool_turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&turn.task_id);
        self.finish(
            &turn.user,
            &turn.task_id,
            &turn.assistant_message_id,
            buffer,
            end,
            &*turn.sink,
        )
        .await;
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
                    .finish_tool(&watched, String::new(), End::Stopped(reason))
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
