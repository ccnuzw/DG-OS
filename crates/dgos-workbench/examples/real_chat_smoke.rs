//! Explicit opt-in smoke and budget calibration for the FR-006 chat workbench against REAL
//! OpenAI-compatible and Anthropic-compatible services. It drives the real `ChatManager`
//! (send, run_generation, cancel, cancel_window, regenerate, skill pinning), not the adapter.
//!
//! Configuration is by command-line parameters; keys are typed at a hidden prompt:
//!
//!   cargo run -p dgos-workbench --example real_chat_smoke --offline -- [options]
//!
//!   --base-url URL            both services (default: the gateway named in `DEFAULT_BASE_URL`)
//!   --openai-base-url URL     OpenAI-compatible service only
//!   --anthropic-base-url URL  Anthropic-compatible service only
//!   --openai-model ID         default `gpt-6.1-sol`
//!   --anthropic-model ID      default `claude-sonnet-5-5`
//!   --only openai|anthropic   run one service
//!   --stub                    also run the loopback stub wiring check (see below)
//!
//! After parsing, the program asks for each selected service's key on the terminal with echo
//! off. Pressing Enter without a key skips that service, which then prints `NOT RUN` and is
//! never counted as a pass. Keys are never accepted as arguments, because arguments are visible
//! to other local processes. If the prompt is interrupted with Ctrl-C, run `stty echo` to restore
//! the terminal. Prompts go to stderr, so stdout holds only the report.
//!
//! Isolation: keys exist only in process memory; `HOME` and the `XDG_*` directories are
//! redirected into a temp directory, each service gets its own temp SQLite database, and the key
//! lives only in `MemoryCredentialVault` (dgos-providers
//! `test-support`). No OS keychain or credential file is touched. At the end the whole temp tree
//! (database, WAL, skill install) is scanned for the key; any hit is a FAIL.
//!
//! Output: only step names, PASS / FAIL / INFO / NOT RUN, status enums, finish reasons, token
//! counts, byte counts, durations and error codes. Reply text, prompts, URLs and keys are never
//! printed. Prompts are minimal ("Reply with the single word: pong") except where a step needs a
//! long answer (cancel, max_tokens) or a long input (calibration, history overflow).
//!
//! Steps per service:
//!   1 streamed send (Started/Delta/Done counts)      2 second turn recalls a nonce (history)
//!   3 DB check (message, task, usage)                4 cancel mid-stream, cancel before the first
//!                                                      delta, closed sink, cancel_window
//!   5 timeout (NOT RUN, see below) and an unknown model id
//!   6 regenerate success replaces; regenerate + cancel keeps the old reply
//!   7 pinned bundled skill: disable / tamper -> skill_unavailable
//!   8 calibration: input size ladder, 16 KiB boundary, history overflow, tiny context window,
//!     max_tokens=1024 length test, reasoning text not persisted, key not on disk
//! MCP is always NOT RUN: this harness starts no MCP server.
//!
//! `--stub` additionally runs the same steps against a loopback SSE stub started
//! inside this file. That run is a *stub wiring check, not real Provider evidence*; its lines are
//! tagged `[stub]`. It exists only to prove the harness itself works.
//!
//! Exit code: 0 when nothing failed (including when everything is NOT RUN), 1 on any FAIL.
//!
//! Things this smoke cannot observe (reported as such, not guessed):
//! * The assembled request is private to `ChatManager`; input bytes are computed from what is
//!   sent plus the host policy length parsed from `src/lib.rs` (upstream prompt tokens come from
//!   the provider usage log).
//! * `ProviderManager` hard-codes the 120 s stream idle timeout and `ChatManager` the 10 minute
//!   generation limit, so an idle timeout cannot be provoked in a bounded run.

use dgos_db::{
    migrations::Migrator,
    sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement},
    sea_orm_migration::MigratorTrait,
    ChatMessage,
};
use dgos_providers::{
    test_support::{seed_provider, MemoryCredentialVault},
    ProviderManager,
};
use dgos_skills::SkillsManager;
use dgos_types::ProviderConfig;
use dgos_workbench::{
    ChatError, ChatEvent, ChatManager, ChatSink, ChatTargetInput, CreateChatSessionInput,
    ListChatMessagesInput, RegenerateChatInput, SendAck, SendChatInput, Sent,
    UpdateChatSessionTargetInput,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        atomic::{AtomicI64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const USER: &str = "smoke-user";
const WINDOW: &str = "main";
const SKILL: &str = "dgos.official.writing.summarize";
const CEILING: usize = 24 * 1024;
const MAX_MESSAGE: usize = 16 * 1024;
const UNKNOWN_MODEL: &str = "dgos-smoke-no-such-model";
const STUB_REASONING: &str = "STUB-REASONING-MARKER-7f3a9c1e-must-never-be-persisted";

// ---------------------------------------------------------------------------------------------
// Configuration

/// The gateway the user named for these runs. Never printed; reports name services by label.
const DEFAULT_BASE_URL: &str = "https://cc.nextcc.cc";
const DEFAULT_OPENAI_MODEL: &str = "gpt-6.1-sol";
const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-5-5";

const USAGE: &str = "usage: real_chat_smoke [--base-url URL] [--openai-base-url URL] \
[--anthropic-base-url URL] [--openai-model ID] [--anthropic-model ID] \
[--only openai|anthropic] [--stub]\nKeys are asked for on the terminal; they are never arguments.";

struct Args {
    openai_base_url: String,
    anthropic_base_url: String,
    openai_model: String,
    anthropic_model: String,
    openai: bool,
    anthropic: bool,
    stub: bool,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut parsed = Args {
        openai_base_url: DEFAULT_BASE_URL.into(),
        anthropic_base_url: DEFAULT_BASE_URL.into(),
        openai_model: DEFAULT_OPENAI_MODEL.into(),
        anthropic_model: DEFAULT_ANTHROPIC_MODEL.into(),
        openai: true,
        anthropic: true,
        stub: false,
    };
    while let Some(flag) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .filter(|value| !value.trim().is_empty() && !value.starts_with("--"))
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match flag.as_str() {
            "--base-url" => {
                let url = value("--base-url")?;
                parsed.openai_base_url = url.clone();
                parsed.anthropic_base_url = url;
            }
            "--openai-base-url" => parsed.openai_base_url = value("--openai-base-url")?,
            "--anthropic-base-url" => parsed.anthropic_base_url = value("--anthropic-base-url")?,
            "--openai-model" => parsed.openai_model = value("--openai-model")?,
            "--anthropic-model" => parsed.anthropic_model = value("--anthropic-model")?,
            "--only" => match value("--only")?.as_str() {
                "openai" => parsed.anthropic = false,
                "anthropic" => parsed.openai = false,
                _ => return Err("--only takes openai or anthropic".into()),
            },
            "--stub" => parsed.stub = true,
            "--key" | "--openai-key" | "--anthropic-key" => {
                return Err(
                    "keys are not accepted as arguments; they are asked for on the terminal".into(),
                )
            }
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(parsed)
}

/// Reads one line from the terminal with echo turned off (`stty -echo`). When stdin is not a
/// terminal (piped input) the line is read as is; when echo cannot be turned off, a warning says
/// the input will be visible. The prompt goes to stderr.
fn read_secret(prompt: &str) -> String {
    use std::io::{BufRead, IsTerminal, Write};
    use std::process::{Command, Stdio};
    let stty = |arg: &str| {
        Command::new("stty")
            .arg(arg)
            .stdin(Stdio::inherit())
            .status()
            .is_ok_and(|status| status.success())
    };
    let terminal = std::io::stdin().is_terminal();
    let hidden = terminal && stty("-echo");
    if terminal && !hidden {
        eprintln!("warning: could not turn off terminal echo; the key will be visible as you type");
    }
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    if hidden {
        stty("echo");
        eprintln!();
    }
    line.trim().to_owned()
}

struct Svc {
    label: &'static str,
    provider_type: &'static str,
    base_url: String,
    key: String,
    model: String,
    stub: Option<Arc<StubState>>,
}

/// One real service when the user typed a key for it; otherwise `NOT RUN`.
fn service(
    label: &'static str,
    provider_type: &'static str,
    base_url: String,
    model: String,
    selected: bool,
) -> Option<Svc> {
    if !selected {
        println!("NOT RUN {label}: excluded by --only");
        return None;
    }
    let key = read_secret(&format!("{label} key (Enter to skip): "));
    if key.is_empty() {
        println!("NOT RUN {label}: no key entered");
        return None;
    }
    Some(Svc {
        label,
        provider_type,
        base_url,
        key,
        model,
        stub: None,
    })
}

// ---------------------------------------------------------------------------------------------
// Event recorder (the sink)

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Plain,
    /// `ChatManager::cancel` from inside the sink on `Started` (before any upstream delta).
    CancelOnStarted,
    /// `ChatManager::cancel` from inside the sink on the n-th delta.
    CancelAt(usize),
    /// The sink returns `false` on the n-th delta: the page is gone.
    CloseAt(usize),
    /// A watcher calls `cancel_window` once n deltas arrived (page reload / window destroyed).
    WindowAt(usize),
}

enum Term {
    Done {
        status: String,
        finish: Option<String>,
        bytes: usize,
        tokens: Option<i64>,
        unknown: bool,
    },
    Error {
        code: String,
        provider_code: Option<u32>,
        unknown: bool,
        discarded: bool,
    },
}

#[derive(Default)]
struct Inner {
    started: bool,
    task_id: Option<String>,
    deltas: usize,
    delta_bytes: usize,
    reasoning_events: usize,
    reasoning_text: String,
    first_delta_ms: Option<u128>,
    ended_ms: u128,
    term: Option<Term>,
}

struct Rec {
    mode: Mode,
    chat: ChatManager,
    t0: Instant,
    inner: Mutex<Inner>,
}

impl ChatSink for Rec {
    fn send(&self, event: ChatEvent) -> bool {
        let mut keep = true;
        let mut cancel: Option<String> = None;
        {
            let mut g = self.inner.lock().unwrap();
            match event {
                ChatEvent::Started { task_id, .. } => {
                    g.started = true;
                    if self.mode == Mode::CancelOnStarted {
                        cancel = Some(task_id.clone());
                    }
                    g.task_id = Some(task_id);
                }
                ChatEvent::Delta { text, .. } => {
                    g.deltas += 1;
                    g.delta_bytes += text.len();
                    if g.first_delta_ms.is_none() {
                        g.first_delta_ms = Some(self.t0.elapsed().as_millis());
                    }
                    match self.mode {
                        Mode::CancelAt(n) if g.deltas == n => cancel = g.task_id.clone(),
                        Mode::CloseAt(n) if g.deltas == n => keep = false,
                        _ => {}
                    }
                }
                ChatEvent::ReasoningDelta { text, .. } => {
                    g.reasoning_events += 1;
                    g.reasoning_text.push_str(&text);
                }
                ChatEvent::Done { message } => {
                    g.term = Some(Term::Done {
                        status: message.status.clone(),
                        finish: message.finish_reason.clone(),
                        bytes: message.content.len(),
                        tokens: message.total_tokens,
                        unknown: message.result_unknown,
                    });
                }
                ChatEvent::Error {
                    error_code,
                    provider_code,
                    result_unknown,
                    regeneration_discarded,
                    ..
                } => {
                    g.term = Some(Term::Error {
                        code: error_code,
                        provider_code,
                        unknown: result_unknown,
                        discarded: regeneration_discarded,
                    });
                }
                _ => {}
            }
            if g.term.is_some() {
                g.ended_ms = self.t0.elapsed().as_millis();
            }
        }
        if let Some(task_id) = cancel {
            let _ = self.chat.cancel(USER, &task_id);
        }
        keep
    }
}

fn term_text(term: &Option<Term>) -> String {
    match term {
        None => "terminal=none".into(),
        Some(Term::Done {
            status,
            finish,
            bytes,
            tokens,
            unknown,
        }) => format!(
            "terminal=done status={status} finish_reason={finish:?} reply_bytes={bytes} total_tokens={tokens:?} result_unknown={unknown}"
        ),
        Some(Term::Error {
            code,
            provider_code,
            unknown,
            discarded,
        }) => format!(
            "terminal=error code={code} provider_code={provider_code:?} result_unknown={unknown} regeneration_discarded={discarded}"
        ),
    }
}

/// `(request_tokens, response_tokens, success, error_code, usage_reported)` of one
/// `provider_usage_logs` row. `usage_reported` is `None` on a database without the D081 column.
type Usage = (i64, i64, i64, Option<String>, Option<i64>);

fn usage_text(usage: &Option<Usage>) -> String {
    match usage {
        None => "usage_row=none".into(),
        Some((req, resp, ok, err, reported)) => format!(
            "usage_row(prompt_tokens={req} completion_tokens={resp} success={ok} error={err:?} usage_reported={reported:?})"
        ),
    }
}

struct Run {
    ack: SendAck,
    inner: Inner,
    window_cancelled: usize,
    usage: Option<Usage>,
    ms: u128,
}

// ---------------------------------------------------------------------------------------------
// Environment of one service

struct Env {
    label: &'static str,
    key: String,
    model: String,
    root: PathBuf,
    db_path: PathBuf,
    db: DatabaseConnection,
    chat: ChatManager,
    providers: ProviderManager,
    skills: SkillsManager,
    vault: Arc<MemoryCredentialVault>,
    provider_id: String,
    fails: usize,
    /// Upstream behaviour the host handled correctly but that differs from the request (for
    /// example an ignored `max_tokens`). Counted and reported; not a failure of the host.
    warns: usize,
    reasoning: Vec<String>,
    nonces: Vec<String>,
    interruptions: Vec<(String, bool, bool)>,
}

impl Env {
    fn out(&self, step: &str, verdict: &str, detail: &str) {
        println!("[{}] {step}: {verdict} {detail}", self.label);
    }

    fn check(&mut self, step: &str, ok: bool, detail: &str) {
        if !ok {
            self.fails += 1;
        }
        self.out(step, if ok { "PASS" } else { "FAIL" }, detail);
    }

    fn info(&self, step: &str, detail: &str) {
        self.out(step, "INFO", detail);
    }

    fn warn(&mut self, step: &str, detail: &str) {
        self.warns += 1;
        self.out(step, "WARN", detail);
    }

    // ---- database ------------------------------------------------------------------------

    async fn one(&self, sql: &str) -> Option<sea_orm_row::Row> {
        self.db
            .query_one(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
            .await
            .ok()
            .flatten()
            .map(sea_orm_row::Row)
    }

    async fn int(&self, sql: &str) -> Option<i64> {
        self.one(sql).await.and_then(|row| row.int(0))
    }

    async fn usage_count(&self) -> i64 {
        self.int("SELECT COUNT(*) FROM provider_usage_logs")
            .await
            .unwrap_or(0)
    }

    async fn chat_task_count(&self) -> i64 {
        self.int("SELECT COUNT(*) FROM workbench_tasks WHERE kind = 'chat'")
            .await
            .unwrap_or(0)
    }

    /// Polls for a usage row newer than `before`; the interrupted-stream row is written from a
    /// spawned `Drop` task, so it can lag the terminal event.
    async fn wait_usage(&self, before: i64, wait_ms: u64) -> Option<Usage> {
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        loop {
            if self.usage_count().await > before {
                let row = self
                    .one("SELECT request_tokens, response_tokens, CAST(success AS INTEGER), error_code, usage_reported FROM provider_usage_logs ORDER BY rowid DESC LIMIT 1")
                    .await?;
                return Some((
                    row.int(0).unwrap_or(-1),
                    row.int(1).unwrap_or(-1),
                    row.int(2).unwrap_or(-1),
                    row.text(3),
                    row.int(4),
                ));
            }
            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// `(status, error_code, total_tokens)` of a task row. `task_id` comes from our own ack.
    async fn task_row(&self, task_id: &str) -> Option<(String, Option<String>, Option<i64>)> {
        let row = self
            .one(&format!(
                "SELECT status, error_code, total_tokens FROM workbench_tasks WHERE id = '{task_id}'"
            ))
            .await?;
        Some((row.text(0)?, row.text(1), row.int(2)))
    }

    async fn msgs(&self, session: &str) -> Vec<ChatMessage> {
        match self
            .chat
            .list_messages(
                USER,
                ListChatMessagesInput {
                    session_id: session.into(),
                    before_seq: None,
                    limit: Some(100),
                },
            )
            .await
        {
            Ok(page) => {
                let mut items = page.items;
                items.sort_by_key(|m| m.seq);
                items
            }
            Err(_) => vec![],
        }
    }

    // ---- sessions and turns --------------------------------------------------------------

    async fn new_session(&mut self, step: &str, skill: Option<&str>) -> Option<String> {
        let target = ChatTargetInput {
            provider_id: Some(self.provider_id.clone()),
            model_id: Some(self.model.clone()),
            skill_id: skill.map(str::to_owned),
            no_skill: skill.is_none(),
        };
        match self
            .chat
            .create_session(
                &self.providers,
                &self.skills,
                USER,
                CreateChatSessionInput {
                    target: Some(target),
                },
            )
            .await
        {
            Ok(session) => Some(session.id),
            Err(error) => {
                self.check(
                    step,
                    false,
                    &format!("create_session code={}", error.code()),
                );
                None
            }
        }
    }

    async fn send(&self, session: &str, text: String) -> Result<Sent, ChatError> {
        self.chat
            .send(
                &self.providers,
                &self.skills,
                USER,
                WINDOW,
                SendChatInput {
                    session_id: session.into(),
                    content: text,
                    client_request_id: uuid::Uuid::new_v4().to_string(),
                    accept_destination: None,
                    tools: vec![],
                },
            )
            .await
    }

    async fn turn(&mut self, session: &str, text: String, mode: Mode) -> Result<Run, ChatError> {
        let before = self.usage_count().await;
        let sent = self.send(session, text).await?;
        Ok(self.run_sent(sent, mode, before).await)
    }

    async fn regen(&mut self, message_id: &str, mode: Mode) -> Result<Run, ChatError> {
        let before = self.usage_count().await;
        let sent = self
            .chat
            .regenerate(
                &self.providers,
                &self.skills,
                USER,
                WINDOW,
                RegenerateChatInput {
                    message_id: message_id.into(),
                    client_request_id: uuid::Uuid::new_v4().to_string(),
                    accept_destination: None,
                },
            )
            .await?;
        Ok(self.run_sent(sent, mode, before).await)
    }

    async fn run_sent(&mut self, sent: Sent, mode: Mode, before: i64) -> Run {
        let Sent { ack, job } = sent;
        let t0 = Instant::now();
        let rec = Rec {
            mode,
            chat: self.chat.clone(),
            t0,
            inner: Mutex::new(Inner::default()),
        };
        let mut window_cancelled = 0usize;
        if let Some(job) = job {
            match mode {
                Mode::WindowAt(n) => {
                    let generation = self.chat.run_generation(&self.providers, job, &rec);
                    let watcher = async {
                        loop {
                            {
                                let g = rec.inner.lock().unwrap();
                                if g.deltas >= n || g.term.is_some() {
                                    break;
                                }
                            }
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                        let ended = rec.inner.lock().unwrap().term.is_some();
                        if !ended {
                            window_cancelled = self.chat.cancel_window(WINDOW);
                        }
                    };
                    tokio::join!(generation, watcher);
                }
                _ => self.chat.run_generation(&self.providers, job, &rec).await,
            }
        }
        let ms = t0.elapsed().as_millis();
        let inner = std::mem::take(&mut *rec.inner.lock().unwrap());
        if !inner.reasoning_text.is_empty() {
            self.reasoning.push(inner.reasoning_text.clone());
        }
        let waited = if inner.deltas > 0 || matches!(inner.term, Some(Term::Done { .. })) {
            3000
        } else {
            400
        };
        let usage = self.wait_usage(before, waited).await;
        Run {
            ack,
            inner,
            window_cancelled,
            usage,
            ms,
        }
    }

    fn err_text(error: &ChatError) -> String {
        format!("chat_error code={} name={}", error.code(), error.message())
    }

    // ---- steps -----------------------------------------------------------------------------

    async fn step_conversation(&mut self) {
        let Some(session) = self.new_session("1 session", None).await else {
            return;
        };
        // Cover update_target: clear and set provider/model on the session explicitly.
        let updated = self
            .chat
            .update_target(
                &self.providers,
                &self.skills,
                USER,
                UpdateChatSessionTargetInput {
                    session_id: session.clone(),
                    provider_id: Some(self.provider_id.clone()),
                    model_id: Some(self.model.clone()),
                    skill_id: None,
                    no_skill: true,
                },
            )
            .await;
        self.check(
            "1 update_target",
            updated.is_ok(),
            &format!("provider+model set on session ok={}", updated.is_ok()),
        );

        let nonce = format!("DGOSN-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        self.nonces.push(nonce.clone());
        let first = self
            .turn(
                &session,
                format!("Remember this token: {nonce}. Reply with the single word: pong"),
                Mode::Plain,
            )
            .await;
        let first = match first {
            Ok(run) => run,
            Err(error) => {
                self.check("1 stream", false, &Self::err_text(&error));
                return;
            }
        };
        let ok = first.inner.started
            && first.inner.deltas >= 1
            && matches!(first.inner.term, Some(Term::Done { .. }));
        self.check(
            "1 stream",
            ok,
            &format!(
                "started={} deltas={} delta_bytes={} reasoning_events={} first_delta_ms={:?} total_ms={} dropped_turns={} {}",
                first.inner.started,
                first.inner.deltas,
                first.inner.delta_bytes,
                first.inner.reasoning_events,
                first.inner.first_delta_ms,
                first.ms,
                first.ack.dropped_turns,
                term_text(&first.inner.term)
            ),
        );

        // 3: DB check of turn 1.
        let messages = self.msgs(&session).await;
        let reply = messages
            .iter()
            .find(|m| m.id == first.ack.assistant_message_id);
        let task = self.task_row(&first.ack.task_id).await;
        match (reply, task) {
            (Some(reply), Some((task_status, task_error, task_tokens))) => {
                let ok = reply.status == "completed"
                    && reply.finish_reason.is_some()
                    && task_status == "succeeded"
                    && task_error.is_none();
                self.check(
                    "3 db",
                    ok,
                    &format!(
                        "message_status={} finish_reason={:?} message_total_tokens={:?} result_unknown={} task_status={task_status} task_error={task_error:?} task_total_tokens={task_tokens:?} reply_bytes={}",
                        reply.status,
                        reply.finish_reason,
                        reply.total_tokens,
                        reply.result_unknown,
                        reply.content.len()
                    ),
                );
                if reply.total_tokens.is_none() {
                    self.info(
                        "3 usage",
                        "usage_missing=true (total_tokens NULL; upstream sent no usable usage)",
                    );
                }
                self.info("3 usage_log", &usage_text(&first.usage));
            }
            _ => self.check("3 db", false, "message or task row missing"),
        }

        // 2: second turn must carry the first turn.
        let second = self
            .turn(
                &session,
                "What was the token I asked you to remember? Reply with only the token.".into(),
                Mode::Plain,
            )
            .await;
        let second = match second {
            Ok(run) => run,
            Err(error) => {
                self.check("2 recall", false, &Self::err_text(&error));
                return;
            }
        };
        let messages = self.msgs(&session).await;
        let contains = messages
            .iter()
            .find(|m| m.id == second.ack.assistant_message_id)
            .is_some_and(|m| m.content.contains(&nonce));
        let carried = match (&first.usage, &second.usage) {
            (Some(a), Some(b)) => {
                format!("prompt_tokens_turn1={} prompt_tokens_turn2={}", a.0, b.0)
            }
            _ => "prompt_tokens=unknown".into(),
        };
        let grew = match (&first.usage, &second.usage) {
            (Some(a), Some(b)) => b.0 > a.0,
            _ => true,
        };
        self.check(
            "2 recall",
            second.ack.dropped_turns == 0
                && contains
                && grew
                && matches!(second.inner.term, Some(Term::Done { .. })),
            &format!(
                "contains_nonce={contains} dropped_turns={} {carried} deltas={} total_ms={} {}",
                second.ack.dropped_turns,
                second.inner.deltas,
                second.ms,
                term_text(&second.inner.term)
            ),
        );
    }

    async fn cancel_case(&mut self, name: &str, mode: Mode, prompt: &str) {
        let Some(session) = self.new_session(name, None).await else {
            return;
        };
        let (n, label) = match mode {
            Mode::CancelOnStarted => (0, "cancel_before_first_delta"),
            Mode::CancelAt(n) => (n, "cancel_after_delta"),
            Mode::CloseAt(n) => (n, "sink_closed_after_delta"),
            Mode::WindowAt(n) => (n, "cancel_window_after_delta"),
            Mode::Plain => (0, "plain"),
        };
        let run = match self.turn(&session, prompt.into(), mode).await {
            Ok(run) => run,
            Err(error) => {
                self.check(name, false, &Self::err_text(&error));
                return;
            }
        };
        let finished_early =
            matches!(run.inner.term, Some(Term::Done { .. })) && run.inner.deltas < n.max(1);
        if finished_early && mode != Mode::CancelOnStarted {
            self.info(
                name,
                &format!(
                    "{label} not exercised: reply ended after {} deltas before the cancel point {}",
                    run.inner.deltas,
                    term_text(&run.inner.term)
                ),
            );
            return;
        }
        let messages = self.msgs(&session).await;
        let reply = messages
            .iter()
            .find(|m| m.id == run.ack.assistant_message_id);
        let task = self.task_row(&run.ack.task_id).await;
        let active = self.chat.has_active_session(USER, &session);
        let (code_ok, unknown) = match &run.inner.term {
            Some(Term::Error { code, unknown, .. }) => (code == "task_cancelled", *unknown),
            _ => (false, false),
        };
        let (msg_ok, msg_text, usage_missing) = match reply {
            Some(m) => {
                let partial = m.content.len();
                // Interrupted with partial bytes, or failed with nothing received.
                let ok = (m.status == "interrupted" && partial > 0)
                    || (m.status == "failed" && partial == 0);
                (
                    ok,
                    format!(
                        "message_status={} partial_bytes={partial} message_error={:?} message_total_tokens={:?} message_result_unknown={}",
                        m.status, m.error_code, m.total_tokens, m.result_unknown
                    ),
                    m.total_tokens.is_none(),
                )
            }
            None => (false, "message=missing".into(), true),
        };
        let (task_ok, task_text) = match &task {
            Some((status, error, tokens)) => (
                status == "interrupted" && error.as_deref() == Some("task_cancelled"),
                format!("task_status={status} task_error={error:?} task_total_tokens={tokens:?}"),
            ),
            None => (false, "task=missing".into()),
        };
        let window_ok = mode != Mode::WindowAt(n) || run.window_cancelled >= 1;
        self.interruptions
            .push((name.to_owned(), usage_missing, unknown));
        self.check(
            name,
            code_ok && msg_ok && task_ok && !active && window_ok,
            &format!(
                "{label}={n} deltas_seen={} cancelled_windows={} {} {msg_text} {task_text} usage_missing={usage_missing} session_released={} {} first_delta_ms={:?} ended_ms={} total_ms={}",
                run.inner.deltas,
                run.window_cancelled,
                term_text(&run.inner.term),
                !active,
                usage_text(&run.usage),
                run.inner.first_delta_ms,
                run.inner.ended_ms,
                run.ms
            ),
        );
    }

    async fn step_cancel(&mut self) {
        let long = "Count from 1 to 400, one number per line, no other text.";
        self.cancel_case("4a cancel_mid_stream", Mode::CancelAt(2), long)
            .await;
        self.cancel_case("4b cancel_before_first_delta", Mode::CancelOnStarted, long)
            .await;
        self.cancel_case("4c sink_closed_mid_stream", Mode::CloseAt(2), long)
            .await;
        self.cancel_case("4d cancel_window_mid_stream", Mode::WindowAt(2), long)
            .await;
        let summary: Vec<String> = self
            .interruptions
            .iter()
            .map(|(name, missing, unknown)| {
                format!(
                    "{}:usage_missing={missing},result_unknown={unknown}",
                    &name[..2]
                )
            })
            .collect();
        self.info("4 interrupted_summary", &summary.join(" "));
    }

    async fn step_timeout_and_unknown_model(&mut self) {
        self.out(
            "5 idle_timeout",
            "NOT RUN",
            "ProviderManager builds adapters with AdapterOptions::default() (stream_idle_timeout 120 s) and ChatManager fixes the generation limit at 10 min; neither is configurable from here, and waiting that long is not a bounded smoke. The interrupted cases in step 4 report usage_missing/result_unknown for host-initiated stops instead.",
        );
        // An unknown model id: informational, the finding is how the upstream and host classify it.
        let before_models = self
            .providers
            .add_manual_model(
                &self.provider_id,
                USER,
                UNKNOWN_MODEL,
                Some("text-generation"),
            )
            .await;
        if before_models.is_err() {
            self.info("5 unknown_model", "could not register the probe model");
            return;
        }
        let target = ChatTargetInput {
            provider_id: Some(self.provider_id.clone()),
            model_id: Some(UNKNOWN_MODEL.into()),
            skill_id: None,
            no_skill: true,
        };
        let session = match self
            .chat
            .create_session(
                &self.providers,
                &self.skills,
                USER,
                CreateChatSessionInput {
                    target: Some(target),
                },
            )
            .await
        {
            Ok(s) => s.id,
            Err(error) => {
                self.info("5 unknown_model", &Self::err_text(&error));
                return;
            }
        };
        match self
            .turn(
                &session,
                "Reply with the single word: pong".into(),
                Mode::Plain,
            )
            .await
        {
            Ok(run) => {
                let provider_error = matches!(
                    &run.inner.term,
                    Some(Term::Error { code, .. }) if code.starts_with("provider_")
                );
                let verdict = if provider_error { "PASS" } else { "INFO" };
                self.out(
                    "5 unknown_model",
                    verdict,
                    &format!(
                        "deltas={} total_ms={} {}",
                        run.inner.deltas,
                        run.ms,
                        term_text(&run.inner.term)
                    ),
                );
            }
            Err(error) => self.info("5 unknown_model", &Self::err_text(&error)),
        }
    }

    async fn step_regenerate(&mut self) {
        let Some(session) = self.new_session("6 session", None).await else {
            return;
        };
        let first = match self
            .turn(
                &session,
                "Reply with the single word: pong".into(),
                Mode::Plain,
            )
            .await
        {
            Ok(run) => run,
            Err(error) => {
                self.check("6a regenerate", false, &Self::err_text(&error));
                return;
            }
        };
        let old_id = first.ack.assistant_message_id.clone();
        let before = self.msgs(&session).await;
        match self.regen(&old_id, Mode::Plain).await {
            Ok(run) => {
                let after = self.msgs(&session).await;
                let old_gone = !after.iter().any(|m| m.id == old_id);
                let new_here = after
                    .iter()
                    .find(|m| m.id == run.ack.assistant_message_id)
                    .map(|m| m.status.clone());
                let done = matches!(run.inner.term, Some(Term::Done { .. }));
                self.check(
                    "6a regenerate_success",
                    old_gone
                        && new_here.as_deref() == Some("completed")
                        && after.len() == before.len()
                        && done,
                    &format!(
                        "old_reply_replaced={old_gone} new_reply_status={new_here:?} messages_before={} messages_after={} dropped_turns={} deltas={} total_ms={} {}",
                        before.len(),
                        after.len(),
                        run.ack.dropped_turns,
                        run.inner.deltas,
                        run.ms,
                        term_text(&run.inner.term)
                    ),
                );
            }
            Err(error) => self.check("6a regenerate_success", false, &Self::err_text(&error)),
        }

        // 6b: regenerate, then cancel; the old reply must stay.
        let Some(session) = self.new_session("6 session", None).await else {
            return;
        };
        let first = match self
            .turn(
                &session,
                "Count from 1 to 40, one number per line, no other text.".into(),
                Mode::Plain,
            )
            .await
        {
            Ok(run) => run,
            Err(error) => {
                self.check("6b regenerate_cancel", false, &Self::err_text(&error));
                return;
            }
        };
        if !matches!(first.inner.term, Some(Term::Done { .. })) {
            self.info(
                "6b regenerate_cancel",
                "first reply did not complete; not exercised",
            );
            return;
        }
        let old_id = first.ack.assistant_message_id.clone();
        let before = self.msgs(&session).await;
        let old_before = before.iter().find(|m| m.id == old_id).cloned();
        match self.regen(&old_id, Mode::CancelAt(2)).await {
            Ok(run) => {
                if matches!(run.inner.term, Some(Term::Done { .. })) {
                    self.info(
                        "6b regenerate_cancel",
                        &format!(
                            "not exercised: regenerated reply ended after {} deltas before the cancel point",
                            run.inner.deltas
                        ),
                    );
                    return;
                }
                let after = self.msgs(&session).await;
                let old_after = after.iter().find(|m| m.id == old_id);
                let kept = match (&old_before, old_after) {
                    (Some(a), Some(b)) => a.content == b.content && b.status == "completed",
                    _ => false,
                };
                let discarded = matches!(
                    run.inner.term,
                    Some(Term::Error {
                        discarded: true,
                        ..
                    })
                );
                let task = self.task_row(&run.ack.task_id).await;
                self.check(
                    "6b regenerate_cancel",
                    kept && discarded && after.len() == before.len(),
                    &format!(
                        "old_reply_kept={kept} failed_attempt_discarded(regeneration_discarded)={discarded} messages_before={} messages_after={} deltas_seen={} task={:?} {}",
                        before.len(),
                        after.len(),
                        run.inner.deltas,
                        task.as_ref().map(|t| (t.0.clone(), t.1.clone())),
                        term_text(&run.inner.term)
                    ),
                );
            }
            Err(error) => self.check("6b regenerate_cancel", false, &Self::err_text(&error)),
        }
    }

    async fn step_skill(&mut self) {
        let catalog = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills");
        if self
            .skills
            .register_bundled_catalog(&catalog)
            .await
            .is_err()
            || self.skills.install_skill(SKILL, USER).await.is_err()
        {
            self.check(
                "7 skill_setup",
                false,
                "bundled catalog registration or install failed",
            );
            return;
        }
        let Some(session) = self.new_session("7 skill_pin", Some(SKILL)).await else {
            return;
        };
        match self
            .turn(
                &session,
                "Summarize in one short sentence: the sky is blue.".into(),
                Mode::Plain,
            )
            .await
        {
            Ok(run) => self.check(
                "7a skill_send",
                matches!(run.inner.term, Some(Term::Done { .. })),
                &format!(
                    "skill_pinned=true deltas={} total_ms={} {} {}",
                    run.inner.deltas,
                    run.ms,
                    term_text(&run.inner.term),
                    usage_text(&run.usage)
                ),
            ),
            Err(error) => {
                self.check("7a skill_send", false, &Self::err_text(&error));
                return;
            }
        }
        let tasks = self.chat_task_count().await;
        let usage_rows = self.usage_count().await;
        let user_msgs = self.msgs(&session).await.len();

        let _ = self.skills.toggle_skill(SKILL, USER, false).await;
        let disabled = self
            .turn(
                &session,
                "Reply with the single word: pong".into(),
                Mode::Plain,
            )
            .await;
        let _ = self.skills.toggle_skill(SKILL, USER, true).await;
        let disabled_ok = matches!(disabled, Err(ChatError::SkillUnavailable));

        let installed = self.skills.list_installed_skills(USER).await.ok();
        let file = installed
            .and_then(|list| list.into_iter().next())
            .map(|skill| PathBuf::from(skill.install_path).join("SKILL.md"));
        let Some(file) = file else {
            self.check(
                "7b skill_disabled",
                disabled_ok,
                "install path not found for tamper step",
            );
            return;
        };
        let original = std::fs::read(&file).ok();
        let tampered_ok = if let Some(original) = &original {
            let _ = std::fs::write(&file, "tampered");
            let result = self
                .turn(
                    &session,
                    "Reply with the single word: pong".into(),
                    Mode::Plain,
                )
                .await;
            let _ = std::fs::write(&file, original);
            matches!(result, Err(ChatError::SkillUnavailable))
        } else {
            false
        };
        let tasks_after = self.chat_task_count().await;
        let usage_after = self.usage_count().await;
        let msgs_after = self.msgs(&session).await.len();
        self.check(
            "7b skill_disabled_or_tampered",
            disabled_ok
                && tampered_ok
                && tasks == tasks_after
                && usage_rows == usage_after
                && user_msgs == msgs_after,
            &format!(
                "disabled->skill_unavailable={disabled_ok} tampered->skill_unavailable={tampered_ok} new_tasks={} new_upstream_calls={} new_messages={}",
                tasks_after - tasks,
                usage_after - usage_rows,
                msgs_after.saturating_sub(user_msgs)
            ),
        );
        match self
            .turn(
                &session,
                "Reply with the single word: pong".into(),
                Mode::Plain,
            )
            .await
        {
            Ok(run) => self.check(
                "7c skill_restored",
                matches!(run.inner.term, Some(Term::Done { .. })),
                &format!(
                    "restored file+enabled -> send ok {}",
                    term_text(&run.inner.term)
                ),
            ),
            Err(error) => self.check("7c skill_restored", false, &Self::err_text(&error)),
        }
    }

    async fn calibrate_one(&mut self, step: &str, text: String) -> Option<(usize, i64)> {
        let session = self.new_session(step, None).await?;
        let content_bytes = text.trim().len();
        let input_est = policy_len().map(|p| p + content_bytes);
        match self.turn(&session, text, Mode::Plain).await {
            Ok(run) => {
                let done = matches!(run.inner.term, Some(Term::Done { .. }));
                let prompt = run.usage.as_ref().map(|u| u.0);
                let ratio = match (input_est, prompt) {
                    (Some(bytes), Some(tokens)) if tokens > 0 => {
                        format!("{:.2}", bytes as f64 / tokens as f64)
                    }
                    _ => "n/a".into(),
                };
                self.check(
                    step,
                    done,
                    &format!(
                        "content_bytes={content_bytes} input_utf8_bytes_est={input_est:?} upstream_prompt_tokens={prompt:?} upstream_completion_tokens={:?} bytes_per_prompt_token={ratio} dropped_turns={} total_ms={} {}",
                        run.usage.as_ref().map(|u| u.1),
                        run.ack.dropped_turns,
                        run.ms,
                        term_text(&run.inner.term)
                    ),
                );
                match (input_est, prompt) {
                    (Some(bytes), Some(tokens)) => Some((bytes, tokens)),
                    _ => None,
                }
            }
            Err(error) => {
                self.check(step, false, &Self::err_text(&error));
                None
            }
        }
    }

    async fn step_calibration(&mut self) {
        match policy_len() {
            Some(len) => self.info(
                "8 input_bytes",
                &format!("host_policy_bytes={len} (parsed from src/lib.rs); input_utf8_bytes_est = host_policy_bytes + message bytes; the assembled request itself is private, so this is computed from inputs, not observed"),
            ),
            None => self.info("8 input_bytes", "host policy length not parsable; input bytes not reported"),
        }
        // Size ladder: single message, no history.
        let mut points: Vec<(usize, i64)> = Vec::new();
        for (label, size) in [
            ("8a ladder_1KiB", 1024usize),
            ("8a ladder_4KiB", 4096),
            ("8a ladder_8KiB", 8192),
            ("8a ladder_15.9KiB", 16282),
        ] {
            if let Some(point) = self.calibrate_one(label, filler(size)).await {
                points.push(point);
            }
        }
        for pair in points.windows(2) {
            let (db, dt) = (
                pair[1].0 as f64 - pair[0].0 as f64,
                pair[1].1 as f64 - pair[0].1 as f64,
            );
            if dt > 0.0 {
                self.info(
                    "8a marginal",
                    &format!(
                        "added_bytes={db} added_prompt_tokens={dt} marginal_bytes_per_token={:.2}",
                        db / dt
                    ),
                );
            }
        }
        // CJK: same byte count, different characters.
        let cjk = cjk_filler(4096);
        let chars = cjk.chars().count();
        self.info(
            "8a cjk",
            &format!("message_bytes={} message_chars={chars}", cjk.len()),
        );
        self.calibrate_one("8a ladder_cjk_4KiB", cjk).await;

        // Boundary: exactly 16 KiB is accepted, 16 KiB + 1 is refused before any upstream call.
        self.calibrate_one("8b boundary_16384", filler(MAX_MESSAGE))
            .await;
        if let Some(session) = self.new_session("8b boundary_16385", None).await {
            let (tasks, usage) = (self.chat_task_count().await, self.usage_count().await);
            let result = self
                .turn(&session, filler(MAX_MESSAGE + 1), Mode::Plain)
                .await;
            let refused = matches!(result, Err(ChatError::InvalidMessageContent));
            let detail = match &result {
                Err(error) => Self::err_text(error),
                Ok(_) => "accepted".into(),
            };
            let (tasks_after, usage_after) =
                (self.chat_task_count().await, self.usage_count().await);
            self.check(
                "8b boundary_16385",
                refused && tasks == tasks_after && usage == usage_after,
                &format!(
                    "{detail} new_tasks={} new_upstream_calls={}",
                    tasks_after - tasks,
                    usage_after - usage
                ),
            );
        }

        self.history_overflow().await;
        self.tiny_context_window().await;
        self.length_test().await;
    }

    async fn history_overflow(&mut self) {
        let Some(session) = self.new_session("8c history_overflow", None).await else {
            return;
        };
        let policy = policy_len();
        let mut drops: Vec<u32> = Vec::new();
        let mut ok = true;
        for round in 1..=5 {
            let text = filler(7168);
            let content = text.trim().len();
            let before = self.msgs(&session).await;
            // Completed turns so far, oldest first: (user bytes + reply bytes).
            let mut turns: Vec<usize> = Vec::new();
            let mut pending: Option<usize> = None;
            for m in &before {
                if m.status != "completed" {
                    continue;
                }
                if m.role == "user" {
                    pending = Some(m.content.len());
                } else if let Some(user) = pending.take() {
                    turns.push(user + m.content.len());
                }
            }
            match self.turn(&session, text, Mode::Plain).await {
                Ok(run) => {
                    let dropped = run.ack.dropped_turns;
                    let kept: usize = turns.iter().skip(dropped as usize).copied().sum();
                    let est = policy.map(|p| p + content + kept);
                    if let Some(bytes) = est {
                        ok &= bytes <= CEILING;
                    }
                    ok &= matches!(run.inner.term, Some(Term::Done { .. }));
                    drops.push(dropped);
                    self.info(
                        "8c history_send",
                        &format!(
                            "send={round} prior_completed_turns={} dropped_turns={dropped} dropped_messages={} input_utf8_bytes_est={est:?} (ceiling {CEILING}) upstream_prompt_tokens={:?} {}",
                            turns.len(),
                            u64::from(dropped) * 2,
                            run.usage.as_ref().map(|u| u.0),
                            term_text(&run.inner.term)
                        ),
                    );
                }
                Err(error) => {
                    ok = false;
                    self.info("8c history_send", &Self::err_text(&error));
                    break;
                }
            }
        }
        let monotonic = drops.windows(2).all(|w| w[0] <= w[1]);
        let some_dropped = drops.last().copied().unwrap_or(0) > 0;
        self.check(
            "8c history_overflow",
            ok && monotonic && some_dropped,
            &format!(
                "dropped_turns_per_send={drops:?} non_decreasing={monotonic} dropped_at_end={some_dropped} estimated_input_within_24KiB={ok}; turns drop whole (user+reply pair, oldest first) per assemble_with: dropped = completed replies - turns kept [from code, not observable upstream]"
            ),
        );
    }

    async fn tiny_context_window(&mut self) {
        let sql = format!(
            "UPDATE models SET context_length = 3072 WHERE provider_id = '{}'",
            self.provider_id
        );
        if self.db.execute_unprepared(&sql).await.is_err() {
            self.info("8d context_too_long", "could not set a tiny context window");
            return;
        }
        let Some(session) = self.new_session("8d context_too_long", None).await else {
            return;
        };
        let (tasks, usage) = (self.chat_task_count().await, self.usage_count().await);
        let long = self.turn(&session, filler(800), Mode::Plain).await;
        let refused = matches!(long, Err(ChatError::ContextTooLong));
        let detail = match &long {
            Err(error) => Self::err_text(error),
            Ok(_) => "accepted".into(),
        };
        let (tasks_after, usage_after) = (self.chat_task_count().await, self.usage_count().await);
        self.check(
            "8d context_too_long",
            refused && tasks == tasks_after && usage == usage_after,
            &format!(
                "window=3072 tokens -> byte room {} ; 800-byte message: {detail} new_tasks={} new_upstream_calls={}",
                3072 - 1024 - 1024,
                tasks_after - tasks,
                usage_after - usage
            ),
        );
        let fits = self.turn(&session, filler(500), Mode::Plain).await;
        let fits_ok = matches!(&fits, Ok(run) if matches!(run.inner.term, Some(Term::Done { .. })));
        self.check(
            "8d context_fits",
            fits_ok,
            &format!(
                "500-byte message under the same window: {}",
                match &fits {
                    Ok(run) => term_text(&run.inner.term),
                    Err(error) => Self::err_text(error),
                }
            ),
        );
        let _ = self
            .db
            .execute_unprepared(&format!(
                "UPDATE models SET context_length = NULL WHERE provider_id = '{}'",
                self.provider_id
            ))
            .await;
    }

    async fn length_test(&mut self) {
        let Some(session) = self.new_session("8e max_tokens", None).await else {
            return;
        };
        // Long enough that any model following it must pass 1024 tokens; plain prose, because a
        // repeated single word was cut short by one model on its own (594 tokens, `stop`).
        let prompt = "Write a detailed essay of at least 3000 words about the history of the \
            printing press, in numbered sections. Do not stop early and do not summarise.";
        match self.turn(&session, prompt.into(), Mode::Plain).await {
            Ok(run) => {
                let completion = run.usage.as_ref().map(|u| u.1);
                let finish = match &run.inner.term {
                    Some(Term::Done { finish, .. }) => finish.clone(),
                    _ => None,
                };
                let error_code = match &run.inner.term {
                    Some(Term::Error { code, .. }) => Some(code.as_str()),
                    _ => None,
                };
                let detail = format!(
                    "finish_reason={finish:?} completion_tokens={completion:?} (limit 1024) deltas={} delta_bytes={} {}",
                    run.inner.deltas,
                    run.inner.delta_bytes,
                    term_text(&run.inner.term)
                );
                let done = matches!(run.inner.term, Some(Term::Done { .. }));
                let within = completion.is_none_or(|c| c <= 1024);
                if done && finish.as_deref() == Some("length") && within {
                    // The upstream honoured the limit and said so: the case D075 describes.
                    self.check("8e max_tokens_1024", true, &detail);
                } else if done && within {
                    self.info(
                        "8e max_tokens_1024",
                        &format!("model stopped on its own below the limit; length not exercised {detail}"),
                    );
                } else if error_code == Some("generation_limit") {
                    // The upstream ignored max_tokens; the host's 64 KiB limit (D074) stopped it.
                    self.warn(
                        "8e max_tokens_1024",
                        &format!("upstream did not enforce max_tokens; host reply limit stopped it {detail}"),
                    );
                } else if done && run.inner.delta_bytes <= 64 * 1024 {
                    // The upstream ignored max_tokens and finished by itself under the host's
                    // 64 KiB limit (first seen 2026-10-31: 7636 completion tokens, `stop`). The
                    // host stored a complete reply, which is correct; length was not exercised.
                    self.warn(
                        "8e max_tokens_1024",
                        &format!("upstream did not enforce max_tokens; reply finished under the host limit {detail}"),
                    );
                } else {
                    self.check("8e max_tokens_1024", false, &detail);
                }
            }
            Err(error) => self.check("8e max_tokens_1024", false, &Self::err_text(&error)),
        }
    }

    async fn step_persistence_scan(&mut self, temp_root: &Path) {
        let _ = self
            .db
            .execute_unprepared("PRAGMA wal_checkpoint(TRUNCATE)")
            .await;
        let mut blobs: Vec<Vec<u8>> = Vec::new();
        for path in [
            self.db_path.clone(),
            PathBuf::from(format!("{}-wal", self.db_path.display())),
        ] {
            if let Ok(bytes) = std::fs::read(&path) {
                blobs.push(bytes);
            }
        }
        let in_db = |needle: &[u8]| blobs.iter().map(|b| count_sub(b, needle)).sum::<usize>();

        // Positive control: text that is stored on purpose is found by the scanner.
        let control = self
            .nonces
            .first()
            .map(|n| in_db(n.as_bytes()))
            .unwrap_or(1);
        // Reasoning windows that are not explained by legitimately stored message text.
        let stored: Vec<String> = match self
            .db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT content FROM chat_messages".to_owned(),
            ))
            .await
        {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|r| r.try_get_by_index::<String>(0).ok())
                .collect(),
            Err(_) => vec![],
        };
        let reasoning_bytes: usize = self.reasoning.iter().map(String::len).sum();
        let (mut windows, mut explained, mut hits) = (0usize, 0usize, 0usize);
        for text in &self.reasoning {
            for window in sample_windows(text, 24, 8) {
                windows += 1;
                if stored.iter().any(|s| s.contains(&window)) {
                    explained += 1;
                } else if in_db(window.as_bytes()) > 0 {
                    hits += 1;
                }
            }
        }
        if self.reasoning.is_empty() {
            self.info(
                "8f reasoning_not_persisted",
                "vacuous: the provider sent no reasoning text in this run (reasoning_events=0), so nothing could be scanned for",
            );
        } else {
            self.check(
                "8f reasoning_not_persisted",
                hits == 0 && control > 0,
                &format!(
                    "reasoning_texts={} reasoning_bytes={reasoning_bytes} windows_checked={windows} explained_by_stored_messages={explained} found_in_db_file_or_wal={hits} positive_control_found={}",
                    self.reasoning.len(),
                    control > 0
                ),
            );
        }

        // The key must not exist anywhere under the temp tree.
        let mut files = Vec::new();
        walk(temp_root, &mut files);
        let mut key_hits = 0usize;
        for file in &files {
            if let Ok(bytes) = std::fs::read(file) {
                key_hits += count_sub(&bytes, self.key.as_bytes());
            }
        }
        let entries = self.vault.entry_count().await;
        self.check(
            "8g key_not_on_disk",
            key_hits == 0 && entries == 1,
            &format!(
                "files_scanned={} key_occurrences={key_hits} memory_vault_entries={entries} os_keychain_used=false (only MemoryCredentialVault is constructed)",
                files.len()
            ),
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Small row helper (keeps the query call sites short)

mod sea_orm_row {
    use dgos_db::sea_orm::QueryResult;

    pub struct Row(pub QueryResult);

    impl Row {
        pub fn int(&self, index: usize) -> Option<i64> {
            self.0.try_get_by_index::<Option<i64>>(index).ok().flatten()
        }
        pub fn text(&self, index: usize) -> Option<String> {
            self.0
                .try_get_by_index::<Option<String>>(index)
                .ok()
                .flatten()
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Input builders and byte helpers

/// ASCII filler of exactly `total` bytes: an instruction to ignore it, then repeated words. The
/// last byte is never whitespace, so `trim()` in the host keeps the exact length.
fn filler(total: usize) -> String {
    const PREFIX: &str = "Ignore the filler below and reply with the single word: pong. ";
    const WORDS: &str = "lorem ipsum dolor sit amet consectetur ";
    let mut text = String::from(PREFIX);
    while text.len() < total {
        text.push_str(WORDS);
    }
    text.truncate(total);
    text.pop();
    text.push('x');
    text
}

/// CJK filler of at most `total` bytes on a character boundary.
fn cjk_filler(total: usize) -> String {
    const PREFIX: &str = "请忽略下面的填充内容,只回复一个词:pong。";
    const UNIT: &str = "这是一段用于测试字节与令牌比例的填充文字。";
    let mut text = String::from(PREFIX);
    while text.len() < total {
        text.push_str(UNIT);
    }
    let mut end = total.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

/// Byte length of the host system policy, parsed from the source so it cannot drift.
fn policy_len() -> Option<usize> {
    const SOURCE: &str = include_str!("../src/lib.rs");
    let marker = "const HOST_POLICY: &str = \"";
    let start = SOURCE.find(marker)? + marker.len();
    let end = SOURCE[start..].find("\";")? + start;
    let literal = &SOURCE[start..end];
    (!literal.contains('\\')).then_some(literal.len())
}

fn count_sub(hay: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || hay.len() < needle.len() {
        return 0;
    }
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Up to `count` evenly spaced substrings of about `len` bytes, skipping near-blank ones.
fn sample_windows(text: &str, len: usize, count: usize) -> Vec<String> {
    let mut out = Vec::new();
    if text.len() < len {
        return out;
    }
    for i in 0..count {
        let mut start = (text.len() - len) * i / count.max(1);
        while !text.is_char_boundary(start) {
            start -= 1;
        }
        let mut end = (start + len).min(text.len());
        while !text.is_char_boundary(end) {
            end += 1;
        }
        let window = &text[start..end];
        if window.chars().filter(|c| !c.is_whitespace()).count() >= 12 {
            out.push(window.to_owned());
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Loopback SSE stub (only for --stub; proves the harness wiring, nothing else)

#[derive(Default)]
struct StubState {
    requests: AtomicUsize,
    last_max_tokens: AtomicI64,
}

fn sse(value: Value) -> String {
    format!("data: {value}\n\n")
}

async fn stub_serve(listener: TcpListener, state: Arc<StubState>) {
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        tokio::spawn(stub_connection(socket, state.clone()));
    }
}

async fn stub_connection(mut socket: tokio::net::TcpStream, state: Arc<StubState>) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let n = socket.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let length = head
        .to_ascii_lowercase()
        .lines()
        .find_map(|l| l.strip_prefix("content-length:").map(str::to_owned))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buffer.len() < header_end + length {
        let n = socket.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        buffer.extend_from_slice(&chunk[..n]);
    }
    let body: Value =
        serde_json::from_slice(&buffer[header_end..header_end + length]).unwrap_or(Value::Null);
    if !head
        .lines()
        .next()
        .unwrap_or("")
        .contains("/chat/completions")
    {
        let _ = socket
            .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await;
        return;
    }
    state.requests.fetch_add(1, Ordering::SeqCst);
    let max_tokens = body["max_tokens"].as_i64().unwrap_or(0);
    state.last_max_tokens.store(max_tokens, Ordering::SeqCst);
    if body["model"].as_str() == Some(UNKNOWN_MODEL) {
        let payload =
            json!({"error": {"message": "model not found", "type": "invalid_request_error"}})
                .to_string();
        let _ = socket
            .write_all(
                format!(
                    "HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
                    payload.len()
                )
                .as_bytes(),
            )
            .await;
        return;
    }
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let texts: Vec<String> = messages
        .iter()
        .map(|m| m["content"].as_str().unwrap_or("").to_owned())
        .collect();
    let last = texts.last().cloned().unwrap_or_default();
    let prior = texts[..texts.len().saturating_sub(1)].join("\n");
    let prompt_tokens = texts.iter().map(String::len).sum::<usize>() / 4 + 1;

    let (deltas, finish, delay): (Vec<String>, &str, u64) = if last.contains("What was the token") {
        let nonce = prior
            .find("DGOSN-")
            .map(|at| prior[at..].chars().take(18).collect::<String>())
            .unwrap_or_else(|| "unknown".into());
        (vec![nonce], "stop", 0)
    } else if let Some(at) = last.find("Count from 1 to ") {
        let n: usize = last[at + "Count from 1 to ".len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap_or(10);
        ((1..=n).map(|i| format!("{i}\n")).collect(), "stop", 15)
    } else if last.contains("history of the printing press") {
        // A well-behaved upstream: stops at the requested max_tokens and says `length`.
        let n = if max_tokens > 0 {
            max_tokens as usize
        } else {
            3000
        };
        (vec!["word ".to_owned(); n], "length", 0)
    } else {
        (vec!["pong".to_owned()], "stop", 0)
    };

    let _ = socket
        .write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
        )
        .await;
    let mut out = vec![sse(
        json!({"choices":[{"delta":{"reasoning_content":STUB_REASONING},"finish_reason":null}]}),
    )];
    let completion = deltas.len();
    for text in &deltas {
        out.push(sse(
            json!({"choices":[{"delta":{"content":text},"finish_reason":null}]}),
        ));
    }
    out.push(sse(
        json!({"choices":[{"delta":{},"finish_reason":finish}]}),
    ));
    out.push(sse(json!({"choices":[],"usage":{"prompt_tokens":prompt_tokens,"completion_tokens":completion,"total_tokens":prompt_tokens+completion}})));
    out.push("data: [DONE]\n\n".to_owned());
    for piece in out {
        if socket.write_all(piece.as_bytes()).await.is_err() {
            return;
        }
        let _ = socket.flush().await;
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Orchestration

async fn run_service(svc: Svc, temp_root: &Path) -> usize {
    let root = temp_root.join(svc.label);
    if std::fs::create_dir_all(&root).is_err() {
        println!("[{}] setup: FAIL temp directory", svc.label);
        return 1;
    }
    let db_path = root.join("chat-smoke.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let setup = async {
        let db = dgos_db::establish_connection(&url)
            .await
            .map_err(|_| "database")?;
        Migrator::up(&db, None).await.map_err(|_| "migrations")?;
        let vault = Arc::new(MemoryCredentialVault::default());
        let provider = seed_provider(
            db.clone(),
            &vault,
            ProviderConfig {
                name: format!("smoke-{}", svc.label),
                provider_type: svc.provider_type.into(),
                base_url: svc.base_url.clone(),
                api_key: svc.key.clone(),
                enabled: true,
                config: None,
            },
            USER,
        )
        .await
        .map_err(|_| "seed provider")?;
        let providers = ProviderManager::with_vault(&url, vault.clone())
            .await
            .map_err(|_| "provider manager")?;
        providers
            .add_manual_model(&provider.id, USER, &svc.model, Some("text-generation"))
            .await
            .map_err(|_| "register model")?;
        Ok::<_, &str>((db, vault, provider.id, providers))
    };
    let (db, vault, provider_id, providers) = match setup.await {
        Ok(parts) => parts,
        Err(stage) => {
            println!("[{}] setup: FAIL stage={stage}", svc.label);
            return 1;
        }
    };
    let mut env = Env {
        label: svc.label,
        key: svc.key.clone(),
        model: svc.model.clone(),
        db_path,
        chat: ChatManager::new(db.clone()),
        skills: SkillsManager::new(db.clone(), root.join("skills")),
        root,
        db,
        providers,
        vault,
        provider_id,
        fails: 0,
        warns: 0,
        reasoning: Vec::new(),
        nonces: Vec::new(),
        interruptions: Vec::new(),
    };
    let _ = &env.root;
    println!(
        "[{}] setup: PASS provider_type={} temp_db=true credential_store=memory",
        env.label, svc.provider_type
    );
    if svc.stub.is_some() {
        println!(
            "[{}] NOTE: stub wiring check, not real Provider evidence",
            env.label
        );
    }

    env.step_conversation().await;
    env.step_cancel().await;
    env.step_timeout_and_unknown_model().await;
    env.step_regenerate().await;
    env.step_skill().await;
    env.step_calibration().await;
    env.step_persistence_scan(temp_root).await;

    if let Some(stub) = &svc.stub {
        println!(
            "[{}] stub: requests_served={} last_max_tokens_in_request={}",
            env.label,
            stub.requests.load(Ordering::SeqCst),
            stub.last_max_tokens.load(Ordering::SeqCst)
        );
    }
    println!(
        "[{}] summary: failures={} warnings={}",
        env.label, env.fails, env.warns
    );
    env.fails
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("{message}");
            }
            eprintln!("{USAGE}");
            return if message.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            };
        }
    };
    let mut services: Vec<Svc> = Vec::new();
    services.extend(service(
        "openai",
        "openai_compatible",
        args.openai_base_url,
        args.openai_model,
        args.openai,
    ));
    services.extend(service(
        "anthropic",
        "anthropic",
        args.anthropic_base_url,
        args.anthropic_model,
        args.anthropic,
    ));
    let stub_requested = args.stub;
    println!("NOT RUN mcp: this harness starts no MCP server; tool turns are not exercised");
    if services.is_empty() && !stub_requested {
        println!("nothing to run: no key entered (see the doc comment for options)");
        return ExitCode::SUCCESS;
    }

    let temp = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(_) => {
            println!("setup: FAIL temp directory");
            return ExitCode::from(1);
        }
    };
    let home = temp.path().join("home");
    let _ = std::fs::create_dir_all(&home);
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
    std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));
    std::env::set_var("XDG_CACHE_HOME", home.join(".cache"));
    println!(
        "isolation: HOME and XDG_* redirected to a temp directory; per-service temp SQLite; key held only in an in-memory vault; user={USER}"
    );

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            println!("setup: FAIL async runtime");
            return ExitCode::from(1);
        }
    };
    let failures = runtime.block_on(async {
        if stub_requested {
            match TcpListener::bind("127.0.0.1:0").await {
                Ok(listener) => {
                    let addr = listener.local_addr().expect("bound address");
                    let state = Arc::new(StubState::default());
                    tokio::spawn(stub_serve(listener, state.clone()));
                    services.push(Svc {
                        label: "stub",
                        provider_type: "openai_compatible",
                        base_url: format!("http://{addr}/v1"),
                        key: format!("stub-key-{}", uuid::Uuid::new_v4().simple()),
                        model: "stub-model".into(),
                        stub: Some(state),
                    });
                }
                Err(_) => println!("NOT RUN stub: could not bind a loopback port"),
            }
        }
        let mut failures = 0usize;
        for svc in services {
            failures += run_service(svc, temp.path()).await;
        }
        failures
    });
    if failures > 0 {
        println!("RESULT: FAIL ({failures} failed checks)");
        ExitCode::from(1)
    } else {
        println!("RESULT: no failures");
        ExitCode::SUCCESS
    }
}
