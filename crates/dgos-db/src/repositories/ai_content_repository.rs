//! Temporary, host-owner/scope-bound content, separate from content-free run audit.
//! No network work or raw tool parameters are recorded here. Leases never renew.
use dgos_types::{AiContextReceipt, AiResource, AiRunContent, DgosError, DgosResult};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, QueryResult, Statement,
    TransactionTrait, Value};
use serde_json::{json, Value as JsonValue};
use sha2::{Digest, Sha256};

const LEASE_SECONDS: i64 = 24 * 60 * 60;
const QUOTA_BYTES: usize = 24 * 1024 * 1024;
const MAX_BODY: usize = 64 * 1024;
const MAX_RECEIPTS: usize = 128;
const MAX_SNAPSHOT: usize = 16 * 1024;

#[derive(Clone)]
pub struct AiContentRepository { db: DatabaseConnection }

impl AiContentRepository {
    pub fn new(db: DatabaseConnection) -> Self { Self { db } }

    pub async fn import_resource(&self, user: &str, scope: &str, name: &str, mime: &str, bytes: &[u8]) -> DgosResult<AiResource> {
        validate_resource(name, mime, bytes)?;
        let tx = self.db.begin().await.map_err(db_error)?;
        authorize(&tx, user, scope, true).await?;
        expire(&tx, now()).await?;
        let resource = AiResource { id: uuid::Uuid::new_v4().to_string(), name: name.into(),
            mime: mime.into(), bytes_len: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)), expires_at: now() + LEASE_SECONDS };
        let changed = tx.execute(stmt(
            "INSERT INTO ai_content_resources(id,user_id,scope,name,mime,bytes,sha256,created_at,expires_at) \
             SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9 WHERE \
             (SELECT COALESCE(SUM(length(bytes)),0) FROM ai_content_resources WHERE user_id=?2 AND scope=?3)+?10<=?11",
            vec![resource.id.clone().into(),user.into(),scope.into(),name.into(),mime.into(),bytes.to_vec().into(),
                resource.sha256.clone().into(),now().into(),resource.expires_at.into(),(bytes.len() as i64).into(),(QUOTA_BYTES as i64).into()],
        )).await.map_err(db_error)?.rows_affected();
        if changed != 1 { return Err(invalid("AI resource scope quota exceeded")); }
        tx.commit().await.map_err(db_error)?;
        Ok(resource)
    }

    pub async fn resource(&self, user: &str, scope: &str, id: &str) -> DgosResult<Option<(AiResource, Vec<u8>)>> {
        let tx = self.db.begin().await.map_err(db_error)?;
        authorize(&tx, user, scope, false).await?;
        let row = tx.query_one(stmt(
            "SELECT id,name,mime,bytes,sha256,expires_at,length(bytes) AS bytes_len FROM ai_content_resources \
             WHERE user_id=?1 AND scope=?2 AND id=?3 AND expires_at>?4 AND length(bytes)<=8388608",
            vec![user.into(),scope.into(),id.into(),now().into()],
        )).await.map_err(db_error)?;
        let result = row.map(|r| -> DgosResult<_> { Ok((AiResource {
            id: get(&r,"id")?, name: get(&r,"name")?, mime: get(&r,"mime")?,
            bytes_len: get::<i64>(&r,"bytes_len")? as u64, sha256: get(&r,"sha256")?, expires_at: get(&r,"expires_at")?,
        }, get(&r,"bytes")?)) }).transpose()?;
        tx.commit().await.map_err(db_error)?;
        Ok(result)
    }

    /// Revalidate a loaded resource without copying its potentially large body.
    pub async fn resource_metadata(&self, user: &str, scope: &str, id: &str) -> DgosResult<Option<AiResource>> {
        let tx = self.db.begin().await.map_err(db_error)?;
        authorize(&tx,user,scope,false).await?;
        let row=tx.query_one(stmt(
            "SELECT id,name,mime,sha256,expires_at,length(bytes) AS bytes_len FROM ai_content_resources \
             WHERE user_id=?1 AND scope=?2 AND id=?3 AND expires_at>?4 AND length(bytes)<=8388608",
            vec![user.into(),scope.into(),id.into(),now().into()],
        )).await.map_err(db_error)?;
        let result=row.map(|r| -> DgosResult<_> { Ok(AiResource {
            id:get(&r,"id")?,name:get(&r,"name")?,mime:get(&r,"mime")?,sha256:get(&r,"sha256")?,
            bytes_len:get::<i64>(&r,"bytes_len")? as u64,expires_at:get(&r,"expires_at")?,
        }) }).transpose()?;
        tx.commit().await.map_err(db_error)?;
        Ok(result)
    }

    pub async fn delete_resource(&self, user: &str, scope: &str, id: &str) -> DgosResult<bool> {
        let tx = self.db.begin().await.map_err(db_error)?;
        authorize(&tx, user, scope, true).await?;
        let changed = tx.execute(stmt("DELETE FROM ai_content_resources WHERE user_id=?1 AND scope=?2 AND id=?3",
            vec![user.into(),scope.into(),id.into()])).await.map_err(db_error)?.rows_affected();
        tx.commit().await.map_err(db_error)?;
        Ok(changed == 1)
    }

    /// A run identity can never be restarted, even after its body expires.
    /// Continuation validation and inherited fingerprints commit with the new identity.
    pub async fn start_run(&self, user: &str, scope: &str, run_id: &str, retain: bool, source_run_id: Option<&str>) -> DgosResult<()> {
        identifier(run_id)?;
        let tx = self.db.begin().await.map_err(db_error)?;
        authorize(&tx, user, scope, true).await?;
        expire(&tx, now()).await?;
        if let Some(source) = source_run_id {
            if source == run_id { return Err(invalid("A run cannot continue itself")); }
            let source = load_run(&tx,user,scope,source).await?.ok_or(DgosError::PermissionDenied)?;
            ensure_continuation(&source)?;
        }
        tx.execute(stmt(
            "INSERT INTO ai_content_runs(run_id,user_id,scope,source_run_id,retain,created_at,updated_at,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?6,?7)",
            vec![run_id.into(),user.into(),scope.into(),source_run_id.into(),retain.into(),now().into(),(now()+LEASE_SECONDS).into()],
        )).await.map_err(db_error)?;
        if let Some(source) = source_run_id {
            tx.execute(stmt(
                "INSERT INTO ai_content_tool_receipts(run_id,call_id,fingerprint,outcome,unknown,inherited) \
                 SELECT ?1,call_id,fingerprint,outcome,unknown,1 FROM ai_content_tool_receipts WHERE run_id=?2",
                vec![run_id.into(),source.into()],
            )).await.map_err(db_error)?;
        }
        tx.commit().await.map_err(db_error)
    }

    /// Opt-out never persists text, including partial text. Metadata snapshots are bounded.
    pub async fn checkpoint(&self, user: &str, scope: &str, run_id: &str, content: &str,
        context_receipts: &[AiContextReceipt], skill_receipts: &JsonValue) -> DgosResult<()> {
        body(content)?;
        if context_receipts.len()>MAX_RECEIPTS { return Err(invalid("Too many context receipts")); }
        for r in context_receipts {
            identifier(&r.resource_id)?;
            if !hash(&r.sha256) || u64::from(r.start)+u64::from(r.bytes)>r.total_bytes || r.total_bytes>8*1024*1024 {
                return Err(invalid("Invalid context receipt"));
            }
        }
        let contexts = serde_json::to_string(context_receipts).map_err(|_| invalid("Invalid context receipts"))?;
        let skills = skill_receipts.to_string();
        if contexts.len()>MAX_SNAPSHOT || skills.len()>MAX_SNAPSHOT { return Err(invalid("Receipt snapshot too large")); }
        let tx = self.db.begin().await.map_err(db_error)?;
        authorize(&tx,user,scope,true).await?;
        expire(&tx,now()).await?;
        let changed = tx.execute(stmt(
            "UPDATE ai_content_runs SET content=CASE WHEN retain=1 AND content_expired=0 AND expires_at>?7 THEN ?4 ELSE NULL END, \
             context_receipts=?5,skill_receipts=?6,updated_at=?7 WHERE user_id=?1 AND scope=?2 AND run_id=?3 AND status='running'",
            vec![user.into(),scope.into(),run_id.into(),content.into(),contexts.into(),skills.into(),now().into()],
        )).await.map_err(db_error)?.rows_affected();
        changed_one(changed)?;
        trim_bodies(&tx,user,scope).await?;
        tx.commit().await.map_err(db_error)
    }

    /// Persisted before external dispatch. Fingerprints, not provider call IDs, prevent replay.
    pub async fn tool_dispatched(&self, user: &str, scope: &str, run_id: &str, call_id: &str, fingerprint: &str) -> DgosResult<()> {
        identifier(call_id)?;
        if !hash(fingerprint) { return Err(invalid("Invalid tool fingerprint")); }
        let tx = self.db.begin().await.map_err(db_error)?;
        authorize(&tx,user,scope,true).await?;
        let changed = tx.execute(stmt(
            "INSERT INTO ai_content_tool_receipts(run_id,call_id,fingerprint) SELECT ?3,?4,?5 \
             WHERE EXISTS(SELECT 1 FROM ai_content_runs WHERE user_id=?1 AND scope=?2 AND run_id=?3 AND status='running' AND expires_at>?6) \
             AND (SELECT COUNT(*) FROM ai_content_tool_receipts WHERE run_id=?3)<?7",
            vec![user.into(),scope.into(),run_id.into(),call_id.into(),fingerprint.to_ascii_lowercase().into(),now().into(),(MAX_RECEIPTS as i64).into()],
        )).await.map_err(db_error)?.rows_affected();
        changed_one(changed)?;
        tx.commit().await.map_err(db_error)
    }

    /// The first observed outcome is immutable; it cannot turn unknown into success later.
    pub async fn tool_settled(&self, user: &str, scope: &str, run_id: &str, call_id: &str, outcome: &str, unknown: bool) -> DgosResult<()> {
        if !matches!(outcome,"succeeded"|"failed"|"cancelled"|"unknown") || (outcome=="unknown" && !unknown) {
            return Err(invalid("Invalid content-free tool outcome"));
        }
        let tx = self.db.begin().await.map_err(db_error)?;
        authorize(&tx,user,scope,true).await?;
        let changed = tx.execute(stmt(
            "UPDATE ai_content_tool_receipts SET outcome=?5,unknown=?6 WHERE run_id=?3 AND call_id=?4 AND outcome IS NULL AND inherited=0 \
             AND EXISTS(SELECT 1 FROM ai_content_runs WHERE user_id=?1 AND scope=?2 AND run_id=?3 AND status='running')",
            vec![user.into(),scope.into(),run_id.into(),call_id.into(),outcome.into(),unknown.into()],
        )).await.map_err(db_error)?.rows_affected();
        changed_one(changed)?;
        tx.commit().await.map_err(db_error)
    }

    pub async fn finish_run(&self, user: &str, scope: &str, run_id: &str, content: Option<&str>, status: &str,
        output_valid: bool, audit_warning: bool) -> DgosResult<()> {
        let tx = self.db.begin().await.map_err(db_error)?;
        finish_in(&tx,user,scope,run_id,content,status,output_valid,audit_warning).await?;
        tx.commit().await.map_err(db_error)
    }

    pub async fn get_run(&self, user: &str, scope: &str, run_id: &str) -> DgosResult<Option<AiRunContent>> {
        let tx=self.db.begin().await.map_err(db_error)?;
        authorize(&tx,user,scope,false).await?;
        let result=load_run(&tx,user,scope,run_id).await?.map(|r| r.public);
        tx.commit().await.map_err(db_error)?;
        Ok(result)
    }

    /// True means this identity exists and is acknowledged, including repeated acknowledgements.
    pub async fn acknowledge(&self, user: &str, scope: &str, run_id: &str) -> DgosResult<bool> {
        let tx=self.db.begin().await.map_err(db_error)?;
        authorize(&tx,user,scope,true).await?;
        let changed=tx.execute(stmt(
            "UPDATE ai_content_runs SET acknowledged_at=COALESCE(acknowledged_at,?4) WHERE user_id=?1 AND scope=?2 AND run_id=?3",
            vec![user.into(),scope.into(),run_id.into(),now().into()],
        )).await.map_err(db_error)?.rows_affected();
        tx.commit().await.map_err(db_error)?;
        Ok(changed==1)
    }

    pub async fn continuation(&self, user: &str, scope: &str, run_id: &str) -> DgosResult<AiRunContent> {
        let tx=self.db.begin().await.map_err(db_error)?;
        authorize(&tx,user,scope,false).await?;
        let result=load_run(&tx,user,scope,run_id).await?.ok_or(DgosError::PermissionDenied)?;
        ensure_continuation(&result)?;
        tx.commit().await.map_err(db_error)?;
        Ok(result.public)
    }

    /// Historical source metadata is distinct from this run's actual deliveries.
    /// Expired ancestor bodies remain valid provenance; the caller revalidates
    /// every returned resource/hash against its current lease before dispatch.
    pub async fn continuation_context_receipts(&self, user: &str, scope: &str, run_id: &str) -> DgosResult<Vec<AiContextReceipt>> {
        let tx=self.db.begin().await.map_err(db_error)?;
        authorize(&tx,user,scope,false).await?;
        let mut next=Some(run_id.to_owned());
        let mut seen=std::collections::HashSet::with_capacity(32);
        let mut receipts=Vec::new();
        while let Some(id)=next {
            if seen.len()>=32 || seen.contains(&id) {
                return Err(invalid("Continuation source chain exceeds its bound or contains a cycle"));
            }
            let row=tx.query_one(stmt(
                "SELECT source_run_id,CASE WHEN length(CAST(context_receipts AS BLOB))<=16384 THEN context_receipts ELSE NULL END AS context_receipts \
                 FROM ai_content_runs WHERE user_id=?1 AND scope=?2 AND run_id=?3",
                vec![user.into(),scope.into(),id.as_str().into()],
            )).await.map_err(db_error)?.ok_or(DgosError::PermissionDenied)?;
            let snapshot:Option<String>=get(&row,"context_receipts")?;
            let snapshot=snapshot.ok_or_else(|| invalid("Receipt snapshot too large"))?;
            let current:Vec<AiContextReceipt>=serde_json::from_str(&snapshot).map_err(|_| invalid("Invalid context receipts"))?;
            if current.len()>MAX_RECEIPTS-receipts.len() {
                return Err(invalid("Continuation context receipts exceed their bound"));
            }
            receipts.extend(current);
            next=get(&row,"source_run_id")?;
            seen.insert(id);
        }
        tx.commit().await.map_err(db_error)?;
        Ok(receipts)
    }

    /// Startup-only crash convergence. No tool is replayed or inferred successful.
    pub async fn recover_interrupted(&self) -> DgosResult<u64> {
        let tx=self.db.begin().await.map_err(db_error)?;
        tx.execute(stmt(
            "UPDATE ai_content_tool_receipts SET outcome='unknown',unknown=1 WHERE outcome IS NULL \
             AND run_id IN (SELECT run_id FROM ai_content_runs WHERE status='running')",vec![],
        )).await.map_err(db_error)?;
        let changed=tx.execute(stmt("UPDATE ai_content_runs SET status='interrupted',updated_at=?1 WHERE status='running'",vec![now().into()]))
            .await.map_err(db_error)?.rows_affected();
        expire(&tx,now()).await?;
        tx.commit().await.map_err(db_error)?;
        Ok(changed)
    }
}

/// Shared transaction boundary for the module audit + content terminal commit.
pub(crate) async fn finish_in(db: &impl ConnectionTrait, user: &str, scope: &str, run_id: &str,
    content: Option<&str>, status: &str, output_valid: bool, audit_warning: bool) -> DgosResult<()> {
    if !matches!(status,"succeeded"|"failed"|"cancelled"|"interrupted") { return Err(invalid("Invalid terminal status")); }
    if let Some(text)=content { body(text)?; }
    authorize(db,user,scope,true).await?;
    expire(db,now()).await?;
    let changed = db.execute(stmt(
        "UPDATE ai_content_runs SET status=?4,output_valid=?5,audit_warning=?6,updated_at=?7, \
         content=CASE WHEN retain=1 AND content_expired=0 AND expires_at>?7 THEN COALESCE(?8,content) ELSE NULL END \
         WHERE user_id=?1 AND scope=?2 AND run_id=?3 AND status='running' \
         AND (?4<>'succeeded' OR (?6=0 AND NOT EXISTS(SELECT 1 FROM ai_content_tool_receipts WHERE run_id=?3 AND (outcome IS NULL OR unknown=1))))",
        vec![user.into(),scope.into(),run_id.into(),status.into(),output_valid.into(),audit_warning.into(),now().into(),content.into()],
    )).await.map_err(db_error)?.rows_affected();
    changed_one(changed)?;
    trim_bodies(db,user,scope).await
}

struct StoredRun { public: AiRunContent, audit_warning: bool }

async fn load_run(db: &impl ConnectionTrait,user: &str,scope: &str,id: &str) -> DgosResult<Option<StoredRun>> {
    // Never fetch expired or oversized bodies into memory, even before cleanup has run.
    let row=db.query_one(stmt(
        "SELECT run_id,status,output_valid,expires_at,acknowledged_at,source_run_id,context_receipts,skill_receipts,audit_warning, \
         (expires_at<=?4 OR content_expired=1) AS expired, \
         CASE WHEN retain=1 AND expires_at>?4 AND content_expired=0 AND length(CAST(content AS BLOB))<=65536 THEN content ELSE NULL END AS content \
         FROM ai_content_runs WHERE user_id=?1 AND scope=?2 AND run_id=?3",
        vec![user.into(),scope.into(),id.into(),now().into()],
    )).await.map_err(db_error)?;
    let Some(row)=row else { return Ok(None) };
    let receipts=db.query_all(stmt(
        "SELECT call_id,fingerprint,outcome,unknown,inherited FROM ai_content_tool_receipts WHERE run_id=?1 ORDER BY call_id LIMIT 129",vec![id.into()],
    )).await.map_err(db_error)?;
    if receipts.len()>MAX_RECEIPTS { return Err(invalid("Too many tool receipts")); }
    let tools=receipts.into_iter().map(|r| -> DgosResult<_> { Ok(json!({
        "call_id":get::<String>(&r,"call_id")?,"fingerprint":get::<String>(&r,"fingerprint")?,
        "outcome":get::<Option<String>>(&r,"outcome")?,"unknown":get::<bool>(&r,"unknown")?,"inherited":get::<bool>(&r,"inherited")?,
    })) }).collect::<DgosResult<Vec<_>>>()?;
    let contexts:String=get(&row,"context_receipts")?;
    let skills:String=get(&row,"skill_receipts")?;
    if contexts.len()>MAX_SNAPSHOT || skills.len()>MAX_SNAPSHOT { return Err(invalid("Receipt snapshot too large")); }
    Ok(Some(StoredRun { audit_warning:get(&row,"audit_warning")?, public:AiRunContent {
        run_id:get(&row,"run_id")?,content:get(&row,"content")?,
        status:if get::<bool>(&row,"expired")? { "expired".into() } else { get(&row,"status")? },
        output_valid:get(&row,"output_valid")?,expires_at:get(&row,"expires_at")?,
        acknowledged_at:get(&row,"acknowledged_at")?,source_run_id:get(&row,"source_run_id")?,
        context_receipts:serde_json::from_str(&contexts).map_err(|_| invalid("Invalid context receipts"))?,
        skill_receipts:serde_json::from_str(&skills).map_err(|_| invalid("Invalid skill receipts"))?,tool_receipts:JsonValue::Array(tools),
    }}))
}

fn ensure_continuation(run: &StoredRun) -> DgosResult<()> {
    if !matches!(run.public.status.as_str(),"succeeded"|"failed"|"cancelled"|"interrupted") || run.public.content.is_none() || run.audit_warning {
        return Err(invalid("Run has no safely retained terminal progress"));
    }
    if run.public.tool_receipts.as_array().is_none_or(|receipts| receipts.iter().any(|r| r["outcome"].is_null() || r["unknown"]==true)) {
        return Err(invalid("Unresolved tool effects prevent continuation"));
    }
    Ok(())
}

async fn authorize(db: &impl ConnectionTrait,user: &str,scope: &str,write: bool) -> DgosResult<()> {
    let suffix=scope.strip_prefix("module:").or_else(|| scope.strip_prefix("chat:"));
    if scope.len()>256 || suffix.is_none_or(str::is_empty) { return Err(invalid("Invalid host content scope")); }
    // A conditional write serializes quota, account deletion and run CAS operations.
    if write {
        changed_one(db.execute(stmt("UPDATE users SET updated_at=updated_at WHERE id=?1 AND deleting_since IS NULL",vec![user.into()]))
            .await.map_err(db_error)?.rows_affected())?;
    } else if db.query_one(stmt("SELECT id FROM users WHERE id=?1 AND deleting_since IS NULL",vec![user.into()]))
        .await.map_err(db_error)?.is_none() { return Err(DgosError::PermissionDenied); }
    if let Some(session)=scope.strip_prefix("chat:") {
        if db.query_one(stmt("SELECT id FROM chat_sessions WHERE id=?1 AND user_id=?2",vec![session.into(),user.into()]))
            .await.map_err(db_error)?.is_none() { return Err(DgosError::PermissionDenied); }
    }
    Ok(())
}

async fn expire(db: &impl ConnectionTrait,time: i64) -> DgosResult<()> {
    db.execute(stmt("DELETE FROM ai_content_resources WHERE expires_at<=?1",vec![time.into()])).await.map_err(db_error)?;
    db.execute(stmt("UPDATE ai_content_runs SET content=NULL,content_expired=1 WHERE expires_at<=?1 AND content_expired=0",vec![time.into()]))
        .await.map_err(db_error)?;
    Ok(())
}

async fn trim_bodies(db: &impl ConnectionTrait,user: &str,scope: &str) -> DgosResult<()> {
    db.execute(stmt(
        "UPDATE ai_content_runs SET content=NULL,content_expired=1 WHERE run_id IN \
         (SELECT run_id FROM ai_content_runs WHERE user_id=?1 AND scope=?2 AND content IS NOT NULL ORDER BY created_at DESC,rowid DESC LIMIT -1 OFFSET 32)",
        vec![user.into(),scope.into()],
    )).await.map_err(db_error)?;
    Ok(())
}

fn validate_resource(name: &str,mime: &str,bytes: &[u8]) -> DgosResult<()> {
    if name.trim().is_empty() || name.chars().count()>255 || name.chars().any(char::is_control) {
        return Err(invalid("Invalid resource name"));
    }
    let valid=match mime {
        "text/plain"|"text/markdown" => bytes.len()<=1024*1024 && std::str::from_utf8(bytes).is_ok(),
        "image/png" => bytes.len()<=8*1024*1024 && bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.len()<=8*1024*1024 && bytes.starts_with(&[0xff,0xd8,0xff]),
        "image/webp" => bytes.len()<=8*1024*1024 && bytes.len()>=12 && &bytes[..4]==b"RIFF" && &bytes[8..12]==b"WEBP",
        _ => false,
    };
    if !valid { return Err(invalid("Unsupported or invalid AI resource")); }
    Ok(())
}
fn identifier(id: &str) -> DgosResult<()> {
    if id.is_empty() || id.len()>128 || id.chars().any(char::is_control) { return Err(invalid("Invalid content identity")); }
    Ok(())
}
fn hash(value: &str) -> bool { value.len()==64 && value.bytes().all(|b| b.is_ascii_hexdigit()) }
fn body(text: &str) -> DgosResult<()> { if text.len()>MAX_BODY { Err(invalid("AI content exceeds 64KiB")) } else { Ok(()) } }
fn changed_one(changed: u64) -> DgosResult<()> { if changed==1 { Ok(()) } else { Err(DgosError::PermissionDenied) } }
fn now() -> i64 { chrono::Utc::now().timestamp() }
fn stmt(sql: &str,values: Vec<Value>) -> Statement { Statement::from_sql_and_values(DbBackend::Sqlite,sql,values) }
fn get<T: sea_orm::TryGetable>(row: &QueryResult,column: &str) -> DgosResult<T> { row.try_get("",column).map_err(db_error) }
fn db_error(_: DbErr) -> DgosError { DgosError::DatabaseError("AI content storage failed".into()) }
fn invalid(_: &str) -> DgosError { DgosError::ModuleCapabilityInputInvalid }
