//! Register every unsolicited owned-thread turn (including native queue auto-start)
//! without ever replaying its input. Native turn identities stay in the session until
//! the existing durable terminal projector confirms settlement.
use super::*;
use std::time::Duration;

impl AgentManager {
    pub(super) async fn start_codex_automatic_supervisor(&self, owner: &str, id: &str, generation: &str) {
        let record = self.inner.records.lock().await.iter().find(|r| r.owner == owner && r.id == id).cloned();
        let Some(record) = record else { return; };
        let (session, lease, provider, uncertain) = {
            let active = self.inner.active.lock().await;
            let Some(entry) = active.get(id) else { return; };
            let Backend::Codex(session) = &entry.backend else { return; };
            (session.clone(), entry.lease.clone(), entry.provider_lease.clone(), entry.native_uncertain.clone())
        };
        let manager = self.clone();
        let generation = generation.to_owned();
        // Registration happens while opening admission is still held, before the
        // generation is returned to a renderer. Account shutdown retains this task.
        self.inner.watchers.lock().await.push(tokio::spawn(async move {
            let mut healthy = true;
            loop {
                let pending = session.automatic_runs().await;
                for run_id in pending {
                    let request_fingerprint = format!("native_automatic:{run_id}");
                    let key = (record.id.clone(), generation.clone());
                    let mut backoff = Duration::from_millis(25);
                    let run = loop {
                        let admission = manager.inner.admission.lock().await;
                        let mut projections = manager.inner.run_projections.lock().await;
                        if projections.contains_key(&key) {
                            drop(projections); drop(admission);
                            tokio::time::sleep(Duration::from_millis(25)).await;
                            continue;
                        }
                        // Read-only recovery of a lost DB commit ACK. Existing
                        // terminal rows are proof of settlement, never resend rights.
                        match manager.inner.repository.run(&record.owner, &run_id).await {
                            Ok(Some(run)) if run.session_id.as_deref() != Some(record.id.as_str()) || run.request_fingerprint != request_fingerprint => {
                                uncertain.store(true, std::sync::atomic::Ordering::Release);
                                return false;
                            },
                            Ok(Some(run)) if matches!(run.status.as_str(), "completed" | "failed" | "cancelled" | "unknown") => {
                                session.settle_automatic_run(&run_id).await;
                                break None;
                            },
                            Ok(Some(run)) => {
                                projections.insert(key.clone(), RunProjection { run_id: run_id.clone(), persistence_failed: false });
                                break Some(run);
                            },
                            Ok(None) => {},
                            Err(_) => {
                                uncertain.store(true, std::sync::atomic::Ordering::Release);
                                drop(projections); drop(admission);
                                if manager.inner.stop.is_cancelled() { return false; }
                                tokio::time::sleep(backoff).await;
                                backoff = (backoff * 2).min(Duration::from_secs(2));
                                continue;
                            },
                        }
                        let configuration = manager.inner.providers.read_session(&record.owner, &record.id).await;
                        let (Some(task_id), Some(project_id), Ok(crate::capabilities::SessionConfigurationView::ProviderBound { snapshot, .. })) = (record.task_id.as_ref(), record.project_id.as_ref(), configuration) else {
                            uncertain.store(true, std::sync::atomic::Ordering::Release);
                            return false;
                        };
                        let run = dgos_db::development_repository::StoredRun {
                            id: run_id.clone(), owner: record.owner.clone(), session_id: Some(record.id.clone()), task_id: task_id.clone(), project_id: project_id.clone(), engine: "codex".into(), source: "native_agent".into(), runtime: None,
                            status: "running".into(), started_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64,
                            finished_at: None, summary: String::new(), reason: Some("observed_native_automatic_turn".into()), events: "[]".into(),
                            // An automatic turn may already have changed files before
                            // its notification arrives. Never invent a pre-start hash.
                            fingerprint: String::new(), start_fingerprint: String::new(), truncated: false, request_fingerprint: request_fingerprint.clone(),
                        };
                        match manager.inner.repository.record_observed_codex_run(&run, &snapshot).await {
                            Ok(()) => {
                                projections.insert(key.clone(), RunProjection { run_id: run_id.clone(), persistence_failed: false });
                                break Some(run);
                            },
                            Err(_) => {
                                // Retain the native identity and retry only its DB
                                // projection. Never retry queue/start or turn/start.
                                uncertain.store(true, std::sync::atomic::Ordering::Release);
                                drop(projections); drop(admission);
                                if manager.inner.stop.is_cancelled() { return false; }
                                tokio::time::sleep(backoff).await;
                                backoff = (backoff * 2).min(Duration::from_secs(2));
                            },
                        }
                    };
                    if let Some(run) = run {
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let _ = tx.send(Ok(()));
                        let settled = crate::executions::watch(manager.clone(), Backend::Codex(session.clone()), run, rx, key).await;
                        if settled {
                            session.settle_automatic_run(&run_id).await;
                            manager.retire_settled_session(&record.id).await;
                        } else { healthy = false; }
                    }
                }
                if session.automatic_runs().await.is_empty() { manager.retire_settled_session(&record.id).await; }
                let closed = !session.native_available();
                if (closed || lease.is_cancelled() || provider.is_cancelled() || manager.inner.stop.is_cancelled()) && session.automatic_runs().await.is_empty() { return healthy; }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }));
    }
}
