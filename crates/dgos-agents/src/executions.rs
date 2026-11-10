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
    pub configuration: Option<crate::capabilities::SessionConfigurationView>,
}
impl RunRecord {
    fn from_stored(r: StoredRun) -> Result<Self, String> {
        Ok(Self { id:r.id,session_id:r.session_id,task_id:r.task_id,engine:r.engine,source:r.source,status:r.status,started_at:r.started_at,finished_at:r.finished_at,summary:r.summary,reason:r.reason,events:serde_json::from_str(&r.events).map_err(|_| "storage_failed")?,fingerprint:r.fingerprint,start_fingerprint:r.start_fingerprint,truncated:r.truncated,configuration:None })
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
            record.configuration = snapshot.map(|raw| {
                let value: Value = serde_json::from_str(&raw).map_err(|_| "storage_failed")?;
                if value.get("binding_id").is_some() {
                    Ok(crate::capabilities::SessionConfigurationView::ProviderBound { snapshot: serde_json::from_value(value).map_err(|_| "storage_failed")?, read_only: true })
                } else { Ok::<_,String>(crate::capabilities::SessionConfigurationView::LegacyHistory { history: Some(value), read_only: true }) }
            }).transpose()?;
            records.push(record);
        }
        Ok(records)
    }

    pub async fn send(&self, owner: &str, id: &str, generation: &str, request_id: String, text: String, settings: Option<crate::session::AgentTurnSettings>, references: Vec<crate::references::ConversationReference>, skills: Vec<crate::session::AgentSkillInput>, images: Vec<crate::images::ImageInput>) -> Result<(), String> {
        uuid::Uuid::parse_str(&request_id).map_err(|_| "invalid_request_id")?;
        let images = crate::images::validate_images(&images)?;
        if (text.trim().is_empty() && references.is_empty() && images.is_empty()) || text.len()>65536 { return Err("invalid_prompt".into()); }
        if references.len() > crate::references::REFERENCE_COUNT_LIMIT { return Err("reference_limit".into()); }
        for reference in &references { crate::references::validate(reference)?; }
        crate::session::validate_skill_inputs(&skills)?;
        let settings = settings.filter(|settings| !settings.is_empty());
        let mut request_hash = Sha256::new();
        // Frame the prompt: literal NUL/JSON text must not alias structured intent.
        request_hash.update((text.len() as u64).to_le_bytes());
        request_hash.update(text.as_bytes());
        if let Some(settings) = &settings {
            request_hash.update([0]);
            request_hash.update(serde_json::to_vec(settings).map_err(|_| "invalid_turn_settings")?);
        }
        if !references.is_empty() {
            request_hash.update(b"\0DG-OS references\0");
            request_hash.update(serde_json::to_vec(&references).map_err(|_| "invalid_reference")?);
        }
        if !skills.is_empty() {
            request_hash.update(b"\0DG-OS skills\0");
            request_hash.update(serde_json::to_vec(&skills).map_err(|_| "agent_skill_invalid")?);
        }
        if !images.is_empty() {
            request_hash.update(b"\0DG-OS images\0");
            for image in &images { request_hash.update(image.fingerprint().as_bytes()); }
        }
        let request_fingerprint = format!("{:x}", request_hash.finalize());
        let mut admission=self.inner.admission.lock().await;
        self.ready().await?;
        let record=self.list(owner).await.into_iter().find(|r|r.id==id).ok_or("session_not_found")?;
        if !images.is_empty() && record.engine != Engine::Codex { return Err("agent_images_unsupported".into()); }
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
        if project.id != project_id || project.path != record.workspace { return Err("file_access_denied".into()); }
        let (_,backend)=self.owned(owner,id).await?;
        self.require_generation(id, generation).await?;
        let control = self.session_control(id).await?;
        let control = control.try_lock_owned().map_err(|_| "session_busy")?;
        if let Some(state) = self.inner.run_projections.lock().await.iter().find_map(|((session, _), state)| (session == id).then_some(state)) {
            return Err(if state.persistence_failed { "storage_failed" } else { "busy" }.into());
        }
        self.native_result_known(id).await?;
        self.validate_handoff_target(owner,id).await?;
        let view = backend.snapshot().await;
        if let Backend::Codex(session) = &backend {
            if !session.automatic_runs().await.is_empty() { return Err("busy".into()); }
        }
        if view.status != "idle" { return Err("busy".into()); }
        if let Some(settings) = &settings {
            view.controls.as_ref().ok_or("turn_controls_unavailable")?.validate(settings)?;
        }
        if !skills.is_empty() {
            let Backend::Codex(session) = &backend else { return Err("agent_skills_unsupported".into()); };
            // Native observations may take tens of seconds. Keep the session control
            // lock, but let unrelated sessions proceed through manager admission.
            let (lease, provider_lease) = self.session_cancellation(id).await?;
            drop(admission);
            let observed = tokio::select! {
                biased;
                _ = lease.cancelled() => return Err("session_closed".into()),
                _ = provider_lease.cancelled() => return Err("provider_revision_changed".into()),
                result = session.observe_native(crate::native_session::SessionDomain::Skills) => result.map_err(|_| "agent_skills_unavailable"),
            }?;
            admission = tokio::select! {
                biased;
                _ = lease.cancelled() => return Err("session_closed".into()),
                _ = provider_lease.cancelled() => return Err("provider_revision_changed".into()),
                guard = self.inner.admission.lock() => guard,
            };
            self.ready().await?;
            let current = self.list(owner).await.into_iter().find(|r| r.id == id).ok_or("session_not_found")?;
            if current.workspace != record.workspace || current.project_id.as_deref() != Some(project_id.as_str()) || current.task_id.as_deref() != Some(task_id.as_str()) { return Err("file_access_denied".into()); }
            let project = self.task_project(owner, &task_id).await?;
            if project.archived { return Err("project_archived".into()); }
            crate::projects::validate_directory(&project).await?;
            if project.id != project_id || project.path != current.workspace { return Err("file_access_denied".into()); }
            let (_, current_backend) = self.owned(owner, id).await?;
            self.require_generation(id, generation).await?;
            self.native_result_known(id).await?;
            self.validate_handoff_target(owner,id).await?;
            if current_backend.snapshot().await.status != "idle" { return Err("busy".into()); }
            if let Backend::Codex(session) = &current_backend {
                if !session.automatic_runs().await.is_empty() { return Err("busy".into()); }
            }
            crate::session::validate_discovered_skills(&skills, &observed)?;
        }
        let text = self.resolve_conversation_references(owner, &record, text, references).await?;
        self.capture_task_baseline(owner,&task_id).await?;
        // Baseline is immutable; the start marker must describe this send, not the first round.
        let fingerprint=self.task_fingerprint(owner,&task_id).await?;
        let run=StoredRun { id:request_id.clone(),owner:owner.into(),session_id:Some(id.into()),task_id,project_id,engine:match record.engine { Engine::Codex=>"codex",Engine::Claude=>"claude" }.into(),source:"native_agent".into(),runtime:None,status:"running".into(),started_at:now(),finished_at:None,summary:String::new(),reason:None,events:"[]".into(),fingerprint:fingerprint.clone(),start_fingerprint:fingerprint,truncated:false,request_fingerprint };
        let configuration = self.inner.providers.read_session(owner, id).await?;
        let crate::capabilities::SessionConfigurationView::ProviderBound { snapshot, .. } = configuration else { return Err("legacy_session_read_only".into()); };
        // Take registration locks before durable admission: no cancellation point may
        // separate a successful receipt from installing its tracked supervisor.
        let mut watchers = self.inner.watchers.lock().await;
        self.reap_watchers(&mut watchers).await;
        let mut projections = self.inner.run_projections.lock().await;
        self.inner.repository.begin_cli_run(&run, &snapshot).await?;
        let key = (id.to_owned(), generation.to_owned());
        projections.insert(key.clone(), crate::manager::RunProjection { run_id: run.id.clone(), persistence_failed: false });
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let manager = self.clone();
        let watched = backend.clone();
        let handle = tokio::spawn(async move {
            let dispatch = async move {
                // The caller owns only result_rx. Aborting it cannot cancel native
                // acknowledgement processing or release this session's control early.
                let result = match backend {
                    Backend::Codex(s) => s.send(request_id, text, settings, skills, images).await,
                    Backend::Claude(s) => s.send(request_id, text, settings).await,
                };
                drop(control);
                let _ = accepted_tx.send(result.clone());
                let _ = result_tx.send(result);
            };
            let (_, settled) = tokio::join!(dispatch, watch(manager, watched, run, accepted_rx, key));
            settled
        });
        watchers.push(handle);
        drop(projections);
        drop(watchers);
        // No manager admission is held while waiting for a native CLI acknowledgement.
        drop(admission);
        result_rx.await.unwrap_or_else(|_| Err("result_unknown".into()))
    }
}

pub(crate) async fn watch(manager: AgentManager, backend: Backend, mut run: StoredRun, mut accepted: tokio::sync::oneshot::Receiver<Result<(),String>>, key: (String, String)) -> bool {
    let mut events:Vec<SessionEvent>=Vec::new();
    let mut bytes=2usize;
    let mut last=0;
    let mut started=false;
    let mut receipt_done=false;
    let mut send_error=None;
    let mut checkpoint_seq=0;
    let mut checkpoint_status=String::new();
    let mut checkpoint_failed = false;
    let mut retry_at = tokio::time::Instant::now();
    let mut backoff = Duration::from_millis(250);
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
        let mut terminal = view.terminal_run.as_ref().filter(|terminal| terminal.run_id == run.id).map(|terminal| terminal.status.clone());
        if terminal.is_none() {
            if let Backend::Codex(session) = &backend { terminal = session.automatic_run_status(&run.id).await; }
        }
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
            // The native terminal may precede a steer acknowledgement. Keep this
            // run reserved until its supervised input receipt and confirmed text
            // have entered the transcript, then consume them in the next snapshot.
            if let Backend::Codex(session) = &backend {
                if !session.steering_settled(last).await {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    continue;
                }
            }
            run.status=status;
            run.finished_at=Some(now());
            run.fingerprint=match manager.task_fingerprint(&run.owner,&run.task_id).await { Ok(value)=>value,Err(_)=>{run.truncated=true;String::new()} };
            if run.status=="failed" && run.reason.is_none() { run.reason=view.error.as_deref().map(sanitize_text).or_else(||Some("engine_failed".into())); }
            run.reason=run.reason.map(|text|bounded(&text,4096));
            run.events = match serde_json::to_string(&events) {
                Ok(value) => value,
                Err(_) => { projection_failed(&manager, &key, &run.id, true).await; return false; },
            };
            // Freeze the exact terminal, timestamp, fingerprint and transcript once.
            // Retry only this idempotent DB projection, never the native request.
            return finish_projection(&manager, &key, &run).await;
        }
        run.status=if view.status=="waiting" {"waiting"} else {"running"}.into();
        if (checkpoint_seq!=last || checkpoint_status!=run.status) && tokio::time::Instant::now() >= retry_at {
            run.events = match serde_json::to_string(&events) {
                Ok(value) => value,
                Err(_) => { projection_failed(&manager, &key, &run.id, true).await; return false; },
            };
            if manager.inner.repository.checkpoint_run(&run).await.is_ok() {
                checkpoint_seq=last;
                checkpoint_status=run.status.clone();
                if checkpoint_failed {
                    projection_failed(&manager, &key, &run.id, false).await;
                    checkpoint_failed = false;
                }
                backoff = Duration::from_millis(250);
            } else {
                if !checkpoint_failed {
                    projection_failed(&manager, &key, &run.id, true).await;
                    checkpoint_failed = true;
                }
                retry_at = tokio::time::Instant::now() + backoff;
                backoff = (backoff * 2).min(Duration::from_secs(2));
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn projection_failed(manager: &AgentManager, key: &(String, String), run_id: &str, failed: bool) {
    if let Some(state) = manager.inner.run_projections.lock().await.get_mut(key).filter(|state| state.run_id == run_id) {
        state.persistence_failed = failed;
    }
}

async fn finish_projection(manager: &AgentManager, key: &(String, String), run: &StoredRun) -> bool {
    let task_status = if matches!(run.status.as_str(), "completed" | "cancelled") { "review" } else { "blocked" };
    let mut backoff = Duration::from_millis(250);
    let mut failure_published = false;
    loop {
        let settled = match manager.inner.repository.finish_run(&run.owner, run, Some(task_status)).await {
            Ok(true) => true,
            // A commit acknowledgement can be lost. A no-op is success only if
            // durable storage already contains this exact observed terminal.
            Ok(false) => manager.inner.repository.run(&run.owner, &run.id).await.ok().flatten().is_some_and(|stored| {
                stored.status == run.status && stored.finished_at == run.finished_at
                    && stored.summary == run.summary && stored.reason == run.reason
                    && stored.events == run.events && stored.fingerprint == run.fingerprint
                    && stored.truncated == run.truncated
            }),
            Err(_) => false,
        };
        if settled {
            let mut projections = manager.inner.run_projections.lock().await;
            if projections.get(key).is_some_and(|state| state.run_id == run.id) { projections.remove(key); }
            drop(projections);
            manager.retire_settled_session(&key.0).await;
            return true;
        }
        if !failure_published {
            projection_failed(manager, key, &run.id, true).await;
            failure_published = true;
        }
        // Shutdown waits a bounded interval and retains our handle on timeout;
        // unresolved truth must remain owned and can still reconcile afterward.
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(2));
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
