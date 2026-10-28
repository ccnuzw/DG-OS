use async_trait::async_trait;
use dgos_db::{
    sea_orm::{Database, EntityTrait},
    sea_orm_migration::MigratorTrait,
};
use dgos_mcp::*;
use serde_json::json;
use std::{collections::HashMap, process::Stdio, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::Mutex,
};

#[derive(Default)]
struct Vault(Mutex<HashMap<String, String>>);
#[async_trait]
impl McpCredentialVault for Vault {
    async fn store(&self, scope: &CredentialScope, token: SecretToken) -> Result<()> {
        self.0
            .lock()
            .await
            .insert(scope.account(), token.expose().to_owned());
        Ok(())
    }
    async fn load(&self, scope: &CredentialScope) -> Result<SecretToken> {
        SecretToken::new(
            self.0
                .lock()
                .await
                .get(&scope.account())
                .cloned()
                .ok_or(McpError::CredentialUnavailable)?,
        )
    }
    async fn remove(&self, scope: &CredentialScope) -> Result<()> {
        self.0.lock().await.remove(&scope.account());
        Ok(())
    }
}
struct Allow;
#[async_trait]
impl McpCallPolicy for Allow {
    async fn allows_call(&self, _: &str, _: &str, _: &str) -> bool {
        true
    }
}

#[test]
fn credential_scope_is_bound_to_user_server_endpoint_and_reference() {
    let original = CredentialScope {
        user_id: "alice".into(),
        server_id: "s1".into(),
        endpoint: "https://example.com/mcp".into(),
        reference: "ref1".into(),
    };
    for scope in [
        CredentialScope {
            user_id: "bob".into(),
            ..original.clone()
        },
        CredentialScope {
            server_id: "s2".into(),
            ..original.clone()
        },
        CredentialScope {
            endpoint: "https://example.com/other".into(),
            ..original.clone()
        },
        CredentialScope {
            reference: "ref2".into(),
            ..original.clone()
        },
    ] {
        assert_ne!(scope.account(), original.account());
    }
    assert!(!original.account().contains("alice"));
    assert!(SecretToken::new("bad token".into()).is_err());
    assert!(SecretToken::new("".into()).is_err());
    assert!(SecretToken::new("token\r\n".into()).is_err());
}

#[tokio::test]
async fn http_authenticated_handshake_call_rotation_and_url_change_cleanup() {
    authenticated_http_flow("streamable").await;
}

#[tokio::test]
async fn legacy_sse_authenticated_handshake_calls_cancellation_and_credential_cleanup() {
    authenticated_http_flow("legacy").await;
}

async fn authenticated_http_flow(mode: &str) {
    let mut child = Command::new("node")
        .arg(format!(
            "{}/tests/fixtures/mcp-http-server.mjs",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg(mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line),
    )
    .await
    .unwrap()
    .unwrap();
    let url = line.trim().to_owned();
    let db = Database::connect("sqlite::memory:").await.unwrap();
    dgos_db::migrations::Migrator::up(&db, None).await.unwrap();
    let vault = Arc::new(Vault::default());
    let manager = McpManager::with_policy_and_vault(db.clone(), Arc::new(Allow), vault.clone());
    let server = manager
        .create_server(
            "alice",
            CreateMcpServerRequest {
                name: "auth".into(),
                transport: TransportKind::Http,
                stdio: None,
                http: Some(HttpConfig {
                    url: url.clone(),
                    credential_ref: None,
                    allow_insecure_http: false,
                }),
                enabled: Some(true),
                auto_connect: None,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        manager.connect("alice", &server.id).await,
        Err(McpError::CredentialUnavailable)
    ));
    let saved = manager
        .set_bearer_token(
            "alice",
            &server.id,
            SecretToken::new("test-only-token-one".into()).unwrap(),
        )
        .await
        .unwrap();
    assert!(saved.credential_ref.is_some());
    let config = manager.server_config("alice", &server.id).await.unwrap();
    assert!(!serde_json::to_string(&config)
        .unwrap()
        .contains("test-only-token"));
    assert!(matches!(
        manager
            .set_bearer_token(
                "bob",
                &server.id,
                SecretToken::new("test-only-token-other".into()).unwrap()
            )
            .await,
        Err(McpError::ServerNotFound)
    ));
    manager.connect("alice", &server.id).await.unwrap();
    let mut input = CallMcpToolRequest {
        call_id: Some(uuid::Uuid::new_v4().to_string()),
        server_id: server.id.clone(),
        tool_name: "echo".into(),
        arguments: json!({"text":"hello"}),
        timeout_ms: None,
        task_ref: None,
        confirmation_token: None,
    };
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    assert_eq!(
        manager
            .call_tool("alice", input.clone())
            .await
            .unwrap()
            .content[0]["text"],
        "hello"
    );
    let mut delayed = input.clone();
    delayed.call_id = Some(uuid::Uuid::new_v4().to_string());
    delayed.arguments = json!({"text":"cancelled-http-result", "delay":400});
    delayed.confirmation_token = Some(manager.confirm_call("alice", &delayed).await.unwrap());
    let call_id = delayed.call_id.clone().unwrap();
    let running = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.call_tool("alice", delayed).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let mut probe = input.clone();
            probe.call_id = Some(uuid::Uuid::new_v4().to_string());
            probe.arguments = json!({"text":"fixture-counts"});
            probe.confirmation_token = Some(manager.confirm_call("alice", &probe).await.unwrap());
            let counts = manager.call_tool("alice", probe).await.unwrap();
            let counts: serde_json::Value =
                serde_json::from_str(counts.content[0]["text"].as_str().unwrap()).unwrap();
            if counts["started"].as_u64().unwrap() >= 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(manager
        .cancel_call("alice", &server.id, &call_id)
        .await
        .unwrap());
    let cancelled = running.await.unwrap().unwrap();
    assert_eq!(cancelled.outcome, "cancelled");
    assert!(cancelled.is_error && !cancelled.audit_warning);
    assert!(!manager
        .cancel_call("alice", &server.id, &call_id)
        .await
        .unwrap());
    let mut probe = input.clone();
    probe.call_id = Some(uuid::Uuid::new_v4().to_string());
    probe.arguments = json!({"text":"fixture-counts"});
    probe.confirmation_token = Some(manager.confirm_call("alice", &probe).await.unwrap());
    let counts = manager.call_tool("alice", probe).await.unwrap();
    let counts: serde_json::Value =
        serde_json::from_str(counts.content[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(counts["cancellations"], 1);
    if mode == "legacy" {
        assert_eq!(counts["pingReplied"], true);
        assert_eq!(counts["unsupportedCode"], -32601);
    }
    let rotated = manager
        .set_bearer_token(
            "alice",
            &server.id,
            SecretToken::new("test-only-token-two".into()).unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(saved.credential_ref, rotated.credential_ref);
    assert_eq!(vault.0.lock().await.len(), 1);
    assert_eq!(rotated.status, "configured");
    assert!(matches!(
        manager.call_tool("alice", input).await,
        Err(McpError::NotConnected)
    ));
    manager.connect("alice", &server.id).await.unwrap();
    let changed = manager
        .update_server(
            "alice",
            &server.id,
            UpdateMcpServerRequest {
                http: Some(HttpConfig {
                    url: format!("{url}/other"),
                    credential_ref: rotated.credential_ref,
                    allow_insecure_http: false,
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(changed.credential_ref.is_none());
    assert!(vault.0.lock().await.is_empty());
    let rows = dgos_db::entities::mcp_servers::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert!(!format!("{rows:?}").contains("test-only-token"));
    manager
        .set_bearer_token(
            "alice",
            &server.id,
            SecretToken::new("test-only-token-three".into()).unwrap(),
        )
        .await
        .unwrap();
    let cleared = manager
        .clear_bearer_token("alice", &server.id)
        .await
        .unwrap();
    assert!(cleared.credential_ref.is_none());
    assert!(vault.0.lock().await.is_empty());
    manager
        .set_bearer_token(
            "alice",
            &server.id,
            SecretToken::new("test-only-token-four".into()).unwrap(),
        )
        .await
        .unwrap();
    manager.delete_server("alice", &server.id).await.unwrap();
    assert!(vault.0.lock().await.is_empty());
    child.kill().await.unwrap();
}
