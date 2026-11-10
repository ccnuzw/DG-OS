//! Guarded in-turn Codex input; acceptance is supervised independently of IPC callers.
use super::*;

impl AgentManager {
    pub async fn steer(
        &self, owner: &str, id: &str, generation: &str, run_id: String,
        request_id: String, text: String,
    ) -> Result<SteeringReceipt, String> {
        Uuid::parse_str(&request_id).map_err(|_| "invalid_request_id")?;
        Uuid::parse_str(&run_id).map_err(|_| "invalid_run_id")?;
        if text.trim().is_empty() || text.len() > 65536 { return Err("invalid_prompt".into()); }
        let admission = self.inner.admission.lock().await;
        self.ready().await?;
        let (_, backend) = self.owned(owner, id).await?;
        self.require_generation(id, generation).await?;
        let record = self.list(owner).await.into_iter().find(|record| record.id == id).ok_or("session_not_found")?;
        let project = self.task_project(owner, record.task_id.as_deref().ok_or("task_not_found")?).await?;
        if project.archived { return Err("project_archived".into()); }
        crate::projects::validate_directory(&project).await?;
        if record.project_id.as_deref() != Some(project.id.as_str()) || record.workspace != project.path {
            return Err("file_access_denied".into());
        }
        let Backend::Codex(session) = backend else { return Err("steering_unsupported".into()); };
        let active = self.inner.active.lock().await;
        let entry = active.get(id).ok_or("session_closed")?;
        if entry.generation.to_string() != generation { return Err("stale_generation".into()); }
        if entry.lease.is_cancelled() || entry.provider_lease.is_cancelled() { return Err("session_closed".into()); }
        let lease = entry.lease.clone();
        let provider_lease = entry.provider_lease.clone();
        drop(active);
        // Register before dispatch with no cancellation point between spawn and retention.
        // Session receipts atomically guard both the run identity and duplicate requests.
        let mut watchers = self.inner.watchers.lock().await;
        self.reap_watchers(&mut watchers).await;
        let (tx, rx) = tokio::sync::oneshot::channel();
        drop(admission);
        watchers.push(tokio::spawn(async move {
            let result = if lease.is_cancelled() || provider_lease.is_cancelled() {
                Err("session_closed".into())
            } else {
                session.steer(run_id, request_id, text).await
            };
            let _ = tx.send(result);
            true
        }));
        drop(watchers);
        rx.await.map_err(|_| "result_unknown".to_owned())?
    }
}

#[cfg(test)]
#[path = "codex_interaction_tests.rs"]
mod tests;
