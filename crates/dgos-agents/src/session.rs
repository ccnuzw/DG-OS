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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode { Default, ReadOnly, AcceptEdits, FullAuto }
pub const AGENT_SKILL_LIMIT: usize = 8;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSkillInput { pub name: String, pub path: String }
pub(crate) fn validate_skill_inputs(skills: &[AgentSkillInput]) -> Result<(), String> {
    if skills.len() > AGENT_SKILL_LIMIT { return Err("agent_skill_limit".into()); }
    for (index, skill) in skills.iter().enumerate() {
        if crate::native_session::safe_name(&skill.name).is_none() || skill.path.is_empty() || skill.path.len() > 2048 || skill.path.chars().any(char::is_control)
            || skills[..index].iter().any(|old| old == skill) { return Err("agent_skill_invalid".into()); }
    }
    Ok(())
}
pub(crate) fn validate_discovered_skills(skills: &[AgentSkillInput], observed: &crate::native_session::Projection) -> Result<(), String> {
    if observed.incomplete { return Err("agent_skills_unavailable".into()); }
    for skill in skills {
        let mut matches = observed.skills.iter().filter(|item| item.name == skill.name && item.path.as_deref() == Some(skill.path.as_str()));
        let item = matches.next().ok_or("agent_skill_unavailable")?;
        if matches.next().is_some() || item.enabled != Some(true) || item.classification != crate::native_session::NativeSkillClassification::Skill { return Err("agent_skill_unavailable".into()); }
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationMode { Default, Plan, Deep }
impl CollaborationMode {
    pub(crate) fn as_str(self) -> &'static str { match self { Self::Default => "default", Self::Plan => "plan", Self::Deep => "deep" } }
    pub(crate) fn planning(self) -> bool { matches!(self, Self::Plan | Self::Deep) }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SteeringState { Applied, Unknown }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SteeringReceipt {
    pub request_id: String,
    pub run_id: String,
    pub state: SteeringState,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResponsePreferences {
    pub language: ResponseLanguage,
    pub brevity: ResponseBrevity,
    pub next_step_suggestions: bool,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseLanguage { FollowInput, Zh, En }
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseBrevity { Concise, Standard, Detailed }
impl ResponsePreferences {
    pub(crate) fn instructions(&self) -> String {
        let language = match self.language {
            ResponseLanguage::FollowInput => "Match the user's input language unless the user explicitly requests another language.",
            ResponseLanguage::Zh => "Respond in Chinese unless the user explicitly requests another language.",
            ResponseLanguage::En => "Respond in English unless the user explicitly requests another language.",
        };
        let brevity = match self.brevity {
            ResponseBrevity::Concise => "Give a direct, concise answer.",
            ResponseBrevity::Standard => "Use normal detail appropriate to the task.",
            ResponseBrevity::Detailed => "Give a thorough explanation with useful details.",
        };
        let suggestions = if self.next_step_suggestions { "Include useful next-step suggestions when appropriate." } else { "Omit unsolicited next-step suggestions." };
        format!("Session response preferences: {language} {brevity} {suggestions}")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentTurnSettings {
    #[serde(default, deserialize_with = "nullable_effort", skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<Option<String>>,
    #[serde(default, deserialize_with = "optional_permission", skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default, deserialize_with = "optional_collaboration", skip_serializing_if = "Option::is_none")]
    pub collaboration_mode: Option<CollaborationMode>,
    #[serde(default, deserialize_with = "optional_response_preferences", skip_serializing_if = "Option::is_none")]
    pub response_preferences: Option<ResponsePreferences>,
}
impl AgentTurnSettings {
    pub(crate) fn is_empty(&self) -> bool { self.reasoning_effort.is_none() && self.permission_mode.is_none() && self.collaboration_mode.is_none() && self.response_preferences.is_none() }
}
fn nullable_effort<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}
fn optional_permission<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<PermissionMode>, D::Error> {
    PermissionMode::deserialize(deserializer).map(Some)
}
fn optional_collaboration<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<CollaborationMode>, D::Error> {
    CollaborationMode::deserialize(deserializer).map(Some)
}
fn optional_response_preferences<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<ResponsePreferences>, D::Error> {
    ResponsePreferences::deserialize(deserializer).map(Some)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionControl<T> {
    pub available: bool,
    pub options: Vec<T>,
    pub current: Option<T>,
    pub source: String,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionControls {
    pub reasoning: SessionControl<String>,
    pub permission: SessionControl<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collaboration: Option<SessionControl<String>>,
    #[serde(default)]
    pub steering: bool,
    #[serde(skip)]
    pub(crate) bound_model: Option<String>,
    #[serde(skip)]
    pub(crate) default_effort: Option<String>,
    #[serde(skip)]
    pub(crate) default_sandbox: Option<Value>,
    #[serde(skip)]
    pub(crate) response_preferences: Option<ResponsePreferences>,
}
impl SessionControls {
    pub fn validate(&self, settings: &AgentTurnSettings) -> Result<(), String> {
        if let Some(effort) = &settings.reasoning_effort {
            if !self.reasoning.available || effort.as_ref().is_some_and(|value| !self.reasoning.options.contains(value)) {
                return Err("unsupported_reasoning_effort".into());
            }
        }
        if let Some(permission) = settings.permission_mode {
            if !self.permission.available || !self.permission.options.contains(&permission) {
                return Err("unsupported_permission_mode".into());
            }
        }
        if let Some(mode) = settings.collaboration_mode {
            if self.collaboration.as_ref().is_none_or(|control| !control.available || !control.options.iter().any(|option| option == mode.as_str())) {
                return Err("unsupported_collaboration_mode".into());
            }
        }
        Ok(())
    }
    pub fn uncertain(&mut self) {
        self.reasoning.current = None;
        self.reasoning.reason = Some("result_unknown".into());
        self.permission.current = None;
        self.permission.reason = Some("result_unknown".into());
        if let Some(control) = &mut self.collaboration { control.current = None; control.reason = Some("result_unknown".into()); }
        self.steering = false;
    }
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
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCursor {
    pub generation: String,
    pub after_seq: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct EventSync {
    pub reset: bool,
    pub after_seq: u64,
    pub first_seq: u64,
}
#[derive(Debug, Clone)]
pub(crate) struct RunTerminal {
    pub run_id: String,
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionView {
    pub native_id: Option<String>,
    pub workspace: String,
    pub status: String,
    pub events: Vec<SessionEvent>,
    pub pending: Vec<PendingRequest>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistence_error: Option<String>,
    #[serde(default)]
    pub history_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<SessionControls>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_version: Option<String>,
    #[serde(skip)]
    retained_bytes: usize,
    #[serde(skip)]
    output_images: crate::images::OutputImageCache,
    // Public observation for run-bound controls; persisted JSON cannot revive execution.
    #[serde(skip_deserializing)]
    pub active_run: Option<String>,
    // The next run is admitted only after this terminal has been durably consumed.
    #[serde(skip)]
    pub(crate) terminal_run: Option<RunTerminal>,
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
            persistence_error: None,
            history_truncated: false,
            controls: None,
            cli_version: None,
            retained_bytes: 0,
            output_images: crate::images::OutputImageCache::default(),
            active_run: None,
            terminal_run: None,
        }
    }
    /// Live histories are append-only after initialization. Reset whenever the
    /// client's cursor no longer covers the retained window; approvals and state
    /// are always current, even when there are no new transcript events.
    pub(crate) fn snapshot_since(&self, after: Option<u64>) -> (Self, EventSync) {
        let first = self.events.first().map_or(0, |event| event.seq);
        let last = self.events.last().map_or(0, |event| event.seq);
        let append = after.is_some_and(|seq| seq <= last && seq.saturating_add(1) >= first);
        let after_seq = after.filter(|_| append).unwrap_or(0);
        let start = if append { self.events.partition_point(|event| event.seq <= after_seq) } else { 0 };
        let view = Self {
            native_id: self.native_id.clone(),
            workspace: self.workspace.clone(),
            status: self.status.clone(),
            events: self.events[start..].to_vec(),
            pending: self.pending.clone(),
            error: self.error.clone(),
            persistence_error: self.persistence_error.clone(),
            history_truncated: self.history_truncated,
            controls: self.controls.clone(),
            cli_version: self.cli_version.clone(),
            retained_bytes: 0,
            output_images: crate::images::OutputImageCache::default(),
            active_run: self.active_run.clone(),
            terminal_run: self.terminal_run.clone(),
        };
        (view, EventSync { reset: !append, after_seq, first_seq: first })
    }
    pub fn preview_image(&self, id: &str) -> Result<crate::images::OutputImagePreview, String> {
        if matches!(self.status.as_str(), "closed" | "unknown") { return Err("image_preview_expired".into()); }
        self.output_images.preview(id)
    }
    pub(crate) fn clear_output_image_previews(&mut self) { self.output_images.clear(); }
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
    pub fn push_run(&mut self, kind: &str, text: Option<String>, mut data: Value, run: Option<String>) {
        let live_output = run.is_some() && !matches!(self.status.as_str(), "starting" | "closed" | "unknown") && data.get("history") != Some(&Value::Bool(true));
        if matches!(self.status.as_str(), "closed" | "unknown") || matches!(kind, "host/closed" | "closed") { self.clear_output_image_previews(); }
        crate::images::redact_native_images(kind, &mut data, live_output.then_some(&mut self.output_images));
        let text = text.map(|text| { let mut value = Value::String(text); crate::images::redact_image_inputs(&mut value); match value { Value::String(text) => text, _ => "[image redacted]".into() } });
        if let (Some(run_id), Some(status)) = (run.as_ref(), data.get("run_status").and_then(Value::as_str)) {
            if matches!(status, "completed" | "failed" | "cancelled" | "unknown") {
                self.terminal_run = Some(RunTerminal { run_id: run_id.clone(), status: status.into() });
            }
        }
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
    pub(crate) fn replace_history_event(&mut self, seq: u64, kind: &str, text: Option<String>, mut data: Value) -> bool {
        crate::images::redact_native_images(kind, &mut data, None);
        let text = text.map(|text| { let mut value = Value::String(text); crate::images::redact_image_inputs(&mut value); match value { Value::String(text) => text, _ => "[image redacted]".into() } });
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
    #[test]
    fn turn_settings_distinguish_omitted_from_explicit_default() {
        let empty: AgentTurnSettings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(empty.is_empty());
        assert_eq!(serde_json::to_value(empty).unwrap(), serde_json::json!({}));
        let reset: AgentTurnSettings = serde_json::from_value(serde_json::json!({"reasoning_effort":null})).unwrap();
        assert_eq!(reset.reasoning_effort, Some(None));
        assert_eq!(reset.permission_mode, None);
        assert_eq!(serde_json::to_value(reset).unwrap(), serde_json::json!({"reasoning_effort":null}));
        let permission: AgentTurnSettings = serde_json::from_value(serde_json::json!({"permission_mode":"accept_edits"})).unwrap();
        assert_eq!(permission.reasoning_effort, None);
        assert_eq!(permission.permission_mode, Some(PermissionMode::AcceptEdits));
        for (name, expected) in [("default", CollaborationMode::Default), ("plan", CollaborationMode::Plan), ("deep", CollaborationMode::Deep)] {
            let mode: AgentTurnSettings = serde_json::from_value(serde_json::json!({"collaboration_mode":name})).unwrap();
            assert_eq!(mode.collaboration_mode, Some(expected));
        }
        for value in [Value::Null, serde_json::json!("read_only"), serde_json::json!({"mode":"plan","settings":{"model":"injected"}})] {
            assert!(serde_json::from_value::<AgentTurnSettings>(serde_json::json!({"collaboration_mode":value})).is_err());
        }
        for invalid in [serde_json::json!({"permission_mode":null}), serde_json::json!({"permission_mode":"bypassPermissions"}), serde_json::json!({"reasoning_effort":2}), serde_json::json!({"always_allow":true})] {
            assert!(serde_json::from_value::<AgentTurnSettings>(invalid).is_err());
        }
    }
    #[test]
    fn response_preferences_round_trip_without_exposing_internal_session_settings() {
        for language in ["follow_input", "zh", "en"] {
            for brevity in ["concise", "standard", "detailed"] {
                let value = serde_json::json!({"response_preferences":{"language":language,"brevity":brevity,"next_step_suggestions":false}});
                let settings: AgentTurnSettings = serde_json::from_value(value.clone()).unwrap();
                assert!(!settings.is_empty());
                assert_eq!(serde_json::to_value(&settings).unwrap(), value);
                assert!(settings.permission_mode.is_none() && settings.collaboration_mode.is_none() && settings.reasoning_effort.is_none());
                let controls = SessionControls {
                    reasoning: SessionControl { available: false, options: vec![], current: None, source: "fixture".into(), reason: None },
                    permission: SessionControl { available: false, options: vec![], current: None, source: "fixture".into(), reason: None },
                    collaboration: None, steering: false, bound_model: None, default_effort: None, default_sandbox: None,
                    response_preferences: settings.response_preferences.clone(),
                };
                assert!(controls.validate(&settings).is_ok());
                assert!(serde_json::to_value(controls).unwrap().get("response_preferences").is_none());
            }
        }
        for preferences in [Value::Null, serde_json::json!({}), serde_json::json!({"language":"fr","brevity":"standard","next_step_suggestions":true}), serde_json::json!({"language":"en","brevity":"standard","next_step_suggestions":null}), serde_json::json!({"language":"en","brevity":"standard","next_step_suggestions":true,"permission_mode":"full_auto"})] {
            assert!(serde_json::from_value::<AgentTurnSettings>(serde_json::json!({"response_preferences":preferences})).is_err());
        }
    }
}

#[cfg(test)]
mod synchronization_tests {
    use super::*;

    #[test]
    fn delta_keeps_current_approval_state_and_resets_outside_retained_window() {
        let mut view = SessionView::new("/tmp/project".into());
        for index in 0..2002 { view.push("output", Some(index.to_string()), Value::Null); }
        view.request(PendingRequest { id: "approval".into(), kind: "command".into(), title: "Approve".into(), data: Value::Null });
        let (delta, sync) = view.snapshot_since(Some(2001));
        assert!(!sync.reset);
        assert_eq!(sync.first_seq, 3);
        assert_eq!(delta.events.iter().map(|event| event.seq).collect::<Vec<_>>(), vec![2002]);
        assert_eq!(delta.pending[0].id, "approval");
        assert_eq!(delta.status, "waiting");
        let (unchanged, sync) = view.snapshot_since(Some(2002));
        assert!(!sync.reset);
        assert!(unchanged.events.is_empty());
        assert_eq!(unchanged.pending[0].id, "approval");
        for cursor in [None, Some(0), Some(1), Some(2003), Some(u64::MAX)] {
            let (full, sync) = view.snapshot_since(cursor);
            assert!(sync.reset);
            assert_eq!(full.events.first().unwrap().seq, 3);
            assert_eq!(full.events.last().unwrap().seq, 2002);
        }
        let (delta, sync) = view.snapshot_since(Some(2));
        assert!(!sync.reset);
        assert_eq!(delta.events.first().unwrap().seq, 3);
    }
}
