use super::*;
use async_trait::async_trait;
use dgos_db::sea_orm::{ConnectionTrait, DbBackend, Statement};
use crate::execution::{ExecutionGuard, ToolReceipt};

#[derive(Clone, Debug, Default)]
pub(super) struct TerminalEvidence {
    pub dispatched: bool, pub result_unknown: bool, pub audit_warning: Option<String>, pub tokens: Option<i64>,
}
/// Every network step revalidates the immutable captured account generation and pinned capability.
pub(super) struct ChatStepGuard<'a> {
    pub manager:&'a ChatManager, pub providers:&'a ProviderManager,
    pub user:&'a str, pub task:&'a str, pub assistant:&'a str,
    pub provider:&'a str, pub model:&'a str, pub revision:Option<i64>,
    pub active:&'a Active, pub pins:&'a [PinnedSkill], pub blocked_effects:&'a [String],
    pub sink:&'a dyn ChatSink,
    pub contexts:&'a [crate::foundation::ResolvedContext],
    pub source_runs:&'a [String],
}
pub(super) fn effect_key(proposal:&ToolProposal)->String {
    // serde_json's default map representation is sorted: object key order is not an effect identity.
    format!("{:x}",Sha256::digest(serde_json::to_vec(&json!([proposal.server_id,proposal.tool_name,proposal.arguments])).unwrap_or_default()))
}
impl ChatStepGuard<'_> {
    async fn validate(&self)->std::result::Result<(),String> {
        if self.active.token.is_cancelled(){return Err("session_invalid".into())}
        self.manager.revalidate_contexts(self.user, &self.active.session_id, self.contexts).await?;
        self.manager.revalidate_source_runs(self.user, &self.active.session_id, self.source_runs).await?;
        let session=self.manager.repo.get_session(self.user,&self.active.session_id).await.map_err(|_|"session_invalid")?;
        if session.provider_id.as_deref()!=Some(self.provider)||session.model_id.as_deref()!=Some(self.model){return Err("provider_changed".into())}
        let (provider,model)=resolve_model(self.providers,self.user,Some(self.provider),Some(self.model)).await.map_err(|_|"provider_changed")?;
        if Some(provider.config_revision)!=self.revision{return Err("provider_changed".into())}
        session.generation.validate(Some(&model)).map_err(|_|"provider_changed")?;
        for pin in self.pins {
            let row=self.manager.db.query_one(Statement::from_sql_and_values(DbBackend::Sqlite,
                "SELECT 1 AS valid FROM skill_user_states s JOIN skill_versions v ON v.id=s.version_id WHERE s.user_id=? AND s.skill_id=? AND s.enabled=1 AND s.version_id=? AND v.content_hash=?",
                vec![self.user.into(),pin.skill_id.clone().into(),pin.version_id.clone().into(),pin.content_hash.clone().into()])).await.map_err(|_|"persistence_failed")?;
            if row.is_none(){return Err("skill_unavailable".into())}
        }
        if self.active.token.is_cancelled(){return Err("session_invalid".into())} Ok(())
    }
    async fn record(&self,kind:&str,phase:&str,status:&str,dispatched:bool,unknown:bool,receipt:Option<Value>)->std::result::Result<(),String>{
        let step=self.manager.repo.record_step(self.user,self.task,kind,phase,status,dispatched,unknown,receipt).await.map_err(|_|"persistence_failed")?;
        if !self.sink.send(ChatEvent::Activity { seq: 0, task_id:self.task.into(),assistant_message_id:self.assistant.into(),step }){self.active.stop("task_cancelled");return Err("task_cancelled".into())}Ok(())
    }
}

#[async_trait]
impl crate::foundation::FoundationGuard for ChatStepGuard<'_> {
    async fn revalidate(&self, contexts: &[crate::foundation::ResolvedContext]) -> std::result::Result<(), String> {
        self.validate().await?;
        self.manager.revalidate_contexts(self.user, &self.active.session_id, contexts).await
    }
}
#[async_trait]
impl ExecutionGuard for ChatStepGuard<'_> {
    async fn before_model(&self)->std::result::Result<(),String>{
        self.validate().await?;
        self.record("model","model","admitted",false,false,None).await?;
        self.validate().await
    }
    async fn before_tool(&self)->std::result::Result<(),String>{self.validate().await}
    async fn approve(&self,_:&ToolProposal)->std::result::Result<bool,String>{Err("explicit_approval_required".into())}
    async fn tool_dispatched(&self,proposal:&ToolProposal)->std::result::Result<(),String>{
        self.validate().await?;
        let key=effect_key(proposal);
        if self.blocked_effects.contains(&key){return Err("completed_effect_replay_blocked".into())}
        self.record("tool","tool","dispatched",true,true,Some(json!({"call_id":proposal.id,"effect_key":key,"server_id":proposal.server_id,"tool_name":proposal.tool_name,"schema_hash":proposal.schema_hash}))).await?;
        self.validate().await
    }
    async fn tool_settled(&self,proposal:&ToolProposal,receipt:&ToolReceipt)->std::result::Result<(),String>{
        self.manager.repo.settle_tool_step(self.user,self.task,&proposal.id,receipt.result_unknown,
            json!({"call_id":receipt.call_id,"effect_key":effect_key(proposal),"server_id":proposal.server_id,"tool_name":proposal.tool_name,"schema_hash":proposal.schema_hash,"outcome":receipt.outcome,"result":receipt.result,"audit_warning":receipt.audit_warning,"result_unknown":receipt.result_unknown})).await.map_err(|_|"persistence_failed".to_string())?;
        let run=self.manager.repo.get_run(self.user,self.task).await.map_err(|_|"persistence_failed".to_string())?;
        let step=run.steps.into_iter().find(|s|s.receipt.as_ref().and_then(|r|r.get("call_id")).and_then(Value::as_str)==Some(proposal.id.as_str())).ok_or("persistence_failed")?;
        if !self.sink.send(ChatEvent::Activity { seq: 0, task_id:self.task.into(),assistant_message_id:self.assistant.into(),step }){self.active.stop("task_cancelled");}
        Ok(())
    }
    async fn model_intermediate(&self, round: u32, content: &str, truncated: bool) -> std::result::Result<(), String> {
        self.record("model_segment", "processing_result", "intermediate", true, false,
            Some(json!({"round":round,"content":content,"truncated":truncated}))).await
    }
}

/// One serialized writer retains the current model segment. Versions, not text length, reject
/// stale queued snapshots: a new round may legitimately be shorter than an earlier plan.
struct CheckpointState {
    content: String, revision: u64, queued_revision: u64, queued_at: std::time::Instant,
}
struct Checkpoint {
    content: String, revision: u64, ack: Option<tokio::sync::oneshot::Sender<bool>>,
}
pub(super) struct CheckpointWriter {
    sender: tokio::sync::mpsc::Sender<Checkpoint>,
    state: Arc<Mutex<CheckpointState>>,
}
impl CheckpointWriter {
    pub fn new(repo: ChatRepository, user: String, task: String) -> Self {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<Checkpoint>(1);
        let state = Arc::new(Mutex::new(CheckpointState { content: String::new(), revision: 0,
            queued_revision: 0, queued_at: std::time::Instant::now() }));
        let progress = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            let mut applied = 0;
            loop {
                let item = tokio::select! {
                    item = receiver.recv() => match item { Some(item) => item, None => break },
                    _ = interval.tick() => {
                        let state = progress.lock().unwrap_or_else(|e| e.into_inner());
                        if state.revision <= applied { continue; }
                        Checkpoint { content: state.content.clone(), revision: state.revision, ack: None }
                    }
                };
                let success = item.revision <= applied || repo.checkpoint(&user, &task, &item.content).await.is_ok();
                if success { applied = applied.max(item.revision); }
                if let Some(ack) = item.ack { let _ = ack.send(success); }
                if !success { break; }
            }
        });
        Self { sender, state }
    }
    pub fn begin_segment(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.content.clear();
        state.revision += 1;
    }
    pub fn push(&self, text: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let room = MAX_REPLY_BYTES.saturating_sub(state.content.len());
        let (piece, _) = split_at_boundary(text, room);
        state.content.push_str(piece);
        state.revision += 1;
        if state.revision.saturating_sub(state.queued_revision) >= 64 || state.queued_at.elapsed() >= Duration::from_secs(1) {
            if self.sender.try_send(Checkpoint { content: state.content.clone(), revision: state.revision, ack: None }).is_ok() {
                state.queued_revision = state.revision;
                state.queued_at = std::time::Instant::now();
            }
        }
    }
    pub fn content(&self) -> String { self.state.lock().unwrap_or_else(|e| e.into_inner()).content.clone() }
    pub async fn flush(&self) -> std::result::Result<(), String> {
        let (ack, done) = tokio::sync::oneshot::channel();
        let checkpoint = { let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            Checkpoint { content: state.content.clone(), revision: state.revision, ack: Some(ack) } };
        self.sender.send(checkpoint).await.map_err(|_| "persistence_failed".to_string())?;
        match done.await { Ok(true) => Ok(()), _ => Err("persistence_failed".into()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_effect_identity_ignores_provider_call_id_and_object_order() {
        let a:Value=serde_json::from_str(r#"{"a":1,"b":2}"#).unwrap();
        let b:Value=serde_json::from_str(r#"{"b":2,"a":1}"#).unwrap();
        let first=ToolProposal{id:"old-call".into(),server_id:"server".into(),server_name:"display".into(),tool_name:"write".into(),schema_hash:"schema".into(),arguments:a,expires_at:0};
        let mut second=first.clone();second.id="new-call".into();second.arguments=b;
        assert_eq!(effect_key(&first),effect_key(&second));
        second.arguments=json!({"a":2,"b":2});assert_ne!(effect_key(&first),effect_key(&second));
    }

    #[tokio::test]
    async fn shorter_new_segments_win_over_late_queued_checkpoint_versions() {
        use dgos_db::{migrations::Migrator, sea_orm_migration::MigratorTrait};
        let db = dgos_db::sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let repo = ChatRepository::new(db);
        let session = repo.create_session("alice", &ChatTarget::default()).await.unwrap();
        repo.begin_generation(&BeginGeneration {
            user: "alice", session_id: &session.id, task_id: "segment-writer", user_message_id: "question",
            assistant_message_id: "answer", user_content: Some("question"), regenerate_of: None,
            provider_id: "provider", model_id: "model", provider_config_revision: None,
            endpoint_fingerprint: "synthetic", skills: &[], generation: &Default::default(), tools: &[], assets: &[],
        }).await.unwrap();
        let writer = CheckpointWriter::new(repo.clone(), "alice".into(), "segment-writer".into());
        writer.begin_segment();
        writer.push("A long intermediate plan");
        writer.flush().await.unwrap();
        writer.begin_segment();
        writer.push("Final.");
        writer.flush().await.unwrap();
        writer.sender.send(Checkpoint { content: "stale longer plan".into(), revision: 1, ack: None }).await.unwrap();
        writer.flush().await.unwrap();
        assert_eq!(repo.get_run("alice", "segment-writer").await.unwrap().partial_content, "Final.");
    }
}
