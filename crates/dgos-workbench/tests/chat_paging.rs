//! FR-006 AC28/AC30 backend: keyset paging over 1000+ sessions and messages, page caps, search
//! validation and pagination, and the D076 timing record for `LIKE` over 10,000 messages.
//! Synthetic rows are inserted directly; this is not a Provider test.

use dgos_db::{
    migrations::Migrator,
    sea_orm::{ConnectionTrait, Database, DatabaseConnection},
    sea_orm_migration::MigratorTrait,
};
use dgos_workbench::{
    ChatError, ChatManager, ListChatMessagesInput, ListChatSessionsInput, SearchChatSessionsInput,
};
use std::{collections::HashSet, time::Instant};

const ALICE: &str = "alice";

/// Production ids are UUIDs and the cursor parser relies on that, so the synthetic rows use
/// UUID-shaped ids too (stable, sortable).
fn sid(s: usize) -> String {
    format!("00000000-0000-4000-8000-{s:012x}")
}

async fn seeded(sessions: usize, messages_per_session: usize) -> (DatabaseConnection, ChatManager) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db.execute_unprepared("BEGIN").await.unwrap();
    for s in 0..sessions {
        // Several sessions share one `updated_at` second so ties must order stably.
        db.execute_unprepared(&format!(
            "INSERT INTO chat_sessions(id,user_id,title,created_at,updated_at) VALUES('{}','{ALICE}','session {s}',1,{})",
            sid(s),
            1000 + (s / 7) as i64
        ))
        .await
        .unwrap();
        for m in 0..messages_per_session {
            db.execute_unprepared(&format!(
                "INSERT INTO chat_messages(id,session_id,user_id,seq,role,content,status,created_at,updated_at) \
                 VALUES('m{s:05}-{m:04}','{}','{ALICE}',{},'{}','synthetic message {s}-{m} lorem ipsum 中文内容','completed',1,1)",
                sid(s),
                m + 1,
                if m % 2 == 0 { "user" } else { "assistant" }
            ))
            .await
            .unwrap();
        }
    }
    db.execute_unprepared("COMMIT").await.unwrap();
    (db.clone(), ChatManager::new(db))
}

#[tokio::test]
async fn a_thousand_sessions_page_without_gaps_repeats_or_oversized_pages() {
    let (_, chat) = seeded(1200, 0).await;
    let mut seen = Vec::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        // A request above the cap is clamped to 50 rather than honoured.
        let page = chat
            .list_sessions(
                ALICE,
                ListChatSessionsInput {
                    cursor: cursor.clone(),
                    limit: Some(10_000),
                },
            )
            .await
            .unwrap();
        assert!(page.items.len() <= 50);
        seen.extend(page.items.into_iter().map(|s| (s.updated_at, s.id)));
        pages += 1;
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(seen.len(), 1200);
    assert_eq!(
        seen.iter().map(|(_, id)| id).collect::<HashSet<_>>().len(),
        1200,
        "no repeats"
    );
    assert!(pages >= 24);
    let mut sorted = seen.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(seen, sorted, "newest first, ties ordered by id descending");
}

#[tokio::test]
async fn a_thousand_messages_page_oldest_last_with_stable_order_inside_one_second() {
    let (_, chat) = seeded(1, 1500).await;
    let mut collected: Vec<i64> = Vec::new();
    let mut before = None;
    loop {
        let page = chat
            .list_messages(
                ALICE,
                ListChatMessagesInput {
                    session_id: sid(0),
                    before_seq: before,
                    limit: Some(9999),
                },
            )
            .await
            .unwrap();
        assert!(page.items.len() <= 100, "the page cap holds");
        let mut seqs: Vec<i64> = page.items.iter().map(|m| m.seq).collect();
        assert!(
            seqs.windows(2).all(|w| w[0] < w[1]),
            "each page is returned oldest first"
        );
        seqs.extend(collected);
        collected = seqs;
        match page.next_before_seq {
            Some(next) => before = Some(next),
            None => break,
        }
    }
    assert_eq!(
        collected,
        (1..=1500).collect::<Vec<_>>(),
        "every message exactly once, in order"
    );
}

#[tokio::test]
async fn search_validates_the_query_and_pages_by_keyset() {
    let (_, chat) = seeded(130, 1).await;
    let search = |query: &str, cursor: Option<String>| {
        let chat = chat.clone();
        let query = query.to_owned();
        async move {
            chat.search_sessions(
                ALICE,
                SearchChatSessionsInput {
                    query,
                    cursor,
                    limit: Some(50),
                },
            )
            .await
        }
    };
    for bad in ["", "   ", &"x".repeat(129)] {
        assert_eq!(
            search(bad, None).await.err(),
            Some(ChatError::InvalidSearchQuery),
            "{} chars",
            bad.chars().count()
        );
    }
    assert!(
        search(&"字".repeat(128), None).await.is_ok(),
        "128 characters is allowed"
    );
    assert!(
        search("  lorem  ", None).await.is_ok(),
        "surrounding spaces are trimmed"
    );
    assert_eq!(
        search("lorem", Some("garbage".into())).await.err(),
        Some(ChatError::InvalidSearchQuery)
    );

    let mut ids = HashSet::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        let page = search("lorem", cursor).await.unwrap();
        assert!(page.items.len() <= 50);
        for hit in &page.items {
            assert!(hit.snippet.contains("lorem"));
            assert!(
                ids.insert(hit.session.id.clone()),
                "no session twice across pages"
            );
        }
        pages += 1;
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!((ids.len(), pages), (130, 3));
    assert!(search("no-such-text-anywhere", None)
        .await
        .unwrap()
        .items
        .is_empty());
}

/// D076 record: the 300 ms threshold applies to a 10,000-message database. The number printed here
/// is the evidence; the assertion only guards against an order-of-magnitude regression because
/// wall-clock thresholds are machine dependent.
#[tokio::test]
async fn like_search_over_ten_thousand_messages_is_timed() {
    let (_, chat) = seeded(1000, 10).await;
    let run = |query: &'static str| {
        let chat = chat.clone();
        async move {
            let started = Instant::now();
            let page = chat
                .search_sessions(
                    ALICE,
                    SearchChatSessionsInput {
                        query: query.into(),
                        cursor: None,
                        limit: Some(50),
                    },
                )
                .await
                .unwrap();
            (started.elapsed(), page.items.len())
        }
    };
    let (worst, hits) = run("zzqx-not-present").await;
    let (common, common_hits) = run("lorem").await;
    println!("D076 timing (debug build, in-memory, 10,000 messages): no-match scan {worst:?} ({hits} hits), common term {common:?} ({common_hits} hits)");
    assert_eq!((hits, common_hits), (0, 50));
    assert!(
        worst.as_millis() < 3000,
        "a full scan must stay far from unusable even in a debug build"
    );
}
