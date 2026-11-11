use super::*;

const MAX_SUMMARY_SOURCE_BYTES: i64 = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatSummary {
    pub id: String,
    pub source_first_seq: i64,
    pub source_last_seq: i64,
    pub source_digest: String,
    pub content: String,
    pub provider_id: String,
    pub model_id: String,
    pub created_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatRunStep {
    pub id: String, pub ordinal: i64, pub kind: String, pub phase: String, pub status: String,
    pub dispatched: bool, pub result_unknown: bool, pub receipt: Option<serde_json::Value>,
    pub created_at: i64, pub updated_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatRun {
    pub task_id: String, pub session_id: String, pub assistant_message_id: String,
    pub source_run_id: Option<String>, pub status: String, pub partial_content: String,
    pub dispatched: bool, pub result_unknown: bool, pub audit_warning: Option<String>,
    pub total_tokens: Option<i64>, pub steps: Vec<ChatRunStep>, pub created_at: i64, pub updated_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SummarySourceRow { pub id: String, pub seq: i64, pub role: String, pub content: String, pub status: String }
async fn source_in(db: &impl ConnectionTrait, user: &str, session: &str, last: i64) -> Result<Vec<SummarySourceRow>> {
    db.query_all(stmt("SELECT id,seq,role,content,status FROM chat_messages WHERE user_id=? AND session_id=? AND seq<=? AND replaces_id IS NULL ORDER BY seq",vec![user.into(),session.into(),last.into()])).await?.iter().map(|r| Ok(SummarySourceRow {id:r.try_get("","id")?,seq:r.try_get("","seq")?,role:r.try_get("","role")?,content:r.try_get("","content")?,status:r.try_get("","status")?})).collect()
}
async fn source_state_in(db: &impl ConnectionTrait, user: &str, session: &str, covered_seq: i64) -> Result<(bool, bool, bool)> {
    let row = db.query_one(stmt(
        "SELECT COALESCE(MAX(CASE WHEN m.role='assistant' AND m.seq>?1 THEN 1 ELSE 0 END),0) AS has_new_assistant, \
         COALESCE(SUM(LENGTH(CAST(m.content AS BLOB))),0)>?2 AS source_too_large, \
         COALESCE(MAX(CASE WHEN m.status='generating' THEN 1 ELSE 0 END),0) AS source_generating \
         FROM chat_sessions s LEFT JOIN chat_messages m ON m.session_id=s.id AND m.user_id=s.user_id AND m.replaces_id IS NULL \
         WHERE s.user_id=?3 AND s.id=?4 GROUP BY s.id",
        vec![covered_seq.into(), MAX_SUMMARY_SOURCE_BYTES.into(), user.into(), session.into()],
    )).await?.ok_or(ChatStoreError::SessionNotFound)?;
    Ok((row.try_get::<i64>("", "has_new_assistant")? != 0,
        row.try_get::<i64>("", "source_too_large")? != 0,
        row.try_get::<i64>("", "source_generating")? != 0))
}
impl ChatRepository {
    /// Busy is account-wide; unresolved effect evidence is scoped to this session.
    /// Missing model usage alone does not make an external tool effect unresolved.
    pub async fn summary_state(&self, user: &str, session: &str, exclude_task: Option<&str>) -> Result<(bool, bool)> {
        let row = self.db.query_one(stmt(
            "SELECT EXISTS(SELECT 1 FROM chat_runs r WHERE r.user_id=s.user_id AND r.status='running' AND (?1 IS NULL OR r.task_id<>?1)) AS busy, \
             EXISTS(SELECT 1 FROM chat_runs r WHERE r.user_id=s.user_id AND r.session_id=s.id AND (r.audit_warning IS NOT NULL OR EXISTS(SELECT 1 FROM chat_run_steps step WHERE step.task_id=r.task_id AND step.kind='tool' AND (step.result_unknown=1 OR step.status='dispatched')))) AS unresolved \
             FROM chat_sessions s WHERE s.user_id=?2 AND s.id=?3",
            vec![exclude_task.map(str::to_owned).into(), user.into(), session.into()],
        )).await?.ok_or(ChatStoreError::SessionNotFound)?;
        Ok((row.try_get::<i64>("", "busy")? != 0, row.try_get::<i64>("", "unresolved")? != 0))
    }
    /// Eligibility metadata without materializing source text. Byte accounting matches
    /// summary_source's UTF-8 content limit, including incomplete source messages.
    pub async fn summary_candidate_state(&self, user: &str, session: &str, covered_seq: i64) -> Result<(bool, bool, bool)> {
        source_state_in(&self.db, user, session, covered_seq).await
    }
    /// Bounded snapshot, including incomplete statuses so unknown work is never summarized as success.
    pub async fn summary_source(&self,user:&str,session:&str)->Result<Vec<SummarySourceRow>> {
        let transaction = self.db.begin().await?;
        let (_, too_large, _) = source_state_in(&transaction, user, session, 0).await?;
        if too_large { return Err(ChatStoreError::Db("summary source too large".into())); }
        let rows = source_in(&transaction, user, session, i64::MAX).await?;
        transaction.commit().await?;
        Ok(rows)
    }
    pub async fn summary(&self,user:&str,session:&str)->Result<Option<ChatSummary>> {
        session_in(&self.db,user,session).await?;
        let Some(r)=self.db.query_one(stmt("SELECT * FROM chat_summaries WHERE user_id=? AND session_id=?",vec![user.into(),session.into()])).await? else{return Ok(None)};
        let last:i64=r.try_get("","source_last_seq")?;
        let saved:String=r.try_get("","source_json")?;
        let source=source_in(&self.db,user,session,last).await?;
        if serde_json::to_string(&source).map_err(|e|ChatStoreError::Db(e.to_string()))?!=saved {return Ok(None)}
        Ok(Some(ChatSummary{id:r.try_get("","id")?,source_first_seq:r.try_get("","source_first_seq")?,source_last_seq:last,source_digest:r.try_get("","source_digest")?,content:r.try_get("","content")?,provider_id:r.try_get("","provider_id")?,model_id:r.try_get("","model_id")?,created_at:r.try_get("","created_at")?}))
    }
    /// Source CAS and install happen in one short transaction; original messages are untouched.
    pub async fn install_summary(&self,user:&str,session:&str,source:&[SummarySourceRow],summary:&ChatSummary,terminal:Option<(&str,&ChatTerminalEvidence,Option<i64>)>,valid:impl Fn()->bool)->Result<bool> {
        let tx=self.db.begin().await?;
        session_in(&tx,user,session).await?;
        let actual=source_in(&tx,user,session,summary.source_last_seq).await?;
        if !valid() || actual!=source {return Ok(false)}
        let source_json=serde_json::to_string(source).map_err(|e|ChatStoreError::Db(e.to_string()))?;
        tx.execute(stmt("INSERT INTO chat_summaries(id,user_id,session_id,source_first_seq,source_last_seq,source_digest,source_json,content,provider_id,model_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(session_id) DO UPDATE SET id=excluded.id,source_first_seq=excluded.source_first_seq,source_last_seq=excluded.source_last_seq,source_digest=excluded.source_digest,source_json=excluded.source_json,content=excluded.content,provider_id=excluded.provider_id,model_id=excluded.model_id,created_at=excluded.created_at",vec![summary.id.clone().into(),user.into(),session.into(),summary.source_first_seq.into(),summary.source_last_seq.into(),summary.source_digest.clone().into(),source_json.into(),summary.content.clone().into(),summary.provider_id.clone().into(),summary.model_id.clone().into(),summary.created_at.into()])).await?;
        if let Some((task,evidence,tokens))=terminal {
            let sealed=tx.execute(stmt("UPDATE chat_runs SET status='completed',dispatched=?,result_unknown=?,audit_warning=?,total_tokens=?,partial_content=?,updated_at=? WHERE task_id=? AND user_id=? AND session_id=? AND kind='summary' AND status='running'",vec![i64::from(evidence.dispatched).into(),i64::from(evidence.result_unknown).into(),evidence.audit_warning.clone().into(),tokens.into(),summary.content.clone().into(),now().into(),task.into(),user.into(),session.into()])).await?;
            if sealed.rows_affected()!=1{return Err(ChatStoreError::RequestConflict)}
        }
        if !valid(){return Ok(false)}tx.commit().await?; Ok(true)
    }
    pub async fn list_runs(&self,user:&str,session:&str)->Result<Vec<ChatRun>> {
        session_in(&self.db,user,session).await?;
        let ids=self.db.query_all(stmt("SELECT task_id FROM chat_runs WHERE user_id=? AND session_id=? ORDER BY created_at DESC,task_id DESC LIMIT 100",vec![user.into(),session.into()])).await?;
        let mut runs=Vec::with_capacity(ids.len());
        for r in ids {runs.push(self.get_run(user,&r.try_get::<String>("","task_id")?).await?)}
        Ok(runs)
    }
    pub async fn get_run(&self,user:&str,task:&str)->Result<ChatRun> {
        // One read transaction prevents a terminal row from being paired with stale or newer
        // steps. This query never recovers, dispatches or replays work.
        let tx=self.db.begin().await?;
        let r=tx.query_one(stmt("SELECT * FROM chat_runs WHERE user_id=? AND task_id=?",vec![user.into(),task.into()])).await?.ok_or(ChatStoreError::MessageNotFound)?;
        let rows=tx.query_all(stmt("SELECT s.* FROM chat_run_steps s JOIN chat_runs r ON r.task_id=s.task_id WHERE r.user_id=? AND r.task_id=? ORDER BY s.ordinal LIMIT 128",vec![user.into(),task.into()])).await?;
        let steps=rows.iter().map(|s|Ok(ChatRunStep{id:s.try_get("","id")?,ordinal:s.try_get("","ordinal")?,kind:s.try_get("","kind")?,phase:s.try_get("","phase")?,status:s.try_get("","status")?,dispatched:s.try_get::<i64>("","dispatched")?!=0,result_unknown:s.try_get::<i64>("","result_unknown")?!=0,receipt:s.try_get::<Option<String>>("","receipt")?.map(|v|serde_json::from_str(&v).map_err(|e|ChatStoreError::Db(e.to_string()))).transpose()?,created_at:s.try_get("","created_at")?,updated_at:s.try_get("","updated_at")?})).collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(ChatRun{task_id:r.try_get("","task_id")?,session_id:r.try_get("","session_id")?,assistant_message_id:r.try_get("","assistant_message_id")?,source_run_id:r.try_get("","source_run_id")?,status:r.try_get("","status")?,partial_content:r.try_get("","partial_content")?,dispatched:r.try_get::<i64>("","dispatched")?!=0,result_unknown:r.try_get::<i64>("","result_unknown")?!=0,audit_warning:r.try_get("","audit_warning")?,total_tokens:r.try_get("","total_tokens")?,steps,created_at:r.try_get("","created_at")?,updated_at:r.try_get("","updated_at")?})
    }
    pub async fn checkpoint(&self,user:&str,task:&str,content:&str)->Result<()> {
        if content.len()>64*1024 {return Err(ChatStoreError::Db("checkpoint limit".into()))}
        let tx=self.db.begin().await?;
        // Chat's one versioned writer serializes current-segment updates, including resets.
        tx.execute(stmt("UPDATE chat_runs SET partial_content=?,updated_at=? WHERE task_id=? AND user_id=? AND status='running'",vec![content.into(),now().into(),task.into(),user.into()])).await?;
        tx.execute(stmt("UPDATE chat_messages SET content=?,updated_at=? WHERE task_id=? AND user_id=? AND role='assistant' AND status='generating'",vec![content.into(),now().into(),task.into(),user.into()])).await?;
        tx.commit().await?;Ok(())
    }
    pub async fn record_step(&self,user:&str,task:&str,kind:&str,phase:&str,status:&str,dispatched:bool,unknown:bool,receipt:Option<serde_json::Value>)->Result<ChatRunStep> {
        let tx=self.db.begin().await?;
        let run=tx.query_one(stmt("SELECT task_id FROM chat_runs WHERE user_id=? AND task_id=? AND status='running'",vec![user.into(),task.into()])).await?.ok_or(ChatStoreError::MessageNotFound)?;drop(run);
        let count=tx.query_one(stmt("SELECT COUNT(*) AS n FROM chat_run_steps WHERE task_id=?",vec![task.into()])).await?.ok_or(ChatStoreError::MessageNotFound)?.try_get::<i64>("","n")?;
        if count>=128{return Err(ChatStoreError::Db("step budget exceeded".into()))}
        let step=ChatRunStep{id:Uuid::new_v4().to_string(),ordinal:count,kind:kind.into(),phase:phase.into(),status:status.into(),dispatched,result_unknown:unknown,receipt,created_at:now(),updated_at:now()};
        tx.execute(stmt("INSERT INTO chat_run_steps(id,task_id,ordinal,kind,phase,status,dispatched,result_unknown,receipt,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)",vec![step.id.clone().into(),task.into(),count.into(),kind.into(),phase.into(),status.into(),i64::from(dispatched).into(),i64::from(unknown).into(),step.receipt.as_ref().map(serde_json::Value::to_string).into(),step.created_at.into(),step.updated_at.into()])).await?;
        tx.execute(stmt("UPDATE chat_runs SET dispatched=MAX(dispatched,?),result_unknown=MAX(result_unknown,?),updated_at=? WHERE task_id=? AND user_id=?",vec![i64::from(dispatched).into(),i64::from(unknown).into(),now().into(),task.into(),user.into()])).await?;
        tx.commit().await?;Ok(step)
    }
    /// Mark the pending dispatch settled by its authoritative call identity; never infer success.
    pub async fn settle_tool_step(&self,user:&str,task:&str,call_id:&str,unknown:bool,receipt:serde_json::Value)->Result<()> {
        let tx=self.db.begin().await?;
        tx.execute(stmt("UPDATE chat_run_steps SET status=?,result_unknown=?,receipt=?,updated_at=? WHERE task_id=? AND kind='tool' AND status='dispatched' AND json_extract(receipt,'$.call_id')=? AND EXISTS(SELECT 1 FROM chat_runs WHERE task_id=? AND user_id=?)",vec![if unknown{"unknown"}else{"settled"}.into(),i64::from(unknown).into(),receipt.to_string().into(),now().into(),task.into(),call_id.into(),task.into(),user.into()])).await?;
        tx.execute(stmt("UPDATE chat_runs SET result_unknown=EXISTS(SELECT 1 FROM chat_run_steps WHERE task_id=? AND result_unknown=1) WHERE task_id=? AND user_id=?",vec![task.into(),task.into(),user.into()])).await?;
        tx.commit().await?;Ok(())
    }
    pub async fn seal_run(&self,user:&str,task:&str,status:&str,dispatched:bool,unknown:bool,audit:Option<&str>,tokens:Option<i64>)->Result<()> {
        self.db.execute(stmt("UPDATE chat_runs SET status=?,dispatched=MAX(dispatched,?),result_unknown=MAX(result_unknown,?),audit_warning=?,total_tokens=?,updated_at=? WHERE task_id=? AND user_id=? AND status='running'",vec![status.into(),i64::from(dispatched).into(),i64::from(unknown).into(),audit.map(str::to_owned).into(),tokens.into(),now().into(),task.into(),user.into()])).await?;Ok(())
    }
    pub async fn begin_summary_run(&self,user:&str,session:&str,task:&str,assistant:&str,identity:&str)->Result<bool> {
        let tx=self.db.begin().await?;session_in(&tx,user,session).await?;
        if let Some(r)=tx.query_one(stmt("SELECT user_id,session_id,request_identity,kind FROM chat_runs WHERE task_id=?",vec![task.into()])).await? {
            if r.try_get::<String>("","user_id")?!=user||r.try_get::<String>("","session_id")?!=session||r.try_get::<Option<String>>("","request_identity")?.as_deref()!=Some(identity)||r.try_get::<String>("","kind")?!="summary"{return Err(ChatStoreError::RequestConflict)}
            return Ok(false)
        }
        if tx.query_one(stmt("SELECT id FROM workbench_tasks WHERE id=?",vec![task.into()])).await?.is_some(){return Err(ChatStoreError::RequestConflict)}
        tx.execute(stmt("INSERT INTO chat_runs(task_id,user_id,session_id,assistant_message_id,kind,request_identity,created_at,updated_at) VALUES(?,?,?,?,'summary',?,?,?)",vec![task.into(),user.into(),session.into(),assistant.into(),identity.into(),now().into(),now().into()])).await?;
        tx.commit().await?;Ok(true)
    }
    /// A manual report is distinct from an authoritative receipt. The original unknown evidence
    /// remains nested and its stable effect identity is still forbidden in future continuations.
    pub async fn manually_resolve_step(&self,user:&str,task:&str,step_id:&str,outcome:&str,note:&str)->Result<ChatRun> {
        let run=self.get_run(user,task).await?;
        if run.status=="running"||run.audit_warning.is_some(){return Err(ChatStoreError::GenerationInProgress)}
        let step=run.steps.iter().find(|s|s.id==step_id&&s.kind=="tool"&&(s.result_unknown||s.status=="dispatched")).ok_or(ChatStoreError::MessageNotFound)?;
        let receipt=serde_json::json!({"resolution":"user_reported_not_authoritatively_verified","reported_outcome":outcome,"note":note,"original_receipt":step.receipt,"effect_key":step.receipt.as_ref().and_then(|r|r.get("effect_key"))});
        let tx=self.db.begin().await?;
        let updated=tx.execute(stmt("UPDATE chat_run_steps SET status='manually_resolved',result_unknown=0,receipt=?,updated_at=? WHERE id=? AND task_id=? AND result_unknown=1 AND EXISTS(SELECT 1 FROM chat_runs WHERE task_id=? AND user_id=? AND status<>'running' AND audit_warning IS NULL)",vec![receipt.to_string().into(),now().into(),step_id.into(),task.into(),task.into(),user.into()])).await?;
        if updated.rows_affected()!=1{return Err(ChatStoreError::MessageNotFound)}
        tx.execute(stmt("UPDATE chat_runs SET result_unknown=EXISTS(SELECT 1 FROM chat_run_steps WHERE task_id=? AND result_unknown=1),updated_at=? WHERE task_id=? AND user_id=?",vec![task.into(),now().into(),task.into(),user.into()])).await?;
        tx.commit().await?;self.get_run(user,task).await
    }
    pub async fn recover_runs(&self,user:Option<&str>)->Result<()> {
        let tx=self.db.begin().await?;
        crate::ChatUsageRepository::recover_pending_in_transaction(&tx,user).await?;
        let filter=if user.is_some(){" AND user_id=?"}else{""};
        let mut args=vec![now().into()];if let Some(user)=user{args.push(user.into())}
        tx.execute(stmt(&format!("UPDATE chat_runs SET status='interrupted',updated_at=?,result_unknown=CASE WHEN EXISTS(SELECT 1 FROM chat_run_steps WHERE task_id=chat_runs.task_id AND (kind='model' OR (kind='tool' AND status='dispatched'))) THEN 1 ELSE result_unknown END WHERE status='running'{filter}"),args)).await?;
        let args=user.map(|u|vec![u.into()]).unwrap_or_default();
        tx.execute(stmt(&format!("UPDATE chat_run_steps SET status='unknown',result_unknown=1 WHERE kind='tool' AND status='dispatched' AND task_id IN (SELECT task_id FROM chat_runs WHERE status='interrupted'{filter})"),args)).await?;
        let args=user.map(|u|vec![u.into()]).unwrap_or_default();
        tx.execute(stmt(&format!("UPDATE chat_run_steps SET status='failed',result_unknown=0,receipt=json_set(receipt,'$.web_activity.status','failed','$.web_activity.error','web_cancelled') WHERE kind='web' AND status='running' AND task_id IN (SELECT task_id FROM chat_runs WHERE status='interrupted'{filter}) AND NOT EXISTS(SELECT 1 FROM chat_run_steps outcome WHERE outcome.task_id=chat_run_steps.task_id AND outcome.kind='web' AND outcome.status<>'running' AND json_extract(outcome.receipt,'$.web_activity.id')=json_extract(chat_run_steps.receipt,'$.web_activity.id'))"),args)).await?;
        tx.commit().await?;Ok(())
    }
}
