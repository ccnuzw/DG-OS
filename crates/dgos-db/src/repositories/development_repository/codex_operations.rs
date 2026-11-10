//! Durable admission and immutable receipts for explicit Codex native operations.
//! A pending row means dispatch may have happened: reads expose unknown, never a
//! replay authorization. Only the caller that inserted the row may perform RPC.
use super::*;
use super::capabilities::{active_project, head};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexOperationJournal {
    pub session_id: String,
    pub generation: String,
    pub fingerprint: String,
    pub state: String,
    /// JSON encoded operation receipt, or `null` when dispatch has no receipt.
    pub result: String,
}

impl DevelopmentRepository {
    pub async fn codex_operation(&self, owner: &str, id: &str) -> Result<Option<CodexOperationJournal>, String> {
        fetch_one(&self.db,
            "SELECT session_id,generation,fingerprint,CASE WHEN state='pending' THEN 'unknown' ELSE state END AS state,result FROM development_codex_operations WHERE owner=? AND request_id=?",
            vec![v(owner), v(id)],
        ).await?.map(|row| Ok(CodexOperationJournal {
            session_id: row.try_get("", "session_id").map_err(db)?,
            generation: row.try_get("", "generation").map_err(db)?,
            fingerprint: row.try_get("", "fingerprint").map_err(db)?,
            state: row.try_get("", "state").map_err(db)?,
            result: row.try_get("", "result").map_err(db)?,
        })).transpose()
    }

    /// Commits admission before native dispatch. Even an identical second begin
    /// conflicts: returning success would authorize two callers to perform RPC.
    pub async fn begin_codex_operation(&self, owner: &str, session_id: &str, generation: &str, request_id: &str, fingerprint: &str) -> Result<(), String> {
        if [owner, session_id, generation, request_id, fingerprint].iter().any(|value| value.is_empty()) {
            return Err("invalid_codex_operation".into());
        }
        let changed = execute(&self.db,
            "INSERT INTO development_codex_operations(owner,request_id,session_id,generation,fingerprint,state,result) SELECT ?,?,?,?,?,'pending','null' FROM development_agent_sessions WHERE owner=? AND id=? AND engine='codex' ON CONFLICT(owner,request_id) DO NOTHING",
            vec![v(owner), v(request_id), v(session_id), v(generation), v(fingerprint), v(owner), v(session_id)],
        ).await?;
        if changed == 1 { return Ok(()); }
        if self.codex_operation(owner, request_id).await?.is_some() {
            Err("codex_operation_conflict".into())
        } else {
            Err("session_not_found".into())
        }
    }

    /// Settles pending dispatch exactly once. Repeating the identical durable
    /// receipt is safe; a different receipt can never replace it, even unknown.
    pub async fn finish_codex_operation(&self, owner: &str, request_id: &str, state: &str, result: &str) -> Result<(), String> {
        validate_receipt(state, result)?;
        let changed = execute(&self.db,
            "UPDATE development_codex_operations SET state=?,result=? WHERE owner=? AND request_id=? AND state='pending'",
            vec![v(state), v(result), v(owner), v(request_id)],
        ).await?;
        if changed == 1 { return Ok(()); }
        match self.codex_operation(owner, request_id).await? {
            Some(previous) if previous.state == state && previous.result == result => Ok(()),
            Some(_) => Err("codex_operation_conflict".into()),
            None => Err("codex_operation_not_found".into()),
        }
    }

    /// Saves native fork history as an independently resumable, CLOSED session.
    /// This does not allocate a process, acquire a Provider lease, or attach the
    /// source backend to the child. Resume must acquire a fresh lease and backend.
    /// The new session, Provider snapshot, closed binding and applied receipt all
    /// commit together, so a crash cannot report a fork whose local index is absent.
    pub async fn persist_codex_fork(&self, owner: &str, source_session_id: &str, source_binding_id: &str, request_id: &str, fork_session_id: &str, native_id: &str, result: &str) -> Result<StoredAgentSession, String> {
        validate_receipt("applied", result)?;
        if uuid::Uuid::parse_str(fork_session_id).is_err() || fork_session_id == source_session_id
            || native_id.is_empty() || native_id.len() > 4096 || source_binding_id.is_empty() {
            return Err("invalid_codex_operation".into());
        }
        let tx = self.db.begin().await.map_err(db)?;
        let outcome = async {
            // First take the SQLite writer lock; a concurrent settlement or
            // revocation cannot race the validation and session insertion below.
            let pending = execute(&tx,
                "UPDATE development_codex_operations SET state=state WHERE owner=? AND request_id=? AND session_id=? AND state='pending'",
                vec![v(owner), v(request_id), v(source_session_id)],
            ).await?;
            if pending != 1 { return Err("codex_operation_conflict".into()); }
            let mut session = session_row(&tx, owner, source_session_id).await?;
            if session.engine != "codex" || session.native_id.as_deref().is_none_or(|id| id == native_id) {
                return Err("invalid_codex_operation".into());
            }
            let project_id = session.project_id.as_deref().ok_or("invalid_cli_binding")?;
            let task_id = session.task_id.as_deref().ok_or("invalid_cli_binding")?;
            active_project(&tx, owner, project_id).await?;
            let row = fetch_one(&tx,
                "SELECT b.revision,b.permission_epoch,b.provider_id,b.model_id,b.provider_revision,c.snapshot FROM development_cli_session_bindings b JOIN development_session_capabilities c ON c.owner=b.owner AND c.session_id=b.session_id AND c.project_id=b.project_id AND c.revision=b.revision AND c.permission_epoch=b.permission_epoch AND c.format='cli_provider_v1' WHERE b.owner=? AND b.session_id=? AND b.binding_id=? AND b.execution_policy='provider_bound' AND b.state='active' AND b.project_id=? AND b.task_id=?",
                vec![v(owner), v(source_session_id), v(source_binding_id), v(project_id), v(task_id)],
            ).await?.ok_or("capability_revoked")?;
            let revision: i64 = row.try_get("", "revision").map_err(db)?;
            let epoch: i64 = row.try_get("", "permission_epoch").map_err(db)?;
            if head(&tx, owner, project_id).await? != Some((revision, epoch)) {
                return Err("capability_revoked".into());
            }
            let raw: String = row.try_get("", "snapshot").map_err(db)?;
            let mut snapshot: CliProviderSnapshot = serde_json::from_str(&raw).map_err(|_| "storage_failed")?;
            let provider_id: String = row.try_get("", "provider_id").map_err(db)?;
            let model_id: String = row.try_get("", "model_id").map_err(db)?;
            let provider_revision: i64 = row.try_get("", "provider_revision").map_err(db)?;
            if snapshot.binding_id != source_binding_id || snapshot.engine != "codex" || snapshot.revision != revision
                || snapshot.permission_epoch != epoch || snapshot.provider_id != provider_id
                || snapshot.model_id != model_id || snapshot.provider_revision != provider_revision {
                return Err("invalid_cli_binding".into());
            }
            if fetch_one(&tx,
                "SELECT id FROM development_agent_sessions WHERE owner=? AND engine='codex' AND native_id=?",
                vec![v(owner), v(native_id)],
            ).await?.is_some() { return Err("codex_operation_conflict".into()); }
            let binding_id = uuid::Uuid::new_v4().to_string();
            snapshot.binding_id = binding_id.clone();
            let encoded = serde_json::to_string(&snapshot).map_err(|_| "storage_failed")?;
            session.id = fork_session_id.into();
            session.native_id = Some(native_id.into());
            execute(&tx,
                "INSERT INTO development_agent_sessions(id,owner,engine,native_id,workspace,executable,model,project_id,task_id) VALUES(?,?,?,?,?,?,?,?,?)",
                session_values(&session),
            ).await?;
            execute(&tx,
                "INSERT INTO development_cli_session_bindings(owner,session_id,binding_id,execution_policy,project_id,task_id,revision,permission_epoch,provider_id,model_id,provider_revision,state,cleanup_state,cleanup_reason,host_instance_id,boot_identity,cleanup_proof,request_digest,updated_at) VALUES(?,?,?,'provider_bound',?,?,?,?,?,?,?,'closed','not_required',NULL,NULL,NULL,'no_execution',NULL,unixepoch())",
                vec![v(owner), v(fork_session_id), v(binding_id), v(session.project_id.clone()), v(session.task_id.clone()), v(revision), v(epoch), v(provider_id), v(model_id), v(provider_revision)],
            ).await?;
            execute(&tx,
                "INSERT INTO development_session_capabilities(owner,session_id,project_id,revision,permission_epoch,snapshot,format) VALUES(?,?,?,?,?,?,'cli_provider_v1')",
                vec![v(owner), v(fork_session_id), v(session.project_id.clone()), v(revision), v(epoch), v(encoded)],
            ).await?;
            let changed = execute(&tx,
                "UPDATE development_codex_operations SET state='applied',result=? WHERE owner=? AND request_id=? AND session_id=? AND state='pending'",
                vec![v(result), v(owner), v(request_id), v(source_session_id)],
            ).await?;
            if changed != 1 { return Err("codex_operation_conflict".into()); }
            Ok::<_, String>(session)
        }.await;
        match outcome {
            Ok(session) => { tx.commit().await.map_err(db)?; Ok(session) }
            Err(error) => { let _ = tx.rollback().await; Err(error) }
        }
    }
}

fn validate_receipt(state: &str, result: &str) -> Result<(), String> {
    if !matches!(state, "applied" | "unknown" | "failed") || serde_json::from_str::<serde::de::IgnoredAny>(result).is_err() {
        return Err("invalid_codex_operation".into());
    }
    Ok(())
}
