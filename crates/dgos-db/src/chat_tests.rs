//! FR-006 stage 1/2 persistence tests: migration shape, user isolation, keyset paging, search,
//! deletion boundary, `secure_delete`, generation transactions and recovery. Synthetic text only.

use crate::migrations::Migrator;
use crate::repositories::chat_repository::*;
use crate::sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};
use crate::sea_orm_migration::MigratorTrait;

const ALICE: &str = "alice";
const BOB: &str = "bob";

async fn memory() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

async fn one(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index::<i64>(0)
        .unwrap()
}

fn begin<'a>(
    user: &'a str,
    session: &'a str,
    task: &'a str,
    content: Option<&'a str>,
    regenerate_of: Option<&'a str>,
    ids: (&'a str, &'a str),
) -> BeginGeneration<'a> {
    BeginGeneration {
        user,
        session_id: session,
        task_id: task,
        user_message_id: ids.0,
        assistant_message_id: ids.1,
        user_content: content,
        regenerate_of,
        provider_id: "provider-1",
        model_id: "model-1",
        provider_config_revision: Some(3),
        endpoint_fingerprint: "fp-a",
        skill: None,
        tools: &[],
    }
}

fn done<'a>(
    user: &'a str,
    assistant: &'a str,
    task: &'a str,
    text: &'a str,
) -> FinalizeGeneration<'a> {
    FinalizeGeneration {
        user,
        assistant_message_id: assistant,
        task_id: task,
        content: text,
        message_status: "completed",
        finish_reason: Some("stop"),
        message_error: None,
        task_status: "succeeded",
        task_error: None,
        total_tokens: Some(12),
    }
}

#[tokio::test]
async fn migration_adds_task_columns_with_defaults_and_enforces_checks() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    // Apply everything before the chat migration, insert an old row, then migrate.
    Migrator::up(&db, Some(22)).await.unwrap();
    db.execute_unprepared(
        "INSERT INTO workbench_tasks(id,user_id,app_id,provider_id,model_id,status,created_at) \
         VALUES('old','u','a','p','m','succeeded',1)",
    )
    .await
    .unwrap();
    Migrator::up(&db, None).await.unwrap();
    assert_eq!(
        one(
            &db,
            "SELECT COUNT(*) FROM workbench_tasks WHERE kind = 'single' AND session_id IS NULL"
        )
        .await,
        1
    );
    let bad_kind = db
        .execute_unprepared(
            "INSERT INTO workbench_tasks(id,user_id,app_id,provider_id,model_id,status,created_at,kind) \
             VALUES('x','u','a','p','m','running',1,'bogus')",
        )
        .await;
    assert!(bad_kind.is_err());
    let bad_role = db
        .execute_unprepared(
            "INSERT INTO chat_sessions(id,user_id,title,created_at,updated_at) VALUES('s','u','t',1,1); \
             INSERT INTO chat_messages(id,session_id,user_id,seq,role,status,created_at,updated_at) \
             VALUES('m','s','u',1,'system','completed',1,1)",
        )
        .await;
    assert!(bad_role.is_err(), "system role must never be stored");
}

#[tokio::test]
async fn a_message_cannot_belong_to_another_users_session() {
    let db = memory().await;
    db.execute_unprepared(
        "INSERT INTO chat_sessions(id,user_id,title,created_at,updated_at) VALUES('s','alice','t',1,1)",
    )
    .await
    .unwrap();
    let cross = db
        .execute_unprepared(
            "INSERT INTO chat_messages(id,session_id,user_id,seq,role,status,created_at,updated_at) \
             VALUES('m','s','bob',1,'user','completed',1,1)",
        )
        .await;
    assert!(
        cross.is_err(),
        "composite foreign key keeps message and session under one user"
    );
}

#[tokio::test]
async fn other_users_and_missing_sessions_are_indistinguishable() {
    let repo = ChatRepository::new(memory().await);
    let session = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    let missing = "00000000-0000-4000-8000-000000000000";
    for id in [session.id.as_str(), missing] {
        assert_eq!(
            repo.get_session(BOB, id).await.unwrap_err(),
            ChatStoreError::SessionNotFound
        );
        assert_eq!(
            repo.rename_session(BOB, id, "x").await.unwrap_err(),
            ChatStoreError::SessionNotFound
        );
        assert_eq!(
            repo.delete_session(BOB, id).await.unwrap_err(),
            ChatStoreError::SessionNotFound
        );
        assert_eq!(
            repo.list_messages(BOB, id, None, 10).await.unwrap_err(),
            ChatStoreError::SessionNotFound
        );
        assert_eq!(
            repo.set_target(BOB, id, &ChatTarget::default())
                .await
                .unwrap_err(),
            ChatStoreError::SessionNotFound
        );
    }
    assert!(repo
        .list_sessions(BOB, None, 10)
        .await
        .unwrap()
        .0
        .is_empty());
    assert_eq!(
        repo.list_sessions(ALICE, None, 10).await.unwrap().0.len(),
        1
    );
}

#[tokio::test]
async fn sessions_page_by_keyset_with_stable_order_inside_one_second() {
    let db = memory().await;
    let repo = ChatRepository::new(db.clone());
    let mut created = Vec::new();
    for _ in 0..7 {
        created.push(
            repo.create_session(ALICE, &ChatTarget::default())
                .await
                .unwrap()
                .id,
        );
    }
    // Force every session into the same second.
    db.execute_unprepared("UPDATE chat_sessions SET updated_at = 100")
        .await
        .unwrap();
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let (page, next) = repo
            .list_sessions(ALICE, cursor.as_deref(), 3)
            .await
            .unwrap();
        assert!(page.len() <= 3);
        seen.extend(page.into_iter().map(|s| s.id));
        match next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let mut expected = created.clone();
    expected.sort();
    expected.reverse();
    assert_eq!(
        seen, expected,
        "ties on updated_at order by id descending, no gaps or repeats"
    );
    assert_eq!(
        repo.list_sessions(ALICE, Some("not-a-cursor"), 3)
            .await
            .unwrap_err(),
        ChatStoreError::InvalidCursor
    );
}

#[tokio::test]
async fn messages_page_newest_first_and_return_oldest_first() {
    let db = memory().await;
    let repo = ChatRepository::new(db.clone());
    let session = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    for i in 0..5 {
        db.execute_unprepared(&format!(
            "INSERT INTO chat_messages(id,session_id,user_id,seq,role,content,status,created_at,updated_at) \
             VALUES('m{i}','{}','alice',{},'user','text {i}','completed',1,1)",
            session.id,
            i + 1
        ))
        .await
        .unwrap();
    }
    let (first, next) = repo
        .list_messages(ALICE, &session.id, None, 2)
        .await
        .unwrap();
    assert_eq!(first.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![4, 5]);
    let (second, next) = repo
        .list_messages(ALICE, &session.id, next, 2)
        .await
        .unwrap();
    assert_eq!(second.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![2, 3]);
    let (third, next) = repo
        .list_messages(ALICE, &session.id, next, 2)
        .await
        .unwrap();
    assert_eq!(third.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1]);
    assert_eq!(next, None);
}

#[tokio::test]
async fn first_message_sets_the_automatic_title_and_a_user_title_is_never_overwritten() {
    let repo = ChatRepository::new(memory().await);
    let session = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    let text = "第一行\n第二行 0123456789 0123456789 0123456789";
    repo.begin_generation(&begin(
        ALICE,
        &session.id,
        "t1",
        Some(text),
        None,
        ("u1", "a1"),
    ))
    .await
    .unwrap();
    let titled = repo.get_session(ALICE, &session.id).await.unwrap();
    assert_eq!(titled.title.chars().count(), 30);
    assert!(!titled.title.contains('\n'));
    assert_eq!(titled.title_source, "auto");
    repo.finalize_generation(&done(ALICE, "a1", "t1", "reply"))
        .await
        .unwrap();
    repo.rename_session(ALICE, &session.id, "我的标题")
        .await
        .unwrap();
    repo.begin_generation(&begin(
        ALICE,
        &session.id,
        "t2",
        Some("second"),
        None,
        ("u2", "a2"),
    ))
    .await
    .unwrap();
    let renamed = repo.get_session(ALICE, &session.id).await.unwrap();
    assert_eq!(
        (renamed.title.as_str(), renamed.title_source.as_str()),
        ("我的标题", "user")
    );
    assert_eq!(
        repo.rename_session(ALICE, &session.id, "   ")
            .await
            .unwrap_err(),
        ChatStoreError::InvalidTitle
    );
}

#[tokio::test]
async fn search_treats_wildcards_literally_and_stays_inside_the_user() {
    let db = memory().await;
    let repo = ChatRepository::new(db.clone());
    let a = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    let b = repo
        .create_session(BOB, &ChatTarget::default())
        .await
        .unwrap();
    for (session, user, id, seq, status, replaces, text) in [
        (
            &a.id,
            ALICE,
            "m1",
            1,
            "completed",
            None,
            "Python 100% done_ok back\\slash",
        ),
        (&a.id, ALICE, "m2", 2, "generating", None, "Python draft"),
        (
            &a.id,
            ALICE,
            "m3",
            3,
            "completed",
            Some("m1"),
            "Python replacement draft",
        ),
        (
            &b.id,
            BOB,
            "m4",
            1,
            "completed",
            None,
            "Python secret of bob",
        ),
    ] {
        let replaces_sql = replaces.map_or("NULL".to_owned(), |r| format!("'{r}'"));
        db.execute_unprepared(&format!(
            "INSERT INTO chat_messages(id,session_id,user_id,seq,role,content,status,replaces_id,created_at,updated_at) \
             VALUES('{id}','{session}','{user}',{seq},'assistant','{text}','{status}',{replaces_sql},1,1)"
        ))
        .await
        .unwrap();
    }
    let hits = |q: &str| {
        let repo = repo.clone();
        let q = q.to_owned();
        async move { repo.search_sessions(ALICE, &q, None, 50).await.unwrap().0 }
    };
    let python = hits("python").await;
    assert_eq!(
        python.len(),
        1,
        "case-insensitive for ASCII, current user only"
    );
    assert_eq!(python[0].session.id, a.id);
    assert_eq!(
        python[0].match_count, 1,
        "generating and regeneration drafts are not searched"
    );
    assert!(python[0].snippet.contains("Python"));
    assert_eq!(hits("100%").await.len(), 1);
    assert_eq!(
        hits("%").await.len(),
        1,
        "a bare percent matches only a literal percent"
    );
    assert_eq!(hits("done_ok").await.len(), 1);
    assert!(
        hits("done_xk").await.is_empty(),
        "underscore is not a wildcard"
    );
    assert_eq!(hits("back\\slash").await.len(), 1);
    assert!(hits("secret of bob").await.is_empty());
}

#[tokio::test]
async fn deleting_a_session_keeps_audit_rows_but_not_content() {
    let db = memory().await;
    let repo = ChatRepository::new(db.clone());
    let session = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    let other = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    repo.begin_generation(&begin(
        ALICE,
        &session.id,
        "t1",
        Some("hello there"),
        None,
        ("u1", "a1"),
    ))
    .await
    .unwrap();
    repo.finalize_generation(&done(ALICE, "a1", "t1", "general reply"))
        .await
        .unwrap();
    repo.begin_generation(&begin(
        ALICE,
        &other.id,
        "t2",
        Some("other session"),
        None,
        ("u2", "a2"),
    ))
    .await
    .unwrap();
    // The other session is still generating, which only blocks deleting that session.
    repo.delete_session(ALICE, &session.id).await.unwrap();
    assert_eq!(
        repo.delete_session(ALICE, &other.id).await.unwrap_err(),
        ChatStoreError::GenerationInProgress
    );
    assert_eq!(
        one(
            &db,
            &format!(
                "SELECT COUNT(*) FROM chat_messages WHERE session_id = '{}'",
                session.id
            )
        )
        .await,
        0,
        "the deleted session's messages are gone"
    );
    assert_eq!(
        one(
            &db,
            &format!(
                "SELECT COUNT(*) FROM chat_messages WHERE session_id = '{}'",
                other.id
            )
        )
        .await,
        2,
        "the refused delete left the other session untouched"
    );
    assert_eq!(
        repo.delete_session(ALICE, &session.id).await.unwrap_err(),
        ChatStoreError::SessionNotFound
    );
    assert_eq!(
        one(&db, "SELECT COUNT(*) FROM workbench_tasks WHERE id = 't1' AND session_id IS NULL AND kind = 'chat'").await,
        1,
        "the audit row survives with its session reference cleared and its kind unchanged"
    );
    assert_eq!(
        one(
            &db,
            "SELECT COUNT(*) FROM workbench_tasks WHERE id = 't2' AND session_id IS NOT NULL"
        )
        .await,
        1
    );
    assert_eq!(
        one(
            &db,
            "SELECT COUNT(*) FROM chat_messages WHERE content LIKE '%general reply%'"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn secure_delete_leaves_no_sentinel_in_the_main_database_file() {
    const SENTINEL: &str = "SENTINEL-FR006-DELETE-0f3c9a";
    let dir = std::env::temp_dir().join(format!("dgos-chat-secure-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("dgos.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let db = crate::establish_connection(&url).await.unwrap();
    let pragma = one(&db, "PRAGMA secure_delete").await;
    assert_eq!(pragma, 1, "establish_connection enables secure_delete");
    Migrator::up(&db, None).await.unwrap();
    let repo = ChatRepository::new(db.clone());
    let session = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    for i in 0..60 {
        let task = format!("t{i}");
        let (u, a) = (format!("u{i}"), format!("a{i}"));
        let text = format!("{SENTINEL} {i} {}", "x".repeat(300));
        repo.begin_generation(&begin(
            ALICE,
            &session.id,
            &task,
            Some(&text),
            None,
            (&u, &a),
        ))
        .await
        .unwrap();
        repo.finalize_generation(&done(ALICE, &a, &task, &text))
            .await
            .unwrap();
    }
    let count = |path: &std::path::Path| {
        let bytes = std::fs::read(path).unwrap();
        bytes
            .windows(SENTINEL.len())
            .filter(|w| *w == SENTINEL.as_bytes())
            .count()
    };
    assert!(count(&path) > 0, "the sentinel is present before deletion");
    repo.delete_session(ALICE, &session.id).await.unwrap();
    assert_eq!(
        count(&path),
        0,
        "the main database file holds no sentinel after deletion"
    );
    drop(repo);
    drop(db);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn generation_lifecycle_replays_conflicts_and_blocks_a_second_active_generation() {
    let db = memory().await;
    let repo = ChatRepository::new(db.clone());
    let one_session = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    let two_session = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    let first = repo
        .begin_generation(&begin(
            ALICE,
            &one_session.id,
            "t1",
            Some("hi"),
            None,
            ("u1", "a1"),
        ))
        .await
        .unwrap();
    assert!(!first.replayed);
    let replay = repo
        .begin_generation(&begin(
            ALICE,
            &one_session.id,
            "t1",
            Some("hi"),
            None,
            ("u9", "a9"),
        ))
        .await
        .unwrap();
    assert_eq!(
        (
            replay.user_message_id.as_str(),
            replay.assistant_message_id.as_str(),
            replay.replayed
        ),
        ("u1", "a1", true)
    );
    assert_eq!(
        repo.begin_generation(&begin(
            ALICE,
            &one_session.id,
            "t1",
            Some("different"),
            None,
            ("u8", "a8")
        ))
        .await
        .unwrap_err(),
        ChatStoreError::RequestConflict
    );
    for (session, task) in [(&one_session.id, "t2"), (&two_session.id, "t3")] {
        assert_eq!(
            repo.begin_generation(&begin(
                ALICE,
                session,
                task,
                Some("again"),
                None,
                ("ux", "ax")
            ))
            .await
            .unwrap_err(),
            ChatStoreError::GenerationInProgress,
            "one active generation per user, in any session"
        );
    }
    assert_eq!(
        one(
            &db,
            "SELECT COUNT(*) FROM workbench_tasks WHERE id IN ('t2','t3')"
        )
        .await,
        0,
        "refused sends leave no rows"
    );
    // Another user is unaffected.
    let bob = repo
        .create_session(BOB, &ChatTarget::default())
        .await
        .unwrap();
    repo.begin_generation(&begin(BOB, &bob.id, "tb", Some("hi"), None, ("ub", "ab")))
        .await
        .unwrap();
    // A task id already used by another user's session is a conflict, not a hijack.
    assert_eq!(
        repo.begin_generation(&begin(BOB, &bob.id, "t1", Some("hi"), None, ("uc", "ac")))
            .await
            .unwrap_err(),
        ChatStoreError::RequestConflict
    );
}

#[tokio::test]
async fn terminal_state_is_written_once_and_unknown_usage_is_null() {
    let db = memory().await;
    let repo = ChatRepository::new(db.clone());
    let session = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    repo.begin_generation(&begin(
        ALICE,
        &session.id,
        "t1",
        Some("hi"),
        None,
        ("u1", "a1"),
    ))
    .await
    .unwrap();
    let interrupted = FinalizeGeneration {
        message_status: "interrupted",
        finish_reason: None,
        message_error: Some("task_cancelled"),
        task_status: "interrupted",
        task_error: Some("task_cancelled"),
        total_tokens: None,
        ..done(ALICE, "a1", "t1", "partial")
    };
    let Finalized::Done { message, discarded } =
        repo.finalize_generation(&interrupted).await.unwrap()
    else {
        panic!("first terminal write must apply");
    };
    assert!(!discarded);
    assert_eq!(
        (message.status.as_str(), message.content.as_str()),
        ("interrupted", "partial")
    );
    assert_eq!(message.total_tokens, None, "unknown usage is not 0");
    assert!(
        message.result_unknown,
        "a cancelled call without usage may still have been billed"
    );
    assert_eq!(
        repo.finalize_generation(&done(ALICE, "a1", "t1", "late complete"))
            .await
            .unwrap(),
        Finalized::AlreadyFinalized
    );
    let kept = repo.get_message(ALICE, "a1").await.unwrap();
    assert_eq!(
        (kept.status.as_str(), kept.content.as_str()),
        ("interrupted", "partial")
    );
    assert_eq!(one(&db, "SELECT COUNT(*) FROM workbench_tasks WHERE id='t1' AND status='interrupted' AND total_tokens IS NULL").await, 1);
}

#[tokio::test]
async fn regeneration_replaces_only_on_success_and_only_the_last_reply() {
    let db = memory().await;
    let repo = ChatRepository::new(db.clone());
    let session = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    repo.begin_generation(&begin(
        ALICE,
        &session.id,
        "t1",
        Some("q1"),
        None,
        ("u1", "a1"),
    ))
    .await
    .unwrap();
    repo.finalize_generation(&done(ALICE, "a1", "t1", "old one"))
        .await
        .unwrap();
    repo.begin_generation(&begin(
        ALICE,
        &session.id,
        "t2",
        Some("q2"),
        None,
        ("u2", "a2"),
    ))
    .await
    .unwrap();
    repo.finalize_generation(&done(ALICE, "a2", "t2", "old two"))
        .await
        .unwrap();

    assert_eq!(
        repo.begin_generation(&begin(
            ALICE,
            &session.id,
            "r0",
            None,
            Some("a1"),
            ("-", "n0")
        ))
        .await
        .unwrap_err(),
        ChatStoreError::NotRegenerable,
        "only the last assistant reply"
    );
    assert_eq!(
        repo.begin_generation(&begin(
            ALICE,
            &session.id,
            "r0",
            None,
            Some("u2"),
            ("-", "n0")
        ))
        .await
        .unwrap_err(),
        ChatStoreError::NotRegenerable,
        "a user message is not regenerable"
    );

    // Failure keeps the old reply and drops the draft.
    let begun = repo
        .begin_generation(&begin(
            ALICE,
            &session.id,
            "r1",
            None,
            Some("a2"),
            ("-", "n1"),
        ))
        .await
        .unwrap();
    assert_eq!(begun.user_message_id, "u2");
    let (shown, _) = repo
        .list_messages(ALICE, &session.id, None, 50)
        .await
        .unwrap();
    assert!(
        !shown.iter().any(|m| m.id == "n1"),
        "the draft is hidden from the message list"
    );
    let failed = FinalizeGeneration {
        message_status: "failed",
        task_status: "failed",
        task_error: Some("provider_1001007"),
        total_tokens: None,
        ..done(ALICE, "n1", "r1", "")
    };
    let Finalized::Done { message, discarded } = repo.finalize_generation(&failed).await.unwrap()
    else {
        panic!("applies");
    };
    assert!(discarded);
    assert_eq!(
        (message.id.as_str(), message.content.as_str()),
        ("a2", "old two")
    );
    assert_eq!(
        one(&db, "SELECT COUNT(*) FROM chat_messages WHERE id = 'n1'").await,
        0
    );
    assert_eq!(
        one(
            &db,
            "SELECT COUNT(*) FROM workbench_tasks WHERE id = 'r1' AND status = 'failed'"
        )
        .await,
        1,
        "the new billed call keeps its own audit row"
    );

    // Success replaces atomically.
    repo.begin_generation(&begin(
        ALICE,
        &session.id,
        "r2",
        None,
        Some("a2"),
        ("-", "n2"),
    ))
    .await
    .unwrap();
    let Finalized::Done { message, discarded } = repo
        .finalize_generation(&done(ALICE, "n2", "r2", "new two"))
        .await
        .unwrap()
    else {
        panic!("applies");
    };
    assert!(!discarded);
    assert_eq!(message.content, "new two");
    let (all, _) = repo
        .list_messages(ALICE, &session.id, None, 50)
        .await
        .unwrap();
    assert_eq!(
        all.iter()
            .map(|m| (m.id.as_str(), m.content.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("u1", "q1"),
            ("a1", "old one"),
            ("u2", "q2"),
            ("n2", "new two")
        ]
    );
}

#[tokio::test]
async fn recovery_ends_generating_messages_and_discards_regeneration_drafts() {
    let db = memory().await;
    let repo = ChatRepository::new(db.clone());
    let session = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    repo.begin_generation(&begin(
        ALICE,
        &session.id,
        "t1",
        Some("q1"),
        None,
        ("u1", "a1"),
    ))
    .await
    .unwrap();
    repo.finalize_generation(&done(ALICE, "a1", "t1", "kept reply"))
        .await
        .unwrap();
    repo.begin_generation(&begin(
        ALICE,
        &session.id,
        "r1",
        None,
        Some("a1"),
        ("-", "n1"),
    ))
    .await
    .unwrap();
    let bob = repo
        .create_session(BOB, &ChatTarget::default())
        .await
        .unwrap();
    repo.begin_generation(&begin(BOB, &bob.id, "tb", Some("q"), None, ("ub", "ab")))
        .await
        .unwrap();
    db.execute_unprepared("UPDATE chat_messages SET content = 'partial text' WHERE id = 'ab'")
        .await
        .unwrap();

    assert_eq!(
        repo.recover_generating().await.unwrap(),
        1,
        "the draft is deleted, the other is recovered"
    );
    assert_eq!(
        one(&db, "SELECT COUNT(*) FROM chat_messages WHERE id = 'n1'").await,
        0
    );
    let recovered = repo.get_message(BOB, "ab").await.unwrap();
    assert_eq!(
        (recovered.status.as_str(), recovered.error_code.as_deref()),
        ("interrupted", Some("process_interrupted"))
    );
    assert_eq!(
        repo.get_message(ALICE, "a1").await.unwrap().content,
        "kept reply"
    );
    assert_eq!(
        repo.recover_generating().await.unwrap(),
        0,
        "recovery is repeatable"
    );
    // Empty generating replies become failed.
    repo.begin_generation(&begin(
        ALICE,
        &session.id,
        "t5",
        Some("q5"),
        None,
        ("u5", "a5"),
    ))
    .await
    .unwrap();
    repo.recover_generating().await.unwrap();
    assert_eq!(
        repo.get_message(ALICE, "a5").await.unwrap().status,
        "failed"
    );
}

#[tokio::test]
async fn history_returns_only_completed_messages_inside_the_byte_budget() {
    let db = memory().await;
    let repo = ChatRepository::new(db.clone());
    let session = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    for i in 1..=4 {
        let (task, u, a) = (format!("t{i}"), format!("u{i}"), format!("a{i}"));
        repo.begin_generation(&begin(
            ALICE,
            &session.id,
            &task,
            Some(&format!("question {i}")),
            None,
            (&u, &a),
        ))
        .await
        .unwrap();
        let answer = format!("answer {i}");
        let outcome = if i == 3 {
            FinalizeGeneration {
                message_status: "failed",
                task_status: "failed",
                total_tokens: None,
                ..done(ALICE, &a, &task, "")
            }
        } else {
            done(ALICE, &a, &task, &answer)
        };
        repo.finalize_generation(&outcome).await.unwrap();
    }
    let all = repo
        .history(ALICE, &session.id, None, 1_000_000)
        .await
        .unwrap();
    let texts: Vec<_> = all.rows.iter().map(|r| r.content.as_str()).collect();
    assert_eq!(
        texts,
        vec![
            "question 1",
            "answer 1",
            "question 2",
            "answer 2",
            "question 3",
            "question 4",
            "answer 4"
        ],
        "the failed reply is excluded; its question is orphaned and the caller drops it"
    );
    assert_eq!(all.total_completed_replies, 3);
    let small = repo.history(ALICE, &session.id, None, 20).await.unwrap();
    assert!(
        small.rows.len() < all.rows.len(),
        "the budget bounds how much is loaded"
    );
    assert_eq!(small.rows.last().unwrap().content, "answer 4");
    assert!(repo
        .history(BOB, &session.id, None, 1_000_000)
        .await
        .unwrap()
        .rows
        .is_empty());
}

#[tokio::test]
async fn pinned_skill_and_no_skill_are_exclusive_and_clearing_resets_the_confirmation() {
    let repo = ChatRepository::new(memory().await);
    let skill = PinnedSkill {
        skill_id: "skill".into(),
        version_id: "skill@1.0.0".into(),
        content_hash: "hash".into(),
    };
    let session = repo
        .create_session(
            ALICE,
            &ChatTarget {
                provider_id: Some("p".into()),
                model_id: Some("m".into()),
                skill: Some(skill.clone()),
                no_skill: true,
            },
        )
        .await
        .unwrap();
    assert!(
        !session.no_skill_confirmed,
        "a pinned skill wins over the no-skill flag"
    );
    assert_eq!(session.skill_version_id.as_deref(), Some("skill@1.0.0"));
    let none = repo
        .set_target(
            ALICE,
            &session.id,
            &ChatTarget {
                no_skill: true,
                ..ChatTarget::default()
            },
        )
        .await
        .unwrap();
    assert!(none.no_skill_confirmed && none.skill_id.is_none());
    let cleared = repo
        .set_target(ALICE, &session.id, &ChatTarget::default())
        .await
        .unwrap();
    assert!(!cleared.no_skill_confirmed && cleared.skill_id.is_none());
}

#[test]
fn like_pattern_title_and_snippet_helpers_are_literal_and_char_safe() {
    assert_eq!(like_pattern("a%b_c\\d"), "%a\\%b\\_c\\\\d%");
    assert_eq!(auto_title("\n\r  \n"), DEFAULT_TITLE);
    assert_eq!(auto_title(&"字".repeat(40)).chars().count(), 30);
    let text = format!("{}needle{}", "前".repeat(100), "后".repeat(200));
    let excerpt = snippet(&text, "NEEDLE");
    assert!(excerpt.contains("needle") && excerpt.starts_with('…') && excerpt.ends_with('…'));
    assert!(!snippet("short text", "zzz").is_empty());
    assert!(result_is_unknown(
        "interrupted",
        None,
        Some("task_cancelled")
    ));
    assert!(!result_is_unknown(
        "interrupted",
        Some(5),
        Some("task_cancelled")
    ));
    assert!(
        !result_is_unknown("failed", None, Some("provider_1001002")),
        "an auth rejection is a known outcome"
    );
    assert!(!result_is_unknown("completed", None, None));
}

#[tokio::test]
async fn store_errors_do_not_echo_message_text() {
    let db = memory().await;
    let repo = ChatRepository::new(db.clone());
    let session = repo
        .create_session(ALICE, &ChatTarget::default())
        .await
        .unwrap();
    // Break the schema so the next write fails with a raw database error.
    db.execute_unprepared("DROP TABLE chat_messages")
        .await
        .unwrap();
    let broken = repo
        .begin_generation(&begin(
            ALICE,
            &session.id,
            "t2",
            Some("SENTINEL-ECHO-9d1"),
            None,
            ("u2", "a2"),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        broken,
        ChatStoreError::Db("database error".into()),
        "database errors are replaced by fixed text"
    );
    assert!(!format!("{broken:?}").contains("SENTINEL-ECHO"));
}
