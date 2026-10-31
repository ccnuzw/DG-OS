//! Persistence for chat sessions and messages (V1-FR-006, D072-D077).
//!
//! Every statement filters by the host-derived `user_id` AND the resource id; another user's
//! resource and a missing one are the same `NotFound`. DTOs never carry `user_id`. Chat text lives
//! only in `chat_*`; `workbench_tasks` stays the content-free audit.
//!
//! The connection pool is a single connection (D077): inside a transaction every statement MUST go
//! through the transaction handle, never `self.db`, or the call deadlocks.

use sea_orm::*;
use serde::Serialize;
use uuid::Uuid;

pub const DEFAULT_TITLE: &str = "新会话";
const TITLE_MAX_CHARS: usize = 80;
const AUTO_TITLE_CHARS: usize = 30;

/// Task `error_code` categories after which the upstream may already have billed the call.
/// Everything else (pre-flight rejections, auth failures, quota) is a known outcome.
const UNKNOWN_RESULT_CODES: &[&str] = &[
    "task_cancelled",
    "process_interrupted",
    "generation_limit",
    "provider_timeout",
    "provider_1001006",
    "provider_1001007",
    "provider_1001008",
    "provider_1001014",
    "provider_1001025",
];

/// D074: an interrupted or failed generation without reported usage may still have been billed.
pub fn result_is_unknown(
    status: &str,
    total_tokens: Option<i64>,
    error_code: Option<&str>,
) -> bool {
    matches!(status, "interrupted" | "failed")
        && total_tokens.is_none()
        && error_code.is_some_and(|code| UNKNOWN_RESULT_CODES.contains(&code))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatStoreError {
    SessionNotFound,
    MessageNotFound,
    GenerationInProgress,
    RequestConflict,
    NotRegenerable,
    InvalidCursor,
    InvalidTitle,
    Db(String),
}

impl From<DbErr> for ChatStoreError {
    fn from(error: DbErr) -> Self {
        let text = error.to_string();
        if text.contains("idx_chat_messages_one_generating")
            || text.contains("chat_messages.user_id")
        {
            ChatStoreError::GenerationInProgress
        } else if text.contains("workbench_tasks.id") {
            ChatStoreError::RequestConflict
        } else {
            // Fixed text: database errors can echo bound values, which may be chat text.
            ChatStoreError::Db("database error".into())
        }
    }
}

type Result<T, E = ChatStoreError> = std::result::Result<T, E>;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub title_source: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub skill_id: Option<String>,
    pub skill_version_id: Option<String>,
    pub no_skill_confirmed: bool,
    pub created_at: i64,
    pub updated_at: i64,
    /// Used to revalidate the pinned skill on every send; never sent to the renderer.
    #[serde(skip)]
    pub skill_content_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: String,
    pub session_id: String,
    pub seq: i64,
    pub role: String,
    pub content: String,
    pub status: String,
    pub task_id: Option<String>,
    pub finish_reason: Option<String>,
    pub error_code: Option<String>,
    /// `None` is "usage unknown"; it is never rendered as 0.
    pub total_tokens: Option<i64>,
    pub result_unknown: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct PinnedSkill {
    pub skill_id: String,
    pub version_id: String,
    pub content_hash: String,
}

/// Session target. `skill` and `no_skill` are mutually exclusive; neither clears the pin and the
/// "no skill" confirmation, so the next send is blocked until the user chooses.
#[derive(Debug, Clone, Default)]
pub struct ChatTarget {
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub skill: Option<PinnedSkill>,
    pub no_skill: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub session: ChatSession,
    pub snippet: String,
    pub match_count: i64,
}

pub struct BeginGeneration<'a> {
    pub user: &'a str,
    pub session_id: &'a str,
    pub task_id: &'a str,
    pub user_message_id: &'a str,
    pub assistant_message_id: &'a str,
    /// `Some` for a send; `None` for a regeneration.
    pub user_content: Option<&'a str>,
    /// The assistant message being regenerated.
    pub regenerate_of: Option<&'a str>,
    pub provider_id: &'a str,
    pub model_id: &'a str,
    pub provider_config_revision: Option<i64>,
    pub endpoint_fingerprint: &'a str,
    pub skill: Option<&'a PinnedSkill>,
    /// (server_id, tool_name, schema_hash)
    pub tools: &'a [(String, String, String)],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Begun {
    pub user_message_id: String,
    pub assistant_message_id: String,
    pub replayed: bool,
}

pub struct FinalizeGeneration<'a> {
    pub user: &'a str,
    pub assistant_message_id: &'a str,
    pub task_id: &'a str,
    pub content: &'a str,
    /// `completed`, `interrupted` or `failed`.
    pub message_status: &'a str,
    pub finish_reason: Option<&'a str>,
    pub message_error: Option<&'a str>,
    /// `succeeded`, `failed` or `interrupted`.
    pub task_status: &'a str,
    pub task_error: Option<&'a str>,
    pub total_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finalized {
    /// `message` is the persisted assistant message; after a discarded regeneration it is the
    /// kept old reply and `discarded` is true.
    Done {
        message: Box<ChatMessage>,
        discarded: bool,
    },
    AlreadyFinalized,
}

#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub seq: i64,
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct History {
    /// Completed messages, oldest first, newest-first budget-limited by the SQL window.
    pub rows: Vec<HistoryRow>,
    /// Completed assistant replies before the bound; each is one completed turn.
    pub total_completed_replies: i64,
}

#[derive(Clone)]
pub struct ChatRepository {
    db: DatabaseConnection,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn stmt(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::Sqlite, sql, values)
}

/// `LIKE` pattern for a literal substring: `%`, `_` and `\` match themselves.
pub fn like_pattern(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for c in query.chars() {
        if matches!(c, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

/// D076: the first 30 characters of the first user message with line breaks removed.
pub fn auto_title(content: &str) -> String {
    let flat: String = content
        .chars()
        .filter(|c| *c != '\n' && *c != '\r')
        .take(AUTO_TITLE_CHARS)
        .collect();
    let flat = flat.trim();
    if flat.is_empty() {
        DEFAULT_TITLE.to_owned()
    } else {
        flat.to_owned()
    }
}

/// A short single-line excerpt around the first case-insensitive (ASCII) match.
pub fn snippet(content: &str, query: &str) -> String {
    let lower = content.to_ascii_lowercase();
    let needle = query.to_ascii_lowercase();
    let position = lower.find(&needle).unwrap_or(0);
    let mut start = position;
    for _ in 0..40 {
        if start == 0 {
            break;
        }
        start -= 1;
        while !content.is_char_boundary(start) {
            start -= 1;
        }
    }
    let mut end = (position + needle.len()).min(content.len());
    for _ in 0..80 {
        if end >= content.len() {
            break;
        }
        end += 1;
        while !content.is_char_boundary(end) {
            end += 1;
        }
    }
    let flat: String = content[start..end]
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        flat.trim(),
        if end < content.len() { "…" } else { "" }
    )
}

fn session_from_row(row: &QueryResult) -> Result<ChatSession> {
    let get = |name: &str| row.try_get::<Option<String>>("", name);
    Ok(ChatSession {
        id: row.try_get("", "id")?,
        title: row.try_get("", "title")?,
        title_source: row.try_get("", "title_source")?,
        provider_id: get("provider_id")?,
        model_id: get("model_id")?,
        skill_id: get("skill_id")?,
        skill_version_id: get("skill_version_id")?,
        skill_content_hash: get("skill_content_hash")?,
        no_skill_confirmed: row.try_get::<i64>("", "no_skill_confirmed")? != 0,
        created_at: row.try_get("", "created_at")?,
        updated_at: row.try_get("", "updated_at")?,
    })
}

const SESSION_COLUMNS: &str = "id, title, title_source, provider_id, model_id, skill_id, \
    skill_version_id, skill_content_hash, no_skill_confirmed, created_at, updated_at";

const MESSAGE_SELECT: &str = "SELECT m.id, m.session_id, m.seq, m.role, m.content, m.status, \
    m.task_id, m.finish_reason, m.error_code, m.created_at, m.updated_at, \
    t.total_tokens AS task_tokens \
    FROM chat_messages m LEFT JOIN workbench_tasks t ON t.id = m.task_id AND t.user_id = m.user_id";

fn message_from_row(row: &QueryResult) -> Result<ChatMessage> {
    let status: String = row.try_get("", "status")?;
    let error_code: Option<String> = row.try_get("", "error_code")?;
    let total_tokens: Option<i64> = row.try_get("", "task_tokens")?;
    let role: String = row.try_get("", "role")?;
    let task_id: Option<String> = row.try_get("", "task_id")?;
    let result_unknown = role == "assistant"
        && task_id.is_some()
        && result_is_unknown(&status, total_tokens, error_code.as_deref());
    Ok(ChatMessage {
        id: row.try_get("", "id")?,
        session_id: row.try_get("", "session_id")?,
        seq: row.try_get("", "seq")?,
        role,
        content: row.try_get("", "content")?,
        status,
        task_id,
        finish_reason: row.try_get("", "finish_reason")?,
        error_code,
        total_tokens,
        result_unknown,
        created_at: row.try_get("", "created_at")?,
        updated_at: row.try_get("", "updated_at")?,
    })
}

async fn session_in(db: &impl ConnectionTrait, user: &str, id: &str) -> Result<ChatSession> {
    let row = db
        .query_one(stmt(
            &format!("SELECT {SESSION_COLUMNS} FROM chat_sessions WHERE id = ? AND user_id = ?"),
            vec![id.into(), user.into()],
        ))
        .await?
        .ok_or(ChatStoreError::SessionNotFound)?;
    session_from_row(&row)
}

async fn has_generating(db: &impl ConnectionTrait, user: &str, session_id: &str) -> Result<bool> {
    Ok(db
        .query_one(stmt(
            "SELECT 1 AS x FROM chat_messages WHERE session_id = ? AND user_id = ? \
             AND role = 'assistant' AND status = 'generating' LIMIT 1",
            vec![session_id.into(), user.into()],
        ))
        .await?
        .is_some())
}

async fn message_in(db: &impl ConnectionTrait, user: &str, id: &str) -> Result<ChatMessage> {
    let row = db
        .query_one(stmt(
            &format!("{MESSAGE_SELECT} WHERE m.id = ? AND m.user_id = ?"),
            vec![id.into(), user.into()],
        ))
        .await?
        .ok_or(ChatStoreError::MessageNotFound)?;
    message_from_row(&row)
}

fn parse_cursor(cursor: &str) -> Result<(i64, String)> {
    let (updated, id) = cursor
        .split_once(':')
        .ok_or(ChatStoreError::InvalidCursor)?;
    let updated = updated
        .parse::<i64>()
        .map_err(|_| ChatStoreError::InvalidCursor)?;
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(ChatStoreError::InvalidCursor);
    }
    Ok((updated, id.to_owned()))
}

fn cursor_for(session: &ChatSession) -> String {
    format!("{}:{}", session.updated_at, session.id)
}

impl ChatRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn create_session(&self, user: &str, target: &ChatTarget) -> Result<ChatSession> {
        let id = Uuid::new_v4().to_string();
        let created = now();
        self.db
            .execute(stmt(
                "INSERT INTO chat_sessions (id, user_id, title, title_source, provider_id, model_id, \
                 skill_id, skill_version_id, skill_content_hash, no_skill_confirmed, created_at, updated_at) \
                 VALUES (?, ?, ?, 'auto', ?, ?, ?, ?, ?, ?, ?, ?)",
                vec![
                    id.clone().into(),
                    user.into(),
                    DEFAULT_TITLE.into(),
                    target.provider_id.clone().into(),
                    target.model_id.clone().into(),
                    target.skill.as_ref().map(|s| s.skill_id.clone()).into(),
                    target.skill.as_ref().map(|s| s.version_id.clone()).into(),
                    target.skill.as_ref().map(|s| s.content_hash.clone()).into(),
                    i64::from(target.no_skill && target.skill.is_none()).into(),
                    created.into(),
                    created.into(),
                ],
            ))
            .await?;
        session_in(&self.db, user, &id).await
    }

    pub async fn get_session(&self, user: &str, id: &str) -> Result<ChatSession> {
        session_in(&self.db, user, id).await
    }

    /// Newest first by `(updated_at, id)`; `limit + 1` rows tell whether another page exists.
    pub async fn list_sessions(
        &self,
        user: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<ChatSession>, Option<String>)> {
        let (updated, id) = match cursor {
            Some(cursor) => parse_cursor(cursor)?,
            None => (i64::MAX, "g".to_owned()),
        };
        let rows = self
            .db
            .query_all(stmt(
                &format!(
                    "SELECT {SESSION_COLUMNS} FROM chat_sessions WHERE user_id = ? \
                     AND (updated_at < ? OR (updated_at = ? AND id < ?)) \
                     ORDER BY updated_at DESC, id DESC LIMIT ?"
                ),
                vec![
                    user.into(),
                    updated.into(),
                    updated.into(),
                    id.into(),
                    (limit as i64 + 1).into(),
                ],
            ))
            .await?;
        let mut sessions = rows
            .iter()
            .map(session_from_row)
            .collect::<Result<Vec<_>>>()?;
        let next = if sessions.len() > limit {
            sessions.truncate(limit);
            sessions.last().map(cursor_for)
        } else {
            None
        };
        Ok((sessions, next))
    }

    pub async fn rename_session(&self, user: &str, id: &str, title: &str) -> Result<ChatSession> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > TITLE_MAX_CHARS {
            return Err(ChatStoreError::InvalidTitle);
        }
        let result = self
            .db
            .execute(stmt(
                "UPDATE chat_sessions SET title = ?, title_source = 'user', updated_at = ? \
                 WHERE id = ? AND user_id = ?",
                vec![title.into(), now().into(), id.into(), user.into()],
            ))
            .await?;
        if result.rows_affected() != 1 {
            return Err(ChatStoreError::SessionNotFound);
        }
        session_in(&self.db, user, id).await
    }

    /// Only later sends are affected; refused while the session is generating.
    pub async fn set_target(
        &self,
        user: &str,
        id: &str,
        target: &ChatTarget,
    ) -> Result<ChatSession> {
        let tx = self.db.begin().await?;
        session_in(&tx, user, id).await?;
        if has_generating(&tx, user, id).await? {
            return Err(ChatStoreError::GenerationInProgress);
        }
        tx.execute(stmt(
            "UPDATE chat_sessions SET provider_id = ?, model_id = ?, skill_id = ?, \
             skill_version_id = ?, skill_content_hash = ?, no_skill_confirmed = ?, updated_at = ? \
             WHERE id = ? AND user_id = ?",
            vec![
                target.provider_id.clone().into(),
                target.model_id.clone().into(),
                target.skill.as_ref().map(|s| s.skill_id.clone()).into(),
                target.skill.as_ref().map(|s| s.version_id.clone()).into(),
                target.skill.as_ref().map(|s| s.content_hash.clone()).into(),
                i64::from(target.no_skill && target.skill.is_none()).into(),
                now().into(),
                id.into(),
                user.into(),
            ],
        ))
        .await?;
        let session = session_in(&tx, user, id).await?;
        tx.commit().await?;
        Ok(session)
    }

    /// One transaction: clear `workbench_tasks.session_id`, delete messages and the session.
    /// Task audit rows and their skill/tool references stay (D072).
    pub async fn delete_session(&self, user: &str, id: &str) -> Result<()> {
        let tx = self.db.begin().await?;
        session_in(&tx, user, id).await?;
        if has_generating(&tx, user, id).await? {
            return Err(ChatStoreError::GenerationInProgress);
        }
        tx.execute(stmt(
            "UPDATE workbench_tasks SET session_id = NULL WHERE session_id = ? AND user_id = ?",
            vec![id.into(), user.into()],
        ))
        .await?;
        tx.execute(stmt(
            "DELETE FROM chat_messages WHERE session_id = ? AND user_id = ?",
            vec![id.into(), user.into()],
        ))
        .await?;
        let deleted = tx
            .execute(stmt(
                "DELETE FROM chat_sessions WHERE id = ? AND user_id = ?",
                vec![id.into(), user.into()],
            ))
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(ChatStoreError::SessionNotFound);
        }
        tx.commit().await?;
        Ok(())
    }

    /// FR-008 dependency (D072): the caller cancels the user's generation first.
    pub async fn purge_user(&self, user: &str) -> Result<u64> {
        let tx = self.db.begin().await?;
        let active = tx
            .query_one(stmt(
                "SELECT 1 AS x FROM chat_messages WHERE user_id = ? AND status = 'generating' LIMIT 1",
                vec![user.into()],
            ))
            .await?;
        if active.is_some() {
            return Err(ChatStoreError::GenerationInProgress);
        }
        tx.execute(stmt(
            "UPDATE workbench_tasks SET session_id = NULL WHERE user_id = ? AND session_id IS NOT NULL",
            vec![user.into()],
        ))
        .await?;
        tx.execute(stmt(
            "DELETE FROM chat_messages WHERE user_id = ?",
            vec![user.into()],
        ))
        .await?;
        let deleted = tx
            .execute(stmt(
                "DELETE FROM chat_sessions WHERE user_id = ?",
                vec![user.into()],
            ))
            .await?;
        tx.commit().await?;
        Ok(deleted.rows_affected())
    }

    /// A page of messages ending before `before_seq`, oldest first. Regeneration drafts are hidden.
    pub async fn list_messages(
        &self,
        user: &str,
        session_id: &str,
        before_seq: Option<i64>,
        limit: usize,
    ) -> Result<(Vec<ChatMessage>, Option<i64>)> {
        session_in(&self.db, user, session_id).await?;
        let rows = self
            .db
            .query_all(stmt(
                &format!(
                    "{MESSAGE_SELECT} WHERE m.session_id = ? AND m.user_id = ? \
                     AND m.replaces_id IS NULL AND m.seq < ? ORDER BY m.seq DESC LIMIT ?"
                ),
                vec![
                    session_id.into(),
                    user.into(),
                    before_seq.unwrap_or(i64::MAX).into(),
                    (limit as i64 + 1).into(),
                ],
            ))
            .await?;
        let mut messages = rows
            .iter()
            .map(message_from_row)
            .collect::<Result<Vec<_>>>()?;
        let next = if messages.len() > limit {
            messages.truncate(limit);
            messages.last().map(|m| m.seq)
        } else {
            None
        };
        messages.reverse();
        Ok((messages, next))
    }

    pub async fn get_message(&self, user: &str, id: &str) -> Result<ChatMessage> {
        message_in(&self.db, user, id).await
    }

    /// Titles and completed/interrupted message text containing `query` as a literal substring.
    /// ASCII case-insensitive only (SQLite `LIKE`); callers validate the query length.
    pub async fn search_sessions(
        &self,
        user: &str,
        query: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<SearchHit>, Option<String>)> {
        let (updated, id) = match cursor {
            Some(cursor) => parse_cursor(cursor)?,
            None => (i64::MAX, "g".to_owned()),
        };
        let pattern = like_pattern(query);
        let rows = self
            .db
            .query_all(stmt(
                &format!(
                    "SELECT {SESSION_COLUMNS} FROM chat_sessions s WHERE s.user_id = ? \
                     AND (s.title LIKE ? ESCAPE '\\' OR EXISTS (SELECT 1 FROM chat_messages m \
                       WHERE m.session_id = s.id AND m.user_id = s.user_id \
                       AND m.status IN ('completed','interrupted') AND m.replaces_id IS NULL \
                       AND m.content LIKE ? ESCAPE '\\')) \
                     AND (s.updated_at < ? OR (s.updated_at = ? AND s.id < ?)) \
                     ORDER BY s.updated_at DESC, s.id DESC LIMIT ?"
                ),
                vec![
                    user.into(),
                    pattern.clone().into(),
                    pattern.clone().into(),
                    updated.into(),
                    updated.into(),
                    id.into(),
                    (limit as i64 + 1).into(),
                ],
            ))
            .await?;
        let mut sessions = rows
            .iter()
            .map(session_from_row)
            .collect::<Result<Vec<_>>>()?;
        let next = if sessions.len() > limit {
            sessions.truncate(limit);
            sessions.last().map(cursor_for)
        } else {
            None
        };
        let mut hits = Vec::with_capacity(sessions.len());
        for session in sessions {
            let first = self
                .db
                .query_one(stmt(
                    "SELECT content FROM chat_messages WHERE session_id = ? AND user_id = ? \
                     AND status IN ('completed','interrupted') AND replaces_id IS NULL \
                     AND content LIKE ? ESCAPE '\\' ORDER BY seq ASC LIMIT 1",
                    vec![
                        session.id.clone().into(),
                        user.into(),
                        pattern.clone().into(),
                    ],
                ))
                .await?;
            let counted = self
                .db
                .query_one(stmt(
                    "SELECT COUNT(*) AS n FROM chat_messages WHERE session_id = ? AND user_id = ? \
                     AND status IN ('completed','interrupted') AND replaces_id IS NULL \
                     AND content LIKE ? ESCAPE '\\'",
                    vec![
                        session.id.clone().into(),
                        user.into(),
                        pattern.clone().into(),
                    ],
                ))
                .await?;
            let messages: i64 = match counted {
                Some(row) => row.try_get("", "n")?,
                None => 0,
            };
            let title_hit = session
                .title
                .to_ascii_lowercase()
                .contains(&query.to_ascii_lowercase());
            let text: String = match first {
                Some(row) => row.try_get("", "content")?,
                None => session.title.clone(),
            };
            hits.push(SearchHit {
                snippet: snippet(&text, query),
                match_count: messages + i64::from(title_hit),
                session,
            });
        }
        Ok((hits, next))
    }

    /// The target fingerprint of the last completed or interrupted reply (D075).
    pub async fn last_endpoint_fingerprint(
        &self,
        user: &str,
        session_id: &str,
    ) -> Result<Option<String>> {
        let row = self
            .db
            .query_one(stmt(
                "SELECT t.endpoint_fingerprint AS fp FROM chat_messages m \
                 JOIN workbench_tasks t ON t.id = m.task_id AND t.user_id = m.user_id \
                 WHERE m.session_id = ? AND m.user_id = ? AND m.role = 'assistant' \
                 AND m.status IN ('completed','interrupted') AND t.endpoint_fingerprint IS NOT NULL \
                 ORDER BY m.seq DESC LIMIT 1",
                vec![session_id.into(), user.into()],
            ))
            .await?;
        Ok(match row {
            Some(row) => row.try_get("", "fp")?,
            None => None,
        })
    }

    /// Completed history before `before_seq` (all of it when `None`), newest-first limited by a
    /// running byte total so an enormous session never loads whole. The crossing row is included
    /// so the caller can tell the budget was exceeded.
    pub async fn history(
        &self,
        user: &str,
        session_id: &str,
        before_seq: Option<i64>,
        max_bytes: usize,
    ) -> Result<History> {
        let bound = before_seq.unwrap_or(i64::MAX);
        let rows = self
            .db
            .query_all(stmt(
                "SELECT seq, role, content FROM ( \
                   SELECT seq, role, content, \
                     SUM(LENGTH(CAST(content AS BLOB))) OVER (ORDER BY seq DESC) AS running, \
                     LENGTH(CAST(content AS BLOB)) AS own \
                   FROM chat_messages \
                   WHERE session_id = ? AND user_id = ? AND status = 'completed' \
                     AND replaces_id IS NULL AND seq < ?) \
                 WHERE running - own <= ? ORDER BY seq ASC",
                vec![
                    session_id.into(),
                    user.into(),
                    bound.into(),
                    (max_bytes as i64).into(),
                ],
            ))
            .await?;
        let rows = rows
            .iter()
            .map(|row| {
                Ok(HistoryRow {
                    seq: row.try_get("", "seq")?,
                    role: row.try_get("", "role")?,
                    content: row.try_get("", "content")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let total = self
            .db
            .query_one(stmt(
                "SELECT COUNT(*) AS n FROM chat_messages WHERE session_id = ? AND user_id = ? \
                 AND role = 'assistant' AND status = 'completed' AND replaces_id IS NULL AND seq < ?",
                vec![session_id.into(), user.into(), bound.into()],
            ))
            .await?;
        Ok(History {
            rows,
            total_completed_replies: match total {
                Some(row) => row.try_get("", "n")?,
                None => 0,
            },
        })
    }

    /// The session a task id already belongs to, if any message carries it.
    pub async fn session_of_task(&self, user: &str, task_id: &str) -> Result<Option<String>> {
        let row = self
            .db
            .query_one(stmt(
                "SELECT session_id FROM chat_messages WHERE user_id = ? AND task_id = ? LIMIT 1",
                vec![user.into(), task_id.into()],
            ))
            .await?;
        Ok(match row {
            Some(row) => Some(row.try_get("", "session_id")?),
            None => None,
        })
    }

    /// Replay lookup before anything is registered or validated: a repeated `client_request_id`
    /// with the same content returns the original ids; different content is a conflict. A
    /// regeneration is looked up by task id alone because the replaced reply may already be gone.
    pub async fn find_replay(&self, p: &BeginGeneration<'_>) -> Result<Option<Begun>> {
        replay(&self.db, p).await
    }

    /// The user message a reply answers and its sequence number: the last `user` message before
    /// `assistant_seq`. Used to rebuild the question when regenerating.
    pub async fn question_before(
        &self,
        user: &str,
        session_id: &str,
        assistant_seq: i64,
    ) -> Result<ChatMessage> {
        let row = self
            .db
            .query_one(stmt(
                &format!(
                    "{MESSAGE_SELECT} WHERE m.session_id = ? AND m.user_id = ? AND m.role = 'user' \
                     AND m.seq < ? ORDER BY m.seq DESC LIMIT 1"
                ),
                vec![session_id.into(), user.into(), assistant_seq.into()],
            ))
            .await?
            .ok_or(ChatStoreError::NotRegenerable)?;
        message_from_row(&row)
    }

    /// The transaction that starts a generation (D074): task row, skill/tool references, user
    /// message and the `generating` assistant placeholder. A repeated task id with the same
    /// content returns the original ids; different content is a conflict.
    pub async fn begin_generation(&self, p: &BeginGeneration<'_>) -> Result<Begun> {
        let tx = self.db.begin().await?;
        if let Some(begun) = replay(&tx, p).await? {
            return Ok(begun);
        }
        session_in(&tx, p.user, p.session_id).await?;
        let stamp = now();
        let mut user_message_id = p.user_message_id.to_owned();
        let mut replaces: Option<String> = None;
        if let Some(target) = p.regenerate_of {
            let old = message_in(&tx, p.user, target).await.map_err(|e| match e {
                ChatStoreError::MessageNotFound => ChatStoreError::NotRegenerable,
                other => other,
            })?;
            let last = tx
                .query_one(stmt(
                    "SELECT MAX(seq) AS s FROM chat_messages WHERE session_id = ? AND user_id = ?",
                    vec![p.session_id.into(), p.user.into()],
                ))
                .await?
                .and_then(|row| row.try_get::<Option<i64>>("", "s").ok().flatten());
            if old.session_id != p.session_id
                || old.role != "assistant"
                || old.status == "generating"
                || Some(old.seq) != last
            {
                return Err(ChatStoreError::NotRegenerable);
            }
            let prior = tx
                .query_one(stmt(
                    "SELECT id FROM chat_messages WHERE session_id = ? AND user_id = ? \
                     AND role = 'user' AND seq < ? ORDER BY seq DESC LIMIT 1",
                    vec![p.session_id.into(), p.user.into(), old.seq.into()],
                ))
                .await?
                .ok_or(ChatStoreError::NotRegenerable)?;
            user_message_id = prior.try_get("", "id")?;
            replaces = Some(old.id);
        }
        tx.execute(stmt(
            "INSERT INTO workbench_tasks (id, user_id, app_id, provider_id, model_id, status, \
             created_at, provider_config_revision, session_id, kind, endpoint_fingerprint) \
             VALUES (?, ?, 'ai-workbench', ?, ?, 'running', ?, ?, ?, 'chat', ?)",
            vec![
                p.task_id.into(),
                p.user.into(),
                p.provider_id.into(),
                p.model_id.into(),
                stamp.into(),
                p.provider_config_revision.into(),
                p.session_id.into(),
                p.endpoint_fingerprint.into(),
            ],
        ))
        .await?;
        if let Some(skill) = p.skill {
            tx.execute(stmt(
                "INSERT INTO workbench_task_skills (task_id, version_id, skill_id, content_hash) \
                 VALUES (?, ?, ?, ?)",
                vec![
                    p.task_id.into(),
                    skill.version_id.clone().into(),
                    skill.skill_id.clone().into(),
                    skill.content_hash.clone().into(),
                ],
            ))
            .await?;
        }
        for (server, tool, hash) in p.tools {
            tx.execute(stmt(
                "INSERT INTO workbench_task_tools (task_id, server_id, tool_name, schema_hash) \
                 VALUES (?, ?, ?, ?)",
                vec![
                    p.task_id.into(),
                    server.clone().into(),
                    tool.clone().into(),
                    hash.clone().into(),
                ],
            ))
            .await?;
        }
        let mut seq = tx
            .query_one(stmt(
                "SELECT COALESCE(MAX(seq), 0) AS s FROM chat_messages WHERE session_id = ?",
                vec![p.session_id.into()],
            ))
            .await?
            .map(|row| row.try_get::<i64>("", "s"))
            .transpose()?
            .unwrap_or(0);
        if let Some(content) = p.user_content {
            // The automatic title is the first user message only, and never over a user title.
            tx.execute(stmt(
                "UPDATE chat_sessions SET title = ? WHERE id = ? AND user_id = ? \
                 AND title_source = 'auto' AND NOT EXISTS \
                 (SELECT 1 FROM chat_messages WHERE session_id = ? AND role = 'user')",
                vec![
                    auto_title(content).into(),
                    p.session_id.into(),
                    p.user.into(),
                    p.session_id.into(),
                ],
            ))
            .await?;
            seq += 1;
            tx.execute(stmt(
                "INSERT INTO chat_messages (id, session_id, user_id, seq, role, content, status, \
                 task_id, created_at, updated_at) VALUES (?, ?, ?, ?, 'user', ?, 'completed', ?, ?, ?)",
                vec![
                    user_message_id.clone().into(),
                    p.session_id.into(),
                    p.user.into(),
                    seq.into(),
                    content.into(),
                    p.task_id.into(),
                    stamp.into(),
                    stamp.into(),
                ],
            ))
            .await?;
        }
        seq += 1;
        tx.execute(stmt(
            "INSERT INTO chat_messages (id, session_id, user_id, seq, role, content, status, \
             task_id, replaces_id, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'assistant', '', 'generating', ?, ?, ?, ?)",
            vec![
                p.assistant_message_id.into(),
                p.session_id.into(),
                p.user.into(),
                seq.into(),
                p.task_id.into(),
                replaces.into(),
                stamp.into(),
                stamp.into(),
            ],
        ))
        .await?;
        tx.execute(stmt(
            "UPDATE chat_sessions SET updated_at = ? WHERE id = ? AND user_id = ?",
            vec![stamp.into(), p.session_id.into(), p.user.into()],
        ))
        .await?;
        tx.commit().await?;
        Ok(Begun {
            user_message_id,
            assistant_message_id: p.assistant_message_id.to_owned(),
            replayed: false,
        })
    }

    /// The terminal transaction (D074/D076). Conditional updates make a second call, or a call
    /// after a competing terminal state, a no-op that reports `AlreadyFinalized`.
    pub async fn finalize_generation(&self, f: &FinalizeGeneration<'_>) -> Result<Finalized> {
        let tx = self.db.begin().await?;
        let stamp = now();
        let updated = tx
            .execute(stmt(
                "UPDATE chat_messages SET content = ?, status = ?, finish_reason = ?, \
                 error_code = ?, updated_at = ? WHERE id = ? AND user_id = ? \
                 AND role = 'assistant' AND status = 'generating'",
                vec![
                    f.content.into(),
                    f.message_status.into(),
                    f.finish_reason.map(str::to_owned).into(),
                    f.message_error.map(str::to_owned).into(),
                    stamp.into(),
                    f.assistant_message_id.into(),
                    f.user.into(),
                ],
            ))
            .await?;
        if updated.rows_affected() != 1 {
            return Ok(Finalized::AlreadyFinalized);
        }
        tx.execute(stmt(
            "UPDATE workbench_tasks SET status = ?, finished_at = ?, error_code = ?, total_tokens = ? \
             WHERE id = ? AND user_id = ? AND status = 'running'",
            vec![
                f.task_status.into(),
                stamp.into(),
                f.task_error.map(str::to_owned).into(),
                f.total_tokens.into(),
                f.task_id.into(),
                f.user.into(),
            ],
        ))
        .await?;
        let own = message_in(&tx, f.user, f.assistant_message_id).await?;
        let replaces = tx
            .query_one(stmt(
                "SELECT replaces_id FROM chat_messages WHERE id = ? AND user_id = ?",
                vec![f.assistant_message_id.into(), f.user.into()],
            ))
            .await?
            .map(|row| row.try_get::<Option<String>>("", "replaces_id"))
            .transpose()?
            .flatten();
        let mut snapshot = own;
        let mut discarded = false;
        if let Some(old_id) = replaces {
            if f.message_status == "completed" {
                tx.execute(stmt(
                    "DELETE FROM chat_messages WHERE id = ? AND user_id = ?",
                    vec![old_id.into(), f.user.into()],
                ))
                .await?;
                tx.execute(stmt(
                    "UPDATE chat_messages SET replaces_id = NULL WHERE id = ? AND user_id = ?",
                    vec![f.assistant_message_id.into(), f.user.into()],
                ))
                .await?;
            } else {
                // Keep the old reply; the half-finished draft is dropped (D076).
                snapshot = message_in(&tx, f.user, &old_id).await?;
                tx.execute(stmt(
                    "DELETE FROM chat_messages WHERE id = ? AND user_id = ?",
                    vec![f.assistant_message_id.into(), f.user.into()],
                ))
                .await?;
                discarded = true;
            }
        }
        tx.execute(stmt(
            "UPDATE chat_sessions SET updated_at = ? WHERE id = ? AND user_id = ?",
            vec![
                stamp.into(),
                snapshot.session_id.clone().into(),
                f.user.into(),
            ],
        ))
        .await?;
        tx.commit().await?;
        Ok(Finalized::Done {
            message: Box::new(snapshot),
            discarded,
        })
    }

    /// Startup recovery (D074): drop regeneration drafts, end every `generating` reply as
    /// `interrupted` (has content) or `failed`. Nothing is resent. Safe because the instance lock
    /// guarantees no other process is generating (D077).
    pub async fn recover_generating(&self) -> Result<u64> {
        let tx = self.db.begin().await?;
        tx.execute(stmt(
            "DELETE FROM chat_messages WHERE status = 'generating' AND replaces_id IS NOT NULL",
            vec![],
        ))
        .await?;
        let recovered = tx
            .execute(stmt(
                "UPDATE chat_messages SET status = CASE WHEN content <> '' THEN 'interrupted' ELSE 'failed' END, \
                 error_code = 'process_interrupted', updated_at = ? WHERE status = 'generating'",
                vec![now().into()],
            ))
            .await?;
        tx.commit().await?;
        Ok(recovered.rows_affected())
    }

    /// A `generating` reply with no live generation behind it (its job was dropped or panicked)
    /// would block the user through the one-generating index. The caller holds the user's
    /// registry slot, so any such row is an orphan: end it as `process_interrupted` and close the
    /// matching running task. Nothing is resent.
    pub async fn recover_user_orphans(&self, user: &str) -> Result<u64> {
        let tx = self.db.begin().await?;
        tx.execute(stmt(
            "DELETE FROM chat_messages WHERE user_id = ? AND status = 'generating' AND replaces_id IS NOT NULL",
            vec![user.into()],
        ))
        .await?;
        let stamp = now();
        tx.execute(stmt(
            "UPDATE workbench_tasks SET status = 'interrupted', error_code = 'process_interrupted', finished_at = ? \
             WHERE user_id = ? AND kind = 'chat' AND status = 'running'",
            vec![stamp.into(), user.into()],
        ))
        .await?;
        let recovered = tx
            .execute(stmt(
                "UPDATE chat_messages SET status = CASE WHEN content <> '' THEN 'interrupted' ELSE 'failed' END, \
                 error_code = 'process_interrupted', updated_at = ? WHERE user_id = ? AND status = 'generating'",
                vec![stamp.into(), user.into()],
            ))
            .await?;
        tx.commit().await?;
        Ok(recovered.rows_affected())
    }
}

/// Replay detection for a repeated `client_request_id` (the task id).
async fn replay(tx: &impl ConnectionTrait, p: &BeginGeneration<'_>) -> Result<Option<Begun>> {
    let existing = tx
        .query_all(stmt(
            "SELECT id, session_id, role, content FROM chat_messages \
             WHERE user_id = ? AND task_id = ? ORDER BY seq ASC",
            vec![p.user.into(), p.task_id.into()],
        ))
        .await?;
    if existing.is_empty() {
        return Ok(None);
    }
    let mut user_message: Option<(String, String)> = None;
    let mut assistant: Option<String> = None;
    for row in &existing {
        let session: String = row.try_get("", "session_id")?;
        if session != p.session_id {
            return Err(ChatStoreError::RequestConflict);
        }
        match row.try_get::<String>("", "role")?.as_str() {
            "user" => user_message = Some((row.try_get("", "id")?, row.try_get("", "content")?)),
            _ => assistant = Some(row.try_get("", "id")?),
        }
    }
    let assistant = assistant.ok_or(ChatStoreError::RequestConflict)?;
    match (p.user_content, user_message) {
        (Some(content), Some((id, stored))) if stored == content => Ok(Some(Begun {
            user_message_id: id,
            assistant_message_id: assistant,
            replayed: true,
        })),
        (None, None) => {
            let prior = tx
                .query_one(stmt(
                    "SELECT m.id AS id FROM chat_messages m WHERE m.session_id = ? AND m.user_id = ? \
                     AND m.role = 'user' AND m.seq < (SELECT seq FROM chat_messages WHERE id = ? AND user_id = ?) \
                     ORDER BY m.seq DESC LIMIT 1",
                    vec![
                        p.session_id.into(),
                        p.user.into(),
                        assistant.clone().into(),
                        p.user.into(),
                    ],
                ))
                .await?;
            Ok(Some(Begun {
                user_message_id: match prior {
                    Some(row) => row.try_get("", "id")?,
                    None => String::new(),
                },
                assistant_message_id: assistant,
                replayed: true,
            }))
        }
        _ => Err(ChatStoreError::RequestConflict),
    }
}
