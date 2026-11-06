//! Cross-engine continuation creates a new native session. It never restores foreign history.
use crate::{AgentManager, Engine, OwnedSessionView, SessionOptions};
use dgos_db::development_repository::StoredHandoff;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareHandoffInput { pub task_id:String,pub source_session_id:String,pub target_engine:Engine,pub remaining:String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptHandoffInput { pub handoff_id:String,pub summary:String,pub confirmed:bool,pub capability_revision:i64,pub options:SessionOptions }
#[derive(Serialize)]
pub struct AcceptedHandoff {pub handoff:StoredHandoff,pub session:OwnedSessionView}
fn engine_name(engine:Engine)->&'static str {match engine {Engine::Codex=>"codex",Engine::Claude=>"claude"}}
impl AgentManager {
    pub async fn list_development_handoffs(&self,owner:&str,task_id:&str)->Result<Vec<StoredHandoff>,String> {
        self.ready().await?;
        self.inner.repository.handoffs(owner,task_id).await
    }
    pub async fn development_handoff(&self,owner:&str,id:&str)->Result<StoredHandoff,String> {
        self.ready().await?;
        self.inner.repository.handoff(owner,id).await
    }
    pub async fn prepare_development_handoff(&self,owner:&str,input:PrepareHandoffInput,lease:CancellationToken)->Result<StoredHandoff,String> {
        let _admission=self.inner.admission.lock().await;
        self.ready().await?;
        if lease.is_cancelled(){return Err("session_invalid".into());}
        if input.remaining.len()>8192 {return Err("invalid_handoff".into());}
        let task=self.inner.repository.task(owner,&input.task_id).await?;
        let project=self.task_project(owner,&task.id).await?;
        if project.archived {return Err("project_archived".into());}
        let source=self.list(owner).await.into_iter().find(|s|s.id==input.source_session_id && s.task_id.as_deref()==Some(task.id.as_str())).ok_or("session_not_found")?;
        if source.engine==input.target_engine {return Err("invalid_handoff".into());}
        self.workspace_available(&project.path,None).await?;
        self.host_workspace_available(&project.path).await?;
        let epoch=self.inner.repository.capability_epoch(owner,&project.id).await?;
        if self.inner.repository.read_session_capabilities(owner,&source.id).await?.is_some_and(|c|c.permission_epoch!=epoch){return Err("capability_revoked".into());}
        let review=self.read_development_review(owner,&task.id).await?;
        let runs=self.list_development_runs(owner,&task.id).await?;
        let source_run=runs.iter().find(|r|r.session_id.as_deref()==Some(source.id.as_str()));
        let mut summary=format!("Task goal\n{}\n\nAcceptance requirements\n{}\n\nContext transfer\n{} → {}. New native session; source session {}. No historical permissions or approvals are transferred.\n\nWork observed\n",task.goal,task.requirements,engine_name(source.engine),engine_name(input.target_engine),source.id);
        for run in runs.iter().take(8) {
            summary.push_str(&format!("- {} {} {}: {}\n",run.id,run.engine,run.status,truncate(&run.summary,1024)));
        }
        summary.push_str("\nCurrent file changes (authorship unconfirmed)\n");
        for change in review.changes.iter().take(100){summary.push_str(&format!("- {} {} (origin={}, pre-existing={})\n",change.kind,change.path,change.origin,change.pre_existing));}
        summary.push_str(&format!("\nReview incomplete: {}; fingerprint: {}\n\nVerification observed by host\n",review.truncated,review.fingerprint));
        let processes=self.list_development_processes(owner,&task.id).await?;
        for process in processes.iter().filter(|p|p.verification_status!="not_requested").take(20){summary.push_str(&format!("- run={} program={} args={:?} cwd={} exit={:?} evidence={} fingerprint={}\n",process.id,process.program,process.args,process.cwd,process.exit_code,process.verification_status,process.fingerprint));}
        if !processes.iter().any(|p|p.verification_status!="not_requested"){summary.push_str("No independent host verification recorded; native summaries are not proof of passing tests.\n");}
        summary.push_str(&format!("\nRemaining questions and requested continuation\n{}\n",input.remaining));
        let summary=crate::executions::sanitize_text(&summary);
        if summary.len()>65536 {return Err("handoff_limit".into());}
        let row=StoredHandoff{id:uuid::Uuid::new_v4().to_string(),owner:owner.into(),project_id:project.id,task_id:task.id,source_session_id:source.id,source_run_id:source_run.map(|r|r.id.clone()),target_engine:engine_name(input.target_engine).into(),permission_epoch:epoch,summary,fingerprint:review.fingerprint,target_session_id:None,created_at:std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64};
        if lease.is_cancelled(){return Err("session_invalid".into());}
        self.inner.repository.insert_handoff(&row).await?;
        Ok(row)
    }
    pub(crate) async fn validate_handoff_target(&self,owner:&str,session_id:&str)->Result<(),String> {
        if let Some(handoff)=self.inner.repository.handoff_for_target(owner,session_id).await? {
            if self.inner.repository.capability_epoch(owner,&handoff.project_id).await?!=handoff.permission_epoch{return Err("capability_revoked".into());}
        }
        Ok(())
    }
    pub async fn accept_development_handoff(&self,owner:&str,input:AcceptHandoffInput,lease:CancellationToken)->Result<AcceptedHandoff,String> {
        if !input.confirmed || !input.options.trust_workspace {return Err("workspace_untrusted".into());}
        if input.options.resume_id.is_some() || input.capability_revision<0 || input.summary.trim().is_empty() || input.summary.len()>65536 {return Err("invalid_handoff".into());}
        // Reject secret-bearing edits instead of silently transferring unreviewed text.
        if crate::executions::sanitize_text(&input.summary)!=input.summary{return Err("secret_argument_denied".into());}
        let (handoff,engine,id)={
            let _admission=self.inner.admission.lock().await;self.ready().await?;
            if lease.is_cancelled(){return Err("session_invalid".into());}
            let handoff=self.inner.repository.handoff(owner,&input.handoff_id).await?;
            if handoff.target_session_id.is_some(){return Err("handoff_already_accepted".into());}
            if self.inner.repository.capability_epoch(owner,&handoff.project_id).await?!=handoff.permission_epoch{return Err("capability_revoked".into());}
            let project=self.task_project(owner,&handoff.task_id).await?;
            self.workspace_available(&project.path,None).await?;self.host_workspace_available(&project.path).await?;
            if self.task_fingerprint(owner,&handoff.task_id).await?!=handoff.fingerprint{return Err("handoff_stale".into());}
            let engine=match handoff.target_engine.as_str(){"codex"=>Engine::Codex,"claude"=>Engine::Claude,_=>return Err("invalid_handoff".into())};
            if input.capability_revision>0 {
                let revision=self.inner.repository.capability_revision(owner,&handoff.project_id,Some(input.capability_revision)).await?.ok_or("capability_unavailable")?;
                let config:crate::capabilities::CapabilityConfiguration=serde_json::from_str(&revision.configuration).map_err(|_|"storage_failed")?;
                if config.engine!=engine{return Err("provider_incompatible".into());}
            }
            let id=uuid::Uuid::new_v4().to_string();
            let row=self.inner.repository.reserve_handoff(owner,&handoff.id,&input.summary,&id).await?;
            (row,engine,id)
        };
        // Durable reservation means failure/unknown is inspectable and never blindly retried.
        // The reviewed summary remains a draft; a separate explicit send creates a normal run.
        let session=self.open_with_capabilities(owner,engine,id,crate::capabilities::SessionCapabilities { options: input.options, revision: Some(input.capability_revision) },Some(handoff.task_id.clone()),lease).await?;
        Ok(AcceptedHandoff{handoff,session})
    }
}
fn truncate(text:&str,limit:usize)->&str {let mut end=text.len().min(limit);while !text.is_char_boundary(end){end-=1;}&text[..end]}

#[cfg(test)]
mod tests;
