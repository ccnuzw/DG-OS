//! FR-006 AC28/AC30/D076 scale measurement: 10,000 and 100,000 chat messages in a file-backed
//! SQLite database migrated with the real migrations and read/written through the real
//! `ChatManager` / `ChatRepository` APIs. Nothing here is a Provider test and nothing is asserted
//! about wall-clock time: the printed table is the evidence. The tests are `#[ignore]` so the
//! normal `cargo test` stays fast. Run:
//!
//! ```text
//! cargo test -p dgos-workbench --test chat_scale --offline -- --ignored --nocapture
//! cargo test -p dgos-workbench --test chat_scale --offline --release -- --ignored --nocapture
//! ```
//!
//! Method notes
//! * Seeding is raw batched multi-row `INSERT` (200 rows per statement) inside one transaction.
//!   That is a seeding shortcut only; every measured operation uses the production repository code.
//! * After seeding the pool is dropped and re-opened (`establish_connection`, the production
//!   entry point with `secure_delete = ON`), so SQLite's per-connection page cache starts cold.
//!   The OS file cache is not flushed.
//! * Each operation reports `1st` (its first call, untimed in the percentiles) and then
//!   p50/p95/max over N timed repetitions (N >= 30). Percentiles are nearest-rank.
//! * Two users (`alice` owns 9 of 10 sessions, `bob` owns every 10th) exercise isolation.
//!   Session 0 is the "big" session (a tenth of all messages) and has the oldest `updated_at`.
//! * Tests share one process-wide lock so the two scales never overlap and disturb timings.

use dgos_db::{
    establish_connection,
    migrations::Migrator,
    sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait},
    sea_orm_migration::MigratorTrait,
    BeginGeneration, ChatRepository, FinalizeGeneration, Finalized,
};
use dgos_workbench::{
    ChatManager, ChatSearchPage, DeleteChatSessionInput, ListChatMessagesInput,
    ListChatSessionsInput, SearchChatSessionsInput,
};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const ALICE: &str = "alice";
const BOB: &str = "bob";
const BATCH: usize = 200;
/// Repetitions for cheap reads; heavier destructive/contention work uses the smaller counts.
const READ_REPS: usize = 50;
const SEARCH_REPS: usize = 30;
const HEAVY_REPS: usize = 30;
const CONTENTION_REPS: usize = 40;
/// Production context ceiling (D018/D075), copied to drive `history` the way `regenerate` does.
const MAX_CONTEXT_BYTES: usize = 24 * 1024;

const NO_MATCH: &str = "zzqx-not-present";
const RARE_OLD: &str = "zqxrareold7731";
const RARE_NEW: &str = "zqxrarenew4482";
const RARE_CJK: &str = "龘";
const COMMON_CJK: &str = "中";
const LITERAL: &str = "50%_off\\sale";
/// Matches `LITERAL` only if `%`/`_`/`\` were treated as wildcards.
const LITERAL_DECOY: &str = "50abcXoffsale";
const BOB_NEEDLE: &str = "bobonlyneedle9";

static SERIAL: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------------------------
// Layout and seeding

#[derive(Clone, Copy)]
struct Layout {
    sessions: usize,
    messages: usize,
    /// Messages in session 0.
    big: usize,
}

impl Layout {
    fn new(sessions: usize, messages: usize) -> Self {
        Self {
            sessions,
            messages,
            big: messages / 10,
        }
    }
    fn is_bob(&self, s: usize) -> bool {
        s % 10 == 9
    }
    fn user(&self, s: usize) -> &'static str {
        if self.is_bob(s) {
            BOB
        } else {
            ALICE
        }
    }
    fn count(&self, s: usize) -> usize {
        if s == 0 {
            return self.big;
        }
        let rest = self.messages - self.big;
        let n = self.sessions - 1;
        rest / n + usize::from(s - 1 < rest % n)
    }
    fn newest_alice(&self) -> usize {
        self.sessions - 2
    }
    /// Needle appended to a given message, if any.
    fn needle(&self, s: usize, m: usize) -> Option<&'static str> {
        match (s, m) {
            (1, 3) => Some(RARE_OLD),
            (3, 1) => Some(RARE_CJK),
            (4, 2) => Some(LITERAL),
            (2, 1) => Some(LITERAL_DECOY),
            (9, 0) => Some(BOB_NEEDLE),
            _ if s == self.newest_alice() && m == 2 => Some(RARE_NEW),
            _ => None,
        }
    }
}

/// Production ids are UUIDs and the cursor parser requires hex digits and dashes.
fn sid(s: usize) -> String {
    format!("00000000-0000-4000-8000-{s:012x}")
}

fn victim_sid(k: usize) -> String {
    format!("00000000-0000-4000-8001-{k:012x}")
}

fn content(lay: &Layout, s: usize, m: usize) -> String {
    let mid = if (s * 31 + m) % 100 == 0 {
        " midfreq"
    } else {
        ""
    };
    let mut c = format!(
        "会话{s}第{m}条 lorem ipsum dolor sit amet, consectetur adipiscing elit{mid}. \
         这是一段用于规模测试的中文内容,包含常用汉字、数字 0123456789 与 English words 以便模拟真实对话长度。"
    );
    if let Some(needle) = lay.needle(s, m) {
        c.push(' ');
        c.push_str(needle);
    }
    c
}

async fn flush(tx: &impl ConnectionTrait, head: &str, rows: &mut Vec<String>, force: bool) {
    if rows.len() >= BATCH || (force && !rows.is_empty()) {
        tx.execute_unprepared(&format!("{head} VALUES {}", rows.join(",")))
            .await
            .unwrap();
        rows.clear();
    }
}

async fn seed(db: &DatabaseConnection, lay: &Layout) {
    let tx = db.begin().await.unwrap();
    let mut rows = Vec::with_capacity(BATCH);
    let sessions_head = "INSERT INTO chat_sessions(id,user_id,title,created_at,updated_at)";
    // Several sessions share one `updated_at` second so ties order by id.
    for s in 0..lay.sessions {
        rows.push(format!(
            "('{}','{}','项目会话 {s}',1,{})",
            sid(s),
            lay.user(s),
            1_000_000 + (s / 7) as i64
        ));
        flush(&tx, sessions_head, &mut rows, false).await;
    }
    flush(&tx, sessions_head, &mut rows, true).await;

    let tasks_head = "INSERT INTO workbench_tasks(id,user_id,app_id,provider_id,model_id,status,\
        created_at,finished_at,error_code,total_tokens,provider_config_revision,session_id,kind,\
        endpoint_fingerprint)";
    for s in 0..lay.sessions {
        for p in 0..lay.count(s).div_ceil(2) {
            rows.push(format!(
                "('t{s}-{p}','{}','ai-workbench','p','m','succeeded',{},{},NULL,100,1,'{}','chat','fp-bench')",
                lay.user(s),
                1000 + p,
                1001 + p,
                sid(s)
            ));
            flush(&tx, tasks_head, &mut rows, false).await;
        }
    }
    flush(&tx, tasks_head, &mut rows, true).await;

    let messages_head = "INSERT INTO chat_messages(id,session_id,user_id,seq,role,content,status,\
        task_id,created_at,updated_at)";
    for s in 0..lay.sessions {
        for m in 0..lay.count(s) {
            rows.push(format!(
                "('m{s}-{m}','{}','{}',{},'{}','{}','completed','t{s}-{}',{},{})",
                sid(s),
                lay.user(s),
                m + 1,
                if m % 2 == 0 { "user" } else { "assistant" },
                content(lay, s, m),
                m / 2,
                1000 + m,
                1000 + m
            ));
            flush(&tx, messages_head, &mut rows, false).await;
        }
    }
    flush(&tx, messages_head, &mut rows, true).await;
    tx.commit().await.unwrap();
}

/// Copy a session with all its messages and task rows (ids prefixed) via `INSERT .. SELECT`.
async fn clone_session(db: &DatabaseConnection, from: &str, to: &str, prefix: &str, user: &str) {
    let tx = db.begin().await.unwrap();
    tx.execute_unprepared(&format!(
        "INSERT INTO chat_sessions(id,user_id,title,created_at,updated_at) \
         VALUES('{to}','{user}','clone',1,1);\
         INSERT INTO workbench_tasks(id,user_id,app_id,provider_id,model_id,status,created_at,\
           finished_at,error_code,total_tokens,provider_config_revision,session_id,kind,\
           endpoint_fingerprint) \
         SELECT '{prefix}'||id,user_id,app_id,provider_id,model_id,status,created_at,finished_at,\
           error_code,total_tokens,provider_config_revision,'{to}',kind,endpoint_fingerprint \
         FROM workbench_tasks WHERE session_id='{from}';\
         INSERT INTO chat_messages(id,session_id,user_id,seq,role,content,status,task_id,\
           created_at,updated_at) \
         SELECT '{prefix}'||id,'{to}',user_id,seq,role,content,status,\
           CASE WHEN task_id IS NULL THEN NULL ELSE '{prefix}'||task_id END,created_at,updated_at \
         FROM chat_messages WHERE session_id='{from}';"
    ))
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

// ---------------------------------------------------------------------------------------------
// Measurement helpers

struct Row {
    name: String,
    first: f64,
    p50: f64,
    p95: f64,
    max: f64,
    n: usize,
    note: String,
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    let rank = ((p * n as f64).ceil() as usize).clamp(1, n);
    sorted[rank - 1]
}

fn summarize(name: &str, first: f64, mut samples: Vec<f64>, note: String) -> Row {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Row {
        name: name.to_owned(),
        first,
        p50: pct(&samples, 0.50),
        p95: pct(&samples, 0.95),
        max: *samples.last().unwrap(),
        n: samples.len(),
        note,
    }
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// Run `$body` once as the untimed-in-percentiles first call, then `$reps` timed repetitions.
/// `$i` is the 0-based call index (0 = first call). Evaluates to the last output.
macro_rules! bench {
    ($rows:expr, $name:expr, $reps:expr, $i:ident, $body:expr) => {{
        let mut samples: Vec<f64> = Vec::with_capacity($reps);
        let mut first = 0.0;
        let mut last = None;
        for $i in 0..=$reps {
            let started = Instant::now();
            let out = $body.await;
            let took = ms(started);
            if $i == 0 {
                first = took;
            } else {
                samples.push(took);
            }
            last = Some(out);
        }
        $rows.push(summarize($name, first, samples, String::new()));
        last.unwrap()
    }};
}

fn note(rows: &mut [Row], text: String) {
    rows.last_mut().unwrap().note = text;
}

fn flag(r: &Row) -> &'static str {
    if r.p50 > 300.0 {
        "p50>300"
    } else if r.p95 > 300.0 {
        "p95>300"
    } else if r.max > 300.0 {
        "max>300"
    } else {
        ""
    }
}

fn print_table(title: &str, rows: &[Row]) {
    println!("{title}");
    println!(
        "{:<46} {:>3} {:>10} {:>10} {:>10} {:>10} {:<8} note",
        "operation", "N", "1st ms", "p50 ms", "p95 ms", "max ms", "flag"
    );
    for r in rows {
        println!(
            "{:<46} {:>3} {:>10.2} {:>10.2} {:>10.2} {:>10.2} {:<8} {}",
            r.name,
            r.n,
            r.first,
            r.p50,
            r.p95,
            r.max,
            flag(r),
            r.note
        );
    }
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn file_len(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn db_size(path: &std::path::Path) -> String {
    let wal = path.with_file_name("dgos.db-wal");
    format!(
        "{:.2} MiB main + {:.2} MiB wal",
        mib(file_len(path)),
        mib(file_len(&wal))
    )
}

async fn pragma_text(db: &DatabaseConnection, pragma: &str) -> String {
    db.query_one(Statement::from_string(
        DbBackend::Sqlite,
        format!("PRAGMA {pragma}"),
    ))
    .await
    .ok()
    .flatten()
    .and_then(|row| {
        row.try_get_by_index::<String>(0)
            .ok()
            .or_else(|| row.try_get_by_index::<i64>(0).ok().map(|n| n.to_string()))
    })
    .unwrap_or_else(|| "?".into())
}

async fn search(
    chat: &ChatManager,
    user: &str,
    query: &str,
    cursor: Option<String>,
) -> ChatSearchPage {
    chat.search_sessions(
        user,
        SearchChatSessionsInput {
            query: query.to_owned(),
            cursor,
            limit: Some(50),
        },
    )
    .await
    .unwrap()
}

// ---------------------------------------------------------------------------------------------
// The scenario

async fn run(lay: Layout) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dgos.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());

    let seeding = Instant::now();
    {
        let db = establish_connection(&url).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        seed(&db, &lay).await;
        let _ = db
            .execute_unprepared("PRAGMA wal_checkpoint(TRUNCATE)")
            .await;
    }
    let seed_ms = ms(seeding);
    let db = establish_connection(&url).await.unwrap();
    let chat = ChatManager::new(db.clone());
    let repo = ChatRepository::new(db.clone());
    let journal = pragma_text(&db, "journal_mode").await;
    let busy = pragma_text(&db, "busy_timeout").await;
    let size_seeded = db_size(&path);
    let total_rows: i64 = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) FROM chat_messages",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index(0)
        .unwrap();
    assert_eq!(total_rows as usize, lay.messages, "seeded message count");

    let mut rows: Vec<Row> = Vec::new();
    let mut facts: Vec<String> = Vec::new();
    let big = sid(0);

    // ---- sessions: caps, isolation, first / middle / deepest keyset page ----------------------
    let expected_alice: HashSet<String> = (0..lay.sessions)
        .filter(|s| !lay.is_bob(*s))
        .map(sid)
        .collect();
    let mut cursors: Vec<Option<String>> = Vec::new();
    let mut seen = HashSet::new();
    let mut max_page = 0;
    let mut cursor = None;
    loop {
        cursors.push(cursor.clone());
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
        max_page = max_page.max(page.items.len());
        for s in page.items {
            assert!(seen.insert(s.id), "no repeats across session pages");
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(seen, expected_alice, "alice sees exactly her sessions");
    assert!(max_page <= 50);
    facts.push(format!(
        "sessions: requested limit 10000 -> max page {max_page} (cap 50); {} pages, {} alice sessions, no repeats, bob's sessions absent",
        cursors.len(),
        seen.len()
    ));
    let bob_sessions = chat
        .list_sessions(
            BOB,
            ListChatSessionsInput {
                cursor: None,
                limit: Some(50),
            },
        )
        .await
        .unwrap();
    assert!(bob_sessions
        .items
        .iter()
        .all(|s| !expected_alice.contains(&s.id)));
    for (label, at) in [
        ("list_sessions first page (limit 50)", 0),
        ("list_sessions middle keyset page", cursors.len() / 2),
        ("list_sessions deepest keyset page", cursors.len() - 1),
    ] {
        let cur = cursors[at].clone();
        let page = bench!(rows, label, READ_REPS, _i, {
            let chat = chat.clone();
            let cur = cur.clone();
            async move {
                chat.list_sessions(
                    ALICE,
                    ListChatSessionsInput {
                        cursor: cur,
                        limit: Some(50),
                    },
                )
                .await
                .unwrap()
            }
        });
        assert!(page.items.len() <= 50);
        note(
            &mut rows,
            format!(
                "page {}/{} items={} more={}",
                at + 1,
                cursors.len(),
                page.items.len(),
                page.next_cursor.is_some()
            ),
        );
    }

    // ---- messages in the big session ------------------------------------------------------------
    let mut collected: Vec<i64> = Vec::new();
    let mut before = None;
    let mut max_msgs = 0;
    let mut msg_pages = 0;
    loop {
        let page = chat
            .list_messages(
                ALICE,
                ListChatMessagesInput {
                    session_id: big.clone(),
                    before_seq: before,
                    limit: Some(9999),
                },
            )
            .await
            .unwrap();
        max_msgs = max_msgs.max(page.items.len());
        msg_pages += 1;
        let mut seqs: Vec<i64> = page.items.iter().map(|m| m.seq).collect();
        seqs.extend(collected);
        collected = seqs;
        match page.next_before_seq {
            Some(next) => before = Some(next),
            None => break,
        }
    }
    assert!(max_msgs <= 100);
    assert_eq!(collected, (1..=lay.big as i64).collect::<Vec<_>>());
    let default_page = chat
        .list_messages(
            ALICE,
            ListChatMessagesInput {
                session_id: big.clone(),
                before_seq: None,
                limit: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(default_page.items.len(), 50, "default message page is 50");
    let stranger = chat
        .list_messages(
            BOB,
            ListChatMessagesInput {
                session_id: big.clone(),
                before_seq: None,
                limit: Some(10),
            },
        )
        .await;
    assert!(stranger.is_err(), "bob cannot read alice's session");
    facts.push(format!(
        "messages: big session {} msgs, requested limit 9999 -> max page {max_msgs} (cap 100), default page {}, {msg_pages} pages, every seq once in order; bob reading alice's session -> error",
        lay.big,
        default_page.items.len()
    ));
    for (label, before_seq) in [
        ("list_messages newest page (limit 100)", None),
        ("list_messages mid before_seq", Some(lay.big as i64 / 2)),
        ("list_messages deepest before_seq=101", Some(101)),
    ] {
        let page = bench!(rows, label, READ_REPS, _i, {
            let chat = chat.clone();
            let session_id = big.clone();
            async move {
                chat.list_messages(
                    ALICE,
                    ListChatMessagesInput {
                        session_id,
                        before_seq,
                        limit: Some(100),
                    },
                )
                .await
                .unwrap()
            }
        });
        assert!(page.items.len() <= 100);
        note(
            &mut rows,
            format!(
                "items={} more={}",
                page.items.len(),
                page.next_before_seq.is_some()
            ),
        );
    }

    // ---- search ----------------------------------------------------------------------------------
    // Correctness first (untimed): isolation and literal escaping.
    assert!(search(&chat, ALICE, BOB_NEEDLE, None)
        .await
        .items
        .is_empty());
    let bob_hit = search(&chat, BOB, BOB_NEEDLE, None).await;
    assert_eq!(bob_hit.items.len(), 1);
    assert_eq!(bob_hit.items[0].session.id, sid(9));
    let literal = search(&chat, ALICE, LITERAL, None).await;
    assert_eq!(
        literal
            .items
            .iter()
            .map(|h| h.session.id.clone())
            .collect::<Vec<_>>(),
        vec![sid(4)],
        "% _ \\ are literals: the wildcard decoy must not match"
    );
    facts.push("search isolation: bob-only needle -> 0 hits for alice, 1 for bob; literal `50%_off\\sale` -> only the real session (decoy `50abcXoffsale` not matched)".into());

    #[derive(Clone, Copy)]
    enum Expect {
        Exactly(usize),
        Capped,
        Any,
    }
    let cases: Vec<(&str, String, Expect)> = vec![
        (
            "search no-match (full scan)",
            NO_MATCH.into(),
            Expect::Exactly(0),
        ),
        (
            "search rare needle, oldest session",
            RARE_OLD.into(),
            Expect::Exactly(1),
        ),
        (
            "search rare needle, newest session",
            RARE_NEW.into(),
            Expect::Exactly(1),
        ),
        ("search ~1% term `midfreq`", "midfreq".into(), Expect::Any),
        ("search common word `lorem`", "lorem".into(), Expect::Capped),
        (
            "search 1-char Chinese common `中`",
            COMMON_CJK.into(),
            Expect::Capped,
        ),
        (
            "search 1-char Chinese rare `龘`",
            RARE_CJK.into(),
            Expect::Exactly(1),
        ),
        (
            "search literal `50%_off\\sale`",
            LITERAL.into(),
            Expect::Exactly(1),
        ),
    ];
    for (label, query, expect) in cases {
        let page = bench!(rows, label, SEARCH_REPS, _i, {
            let chat = chat.clone();
            let query = query.clone();
            async move { search(&chat, ALICE, &query, None).await }
        });
        match expect {
            Expect::Exactly(n) => assert_eq!(page.items.len(), n, "{label}"),
            Expect::Capped => {
                assert_eq!(page.items.len(), 50, "{label}");
                assert!(page.next_cursor.is_some());
            }
            Expect::Any => assert!(page.items.len() <= 50),
        }
        note(
            &mut rows,
            format!(
                "hits={} more={}",
                page.items.len(),
                page.next_cursor.is_some()
            ),
        );
    }
    // Search keyset: walk `lorem`, then time the deepest page.
    let mut search_cursors: Vec<Option<String>> = Vec::new();
    let mut hit_ids = HashSet::new();
    let mut max_hits = 0;
    let mut cursor = None;
    loop {
        search_cursors.push(cursor.clone());
        let page = search(&chat, ALICE, "lorem", cursor.clone()).await;
        max_hits = max_hits.max(page.items.len());
        for hit in page.items {
            assert!(hit_ids.insert(hit.session.id), "no session twice");
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert!(max_hits <= 50);
    assert_eq!(hit_ids, expected_alice);
    facts.push(format!(
        "search: `lorem` walk -> max page {max_hits} (cap 50), {} pages, {} sessions once each",
        search_cursors.len(),
        hit_ids.len()
    ));
    let deep = search_cursors.last().cloned().unwrap();
    let page = bench!(
        rows,
        "search `lorem` deepest keyset page",
        SEARCH_REPS,
        _i,
        {
            let chat = chat.clone();
            let deep = deep.clone();
            async move { search(&chat, ALICE, "lorem", deep).await }
        }
    );
    note(
        &mut rows,
        format!(
            "page {}/{} hits={}",
            search_cursors.len(),
            search_cursors.len(),
            page.items.len()
        ),
    );

    // ---- regeneration preconditions (repository calls, no Provider) ------------------------------
    let last_assistant = format!("m0-{}", lay.big - 1);
    let last_seq = lay.big as i64;
    let fresh_task = "00000000-0000-4000-8000-0000000fffff".to_owned();
    let probe = BeginGeneration {
        user: ALICE,
        session_id: &big,
        task_id: &fresh_task,
        user_message_id: "",
        assistant_message_id: "",
        user_content: None,
        regenerate_of: Some(&last_assistant),
        provider_id: "",
        model_id: "",
        provider_config_revision: None,
        endpoint_fingerprint: "",
        skill: None,
        tools: &[],
    };
    let missed = bench!(
        rows,
        "regen: session_of_task (miss)",
        READ_REPS,
        _i,
        repo.session_of_task(ALICE, &fresh_task)
    )
    .unwrap();
    assert!(missed.is_none());
    let msg = bench!(
        rows,
        "regen: get_message(last assistant)",
        READ_REPS,
        _i,
        repo.get_message(ALICE, &last_assistant)
    )
    .unwrap();
    assert_eq!((msg.role.as_str(), msg.seq), ("assistant", last_seq));
    let replay = bench!(
        rows,
        "regen: find_replay (miss)",
        READ_REPS,
        _i,
        repo.find_replay(&probe)
    )
    .unwrap();
    assert!(replay.is_none());
    let question = bench!(
        rows,
        "regen: question_before(last seq)",
        READ_REPS,
        _i,
        repo.question_before(ALICE, &big, last_seq)
    )
    .unwrap();
    assert_eq!(question.seq, last_seq - 1);
    bench!(
        rows,
        "regen: get_session (prepare)",
        READ_REPS,
        _i,
        repo.get_session(ALICE, &big)
    )
    .unwrap();
    let fp = bench!(
        rows,
        "regen: last_endpoint_fingerprint",
        READ_REPS,
        _i,
        repo.last_endpoint_fingerprint(ALICE, &big)
    )
    .unwrap();
    assert_eq!(fp.as_deref(), Some("fp-bench"));
    let history = bench!(
        rows,
        "regen: history(24 KiB window)",
        READ_REPS,
        _i,
        repo.history(ALICE, &big, Some(question.seq), MAX_CONTEXT_BYTES)
    )
    .unwrap();
    note(
        &mut rows,
        format!(
            "rows={} completed_replies_before={}",
            history.rows.len(),
            history.total_completed_replies
        ),
    );
    bench!(
        rows,
        "regen: all preconditions in sequence",
        READ_REPS,
        _i,
        async {
            repo.session_of_task(ALICE, &fresh_task).await.unwrap();
            repo.get_message(ALICE, &last_assistant).await.unwrap();
            repo.find_replay(&probe).await.unwrap();
            let q = repo.question_before(ALICE, &big, last_seq).await.unwrap();
            repo.get_session(ALICE, &big).await.unwrap();
            repo.last_endpoint_fingerprint(ALICE, &big).await.unwrap();
            repo.history(ALICE, &big, Some(q.seq), MAX_CONTEXT_BYTES)
                .await
                .unwrap()
        }
    );
    note(&mut rows, format!("big session = {} messages", lay.big));

    // ---- regeneration write path on a clone of the big session --------------------------------------
    let regen_sid = victim_sid(9_000);
    clone_session(&db, &big, &regen_sid, "rg-", ALICE).await;
    {
        let mut last_id = format!("rg-m0-{}", lay.big - 1);
        let mut begin_samples = Vec::new();
        let mut finalize_samples = Vec::new();
        let (mut begin_first, mut finalize_first) = (0.0, 0.0);
        for k in 0..=HEAVY_REPS {
            let task = format!("regen-task-{k}");
            let new_id = format!("regen-msg-{k}");
            let started = Instant::now();
            repo.begin_generation(&BeginGeneration {
                user: ALICE,
                session_id: &regen_sid,
                task_id: &task,
                user_message_id: "",
                assistant_message_id: &new_id,
                user_content: None,
                regenerate_of: Some(&last_id),
                provider_id: "p",
                model_id: "m",
                provider_config_revision: Some(1),
                endpoint_fingerprint: "fp-bench",
                skill: None,
                tools: &[],
            })
            .await
            .unwrap();
            let begin = ms(started);
            let started = Instant::now();
            let done = repo
                .finalize_generation(&FinalizeGeneration {
                    user: ALICE,
                    assistant_message_id: &new_id,
                    task_id: &task,
                    content: "regenerated 重新生成",
                    message_status: "completed",
                    finish_reason: Some("stop"),
                    message_error: None,
                    task_status: "succeeded",
                    task_error: None,
                    total_tokens: Some(5),
                })
                .await
                .unwrap();
            let finalize = ms(started);
            assert!(matches!(
                done,
                Finalized::Done {
                    discarded: false,
                    ..
                }
            ));
            if k == 0 {
                begin_first = begin;
                finalize_first = finalize;
            } else {
                begin_samples.push(begin);
                finalize_samples.push(finalize);
            }
            last_id = new_id;
        }
        rows.push(summarize(
            "regen write: begin_generation (regenerate)",
            begin_first,
            begin_samples,
            format!("on a {}-message clone", lay.big),
        ));
        rows.push(summarize(
            "regen write: finalize_generation (completed)",
            finalize_first,
            finalize_samples,
            "no concurrent load".into(),
        ));
    }

    // ---- delete of a large session --------------------------------------------------------------------
    for k in 0..=HEAVY_REPS {
        clone_session(&db, &big, &victim_sid(k), &format!("v{k}-"), ALICE).await;
    }
    bench!(
        rows,
        "delete_chat_session (large session)",
        HEAVY_REPS,
        i,
        {
            let chat = chat.clone();
            let session_id = victim_sid(i);
            async move {
                chat.delete_session(ALICE, DeleteChatSessionInput { session_id })
                    .await
                    .unwrap()
            }
        }
    );
    note(
        &mut rows,
        format!(
            "{} messages + {} task rows each",
            lay.big,
            lay.big.div_ceil(2)
        ),
    );
    let gone = chat
        .list_messages(
            ALICE,
            ListChatMessagesInput {
                session_id: victim_sid(HEAVY_REPS),
                before_seq: None,
                limit: Some(1),
            },
        )
        .await;
    assert!(gone.is_err(), "deleted session is gone");
    let left: i64 = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) FROM chat_messages WHERE id LIKE 'v%-m%'",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index(0)
        .unwrap();
    assert_eq!(left, 0, "no orphan messages after delete");

    // ---- contention: searches running while a terminal write is issued ---------------------------------
    let phases: [(usize, &str); 3] = [(0, "baseline"), (1, "1 searcher"), (4, "4 searchers")];
    {
        let tx = db.begin().await.unwrap();
        for k in 0..phases.len() * CONTENTION_REPS {
            tx.execute_unprepared(&format!(
                "INSERT INTO chat_sessions(id,user_id,title,created_at,updated_at) VALUES('cs{k}','cu{k}','c',1,1);\
                 INSERT INTO workbench_tasks(id,user_id,app_id,provider_id,model_id,status,created_at,\
                   session_id,kind,endpoint_fingerprint) \
                   VALUES('ct{k}','cu{k}','ai-workbench','p','m','running',1,'cs{k}','chat','fp-bench');\
                 INSERT INTO chat_messages(id,session_id,user_id,seq,role,content,status,task_id,created_at,updated_at) \
                   VALUES('cm{k}','cs{k}','cu{k}',1,'assistant','','generating','ct{k}',1,1);"
            ))
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
    }
    let mut lcg: u64 = 0x2545_F491_4F6C_DD1D;
    for (phase, (searchers, label)) in phases.iter().enumerate() {
        let stop = Arc::new(AtomicBool::new(false));
        let handles: Vec<_> = (0..*searchers)
            .map(|_| {
                let chat = chat.clone();
                let stop = stop.clone();
                tokio::spawn(async move {
                    let mut durations = Vec::new();
                    while !stop.load(Ordering::Relaxed) {
                        let started = Instant::now();
                        search(&chat, ALICE, NO_MATCH, None).await;
                        durations.push(ms(started));
                    }
                    durations
                })
            })
            .collect();
        if *searchers > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let mut waits = Vec::new();
        let mut failures = Vec::new();
        for k in 0..CONTENTION_REPS {
            let idx = phase * CONTENTION_REPS + k;
            lcg = lcg
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            // Random offset so the write lands at varying points of the running scans.
            tokio::time::sleep(Duration::from_millis(3 + (lcg >> 33) % 40)).await;
            let (message, task) = (format!("cm{idx}"), format!("ct{idx}"));
            let user = format!("cu{idx}");
            let started = Instant::now();
            let result = repo
                .finalize_generation(&FinalizeGeneration {
                    user: &user,
                    assistant_message_id: &message,
                    task_id: &task,
                    content: "done 完成",
                    message_status: "completed",
                    finish_reason: Some("stop"),
                    message_error: None,
                    task_status: "succeeded",
                    task_error: None,
                    total_tokens: Some(3),
                })
                .await;
            waits.push(ms(started));
            match result {
                Ok(Finalized::Done { .. }) => {}
                Ok(Finalized::AlreadyFinalized) => failures.push("AlreadyFinalized".to_owned()),
                Err(error) => failures.push(format!("{error:?}")),
            }
        }
        stop.store(true, Ordering::Relaxed);
        let mut searches = Vec::new();
        for handle in handles {
            searches.extend(handle.await.unwrap());
        }
        let name = format!("contention write ({label})");
        rows.push(summarize(
            &name,
            waits[0],
            waits.clone(),
            format!("failures={}", failures.len()),
        ));
        if !failures.is_empty() {
            facts.push(format!(
                "contention {label}: finalize failures {failures:?}"
            ));
        }
        if !searches.is_empty() {
            let count = searches.len();
            let first = searches[0];
            rows.push(summarize(
                &format!("contention search ({label})"),
                first,
                searches,
                format!("{count} searches completed during the phase"),
            ));
        }
    }

    println!();
    println!(
        "=== chat_scale | build={} | {} messages / {} sessions / 2 users (big session {} msgs) ===",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        lay.messages,
        lay.sessions,
        lay.big
    );
    println!(
        "seed {:.0} ms | journal_mode={journal} busy_timeout={busy}ms | db after seed+reopen: {size_seeded} | at end: {}",
        seed_ms,
        db_size(&path)
    );
    for line in &facts {
        println!("  - {line}");
    }
    print_table("", &rows);
}

fn scale(sessions: usize, messages: usize) {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(run(Layout::new(sessions, messages)));
}

/// 1,000 sessions, 10,000 messages (session 0 holds 1,000 of them).
#[test]
#[ignore = "scale measurement; run explicitly with --ignored --nocapture"]
fn scale_10k_messages() {
    scale(1_000, 10_000);
}

/// 2,000 sessions, 100,000 messages (session 0 holds 10,000 of them).
#[test]
#[ignore = "scale measurement; run explicitly with --ignored --nocapture"]
fn scale_100k_messages() {
    scale(2_000, 100_000);
}
