//! Content-free module AI audit. SQLite triggers in migration 31 make every
//! permission/lifecycle mutation share the dispatch transaction's write order.
use super::ModulePermission;
use dgos_types::{
    AiFailure, AiRunListInput, AiRunPage, AiRunStatus, AiRunSummary, AiSkillRef,
    AiTerminal, AiToolSelection, DgosError, DgosResult, GenerationParameters,
};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbBackend, DbErr, QueryResult, Statement,
    TransactionTrait, Value,
};

pub struct NewAiRun {
    pub run_id: String,
    pub user_id: String,
    pub module_id: String,
    pub module_version: String,
    pub module_integrity: String,
    pub runtime_id: String,
    pub request_id: String,
    pub authorization_epoch: i64,
    pub provider_id: String,
    pub model_id: String,
    pub provider_revision: i64,
    pub skill_refs: Vec<AiSkillRef>,
    pub tools: Vec<AiToolSelection>,
    pub generation: GenerationParameters,
}

#[derive(Clone)]
pub struct AiRunRepository {
    db: DatabaseConnection,
}

// A missing account never falls back to the pre-account default-user mode.
const AUTHORIZED: &str = "SELECT e.epoch FROM module_authorization_epochs e \
    JOIN modules m ON m.id=e.module_id AND m.is_enabled=1 \
    JOIN users u ON u.id=e.user_id AND u.deleting_since IS NULL \
    WHERE e.user_id=?1 AND e.module_id=?2 AND e.epoch=?3 AND e.epoch<9223372036854775807";
const SUMMARY: &str = "SELECT t.id,t.provider_id,t.model_id,t.created_at,t.finished_at,t.status, \
    b.request_id,b.terminal_status,b.error_domain,b.error_code,b.terminal_reason,b.result_unknown \
    FROM ai_run_bindings b JOIN workbench_tasks t ON t.id=b.task_id AND t.user_id=b.user_id \
    WHERE b.user_id=?1 AND b.module_id=?2";

impl AiRunRepository {
    pub fn new(db: DatabaseConnection) -> Self { Self { db } }

    /// Establish the first epoch only for a live account and enabled module.
    /// Existing tombstones are returned unchanged, including after uninstall.
    pub async fn authorization_epoch(&self, user: &str, module: &str) -> DgosResult<i64> {
        let tx = self.db.begin().await.map_err(db_error)?;
        tx.execute(stmt(
            "INSERT INTO module_authorization_epochs(user_id,module_id,epoch) \
             SELECT ?1,?2,0 WHERE EXISTS(SELECT 1 FROM users WHERE id=?1 AND deleting_since IS NULL) \
             AND EXISTS(SELECT 1 FROM modules WHERE id=?2 AND is_enabled=1) \
             ON CONFLICT(user_id,module_id) DO NOTHING",
            vec![user.into(), module.into()],
        )).await.map_err(db_error)?;
        let row = tx.query_one(stmt(
            "SELECT e.epoch FROM module_authorization_epochs e JOIN users u ON u.id=e.user_id \
             AND u.deleting_since IS NULL WHERE e.user_id=?1 AND e.module_id=?2",
            vec![user.into(), module.into()],
        )).await.map_err(db_error)?.ok_or(DgosError::PermissionDenied)?;
        let epoch: i64 = row.try_get("", "epoch").map_err(db_error)?;
        if epoch == i64::MAX { return Err(DgosError::PermissionDenied); }
        tx.commit().await.map_err(db_error)?;
        Ok(epoch)
    }

    /// Read-only and fail closed. Never recreates a purged epoch/account.
    pub async fn ensure_authorization_epoch(&self, user: &str, module: &str, expected: i64) -> DgosResult<()> {
        if self.db.query_one(stmt(AUTHORIZED, vec![user.into(), module.into(), expected.into()]))
            .await.map_err(db_error)?.is_none() {
            return Err(DgosError::PermissionDenied);
        }
        Ok(())
    }

    /// Direct capability dispatch uses the same SQLite write serialization,
    /// without manufacturing an AI task or changing the epoch. The caller
    /// validates the bound package identity and exact resource before admission.
    pub async fn admit_capability_step(&self, user: &str, module: &str, expected_epoch: i64, permission: ModulePermission) -> DgosResult<()> {
        let tx = self.db.begin().await.map_err(db_error)?;
        let changed = tx.execute(stmt(
            "UPDATE module_authorization_epochs SET epoch=epoch \
             WHERE user_id=?1 AND module_id=?2 AND epoch=?3 AND epoch<9223372036854775807 \
             AND EXISTS(SELECT 1 FROM users WHERE id=?1 AND deleting_since IS NULL) \
             AND EXISTS(SELECT 1 FROM modules m WHERE m.id=?2 AND m.is_enabled=1 \
                 AND EXISTS(SELECT 1 FROM json_each(m.manifest,'$.permissions') WHERE value=?4)) \
             AND EXISTS(SELECT 1 FROM module_permissions WHERE user_id=?1 AND module_id=?2 AND permission=?4 AND granted=1)",
            vec![user.into(),module.into(),expected_epoch.into(),permission.as_str().into()],
        )).await.map_err(db_error)?.rows_affected();
        if changed != 1 { return Err(DgosError::PermissionDenied); }
        tx.commit().await.map_err(db_error)
    }

    /// Task, immutable binding and ordered selections commit together. No prompt,
    /// guidance, reasoning or generated output enters these audit tables.
    pub async fn begin(&self, input: &NewAiRun) -> DgosResult<()> {
        let tx = self.db.begin().await.map_err(db_error)?;
        // This conditional write is also the authorization linearization point.
        let inserted = tx.execute(stmt(
            "INSERT INTO workbench_tasks(id,user_id,app_id,provider_id,model_id,status,created_at,kind,provider_config_revision,generation_parameters) \
             SELECT ?1,?2,?3,?4,?5,'running',?6,'single',?7,?8 \
             WHERE EXISTS(SELECT 1 FROM module_authorization_epochs e \
                 JOIN users u ON u.id=e.user_id AND u.deleting_since IS NULL \
                 JOIN modules m ON m.id=e.module_id AND m.is_enabled=1 \
                 WHERE e.user_id=?2 AND e.module_id=?3 AND e.epoch=?9 AND e.epoch<9223372036854775807 \
                 AND m.current_version=?10 AND m.integrity=?11)",
            vec![input.run_id.clone().into(), input.user_id.clone().into(), input.module_id.clone().into(),
                input.provider_id.clone().into(), input.model_id.clone().into(), now().into(),
                input.provider_revision.into(), input.generation.audit_json().to_string().into(),
                input.authorization_epoch.into(), input.module_version.clone().into(), input.module_integrity.clone().into()],
        )).await.map_err(db_error)?.rows_affected();
        if inserted != 1 { return Err(DgosError::PermissionDenied); }
        tx.execute(stmt(
            "INSERT INTO ai_run_bindings(task_id,user_id,module_id,module_version,module_integrity,runtime_id,request_id,authorization_epoch,skill_refs,tool_refs) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            vec![input.run_id.clone().into(), input.user_id.clone().into(), input.module_id.clone().into(),
                input.module_version.clone().into(), input.module_integrity.clone().into(), input.runtime_id.clone().into(),
                input.request_id.clone().into(), input.authorization_epoch.into(),
                serde_json::to_string(&input.skill_refs).map_err(|_| invalid())?.into(),
                serde_json::to_string(&input.tools).map_err(|_| invalid())?.into()],
        )).await.map_err(db_error)?;
        for (position, skill) in input.skill_refs.iter().enumerate() {
            let inserted = tx.execute(stmt(
                "INSERT INTO workbench_task_skills(task_id,version_id,skill_id,content_hash,position) \
                 SELECT ?1,v.id,v.skill_id,v.content_hash,?2 FROM skill_versions v \
                 JOIN skill_user_states s ON s.version_id=v.id AND s.skill_id=v.skill_id \
                 WHERE s.user_id=?3 AND s.enabled=1 AND v.skill_id=?4 AND v.version=?5 AND v.content_hash=?6",
                vec![input.run_id.clone().into(), (position as i64).into(), input.user_id.clone().into(),
                    skill.skill_id.clone().into(), skill.version.clone().into(), skill.content_hash.clone().into()],
            )).await.map_err(db_error)?.rows_affected();
            if inserted != 1 { return Err(DgosError::PermissionDenied); }
        }
        for tool in &input.tools {
            tx.execute(stmt(
                "INSERT INTO workbench_task_tools(task_id,server_id,tool_name,schema_hash) VALUES(?1,?2,?3,?4)",
                vec![input.run_id.clone().into(),tool.server_id.clone().into(),tool.tool_name.clone().into(),tool.schema_hash.clone().into()],
            )).await.map_err(db_error)?;
        }
        tx.commit().await.map_err(db_error)
    }

    /// Successful commit is a durable in-flight admission. Revocation committed
    /// first rejects this write; revocation committed afterward cancels local
    /// waiting and prevents the next admission, but cannot undo external I/O.
    /// Identity/grants/pins and lease are additionally checked by the caller.
    pub async fn admit_step(&self, user: &str, module: &str, expected_epoch: i64, run_id: &str) -> DgosResult<()> {
        let tx = self.db.begin().await.map_err(db_error)?;
        let changed = tx.execute(stmt(
            "UPDATE ai_run_bindings SET dispatch_count=dispatch_count+1,last_dispatch_at=?5 \
             WHERE task_id=?4 AND user_id=?1 AND module_id=?2 AND authorization_epoch=?3 AND terminal_status IS NULL \
             AND EXISTS(SELECT 1 FROM module_authorization_epochs e \
                 JOIN users u ON u.id=e.user_id AND u.deleting_since IS NULL \
                 JOIN modules m ON m.id=e.module_id AND m.is_enabled=1 \
                 WHERE e.user_id=?1 AND e.module_id=?2 AND e.epoch=?3 AND e.epoch<9223372036854775807 \
                 AND m.current_version=ai_run_bindings.module_version AND m.integrity=ai_run_bindings.module_integrity) \
             AND EXISTS(SELECT 1 FROM workbench_tasks t WHERE t.id=?4 AND t.user_id=?1 AND t.status='running')",
            vec![user.into(),module.into(),expected_epoch.into(),run_id.into(),now().into()],
        )).await.map_err(db_error)?.rows_affected();
        if changed != 1 { return Err(DgosError::PermissionDenied); }
        tx.commit().await.map_err(db_error)
    }

    /// Exactly one running -> terminal CAS. A deleted task/binding stays deleted.
    pub async fn finish(&self, user: &str, module: &str, run_id: &str, terminal: &AiTerminal) -> DgosResult<bool> {
        if terminal.status == AiRunStatus::Running { return Err(invalid()); }
        let tx = self.db.begin().await.map_err(db_error)?;
        let error = terminal.error.as_ref();
        let changed = tx.execute(stmt(
            "UPDATE ai_run_bindings SET terminal_status=?4,error_domain=?5,error_code=?6,terminal_reason=?7,result_unknown=?8,audit_warning=?9 \
             WHERE task_id=?3 AND user_id=?1 AND module_id=?2 AND terminal_status IS NULL \
             AND EXISTS(SELECT 1 FROM workbench_tasks t WHERE t.id=?3 AND t.user_id=?1 AND t.status='running')",
            vec![user.into(),module.into(),run_id.into(),terminal.status.as_str().into(),
                error.map(|e| e.domain.clone()).into(),error.map(|e| i64::from(e.code)).into(),
                error.map(|e| e.reason.clone()).into(),(terminal.result_unknown || error.is_some_and(|e| e.result_unknown)).into(),
                terminal.audit_warning.into()],
        )).await.map_err(db_error)?.rows_affected();
        if changed == 0 { tx.commit().await.map_err(db_error)?; return Ok(false); }
        let task_status = if terminal.status == AiRunStatus::Cancelled { "interrupted" } else { terminal.status.as_str() };
        let task_changed = tx.execute(stmt(
            "UPDATE workbench_tasks SET status=?4,finished_at=?5,error_code=?6,total_tokens=?7 \
             WHERE id=?3 AND user_id=?1 AND app_id=?2 AND kind='single' AND status='running'",
            vec![user.into(),module.into(),run_id.into(),task_status.into(),now().into(),
                error.map(|e| e.code.to_string()).into(),terminal.total_tokens.into()],
        )).await.map_err(db_error)?.rows_affected();
        if task_changed != 1 { return Err(DgosError::DatabaseError("AI task binding mismatch".into())); }
        tx.commit().await.map_err(db_error)?;
        Ok(true)
    }

    pub async fn get(&self, user: &str, module: &str, run_id: &str) -> DgosResult<Option<AiRunSummary>> {
        self.db.query_one(stmt(&format!("{SUMMARY} AND b.task_id=?3"),vec![user.into(),module.into(),run_id.into()]))
            .await.map_err(db_error)?.map(summary).transpose()
    }

    /// Owner/module-scoped keyset pagination. A foreign or deleted cursor is an
    /// invalid selector, never a global lookup or a silently restarted page.
    pub async fn list(&self, user: &str, module: &str, input: AiRunListInput) -> DgosResult<AiRunPage> {
        let limit = input.limit.unwrap_or(50).clamp(1, 100) as usize;
        let tx = self.db.begin().await.map_err(db_error)?;
        let mut values = vec![user.into(),module.into()];
        let mut sql = SUMMARY.to_owned();
        if let Some(before) = input.before {
            let cursor = tx.query_one(stmt(
                "SELECT t.created_at FROM ai_run_bindings b JOIN workbench_tasks t ON t.id=b.task_id AND t.user_id=b.user_id \
                 WHERE b.user_id=?1 AND b.module_id=?2 AND b.task_id=?3",
                vec![user.into(),module.into(),before.clone().into()],
            )).await.map_err(db_error)?.ok_or_else(invalid)?;
            let created: i64 = cursor.try_get("", "created_at").map_err(db_error)?;
            sql.push_str(" AND (t.created_at < ?3 OR (t.created_at = ?3 AND t.id < ?4))");
            values.push(created.into()); values.push(before.into());
        }
        sql.push_str(&format!(" ORDER BY t.created_at DESC,t.id DESC LIMIT {}",limit+1));
        let mut items: Vec<_> = tx.query_all(stmt(&sql, values)).await.map_err(db_error)?
            .into_iter().map(summary).collect::<DgosResult<_>>()?;
        let next_before = if items.len() > limit {
            items.truncate(limit); items.last().map(|item| item.run_id.clone())
        } else { None };
        tx.commit().await.map_err(db_error)?;
        Ok(AiRunPage { items, next_before })
    }

    /// Crash convergence only: no prompt, approvals or external effects replay.
    /// The same trigger also covers legacy Workbench recovery's direct updates.
    pub async fn recover_interrupted(&self) -> DgosResult<u64> {
        let tx = self.db.begin().await.map_err(db_error)?;
        let changed = tx.execute(stmt(
            "UPDATE workbench_tasks SET status='interrupted',finished_at=?1,error_code='process_interrupted' \
             WHERE status='running' AND EXISTS(SELECT 1 FROM ai_run_bindings b WHERE b.task_id=workbench_tasks.id AND b.user_id=workbench_tasks.user_id)",
            vec![now().into()],
        )).await.map_err(db_error)?.rows_affected();
        tx.commit().await.map_err(db_error)?;
        Ok(changed)
    }
}

fn summary(row: QueryResult) -> DgosResult<AiRunSummary> {
    let terminal: Option<String> = row.try_get("", "terminal_status").map_err(db_error)?;
    let task_status: String = row.try_get("", "status").map_err(db_error)?;
    let status = match terminal.as_deref().unwrap_or(&task_status) {
        "running" => AiRunStatus::Running, "succeeded" => AiRunStatus::Succeeded,
        "failed" => AiRunStatus::Failed, "cancelled" => AiRunStatus::Cancelled,
        "interrupted" => AiRunStatus::Interrupted,
        _ => return Err(DgosError::DatabaseError("invalid AI task status".into())),
    };
    let result_unknown = row.try_get("", "result_unknown").map_err(db_error)?;
    let domain: Option<String> = row.try_get("", "error_domain").map_err(db_error)?;
    let code: Option<i64> = row.try_get("", "error_code").map_err(db_error)?;
    let reason: Option<String> = row.try_get("", "terminal_reason").map_err(db_error)?;
    let error = match (domain,code,reason) {
        (Some(domain),Some(code),Some(reason)) => Some(AiFailure {
            domain,code:u32::try_from(code).map_err(|_| DgosError::DatabaseError("invalid AI error code".into()))?,
            reason,result_unknown,
        }),
        _ => None,
    };
    Ok(AiRunSummary {
        run_id:row.try_get("", "id").map_err(db_error)?, request_id:row.try_get("", "request_id").map_err(db_error)?,
        provider_id:row.try_get("", "provider_id").map_err(db_error)?, model_id:row.try_get("", "model_id").map_err(db_error)?,
        created_at:row.try_get("", "created_at").map_err(db_error)?, finished_at:row.try_get("", "finished_at").map_err(db_error)?,
        status,error,result_unknown,
    })
}
fn stmt(sql: &str, values: Vec<Value>) -> Statement { Statement::from_sql_and_values(DbBackend::Sqlite,sql,values) }
fn now() -> i64 { chrono::Utc::now().timestamp() }
fn invalid() -> DgosError { DgosError::ModuleCapabilityInputInvalid }
fn db_error(_: DbErr) -> DgosError { DgosError::DatabaseError("AI audit operation failed".into()) }
