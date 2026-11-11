//! Durable, session-owned projection of canonical Provider usage. Audit and fact settlement
//! share a transaction; this repository never infers usage from final messages or pagination.

use dgos_types::{DgosError, DgosResult, TokenUsage};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, QueryResult, Statement, TransactionTrait};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ChatStoreError;

/// IPC uses JavaScript numbers. Refuse an unrepresentable aggregate rather than silently
/// rounding a long-lived session's totals. SQLite itself sums signed 64-bit integers.
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatUsageCoverage {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    pub duration: i64,
    pub total: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatUsageLatest {
    pub model_id: String,
    pub total_tokens: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub duration_ms: Option<i64>,
    pub created_at: Option<i64>,
    pub partial: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ChatUsageSummary {
    pub total_tokens: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub duration_ms: Option<i64>,
    pub cache_hit_rate: Option<f64>,
    pub call_count: i64,
    pub reported_count: i64,
    pub pending_count: i64,
    pub partial: bool,
    pub legacy: bool,
    pub coverage: ChatUsageCoverage,
    pub latest: Option<ChatUsageLatest>,
}

#[derive(Clone)]
pub struct ChatUsageRepository {
    db: DatabaseConnection,
}

impl ChatUsageRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Durably admit one actual dispatch, immediately before network I/O. A replayed run
    /// does not reach this method; a genuinely new dispatch always gets a new attempt ID.
    /// Non-chat execution and another account's run never create a chat usage fact.
    pub async fn begin(
        &self,
        user_id: &str,
        provider_id: &str,
        model_id: &str,
        run_id: &str,
    ) -> DgosResult<Option<String>> {
        let id = Uuid::new_v4().to_string();
        let result = self.db.execute(statement(
            "INSERT INTO chat_usage_attempts(id,user_id,session_id,run_id,provider_id,model_id,source,status,created_at) \
             SELECT ?,r.user_id,r.session_id,r.task_id,?,?,'measured','pending',? \
             FROM chat_runs r JOIN chat_sessions s ON s.id=r.session_id AND s.user_id=r.user_id \
             WHERE r.user_id=? AND r.task_id=? AND r.status='running'",
            vec![id.clone().into(), provider_id.into(), model_id.into(), stamp().into(), user_id.into(), run_id.into()],
        )).await.map_err(database_error)?;
        Ok((result.rows_affected() == 1).then_some(id))
    }

    /// Settle once, using only canonical field-presence metadata. The Provider inserts its
    /// audit only when this returns true and commits both writes in the same transaction.
    /// A duplicate terminal receipt is a no-op, including attempts recovered as unknown.
    pub async fn settle_in_transaction<C: ConnectionTrait>(
        transaction: &C,
        attempt_id: &str,
        usage: Option<&TokenUsage>,
        duration_ms: Option<i64>,
        outcome: &str,
    ) -> DgosResult<bool> {
        if !matches!(outcome, "success" | "failure" | "interrupted" | "unknown") {
            return Err(DgosError::DatabaseError("invalid usage outcome".into()));
        }
        if duration_ms.is_some_and(|value| !(0..=MAX_SAFE_INTEGER).contains(&value)) {
            return Err(DgosError::DatabaseError("invalid usage duration".into()));
        }
        let input = usage.filter(|value| value.prompt_tokens_reported).map(|value| i64::from(value.prompt_tokens));
        let output = usage.filter(|value| value.completion_tokens_reported).map(|value| i64::from(value.completion_tokens));
        let total = usage.filter(|value| value.total_tokens_reported).map(|value| i64::from(value.total_tokens));
        let cache_read = usage.and_then(|value| value.cache_read_tokens).map(i64::from);
        let cache_write = usage.and_then(|value| value.cache_write_tokens).map(i64::from);
        let reasoning = usage.and_then(|value| value.reasoning_tokens).map(i64::from);
        let result = transaction.execute(statement(
            "UPDATE chat_usage_attempts SET status=?,input_tokens=?,output_tokens=?,total_tokens=?,\
             cache_read_tokens=?,cache_write_tokens=?,reasoning_tokens=?,duration_ms=?,settled_at=? \
             WHERE id=? AND source='measured' AND status='pending'",
            vec![outcome.into(), input.into(), output.into(), total.into(), cache_read.into(), cache_write.into(),
                 reasoning.into(), duration_ms.into(), stamp().into(), attempt_id.into()],
        )).await.map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }

    /// Remove only a privately owned admission that never reached network dispatch. The
    /// Provider serializes this with admission completion and settlement; UUIDs are not IPC.
    pub async fn abort_undispatched<C: ConnectionTrait>(transaction: &C, attempt_id: &str) -> DgosResult<bool> {
        let result = transaction.execute(statement(
            "DELETE FROM chat_usage_attempts WHERE id=? AND source='measured' AND status='pending'",
            vec![attempt_id.into()],
        )).await.map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }

    /// The caller holds the exclusive startup/account registry boundary. Live terminal
    /// writes deliberately do not use this: their pending receipts may still be settling.
    pub(crate) async fn recover_pending_in_transaction<C: ConnectionTrait>(transaction: &C, user_id: Option<&str>) -> Result<(), sea_orm::DbErr> {
        transaction.execute(statement(
            "UPDATE chat_usage_attempts SET status='unknown',settled_at=?1 \
             WHERE source='measured' AND status='pending' AND (?2 IS NULL OR user_id=?2)",
            vec![stamp().into(), user_id.map(str::to_owned).into()],
        )).await?;
        Ok(())
    }

    /// Aggregate all persisted calls, not the recent audit list or loaded messages. Owner,
    /// aggregate and latest settled fact are read from one consistent database snapshot.
    pub async fn summary(&self, user_id: &str, session_id: &str) -> Result<ChatUsageSummary, ChatStoreError> {
        let transaction = self.db.begin().await?;
        let row = transaction.query_one(statement(
            "SELECT COUNT(f.id) AS fact_count, \
             COALESCE(SUM(CASE WHEN f.source<>'legacy_total' THEN 1 ELSE 0 END),0) AS call_count, \
             COALESCE(SUM(CASE WHEN f.source<>'legacy_total' AND (f.total_tokens IS NOT NULL OR f.input_tokens IS NOT NULL OR f.output_tokens IS NOT NULL OR f.cache_read_tokens IS NOT NULL OR f.cache_write_tokens IS NOT NULL OR f.reasoning_tokens IS NOT NULL) THEN 1 ELSE 0 END),0) AS reported_count, \
             COALESCE(SUM(CASE WHEN f.status='pending' THEN 1 ELSE 0 END),0) AS pending_count, \
             COALESCE(MAX(CASE WHEN f.source<>'measured' THEN 1 ELSE 0 END),0) AS legacy, \
             COALESCE(MAX(CASE WHEN f.source='measured' AND f.status IN ('failure','interrupted','unknown') THEN 1 ELSE 0 END),0) AS terminal_incomplete, \
             SUM(f.total_tokens) AS total_tokens,SUM(f.input_tokens) AS input_tokens,SUM(f.output_tokens) AS output_tokens, \
             SUM(f.cache_read_tokens) AS cache_read_tokens,SUM(f.cache_write_tokens) AS cache_write_tokens, \
             SUM(f.reasoning_tokens) AS reasoning_tokens,SUM(f.duration_ms) AS duration_ms, \
             COALESCE(SUM(CASE WHEN f.source<>'legacy_total' AND f.input_tokens IS NOT NULL THEN 1 ELSE 0 END),0) AS input_coverage, \
             COALESCE(SUM(CASE WHEN f.source<>'legacy_total' AND f.output_tokens IS NOT NULL THEN 1 ELSE 0 END),0) AS output_coverage, \
             COALESCE(SUM(CASE WHEN f.source<>'legacy_total' AND f.cache_read_tokens IS NOT NULL THEN 1 ELSE 0 END),0) AS cache_read_coverage, \
             COALESCE(SUM(CASE WHEN f.source<>'legacy_total' AND f.cache_write_tokens IS NOT NULL THEN 1 ELSE 0 END),0) AS cache_write_coverage, \
             COALESCE(SUM(CASE WHEN f.source<>'legacy_total' AND f.reasoning_tokens IS NOT NULL THEN 1 ELSE 0 END),0) AS reasoning_coverage, \
             COALESCE(SUM(CASE WHEN f.source<>'legacy_total' AND f.duration_ms IS NOT NULL THEN 1 ELSE 0 END),0) AS duration_coverage, \
             COALESCE(SUM(CASE WHEN f.source<>'legacy_total' AND f.total_tokens IS NOT NULL THEN 1 ELSE 0 END),0) AS total_coverage \
             FROM chat_sessions s LEFT JOIN chat_usage_attempts f ON f.session_id=s.id AND f.user_id=s.user_id \
             WHERE s.id=? AND s.user_id=? GROUP BY s.id",
            vec![session_id.into(), user_id.into()],
        )).await?.ok_or(ChatStoreError::SessionNotFound)?;
        let latest = transaction.query_one(statement(
            "SELECT model_id,total_tokens,input_tokens,output_tokens,cache_read_tokens,duration_ms,created_at,source<>'measured' AS legacy, \
             source='measured' AND status IN ('failure','interrupted','unknown') AS terminal_incomplete \
             FROM chat_usage_attempts WHERE user_id=? AND session_id=? AND status<>'pending' \
             ORDER BY created_at DESC,id DESC LIMIT 1",
            vec![user_id.into(), session_id.into()],
        )).await?.as_ref().map(latest_from_row).transpose()?;
        transaction.commit().await?;
        let fact_count = integer(&row, "fact_count")?;
        let call_count = integer(&row, "call_count")?;
        let reported_count = integer(&row, "reported_count")?;
        let pending_count = integer(&row, "pending_count")?;
        let legacy = integer(&row, "legacy")? != 0;
        let terminal_incomplete = integer(&row, "terminal_incomplete")? != 0;
        let coverage = ChatUsageCoverage {
            input: integer(&row, "input_coverage")?,
            output: integer(&row, "output_coverage")?,
            cache_read: integer(&row, "cache_read_coverage")?,
            cache_write: integer(&row, "cache_write_coverage")?,
            reasoning: integer(&row, "reasoning_coverage")?,
            duration: integer(&row, "duration_coverage")?,
            total: integer(&row, "total_coverage")?,
        };
        let total_tokens = if fact_count == 0 { Some(0) } else { optional_integer(&row, "total_tokens")? };
        let input_tokens = optional_integer(&row, "input_tokens")?;
        let cache_read_tokens = optional_integer(&row, "cache_read_tokens")?;
        // Interim cumulative snapshots are known subtotals, not a complete cache denominator.
        // A subset cache numerator must never be divided by the whole-session input sum.
        let cache_hit_rate = if !legacy && !terminal_incomplete && call_count > 0 && coverage.input == call_count && coverage.cache_read == call_count {
            match (input_tokens, cache_read_tokens) {
                (Some(input), Some(cache)) if input > 0 && cache <= input => Some(cache as f64 / input as f64),
                _ => None,
            }
        } else { None };
        let partial = legacy || terminal_incomplete || pending_count > 0 || coverage.total < call_count
            || [coverage.input, coverage.output, coverage.cache_read, coverage.cache_write, coverage.reasoning, coverage.duration]
                .into_iter().any(|known| known > 0 && known < call_count);
        Ok(ChatUsageSummary {
            total_tokens,
            input_tokens,
            output_tokens: optional_integer(&row, "output_tokens")?,
            cache_read_tokens,
            cache_write_tokens: optional_integer(&row, "cache_write_tokens")?,
            reasoning_tokens: optional_integer(&row, "reasoning_tokens")?,
            duration_ms: optional_integer(&row, "duration_ms")?,
            cache_hit_rate,
            call_count,
            reported_count,
            pending_count,
            partial,
            legacy,
            coverage,
            latest,
        })
    }
}

fn statement(sql: &str, values: Vec<sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::Sqlite, sql, values)
}

fn stamp() -> i64 { chrono::Utc::now().timestamp_millis() }

fn database_error(_: sea_orm::DbErr) -> DgosError {
    DgosError::DatabaseError("chat usage persistence failed".into())
}

fn checked(value: i64) -> Result<i64, ChatStoreError> {
    if (0..=MAX_SAFE_INTEGER).contains(&value) { Ok(value) }
    else { Err(ChatStoreError::Db("chat usage exceeds transport integer range".into())) }
}

fn integer(row: &QueryResult, column: &str) -> Result<i64, ChatStoreError> {
    checked(row.try_get("", column)?)
}

fn optional_integer(row: &QueryResult, column: &str) -> Result<Option<i64>, ChatStoreError> {
    row.try_get::<Option<i64>>("", column)?.map(checked).transpose()
}

fn latest_from_row(row: &QueryResult) -> Result<ChatUsageLatest, ChatStoreError> {
    let total_tokens = optional_integer(row, "total_tokens")?;
    let input_tokens = optional_integer(row, "input_tokens")?;
    let output_tokens = optional_integer(row, "output_tokens")?;
    let duration_ms = optional_integer(row, "duration_ms")?;
    let legacy = integer(row, "legacy")? != 0;
    let terminal_incomplete = integer(row, "terminal_incomplete")? != 0;
    let timestamp = integer(row, "created_at")?;
    let created_at = (timestamp != 0).then_some(timestamp);
    Ok(ChatUsageLatest {
        model_id: row.try_get("", "model_id")?,
        total_tokens,
        input_tokens,
        output_tokens,
        cache_read_tokens: optional_integer(row, "cache_read_tokens")?,
        duration_ms,
        created_at,
        partial: legacy || terminal_incomplete || created_at.is_none() || total_tokens.is_none() || input_tokens.is_none()
            || output_tokens.is_none() || duration_ms.is_none(),
    })
}
