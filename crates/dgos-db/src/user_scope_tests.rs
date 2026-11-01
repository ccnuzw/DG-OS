//! V1-FR-008 data side (D087, D090): every user-scoped table has a purge
//! entry, a purge removes exactly one user's rows (children included) and is
//! idempotent, the users and host settings repositories, and the MCP
//! credential outbox. Real migrated SQLite with `secure_delete`.

use crate::migrations::Migrator;
use crate::{
    HostSettingsRepository, McpCredentialCleanup, McpCredentialCleanupRepository, NewUser,
    UserDataDomain, UserDataRepository, UserRepository, LAST_SIGN_IN_USER,
    RETAINED_FOR_VAULT_RETRY,
};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::MigratorTrait;
use std::collections::BTreeSet;

async fn database() -> (tempfile::TempDir, DatabaseConnection) {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("dgos.db").display());
    let db = crate::establish_connection(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    (dir, db)
}

async fn count(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one(Statement::from_string(DbBackend::Sqlite, sql))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index(0)
        .unwrap()
}

/// One row per user-scoped table and per child table that hangs off one,
/// for `user`. `n` keeps primary keys apart.
fn seed(user: &str, n: u32) -> String {
    format!(
        "
        INSERT INTO providers (id, name, provider_type, base_url, api_key, created_at, updated_at, user_id, credential_ref)
          VALUES ('prov-{n}', 'p', 'openai', 'https://p{n}.example/v1', '', 't', 't', '{user}', 'pref-{n}');
        INSERT INTO models (id, provider_id, model_id, model_name, capabilities, synced_at)
          VALUES ('model-{n}', 'prov-{n}', 'm', 'm', '{{}}', 't');
        INSERT INTO provider_usage_logs (id, provider_id, model_id, user_id, success, created_at)
          VALUES ('pul-{n}', 'prov-{n}', 'm', '{user}', 1, 't');
        INSERT INTO usage_logs (id, provider_id, model_id, user_id, success, created_at)
          VALUES ('ul-{n}', 'prov-{n}', 'm', '{user}', 1, 't');
        INSERT INTO mcp_servers (id, user_id, name, transport, config_json, credential_ref, created_at, updated_at)
          VALUES ('srv-{n}', '{user}', 's', 'http', '{{\"url\":\"https://m{n}.example/mcp\"}}', 'mref-{n}', 1, 1);
        INSERT INTO mcp_servers (id, user_id, name, transport, config_json, credential_ref, created_at, updated_at, deleted_at)
          VALUES ('srv-old-{n}', '{user}', 'old', 'http', '{{\"url\":\"https://o{n}.example/mcp\"}}', 'mref-old-{n}', 1, 1, 2);
        INSERT INTO mcp_server_tools (server_id, tool_name, input_schema_json, schema_hash, discovered_at)
          VALUES ('srv-{n}', 't', '{{}}', 'h', 1);
        INSERT INTO mcp_usage_logs (user_id, server_id, tool_name, outcome, created_at)
          VALUES ('{user}', 'srv-{n}', 't', 'ok', 1);
        INSERT INTO mcp_connection_logs (user_id, server_id, event, status, created_at)
          VALUES ('{user}', 'srv-{n}', 'connected', 'ready', 1);
        INSERT INTO workbench_tasks (id, user_id, app_id, provider_id, model_id, status, created_at)
          VALUES ('task-{n}', '{user}', 'a', 'prov-{n}', 'm', 'succeeded', 1);
        INSERT INTO workbench_task_skills (task_id, version_id, skill_id, content_hash)
          VALUES ('task-{n}', 'ver-1', 'skill-1', 'h');
        INSERT INTO workbench_task_tools (task_id, server_id, tool_name, schema_hash)
          VALUES ('task-{n}', 'srv-{n}', 't', 'h');
        INSERT INTO chat_sessions (id, user_id, title, created_at, updated_at)
          VALUES ('chat-{n}', '{user}', 't', 1, 1);
        INSERT INTO chat_messages (id, session_id, user_id, seq, role, status, task_id, created_at, updated_at)
          VALUES ('msg-{n}', 'chat-{n}', '{user}', 1, 'user', 'completed', 'task-{n}', 1, 1);
        INSERT INTO skills (id, skill_id, name, version, install_path, call_type, manifest, installed_at, updated_at, user_id)
          VALUES ('legacy-{n}', 'skill-1', 's', '1', '/x', 'cli', '{{}}', 1, 1, '{user}');
        INSERT INTO skill_user_states (user_id, skill_id, version_id, enabled, created_at)
          VALUES ('{user}', 'skill-1', 'ver-1', 1, 1);
        INSERT INTO skill_usage_logs (id, skill_id, user_id, input_params, success, latency_ms, created_at)
          VALUES ('sul-{n}', 'skill-1', '{user}', '{{}}', 1, 1, 1);
        INSERT INTO module_permissions (id, user_id, module_id, permission, granted)
          VALUES ('perm-{n}', '{user}', 'mod-1', 'storage.read', 1);
        INSERT INTO module_permission_audits (id, user_id, module_id, permission, action, created_at)
          VALUES ('audit-{n}', '{user}', 'mod-1', 'storage.read', 'granted', 1);
        INSERT INTO module_storage (user_id, module_id, key, value, size_bytes, updated_at)
          VALUES ('{user}', 'mod-1', 'k', '1', 1, 1);
        INSERT INTO module_crash_logs (id, user_id, module_id, module_version, runtime_id, source, error_name, stack, created_at)
          VALUES ('crash-{n}', '{user}', 'mod-1', '1.0.0', 'rt', 'runtime', 'E', '', 1);
        INSERT INTO system_settings (user_id, setting_key, setting_value, updated_at)
          VALUES ('{user}', 'appearance.theme', '\"dark\"', 1);
        INSERT INTO lan_audit_events (id, first_at, last_at, event, reason, user_id)
          VALUES ('lan-{n}', 1, 1, 'session_revoked', 'logout', '{user}');
        "
    )
}

const GLOBAL: &str = "
    INSERT INTO modules (id, module_name, display_name, current_version, manifest, integrity, installed_at, updated_at)
      VALUES ('mod-1', 'mod-1', 'M', '1.0.0', '{}', 'aaaa', 1, 1);
    INSERT INTO skill_versions (id, skill_id, version, content_hash, install_path, verification_status, file_manifest, installed_at, metadata)
      VALUES ('ver-1', 'skill-1', '1', 'h', '/x', 'verified', '[]', 1, '{}');
    INSERT INTO lan_audit_events (id, first_at, last_at, event, reason, peer_ip)
      VALUES ('lan-anon', 1, 1, 'request_rejected', 'host_mismatch', '192.168.1.9');
";

/// Rows of `user` in every user-scoped table plus the child tables.
async fn rows_of(db: &DatabaseConnection, user: &str, n: u32) -> Vec<(String, i64)> {
    let mut out = Vec::new();
    for table in db_user_tables(db).await {
        out.push((
            table.clone(),
            count(
                db,
                &format!("SELECT COUNT(*) FROM {table} WHERE user_id = '{user}'"),
            )
            .await,
        ));
    }
    for (table, column, id) in [
        ("models", "provider_id", format!("prov-{n}")),
        ("mcp_server_tools", "server_id", format!("srv-{n}")),
        ("workbench_task_skills", "task_id", format!("task-{n}")),
        ("workbench_task_tools", "task_id", format!("task-{n}")),
    ] {
        out.push((
            table.to_owned(),
            count(
                db,
                &format!("SELECT COUNT(*) FROM {table} WHERE {column} = '{id}'"),
            )
            .await,
        ));
    }
    out
}

async fn db_user_tables(db: &DatabaseConnection) -> Vec<String> {
    UserDataRepository::new(db.clone())
        .user_scoped_tables()
        .await
        .unwrap()
}

/// D087/D090 regression guard: a migration that adds a `user_id` table
/// without a purge entry (or a retained-outbox entry) fails here.
#[tokio::test]
async fn every_user_scoped_table_has_exactly_one_purge_entry() {
    let (_dir, db) = database().await;
    let in_schema: BTreeSet<String> = db_user_tables(&db).await.into_iter().collect();
    let mut registered = BTreeSet::new();
    for domain in UserDataDomain::ORDER {
        for table in domain.user_tables() {
            assert!(
                registered.insert(table.to_string()),
                "{table} is registered twice"
            );
        }
    }
    for table in RETAINED_FOR_VAULT_RETRY {
        assert!(registered.insert(table.to_string()), "{table} twice");
    }
    let missing: Vec<_> = in_schema.difference(&registered).collect();
    let stale: Vec<_> = registered.difference(&in_schema).collect();
    assert!(
        missing.is_empty(),
        "user_id tables without a purge entry (add them to UserDataDomain): {missing:?}"
    );
    assert!(
        stale.is_empty(),
        "registered tables not in the schema: {stale:?}"
    );
    assert!(!in_schema.contains("users"), "users has no user_id column");
}

#[tokio::test]
async fn purging_a_user_removes_only_that_users_rows_and_is_idempotent() {
    let (_dir, db) = database().await;
    db.execute_unprepared(GLOBAL).await.unwrap();
    db.execute_unprepared(&seed("alice", 1)).await.unwrap();
    db.execute_unprepared(&seed("bob", 2)).await.unwrap();
    let data = UserDataRepository::new(db.clone());

    for (table, rows) in rows_of(&db, "alice", 1).await {
        if !RETAINED_FOR_VAULT_RETRY.contains(&table.as_str()) {
            assert!(rows > 0, "fixture seeds {table}");
        }
    }
    let bob_before = rows_of(&db, "bob", 2).await;
    for domain in UserDataDomain::ORDER {
        data.purge(domain, "alice").await.unwrap();
    }
    for (table, rows) in rows_of(&db, "alice", 1).await {
        assert_eq!(rows, 0, "{table} still has alice's rows");
    }
    assert_eq!(rows_of(&db, "bob", 2).await, bob_before);
    // Global rows stay.
    assert_eq!(count(&db, "SELECT COUNT(*) FROM modules").await, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM skill_versions").await, 1);
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM lan_audit_events WHERE user_id IS NULL"
        )
        .await,
        1,
        "unauthenticated LAN audit rows are not any user's data"
    );
    // A resumed deletion repeats every purge harmlessly.
    for domain in UserDataDomain::ORDER {
        assert_eq!(data.purge(domain, "alice").await.unwrap(), 0);
    }
}

#[tokio::test]
async fn chat_purge_waits_for_a_running_generation() {
    let (_dir, db) = database().await;
    db.execute_unprepared(
        "INSERT INTO chat_sessions (id, user_id, title, created_at, updated_at) VALUES ('c', 'alice', 't', 1, 1);
         INSERT INTO chat_messages (id, session_id, user_id, seq, role, status, created_at, updated_at)
           VALUES ('m', 'c', 'alice', 1, 'assistant', 'generating', 1, 1);",
    )
    .await
    .unwrap();
    let data = UserDataRepository::new(db.clone());
    assert!(data.purge(UserDataDomain::Chat, "alice").await.is_err());
    assert_eq!(count(&db, "SELECT COUNT(*) FROM chat_messages").await, 1);
}

#[tokio::test]
async fn marking_a_deletion_lists_every_vault_item_of_the_user_once() {
    let (_dir, db) = database().await;
    db.execute_unprepared(GLOBAL).await.unwrap();
    db.execute_unprepared(&seed("alice", 1)).await.unwrap();
    db.execute_unprepared(&seed("bob", 2)).await.unwrap();
    let users = UserRepository::new(db.clone());
    for (id, name) in [("alice", "alice"), ("bob", "bob")] {
        users
            .insert(NewUser {
                id,
                username: name,
                username_key: name,
                password_hash: "$argon2id$x",
                now: 1,
            })
            .await
            .unwrap();
    }
    users.mark_deleting("alice", 5).await.unwrap();
    users.mark_deleting("alice", 9).await.unwrap();
    assert_eq!(
        count(&db, "SELECT deleting_since FROM users WHERE id = 'alice'").await,
        5,
        "the first marker wins"
    );
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM provider_credential_cleanup WHERE user_id = 'alice' AND endpoint = 'https://p1.example/v1'").await,
        1
    );
    let mcp = McpCredentialCleanupRepository::new(db.clone())
        .pending()
        .await
        .unwrap();
    let mut refs: Vec<_> = mcp
        .iter()
        .map(|entry| entry.credential_ref.as_str())
        .collect();
    refs.sort();
    assert_eq!(
        refs,
        ["mref-1", "mref-old-1"],
        "soft-deleted servers included"
    );
    assert!(mcp.iter().all(|entry| entry.user_id == "alice"));
    assert!(mcp
        .iter()
        .any(|entry| entry.endpoint == "https://m1.example/mcp" && entry.server_id == "srv-1"));
    assert_eq!(users.deleting_ids().await.unwrap(), ["alice"]);
    assert!(users.find_active_by_key("alice").await.unwrap().is_none());
    assert_eq!(users.list_active().await.unwrap().len(), 1);
    assert_eq!(
        UserDataRepository::new(db.clone())
            .pending_credential_cleanup("alice")
            .await
            .unwrap(),
        3
    );
}

#[tokio::test]
async fn users_are_unique_by_folded_name_and_removal_clears_only_their_pointer() {
    let (_dir, db) = database().await;
    let users = UserRepository::new(db.clone());
    let settings = HostSettingsRepository::new(db.clone());
    let new = |id, username, key| NewUser {
        id,
        username,
        username_key: key,
        password_hash: "$argon2id$x",
        now: 1,
    };
    users
        .insert(new("default-user", "Admin", "admin"))
        .await
        .unwrap();
    assert_eq!(
        users
            .insert(new("u2", "ADMIN", "admin"))
            .await
            .unwrap_err()
            .error_code(),
        1008001
    );
    assert!(users.default_user_taken().await.unwrap());
    let record = users.find_active_by_key("admin").await.unwrap().unwrap();
    assert_eq!(record.username, "Admin");
    assert!(
        !format!("{record:?}").contains("argon2"),
        "Debug hides the hash"
    );

    users.insert(new("u2", "bob", "bob")).await.unwrap();
    settings.set(LAST_SIGN_IN_USER, "u2", 1).await.unwrap();
    settings.set("lan.port", "8080", 1).await.unwrap();
    users.remove("default-user").await.unwrap();
    assert_eq!(
        settings.get(LAST_SIGN_IN_USER).await.unwrap().as_deref(),
        Some("u2")
    );
    users.remove("u2").await.unwrap();
    assert_eq!(settings.get(LAST_SIGN_IN_USER).await.unwrap(), None);
    assert_eq!(
        settings.get("lan.port").await.unwrap().as_deref(),
        Some("8080")
    );
    assert_eq!(users.count_all().await.unwrap(), 0);
}

#[tokio::test]
async fn mcp_outbox_is_idempotent_and_tracks_live_references() {
    let (_dir, db) = database().await;
    let outbox = McpCredentialCleanupRepository::new(db.clone());
    let entry = McpCredentialCleanup {
        user_id: "alice".into(),
        server_id: "srv".into(),
        endpoint: "https://m.example/mcp".into(),
        credential_ref: "ref".into(),
    };
    outbox.stage(&entry).await.unwrap();
    outbox.stage(&entry).await.unwrap();
    assert_eq!(outbox.pending().await.unwrap(), [entry.clone()]);
    assert!(!outbox.is_active("ref").await.unwrap());
    db.execute_unprepared(
        "INSERT INTO mcp_servers (id, user_id, name, transport, config_json, credential_ref, created_at, updated_at)
         VALUES ('srv', 'alice', 's', 'http', '{}', 'ref', 1, 1)",
    )
    .await
    .unwrap();
    assert!(outbox.is_active("ref").await.unwrap());
    outbox.acknowledge("ref").await.unwrap();
    outbox.acknowledge("ref").await.unwrap();
    assert!(outbox.pending().await.unwrap().is_empty());
}

#[tokio::test]
async fn orphan_rows_are_counted_only_once_accounts_exist() {
    let (_dir, db) = database().await;
    db.execute_unprepared(
        "INSERT INTO system_settings (user_id, setting_key, setting_value, updated_at) VALUES ('default-user', 'k', 'v', 1)",
    )
    .await
    .unwrap();
    let data = UserDataRepository::new(db.clone());
    assert_eq!(
        data.orphan_count().await.unwrap(),
        0,
        "no accounts: default-user owns it"
    );
    UserRepository::new(db.clone())
        .insert(NewUser {
            id: "u2",
            username: "bob",
            username_key: "bob",
            password_hash: "$argon2id$x",
            now: 1,
        })
        .await
        .unwrap();
    assert_eq!(data.orphan_count().await.unwrap(), 1);
}
