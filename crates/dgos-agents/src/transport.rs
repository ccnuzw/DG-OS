use crate::diagnostics::ProcessDiagnostic;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::{mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

type Reply = oneshot::Sender<Result<Value, String>>;
const MAX_PENDING: usize = 128;
const EVENT_CAPACITY: usize = 128;

#[derive(Default)]
struct Pending {
    replies: HashMap<u64, Reply>,
    closed: bool,
}

struct Registration<'a> {
    pending: &'a Mutex<Pending>,
    id: u64,
}
impl Drop for Registration<'_> {
    fn drop(&mut self) {
        self.pending.lock().unwrap().replies.remove(&self.id);
    }
}

fn lose_connection(pending: &Mutex<Pending>, lost: &CancellationToken) {
    lost.cancel();
    let mut pending = pending.lock().unwrap();
    pending.closed = true;
    for (_, reply) in pending.replies.drain() {
        let _ = reply.send(Err("connection_lost".into()));
    }
}

enum Envelope {
    Response,
    Request,
    Notification,
}

fn classify(value: &Value) -> Result<Envelope, &'static str> {
    let object = value.as_object().ok_or("invalid_output")?;
    if object.get("jsonrpc").is_some_and(|version| version != "2.0") {
        return Err("invalid_output");
    }
    let id = object.get("id");
    let valid_id = |id: &Value| id.is_string() || id.as_u64().is_some() || id.as_i64().is_some();
    if let Some(method) = object.get("method") {
        if method.as_str().is_none_or(str::is_empty)
            || object.contains_key("result") || object.contains_key("error")
            || object.get("params").is_some_and(|params| !params.is_object() && !params.is_array()) {
            return Err("invalid_output");
        }
        return match id {
            Some(id) if valid_id(id) => Ok(Envelope::Request),
            Some(_) => Err("invalid_output"),
            None => Ok(Envelope::Notification),
        };
    }
    if id.is_none_or(|id| !id.is_null() && !valid_id(id))
        || object.contains_key("result") == object.contains_key("error") {
        return Err("invalid_output");
    }
    if let Some(error) = object.get("error") {
        if !error.is_object() || error.get("code").and_then(Value::as_i64).is_none()
            || error.get("message").and_then(Value::as_str).is_none() {
            return Err("invalid_output");
        }
    }
    Ok(Envelope::Response)
}

#[derive(Clone)]
pub(crate) struct Transport {
    input: mpsc::Sender<Value>,
    pending: Arc<Mutex<Pending>>,
    next: Arc<AtomicU64>,
    stop: CancellationToken,
    connection_lost: CancellationToken,
    finished: CancellationToken,
    reaped: Arc<AtomicBool>,
    diagnostic: ProcessDiagnostic,
    pid: Option<u32>,
}
pub(crate) struct StartupError {
    pub error: String,
    pub child: Option<Transport>,
}
impl From<String> for StartupError {
    fn from(error: String) -> Self { Self { error, child: None } }
}
impl From<&str> for StartupError {
    fn from(error: &str) -> Self { error.to_owned().into() }
}
impl Transport {
    /// Native observations must stop being current as soon as IO ends, before reaping.
    pub(crate) fn native_available(&self) -> bool {
        !self.stop.is_cancelled() && !self.connection_lost.is_cancelled() && !self.input.is_closed()
    }
    pub(crate) fn connection_lost(&self) -> CancellationToken {
        self.connection_lost.clone()
    }
    pub(crate) fn stopped(&self) -> bool {
        self.finished.is_cancelled() && self.reaped.load(Ordering::Acquire)
    }
    pub(crate) async fn startup_failure(&self, error: String) -> StartupError {
        self.close().await;
        StartupError { error, child: Some(self.clone()) }
    }
}
impl Transport {
    pub fn spawn(
        mut command: Command,
        lease: CancellationToken,
        diagnostic: ProcessDiagnostic,
        provider_lease: Arc<dgos_providers::CliProviderLease>,
    ) -> Result<(Self, mpsc::Receiver<Value>), String> {
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let spawn = diagnostic.phase("spawn", None);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(_) => { spawn.finish(Some("cli_start_failed")); return Err("cli_start_failed".into()); }
        };
        spawn.finish(None);
        let pid = child.id();
        diagnostic.record("spawned", pid, "completed", "none");
        #[cfg(unix)]
        let group = child.id().map(|pid| pid as i32);
        let mut stdin = child.stdin.take().expect("piped child stdin");
        let stdout = child.stdout.take().expect("piped child stdout");
        let (input, mut writes) = mpsc::channel::<Value>(64);
        let (events, received) = mpsc::channel(EVENT_CAPACITY);
        let pending: Arc<Mutex<Pending>> = Arc::default();
        let stop = lease.child_token();
        let connection_lost = stop.child_token();
        let finished = CancellationToken::new();
        let reaped_flag = Arc::new(AtomicBool::new(false));
        let handle = Self {
            input,
            pending: pending.clone(),
            next: Arc::new(AtomicU64::new(1)),
            stop: stop.clone(),
            connection_lost: connection_lost.clone(),
            finished: finished.clone(),
            reaped: reaped_flag.clone(),
            diagnostic: diagnostic.clone(),
            pid,
        };
        tokio::spawn(async move {
            let _provider_lease = provider_lease;
            let io = async {
                let reader = async {
                    let mut reader = BufReader::new(stdout);
                    let mut buffer = Vec::new();
                    loop {
                        buffer.clear();
                        loop {
                            let bytes = reader.fill_buf().await.map_err(|_| "connection_lost")?;
                            if bytes.is_empty() {
                                return Err("connection_lost");
                            }
                            let n = bytes
                                .iter()
                                .position(|b| *b == b'\n')
                                .map_or(bytes.len(), |i| i + 1);
                            if buffer.len() + n > 4 * 1024 * 1024 {
                                return Err("invalid_output");
                            }
                            buffer.extend_from_slice(&bytes[..n]);
                            reader.consume(n);
                            if buffer.last() == Some(&b'\n') {
                                break;
                            }
                        }
                        let value: Value = serde_json::from_slice(&buffer).map_err(|_| "invalid_output")?;
                        match classify(&value)? {
                            Envelope::Response => {
                                // A late, cancelled, or foreign response is never a server request.
                                let reply = value.get("id").and_then(Value::as_u64)
                                    .and_then(|id| pending.lock().unwrap().replies.remove(&id));
                                if let Some(reply) = reply {
                                    let result = if value.get("error").is_some() {
                                        Err(if value.pointer("/error/code").and_then(Value::as_i64) == Some(-32601) { "method_unsupported" } else { "engine_rejected" }.into())
                                    } else {
                                        Ok(value["result"].clone())
                                    };
                                    let _ = reply.send(result);
                                }
                            }
                            Envelope::Request | Envelope::Notification => {
                                events.try_send(value).map_err(|error| match error {
                                    mpsc::error::TrySendError::Full(_) => "event_overload",
                                    mpsc::error::TrySendError::Closed(_) => "connection_lost",
                                })?;
                            }
                        }
                    }
                };
                let writer = async {
                    while let Some(value) = writes.recv().await {
                        let mut bytes = serde_json::to_vec(&value).map_err(|_| "invalid_output")?;
                        bytes.push(b'\n');
                        stdin.write_all(&bytes).await.map_err(|_| "connection_lost")?;
                    }
                    Err::<(), &str>("connection_lost")
                };
                tokio::select! { result = reader => result, result = writer => result }
            };
            tokio::select! {
                biased;
                _ = stop.cancelled() => diagnostic.record("transport", pid, "cancelled", "cancelled"),
                _ = connection_lost.cancelled() => diagnostic.record("transport", pid, "failed", "connection_lost"),
                result = io => {
                    if result == Err("event_overload") {
                        diagnostic.record("event_overload", pid, "failed", "connection_lost");
                    }
                    diagnostic.record("transport", pid, "failed", "connection_lost");
                }
            }
            // Signal loss and fail RPCs before the potentially slow process-group reap.
            lose_connection(&pending, &connection_lost);
            writes.close();
            drop(events);
            let cleanup = diagnostic.phase("cleanup", pid);
            // The CLI and its ordinary tool descendants share this dedicated process group.
            #[cfg(unix)]
            if let Some(group) = group {
                unsafe {
                    libc::kill(-group, libc::SIGTERM);
                }
            }
            let exit = diagnostic.phase("exit_wait", pid);
            let mut reaped = matches!(
                tokio::time::timeout(Duration::from_secs(1), child.wait()).await,
                Ok(Ok(_))
            );
            #[cfg(unix)]
            if let Some(group) = group {
                unsafe {
                    libc::kill(-group, libc::SIGKILL);
                }
            }
            if !reaped {
                let _ = child.start_kill();
                reaped = matches!(
                    tokio::time::timeout(Duration::from_secs(2), child.wait()).await,
                    Ok(Ok(_))
                );
            }
            while !reaped {
                diagnostic.record("exit_wait", pid, "pending", "cleanup_pending");
                // Never release the actual child or authorization on an uncertain wait.
                reaped = child.wait().await.is_ok();
                if !reaped { tokio::time::sleep(Duration::from_millis(100)).await; }
            }
            exit.finish((!reaped).then_some("cleanup_pending"));
            cleanup.finish((!reaped).then_some("cleanup_pending"));
            // `finished` remains cleanup proof, not an IO-liveness signal.
            reaped_flag.store(reaped, Ordering::Release);
            finished.cancel();
        });
        Ok((handle, received))
    }
    pub(crate) async fn startup_request(&self, stage: &'static str, method: &str, params: Value) -> Result<Value, String> {
        let phase = self.diagnostic.phase(stage, self.pid);
        let result = self.request_with_errors(method, params, "cli_handshake_timeout", "cli_handshake_failed").await.map_err(|error| {
            if self.stop.is_cancelled() { "cancelled".into() }
            else if error == "cli_handshake_timeout" { error }
            else { "cli_handshake_failed".into() }
        });
        if self.stop.is_cancelled() {
            self.diagnostic.record(stage, self.pid, "cancelled", "cancelled");
        }
        phase.finish(result.as_ref().err().map(String::as_str));
        result
    }
    pub(crate) async fn startup_write(&self, value: Value) -> Result<(), String> {
        let phase = self.diagnostic.phase("handshake_initialized", self.pid);
        let result = self.write(value).await.map_err(|error| {
            if self.stop.is_cancelled() { "cancelled".to_owned() }
            else if error == "result_unknown" { "cli_handshake_timeout".into() }
            else { "cli_handshake_failed".into() }
        });
        phase.finish(result.as_ref().err().map(String::as_str));
        result
    }
    pub(crate) fn startup_result(&self, error: Option<&str>) {
        let phase = self.diagnostic.phase("startup", self.pid);
        phase.finish(error);
    }
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        self.request_with_errors(method, params, "result_unknown", "result_unknown").await
    }
    async fn request_with_errors(&self, method: &str, params: Value, timeout_error: &'static str, lost_error: &'static str) -> Result<Value, String> {
        if !self.native_available() {
            return Err("closed".into());
        }
        let id = self.next.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| "closed")?;
        let (tx, rx) = oneshot::channel();
        let registration = {
            let mut pending = self.pending.lock().unwrap();
            if pending.closed || !self.native_available() {
                return Err("closed".into());
            }
            if pending.replies.len() >= MAX_PENDING {
                drop(pending);
                lose_connection(&self.pending, &self.connection_lost);
                return Err("connection_lost".into());
            }
            pending.replies.insert(id, tx);
            Registration { pending: &self.pending, id }
        };
        self.write(json!({"id":id,"method":method,"params":params})).await?;
        let result = match tokio::time::timeout(Duration::from_secs(45), rx).await {
            Ok(Ok(result)) => result,
            outcome => {
                self.diagnostic.record("request_wait", self.pid, "failed", if outcome.is_err() { "timeout" } else { "connection_lost" });
                Err(if outcome.is_err() { timeout_error } else { lost_error }.into())
            }
        };
        drop(registration);
        result
    }
    pub async fn write(&self, value: Value) -> Result<(), String> {
        if !self.native_available() {
            return Err("closed".into());
        }
        tokio::select! {
            biased;
            _ = self.connection_lost.cancelled() => Err("connection_lost".into()),
            outcome = tokio::time::timeout(Duration::from_secs(5), self.input.send(value)) => {
                match outcome {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(_)) => {
                        lose_connection(&self.pending, &self.connection_lost);
                        Err("connection_lost".into())
                    }
                    Err(_) => Err("result_unknown".into()),
                }
            }
        }
    }
    pub async fn close(&self) -> bool {
        self.stop.cancel();
        lose_connection(&self.pending, &self.connection_lost);
        tokio::time::timeout(Duration::from_secs(4), self.finished.cancelled())
            .await
            .is_ok()
            && self.reaped.load(Ordering::Acquire)
    }
    /// Resolves after process cleanup; `true` only if the child was reaped.
    pub async fn wait_stopped(&self) -> bool {
        self.finished.cancelled().await;
        self.reaped.load(Ordering::Acquire)
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
