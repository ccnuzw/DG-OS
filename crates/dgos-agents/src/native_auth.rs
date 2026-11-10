//! Dedicated, memory-only CLI OAuth. No terminal/run/event/journal projection.
use super::*;
use crate::native_session::{NativeAuthCallback, NativeAuthKind, NativeAuthRequest, NativeAuthSnapshot, StartNativeAuth};
use std::{collections::BTreeMap, ffi::OsString, sync::atomic::{AtomicBool, Ordering}, time::Duration};
use tokio::sync::Notify;

pub(crate) struct LaunchContext {
    pub executable: PathBuf, pub workspace: String, pub env: BTreeMap<OsString, OsString>,
}
pub(crate) struct Operation {
    owner: String, input: StartNativeAuth, state: Mutex<NativeAuthSnapshot>,
    cancel: CancellationToken, stopped: AtomicBool, done: Notify,
    terminal: std::sync::Mutex<Option<std::fs::File>>,
    callback_submitted: AtomicBool,
    _permit: std::sync::Mutex<Option<dgos_db::ActivityPermit>>,
    authority: Vec<CancellationToken>,
}
fn now_ms() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64 }
fn terminal(state: &str) -> bool { matches!(state, "succeeded" | "failed" | "stopped" | "auth_unknown") }
fn clear_interaction(state: &mut NativeAuthSnapshot) { state.authorization_url = None; state.callback_required = false; }
fn valid_authorization(text: &str) -> Option<reqwest::Url> {
    if text.len() > 16384 || text.chars().any(char::is_control) { return None; }
    let url = reqwest::Url::parse(text).ok()?;
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() { return None; }
    if url.scheme() != "https" && !(url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))) { return None; }
    {
    let pairs: Vec<_> = url.query_pairs().collect();
    let mut states = pairs.iter().filter(|(key, _)| key == "state");
    let state = states.next()?.1.as_ref();
    let mut redirects = pairs.iter().filter(|(key, _)| key == "redirect_uri");
    let redirect = reqwest::Url::parse(redirects.next()?.1.as_ref()).ok()?;
    if state.is_empty() || states.next().is_some() || redirects.next().is_some()
        || !redirect.username().is_empty() || redirect.password().is_some() || redirect.fragment().is_some()
        || redirect.host_str().is_none() || !matches!(redirect.scheme(), "http" | "https") { return None; }
    }
    Some(url)
}
fn callback_matches(auth: &str, callback: &str) -> bool {
    if callback.len() > 16384 || callback.chars().any(char::is_control) { return false; }
    let Some(auth) = valid_authorization(auth) else { return false; };
    let Ok(callback) = reqwest::Url::parse(callback) else { return false; };
    let pairs: Vec<_> = auth.query_pairs().collect();
    let Some((_, redirect)) = pairs.iter().find(|(key, _)| key == "redirect_uri") else { return false; };
    let Ok(redirect) = reqwest::Url::parse(redirect) else { return false; };
    let expected = pairs.iter().find(|(key, _)| key == "state").map(|(_, value)| value.as_ref());
    let states: Vec<_> = callback.query_pairs().filter(|(key, _)| key == "state").map(|(_, value)| value.into_owned()).collect();
    callback.username().is_empty() && callback.password().is_none() && callback.fragment().is_none()
        && callback.scheme() == redirect.scheme() && callback.host_str() == redirect.host_str()
        && callback.port_or_known_default() == redirect.port_or_known_default() && callback.path() == redirect.path()
        && states.len() == 1 && Some(states[0].as_str()) == expected
        && callback.query_pairs().filter(|(key, _)| key == "code" || key == "error").count() == 1
        && callback.query_pairs().any(|(key, value)| (key == "code" || key == "error") && !value.is_empty())
}
impl AgentManager {
    pub async fn start_native_auth(&self, owner: &str, input: StartNativeAuth, lease: &CancellationToken) -> Result<NativeAuthSnapshot, String> {
        Uuid::parse_str(&input.request_id).map_err(|_| "invalid_request_id")?;
        if crate::native_session::safe_name(&input.server_name).is_none() || input.server_name.starts_with('-') { return Err("native_extension_InvalidFormat".into()); }
        let admission = self.admit_native_extensions(lease).await?;
        let expires_at = now_ms() + 180000;
        let (record, backend, _) = self.native_owned(owner, &input.session_id, Some(&input.generation), lease).await?;
        if let Some(old) = self.inner.native_auth.lock().await.get(&input.request_id).cloned() {
            if old.owner != owner || old.input != input { return Err("request_conflict".into()); }
            let snapshot = old.state.lock().await.clone();
            return Ok(snapshot);
        }
        let launch = match &backend {
            Backend::Claude(session) => session.auth_launch(),
            Backend::Codex(session) => session.auth_launch(),
        }.ok_or("native_extension_Unsupported")?;
        let control = self.session_control(&input.session_id).await?;
        let _control = control.try_lock().map_err(|_| "session_busy")?;
        if !matches!(backend.snapshot().await.status.as_str(), "idle" | "error") { return Err("session_busy".into()); }
        drop(admission);
        // Status evidence, never a filesystem path or renderer-provided scope selector.
        let read = async { match &backend {
            Backend::Claude(session) => session.observe_native(crate::native_session::SessionDomain::Mcp).await,
            Backend::Codex(session) => session.observe_native(crate::native_session::SessionDomain::Mcp).await,
        } };
        let observed = tokio::select! { biased; _ = lease.cancelled() => return Err("session_invalid".into()), result = read => result }?;
        if !crate::native_session::auth_server_allowed(record.engine, &observed, &input.server_name) { return Err("native_extension_Unsupported".into()); }
        let _admission = self.admit_native_extensions(lease).await?;
        self.native_owned(owner, &input.session_id, Some(&input.generation), lease).await?;
        let (session_lease, provider_lease) = self.session_cancellation(&input.session_id).await?;
        let mut operations = self.inner.native_auth.lock().await;
        if let Some(old) = operations.get(&input.request_id) {
            if old.owner != owner || old.input != input { return Err("request_conflict".into()); }
            return Ok(old.state.lock().await.clone());
        }
        if operations.len() >= 256 { return Err("request_capacity".into()); }
        let running: Vec<_> = operations.values().filter(|op| !op.stopped.load(Ordering::Acquire)).collect();
        if !running.is_empty() { return Err("session_busy".into()); }
        let state = NativeAuthSnapshot { session_id: input.session_id.clone(), generation: input.generation.clone(), request_id: input.request_id.clone(), server_name: input.server_name.clone(), kind: input.kind, state: "starting".into(), reason: None, credential_outcome: "unknown".into(), authorization_url: None, callback_required: false, expires_at, exit_code: None };
        let permit = self.inner.gate.admit().map_err(|_| "maintenance_active")?;
        let op = Arc::new(Operation { owner: owner.into(), input: input.clone(), state: Mutex::new(state.clone()), cancel: CancellationToken::new(), stopped: AtomicBool::new(false), done: Notify::new(), terminal: std::sync::Mutex::new(None), callback_submitted: AtomicBool::new(false), _permit: std::sync::Mutex::new(Some(permit)), authority: vec![lease.clone(), session_lease.clone(), provider_lease.clone(), self.inner.stop.clone()] });
        let mut watchers = self.inner.watchers.lock().await;
        self.reap_watchers(&mut watchers).await;
        operations.insert(input.request_id.clone(), op.clone());
        let stop = self.inner.stop.clone(); let account = lease.clone();
        let watcher = tokio::spawn(async move { supervise(op, launch, account, session_lease, provider_lease, stop).await });
        watchers.push(watcher);
        Ok(state)
    }
    async fn owned_native_auth(&self, owner: &str, input: &NativeAuthRequest, lease: &CancellationToken) -> Result<Arc<Operation>, String> {
        Uuid::parse_str(&input.request_id).map_err(|_| "invalid_request_id")?;
        self.native_owned(owner, &input.session_id, Some(&input.generation), lease).await?;
        self.inner.native_auth.lock().await.get(&input.request_id)
            .filter(|op| op.owner == owner && op.input.session_id == input.session_id && op.input.generation == input.generation).cloned().ok_or_else(|| "native_auth_not_found".into())
    }
    pub async fn read_native_auth(&self, owner: &str, input: NativeAuthRequest, lease: &CancellationToken) -> Result<NativeAuthSnapshot, String> {
        let op = self.owned_native_auth(owner, &input, lease).await?;
        let state = op.state.lock().await.clone();
        self.native_owned(owner, &input.session_id, Some(&input.generation), lease).await?;
        Ok(state)
    }
    pub async fn cancel_native_auth(&self, owner: &str, input: NativeAuthRequest, lease: &CancellationToken) -> Result<NativeAuthSnapshot, String> {
        let op = self.owned_native_auth(owner, &input, lease).await?;
        request_stop(&op, "cancelled").await;
        await_stopped(&op).await;
        self.read_native_auth(owner, input, lease).await
    }
    pub async fn submit_native_auth_callback(&self, owner: &str, input: NativeAuthCallback, lease: &CancellationToken) -> Result<NativeAuthSnapshot, String> {
        let request = NativeAuthRequest { session_id: input.session_id, generation: input.generation, request_id: input.request_id };
        let op = self.owned_native_auth(owner, &request, lease).await?;
        let file = {
            let mut state = op.state.lock().await;
            if op.cancel.is_cancelled() || op.authority.iter().any(CancellationToken::is_cancelled) || state.state != "waiting_callback" || !state.callback_required || now_ms() >= state.expires_at { return Err("request_expired".into()); }
            if !state.authorization_url.as_deref().is_some_and(|auth| callback_matches(auth, &input.callback_url)) { return Err("native_auth_invalid_callback".into()); }
            op.callback_submitted.store(true, Ordering::Release);
            clear_interaction(&mut state); state.state = "starting".into();
            op.terminal.lock().map_err(|_| "request_expired")?.as_ref().ok_or("request_expired")?.try_clone().map_err(|_| "request_expired")?
        };
        let writing = op.clone();
        let result = tokio::task::spawn_blocking(move || write_callback(file, input.callback_url, &writing)).await;
        if !matches!(result, Ok(Ok(()))) { request_stop(&op, "interaction_unavailable").await; }
        self.read_native_auth(owner, request, lease).await
    }
    pub(super) async fn stop_native_auth(&self, owner: Option<&str>, session: Option<&str>) -> bool {
        let operations: Vec<_> = self.inner.native_auth.lock().await.values().filter(|op| owner.is_none_or(|owner| op.owner == owner) && session.is_none_or(|id| op.input.session_id == id)).cloned().collect();
        for op in &operations { request_stop(op, "revoked").await; }
        for op in &operations { await_stopped(op).await; }
        operations.iter().all(|op| op.stopped.load(Ordering::Acquire))
    }
}
async fn request_stop(op: &Operation, reason: &str) {
    let mut state = op.state.lock().await;
    if terminal(&state.state) { return; }
    clear_interaction(&mut state); state.state = "stopping".into(); state.reason = Some(reason.into()); op.cancel.cancel();
}
async fn await_stopped(op: &Operation) {
    let notified = op.done.notified(); tokio::pin!(notified); notified.as_mut().enable();
    if !op.stopped.load(Ordering::Acquire) { let _ = tokio::time::timeout(Duration::from_secs(5), notified).await; }
}

#[cfg(unix)]
fn write_callback(mut file: std::fs::File, mut callback: String, op: &Operation) -> Result<(), ()> {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    // Native readline may change termios after startup; set it at submission too.
    let mut term: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(file.as_raw_fd(), &mut term) } < 0 { return Err(()); }
    unsafe { libc::cfmakeraw(&mut term); }
    if unsafe { libc::tcsetattr(file.as_raw_fd(), libc::TCSANOW, &term) } < 0 { return Err(()); }
    callback.push('\n');
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut remaining = callback.as_bytes();
    while !remaining.is_empty() {
        if std::time::Instant::now() >= deadline || op.cancel.is_cancelled() || op.authority.iter().any(CancellationToken::is_cancelled) { return Err(()); }
        match file.write(&remaining[..remaining.len().min(256)]) {
            Ok(0) => return Err(()), Ok(count) => remaining = &remaining[count..],
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => return Err(()),
        }
    }
    Ok(())
}
#[cfg(not(unix))]
fn write_callback(_: std::fs::File, _: String, _: &Operation) -> Result<(), ()> { Err(()) }

#[cfg(unix)]
fn launch(op: &Operation, context: &LaunchContext, expires_at: i64) -> Result<(crate::runtime::OwnedChild, std::sync::mpsc::Receiver<Vec<u8>>), ()> {
    use std::{io::Read, os::{fd::{AsRawFd, FromRawFd}, unix::process::CommandExt}, process::Stdio};
    let mut command = std::process::Command::new(&context.executable);
    command.env_clear().envs(&context.env).current_dir(&context.workspace).env("TERM", "dumb").env("NO_COLOR", "1");
    // No shell, browser, debug logfile, model prompt or caller-selected CLI flags.
    for key in ["DEBUG", "CLAUDE_CODE_DEBUG", "CLAUDE_DEBUG", "CLAUDE_CODE_DEBUG_LOG_FILE", "RUST_LOG", "RUST_BACKTRACE", "CODEX_LOG_DIR"] { command.env_remove(key); }
    command.args(["mcp", if op.input.kind == NativeAuthKind::Login { "login" } else { "logout" }, &op.input.server_name]);
    if op.input.kind == NativeAuthKind::Login { command.arg("--no-browser"); }
    let mut master = -1; let mut slave = -1;
    if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) } < 0 { return Err(()); }
    let master = unsafe { std::fs::File::from_raw_fd(master) }; let slave = unsafe { std::fs::File::from_raw_fd(slave) };
    if unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 { return Err(()); }
    if unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 { return Err(()); }
    // Raw mode avoids canonical truncation of full callback URLs and disables echo.
    let mut term: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut term) } < 0 { return Err(()); }
    unsafe { libc::cfmakeraw(&mut term); }
    if unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &term) } < 0 { return Err(()); }
    command.stdin(Stdio::from(slave.try_clone().map_err(|_| ())?)).stdout(Stdio::from(slave.try_clone().map_err(|_| ())?)).stderr(Stdio::from(slave));
    unsafe { command.pre_exec(|| { if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as libc::c_ulong, 0) < 0 { return Err(std::io::Error::last_os_error()); } Ok(()) }); }
    let writer = master.try_clone().map_err(|_| ())?;
    // PTY preparation must not extend the admission lifetime either. A logout
    // can mutate credentials immediately on startup, before we can kill it.
    if now_ms() >= expires_at || op.cancel.is_cancelled() || op.authority.iter().any(CancellationToken::is_cancelled) { return Err(()); }
    let child = command.spawn().map_err(|_| ())?; let group = child.id() as i32;
    let child = crate::runtime::OwnedChild { child, group, reaped: false };
    *op.terminal.lock().map_err(|_| ())? = Some(writer);
    let (tx, rx) = std::sync::mpsc::sync_channel(16);
    std::thread::spawn(move || { let mut master = master; let mut buffer = [0; 4096]; loop { match master.read(&mut buffer) { Ok(0) => break, Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(10)), Err(_) => break, Ok(count) => if tx.send(buffer[..count].to_vec()).is_err() { break; } } } });
    Ok((child, rx))
}
#[cfg(unix)]
async fn supervise(op: Arc<Operation>, context: Arc<LaunchContext>, account: CancellationToken, session: CancellationToken, provider: CancellationToken, stop: CancellationToken) -> bool {
    let expires_at = op.state.lock().await.expires_at;
    if account.is_cancelled() || session.is_cancelled() || provider.is_cancelled() || stop.is_cancelled() { request_stop(&op, "revoked").await; }
    if now_ms() >= expires_at { request_stop(&op, "expired").await; }
    let launched = if op.cancel.is_cancelled() { Err(()) } else { launch(&op, &context, expires_at) };
    let Ok((mut child, output)) = launched else {
        if now_ms() >= expires_at { request_stop(&op, "expired").await; }
        else if op.authority.iter().any(CancellationToken::is_cancelled) { request_stop(&op, "revoked").await; }
        let mut state = op.state.lock().await; state.state = if op.cancel.is_cancelled() { "stopped" } else { "failed" }.into();
        if state.reason.is_none() { state.reason = Some("interaction_unavailable".into()); } clear_interaction(&mut state);
        op._permit.lock().unwrap_or_else(|e| e.into_inner()).take();
        op.stopped.store(true, Ordering::Release); op.done.notify_waiters(); return true;
    };
    let deadline = tokio::time::Instant::now() + Duration::from_millis((expires_at - now_ms()).max(0) as u64);
    let mut raw = Vec::new(); let mut total = 0usize; let mut naturally_exited = false;
    loop {
        while let Ok(bytes) = output.try_recv() {
            total += bytes.len();
            if total > 65536 { request_stop(&op, "interaction_unavailable").await; break; }
            raw.extend_from_slice(&bytes);
        }
        if !raw.is_empty() {
            let text = String::from_utf8_lossy(&raw);
            let mut state = op.state.lock().await;
            if !op.callback_submitted.load(Ordering::Acquire) && !op.cancel.is_cancelled() && state.state == "starting" && state.authorization_url.is_none() && state.kind == NativeAuthKind::Login {
                let complete = text.rfind(char::is_whitespace).map_or("", |end| &text[..end]);
                let url = complete.split_whitespace().find_map(|part| valid_authorization(part).map(|url| url.to_string()));
                if let Some(url) = url { state.authorization_url = Some(url); }
            }
            if !op.callback_submitted.load(Ordering::Acquire) && !op.cancel.is_cancelled() && state.state == "starting" && state.authorization_url.is_some() && (text.contains("Paste") || text.contains("paste")) {
                state.state = "waiting_callback".into(); state.callback_required = true;
            }
        }
        match child.exited() { Ok(true) => { naturally_exited = true; break; }, Err(_) => { request_stop(&op, "cleanup_pending").await; break; }, _ => {} }
        let reason = tokio::select! { biased;
            _ = account.cancelled() => Some("revoked"), _ = session.cancelled() => Some("revoked"),
            _ = provider.cancelled() => Some("revoked"), _ = stop.cancelled() => Some("revoked"),
            _ = op.cancel.cancelled() => Some("cancelled"), _ = tokio::time::sleep_until(deadline) => Some("expired"),
            _ = tokio::time::sleep(Duration::from_millis(40)) => None,
        };
        if let Some(reason) = reason { if !op.cancel.is_cancelled() { request_stop(&op, reason).await; } break; }
    }
    raw.clear();
    { let mut state = op.state.lock().await; clear_interaction(&mut state); if !naturally_exited { state.state = "stopping".into(); } }
    op.terminal.lock().unwrap_or_else(|e| e.into_inner()).take();
    let (exit_code, clean) = child.cleanup_auth().await;
    let mut state = op.state.lock().await; state.exit_code = exit_code;
    if !clean { state.state = "auth_unknown".into(); state.reason = Some("cleanup_pending".into()); }
    else if state.reason.is_some() || !naturally_exited { state.state = "stopped".into(); }
    else if exit_code == Some(0) { state.state = "succeeded".into(); state.credential_outcome = "confirmed".into(); }
    else { state.state = "failed".into(); state.reason = Some("command_failed".into()); }
    op.done.notify_waiters();
    drop(state);
    if !clean {
        // Keep the unreaped leader and permit; only this supervisor may retry.
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let (_, clean) = child.cleanup_auth().await;
            if clean {
                let mut state = op.state.lock().await;
                state.state = "stopped".into(); state.reason = Some("cleanup_pending".into());
                break;
            }
        }
    }
    op._permit.lock().unwrap_or_else(|e| e.into_inner()).take();
    op.stopped.store(true, Ordering::Release); op.done.notify_waiters(); true
}
#[cfg(not(unix))]
async fn supervise(op: Arc<Operation>, _: Arc<LaunchContext>, _: CancellationToken, _: CancellationToken, _: CancellationToken, _: CancellationToken) -> bool {
    let mut state = op.state.lock().await; state.state = "failed".into(); state.reason = Some("interaction_unavailable".into());
    op._permit.lock().unwrap_or_else(|e| e.into_inner()).take();
    op.stopped.store(true, Ordering::Release); op.done.notify_waiters(); true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_cannot_change_state_or_redirect_authority() {
        let auth = "https://example.test/authorize?state=one&redirect_uri=http%3A%2F%2Flocalhost%3A18999%2Fcallback";
        assert!(callback_matches(auth, "http://localhost:18999/callback?code=x&state=one"));
        for callback in ["http://localhost:18999/callback?code=x&state=old", "http://localhost:18998/callback?code=x&state=one", "http://localhost:18999/other?code=x&state=one", "http://localhost:18999/callback?code=x&state=one&state=old", "http://user@localhost:18999/callback?code=x&state=one"] { assert!(!callback_matches(auth, callback)); }
    }
    #[tokio::test]
    async fn cancellation_consumes_interaction_without_claiming_credentials_absent() {
        let input = StartNativeAuth { session_id: "session".into(), generation: "old".into(), request_id: Uuid::new_v4().to_string(), server_name: "fixture".into(), kind: NativeAuthKind::Login };
        let op = Operation { owner: "owner".into(), input: input.clone(), state: Mutex::new(NativeAuthSnapshot { session_id: input.session_id, generation: input.generation, request_id: input.request_id, server_name: input.server_name, kind: input.kind, state: "waiting_callback".into(), reason: None, credential_outcome: "unknown".into(), authorization_url: Some("transient".into()), callback_required: true, expires_at: now_ms()+180000, exit_code: None }), cancel: CancellationToken::new(), stopped: AtomicBool::new(false), done: Notify::new(), terminal: std::sync::Mutex::new(None), callback_submitted: AtomicBool::new(false), _permit: std::sync::Mutex::new(None), authority: vec![] };
        request_stop(&op, "expired").await;
        let state = op.state.lock().await;
        assert_eq!(state.state, "stopping"); assert_eq!(state.reason.as_deref(), Some("expired"));
        assert!(state.authorization_url.is_none() && !state.callback_required);
        assert_eq!(state.credential_outcome, "unknown"); assert!(!op.stopped.load(Ordering::Acquire));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn dedicated_pty_cancel_waits_and_preserves_unknown_credential_effects() {
        let root = std::env::temp_dir().join(format!("dgos-oauth-pty-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let executable = root.join("claude");
        crate::test_support::prepare_shell_fixture(&executable, r#"
printf '%s\n' 'https://example.test/authorize?state=one&redirect_uri=http%3A%2F%2Flocalhost%3A18999%2Fcallback'
printf '%s' 'Paste the complete redirect URL: '
IFS= read -r callback
printf x > callback-received
"#).await;
        let input = StartNativeAuth { session_id: "session".into(), generation: "generation".into(), request_id: Uuid::new_v4().to_string(), server_name: "fixture".into(), kind: NativeAuthKind::Login };
        let op = Arc::new(Operation { owner: "owner".into(), input: input.clone(), state: Mutex::new(NativeAuthSnapshot { session_id: input.session_id, generation: input.generation, request_id: input.request_id, server_name: input.server_name, kind: input.kind, state: "starting".into(), reason: None, credential_outcome: "unknown".into(), authorization_url: None, callback_required: false, expires_at: now_ms()+180000, exit_code: None }), cancel: CancellationToken::new(), stopped: AtomicBool::new(false), done: Notify::new(), terminal: std::sync::Mutex::new(None), callback_submitted: AtomicBool::new(false), _permit: std::sync::Mutex::new(None), authority: vec![] });
        let context = Arc::new(LaunchContext { executable, workspace: root.to_string_lossy().into(), env: BTreeMap::new() });
        let watched = op.clone();
        let task = tokio::spawn(async move { supervise(watched, context, CancellationToken::new(), CancellationToken::new(), CancellationToken::new(), CancellationToken::new()).await });
        tokio::time::timeout(Duration::from_secs(5), async { while op.state.lock().await.state != "waiting_callback" { tokio::time::sleep(Duration::from_millis(10)).await; } }).await.unwrap();
        request_stop(&op, "cancelled").await;
        assert!(task.await.unwrap());
        let state = op.state.lock().await;
        assert_eq!(state.state, "stopped"); assert_eq!(state.credential_outcome, "unknown");
        assert!(op.stopped.load(Ordering::Acquire) && state.authorization_url.is_none() && !state.callback_required);
        assert!(!root.join("callback-received").exists());
        drop(state);
        // A full URL larger than the canonical PTY line limit must arrive intact.
        let executable = root.join("long-callback");
        crate::test_support::prepare_shell_fixture(&executable, r#"
printf '%s\n' 'https://example.test/authorize?state=one&redirect_uri=http%3A%2F%2Flocalhost%3A18999%2Fcallback'
printf '%s' 'Paste the complete redirect URL: '
IFS= read -r callback
printf '%s' "$callback" > callback-received
"#).await;
        let input = op.input.clone();
        let mut snapshot = op.state.lock().await.clone(); snapshot.state = "starting".into(); snapshot.reason = None;
        let next = Arc::new(Operation { owner: "owner".into(), input, state: Mutex::new(snapshot), cancel: CancellationToken::new(), stopped: AtomicBool::new(false), done: Notify::new(), terminal: std::sync::Mutex::new(None), callback_submitted: AtomicBool::new(false), _permit: std::sync::Mutex::new(None), authority: vec![] });
        let context = Arc::new(LaunchContext { executable, workspace: root.to_string_lossy().into(), env: BTreeMap::new() });
        let watched = next.clone();
        let task = tokio::spawn(async move { supervise(watched, context, CancellationToken::new(), CancellationToken::new(), CancellationToken::new(), CancellationToken::new()).await });
        tokio::time::timeout(Duration::from_secs(5), async { while next.state.lock().await.state != "waiting_callback" { tokio::time::sleep(Duration::from_millis(10)).await; } }).await.unwrap();
        let callback = format!("http://localhost:18999/callback?state=one&code={}", "x".repeat(15000));
        let file = next.terminal.lock().unwrap().as_ref().unwrap().try_clone().unwrap();
        let writer = next.clone(); let submitted = callback.clone();
        assert!(tokio::task::spawn_blocking(move || write_callback(file, submitted, &writer)).await.unwrap().is_ok());
        assert!(task.await.unwrap());
        assert_eq!(std::fs::read_to_string(root.join("callback-received")).unwrap(), callback);
        assert_eq!(next.state.lock().await.state, "succeeded");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn ambiguous_authorization_and_callback_parameters_are_rejected() {
        let auth = "https://example.test/authorize?state=one&redirect_uri=http%3A%2F%2Flocalhost%3A18999%2Fcallback";
        for suffix in ["&state=two", "&redirect_uri=https%3A%2F%2Fother.test"] { assert!(valid_authorization(&format!("{auth}{suffix}")).is_none()); }
        for query in ["code=x&code=y&state=one", "code=x&error=denied&state=one", "code=&state=one"] {
            assert!(!callback_matches(auth, &format!("http://localhost:18999/callback?{query}")));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn expired_login_and_logout_never_execute_the_cli() {
        let root = std::env::temp_dir().join(format!("dgos-oauth-expiry-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let executable = root.join("codex");
        // An immediate credential-write surrogate catches a too-late kill after
        // spawn; blocking on input would conceal that the process ever ran.
        crate::test_support::prepare_shell_fixture(&executable, "printf x > credential-write\nexit 0\n").await;
        for kind in [NativeAuthKind::Login, NativeAuthKind::Logout] {
        let input = StartNativeAuth { session_id: "session".into(), generation: "generation".into(), request_id: Uuid::new_v4().to_string(), server_name: "fixture".into(), kind };
        let op = Arc::new(Operation { owner: "owner".into(), input: input.clone(), state: Mutex::new(NativeAuthSnapshot { session_id: input.session_id, generation: input.generation, request_id: input.request_id, server_name: input.server_name, kind: input.kind, state: "starting".into(), reason: None, credential_outcome: "unknown".into(), authorization_url: None, callback_required: false, expires_at: now_ms()-1, exit_code: None }), cancel: CancellationToken::new(), stopped: AtomicBool::new(false), done: Notify::new(), terminal: std::sync::Mutex::new(None), callback_submitted: AtomicBool::new(false), _permit: std::sync::Mutex::new(None), authority: vec![] });
        let context = Arc::new(LaunchContext { executable: executable.clone(), workspace: root.to_string_lossy().into(), env: BTreeMap::new() });
        assert!(tokio::time::timeout(Duration::from_secs(5), supervise(op.clone(), context, CancellationToken::new(), CancellationToken::new(), CancellationToken::new(), CancellationToken::new())).await.unwrap());
        let state = op.state.lock().await;
        assert_eq!(state.state, "stopped"); assert_eq!(state.reason.as_deref(), Some("expired"));
        assert_eq!(state.credential_outcome, "unknown");
        assert!(op.stopped.load(Ordering::Acquire) && state.authorization_url.is_none() && !state.callback_required);
        assert_eq!(state.exit_code, None, "expired requests never spawn");
        assert!(!root.join("credential-write").exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
