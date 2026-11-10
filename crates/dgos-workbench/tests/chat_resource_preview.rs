use base64::{engine::general_purpose::STANDARD, Engine as _};
use dgos_db::{
    migrations::Migrator,
    sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement},
    sea_orm_migration::MigratorTrait,
    AiContentRepository,
};
use dgos_types::{AiResource, AiResourceImport, PreviewChatResourceInput};
use dgos_workbench::{ChatError, ChatManager};
use sha2::{Digest, Sha256};

const SESSION: &str = "alice-session";

async fn fixture() -> (DatabaseConnection, ChatManager) {
    let db = dgos_db::establish_connection("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db.execute_unprepared(
        "INSERT INTO users(id,username,username_key,password_hash,created_at,updated_at) \
         VALUES('alice','Alice','alice','x',1,1),('bob','Bob','bob','x',1,1); \
         INSERT INTO chat_sessions(id,user_id,title,created_at,updated_at) \
         VALUES('alice-session','alice','chat',1,1),('alice-other','alice','other',1,1), \
         ('bob-session','bob','chat',1,1);",
    ).await.unwrap();
    let chat = ChatManager::new(db.clone(), dgos_db::MaintenanceGate::new());
    (db, chat)
}

async fn import(chat: &ChatManager, mime: &str, bytes: &[u8]) -> AiResource {
    chat.import_ai_resource("alice", SESSION, AiResourceImport {
        name: "resource".into(),
        mime: mime.into(),
        base64: STANDARD.encode(bytes),
    }).await.unwrap()
}

fn input(session: &str, resource: &AiResource) -> PreviewChatResourceInput {
    PreviewChatResourceInput {
        session_id: session.into(),
        resource_id: resource.id.clone(),
        sha256: resource.sha256.clone(),
    }
}

#[tokio::test]
async fn unicode_preview_covers_the_full_text_import_limit_without_renewing_retention() {
    let (db, chat) = fixture().await;
    let mut content = "正文🙂\n".repeat((1024 * 1024) / "正文🙂\n".len());
    content.extend(std::iter::repeat_n('x', 1024 * 1024 - content.len()));
    let resource = import(&chat, "text/markdown", content.as_bytes()).await;
    let preview = chat.preview_chat_resource("alice", input(SESSION, &resource)).await.unwrap();
    let decoded = STANDARD.decode(&preview.base64).unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), content);
    assert_eq!(preview.mime, "text/markdown");
    assert_eq!(preview.start, 0);
    assert_eq!(preview.bytes, 1024 * 1024);
    assert_eq!(preview.total_bytes, resource.bytes_len);
    assert!(!preview.truncated);
    let wire = serde_json::to_value(&preview).unwrap();
    assert_eq!(wire.as_object().unwrap().len(), 6);
    let metadata = AiContentRepository::new(db.clone())
        .resource_metadata("alice", &format!("chat:{SESSION}"), &resource.id)
        .await.unwrap().unwrap();
    assert_eq!(metadata.expires_at, resource.expires_at);
    for table in ["ai_content_runs", "workbench_tasks"] {
        let row = db.query_one(Statement::from_string(DbBackend::Sqlite,
            format!("SELECT COUNT(*) AS n FROM {table}")))
            .await.unwrap().unwrap();
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 0, "preview does not invoke or retain a run");
    }
}

#[tokio::test]
async fn image_preview_covers_the_full_image_import_limit() {
    let (_, chat) = fixture().await;
    let mut bytes = vec![0; 8 * 1024 * 1024];
    bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    let resource = import(&chat, "image/png", &bytes).await;
    let preview = chat.preview_chat_resource("alice", input(SESSION, &resource)).await.unwrap();
    assert_eq!(STANDARD.decode(preview.base64).unwrap(), bytes);
    assert_eq!(preview.mime, "image/png");
    assert_eq!(preview.start, 0);
    assert_eq!(preview.bytes, 8 * 1024 * 1024);
    assert_eq!(preview.total_bytes, resource.bytes_len);
    assert!(!preview.truncated);
}

#[tokio::test]
async fn preview_denies_foreign_users_sessions_and_reference_hashes() {
    let (db, chat) = fixture().await;
    let resource = import(&chat, "text/plain", "私有正文".as_bytes()).await;
    assert_eq!(chat.preview_chat_resource("bob", input(SESSION, &resource)).await.err(),
        Some(ChatError::SessionNotFound));
    for session in ["alice-other", "bob-session"] {
        let user = if session == "bob-session" { "bob" } else { "alice" };
        assert_eq!(chat.preview_chat_resource(user, input(session, &resource)).await.err(),
            Some(ChatError::ToolTurnInvalid));
    }
    let mut changed = input(SESSION, &resource);
    changed.sha256 = "0".repeat(64);
    assert_eq!(chat.preview_chat_resource("alice", changed).await.err(),
        Some(ChatError::ToolTurnInvalid));
    // Stored metadata matching the caller is insufficient if the underlying content changed.
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "UPDATE ai_content_resources SET bytes=?1 WHERE id=?2",
        vec![b"changed".to_vec().into(), resource.id.clone().into()]))
        .await.unwrap();
    assert_eq!(chat.preview_chat_resource("alice", input(SESSION, &resource)).await.err(),
        Some(ChatError::ToolTurnInvalid));
}

#[tokio::test]
async fn preview_denies_expired_deleted_resources_and_cancelled_account_leases() {
    let (db, chat) = fixture().await;
    let resource = import(&chat, "text/plain", b"temporary").await;
    let lease = tokio_util::sync::CancellationToken::new();
    let leased = chat.with_account_lease(lease.clone());
    lease.cancel();
    assert_eq!(leased.preview_chat_resource("alice", input(SESSION, &resource)).await.err(),
        Some(ChatError::Provider(dgos_types::DgosError::SessionInvalid.error_code())));
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "UPDATE ai_content_resources SET expires_at=0 WHERE id=?1",
        vec![resource.id.clone().into()])).await.unwrap();
    assert_eq!(chat.preview_chat_resource("alice", input(SESSION, &resource)).await.err(),
        Some(ChatError::ToolTurnInvalid));
    let resource = import(&chat, "text/plain", b"deleted").await;
    assert!(chat.delete_ai_resource("alice", SESSION, &resource.id).await.unwrap());
    assert_eq!(chat.preview_chat_resource("alice", input(SESSION, &resource)).await.err(),
        Some(ChatError::ToolTurnInvalid));
}

#[tokio::test]
async fn preview_requires_a_live_account_and_existing_owned_session() {
    let (db, chat) = fixture().await;
    let resource = import(&chat, "text/plain", b"private").await;
    db.execute_unprepared("UPDATE users SET deleting_since=1 WHERE id='alice'")
        .await.unwrap();
    assert!(chat.preview_chat_resource("alice", input(SESSION, &resource)).await.is_err());
    db.execute_unprepared("UPDATE users SET deleting_since=NULL WHERE id='alice'; \
        DELETE FROM chat_sessions WHERE id='alice-session'").await.unwrap();
    assert_eq!(chat.preview_chat_resource("alice", input(SESSION, &resource)).await.err(),
        Some(ChatError::SessionNotFound));
}

#[tokio::test]
async fn preview_rejects_stored_text_beyond_the_import_limit_even_with_a_matching_hash() {
    let (db, chat) = fixture().await;
    let mut resource = import(&chat, "text/plain", b"text").await;
    let oversized = vec![b'x'; 1024 * 1024 + 1];
    resource.sha256 = format!("{:x}", Sha256::digest(&oversized));
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "UPDATE ai_content_resources SET bytes=?1,sha256=?2 WHERE id=?3",
        vec![oversized.into(), resource.sha256.clone().into(), resource.id.clone().into()]))
        .await.unwrap();
    assert_eq!(chat.preview_chat_resource("alice", input(SESSION, &resource)).await.err(),
        Some(ChatError::ToolTurnInvalid));
}

#[test]
fn preview_input_accepts_only_the_resource_reference_not_identity_or_content_sources() {
    let valid = serde_json::json!({"session_id":SESSION,"resource_id":"r","sha256":"h"});
    assert!(serde_json::from_value::<PreviewChatResourceInput>(valid.clone()).is_ok());
    for key in ["user_id", "scope", "url", "path", "start", "max_bytes"] {
        let mut value = valid.clone();
        value[key] = "not permitted".into();
        assert!(serde_json::from_value::<PreviewChatResourceInput>(value).is_err());
    }
}
