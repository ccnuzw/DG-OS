//! Durable, generation-bound advanced Codex operations. IPC cancellation never cancels dispatch.
use super::*;
use crate::codex_operations::*;
use sha2::{Digest, Sha256};

fn fingerprint(action: &CodexAction) -> Result<String, String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(action).map_err(|_| "invalid_codex_operation")?)))
}
fn unknown_receipt(input: &InspectCodexOperation) -> CodexOperationReceipt {
    CodexOperationReceipt { session_id: input.session_id.clone(), generation: input.generation.clone(), request_id: input.request_id.clone(), state: CodexOperationState::Unknown, run_id: None, fork_session_id: None }
}
fn safe_refusal(error: &str) -> bool {
    matches!(error, "engine_rejected" | "method_unsupported" | "codex_operation_unsupported" | "invalid_codex_operation" | "busy" | "session_busy" | "codex_child_unobserved" | "not_running" | "codex_terminal_unobserved" | "codex_queue_unobserved" | "codex_queue_empty" | "codex_queue_membership_changed" | "codex_turn_unobserved" | "codex_observation_incomplete" | "session_closed")
}
impl AgentManager {
    async fn codex_scope(&self, owner: &str, id: &str, generation: &str) -> Result<(SessionRecord, CodexSession), String> {
        self.ready().await?;
        let record = self.inner.records.lock().await.iter().find(|r| r.owner == owner && r.id == id).cloned().ok_or("session_not_found")?;
        let (_, backend) = self.owned(owner, id).await?;
        self.require_generation(id, generation).await?;
        let Backend::Codex(session) = backend else { return Err("codex_operation_unsupported".into()); };
        let project = self.task_project(owner, record.task_id.as_deref().ok_or("task_not_found")?).await?;
        if project.archived { return Err("project_archived".into()); }
        crate::projects::validate_directory(&project).await?;
        if record.project_id.as_deref() != Some(project.id.as_str()) || record.workspace != project.path { return Err("file_access_denied".into()); }
        Ok((record, session))
    }
    pub async fn observe_codex_operations(&self, owner: &str, input: CodexScope) -> Result<CodexOperationsSnapshot, String> {
        let admission = self.inner.admission.lock().await;
        let (_, session) = self.codex_scope(owner, &input.session_id, &input.generation).await?;
        let (lease, provider) = self.session_cancellation(&input.session_id).await?;
        drop(admission);
        let mut snapshot = tokio::select! { biased;
            _ = lease.cancelled() => return Err("session_closed".into()),
            _ = provider.cancelled() => return Err("provider_revision_changed".into()),
            result = session.observe_operations() => result?,
        };
        self.require_generation(&input.session_id, &input.generation).await?;
        snapshot.session_id = input.session_id;
        snapshot.generation = input.generation;
        Ok(snapshot)
    }
    pub async fn read_codex_child_history(&self, owner: &str, input: ReadCodexChildHistory) -> Result<CodexChildHistory, String> {
        let admission = self.inner.admission.lock().await;
        let (_, session) = self.codex_scope(owner, &input.session_id, &input.generation).await?;
        let (lease, provider) = self.session_cancellation(&input.session_id).await?;
        drop(admission);
        let history = tokio::select! { biased;
            _ = lease.cancelled() => return Err("session_closed".into()),
            _ = provider.cancelled() => return Err("provider_revision_changed".into()),
            result = session.read_child_history(&input.child_thread_id) => result?,
        };
        self.require_generation(&input.session_id, &input.generation).await?;
        Ok(history)
    }
    /// Inspect is read-only and works for suspended/closed records. A recovered pending
    /// journal is unknown, never authorization to issue the native request again.
    pub async fn inspect_codex_operation(&self, owner: &str, input: InspectCodexOperation) -> Result<Option<CodexOperationReceipt>, String> {
        self.ready().await?;
        Uuid::parse_str(&input.request_id).map_err(|_| "invalid_request_id")?;
        if !self.inner.records.lock().await.iter().any(|r| r.owner == owner && r.id == input.session_id) { return Err("session_not_found".into()); }
        let Some(journal) = self.inner.repository.codex_operation(owner, &input.request_id).await? else { return Ok(None); };
        if journal.session_id != input.session_id || journal.generation != input.generation { return Err("request_conflict".into()); }
        if journal.state == "failed" {
            let value: Value = serde_json::from_str(&journal.result).map_err(|_| "storage_failed")?;
            return Err(value["error"].as_str().unwrap_or("engine_rejected").to_owned());
        }
        if journal.result == "null" { return Ok(Some(unknown_receipt(&input))); }
        Ok(Some(serde_json::from_str(&journal.result).map_err(|_| "storage_failed")?))
    }
    pub async fn control_codex_operation(&self, owner: &str, input: ControlCodexOperation) -> Result<CodexOperationReceipt, String> {
        Uuid::parse_str(&input.request_id).map_err(|_| "invalid_request_id")?;
        input.action.validate()?;
        let hash = fingerprint(&input.action)?;
        let admission = self.inner.admission.lock().await;
        self.ready().await?;
        // A durable receipt is authoritative even when the live generation is gone.
        if let Some(journal) = self.inner.repository.codex_operation(owner, &input.request_id).await? {
            if journal.session_id != input.session_id || journal.generation != input.generation || journal.fingerprint != hash { return Err("request_conflict".into()); }
            drop(admission);
            return self.inspect_codex_operation(owner, InspectCodexOperation { session_id: input.session_id, generation: input.generation, request_id: input.request_id }).await?.ok_or_else(|| "result_unknown".into());
        }
        let (record, session) = self.codex_scope(owner, &input.session_id, &input.generation).await?;
        let control = self.session_control(&input.session_id).await?.try_lock_owned().map_err(|_| "session_busy")?;
        self.native_result_known(&input.session_id).await?;
        self.validate_handoff_target(owner, &input.session_id).await?;
        let view = session.snapshot().await;
        // Queue editing, process termination and observed child controls may be used
        // while the parent is running. Conversation replacement cannot race a run.
        let idle_only = matches!(input.action, CodexAction::Review { .. } | CodexAction::Compact {} | CodexAction::Fork { .. } | CodexAction::Revert { .. } | CodexAction::QueueStart { .. });
        if idle_only && (view.status != "idle" || self.inner.run_projections.lock().await.keys().any(|(id, _)| id == &input.session_id)) { return Err("busy".into()); }
        if idle_only && !session.automatic_runs().await.is_empty() { return Err("busy".into()); }
        if let CodexAction::Fork { new_session_id, .. } = &input.action {
            if self.inner.records.lock().await.iter().any(|r| &r.id == new_session_id) { return Err("request_conflict".into()); }
        }
        if matches!(input.action, CodexAction::QueueAdd { .. }) {
            self.capture_task_baseline(owner, record.task_id.as_deref().ok_or("task_not_found")?).await?;
        }
        let (lease, provider_lease, binding_id, uncertain) = {
            let active = self.inner.active.lock().await;
            let entry = active.get(&input.session_id).ok_or("session_closed")?;
            (entry.lease.clone(), entry.provider_lease.clone(), entry.binding.binding_id.clone(), entry.native_uncertain.clone())
        };
        let run = if input.action.starts_run() {
            let task_id = record.task_id.clone().ok_or("task_not_found")?;
            self.capture_task_baseline(owner, &task_id).await?;
            let fingerprint = self.task_fingerprint(owner, &task_id).await?;
            Some(dgos_db::development_repository::StoredRun {
                id: input.request_id.clone(), owner: owner.into(), session_id: Some(input.session_id.clone()), task_id,
                project_id: record.project_id.clone().ok_or("task_not_found")?, engine: "codex".into(), source: "native_agent".into(), runtime: None,
                status: "running".into(), started_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64,
                finished_at: None, summary: String::new(), reason: None, events: "[]".into(), fingerprint: fingerprint.clone(), start_fingerprint: fingerprint, truncated: false, request_fingerprint: hash.clone(),
            })
        } else { None };
        let configuration = self.inner.providers.read_session(owner, &input.session_id).await?;
        let crate::capabilities::SessionConfigurationView::ProviderBound { snapshot, .. } = configuration else { return Err("legacy_session_read_only".into()); };
        let mut watchers = self.inner.watchers.lock().await;
        self.reap_watchers(&mut watchers).await;
        let mut projections = self.inner.run_projections.lock().await;
        self.inner.repository.begin_codex_operation(owner, &input.session_id, &input.generation, &input.request_id, &hash).await?;
        if let Some(run) = &run {
            self.inner.repository.begin_cli_run(run, &snapshot).await?;
            projections.insert((input.session_id.clone(), input.generation.clone()), RunProjection { run_id: run.id.clone(), persistence_failed: false });
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let manager = self.clone();
        let watched = Backend::Codex(session.clone());
        let key = (input.session_id.clone(), input.generation.clone());
        let owner = owner.to_owned();
        watchers.push(tokio::spawn(async move {
            let run_id = run.as_ref().map(|run| run.id.clone());
            let dispatch = async {
                let result = if lease.is_cancelled() || provider_lease.is_cancelled() { Err("session_closed".into()) }
                    else { session.execute_operation(input.action.clone(), input.request_id.clone()).await };
                let mut receipt = CodexOperationReceipt { session_id: input.session_id.clone(), generation: input.generation.clone(), request_id: input.request_id.clone(), state: CodexOperationState::Applied, run_id, fork_session_id: None };
                let mut error = None;
                let mut fork_stored = false;
                match result {
                    Ok(native_id) => {
                        if let CodexAction::Fork { new_session_id, .. } = &input.action {
                            if let Some(native_id) = native_id {
                                receipt.fork_session_id = Some(new_session_id.clone());
                                let json = serde_json::to_string(&receipt).unwrap_or_else(|_| "null".into());
                                match manager.inner.repository.persist_codex_fork(&owner, &input.session_id, &binding_id, &input.request_id, new_session_id, &native_id, &json).await {
                                    Ok(stored) => match SessionRecord::from_stored(stored) {
                                        Ok(record) => { manager.inner.records.lock().await.push(record); fork_stored = true; },
                                        Err(_) => { receipt.state = CodexOperationState::Unknown; },
                                    },
                                    Err(_) => {
                                        // A lost SQLite commit ACK must not hide a successfully
                                        // persisted fork, and must never trigger another native fork.
                                        let durable = manager.inner.repository.codex_operation(&owner, &input.request_id).await.ok().flatten();
                                        if durable.as_ref().is_some_and(|journal| journal.state == "applied" && journal.result == json) {
                                            if let Ok(records) = manager.inner.repository.sessions().await {
                                                if let Some(stored) = records.into_iter().find(|record| record.owner == owner && record.id == *new_session_id && record.native_id.as_deref() == Some(native_id.as_str())) {
                                                    if let Ok(record) = SessionRecord::from_stored(stored) {
                                                        let mut records = manager.inner.records.lock().await;
                                                        if !records.iter().any(|old| old.id == record.id) { records.push(record); }
                                                        fork_stored = true;
                                                    }
                                                }
                                            }
                                        }
                                        if !fork_stored { receipt.state = CodexOperationState::Unknown; receipt.fork_session_id = None; }
                                    },
                                }
                            } else { receipt.state = CodexOperationState::Unknown; }
                        }
                    },
                    Err(native_error) if safe_refusal(&native_error) => { error = Some(native_error); },
                    Err(_) => { receipt.state = CodexOperationState::Unknown; },
                }
                let state = if error.is_some() { "failed" } else if receipt.state == CodexOperationState::Applied { "applied" } else { "unknown" };
                let json = if let Some(error) = &error { serde_json::json!({"error":error}).to_string() } else { serde_json::to_string(&receipt).unwrap_or_else(|_| "null".into()) };
                let persisted = fork_stored || manager.inner.repository.finish_codex_operation(&owner, &input.request_id, state, &json).await.is_ok();
                if !persisted { receipt.state = CodexOperationState::Unknown; receipt.fork_session_id = None; error = None; }
                if receipt.state == CodexOperationState::Unknown { uncertain.store(true, std::sync::atomic::Ordering::Release); }
                let accepted = if let Some(error) = &error { Err(error.clone()) } else if receipt.state == CodexOperationState::Unknown { Err("result_unknown".into()) } else { Ok(()) };
                let _ = accepted_tx.send(accepted);
                drop(control);
                let _ = tx.send(match error { Some(error) => Err(error), None => Ok(receipt) });
                persisted
            };
            let observe = async { if let Some(run) = run { crate::executions::watch(manager.clone(), watched, run, accepted_rx, key).await } else { true } };
            let (persisted, settled) = tokio::join!(dispatch, observe);
            persisted && settled
        }));
        drop(projections);
        drop(watchers);
        drop(admission);
        rx.await.map_err(|_| "result_unknown".to_owned())?
    }
}

#[cfg(all(test, unix))]
#[path = "codex_operations_manager_tests.rs"]
mod tests;
