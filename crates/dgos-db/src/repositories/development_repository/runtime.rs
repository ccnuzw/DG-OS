use super::*;

impl DevelopmentRepository {
    pub async fn development_commands(&self, owner: &str, project_id: &str) -> Result<Vec<serde_json::Value>,String> {
        project_row(&self.db,owner,project_id).await?;
        fetch(&self.db,"SELECT configuration FROM development_commands WHERE owner=? AND project_id=? ORDER BY id",vec![v(owner),v(project_id)]).await?.into_iter().map(|r| { let text:String=r.try_get("","configuration").map_err(db)?; serde_json::from_str(&text).map_err(|_|"storage_failed".into()) }).collect()
    }
    pub async fn save_development_commands(&self, owner: &str, project_id: &str, commands: &[(String,String)]) -> Result<(),String> {
        if commands.len()>100 || commands.iter().any(|(id,c)|id.is_empty() || id.len()>128 || c.len()>65536) { return Err("invalid_command".into()); }
        let tx=self.db.begin().await.map_err(db)?;
        let result=async {
            project_row(&tx,owner,project_id).await?;
            execute(&tx,"DELETE FROM development_commands WHERE owner=? AND project_id=?",vec![v(owner),v(project_id)]).await?;
            for (id,c) in commands { execute(&tx,"INSERT INTO development_commands(owner,project_id,id,configuration) VALUES(?,?,?,?)",vec![v(owner),v(project_id),v(id.clone()),v(c.clone())]).await?; }
            Ok::<_,String>(())
        }.await;
        match result { Ok(())=>tx.commit().await.map_err(db),Err(e)=>{let _=tx.rollback().await;Err(e)} }
    }
    /// Runtime JSON contains only a bounded, redacted snapshot. It is not a restart instruction.
    pub async fn checkpoint_host_run(&self, run: &StoredRun) -> Result<(),String> {
        if run.engine!="host" || run.session_id.is_some() || run.runtime.as_ref().is_none_or(|r|r.len()>16*1024*1024) { return Err("storage_failed".into()); }
        let tx=self.db.begin().await.map_err(db)?;
        let result=async {
            let changed=execute(&tx,"UPDATE development_runs SET status=?,finished_at=?,reason=?,fingerprint=?,truncated=?,runtime=? WHERE owner=? AND id=? AND task_id=? AND project_id=? AND engine='host' AND status IN ('running','waiting')",vec![v(run.status.clone()),v(run.finished_at),v(run.reason.clone()),v(run.fingerprint.clone()),v(run.truncated as i64),v(run.runtime.clone()),v(run.owner.clone()),v(run.id.clone()),v(run.task_id.clone()),v(run.project_id.clone())]).await?;
            if changed!=1 {return Err("storage_failed".into());}
            if !matches!(run.status.as_str(),"running"|"waiting") {
                let status=if run.status=="completed" {"review"} else {"blocked"};
                execute(&tx,"UPDATE development_tasks SET status=? WHERE owner=? AND id=? AND status NOT IN ('completed','cancelled')",vec![v(status),v(run.owner.clone()),v(run.task_id.clone())]).await?;
            }
            Ok::<_,String>(())
        }.await;
        match result {Ok(())=>tx.commit().await.map_err(db),Err(error)=>{let _=tx.rollback().await;Err(error)}}
    }
}
