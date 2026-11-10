use super::*;

impl AgentManager {
    /// Joining completed tasks is part of normal operation, not only shutdown. A
    /// failed result remains sticky after its handle has been removed.
    pub(crate) async fn reap_completed_tasks(&self) {
        {
            let mut watchers = self.inner.watchers.lock().await;
            self.reap_watchers(&mut watchers).await;
        }
        let mut cleanup = self.inner.cleanup_watchers.lock().await;
        self.reap_watchers(&mut cleanup).await;
    }

    pub(super) async fn cancel_stopped_generation(&self, id: &str, slot: Uuid) {
        if let Some(active) = self.inner.active.lock().await.get(id).filter(|entry| entry.slot == slot) {
            active.lease.cancel();
        }
    }

    /// Recovered slots have no owned backend to await. Only a later durable
    /// stopped/not-required receipt can release them; unknown stays occupied.
    /// Caller holds admission, or shutdown has already stopped admission.
    pub(super) async fn reconcile_inactive_slots(&self) {
        let slots: Vec<_> = self.inner.slots.lock().await.iter().map(|(id, slot)| (id.clone(), *slot)).collect();
        for (id, slot) in slots {
            if self.inner.active.lock().await.contains_key(&id) { continue; }
            if self.inner.retained.lock().await.iter().any(|entry| entry.slot == slot) { continue; }
            let owner = self.inner.records.lock().await.iter().find(|record| record.id == id).map(|record| record.owner.clone());
            if let Some(owner) = owner {
                if self.slot_is_settled(&owner, &id).await { self.release_slot(&id, slot).await; }
            }
        }
    }

    pub(super) async fn mark_cleanup_settled(&self, id: &str, slot: Uuid) {
        if let Some(active) = self.inner.active.lock().await.get_mut(id).filter(|active| active.slot == slot) {
            active.cleanup_settled = true;
        }
    }

    /// A stopped child is insufficient: both its cleanup receipt and every run
    /// projection must be durable before releasing the backend, lease and permit.
    pub(crate) async fn retire_settled_session(&self, id: &str) {
        let candidate = self.inner.active.lock().await.get(id)
            .filter(|active| active.cleanup_settled)
            .map(|active| (active.backend.clone(), active.generation, active.slot));
        let Some((backend, generation, slot)) = candidate else { return; };
        let projections = self.inner.run_projections.lock().await;
        if projections.contains_key(&(id.to_owned(), generation.to_string())) { return; }
        if let Backend::Codex(session) = &backend {
            if !session.automatic_runs().await.is_empty() { return; }
        }
        let view = terminal_view(backend.snapshot().await);
        let mut active = self.inner.active.lock().await;
        if !active.get(id).is_some_and(|entry| entry.generation == generation && entry.slot == slot && entry.cleanup_settled) { return; }
        let mut terminal = self.inner.terminal.lock().await;
        terminal.retain(|entry| entry.id != id);
        terminal.push_back(TerminalSession { id: id.into(), generation, view });
        while terminal.len() > TERMINAL_SESSION_LIMIT { terminal.pop_front(); }
        // Publish the lightweight snapshot before dropping the heavy generation.
        if let Some(entry) = active.remove(id) { entry.lease.cancel(); }
        self.release_slot(id, slot).await;
    }
}

fn json_weight(value: &Value) -> usize {
    match value {
        Value::String(value) => value.len().saturating_mul(6).saturating_add(2),
        Value::Array(values) => values.iter().fold(2usize, |size, value| size.saturating_add(json_weight(value)).saturating_add(1)),
        Value::Object(values) => values.iter().fold(2usize, |size, (key, value)| size.saturating_add(key.len().saturating_mul(6)).saturating_add(json_weight(value)).saturating_add(4)),
        _ => 32,
    }
}

fn terminal_view(mut view: SessionView) -> SessionView {
    let mut bytes = 0usize;
    let mut keep = 0usize;
    for event in view.events.iter().rev().take(128) {
        let incoming = event.text.as_ref().map_or(0, |text| text.len().saturating_mul(6))
            .saturating_add(json_weight(&event.data))
            .saturating_add(event.kind.len()).saturating_add(128);
        if bytes.saturating_add(incoming) > TERMINAL_EVENT_BYTES { break; }
        bytes += incoming;
        keep += 1;
    }
    if keep < view.events.len() {
        view.history_truncated = true;
        view.events.drain(..view.events.len() - keep);
        view.events.shrink_to_fit();
    }
    view.status = "closed".into();
    view.pending.clear();
    view.pending.shrink_to_fit();
    view.controls = None;
    view.active_run = None;
    view.terminal_run = None;
    view
}

#[cfg(test)]
#[path = "manager_lifecycle_tests.rs"]
mod tests;
