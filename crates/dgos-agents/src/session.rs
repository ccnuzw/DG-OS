use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

pub type SessionError = String;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    Codex,
    Claude,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionOptions {
    pub workspace: String,
    pub resume_id: Option<String>,
    pub executable: Option<String>,
    pub model: Option<String>,
    pub trust_workspace: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub seq: u64,
    pub kind: String,
    pub text: Option<String>,
    pub data: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingRequest {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub data: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionView {
    pub native_id: Option<String>,
    pub workspace: String,
    pub status: String,
    pub events: Vec<SessionEvent>,
    pub pending: Vec<PendingRequest>,
    pub error: Option<String>,
    #[serde(default)]
    pub history_truncated: bool,
    #[serde(skip)]
    retained_bytes: usize,
    #[serde(skip)]
    pub active_run: Option<String>,
}
impl SessionView {
    pub fn new(workspace: String) -> Self {
        Self {
            native_id: None,
            workspace,
            status: "starting".into(),
            events: vec![],
            pending: vec![],
            error: None,
            history_truncated: false,
            retained_bytes: 0,
            active_run: None,
        }
    }
    pub fn request(&mut self, pending: PendingRequest) {
        if let Some(existing) = self.pending.iter_mut().find(|p| p.id == pending.id) {
            *existing = pending;
        } else {
            self.pending.push(pending);
        }
        self.status = "waiting".into();
    }
    pub fn push(&mut self, kind: &str, text: Option<String>, data: Value) {
        self.push_run(kind, text, data, self.active_run.clone());
    }
    pub fn push_run(&mut self, kind: &str, text: Option<String>, data: Value, run: Option<String>) {
        let seq = self.events.last().map_or(1, |event| event.seq + 1);
        let size = |event: &SessionEvent| {
            event.text.as_ref().map_or(0, String::len) + json_size(&event.data)
        };
        let incoming = text.as_ref().map_or(0, String::len) + json_size(&data);
        while !self.events.is_empty()
            && (self.events.len() >= 2000 || self.retained_bytes + incoming > 16 * 1024 * 1024)
        {
            self.retained_bytes -= size(&self.events.remove(0));
            self.history_truncated = true;
        }
        self.retained_bytes += incoming;
        self.events.push(SessionEvent {
            seq,
            kind: kind.into(),
            text,
            data,
            run,
        });
    }
    pub(crate) fn replace_history_event(&mut self, seq: u64, kind: &str, text: Option<String>, data: Value) -> bool {
        let Some(event) = self.events.iter_mut().find(|event| event.seq == seq) else { return false; };
        let old = event.text.as_ref().map_or(0, String::len) + json_size(&event.data);
        let incoming = text.as_ref().map_or(0, String::len) + json_size(&data);
        if self.retained_bytes - old + incoming > 16 * 1024 * 1024 {
            self.history_truncated = true;
            return false;
        }
        self.retained_bytes = self.retained_bytes - old + incoming;
        event.kind = kind.into();
        event.text = text;
        event.data = data;
        true
    }
}
pub fn resolve_executable(name: &str, explicit: Option<&str>) -> Result<PathBuf, String> {
    if let Some(explicit) = explicit.filter(|s| !s.is_empty()) {
        let path = PathBuf::from(explicit);
        return if path.is_absolute() && is_executable_file(&path) {
            Ok(path)
        } else {
            Err("invalid_executable".into())
        };
    }
    let mut directories: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        directories.extend([
            home.join(".local/bin"),
            home.join(".npm-global/bin"),
            home.join(".bun/bin"),
        ]);
    }
    directories.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    directories
        .into_iter()
        .map(|p| p.join(name))
        .find(|p| is_executable_file(p))
        .ok_or_else(|| "adapter_unavailable".into())
}

fn is_executable_file(path: &std::path::Path) -> bool {
    let Ok(metadata) = path.metadata() else { return false; };
    if !metadata.is_file() { return false; }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    { true }
}

#[cfg(all(test, unix))]
mod executable_tests {
    use super::resolve_executable;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[test]
    fn explicit_cli_requires_executable_file_and_allows_install_symlink() {
        let root = std::env::temp_dir().join(format!("dgos-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let cli = root.join("cli");
        std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(resolve_executable("cli", cli.to_str()), Err("invalid_executable".into()));
        assert_eq!(resolve_executable("cli", root.to_str()), Err("invalid_executable".into()));
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = root.join("cli-link");
        symlink(&cli, &link).unwrap();
        assert_eq!(resolve_executable("cli", link.to_str()), Ok(link.clone()));
        std::fs::remove_file(&cli).unwrap();
        assert_eq!(resolve_executable("cli", link.to_str()), Err("invalid_executable".into()));
        std::fs::remove_dir_all(root).unwrap();
    }
}

fn json_size(value: &Value) -> usize {
    match value {
        Value::String(s) => s.len(),
        Value::Array(a) => a.iter().map(json_size).sum(),
        Value::Object(o) => o.iter().map(|(k, v)| k.len() + json_size(v)).sum(),
        _ => 16,
    }
}

#[cfg(test)]
mod pending_tests {
    use super::*;

    #[test]
    fn repeated_native_request_replaces_payload_without_duplicate_controls() {
        let mut view = SessionView::new("/tmp".into());
        for title in ["first", "updated"] {
            view.request(PendingRequest { id: "request".into(), kind: "approval".into(), title: title.into(), data: Value::Null });
        }
        assert_eq!(view.pending.len(), 1);
        assert_eq!(view.pending[0].title, "updated");
        assert_eq!(view.status, "waiting");
    }
}
