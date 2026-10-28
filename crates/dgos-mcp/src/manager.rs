use crate::calls::{validate_call_id, ActiveCalls};
use crate::credentials::{CredentialScope, McpCredentialVault, SecretToken, UnavailableVault};
use crate::error::{McpError, Result};
use crate::policy::{DenyCalls, McpCallPolicy};
use crate::protocol::{
    initialize_params, parse_call_result, parse_tools, request, JsonRpcNotification, ToolDefinition,
};
use crate::transport::{next_id, response_result, Session};
use chrono::Utc;
use dgos_db::{
    entities::{mcp_connection_logs, mcp_server_tools, mcp_servers, mcp_usage_logs},
    sea_orm::{
        ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
        QuerySelect, Set, TransactionTrait,
    },
};
use dgos_types::ExecutionProvenance;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};
use tokio::{
    sync::{Mutex, RwLock, Semaphore},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportKind {
    Stdio,
    Http,
}

impl TransportKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StdioConfig {
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HttpConfig {
    pub url: String,
    pub credential_ref: Option<String>,
    #[serde(default)]
    pub allow_insecure_http: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateMcpServerRequest {
    pub name: String,
    pub transport: TransportKind,
    pub stdio: Option<StdioConfig>,
    pub http: Option<HttpConfig>,
    pub enabled: Option<bool>,
    pub auto_connect: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UpdateMcpServerRequest {
    pub name: Option<String>,
    pub transport: Option<TransportKind>,
    pub stdio: Option<StdioConfig>,
    pub http: Option<HttpConfig>,
    pub enabled: Option<bool>,
    pub auto_connect: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpServer {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub transport: String,
    pub enabled: bool,
    pub auto_connect: bool,
    pub status: String,
    pub last_error_class: Option<String>,
    pub reconnect_attempts: u32,
    pub reconnect_exhausted: bool,
    pub credential_ref: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpTool {
    pub server_id: String,
    pub tool_name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    pub schema_hash: String,
    pub discovered_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallMcpToolRequest {
    pub server_id: String,
    pub tool_name: String,
    pub arguments: Value,
    #[serde(default)]
    pub call_id: Option<String>,
    pub task_ref: Option<String>,
    pub timeout_ms: Option<u64>,
    pub confirmation_token: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallMcpToolResponse {
    pub server_id: String,
    pub tool_name: String,
    pub call_id: String,
    pub is_error: bool,
    pub content: Value,
    pub structured_content: Option<Value>,
    pub duration_ms: i64,
    pub usage_log_id: i32,
    pub audit_warning: bool,
    pub outcome: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpUsageLog {
    pub id: i32,
    pub server_id: String,
    pub tool_name: String,
    pub task_ref: Option<String>,
    pub caller: Option<String>,
    pub outcome: String,
    pub error_class: Option<String>,
    pub duration_ms: Option<i32>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpConnectionLog {
    pub id: i32,
    pub server_id: String,
    pub event: String,
    pub status: String,
    pub error_class: Option<String>,
    pub duration_ms: Option<i32>,
    pub reconnect_attempt: Option<i32>,
    pub created_at: i64,
}

#[derive(Clone)]
pub struct McpManager {
    db: DatabaseConnection,
    sessions: Arc<RwLock<HashMap<String, Arc<Session>>>>,
    manual_disconnects: Arc<RwLock<HashSet<String>>>,
    reconnects: Arc<RwLock<HashMap<String, RetryState>>>,
    ever_connected: Arc<RwLock<HashSet<String>>>,
    policy: Arc<dyn McpCallPolicy>,
    vault: Arc<dyn McpCredentialVault>,
    confirmations: Arc<RwLock<HashMap<String, Confirmation>>>,
    lifecycle: Arc<RwLock<HashMap<String, Arc<RwLock<()>>>>>,
    tool_snapshots: Arc<RwLock<HashMap<String, Arc<RwLock<()>>>>>,
    tool_refreshes: Arc<RwLock<HashMap<String, Arc<RwLock<()>>>>>,
    active_calls: ActiveCalls,
    call_slots: Arc<Semaphore>,
    stopping: CancellationToken,
    runtime: Arc<RwLock<()>>,
    monitor: Arc<StdMutex<Option<JoinHandle<()>>>>,
    shutdown_lock: Arc<Mutex<()>>,
    client: Client,
}

struct Confirmation {
    server_id: String,
    fingerprint: String,
    expires_at: Instant,
}

#[derive(Clone, Copy)]
struct RetryState {
    attempts: u32,
    next_at: Instant,
}

impl McpManager {
    pub fn new(db: DatabaseConnection) -> Self {
        Self::with_policy(db, Arc::new(DenyCalls))
    }

    pub fn with_policy(db: DatabaseConnection, policy: Arc<dyn McpCallPolicy>) -> Self {
        Self::with_policy_and_vault(db, policy, Arc::new(UnavailableVault))
    }

    pub fn with_policy_and_vault(
        db: DatabaseConnection,
        policy: Arc<dyn McpCallPolicy>,
        vault: Arc<dyn McpCredentialVault>,
    ) -> Self {
        Self {
            db,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            manual_disconnects: Arc::new(RwLock::new(HashSet::new())),
            reconnects: Arc::new(RwLock::new(HashMap::new())),
            ever_connected: Arc::new(RwLock::new(HashSet::new())),
            policy,
            vault,
            confirmations: Arc::new(RwLock::new(HashMap::new())),
            lifecycle: Arc::new(RwLock::new(HashMap::new())),
            tool_snapshots: Arc::new(RwLock::new(HashMap::new())),
            tool_refreshes: Arc::new(RwLock::new(HashMap::new())),
            active_calls: ActiveCalls::default(),
            call_slots: Arc::new(Semaphore::new(8)),
            stopping: CancellationToken::new(),
            runtime: Arc::new(RwLock::new(())),
            monitor: Arc::new(StdMutex::new(None)),
            shutdown_lock: Arc::new(Mutex::new(())),
            // Never redirect credentials or tool arguments to a second origin.
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("HTTP client"),
        }
    }

    /// Starts the bounded auto-connect and health monitor. It never blocks app startup.
    pub fn spawn_monitor(self: Arc<Self>) {
        let mut monitor = self.monitor.lock().unwrap();
        if monitor.is_some() || self.stopping.is_cancelled() {
            return;
        }
        let stopping = self.stopping.clone();
        let manager = Arc::downgrade(&self);
        *monitor = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tokio::select! {
                    biased;
                    _ = stopping.cancelled() => break,
                    _ = interval.tick() => {}
                }
                let Some(manager) = manager.upgrade() else {
                    break;
                };
                manager.monitor_tick().await;
            }
        }));
    }

    /// Stop admission first, then drain work before releasing shared sessions.
    /// The desktop host bounds the overall wait; crash recovery handles leftovers.
    pub async fn shutdown(&self) -> Result<()> {
        self.stopping.cancel();
        self.call_slots.close();
        let _shutdown = self.shutdown_lock.lock().await;
        let monitor = self.monitor.lock().unwrap().take();
        if let Some(monitor) = monitor {
            let _ = monitor.await;
        }
        let _runtime = self.runtime.write().await;
        self.confirmations.write().await.clear();
        let sessions = std::mem::take(&mut *self.sessions.write().await);
        let mut closing = tokio::task::JoinSet::new();
        for (id, session) in sessions {
            closing.spawn(async move {
                session.close().await;
                id
            });
        }
        while let Some(result) = closing.join_next().await {
            if let Ok(id) = result {
                if let Some(row) = mcp_servers::Entity::find_by_id(&id)
                    .one(&self.db)
                    .await
                    .map_err(map_db)?
                {
                    self.record_connection_event(
                        &row.user_id,
                        &id,
                        "disconnected",
                        "configured",
                        None,
                        None,
                        None,
                    )
                    .await;
                }
            }
        }
        self.reconnects.write().await.clear();
        self.recover_runtime_status().await
    }

    async fn monitor_tick(&self) {
        let Ok(_runtime) = cancellable(&self.stopping, self.runtime.read()).await else {
            return;
        };
        let rows = match mcp_servers::Entity::find()
            .filter(mcp_servers::Column::DeletedAt.is_null())
            .filter(mcp_servers::Column::Enabled.eq(true))
            .all(&self.db)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(error = %error, "MCP monitor database query failed");
                return;
            }
        };
        for row in rows {
            if self.stopping.is_cancelled() {
                break;
            }
            let lifecycle = self.lifecycle_lock(&row.id).await;
            let Ok(_guard) = cancellable(&self.stopping, lifecycle.write()).await else {
                break;
            };
            let Some(row) = self.get_model(&row.user_id, &row.id).await.ok().flatten() else {
                continue;
            };
            if !row.enabled || self.manual_disconnects.read().await.contains(&row.id) {
                continue;
            }
            let session = self.sessions.read().await.get(&row.id).cloned();
            if let Some(session) = session {
                let started = Instant::now();
                if let Err(error) = cancellable(
                    &self.stopping,
                    session.request(&request(next_id(), "ping", json!({})), 5_000),
                )
                .await
                .and_then(|response| response)
                .and_then(response_result)
                {
                    let removed = self.sessions.write().await.remove(&row.id);
                    if let Some(removed) = removed {
                        removed.close().await;
                    }
                    let _ = self
                        .set_status(&row.id, "degraded", Some(error.class()))
                        .await;
                    self.record_connection_event(
                        &row.user_id,
                        &row.id,
                        "disconnected",
                        "degraded",
                        Some(error.class()),
                        Some(started.elapsed().as_millis() as i32),
                        None,
                    )
                    .await;
                }
            } else if row.auto_connect
                || (matches!(row.status.as_str(), "degraded" | "error")
                    && self.ever_connected.read().await.contains(&row.id))
            {
                let retry_allowed = self
                    .reconnects
                    .read()
                    .await
                    .get(&row.id)
                    .map(|retry| retry.attempts < 3 && Instant::now() >= retry.next_at)
                    .unwrap_or(true);
                if retry_allowed {
                    match self.connect_inner_manager(&row.user_id, &row.id).await {
                        Ok(_) => {
                            self.reconnects.write().await.remove(&row.id);
                        }
                        Err(error) => {
                            let recoverable =
                                matches!(
                                    error,
                                    McpError::ConnectionFailed(_)
                                        | McpError::Transport(_)
                                        | McpError::Timeout
                                ) && self.ever_connected.read().await.contains(&row.id);
                            let mut retries = self.reconnects.write().await;
                            let attempts = if recoverable {
                                retries
                                    .get(&row.id)
                                    .map(|retry| retry.attempts.saturating_add(1))
                                    .unwrap_or(1)
                            } else {
                                3
                            };
                            let shift = attempts.saturating_sub(1).min(5);
                            let delay = Duration::from_secs(30 * (1_u64 << shift));
                            retries.insert(
                                row.id.clone(),
                                RetryState {
                                    attempts,
                                    next_at: Instant::now() + delay.min(Duration::from_secs(600)),
                                },
                            );
                            drop(retries);
                            self.record_connection_event(
                                &row.user_id,
                                &row.id,
                                if attempts >= 3 {
                                    "reconnect_exhausted"
                                } else {
                                    "reconnect_scheduled"
                                },
                                "error",
                                Some(error.class()),
                                None,
                                Some(attempts as i32),
                            )
                            .await;
                        }
                    }
                }
            }
        }
    }

    pub async fn recover_runtime_status(&self) -> Result<()> {
        mcp_servers::Entity::update_many()
            .filter(mcp_servers::Column::DeletedAt.is_null())
            .col_expr(
                mcp_servers::Column::Status,
                dgos_db::sea_orm::sea_query::Expr::value("configured"),
            )
            .col_expr(
                mcp_servers::Column::LastErrorClass,
                dgos_db::sea_orm::sea_query::Expr::value(Option::<String>::None),
            )
            .exec(&self.db)
            .await
            .map_err(map_db)?;
        mcp_usage_logs::Entity::update_many()
            .filter(mcp_usage_logs::Column::Outcome.eq("pending"))
            .col_expr(
                mcp_usage_logs::Column::Outcome,
                dgos_db::sea_orm::sea_query::Expr::value("interrupted"),
            )
            .col_expr(
                mcp_usage_logs::Column::ErrorClass,
                dgos_db::sea_orm::sea_query::Expr::value("mcp_interrupted"),
            )
            .exec(&self.db)
            .await
            .map_err(map_db)?;
        Ok(())
    }

    pub async fn list_servers(&self, user_id: &str) -> Result<Vec<McpServer>> {
        let rows = mcp_servers::Entity::find()
            .filter(mcp_servers::Column::UserId.eq(user_id))
            .filter(mcp_servers::Column::DeletedAt.is_null())
            .order_by_asc(mcp_servers::Column::Name)
            .all(&self.db)
            .await
            .map_err(|e| McpError::Database(e.to_string()))?;
        let retries = self.reconnects.read().await;
        Ok(rows
            .into_iter()
            .map(|row| {
                let attempts = retries.get(&row.id).map_or(0, |retry| retry.attempts);
                Self::server_from_model(row, attempts)
            })
            .collect())
    }

    pub async fn create_server(
        &self,
        user_id: &str,
        input: CreateMcpServerRequest,
    ) -> Result<McpServer> {
        validate_name(&input.name)?;
        let (config, credential_ref) = config_json(&input.transport, input.stdio, input.http)?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        let row = mcp_servers::ActiveModel {
            id: Set(id.clone()),
            user_id: Set(user_id.to_string()),
            name: Set(input.name),
            transport: Set(input.transport.as_str().to_string()),
            config_json: Set(config),
            credential_ref: Set(credential_ref),
            enabled: Set(input.enabled.unwrap_or(false)),
            auto_connect: Set(input.auto_connect.unwrap_or(false)),
            status: Set("configured".into()),
            last_error_class: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        };
        row.insert(&self.db).await.map_err(map_db)?;
        self.get_server(user_id, &id)
            .await?
            .ok_or(McpError::ServerNotFound)
    }

    pub async fn update_server(
        &self,
        user_id: &str,
        id: &str,
        input: UpdateMcpServerRequest,
    ) -> Result<McpServer> {
        let lifecycle = self.lifecycle_lock(id).await;
        let _guard = lifecycle.write().await;
        let current = self
            .get_model(user_id, id)
            .await?
            .ok_or(McpError::ServerNotFound)?;
        let transport = input
            .transport
            .unwrap_or_else(|| parse_transport(&current.transport));
        let (config, mut credential_ref) = match (input.stdio, input.http) {
            (None, None) if transport.as_str() == current.transport => {
                (current.config_json.clone(), current.credential_ref.clone())
            }
            (stdio, http) => config_json(&transport, stdio, http)?,
        };
        let previous_scope = current
            .credential_ref
            .clone()
            .map(|reference| Self::credential_scope(&current, reference))
            .transpose()?;
        let endpoint_changed = transport.as_str() != current.transport
            || (current.transport == "http"
                && serde_json::from_str::<HttpConfig>(&config)
                    .ok()
                    .map(|config| config.url)
                    != serde_json::from_str::<HttpConfig>(&current.config_json)
                        .ok()
                        .map(|config| config.url));
        if endpoint_changed {
            credential_ref = None;
        }
        if let Some(name) = &input.name {
            validate_name(name)?;
        }
        if self.sessions.read().await.contains_key(id) {
            self.disconnect_inner(user_id, id).await?;
        }
        let mut active: mcp_servers::ActiveModel = current.into();
        if let Some(name) = input.name {
            validate_name(&name)?;
            active.name = Set(name);
        }
        active.transport = Set(transport.as_str().into());
        active.config_json = Set(config);
        active.credential_ref = Set(credential_ref);
        if let Some(enabled) = input.enabled {
            active.enabled = Set(enabled);
        }
        if let Some(auto_connect) = input.auto_connect {
            active.auto_connect = Set(auto_connect);
        }
        active.status = Set("configured".into());
        active.last_error_class = Set(None);
        active.updated_at = Set(Utc::now().timestamp());
        self.manual_disconnects.write().await.remove(id);
        self.reconnects.write().await.remove(id);
        let row = active.update(&self.db).await.map_err(map_db)?;
        if let Some(previous) = previous_scope {
            if row.credential_ref.as_deref() != Some(&previous.reference) || endpoint_changed {
                if self.vault.remove(&previous).await.is_err() {
                    tracing::warn!("MCP obsolete credential cleanup failed");
                }
            }
        }
        Ok(Self::server_from_model(row, 0))
    }

    pub async fn toggle(&self, user_id: &str, id: &str, enabled: bool) -> Result<McpServer> {
        let lifecycle = self.lifecycle_lock(id).await;
        let _guard = lifecycle.write().await;
        let model = self
            .get_model(user_id, id)
            .await?
            .ok_or(McpError::ServerNotFound)?;
        if !enabled {
            self.disconnect_inner(user_id, id).await?;
        }
        let mut active: mcp_servers::ActiveModel = model.into();
        active.enabled = Set(enabled);
        active.status = Set(if self.sessions.read().await.contains_key(id) {
            "ready"
        } else {
            "configured"
        }
        .into());
        active.last_error_class = Set(None);
        active.updated_at = Set(Utc::now().timestamp());
        if enabled {
            self.manual_disconnects.write().await.remove(id);
            self.reconnects.write().await.remove(id);
        }
        Ok(Self::server_from_model(
            active.update(&self.db).await.map_err(map_db)?,
            0,
        ))
    }

    pub async fn delete_server(&self, user_id: &str, id: &str) -> Result<()> {
        let lifecycle = self.lifecycle_lock(id).await;
        let _guard = lifecycle.write().await;
        let model = self
            .get_model(user_id, id)
            .await?
            .ok_or(McpError::ServerNotFound)?;
        self.disconnect_inner(user_id, id).await?;
        let result = mcp_servers::Entity::update_many()
            .filter(mcp_servers::Column::Id.eq(id))
            .filter(mcp_servers::Column::UserId.eq(user_id))
            .col_expr(
                mcp_servers::Column::DeletedAt,
                dgos_db::sea_orm::sea_query::Expr::value(Utc::now().timestamp()),
            )
            .exec(&self.db)
            .await
            .map_err(map_db)?;
        if result.rows_affected == 0 {
            return Err(McpError::ServerNotFound);
        }
        if let Some(reference) = model.credential_ref.clone() {
            if self
                .vault
                .remove(&Self::credential_scope(&model, reference)?)
                .await
                .is_err()
            {
                tracing::warn!("MCP deleted server credential cleanup failed");
            }
        }
        Ok(())
    }

    pub async fn connect(&self, user_id: &str, id: &str) -> Result<McpServer> {
        let _runtime = cancellable(&self.stopping, self.runtime.read()).await?;
        let lifecycle = self.lifecycle_lock(id).await;
        let _guard = cancellable(&self.stopping, lifecycle.write()).await?;
        self.reconnects.write().await.remove(id);
        self.connect_inner_manager(user_id, id).await
    }

    async fn connect_inner_manager(&self, user_id: &str, id: &str) -> Result<McpServer> {
        let model = self
            .get_model(user_id, id)
            .await?
            .ok_or(McpError::ServerNotFound)?;
        if !model.enabled {
            return Err(McpError::Disabled);
        }
        self.manual_disconnects.write().await.remove(id);
        self.set_status(id, "connecting", None).await?;
        let started = Instant::now();
        let result = async {
            let (session, tools) = self.connect_inner(&model).await?;
            if let Err(error) = self.persist_tools(id, tools).await {
                session.close().await;
                return Err(error);
            }
            Ok(session)
        }
        .await;
        let attempt = self
            .reconnects
            .read()
            .await
            .get(id)
            .map(|retry| (retry.attempts + 1) as i32);
        match result {
            Ok(session) => {
                self.revoke_confirmations(id).await;
                let new_session = Arc::new(session);
                let old_session = self
                    .sessions
                    .write()
                    .await
                    .insert(id.to_string(), new_session);
                if let Some(old_session) = old_session {
                    old_session.close().await;
                }
                self.ever_connected.write().await.insert(id.to_owned());
                self.set_status(id, "ready", None).await?;
                self.record_connection_event(
                    &model.user_id,
                    id,
                    "connected",
                    "ready",
                    None,
                    Some(started.elapsed().as_millis() as i32),
                    attempt,
                )
                .await;
            }
            Err(error) => {
                let status = if self.sessions.read().await.contains_key(id) {
                    "degraded"
                } else {
                    "error"
                };
                self.set_status(id, status, Some(error.class())).await?;
                self.record_connection_event(
                    &model.user_id,
                    id,
                    "connect_failed",
                    status,
                    Some(error.class()),
                    Some(started.elapsed().as_millis() as i32),
                    attempt,
                )
                .await;
                return Err(error);
            }
        }
        self.get_server(user_id, id)
            .await?
            .ok_or(McpError::ServerNotFound)
    }

    async fn persist_tools(&self, id: &str, tools: Vec<ToolDefinition>) -> Result<()> {
        let now = Utc::now().timestamp();
        let tx = self.db.begin().await.map_err(map_db)?;
        mcp_server_tools::Entity::delete_many()
            .filter(mcp_server_tools::Column::ServerId.eq(id))
            .exec(&tx)
            .await
            .map_err(map_db)?;
        for tool in tools {
            let input_schema = serde_json::to_string(&tool.input_schema)
                .map_err(|e| McpError::Serialization(e.to_string()))?;
            let output_schema = tool
                .output_schema
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| McpError::Serialization(e.to_string()))?;
            let hash = hash_text(
                &json!({
                    "inputSchema": tool.input_schema,
                    "outputSchema": tool.output_schema,
                })
                .to_string(),
            );
            mcp_server_tools::ActiveModel {
                id: dgos_db::sea_orm::NotSet,
                server_id: Set(id.to_string()),
                tool_name: Set(tool.name),
                title: Set(tool.title),
                description: Set(tool.description),
                input_schema_json: Set(input_schema),
                output_schema_json: Set(output_schema),
                schema_hash: Set(hash),
                discovered_at: Set(now),
            }
            .insert(&tx)
            .await
            .map_err(map_db)?;
        }
        tx.commit().await.map_err(map_db)?;
        Ok(())
    }

    async fn connect_inner(
        &self,
        model: &mcp_servers::Model,
    ) -> Result<(Session, Vec<ToolDefinition>)> {
        let session = match model.transport.as_str() {
            "stdio" => {
                let config: StdioConfig = serde_json::from_str(&model.config_json)
                    .map_err(|e| McpError::InvalidConfig(e.to_string()))?;
                Session::stdio(&config.executable, &config.args, config.cwd.as_deref()).await?
            }
            "http" => {
                let config: HttpConfig = serde_json::from_str(&model.config_json)
                    .map_err(|e| McpError::InvalidConfig(e.to_string()))?;
                let credential = match model.credential_ref.as_deref() {
                    Some(reference) => {
                        let scope = CredentialScope {
                            user_id: model.user_id.clone(),
                            server_id: model.id.clone(),
                            endpoint: config.url.clone(),
                            reference: reference.to_owned(),
                        };
                        Some(cancellable(&self.stopping, self.vault.load(&scope)).await??)
                    }
                    None => None,
                };
                Session::http_with_consent(
                    config.url,
                    self.client.clone(),
                    credential,
                    config.allow_insecure_http,
                )?
            }
            _ => return Err(McpError::UnsupportedTransport),
        };
        let handshake = async {
            let init = session
                .request(
                    &request(next_id(), "initialize", initialize_params()),
                    15_000,
                )
                .await?;
            let init_result =
                response_result(init).map_err(|e| McpError::HandshakeFailed(e.to_string()))?;
            session.complete_initialize(init_result)?;
            session
                .notify(&JsonRpcNotification {
                    jsonrpc: "2.0",
                    method: "notifications/initialized".into(),
                    params: json!({}),
                })
                .await?;
            let tools = Self::discover_tools(&session).await?;
            Ok(tools)
        };
        let result = cancellable(
            &self.stopping,
            tokio::time::timeout(Duration::from_secs(30), handshake),
        )
        .await
        .and_then(|result| result.map_err(|_| McpError::Timeout))
        .and_then(|result| result);
        let tools = match result {
            Ok(tools) => tools,
            Err(error) => {
                session.close().await;
                return Err(error);
            }
        };
        Ok((session, tools))
    }

    /// Discover tools on an already initialized session; snapshot publication
    /// is separately synchronized so network discovery can overlap tool calls.
    async fn discover_tools(session: &Session) -> Result<Vec<ToolDefinition>> {
        if !session.has_tools()? {
            return Ok(Vec::new());
        }
        let mut tools = Vec::new();
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut names = HashSet::new();
        loop {
            let params = cursor
                .as_ref()
                .map(|cursor| json!({"cursor": cursor}))
                .unwrap_or(json!({}));
            let page = response_result(
                session
                    .request(&request(next_id(), "tools/list", params), 15_000)
                    .await?,
            )?;
            for tool in parse_tools(page.clone())? {
                if tool.name.is_empty() || !names.insert(tool.name.clone()) || tools.len() >= 1_000
                {
                    return Err(McpError::Protocol("invalid or excessive tool list".into()));
                }
                // Compile without external resolution: discovery must not fetch schema URLs.
                if tool.input_schema.get("type").and_then(Value::as_str) != Some("object") {
                    return Err(McpError::Protocol(
                        "input schema must describe an object".into(),
                    ));
                }
                jsonschema::validator_for(&tool.input_schema)
                    .map_err(|_| McpError::Protocol("invalid input schema".into()))?;
                if let Some(output_schema) = &tool.output_schema {
                    if output_schema.get("type").and_then(Value::as_str) != Some("object") {
                        return Err(McpError::Protocol(
                            "output schema must describe an object".into(),
                        ));
                    }
                    jsonschema::validator_for(output_schema)
                        .map_err(|_| McpError::Protocol("invalid output schema".into()))?;
                }
                tools.push(tool);
            }
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            match &cursor {
                Some(value) if cursors.len() < 100 && cursors.insert(value.clone()) => {}
                Some(_) => return Err(McpError::Protocol("invalid tool pagination".into())),
                None => break,
            }
        }
        Ok(tools)
    }

    pub async fn disconnect(&self, user_id: &str, id: &str) -> Result<McpServer> {
        let lifecycle = self.lifecycle_lock(id).await;
        let _guard = lifecycle.write().await;
        self.disconnect_inner(user_id, id).await
    }

    async fn disconnect_inner(&self, user_id: &str, id: &str) -> Result<McpServer> {
        self.get_model(user_id, id)
            .await?
            .ok_or(McpError::ServerNotFound)?;
        self.revoke_confirmations(id).await;
        if let Some(session) = self.sessions.write().await.remove(id) {
            session.close().await;
        }
        self.manual_disconnects.write().await.insert(id.to_string());
        self.reconnects.write().await.remove(id);
        self.set_status(id, "configured", None).await?;
        self.record_connection_event(user_id, id, "disconnected", "configured", None, None, None)
            .await;
        self.get_server(user_id, id)
            .await?
            .ok_or(McpError::ServerNotFound)
    }

    pub async fn list_tools(&self, user_id: &str, id: &str) -> Result<Vec<McpTool>> {
        self.get_model(user_id, id)
            .await?
            .ok_or(McpError::ServerNotFound)?;
        let rows = mcp_server_tools::Entity::find()
            .filter(mcp_server_tools::Column::ServerId.eq(id))
            .order_by_asc(mcp_server_tools::Column::ToolName)
            .all(&self.db)
            .await
            .map_err(map_db)?;
        rows.into_iter().map(Self::tool_from_model).collect()
    }

    pub async fn server_config(&self, user_id: &str, id: &str) -> Result<CreateMcpServerRequest> {
        let model = self
            .get_model(user_id, id)
            .await?
            .ok_or(McpError::ServerNotFound)?;
        let transport = parse_transport(&model.transport);
        Ok(CreateMcpServerRequest {
            name: model.name,
            stdio: if model.transport == "stdio" {
                Some(
                    serde_json::from_str(&model.config_json)
                        .map_err(|_| McpError::InvalidConfig("invalid stdio config".into()))?,
                )
            } else {
                None
            },
            http: if model.transport == "http" {
                let mut config: HttpConfig = serde_json::from_str(&model.config_json)
                    .map_err(|_| McpError::InvalidConfig("invalid HTTP config".into()))?;
                config.credential_ref = model.credential_ref;
                Some(config)
            } else {
                None
            },
            transport,
            enabled: Some(model.enabled),
            auto_connect: Some(model.auto_connect),
        })
    }

    pub async fn set_bearer_token(
        &self,
        user_id: &str,
        id: &str,
        token: SecretToken,
    ) -> Result<McpServer> {
        let lifecycle = self.lifecycle_lock(id).await;
        let _guard = lifecycle.write().await;
        let model = self
            .get_model(user_id, id)
            .await?
            .ok_or(McpError::ServerNotFound)?;
        let scope = Self::credential_scope(&model, uuid::Uuid::new_v4().to_string())?;
        self.disconnect_inner(user_id, id).await?;
        self.vault.store(&scope, token).await?;
        let previous = model
            .credential_ref
            .clone()
            .map(|reference| Self::credential_scope(&model, reference))
            .transpose()?;
        let mut active: mcp_servers::ActiveModel = model.into();
        active.credential_ref = Set(Some(scope.reference.clone()));
        active.status = Set("configured".into());
        active.updated_at = Set(Utc::now().timestamp());
        if let Err(error) = active.update(&self.db).await {
            let _ = self.vault.remove(&scope).await;
            return Err(map_db(error));
        }
        if let Some(previous) = previous {
            if self.vault.remove(&previous).await.is_err() {
                tracing::warn!("MCP obsolete credential cleanup failed");
            }
        }
        self.get_server(user_id, id)
            .await?
            .ok_or(McpError::ServerNotFound)
    }

    pub async fn clear_bearer_token(&self, user_id: &str, id: &str) -> Result<McpServer> {
        let lifecycle = self.lifecycle_lock(id).await;
        let _guard = lifecycle.write().await;
        let model = self
            .get_model(user_id, id)
            .await?
            .ok_or(McpError::ServerNotFound)?;
        self.disconnect_inner(user_id, id).await?;
        if let Some(reference) = model.credential_ref.clone() {
            self.vault
                .remove(&Self::credential_scope(&model, reference)?)
                .await?;
        }
        let mut active: mcp_servers::ActiveModel = model.into();
        active.credential_ref = Set(None);
        active.status = Set("configured".into());
        active.updated_at = Set(Utc::now().timestamp());
        Ok(Self::server_from_model(
            active.update(&self.db).await.map_err(map_db)?,
            0,
        ))
    }

    fn credential_scope(model: &mcp_servers::Model, reference: String) -> Result<CredentialScope> {
        if model.transport != "http" {
            return Err(McpError::InvalidConfig(
                "Bearer tokens are only supported for HTTP".into(),
            ));
        }
        let config: HttpConfig = serde_json::from_str(&model.config_json)
            .map_err(|_| McpError::InvalidConfig("invalid HTTP config".into()))?;
        Ok(CredentialScope {
            user_id: model.user_id.clone(),
            server_id: model.id.clone(),
            endpoint: config.url,
            reference,
        })
    }

    pub async fn refresh_tools(&self, user_id: &str, id: &str) -> Result<Vec<McpTool>> {
        let _runtime = cancellable(&self.stopping, self.runtime.read()).await?;
        let lifecycle = self.lifecycle_lock(id).await;
        let _guard = cancellable(&self.stopping, lifecycle.read()).await?;
        let refresh = Self::server_lock(&self.tool_refreshes, id).await;
        let _refresh = cancellable(&self.stopping, refresh.write()).await?;
        self.get_model(user_id, id)
            .await?
            .ok_or(McpError::ServerNotFound)?;
        let session = self
            .sessions
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or(McpError::NotConnected)?;
        // Discover first; only replace the persisted snapshot after the entire
        // response has validated, preserving the previous snapshot on failure.
        let tools = cancellable(
            &self.stopping,
            tokio::time::timeout(Duration::from_secs(30), Self::discover_tools(&session)),
        )
        .await?
        .map_err(|_| McpError::Timeout)??;
        let snapshot = Self::server_lock(&self.tool_snapshots, id).await;
        let _snapshot = snapshot.write().await;
        self.persist_tools(id, tools).await?;
        self.revoke_confirmations(id).await;
        self.list_tools(user_id, id).await
    }

    pub async fn call_tool(
        &self,
        user_id: &str,
        input: CallMcpToolRequest,
    ) -> Result<CallMcpToolResponse> {
        self.call_tool_with_schema(user_id, input, None, None, None)
            .await
    }

    pub async fn call_tool_with_provenance(
        &self,
        user_id: &str,
        input: CallMcpToolRequest,
        provenance: &ExecutionProvenance,
    ) -> Result<CallMcpToolResponse> {
        self.call_tool_with_schema(user_id, input, None, None, Some(provenance))
            .await
    }

    /// Hosts pin a task's tool contract; checking under the dispatch snapshot
    /// lock closes the race between UI review, confirmation and refresh.
    pub async fn call_tool_at_schema(
        &self,
        user_id: &str,
        input: CallMcpToolRequest,
        schema_hash: &str,
    ) -> Result<CallMcpToolResponse> {
        self.call_tool_with_schema(user_id, input, Some(schema_hash), None, None)
            .await
    }

    pub async fn call_tool_at_schema_cancellable(
        &self,
        user_id: &str,
        input: CallMcpToolRequest,
        schema_hash: &str,
        cancellation: &CancellationToken,
    ) -> Result<CallMcpToolResponse> {
        self.call_tool_with_schema(user_id, input, Some(schema_hash), Some(cancellation), None)
            .await
    }

    pub async fn call_tool_at_schema_cancellable_with_provenance(
        &self,
        user_id: &str,
        input: CallMcpToolRequest,
        schema_hash: &str,
        cancellation: &CancellationToken,
        provenance: &ExecutionProvenance,
    ) -> Result<CallMcpToolResponse> {
        self.call_tool_with_schema(
            user_id,
            input,
            Some(schema_hash),
            Some(cancellation),
            Some(provenance),
        )
        .await
    }

    async fn call_tool_with_schema(
        &self,
        user_id: &str,
        input: CallMcpToolRequest,
        schema_hash: Option<&str>,
        cancellation: Option<&CancellationToken>,
        provenance: Option<&ExecutionProvenance>,
    ) -> Result<CallMcpToolResponse> {
        let _runtime = cancellable(&self.stopping, self.runtime.read()).await?;
        let call_id = input
            .call_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let active = self
            .active_calls
            .register(user_id, &input.server_id, &call_id)?;
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            active.token.cancel();
        }
        let call = self.call_tool_inner(
            user_id,
            &input,
            &call_id,
            &active.token,
            schema_hash,
            provenance,
        );
        tokio::pin!(call);
        let external_cancel = async {
            match cancellation {
                Some(token) => token.cancelled().await,
                None => std::future::pending::<()>().await,
            }
        };
        let result = tokio::select! {
            biased;
            result = &mut call => result,
            _ = self.stopping.cancelled() => { active.token.cancel(); call.await },
            _ = external_cancel => { active.token.cancel(); call.await },
        };
        if let Err(error) = &result {
            self.record_rejected_call(user_id, &input, error, provenance)
                .await;
        }
        result
    }

    async fn call_tool_inner(
        &self,
        user_id: &str,
        input: &CallMcpToolRequest,
        call_id: &str,
        cancellation: &CancellationToken,
        schema_hash: Option<&str>,
        provenance: Option<&ExecutionProvenance>,
    ) -> Result<CallMcpToolResponse> {
        let _slot = cancellable(
            cancellation,
            tokio::time::timeout(Duration::from_secs(5), self.call_slots.acquire()),
        )
        .await?
        .map_err(|_| McpError::Timeout)?
        .map_err(|_| McpError::Cancelled)?;
        let lifecycle = self.lifecycle_lock(&input.server_id).await;
        let _guard = cancellable(cancellation, lifecycle.read()).await?;
        let snapshot = Self::server_lock(&self.tool_snapshots, &input.server_id).await;
        let _snapshot = cancellable(cancellation, snapshot.read()).await?;
        if !cancellable(
            cancellation,
            self.policy
                .allows_call(user_id, &input.server_id, &input.tool_name),
        )
        .await?
        {
            return Err(McpError::PermissionDenied);
        }
        let model = self
            .get_model(user_id, &input.server_id)
            .await?
            .ok_or(McpError::ServerNotFound)?;
        if !model.enabled {
            return Err(McpError::Disabled);
        }
        if !self.sessions.read().await.contains_key(&input.server_id) {
            return Err(McpError::NotConnected);
        }
        let tool = mcp_server_tools::Entity::find()
            .filter(mcp_server_tools::Column::ServerId.eq(&input.server_id))
            .filter(mcp_server_tools::Column::ToolName.eq(&input.tool_name))
            .one(&self.db)
            .await
            .map_err(map_db)?
            .ok_or(McpError::ToolNotFound)?;
        validate_arguments(&tool.input_schema_json, &input.arguments)?;
        if schema_hash.is_some_and(|expected| expected != tool.schema_hash) {
            return Err(McpError::ToolInputInvalid(
                "Selected tool schema changed".into(),
            ));
        }
        let fingerprint = call_fingerprint(user_id, &input, &tool.schema_hash);
        let confirmation = match input.confirmation_token.as_deref() {
            Some(token) => self.confirmations.write().await.remove(token),
            None => None,
        };
        // Tool annotations/descriptions are untrusted. Until a host allowlist exists,
        // every tool has unknown side effects and needs explicit one-use consent.
        if !confirmation.is_some_and(|consent| {
            consent.expires_at > Instant::now() && consent.fingerprint == fingerprint
        }) {
            return Err(McpError::ConfirmationRequired);
        }
        let session = self
            .sessions
            .read()
            .await
            .get(&input.server_id)
            .cloned()
            .ok_or(McpError::NotConnected)?;
        let request_hash = hash_text(&serde_json::to_string(&input.arguments).unwrap_or_default());
        // Persist intent before external side effects. An audit failure must not
        // encourage a user to repeat a tool that already ran successfully.
        let audit = mcp_usage_logs::ActiveModel {
            id: dgos_db::sea_orm::NotSet,
            user_id: Set(user_id.to_string()),
            server_id: Set(input.server_id.clone()),
            tool_name: Set(input.tool_name.clone()),
            task_ref: Set(input.task_ref.clone()),
            caller: Set(provenance.map(|p| format!("{:?}", p.caller).to_lowercase())),
            request_hash: Set(Some(request_hash)),
            outcome: Set("pending".into()),
            error_class: Set(None),
            duration_ms: Set(None),
            created_at: Set(Utc::now().timestamp()),
        }
        .insert(&self.db)
        .await
        .map_err(map_db)?;
        let usage_log_id = audit.id;
        let started = std::time::Instant::now();
        let timeout_ms = input.timeout_ms.unwrap_or(30_000).clamp(100, 120_000);
        let params = json!({ "name": input.tool_name, "arguments": input.arguments });
        let wire = request(next_id(), "tools/call", params);
        let result = if cancellation.is_cancelled() {
            Err(McpError::Cancelled)
        } else {
            let result = cancellable(cancellation, session.request(&wire, timeout_ms))
                .await
                .and_then(|value| value);
            if matches!(result, Err(McpError::Cancelled | McpError::Timeout)) {
                let notification = JsonRpcNotification {
                    jsonrpc: "2.0",
                    method: "notifications/cancelled".into(),
                    params: json!({ "requestId": wire.id, "reason": "Host stopped waiting for this request" }),
                };
                let _ = tokio::time::timeout(Duration::from_secs(1), session.notify(&notification))
                    .await;
            }
            result
        };
        let elapsed = started.elapsed().as_millis() as i32;
        let mut structured_content = None;
        let (outcome, error_class, is_error, content) = match result {
            Ok(response) => match response_result(response)
                .and_then(|value| parse_call_result(value, tool.output_schema_json.as_deref()))
            {
                Ok(value) => {
                    structured_content = value.structured_content;
                    let is_error = value.is_error;
                    (
                        if is_error { "error" } else { "success" },
                        is_error.then(|| "mcp_tool_call_failed".to_owned()),
                        is_error,
                        value.content,
                    )
                }
                Err(error) => (
                    call_error_outcome(&error),
                    Some(error.class().to_string()),
                    true,
                    json!({"error": error.to_string()}),
                ),
            },
            Err(error) => (
                call_error_outcome(&error),
                Some(error.class().to_string()),
                true,
                json!({"error": error.to_string()}),
            ),
        };
        let mut log: mcp_usage_logs::ActiveModel = audit.into();
        log.outcome = Set(outcome.into());
        log.error_class = Set(error_class);
        log.duration_ms = Set(Some(elapsed));
        let audit_warning = log.update(&self.db).await.is_err();
        if audit_warning {
            tracing::error!(
                usage_log_id,
                "MCP audit outcome persistence failed; tool must not be automatically retried"
            );
        }
        Ok(CallMcpToolResponse {
            server_id: input.server_id.clone(),
            tool_name: input.tool_name.clone(),
            call_id: call_id.into(),
            outcome: outcome.into(),
            is_error,
            content,
            structured_content,
            duration_ms: elapsed as i64,
            usage_log_id,
            audit_warning,
        })
    }

    /// Cancels an in-flight call by its caller-supplied opaque id. Dropping the
    /// transport future removes its pending request; the remote server may have
    /// already started side effects, so cancellation is never reported as undo.
    pub async fn cancel_call(&self, user_id: &str, server_id: &str, call_id: &str) -> Result<bool> {
        self.active_calls.cancel(user_id, server_id, call_id)
    }

    async fn record_rejected_call(
        &self,
        user_id: &str,
        input: &CallMcpToolRequest,
        error: &McpError,
        provenance: Option<&ExecutionProvenance>,
    ) {
        // Do not let an untrusted caller forge audit rows against another user's Server.
        if !matches!(self.get_model(user_id, &input.server_id).await, Ok(Some(_))) {
            return;
        }
        if matches!(error, McpError::Cancelled) {
            if let Some(token) = &input.confirmation_token {
                let mut confirmations = self.confirmations.write().await;
                if confirmations
                    .get(token)
                    .is_some_and(|value| value.server_id == input.server_id)
                {
                    confirmations.remove(token);
                }
            }
        }
        let row = mcp_usage_logs::ActiveModel {
            id: dgos_db::sea_orm::NotSet,
            user_id: Set(user_id.to_owned()),
            server_id: Set(input.server_id.clone()),
            tool_name: Set(input.tool_name.clone()),
            task_ref: Set(input.task_ref.clone()),
            caller: Set(provenance.map(|p| format!("{:?}", p.caller).to_lowercase())),
            request_hash: Set(Some(hash_text(&input.arguments.to_string()))),
            outcome: Set(match error {
                McpError::PermissionDenied
                | McpError::Disabled
                | McpError::ConfirmationRequired
                | McpError::ToolInputInvalid(_)
                | McpError::ToolNotFound => "denied",
                _ => call_error_outcome(error),
            }
            .into()),
            error_class: Set(Some(error.class().into())),
            duration_ms: Set(Some(0)),
            created_at: Set(Utc::now().timestamp()),
        };
        if row.insert(&self.db).await.is_err() {
            tracing::warn!("MCP rejected call audit could not be persisted");
        }
    }

    pub async fn list_usage_logs(
        &self,
        user_id: &str,
        server_id: Option<&str>,
        limit: Option<u64>,
    ) -> Result<Vec<McpUsageLog>> {
        let mut query = mcp_usage_logs::Entity::find()
            .filter(mcp_usage_logs::Column::UserId.eq(user_id))
            .order_by_desc(mcp_usage_logs::Column::CreatedAt)
            .order_by_desc(mcp_usage_logs::Column::Id);
        if let Some(id) = server_id {
            query = query.filter(mcp_usage_logs::Column::ServerId.eq(id));
        }
        query = query.limit(limit.unwrap_or(50).clamp(1, 100));
        let rows = query.all(&self.db).await.map_err(map_db)?;
        Ok(rows
            .into_iter()
            .map(|row| McpUsageLog {
                id: row.id,
                server_id: row.server_id,
                tool_name: row.tool_name,
                task_ref: row.task_ref,
                caller: row.caller,
                outcome: row.outcome,
                error_class: row.error_class,
                duration_ms: row.duration_ms,
                created_at: row.created_at,
            })
            .collect())
    }

    pub async fn list_connection_logs(
        &self,
        user_id: &str,
        server_id: Option<&str>,
        limit: Option<u64>,
    ) -> Result<Vec<McpConnectionLog>> {
        let mut query = mcp_connection_logs::Entity::find()
            .filter(mcp_connection_logs::Column::UserId.eq(user_id))
            .order_by_desc(mcp_connection_logs::Column::CreatedAt)
            .order_by_desc(mcp_connection_logs::Column::Id);
        if let Some(id) = server_id {
            query = query.filter(mcp_connection_logs::Column::ServerId.eq(id));
        }
        let rows = query
            .limit(limit.unwrap_or(50).clamp(1, 100))
            .all(&self.db)
            .await
            .map_err(map_db)?;
        Ok(rows
            .into_iter()
            .map(|row| McpConnectionLog {
                id: row.id,
                server_id: row.server_id,
                event: row.event,
                status: row.status,
                error_class: row.error_class,
                duration_ms: row.duration_ms,
                reconnect_attempt: row.reconnect_attempt,
                created_at: row.created_at,
            })
            .collect())
    }

    async fn record_connection_event(
        &self,
        user_id: &str,
        server_id: &str,
        event: &str,
        status: &str,
        error_class: Option<&str>,
        duration_ms: Option<i32>,
        reconnect_attempt: Option<i32>,
    ) {
        let row = mcp_connection_logs::ActiveModel {
            id: dgos_db::sea_orm::NotSet,
            user_id: Set(user_id.to_owned()),
            server_id: Set(server_id.to_owned()),
            event: Set(event.to_owned()),
            status: Set(status.to_owned()),
            error_class: Set(error_class.map(str::to_owned)),
            duration_ms: Set(duration_ms),
            reconnect_attempt: Set(reconnect_attempt),
            created_at: Set(Utc::now().timestamp()),
        };
        if let Err(error) = row.insert(&self.db).await {
            tracing::warn!(%error, "MCP connection event audit failed");
        }
    }

    async fn get_model(&self, user_id: &str, id: &str) -> Result<Option<mcp_servers::Model>> {
        mcp_servers::Entity::find_by_id(id)
            .filter(mcp_servers::Column::UserId.eq(user_id))
            .filter(mcp_servers::Column::DeletedAt.is_null())
            .one(&self.db)
            .await
            .map_err(map_db)
    }

    async fn lifecycle_lock(&self, id: &str) -> Arc<RwLock<()>> {
        Self::server_lock(&self.lifecycle, id).await
    }

    async fn server_lock(
        locks: &RwLock<HashMap<String, Arc<RwLock<()>>>>,
        id: &str,
    ) -> Arc<RwLock<()>> {
        locks
            .write()
            .await
            .entry(id.to_owned())
            .or_default()
            .clone()
    }

    async fn revoke_confirmations(&self, server_id: &str) {
        self.confirmations
            .write()
            .await
            .retain(|_, consent| consent.server_id != server_id);
    }

    /// Called only after a trusted host UI has presented the exact tool and arguments.
    pub async fn confirm_call(&self, user_id: &str, input: &CallMcpToolRequest) -> Result<String> {
        let _runtime = cancellable(&self.stopping, self.runtime.read()).await?;
        if let Some(id) = &input.call_id {
            validate_call_id(id)?;
        }
        let lifecycle = self.lifecycle_lock(&input.server_id).await;
        let _guard = lifecycle.read().await;
        let snapshot = Self::server_lock(&self.tool_snapshots, &input.server_id).await;
        let _snapshot = snapshot.read().await;
        if !self
            .policy
            .allows_call(user_id, &input.server_id, &input.tool_name)
            .await
        {
            return Err(McpError::PermissionDenied);
        }
        let server = self
            .get_model(user_id, &input.server_id)
            .await?
            .ok_or(McpError::ServerNotFound)?;
        if !server.enabled {
            return Err(McpError::Disabled);
        }
        if !self.sessions.read().await.contains_key(&input.server_id) {
            return Err(McpError::NotConnected);
        }
        let tool = mcp_server_tools::Entity::find()
            .filter(mcp_server_tools::Column::ServerId.eq(&input.server_id))
            .filter(mcp_server_tools::Column::ToolName.eq(&input.tool_name))
            .one(&self.db)
            .await
            .map_err(map_db)?
            .ok_or(McpError::ToolNotFound)?;
        validate_arguments(&tool.input_schema_json, &input.arguments)?;
        let mut confirmations = self.confirmations.write().await;
        confirmations.retain(|_, value| value.expires_at > Instant::now());
        if confirmations.len() >= 256 {
            return Err(McpError::PermissionDenied);
        }
        let token = uuid::Uuid::new_v4().to_string();
        confirmations.insert(
            token.clone(),
            Confirmation {
                server_id: input.server_id.clone(),
                fingerprint: call_fingerprint(user_id, input, &tool.schema_hash),
                expires_at: Instant::now() + Duration::from_secs(120),
            },
        );
        Ok(token)
    }

    async fn get_server(&self, user_id: &str, id: &str) -> Result<Option<McpServer>> {
        let row = self.get_model(user_id, id).await?;
        let retries = self.reconnects.read().await;
        Ok(row.map(|row| {
            let attempts = retries.get(&row.id).map_or(0, |retry| retry.attempts);
            Self::server_from_model(row, attempts)
        }))
    }

    async fn set_status(&self, id: &str, status: &str, error_class: Option<&str>) -> Result<()> {
        mcp_servers::Entity::update_many()
            .filter(mcp_servers::Column::Id.eq(id))
            .col_expr(
                mcp_servers::Column::Status,
                dgos_db::sea_orm::sea_query::Expr::value(status),
            )
            .col_expr(
                mcp_servers::Column::LastErrorClass,
                dgos_db::sea_orm::sea_query::Expr::value(error_class.map(str::to_string)),
            )
            .col_expr(
                mcp_servers::Column::UpdatedAt,
                dgos_db::sea_orm::sea_query::Expr::value(Utc::now().timestamp()),
            )
            .exec(&self.db)
            .await
            .map_err(map_db)?;
        Ok(())
    }

    fn server_from_model(row: mcp_servers::Model, reconnect_attempts: u32) -> McpServer {
        let reconnect_exhausted = row.enabled && row.status == "error" && reconnect_attempts >= 3;
        McpServer {
            id: row.id,
            user_id: row.user_id,
            name: row.name,
            transport: row.transport,
            enabled: row.enabled,
            auto_connect: row.auto_connect,
            status: row.status,
            last_error_class: row.last_error_class,
            reconnect_attempts,
            reconnect_exhausted,
            credential_ref: row.credential_ref,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
    fn tool_from_model(row: mcp_server_tools::Model) -> Result<McpTool> {
        Ok(McpTool {
            server_id: row.server_id,
            tool_name: row.tool_name,
            title: row.title,
            description: row.description,
            input_schema: serde_json::from_str(&row.input_schema_json)
                .map_err(|e| McpError::Serialization(e.to_string()))?,
            output_schema: row
                .output_schema_json
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(|e| McpError::Serialization(e.to_string()))?,
            schema_hash: row.schema_hash,
            discovered_at: row.discovered_at,
        })
    }
}

async fn cancellable<T>(
    token: &CancellationToken,
    operation: impl std::future::Future<Output = T>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = token.cancelled() => Err(McpError::Cancelled),
        result = operation => Ok(result),
    }
}

fn call_error_outcome(error: &McpError) -> &'static str {
    match error {
        McpError::Cancelled => "cancelled",
        McpError::Timeout => "timeout",
        _ => "error",
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.len() > 100 {
        Err(McpError::InvalidConfig("name must be 1-100 bytes".into()))
    } else {
        Ok(())
    }
}

fn config_json(
    transport: &TransportKind,
    stdio: Option<StdioConfig>,
    http: Option<HttpConfig>,
) -> Result<(String, Option<String>)> {
    match transport {
        TransportKind::Stdio => {
            let config =
                stdio.ok_or_else(|| McpError::InvalidConfig("stdio config is required".into()))?;
            crate::transport::validate_executable(&config.executable)?;
            Ok((
                serde_json::to_string(&config)
                    .map_err(|e| McpError::Serialization(e.to_string()))?,
                None,
            ))
        }
        TransportKind::Http => {
            let config =
                http.ok_or_else(|| McpError::InvalidConfig("http config is required".into()))?;
            crate::transport::validate_http_url_with_consent(
                &config.url,
                config.allow_insecure_http,
            )?;
            let credential = config.credential_ref.clone();
            let persisted =
                json!({ "url": config.url, "allow_insecure_http": config.allow_insecure_http });
            Ok((
                serde_json::to_string(&persisted)
                    .map_err(|e| McpError::Serialization(e.to_string()))?,
                credential,
            ))
        }
    }
}

fn parse_transport(value: &str) -> TransportKind {
    if value == "http" {
        TransportKind::Http
    } else {
        TransportKind::Stdio
    }
}

fn validate_arguments(schema: &str, arguments: &Value) -> Result<()> {
    let schema: Value =
        serde_json::from_str(schema).map_err(|e| McpError::ToolInputInvalid(e.to_string()))?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|_| McpError::ToolInputInvalid("invalid input schema".into()))?;
    if !arguments.is_object() || !validator.is_valid(arguments) {
        // Validation errors can embed the complete input; do not return them via IPC.
        return Err(McpError::ToolInputInvalid(
            "arguments do not match the tool input schema".into(),
        ));
    }
    Ok(())
}

fn call_fingerprint(user: &str, input: &CallMcpToolRequest, schema_hash: &str) -> String {
    hash_text(
        &json!([
            user,
            input.server_id,
            input.tool_name,
            input.arguments,
            input.task_ref,
            input.call_id,
            input.timeout_ms,
            schema_hash
        ])
        .to_string(),
    )
}

fn hash_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn map_db(error: dgos_db::sea_orm::DbErr) -> McpError {
    if matches!(
        error.sql_err(),
        Some(dgos_db::sea_orm::SqlErr::UniqueConstraintViolation(_))
    ) {
        McpError::DuplicateServer
    } else {
        McpError::Database("MCP database operation failed".into())
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use dgos_db::{sea_orm::Database, sea_orm_migration::MigratorTrait};

    async fn setup() -> (McpManager, String) {
        setup_mode("normal").await
    }

    async fn setup_mode(mode: &str) -> (McpManager, String) {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        dgos_db::migrations::Migrator::up(&db, None).await.unwrap();
        let manager = McpManager::new(db);
        let server = manager
            .create_server(
                "alice",
                CreateMcpServerRequest {
                    name: "monitor".into(),
                    transport: TransportKind::Stdio,
                    stdio: Some(StdioConfig {
                        executable: "node".into(),
                        args: vec![
                            format!(
                                "{}/tests/fixtures/mcp-server.mjs",
                                env!("CARGO_MANIFEST_DIR")
                            ),
                            mode.into(),
                        ],
                        cwd: None,
                    }),
                    http: None,
                    enabled: Some(true),
                    auto_connect: Some(true),
                },
            )
            .await
            .unwrap();
        (manager, server.id)
    }

    #[tokio::test]
    async fn monitor_does_not_retain_manager_after_owner_is_dropped() {
        let (manager, _) = setup().await;
        let manager = Arc::new(manager);
        let weak = Arc::downgrade(&manager);
        manager.clone().spawn_monitor();
        drop(manager);
        tokio::task::yield_now().await;
        assert!(weak.upgrade().is_none());
    }

    #[tokio::test]
    async fn monitor_respects_manual_disconnect_and_disable() {
        let (manager, id) = setup().await;
        manager.monitor_tick().await;
        assert!(manager.sessions.read().await.contains_key(&id));
        manager.disconnect("alice", &id).await.unwrap();
        manager.monitor_tick().await;
        assert!(!manager.sessions.read().await.contains_key(&id));
        manager.connect("alice", &id).await.unwrap();
        manager.toggle("alice", &id, false).await.unwrap();
        manager.monitor_tick().await;
        assert!(!manager.sessions.read().await.contains_key(&id));
    }

    #[tokio::test]
    async fn monitor_retry_is_bounded_backed_off_and_manual_connect_resets_it() {
        let (manager, id) = setup().await;
        manager.connect("alice", &id).await.unwrap();
        manager
            .sessions
            .write()
            .await
            .remove(&id)
            .unwrap()
            .close()
            .await;
        let model = manager.get_model("alice", &id).await.unwrap().unwrap();
        let saved_config = model.config_json.clone();
        let mut active: mcp_servers::ActiveModel = model.into();
        active.config_json =
            Set(json!({"executable":"/nonexistent/dgos-mcp-fixture", "args":[]}).to_string());
        active.update(&manager.db).await.unwrap();
        manager.monitor_tick().await;
        assert_eq!(manager.reconnects.read().await[&id].attempts, 1);
        manager.monitor_tick().await;
        assert_eq!(manager.reconnects.read().await[&id].attempts, 1);
        for expected in 2..=3 {
            manager
                .reconnects
                .write()
                .await
                .get_mut(&id)
                .unwrap()
                .next_at = Instant::now();
            manager.monitor_tick().await;
            assert_eq!(manager.reconnects.read().await[&id].attempts, expected);
        }
        let unavailable = manager.list_servers("alice").await.unwrap();
        assert_eq!(unavailable[0].reconnect_attempts, 3);
        assert!(unavailable[0].reconnect_exhausted);
        assert!(manager.list_servers("bob").await.unwrap().is_empty());
        let next = manager.reconnects.read().await[&id].next_at;
        manager.monitor_tick().await;
        assert_eq!(manager.reconnects.read().await[&id].next_at, next);
        let mut active: mcp_servers::ActiveModel = manager
            .get_model("alice", &id)
            .await
            .unwrap()
            .unwrap()
            .into();
        active.config_json = Set(saved_config);
        active.update(&manager.db).await.unwrap();
        manager.connect("alice", &id).await.unwrap();
        assert!(!manager.reconnects.read().await.contains_key(&id));
        assert!(!manager.list_servers("alice").await.unwrap()[0].reconnect_exhausted);
        manager.disconnect("alice", &id).await.unwrap();
    }

    #[tokio::test]
    async fn recovery_resets_stale_ready_status_without_starting_processes() {
        let (manager, id) = setup().await;
        manager
            .set_status(&id, "ready", Some("old_error"))
            .await
            .unwrap();
        manager.recover_runtime_status().await.unwrap();
        assert!(manager.sessions.read().await.is_empty());
        let server = manager.get_server("alice", &id).await.unwrap().unwrap();
        assert_eq!(server.status, "configured");
        assert!(server.last_error_class.is_none());
    }

    #[tokio::test]
    async fn heartbeat_rpc_error_and_timeout_disconnect_then_recover_without_startup_autoconnect() {
        for mode in ["ping-error", "ping-silent"] {
            let (manager, id) = setup_mode(mode).await;
            manager
                .update_server(
                    "alice",
                    &id,
                    UpdateMcpServerRequest {
                        auto_connect: Some(false),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            manager.monitor_tick().await;
            assert!(manager.sessions.read().await.is_empty());
            manager.connect("alice", &id).await.unwrap();
            manager.monitor_tick().await;
            assert!(!manager.sessions.read().await.contains_key(&id));
            assert_eq!(
                manager
                    .get_server("alice", &id)
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                "degraded"
            );
            let logs = manager
                .list_connection_logs("alice", Some(&id), None)
                .await
                .unwrap();
            assert!(logs
                .iter()
                .any(|log| log.event == "disconnected" && log.error_class.is_some()));
            manager.monitor_tick().await;
            assert!(manager.sessions.read().await.contains_key(&id));
            assert_eq!(
                manager
                    .get_server("alice", &id)
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                "ready"
            );
            manager.disconnect("alice", &id).await.unwrap();
            manager.monitor_tick().await;
            assert!(!manager.sessions.read().await.contains_key(&id));
            manager.toggle("alice", &id, false).await.unwrap();
            manager.toggle("alice", &id, true).await.unwrap();
            manager.monitor_tick().await;
            assert!(!manager.sessions.read().await.contains_key(&id));
        }
    }

    #[tokio::test]
    async fn queued_call_can_cancel_before_dispatch_without_waiting_for_call_slots() {
        struct Allow;
        #[async_trait::async_trait]
        impl McpCallPolicy for Allow {
            async fn allows_call(&self, _: &str, _: &str, _: &str) -> bool {
                true
            }
        }
        let (original, id) = setup_mode("cancellation-observable").await;
        let manager = McpManager::with_policy(original.db.clone(), Arc::new(Allow));
        manager.connect("alice", &id).await.unwrap();
        let mut input = CallMcpToolRequest {
            server_id: id.clone(),
            tool_name: "echo".into(),
            arguments: json!({"text":"queued"}),
            call_id: Some("queued-call".into()),
            task_ref: None,
            timeout_ms: None,
            confirmation_token: None,
        };
        input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
        let all_slots = manager.call_slots.acquire_many(8).await.unwrap();
        let cancelling = async {
            tokio::time::timeout(Duration::from_secs(1), async {
                while !manager
                    .cancel_call("alice", &id, "queued-call")
                    .await
                    .unwrap()
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        };
        let (result, _) = tokio::join!(manager.call_tool("alice", input.clone()), cancelling);
        assert!(matches!(result, Err(McpError::Cancelled)));
        assert!(!manager
            .cancel_call("alice", &id, "queued-call")
            .await
            .unwrap());
        let logs = manager
            .list_usage_logs("alice", Some(&id), None)
            .await
            .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].outcome, "cancelled");
        assert!(manager.sessions.read().await.contains_key(&id));
        drop(all_slots);
        assert!(matches!(
            manager.call_tool("alice", input).await,
            Err(McpError::ConfirmationRequired)
        ));
        let mut probe = CallMcpToolRequest {
            server_id: id.clone(),
            tool_name: "echo".into(),
            arguments: json!({"text":"fixture-counts"}),
            call_id: Some("queued-probe".into()),
            task_ref: None,
            timeout_ms: None,
            confirmation_token: None,
        };
        probe.confirmation_token = Some(manager.confirm_call("alice", &probe).await.unwrap());
        let response = manager.call_tool("alice", probe).await.unwrap();
        let counts: Value =
            serde_json::from_str(response.content[0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(counts["started"], 0);
        manager.disconnect("alice", &id).await.unwrap();
    }
}
