//! Agent session index, native-session classification, selection and the one-time legacy
//! JSON import. Session rows keep native ids and workspace paths; project and task
//! references are validated here before they are written.
use super::*;
use sea_orm::TransactionTrait;
use std::collections::HashSet;

fn valid_engine(engine: &str) -> bool {
    matches!(engine, "codex" | "claude")
}

impl DevelopmentRepository {
    /// Whether the one-time legacy import marker exists.
    pub async fn legacy_imported(&self) -> Result<bool, String> {
        Ok(fetch_one(&self.db, "SELECT key FROM development_migrations WHERE key=?", vec![v(LEGACY_KEY)]).await?.is_some())
    }

    /// Imports every legacy session with no project or task, and writes the marker, in one
    /// transaction. Any invalid row or database error rolls back the whole import.
    pub async fn initialize_legacy(&self, records: &[StoredAgentSession]) -> Result<(), String> {
        if self.legacy_imported().await? { return Ok(()); }
        let mut ids = HashSet::new();
        for r in records {
            if uuid::Uuid::parse_str(&r.id).is_err() || !ids.insert(r.id.as_str()) || r.owner.is_empty()
                || !valid_engine(&r.engine) || !std::path::Path::new(&r.workspace).is_absolute()
                || r.project_id.is_some() || r.task_id.is_some()
            {
                return Err("session_index_invalid".into());
            }
        }
        let txn = self.db.begin().await.map_err(db)?;
        let result = async {
            if fetch_one(&txn, "SELECT key FROM development_migrations WHERE key=?", vec![v(LEGACY_KEY)]).await?.is_some() { return Ok(()); }
            for r in records {
                execute(&txn, "INSERT INTO development_agent_sessions(id,owner,engine,native_id,workspace,executable,model,project_id,task_id) VALUES(?,?,?,?,?,?,?,?,?)", session_values(r)).await?;
            }
            execute(&txn, "INSERT INTO development_migrations(key) VALUES(?)", vec![v(LEGACY_KEY)]).await?;
            Ok::<(), String>(())
        }.await;
        match result {
            Ok(()) => txn.commit().await.map_err(db),
            Err(error) => {
                let _ = txn.rollback().await;
                Err(error)
            }
        }
    }

    /// Every session row, all accounts. The manager filters by owner when it answers a request.
    pub async fn sessions(&self) -> Result<Vec<StoredAgentSession>, String> {
        fetch(&self.db, &format!("SELECT {SESSION_COLUMNS} FROM development_agent_sessions ORDER BY id"), vec![])
            .await?.iter().map(session_from).collect()
    }

    pub async fn agent_session(&self, owner: &str, id: &str) -> Result<Option<StoredAgentSession>, String> {
        fetch_one(&self.db, &format!("SELECT {SESSION_COLUMNS} FROM development_agent_sessions WHERE owner=? AND id=?"), vec![v(owner), v(id)])
            .await?.as_ref().map(session_from).transpose()
    }

    /// Inserts a session, or updates an existing row of the same owner and engine. The row
    /// keeps its engine and owner. A different owner's id matches no row and returns `session_not_found`.
    pub async fn save_session(&self, session: &StoredAgentSession) -> Result<(), String> {
        if session.owner.is_empty() || !valid_engine(&session.engine) || !std::path::Path::new(&session.workspace).is_absolute() {
            return Err("session_index_invalid".into());
        }
        if session.project_id.is_some() != session.task_id.is_some() { return Err("invalid_selection".into()); }
        let changed = execute(
            &self.db,
            "INSERT INTO development_agent_sessions(id,owner,engine,native_id,workspace,executable,model,project_id,task_id) VALUES(?,?,?,?,?,?,?,?,?)
             ON CONFLICT(id) DO UPDATE SET native_id=excluded.native_id,workspace=excluded.workspace,executable=excluded.executable,
             model=excluded.model,project_id=excluded.project_id,task_id=excluded.task_id
             WHERE development_agent_sessions.owner=excluded.owner AND development_agent_sessions.engine=excluded.engine
             AND development_agent_sessions.project_id IS excluded.project_id AND development_agent_sessions.task_id IS excluded.task_id",
            session_values(session),
        ).await?;
        if changed == 0 { Err("session_not_found".into()) } else { Ok(()) }
    }

    /// Attaches an unclassified native session to its task's bound directory.
    /// Already classified sessions are not reassigned.
    pub async fn classify(&self, owner: &str, session_id: &str, task_id: &str) -> Result<(), String> {
        let session = session_row(&self.db, owner, session_id).await?;
        if session.task_id.is_some() || session.project_id.is_some() { return Err("invalid_task".into()); }
        let task = task_row(&self.db, owner, task_id).await?;
        let project = project_row(&self.db, owner, &task.project_id).await?;
        if project.archived { return Err("project_archived".into()); }
        let workspace = self.task_workspace(owner, task_id).await?.map(|w| w.path).unwrap_or_else(|| project.path.clone());
        if workspace != session.workspace { return Err("invalid_task".into()); }
        let changed = execute(
            &self.db,
            "UPDATE development_agent_sessions SET project_id=?,task_id=? WHERE owner=? AND id=? AND project_id IS NULL AND task_id IS NULL",
            vec![v(project.id), v(task.id), v(owner), v(session_id)],
        ).await?;
        if changed == 0 { Err("invalid_task".into()) } else { Ok(()) }
    }

    /// Stores a selection after validating the whole chain. A session with a project must match
    /// `project_id` and `task_id` exactly. An unclassified session may only be selected alone.
    pub async fn select(&self, owner: &str, selection: &Selection) -> Result<(), String> {
        if selection.task_id.is_some() && selection.project_id.is_none() { return Err("invalid_selection".into()); }
        let txn = self.db.begin().await.map_err(db)?;
        if let Some(id) = &selection.project_id { project_row(&txn, owner, id).await?; }
        if let Some(id) = &selection.task_id {
            let task = task_row(&txn, owner, id).await?;
            if Some(&task.project_id) != selection.project_id.as_ref() { return Err("invalid_selection".into()); }
        }
        if let Some(id) = &selection.session_id {
            let session = session_row(&txn, owner, id).await?;
            if session.project_id.is_none() {
                if selection.project_id.is_some() || selection.task_id.is_some() { return Err("invalid_selection".into()); }
            } else if session.project_id != selection.project_id || session.task_id != selection.task_id {
                return Err("invalid_selection".into());
            }
        }
        execute(
            &txn,
            "INSERT INTO development_selection(owner,project_id,task_id,session_id) VALUES(?,?,?,?) \
             ON CONFLICT(owner) DO UPDATE SET project_id=excluded.project_id,task_id=excluded.task_id,session_id=excluded.session_id",
            vec![v(owner), v(selection.project_id.clone()), v(selection.task_id.clone()), v(selection.session_id.clone())],
        ).await?;
        if let Some(id) = &selection.project_id {
            execute(&txn, "UPDATE development_projects SET last_opened_at=unixepoch() WHERE owner=? AND id=?", vec![v(owner),v(id)]).await?;
        }
        txn.commit().await.map_err(db)
    }

}
