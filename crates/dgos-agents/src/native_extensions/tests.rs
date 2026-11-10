use super::*;
use mcp::{Change, Field, ServerPatch, Transport};
use std::{fs, io::{Seek, SeekFrom, Write}, os::unix::fs::{symlink, PermissionsExt}, path::PathBuf};

struct Fixture { root: PathBuf, roots: TrustedRoots }
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!("dgos-native-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        for child in ["home", "codex", "claude", "project", "journal"] { fs::create_dir(root.join(child)).unwrap(); fs::set_permissions(root.join(child), fs::Permissions::from_mode(0o700)).unwrap(); }
        let roots = TrustedRoots { os_home: root.join("home"), codex_home: root.join("codex"), claude_home: root.join("claude"), claude_user_file: root.join("home/.claude.json"), project: Some(root.join("project")), journal: root.join("journal") };
        Self { root, roots }
    }
    fn context(&self, principal: &str, lease: &str, access: Access) -> HostContext { HostContext::from_host(principal.into(), lease.into(), self.roots.clone(), access).unwrap() }
    fn api(&self) -> NativeExtensions { NativeExtensions::new(self.context("alice", "lease-a", Access::Manage)).unwrap() }
    fn config(&self) -> PathBuf { self.roots.codex_home.join("config.toml") }
    fn write(&self, text: &str) { fs::write(self.config(), text).unwrap(); fs::set_permissions(self.config(), fs::Permissions::from_mode(0o600)).unwrap(); }
    fn journal(&self, id: &OperationId) -> PathBuf { self.roots.journal.join(format!("{}.json", id.as_str())) }
    fn alter_journal(&self, id: &OperationId, key: &str, value: serde_json::Value) {
        let path = self.journal(id);
        let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        record[key] = value;
        fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    }
}
impl Drop for Fixture { fn drop(&mut self) { fs::remove_dir_all(&self.root).unwrap(); } }
fn add() -> Change { Change::Upsert { name: "fixture".into(), fields: ServerPatch { program: Field::Set("never-executed-fixture".into()), ..Default::default() } } }
fn enabled(value: bool) -> Change { Change::SetEnabled { name: "fixture".into(), enabled: value } }
const ORIGINAL: &str = "# untouched comment\nunknown = 'private-fixture-sentinel'\n[mcp_servers.fixture]\ncommand = 'never-executed-fixture'\nenabled = true # keep enabled comment\nopaque = { token = 'private-fixture-sentinel' }\n";


#[test]
fn missing_mcp_config_is_complete_and_can_be_created_transactionally() {
    let fixture = Fixture::new();
    let api = fixture.api();
    let discovery = api.discover_mcp(Target::Codex, Scope::OsUser).unwrap();
    assert!(discovery.instances.is_empty() && !discovery.incomplete);
    let preview = api.preview_mcp(Target::Codex, Scope::OsUser, add()).unwrap();
    assert!(!fixture.config().exists());
    assert_eq!(api.commit(&preview.receipt.operation_id).unwrap().state, State::Committed);
    assert!(fs::read_to_string(fixture.config()).unwrap().contains("never-executed-fixture"));
}
#[test]
fn isolated_config_smoke() {
    let fixture = Fixture::new(); fixture.write(ORIGINAL);
    let api = fixture.api();
    let before = api.discover_mcp(Target::Codex, Scope::OsUser).unwrap();
    assert_eq!(before.instances[0].enabled, Some(true));
    let preview = api.preview_mcp(Target::Codex, Scope::OsUser, enabled(false)).unwrap();
    assert_eq!(fs::read_to_string(fixture.config()).unwrap(), ORIGINAL);
    let receipt = api.commit(&preview.receipt.operation_id).unwrap();
    assert_eq!(receipt.state, State::Committed);
    assert!(receipt.pending_activation && receipt.recovery_material_retained);
    let output = fs::read_to_string(fixture.config()).unwrap();
    assert!(output.contains("enabled = false # keep enabled comment"));
    assert!(output.contains("# untouched comment") && output.contains("opaque = { token = 'private-fixture-sentinel' }"));
    let restored = NativeExtensions::new(fixture.context("alice", "lease-b", Access::Manage)).unwrap();
    assert_eq!(restored.inspect(&preview.receipt.operation_id).unwrap().state, State::Committed);
    assert_eq!(api.commit(&preview.receipt.operation_id).unwrap().state, State::Committed);
    let discovery = api.discover_mcp(Target::Codex, Scope::OsUser).unwrap();
    assert_eq!(discovery.instances[0].enabled, Some(false));
    let backup = api.recovery_material(&preview.receipt.operation_id).unwrap();
    assert_eq!(fs::read_to_string(backup.directory.join("payload")).unwrap(), ORIGINAL);
    for safe in [serde_json::to_string(&preview).unwrap(), serde_json::to_string(&receipt).unwrap(), serde_json::to_string(&discovery).unwrap(), fs::read_to_string(fixture.journal(&preview.receipt.operation_id)).unwrap()] { assert!(!safe.contains("private-fixture-sentinel")); }
}

#[test]
fn external_edits_and_racing_creates_are_preserved() {
    for existed in [true, false] {
        let fixture = Fixture::new(); if existed { fixture.write(ORIGINAL); }
        let api = fixture.api(); let preview = api.preview_mcp(Target::Codex, Scope::OsUser, add()).unwrap();
        let external = "external = 'editor-version'\n"; fixture.write(external);
        assert_eq!(api.commit(&preview.receipt.operation_id).unwrap().state, State::Conflict);
        assert_eq!(fs::read_to_string(fixture.config()).unwrap(), external);
    }
}

#[test]
fn no_replace_primitive_preserves_a_last_instant_creator() {
    let fixture = Fixture::new();
    let parent = super::paths::Directory::open(&fixture.roots.codex_home).unwrap();
    let stage = parent.child(std::ffi::OsStr::new("stage"), true).unwrap();
    stage.write_new("payload", b"new = true\n", 0o600).unwrap();
    fixture.write("external = true\n");
    assert!(parent.publish(&stage, "payload", "config.toml", false).is_err());
    assert_eq!(fs::read_to_string(fixture.config()).unwrap(), "external = true\n");
    assert_eq!(stage.read("payload").unwrap().unwrap().0, "new = true\n");
}

#[test]
fn duplicate_json_keys_are_rejected_in_nested_objects_and_arrays() {
    for text in [r#"{"mcpServers":{},"mcpServers":{}}"#, r#"{"unknown":[{"token":"a","token":"b"}]}"#, r#"{"unknown":1,"\u0075nknown":2}"#] {
        assert_eq!(config::json_read(text).unwrap_err(), Error::InvalidFormat);
    }
}

#[test]
fn json_unknown_fields_and_other_scopes_remain_byte_preserved() {
    let fixture = Fixture::new(); let api = fixture.api();
    let original = r#"{ "unknown" : {"token":"private-fixture-sentinel"}, "mcpServers":{"fixture":{"command":"never-executed-fixture","opaque":1}}, "projects":{"other":{"mcpServers":{"keep":{"command":"keep"}}}} }"#;
    fs::write(&fixture.roots.claude_user_file, original).unwrap();
    let change = Change::Upsert { name: "fixture".into(), fields: ServerPatch { program: Field::Set("new-program".into()), ..Default::default() } };
    let p = api.preview_mcp(Target::Claude, Scope::OsUser, change).unwrap();
    assert_eq!(api.commit(&p.receipt.operation_id).unwrap().state, State::Committed);
    assert_eq!(fs::read_to_string(&fixture.roots.claude_user_file).unwrap(), original.replacen("never-executed-fixture", "new-program", 1));
    assert!(!serde_json::to_string(&api.discover_mcp(Target::Claude, Scope::OsUser).unwrap()).unwrap().contains("private-fixture-sentinel"));
}

#[test]
fn symlink_files_and_replaced_roots_cannot_be_committed() {
    let fixture = Fixture::new(); fixture.write(ORIGINAL); let api = fixture.api();
    let p = api.preview_mcp(Target::Codex, Scope::OsUser, enabled(false)).unwrap();
    let external = fixture.root.join("external"); fs::write(&external, "external bytes").unwrap();
    fs::remove_file(fixture.config()).unwrap(); symlink(&external, fixture.config()).unwrap();
    assert!(api.commit(&p.receipt.operation_id).is_err());
    assert_eq!(fs::read_to_string(&external).unwrap(), "external bytes");
    fs::rename(&fixture.roots.codex_home, fixture.root.join("old-codex")).unwrap();
    fs::create_dir(&fixture.roots.codex_home).unwrap();
    assert_eq!(api.discover_mcp(Target::Codex, Scope::OsUser).unwrap_err(), Error::RootChanged);
}

#[test]
fn symlink_project_directory_is_never_followed() {
    let fixture = Fixture::new(); let api = fixture.api();
    symlink(&fixture.roots.codex_home, fixture.roots.project.as_ref().unwrap().join(".codex")).unwrap();
    assert!(api.preview_mcp(Target::Codex, Scope::ProjectShared, add()).is_err());
    assert!(!fixture.config().exists());
}

#[test]
fn read_only_revoked_wrong_account_and_wrong_lease_are_rejected() {
    let fixture = Fixture::new(); fixture.write(ORIGINAL);
    let read = NativeExtensions::new(fixture.context("alice", "lease-a", Access::Read)).unwrap();
    assert_eq!(read.preview_mcp(Target::Codex, Scope::OsUser, enabled(false)).unwrap_err(), Error::AccessDenied);
    let context = fixture.context("alice", "lease-a", Access::Manage);
    let api = NativeExtensions::new(context.clone()).unwrap();
    let p = api.preview_mcp(Target::Codex, Scope::OsUser, enabled(false)).unwrap();
    assert_eq!(read.inspect(&p.receipt.operation_id).unwrap_err(), Error::AccessDenied);
    let bob = NativeExtensions::new(fixture.context("bob", "lease-a", Access::Manage)).unwrap();
    let reauthorized = NativeExtensions::new(fixture.context("alice", "lease-b", Access::Manage)).unwrap();
    assert_eq!(reauthorized.inspect(&p.receipt.operation_id).unwrap().state, State::Previewed);
    assert_eq!(bob.inspect(&p.receipt.operation_id).unwrap_err(), Error::NotFound);
    let other_lease = NativeExtensions::new(fixture.context("alice", "lease-b", Access::Manage)).unwrap();
    assert_eq!(other_lease.commit(&p.receipt.operation_id).unwrap_err(), Error::AccessDenied);
    context.revoke().unwrap();
    assert_eq!(api.commit(&p.receipt.operation_id).unwrap_err(), Error::AccessDenied);
    assert_eq!(fs::read_to_string(fixture.config()).unwrap(), ORIGINAL);
}

#[test]
fn writable_by_others_expired_and_cancelled_previews_never_publish() {
    let fixture = Fixture::new(); fixture.write(ORIGINAL); let api = fixture.api();
    fs::set_permissions(fixture.config(), fs::Permissions::from_mode(0o666)).unwrap();
    assert_eq!(api.preview_mcp(Target::Codex, Scope::OsUser, enabled(false)).unwrap_err(), Error::AccessDenied);
    fs::set_permissions(fixture.config(), fs::Permissions::from_mode(0o600)).unwrap();
    let p = api.preview_mcp(Target::Codex, Scope::OsUser, enabled(false)).unwrap();
    fixture.alter_journal(&p.receipt.operation_id, "expires_at", serde_json::json!(0));
    assert_eq!(api.commit(&p.receipt.operation_id).unwrap_err(), Error::Expired);
    let p = api.preview_mcp(Target::Codex, Scope::OsUser, enabled(false)).unwrap();
    assert_eq!(api.cancel(&p.receipt.operation_id).unwrap().state, State::Cancelled);
    assert_eq!(api.commit(&p.receipt.operation_id).unwrap().state, State::Cancelled);
    assert_eq!(fs::read_to_string(fixture.config()).unwrap(), ORIGINAL);
}

#[test]
fn crashed_exchange_is_inspected_without_republishing_and_retains_old_fd_writes() {
    let fixture = Fixture::new(); fixture.write(ORIGINAL); let api = fixture.api();
    let mut old_fd = fs::OpenOptions::new().write(true).open(fixture.config()).unwrap();
    let p = api.preview_mcp(Target::Codex, Scope::OsUser, enabled(false)).unwrap();
    let backup = api.recovery_material(&p.receipt.operation_id).unwrap();
    let parent = super::paths::Directory::open(&fixture.roots.codex_home).unwrap();
    let stage = super::paths::Directory::open(&backup.directory).unwrap();
    fixture.alter_journal(&p.receipt.operation_id, "state", serde_json::json!("Publishing"));
    parent.publish(&stage, "payload", "config.toml", true).unwrap();
    let restarted = fixture.api();
    assert_eq!(restarted.inspect(&p.receipt.operation_id).unwrap().state, State::Committed);
    old_fd.seek(SeekFrom::End(0)).unwrap(); old_fd.write_all(b"# late external old-fd write\n").unwrap(); old_fd.sync_all().unwrap();
    assert!(fs::read_to_string(backup.directory.join("payload")).unwrap().ends_with("# late external old-fd write\n"));
    assert!(!fs::read_to_string(fixture.config()).unwrap().contains("late external"));
    assert_eq!(restarted.commit(&p.receipt.operation_id).unwrap().state, State::Committed);
}

#[test]
fn displaced_external_version_requires_recovery_without_swap_back() {
    let fixture = Fixture::new(); fixture.write(ORIGINAL); let api = fixture.api();
    let p = api.preview_mcp(Target::Codex, Scope::OsUser, enabled(false)).unwrap();
    let backup = api.recovery_material(&p.receipt.operation_id).unwrap();
    fixture.write("external = 'last-instant-edit'\n");
    let parent = super::paths::Directory::open(&fixture.roots.codex_home).unwrap();
    let stage = super::paths::Directory::open(&backup.directory).unwrap();
    fixture.alter_journal(&p.receipt.operation_id, "state", serde_json::json!("Publishing"));
    parent.publish(&stage, "payload", "config.toml", true).unwrap();
    assert_eq!(fixture.api().inspect(&p.receipt.operation_id).unwrap().state, State::RecoveryRequired);
    assert_eq!(fs::read_to_string(backup.directory.join("payload")).unwrap(), "external = 'last-instant-edit'\n");
    assert!(fs::read_to_string(fixture.config()).unwrap().contains("enabled = false"));
}

#[test]
fn process_lock_blocks_an_independent_descriptor() {
    let fixture = Fixture::new(); let api = fixture.api();
    let dir = super::paths::Directory::open(&fixture.roots.codex_home).unwrap(); let _held = dir.lock().unwrap();
    assert_eq!(api.preview_mcp(Target::Codex, Scope::OsUser, add()).unwrap_err(), Error::Busy);
    assert!(!fixture.config().exists());
}

#[test]
fn claude_name_rules_support_four_states_inheritance_and_config_cas() {
    use skills::{RuleState, VisibilityChange};
    let fixture = Fixture::new(); let api = fixture.api();
    fs::write(&fixture.roots.claude_user_file, "{\"mcpServers\":{}}").unwrap();
    let path = fixture.roots.project.as_ref().unwrap().join(".claude/settings.local.json");
    assert!(api.discover_skill_rules(Target::Claude, Scope::ProjectLocal).unwrap().rules.is_empty());
    for state in [RuleState::On, RuleState::NameOnly, RuleState::UserInvocableOnly, RuleState::Off] {
        let preview = api.preview_skill_visibility(Target::Claude, Scope::ProjectLocal, VisibilityChange::ClaudeName { name: "absent:fixture".into(), state }).unwrap();
        let evidence = preview.rule.as_ref().unwrap();
        assert_eq!(evidence.evidence_version, 1);
        assert!(evidence.current_and_future && evidence.plugins_excluded && evidence.config_change_hook_possible);
        assert_eq!(api.commit(&preview.receipt.operation_id).unwrap().state, State::Committed);
        let rules = api.discover_skill_rules(Target::Claude, Scope::ProjectLocal).unwrap();
        assert_eq!(rules.rules[0].name, "absent:fixture");
        assert_eq!(rules.rules[0].state, state);
    }
    let change = || VisibilityChange::ClaudeName { name: "absent:fixture".into(), state: RuleState::Inherit };
    let cancelled = api.preview_skill_visibility(Target::Claude, Scope::ProjectLocal, change()).unwrap();
    assert_eq!(cancelled.rule.as_ref().unwrap().previous, Some(RuleState::Off));
    assert_eq!(api.cancel(&cancelled.receipt.operation_id).unwrap().state, State::Cancelled);
    assert_eq!(api.discover_skill_rules(Target::Claude, Scope::ProjectLocal).unwrap().rules[0].state, RuleState::Off);
    let conflict = api.preview_skill_visibility(Target::Claude, Scope::ProjectLocal, change()).unwrap();
    fs::write(&path, "{\"skillOverrides\":{\"absent:fixture\":\"on\"},\"external\":true}").unwrap();
    assert_eq!(api.commit(&conflict.receipt.operation_id).unwrap().state, State::Conflict);
    let inherit = api.preview_skill_visibility(Target::Claude, Scope::ProjectLocal, change()).unwrap();
    assert_eq!(api.commit(&inherit.receipt.operation_id).unwrap().state, State::Committed);
    assert!(api.discover_skill_rules(Target::Claude, Scope::ProjectLocal).unwrap().rules.is_empty());
    assert!(fs::read_to_string(path).unwrap().contains("\"external\":true"));
    assert_eq!(fs::read_to_string(&fixture.roots.claude_user_file).unwrap(), "{\"mcpServers\":{}}");
    let old = api.preview_skill_visibility(Target::Claude, Scope::OsUser, change()).unwrap();
    fixture.alter_journal(&old.receipt.operation_id, "rule", serde_json::Value::Null);
    assert_eq!(api.commit(&old.receipt.operation_id).unwrap_err(), Error::Unsupported);
    let unknown = api.preview_skill_visibility(Target::Claude, Scope::OsUser, change()).unwrap();
    let mut evidence = serde_json::to_value(unknown.rule.unwrap()).unwrap(); evidence["evidence_version"] = serde_json::json!(2);
    fixture.alter_journal(&unknown.receipt.operation_id, "rule", evidence);
    assert_eq!(api.commit(&unknown.receipt.operation_id).unwrap_err(), Error::Unsupported);
}

#[test]
fn claude_rule_editor_preserves_unrelated_bytes_and_rejects_ambiguous_settings() {
    use skills::{RuleState, VisibilityChange};
    let change = |state| VisibilityChange::ClaudeName { name: "fixture".into(), state };
    let original = "{\n  \"unknown\" : [ 1, 2 ], \"skillOverrides\": { \"other\":\"name-only\", \"fixture\" : \"off\" }, \"secret\":\"untouched\"\n}\n";
    let output = skills::edit_claude_visibility(original, &change(RuleState::On)).unwrap();
    assert_eq!(output, original.replacen("\"off\"", "\"on\"", 1));
    let removed = skills::edit_claude_visibility(original, &change(RuleState::Inherit)).unwrap();
    assert!(removed.contains("\"unknown\" : [ 1, 2 ]") && removed.contains("\"secret\":\"untouched\""));
    assert_eq!(skills::read_rules(&removed).unwrap()[0].name, "other");
    for invalid in [r#"{"skillOverrides":false}"#, r#"{"skillOverrides":{"x":true}}"#, r#"{"skillOverrides":{"x":"inherit"}}"#, r#"{"skillOverrides":{"x":"bad"}}"#, r#"{"skillOverrides":{"x":"on","x":"off"}}"#, r#"{"unknown":1,"unknown":2}"#] {
        assert_eq!(skills::edit_claude_visibility(invalid, &change(RuleState::On)).unwrap_err(), Error::InvalidFormat);
    }
    assert_eq!(skills::edit_claude_visibility("{}", &VisibilityChange::ClaudeName { name: " fixture".into(), state: RuleState::On }).unwrap_err(), Error::InvalidFormat);
}

#[test]
fn claude_rule_layers_are_separate_from_mcp_and_resource_roots() {
    let fixture = Fixture::new(); let api = fixture.api();
    let project = fixture.roots.project.as_ref().unwrap();
    for (scope, state, path) in [
        (Scope::OsUser, skills::RuleState::On, fixture.roots.claude_home.join("settings.json")),
        (Scope::ProjectShared, skills::RuleState::NameOnly, project.join(".claude/settings.json")),
        (Scope::ProjectLocal, skills::RuleState::Off, project.join(".claude/settings.local.json")),
    ] {
        let preview = api.preview_skill_visibility(Target::Claude, scope.clone(), skills::VisibilityChange::ClaudeName { name: "same".into(), state }).unwrap();
        assert_eq!(preview.rule.as_ref().unwrap().scope, scope);
        assert_eq!(preview.rule.as_ref().unwrap().previous, None);
        assert_eq!(api.commit(&preview.receipt.operation_id).unwrap().state, State::Committed);
        assert_eq!(skills::read_rules(&fs::read_to_string(path).unwrap()).unwrap()[0].state, state);
    }
    assert!(!fixture.roots.claude_user_file.exists());
    assert_eq!(api.discover_resources(Target::Claude, Scope::ProjectLocal, "").unwrap_err(), Error::Unsupported);
    assert_eq!(api.preview_skill_visibility(Target::Codex, Scope::OsUser, skills::VisibilityChange::ClaudeName { name: "same".into(), state: skills::RuleState::Off }).unwrap_err(), Error::InvalidFormat);
    assert_eq!(api.preview_skill_visibility(Target::Claude, Scope::OsUser, skills::VisibilityChange::CodexPath { selector: "/same/SKILL.md".into(), visible: false }).unwrap_err(), Error::InvalidFormat);
}

#[test]
fn discovery_marks_symlink_and_script_without_executing_them() {
    let fixture = Fixture::new(); let api = fixture.api();
    let root = fixture.roots.claude_home.join("skills"); fs::create_dir(&root).unwrap();
    let loose = root.join("fixture"); fs::create_dir(&loose).unwrap();
    fs::write(loose.join("SKILL.md"), "---\nname: fixture\n---\nprivate-fixture-body").unwrap();
    fs::create_dir(loose.join("scripts")).unwrap();
    symlink(&loose, root.join("linked")).unwrap();
    let found = api.discover_skills(Target::Claude, Scope::OsUser).unwrap();
    assert_eq!(found.instances[0].owner, Owner::Loose);
    assert!(found.instances[0].readable && found.instances[0].has_scripts);
    assert_eq!(found.instances[1].owner, Owner::Symlink);
    assert!(!found.instances[1].readable);
    assert!(!serde_json::to_string(&found).unwrap().contains("private-fixture-body"));
}

#[test]
fn invalid_transport_edits_and_empty_json_files_are_rejected() {
    let fixture = Fixture::new(); let api = fixture.api();
    let bad = Change::Upsert { name: "fixture".into(), fields: ServerPatch { transport: Field::Set(Transport::Http), ..Default::default() } };
    assert_eq!(api.preview_mcp(Target::Claude, Scope::OsUser, bad).unwrap_err(), Error::InvalidFormat);
    fs::write(&fixture.roots.claude_user_file, "").unwrap();
    assert_eq!(api.preview_mcp(Target::Claude, Scope::OsUser, add()).unwrap_err(), Error::InvalidFormat);
}

#[test]
fn environment_header_edit_preserves_existing_secret_headers() {
    let fixture = Fixture::new(); let api = fixture.api();
    let original = r#"{"mcpServers":{"fixture":{"type":"http","url":"https://example.invalid/mcp","headers":{"Authorization":"private-fixture-sentinel","X-Existing":"unchanged"}}}}"#;
    fs::write(&fixture.roots.claude_user_file, original).unwrap();
    let patch = Change::Upsert { name: "fixture".into(), fields: ServerPatch { header_env: Field::Set(vec![("X-New".into(), "FIXTURE_HEADER".into())]), ..Default::default() } };
    let p = api.preview_mcp(Target::Claude, Scope::OsUser, patch).unwrap();
    assert_eq!(api.commit(&p.receipt.operation_id).unwrap().state, State::Committed);
    let output = fs::read_to_string(&fixture.roots.claude_user_file).unwrap();
    assert!(output.contains(r#""Authorization":"private-fixture-sentinel","X-Existing":"unchanged""#));
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(parsed["mcpServers"]["fixture"]["headers"]["X-New"], "${FIXTURE_HEADER}");
}

#[test]
fn codex_visibility_changes_only_the_exact_skill_entry() {
    let original = "unknown = 'keep'\n[[skills.config]]\npath = '/skills/first/SKILL.md'\nenabled = true # first\nopaque = 'keep-first'\n[[skills.config]]\npath = '/skills/second/SKILL.md'\nenabled = true # second\n";
    let output = skills::edit_codex_visibility(original, &skills::VisibilityChange::CodexPath { selector: "/skills/first/SKILL.md".into(), visible: false }).unwrap();
    assert_eq!(output, original.replacen("enabled = true # first", "enabled = false # first", 1));
    let duplicate = format!("{original}[[skills.config]]\npath = '/skills/first/SKILL.md'\nenabled = true\n");
    assert_eq!(skills::edit_codex_visibility(&duplicate, &skills::VisibilityChange::CodexPath { selector: "/skills/first/SKILL.md".into(), visible: false }).unwrap_err(), Error::Conflict);
}

#[test]
fn claude_project_local_edits_preserve_user_and_other_project_servers() {
    let fixture = Fixture::new(); let api = fixture.api();
    let project = fixture.roots.project.as_ref().unwrap().to_str().unwrap();
    let original = serde_json::json!({"mcpServers":{"user":{"command":"user-program"}}, "projects":{project:{"mcpServers":{"fixture":{"command":"local-program"}}}, "other":{"mcpServers":{"other":{"command":"other-program"}}}}});
    fs::write(&fixture.roots.claude_user_file, serde_json::to_vec(&original).unwrap()).unwrap();
    let p = api.preview_mcp(Target::Claude, Scope::ProjectLocal, Change::Remove { name: "fixture".into() }).unwrap();
    assert_eq!(api.commit(&p.receipt.operation_id).unwrap().state, State::Committed);
    let output: serde_json::Value = serde_json::from_slice(&fs::read(&fixture.roots.claude_user_file).unwrap()).unwrap();
    assert_eq!(output["mcpServers"], original["mcpServers"]);
    assert_eq!(output["projects"]["other"], original["projects"]["other"]);
    assert_eq!(output["projects"][project]["mcpServers"], serde_json::json!({}));
}

#[test]
fn claude_project_local_operations_cannot_cross_authorized_projects() {
    for state in [State::Previewed, State::Prepared, State::Publishing, State::Unknown, State::Committed] {
        let fixture = Fixture::new();
        let api = fixture.api();
        let original = b"{\"mcpServers\":{}}";
        fs::write(&fixture.roots.claude_user_file, original).unwrap();
        let preview = api.preview_mcp(Target::Claude, Scope::ProjectLocal, add()).unwrap();
        let id = &preview.receipt.operation_id;
        if state == State::Committed { api.commit(id).unwrap(); }
        else { fixture.alter_journal(id, "state", serde_json::to_value(state).unwrap()); }
        let before = fs::read(&fixture.roots.claude_user_file).unwrap();
        let journal = fs::read(fixture.journal(id)).unwrap();
        let project_b = fixture.root.join("project-b"); fs::create_dir(&project_b).unwrap();
        let mut context = fixture.context("alice", "lease-a", Access::Manage);
        context.roots.project = Some(project_b);
        let other = NativeExtensions::new(context).unwrap();
        assert_eq!(other.commit(id).unwrap_err(), Error::AccessDenied);
        assert_eq!(other.inspect(id).unwrap_err(), Error::AccessDenied);
        assert!(matches!(other.recovery_material(id), Err(Error::AccessDenied)));
        assert_eq!(other.cancel(id).unwrap_err(), Error::AccessDenied);
        assert_eq!(fs::read(&fixture.roots.claude_user_file).unwrap(), before);
        assert_eq!(fs::read(fixture.journal(id)).unwrap(), journal);
        // The original authorized project can still resolve its own operation.
        assert_eq!(api.inspect(id).unwrap().state, if matches!(state, State::Publishing | State::Unknown) { State::Conflict } else { state });
    }
}

#[test]
fn project_local_recovery_rejects_replaced_root_and_unbound_legacy_journal() {
    let fixture = Fixture::new(); let api = fixture.api();
    fs::write(&fixture.roots.claude_user_file, b"{}").unwrap();
    let preview = api.preview_mcp(Target::Claude, Scope::ProjectLocal, add()).unwrap();
    let id = &preview.receipt.operation_id;
    let project = fixture.roots.project.as_ref().unwrap();
    fs::rename(project, fixture.root.join("old-project")).unwrap(); fs::create_dir(project).unwrap();
    let replacement = fixture.api();
    assert_eq!(replacement.commit(id).unwrap_err(), Error::RootChanged);
    assert_eq!(replacement.inspect(id).unwrap_err(), Error::RootChanged);
    assert!(matches!(replacement.recovery_material(id), Err(Error::RootChanged)));
    assert_eq!(fs::read(&fixture.roots.claude_user_file).unwrap(), b"{}");
    let path = fixture.journal(id);
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record.as_object_mut().unwrap().remove("project");
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert_eq!(replacement.commit(id).unwrap_err(), Error::AccessDenied);
    assert_eq!(replacement.inspect(id).unwrap_err(), Error::AccessDenied);
    assert!(matches!(replacement.recovery_material(id), Err(Error::AccessDenied)));
}

#[test]
fn managed_and_unknown_transport_preimages_cannot_be_removed_or_rewritten() {
    for target in [Target::Codex, Target::Claude] {
        for (marker, expected) in [("plugin", Owner::Plugin), ("managed", Owner::Managed), ("synced", Owner::Synced), ("unrecognized", Owner::Unknown)] {
            let fixture = Fixture::new(); let api = fixture.api();
            let original = if target == Target::Codex {
                format!("[mcp_servers.fixture]\ncommand = 'never-run'\nowner = '{marker}'\n")
            } else { format!(r#"{{"mcpServers":{{"fixture":{{"command":"never-run","owner":"{marker}"}}}}}}"#) };
            let path = if target == Target::Codex { fixture.config() } else { fixture.roots.claude_user_file.clone() };
            fs::write(&path, &original).unwrap();
            assert_eq!(api.discover_mcp(target, Scope::OsUser).unwrap().instances[0].owner, expected);
            for change in [add(), Change::Remove { name: "fixture".into() }, enabled(false)] {
                assert_eq!(api.preview_mcp(target, Scope::OsUser, change).unwrap_err(), Error::Unsupported);
                assert_eq!(fs::read_to_string(&path).unwrap(), original);
            }
        }
        let fixture = Fixture::new(); let api = fixture.api();
        let original = if target == Target::Codex { "[mcp_servers.fixture]\ntype = 'future'\ncommand = 'never-run'\n" } else { r#"{"mcpServers":{"fixture":{"type":"future","command":"never-run"}}}"# };
        let path = if target == Target::Codex { fixture.config() } else { fixture.roots.claude_user_file.clone() };
        fs::write(&path, original).unwrap();
        assert_eq!(api.discover_mcp(target, Scope::OsUser).unwrap().instances[0].transport, Transport::Unknown);
        for change in [add(), Change::Remove { name: "fixture".into() }] {
            assert_eq!(api.preview_mcp(target, Scope::OsUser, change).unwrap_err(), Error::Unsupported);
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        }
    }
}

#[test]
fn all_native_scopes_preserve_same_named_neighbors_and_private_values() {
    for (target, scope) in [(Target::Codex, Scope::OsUser), (Target::Codex, Scope::ProjectShared), (Target::Claude, Scope::OsUser), (Target::Claude, Scope::ProjectLocal), (Target::Claude, Scope::ProjectShared)] {
        let fixture = Fixture::new(); let api = fixture.api();
        let path = fixture.context("alice", "lease-a", Access::Manage).config_path(target, &scope).unwrap();
        if !path.parent().unwrap().exists() { fs::create_dir(path.parent().unwrap()).unwrap(); }
        let original = if target == Target::Codex {
            "# retain\n[mcp_servers.fixture]\nurl = 'https://example.invalid/mcp'\nhttp_headers = { Authorization = 'fixture-private-value' }\nopaque = 'keep'\n[mcp_servers.other]\ncommand = 'same-name-neighbor'\n".to_owned()
        } else {
            let server = serde_json::json!({"type":"http", "url":"https://example.invalid/mcp", "headers":{"Authorization":"fixture-private-value"}, "opaque":"keep"});
            let project = fixture.roots.project.as_ref().unwrap().to_str().unwrap();
            let mut document = serde_json::json!({"mcpServers":{"fixture":server.clone()}, "projects":{project:{"mcpServers":{"fixture":server.clone()}}, "other":{"mcpServers":{"fixture":server}}}});
            let selected = if scope == Scope::ProjectLocal { &mut document["projects"][project]["mcpServers"]["fixture"] } else { &mut document["mcpServers"]["fixture"] };
            selected["url"] = serde_json::json!("https://selected.invalid/mcp");
            serde_json::to_string(&document).unwrap()
        };
        fs::write(&path, &original).unwrap();
        let change = Change::Upsert { name: "fixture".into(), fields: ServerPatch { url: Field::Set("https://updated.invalid/mcp".into()), ..Default::default() } };
        let preview = api.preview_mcp(target, scope.clone(), change).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        let receipt = api.commit(&preview.receipt.operation_id).unwrap();
        assert_eq!(receipt.state, State::Committed);
        let output = fs::read_to_string(&path).unwrap();
        let expected = original.replacen(if target == Target::Codex { "https://example.invalid/mcp" } else { "https://selected.invalid/mcp" }, "https://updated.invalid/mcp", 1);
        if target == Target::Claude { assert_eq!(output, expected); }
        else {
            assert!(output.contains("http_headers = { Authorization = 'fixture-private-value' }"));
            assert!(output.contains("opaque = 'keep'\n[mcp_servers.other]\ncommand = 'same-name-neighbor'"));
            let doc = output.parse::<toml_edit::DocumentMut>().unwrap();
            assert_eq!(doc["mcp_servers"]["fixture"]["url"].as_str(), Some("https://updated.invalid/mcp"));
        }
        let discovery = api.discover_mcp(target, scope).unwrap();
        let instance = discovery.instances.iter().find(|v| v.native_key == "fixture").unwrap();
        assert!(instance.authentication.configured);
        assert_eq!(instance.authentication.credential_present, None);
        assert_eq!(instance.authentication.authenticated, None);
        assert_eq!(instance.authentication.connected, None);
        assert_eq!(instance.authentication.approved, None);
        assert_eq!(instance.authentication.active_session, None);
        for safe in [serde_json::to_string(&preview).unwrap(), serde_json::to_string(&receipt).unwrap(), serde_json::to_string(&discovery).unwrap(), fs::read_to_string(fixture.journal(&preview.receipt.operation_id)).unwrap()] {
            assert!(!safe.contains("fixture-private-value"));
        }
    }
}

#[test]
fn unsafe_mcp_inputs_never_reach_native_files_or_journals() {
    let bad_fields = vec![
        ServerPatch { url: Field::Set("https://user:password@example.invalid/mcp".into()), ..Default::default() },
        ServerPatch { url: Field::Set("https://example.invalid/mcp?token=fixture-private-value".into()), ..Default::default() },
        ServerPatch { url: Field::Set("https://example.invalid/mcp#fragment".into()), ..Default::default() },
        ServerPatch { url: Field::Set("https:///missing-host".into()), ..Default::default() },
        ServerPatch { url: Field::Set("http://localhost.evil.invalid:80/mcp".into()), ..Default::default() },
        ServerPatch { program: Field::Set("sh -c never-run".into()), ..Default::default() },
        ServerPatch { args: Field::Set(vec!["--token".into(), "fixture-private-value".into()]), ..Default::default() },
        ServerPatch { args: Field::Set(vec!["--password=fixture-private-value".into()]), ..Default::default() },
        ServerPatch { args: Field::Set(vec!["Bearer fixture-private-value".into()]), ..Default::default() },
        ServerPatch { program: Field::Set("****".into()), ..Default::default() },
        ServerPatch { header_env: Field::Set(vec![("X-Test".into(), "****".into())]), ..Default::default() },
        ServerPatch { header_env: Field::Set(vec![("X-Test".into(), "GOOD_ENV".into()), ("x-test".into(), "OTHER_ENV".into())]), ..Default::default() },
        ServerPatch { header_env: Field::Set(vec![("X-Test\r\nAuthorization".into(), "GOOD_ENV".into())]), ..Default::default() },
    ];
    for target in [Target::Codex, Target::Claude] {
        let fixture = Fixture::new(); let api = fixture.api();
        for fields in bad_fields.iter().cloned() {
            let error = api.preview_mcp(target, Scope::OsUser, Change::Upsert { name: "fixture".into(), fields }).unwrap_err();
            assert!(!error.to_string().contains("fixture-private-value"));
        }
        assert!(!fixture.config().exists());
        assert!(!fixture.roots.claude_user_file.exists());
        assert_eq!(fs::read_dir(&fixture.roots.journal).unwrap().count(), 0);
    }
}

#[test]
fn codex_environment_references_round_trip_and_remove_without_static_header_loss() {
    let fixture = Fixture::new(); let api = fixture.api();
    let original = "[mcp_servers.fixture]\nurl = 'https://example.invalid/mcp'\nhttp_headers = { Authorization = 'fixture-private-value' }\n";
    fixture.write(original);
    let fields = ServerPatch { bearer_env: Field::Set("FIXTURE_TOKEN".into()), header_env: Field::Set(vec![("X-Client".into(), "FIXTURE_CLIENT".into())]), ..Default::default() };
    let p = api.preview_mcp(Target::Codex, Scope::OsUser, Change::Upsert { name: "fixture".into(), fields }).unwrap();
    assert_eq!(api.commit(&p.receipt.operation_id).unwrap().state, State::Committed);
    let text = fs::read_to_string(fixture.config()).unwrap(); let doc = text.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(doc["mcp_servers"]["fixture"]["bearer_token_env_var"].as_str(), Some("FIXTURE_TOKEN"));
    assert_eq!(doc["mcp_servers"]["fixture"]["env_http_headers"]["X-Client"].as_str(), Some("FIXTURE_CLIENT"));
    let fields = ServerPatch { bearer_env: Field::Remove, header_env: Field::Remove, ..Default::default() };
    let p = api.preview_mcp(Target::Codex, Scope::OsUser, Change::Upsert { name: "fixture".into(), fields }).unwrap();
    assert_eq!(api.commit(&p.receipt.operation_id).unwrap().state, State::Committed);
    let text = fs::read_to_string(fixture.config()).unwrap(); let doc = text.parse::<toml_edit::DocumentMut>().unwrap();
    assert!(doc["mcp_servers"]["fixture"].get("bearer_token_env_var").is_none());
    assert!(doc["mcp_servers"]["fixture"].get("env_http_headers").is_none());
    assert!(text.contains("http_headers = { Authorization = 'fixture-private-value' }"));
}

#[test]
fn malformed_configs_and_header_case_collisions_are_not_repaired_by_edits() {
    for (target, original) in [
        (Target::Codex, "[mcp_servers.fixture]\ncommand='one'\ncommand='two'\n"),
        (Target::Codex, "[mcp_servers.fixture\ncommand='one'\n"),
        (Target::Claude, r#"{"mcpServers":{"fixture":{"command":"one","command":"two"}}}"#),
        (Target::Claude, r#"{"mcpServers":[]}"#),
    ] {
        let fixture = Fixture::new(); let api = fixture.api();
        let path = if target == Target::Codex { fixture.config() } else { fixture.roots.claude_user_file.clone() };
        fs::write(&path, original).unwrap();
        assert_eq!(api.preview_mcp(target, Scope::OsUser, add()).unwrap_err(), Error::InvalidFormat);
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }
    let fixture = Fixture::new(); let api = fixture.api();
    let original = r#"{"mcpServers":{"fixture":{"type":"http","url":"https://example.invalid/mcp","headers":{"X-Existing":"fixture-private-value"}}}}"#;
    fs::write(&fixture.roots.claude_user_file, original).unwrap();
    let fields = ServerPatch { header_env: Field::Set(vec![("x-existing".into(), "NEW_ENV".into())]), ..Default::default() };
    assert_eq!(api.preview_mcp(Target::Claude, Scope::OsUser, Change::Upsert { name: "fixture".into(), fields }).unwrap_err(), Error::Conflict);
    assert_eq!(fs::read_to_string(&fixture.roots.claude_user_file).unwrap(), original);
    assert_eq!(api.preview_mcp(Target::Claude, Scope::OsUser, enabled(false)).unwrap_err(), Error::Unsupported);
}

#[test]
fn native_field_bounds_and_transport_mismatches_are_rejected() {
    let fixture = Fixture::new(); let api = fixture.api();
    for fields in [
        ServerPatch { bearer_env: Field::Set("[redacted]".into()), ..Default::default() },
        ServerPatch { bearer_env: Field::Set("9BAD_ENV".into()), ..Default::default() },
        ServerPatch { forwarded_env: Field::Set(vec!["GOOD_ENV".into(), "GOOD_ENV".into()]), ..Default::default() },
        ServerPatch { forwarded_env: Field::Set(vec!["BAD=VALUE".into()]), ..Default::default() },
        ServerPatch { program: Field::Set("p".repeat(4097)), ..Default::default() },
        ServerPatch { args: Field::Set(vec!["a".into(); 257]), ..Default::default() },
        ServerPatch { header_env: Field::Set((0..65).map(|i| (format!("X-{i}"), "GOOD_ENV".into())).collect()), ..Default::default() },
        ServerPatch { program: Field::Set("never-run".into()), url: Field::Set("https://example.invalid/mcp".into()), ..Default::default() },
        ServerPatch { program: Field::Set("never-run".into()), bearer_env: Field::Set("TOKEN_ENV".into()), ..Default::default() },
    ] {
        assert!(matches!(api.preview_mcp(Target::Codex, Scope::OsUser, Change::Upsert { name: "fixture".into(), fields }), Err(Error::InvalidFormat | Error::Limit)));
        assert!(!fixture.config().exists());
    }
}

#[test]
fn enclosing_owner_markers_block_creation_without_claiming_other_scope_ownership() {
    let fixture = Fixture::new(); let api = fixture.api();
    let project = fixture.roots.project.as_ref().unwrap().to_str().unwrap();
    let original = serde_json::json!({"mcpServers":{"fixture":{"command":"user-program"}}, "projects":{project:{"owner":"managed", "mcpServers":{}}}});
    fs::write(&fixture.roots.claude_user_file, serde_json::to_vec(&original).unwrap()).unwrap();
    let before = fs::read(&fixture.roots.claude_user_file).unwrap();
    assert_eq!(api.preview_mcp(Target::Claude, Scope::ProjectLocal, add()).unwrap_err(), Error::Unsupported);
    assert_eq!(fs::read(&fixture.roots.claude_user_file).unwrap(), before);
    let preview = api.preview_mcp(Target::Claude, Scope::OsUser, add()).unwrap();
    assert_eq!(api.commit(&preview.receipt.operation_id).unwrap().state, State::Committed);
    let after: serde_json::Value = serde_json::from_slice(&fs::read(&fixture.roots.claude_user_file).unwrap()).unwrap();
    assert_eq!(after["projects"], original["projects"]);
    let fixture = Fixture::new(); let api = fixture.api();
    fixture.write("owner = 'managed'\n");
    assert_eq!(api.preview_mcp(Target::Codex, Scope::OsUser, add()).unwrap_err(), Error::Unsupported);
    assert_eq!(fs::read_to_string(fixture.config()).unwrap(), "owner = 'managed'\n");
}

#[test]
fn literal_dotted_names_and_environment_reference_updates_do_not_alias() {
    let fixture = Fixture::new(); let api = fixture.api();
    fixture.write("[mcp_servers.\"a.b\"]\nurl = 'https://example.invalid/mcp'\n[mcp_servers.a]\ncommand = 'unrelated'\n");
    let fields = ServerPatch { header_env: Field::Set(vec![("X-Env".into(), "FIRST_ENV".into())]), ..Default::default() };
    let p = api.preview_mcp(Target::Codex, Scope::OsUser, Change::Upsert { name: "a.b".into(), fields }).unwrap();
    api.commit(&p.receipt.operation_id).unwrap();
    let fields = ServerPatch { header_env: Field::Set(vec![("X-Env".into(), "SECOND_ENV".into())]), ..Default::default() };
    let p = api.preview_mcp(Target::Codex, Scope::OsUser, Change::Upsert { name: "a.b".into(), fields }).unwrap();
    api.commit(&p.receipt.operation_id).unwrap();
    let text = fs::read_to_string(fixture.config()).unwrap(); let doc = text.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(doc["mcp_servers"]["a.b"]["env_http_headers"]["X-Env"].as_str(), Some("SECOND_ENV"));
    assert_eq!(doc["mcp_servers"]["a"]["command"].as_str(), Some("unrelated"));
    assert!(doc["mcp_servers"]["a"].get("b").is_none());
}

#[test]
fn codex_static_header_cannot_be_shadowed_by_an_environment_header() {
    let fixture = Fixture::new(); let api = fixture.api();
    let original = "[mcp_servers.fixture]\nurl='https://example.invalid/mcp'\nhttp_headers={Authorization='fixture-private-value'}\n";
    fixture.write(original);
    for header in ["Authorization", "authorization"] {
        let fields = ServerPatch { header_env: Field::Set(vec![(header.into(), "TOKEN_ENV".into())]), ..Default::default() };
        assert_eq!(api.preview_mcp(Target::Codex, Scope::OsUser, Change::Upsert { name: "fixture".into(), fields }).unwrap_err(), Error::Conflict);
        assert_eq!(fs::read_to_string(fixture.config()).unwrap(), original);
    }
}

#[test]
fn native_stdio_environment_forwarding_is_explicit_and_does_not_claim_authentication() {
    let fixture = Fixture::new(); let api = fixture.api();
    let fields = ServerPatch { program: Field::Set("never-run".into()), args: Field::Set(vec!["--mode".into(), "stdio".into(), "".into()]), forwarded_env: Field::Set(vec!["TOKEN_ENV".into()]), ..Default::default() };
    let preview = api.preview_mcp(Target::Codex, Scope::OsUser, Change::Upsert { name: "fixture".into(), fields }).unwrap();
    assert_eq!(api.commit(&preview.receipt.operation_id).unwrap().state, State::Committed);
    let text = fs::read_to_string(fixture.config()).unwrap(); let doc = text.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(doc["mcp_servers"]["fixture"]["env_vars"][0].as_str(), Some("TOKEN_ENV"));
    assert_eq!(doc["mcp_servers"]["fixture"]["args"][2].as_str(), Some(""));
    let found = api.discover_mcp(Target::Codex, Scope::OsUser).unwrap();
    assert_eq!(found.instances[0].transport, Transport::Stdio);
    assert_eq!(found.instances[0].authentication.authenticated, None);
    assert_eq!(found.instances[0].authentication.login, mcp::auth::OperationSupport::Unsupported);
}
