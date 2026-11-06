use super::*;
use serde_json::{json, Value as Json};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredWorkspace {
    pub id: String, pub owner: String, pub project_id: String, pub path: String,
    pub directory_identity: String, pub common_dir: Option<String>, pub common_identity: Option<String>,
    pub kind: String, pub created_operation: Option<String>, pub base_commit: Option<String>,
}

pub struct GitOperationIntent<'a> {
    pub id: &'a str,
    pub project_id: &'a str,
    pub task_id: &'a str,
    pub workspace_id: Option<&'a str>,
    pub kind: &'a str,
    pub fingerprint: &'a str,
    pub details: &'a Json,
}

const COLUMNS: &str = "id,owner,project_id,path,directory_identity,common_dir,common_identity,kind,created_operation,base_commit";
fn from_row(r: &QueryResult) -> Result<StoredWorkspace,String> {
    Ok(StoredWorkspace { id:r.try_get("","id").map_err(db)?,owner:r.try_get("","owner").map_err(db)?,project_id:r.try_get("","project_id").map_err(db)?,path:r.try_get("","path").map_err(db)?,directory_identity:r.try_get("","directory_identity").map_err(db)?,common_dir:r.try_get("","common_dir").map_err(db)?,common_identity:r.try_get("","common_identity").map_err(db)?,kind:r.try_get("","kind").map_err(db)?,created_operation:r.try_get("","created_operation").map_err(db)?,base_commit:r.try_get("","base_commit").map_err(db)? })
}
fn operation(r:&QueryResult)->Result<Json,String> {
    let details:Option<String>=r.try_get("","details").map_err(db)?;
    Ok(json!({"id":r.try_get::<String>("","id").map_err(db)?,"kind":r.try_get::<String>("","kind").map_err(db)?,"status":r.try_get::<String>("","status").map_err(db)?,"project_id":r.try_get::<Option<String>>("","project_id").map_err(db)?,"task_id":r.try_get::<Option<String>>("","task_id").map_err(db)?,"workspace_id":r.try_get::<Option<String>>("","workspace_id").map_err(db)?,"request_fingerprint":r.try_get::<Option<String>>("","request_fingerprint").map_err(db)?,"details":details.map(|s|serde_json::from_str::<Json>(&s)).transpose().map_err(|_|"storage_failed")?}))
}
impl DevelopmentRepository {
    pub async fn workspaces(&self,owner:&str,project_id:&str)->Result<Vec<StoredWorkspace>,String> {
        project_row(&self.db,owner,project_id).await?;
        fetch(&self.db,&format!("SELECT {COLUMNS} FROM development_workspaces WHERE owner=? AND project_id=? ORDER BY kind,id"),vec![v(owner),v(project_id)]).await?.iter().map(from_row).collect()
    }
    pub async fn workspace_by_id(&self,owner:&str,id:&str)->Result<Option<StoredWorkspace>,String> {
        fetch_one(&self.db,&format!("SELECT {COLUMNS} FROM development_workspaces WHERE owner=? AND id=?"),vec![v(owner),v(id)]).await?.as_ref().map(from_row).transpose()
    }
    pub async fn task_workspace(&self,owner:&str,task_id:&str)->Result<Option<StoredWorkspace>,String> {
        task_row(&self.db,owner,task_id).await?;
        fetch_one(&self.db,&format!("SELECT {} FROM development_workspaces w JOIN development_task_workspaces t ON t.workspace_id=w.id AND t.owner=w.owner AND t.project_id=w.project_id WHERE t.owner=? AND t.task_id=?", COLUMNS.split(',').map(|s|format!("w.{s}")).collect::<Vec<_>>().join(",")),vec![v(owner),v(task_id)]).await?.as_ref().map(from_row).transpose()
    }
    pub async fn insert_workspace(&self,w:&StoredWorkspace)->Result<(),String> {
        project_row(&self.db,&w.owner,&w.project_id).await?;
        if !std::path::Path::new(&w.path).is_absolute() || w.directory_identity.is_empty() || !matches!(w.kind.as_str(),"original"|"imported"|"managed") || w.common_dir.is_some()!=w.common_identity.is_some() {return Err("invalid_workspace".into());}
        execute(&self.db,"INSERT INTO development_workspaces(id,owner,project_id,path,directory_identity,common_dir,common_identity,kind,created_operation,base_commit) VALUES(?,?,?,?,?,?,?,?,?,?)",vec![v(w.id.clone()),v(w.owner.clone()),v(w.project_id.clone()),v(w.path.clone()),v(w.directory_identity.clone()),v(w.common_dir.clone()),v(w.common_identity.clone()),v(w.kind.clone()),v(w.created_operation.clone()),v(w.base_commit.clone())]).await.map(|_|())
    }
    pub async fn bind_task_workspace(&self,owner:&str,task_id:&str,workspace_id:&str)->Result<(),String> {
        let txn=self.db.begin().await.map_err(db)?;let task=task_row(&txn,owner,task_id).await?;
        let workspace=fetch_one(&txn,"SELECT path FROM development_workspaces WHERE owner=? AND id=? AND project_id=?",vec![v(owner),v(workspace_id),v(task.project_id.clone())]).await?.ok_or("workspace_not_found")?;
        let path:String=workspace.try_get("","path").map_err(db)?;
        let old=fetch_one(&txn,"SELECT w.path FROM development_task_workspaces t JOIN development_workspaces w ON w.id=t.workspace_id WHERE t.owner=? AND t.task_id=?",vec![v(owner),v(task_id)]).await?;
        let old_path=if let Some(old)=old {old.try_get::<String>("","path").map_err(db)?} else {project_row(&txn,owner,&task.project_id).await?.path};
        if path!=old_path && fetch_one(&txn,"SELECT 1 AS found FROM development_agent_sessions WHERE owner=? AND task_id=? UNION ALL SELECT 1 FROM development_review_baselines WHERE owner=? AND task_id=? LIMIT 1",vec![v(owner),v(task_id),v(owner),v(task_id)]).await?.is_some() {return Err("workspace_immutable".into());}
        execute(&txn,"INSERT INTO development_task_workspaces(task_id,owner,project_id,workspace_id) VALUES(?,?,?,?) ON CONFLICT(task_id) DO UPDATE SET workspace_id=excluded.workspace_id WHERE development_task_workspaces.owner=excluded.owner AND development_task_workspaces.project_id=excluded.project_id",vec![v(task_id),v(owner),v(task.project_id),v(workspace_id)]).await?;
        txn.commit().await.map_err(db)
    }
    pub async fn workspace_needed(&self,owner:&str,id:&str,requesting_task:&str)->Result<bool,String> {
        Ok(fetch_one(&self.db,"SELECT 1 AS found FROM development_task_workspaces t JOIN development_tasks d ON d.id=t.task_id AND d.owner=t.owner WHERE t.owner=? AND t.workspace_id=? AND (t.task_id!=? OR d.status NOT IN ('completed','cancelled')) LIMIT 1",vec![v(owner),v(id),v(requesting_task)]).await?.is_some())
    }
    pub async fn start_git_operation(&self,owner:&str,intent:&GitOperationIntent<'_>)->Result<(),String> {
        let GitOperationIntent { id, project_id, task_id, workspace_id, kind, fingerprint, details } = *intent;
        let task=self.task(owner,task_id).await?;if task.project_id!=project_id {return Err("task_not_found".into());}
        if let Some(w)=workspace_id {if self.workspace_by_id(owner,w).await?.is_none_or(|w|w.project_id!=project_id) {return Err("workspace_not_found".into());}}
        if let Some(old)=self.git_operation(owner,id).await? {return if old["request_fingerprint"].as_str()==Some(fingerprint) {Ok(())} else {Err("request_conflict".into())};}
        let path=details["path"].as_str().unwrap_or("git");
        execute(&self.db,"INSERT INTO development_project_operations(id,owner,project_id,task_id,workspace_id,kind,path,status,request_fingerprint,details) VALUES(?,?,?,?,?,?,?,'pending',?,?)",vec![v(id),v(owner),v(project_id),v(task_id),v(workspace_id.map(str::to_owned)),v(kind),v(path),v(fingerprint),v(serde_json::to_string(details).map_err(|_|"storage_failed")?)]).await.map(|_|())
    }
    pub async fn git_operation(&self,owner:&str,id:&str)->Result<Option<Json>,String> {
        fetch_one(&self.db,"SELECT * FROM development_project_operations WHERE owner=? AND id=?",vec![v(owner),v(id)]).await?.as_ref().map(operation).transpose()
    }
    pub async fn finish_git_operation(&self,owner:&str,id:&str,status:&str,details:&Json)->Result<(),String> {
        if !matches!(status,"succeeded"|"failed"|"unknown") {return Err("storage_failed".into());}
        if execute(&self.db,"UPDATE development_project_operations SET status=?,details=? WHERE owner=? AND id=?",vec![v(status),v(serde_json::to_string(details).map_err(|_|"storage_failed")?),v(owner),v(id)]).await?==0 {return Err("storage_failed".into());}Ok(())
    }
    pub async fn update_git_operation_details(&self,owner:&str,id:&str,details:&Json)->Result<(),String> {
        if execute(&self.db,"UPDATE development_project_operations SET details=? WHERE owner=? AND id=? AND status='pending'",vec![v(serde_json::to_string(details).map_err(|_|"storage_failed")?),v(owner),v(id)]).await?==0 {return Err("storage_failed".into());}Ok(())
    }
    pub async fn list_git_operations(&self,owner:&str,task_id:&str)->Result<Vec<Json>,String> {
        self.task(owner,task_id).await?;
        fetch(&self.db,"SELECT * FROM development_project_operations WHERE owner=? AND task_id=? ORDER BY rowid",vec![v(owner),v(task_id)]).await?.iter().map(operation).collect()
    }
    pub async fn host_file_ownership(&self,owner:&str,task_id:&str,path:&str)->Result<Option<Json>,String> {
        self.task(owner,task_id).await?;
        fetch_one(&self.db,"SELECT before_hash,after_hash,valid FROM development_file_writes WHERE owner=? AND task_id=? AND path=?",vec![v(owner),v(task_id),v(path)]).await?.map(|r|Ok::<_,String>(json!({"before_hash":r.try_get::<String>("","before_hash").map_err(db)?,"after_hash":r.try_get::<String>("","after_hash").map_err(db)?,"valid":r.try_get::<i64>("","valid").map_err(db)?!=0}))).transpose()
    }
    pub async fn save_host_file_ownership(&self,owner:&str,task_id:&str,path:&str,before_hash:&str,after_hash:&str)->Result<(),String> {
        let task=self.task(owner,task_id).await?;
        let baseline=self.review_baseline(owner,task_id).await?.ok_or("baseline_missing")?;
        let observation=self.review_observation(owner,task_id).await?;
        let excluded=baseline["initial_changes"].as_array().is_some_and(|a|a.iter().any(|v|v.as_str().and_then(|s|s.get(3..)).is_some_and(|p|p==path || (p.ends_with('/')&&path.starts_with(p))))) || observation.as_ref().and_then(|o|o["external"].as_array()).is_some_and(|a|a.iter().any(|p|p.as_str()==Some(path)));
        let old=self.host_file_ownership(owner,task_id,path).await?;
        let valid=!excluded && match &old {Some(old)=>old["valid"].as_bool()==Some(true)&&old["after_hash"].as_str()==Some(before_hash),None=>baseline["files"][path]["hash"].as_str()==Some(before_hash)};
        execute(&self.db,"INSERT INTO development_file_writes(owner,task_id,project_id,path,before_hash,after_hash,valid) VALUES(?,?,?,?,?,?,?) ON CONFLICT(owner,task_id,path) DO UPDATE SET after_hash=excluded.after_hash,valid=development_file_writes.valid AND excluded.valid",vec![v(owner),v(task_id),v(task.project_id),v(path),v(before_hash),v(after_hash),v(valid as i64)]).await.map(|_|())
    }
}
