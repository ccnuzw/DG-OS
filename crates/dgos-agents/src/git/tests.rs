use super::*;
use std::process::Command as StdCommand;
#[path = "pull_tests.rs"]
mod pull;
#[path = "details_restore_tests.rs"]
mod details_restore;
#[path = "commit_tests.rs"]
mod commit_details;

struct Fixture { root: PathBuf, path: PathBuf }
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-delivery-{}", uuid::Uuid::new_v4()));
        let path = root.join("code"); std::fs::create_dir_all(&path).unwrap();
        git(&path, &["init", "-b", "main"]); git(&path, &["config", "user.name", "Delivery fixture"]); git(&path, &["config", "user.email", "fixture@example.invalid"]);
        std::fs::write(path.join("code.txt"), "base\n").unwrap(); git(&path, &["add", "code.txt"]); git(&path, &["commit", "-m", "base"]);
        Self { root, path }
    }
    async fn manager_task(&self) -> (AgentManager, String) { let manager = crate::test_support::manager(&self.root).await; let task = crate::test_support::task(&manager, &self.path).await; (manager, task) }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); } }
fn git(path: &Path, args: &[&str]) -> String { let out = StdCommand::new("git").current_dir(path).args(args).output().unwrap(); assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr)); String::from_utf8(out.stdout).unwrap().trim().into() }
fn request(task: &str, review: &GitReview, action: GitAction) -> ExecuteGitInput { ExecuteGitInput { id: uuid::Uuid::new_v4().to_string(), task_id: task.into(), review_token: review.review_token.clone(), action } }
async fn execute(manager: &AgentManager, task: &str, action: GitAction) -> Value { let review = manager.read_development_git("alice", task).await.unwrap(); manager.execute_development_git("alice", request(task, &review, action), CancellationToken::new()).await.unwrap() }

#[tokio::test]
async fn explicit_stage_full_index_confirmation_receipt_replay_and_nonforce_push() {
    let fixture = Fixture::new();
    std::fs::write(fixture.path.join("user.txt"), "preexisting staged\n").unwrap(); git(&fixture.path, &["add", "user.txt"]);
    std::fs::write(fixture.path.join("code.txt"), "task change\n").unwrap();
    std::fs::write(fixture.path.join("other.txt"), "not selected\n").unwrap();
    let (manager, task) = fixture.manager_task().await;
    let before = manager.read_development_git("alice", &task).await.unwrap();
    assert!(before.files.iter().any(|f| f.path == "user.txt" && f.index == "A"));
    let intent = request(&task, &before, GitAction::Stage { paths: vec!["code.txt".into()] });
    let receipt = manager.execute_development_git("alice", intent.clone(), CancellationToken::new()).await.unwrap();
    assert_eq!(receipt["status"], "succeeded");
    assert_eq!(git(&fixture.path, &["diff", "--cached", "--name-only"]), "code.txt\nuser.txt");
    assert_eq!(manager.execute_development_git("alice", intent.clone(), CancellationToken::new()).await.unwrap(), receipt);
    let mut conflict = intent; conflict.action = GitAction::Unstage { paths: vec!["code.txt".into()] };
    assert_eq!(manager.execute_development_git("alice", conflict, CancellationToken::new()).await.unwrap_err(), "request_conflict");
    let review = manager.read_development_git("alice", &task).await.unwrap();
    assert_eq!(review.initial_staged_paths, vec!["user.txt"]);
    let denied = request(&task, &review, GitAction::Commit { message: "delivery".into(), confirm_full_index: false, confirm_preexisting_staged: true });
    assert_eq!(manager.execute_development_git("alice", denied, CancellationToken::new()).await.unwrap_err(), "git_full_index_confirmation_required");
    let denied = request(&task, &review, GitAction::Commit { message: "delivery".into(), confirm_full_index: true, confirm_preexisting_staged: false });
    assert_eq!(manager.execute_development_git("alice", denied, CancellationToken::new()).await.unwrap_err(), "git_preexisting_staged_confirmation_required");
    let committed = execute(&manager, &task, GitAction::Commit { message: "delivery".into(), confirm_full_index: true, confirm_preexisting_staged: true }).await;
    assert_eq!(committed["status"], "succeeded");
    assert_eq!(git(&fixture.path, &["show", "HEAD:code.txt"]), "task change");
    let remote = fixture.root.join("remote.git"); git(&fixture.root, &["init", "--bare", remote.to_str().unwrap()]); git(&fixture.path, &["remote", "add", "origin", remote.to_str().unwrap()]);
    let pushed = execute(&manager, &task, GitAction::Push { remote: "origin".into(), trust_directory: true, trust_network: true }).await;
    assert_eq!(pushed["status"], "succeeded");
    assert_eq!(git(&remote, &["rev-parse", "refs/heads/main"]), git(&fixture.path, &["rev-parse", "HEAD"]));
    assert_eq!(std::fs::read_to_string(fixture.path.join("other.txt")).unwrap(), "not selected\n");
}

#[tokio::test]
async fn literal_paths_stale_review_and_unstage_preserve_worktree() {
    let fixture = Fixture::new(); let (manager, task) = fixture.manager_task().await;
    std::fs::write(fixture.path.join("*.txt"), "literal\n").unwrap();
    std::fs::write(fixture.path.join("other.txt"), "other\n").unwrap();
    assert_eq!(execute(&manager, &task, GitAction::Stage { paths: vec!["*.txt".into()] }).await["status"], "succeeded");
    assert_eq!(git(&fixture.path, &["diff", "--cached", "--name-only"]), "*.txt");
    assert_eq!(execute(&manager, &task, GitAction::Unstage { paths: vec!["*.txt".into()] }).await["status"], "succeeded");
    assert_eq!(std::fs::read_to_string(fixture.path.join("*.txt")).unwrap(), "literal\n");
    let review = manager.read_development_git("alice", &task).await.unwrap();
    std::fs::write(fixture.path.join("other.txt"), "new external content\n").unwrap();
    let stale = request(&task, &review, GitAction::Stage { paths: vec!["other.txt".into()] });
    assert_eq!(manager.execute_development_git("alice", stale, CancellationToken::new()).await.unwrap_err(), "git_review_stale");
    assert!(git(&fixture.path, &["diff", "--cached", "--name-only"]).is_empty());
    let denied = execute_invalid_path(&manager, &task, "../outside").await;
    assert_eq!(denied, "file_access_denied");
}
async fn execute_invalid_path(manager: &AgentManager, task: &str, path: &str) -> String { let review = manager.read_development_git("alice", task).await.unwrap(); manager.execute_development_git("alice", request(task, &review, GitAction::Stage { paths: vec![path.into()] }), CancellationToken::new()).await.unwrap_err() }

#[tokio::test]
async fn dirty_switch_rejects_and_merge_conflicts_remain_reviewable_until_explicit_commit() {
    let fixture = Fixture::new();
    git(&fixture.path, &["checkout", "-b", "topic"]); std::fs::write(fixture.path.join("code.txt"), "topic\n").unwrap(); git(&fixture.path, &["commit", "-am", "topic"]);
    git(&fixture.path, &["checkout", "main"]); std::fs::write(fixture.path.join("code.txt"), "main\n").unwrap(); git(&fixture.path, &["commit", "-am", "main"]);
    let (manager, task) = fixture.manager_task().await;
    std::fs::write(fixture.path.join("untracked.txt"), "keep\n").unwrap();
    let review = manager.read_development_git("alice", &task).await.unwrap();
    assert_eq!(manager.execute_development_git("alice", request(&task, &review, GitAction::Switch { branch: "topic".into() }), CancellationToken::new()).await.unwrap_err(), "git_worktree_dirty");
    std::fs::remove_file(fixture.path.join("untracked.txt")).unwrap();
    let merged = execute(&manager, &task, GitAction::Merge { branch: "topic".into() }).await;
    assert_eq!(merged["status"], "failed");
    let review = manager.read_development_git("alice", &task).await.unwrap(); assert_eq!(review.conflicts.len(), 3); assert!(!review.merge_heads.is_empty());
    assert!(std::fs::read_to_string(fixture.path.join("code.txt")).unwrap().contains("<<<<<<<"));
    let denied = request(&task, &review, GitAction::Commit { message: "unresolved".into(), confirm_full_index: true, confirm_preexisting_staged: true });
    assert_eq!(manager.execute_development_git("alice", denied, CancellationToken::new()).await.unwrap_err(), "git_unmerged");
    std::fs::write(fixture.path.join("code.txt"), "resolved\n").unwrap();
    assert_eq!(execute(&manager, &task, GitAction::Stage { paths: vec!["code.txt".into()] }).await["status"], "succeeded");
    assert_eq!(execute(&manager, &task, GitAction::Commit { message: "resolve".into(), confirm_full_index: true, confirm_preexisting_staged: true }).await["status"], "succeeded");
    assert_eq!(git(&fixture.path, &["rev-list", "--parents", "-n", "1", "HEAD"]).split_whitespace().count(), 3);
}

#[tokio::test]
async fn hooks_filters_and_unsafe_remotes_cannot_execute_project_programs() {
    let fixture = Fixture::new(); let marker = fixture.root.join("executed");
    let script = format!("#!/bin/sh\ntouch '{}'\n", marker.display());
    for name in ["pre-commit", "post-commit", "pre-push"] { let hook = fixture.path.join(".git/hooks").join(name); std::fs::write(&hook, &script).unwrap(); #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o755)).unwrap(); } }
    git(&fixture.path, &["config", "filter.evil.clean", &format!("touch {}; cat", marker.display())]);
    std::fs::write(fixture.path.join(".gitattributes"), "*.txt filter=evil diff=evil\n").unwrap();
    git(&fixture.path, &["config", "diff.evil.textconv", &format!("touch {}", marker.display())]);
    std::fs::write(fixture.path.join("code.txt"), "safe\n").unwrap();
    let (manager, task) = fixture.manager_task().await;
    assert_eq!(execute(&manager, &task, GitAction::Stage { paths: vec!["code.txt".into()] }).await["status"], "succeeded");
    assert_eq!(execute(&manager, &task, GitAction::Commit { message: "safe".into(), confirm_full_index: true, confirm_preexisting_staged: true }).await["status"], "succeeded");
    assert!(!marker.exists());
    git(&fixture.path, &["config", "filter.evil.required", "true"]);
    assert_eq!(manager.read_development_git("alice", &task).await.unwrap_err(), "git_required_filter");
    git(&fixture.path, &["config", "filter.evil.required", "false"]);
    git(&fixture.path, &["remote", "add", "bad", "ext::sh -c anything"]);
    let review = manager.read_development_git("alice", &task).await.unwrap();
    let push = request(&task, &review, GitAction::Push { remote: "bad".into(), trust_directory: true, trust_network: true });
    assert!(manager.execute_development_git("alice", push, CancellationToken::new()).await.is_err());
    assert!(!marker.exists());
}

#[tokio::test]
async fn restart_reconciliation_reads_index_without_replaying_mutation() {
    let fixture = Fixture::new(); let (manager, task) = fixture.manager_task().await;
    std::fs::write(fixture.path.join("code.txt"), "intended\n").unwrap();
    let review = manager.read_development_git("alice", &task).await.unwrap();
    let input = request(&task, &review, GitAction::Stage { paths: vec!["code.txt".into()] });
    let project = manager.task_project("alice", &task).await.unwrap();
    let mut details = json!({"request":input,"before":review,"initial_index":{"entries":review.index,"staged_paths":[]}});
    manager.prepare_git_action("alice", &project, &input, &review, &mut details, &CancellationToken::new()).await.unwrap();
    manager.inner.repository.start_git_operation("alice", &GitOperationIntent {
        id: &input.id,
        project_id: &project.id,
        task_id: &task,
        workspace_id: None,
        kind: "git_stage",
        fingerprint: &digest(&serde_json::to_vec(&input).unwrap()),
        details: &details,
    }).await.unwrap();
    git(&fixture.path, &["add", "code.txt"]); // External process completed before a simulated crash.
    let restarted = crate::test_support::manager(&fixture.root).await;
    assert_eq!(restarted.inner.repository.git_operation("alice", &input.id).await.unwrap().unwrap()["status"], "unknown");
    let result = restarted.reconcile_development_git("alice", ReconcileGitInput { id: input.id.clone(), trust_directory: false, trust_network: false }, CancellationToken::new()).await.unwrap();
    assert_eq!(result["status"], "succeeded");
    std::fs::write(fixture.path.join("code.txt"), "later external edit\n").unwrap();
    // An identical UUID returns the stored receipt and cannot stage later bytes.
    restarted.execute_development_git("alice", input, CancellationToken::new()).await.unwrap();
    assert_eq!(git(&fixture.path, &["show", ":code.txt"]), "intended");
}

#[tokio::test]
async fn unresolved_stage_after_restart_remains_unknown_and_duplicate_never_stages() {
    let fixture = Fixture::new(); let (manager, task) = fixture.manager_task().await;
    std::fs::write(fixture.path.join("code.txt"), "reviewed bytes\n").unwrap();
    let review = manager.read_development_git("alice", &task).await.unwrap();
    let input = request(&task, &review, GitAction::Stage { paths: vec!["code.txt".into()] });
    let project = manager.task_project("alice", &task).await.unwrap();
    let mut details = json!({"request":input,"before":review,"initial_index":{"entries":review.index,"staged_paths":[]}});
    manager.prepare_git_action("alice", &project, &input, &review, &mut details, &CancellationToken::new()).await.unwrap();
    manager.inner.repository.start_git_operation("alice", &GitOperationIntent {
        id: &input.id, project_id: &project.id, task_id: &task, workspace_id: None,
        kind: "git_stage", fingerprint: &digest(&serde_json::to_vec(&input).unwrap()), details: &details,
    }).await.unwrap();
    // A crash happened after intent persistence; execution is unproven.
    std::fs::write(fixture.path.join("code.txt"), "later external bytes\n").unwrap();
    let index = git(&fixture.path, &["show", ":code.txt"]);
    let restarted = crate::test_support::manager(&fixture.root).await;
    let result = restarted.reconcile_development_git("alice", ReconcileGitInput { id: input.id.clone(), trust_directory: false, trust_network: false }, CancellationToken::new()).await.unwrap();
    assert_eq!(result["status"], "unknown");
    let receipt = restarted.execute_development_git("alice", input.clone(), CancellationToken::new()).await.unwrap();
    assert_eq!(receipt["status"], "unknown");
    assert_eq!(git(&fixture.path, &["show", ":code.txt"]), index);
    assert_eq!(std::fs::read_to_string(fixture.path.join("code.txt")).unwrap(), "later external bytes\n");
    let next = crate::test_support::manager(&fixture.root).await;
    assert_eq!(next.execute_development_git("alice", input, CancellationToken::new()).await.unwrap()["status"], "unknown");
    assert_eq!(git(&fixture.path, &["show", ":code.txt"]), index);
    assert!(manager.shutdown().await); assert!(restarted.shutdown().await); assert!(next.shutdown().await);
}

#[tokio::test]
async fn discard_requires_persisted_save_ownership_and_preserves_index() {
    let fixture = Fixture::new(); let (manager, task) = fixture.manager_task().await;
    let project = manager.task_project("alice", &task).await.unwrap();
    let baseline = manager.capture_task_baseline("alice", &task).await.unwrap();
    let original_hash = digest(b"base\n");
    manager.save_development_file("alice", files::SaveFileInput { project_id: project.id.clone(), task_id: task.clone(), path: "code.txt".into(), expected_hash: original_hash, text: "host edit\n".into() }, CancellationToken::new()).await.unwrap();
    manager.read_development_review("alice", &task).await.unwrap();
    let restored = execute(&manager, &task, GitAction::Discard { baseline_version: baseline.clone(), files: vec![DiscardFile { path: "code.txt".into(), expected_hash: digest(b"host edit\n") }], confirmed: true }).await;
    assert_eq!(restored["status"], "succeeded");
    assert_eq!(std::fs::read_to_string(fixture.path.join("code.txt")).unwrap(), "base\n");
    assert!(git(&fixture.path, &["diff", "--cached", "--name-only"]).is_empty());
    std::fs::write(fixture.path.join("code.txt"), "arbitrary native edit\n").unwrap();
    let review = manager.read_development_git("alice", &task).await.unwrap();
    let input = request(&task, &review, GitAction::Discard { baseline_version: baseline, files: vec![DiscardFile { path: "code.txt".into(), expected_hash: digest(b"arbitrary native edit\n") }], confirmed: true });
    assert_eq!(manager.execute_development_git("alice", input, CancellationToken::new()).await.unwrap_err(), "discard_ownership_uncertain");
}

#[tokio::test]
async fn active_service_blocks_same_directory_and_sibling_repository_mutation() {
    let fixture = Fixture::new(); let (manager, task) = fixture.manager_task().await;
    git(&fixture.path, &["branch", "topic"]);
    let switched = execute(&manager, &task, GitAction::Switch { branch: "topic".into() }).await;
    assert_eq!(switched["status"], "succeeded");
    assert_eq!(git(&fixture.path, &["branch", "--show-current"]), "topic");
    let sibling = fixture.root.join("sibling");
    git(&fixture.path, &["worktree", "add", "-b", "sibling", sibling.to_str().unwrap()]);
    let sibling_task = crate::test_support::task(&manager, &sibling).await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); let port = listener.local_addr().unwrap().port(); drop(listener);
    let service = manager.start_development_process("alice", crate::runtime::StartProcessInput {
        id: uuid::Uuid::new_v4().to_string(), task_id: sibling_task.clone(), kind: "service".into(), program: "/bin/sh".into(), args: vec!["-c".into(), "while :; do sleep 1; done".into()], cwd: String::new(), env_refs: BTreeMap::new(), trust_workspace: true, verification: false,
        readiness: Some(crate::runtime::ReadinessInput { port, path: "/".into(), timeout_ms: 120000 }), native_run_id: None,
    }, CancellationToken::new()).await.unwrap();
    let review = manager.read_development_git("alice", &sibling_task).await.unwrap();
    let denied = manager.execute_development_git("alice", request(&sibling_task, &review, GitAction::Switch { branch: "main".into() }), CancellationToken::new()).await.unwrap_err();
    assert_eq!(denied, "workspace_busy");
    let review = manager.read_development_git("alice", &task).await.unwrap();
    let denied = manager.execute_development_git("alice", request(&task, &review, GitAction::Switch { branch: "main".into() }), CancellationToken::new()).await.unwrap_err();
    assert_eq!(denied, "repository_busy");
    let stopped = manager.stop_development_process("alice", &service.id).await.unwrap();
    assert!(stopped.finished_at.is_some());
    assert_eq!(execute(&manager, &task, GitAction::Switch { branch: "main".into() }).await["status"], "succeeded");
    assert!(manager.shutdown().await);
}

#[test]
fn truncated_output_is_never_a_complete_git_version() {
    assert_eq!(complete((b"partial".to_vec(), true)).unwrap_err(), "git_output_limit");
}

#[tokio::test]
async fn empty_index_finishes_pending_merge_with_exact_parents_but_rejects_ordinary_empty_commit() {
    let fixture = Fixture::new();
    git(&fixture.path, &["checkout", "-b", "topic"]);
    git(&fixture.path, &["commit", "--allow-empty", "-m", "topic history"]);
    let topic = git(&fixture.path, &["rev-parse", "HEAD"]);
    git(&fixture.path, &["checkout", "main"]);
    git(&fixture.path, &["commit", "--allow-empty", "-m", "main history"]);
    let main = git(&fixture.path, &["rev-parse", "HEAD"]);
    let (manager, task) = fixture.manager_task().await;
    let review = manager.read_development_git("alice", &task).await.unwrap();
    assert!(review.staged_diff.is_empty()); assert!(review.merge_heads.is_empty());
    let denied = request(&task, &review, GitAction::Commit { message: "empty".into(), confirm_full_index: true, confirm_preexisting_staged: true });
    assert_eq!(manager.execute_development_git("alice", denied, CancellationToken::new()).await.unwrap_err(), "git_empty_commit");
    assert_eq!(git(&fixture.path, &["rev-parse", "HEAD"]), main);
    assert_eq!(execute(&manager, &task, GitAction::Merge { branch: "topic".into() }).await["status"], "succeeded");
    let review = manager.read_development_git("alice", &task).await.unwrap();
    assert!(review.staged_diff.is_empty()); assert!(review.conflicts.is_empty()); assert_eq!(review.merge_heads, vec![topic.clone()]);
    let denied = request(&task, &review, GitAction::Commit { message: "finish merge".into(), confirm_full_index: false, confirm_preexisting_staged: true });
    assert_eq!(manager.execute_development_git("alice", denied, CancellationToken::new()).await.unwrap_err(), "git_full_index_confirmation_required");
    let input = request(&task, &review, GitAction::Commit { message: "finish merge".into(), confirm_full_index: true, confirm_preexisting_staged: false });
    let receipt = manager.execute_development_git("alice", input.clone(), CancellationToken::new()).await.unwrap();
    assert_eq!(receipt["status"], "succeeded");
    let head = git(&fixture.path, &["rev-parse", "HEAD"]);
    assert_eq!(git(&fixture.path, &["rev-list", "--parents", "-n", "1", "HEAD"]), format!("{head} {main} {topic}"));
    assert_eq!(git(&fixture.path, &["rev-parse", "HEAD^{tree}"]), git(&fixture.path, &["rev-parse", "HEAD^1^{tree}"]));
    assert!(manager.read_development_git("alice", &task).await.unwrap().merge_heads.is_empty());
    let restarted = crate::test_support::manager(&fixture.root).await;
    assert_eq!(restarted.execute_development_git("alice", input, CancellationToken::new()).await.unwrap(), receipt);
    assert_eq!(git(&fixture.path, &["rev-parse", "HEAD"]), head);
    assert!(manager.shutdown().await); assert!(restarted.shutdown().await);
}
