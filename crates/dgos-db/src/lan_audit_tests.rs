//! V1-FR-010 audit store (B10-65～B10-68): 500-row cap, 60-second merge of
//! identical rejections, visibility (own rows plus unauthenticated rows),
//! paging, and the forbidden-columns guard. Real migrated SQLite.

use crate::migrations::Migrator;
use crate::{
    LanAuditKind, LanAuditRecord, LanAuditRepository, UserDataDomain, UserDataRepository,
    MAX_LAN_AUDIT_ROWS,
};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::MigratorTrait;
use std::net::Ipv4Addr;

async fn database() -> (tempfile::TempDir, DatabaseConnection) {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("dgos.db").display());
    let db = crate::establish_connection(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    (dir, db)
}

async fn rows(db: &DatabaseConnection) -> i64 {
    db.query_one(Statement::from_string(
        DbBackend::Sqlite,
        "SELECT COUNT(*) FROM lan_audit_events",
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get_by_index(0)
    .unwrap()
}

fn rejected(at: i64, reason: &'static str, ip: [u8; 4]) -> LanAuditRecord {
    LanAuditRecord {
        at,
        event: LanAuditKind::RequestRejected,
        reason: Some(reason),
        peer_ip: Some(Ipv4Addr::from(ip)),
        session_short_id: None,
        user_id: None,
        error_code: Some(1010007),
    }
}

fn revoked(at: i64, user: &str) -> LanAuditRecord {
    LanAuditRecord {
        at,
        event: LanAuditKind::SessionRevoked,
        reason: Some("logout"),
        peer_ip: Some(Ipv4Addr::new(192, 168, 1, 20)),
        session_short_id: Some("0123abcd".into()),
        user_id: Some(user.into()),
        error_code: None,
    }
}

#[tokio::test]
async fn identical_rejections_merge_for_sixty_seconds_then_start_a_new_row() {
    let (_dir, db) = database().await;
    let audit = LanAuditRepository::new(db.clone());
    audit
        .append(rejected(1000, "host_mismatch", [192, 168, 1, 9]))
        .await
        .unwrap();
    audit
        .append(rejected(1030, "host_mismatch", [192, 168, 1, 9]))
        .await
        .unwrap();
    // The window is measured from the last occurrence.
    audit
        .append(rejected(1089, "host_mismatch", [192, 168, 1, 9]))
        .await
        .unwrap();
    // Another reason, another source: separate rows.
    audit
        .append(rejected(1090, "origin_mismatch", [192, 168, 1, 9]))
        .await
        .unwrap();
    audit
        .append(rejected(1090, "host_mismatch", [192, 168, 1, 10]))
        .await
        .unwrap();
    // Past the window.
    audit
        .append(rejected(1150, "host_mismatch", [192, 168, 1, 9]))
        .await
        .unwrap();
    let page = audit.list("alice", None, None).await.unwrap();
    let merged: Vec<_> = page
        .items
        .iter()
        .filter(|e| {
            e.reason.as_deref() == Some("host_mismatch")
                && e.peer_ip.as_deref() == Some("192.168.1.9")
        })
        .map(|e| (e.first_at, e.last_at, e.count))
        .collect();
    assert_eq!(merged, vec![(1150, 1150, 1), (1000, 1089, 3)]);
    assert_eq!(rows(&db).await, 4);

    // Revocations are never merged.
    audit.append(revoked(2000, "alice")).await.unwrap();
    audit.append(revoked(2000, "alice")).await.unwrap();
    assert_eq!(rows(&db).await, 6);
}

#[tokio::test]
async fn the_table_keeps_only_the_newest_500_rows() {
    let (_dir, db) = database().await;
    let audit = LanAuditRepository::new(db.clone());
    for i in 0..(MAX_LAN_AUDIT_ROWS + 25) {
        audit.append(revoked(i, "alice")).await.unwrap();
    }
    assert_eq!(rows(&db).await, MAX_LAN_AUDIT_ROWS);
    let oldest: i64 = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT MIN(first_at) FROM lan_audit_events",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index(0)
        .unwrap();
    assert_eq!(oldest, 25);
}

#[tokio::test]
async fn listing_shows_own_and_unauthenticated_rows_newest_first_in_pages() {
    let (_dir, db) = database().await;
    let audit = LanAuditRepository::new(db.clone());
    audit.append(revoked(1, "alice")).await.unwrap();
    audit.append(revoked(2, "bob")).await.unwrap();
    audit
        .append(rejected(3, "peer_not_private", [8, 8, 8, 8]))
        .await
        .unwrap();
    audit.append(revoked(4, "alice")).await.unwrap();

    let first = audit.list("alice", None, Some(2)).await.unwrap();
    assert_eq!(
        first.items.iter().map(|e| e.first_at).collect::<Vec<_>>(),
        vec![4, 3]
    );
    let cursor = first.next_cursor.expect("a second page");
    let second = audit.list("alice", Some(&cursor), Some(2)).await.unwrap();
    assert_eq!(
        second.items.iter().map(|e| e.first_at).collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(second.next_cursor, None);
    // A garbage cursor yields nothing rather than an error or someone else's rows.
    assert!(audit
        .list("alice", Some("x"), None)
        .await
        .unwrap()
        .items
        .is_empty());

    // Account deletion removes the user's rows, keeps the unauthenticated ones.
    UserDataRepository::new(db.clone())
        .purge(UserDataDomain::Lan, "alice")
        .await
        .unwrap();
    let after = audit.list("alice", None, None).await.unwrap();
    assert_eq!(
        after.items.iter().map(|e| e.first_at).collect::<Vec<_>>(),
        vec![3]
    );
    assert_eq!(audit.list("bob", None, None).await.unwrap().items.len(), 2);
}

#[tokio::test]
async fn a_malformed_short_id_is_not_stored() {
    let (_dir, db) = database().await;
    let audit = LanAuditRepository::new(db.clone());
    let mut record = revoked(1, "alice");
    record.session_short_id = Some("0123abcd-full-session-id".into());
    audit.append(record).await.unwrap();
    let page = audit.list("alice", None, None).await.unwrap();
    assert_eq!(page.items[0].session_short_id, None);
}

/// D099: the table has no column that could hold credentials, tokens, the
/// User-Agent, device labels, paths or content. A migration adding one fails here.
#[tokio::test]
async fn the_audit_schema_has_only_the_permitted_columns() {
    let (_dir, db) = database().await;
    let columns: Vec<String> = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM pragma_table_info('lan_audit_events') ORDER BY cid",
        ))
        .await
        .unwrap()
        .iter()
        .map(|row| row.try_get("", "name").unwrap())
        .collect();
    assert_eq!(
        columns,
        [
            "id",
            "first_at",
            "last_at",
            "event",
            "reason",
            "peer_ip",
            "session_short_id",
            "user_id",
            "error_code",
            "count"
        ]
    );
    // The event column is a closed set.
    assert!(db
        .execute_unprepared(
            "INSERT INTO lan_audit_events (id, first_at, last_at, event) VALUES ('x', 1, 1, 'chat_text')"
        )
        .await
        .is_err());
}
