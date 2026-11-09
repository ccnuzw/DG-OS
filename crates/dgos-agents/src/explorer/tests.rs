use super::*;
use crate::{files::{CreateEntryInput, EntryKind}, git::{ProjectDiffInput, ProjectDiffSource, ProjectGitHistoryInput}, projects::{CreateProjectInput, UpdateProjectInput}};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-explorer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("code")).unwrap(); Self(root)
    }
    fn code(&self) -> PathBuf { self.0.join("code") }
    async fn manager_project(&self) -> (AgentManager, String) {
        let manager = crate::test_support::manager(&self.0).await;
        let project = manager.create_project("alice", CreateProjectInput { kind: "import".into(), name: "Explorer".into(), path: self.code().to_string_lossy().into(), source: None }, CancellationToken::new()).await.unwrap();
        (manager, project.id)
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn entry(project: &str, path: &str, kind: EntryKind) -> CreateEntryInput {
    CreateEntryInput { project_id: project.into(), task_id: None, path: path.into(), kind, trust_directory: true }
}
fn git(path: &Path, args: &[&str]) -> String {
    let result = std::process::Command::new("git").current_dir(path)
        .env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", "-c", "init.templateDir=", "-c", "commit.gpgSign=false"])
        .args(args).output().unwrap();
    assert!(result.status.success(), "fixture Git failed: {}", String::from_utf8_lossy(&result.stderr));
    String::from_utf8(result.stdout).unwrap().trim().into()
}

#[tokio::test]
async fn exclusive_creation_is_taskless_owned_trusted_nonrecursive_and_archived_guarded() {
    let fixture = Fixture::new(); let (manager, project) = fixture.manager_project().await;
    let mut untrusted = entry(&project, "denied", EntryKind::File); untrusted.trust_directory = false;
    assert_eq!(manager.create_development_entry("alice", untrusted, CancellationToken::new()).await.unwrap_err(), "workspace_untrusted");
    manager.create_development_entry("alice", entry(&project, "src", EntryKind::Directory), CancellationToken::new()).await.unwrap();
    manager.create_development_entry("alice", entry(&project, "src/new.txt", EntryKind::File), CancellationToken::new()).await.unwrap();
    std::fs::write(fixture.code().join("src/new.txt"), "user work").unwrap();
    for kind in [EntryKind::File, EntryKind::Directory] {
        assert_eq!(manager.create_development_entry("alice", entry(&project, "src/new.txt", kind), CancellationToken::new()).await.unwrap_err(), "file_exists");
    }
    assert_eq!(std::fs::read_to_string(fixture.code().join("src/new.txt")).unwrap(), "user work");
    assert!(manager.create_development_entry("alice", entry(&project, "missing/child", EntryKind::File), CancellationToken::new()).await.is_err());
    assert!(!fixture.code().join("missing").exists());
    for path in ["../escape", ".git/config", ".env", "src/private.key"] {
        assert!(manager.create_development_entry("alice", entry(&project, path, EntryKind::File), CancellationToken::new()).await.is_err());
    }
    assert!(manager.create_development_entry("bob", entry(&project, "foreign", EntryKind::File), CancellationToken::new()).await.is_err());
    let cancelled = CancellationToken::new(); cancelled.cancel();
    assert_eq!(manager.create_development_entry("alice", entry(&project, "cancelled", EntryKind::File), cancelled).await.unwrap_err(), "session_invalid");
    assert!(!fixture.code().join("cancelled").exists());
    assert!(manager.inner.repository.workspace("alice").await.unwrap().tasks.is_empty());
    manager.update_project("alice", UpdateProjectInput { project_id: project.clone(), action: "archive".into(), value: None }, CancellationToken::new()).await.unwrap();
    assert_eq!(manager.create_development_entry("alice", entry(&project, "archived", EntryKind::File), CancellationToken::new()).await.unwrap_err(), "project_archived");
    assert!(manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn creation_rejects_symlink_ancestors_and_replaced_project_roots() {
    let fixture = Fixture::new(); let (manager, project) = fixture.manager_project().await;
    std::fs::create_dir(fixture.0.join("outside")).unwrap();
    std::os::unix::fs::symlink(fixture.0.join("outside"), fixture.code().join("link")).unwrap();
    assert!(manager.create_development_entry("alice", entry(&project, "link/escape", EntryKind::File), CancellationToken::new()).await.is_err());
    assert!(!fixture.0.join("outside/escape").exists());
    std::fs::rename(fixture.code(), fixture.0.join("original")).unwrap(); std::fs::create_dir(fixture.code()).unwrap();
    assert_eq!(manager.create_development_entry("alice", entry(&project, "replaced", EntryKind::File), CancellationToken::new()).await.unwrap_err(), "workspace_changed");
    assert!(!fixture.code().join("replaced").exists()); assert!(manager.shutdown().await);
}

#[tokio::test]
async fn project_git_reads_preserve_head_index_worktree_and_create_no_tasks_or_tokens() {
    let fixture = Fixture::new(); let code = fixture.code();
    git(&code, &["init", "-b", "main"]); git(&code, &["config", "user.name", "Explorer fixture"]); git(&code, &["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(code.join("literal[1].txt"), "base\n").unwrap(); git(&code, &["add", "."]); git(&code, &["commit", "-m", "base"]);
    git(&code, &["commit", "--allow-empty", "-m", "second"]);
    std::fs::write(code.join("literal[1].txt"), "staged\n").unwrap(); git(&code, &["add", "."]);
    std::fs::write(code.join("literal[1].txt"), "worktree\n").unwrap();
    let (manager, project) = fixture.manager_project().await;
    let before_head = git(&code, &["rev-parse", "HEAD"]); let before_index = std::fs::read(code.join(".git/index")).unwrap();
    let overview = manager.read_development_project_git("alice", &project).await.unwrap();
    assert_eq!(overview.branch.as_deref(), Some("main")); assert!(overview.files.iter().any(|file| file.path == "literal[1].txt"));
    let serialized = serde_json::to_value(&overview).unwrap(); assert!(serialized.get("review_token").is_none() && serialized.get("task_id").is_none());
    for (source, before, after) in [(ProjectDiffSource::Index, "base\n", "staged\n"), (ProjectDiffSource::Worktree, "staged\n", "worktree\n")] {
        let diff = manager.read_development_project_diff("alice", ProjectDiffInput { project_id: project.clone(), path: "literal[1].txt".into(), source }).await.unwrap();
        assert_eq!(diff.before.as_deref(), Some(before)); assert_eq!(diff.after.as_deref(), Some(after));
    }
    let first = manager.read_development_project_git_history("alice", ProjectGitHistoryInput { project_id: project.clone(), start_oid: None, cursor: None, limit: 1 }).await.unwrap();
    assert_eq!(first.entries[0].subject, "second");
    let next = manager.read_development_project_git_history("alice", ProjectGitHistoryInput { project_id: project.clone(), start_oid: Some(first.start_oid), cursor: first.next_cursor, limit: 1 }).await.unwrap();
    assert_eq!(next.entries[0].subject, "base");
    assert!(manager.read_development_project_git("bob", &project).await.is_err());
    assert_eq!(git(&code, &["rev-parse", "HEAD"]), before_head); assert_eq!(std::fs::read(code.join(".git/index")).unwrap(), before_index);
    assert_eq!(std::fs::read_to_string(code.join("literal[1].txt")).unwrap(), "worktree\n");
    assert!(manager.inner.repository.workspace("alice").await.unwrap().tasks.is_empty()); assert!(manager.shutdown().await);
}

#[tokio::test]
async fn project_git_overview_and_history_remain_available_with_blocked_untracked_files() {
    let fixture = Fixture::new(); let code = fixture.code();
    git(&code, &["init", "-b", "main"]); git(&code, &["config", "user.name", "Explorer fixture"]); git(&code, &["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(code.join("source.txt"), "base\n").unwrap(); git(&code, &["add", "."]); git(&code, &["commit", "-m", "base"]);
    std::fs::write(code.join("source.txt"), "staged\n").unwrap(); git(&code, &["add", "source.txt"]);
    std::fs::write(code.join("source.txt"), "worktree\n").unwrap();
    std::fs::write(code.join(".env"), "SECRET=must-not-preview\n").unwrap();
    std::fs::write(code.join("binary.dat"), [0, 1, 2]).unwrap();
    std::fs::write(code.join("large.txt"), vec![b'a'; crate::files::FILE_LIMIT + 1]).unwrap();
    #[cfg(unix)] {
        std::fs::write(fixture.0.join("outside.txt"), "outside-must-not-preview\n").unwrap();
        std::os::unix::fs::symlink(fixture.0.join("outside.txt"), code.join("link.txt")).unwrap();
    }
    let (manager, project) = fixture.manager_project().await;
    let before_head = git(&code, &["rev-parse", "HEAD"]); let before_index = std::fs::read(code.join(".git/index")).unwrap();
    let overview = manager.read_development_project_git("alice", &project).await.unwrap();
    assert_eq!(overview.kind, "git"); assert_eq!(overview.branch.as_deref(), Some("main")); assert_eq!(overview.head.as_deref(), Some(before_head.as_str()));
    assert!(overview.staged_diff.contains("+staged")); assert!(overview.worktree_diff.contains("+worktree"));
    let mut blocked = vec![".env", "binary.dat", "large.txt"];
    #[cfg(unix)] blocked.push("link.txt");
    for path in &blocked {
        let status = overview.files.iter().find(|file| file.path == *path).unwrap();
        assert_eq!((&*status.index, &*status.worktree), ("?", "?"));
        assert!(manager.read_development_project_diff("alice", ProjectDiffInput { project_id: project.clone(), path: (*path).into(), source: ProjectDiffSource::Worktree }).await.is_err());
    }
    let serialized = serde_json::to_value(&overview).unwrap();
    for field in ["task_id", "review_token", "file_versions", "baseline_version"] { assert!(serialized.get(field).is_none()); }
    let serialized = serialized.to_string(); assert!(!serialized.contains("must-not-preview"));
    let history = manager.read_development_project_git_history("alice", ProjectGitHistoryInput { project_id: project, start_oid: None, cursor: None, limit: 10 }).await.unwrap();
    assert_eq!(history.entries.len(), 1); assert_eq!(history.entries[0].oid, before_head); assert_eq!(history.entries[0].subject, "base");
    assert_eq!(git(&code, &["rev-parse", "HEAD"]), before_head); assert_eq!(std::fs::read(code.join(".git/index")).unwrap(), before_index);
    assert_eq!(std::fs::read_to_string(code.join("source.txt")).unwrap(), "worktree\n");
    assert_eq!(std::fs::read_to_string(code.join(".env")).unwrap(), "SECRET=must-not-preview\n");
    assert_eq!(std::fs::read(code.join("binary.dat")).unwrap(), [0, 1, 2]);
    assert_eq!(std::fs::metadata(code.join("large.txt")).unwrap().len(), crate::files::FILE_LIMIT as u64 + 1);
    #[cfg(unix)] assert_eq!(std::fs::read_link(code.join("link.txt")).unwrap(), fixture.0.join("outside.txt"));
    assert!(manager.inner.repository.workspace("alice").await.unwrap().tasks.is_empty()); assert!(manager.shutdown().await);
}

#[tokio::test]
async fn task_git_review_still_requires_complete_versions_for_every_changed_file() {
    let fixture = Fixture::new(); let code = fixture.code(); git(&code, &["init", "-b", "main"]);
    let (manager, project) = fixture.manager_project().await;
    let task = manager.save_development_task("alice", crate::projects::SaveTaskInput { id: None, project_id: project, goal: "Strict review".into(), requirements: "Require every file version".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
    let mut blocked = vec![".env", "binary.dat", "large.txt"];
    #[cfg(unix)] blocked.push("link.txt");
    for path in blocked {
        match path {
            ".env" => std::fs::write(code.join(path), "SECRET=hidden\n").unwrap(),
            "binary.dat" => std::fs::write(code.join(path), [0, 1, 2]).unwrap(),
            "large.txt" => std::fs::write(code.join(path), vec![b'a'; crate::files::FILE_LIMIT + 1]).unwrap(),
            #[cfg(unix)] "link.txt" => std::os::unix::fs::symlink(fixture.0.join("outside.txt"), code.join(path)).unwrap(),
            _ => unreachable!(),
        }
        assert_eq!(manager.read_development_git("alice", &task.id).await.unwrap_err(), "git_review_incomplete", "{path}");
        std::fs::remove_file(code.join(path)).unwrap();
    }
    assert!(manager.inner.repository.review_baseline("alice", &task.id).await.unwrap().is_none());
    assert!(manager.inner.repository.list_git_operations("alice", &task.id).await.unwrap().is_empty());
    assert!(!manager.read_development_git("alice", &task.id).await.unwrap().review_token.is_empty());
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn project_directory_and_unborn_git_have_truthful_empty_views() {
    let fixture = Fixture::new(); let (manager, project) = fixture.manager_project().await;
    assert_eq!(manager.read_development_project_git("alice", &project).await.unwrap().kind, "directory");
    git(&fixture.code(), &["init", "-b", "main"]);
    let review = manager.read_development_project_git("alice", &project).await.unwrap(); assert!(review.head.is_none());
    let page = manager.read_development_project_git_history("alice", ProjectGitHistoryInput { project_id: project, start_oid: None, cursor: None, limit: 10 }).await.unwrap();
    assert!(page.entries.is_empty()); assert!(manager.shutdown().await);
}

#[tokio::test]
async fn project_git_ignores_task_workspace_bindings() {
    let fixture = Fixture::new(); let code = fixture.code();
    git(&code, &["init", "-b", "original"]);
    let other = fixture.0.join("isolated"); std::fs::create_dir(&other).unwrap();
    git(&other, &["init", "-b", "isolated"]);
    let (manager, project) = fixture.manager_project().await;
    let task = manager.save_development_task("alice", crate::projects::SaveTaskInput { id: None, project_id: project.clone(), goal: "Bound workspace".into(), requirements: "Keep original reads separate".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
    let (path, directory_identity) = crate::projects::directory(other.to_str().unwrap()).await.unwrap();
    let workspace = dgos_db::development_repository::StoredWorkspace { id: uuid::Uuid::new_v4().to_string(), owner: "alice".into(), project_id: project.clone(), path, directory_identity, common_dir: None, common_identity: None, kind: "imported".into(), created_operation: None, base_commit: None };
    manager.inner.repository.insert_workspace(&workspace).await.unwrap();
    manager.inner.repository.bind_task_workspace("alice", &task.id, &workspace.id).await.unwrap();
    assert_eq!(manager.read_development_git("alice", &task.id).await.unwrap().branch.as_deref(), Some("isolated"));
    assert_eq!(manager.read_development_project_git("alice", &project).await.unwrap().branch.as_deref(), Some("original"));
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn summary_service_keeps_absence_null_and_does_not_open_native_sessions() {
    let fixture = Fixture::new(); let (manager, _) = fixture.manager_project().await;
    let session = dgos_db::development_repository::StoredAgentSession { id: uuid::Uuid::new_v4().to_string(), owner: "alice".into(), engine: "codex".into(), native_id: None, workspace: fixture.code().to_string_lossy().into(), executable: None, model: None, project_id: None, task_id: None };
    manager.inner.repository.save_session(&session).await.unwrap();
    let rows = manager.read_development_session_summaries("alice").await.unwrap();
    assert_eq!(rows.len(), 1); assert_eq!(rows[0].session_id, session.id);
    assert!(rows[0].preview.is_none() && rows[0].last_activity_at.is_none());
    assert!(manager.read_development_session_summaries("bob").await.unwrap().is_empty());
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn simultaneous_creation_has_exactly_one_winner() {
    let fixture = Fixture::new(); let (manager, project) = fixture.manager_project().await;
    let first = manager.create_development_entry("alice", entry(&project, "exclusive.txt", EntryKind::File), CancellationToken::new());
    let second = manager.create_development_entry("alice", entry(&project, "exclusive.txt", EntryKind::File), CancellationToken::new());
    let (first, second) = tokio::join!(first, second);
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let error = first.err().or_else(|| second.err()).unwrap(); assert_eq!(error, "file_exists");
    assert_eq!(std::fs::metadata(fixture.code().join("exclusive.txt")).unwrap().len(), 0);
    assert!(manager.shutdown().await);
}
