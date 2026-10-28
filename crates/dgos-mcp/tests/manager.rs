use async_trait::async_trait;
use dgos_db::{
    sea_orm::{ConnectionTrait, Database, DatabaseConnection, EntityTrait},
    sea_orm_migration::MigratorTrait,
};
use dgos_mcp::*;
use serde_json::json;
use std::sync::Arc;

struct Allow;
#[async_trait]
impl McpCallPolicy for Allow {
    async fn allows_call(&self, user: &str, _: &str, _: &str) -> bool {
        user == "alice"
    }
}
async fn setup(allow: bool) -> (DatabaseConnection, McpManager) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    dgos_db::migrations::Migrator::up(&db, None).await.unwrap();
    let manager = if allow {
        McpManager::with_policy(db.clone(), Arc::new(Allow))
    } else {
        McpManager::new(db.clone())
    };
    (db, manager)
}
fn config(name: &str, mode: &str) -> CreateMcpServerRequest {
    CreateMcpServerRequest {
        name: name.into(),
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
        auto_connect: None,
    }
}
fn call(server: &str) -> CallMcpToolRequest {
    CallMcpToolRequest {
        server_id: server.into(),
        tool_name: "echo".into(),
        arguments: json!({"text":"private-value"}),
        call_id: Some(uuid::Uuid::new_v4().to_string()),
        task_ref: None,
        timeout_ms: None,
        confirmation_token: None,
    }
}

#[tokio::test]
async fn server_crud_isolation_defaults_and_duplicates() {
    let (_, manager) = setup(true).await;
    let mut input = config("local", "normal");
    input.enabled = None;
    let server = manager.create_server("alice", input).await.unwrap();
    assert!(!server.enabled && !server.auto_connect);
    assert!(manager.list_servers("bob").await.unwrap().is_empty());
    assert!(matches!(
        manager.connect("bob", &server.id).await,
        Err(McpError::ServerNotFound)
    ));
    assert!(matches!(
        manager.connect("alice", &server.id).await,
        Err(McpError::Disabled)
    ));
    assert!(manager
        .create_server("alice", config("local", "normal"))
        .await
        .is_err());
    assert!(manager
        .create_server("bob", config("local", "normal"))
        .await
        .is_ok());
    manager.toggle("alice", &server.id, true).await.unwrap();
    manager
        .update_server(
            "alice",
            &server.id,
            UpdateMcpServerRequest {
                name: Some("renamed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    manager.delete_server("alice", &server.id).await.unwrap();
    assert!(manager.list_servers("alice").await.unwrap().is_empty());
    assert!(manager
        .create_server("alice", config("renamed", "normal"))
        .await
        .is_ok());
}

#[tokio::test]
async fn remote_http_requires_explicit_persisted_consent_and_old_configs_default_safe() {
    let (_, manager) = setup(true).await;
    let mut request = config("remote", "normal");
    request.transport = TransportKind::Http;
    request.stdio = None;
    request.http =
        Some(serde_json::from_value(json!({"url":"http://example.invalid/mcp"})).unwrap());
    assert!(!request.http.as_ref().unwrap().allow_insecure_http);
    assert!(matches!(
        manager.create_server("alice", request.clone()).await,
        Err(McpError::InvalidConfig(_))
    ));
    request.http.as_mut().unwrap().allow_insecure_http = true;
    let server = manager.create_server("alice", request).await.unwrap();
    let saved = manager
        .server_config("alice", &server.id)
        .await
        .unwrap()
        .http
        .unwrap();
    assert!(saved.allow_insecure_http);
    assert_eq!(saved.url, "http://example.invalid/mcp");
    assert_eq!(server.status, "configured");
    assert!(manager
        .list_connection_logs("alice", Some(&server.id), None)
        .await
        .unwrap()
        .is_empty());
    let safe = serde_json::from_value(json!({"url":"https://example.invalid/mcp"})).unwrap();
    manager
        .update_server(
            "alice",
            &server.id,
            UpdateMcpServerRequest {
                http: Some(safe),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        !manager
            .server_config("alice", &server.id)
            .await
            .unwrap()
            .http
            .unwrap()
            .allow_insecure_http
    );
}

#[tokio::test]
async fn output_contracts_are_validated_before_return_and_audited_without_payloads() {
    for (mode, expected) in [
        ("output-valid", "success"),
        ("output-missing", "mcp_protocol_error"),
        ("output-wrong", "mcp_protocol_error"),
        ("malformed-result", "mcp_protocol_error"),
        ("output-error", "mcp_tool_call_failed"),
    ] {
        let (db, manager) = setup(true).await;
        let server = manager
            .create_server("alice", config(mode, mode))
            .await
            .unwrap();
        manager.connect("alice", &server.id).await.unwrap();
        let mut input = call(&server.id);
        input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
        let result = manager.call_tool("alice", input).await.unwrap();
        assert_eq!(result.is_error, expected != "success", "{mode}");
        if expected == "mcp_protocol_error" {
            assert!(!result.content.to_string().contains("private-value"));
            assert!(result.structured_content.is_none());
        }
        let logs = manager
            .list_usage_logs("alice", Some(&server.id), None)
            .await
            .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(
            logs[0].outcome,
            if expected == "success" {
                "success"
            } else {
                "error"
            }
        );
        assert_eq!(
            logs[0].error_class.as_deref(),
            (expected != "success").then_some(expected)
        );
        let rows = dgos_db::entities::mcp_usage_logs::Entity::find()
            .all(&db)
            .await
            .unwrap();
        assert!(!format!("{rows:?}").contains("private-value"));
        manager.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn output_only_schema_refresh_invalidates_pins_and_single_use_consent() {
    let (_, manager) = setup(true).await;
    let server = manager
        .create_server("alice", config("output-change", "output-change"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let before = manager.list_tools("alice", &server.id).await.unwrap();
    let mut input = call(&server.id);
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    let after = manager.refresh_tools("alice", &server.id).await.unwrap();
    assert_eq!(before[0].input_schema, after[0].input_schema);
    assert_ne!(before[0].schema_hash, after[0].schema_hash);
    assert!(matches!(
        manager
            .call_tool_at_schema("alice", input.clone(), &before[0].schema_hash)
            .await,
        Err(McpError::ToolInputInvalid(_))
    ));
    assert!(matches!(
        manager.call_tool("alice", input).await,
        Err(McpError::ConfirmationRequired)
    ));
    let mut fresh = call(&server.id);
    fresh.confirmation_token = Some(manager.confirm_call("alice", &fresh).await.unwrap());
    assert_eq!(
        manager
            .call_tool_at_schema("alice", fresh, &after[0].schema_hash)
            .await
            .unwrap()
            .outcome,
        "success"
    );
    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_output_schema_refresh_preserves_the_previous_snapshot_and_consent() {
    let (_, manager) = setup(true).await;
    let server = manager
        .create_server("alice", config("invalid-refresh", "output-invalid-refresh"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let before =
        serde_json::to_value(manager.list_tools("alice", &server.id).await.unwrap()).unwrap();
    let mut input = call(&server.id);
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    assert!(matches!(
        manager.refresh_tools("alice", &server.id).await,
        Err(McpError::Protocol(_))
    ));
    assert_eq!(
        serde_json::to_value(manager.list_tools("alice", &server.id).await.unwrap()).unwrap(),
        before
    );
    assert_eq!(
        manager.call_tool("alice", input).await.unwrap().outcome,
        "success"
    );
    manager.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn burst_of_128_calls_is_bounded_correlated_and_fully_audited() {
    let (db, manager) = setup(true).await;
    let manager = Arc::new(manager);
    let server = manager
        .create_server("alice", config("pressure", "pressure"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let mut inputs = Vec::new();
    for index in 0..128 {
        let mut input = call(&server.id);
        input.arguments = json!({"text":format!("synthetic-pressure-{index}")});
        input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
        inputs.push(input);
    }
    let barrier = Arc::new(tokio::sync::Barrier::new(129));
    let mut calls = tokio::task::JoinSet::new();
    let started = std::time::Instant::now();
    for input in inputs {
        let manager = manager.clone();
        let barrier = barrier.clone();
        calls.spawn(async move {
            barrier.wait().await;
            let expected = input.arguments["text"].clone();
            let call_id = input.call_id.clone().unwrap();
            let response = manager.call_tool("alice", input).await.unwrap();
            assert_eq!(response.call_id, call_id);
            assert_eq!(response.content[0]["text"], expected);
            assert_eq!(response.outcome, "success");
            assert!(!response.audit_warning);
            assert!(
                response.structured_content.unwrap()["maxPressure"]
                    .as_u64()
                    .unwrap()
                    <= 8
            );
            response.usage_log_id
        });
    }
    barrier.wait().await;
    let mut ids = std::collections::HashSet::new();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while let Some(result) = calls.join_next().await {
            assert!(ids.insert(result.unwrap()));
        }
    })
    .await
    .unwrap();
    let qps = 128.0 / started.elapsed().as_secs_f64();
    eprintln!(
        "MCP synthetic 128-call burst: {qps:.1} QPS (includes audit persistence, 8-call cap)"
    );
    if let Ok(minimum) = std::env::var("DGOS_MCP_PERF_MIN_QPS") {
        assert!(
            qps >= minimum.parse::<f64>().expect("numeric QPS minimum"),
            "measured {qps:.1} QPS"
        );
    }
    let rows = dgos_db::entities::mcp_usage_logs::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(ids.len(), 128);
    assert_eq!(rows.len(), 128);
    assert!(rows.iter().all(|row| row.outcome == "success"
        && row.request_hash.is_some()
        && ids.contains(&row.id)));
    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn tools_discovery_schema_confirmation_and_redacted_audit() {
    let (db, manager) = setup(true).await;
    let server = manager
        .create_server("alice", config("local", "normal"))
        .await
        .unwrap();
    assert_eq!(
        manager.connect("alice", &server.id).await.unwrap().status,
        "ready"
    );
    assert_eq!(
        manager.list_tools("alice", &server.id).await.unwrap().len(),
        2
    );
    let mut input = call(&server.id);
    assert!(matches!(
        manager.call_tool("alice", input.clone()).await,
        Err(McpError::ConfirmationRequired)
    ));
    input.confirmation_token = Some("confirmed".into());
    assert!(matches!(
        manager.call_tool("alice", input.clone()).await,
        Err(McpError::ConfirmationRequired)
    ));
    input.arguments = json!({"text": 1});
    assert!(matches!(
        manager.confirm_call("alice", &input).await,
        Err(McpError::ToolInputInvalid(_))
    ));
    input.arguments = json!({"text": "private-value", "unknown": 1});
    assert!(matches!(
        manager.confirm_call("alice", &input).await,
        Err(McpError::ToolInputInvalid(_))
    ));
    input = call(&server.id);
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    let result = manager.call_tool("alice", input.clone()).await.unwrap();
    assert!(!result.is_error && !result.audit_warning);
    assert_eq!(result.content[0]["text"], "private-value");
    assert!(matches!(
        manager.call_tool("alice", input).await,
        Err(McpError::ConfirmationRequired)
    ));
    let logs = dgos_db::entities::mcp_usage_logs::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(logs.len(), 4);
    assert_eq!(
        logs.iter().filter(|log| log.outcome == "success").count(),
        1
    );
    assert_eq!(logs.iter().filter(|log| log.outcome == "denied").count(), 3);
    assert!(!format!("{logs:?}").contains("private-value"));
    assert_eq!(logs[0].request_hash.as_ref().unwrap().len(), 64);
    assert!(manager
        .list_usage_logs("bob", None, None)
        .await
        .unwrap()
        .is_empty());
    manager.disconnect("alice", &server.id).await.unwrap();
    assert_eq!(
        manager.list_tools("alice", &server.id).await.unwrap().len(),
        2
    );
}

#[tokio::test]
async fn policy_denies_by_default_and_token_is_bound_to_arguments() {
    let (_, denied) = setup(false).await;
    assert!(matches!(
        denied.call_tool("alice", call("no-server")).await,
        Err(McpError::PermissionDenied)
    ));
    let (_, manager) = setup(true).await;
    let server = manager
        .create_server("alice", config("local", "normal"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let mut input = call(&server.id);
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    input.arguments = json!({"text": "changed"});
    assert!(matches!(
        manager.call_tool("alice", input).await,
        Err(McpError::ConfirmationRequired)
    ));
    let mut input = call(&server.id);
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    input.call_id = Some("changed-call-id".into());
    assert!(matches!(
        manager.call_tool("alice", input).await,
        Err(McpError::ConfirmationRequired)
    ));
    let mut input = call(&server.id);
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    input.timeout_ms = Some(120_000);
    assert!(matches!(
        manager.call_tool("alice", input).await,
        Err(McpError::ConfirmationRequired)
    ));
    manager.disconnect("alice", &server.id).await.unwrap();
}

#[tokio::test]
async fn concurrent_calls_match_ids_and_timeout_does_not_poison_later_calls() {
    let (_, manager) = setup(true).await;
    let server = manager
        .create_server("alice", config("local", "normal"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let mut slow = call(&server.id);
    slow.arguments = json!({"text":"slow", "delay":250});
    slow.timeout_ms = Some(100);
    slow.confirmation_token = Some(manager.confirm_call("alice", &slow).await.unwrap());
    let mut fast = call(&server.id);
    fast.arguments = json!({"text":"fast", "delay":10});
    fast.confirmation_token = Some(manager.confirm_call("alice", &fast).await.unwrap());
    let (slow, fast) = tokio::join!(
        manager.call_tool("alice", slow),
        manager.call_tool("alice", fast)
    );
    let slow = slow.unwrap();
    assert!(slow.is_error);
    assert_eq!(slow.outcome, "timeout");
    assert_eq!(fast.unwrap().content[0]["text"], "fast");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let mut later = call(&server.id);
    later.confirmation_token = Some(manager.confirm_call("alice", &later).await.unwrap());
    assert_eq!(
        manager.call_tool("alice", later).await.unwrap().content[0]["text"],
        "private-value"
    );
    manager.disconnect("alice", &server.id).await.unwrap();
}

async fn wait_for_pending(manager: &McpManager, server: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if manager
                .list_usage_logs("alice", Some(server), None)
                .await
                .unwrap()
                .iter()
                .any(|log| log.outcome == "pending")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancelled_stdio_call_keeps_other_calls_alive_and_ignores_late_response() {
    let (_, manager) = setup(true).await;
    let manager = Arc::new(manager);
    let server = manager
        .create_server("alice", config("cancel", "cancellation-observable"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let mut input = call(&server.id);
    input.arguments = json!({"text":"late-response", "delay":400});
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    let call_id = input.call_id.clone().unwrap();
    let running = {
        let manager = manager.clone();
        let input = input.clone();
        tokio::spawn(async move { manager.call_tool("alice", input).await })
    };
    let mut survivor = call(&server.id);
    survivor.arguments = json!({"text":"surviving-call", "delay":600});
    survivor.confirmation_token = Some(manager.confirm_call("alice", &survivor).await.unwrap());
    let survivor = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.call_tool("alice", survivor).await })
    };
    wait_for_pending(&manager, &server.id).await;
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let mut probe = call(&server.id);
            probe.arguments = json!({"text":"fixture-counts"});
            probe.confirmation_token = Some(manager.confirm_call("alice", &probe).await.unwrap());
            let result = manager.call_tool("alice", probe).await.unwrap();
            let counts: serde_json::Value =
                serde_json::from_str(result.content[0]["text"].as_str().unwrap()).unwrap();
            if counts["started"].as_u64().unwrap() >= 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        manager.call_tool("alice", input).await,
        Err(McpError::InvalidConfig(_))
    ));
    assert!(!manager
        .cancel_call("bob", &server.id, &call_id)
        .await
        .unwrap());
    assert!(!manager
        .cancel_call("alice", "another-server", &call_id)
        .await
        .unwrap());
    assert!(manager
        .cancel_call("alice", &server.id, &call_id)
        .await
        .unwrap());
    let result = running.await.unwrap().unwrap();
    assert_eq!(result.outcome, "cancelled");
    assert_eq!(result.call_id, call_id);
    assert!(result.is_error && !result.audit_warning);
    assert!(!manager
        .cancel_call("alice", &server.id, &call_id)
        .await
        .unwrap());
    let survived = survivor.await.unwrap().unwrap();
    assert_eq!(survived.outcome, "success");
    assert_eq!(survived.content[0]["text"], "surviving-call");
    tokio::time::sleep(std::time::Duration::from_millis(450)).await;
    let mut probe = call(&server.id);
    probe.arguments = json!({"text":"fixture-counts"});
    probe.confirmation_token = Some(manager.confirm_call("alice", &probe).await.unwrap());
    let result = manager.call_tool("alice", probe).await.unwrap();
    let counts: serde_json::Value =
        serde_json::from_str(result.content[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(counts["cancellations"], 1);
    assert_eq!(counts["started"], 2);
    let logs = manager
        .list_usage_logs("alice", Some(&server.id), None)
        .await
        .unwrap();
    assert_eq!(
        logs.iter().filter(|log| log.outcome == "cancelled").count(),
        1
    );
    assert!(!format!("{logs:?}").contains("late-response"));
    manager.disconnect("alice", &server.id).await.unwrap();
}

#[tokio::test]
async fn rejected_calls_are_audited_only_for_owned_servers_without_arguments() {
    let (_, manager) = setup(false).await;
    let own = manager
        .create_server("alice", config("own", "normal"))
        .await
        .unwrap();
    let foreign = manager
        .create_server("bob", config("foreign", "normal"))
        .await
        .unwrap();
    assert!(matches!(
        manager.call_tool("alice", call(&own.id)).await,
        Err(McpError::PermissionDenied)
    ));
    assert!(matches!(
        manager.call_tool("alice", call(&foreign.id)).await,
        Err(McpError::PermissionDenied)
    ));
    let logs = manager.list_usage_logs("alice", None, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].outcome, "denied");
    assert_eq!(logs[0].server_id, own.id);
    assert!(!format!("{logs:?}").contains("private-value"));
    assert!(manager
        .list_usage_logs("bob", None, None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn abandoned_pending_call_recovers_as_interrupted_without_replaying_or_rewriting_terminal_log(
) {
    let (db, manager) = setup(true).await;
    let manager = Arc::new(manager);
    let server = manager
        .create_server("alice", config("interrupted", "normal"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let mut complete = call(&server.id);
    complete.confirmation_token = Some(manager.confirm_call("alice", &complete).await.unwrap());
    let completed = manager.call_tool("alice", complete).await.unwrap();
    let mut input = call(&server.id);
    input.arguments = json!({"text":"interrupted-value", "delay":5000});
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    let call_id = input.call_id.clone().unwrap();
    let running = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.call_tool("alice", input).await })
    };
    wait_for_pending(&manager, &server.id).await;
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    assert!(!manager
        .cancel_call("alice", &server.id, &call_id)
        .await
        .unwrap());
    manager.disconnect("alice", &server.id).await.unwrap();
    let restored = McpManager::new(db);
    restored.recover_runtime_status().await.unwrap();
    restored.recover_runtime_status().await.unwrap();
    let logs = restored
        .list_usage_logs("alice", Some(&server.id), None)
        .await
        .unwrap();
    assert_eq!(
        logs.iter()
            .find(|log| log.id == completed.usage_log_id)
            .unwrap()
            .outcome,
        "success"
    );
    assert_eq!(
        logs.iter()
            .filter(|log| log.outcome == "interrupted")
            .count(),
        1
    );
    assert_eq!(
        logs.iter()
            .find(|log| log.outcome == "interrupted")
            .unwrap()
            .error_class
            .as_deref(),
        Some("mcp_interrupted")
    );
    assert!(!format!("{logs:?}").contains("interrupted-value"));
    assert_eq!(
        restored.list_servers("alice").await.unwrap()[0].status,
        "configured"
    );
}

#[tokio::test]
async fn final_audit_failure_reports_executed_result_and_retains_unknown_pending_intent() {
    let (db, manager) = setup(true).await;
    let server = manager
        .create_server("alice", config("audit-failure", "normal"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    db.execute_unprepared("CREATE TRIGGER fail_mcp_audit_update BEFORE UPDATE ON mcp_usage_logs BEGIN SELECT RAISE(ABORT, 'synthetic audit update failure'); END;").await.unwrap();
    let mut input = call(&server.id);
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    let result = manager.call_tool("alice", input).await.unwrap();
    assert!(!result.is_error && result.audit_warning);
    assert_eq!(result.outcome, "success");
    assert_eq!(result.content[0]["text"], "private-value");
    assert_eq!(
        manager.list_usage_logs("alice", None, None).await.unwrap()[0].outcome,
        "pending"
    );
    db.execute_unprepared("DROP TRIGGER fail_mcp_audit_update")
        .await
        .unwrap();
    manager.disconnect("alice", &server.id).await.unwrap();
    let restored = McpManager::new(db);
    restored.recover_runtime_status().await.unwrap();
    assert_eq!(
        restored.list_usage_logs("alice", None, None).await.unwrap()[0].outcome,
        "interrupted"
    );
}

#[tokio::test]
async fn connection_rejects_process_exit_malformed_protocol_and_pagination() {
    let (_, manager) = setup(true).await;
    for mode in [
        "exit",
        "malformed",
        "incompatible",
        "pagination-cycle",
        "bad-schema",
        "bad-output-schema",
        "missing-capabilities",
        "invalid-tools-capability",
        "missing-server-info",
    ] {
        let server = manager
            .create_server("alice", config(mode, mode))
            .await
            .unwrap();
        assert!(
            manager.connect("alice", &server.id).await.is_err(),
            "{mode}"
        );
        assert_eq!(
            manager
                .list_servers("alice")
                .await
                .unwrap()
                .iter()
                .find(|s| s.id == server.id)
                .unwrap()
                .status,
            "error"
        );
        assert!(manager
            .list_tools("alice", &server.id)
            .await
            .unwrap()
            .is_empty());
    }
}

#[tokio::test]
async fn handshake_negotiates_supported_versions_and_respects_no_tools_capability() {
    let (_, manager) = setup(true).await;
    for mode in [
        "version-2024-11-05",
        "version-2025-03-26",
        "version-2025-06-18",
        "no-tools",
        "server-requests",
    ] {
        let server = manager
            .create_server("alice", config(mode, mode))
            .await
            .unwrap();
        assert_eq!(
            manager.connect("alice", &server.id).await.unwrap().status,
            "ready",
            "{mode}"
        );
        let tools = manager.refresh_tools("alice", &server.id).await.unwrap();
        assert_eq!(
            tools.len(),
            if mode == "no-tools" { 0 } else { 2 },
            "{mode}"
        );
        if mode == "no-tools" {
            assert!(matches!(
                manager.confirm_call("alice", &call(&server.id)).await,
                Err(McpError::ToolNotFound)
            ));
        }
        manager.disconnect("alice", &server.id).await.unwrap();
    }
}

#[tokio::test]
async fn config_validation_rejects_shell_url_spoof_and_transport_mismatch() {
    let (_, manager) = setup(true).await;
    let mut shell = config("shell", "normal");
    shell.stdio.as_mut().unwrap().executable = "/bin/sh".into();
    assert!(manager.create_server("alice", shell).await.is_err());
    for url in [
        "http://localhost.evil.test/mcp",
        "http://127.0.0.1.evil.test/mcp",
        "https://secret@example.com/mcp",
        "ftp://localhost/mcp",
    ] {
        let input = CreateMcpServerRequest {
            name: "http".into(),
            transport: TransportKind::Http,
            stdio: None,
            http: Some(HttpConfig {
                url: url.into(),
                credential_ref: None,
                allow_insecure_http: false,
            }),
            enabled: None,
            auto_connect: None,
        };
        assert!(
            manager.create_server("alice", input).await.is_err(),
            "{url}"
        );
    }
    let server = manager
        .create_server("alice", config("local", "normal"))
        .await
        .unwrap();
    assert!(manager
        .update_server(
            "alice",
            &server.id,
            UpdateMcpServerRequest {
                transport: Some(TransportKind::Http),
                ..Default::default()
            }
        )
        .await
        .is_err());
}

#[tokio::test]
async fn invalid_update_does_not_disconnect_or_replace_existing_snapshot() {
    let (_, manager) = setup(true).await;
    let server = manager
        .create_server("alice", config("local", "normal"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    assert!(manager
        .update_server(
            "alice",
            &server.id,
            UpdateMcpServerRequest {
                name: Some(String::new()),
                ..Default::default()
            }
        )
        .await
        .is_err());
    assert_eq!(
        manager.list_servers("alice").await.unwrap()[0].status,
        "ready"
    );
    assert_eq!(
        manager.list_tools("alice", &server.id).await.unwrap().len(),
        2
    );
    let mut input = call(&server.id);
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    assert!(!manager.call_tool("alice", input).await.unwrap().is_error);
    manager.disconnect("alice", &server.id).await.unwrap();
}

#[tokio::test]
async fn disabling_waits_for_inflight_call_then_revokes_connections_and_consent() {
    let (_, manager) = setup(true).await;
    let server = manager
        .create_server("alice", config("local", "normal"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let mut input = call(&server.id);
    input.arguments = json!({"text":"inflight", "delay":150});
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    let disable = async {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        manager.toggle("alice", &server.id, false).await.unwrap()
    };
    let (response, disabled) = tokio::join!(manager.call_tool("alice", input), disable);
    assert_eq!(response.unwrap().content[0]["text"], "inflight");
    assert!(!disabled.enabled);
    assert!(matches!(
        manager.call_tool("alice", call(&server.id)).await,
        Err(McpError::Disabled)
    ));
}

#[tokio::test]
async fn tool_reported_errors_are_audited_as_errors() {
    let (_, manager) = setup(true).await;
    let server = manager
        .create_server("alice", config("local", "normal"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let mut input = call(&server.id);
    input.arguments = json!({"text":"failure", "fail":true});
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    assert!(manager.call_tool("alice", input).await.unwrap().is_error);
    let logs = manager
        .list_usage_logs("alice", Some(&server.id), None)
        .await
        .unwrap();
    assert_eq!(logs[0].outcome, "error");
    assert_eq!(logs[0].error_class.as_deref(), Some("mcp_tool_call_failed"));
    manager.disconnect("alice", &server.id).await.unwrap();
}

#[tokio::test]
async fn credential_refs_are_separate_from_config_and_fail_closed_without_vault() {
    let (db, manager) = setup(true).await;
    let server = manager
        .create_server(
            "alice",
            CreateMcpServerRequest {
                name: "authenticated".into(),
                transport: TransportKind::Http,
                stdio: None,
                http: Some(HttpConfig {
                    url: "https://example.com/mcp".into(),
                    credential_ref: Some("vault:token".into()),
                    allow_insecure_http: false,
                }),
                enabled: Some(true),
                auto_connect: None,
            },
        )
        .await
        .unwrap();
    let row = dgos_db::entities::mcp_servers::Entity::find_by_id(&server.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(!row.config_json.contains("vault:token"));
    assert_eq!(row.credential_ref.as_deref(), Some("vault:token"));
    assert!(matches!(
        manager.connect("alice", &server.id).await,
        Err(McpError::CredentialUnavailable)
    ));
}

#[tokio::test]
async fn refresh_uses_same_session_replaces_snapshot_and_revokes_only_its_consent() {
    let (_, manager) = setup(true).await;
    let server = manager
        .create_server("alice", config("refresh", "refresh-change"))
        .await
        .unwrap();
    let other = manager
        .create_server("alice", config("other", "normal"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    manager.connect("alice", &other.id).await.unwrap();
    let mut stale = call(&server.id);
    stale.confirmation_token = Some(manager.confirm_call("alice", &stale).await.unwrap());
    let mut unchanged = call(&other.id);
    unchanged.confirmation_token = Some(manager.confirm_call("alice", &unchanged).await.unwrap());

    let tools = manager.refresh_tools("alice", &server.id).await.unwrap();
    // The fixture changes its second page only on a second discovery within
    // the same process; reconnecting would incorrectly return echo_second.
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.tool_name.as_str())
            .collect::<Vec<_>>(),
        vec!["echo", "echo_changed"]
    );
    assert!(matches!(
        manager.call_tool("alice", stale).await,
        Err(McpError::ConfirmationRequired)
    ));
    assert!(
        !manager
            .call_tool("alice", unchanged)
            .await
            .unwrap()
            .is_error
    );
    let mut fresh = call(&server.id);
    fresh.confirmation_token = Some(manager.confirm_call("alice", &fresh).await.unwrap());
    assert!(!manager.call_tool("alice", fresh).await.unwrap().is_error);
    assert_eq!(
        manager
            .list_servers("alice")
            .await
            .unwrap()
            .iter()
            .find(|row| row.id == server.id)
            .unwrap()
            .status,
        "ready"
    );
    manager.disconnect("alice", &server.id).await.unwrap();
    manager.disconnect("alice", &other.id).await.unwrap();
    assert!(matches!(
        manager.refresh_tools("alice", &server.id).await,
        Err(McpError::NotConnected)
    ));
    assert!(matches!(
        manager.refresh_tools("bob", &server.id).await,
        Err(McpError::ServerNotFound)
    ));
}

#[tokio::test]
async fn failed_refresh_preserves_entire_snapshot_and_live_session() {
    let (_, manager) = setup(true).await;
    let server = manager
        .create_server("alice", config("refresh", "refresh-failure"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let before =
        serde_json::to_value(manager.list_tools("alice", &server.id).await.unwrap()).unwrap();
    let mut input = call(&server.id);
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    // A valid first page followed by an RPC error must not publish half a list.
    assert!(manager.refresh_tools("alice", &server.id).await.is_err());
    assert_eq!(
        serde_json::to_value(manager.list_tools("alice", &server.id).await.unwrap()).unwrap(),
        before
    );
    assert!(!manager.call_tool("alice", input).await.unwrap().is_error);
    let refreshed = manager.refresh_tools("alice", &server.id).await.unwrap();
    assert!(refreshed
        .iter()
        .any(|tool| tool.tool_name == "echo_replacement"));
    manager.disconnect("alice", &server.id).await.unwrap();
}

#[tokio::test]
async fn five_simultaneous_calls_complete_with_reverse_order_responses() {
    let (_, manager) = setup(true).await;
    let manager = Arc::new(manager);
    let server = manager
        .create_server("alice", config("parallel", "five-barrier"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let mut inputs = Vec::new();
    for index in 0..5 {
        let mut input = call(&server.id);
        input.arguments = json!({"text": format!("parallel-{index}")});
        input.timeout_ms = Some(3000);
        input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
        inputs.push(input);
    }
    let mut calls = tokio::task::JoinSet::new();
    for input in inputs {
        let manager = manager.clone();
        calls.spawn(async move {
            let expected = input.arguments["text"].clone();
            let response = manager.call_tool("alice", input).await.unwrap();
            assert!(!response.is_error && !response.audit_warning);
            assert_eq!(response.content[0]["text"], expected);
        });
    }
    // The fixture sends no response until all five calls have arrived.
    while let Some(result) = calls.join_next().await {
        result.unwrap();
    }
    let logs = manager
        .list_usage_logs("alice", Some(&server.id), None)
        .await
        .unwrap();
    assert_eq!(logs.len(), 5);
    assert!(logs.iter().all(|log| log.outcome == "success"));
    manager.disconnect("alice", &server.id).await.unwrap();
}

#[tokio::test]
async fn connection_events_are_persisted_isolated_bounded_and_redacted() {
    let (db, manager) = setup(true).await;
    let server = manager
        .create_server("alice", config("private-server-name", "normal"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    manager.disconnect("alice", &server.id).await.unwrap();
    let logs = manager
        .list_connection_logs("alice", Some(&server.id), None)
        .await
        .unwrap();
    assert_eq!(logs.len(), 2);
    assert!(logs
        .iter()
        .any(|log| log.event == "connected" && log.status == "ready" && log.duration_ms.is_some()));
    assert!(logs
        .iter()
        .any(|log| log.event == "disconnected" && log.status == "configured"));
    assert!(!format!("{logs:?}").contains("mcp-server.mjs"));
    assert!(!format!("{logs:?}").contains("private-server-name"));
    assert_eq!(
        manager
            .list_connection_logs("alice", None, Some(1))
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(manager
        .list_connection_logs("bob", Some(&server.id), None)
        .await
        .unwrap()
        .is_empty());

    let restored = McpManager::new(db);
    assert_eq!(
        restored
            .list_connection_logs("alice", None, None)
            .await
            .unwrap()
            .len(),
        2
    );
    let broken = restored
        .create_server("alice", config("broken", "incompatible"))
        .await
        .unwrap();
    assert!(restored.connect("alice", &broken.id).await.is_err());
    let logs = restored
        .list_connection_logs("alice", Some(&broken.id), None)
        .await
        .unwrap();
    assert_eq!(logs[0].event, "connect_failed");
    assert!(logs[0].error_class.is_some());
}

#[tokio::test]
async fn snapshot_database_failure_rolls_back_without_stuck_connecting_status() {
    let (db, manager) = setup(true).await;
    let server = manager
        .create_server("alice", config("refresh", "refresh-change"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let before =
        serde_json::to_value(manager.list_tools("alice", &server.id).await.unwrap()).unwrap();
    db.execute_unprepared("CREATE TRIGGER fail_mcp_snapshot BEFORE INSERT ON mcp_server_tools BEGIN SELECT RAISE(ABORT, 'synthetic DB failure'); END;").await.unwrap();
    assert!(matches!(
        manager.refresh_tools("alice", &server.id).await,
        Err(McpError::Database(_))
    ));
    assert_eq!(
        serde_json::to_value(manager.list_tools("alice", &server.id).await.unwrap()).unwrap(),
        before
    );
    assert!(matches!(
        manager.connect("alice", &server.id).await,
        Err(McpError::Database(_))
    ));
    assert_eq!(
        manager.list_servers("alice").await.unwrap()[0].status,
        "degraded"
    );
    assert_eq!(
        serde_json::to_value(manager.list_tools("alice", &server.id).await.unwrap()).unwrap(),
        before
    );
    db.execute_unprepared("DROP TRIGGER fail_mcp_snapshot")
        .await
        .unwrap();
    let mut input = call(&server.id);
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    assert!(!manager.call_tool("alice", input).await.unwrap().is_error);
    manager.disconnect("alice", &server.id).await.unwrap();
}

#[tokio::test]
async fn shutdown_cancels_dispatched_calls_drains_audit_and_refuses_new_work() {
    let (_, manager) = setup(true).await;
    let manager = Arc::new(manager);
    let server = manager
        .create_server("alice", config("shutdown", "normal"))
        .await
        .unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let mut input = call(&server.id);
    input.arguments = json!({"text":"synthetic-shutdown", "delay":10_000});
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    let unused = manager
        .confirm_call("alice", &call(&server.id))
        .await
        .unwrap();
    let task_manager = manager.clone();
    let task = tokio::spawn(async move { task_manager.call_tool("alice", input).await.unwrap() });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if manager
                .list_usage_logs("alice", Some(&server.id), None)
                .await
                .unwrap()
                .iter()
                .any(|row| row.outcome == "pending")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    manager.clone().spawn_monitor();
    tokio::time::timeout(std::time::Duration::from_secs(3), manager.shutdown())
        .await
        .unwrap()
        .unwrap();
    let response = task.await.unwrap();
    assert_eq!(response.outcome, "cancelled");
    assert!(!response.audit_warning);
    let logs = manager
        .list_usage_logs("alice", Some(&server.id), None)
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].outcome, "cancelled");
    assert_eq!(
        manager.list_servers("alice").await.unwrap()[0].status,
        "configured"
    );
    assert!(matches!(
        manager.connect("alice", &server.id).await,
        Err(McpError::Cancelled)
    ));
    assert!(matches!(
        manager.refresh_tools("alice", &server.id).await,
        Err(McpError::Cancelled)
    ));
    assert!(matches!(
        manager.confirm_call("alice", &call(&server.id)).await,
        Err(McpError::Cancelled)
    ));
    let mut old_consent = call(&server.id);
    old_consent.confirmation_token = Some(unused);
    assert!(matches!(
        manager.call_tool("alice", old_consent).await,
        Err(McpError::Cancelled)
    ));
    let events = manager
        .list_connection_logs("alice", Some(&server.id), None)
        .await
        .unwrap();
    assert!(events
        .iter()
        .any(|row| row.event == "disconnected" && row.status == "configured"));
    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_interrupts_a_handshake_without_publishing_a_late_connection() {
    let (_, manager) = setup(true).await;
    let manager = Arc::new(manager);
    let server = manager
        .create_server("alice", config("shutdown-handshake", "silent"))
        .await
        .unwrap();
    let connecting = manager.clone();
    let id = server.id.clone();
    let task = tokio::spawn(async move { connecting.connect("alice", &id).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while manager.list_servers("alice").await.unwrap()[0].status != "connecting" {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), manager.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(task.await.unwrap(), Err(McpError::Cancelled)));
    assert_eq!(
        manager.list_servers("alice").await.unwrap()[0].status,
        "configured"
    );
    assert!(manager
        .list_tools("alice", &server.id)
        .await
        .unwrap()
        .is_empty());
}
