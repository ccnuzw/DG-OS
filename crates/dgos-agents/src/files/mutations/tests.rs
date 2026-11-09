use super::*;
use crate::projects::{CreateProjectInput, SaveTaskInput, UpdateProjectInput};
use std::path::PathBuf;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-entry-mutations-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("code")).unwrap(); Self(std::fs::canonicalize(root).unwrap())
    }
    fn code(&self) -> PathBuf { self.0.join("code") }
    async fn manager(&self) -> (AgentManager, Project) {
        let manager = crate::test_support::manager(&self.0).await;
        let project = manager.create_project("alice", CreateProjectInput { kind: "import".into(), name: "Entries".into(), path: self.code().to_str().unwrap().into(), source: None }, CancellationToken::new()).await.unwrap();
        (manager, project)
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn inspect(project: &Project, path: &str) -> InspectEntryInput { InspectEntryInput { project_id: project.id.clone(), task_id: None, path: path.into() } }
fn input(project: &Project, path: &str, version: &str, action: MutationAction, destination: Option<&str>) -> MutateEntryInput {
    MutateEntryInput { project_id: project.id.clone(), task_id: None, path: path.into(), expected_version: version.into(), action, destination: destination.map(str::to_owned), trust_directory: true }
}
fn current(project: &Project, path: &str) -> String { inspect_at(project, path, &CancellationToken::new()).unwrap().version }

#[tokio::test]
async fn recursion_copies_binary_and_large_files_renames_and_deletes_without_overwrite() {
    let f = Fixture::new(); let (manager, project) = f.manager().await;
    std::fs::create_dir_all(f.code().join("tree/nested/empty")).unwrap();
    let bytes = vec![0u8; FILE_LIMIT * 2 + 17];
    std::fs::write(f.code().join("tree/nested/data.bin"), &bytes).unwrap();
    std::fs::write(f.code().join("existing"), "keep").unwrap();
    let version = current(&project, "tree");
    for action in [MutationAction::Copy, MutationAction::Rename] {
        assert_eq!(manager.mutate_development_entry("alice", input(&project, "tree", &version, action, Some("existing")), CancellationToken::new()).await.unwrap_err(), "file_exists");
    }
    assert_eq!(std::fs::read_to_string(f.code().join("existing")).unwrap(), "keep");
    manager.mutate_development_entry("alice", input(&project, "tree", &version, MutationAction::Copy, Some("copy")), CancellationToken::new()).await.unwrap();
    assert_eq!(std::fs::read(f.code().join("copy/nested/data.bin")).unwrap(), bytes);
    assert!(f.code().join("copy/nested/empty").is_dir());
    assert_eq!(current(&project, "tree"), version);
    let copied = current(&project, "copy");
    manager.mutate_development_entry("alice", input(&project, "copy", &copied, MutationAction::Rename, Some("renamed")), CancellationToken::new()).await.unwrap();
    assert!(!f.code().join("copy").exists());
    let renamed = current(&project, "renamed");
    let deleted = manager.mutate_development_entry("alice", input(&project, "renamed", &renamed, MutationAction::Delete, None), CancellationToken::new()).await.unwrap();
    assert_eq!(deleted.path, "renamed"); assert_eq!(deleted.kind, EntryKind::Directory);
    assert!(!f.code().join("renamed").exists()); assert!(f.code().join("tree/nested/data.bin").exists());
    assert!(manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn directory_copy_preserves_private_parent_and_nested_source_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new(); let (manager, project) = f.manager().await;
    std::fs::create_dir_all(f.code().join("tree/nested")).unwrap();
    std::fs::write(f.code().join("tree/nested/data"), "private tree, readable file").unwrap();
    for (path, mode) in [("tree", 0o700), ("tree/nested", 0o755), ("tree/nested/data", 0o644)] {
        std::fs::set_permissions(f.code().join(path), std::fs::Permissions::from_mode(mode)).unwrap();
    }
    let version = current(&project, "tree");
    manager.mutate_development_entry("alice", input(&project, "tree", &version, MutationAction::Copy, Some("copy")), CancellationToken::new()).await.unwrap();
    for (path, mode) in [("copy", 0o700), ("copy/nested", 0o755), ("copy/nested/data", 0o644)] {
        assert_eq!(std::fs::metadata(f.code().join(path)).unwrap().permissions().mode() & 0o777, mode, "{path}");
    }
    assert_eq!(std::fs::read_to_string(f.code().join("copy/nested/data")).unwrap(), "private tree, readable file");
    assert_eq!(current(&project, "tree"), version);
    assert!(manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn directory_copy_keeps_partial_contents_private_and_reports_unknown_without_rollback() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new(); let (manager, project) = f.manager().await;
    std::fs::create_dir(f.code().join("tree")).unwrap();
    std::fs::set_permissions(f.code().join("tree"), std::fs::Permissions::from_mode(0o755)).unwrap();
    for name in ["a", "b"] {
        let path = f.code().join("tree").join(name);
        std::fs::write(&path, name).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    let lease = CancellationToken::new(); let mut budget = Budget::new(&lease);
    let (parent, mut node) = snapshot(&project, "tree", &mut budget).unwrap();
    // Force a deterministic failure in the second file's content check, after
    // the first file has regained its public mode. No global umask changes.
    if let Body::Directory(_, children) = &mut node.body {
        assert_eq!(children[1].name, "b");
        children[1].digest[0] ^= 1;
    } else { panic!("expected directory"); }
    let mut changed = false;
    let result = copy_node(&project, &parent, &mut node, &parent, "copy", "copy", &budget, &mut changed)
        .map(|()| CreatedEntry { path: "copy".into(), kind: EntryKind::Directory });
    assert!(changed);
    assert_eq!(mutation_result(result, changed).unwrap_err(), "file_mutation_unknown");
    assert_eq!(std::fs::metadata(f.code().join("copy")).unwrap().permissions().mode() & 0o077, 0);
    assert_eq!(std::fs::metadata(f.code().join("copy/a")).unwrap().permissions().mode() & 0o777, 0o644);
    assert_eq!(std::fs::read_to_string(f.code().join("copy/a")).unwrap(), "a");
    assert_eq!(std::fs::read_to_string(f.code().join("copy/b")).unwrap(), "b");
    assert_eq!(std::fs::read_to_string(f.code().join("tree/b")).unwrap(), "b");
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn stale_recursive_snapshot_and_unsafe_descendants_reject_before_side_effects() {
    let f = Fixture::new(); let (manager, project) = f.manager().await;
    std::fs::create_dir(f.code().join("tree")).unwrap();
    std::fs::write(f.code().join("tree/code"), "before").unwrap();
    let before = current(&project, "tree");
    std::fs::write(f.code().join("tree/code"), "after").unwrap();
    for (action, destination) in [(MutationAction::Copy, Some("copy")), (MutationAction::Rename, Some("rename")), (MutationAction::Delete, None)] {
        assert_eq!(manager.mutate_development_entry("alice", input(&project, "tree", &before, action, destination), CancellationToken::new()).await.unwrap_err(), "file_changed");
    }
    assert!(!f.code().join("copy").exists()); assert!(!f.code().join("rename").exists());
    let version = current(&project, "tree");
    for path in ["../escape", "/tmp/escape", ".git/config", "tree/private.key", "tree/.env"] {
        assert!(manager.mutate_development_entry("alice", input(&project, "tree", &version, MutationAction::Copy, Some(path)), CancellationToken::new()).await.is_err());
    }
    assert!(manager.mutate_development_entry("alice", input(&project, "tree", &version, MutationAction::Copy, Some("tree/child")), CancellationToken::new()).await.is_err());
    std::fs::write(f.code().join("tree/.env"), "secret").unwrap();
    assert_eq!(manager.inspect_development_entry("alice", inspect(&project, "tree"), CancellationToken::new()).await.unwrap_err(), "file_sensitive");
    assert!(manager.mutate_development_entry("alice", input(&project, "tree", &version, MutationAction::Delete, None), CancellationToken::new()).await.is_err());
    assert_eq!(std::fs::read_to_string(f.code().join("tree/code")).unwrap(), "after");
    assert!(manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn links_special_entries_git_and_replaced_roots_are_rejected() {
    use std::{os::unix::fs::symlink, ffi::CString, os::unix::ffi::OsStrExt};
    let f = Fixture::new(); let (manager, project) = f.manager().await;
    std::fs::create_dir(f.0.join("outside")).unwrap(); std::fs::create_dir(f.code().join("tree")).unwrap();
    std::fs::write(f.0.join("outside/file"), "external").unwrap();
    symlink(f.0.join("outside"), f.code().join("link")).unwrap();
    symlink(f.0.join("outside/file"), f.code().join("tree/link")).unwrap();
    for path in ["link/file", "tree", "tree/link"] { assert!(manager.inspect_development_entry("alice", inspect(&project, path), CancellationToken::new()).await.is_err()); }
    std::fs::remove_file(f.code().join("tree/link")).unwrap();
    std::fs::hard_link(f.0.join("outside/file"), f.code().join("tree/hard")).unwrap();
    assert!(manager.inspect_development_entry("alice", inspect(&project, "tree"), CancellationToken::new()).await.is_err());
    std::fs::remove_file(f.code().join("tree/hard")).unwrap();
    let fifo = CString::new(f.code().join("tree/pipe").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(manager.inspect_development_entry("alice", inspect(&project, "tree"), CancellationToken::new()).await.is_err());
    std::fs::remove_file(f.code().join("tree/pipe")).unwrap(); std::fs::create_dir(f.code().join("tree/.git")).unwrap();
    assert!(manager.inspect_development_entry("alice", inspect(&project, "tree"), CancellationToken::new()).await.is_err());
    std::fs::rename(f.code(), f.0.join("original")).unwrap(); std::fs::create_dir(f.code()).unwrap();
    assert_eq!(manager.inspect_development_entry("alice", inspect(&project, "tree"), CancellationToken::new()).await.unwrap_err(), "workspace_changed");
    assert_eq!(std::fs::read_to_string(f.0.join("outside/file")).unwrap(), "external");
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn owner_task_trust_archive_and_cancelled_admission_guards() {
    let f = Fixture::new(); let (manager, project) = f.manager().await;
    std::fs::write(f.code().join("code"), "keep").unwrap(); let version = current(&project, "code");
    assert!(manager.inspect_development_entry("bob", inspect(&project, "code"), CancellationToken::new()).await.is_err());
    assert!(manager.mutate_development_entry("bob", input(&project, "code", &version, MutationAction::Delete, None), CancellationToken::new()).await.is_err());
    let mut untrusted = input(&project, "code", &version, MutationAction::Delete, None); untrusted.trust_directory = false;
    assert_eq!(manager.mutate_development_entry("alice", untrusted, CancellationToken::new()).await.unwrap_err(), "workspace_untrusted");
    let task = manager.save_development_task("alice", SaveTaskInput { id: None, project_id: project.id.clone(), goal: "Edit".into(), requirements: String::new(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
    let mut wrong = input(&project, "code", &version, MutationAction::Delete, None); wrong.project_id = uuid::Uuid::new_v4().to_string(); wrong.task_id = Some(task.id);
    assert_eq!(manager.mutate_development_entry("alice", wrong, CancellationToken::new()).await.unwrap_err(), "task_not_found");
    let held = manager.admit_native_extensions(&CancellationToken::new()).await.unwrap();
    let lease = CancellationToken::new();
    let pending = manager.mutate_development_entry("alice", input(&project, "code", &version, MutationAction::Delete, None), lease.clone());
    tokio::pin!(pending);
    assert!(tokio::time::timeout(Duration::from_millis(10), &mut pending).await.is_err());
    lease.cancel();
    assert_eq!(pending.await.unwrap_err(), "session_invalid"); drop(held);
    assert_eq!(std::fs::read_to_string(f.code().join("code")).unwrap(), "keep");
    manager.update_project("alice", UpdateProjectInput { project_id: project.id.clone(), action: "archive".into(), value: None }, CancellationToken::new()).await.unwrap();
    assert_eq!(manager.mutate_development_entry("alice", input(&project, "code", &version, MutationAction::Delete, None), CancellationToken::new()).await.unwrap_err(), "project_archived");
    assert_eq!(manager.inspect_development_entry("alice", inspect(&project, "code"), CancellationToken::new()).await.unwrap_err(), "project_archived");
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn directory_opener_admission_resolves_owned_folder_or_file_parent() {
    let f = Fixture::new(); let (manager, project) = f.manager().await;
    std::fs::create_dir(f.code().join("folder")).unwrap(); std::fs::write(f.code().join("folder/code"), "x").unwrap();
    let lease = CancellationToken::new();
    for path in ["folder", "folder/code"] {
        let held = manager.admit_development_entry_directory("alice", inspect(&project, path), &lease).await.unwrap();
        assert_eq!(Path::new(&held.path), f.code().join("folder")); held.validate(&lease).unwrap();
    }
    let held = manager.admit_development_entry_directory("alice", inspect(&project, "folder/code"), &lease).await.unwrap();
    std::fs::rename(f.code().join("folder"), f.code().join("moved")).unwrap(); std::fs::create_dir(f.code().join("folder")).unwrap();
    assert!(held.validate(&lease).is_err()); drop(held);
    assert!(manager.shutdown().await);
}

#[test]
fn recursive_delete_preserves_external_additions_and_reports_unknown_after_partial_effects() {
    let f = Fixture::new();
    std::fs::create_dir(f.code().join("tree")).unwrap(); std::fs::write(f.code().join("tree/a"), "a").unwrap();
    #[cfg(unix)] let identity = { use std::os::unix::fs::MetadataExt; let m = std::fs::metadata(f.code()).unwrap(); format!("{}:{}", m.dev(), m.ino()) };
    #[cfg(not(unix))] let identity = String::new();
    let project = Project { id: String::new(), owner: "alice".into(), name: "x".into(), path: f.code().to_str().unwrap().into(), directory_identity: identity, archived: false, created_at: 0, last_opened_at: 0 };
    let lease = CancellationToken::new(); let mut budget = Budget::new(&lease);
    let (parent, mut node) = snapshot(&project, "tree", &mut budget).unwrap();
    // Simulate an external addition after the preflight and first successful unlink.
    if let Body::Directory(dir, children) = &mut node.body {
        dir.remove_file("a").unwrap(); children.clear();
        std::fs::write(f.code().join("tree/external"), "user").unwrap();
        node.metadata = parent.symlink_metadata("tree").unwrap();
    }
    let mut changed = true;
    let result = delete_node(&project, &parent, &mut node, &budget, &mut changed).map(|()| CreatedEntry { path: "tree".into(), kind: EntryKind::Directory });
    assert_eq!(mutation_result(result, changed).unwrap_err(), "file_mutation_unknown");
    assert_eq!(std::fs::read_to_string(f.code().join("tree/external")).unwrap(), "user");
}

#[test]
fn cancelled_partial_mutation_is_unknown_and_exclusive_rename_never_replaces() {
    let f = Fixture::new();
    std::fs::write(f.code().join("source"), "source").unwrap();
    std::fs::write(f.code().join("destination"), "user work").unwrap();
    let root = Dir::open_ambient_dir(f.code(), ambient_authority()).unwrap();
    assert_eq!(rename_exclusive(&root, "source", &root, "destination").unwrap_err(), "file_exists");
    assert_eq!(std::fs::read_to_string(f.code().join("destination")).unwrap(), "user work");
    assert_eq!(std::fs::read_to_string(f.code().join("source")).unwrap(), "source");
    let lease = CancellationToken::new(); let budget = Budget::new(&lease); lease.cancel();
    let before = mutation_result(budget.check().map(|()| CreatedEntry { path: "source".into(), kind: EntryKind::File }), false);
    assert_eq!(before.unwrap_err(), "session_invalid");
    let after = mutation_result(budget.check().map(|()| CreatedEntry { path: "source".into(), kind: EntryKind::File }), true);
    assert_eq!(after.unwrap_err(), "file_mutation_unknown");
    assert!(f.code().join("source").exists());
}

#[tokio::test]
async fn task_workspace_binding_keeps_mutations_and_reveal_out_of_original_directory() {
    let f = Fixture::new(); let (manager, project) = f.manager().await;
    std::fs::write(f.code().join("code"), "original").unwrap();
    let isolated = f.0.join("isolated"); std::fs::create_dir(&isolated).unwrap();
    std::fs::write(isolated.join("code"), "isolated").unwrap();
    let task = manager.save_development_task("alice", SaveTaskInput { id: None, project_id: project.id.clone(), goal: "Isolated".into(), requirements: String::new(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
    let (path, directory_identity) = crate::projects::directory(isolated.to_str().unwrap()).await.unwrap();
    let workspace = dgos_db::development_repository::StoredWorkspace { id: uuid::Uuid::new_v4().to_string(), owner: "alice".into(), project_id: project.id.clone(), path, directory_identity, common_dir: None, common_identity: None, kind: "imported".into(), created_operation: None, base_commit: None };
    manager.inner.repository.insert_workspace(&workspace).await.unwrap();
    manager.inner.repository.bind_task_workspace("alice", &task.id, &workspace.id).await.unwrap();
    let mut request = inspect(&project, "code"); request.task_id = Some(task.id.clone());
    let snapshot = manager.inspect_development_entry("alice", request, CancellationToken::new()).await.unwrap();
    let mut request = input(&project, "code", &snapshot.version, MutationAction::Copy, Some("copied")); request.task_id = Some(task.id.clone());
    manager.mutate_development_entry("alice", request, CancellationToken::new()).await.unwrap();
    assert_eq!(std::fs::read_to_string(isolated.join("copied")).unwrap(), "isolated");
    assert!(!f.code().join("copied").exists());
    let mut request = inspect(&project, "copied"); request.task_id = Some(task.id.clone());
    let admission = manager.admit_development_entry_directory("alice", request, &CancellationToken::new()).await.unwrap();
    assert_eq!(Path::new(&admission.path), isolated); drop(admission);
    assert_eq!(std::fs::read_to_string(f.code().join("code")).unwrap(), "original");
    assert!(manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn active_host_process_blocks_tree_mutations_until_stopped() {
    let f = Fixture::new(); let (manager, project) = f.manager().await;
    std::fs::write(f.code().join("code"), "keep").unwrap();
    let version = current(&project, "code");
    let task = manager.save_development_task("alice", SaveTaskInput { id: None, project_id: project.id.clone(), goal: "Busy".into(), requirements: String::new(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    manager.start_development_process("alice", crate::runtime::StartProcessInput { id: id.clone(), task_id: task.id, kind: "command".into(), program: "/bin/sh".into(), args: vec!["-c".into(), "sleep 30".into()], cwd: String::new(), env_refs: Default::default(), trust_workspace: true, verification: false, readiness: None, native_run_id: None }, CancellationToken::new()).await.unwrap();
    assert_eq!(manager.mutate_development_entry("alice", input(&project, "code", &version, MutationAction::Delete, None), CancellationToken::new()).await.unwrap_err(), "workspace_busy");
    assert_eq!(std::fs::read_to_string(f.code().join("code")).unwrap(), "keep");
    manager.stop_development_process("alice", &id).await.unwrap();
    assert!(manager.shutdown().await);
}
