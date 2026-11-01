//! LAN audit log (V1-FR-010, D099 item 2, data model §4.8).
//!
//! The record type only admits what the table may hold: a fixed event, a
//! fixed `&'static str` reason, an IPv4 address, a session short id (eight
//! hex digits, anything else is dropped), the bound user and an error code.
//! There is no field for access codes, tokens, cookies, User-Agent, device
//! labels, paths, bodies or chat content, so callers cannot write them.
//!
//! Each append is one transaction: merge into a recent identical rejection,
//! or insert and trim the table to the newest 500 rows.

use dgos_types::{DgosError, DgosResult};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement, TransactionTrait, Value,
};
use serde::Serialize;
use std::net::Ipv4Addr;

/// Whole-table retention (D099).
pub const MAX_LAN_AUDIT_ROWS: i64 = 500;
/// Identical rejections inside this window update one row (D099).
pub const LAN_AUDIT_MERGE_SECS: i64 = 60;
const DEFAULT_PAGE: u32 = 50;
const MAX_PAGE: u32 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LanAuditKind {
    PairSucceeded,
    PairRejected,
    PairRateLimited,
    CodeVoided,
    SessionRevoked,
    RequestRejected,
}

impl LanAuditKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PairSucceeded => "pair_succeeded",
            Self::PairRejected => "pair_rejected",
            Self::PairRateLimited => "pair_rate_limited",
            Self::CodeVoided => "code_voided",
            Self::SessionRevoked => "session_revoked",
            Self::RequestRejected => "request_rejected",
        }
    }

    /// Rejections are merged; successes, voids and revocations always get a row.
    fn merges(self) -> bool {
        matches!(
            self,
            Self::PairRejected | Self::PairRateLimited | Self::RequestRejected
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanAuditRecord {
    pub at: i64,
    pub event: LanAuditKind,
    pub reason: Option<&'static str>,
    pub peer_ip: Option<Ipv4Addr>,
    pub session_short_id: Option<String>,
    pub user_id: Option<String>,
    pub error_code: Option<u32>,
}

/// Interface contract §14.8 `LanAuditEvent`. No user id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LanAuditEvent {
    pub id: String,
    pub first_at: i64,
    pub last_at: i64,
    pub event: String,
    pub reason: Option<String>,
    pub peer_ip: Option<String>,
    pub session_short_id: Option<String>,
    pub error_code: Option<i64>,
    pub count: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LanAuditPage {
    pub items: Vec<LanAuditEvent>,
    pub next_cursor: Option<String>,
}

#[derive(Clone)]
pub struct LanAuditRepository {
    db: DatabaseConnection,
}

fn short_id(value: Option<String>) -> Option<String> {
    value.filter(|id| id.len() == 8 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

fn stmt<const N: usize>(sql: &str, values: [Value; N]) -> Statement {
    Statement::from_sql_and_values(DbBackend::Sqlite, sql, values)
}

fn db_error(error: DbErr) -> DgosError {
    DgosError::DatabaseError(error.to_string())
}

impl LanAuditRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Insert or merge one event and trim to `MAX_LAN_AUDIT_ROWS`, in one transaction.
    pub async fn append(&self, record: LanAuditRecord) -> DgosResult<()> {
        let peer: Option<String> = record.peer_ip.map(|ip| ip.to_string());
        let reason: Option<String> = record.reason.map(str::to_owned);
        let tx = self.db.begin().await.map_err(db_error)?;
        if record.event.merges() {
            let row = tx
                .query_one(stmt(
                    "SELECT id FROM lan_audit_events WHERE event = ? AND reason IS ? AND peer_ip IS ? \
                     AND last_at > ? ORDER BY rowid DESC LIMIT 1",
                    [
                        record.event.as_str().into(),
                        reason.clone().into(),
                        peer.clone().into(),
                        (record.at - LAN_AUDIT_MERGE_SECS).into(),
                    ],
                ))
                .await
                .map_err(db_error)?;
            if let Some(row) = row {
                let id: String = row.try_get("", "id").map_err(db_error)?;
                tx.execute(stmt(
                    "UPDATE lan_audit_events SET last_at = ?, count = count + 1 WHERE id = ?",
                    [record.at.into(), id.into()],
                ))
                .await
                .map_err(db_error)?;
                return tx.commit().await.map_err(db_error);
            }
        }
        tx.execute(stmt(
            "INSERT INTO lan_audit_events (id, first_at, last_at, event, reason, peer_ip, \
             session_short_id, user_id, error_code, count) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1)",
            [
                uuid::Uuid::new_v4().to_string().into(),
                record.at.into(),
                record.at.into(),
                record.event.as_str().into(),
                reason.into(),
                peer.into(),
                short_id(record.session_short_id).into(),
                record.user_id.into(),
                record.error_code.map(i64::from).into(),
            ],
        ))
        .await
        .map_err(db_error)?;
        tx.execute(stmt(
            "DELETE FROM lan_audit_events WHERE rowid NOT IN \
             (SELECT rowid FROM lan_audit_events ORDER BY rowid DESC LIMIT ?)",
            [MAX_LAN_AUDIT_ROWS.into()],
        ))
        .await
        .map_err(db_error)?;
        tx.commit().await.map_err(db_error)
    }

    /// Rows of `user` and of unauthenticated sources (`user_id` NULL), newest
    /// first. `cursor` is the opaque value of the previous page's `next_cursor`.
    pub async fn list(
        &self,
        user: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> DgosResult<LanAuditPage> {
        let limit = limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE) as i64;
        let before = match cursor {
            None => i64::MAX,
            Some(raw) => match raw.parse::<i64>() {
                Ok(value) if value > 0 => value,
                _ => {
                    return Ok(LanAuditPage {
                        items: Vec::new(),
                        next_cursor: None,
                    })
                }
            },
        };
        let rows = self
            .db
            .query_all(stmt(
                "SELECT rowid AS rid, id, first_at, last_at, event, reason, peer_ip, \
                 session_short_id, error_code, count FROM lan_audit_events \
                 WHERE (user_id = ? OR user_id IS NULL) AND rowid < ? ORDER BY rowid DESC LIMIT ?",
                [user.into(), before.into(), (limit + 1).into()],
            ))
            .await
            .map_err(db_error)?;
        let mut items = Vec::with_capacity(rows.len().min(limit as usize));
        let mut last_rowid = None;
        for row in rows.iter().take(limit as usize) {
            last_rowid = Some(row.try_get::<i64>("", "rid").map_err(db_error)?);
            items.push(LanAuditEvent {
                id: row.try_get("", "id").map_err(db_error)?,
                first_at: row.try_get("", "first_at").map_err(db_error)?,
                last_at: row.try_get("", "last_at").map_err(db_error)?,
                event: row.try_get("", "event").map_err(db_error)?,
                reason: row.try_get("", "reason").map_err(db_error)?,
                peer_ip: row.try_get("", "peer_ip").map_err(db_error)?,
                session_short_id: row.try_get("", "session_short_id").map_err(db_error)?,
                error_code: row.try_get("", "error_code").map_err(db_error)?,
                count: row.try_get("", "count").map_err(db_error)?,
            });
        }
        let next_cursor = if rows.len() as i64 > limit {
            last_rowid.map(|rowid| rowid.to_string())
        } else {
            None
        };
        Ok(LanAuditPage { items, next_cursor })
    }
}
