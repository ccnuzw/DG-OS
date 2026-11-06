//! Bounded host execution projection. Pending requests and approval responses are never persisted.
use crate::{manager::Backend, AgentManager, Engine, SessionEvent};
use dgos_db::development_repository::StoredRun;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_EVENTS: usize = 2000;
fn now() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 }

#[derive(Clone, Debug, Serialize)]
pub struct RunRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub task_id: String,
    pub engine: String,
    pub source: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub summary: String,
    pub reason: Option<String>,
    pub events: Vec<SessionEvent>,
    pub fingerprint: String,
    pub start_fingerprint: String,
    pub truncated: bool,
    pub capabilities: Option<crate::capabilities::CapabilitySnapshot>,
}
impl RunRecord {
    fn from_stored(r: StoredRun) -> Result<Self, String> {
        Ok(Self { id:r.id,session_id:r.session_id,task_id:r.task_id,engine:r.engine,source:r.source,status:r.status,started_at:r.started_at,finished_at:r.finished_at,summary:r.summary,reason:r.reason,events:serde_json::from_str(&r.events).map_err(|_| "storage_failed")?,fingerprint:r.fingerprint,start_fingerprint:r.start_fingerprint,truncated:r.truncated,capabilities:None })
    }
}

impl AgentManager {
    pub async fn list_development_runs(&self, owner: &str, task_id: &str) -> Result<Vec<RunRecord>, String> {
        self.ready().await?;
        self.inner.repository.task(owner, task_id).await?;
        let mut records = Vec::new();
        for stored in self.inner.repository.runs(owner, task_id).await? {
            let snapshot = self.inner.repository.run_capabilities(owner, &stored.id).await?;
            let mut record = RunRecord::from_stored(stored)?;
            record.capabilities = snapshot.map(|s| serde_json::from_str(&s).map_err(|_| "storage_failed")).transpose()?;
            records.push(record);
        }
        Ok(records)
    }

    pub async fn send(&self, owner: &str, id: &str, request_id: String, text: String) -> Result<(), String> {
        uuid::Uuid::parse_str(&request_id).map_err(|_| "invalid_request_id")?;
        if text.trim().is_empty() || text.len()>65536 { return Err("invalid_prompt".into()); }
        let request_fingerprint=format!("{:x}",Sha256::digest(text.as_bytes()));
        let admission=self.inner.admission.lock().await;
        self.ready().await?;
        let record=self.list(owner).await.into_iter().find(|r|r.id==id).ok_or("session_not_found")?;
        if let Some(old)=self.inner.repository.run(owner,&request_id).await? {
            if old.session_id.as_deref()!=Some(id) || old.request_fingerprint!=request_fingerprint { return Err("request_conflict".into()); }
            // A persisted receipt never grants permission to resend after a crash.
            return if old.status=="unknown" { Err("result_unknown".into()) } else { Ok(()) };
        }
        let task_id=record.task_id.clone().ok_or("task_not_found")?;
        let project_id=record.project_id.clone().ok_or("task_not_found")?;
        let project=self.task_project(owner,&task_id).await?;
        if project.archived { return Err("project_archived".into()); }
        crate::projects::validate_directory(&project).await?;
        let (_,backend)=self.owned(owner,id).await?;
        self.validate_handoff_target(owner,id).await?;
        if backend.snapshot().await.status!="idle" { return Err("busy".into()); }
        self.capture_task_baseline(owner,&task_id).await?;
        // Baseline is immutable; the start marker must describe this send, not the first round.
        let fingerprint=self.task_fingerprint(owner,&task_id).await?;
        let run=StoredRun { id:request_id.clone(),owner:owner.into(),session_id:Some(id.into()),task_id,project_id,engine:match record.engine { Engine::Codex=>"codex",Engine::Claude=>"claude" }.into(),source:"native_agent".into(),runtime:None,status:"running".into(),started_at:now(),finished_at:None,summary:String::new(),reason:None,events:"[]".into(),fingerprint:fingerprint.clone(),start_fingerprint:fingerprint,truncated:false,request_fingerprint };
        self.inner.repository.start_run(&run).await?;
        if let Some(service) = self.capabilities() {
            if let Err(error) = service.begin_run(owner, id, &request_id).await {
                let mut failed = run.clone(); failed.status = "failed".into(); failed.reason = Some(error.clone()); failed.finished_at = Some(now());
                self.inner.repository.finish_run(owner, &failed, Some("blocked")).await?;
                return Err(error);
            }
        }
        let (result_tx,result_rx)=tokio::sync::oneshot::channel();
        let manager=self.clone();
        let watched=backend.clone();
        let handle=tokio::spawn(async move { watch(manager,watched,run,result_rx).await });
        { let mut watchers=self.inner.watchers.lock().await; watchers.retain(|h|!h.is_finished()); watchers.push(handle); }
        // No manager admission is held while waiting for a native CLI acknowledgement.
        drop(admission);
        let result=match backend { Backend::Codex(s)=>s.send(request_id,text).await,Backend::Claude(s)=>s.send(request_id,text).await };
        let _=result_tx.send(result.clone());
        result
    }
}

async fn watch(manager: AgentManager, backend: Backend, mut run: StoredRun, mut accepted: tokio::sync::oneshot::Receiver<Result<(),String>>) -> bool {
    let mut events:Vec<SessionEvent>=Vec::new();
    let mut bytes=2usize;
    let mut last=0;
    let mut started=false;
    let mut receipt_done=false;
    let mut send_error=None;
    let mut checkpoint_seq=0;
    let mut checkpoint_status=String::new();
    loop {
        if !receipt_done {
            match accepted.try_recv() {
                Ok(result)=>{receipt_done=true;send_error=result.err();},
                Err(tokio::sync::oneshot::error::TryRecvError::Closed)=>{receipt_done=true;send_error=Some("result_unknown".into());},
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)=>{},
            }
        }
        let view=backend.snapshot().await;
        if last>0 && view.events.first().is_some_and(|e|e.seq>last+1) { run.truncated=true; }
        let mut terminal=None;
        for event in view.events.iter().filter(|e|e.seq>last && e.run.as_deref()==Some(run.id.as_str())) {
            started=true;
            if let Some(status)=event.data.get("run_status").and_then(Value::as_str).filter(|s|matches!(*s,"completed"|"failed"|"cancelled"|"unknown")) { terminal=Some(status.to_owned()); }
            if matches!(event.data.get("run_status").and_then(Value::as_str),Some("failed"|"unknown")) || event.kind=="error" || event.kind=="failure" {
                let safe=sanitize_json(event.data.clone());
                let details=event.text.as_deref().map(sanitize_text).or_else(||safe.get("error").or_else(||safe.get("errors")).map(Value::to_string));
                if let Some(details)=details { run.reason=Some(bounded(&details,4096)); }
            }
            let mut event=event.clone();
            event.text=event.text.map(|s|sanitize_text(&s));
            event.data=sanitize_json(event.data);
            // Commands are observations, never model assertions of verified results.
            if let Some(object)=event.data.as_object_mut() { if object.contains_key("command") { object.insert("evidence".into(),Value::String("observed_native_command; content_during_command_unconfirmed".into())); } }
            let size=serde_json::to_vec(&event).map_or(MAX_BYTES+1,|b|b.len()+1);
            if size>MAX_BYTES-2 { run.truncated=true; continue; }
            while !events.is_empty() && (events.len()>=MAX_EVENTS || bytes+size>MAX_BYTES) {
                bytes-=serde_json::to_vec(&events.remove(0)).map_or(0,|b|b.len()+1); run.truncated=true;
            }
            bytes+=size;
            if let Some(text)=event.text.as_deref().filter(|_|matches!(event.kind.as_str(),"agentMessage"|"result"|"text_delta"|"item/agentMessage/delta")) { run.summary=bounded(text,16384); }
            events.push(event);
        }
        last=view.events.last().map_or(last,|e|e.seq);
        if view.history_truncated { run.truncated=true; }
        if terminal.is_none() {
            if let Some(error)=send_error.as_ref() {
                run.reason=Some(bounded(&sanitize_text(error),4096));
                terminal=Some(if matches!(error.as_str(),"result_unknown"|"connection_lost"|"closed") {"unknown"} else {"failed"}.into());
            } else if matches!(view.status.as_str(),"unknown"|"closed"|"error") && (started || receipt_done) {
                terminal=Some(if view.status=="error" {"failed"} else {"unknown"}.into());
                run.reason=view.error.as_deref().map(sanitize_text).or_else(||Some("session_closed_without_terminal".into()));
            } else if manager.inner.stop.is_cancelled() { terminal=Some("unknown".into());run.reason=Some("shutdown".into()); }
        }
        if let Some(status)=terminal {
            if let Some(service) = manager.capabilities() { if let Some(session) = &run.session_id { service.end_run(session).await; } }
            run.status=status;
            run.finished_at=Some(now());
            run.fingerprint=match manager.task_fingerprint(&run.owner,&run.task_id).await { Ok(value)=>value,Err(_)=>{run.truncated=true;String::new()} };
            if run.status=="failed" && run.reason.is_none() { run.reason=view.error.as_deref().map(sanitize_text).or_else(||Some("engine_failed".into())); }
            run.reason=run.reason.map(|text|bounded(&text,4096));
            run.events=match serde_json::to_string(&events) { Ok(value)=>value,Err(_)=>return false };
            let task_status=if run.status=="completed" || run.status=="cancelled" {"review"} else {"blocked"};
            return manager.inner.repository.finish_run(&run.owner,&run,Some(task_status)).await.is_ok();
        }
        run.status=if view.status=="waiting" {"waiting"} else {"running"}.into();
        if checkpoint_seq!=last || checkpoint_status!=run.status {
            run.events=match serde_json::to_string(&events) { Ok(value)=>value,Err(_)=>return false };
            if manager.inner.repository.checkpoint_run(&run).await.is_err() { return false; }
            checkpoint_seq=last;
            checkpoint_status=run.status.clone();
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn bounded(text:&str,max:usize)->String { let mut n=text.len().min(max);while !text.is_char_boundary(n){n-=1;}text[..n].to_owned() }
fn sensitive(key:&str)->bool {
    let key=key.to_ascii_lowercase().replace('-',"_");
    ["password","passwd","secret","token","api_key","apikey","authorization","cookie","credential","private_key"].iter().any(|k|key.contains(k))
}

/// Best-effort projection redaction: known token formats, auth headers, secret assignments,
/// private-key blocks, and secret-keyed JSON. This is never used as permission to read a sensitive file.
pub(crate) fn sanitize_text(text:&str)->String {
    let mut output=String::with_capacity(text.len());
    let mut redact_next=false;
    let mut private_key=false;
    for part in text.split_inclusive(char::is_whitespace) {
        let token=part.trim_end_matches(char::is_whitespace);
        let whitespace=&part[token.len()..];
        if token.contains("-----BEGIN") { private_key=true; }
        let trimmed=token.trim_matches(|c:char|matches!(c,'\''|'"'|'{'|'}'|'['|']'|','|';'));
        let lower=trimmed.to_ascii_lowercase();
        let assignment=trimmed.split_once('=').or_else(||trimmed.split_once(':'));
        let known= ["sk-","ghp_","github_pat_","gho_","xoxb-","xoxp-","AKIA","ASIA"].iter().any(|p|trimmed.starts_with(p)) || (trimmed.starts_with("eyJ") && trimmed.matches('.').count()==2)
            || trimmed.split_once("://").is_some_and(|(_,rest)|rest.split('/').next().is_some_and(|authority|authority.contains('@')));
        let secret_assignment=assignment.is_some_and(|(key,_)|sensitive(key));
        let secret_label=sensitive(trimmed.trim_end_matches([':', '='])) && (trimmed.ends_with(':') || trimmed.ends_with('=') || trimmed.starts_with("--"));
        if private_key || redact_next || known || secret_assignment { output.push_str("[REDACTED]"); } else { output.push_str(token); }
        output.push_str(whitespace);
        redact_next=matches!(lower.as_str(),"bearer"|"basic") || secret_label;
        if token.contains("-----END") { private_key=false; }
    }
    output
}

pub(crate) fn sanitize_json(value:Value)->Value {
    match value {
        Value::String(text)=>Value::String(sanitize_text(&text)),
        Value::Array(values)=>Value::Array(values.into_iter().map(sanitize_json).collect()),
        Value::Object(values)=>Value::Object(values.into_iter().map(|(key,value)|{let value=if sensitive(&key) {Value::String("[REDACTED]".into())} else {sanitize_json(value)};(key,value)}).collect()),
        value=>value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistence_redacts_auth_secrets_without_destroying_command_results() {
        let input=serde_json::json!({"command":"curl -H 'Authorization: Bearer actual-value' https://user:pass@example.test --api-key private-value","exitCode":7,"result":{"password":"hidden","output":"token=hidden ghp_hidden"}});
        let output=sanitize_json(input);
        let text=output.to_string();
        for secret in ["actual-value","user:pass","private-value","hidden"] { assert!(!text.contains(secret)); }
        assert_eq!(output["exitCode"],7);
        assert!(text.contains("curl"));
        assert!(text.contains("REDACTED"));
    }
}
