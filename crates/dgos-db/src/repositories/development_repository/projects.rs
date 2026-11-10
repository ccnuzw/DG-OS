//! Project, task and operation metadata. Callers hold the manager admission lock and
//! have already checked the directory. Every write here is scoped to the owner.
use super::*;
use sea_orm::TransactionTrait;

pub(super) fn require_owner(owner: &str) -> Result<(), String> {
    if owner.is_empty() { Err("invalid_project".into()) } else { Ok(()) }
}

impl DevelopmentRepository {
    pub async fn insert_project(&self, project: &Project) -> Result<(), String> {
        require_owner(&project.owner)?;
        check_project_fields(&project.name, &project.path, &project.directory_identity)?;
        execute(
            &self.db,
            "INSERT INTO development_projects(id,owner,name,path,directory_identity,archived,created_at,last_opened_at) VALUES(?,?,?,?,?,?,?,?)",
            vec![
                v(project.id.clone()), v(project.owner.clone()), v(project.name.clone()), v(project.path.clone()),
                v(project.directory_identity.clone()), v(project.archived as i64), v(project.created_at), v(project.last_opened_at),
            ],
        ).await.map(|_| ())
    }

    /// Updates name, path, identity and archived state. A missing row for this owner is `project_not_found`.
    /// Existing task/session evidence fixes the directory once work has started.
    pub async fn update_project(&self, project: &Project) -> Result<(), String> {
        require_owner(&project.owner)?;
        check_project_fields(&project.name, &project.path, &project.directory_identity)?;
        let txn = self.db.begin().await.map_err(db)?;
        let previous = project_row(&txn, &project.owner, &project.id).await?;
        if (previous.path != project.path || previous.directory_identity != project.directory_identity)
            && fetch_one(&txn, "SELECT 1 AS found FROM development_agent_sessions WHERE owner=? AND project_id=? UNION ALL SELECT 1 FROM development_review_baselines WHERE owner=? AND project_id=? UNION ALL SELECT 1 FROM development_workspaces WHERE owner=? AND project_id=? LIMIT 1", vec![v(project.owner.clone()),v(project.id.clone()),v(project.owner.clone()),v(project.id.clone()),v(project.owner.clone()),v(project.id.clone())]).await?.is_some() {
            return Err("workspace_immutable".into());
        }
        let changed = execute(
            &txn,
            "UPDATE development_projects SET name=?,path=?,directory_identity=?,archived=?,last_opened_at=? WHERE owner=? AND id=?",
            vec![
                v(project.name.clone()), v(project.path.clone()), v(project.directory_identity.clone()),
                v(project.archived as i64), v(project.last_opened_at), v(project.owner.clone()), v(project.id.clone()),
            ],
        ).await?;
        if changed == 0 {
            let _ = txn.rollback().await;
            return Err("project_not_found".into());
        }
        txn.commit().await.map_err(db)
    }

    /// Removes the project and its tasks. Sessions keep their native history and become unclassified.
    /// Selection rows that name the project, or a session of the project, are cleared entirely.
    pub async fn unlink_project(&self, owner: &str, project_id: &str) -> Result<(), String> {
        let txn = self.db.begin().await.map_err(db)?;
        project_row(&txn, owner, project_id).await?;
        execute(&txn, "UPDATE development_capability_heads SET permission_epoch=permission_epoch+1,revision=NULL WHERE owner=? AND project_id=?", vec![v(owner),v(project_id)]).await?;
        execute(&txn, "INSERT OR IGNORE INTO development_cli_legacy_history(owner,session_id,original_project_id,original_task_id,session_record,capability_snapshot,referenced_configuration,original_revision,original_permission_epoch,archived_at) SELECT s.owner,s.id,s.project_id,s.task_id,json_object('engine',s.engine,'native_id',s.native_id,'workspace',s.workspace,'executable',s.executable,'model',s.model),c.snapshot,r.configuration,c.revision,c.permission_epoch,unixepoch() FROM development_agent_sessions s LEFT JOIN development_session_capabilities c ON c.owner=s.owner AND c.session_id=s.id LEFT JOIN development_capability_revisions r ON r.owner=c.owner AND r.project_id=c.project_id AND r.revision=c.revision WHERE s.owner=? AND s.project_id=?", vec![v(owner),v(project_id)]).await?;
        execute(&txn, "UPDATE development_cli_session_bindings SET state='revoked',cleanup_state=CASE WHEN state IN ('opening','active') THEN 'pending' ELSE cleanup_state END,cleanup_reason=CASE WHEN state IN ('opening','active') THEN 'project_unlinked' ELSE cleanup_reason END,updated_at=unixepoch() WHERE owner=? AND project_id=? AND execution_policy='provider_bound' AND state IN ('opening','active')", vec![v(owner),v(project_id)]).await?;
        execute(
            &txn,
            "UPDATE development_selection SET project_id=NULL,task_id=NULL,session_id=NULL WHERE owner=? AND (project_id=? OR session_id IN (SELECT id FROM development_agent_sessions WHERE owner=? AND project_id=?))",
            vec![v(owner), v(project_id), v(owner), v(project_id)],
        ).await?;
        execute(&txn, "DELETE FROM development_session_capabilities WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?;
        execute(&txn, "DELETE FROM development_handoffs WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?;
        execute(&txn, "DELETE FROM development_file_writes WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?;
        execute(&txn, "DELETE FROM development_task_workspaces WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?;
        execute(&txn, "DELETE FROM development_workspaces WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?;
        execute(&txn, "UPDATE development_agent_sessions SET project_id=NULL,task_id=NULL WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?;
        execute(&txn, "UPDATE development_project_operations SET project_id=NULL WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?;
        execute(&txn, "DELETE FROM development_runs WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?;
        execute(&txn, "DELETE FROM development_review_observations WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?;
        execute(&txn, "DELETE FROM development_review_baselines WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?;
        execute(&txn, "DELETE FROM development_tasks WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?;
        execute(&txn, "DELETE FROM development_projects WHERE owner=? AND id=?", vec![v(owner), v(project_id)]).await?;
        txn.commit().await.map_err(db)
    }

    /// Creates or updates a task. The project is fixed after creation: an update that names a
    /// different owner or project matches no row and returns `task_not_found`.
    pub async fn save_task(&self, task: &DevelopmentTask) -> Result<(), String> {
        require_owner(&task.owner)?;
        check_task_fields(&task.goal, &task.requirements, &task.status)?;
        let changed = execute(
            &self.db,
            "INSERT INTO development_tasks(id,owner,project_id,goal,requirements,status,created_at) VALUES(?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET goal=excluded.goal,requirements=excluded.requirements,status=excluded.status \
             WHERE development_tasks.owner=excluded.owner AND development_tasks.project_id=excluded.project_id",
            vec![
                v(task.id.clone()), v(task.owner.clone()), v(task.project_id.clone()), v(task.goal.clone()),
                v(task.requirements.clone()), v(task.status.clone()), v(task.created_at),
            ],
        ).await?;
        if changed == 0 { Err("task_not_found".into()) } else { Ok(()) }
    }

    /// Records the intent before any disk operation. Status starts as `pending`.
    pub async fn start_operation(&self, operation: &ProjectOperation) -> Result<(), String> {
        require_owner(&operation.owner)?;
        if operation.status != "pending" || !matches!(operation.kind.as_str(), "create" | "clone") {
            return Err("invalid_project".into());
        }
        execute(
            &self.db,
            "INSERT INTO development_project_operations(id,owner,kind,path,status,project_id) VALUES(?,?,?,?,?,?)",
            vec![
                v(operation.id.clone()), v(operation.owner.clone()), v(operation.kind.clone()),
                v(operation.path.clone()), v(operation.status.clone()), v(operation.project_id.clone()),
            ],
        ).await.map(|_| ())
    }

    /// Stores the final status: `succeeded`, `failed` or `unknown`.
    pub async fn finish_operation(&self, owner: &str, id: &str, status: &str, project_id: Option<&str>) -> Result<(), String> {
        if !matches!(status, "succeeded" | "failed" | "unknown") { return Err("storage_failed".into()); }
        let changed = execute(
            &self.db,
            "UPDATE development_project_operations SET status=?,project_id=? WHERE owner=? AND id=?",
            vec![v(status), v(project_id.map(str::to_owned)), v(owner), v(id)],
        ).await?;
        if changed == 0 { Err("storage_failed".into()) } else { Ok(()) }
    }

    /// Startup: a `pending` operation was interrupted by a crash, so its disk effect is unknown.
    /// It is marked `unknown` and never resumed automatically.
    pub async fn recover_operations(&self) -> Result<(), String> {
        execute(&self.db, "UPDATE development_project_operations SET status='unknown' WHERE status='pending'", vec![]).await.map(|_| ())
    }
}
