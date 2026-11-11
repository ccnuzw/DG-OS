use super::*;

#[derive(Serialize)]
pub struct StartResolution {
    pub status: &'static str,
    pub session: Option<OwnedSessionView>,
}

impl AgentManager {
    pub(super) async fn read_session_record(&self, owner: &str, id: &str) -> Result<Option<SessionRecord>, String> {
        if let Some(record) = self.inner.records.lock().await.iter().find(|record| record.owner == owner && record.id == id).cloned() {
            return Ok(Some(record));
        }
        let Some(stored) = self.inner.repository.agent_session(owner, id).await? else { return Ok(None); };
        let record = SessionRecord::from_stored(stored)?;
        let mut records = self.inner.records.lock().await;
        if !records.iter().any(|item| item.id == id) { records.push(record.clone()); }
        Ok(Some(record))
    }

    /// Bounded, single-flight observation. It never starts or signals an old process.
    pub(super) async fn reconcile_process_cleanup(&self, force: bool) -> Result<(), String> {
        let mut last = if force { self.inner.cleanup_observed.lock().await } else {
            let Ok(guard) = self.inner.cleanup_observed.try_lock() else { return Ok(()); };
            guard
        };
        if !force && last.is_some_and(|last| last.elapsed() < std::time::Duration::from_secs(1)) { return Ok(()); }
        self.inner.providers.recheck_cleanup().await?;
        let retained: Vec<_> = self.inner.retained.lock().await.iter().map(|entry| (entry._owner.clone(), entry._id.clone(), entry.slot)).collect();
        for (owner, id, slot) in retained {
            if self.slot_is_settled(&owner, &id).await {
                self.inner.retained.lock().await.retain(|entry| entry.slot != slot);
                self.release_slot(&id, slot).await;
            }
        }
        let active: Vec<_> = self.inner.active.lock().await.iter().map(|(id, entry)| (id.clone(), entry.binding.clone(), entry.slot)).collect();
        for (id, binding, slot) in active {
            let owner = self.inner.records.lock().await.iter().find(|record| record.id == id).map(|record| record.owner.clone());
            if let Some(owner) = owner {
                let configuration = self.inner.repository.read_cli_session_configuration(&owner, &id).await?;
                if configuration.binding.is_some_and(|current| current.binding_id == binding.binding_id && current.cleanup_state == "stopped") {
                    self.mark_cleanup_settled(&id, slot).await;
                    self.retire_settled_session(&id).await;
                }
            }
        }
        self.reconcile_inactive_slots().await;
        *last = Some(tokio::time::Instant::now());
        Ok(())
    }

    /// Startup reconciliation and late-create fencing share the admission lock.
    /// A durable reservation is never confused with an absent in-memory projection.
    pub async fn reconcile_start(&self, owner: &str, id: &str, task_id: &str, lease: &CancellationToken) -> Result<StartResolution, String> {
        let request_id = Uuid::parse_str(id).map_err(|_| "invalid_request_id")?;
        self.ready().await?;
        let _admission = tokio::select! {
            biased;
            _ = lease.cancelled() => return Err("session_invalid".into()),
            guard = self.inner.admission.lock() => guard,
        };
        if self.inner.stop.is_cancelled() || lease.is_cancelled() { return Err("session_invalid".into()); }
        self.inner.repository.task(owner, task_id).await?;
        self.reconcile_process_cleanup(true).await?;
        if let Some(record) = self.read_session_record(owner, id).await? {
            if record.task_id.as_deref() != Some(task_id) { return Err("session_not_found".into()); }
            let result = match self.snapshot(owner, id).await {
                Ok(session) => StartResolution { status: if session.view.status == "closed" { "closed" } else { "ready" }, session: Some(session) },
                Err(error) if error == "cleanup_pending" => StartResolution { status: "recovering", session: None },
                Err(error) => return Err(error),
            };
            if lease.is_cancelled() { return Err("session_invalid".into()); }
            return Ok(result);
        }
        if self.inner.slots.lock().await.contains_key(id) || self.inner.active.lock().await.contains_key(id) {
            return Ok(StartResolution { status: "recovering", session: None });
        }
        let mut cancelled = self.inner.cancelled_starts.lock().await;
        if lease.is_cancelled() { return Err("session_invalid".into()); }
        if !cancelled.get(owner).is_some_and(|ids| ids.contains(&request_id)) && cancelled.values().map(HashSet::len).sum::<usize>() >= 4096 { return Err("request_capacity".into()); }
        cancelled.entry(owner.to_owned()).or_default().insert(request_id);
        Ok(StartResolution { status: "not_started", session: None })
    }
}

impl AgentManager {
    pub(crate) async fn close_project_runs(&self, owner: &str, project_id: &str, lease: &CancellationToken) -> Result<(), String> {
        self.reconcile_process_cleanup(true).await?;
        let sessions: Vec<_> = self.inner.records.lock().await.iter().filter(|record| record.owner == owner && record.project_id.as_deref() == Some(project_id)).map(|record| record.id.clone()).collect();
        for id in sessions {
            if lease.is_cancelled() { return Err("session_invalid".into()); }
            let active = self.inner.active.lock().await.contains_key(&id);
            if active && !self.close_session_admitted(owner, &id).await? { return Err("cleanup_pending".into()); }
            if self.inner.retained.lock().await.iter().any(|entry| entry._id == id) { return Err("cleanup_pending".into()); }
        }
        let processes: Vec<_> = self.inner.host.lock().await.iter().filter(|(_, process)| process.owner == owner && process.project_id == project_id).map(|(id, _)| id.clone()).collect();
        for id in processes {
            if lease.is_cancelled() { return Err("session_invalid".into()); }
            let stopped = self.stop_development_process(owner, &id).await?;
            if matches!(stopped.status.as_str(), "unknown" | "cleanup_pending") { return Err("cleanup_pending".into()); }
        }
        self.reconcile_process_cleanup(true).await
    }
}
