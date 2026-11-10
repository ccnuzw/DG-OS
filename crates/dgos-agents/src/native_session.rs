//! Bounded, credential-free projections of existing native control channels.
use serde::{Deserialize, Serialize};
use serde_json::Value;

// OAuth snapshots are transient IPC only; never append them to SessionView or journals.
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NativeAuthKind { Login, Logout }
#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StartNativeAuth {
    pub session_id: String, pub generation: String, pub request_id: String,
    pub server_name: String, pub kind: NativeAuthKind,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAuthRequest { pub session_id: String, pub generation: String, pub request_id: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAuthCallback {
    pub session_id: String, pub generation: String, pub request_id: String, pub callback_url: String,
}
#[derive(Clone, Serialize)]
pub struct NativeAuthSnapshot {
    pub session_id: String, pub generation: String, pub request_id: String,
    pub server_name: String, pub kind: NativeAuthKind, pub state: String,
    pub reason: Option<String>, pub credential_outcome: String,
    pub authorization_url: Option<String>, pub callback_required: bool,
    pub expires_at: i64, pub exit_code: Option<i32>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionDomain { Skills, Mcp }
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeSessionAction {
    ReloadSkills {},
    ReloadPlugins { hold_on_cache_impact: bool },
    ReconnectMcp { server_name: String },
    ToggleMcp { server_name: String, enabled: bool },
    NativeLogin { server_name: String },
    NativeLogout { server_name: String },
}
impl NativeSessionAction {
    pub fn domain(&self) -> SessionDomain {
        match self { Self::ReloadSkills {} | Self::ReloadPlugins { .. } => SessionDomain::Skills, _ => SessionDomain::Mcp }
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            Self::ReconnectMcp { server_name } | Self::ToggleMcp { server_name, .. } | Self::NativeLogin { server_name } | Self::NativeLogout { server_name } if safe_name(server_name).is_none() => Err("native_extension_InvalidFormat".into()),
            _ => Ok(()),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NativeSessionCapabilities {
    pub observe_skills: bool, pub observe_mcp: bool, pub reload_skills: bool,
    pub reload_plugins: bool, pub reconnect_mcp: bool, pub toggle_mcp: bool,
    pub native_login: bool, pub native_logout: bool,
    pub native_auth_guidance: Option<NativeAuthGuidance>,
    pub mcp_control_guidance: Option<NativeMcpControlGuidance>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeMcpControlGuidance {
    pub verified_version: String, pub reason: String, pub inspect_argv: Vec<String>,
}
impl NativeMcpControlGuidance {
    pub(crate) fn codex() -> Self {
        // 0.167.0 sessionless experimental schema: config/mcpServer/reload has
        // null params and an empty response. It is not a per-thread/server
        // reconnect or toggle contract and cannot certify runtime activation.
        Self { verified_version: crate::cli::codex_protocol_reference().version.into(),
            reason: "missing_session_scoped_mcp_control_contract".into(),
            inspect_argv: ["codex", "mcp", "get", "<NAME>"].map(str::to_owned).into() }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeAuthGuidance {
    pub verified_version: String, pub reason: String,
    pub login_argv: Vec<String>, pub logout_argv: Vec<String>,
}
impl NativeAuthGuidance {
    pub(crate) fn installed(engine: crate::Engine) -> Self {
        let (command, version, reason) = match engine {
            crate::Engine::Codex => ("codex", crate::cli::codex_protocol_reference().version, "isolated_auth_process_unavailable"),
            crate::Engine::Claude => ("claude", "2.1.319", "missing_session_interaction_cancel_contract"),
        };
        Self { verified_version: version.into(), reason: reason.into(),
            login_argv: [command, "mcp", "login", "<NAME>"].map(str::to_owned).into(),
            logout_argv: [command, "mcp", "logout", "<NAME>"].map(str::to_owned).into() }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NativeSkillClassification { Skill, BuiltinCommand, Unknown }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeSkillStatus {
    pub name: String, pub enabled: Option<bool>, pub classification: NativeSkillClassification,
    pub builtin: Option<bool>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub plugin_name: Option<String>,
    // Native-reported provenance only. No root, instance token or file authority is issued here.
    pub path: Option<String>, pub scope: Option<String>, pub plugin_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeMcpCallEvidence {
    pub tool: String, pub status: String, pub event_seq: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeMcpStatus {
    pub name: String, pub status: String, pub auth_status: Option<String>,
    pub source: Option<String>, pub scope: Option<String>, pub tools: Vec<String>,
    pub schema_observed: bool,
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub last_call: Option<NativeMcpCallEvidence>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeSkillCandidate {
    pub name: String, pub source: String, pub state: String, pub advertised: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeSessionObservation {
    pub session_id: String, pub generation: String, pub engine: crate::Engine,
    pub observed_at: i64, pub domain: SessionDomain, pub capabilities: NativeSessionCapabilities,
    pub skills: Vec<NativeSkillStatus>, pub mcp_servers: Vec<NativeMcpStatus>, pub incomplete: bool,
    #[serde(default)]
    pub source_incomplete: bool,
    pub skill_candidates: Vec<NativeSkillCandidate>,
    pub skill_candidates_incomplete: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeSessionHandle {
    pub session_id: String, pub generation: String, pub project_id: String,
    pub engine: crate::Engine, pub capabilities: NativeSessionCapabilities,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NativeActionState { Applied, Held, Unknown }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeSessionReceipt {
    pub session_id: String, pub generation: String, pub request_id: String,
    pub state: NativeActionState,
}
#[derive(Default)]
pub(crate) struct Projection {
    pub skills: Vec<NativeSkillStatus>, pub mcp_servers: Vec<NativeMcpStatus>, pub incomplete: bool,
    pub source_incomplete: bool,
    pub skill_candidates: Vec<NativeSkillCandidate>, pub skill_candidates_incomplete: bool,
}
pub(crate) fn safe_name(text: &str) -> Option<String> {
    // Names are identifiers, never arbitrary native descriptions, URLs, errors or paths.
    (!text.is_empty() && text.len() <= 128 && text.bytes().all(|c| c.is_ascii_alphanumeric() || b"_-.:@".contains(&c)))
        .then(|| text.to_owned())
}
fn safe_command_name(text: &str) -> Option<String> {
    (!text.is_empty() && text.len() <= 128 && !text.starts_with('/')
        && text.bytes().all(|c| c.is_ascii_alphanumeric() || b"_-.:@/".contains(&c)))
        .then(|| text.to_owned())
}
fn name(value: &Value) -> Option<String> { safe_name(value.as_str()?) }
fn category(value: &Value, choices: &[&str]) -> Option<String> {
    value.as_str().filter(|text| choices.contains(text)).map(str::to_owned)
}
fn observed_text(value: &Value, limit: usize, incomplete: &mut bool) -> Option<String> {
    if value.is_null() { return None; }
    match value.as_str().filter(|text| !text.is_empty() && text.len() <= limit && !text.chars().any(char::is_control)) {
        Some(text) => Some(text.to_owned()),
        None => { *incomplete = true; None }
    }
}
pub(crate) fn skills(values: &[Value], codex: bool, out: &mut Projection) {
    out.incomplete |= values.len() > 256;
    // Command identifiers and aliases are dispatch tokens, never resource paths.
    let mut dispatch_names = std::collections::HashMap::new();
    if !codex {
        out.source_incomplete = true;
        for value in values {
            let mut names = std::collections::HashSet::new();
            if let Some(name) = value["name"].as_str() { names.insert(name); }
            if let Some(aliases) = value["aliases"].as_array() {
                names.extend(aliases.iter().filter_map(Value::as_str));
            }
            for name in names { *dispatch_names.entry(name).or_insert(0usize) += 1; }
        }
    }
    for value in values.iter().take(256) {
        if out.skills.len() >= 256 { out.incomplete = true; break; }
        let Some(name) = value["name"].as_str().and_then(if codex { safe_name } else { safe_command_name }) else { out.incomplete = true; continue; };
        let mut aliases = Vec::new();
        if !codex && !value["aliases"].is_null() {
            if let Some(values) = value["aliases"].as_array() {
                out.incomplete |= values.len() > 64;
                for alias in values.iter().take(64) {
                    if let Some(alias) = alias.as_str().and_then(safe_command_name) {
                        if !aliases.contains(&alias) { aliases.push(alias); }
                    } else { out.incomplete = true; }
                }
            } else { out.incomplete = true; }
        }
        let (enabled, builtin, path, scope, plugin_id, source, plugin_name, classification) = if codex {
            let path = observed_text(&value["path"], 2048, &mut out.incomplete);
            let scope = category(&value["scope"], &["user", "repo", "system", "admin"]);
            let plugin_id = observed_text(&value["pluginId"], 256, &mut out.incomplete);
            let enabled = value["enabled"].as_bool();
            out.incomplete |= path.is_none() || scope.is_none() || enabled.is_none();
            (enabled, None, path, scope, plugin_id, None, None, NativeSkillClassification::Skill)
        } else {
            let builtin = value["builtin"].as_bool();
            out.incomplete |= !value["builtin"].is_null() && builtin.is_none();
            let active = &value["active_source"];
            let reliable = builtin != Some(true) && dispatch_names.get(name.as_str()) == Some(&1) && active.is_object();
            let source = if reliable { observed_text(&active["source"], 128, &mut out.source_incomplete) } else { None };
            let (path, plugin_id, plugin_name) = if source.is_some() {
                (observed_text(&active["path"], 2048, &mut out.source_incomplete),
                    observed_text(&active["plugin_id"], 256, &mut out.source_incomplete),
                    observed_text(&active["plugin_name"], 256, &mut out.source_incomplete))
            } else { (None, None, None) };
            let classification = if builtin == Some(true) { NativeSkillClassification::BuiltinCommand }
                else if source.is_some() { NativeSkillClassification::Skill } else { NativeSkillClassification::Unknown };
            (None, builtin, path, None, plugin_id, source, plugin_name, classification)
        };
        out.skills.push(NativeSkillStatus { name, aliases, enabled, classification, builtin, path, scope, plugin_id, source, plugin_name });
    }
}
pub(crate) fn skill_candidates(value: &Value, out: &mut Projection) {
    let Some(values) = value.as_array() else { out.skill_candidates_incomplete = true; return; };
    out.skill_candidates_incomplete |= values.len() > 256;
    for value in values.iter().take(256) {
        let name = value["name"].as_str().and_then(safe_command_name);
        let source = observed_text(&value["source"], 128, &mut out.skill_candidates_incomplete);
        let state = category(&value["state"], &["on", "name-only", "user-invocable-only", "off"]);
        if let (Some(name), Some(source), Some(state), Some(advertised)) = (name, source, state, value["advertised"].as_bool()) {
            out.skill_candidates.push(NativeSkillCandidate { name, source, state, advertised });
        } else { out.skill_candidates_incomplete = true; }
    }
}
fn tool_names<'a>(names: impl Iterator<Item = Option<&'a str>>, tools: &mut Vec<String>, incomplete: &mut bool) {
    for (index, candidate) in names.enumerate() {
        if index >= 64 { *incomplete = true; break; }
        if let Some(name) = candidate.and_then(safe_name) { tools.push(name); } else { *incomplete = true; }
    }
}
pub(crate) fn mcp(values: &[Value], codex: bool, out: &mut Projection) {
    out.incomplete |= values.len() > 256;
    for value in values.iter().take(256) {
        if out.mcp_servers.len() >= 256 { out.incomplete = true; break; }
        let Some(name) = value.get("name").and_then(name) else { out.incomplete = true; continue; };
        let status = category(&value[if codex { "runtimeStatus" } else { "status" }], &["notStarted", "starting", "connected", "authenticationRequired", "failed", "needs-auth", "pending", "disabled", "cancelled"]).unwrap_or_else(|| "unknown".into());
        out.incomplete |= status == "unknown" || out.mcp_servers.iter().any(|server| server.name == name);
        let mut tools = Vec::new();
        // Native catalogs may retain the last successful handshake after disconnection.
        // Only a currently connected server supplies a current tool list.
        if status == "connected" {
            if let Some(candidates) = value["tools"].as_object().filter(|_| codex) {
                tool_names(candidates.keys().map(|key| Some(key.as_str())), &mut tools, &mut out.incomplete);
            } else if let Some(candidates) = value["tools"].as_array() {
                tool_names(candidates.iter().map(|v| v["name"].as_str()), &mut tools, &mut out.incomplete);
            } else { out.incomplete = true; }
        }
        if codex && status == "connected" { out.incomplete |= value.get("toolsError").is_some_and(|error| !error.is_null()); }
        let plugin_id = if codex { observed_text(&value["pluginId"], 256, &mut out.source_incomplete) } else { None };
        // Codex reports pluginId, not a configuration layer/path. Never invent
        // provenance from fields absent from its verified status contract.
        let source = if codex { plugin_id.as_ref().map(|_| "plugin".into()) }
            else { category(&value["source"], &["sdk", "plugin", "user", "project", "local", "dynamic", "managed", "enterprise", "claudeai", "agent"]) };
        out.mcp_servers.push(NativeMcpStatus { name, status,
            auth_status: category(&value["authStatus"], &["unknown", "unsupported", "notLoggedIn", "bearerToken", "oAuth"]),
            source, scope: if codex { None } else { category(&value["scope"], &["user", "project", "local", "managed", "claudeai"]) },
            tools, schema_observed: false, plugin_id, last_call: None });
    }
}

/// Retained events were already scoped to this native thread/turn. Project only
/// the latest observed call per discovered server; never arguments or outputs.
/// This is historical evidence, not a continuing connection or tool approval.
pub(crate) fn mcp_call_evidence(events: &[crate::session::SessionEvent], out: &mut Projection) {
    for event in events.iter().rev() {
        if !matches!(event.kind.as_str(), "mcpToolCall" | "item/started") || event.data["type"] != "mcpToolCall" { continue; }
        let item = &event.data;
        let (Some(server), Some(tool), Some(status)) = (
            item["server"].as_str().and_then(safe_name), item["tool"].as_str().and_then(safe_name),
            category(&item["status"], &["inProgress", "completed", "failed"]),
        ) else { continue; };
        for observed in out.mcp_servers.iter_mut().filter(|observed| observed.name == server && observed.last_call.is_none()) {
            observed.last_call = Some(NativeMcpCallEvidence { tool: tool.clone(), status: status.clone(), event_seq: event.seq });
        }
    }
}

pub(crate) fn auth_server_allowed(engine: crate::Engine, observed: &Projection, name: &str) -> bool {
    if observed.incomplete { return false; }
    let mut matches = observed.mcp_servers.iter().filter(|server| server.name == name);
    let Some(server) = matches.next() else { return false; };
    if matches.next().is_some() { return false; }
    match engine {
        crate::Engine::Claude => matches!(server.source.as_deref(), Some("user" | "project" | "local")),
        // Authentication is resolved afresh by the CLI, not applied to a thread.
        // Plugin names and changed/unknown runtime configurations are unsafe to
        // dispatch by bare name. Bearer/stdio servers do not supply native OAuth.
        crate::Engine::Codex => !observed.source_incomplete && server.plugin_id.is_none() && server.status != "unknown"
            && matches!(server.auth_status.as_deref(), Some("notLoggedIn" | "oAuth")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn codex_skill_metadata_is_observational_not_file_authority() {
        let mut p = Projection::default();
        skills(&[json!({"name":"fixture", "enabled":true, "path":"/native/plugin/skills/fixture/SKILL.md", "scope":"repo", "pluginId":"fixture@marketplace", "description":"secret-SENTINEL", "dependencies":{"secret":"secret-SENTINEL"}, "root":"secret-SENTINEL", "instance":{"id":"secret-SENTINEL"}})], true, &mut p);
        let skill = &p.skills[0];
        assert_eq!(skill.classification, NativeSkillClassification::Skill);
        assert_eq!(skill.path.as_deref(), Some("/native/plugin/skills/fixture/SKILL.md"));
        assert_eq!(skill.scope.as_deref(), Some("repo"));
        assert_eq!(skill.plugin_id.as_deref(), Some("fixture@marketplace"));
        assert_eq!(skill.enabled, Some(true));
        assert_eq!(skill.builtin, None);
        assert!(!p.incomplete);
        let encoded = serde_json::to_string(skill).unwrap();
        assert!(!encoded.contains("secret-SENTINEL"));
        assert!(!encoded.contains("instance") && !encoded.contains("root"));
        assert!(serde_json::from_value::<NativeSessionAction>(json!({"kind":"reload_skills", "path":skill.path})).is_err());
    }
    #[test]
    fn codex_source_scope_and_legacy_path_spelling_are_not_reinterpreted() {
        for scope in ["user", "repo", "system", "admin"] {
            for path in [r"C:\native\skills\fixture\SKILL.md", "../native/fixture/SKILL.md"] {
                let mut p = Projection::default();
                skills(&[json!({"name":"fixture", "path":path, "scope":scope, "enabled":false, "pluginId":null})], true, &mut p);
                assert_eq!(p.skills[0].path.as_deref(), Some(path));
                assert_eq!(p.skills[0].scope.as_deref(), Some(scope));
                assert_eq!(p.skills[0].plugin_id, None);
                assert!(!p.incomplete);
            }
        }
    }
    #[test]
    fn claude_only_explicit_builtin_marker_classifies_commands() {
        let mut p = Projection::default();
        skills(&[json!({"name":"help", "builtin":true}), json!({"name":"help"}), json!({"name":"mcp__fixture__tool", "builtin":false, "path":"/unproven", "scope":"repo", "pluginId":"unproven"}), json!({"name":"legacy-command"}), json!({"name":"skill-name"})], false, &mut p);
        assert_eq!(p.skills[0].classification, NativeSkillClassification::BuiltinCommand);
        assert_eq!(p.skills[0].builtin, Some(true));
        for command in &p.skills[1..] {
            assert_eq!(command.classification, NativeSkillClassification::Unknown);
            assert!(command.path.is_none() && command.scope.is_none() && command.plugin_id.is_none() && command.enabled.is_none());
        }
        assert_eq!(p.skills[1].builtin, None);
        assert_eq!(p.skills[2].builtin, Some(false));
        assert!(!p.incomplete, "source unknown is distinct from catalog truncation");
        assert!(p.source_incomplete);
    }
    #[test]
    fn claude_active_sources_preserve_metadata_without_authority_or_guesses() {
        let mut p = Projection::default();
        skills(&[
            json!({"name":"apps/web:deploy", "builtin":false, "aliases":["deploy", "deploy"], "active_source":{"source":"plugin", "plugin_name":"fixture", "plugin_id":"fixture@marketplace", "path":"/native/skills/deploy/SKILL.md", "root":"secret"}}),
            json!({"name":"source-only", "builtin":false, "active_source":{"source":"project", "plugin_name":"not-an-id"}}),
        ], false, &mut p);
        let skill = &p.skills[0];
        assert_eq!(skill.name, "apps/web:deploy");
        assert_eq!(skill.aliases, vec!["deploy"]);
        assert_eq!(skill.classification, NativeSkillClassification::Skill);
        assert_eq!(skill.source.as_deref(), Some("plugin"));
        assert_eq!(skill.path.as_deref(), Some("/native/skills/deploy/SKILL.md"));
        assert_eq!(skill.plugin_id.as_deref(), Some("fixture@marketplace"));
        assert_eq!(skill.plugin_name.as_deref(), Some("fixture"));
        assert!(skill.scope.is_none() && skill.enabled.is_none());
        assert!(p.skills[1].path.is_none() && p.skills[1].plugin_id.is_none());
        assert!(!p.incomplete && p.source_incomplete);
        assert!(safe_name("apps/web:deploy").is_none(), "command names never widen MCP or resource identifiers");
        assert!(!serde_json::to_string(skill).unwrap().contains("secret"));
        assert!(serde_json::from_value::<NativeSessionAction>(json!({"kind":"reload_skills", "path":skill.path})).is_err());
    }
    #[test]
    fn claude_sources_accept_omitted_builtin_but_reject_builtin_and_colliding_names() {
        let source = json!({"source":"project", "path":"/unproven", "plugin_id":"unproven"});
        let mut p = Projection::default();
        skills(&[
            json!({"name":"help", "builtin":true, "active_source":source}),
            json!({"name":"legacy", "active_source":source}),
            json!({"name":"duplicate", "builtin":false, "active_source":source}),
            json!({"name":"duplicate", "builtin":false, "active_source":source}),
            json!({"name":"alias-collision", "builtin":false, "active_source":source}),
            json!({"name":"other", "builtin":false, "aliases":["alias-collision"]}),
        ], false, &mut p);
        assert_eq!(p.skills[0].classification, NativeSkillClassification::BuiltinCommand);
        assert_eq!(p.skills[1].source.as_deref(), Some("project"));
        assert_eq!(p.skills[1].classification, NativeSkillClassification::Skill);
        for skill in p.skills.iter().enumerate().filter_map(|(i, skill)| (i != 1).then_some(skill)) {
            assert!(skill.source.is_none() && skill.path.is_none() && skill.plugin_id.is_none() && skill.plugin_name.is_none());
        }
        assert!(!p.incomplete && p.source_incomplete);
    }
    #[test]
    fn claude_source_errors_do_not_invalidate_command_dispatch() {
        let mut p = Projection::default();
        skills(&[json!({"name":"fixture", "builtin":false, "active_source":{"source":"project", "path":"/native/\nSKILL.md", "plugin_id":123}})], false, &mut p);
        assert_eq!(p.skills[0].classification, NativeSkillClassification::Skill);
        assert_eq!(p.skills[0].source.as_deref(), Some("project"));
        assert!(p.skills[0].path.is_none() && p.skills[0].plugin_id.is_none());
        assert!(!p.incomplete && p.source_incomplete);
        let mut codex = Projection::default();
        skills(&[json!({"name":"fixture", "enabled":true, "path":"/native/SKILL.md", "scope":"user", "aliases":["ignored"]})], true, &mut codex);
        assert!(codex.skills[0].aliases.is_empty() && codex.skills[0].source.is_none());
        assert!(!codex.source_incomplete);
    }
    #[test]
    fn skill_source_limits_drop_invalid_metadata_and_preserve_incompleteness() {
        for field in ["path", "scope", "pluginId"] {
            let mut value = json!({"name":"fixture", "enabled":true, "path":"/native/SKILL.md", "scope":"user", "pluginId":null});
            value[field] = json!(if field == "scope" { "future-source".to_owned() } else { "x".repeat(2049) });
            let mut p = Projection::default();
            skills(&[value], true, &mut p);
            assert!(p.incomplete);
            assert_eq!(p.skills.len(), 1);
        }
        let mut p = Projection::default();
        skills(&[json!({"name":"fixture", "enabled":true, "path":"/native/\nSKILL.md", "scope":"admin"})], true, &mut p);
        assert!(p.incomplete && p.skills[0].path.is_none());
        let mut p = Projection::default();
        skills(&vec![json!({"name":"fixture", "builtin":true}); 257], false, &mut p);
        assert!(p.incomplete);
        assert_eq!(p.skills.len(), 256);
        let mut p = Projection::default();
        skills(&[json!({"name":"fixture", "builtin":"true"}), json!({"name":"/not-an-identifier"})], false, &mut p);
        assert!(p.incomplete);
        assert_eq!(p.skills.len(), 1);
        assert_eq!(p.skills[0].classification, NativeSkillClassification::Unknown);
    }
    #[test]
    fn projection_excludes_native_config_errors_and_payloads() {
        let mut p = Projection::default();
        mcp(&[json!({"name":"fixture", "status":"connected", "source":"user", "config":{"env":{"TOKEN":"secret"}}, "error":"secret", "tools":[{"name":"echo","description":"secret", "inputSchema":{"secret":true}}]})], false, &mut p);
        let encoded = serde_json::to_string(&p.mcp_servers).unwrap();
        assert!(!encoded.contains("secret")); assert!(!p.mcp_servers[0].schema_observed);
        assert_eq!(p.mcp_servers[0].tools, vec!["echo"]);
    }
    #[test]
    fn bounded_projection_preserves_unknown_and_incomplete() {
        let mut p = Projection::default();
        mcp(&vec![json!({"name":"fixture", "runtimeStatus":null, "tools":{}}); 257], true, &mut p);
        assert!(p.incomplete); assert_eq!(p.mcp_servers.len(), 256);
        assert_eq!(p.mcp_servers[0].status, "unknown");
        assert!(serde_json::from_value::<NativeSessionAction>(json!({"kind":"reload_skills", "force":true})).is_err());
    }
    #[test]
    fn unsafe_identifiers_and_large_tool_catalogs_never_escape() {
        let mut p = Projection::default();
        let tools = (0..65).map(|index| (format!("tool{index}"), serde_json::json!({"description":"secret"}))).collect::<serde_json::Map<String, Value>>();
        mcp(&[json!({"name":"fixture","runtimeStatus":"connected","tools":tools}), json!({"name":"https://credential@example.test","runtimeStatus":"connected"}), json!({"name":"/private/secret","runtimeStatus":"connected"})], true, &mut p);
        assert!(p.incomplete);
        assert_eq!(p.mcp_servers.len(), 1);
        assert_eq!(p.mcp_servers[0].tools.len(), 64);
        assert_eq!(p.mcp_servers[0].status, "connected");
        assert!(!serde_json::to_string(&p.mcp_servers).unwrap().contains("secret"));
    }
    #[test]
    fn catalog_shapes_and_missing_entries_remain_explicit() {
        let mut p = Projection::default();
        mcp(&[json!({"name":"fixture","runtimeStatus":"connected","tools":[{"name":"echo"},{"description":"secret"},null,{"name":"/secret"}]})], true, &mut p);
        assert_eq!(p.mcp_servers[0].tools, vec!["echo"]);
        assert!(p.incomplete);
        let mut p = Projection::default();
        mcp(&[json!({"name":"fixture","status":"connected"})], false, &mut p);
        assert!(p.incomplete, "missing connected catalog is not observed empty");
        for status in ["failed", "needs-auth", "pending", "disabled", "unknown"] {
            let mut p = Projection::default();
            mcp(&[json!({"name":"fixture","status":status,"tools":[{"name":"stale"}]})], false, &mut p);
            assert!(p.mcp_servers[0].tools.is_empty());
        }
        let mut p = Projection::default();
        mcp(&[json!({"name":"fixture","runtimeStatus":"connected","tools":{},"toolsError":{}})], true, &mut p);
        assert!(p.incomplete);
    }
    #[test]
    fn codex_oauth_admission_uses_native_inventory_not_guessed_config_source() {
        let server = json!({"name":"fixture", "runtimeStatus":"authenticationRequired", "authStatus":"notLoggedIn", "tools":{}, "toolsError":"authentication needed", "source":"user", "scope":"project"});
        let mut out = Projection::default();
        mcp(&[server.clone()], true, &mut out);
        assert!(!out.incomplete);
        assert!(out.mcp_servers[0].source.is_none() && out.mcp_servers[0].scope.is_none());
        assert!(auth_server_allowed(crate::Engine::Codex, &out, "fixture"));
        assert!(!auth_server_allowed(crate::Engine::Claude, &out, "fixture"));
        for patch in [json!({"pluginId":"fixture@marketplace"}), json!({"pluginId":12}), json!({"runtimeStatus":null}), json!({"authStatus":"bearerToken"}), json!({"authStatus":"unsupported"})] {
            let mut value = server.clone();
            value.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
            let mut out = Projection::default(); mcp(&[value], true, &mut out);
            assert!(!auth_server_allowed(crate::Engine::Codex, &out, "fixture"));
        }
        out.incomplete = true;
        assert!(!auth_server_allowed(crate::Engine::Codex, &out, "fixture"));
        out.incomplete = false; out.mcp_servers.push(out.mcp_servers[0].clone());
        assert!(!auth_server_allowed(crate::Engine::Codex, &out, "fixture"));
    }

    #[test]
    fn latest_call_evidence_preserves_native_status_without_arguments_or_outputs() {
        let mut out = Projection::default();
        mcp(&[json!({"name":"fixture", "runtimeStatus":"connected", "authStatus":"oAuth", "tools":{}})], true, &mut out);
        let event = |seq, kind: &str, status: &str, server: &str| crate::session::SessionEvent {
            seq, kind: kind.into(), text: Some("private-SENTINEL".into()), run: None,
            data: json!({"type":"mcpToolCall", "server":server, "tool":"echo", "status":status, "arguments":{"token":"private-SENTINEL"}, "error":{"message":"private-SENTINEL"}, "result":{"content":["private-SENTINEL"]}}),
        };
        mcp_call_evidence(&[event(1, "item/started", "inProgress", "fixture"), event(2, "mcpToolCall", "failed", "fixture"), event(3, "mcpToolCall", "completed", "another"), event(4, "untrusted", "completed", "fixture")], &mut out);
        let call = out.mcp_servers[0].last_call.as_ref().unwrap();
        assert_eq!((call.tool.as_str(), call.status.as_str(), call.event_seq), ("echo", "failed", 2));
        assert!(!serde_json::to_string(&out.mcp_servers).unwrap().contains("private-SENTINEL"));
        let mut no_call = Projection::default();
        mcp(&[json!({"name":"fixture", "runtimeStatus":"connected", "tools":{"echo":{}}})], true, &mut no_call);
        mcp_call_evidence(&[], &mut no_call);
        assert!(no_call.mcp_servers[0].last_call.is_none(), "configured/discovered does not imply called");
    }
}
