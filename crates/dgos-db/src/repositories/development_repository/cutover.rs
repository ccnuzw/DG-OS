use super::*;
use super::capabilities::{active_project, head};
use sea_orm::TransactionTrait;

pub(super) const CUTOVER_KEY: &str = "d136-cli-provider-v1";
const MAX_CONFIGURATION: usize = 65_536;
const MAX_SNAPSHOT: usize = 1_048_576;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliProviderConfiguration {
    pub engine: String,
    pub provider_id: String,
    pub model_id: String,
    pub provider_revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliProviderSnapshot {
    pub binding_id: String,
    pub engine: String,
    pub provider_id: String,
    pub model_id: String,
    pub provider_revision: i64,
    pub revision: i64,
    pub permission_epoch: i64,
    #[serde(flatten)]
    pub evidence: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpeningBinding {
    pub binding_id: String,
    pub project_id: String,
    pub task_id: String,
    pub revision: i64,
    pub permission_epoch: i64,
    pub provider_id: String,
    pub model_id: String,
    pub provider_revision: i64,
    pub host_instance_id: String,
    pub boot_identity: Option<String>,
    pub request_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCliBinding {
    pub owner: String,
    pub session_id: String,
    pub binding_id: String,
    pub execution_policy: String,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
    pub revision: Option<i64>,
    pub permission_epoch: i64,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub provider_revision: Option<i64>,
    pub state: String,
    pub cleanup_state: String,
    pub cleanup_reason: Option<String>,
    pub host_instance_id: Option<String>,
    pub boot_identity: Option<String>,
    pub cleanup_proof: Option<String>,
    pub request_digest: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutoverContext {
    pub boot_identity: Option<String>,
    /// Sessions whose owned process groups were actually observed stopped by the host.
    pub owned_exit_sessions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutoverReceipt {
    pub already_complete: bool,
    pub archived_sessions: u64,
    pub cleanup_pending_sessions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpeningReceipt {
    pub binding: StoredCliBinding,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CliRevocationScope {
    Project { project_id: String },
    Provider { provider_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupTarget {
    pub owner: String,
    pub session_id: String,
    pub binding_id: String,
    pub host_instance_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupObservation {
    pub binding_id: String,
    pub state: String,
    pub proof: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupReceipt {
    pub matched: bool,
    pub state: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostBootObservation {
    pub boot_identity: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedCutoverView {
    pub complete: bool,
    pub local_cleanup_pending: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionConfiguration {
    pub legacy_read_only: bool,
    pub binding: Option<StoredCliBinding>,
    pub snapshot: Option<String>,
    pub legacy_history: Option<String>,
}

impl DevelopmentRepository {
    pub async fn migrate_cli_provider_v1(&self, context: &CutoverContext) -> Result<CutoverReceipt, String> {
        let tx = self.db.begin().await.map_err(db)?;
        let result = async {
            if fetch_one(&tx, "SELECT key FROM development_migrations WHERE key=?", vec![v(CUTOVER_KEY)]).await?.is_some() {
                return Ok(CutoverReceipt { already_complete: true, archived_sessions: 0, cleanup_pending_sessions: Vec::new() });
            }
            execute(&tx, "UPDATE development_capability_heads SET permission_epoch=permission_epoch", vec![]).await?;
            let old = fetch(&tx, "SELECT s.id,s.owner,s.engine,s.native_id,s.workspace,s.executable,s.model,s.project_id,s.task_id,c.snapshot,c.revision,c.permission_epoch,r.configuration FROM development_agent_sessions s LEFT JOIN development_session_capabilities c ON c.owner=s.owner AND c.session_id=s.id LEFT JOIN development_capability_revisions r ON r.owner=c.owner AND r.project_id=c.project_id AND r.revision=c.revision WHERE NOT EXISTS(SELECT 1 FROM development_cli_session_bindings b WHERE b.owner=s.owner AND b.session_id=s.id) ORDER BY s.owner,s.id", vec![]).await?;
            for row in &old {
                let id: String = row.try_get("", "id").map_err(db)?;
                let owner: String = row.try_get("", "owner").map_err(db)?;
                let project: Option<String> = row.try_get("", "project_id").map_err(db)?;
                let task: Option<String> = row.try_get("", "task_id").map_err(db)?;
                let native_id: Option<String> = row.try_get("", "native_id").map_err(db)?;
                let engine: String = row.try_get("", "engine").map_err(db)?;
                let workspace: String = row.try_get("", "workspace").map_err(db)?;
                let executable: Option<String> = row.try_get("", "executable").map_err(db)?;
                let model: Option<String> = row.try_get("", "model").map_err(db)?;
                let session_record = serde_json::json!({"engine":engine,"native_id":native_id,"workspace":workspace,"executable":executable,"model":model,"original_project_id":project,"original_task_id":task}).to_string();
                let snapshot: Option<String> = row.try_get("", "snapshot").map_err(db)?;
                let revision: Option<i64> = row.try_get("", "revision").map_err(db)?;
                let epoch: Option<i64> = row.try_get("", "permission_epoch").map_err(db)?;
                let configuration: Option<String> = row.try_get("", "configuration").map_err(db)?;
                if session_record.len() > MAX_SNAPSHOT || snapshot.as_ref().is_some_and(|s| s.len() > MAX_SNAPSHOT) || configuration.as_ref().is_some_and(|s| s.len() > MAX_CONFIGURATION) {
                    return Err("storage_failed".into());
                }
                execute(&tx, "INSERT INTO development_cli_legacy_history(owner,session_id,original_project_id,original_task_id,session_record,capability_snapshot,referenced_configuration,original_revision,original_permission_epoch,archived_at) VALUES(?,?,?,?,?,?,?,?,?,unixepoch()) ON CONFLICT(owner,session_id) DO NOTHING", vec![v(owner.clone()),v(id.clone()),v(project.clone()),v(task.clone()),v(session_record),v(snapshot),v(configuration),v(revision),v(epoch)]).await?;
                let running: bool = fetch_one(&tx, "SELECT 1 AS yes FROM development_runs WHERE owner=? AND session_id=? AND status IN ('running','waiting') LIMIT 1", vec![v(owner.clone()),v(id.clone())]).await?.is_some();
                let confirmed = context.owned_exit_sessions.iter().any(|s| s == &id);
                if running { execute(&tx, "UPDATE development_runs SET status='unknown',finished_at=unixepoch(),reason='interrupted_by_cutover' WHERE owner=? AND session_id=? AND status IN ('running','waiting')", vec![v(owner.clone()),v(id.clone())]).await?; }
                let state = if running { "unknown" } else { "closed" };
                let cleanup_state = if confirmed { "stopped" } else { "not_required" };
                let proof = if confirmed { Some("owned_exit") } else { None };
                let binding_id = format!("legacy:{id}");
                execute(&tx, "INSERT INTO development_cli_session_bindings(owner,session_id,binding_id,execution_policy,project_id,task_id,revision,permission_epoch,provider_id,model_id,provider_revision,state,cleanup_state,cleanup_reason,host_instance_id,boot_identity,cleanup_proof,request_digest,updated_at) VALUES(?,?,?,'legacy_read_only',?, ?,NULL,?,NULL,NULL,NULL,?,?,?,NULL,?,?,NULL,unixepoch())", vec![v(owner),v(id),v(binding_id),v(project),v(task),v(epoch.unwrap_or(0)),v(state),v(cleanup_state),v(if running { Some("external_result_unknown") } else { None }),v(context.boot_identity.clone()),v(proof)]).await?;
            }
            execute(&tx, "UPDATE development_capability_heads SET revision=NULL,permission_epoch=permission_epoch+1", vec![]).await?;
            execute(&tx, "INSERT INTO development_migrations(key,cleanup_boot_identity) VALUES(?,?)", vec![v(CUTOVER_KEY),v(context.boot_identity.clone())]).await?;
            Ok::<_, String>(CutoverReceipt { already_complete: false, archived_sessions: old.len() as u64, cleanup_pending_sessions: Vec::new() })
        }.await;
        match result { Ok(receipt) => { tx.commit().await.map_err(db)?; Ok(receipt) }, Err(error) => { let _ = tx.rollback().await; Err(error) } }
    }

    pub async fn save_cli_provider_revision(&self, owner: &str, project_id: &str, expected_revision: i64, configuration: &CliProviderConfiguration) -> Result<StoredCapabilityRevision, String> {
        if expected_revision < 0 || configuration.provider_revision < 0 || !matches!(configuration.engine.as_str(), "codex" | "claude") || configuration.provider_id.is_empty() || configuration.model_id.is_empty() { return Err("invalid_capabilities".into()); }
        let encoded = serde_json::to_string(configuration).map_err(|_| "invalid_capabilities")?;
        if encoded.len() > MAX_CONFIGURATION { return Err("invalid_capabilities".into()); }
        let tx = self.db.begin().await.map_err(db)?;
        let result = async {
            execute(&tx, "UPDATE development_capability_heads SET permission_epoch=permission_epoch WHERE owner=? AND project_id=?", vec![v(owner),v(project_id)]).await?;
            active_project(&tx, owner, project_id).await?;
            if fetch_one(&tx, "SELECT 1 AS complete FROM development_migrations WHERE key=?", vec![v(CUTOVER_KEY)]).await?.is_none() { return Err("cli_cutover_pending".into()); }
            let current: i64 = fetch_one(&tx, "SELECT COALESCE(revision,0) AS revision FROM development_capability_heads WHERE owner=? AND project_id=?", vec![v(owner),v(project_id)]).await?.map(|r| r.try_get("", "revision").map_err(db)).transpose()?.unwrap_or(0);
            if current != expected_revision { return Err("capability_conflict".into()); }
            let maximum: i64 = fetch_one(&tx, "SELECT COALESCE(MAX(revision),0) AS revision FROM development_capability_revisions WHERE owner=? AND project_id=?", vec![v(owner),v(project_id)]).await?.ok_or("storage_failed")?.try_get("", "revision").map_err(db)?;
            let revision = maximum.checked_add(1).ok_or("capability_conflict")?;
            let epoch = fetch_one(&tx, "SELECT permission_epoch FROM development_capability_heads WHERE owner=? AND project_id=?", vec![v(owner),v(project_id)]).await?.map(|r| r.try_get("", "permission_epoch").map_err(db)).transpose()?.unwrap_or(0);
            execute(&tx,"INSERT INTO development_capability_revisions(owner,project_id,revision,configuration,created_at,format) VALUES(?,?,?,?,unixepoch(),'cli_provider_v1')",vec![v(owner),v(project_id),v(revision),v(encoded.clone())]).await?;
            if expected_revision == 0 {
                execute(&tx,"INSERT INTO development_capability_heads(owner,project_id,revision,permission_epoch) VALUES(?,?,?,?) ON CONFLICT(owner,project_id) DO UPDATE SET revision=excluded.revision WHERE development_capability_heads.revision IS NULL",vec![v(owner),v(project_id),v(revision),v(epoch)]).await?;
            } else {
                let changed = execute(&tx,"UPDATE development_capability_heads SET revision=? WHERE owner=? AND project_id=? AND revision=?",vec![v(revision),v(owner),v(project_id),v(expected_revision)]).await?;
                if changed != 1 { return Err("capability_conflict".into()); }
            }
            let created_at = fetch_one(&tx,"SELECT created_at FROM development_capability_revisions WHERE owner=? AND project_id=? AND revision=?",vec![v(owner),v(project_id),v(revision)]).await?.ok_or("storage_failed")?.try_get("","created_at").map_err(db)?;
            Ok::<_,String>(StoredCapabilityRevision { revision, permission_epoch: epoch, configuration: encoded, created_at })
        }.await;
        match result { Ok(value) => { tx.commit().await.map_err(db)?; Ok(value) }, Err(error) => { let _ = tx.rollback().await; Err(error) } }
    }

    pub async fn reserve_cli_session(&self, owner: &str, session: &StoredAgentSession, opening: &OpeningBinding, snapshot: &CliProviderSnapshot) -> Result<OpeningReceipt, String> {
        let encoded = encode_snapshot(snapshot)?;
        validate_opening(owner, session, opening, snapshot)?;
        let tx = self.db.begin().await.map_err(db)?;
        let result = async {
            execute(&tx,"UPDATE development_capability_heads SET permission_epoch=permission_epoch WHERE owner=? AND project_id=?",vec![v(owner),v(opening.project_id.clone())]).await?;
            if fetch_one(&tx, "SELECT 1 AS ready FROM development_migrations WHERE key=?", vec![v(CUTOVER_KEY)]).await?.is_none() { return Err("cli_cutover_pending".into()); }
            active_project(&tx, owner, &opening.project_id).await?;
            let task = task_row(&tx, owner, &opening.task_id).await?;
            if task.project_id != opening.project_id || session.owner != owner || session.project_id.as_deref()!=Some(&opening.project_id) || session.task_id.as_deref()!=Some(&opening.task_id) || session.engine != snapshot.engine { return Err("invalid_cli_binding".into()); }
            let head = head(&tx, owner, &opening.project_id).await?.ok_or("capability_conflict")?;
            if head != (opening.revision, opening.permission_epoch) { return Err("capability_revoked".into()); }
            let revision = fetch_one(&tx,"SELECT configuration,format FROM development_capability_revisions WHERE owner=? AND project_id=? AND revision=?",vec![v(owner),v(opening.project_id.clone()),v(opening.revision)]).await?.ok_or("capability_conflict")?;
            let format: String = revision.try_get("", "format").map_err(db)?;
            if format != "cli_provider_v1" { return Err("legacy_session_read_only".into()); }
            let config: String = revision.try_get("", "configuration").map_err(db)?;
            let parsed: CliProviderConfiguration = serde_json::from_str(&config).map_err(|_| "storage_failed")?;
            if parsed.engine!=snapshot.engine || parsed.provider_id!=snapshot.provider_id || parsed.model_id!=snapshot.model_id || parsed.provider_revision!=snapshot.provider_revision { return Err("provider_revision_changed".into()); }
            let previous = fetch_one(&tx,"SELECT * FROM development_agent_sessions WHERE owner=? AND id=?",vec![v(owner),v(session.id.clone())]).await?;
            match previous {
                None => {
                    execute(&tx,"INSERT INTO development_agent_sessions(id,owner,engine,native_id,workspace,executable,model,project_id,task_id) VALUES(?,?,?,?,?,?,?,?,?)",session_values(session)).await?;
                    execute(&tx,"INSERT INTO development_cli_session_bindings(owner,session_id,binding_id,execution_policy,project_id,task_id,revision,permission_epoch,provider_id,model_id,provider_revision,state,cleanup_state,host_instance_id,boot_identity,request_digest,updated_at) VALUES(?,?,?,'provider_bound',?,?,?,?,?,?,?,'opening','pending',?,?,?,unixepoch())",vec![v(owner),v(session.id.clone()),v(opening.binding_id.clone()),v(opening.project_id.clone()),v(opening.task_id.clone()),v(opening.revision),v(opening.permission_epoch),v(opening.provider_id.clone()),v(opening.model_id.clone()),v(opening.provider_revision),v(opening.host_instance_id.clone()),v(opening.boot_identity.clone()),v(opening.request_digest.clone())]).await?;
                }
                Some(row) => {
                    let existing = session_from(&row)?;
                    if existing.owner != owner || existing.engine != session.engine || existing.workspace != session.workspace || existing.project_id != session.project_id || existing.task_id != session.task_id { return Err("invalid_cli_binding".into()); }
                    let old = load_binding(&tx,owner,&session.id).await?;
                    if old.execution_policy != "provider_bound" || old.state != "closed" || !matches!(old.cleanup_state.as_str(),"stopped"|"not_required") { return Err("legacy_session_read_only".into()); }
                    if old.binding_id == opening.binding_id || old.provider_id.as_deref() != Some(&opening.provider_id) || old.model_id.as_deref() != Some(&opening.model_id) || old.provider_revision != Some(opening.provider_revision) || old.revision != Some(opening.revision) || old.permission_epoch != opening.permission_epoch || existing.native_id != session.native_id || existing.executable != session.executable || existing.model != session.model { return Err("invalid_cli_binding".into()); }
                    let changed=execute(&tx,"UPDATE development_cli_session_bindings SET binding_id=?,revision=?,permission_epoch=?,provider_id=?,model_id=?,provider_revision=?,state='opening',cleanup_state='pending',cleanup_reason=NULL,host_instance_id=?,boot_identity=?,cleanup_proof=NULL,request_digest=?,updated_at=unixepoch() WHERE owner=? AND session_id=? AND binding_id=? AND state='closed' AND cleanup_state IN ('stopped','not_required')",vec![v(opening.binding_id.clone()),v(opening.revision),v(opening.permission_epoch),v(opening.provider_id.clone()),v(opening.model_id.clone()),v(opening.provider_revision),v(opening.host_instance_id.clone()),v(opening.boot_identity.clone()),v(opening.request_digest.clone()),v(owner),v(session.id.clone()),v(old.binding_id)]).await?;
                    if changed != 1 { return Err("capability_revoked".into()); }
                }
            }
            let existing_snapshot = fetch_one(&tx,"SELECT format FROM development_session_capabilities WHERE owner=? AND session_id=?",vec![v(owner),v(session.id.clone())]).await?;
            if existing_snapshot.is_some() {
                execute(&tx,"UPDATE development_session_capabilities SET project_id=?,revision=?,permission_epoch=?,snapshot=?,format='cli_provider_v1' WHERE owner=? AND session_id=?",vec![v(opening.project_id.clone()),v(opening.revision),v(opening.permission_epoch),v(encoded),v(owner),v(session.id.clone())]).await?;
            } else {
                execute(&tx,"INSERT INTO development_session_capabilities(owner,session_id,project_id,revision,permission_epoch,snapshot,format) VALUES(?,?,?,?,?,?, 'cli_provider_v1')",vec![v(owner),v(session.id.clone()),v(opening.project_id.clone()),v(opening.revision),v(opening.permission_epoch),v(encoded)]).await?;
            }
            load_binding(&tx,owner,&session.id).await
        }.await;
        match result { Ok(binding) => { tx.commit().await.map_err(db)?; Ok(OpeningReceipt { binding }) }, Err(error) => { let _ = tx.rollback().await; Err(error) } }
    }

    pub async fn activate_cli_session(&self, owner: &str, session_id: &str, binding_id: &str, host_instance_id: &str, native_id: &str, expected_epoch: i64) -> Result<StoredCliBinding, String> {
        if native_id.is_empty() || native_id.len() > 4096 { return Err("invalid_cli_binding".into()); }
        let tx = self.db.begin().await.map_err(db)?;
        let result = async {
            execute(&tx,"UPDATE development_cli_session_bindings SET updated_at=updated_at WHERE owner=? AND session_id=?",vec![v(owner),v(session_id)]).await?;
            let row = load_binding(&tx,owner,session_id).await?;
            if row.binding_id!=binding_id || row.host_instance_id.as_deref()!=Some(host_instance_id) || row.state!="opening" || row.permission_epoch!=expected_epoch { return Err("capability_revoked".into()); }
            let head_now = head(&tx, owner, row.project_id.as_deref().ok_or("invalid_cli_binding")?).await?.ok_or("capability_revoked")?;
            if head_now.1 != expected_epoch || head_now.0 != row.revision.unwrap_or_default() { return Err("capability_revoked".into()); }
            let changed = execute(&tx,"UPDATE development_agent_sessions SET native_id=? WHERE owner=? AND id=? AND (native_id IS NULL OR native_id=?)",vec![v(native_id),v(owner),v(session_id),v(native_id)]).await?;
            if changed != 1 { return Err("invalid_cli_binding".into()); }
            let changed=execute(&tx,"UPDATE development_cli_session_bindings SET state='active',cleanup_state='not_required',cleanup_reason=NULL,updated_at=unixepoch() WHERE owner=? AND session_id=? AND binding_id=? AND host_instance_id=? AND state='opening' AND permission_epoch=?",vec![v(owner),v(session_id),v(binding_id),v(host_instance_id),v(expected_epoch)]).await?;
            if changed!=1 { return Err("capability_revoked".into()); }
            load_binding(&tx,owner,session_id).await
        }.await;
        match result { Ok(binding)=>{tx.commit().await.map_err(db)?;Ok(binding)},Err(error)=>{let _=tx.rollback().await;Err(error)} }
    }

    /// Persist exactly the process generation captured by a Provider notification.
    /// A delayed notification cannot revoke a later resume of the same session.
    pub async fn revoke_cli_binding(&self, target: &CleanupTarget) -> Result<bool,String> {
        execute(&self.db,"UPDATE development_cli_session_bindings SET state='revoked',cleanup_state=CASE WHEN state IN ('opening','active') THEN 'pending' ELSE cleanup_state END,cleanup_reason='provider_invalidated',updated_at=unixepoch() WHERE owner=? AND session_id=? AND binding_id=? AND host_instance_id=? AND execution_policy='provider_bound' AND state IN ('opening','active','closed','unknown')",vec![v(target.owner.clone()),v(target.session_id.clone()),v(target.binding_id.clone()),v(target.host_instance_id.clone())]).await.map(|changed|changed==1)
    }

    pub async fn revoke_cli_bindings(&self, owner: &str, scope: &CliRevocationScope) -> Result<Vec<CleanupTarget>, String> {
        let tx=self.db.begin().await.map_err(db)?;
        let result=async {
            execute(&tx,"UPDATE development_cli_session_bindings SET updated_at=updated_at WHERE owner=?",vec![v(owner)]).await?;
            let rows=match scope {
                CliRevocationScope::Project{project_id}=>{
                    project_row(&tx,owner,project_id).await?;
                    execute(&tx,"INSERT INTO development_capability_heads(owner,project_id,revision,permission_epoch) VALUES(?,?,NULL,1) ON CONFLICT(owner,project_id) DO UPDATE SET permission_epoch=permission_epoch+1,revision=NULL",vec![v(owner),v(project_id)]).await?;
                    fetch(&tx,"SELECT session_id,binding_id,host_instance_id FROM development_cli_session_bindings WHERE owner=? AND project_id=? AND execution_policy='provider_bound' AND state IN ('opening','active')",vec![v(owner),v(project_id)]).await?
                },
                CliRevocationScope::Provider{provider_id}=>{
                    execute(&tx,"UPDATE development_cli_session_bindings SET state='revoked',cleanup_reason='revocation',updated_at=unixepoch() WHERE owner=? AND provider_id=? AND execution_policy='provider_bound' AND state='closed'",vec![v(owner),v(provider_id)]).await?;
                    fetch(&tx,"SELECT session_id,binding_id,host_instance_id FROM development_cli_session_bindings WHERE owner=? AND provider_id=? AND execution_policy='provider_bound' AND state IN ('opening','active')",vec![v(owner),v(provider_id)]).await?
                }
            };
            let mut targets=Vec::new();
            for row in rows {
                let session_id:String=row.try_get("","session_id").map_err(db)?;
                let binding_id:String=row.try_get("","binding_id").map_err(db)?;
                let host:String=row.try_get("","host_instance_id").map_err(db)?;
                execute(&tx,"UPDATE development_cli_session_bindings SET state='revoked',cleanup_state='pending',cleanup_reason='revocation',updated_at=unixepoch() WHERE owner=? AND session_id=? AND binding_id=? AND host_instance_id=? AND state IN ('opening','active')",vec![v(owner),v(session_id.clone()),v(binding_id.clone()),v(host.clone())]).await?;
                targets.push(CleanupTarget{owner:owner.into(),session_id,binding_id,host_instance_id:host});
            }
            Ok::<_,String>(targets)
        }.await;
        match result{Ok(v)=>{tx.commit().await.map_err(db)?;Ok(v)},Err(e)=>{let _=tx.rollback().await;Err(e)}}
    }

    pub async fn record_cli_cleanup(&self, owner: &str, session_id: &str, expected_binding_id: &str, expected_host_instance_id: &str, observation: &CleanupObservation) -> Result<CleanupReceipt,String> {
        let valid_proof = match observation.state.as_str() {
            "closed" | "revoked" => matches!(observation.proof.as_str(), "owned_exit" | "no_execution"),
            "unknown" => observation.proof.is_empty(),
            _ => false,
        };
        if observation.binding_id!=expected_binding_id || !valid_proof || observation.reason.as_ref().is_some_and(|reason| reason.len()>512) { return Err("invalid_cli_binding".into()); }
        let target_cleanup = if observation.state == "unknown" { "unknown" } else { "stopped" };
        let proof = (!observation.proof.is_empty()).then_some(observation.proof.as_str());
        let changed=execute(&self.db,"UPDATE development_cli_session_bindings SET state=CASE WHEN state='revoked' THEN 'revoked' ELSE ? END,cleanup_state=?,cleanup_reason=?,cleanup_proof=?,updated_at=unixepoch() WHERE owner=? AND session_id=? AND binding_id=? AND host_instance_id=? AND ((state='revoked' AND cleanup_state IN ('pending','unknown')) OR (state IN ('opening','active','unknown') AND ? IN ('closed','unknown')))",vec![v(observation.state.clone()),v(target_cleanup),v(observation.reason.clone()),v(proof),v(owner),v(session_id),v(expected_binding_id),v(expected_host_instance_id),v(observation.state.clone())]).await?;
        Ok(CleanupReceipt{matched:changed==1,state:if changed==1{Some(target_cleanup.into())}else{None}})
    }

    /// Mark abandoned opening/active bindings from another host as unknown;
    /// this records no process-exit claim and never revives a lease.
    pub async fn recover_cli_bindings(&self, current_host_instance_id: &str, current_boot_identity: Option<&str>) -> Result<u64, String> {
        if current_host_instance_id.is_empty() || current_host_instance_id.len() > 256 || current_boot_identity.is_some_and(|boot| boot.is_empty() || boot.len() > 512) { return Err("invalid_cli_binding".into()); }
        execute(&self.db, "UPDATE development_cli_session_bindings SET state='unknown',cleanup_state='unknown',cleanup_reason='host_restart_cleanup_unconfirmed',cleanup_proof=NULL,updated_at=unixepoch() WHERE execution_policy='provider_bound' AND host_instance_id<>? AND state IN ('opening','active')", vec![v(current_host_instance_id)]).await
    }

    /// A boot change proves only local process absence. Machine identity must
    /// match; a different machine is not evidence of a reboot.
    pub async fn recheck_cli_cleanup(&self, proof: &HostBootObservation) -> Result<u64, String> {
        let (machine, boot) = proof.boot_identity.split_once(':').ok_or("invalid_cli_binding")?;
        if machine.is_empty() || boot.is_empty() || boot.contains(':') || proof.boot_identity.len() > 512 { return Err("invalid_cli_binding".into()); }
        execute(&self.db, "UPDATE development_cli_session_bindings SET cleanup_state='stopped',cleanup_proof='boot_changed',cleanup_reason=NULL,updated_at=unixepoch() WHERE cleanup_state IN ('pending','unknown') AND boot_identity IS NOT NULL AND instr(boot_identity,':')>1 AND length(boot_identity)>instr(boot_identity,':') AND substr(boot_identity,1,instr(boot_identity,':')-1)=? AND instr(substr(boot_identity,instr(boot_identity,':')+1),':')=0 AND substr(boot_identity,instr(boot_identity,':')+1)<>? AND state IN ('closed','unknown','revoked')", vec![v(machine), v(boot)]).await
    }
    pub async fn read_cli_cutover(&self,owner:&str)->Result<OwnedCutoverView,String>{
        let row=fetch_one(&self.db,"SELECT (SELECT COUNT(*) FROM development_migrations WHERE key=?) AS complete,(SELECT COUNT(*) FROM development_cli_session_bindings WHERE owner=? AND cleanup_state IN ('pending','unknown')) AS pending",vec![v(CUTOVER_KEY),v(owner)]).await?.ok_or("storage_failed")?;
        let pending: i64 = row.try_get("", "pending").map_err(db)?;
        Ok(OwnedCutoverView{complete:row.try_get::<i64>("","complete").map_err(db)?!=0,local_cleanup_pending:pending.max(0) as u64})
    }

    pub async fn read_cli_session_configuration(&self,owner:&str,session_id:&str)->Result<SessionConfiguration,String>{
        let row=fetch_one(&self.db,"SELECT b.*,c.snapshot,h.session_record FROM development_cli_session_bindings b LEFT JOIN development_session_capabilities c ON c.owner=b.owner AND c.session_id=b.session_id AND c.format='cli_provider_v1' LEFT JOIN development_cli_legacy_history h ON h.owner=b.owner AND h.session_id=b.session_id WHERE b.owner=? AND b.session_id=?",vec![v(owner),v(session_id)]).await?.ok_or("session_not_found")?;
        let binding=binding_from(&row)?;
        Ok(SessionConfiguration{legacy_read_only:binding.execution_policy=="legacy_read_only",snapshot:row.try_get("","snapshot").map_err(db)?,legacy_history:row.try_get("","session_record").map_err(db)?,binding:Some(binding)})
    }
}

fn validate_opening(owner:&str,session:&StoredAgentSession,opening:&OpeningBinding,snapshot:&CliProviderSnapshot)->Result<(),String>{
    if !std::path::Path::new(&session.workspace).is_absolute() || session.id.is_empty() || opening.binding_id.len()>256 || opening.host_instance_id.len()>256 || opening.boot_identity.as_ref().is_some_and(|boot|boot.is_empty() || boot.len()>256) || opening.request_digest.is_empty() { return Err("invalid_cli_binding".into()); }
    if owner.is_empty()||session.owner!=owner||session.project_id.as_deref()!=Some(&opening.project_id)||session.task_id.as_deref()!=Some(&opening.task_id)||!matches!(session.engine.as_str(),"codex"|"claude")||opening.binding_id.is_empty()||opening.revision<=0||opening.permission_epoch<0||opening.provider_revision<0||opening.provider_id.is_empty()||opening.model_id.is_empty()||opening.host_instance_id.is_empty()||opening.request_digest.len()>256||snapshot.binding_id!=opening.binding_id||snapshot.revision!=opening.revision||snapshot.permission_epoch!=opening.permission_epoch||snapshot.provider_id!=opening.provider_id||snapshot.model_id!=opening.model_id||snapshot.provider_revision!=opening.provider_revision||snapshot.engine!=session.engine{return Err("invalid_cli_binding".into())} Ok(())
}
fn encode_snapshot(snapshot:&CliProviderSnapshot)->Result<String,String>{let value=serde_json::to_string(snapshot).map_err(|_|"invalid_capabilities")?;if value.len()>MAX_SNAPSHOT{return Err("invalid_capabilities".into())}Ok(value)}
async fn load_binding<C:ConnectionTrait>(c:&C,owner:&str,session:&str)->Result<StoredCliBinding,String>{let row=fetch_one(c,"SELECT * FROM development_cli_session_bindings WHERE owner=? AND session_id=?",vec![v(owner),v(session)]).await?.ok_or("session_not_found")?;binding_from(&row)}
fn binding_from(r:&QueryResult)->Result<StoredCliBinding,String>{Ok(StoredCliBinding{owner:r.try_get("","owner").map_err(db)?,session_id:r.try_get("","session_id").map_err(db)?,binding_id:r.try_get("","binding_id").map_err(db)?,execution_policy:r.try_get("","execution_policy").map_err(db)?,project_id:r.try_get("","project_id").map_err(db)?,task_id:r.try_get("","task_id").map_err(db)?,revision:r.try_get("","revision").map_err(db)?,permission_epoch:r.try_get("","permission_epoch").map_err(db)?,provider_id:r.try_get("","provider_id").map_err(db)?,model_id:r.try_get("","model_id").map_err(db)?,provider_revision:r.try_get("","provider_revision").map_err(db)?,state:r.try_get("","state").map_err(db)?,cleanup_state:r.try_get("","cleanup_state").map_err(db)?,cleanup_reason:r.try_get("","cleanup_reason").map_err(db)?,host_instance_id:r.try_get("","host_instance_id").map_err(db)?,boot_identity:r.try_get("","boot_identity").map_err(db)?,cleanup_proof:r.try_get("","cleanup_proof").map_err(db)?,request_digest:r.try_get("","request_digest").map_err(db)?,updated_at:r.try_get("","updated_at").map_err(db)?})}
