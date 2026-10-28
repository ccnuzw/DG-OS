//! Opt-in compatibility gate for the official MCP filesystem Server.
//!
//! This is intentionally ignored in normal offline CI: the package is an
//! external executable and must be supplied by the release validation job.
use async_trait::async_trait;
use dgos_db::{sea_orm::Database, sea_orm_migration::MigratorTrait};
use dgos_mcp::*;
use serde_json::json;
use std::{env, path::PathBuf, process::Stdio, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

struct Allow;
#[async_trait]
impl McpCallPolicy for Allow {
    async fn allows_call(&self, _: &str, _: &str, _: &str) -> bool {
        true
    }
}

#[tokio::test]
#[ignore = "requires an installed official @modelcontextprotocol/server-filesystem executable"]
async fn official_filesystem_server_stdio_connects_lists_and_calls() {
    let executable = env::var("DGOS_OFFICIAL_NODE").unwrap_or_else(|_| "node".into());
    let server_script = PathBuf::from(env::var("DGOS_OFFICIAL_FILESYSTEM_SERVER").expect(
        "set DGOS_OFFICIAL_FILESYSTEM_SERVER to @modelcontextprotocol/server-filesystem/dist/index.js",
    ));
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("compat.txt"),
        "DGOS official compatibility\n",
    )
    .unwrap();
    let db = Database::connect("sqlite::memory:").await.unwrap();
    dgos_db::migrations::Migrator::up(&db, None).await.unwrap();
    let manager = McpManager::with_policy(db, Arc::new(Allow));
    let server = manager
        .create_server(
            "compat-user",
            CreateMcpServerRequest {
                name: "official-filesystem".into(),
                transport: TransportKind::Stdio,
                stdio: Some(StdioConfig {
                    executable,
                    args: vec![
                        server_script.to_string_lossy().into_owned(),
                        root.path().to_string_lossy().into_owned(),
                    ],
                    cwd: None,
                }),
                http: None,
                enabled: Some(true),
                auto_connect: Some(false),
            },
        )
        .await
        .unwrap();
    manager.connect("compat-user", &server.id).await.unwrap();
    let tools = manager.list_tools("compat-user", &server.id).await.unwrap();
    assert!(
        tools.iter().any(|tool| tool.tool_name == "list_directory"),
        "official tools: {tools:?}"
    );
    let mut request = CallMcpToolRequest {
        server_id: server.id.clone(),
        tool_name: "list_directory".into(),
        arguments: json!({"path": root.path()}),
        call_id: Some("official-compat-call".into()),
        task_ref: Some("official-compat".into()),
        timeout_ms: Some(15_000),
        confirmation_token: None,
    };
    request.confirmation_token = Some(manager.confirm_call("compat-user", &request).await.unwrap());
    let response = manager.call_tool("compat-user", request).await.unwrap();
    assert!(
        !response.is_error,
        "official filesystem response: {:?}",
        response.content
    );
    assert!(response.content.to_string().contains("compat.txt"));
    manager.disconnect("compat-user", &server.id).await.unwrap();
}

#[tokio::test]
#[ignore = "requires an isolated prefix with official @modelcontextprotocol/sdk and zod"]
async fn official_sdk_streamable_http_connects_lists_and_calls() {
    official_sdk_http_flow("streamable").await;
}

#[tokio::test]
#[ignore = "requires an isolated prefix with official @modelcontextprotocol/sdk and zod"]
async fn official_sdk_legacy_sse_connects_lists_and_calls() {
    official_sdk_http_flow("legacy").await;
}

async fn official_sdk_http_flow(mode: &str) {
    let prefix = env::var("DGOS_OFFICIAL_MCP_SDK_PREFIX")
        .expect("set DGOS_OFFICIAL_MCP_SDK_PREFIX to an isolated npm install prefix");
    let node = env::var("DGOS_OFFICIAL_NODE").unwrap_or_else(|_| "node".into());
    let mut child = Command::new(node)
        .arg(format!(
            "{}/tests/fixtures/mcp-official-sdk-http.mjs",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg(prefix)
        .arg(mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut url = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut url),
    )
    .await
    .unwrap()
    .unwrap();
    let db = Database::connect("sqlite::memory:").await.unwrap();
    dgos_db::migrations::Migrator::up(&db, None).await.unwrap();
    let manager = McpManager::with_policy(db, Arc::new(Allow));
    let server = manager
        .create_server(
            "compat-user",
            CreateMcpServerRequest {
                name: "official-sdk-http".into(),
                transport: TransportKind::Http,
                stdio: None,
                http: Some(HttpConfig {
                    url: url.trim().to_owned(),
                    credential_ref: None,
                    allow_insecure_http: false,
                }),
                enabled: Some(true),
                auto_connect: Some(false),
            },
        )
        .await
        .unwrap();
    manager.connect("compat-user", &server.id).await.unwrap();
    let tools = manager.list_tools("compat-user", &server.id).await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool_name, "echo");
    assert!(tools[0].output_schema.is_some());
    let mut request = CallMcpToolRequest {
        server_id: server.id.clone(),
        tool_name: "echo".into(),
        arguments: json!({"text": "DGOS SDK compatibility"}),
        call_id: Some("official-http-compat".into()),
        task_ref: Some("official-compat".into()),
        timeout_ms: Some(15000),
        confirmation_token: None,
    };
    request.confirmation_token = Some(manager.confirm_call("compat-user", &request).await.unwrap());
    let response = manager.call_tool("compat-user", request).await.unwrap();
    assert_eq!(response.outcome, "success");
    assert!(!response.is_error && !response.audit_warning);
    assert_eq!(response.content[0]["text"], "DGOS SDK compatibility");
    assert_eq!(
        response.structured_content,
        Some(json!({"text": "DGOS SDK compatibility"}))
    );
    assert_eq!(
        manager
            .refresh_tools("compat-user", &server.id)
            .await
            .unwrap()
            .len(),
        1
    );
    let logs = manager
        .list_usage_logs("compat-user", Some(&server.id), None)
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert!(!serde_json::to_string(&logs)
        .unwrap()
        .contains("DGOS SDK compatibility"));
    manager.disconnect("compat-user", &server.id).await.unwrap();
    child.kill().await.unwrap();
}
