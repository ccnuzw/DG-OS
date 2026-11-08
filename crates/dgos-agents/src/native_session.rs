//! Bounded, credential-free projections of existing native control channels.
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeAuthGuidance {
    pub verified_version: String, pub reason: String,
    pub login_argv: Vec<String>, pub logout_argv: Vec<String>,
}
impl NativeAuthGuidance {
    pub(crate) fn installed(engine: crate::Engine) -> Self {
        let (command, version) = match engine { crate::Engine::Codex => ("codex", "0.167.0"), crate::Engine::Claude => ("claude", "2.1.319") };
        Self { verified_version: version.into(), reason: "missing_session_interaction_cancel_contract".into(),
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
    // Native-reported provenance only. No root, instance token or file authority is issued here.
    pub path: Option<String>, pub scope: Option<String>, pub plugin_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeMcpStatus {
    pub name: String, pub status: String, pub auth_status: Option<String>,
    pub source: Option<String>, pub scope: Option<String>, pub tools: Vec<String>,
    pub schema_observed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeSessionObservation {
    pub session_id: String, pub generation: String, pub engine: crate::Engine,
    pub observed_at: i64, pub domain: SessionDomain, pub capabilities: NativeSessionCapabilities,
    pub skills: Vec<NativeSkillStatus>, pub mcp_servers: Vec<NativeMcpStatus>, pub incomplete: bool,
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
}
pub(crate) fn safe_name(text: &str) -> Option<String> {
    // Names are identifiers, never arbitrary native descriptions, URLs, errors or paths.
    (!text.is_empty() && text.len() <= 128 && text.bytes().all(|c| c.is_ascii_alphanumeric() || b"_-.:@".contains(&c)))
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
    for value in values.iter().take(256) {
        if out.skills.len() >= 256 { out.incomplete = true; break; }
        let Some(name) = value.get("name").and_then(name) else { out.incomplete = true; continue; };
        let (enabled, builtin, path, scope, plugin_id, classification) = if codex {
            let path = observed_text(&value["path"], 2048, &mut out.incomplete);
            let scope = category(&value["scope"], &["user", "repo", "system", "admin"]);
            let plugin_id = observed_text(&value["pluginId"], 256, &mut out.incomplete);
            let enabled = value["enabled"].as_bool();
            out.incomplete |= path.is_none() || scope.is_none() || enabled.is_none();
            (enabled, None, path, scope, plugin_id, NativeSkillClassification::Skill)
        } else {
            // SDK 0.3.317 SlashCommand exposes builtin, not a source/path/owner.
            // Names (including built-in collisions and MCP prefixes) cannot prove provenance.
            let builtin = value["builtin"].as_bool();
            out.incomplete |= !value["builtin"].is_null() && builtin.is_none();
            (None, builtin, None, None, None, if builtin == Some(true) { NativeSkillClassification::BuiltinCommand } else { NativeSkillClassification::Unknown })
        };
        out.skills.push(NativeSkillStatus { name, enabled, classification, builtin, path, scope, plugin_id });
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
        if codex { out.incomplete |= value.get("toolsError").is_some_and(|error| !error.is_null()); }
        out.mcp_servers.push(NativeMcpStatus { name, status,
            auth_status: category(&value["authStatus"], &["unknown", "unsupported", "notLoggedIn", "bearerToken", "oAuth"]),
            source: category(&value["source"], &["sdk", "plugin", "user", "project", "local", "dynamic", "managed", "enterprise", "claudeai", "agent"]),
            scope: category(&value["scope"], &["user", "project", "local", "managed", "claudeai"]),
            tools, schema_observed: false });
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
}
