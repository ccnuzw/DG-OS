//! D134 project, task and agent-session metadata. This SQLite database is the only write
//! source for these rows. Every query filters by the host-derived owner, and composite
//! foreign keys (migration `m20261104_000026_project_tasks`) keep references inside one
//! account and project. Failures use the stable strings from the V7 project contract.
mod projects;
mod executions;
mod cutover;
mod sessions;
mod runtime;
mod capabilities;
mod workspaces;
pub use workspaces::{GitOperationIntent, StoredWorkspace};
mod handoffs;
pub use handoffs::StoredHandoff;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod capability_tests;

pub use capabilities::{StoredCapabilityRevision, StoredSessionCapabilities};
pub use executions::StoredRun;
pub use cutover::{CleanupObservation, CleanupReceipt, CleanupTarget, CliProviderConfiguration, CliProviderSnapshot, CliRevocationScope, CutoverContext, CutoverReceipt, HostBootObservation, OpeningBinding, OpeningReceipt, OwnedCutoverView, SessionConfiguration, StoredCliBinding};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, QueryResult, Statement, TransactionTrait, Value};
use serde::{Deserialize, Serialize};

/// Global marker written in the same transaction as the legacy session import.
pub const LEGACY_KEY: &str = "agent-json-v1";
pub(crate) const TASK_STATUSES: [&str; 6] = ["todo", "in_progress", "blocked", "review", "completed", "cancelled"];
pub(crate) const PROJECT_COLUMNS: &str = "id, owner, name, path, directory_identity, archived, created_at, last_opened_at";
pub(crate) const TASK_COLUMNS: &str = "id, owner, project_id, goal, requirements, status, created_at";
pub(crate) const OPERATION_COLUMNS: &str = "id, owner, kind, path, status, project_id";
pub(crate) const SESSION_COLUMNS: &str = "id, owner, engine, native_id, workspace, executable, model, project_id, task_id";

/// Agent session row. `engine` is `codex` or `claude`; project and task are both set or both unset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredAgentSession {
    pub id: String,
    pub owner: String,
    pub engine: String,
    pub native_id: Option<String>,
    pub workspace: String,
    pub executable: Option<String>,
    pub model: Option<String>,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub path: String,
    pub directory_identity: String,
    pub archived: bool,
    pub created_at: i64,
    pub last_opened_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevelopmentTask {
    pub id: String,
    pub owner: String,
    pub project_id: String,
    pub goal: String,
    pub requirements: String,
    pub status: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectOperation {
    pub id: String,
    pub owner: String,
    pub kind: String,
    pub path: String,
    pub status: String,
    pub project_id: Option<String>,
}

/// Selected project, task and session. Selection is validated as a whole chain before it is stored.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub project_id: Option<String>,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DevelopmentWorkspace {
    pub projects: Vec<Project>,
    pub tasks: Vec<DevelopmentTask>,
    pub sessions: Vec<StoredAgentSession>,
    pub selection: Selection,
    pub operations: Vec<ProjectOperation>,
}

#[derive(Clone)]
pub struct DevelopmentRepository {
    db: DatabaseConnection,
}

impl DevelopmentRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Everything one account can see. Other accounts' rows never appear.
    pub async fn workspace(&self, owner: &str) -> Result<DevelopmentWorkspace, String> {
        let txn = self.db.begin().await.map_err(db)?;
        let projects = fetch(&txn, &format!("SELECT {PROJECT_COLUMNS} FROM development_projects WHERE owner=? ORDER BY last_opened_at DESC, created_at, id"), vec![v(owner)])
            .await?.iter().map(project_from).collect::<Result<Vec<_>, _>>()?;
        let tasks = fetch(&txn, &format!("SELECT {TASK_COLUMNS} FROM development_tasks WHERE owner=? ORDER BY created_at, id"), vec![v(owner)])
            .await?.iter().map(task_from).collect::<Result<Vec<_>, _>>()?;
        let sessions = fetch(&txn, &format!("SELECT {SESSION_COLUMNS} FROM development_agent_sessions WHERE owner=? ORDER BY id"), vec![v(owner)])
            .await?.iter().map(session_from).collect::<Result<Vec<_>, _>>()?;
        let operations = fetch(&txn, &format!("SELECT {OPERATION_COLUMNS} FROM development_project_operations WHERE owner=? ORDER BY id"), vec![v(owner)])
            .await?.iter().map(operation_from).collect::<Result<Vec<_>, _>>()?;
        let selection = match fetch_one(&txn, "SELECT project_id, task_id, session_id FROM development_selection WHERE owner=?", vec![v(owner)]).await? {
            None => Selection::default(),
            Some(row) => Selection {
                project_id: row.try_get("", "project_id").map_err(db)?,
                task_id: row.try_get("", "task_id").map_err(db)?,
                session_id: row.try_get("", "session_id").map_err(db)?,
            },
        };
        txn.commit().await.map_err(db)?;
        Ok(DevelopmentWorkspace { projects, tasks, sessions, selection, operations })
    }

    pub async fn selection(&self, owner: &str) -> Result<Selection, String> {
        match fetch_one(&self.db, "SELECT project_id, task_id, session_id FROM development_selection WHERE owner=?", vec![v(owner)]).await? {
            None => Ok(Selection::default()),
            Some(row) => Ok(Selection {
                project_id: row.try_get("", "project_id").map_err(db)?,
                task_id: row.try_get("", "task_id").map_err(db)?,
                session_id: row.try_get("", "session_id").map_err(db)?,
            }),
        }
    }

    pub async fn project(&self, owner: &str, id: &str) -> Result<Project, String> {
        project_row(&self.db, owner, id).await
    }

    pub async fn task(&self, owner: &str, id: &str) -> Result<DevelopmentTask, String> {
        task_row(&self.db, owner, id).await
    }
}

pub(crate) fn db(_: DbErr) -> String {
    "storage_failed".into()
}

/// Constraint failures on user-controlled identity (owner+path) are invalid input. Other
/// SQL errors are storage failures.
pub(crate) fn write_error(error: DbErr) -> String {
    if error.to_string().contains("UNIQUE constraint failed") { "invalid_project".into() } else { db(error) }
}

pub(crate) fn v<T: Into<Value>>(value: T) -> Value {
    value.into()
}

pub(crate) async fn execute<C: ConnectionTrait>(c: &C, sql: &str, values: Vec<Value>) -> Result<u64, String> {
    c.execute(Statement::from_sql_and_values(DbBackend::Sqlite, sql, values)).await
        .map(|r| r.rows_affected()).map_err(write_error)
}

pub(crate) async fn fetch<C: ConnectionTrait>(c: &C, sql: &str, values: Vec<Value>) -> Result<Vec<QueryResult>, String> {
    c.query_all(Statement::from_sql_and_values(DbBackend::Sqlite, sql, values)).await.map_err(db)
}

pub(crate) async fn fetch_one<C: ConnectionTrait>(c: &C, sql: &str, values: Vec<Value>) -> Result<Option<QueryResult>, String> {
    c.query_one(Statement::from_sql_and_values(DbBackend::Sqlite, sql, values)).await.map_err(db)
}

pub(crate) async fn project_row<C: ConnectionTrait>(c: &C, owner: &str, id: &str) -> Result<Project, String> {
    match fetch_one(c, &format!("SELECT {PROJECT_COLUMNS} FROM development_projects WHERE owner=? AND id=?"), vec![v(owner), v(id)]).await? {
        Some(row) => project_from(&row),
        None => Err("project_not_found".into()),
    }
}

pub(crate) async fn task_row<C: ConnectionTrait>(c: &C, owner: &str, id: &str) -> Result<DevelopmentTask, String> {
    match fetch_one(c, &format!("SELECT {TASK_COLUMNS} FROM development_tasks WHERE owner=? AND id=?"), vec![v(owner), v(id)]).await? {
        Some(row) => task_from(&row),
        None => Err("task_not_found".into()),
    }
}

pub(crate) async fn session_row<C: ConnectionTrait>(c: &C, owner: &str, id: &str) -> Result<StoredAgentSession, String> {
    match fetch_one(c, &format!("SELECT {SESSION_COLUMNS} FROM development_agent_sessions WHERE owner=? AND id=?"), vec![v(owner), v(id)]).await? {
        Some(row) => session_from(&row),
        None => Err("session_not_found".into()),
    }
}

pub(crate) fn check_project_fields(name: &str, path: &str, identity: &str) -> Result<(), String> {
    let chars = name.chars().count();
    if !(1..=200).contains(&chars) { return Err("invalid_project".into()); }
    if !std::path::Path::new(path).is_absolute() || identity.is_empty() { return Err("invalid_workspace".into()); }
    Ok(())
}

pub(crate) fn check_task_fields(goal: &str, requirements: &str, status: &str) -> Result<(), String> {
    if !(1..=8192).contains(&goal.len()) || !(1..=16384).contains(&requirements.len()) || !TASK_STATUSES.contains(&status) {
        return Err("invalid_task".into());
    }
    Ok(())
}

pub(crate) fn session_values(r: &StoredAgentSession) -> Vec<Value> {
    vec![v(r.id.clone()), v(r.owner.clone()), v(r.engine.clone()), v(r.native_id.clone()), v(r.workspace.clone()), v(r.executable.clone()), v(r.model.clone()), v(r.project_id.clone()), v(r.task_id.clone())]
}

fn project_from(r: &QueryResult) -> Result<Project, String> {
    Ok(Project {
        id: r.try_get("", "id").map_err(db)?,
        owner: r.try_get("", "owner").map_err(db)?,
        name: r.try_get("", "name").map_err(db)?,
        path: r.try_get("", "path").map_err(db)?,
        directory_identity: r.try_get("", "directory_identity").map_err(db)?,
        archived: r.try_get::<i64>("", "archived").map_err(db)? != 0,
        created_at: r.try_get("", "created_at").map_err(db)?,
        last_opened_at: r.try_get("", "last_opened_at").map_err(db)?,
    })
}

fn task_from(r: &QueryResult) -> Result<DevelopmentTask, String> {
    Ok(DevelopmentTask {
        id: r.try_get("", "id").map_err(db)?,
        owner: r.try_get("", "owner").map_err(db)?,
        project_id: r.try_get("", "project_id").map_err(db)?,
        goal: r.try_get("", "goal").map_err(db)?,
        requirements: r.try_get("", "requirements").map_err(db)?,
        status: r.try_get("", "status").map_err(db)?,
        created_at: r.try_get("", "created_at").map_err(db)?,
    })
}

fn operation_from(r: &QueryResult) -> Result<ProjectOperation, String> {
    Ok(ProjectOperation {
        id: r.try_get("", "id").map_err(db)?,
        owner: r.try_get("", "owner").map_err(db)?,
        kind: r.try_get("", "kind").map_err(db)?,
        path: r.try_get("", "path").map_err(db)?,
        status: r.try_get("", "status").map_err(db)?,
        project_id: r.try_get("", "project_id").map_err(db)?,
    })
}

fn session_from(r: &QueryResult) -> Result<StoredAgentSession, String> {
    Ok(StoredAgentSession {
        id: r.try_get("", "id").map_err(db)?,
        owner: r.try_get("", "owner").map_err(db)?,
        engine: r.try_get("", "engine").map_err(db)?,
        native_id: r.try_get("", "native_id").map_err(db)?,
        workspace: r.try_get("", "workspace").map_err(db)?,
        executable: r.try_get("", "executable").map_err(db)?,
        model: r.try_get("", "model").map_err(db)?,
        project_id: r.try_get("", "project_id").map_err(db)?,
        task_id: r.try_get("", "task_id").map_err(db)?,
    })
}
