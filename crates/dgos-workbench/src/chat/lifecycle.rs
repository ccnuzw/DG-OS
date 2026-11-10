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
        if !self.sink.send(ChatEvent::Activity{task_id:self.task.into(),assistant_message_id:self.assistant.into(),step}){self.active.stop("task_cancelled");return Err("task_cancelled".into())}Ok(())
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
        if !self.sink.send(ChatEvent::Activity{task_id:self.task.into(),assistant_message_id:self.assistant.into(),step}){self.active.stop("task_cancelled");}
        Ok(())
    }
}

/// At most one queued cumulative snapshot; no SQLite write is issued for individual tokens.
pub(super) struct CheckpointWriter {
    sender:tokio::sync::mpsc::Sender<(String,Option<tokio::sync::oneshot::Sender<bool>>)>,
    state:Arc<Mutex<(String,usize,std::time::Instant)>>,
    worker:Mutex<Option<tokio::task::JoinHandle<std::result::Result<(),ChatStoreError>>>>,
}
impl CheckpointWriter {
    pub fn new(repo:ChatRepository,user:String,task:String)->Self {
        let (sender,mut receiver)=tokio::sync::mpsc::channel::<(String,Option<tokio::sync::oneshot::Sender<bool>>)>(1);
        let state=Arc::new(Mutex::new((String::new(),0,std::time::Instant::now())));
        let progress=state.clone();
        let worker=tokio::spawn(async move{
            let mut interval=tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    item=receiver.recv()=>match item {
                        Some((content,ack))=>{let result=repo.checkpoint(&user,&task,&content).await;if let Some(ack)=ack{let _=ack.send(result.is_ok());}result?;},
                        None=>break,
                    },
                    _=interval.tick()=>{
                        let snapshot={let mut s=progress.lock().unwrap_or_else(|e|e.into_inner());if s.0.len()>s.1{s.1=s.0.len();s.2=std::time::Instant::now();Some(s.0.clone())}else{None}};
                        if let Some(content)=snapshot{repo.checkpoint(&user,&task,&content).await?;}
                    },
                }
            }
            Ok(())
        });
        Self{sender,state,worker:Mutex::new(Some(worker))}
    }
    pub fn push(&self,text:&str){
        let mut s=self.state.lock().unwrap_or_else(|e|e.into_inner());
        let room=MAX_REPLY_BYTES.saturating_sub(s.0.len());let (piece,_)=split_at_boundary(text,room);s.0.push_str(piece);
        if s.0.len().saturating_sub(s.1)>=4096||s.2.elapsed()>=Duration::from_secs(1){
            if self.sender.try_send((s.0.clone(),None)).is_ok(){s.1=s.0.len();s.2=std::time::Instant::now()}
        }
    }
    pub fn content(&self)->String{self.state.lock().unwrap_or_else(|e|e.into_inner()).0.clone()}
    pub async fn flush(&self)->std::result::Result<(),String>{
        let (ack,done)=tokio::sync::oneshot::channel();
        self.sender.send((self.content(),Some(ack))).await.map_err(|_|"persistence_failed".to_string())?;
        match done.await{Ok(true)=>Ok(()),_=>Err("persistence_failed".into())}
    }
}
impl Drop for CheckpointWriter {
    fn drop(&mut self){if let Some(worker)=self.worker.get_mut().unwrap_or_else(|e|e.into_inner()).take(){
        // Dropping the final sender closes the bounded writer; it drains the final snapshot.
        drop(worker);
    }}
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
}
