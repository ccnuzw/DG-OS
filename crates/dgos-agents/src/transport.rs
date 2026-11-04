use crate::diagnostics::ProcessDiagnostic;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::{mpsc, oneshot, Mutex},
};
use tokio_util::sync::CancellationToken;

type Reply = oneshot::Sender<Result<Value, String>>;
#[derive(Clone)]
pub(crate) struct Transport {
    input: mpsc::Sender<Value>,
    pending: Arc<Mutex<HashMap<u64, Reply>>>,
    next: Arc<AtomicU64>,
    stop: CancellationToken,
    finished: CancellationToken,
    reaped: Arc<AtomicBool>,
    diagnostic: ProcessDiagnostic,
    pid: Option<u32>,
}
impl Transport {
    pub fn spawn(
        mut command: Command,
        lease: CancellationToken,
        diagnostic: ProcessDiagnostic,
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
            Err(_) => { spawn.finish(Some("spawn_failed")); return Err("spawn_failed".into()); }
        };
        spawn.finish(None);
        let pid = child.id();
        diagnostic.record("spawned", pid, "completed", "none");
        #[cfg(unix)]
        let group = child.id().map(|pid| pid as i32);
        let mut stdin = child.stdin.take().ok_or("spawn_failed")?;
        let stdout = child.stdout.take().ok_or("spawn_failed")?;
        let (input, mut writes) = mpsc::channel::<Value>(64);
        let (events, received) = mpsc::channel(128);
        let pending: Arc<Mutex<HashMap<u64, Reply>>> = Arc::default();
        let stop = lease.child_token();
        let finished = CancellationToken::new();
        let reaped_flag = Arc::new(AtomicBool::new(false));
        let handle = Self {
            input,
            pending: pending.clone(),
            next: Arc::new(AtomicU64::new(1)),
            stop: stop.clone(),
            finished: finished.clone(),
            reaped: reaped_flag.clone(),
            diagnostic: diagnostic.clone(),
            pid,
        };
        tokio::spawn(async move {
            let io = async {
                let reader = async {
                    let mut reader = BufReader::new(stdout);
                    let mut buffer = Vec::new();
                    loop {
                        buffer.clear();
                        loop {
                            let bytes = reader.fill_buf().await.map_err(|_| ())?;
                            if bytes.is_empty() {
                                return Err(());
                            }
                            let n = bytes
                                .iter()
                                .position(|b| *b == b'\n')
                                .map_or(bytes.len(), |i| i + 1);
                            if buffer.len() + n > 4 * 1024 * 1024 {
                                return Err(());
                            }
                            buffer.extend_from_slice(&bytes[..n]);
                            reader.consume(n);
                            if buffer.last() == Some(&b'\n') {
                                break;
                            }
                        }
                        let value: Value = serde_json::from_slice(&buffer).map_err(|_| ())?;
                        if value.get("method").is_none() {
                            if let Some(id) = value.get("id").and_then(Value::as_u64) {
                                if let Some(reply) = pending.lock().await.remove(&id) {
                                    let result = if value.get("error").is_some() {
                                        Err("engine_rejected".into())
                                    } else {
                                        Ok(value.get("result").cloned().unwrap_or(Value::Null))
                                    };
                                    let _ = reply.send(result);
                                    continue;
                                }
                            }
                        }
                        events.send(value).await.map_err(|_| ())?;
                    }
                };
                let writer = async {
                    while let Some(value) = writes.recv().await {
                        let mut bytes = serde_json::to_vec(&value).map_err(|_| ())?;
                        bytes.push(b'\n');
                        stdin.write_all(&bytes).await.map_err(|_| ())?;
                    }
                    Err::<(), ()>(())
                };
                tokio::select! { result = reader => result, result = writer => result }
            };
            tokio::select! {
                _ = stop.cancelled() => diagnostic.record("transport", pid, "cancelled", "cancelled"),
                _ = io => diagnostic.record("transport", pid, "failed", "connection_lost"),
            }
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
            exit.finish((!reaped).then_some("cleanup_pending"));
            cleanup.finish((!reaped).then_some("cleanup_pending"));
            for (_, reply) in pending.lock().await.drain() {
                let _ = reply.send(Err("connection_lost".into()));
            }
            reaped_flag.store(reaped, Ordering::Release);
            let _ = events.try_send(json!({"method":"host/closed"}));
            finished.cancel();
        });
        Ok((handle, received))
    }
    pub(crate) async fn startup_request(&self, stage: &'static str, method: &str, params: Value) -> Result<Value, String> {
        let phase = self.diagnostic.phase(stage, self.pid);
        let result = self.request(method, params).await;
        if self.stop.is_cancelled() {
            self.diagnostic.record(stage, self.pid, "cancelled", "cancelled");
        }
        phase.finish(result.as_ref().err().map(String::as_str));
        result
    }
    pub(crate) async fn startup_write(&self, value: Value) -> Result<(), String> {
        let phase = self.diagnostic.phase("handshake_initialized", self.pid);
        let result = self.write(value).await;
        phase.finish(result.as_ref().err().map(String::as_str));
        result
    }
    pub(crate) fn startup_result(&self, error: Option<&str>) {
        let phase = self.diagnostic.phase("startup", self.pid);
        phase.finish(error);
    }
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        if self.stop.is_cancelled() || self.finished.is_cancelled() {
            return Err("closed".into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        if self
            .write(json!({"id":id,"method":method,"params":params}))
            .await
            .is_err()
        {
            self.pending.lock().await.remove(&id);
            return Err("connection_lost".into());
        }
        match tokio::time::timeout(Duration::from_secs(45), rx).await {
            Ok(Ok(result)) => result,
            outcome => {
                self.diagnostic.record("request_wait", self.pid, "failed", if outcome.is_err() { "timeout" } else { "connection_lost" });
                self.pending.lock().await.remove(&id);
                Err("result_unknown".into())
            }
        }
    }
    pub async fn write(&self, value: Value) -> Result<(), String> {
        if self.stop.is_cancelled() || self.finished.is_cancelled() {
            return Err("closed".into());
        }
        tokio::time::timeout(Duration::from_secs(5), self.input.send(value))
            .await
            .map_err(|_| "result_unknown")?
            .map_err(|_| "connection_lost".into())
    }
    pub async fn close(&self) -> bool {
        self.stop.cancel();
        tokio::time::timeout(Duration::from_secs(4), self.finished.cancelled())
            .await
            .is_ok()
            && self.reaped.load(Ordering::Acquire)
    }
    /// Resolves once the child's IO loop has ended. `true` only if the child was reaped.
    pub async fn wait_stopped(&self) -> bool {
        self.finished.cancelled().await;
        self.reaped.load(Ordering::Acquire)
    }
}
