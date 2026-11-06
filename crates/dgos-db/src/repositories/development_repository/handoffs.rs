//! Reviewable context transfer. No native history, grants or credentials are copied.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredHandoff {
    pub id: String, pub owner: String, pub project_id: String, pub task_id: String,
    pub source_session_id: String, pub source_run_id: Option<String>, pub target_engine: String,
    pub permission_epoch: i64, pub summary: String, pub fingerprint: String,
    pub target_session_id: Option<String>, pub created_at: i64,
}
impl DevelopmentRepository {
    pub async fn insert_handoff(&self, row: &StoredHandoff) -> Result<(), String> {
        if row.summary.is_empty() || row.summary.len()>65536 || !matches!(row.target_engine.as_str(),"codex"|"claude") { return Err("invalid_handoff".into()); }
        let tx=self.db.begin().await.map_err(db)?;
        let task=task_row(&tx,&row.owner,&row.task_id).await?;
        let source=session_row(&tx,&row.owner,&row.source_session_id).await?;
        if task.project_id!=row.project_id || source.task_id.as_deref()!=Some(row.task_id.as_str()) || source.engine==row.target_engine {return Err("invalid_handoff".into());}
        execute(&tx,"INSERT INTO development_handoffs(id,owner,project_id,task_id,source_session_id,source_run_id,target_engine,permission_epoch,summary,fingerprint,target_session_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,NULL,?)",vec![v(row.id.clone()),v(row.owner.clone()),v(row.project_id.clone()),v(row.task_id.clone()),v(row.source_session_id.clone()),v(row.source_run_id.clone()),v(row.target_engine.clone()),v(row.permission_epoch),v(row.summary.clone()),v(row.fingerprint.clone()),v(row.created_at)]).await?;
        tx.commit().await.map_err(db)
    }
    pub async fn handoff(&self,owner:&str,id:&str)->Result<StoredHandoff,String> {
        let row=fetch_one(&self.db,"SELECT * FROM development_handoffs WHERE owner=? AND id=?",vec![v(owner),v(id)]).await?.ok_or("handoff_not_found")?;
        handoff_from(&row)
    }
    pub async fn handoffs(&self,owner:&str,task_id:&str)->Result<Vec<StoredHandoff>,String> {
        task_row(&self.db,owner,task_id).await?;
        fetch(&self.db,"SELECT * FROM development_handoffs WHERE owner=? AND task_id=? ORDER BY created_at,id",vec![v(owner),v(task_id)]).await?.iter().map(handoff_from).collect()
    }
    pub async fn handoff_for_target(&self,owner:&str,session_id:&str)->Result<Option<StoredHandoff>,String> {
        fetch_one(&self.db,"SELECT * FROM development_handoffs WHERE owner=? AND target_session_id=?",vec![v(owner),v(session_id)]).await?.as_ref().map(handoff_from).transpose()
    }
    /// Reserve the target before process creation. A crash never permits a second native launch.
    pub async fn reserve_handoff(&self,owner:&str,id:&str,summary:&str,target:&str)->Result<StoredHandoff,String> {
        if summary.trim().is_empty() || summary.len()>65536 {return Err("invalid_handoff".into());}
        let changed=execute(&self.db,"UPDATE development_handoffs SET summary=?,target_session_id=? WHERE owner=? AND id=? AND target_session_id IS NULL",vec![v(summary),v(target),v(owner),v(id)]).await?;
        if changed!=1 {return Err("handoff_already_accepted".into());}
        self.handoff(owner,id).await
    }
}
fn handoff_from(r:&QueryResult)->Result<StoredHandoff,String> {
    Ok(StoredHandoff {id:r.try_get("","id").map_err(db)?,owner:r.try_get("","owner").map_err(db)?,project_id:r.try_get("","project_id").map_err(db)?,task_id:r.try_get("","task_id").map_err(db)?,source_session_id:r.try_get("","source_session_id").map_err(db)?,source_run_id:r.try_get("","source_run_id").map_err(db)?,target_engine:r.try_get("","target_engine").map_err(db)?,permission_epoch:r.try_get("","permission_epoch").map_err(db)?,summary:r.try_get("","summary").map_err(db)?,fingerprint:r.try_get("","fingerprint").map_err(db)?,target_session_id:r.try_get("","target_session_id").map_err(db)?,created_at:r.try_get("","created_at").map_err(db)?})
}
