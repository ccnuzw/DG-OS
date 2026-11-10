use super::*;
use package::{NativeSkillPackage, PackageFile};
use std::{fs, io::{Cursor, Write}, os::unix::fs::{PermissionsExt, symlink}, path::PathBuf};
struct Fixture { root: PathBuf, context: HostContext }
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!("dgos-resource-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap(); fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let roots = TrustedRoots { os_home: root.clone(), codex_home: root.join(".codex"), claude_home: root.join(".claude"), claude_user_file: root.join(".claude.json"), project: Some(root.clone()), journal: root.join("app/journal") };
        let context = HostContext::from_host("alice".into(), "lease".into(), roots, Access::Manage).unwrap();
        Self { root, context }
    }
    fn api(&self) -> NativeExtensions { NativeExtensions::new(self.context.clone()).unwrap() }
    fn skill(&self) -> PathBuf { self.root.join(".agents/skills/fixture") }
    fn install(&self, api: &NativeExtensions, target: Target, scope: Scope) -> OperationId {
        let package = pkg("first"); let source = source(&package);
        let p = api.preview_resource(target, scope, "fixture", ResourceAction::Install { package, source }).unwrap();
        let id = p.receipt.operation_id; assert_eq!(api.commit_resource(&id).unwrap().state, State::Committed); id
    }
}
impl Drop for Fixture { fn drop(&mut self) { fs::remove_dir_all(&self.root).unwrap(); } }
fn pkg(label: &str) -> NativeSkillPackage {
    NativeSkillPackage::from_files(vec![
        PackageFile { path: "SKILL.md".into(), bytes: format!("---\nname: fixture\ndescription: Deterministic resource\nunknown: keep-{label}\ndependencies:\n  tools: [fixture-tool]\n  packages: [unverified-package]\nrequired_tools: [external-api]\n---\n[resource](references/file.bin)\n{label}\n").into_bytes(), executable: false },
        PackageFile { path: "references/file.bin".into(), bytes: vec![0, 255, 1], executable: false },
        PackageFile { path: "scripts/never-run.sh".into(), bytes: b"#!/bin/sh\necho forbidden > execution-marker\n".to_vec(), executable: true },
    ]).unwrap()
}
fn source(p: &NativeSkillPackage) -> ResourceSource { ResourceSource { source_hash: p.content_hash().into(), commit: Some("a".repeat(40)) } }

#[test]
fn unrelated_journal_entries_do_not_consume_resource_history_budget() {
    let f = Fixture::new(); let api = f.api();
    f.install(&api, Target::Codex, Scope::OsUser);
    let journal = f.root.join("app/journal");
    for i in 0..4097 { fs::write(journal.join(format!("host-{i}.json")), b"unrelated").unwrap(); }
    let summary = api.resource_summary(Target::Codex, Scope::OsUser, "fixture").unwrap();
    assert!(summary.source.is_some());
}

#[test]
fn resource_journal_namespace_enforces_its_own_history_bound() {
    let f = Fixture::new(); let api = f.api();
    f.install(&api, Target::Codex, Scope::OsUser);
    let journal = f.root.join("app/journal/resources");
    for _ in 0..4096 { fs::write(journal.join(format!("resource-{}.json", uuid::Uuid::new_v4())), b"invalid history").unwrap(); }
    assert_eq!(api.resource_summary(Target::Codex, Scope::OsUser, "fixture").unwrap_err(), Error::Limit);
}

#[test]
fn symlink_skill_is_projected_read_only_without_blocking_loose_resources() {
    let f = Fixture::new(); let api = f.api();
    f.install(&api, Target::Codex, Scope::OsUser);
    symlink("fixture", f.root.join(".agents/skills/linked" )).unwrap();
    let discovery = api.discover_resources(Target::Codex, Scope::OsUser, "").unwrap();
    assert!(!discovery.incomplete);
    let link = discovery.readonly_entries.iter().find(|entry| entry.slug == "linked").unwrap();
    assert_eq!(link.target, "fixture");
    assert_eq!(link.status, ResourceLinkStatus::WithinAuthorizedRoot);
    assert_eq!(link.resolved_target.as_deref(), Some("fixture"));
    let summary = api.resource_summary(Target::Codex, Scope::OsUser, "fixture").unwrap();
    let package = pkg("update"); let source = source(&package);
    let preview = api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Update { expected: summary.instance, package, source }).unwrap();
    assert_eq!(api.commit_resource(&preview.receipt.operation_id).unwrap().state, State::Committed);
    assert!(f.root.join(".agents/skills/linked/SKILL.md").exists());
}
#[test]
fn isolated_resource_smoke() {
    let f = Fixture::new(); let api = f.api();
    let id = f.install(&api, Target::Codex, Scope::OsUser);
    assert_eq!(fs::read(f.skill().join("references/file.bin")).unwrap(), vec![0,255,1]);
    assert_ne!(fs::metadata(f.skill().join("scripts/never-run.sh")).unwrap().permissions().mode() & 0o111, 0);
    let before = api.resource_summary(Target::Codex, Scope::OsUser, "fixture").unwrap();
    assert!(before.can_update); assert!(before.metadata.dependencies.declared);
    assert_eq!(api.discover_resources(Target::Codex, Scope::OsUser, "deterministic").unwrap().instances[0].instance, before.instance);
    let update = pkg("second"); let src = source(&update);
    let p = api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Update { expected: before.instance, package: update, source: src }).unwrap();
    assert!(fs::read_to_string(f.skill().join("SKILL.md")).unwrap().contains("keep-first"));
    assert_eq!(api.commit_resource(&p.receipt.operation_id).unwrap().state, State::Committed);
    let retained = api.resource_recovery_material(&p.receipt.operation_id).unwrap();
    assert!(fs::read_to_string(retained.directory.join("payload/SKILL.md")).unwrap().contains("keep-first"));
    let summary = api.resource_summary(Target::Codex, Scope::OsUser, "fixture").unwrap();
    let p = api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Remove { expected: summary.instance }).unwrap();
    assert_eq!(api.commit_resource(&p.receipt.operation_id).unwrap().state, State::Committed);
    assert!(!f.skill().exists());
    assert!(api.resource_recovery_material(&p.receipt.operation_id).unwrap().directory.join("payload/SKILL.md").exists());
    assert_eq!(api.inspect_resource(&id).unwrap().state, State::Committed);
    assert!(!f.root.join("execution-marker").exists());
    assert!(!f.root.join(".codex").exists() && !f.root.join(".claude").exists());
}
#[test]
fn first_use_missing_roots_are_read_only_and_independent() {
    let f = Fixture::new(); let api = f.api();
    for target in [Target::Codex, Target::Claude] {
        let found = api.discover_resources(target, Scope::OsUser, "").unwrap(); assert!(found.missing && !found.incomplete);
        assert!(!api.discover_mcp(target, Scope::OsUser).unwrap().incomplete);
        assert!(api.discover_skills(target, Scope::OsUser).unwrap().incomplete);
    }
    assert!(!f.root.join("app").exists() && !f.root.join(".claude").exists());
    let change = mcp::Change::Upsert { name: "fixture".into(), fields: mcp::ServerPatch { program: mcp::Field::Set("never-run".into()), ..Default::default() } };
    let p = api.preview_mcp(Target::Codex, Scope::OsUser, change).unwrap();
    assert_eq!(api.commit(&p.receipt.operation_id).unwrap().state, State::Committed);
    assert!(f.root.join(".codex/config.toml").exists()); assert!(!f.root.join(".claude").exists());
    assert_eq!(f.api().inspect(&p.receipt.operation_id).unwrap().state, State::Committed);
    fs::rename(f.root.join(".codex"), f.root.join("old-codex")).unwrap();
    symlink(f.root.join("old-codex"), f.root.join(".codex")).unwrap();
    // The other engine can still be installed despite an unusable Codex home.
    f.install(&f.api(), Target::Claude, Scope::OsUser);
    assert!(f.root.join(".claude/skills/fixture/SKILL.md").exists());
    assert!(f.api().discover_mcp(Target::Codex, Scope::OsUser).is_err());
}
#[test]
fn resource_external_edits_root_replacements_and_revocation_reject_commit() {
    let f = Fixture::new(); let api = f.api(); f.install(&api, Target::Codex, Scope::OsUser);
    let s = api.resource_summary(Target::Codex, Scope::OsUser, "fixture").unwrap();
    let p = api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Remove { expected: s.instance }).unwrap();
    fs::write(f.skill().join("references/file.bin"), b"external").unwrap();
    assert_eq!(api.commit_resource(&p.receipt.operation_id).unwrap().state, State::Conflict);
    let s = api.resource_summary(Target::Codex, Scope::OsUser, "fixture").unwrap(); assert!(!s.can_update);
    let p = api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Remove { expected: s.instance }).unwrap();
    f.context.revoke().unwrap(); assert_eq!(api.commit_resource(&p.receipt.operation_id).unwrap_err(), Error::AccessDenied);
    assert_eq!(fs::read(f.skill().join("references/file.bin")).unwrap(), b"external");
}
#[test]
fn unknown_source_cannot_be_updated_and_scopes_are_independent() {
    let f = Fixture::new(); let mut context = f.context.clone();
    let project = f.root.join("project"); fs::create_dir(&project).unwrap(); context.roots.project = Some(project.clone());
    let api = NativeExtensions::new(context).unwrap();
    f.install(&api, Target::Claude, Scope::OsUser); f.install(&api, Target::Claude, Scope::ProjectShared);
    let user = api.resource_summary(Target::Claude, Scope::OsUser, "fixture").unwrap();
    let shared = api.resource_summary(Target::Claude, Scope::ProjectShared, "fixture").unwrap();
    assert_ne!(user.instance.id, shared.instance.id);
    let p = api.preview_resource(Target::Claude, Scope::OsUser, "fixture", ResourceAction::Remove { expected: user.instance }).unwrap();
    api.commit_resource(&p.receipt.operation_id).unwrap();
    assert!(project.join(".claude/skills/fixture/SKILL.md").exists());
    let raw = f.root.join(".agents/skills/untracked"); fs::create_dir_all(&raw).unwrap();
    fs::write(raw.join("SKILL.md"), b"---\nname: untracked\n---\nbody").unwrap();
    let unknown = api.resource_summary(Target::Codex, Scope::OsUser, "untracked").unwrap(); assert!(!unknown.can_update);
    let package = pkg("new"); let source = source(&package);
    assert_eq!(api.preview_resource(Target::Codex, Scope::OsUser, "untracked", ResourceAction::Update { expected: unknown.instance, package, source }).unwrap_err(), Error::Conflict);
}
#[test]
fn plugin_managed_and_symlink_instances_are_not_adopted() {
    for marker in [".claude-plugin", ".managed", ".CLAUDE-PLUGIN", ".MANAGED", "PLUGIN.JSON", "MANAGED-POLICY.JSON"] {
        let f = Fixture::new(); let api = f.api(); f.install(&api, Target::Codex, Scope::OsUser);
        fs::create_dir(f.skill().join(marker)).unwrap();
        let summary = api.resource_summary(Target::Codex, Scope::OsUser, "fixture").unwrap();
        assert!(!summary.can_update);
        assert_eq!(api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Remove { expected: summary.instance }).unwrap_err(), Error::Unsupported);
    }
    let f = Fixture::new(); let api = f.api(); fs::create_dir_all(f.root.join(".agents/skills")).unwrap();
    symlink(&f.root, f.skill()).unwrap();
    assert!(api.resource_summary(Target::Codex, Scope::OsUser, "fixture").is_err());
}
#[test]
fn static_dependencies_never_execute_candidate() {
    let f = Fixture::new(); let api = f.api(); f.install(&api, Target::Codex, Scope::OsUser);
    let bin = f.root.join("bin"); fs::create_dir(&bin).unwrap();
    fs::write(bin.join("fixture-tool"), b"#!/bin/sh\necho executed > marker\n").unwrap();
    fs::set_permissions(bin.join("fixture-tool"), fs::Permissions::from_mode(0o700)).unwrap();
    let env = dependencies::Environment::from_host(vec![bin]).unwrap();
    let report = api.scan_resource_dependencies(Target::Codex, Scope::OsUser, "fixture", &env).unwrap();
    assert_eq!(report.tools[0].availability, dependencies::Availability::FoundUnprobed); assert!(report.unknown);
    assert!(!f.root.join("marker").exists());
    assert!(env.locate(&dependencies::Declarations::default()).unknown);
}
#[test]
fn package_limits_paths_aliases_references_and_plugins_are_rejected() {
    for path in ["../escape", "/absolute", "a//b", "a\\b", "a/../b", "a/b/c/d/e/f/g/h/i"] {
        assert!(NativeSkillPackage::from_files(vec![PackageFile { path: path.into(), bytes: vec![], executable: false }]).is_err());
    }
    for pair in [["Refs/a", "refs/b"], ["a", "a/b"], ["A", "a"]] {
        assert!(NativeSkillPackage::from_files(pair.into_iter().map(|path| PackageFile { path: path.into(), bytes: vec![], executable: false }).collect()).is_err());
    }
    let body = b"---\nname: fixture\n---\n[escape](../secret)\n";
    assert!(NativeSkillPackage::from_files(vec![PackageFile { path: "SKILL.md".into(), bytes: body.to_vec(), executable: false }]).is_err());
    let mut files: Vec<_> = pkg("first").files.into_values().collect();
    files.push(PackageFile { path: ".claude-plugin/plugin.json".into(), bytes: b"{}".to_vec(), executable: false });
    assert!(matches!(NativeSkillPackage::from_files(files), Err(Error::Unsupported)));
    assert!(matches!(NativeSkillPackage::from_files(vec![PackageFile { path: "SKILL.md".into(), bytes: vec![b'x'; 256*1024+1], executable: false }]), Err(Error::Limit)));
}
fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, bytes) in entries { writer.start_file(*path, zip::write::SimpleFileOptions::default()).unwrap(); writer.write_all(bytes).unwrap(); }
    writer.finish().unwrap().into_inner()
}
#[test]
fn zip_boundaries_are_checked_even_outside_selection() {
    let skill = b"---\nname: fixture\n---\nbody\n";
    let bytes = archive(&[("repo/SKILL.md", skill), ("repo/scripts/run.sh", b"exit 19")]);
    let package = NativeSkillPackage::from_zip(&bytes, None).unwrap(); assert_eq!(package.summary()[0].path, "SKILL.md");
    for bytes in [archive(&[("repo/SKILL.md", skill), ("../escape", b"bad")]), archive(&[("SKILL.md", skill), ("A", b"1"), ("a", b"2")]), archive(&[("SKILL.md", skill), ("large", &vec![0; package::FILE_LIMIT+1])])] {
        assert!(NativeSkillPackage::from_zip(&bytes, None).is_err());
    }
}
#[test]
fn claude_mixed_headers_group_remove_rejects_without_modifying_secrets() {
    let original = r#"{"mcpServers":{"fixture":{"type":"http","url":"https://example.invalid/mcp","headers":{"Authorization":"static-secret","X-Env":"${FIXTURE_ENV}"}}}}"#;
    let change = mcp::Change::Upsert { name: "fixture".into(), fields: mcp::ServerPatch { header_env: mcp::Field::Remove, ..Default::default() } };
    let f = Fixture::new(); fs::write(f.root.join(".claude.json"), original).unwrap(); let api = f.api();
    assert_eq!(api.preview_mcp(Target::Claude, Scope::OsUser, change).unwrap_err(), Error::Unsupported);
    assert_eq!(fs::read_to_string(f.root.join(".claude.json")).unwrap(), original);
}
#[test]
fn acl_inheritance_on_parent_or_existing_stage_is_rejected_before_payload_write() {
    let f = Fixture::new(); let api = f.api();
    fs::create_dir_all(f.root.join(".codex")).unwrap();
    let ordinary = super::paths::Directory::open(&f.root.join(".codex")).unwrap();
    ordinary.writable_owner().unwrap();
    ordinary.write_new("normal", b"private", 0o600).unwrap();
    let normal = fs::File::open(f.root.join(".codex/normal")).unwrap();
    super::paths::no_extended_acl(&normal).unwrap();
    assert_eq!(fs::metadata(f.root.join(".codex/normal")).unwrap().permissions().mode() & 0o777, 0o600);
    let status = std::process::Command::new("/bin/chmod").args(["+a", "everyone allow read,write,delete,add_file,add_subdirectory,file_inherit,directory_inherit"]).arg(f.root.join(".codex")).status().unwrap(); assert!(status.success());
    let inherited = f.root.join(".codex/inherited");
    fs::write(&inherited, b"fixture-only").unwrap();
    fs::set_permissions(&inherited, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(super::paths::no_extended_acl(&fs::File::open(&inherited).unwrap()).unwrap_err(), Error::AccessDenied);
    let dir = super::paths::Directory::open(&f.root.join(".codex")).unwrap();
    assert_eq!(dir.writable_owner().unwrap_err(), Error::AccessDenied);
    assert!(dir.write_new("payload", b"private-secret", 0o600).is_err()); assert!(!f.root.join(".codex/payload").exists());
    let change = mcp::Change::Upsert { name: "fixture".into(), fields: mcp::ServerPatch { program: mcp::Field::Set("never-run".into()), ..Default::default() } };
    assert_eq!(api.preview_mcp(Target::Codex, Scope::OsUser, change).unwrap_err(), Error::AccessDenied);
    assert!(!f.root.join(".codex/.dgos-native-recovery").exists());
    let stage = f.root.join("stage"); fs::create_dir(&stage).unwrap(); fs::set_permissions(&stage, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(std::process::Command::new("/bin/chmod").args(["+a", "everyone allow read,search"]).arg(&stage).status().unwrap().success());
    assert_eq!(super::paths::Directory::open(&stage).unwrap().private().unwrap_err(), Error::AccessDenied);
}

#[test]
fn directory_exchange_crash_recovery_retains_all_versions_without_replay() {
    let f = Fixture::new(); let api = f.api(); f.install(&api, Target::Codex, Scope::OsUser);
    let old = api.resource_summary(Target::Codex, Scope::OsUser, "fixture").unwrap();
    let package = pkg("next"); let source = source(&package);
    let p = api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Update { expected: old.instance, package, source }).unwrap();
    let recovery = api.resource_recovery_material(&p.receipt.operation_id).unwrap();
    let journal = f.root.join("app/journal/resources").join(format!("resource-{}.json", p.receipt.operation_id.as_str()));
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
    record["state"] = serde_json::json!("Publishing"); fs::write(&journal, serde_json::to_vec(&record).unwrap()).unwrap();
    let parent = super::paths::Directory::open(&f.root.join(".agents/skills")).unwrap();
    let stage = super::paths::Directory::open(&recovery.directory).unwrap();
    parent.publish(&stage, "payload", "fixture", true).unwrap();
    assert_eq!(f.api().inspect_resource(&p.receipt.operation_id).unwrap().state, State::Committed);
    assert!(fs::read_to_string(recovery.directory.join("payload/SKILL.md")).unwrap().contains("keep-first"));
    assert!(fs::read_to_string(f.skill().join("SKILL.md")).unwrap().contains("keep-next"));
    assert_eq!(api.commit_resource(&p.receipt.operation_id).unwrap().state, State::Committed);
}

#[test]
fn directory_replacement_and_racing_install_preserve_external_tree() {
    let f = Fixture::new(); let api = f.api();
    let package = pkg("first"); let source = source(&package);
    let p = api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Install { package, source }).unwrap();
    fs::create_dir(f.skill()).unwrap(); fs::write(f.skill().join("external"), b"keep").unwrap();
    assert_eq!(api.commit_resource(&p.receipt.operation_id).unwrap().state, State::Conflict);
    assert_eq!(fs::read(f.skill().join("external")).unwrap(), b"keep");
    fs::rename(f.root.join(".agents/skills"), f.root.join("displaced-skills")).unwrap();
    fs::create_dir(f.root.join(".agents/skills")).unwrap();
    assert_eq!(api.inspect_resource(&p.receipt.operation_id).unwrap_err(), Error::RootChanged);
}

#[test]
fn local_packages_reject_hardlinks_symlinks_and_special_files() {
    let f = Fixture::new(); let api = f.api(); f.install(&api, Target::Codex, Scope::OsUser);
    let bytes = fs::read(f.skill().join("SKILL.md")).unwrap();
    fs::hard_link(f.skill().join("SKILL.md"), f.root.join("hardlink")).unwrap();
    assert!(NativeSkillPackage::from_directory(&f.skill()).is_err());
    fs::remove_file(f.root.join("hardlink")).unwrap();
    symlink(f.root.join("outside"), f.skill().join("link")).unwrap();
    assert!(NativeSkillPackage::from_directory(&f.skill()).is_err());
    fs::remove_file(f.skill().join("link")).unwrap();
    let fifo = std::ffi::CString::new(f.skill().join("fifo").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(NativeSkillPackage::from_directory(&f.skill()).is_err());
    assert_eq!(fs::read(f.skill().join("SKILL.md")).unwrap(), bytes);
}

#[test]
fn source_binding_expiry_and_account_boundaries_prevent_resource_publication() {
    let f = Fixture::new(); let api = f.api(); let package = pkg("first");
    let wrong = ResourceSource { source_hash: "0".repeat(64), commit: None };
    assert_eq!(api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Install { package, source: wrong }).unwrap_err(), Error::Conflict);
    let package = pkg("first"); let source = source(&package);
    let p = api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Install { package, source }).unwrap();
    let mut bob = f.context.clone(); bob.principal = "bob".into();
    assert_eq!(NativeExtensions::new(bob).unwrap().inspect_resource(&p.receipt.operation_id).unwrap_err(), Error::NotFound);
    let reauthorized_context = HostContext::from_host("alice".into(), "lease-b".into(), f.context.roots.clone(), Access::Manage).unwrap();
    let reauthorized = NativeExtensions::new(reauthorized_context).unwrap();
    assert_eq!(reauthorized.inspect_resource(&p.receipt.operation_id).unwrap().state, State::Previewed);
    let read_context = HostContext::from_host("alice".into(), "lease-a".into(), f.context.roots.clone(), Access::Read).unwrap();
    let read = NativeExtensions::new(read_context).unwrap();
    assert_eq!(read.inspect_resource(&p.receipt.operation_id).unwrap_err(), Error::AccessDenied);
    let journal = f.root.join("app/journal/resources").join(format!("resource-{}.json", p.receipt.operation_id.as_str()));
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
    record["expires_at"] = serde_json::json!(0); fs::write(journal, serde_json::to_vec(&record).unwrap()).unwrap();
    assert_eq!(api.commit_resource(&p.receipt.operation_id).unwrap_err(), Error::Expired);
    assert!(!f.skill().exists());
}

#[test]
fn source_execution_bits_survive_local_and_zip_installation() {
    let f = Fixture::new(); let api = f.api(); f.install(&api, Target::Codex, Scope::OsUser);
    let script = f.skill().join("scripts/never-run.sh");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o710)).unwrap();
    let package = NativeSkillPackage::from_directory(&f.skill()).unwrap(); let src = source(&package);
    let p = api.preview_resource(Target::Claude, Scope::OsUser, "fixture", ResourceAction::Install { package, source: src }).unwrap();
    api.commit_resource(&p.receipt.operation_id).unwrap();
    assert_eq!(fs::metadata(f.root.join(".claude/skills/fixture/scripts/never-run.sh")).unwrap().permissions().mode() & 0o111, 0o110);
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("SKILL.md", zip::write::SimpleFileOptions::default()).unwrap();
    writer.write_all(b"---\nname: fixture\n---\nbody").unwrap();
    writer.start_file("scripts/check", zip::write::SimpleFileOptions::default().unix_permissions(0o701)).unwrap();
    writer.write_all(b"#!/bin/sh\nexit 29").unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let project = f.root.join("separate-project"); fs::create_dir(&project).unwrap();
    let mut context = f.context.clone(); context.roots.project = Some(project.clone());
    let project_api = NativeExtensions::new(context).unwrap();
    let package = NativeSkillPackage::from_zip(&bytes, None).unwrap(); let src = source(&package);
    let p = project_api.preview_resource(Target::Claude, Scope::ProjectShared, "fixture", ResourceAction::Install { package, source: src }).unwrap();
    project_api.commit_resource(&p.receipt.operation_id).unwrap();
    assert_eq!(fs::metadata(project.join(".claude/skills/fixture/scripts/check")).unwrap().permissions().mode() & 0o111, 0o101);
}

#[test]
fn exchanged_tree_modified_through_old_inode_requires_recovery_without_replay() {
    let f = Fixture::new(); let api = f.api(); f.install(&api, Target::Codex, Scope::OsUser);
    let expected = api.resource_summary(Target::Codex, Scope::OsUser, "fixture").unwrap().instance;
    let mut old = fs::OpenOptions::new().write(true).truncate(false).open(f.skill().join("SKILL.md")).unwrap();
    let package = pkg("next"); let source = source(&package);
    let p = api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Update { expected, package, source }).unwrap();
    let recovery = api.resource_recovery_material(&p.receipt.operation_id).unwrap();
    let journal = f.root.join("app/journal/resources").join(format!("resource-{}.json", p.receipt.operation_id.as_str()));
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
    record["state"] = serde_json::json!("Publishing"); fs::write(journal, serde_json::to_vec(&record).unwrap()).unwrap();
    let parent = super::paths::Directory::open(&f.root.join(".agents/skills")).unwrap();
    let stage = super::paths::Directory::open(&recovery.directory).unwrap();
    parent.publish(&stage, "payload", "fixture", true).unwrap();
    old.write_all(b"external edit retained").unwrap(); old.sync_all().unwrap();
    assert_eq!(api.inspect_resource(&p.receipt.operation_id).unwrap().state, State::RecoveryRequired);
    assert_eq!(api.commit_resource(&p.receipt.operation_id).unwrap().state, State::RecoveryRequired);
    assert!(fs::read_to_string(recovery.directory.join("payload/SKILL.md")).unwrap().starts_with("external edit retained"));
    assert!(fs::read_to_string(f.skill().join("SKILL.md")).unwrap().contains("keep-next"));
}

#[test]
fn archive_collisions_outside_selected_skill_are_not_ignored() {
    let skill = b"---\nname: fixture\n---\nbody";
    for pair in [["repo/other/A/x", "repo/other/a/y"], ["repo/other/a", "repo/other/a/x"]] {
        let bytes = archive(&[("repo/chosen/SKILL.md", skill), (pair[0], b"a"), (pair[1], b"b")]);
        assert!(matches!(NativeSkillPackage::from_zip(&bytes, Some("chosen")), Err(Error::Conflict)));
    }
}

const RESERVED_CASE_PATHS: &[&str] = &[
    ".CLAUDE-PLUGIN/plugin.json", "PLUGIN.JSON", "HoOkS/HoOkS.JsOn",
    ".MANAGED/policy", ".SYSTEM/policy", "MANAGED/policy", "SyNcEd/skill",
    "MANAGED-POLICY.JSON", ".DGOS-NATIVE-SOURCE.JSON", "nested/.ClAuDe-PlUgIn/plugin.json",
];

#[test]
fn reserved_markers_reject_ascii_case_variants_in_files_and_zip() {
    let skill = b"---\nname: fixture\n---\nbody\n";
    for path in RESERVED_CASE_PATHS {
        let mut files: Vec<_> = pkg("first").files.into_values().collect();
        files.push(PackageFile { path: (*path).into(), bytes: b"{}".to_vec(), executable: false });
        assert!(matches!(NativeSkillPackage::from_files(files), Err(Error::Unsupported)), "{path}");
        let wrapped = format!("repo/chosen/{path}");
        let bytes = archive(&[("repo/chosen/SKILL.md", skill), (&wrapped, b"{}")]);
        assert!(matches!(NativeSkillPackage::from_zip(&bytes, Some("chosen")), Err(Error::Unsupported)), "{path}");
    }
}

#[test]
fn commit_rechecks_staged_markers_even_with_matching_tree_snapshot() {
    for path in RESERVED_CASE_PATHS {
        for updating in [false, true] {
            let f = Fixture::new(); let api = f.api();
            let action = if updating {
                f.install(&api, Target::Codex, Scope::OsUser);
                let expected = api.resource_summary(Target::Codex, Scope::OsUser, "fixture").unwrap().instance;
                let package = pkg("next"); let source = source(&package);
                ResourceAction::Update { expected, package, source }
            } else {
                let package = pkg("first"); let source = source(&package);
                ResourceAction::Install { package, source }
            };
            let preview = api.preview_resource(Target::Codex, Scope::OsUser, "fixture", action).unwrap();
            let id = &preview.receipt.operation_id;
            let recovery = api.resource_recovery_material(id).unwrap();
            let payload = recovery.directory.join("payload");
            let marker = payload.join(path);
            fs::create_dir_all(marker.parent().unwrap()).unwrap(); fs::write(&marker, b"{}").unwrap();
            // Model an old accepted package, not merely an after-hash mismatch.
            let snapshot = super::resource_tree::scan(&super::paths::Directory::open(&payload).unwrap()).unwrap().0;
            let journal = f.root.join("app/journal/resources").join(format!("resource-{}.json", id.as_str()));
            let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
            record["after"] = serde_json::to_value(snapshot).unwrap();
            fs::write(&journal, serde_json::to_vec(&record).unwrap()).unwrap();
            assert_eq!(api.commit_resource(id).unwrap_err(), Error::Unsupported, "{path}");
            if updating { assert!(fs::read_to_string(f.skill().join("SKILL.md")).unwrap().contains("keep-first")); }
            else { assert!(!f.skill().exists()); }
            assert_eq!(fs::read(&marker).unwrap(), b"{}");
            assert!(!f.root.join("execution-marker").exists());
        }
    }
}

#[test]
fn reserved_slug_case_variants_cannot_be_installed_as_loose_resources() {
    let f = Fixture::new(); let api = f.api();
    for slug in ["MANAGED", "SyNcEd"] {
        let package = pkg("first"); let source = source(&package);
        assert_eq!(api.preview_resource(Target::Codex, Scope::OsUser, slug, ResourceAction::Install { package, source }).unwrap_err(), Error::Unsupported);
    }
    assert!(!f.root.join(".agents").exists());
}

#[test]
fn empty_reserved_directories_are_rejected_in_directory_zip_and_commit() {
    let f = Fixture::new(); let api = f.api();
    let package = pkg("first"); let source = source(&package);
    let preview = api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Install { package, source }).unwrap();
    let id = &preview.receipt.operation_id;
    let payload = api.resource_recovery_material(id).unwrap().directory.join("payload");
    fs::create_dir(payload.join(".CLAUDE-PLUGIN")).unwrap();
    assert!(matches!(NativeSkillPackage::from_directory(&payload), Err(Error::Unsupported)));
    assert_eq!(api.commit_resource(id).unwrap_err(), Error::Unsupported);
    assert!(!f.skill().exists());
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("SKILL.md", zip::write::SimpleFileOptions::default()).unwrap();
    writer.write_all(b"---\nname: fixture\n---\nbody").unwrap();
    writer.add_directory(".CLAUDE-PLUGIN/", zip::write::SimpleFileOptions::default()).unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    assert!(matches!(NativeSkillPackage::from_zip(&bytes, None), Err(Error::Unsupported)));
}

#[test]
fn project_resource_operations_reject_other_or_missing_authorized_project() {
    for target in [Target::Codex, Target::Claude] {
        let f = Fixture::new();
        let a = f.root.join("project-a"); let b = f.root.join("project-b");
        fs::create_dir(&a).unwrap(); fs::create_dir(&b).unwrap();
        let context = |project| {
            let mut roots = f.context.roots.clone(); roots.project = project;
            HostContext::from_host("alice".into(), "lease".into(), roots, Access::Manage).unwrap()
        };
        let api = NativeExtensions::new(context(Some(a.clone()))).unwrap();
        let package = pkg("first"); let source = source(&package);
        let preview = api.preview_resource(target, Scope::ProjectShared, "fixture", ResourceAction::Install { package, source }).unwrap();
        let id = preview.receipt.operation_id;
        let journal = f.root.join("app/journal/resources").join(format!("resource-{}.json", id.as_str()));
        let original = fs::read(&journal).unwrap();
        for project in [Some(b), None] {
            let other = NativeExtensions::new(context(project)).unwrap();
            assert_eq!(other.cancel_resource(&id).unwrap_err(), Error::AccessDenied);
            assert_eq!(other.commit_resource(&id).unwrap_err(), Error::AccessDenied);
            assert_eq!(other.inspect_resource(&id).unwrap_err(), Error::AccessDenied);
            assert!(matches!(other.resource_recovery_material(&id), Err(Error::AccessDenied)));
            assert_eq!(fs::read(&journal).unwrap(), original);
        }
        assert_eq!(api.cancel_resource(&id).unwrap().state, State::Cancelled);
    }
}

#[test]
fn installed_reserved_trees_are_read_only_and_never_removed() {
    for path in RESERVED_CASE_PATHS {
        let f = Fixture::new(); let api = f.api(); f.install(&api, Target::Codex, Scope::OsUser);
        let marker = f.skill().join(path);
        fs::create_dir_all(marker.parent().unwrap()).unwrap(); fs::write(&marker, b"{}").unwrap();
        let summary = api.resource_summary(Target::Codex, Scope::OsUser, "fixture").unwrap();
        assert_ne!(summary.owner, Owner::Loose, "{path}");
        assert!(!summary.can_update);
        assert_eq!(api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Remove { expected: summary.instance }).unwrap_err(), Error::Unsupported, "{path}");
        assert_eq!(fs::read(&marker).unwrap(), b"{}");
    }
}

#[test]
fn historical_removal_preview_cannot_remove_a_reserved_tree() {
    for path in RESERVED_CASE_PATHS {
        let f = Fixture::new(); let api = f.api(); f.install(&api, Target::Codex, Scope::OsUser);
        let expected = api.resource_summary(Target::Codex, Scope::OsUser, "fixture").unwrap().instance;
        let preview = api.preview_resource(Target::Codex, Scope::OsUser, "fixture", ResourceAction::Remove { expected }).unwrap();
        let marker = f.skill().join(path);
        fs::create_dir_all(marker.parent().unwrap()).unwrap(); fs::write(&marker, b"{}").unwrap();
        let snapshot = super::resource_tree::scan(&super::paths::Directory::open(&f.skill()).unwrap()).unwrap().0;
        let journal = f.root.join("app/journal/resources").join(format!("resource-{}.json", preview.receipt.operation_id.as_str()));
        let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
        record["before"] = serde_json::to_value(snapshot).unwrap();
        fs::write(&journal, serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(api.commit_resource(&preview.receipt.operation_id).unwrap_err(), Error::Unsupported, "{path}");
        assert_eq!(fs::read(&marker).unwrap(), b"{}");
        assert!(fs::read_to_string(f.skill().join("SKILL.md")).unwrap().contains("keep-first"));
    }
}

#[test]
fn removal_preview_lists_exact_resources_and_bounded_same_name_files() {
    let f = Fixture::new();
    let project = f.root.join("project"); fs::create_dir(&project).unwrap();
    let mut context = f.context.clone(); context.roots.project = Some(project.clone());
    let api = NativeExtensions::new(context).unwrap();
    f.install(&api, Target::Claude, Scope::OsUser);
    f.install(&api, Target::Claude, Scope::ProjectShared);
    let summary = api.resource_summary(Target::Claude, Scope::OsUser, "fixture").unwrap();
    let preview = api.preview_resource(Target::Claude, Scope::OsUser, "fixture", ResourceAction::Remove { expected: summary.instance }).unwrap();
    assert!(preview.after_hash.is_none());
    assert_eq!(preview.metadata.as_ref().unwrap().name, "fixture");
    assert_eq!(preview.resources.len(), 3);
    assert!(preview.resources.iter().any(|file| file.path == "scripts/never-run.sh" && file.executable));
    assert!(preview.resources.iter().any(|file| file.path == "references/file.bin" && file.bytes == 3));
    let impact = preview.impact.unwrap();
    assert_eq!(impact.coverage, "authorized_loose_roots");
    assert_eq!(impact.native_name, "fixture");
    assert!(!impact.visibility_writable);
    assert!(!impact.incomplete);
    assert!(impact.same_name.iter().any(|entry| entry.scope == Scope::OsUser));
    assert!(impact.same_name.iter().any(|entry| entry.scope == Scope::ProjectShared));
    assert!(f.root.join(".claude/skills/fixture/SKILL.md").exists());
    assert!(f.root.join(".claude/skills/fixture/references/file.bin").exists());
    assert!(project.join(".claude/skills/fixture/SKILL.md").exists());
}

#[test]
fn skill_root_ownership_cannot_be_bypassed_by_a_loose_child() {
    for marker in [".claude-plugin", ".MANAGED", "PLUGIN.JSON"] {
        let f = Fixture::new(); let api = f.api();
        f.install(&api, Target::Claude, Scope::OsUser);
        let summary = api.resource_summary(Target::Claude, Scope::OsUser, "fixture").unwrap();
        let preview = api.preview_resource(Target::Claude, Scope::OsUser, "fixture", ResourceAction::Remove { expected: summary.instance }).unwrap();
        fs::write(f.root.join(".claude/skills").join(marker), b"{}").unwrap();
        let protected = api.resource_summary(Target::Claude, Scope::OsUser, "fixture").unwrap();
        assert_ne!(protected.owner, Owner::Loose);
        assert!(!protected.can_update);
        let discovered = api.discover_skills(Target::Claude, Scope::OsUser).unwrap();
        assert_ne!(discovered.instances.iter().find(|entry| entry.directory_name == "fixture").unwrap().owner, Owner::Loose);
        assert_eq!(api.commit_resource(&preview.receipt.operation_id).unwrap_err(), Error::Unsupported);
        assert_eq!(api.preview_resource(Target::Claude, Scope::OsUser, "fixture", ResourceAction::Remove { expected: protected.instance }).unwrap_err(), Error::Unsupported);
        let package = pkg("new"); let source = source(&package);
        assert_eq!(api.preview_resource(Target::Claude, Scope::OsUser, "new", ResourceAction::Install { package, source }).unwrap_err(), Error::Unsupported);
        assert!(f.root.join(".claude/skills/fixture/SKILL.md").exists());
    }
}

#[test]
fn mcp_creation_cannot_exceed_complete_discovery_limit() {
    let mut servers = serde_json::Map::new();
    for index in 0..2048 { servers.insert(format!("s{index:04}"), serde_json::json!({"command":"never-run"})); }
    let original = serde_json::json!({"mcpServers":servers}).to_string();
    let change = mcp::Change::Upsert { name: "new".into(), fields: mcp::ServerPatch { program: mcp::Field::Set("never-run".into()), ..Default::default() } };
    mcp::validate_original(&original, Target::Claude, &Scope::OsUser, None, &change).unwrap();
    let patches = mcp::patches(Target::Claude, &Scope::OsUser, None, change.clone()).unwrap();
    let edited = super::config::apply(&original, Target::Claude, &patches).unwrap();
    assert_eq!(mcp::validate_edited(&edited, Target::Claude, &Scope::OsUser, None, &change).unwrap_err(), Error::Limit);
}

#[test]
fn claude_update_previews_both_declared_names_without_claiming_visibility() {
    let f = Fixture::new(); let api = f.api();
    f.install(&api, Target::Claude, Scope::OsUser);
    let summary = api.resource_summary(Target::Claude, Scope::OsUser, "fixture").unwrap();
    let package = NativeSkillPackage::from_files(vec![PackageFile { path: "SKILL.md".into(), bytes: b"---\nname: renamed\ndescription: renamed fixture\n---\nbody\n".to_vec(), executable: false }]).unwrap();
    let source = source(&package);
    symlink("fixture", f.root.join(".claude/skills/linked")).unwrap();
    let preview = api.preview_resource(Target::Claude, Scope::OsUser, "fixture", ResourceAction::Update { expected: summary.instance, package, source }).unwrap();
    let impact = preview.impact.unwrap();
    assert_eq!(impact.native_name, "renamed");
    assert_eq!(impact.previous_name.as_deref(), Some("fixture"));
    assert!(impact.same_name.iter().any(|entry| entry.slug == "fixture" && entry.native_name == "fixture"));
    assert!(impact.incomplete); // A linked instance's metadata was not read.
    assert!(!impact.visibility_writable);
    assert!(fs::read_to_string(f.root.join(".claude/skills/fixture/SKILL.md")).unwrap().contains("name: fixture"));
}
