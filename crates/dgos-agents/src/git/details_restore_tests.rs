use super::*;

#[tokio::test]
async fn details_real_history_counts_binary_upstream_and_owner_boundaries() {
    let f = Fixture::new();
    std::fs::write(f.path.join("code.txt"), "base\nstaged\n").unwrap(); git(&f.path, &["add", "code.txt"]);
    std::fs::write(f.path.join("code.txt"), "base\nstaged\nworktree\n").unwrap();
    std::fs::write(f.path.join("new.txt"), "one\ntwo").unwrap();
    std::fs::write(f.path.join("binary.dat"), b"\0binary").unwrap();
    git(&f.path, &["remote", "add", "origin", "https://example.invalid/repo"]);
    let head = git(&f.path, &["rev-parse", "HEAD"]);
    git(&f.path, &["update-ref", "refs/remotes/origin/main", &head]);
    git(&f.path, &["branch", "--set-upstream-to=origin/main", "main"]);
    let (manager, task) = f.manager_task().await;
    let project = manager.task_project("alice", &task).await.unwrap();
    let input = GitDetailsInput { project_id: project.id.clone(), task_id: Some(task.clone()), path: None };
    let details = manager.read_development_git_details("alice", input.clone(), CancellationToken::new()).await.unwrap();
    assert_eq!(details.branches, vec!["main"]); assert_eq!(details.remotes, vec!["origin"]);
    assert_eq!(details.remote_branches["origin"], vec!["main"]);
    assert_eq!(details.upstream.as_deref(), Some("origin/main")); assert_eq!((details.ahead, details.behind), (Some(0), Some(0)));
    let tracked = details.files.iter().find(|v| v.path == "code.txt").unwrap();
    assert_eq!((tracked.index_added, tracked.index_deleted, tracked.worktree_added, tracked.worktree_deleted), (Some(1), Some(0), Some(1), Some(0)));
    assert_eq!(details.files.iter().find(|v| v.path == "new.txt").unwrap().worktree_added, Some(2));
    assert_eq!(details.files.iter().find(|v| v.path == "binary.dat").unwrap().worktree_added, None);
    assert_eq!(details.history[0].subject, "base");
    assert!(manager.read_development_git_details("bob", input.clone(), CancellationToken::new()).await.is_err());
    let cancelled = CancellationToken::new(); cancelled.cancel();
    assert_eq!(manager.read_development_git_details("alice", input.clone(), cancelled).await.unwrap_err(), "session_invalid");
    let mut selected = input.clone(); selected.path = Some("../outside".into());
    assert_eq!(manager.read_development_git_details("alice", selected, CancellationToken::new()).await.unwrap_err(), "file_access_denied");
    let mut selected = input; selected.path = Some(".env".into());
    assert_eq!(manager.read_development_git_details("alice", selected, CancellationToken::new()).await.unwrap_err(), "file_sensitive");
}

#[tokio::test]
async fn history_is_path_scoped_bounded_and_accepts_deleted_paths() {
    let f = Fixture::new();
    std::fs::write(f.path.join("gone.txt"), "historic\n").unwrap(); git(&f.path, &["add", "gone.txt"]); git(&f.path, &["commit", "-m", "add historic"]);
    git(&f.path, &["rm", "gone.txt"]); git(&f.path, &["commit", "-m", "remove historic"]);
    for _ in 0..51 { git(&f.path, &["commit", "--allow-empty", "-m", "unrelated"]); }
    let (manager, task) = f.manager_task().await;
    let project = manager.task_project("alice", &task).await.unwrap();
    let input = GitDetailsInput { project_id: project.id, task_id: None, path: None };
    let all = manager.read_development_git_details("alice", input.clone(), CancellationToken::new()).await.unwrap();
    assert_eq!(all.history.len(), 50); assert!(all.history_truncated);
    let selected = manager.read_development_git_details("alice", GitDetailsInput { path: Some("gone.txt".into()), ..input }, CancellationToken::new()).await.unwrap();
    assert_eq!(selected.history.iter().map(|v|v.subject.as_str()).collect::<Vec<_>>(), vec!["remove historic", "add historic"]);
    assert!(!selected.history_truncated);
}

#[tokio::test]
async fn untrack_preserves_modified_bytes_and_restore_uses_index_not_head() {
    let f = Fixture::new(); let (manager, task) = f.manager_task().await;
    std::fs::write(f.path.join("code.txt"), "staged\n").unwrap(); git(&f.path, &["add", "code.txt"]);
    let index = git(&f.path, &["write-tree"]);
    std::fs::write(f.path.join("code.txt"), "unstaged\n").unwrap();
    let review = manager.read_development_git("alice", &task).await.unwrap();
    let denied = request(&task, &review, GitAction::Restore { paths: vec!["code.txt".into()], confirmed: false });
    assert_eq!(manager.execute_development_git("alice", denied, CancellationToken::new()).await.unwrap_err(), "restore_confirmation_required");
    assert_eq!(std::fs::read(f.path.join("code.txt")).unwrap(), b"unstaged\n");
    assert_eq!(execute(&manager, &task, GitAction::Restore { paths: vec!["code.txt".into()], confirmed: true }).await["status"], "succeeded");
    assert_eq!(std::fs::read(f.path.join("code.txt")).unwrap(), b"staged\n"); assert_eq!(git(&f.path, &["write-tree"]), index);
    std::fs::remove_file(f.path.join("code.txt")).unwrap();
    assert_eq!(execute(&manager, &task, GitAction::Restore { paths: vec!["code.txt".into()], confirmed: true }).await["status"], "succeeded");
    assert_eq!(std::fs::read(f.path.join("code.txt")).unwrap(), b"staged\n");
    std::fs::write(f.path.join("code.txt"), "preserve local\n").unwrap();
    assert_eq!(execute(&manager, &task, GitAction::Untrack { paths: vec!["code.txt".into()] }).await["status"], "succeeded");
    assert_eq!(std::fs::read(f.path.join("code.txt")).unwrap(), b"preserve local\n"); assert!(git(&f.path, &["ls-files"]).is_empty());
    let review = manager.read_development_git("alice", &task).await.unwrap();
    let denied = request(&task, &review, GitAction::Restore { paths: vec!["code.txt".into()], confirmed: false });
    assert_eq!(manager.execute_development_git("alice", denied, CancellationToken::new()).await.unwrap_err(), "restore_confirmation_required");
    assert_eq!(execute(&manager, &task, GitAction::Restore { paths: vec!["code.txt".into()], confirmed: true }).await["status"], "succeeded");
    assert!(!f.path.join("code.txt").exists());
}

async fn restore_hunk(manager: &AgentManager, task: &str, path: &str, ordinal: usize) -> Value {
    let project = manager.task_project("alice", task).await.unwrap();
    let diff = manager.read_development_project_diff("alice", ProjectDiffInput { project_id: project.id, path: path.into(), source: ProjectDiffSource::Worktree }).await.unwrap();
    execute(manager, task, GitAction::RestoreHunk { path: path.into(), diff_hash: diff.diff_hash.unwrap(), hunk_index: ordinal, confirmed: true }).await
}
#[tokio::test]
async fn hunk_restore_preserves_other_hunks_crlf_missing_newlines_and_index() {
    let f = Fixture::new();
    let base = (0..30).map(|n|format!("line {n}\r\n")).collect::<String>();
    std::fs::write(f.path.join("code.txt"), &base).unwrap(); git(&f.path, &["commit", "-am", "lines"]);
    let (manager, task) = f.manager_task().await; let index = git(&f.path, &["write-tree"]);
    let changed = base.replace("line 1\r\n", "first changed\r\n").replace("line 28\r\n", "second changed\r\n");
    std::fs::write(f.path.join("code.txt"), &changed).unwrap();
    assert_eq!(restore_hunk(&manager, &task, "code.txt", 1).await["status"], "succeeded");
    assert_eq!(std::fs::read_to_string(f.path.join("code.txt")).unwrap(), base.replace("line 1\r\n", "first changed\r\n"));
    assert_eq!(git(&f.path, &["write-tree"]), index);
    std::fs::write(f.path.join("code.txt"), "no newline").unwrap();
    assert_eq!(restore_hunk(&manager, &task, "code.txt", 0).await["status"], "succeeded");
    assert_eq!(std::fs::read_to_string(f.path.join("code.txt")).unwrap(), base);
    std::fs::remove_file(f.path.join("code.txt")).unwrap();
    assert_eq!(restore_hunk(&manager, &task, "code.txt", 0).await["status"], "succeeded");
    assert_eq!(std::fs::read_to_string(f.path.join("code.txt")).unwrap(), base);
    std::fs::write(f.path.join("new.txt"), "new without newline").unwrap();
    assert_eq!(restore_hunk(&manager, &task, "new.txt", 0).await["status"], "succeeded");
    assert!(!f.path.join("new.txt").exists());
    std::fs::write(f.path.join("code.txt"), "original without newline").unwrap(); git(&f.path, &["commit", "-am", "no newline"]);
    std::fs::write(f.path.join("code.txt"), "replacement\n").unwrap();
    assert_eq!(restore_hunk(&manager, &task, "code.txt", 0).await["status"], "succeeded");
    assert_eq!(std::fs::read(f.path.join("code.txt")).unwrap(), b"original without newline");
}

#[tokio::test]
async fn stale_diff_and_ordinal_never_overwrite_and_raw_secrets_survive_other_hunk() {
    let f = Fixture::new();
    let base = format!("{}\n{}tail\n", "api_key=original-private-value", "unchanged\n".repeat(20));
    std::fs::write(f.path.join("code.txt"), &base).unwrap(); git(&f.path, &["commit", "-am", "private raw host fixture"]);
    let (manager, task) = f.manager_task().await;
    let changed = base.replace("original-private-value", "edited-private-value").replace("tail", "changed tail");
    std::fs::write(f.path.join("code.txt"), &changed).unwrap();
    assert_eq!(restore_hunk(&manager, &task, "code.txt", 1).await["status"], "succeeded");
    assert_eq!(std::fs::read_to_string(f.path.join("code.txt")).unwrap(), base.replace("original-private-value", "edited-private-value"));
    let project = manager.task_project("alice", &task).await.unwrap();
    let review = manager.read_development_git("alice", &task).await.unwrap();
    let diff = manager.read_development_project_diff("alice", ProjectDiffInput { project_id: project.id, path: "code.txt".into(), source: ProjectDiffSource::Worktree }).await.unwrap();
    assert!(!diff.diff.contains("edited-private-value"));
    assert!(!review.worktree_diff.contains("edited-private-value"));
    assert_ne!(diff.diff_hash.as_deref(), Some(digest(diff.diff.as_bytes()).as_str()));
    let unconfirmed = request(&task, &review, GitAction::RestoreHunk { path: "code.txt".into(), diff_hash: diff.diff_hash.clone().unwrap(), hunk_index: 0, confirmed: false });
    assert_eq!(manager.execute_development_git("alice", unconfirmed, CancellationToken::new()).await.unwrap_err(), "restore_confirmation_required");
    assert_eq!(std::fs::read_to_string(f.path.join("code.txt")).unwrap(), base.replace("original-private-value", "edited-private-value"));
    let invalid = request(&task, &review, GitAction::RestoreHunk { path: "code.txt".into(), diff_hash: diff.diff_hash.clone().unwrap(), hunk_index: 99, confirmed: true });
    assert_eq!(manager.execute_development_git("alice", invalid, CancellationToken::new()).await.unwrap_err(), "invalid_git_hunk");
    std::fs::write(f.path.join("code.txt"), "external replacement\n").unwrap();
    let latest = manager.read_development_git("alice", &task).await.unwrap();
    let stale = request(&task, &latest, GitAction::RestoreHunk { path: "code.txt".into(), diff_hash: diff.diff_hash.unwrap(), hunk_index: 0, confirmed: true });
    assert_eq!(manager.execute_development_git("alice", stale, CancellationToken::new()).await.unwrap_err(), "git_review_stale");
    assert_eq!(std::fs::read(f.path.join("code.txt")).unwrap(), b"external replacement\n");
}

#[tokio::test]
async fn restore_binary_and_nested_deleted_file_without_index_mutation() {
    let f = Fixture::new();
    std::fs::create_dir(f.path.join("nested")).unwrap(); std::fs::write(f.path.join("nested/binary.dat"), b"\0original").unwrap();
    git(&f.path, &["add", "nested/binary.dat"]); git(&f.path, &["commit", "-m", "binary"]);
    let (manager, task) = f.manager_task().await; let index = git(&f.path, &["write-tree"]);
    std::fs::write(f.path.join("nested/binary.dat"), b"\0changed").unwrap();
    assert_eq!(execute(&manager, &task, GitAction::Restore { paths: vec!["nested/binary.dat".into()], confirmed: true }).await["status"], "succeeded");
    assert_eq!(std::fs::read(f.path.join("nested/binary.dat")).unwrap(), b"\0original");
    std::fs::remove_dir_all(f.path.join("nested")).unwrap();
    assert_eq!(execute(&manager, &task, GitAction::Restore { paths: vec!["nested/binary.dat".into()], confirmed: true }).await["status"], "succeeded");
    assert_eq!(std::fs::read(f.path.join("nested/binary.dat")).unwrap(), b"\0original"); assert_eq!(git(&f.path, &["write-tree"]), index);
}

#[tokio::test]
async fn unknown_partial_restore_never_reconciles_success_and_never_retries() {
    let f = Fixture::new();
    std::fs::write(f.path.join("other.txt"), "other base\n").unwrap(); git(&f.path, &["add", "other.txt"]); git(&f.path, &["commit", "-m", "other"]);
    let (manager, task) = f.manager_task().await;
    std::fs::write(f.path.join("code.txt"), "changed code\n").unwrap(); std::fs::write(f.path.join("other.txt"), "changed other\n").unwrap();
    let review = manager.read_development_git("alice", &task).await.unwrap();
    let input = request(&task, &review, GitAction::Restore { paths: vec!["code.txt".into(), "other.txt".into()], confirmed: true });
    let project = manager.task_project("alice", &task).await.unwrap();
    let mut details = json!({"request":input,"before":review,"initial_index":{"entries":review.index,"staged_paths":[]}});
    manager.prepare_git_action("alice", &project, &input, &review, &mut details, &CancellationToken::new()).await.unwrap();
    manager.inner.repository.start_git_operation("alice", &GitOperationIntent { id: &input.id, project_id: &project.id, task_id: &task, workspace_id: None, kind: "git_restore", fingerprint: &digest(&serde_json::to_vec(&input).unwrap()), details: &details }).await.unwrap();
    std::fs::write(f.path.join("code.txt"), "base\n").unwrap();
    let result = manager.reconcile_development_git("alice", ReconcileGitInput { id: input.id.clone(), trust_directory: false, trust_network: false }, CancellationToken::new()).await.unwrap();
    assert_eq!(result["status"], "unknown");
    manager.execute_development_git("alice", input, CancellationToken::new()).await.unwrap();
    assert_eq!(std::fs::read(f.path.join("other.txt")).unwrap(), b"changed other\n");
}

#[tokio::test]
async fn hunk_create_cas_never_overwrites_external_file_and_links_are_denied() {
    let f = Fixture::new(); let (manager, task) = f.manager_task().await;
    std::fs::remove_file(f.path.join("code.txt")).unwrap();
    let review = manager.read_development_git("alice", &task).await.unwrap();
    let input = request(&task, &review, GitAction::Restore { paths: vec!["code.txt".into()], confirmed: true });
    std::fs::write(f.path.join("code.txt"), "external winner\n").unwrap();
    assert_eq!(manager.execute_development_git("alice", input, CancellationToken::new()).await.unwrap_err(), "git_review_stale");
    assert_eq!(std::fs::read(f.path.join("code.txt")).unwrap(), b"external winner\n");
    #[cfg(unix)] {
        std::fs::remove_file(f.path.join("code.txt")).unwrap();
        let outside = f.root.join("outside"); std::fs::write(&outside, "outside\n").unwrap();
        std::os::unix::fs::symlink(&outside, f.path.join("code.txt")).unwrap();
        assert!(manager.read_development_git("alice", &task).await.is_err());
        let project = manager.task_project("alice", &task).await.unwrap();
        assert_eq!(manager.read_development_git_details("alice", GitDetailsInput { project_id: project.id, task_id: Some(task), path: Some("code.txt".into()) }, CancellationToken::new()).await.unwrap_err(), "file_access_denied");
        assert_eq!(std::fs::read(outside).unwrap(), b"outside\n");
    }
}

#[tokio::test]
async fn complete_untrack_and_hunk_receipts_reconcile_from_evidence_only() {
    for untrack in [true, false] {
        let f = Fixture::new(); let (manager, task) = f.manager_task().await;
        std::fs::write(f.path.join("code.txt"), "changed\n").unwrap();
        let review = manager.read_development_git("alice", &task).await.unwrap();
        let project = manager.task_project("alice", &task).await.unwrap();
        let action = if untrack { GitAction::Untrack { paths: vec!["code.txt".into()] } } else {
            let diff = manager.read_development_project_diff("alice", ProjectDiffInput { project_id: project.id.clone(), path: "code.txt".into(), source: ProjectDiffSource::Worktree }).await.unwrap();
            GitAction::RestoreHunk { path: "code.txt".into(), diff_hash: diff.diff_hash.unwrap(), hunk_index: 0, confirmed: true }
        };
        let input = request(&task, &review, action);
        let mut details = json!({"request":input,"before":review,"initial_index":{"entries":review.index,"staged_paths":[]}});
        manager.prepare_git_action("alice", &project, &input, &review, &mut details, &CancellationToken::new()).await.unwrap();
        manager.inner.repository.start_git_operation("alice", &GitOperationIntent { id: &input.id, project_id: &project.id, task_id: &task, workspace_id: None, kind: input.action.operation_kind(), fingerprint: &digest(&serde_json::to_vec(&input).unwrap()), details: &details }).await.unwrap();
        if untrack { git(&f.path, &["rm", "--cached", "--force", "code.txt"]); } else { std::fs::write(f.path.join("code.txt"), "base\n").unwrap(); }
        let result = manager.reconcile_development_git("alice", ReconcileGitInput { id: input.id.clone(), trust_directory: false, trust_network: false }, CancellationToken::new()).await.unwrap();
        assert_eq!(result["status"], "succeeded");
        assert_eq!(std::fs::read(f.path.join("code.txt")).unwrap(), if untrack { b"changed\n".to_vec() } else { b"base\n".to_vec() });
    }
}

#[tokio::test]
async fn staged_deletions_remain_inspectable_and_unstage_without_restoring_disk() {
    let f = Fixture::new();
    std::fs::create_dir(f.path.join("nested")).unwrap();
    std::fs::write(f.path.join("nested/gone.txt"), "removed\n").unwrap();
    git(&f.path, &["add", "nested/gone.txt"]); git(&f.path, &["commit", "-m", "nested file"]);
    let (manager, task) = f.manager_task().await;
    let project = manager.task_project("alice", &task).await.unwrap();
    git(&f.path, &["rm", "nested/gone.txt"]);
    let details = manager.read_development_git_details("alice", GitDetailsInput { project_id: project.id, task_id: Some(task.clone()), path: None }, CancellationToken::new()).await.unwrap();
    let deleted = details.files.iter().find(|file| file.path == "nested/gone.txt").unwrap();
    assert_eq!((deleted.index_added, deleted.index_deleted), (Some(0), Some(1)));
    assert_eq!(execute(&manager, &task, GitAction::Unstage { paths: vec!["nested/gone.txt".into()] }).await["status"], "succeeded");
    assert!(!f.path.join("nested/gone.txt").exists());
    assert_eq!(git(&f.path, &["show", ":nested/gone.txt"]), "removed");
    assert!(git(&f.path, &["diff", "--cached", "--name-only"]).is_empty());
}

#[tokio::test]
async fn deleted_protected_files_cannot_bypass_complete_mutation_review() {
    let f = Fixture::new();
    std::fs::write(f.path.join(".env"), "SECRET=private\n").unwrap();
    git(&f.path, &["add", ".env"]); git(&f.path, &["commit", "-m", "protected fixture"]);
    let (manager, task) = f.manager_task().await;
    git(&f.path, &["rm", ".env"]);
    std::fs::write(f.path.join("code.txt"), "otherwise safe\n").unwrap();
    git(&f.path, &["add", "code.txt"]);
    assert_eq!(manager.read_development_git("alice", &task).await.unwrap_err(), "git_review_incomplete");
    assert_eq!(git(&f.path, &["show", "HEAD:.env"]), "SECRET=private");
}

#[tokio::test]
async fn only_worktree_diffs_carry_a_restore_hunk_hash() {
    let f = Fixture::new(); let (manager, task) = f.manager_task().await;
    let project = manager.task_project("alice", &task).await.unwrap();
    std::fs::write(f.path.join("code.txt"), "staged\n").unwrap(); git(&f.path, &["add", "code.txt"]);
    std::fs::write(f.path.join("code.txt"), "working\n").unwrap();
    let index = manager.read_development_project_diff("alice", ProjectDiffInput { project_id: project.id.clone(), path: "code.txt".into(), source: ProjectDiffSource::Index }).await.unwrap();
    let working = manager.read_development_project_diff("alice", ProjectDiffInput { project_id: project.id, path: "code.txt".into(), source: ProjectDiffSource::Worktree }).await.unwrap();
    assert!(!index.diff.is_empty()); assert!(index.diff_hash.is_none());
    assert!(!working.diff.is_empty()); assert!(working.diff_hash.is_some());
}
