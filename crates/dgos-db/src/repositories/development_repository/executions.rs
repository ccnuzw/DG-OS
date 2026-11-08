
use super::*;
use super::capabilities::head;
use sea_orm::TransactionTrait;

// File text is bounded to 8MiB by the review layer; JSON metadata/escaping has headroom.
const MAX_BASELINE: usize = 12 * 1024 * 1024;
const MAX_OBSERVATION: usize = 1024 * 1024;
const MAX_EVENTS: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredRun {
    pub id: String,
    pub owner: String,
    pub session_id: Option<String>,
    pub task_id: String,
    pub project_id: String,
    pub engine: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub summary: String,
    pub reason: Option<String>,
    pub events: String,
    pub fingerprint: String,
    pub start_fingerprint: String,
    pub truncated: bool,
    pub request_fingerprint: String,
    pub source: String,
    pub runtime: Option<String>,
}

impl DevelopmentRepository {
    /// Atomically stores a Provider-bound native run and its immutable binding snapshot.
    pub async fn begin_cli_run(&self, run: &StoredRun, snapshot: &super::CliProviderSnapshot) -> Result<(), String> {
        if run.source != "native_agent" || run.status != "running" || run.id.is_empty() || run.owner.is_empty() || run.session_id.is_none() || run.events.len()>MAX_EVENTS || run.summary.len()>16384 {
            return Err("invalid_cli_binding".into());
        }
        let encoded = serde_json::to_string(snapshot).map_err(|_| "invalid_capabilities")?;
        if encoded.len() > 1_048_576 { return Err("invalid_capabilities".into()); }
        let tx = self.db.begin().await.map_err(db)?;
        let result = async {
            execute(&tx, "UPDATE development_capability_heads SET permission_epoch=permission_epoch WHERE owner=? AND project_id=?", vec![v(run.owner.clone()),v(run.project_id.clone())]).await?;
            super::capabilities::active_project(&tx,&run.owner,&run.project_id).await?;
            let session_id = run.session_id.as_deref().ok_or("invalid_cli_binding")?;
            let binding = fetch_one(&tx, "SELECT b.binding_id,b.project_id,b.task_id,b.revision,b.permission_epoch,b.state,b.execution_policy,s.engine,b.provider_id,b.model_id,b.provider_revision FROM development_cli_session_bindings b JOIN development_agent_sessions s ON s.owner=b.owner AND s.id=b.session_id WHERE b.owner=? AND b.session_id=?", vec![v(run.owner.clone()),v(session_id)]).await?.ok_or("legacy_session_read_only")?;
            let binding_id: String = binding.try_get("", "binding_id").map_err(db)?;
            let project: Option<String> = binding.try_get("", "project_id").map_err(db)?;
            let task: Option<String> = binding.try_get("", "task_id").map_err(db)?;
            let revision: Option<i64> = binding.try_get("", "revision").map_err(db)?;
            let epoch: i64 = binding.try_get("", "permission_epoch").map_err(db)?;
            let state: String = binding.try_get("", "state").map_err(db)?;
            let policy: String = binding.try_get("", "execution_policy").map_err(db)?;
            let engine: Option<String> = binding.try_get("", "engine").map_err(db)?;
            let provider: Option<String> = binding.try_get("", "provider_id").map_err(db)?;
            let model: Option<String> = binding.try_get("", "model_id").map_err(db)?;
            let provider_revision: Option<i64> = binding.try_get("", "provider_revision").map_err(db)?;
            if binding_id != snapshot.binding_id || policy != "provider_bound" || state != "active" || project.as_deref()!=Some(&run.project_id) || task.as_deref()!=Some(&run.task_id) || revision!=Some(snapshot.revision) || epoch!=snapshot.permission_epoch || engine.as_deref()!=Some(snapshot.engine.as_str()) || provider.as_deref()!=Some(snapshot.provider_id.as_str()) || model.as_deref()!=Some(snapshot.model_id.as_str()) || provider_revision!=Some(snapshot.provider_revision) || run.engine!=snapshot.engine { return Err("capability_revoked".into()); }
            let head_now = head(&tx,&run.owner,&run.project_id).await?.ok_or("capability_revoked")?;
            if head_now.0 != snapshot.revision || head_now.1 != snapshot.permission_epoch { return Err("capability_revoked".into()); }
            let session=session_row(&tx,&run.owner,session_id).await?;
            if session.engine!=run.engine { return Err("invalid_cli_binding".into()); }
            let task_row = task_row(&tx,&run.owner,&run.task_id).await?;
            if task_row.project_id!=run.project_id { return Err("task_not_found".into()); }
            if fetch_one(&tx,"SELECT 1 AS busy FROM development_runs WHERE owner=? AND session_id=? AND status IN ('running','waiting')",vec![v(run.owner.clone()),v(session_id)]).await?.is_some(){return Err("busy".into());}
            execute(&tx, "INSERT INTO development_runs(id,owner,session_id,task_id,project_id,engine,status,started_at,finished_at,summary,reason,events,fingerprint,start_fingerprint,truncated,request_fingerprint,source,runtime) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", vec![v(run.id.clone()),v(run.owner.clone()),v(run.session_id.clone()),v(run.task_id.clone()),v(run.project_id.clone()),v(run.engine.clone()),v(run.status.clone()),v(run.started_at),v(run.finished_at),v(run.summary.clone()),v(run.reason.clone()),v(run.events.clone()),v(run.fingerprint.clone()),v(run.start_fingerprint.clone()),v(run.truncated as i64),v(run.request_fingerprint.clone()),v("native_agent"),v(run.runtime.clone())]).await?;
            execute(&tx,"INSERT INTO development_run_capabilities(owner,run_id,snapshot,format) VALUES(?,?,?,'cli_provider_v1')",vec![v(run.owner.clone()),v(run.id.clone()),v(encoded)]).await?;
            execute(&tx,"UPDATE development_tasks SET status='in_progress' WHERE owner=? AND id=? AND status NOT IN ('completed','cancelled')",vec![v(run.owner.clone()),v(run.task_id.clone())]).await?;
            Ok::<(),String>(())
        }.await;
        match result { Ok(())=>{tx.commit().await.map_err(db)},Err(error)=>{let _=tx.rollback().await;Err(error)} }
    }
    pub async fn review_baseline(&self, owner: &str, task_id: &str) -> Result<Option<serde_json::Value>, String> {
        let row = fetch_one(&self.db, "SELECT b.baseline FROM development_review_baselines b JOIN development_tasks t ON t.id=b.task_id AND t.owner=b.owner WHERE b.owner=? AND b.task_id=?", vec![v(owner), v(task_id)]).await?;
        row.map(|r| r.try_get::<String>("", "baseline").map_err(db).and_then(|s| serde_json::from_str(&s).map_err(|_| "storage_failed".into()))).transpose()
    }

    pub async fn save_review_baseline(&self, owner: &str, task_id: &str, value: &serde_json::Value) -> Result<(), String> {
        let encoded = serde_json::to_string(value).map_err(|_| "storage_failed")?;
        if encoded.len() > MAX_BASELINE { return Err("storage_failed".into()); }
        let task = task_row(&self.db, owner, task_id).await?;
        execute(&self.db, "INSERT OR IGNORE INTO development_review_baselines(owner,task_id,project_id,baseline) VALUES(?,?,?,?)", vec![v(owner), v(task_id), v(task.project_id), v(encoded)]).await.map(|_| ())
    }

    pub async fn review_observation(&self, owner: &str, task_id: &str) -> Result<Option<serde_json::Value>, String> {
        let row = fetch_one(&self.db, "SELECT o.observation FROM development_review_observations o JOIN development_tasks t ON t.id=o.task_id AND t.owner=o.owner WHERE o.owner=? AND o.task_id=?", vec![v(owner), v(task_id)]).await?;
        row.map(|r| r.try_get::<String>("", "observation").map_err(db).and_then(|s| serde_json::from_str(&s).map_err(|_| "storage_failed".into()))).transpose()
    }

    pub async fn save_review_observation(&self, owner: &str, task_id: &str, value: &serde_json::Value) -> Result<(), String> {
        let encoded = serde_json::to_string(value).map_err(|_| "storage_failed")?;
        if encoded.len() > MAX_OBSERVATION { return Err("storage_failed".into()); }
        let task = task_row(&self.db, owner, task_id).await?;
        execute(&self.db, "INSERT INTO development_review_observations(owner,task_id,project_id,observation) VALUES(?,?,?,?) ON CONFLICT(owner,task_id) DO UPDATE SET observation=excluded.observation", vec![v(owner), v(task_id), v(task.project_id), v(encoded)]).await.map(|_| ())
    }

    pub async fn start_run(&self, run: &StoredRun) -> Result<(), String> {
        if run.owner.is_empty() || run.status != "running" || run.id.is_empty() || run.events.len() > MAX_EVENTS { return Err("storage_failed".into()); }
        let tx = self.db.begin().await.map_err(db)?;
        let result = async {
            if run.session_id.is_some() && fetch_one(&tx,"SELECT 1 AS complete FROM development_migrations WHERE key=?",vec![v(super::cutover::CUTOVER_KEY)]).await?.is_some() {
                return Err("legacy_session_read_only".into());
            }
            if let Some(session_id)=&run.session_id {
                if fetch_one(&tx,"SELECT 1 AS bound FROM development_cli_session_bindings WHERE owner=? AND session_id=?",vec![v(run.owner.clone()),v(session_id.clone())]).await?.is_some() { return Err("legacy_session_read_only".into()); }
            }
            let task=task_row(&tx,&run.owner,&run.task_id).await?;
            if task.project_id!=run.project_id { return Err("task_not_found".into()); }
            if let Some(session_id)=&run.session_id {
                let session=session_row(&tx,&run.owner,session_id).await?;
                if run.source!="native_agent" || session.task_id.as_deref()!=Some(run.task_id.as_str()) || session.project_id.as_deref()!=Some(run.project_id.as_str()) || session.engine!=run.engine { return Err("task_not_found".into()); }
                if fetch_one(&tx,"SELECT id FROM development_runs WHERE owner=? AND session_id=? AND status IN ('running','waiting')",vec![v(run.owner.clone()),v(session_id.clone())]).await?.is_some() { return Err("busy".into()); }
            } else if run.engine!="host" { return Err("storage_failed".into()); }
            execute(&tx, "INSERT INTO development_runs(id,owner,session_id,task_id,project_id,engine,status,started_at,finished_at,summary,reason,events,fingerprint,start_fingerprint,truncated,request_fingerprint,source,runtime) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", vec![v(run.id.clone()),v(run.owner.clone()),v(run.session_id.clone()),v(run.task_id.clone()),v(run.project_id.clone()),v(run.engine.clone()),v(run.status.clone()),v(run.started_at),v(run.finished_at),v(run.summary.clone()),v(run.reason.clone()),v(run.events.clone()),v(run.fingerprint.clone()),v(run.start_fingerprint.clone()),v(run.truncated as i64),v(run.request_fingerprint.clone()),v(run.source.clone()),v(run.runtime.clone())]).await?;
            execute(&tx, "UPDATE development_tasks SET status='in_progress' WHERE owner=? AND id=? AND status NOT IN ('completed','cancelled')", vec![v(run.owner.clone()),v(run.task_id.clone())]).await?;
            execute(&tx, "DELETE FROM development_runs WHERE owner=? AND task_id=? AND status NOT IN ('running','waiting') AND id NOT IN (SELECT id FROM development_runs WHERE owner=? AND task_id=? ORDER BY started_at DESC,id DESC LIMIT 50)", vec![v(run.owner.clone()),v(run.task_id.clone()),v(run.owner.clone()),v(run.task_id.clone())]).await?;
            Ok::<(),String>(())
        }.await;
        match result { Ok(()) => tx.commit().await.map_err(db), Err(e) => { let _=tx.rollback().await; Err(e) } }
    }

    pub async fn checkpoint_run(&self, run: &StoredRun) -> Result<(), String> {
        if !matches!(run.status.as_str(),"running"|"waiting") || run.events.len()>MAX_EVENTS || run.summary.len()>16384 { return Err("storage_failed".into()); }
        execute(&self.db,"UPDATE development_runs SET status=?,summary=?,events=?,truncated=? WHERE owner=? AND id=? AND status IN ('running','waiting')",vec![v(run.status.clone()),v(run.summary.clone()),v(run.events.clone()),v(run.truncated as i64),v(run.owner.clone()),v(run.id.clone())]).await.map(|_|())
    }

    pub async fn finish_run(&self, owner: &str, run: &StoredRun, task_status: Option<&str>) -> Result<bool, String> {
        if owner!=run.owner || !matches!(run.status.as_str(),"completed"|"failed"|"cancelled"|"unknown") || run.events.len()>MAX_EVENTS || run.summary.len()>16384 || run.reason.as_ref().is_some_and(|r|r.len()>4096) { return Err("storage_failed".into()); }
        let tx=self.db.begin().await.map_err(db)?;
        let result=async {
            let changed=execute(&tx,"UPDATE development_runs SET status=?,finished_at=?,summary=?,reason=?,events=?,fingerprint=?,truncated=? WHERE owner=? AND id=? AND status IN ('running','waiting')",vec![v(run.status.clone()),v(run.finished_at),v(run.summary.clone()),v(run.reason.clone()),v(run.events.clone()),v(run.fingerprint.clone()),v(run.truncated as i64),v(owner),v(run.id.clone())]).await?;
            if changed>0 { if let Some(status)=task_status { if !matches!(status,"review"|"blocked") { return Err("storage_failed".into()); } execute(&tx,"UPDATE development_tasks SET status=? WHERE owner=? AND id=(SELECT task_id FROM development_runs WHERE owner=? AND id=?) AND status NOT IN ('completed','cancelled')",vec![v(status),v(owner),v(owner),v(run.id.clone())]).await?; } }
            Ok::<bool,String>(changed>0)
        }.await;
        match result { Ok(value)=>{tx.commit().await.map_err(db)?;Ok(value)},Err(e)=>{let _=tx.rollback().await;Err(e)} }
    }

    pub async fn run(&self, owner: &str, id: &str) -> Result<Option<StoredRun>, String> {
        let rows=fetch(&self.db,"SELECT * FROM development_runs WHERE owner=? AND id=?",vec![v(owner),v(id)]).await?;
        rows.first().map(stored_run).transpose()
    }

    pub async fn runs(&self, owner: &str, task_id: &str) -> Result<Vec<StoredRun>, String> {
        let rows=fetch(&self.db,"SELECT * FROM development_runs WHERE owner=? AND task_id=? ORDER BY started_at DESC,id DESC LIMIT 50",vec![v(owner),v(task_id)]).await?;
        rows.iter().map(stored_run).collect()
    }

    pub async fn recover_runs(&self) -> Result<(), String> {
        let tx=self.db.begin().await.map_err(db)?;
        let result=async {
            execute(&tx,"UPDATE development_tasks SET status='blocked' WHERE status='in_progress' AND id IN (SELECT task_id FROM development_runs WHERE status IN ('running','waiting'))",vec![]).await?;
            execute(&tx,"UPDATE development_runs SET status='unknown',finished_at=unixepoch(),reason='interrupted_by_restart' WHERE status IN ('running','waiting')",vec![]).await?;
            Ok::<(),String>(())
        }.await;
        match result { Ok(())=>tx.commit().await.map_err(db),Err(e)=>{let _=tx.rollback().await;Err(e)} }
    }
}

fn stored_run(r: &QueryResult) -> Result<StoredRun,String> {
    Ok(StoredRun { id:r.try_get("","id").map_err(db)?, owner:r.try_get("","owner").map_err(db)?, session_id:r.try_get("","session_id").map_err(db)?, task_id:r.try_get("","task_id").map_err(db)?, project_id:r.try_get("","project_id").map_err(db)?, engine:r.try_get("","engine").map_err(db)?, status:r.try_get("","status").map_err(db)?, started_at:r.try_get("","started_at").map_err(db)?, finished_at:r.try_get("","finished_at").map_err(db)?, summary:r.try_get("","summary").map_err(db)?, reason:r.try_get("","reason").map_err(db)?, events:r.try_get("","events").map_err(db)?, fingerprint:r.try_get("","fingerprint").map_err(db)?, start_fingerprint:r.try_get("","start_fingerprint").map_err(db)?, truncated:r.try_get::<i64>("","truncated").map_err(db)? != 0, request_fingerprint:r.try_get("","request_fingerprint").map_err(db)?, source:r.try_get("","source").map_err(db)?, runtime:r.try_get("","runtime").map_err(db)? })
}
