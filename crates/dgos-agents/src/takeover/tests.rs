//! Controlled native protocol fixture; no installed CLI compatibility claim.
use super::*;
use crate::{Engine, SessionOptions, files::SaveFileInput, projects::{CreateProjectInput, SaveTaskInput}};
use dgos_db::development_repository::StoredWorkspace;
use std::path::{Path, PathBuf};

fn git(path: &Path, args: &[&str]) {
    let output = std::process::Command::new("git").current_dir(path).args(args).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}
struct Fixture { root: PathBuf, manager: AgentManager }
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-takeover-{}", uuid::Uuid::new_v4()));
        for name in ["original", "worktree", "other"] { std::fs::create_dir_all(root.join(name)).unwrap(); }
        let root = root.canonicalize().unwrap();
        crate::test_support::codex_fixture(&root.join("codex"), &r##"
while IFS= read -r line; do
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"thread/start"'*|*'"method":"thread/resume"'*) printf '{"id":3,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"fixture-thread","turns":[]}}}\n' "$PWD";;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
        let manager = crate::test_support::manager(&root).await;
        Self { root, manager }
    }
    async fn task(&self, owner: &str, directory: &str) -> ReadTakeoverInput {
        let project = self.manager.create_project(owner, CreateProjectInput { kind: "import".into(), name: directory.into(), path: self.root.join(directory).to_string_lossy().into(), source: None }, CancellationToken::new()).await.unwrap();
        let task = self.manager.save_development_task(owner, SaveTaskInput { id: None, project_id: project.id.clone(), goal: "take over".into(), requirements: "Close only confirmed sessions in the exact task directory".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
        ReadTakeoverInput { project_id: project.id, task_id: task.id }
    }
    async fn open(&self, owner: &str, scope: &ReadTakeoverInput, id: &str) {
        let options = SessionOptions { workspace: self.root.to_string_lossy().into(), resume_id: None, executable: Some(self.root.join("codex").to_string_lossy().into()), model: None, trust_workspace: true };
        let create = crate::test_support::create(&self.manager, owner, &scope.task_id, options).await;
        self.manager.open_session(owner, Engine::Codex, id.into(), create, Some(scope.task_id.clone()), CancellationToken::new()).await.unwrap();
    }
    async fn view(&self, owner: &str, scope: &ReadTakeoverInput) -> TakeoverView {
        self.manager.read_development_takeover(owner, scope.clone(), CancellationToken::new()).await.unwrap()
    }
    async fn close(&self, owner: &str, scope: &ReadTakeoverInput, token: String) -> Result<TakeoverView, String> {
        self.manager.close_development_takeover(owner, CloseTakeoverInput { project_id: scope.project_id.clone(), task_id: scope.task_id.clone(), token, confirmed: true }, CancellationToken::new()).await
    }
    async fn bind_directory(&self, scope: &ReadTakeoverInput, path: &Path) {
        let (path, identity) = crate::projects::directory(path.to_str().unwrap()).await.unwrap();
        let git = crate::workspaces::git_identity(&path, &CancellationToken::new(), &self.manager.inner.stop).await.ok();
        let (common_dir, common_identity) = match git { Some((path, identity)) => (Some(path), Some(identity)), None => (None, None) };
        let workspace = StoredWorkspace { id: uuid::Uuid::new_v4().to_string(), owner: "alice".into(), project_id: scope.project_id.clone(), path, directory_identity: identity, common_dir, common_identity, kind: "imported".into(), created_operation: None, base_commit: None };
        self.manager.inner.repository.insert_workspace(&workspace).await.unwrap();
        self.manager.inner.repository.bind_task_workspace("alice", &scope.task_id, &workspace.id).await.unwrap();
    }
}
impl Drop for Fixture { fn drop(&mut self) { self.manager.close(); let _ = std::fs::remove_dir_all(&self.root); } }

#[tokio::test]
async fn closes_only_actual_task_directory_preserving_other_directory_and_account() {
    let f = Fixture::new().await;
    let source = f.root.join("original");
    git(&source, &["init", "-q"]);
    git(&source, &["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-qm", "base"]);
    std::fs::remove_dir(f.root.join("worktree")).unwrap();
    git(&source, &["worktree", "add", "-q", "-b", "takeover-isolated", f.root.join("worktree").to_str().unwrap()]);
    let original = f.task("alice", "original").await;
    let target_task = f.manager.save_development_task("alice", SaveTaskInput { id: None, project_id: original.project_id.clone(), goal: "isolated task".into(), requirements: "Preserve the original directory session".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
    let target = ReadTakeoverInput { project_id: original.project_id.clone(), task_id: target_task.id };
    f.bind_directory(&target, &f.root.join("worktree")).await;
    let foreign = f.task("bob", "other").await;
    let ids: Vec<_> = (0..3).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    f.open("alice", &original, &ids[0]).await;
    f.open("alice", &target, &ids[1]).await;
    f.open("bob", &foreign, &ids[2]).await;
    let view = f.view("alice", &target).await;
    assert_eq!(view.directory, f.root.join("worktree").to_string_lossy());
    assert_eq!(view.sessions.len(), 1); assert_eq!(view.sessions[0].id, ids[1]);
    let after = f.close("alice", &target, view.token).await.unwrap();
    assert!(after.sessions.is_empty());
    assert_eq!(f.manager.snapshot("alice", &ids[1]).await.unwrap().view.status, "closed");
    assert_ne!(f.manager.snapshot("alice", &ids[0]).await.unwrap().view.status, "closed");
    assert_ne!(f.manager.snapshot("bob", &ids[2]).await.unwrap().view.status, "closed");
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn foreign_exact_directory_is_redacted_and_never_closed() {
    let f = Fixture::new().await;
    let scope = f.task("alice", "original").await;
    let foreign = f.task("bob", "original").await;
    let id = uuid::Uuid::new_v4().to_string(); f.open("bob", &foreign, &id).await;
    let view = f.view("alice", &scope).await;
    assert!(view.restricted); assert!(!view.can_close); assert!(view.sessions.is_empty());
    let encoded = serde_json::to_string(&view).unwrap(); assert!(!encoded.contains(&id)); assert!(!encoded.contains(&foreign.task_id)); assert!(!encoded.contains("bob"));
    assert_eq!(f.close("alice", &scope, view.token).await.unwrap_err(), "workspace_busy");
    assert_ne!(f.manager.snapshot("bob", &id).await.unwrap().view.status, "closed");
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn revoked_epoch_reopened_generation_and_replaced_directory_reject_old_confirmation() {
    let f = Fixture::new().await; let scope = f.task("alice", "original").await;
    let id = uuid::Uuid::new_v4().to_string(); f.open("alice", &scope, &id).await;
    let view = f.view("alice", &scope).await;
    f.manager.inner.providers.revoke("alice", &scope.project_id).await.unwrap();
    assert_eq!(f.close("alice", &scope, view.token).await.unwrap_err(), "takeover_stale");
    let view = f.view("alice", &scope).await;
    assert!(f.manager.close_session("alice", &id).await.unwrap());
    let id = uuid::Uuid::new_v4().to_string();
    f.open("alice", &scope, &id).await;
    assert_eq!(f.close("alice", &scope, view.token).await.unwrap_err(), "takeover_stale");
    let view = f.view("alice", &scope).await;
    std::fs::rename(f.root.join("original"), f.root.join("previous-directory")).unwrap();
    std::fs::create_dir(f.root.join("original")).unwrap();
    assert_eq!(f.close("alice", &scope, view.token).await.unwrap_err(), "workspace_changed");
    assert_ne!(f.manager.snapshot("alice", &id).await.unwrap().view.status, "closed");
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn cancelled_waiting_admission_and_close_failure_never_release_write_guard() {
    let f = Fixture::new().await; let scope = f.task("alice", "original").await;
    let id = uuid::Uuid::new_v4().to_string(); f.open("alice", &scope, &id).await;
    let view = f.view("alice", &scope).await;
    let admission = f.manager.inner.admission.lock().await;
    let manager = f.manager.clone(); let lease = CancellationToken::new(); let pending_lease = lease.clone();
    let input = CloseTakeoverInput { project_id: scope.project_id.clone(), task_id: scope.task_id.clone(), token: view.token, confirmed: true };
    let pending = tokio::spawn(async move { manager.close_development_takeover("alice", input, pending_lease).await });
    lease.cancel(); drop(admission);
    assert_eq!(pending.await.unwrap().unwrap_err(), "session_invalid");
    let view = f.view("alice", &scope).await;
    f.manager.inner.fail_takeover_close.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(f.close("alice", &scope, view.token).await.unwrap_err(), "cleanup_pending");
    assert_eq!(f.manager.workspace_available(&f.root.join("original").to_string_lossy(), None).await.unwrap_err(), "workspace_busy");
    let error = f.manager.save_development_file("alice", SaveFileInput { project_id: scope.project_id.clone(), task_id: scope.task_id.clone(), path: "draft.txt".into(), expected_hash: String::new(), text: "kept draft".into() }, CancellationToken::new()).await.unwrap_err();
    assert_eq!(error, "workspace_busy"); assert!(!f.root.join("original/draft.txt").exists());
    assert_ne!(f.manager.snapshot("alice", &id).await.unwrap().view.status, "closed");
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn host_occupancy_is_preserved_and_unconfirmed_close_is_denied() {
    let f = Fixture::new().await;
    let scope = f.task("alice", "original").await;
    let process = f.manager.start_development_process("alice", crate::runtime::StartProcessInput {
        id: uuid::Uuid::new_v4().to_string(), task_id: scope.task_id.clone(), kind: "command".into(),
        program: "/bin/sh".into(), args: vec!["-c".into(), "sleep 30".into()], cwd: String::new(),
        env_refs: Default::default(), trust_workspace: true, verification: false, readiness: None, native_run_id: None,
    }, CancellationToken::new()).await.unwrap();
    let view = f.view("alice", &scope).await;
    assert!(view.host_busy); assert!(!view.can_close); assert!(view.sessions.is_empty());
    assert_eq!(f.close("alice", &scope, view.token).await.unwrap_err(), "workspace_busy");
    assert!(f.manager.inner.host.lock().await.contains_key(&process.id));
    f.manager.stop_development_process("alice", &process.id).await.unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    f.open("alice", &scope, &id).await;
    let view = f.view("alice", &scope).await;
    let error = f.manager.close_development_takeover("alice", CloseTakeoverInput {
        project_id: scope.project_id, task_id: scope.task_id, token: view.token, confirmed: false,
    }, CancellationToken::new()).await.unwrap_err();
    assert_eq!(error, "takeover_confirmation_required");
    assert_ne!(f.manager.snapshot("alice", &id).await.unwrap().view.status, "closed");
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn same_directory_session_from_another_owned_task_can_be_closed() {
    let f = Fixture::new().await;
    let scope = f.task("alice", "original").await;
    let other = f.manager.save_development_task("alice", SaveTaskInput {
        id: None, project_id: scope.project_id.clone(), goal: "other task".into(), requirements: "Close the related directory session explicitly".into(), status: "todo".into(),
    }, CancellationToken::new()).await.unwrap();
    let other_scope = ReadTakeoverInput { project_id: scope.project_id.clone(), task_id: other.id.clone() };
    let id = uuid::Uuid::new_v4().to_string();
    f.open("alice", &other_scope, &id).await;
    assert!(f.manager.read_development_takeover("bob", scope.clone(), CancellationToken::new()).await.is_err());
    let view = f.view("alice", &scope).await;
    assert!(view.can_close); assert_eq!(view.sessions[0].task_id.as_deref(), Some(other.id.as_str()));
    assert!(f.close("alice", &scope, view.token).await.unwrap().sessions.is_empty());
    assert_eq!(f.manager.snapshot("alice", &id).await.unwrap().view.status, "closed");
    assert!(f.manager.shutdown().await);
}
