use super::*;

fn scope(project: &Project, task: Option<&str>) -> GitScopeInput {
    GitScopeInput { project_id: project.id.clone(), task_id: task.map(str::to_owned) }
}

#[tokio::test]
async fn aggregate_patches_do_not_gate_lists_reviews_stats_history_or_small_staged_preview() {
    let f = Fixture::new();
    let old = format!("before:{}\n", "a".repeat(63)).repeat(1_200); let new = format!("after!:{}\n", "b".repeat(63)).repeat(1_200);
    assert!(old.len() < files::FILE_LIMIT && new.len() < files::FILE_LIMIT);
    for i in 0..8 { std::fs::write(f.path.join(format!("bulk-{i}.txt")), &old).unwrap(); }
    git(&f.path, &["add", "."]); git(&f.path, &["commit", "-m", "bounded bulk sources"]);
    for i in 0..8 { std::fs::write(f.path.join(format!("bulk-{i}.txt")), &new).unwrap(); }
    std::fs::write(f.path.join("code.txt"), "small staged change\n").unwrap(); git(&f.path, &["add", "code.txt"]);
    assert!(git(&f.path, &["diff", "--binary"]).len() > 1024 * 1024);
    let (manager, task) = f.manager_task().await;
    let project = manager.task_project("alice", &task).await.unwrap();
    let head = git(&f.path, &["rev-parse", "HEAD"]); let index = std::fs::read(f.path.join(".git/index")).unwrap();
    let original = manager.read_development_project_git("alice", &project.id, None).await.unwrap();
    let task_overview = manager.read_development_project_git("alice", &project.id, Some(&task)).await.unwrap();
    assert_eq!(original.files.len(), 9); assert_eq!(original.snapshot_token, task_overview.snapshot_token);
    let review = manager.read_development_git("alice", &task).await.unwrap();
    assert_eq!(review.snapshot_token, original.snapshot_token); assert_eq!(review.file_versions.len(), 9);
    for value in [serde_json::to_value(&original).unwrap(), serde_json::to_value(&review).unwrap()] {
        assert!(value.get("staged_diff").is_none() && value.get("worktree_diff").is_none());
    }
    let details = manager.read_development_git_details("alice", GitDetailsInput { project_id: project.id.clone(), task_id: Some(task.clone()), path: None }, CancellationToken::new()).await.unwrap();
    assert_eq!(details.files.len(), 9); assert_eq!(details.history[0].oid, head);
    let stats = details.files.iter().find(|file| file.path == "bulk-0.txt").unwrap();
    assert_eq!((stats.worktree_added, stats.worktree_deleted), (Some(1_200), Some(1_200)));
    let history = manager.read_development_git_history("alice", GitHistoryInput { task_id: task.clone(), start_oid: None, cursor: None, limit: 10 }).await.unwrap();
    assert_eq!(history.entries[0].oid, head);
    let project_history = manager.read_development_project_git_history("alice", ProjectGitHistoryInput { project_id: project.id.clone(), start_oid: None, cursor: None, limit: 10 }).await.unwrap();
    assert_eq!(project_history.entries[0].oid, head);
    let file = manager.read_development_project_diff("alice", ProjectDiffInput { project_id: project.id.clone(), path: "bulk-0.txt".into(), source: ProjectDiffSource::Worktree }).await.unwrap();
    assert_eq!(file.before.as_deref(), Some(old.as_str())); assert_eq!(file.after.as_deref(), Some(new.as_str()));
    let staged = manager.read_development_staged_diff("alice", scope(&project, Some(&task)), CancellationToken::new()).await.unwrap();
    assert_eq!(staged.snapshot_token, original.snapshot_token);
    assert!(staged.diff.contains("+small staged change") && !staged.diff.contains("bulk-"));
    assert_eq!(git(&f.path, &["rev-parse", "HEAD"]), head); assert_eq!(std::fs::read(f.path.join(".git/index")).unwrap(), index);
    git(&f.path, &["add", "."]);
    assert!(git(&f.path, &["diff", "--cached", "--binary"]).len() > 1024 * 1024);
    assert_eq!(manager.read_development_staged_diff("alice", scope(&project, None), CancellationToken::new()).await.unwrap_err(), "git_output_limit");
    let staged_overview = manager.read_development_project_git("alice", &project.id, Some(&task)).await.unwrap();
    let staged_review = manager.read_development_git("alice", &task).await.unwrap();
    assert_eq!(staged_overview.snapshot_token, staged_review.snapshot_token);
    let committed = manager.execute_development_git("alice", request(&task, &staged_review, GitAction::Commit { message: "bounded files, large aggregate".into(), confirm_full_index: true, confirm_preexisting_staged: true }), CancellationToken::new()).await.unwrap();
    assert_eq!(committed["status"], "succeeded");
    assert!(manager.read_development_project_git("alice", &project.id, None).await.unwrap().files.is_empty());
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn tracked_huge_binary_sensitive_and_link_entries_keep_listing_without_granting_writes() {
    let f = Fixture::new();
    std::fs::write(f.path.join("huge.txt"), "base\n").unwrap();
    std::fs::write(f.path.join("binary.dat"), [0, 1, 2]).unwrap();
    std::fs::write(f.path.join(".env"), "SECRET=original-hidden\n").unwrap();
    git(&f.path, &["add", "."]); git(&f.path, &["commit", "-m", "protected sources"]);
    let (manager, task) = f.manager_task().await; let project = manager.task_project("alice", &task).await.unwrap();
    let complete = manager.read_development_git("alice", &task).await.unwrap();
    let stale_write = request(&task, &complete, GitAction::Stage { paths: vec!["code.txt".into()] });
    std::fs::write(f.path.join("huge.txt"), vec![b'x'; 2 * 1024 * 1024]).unwrap();
    std::fs::write(f.path.join("binary.dat"), vec![0; 2 * 1024 * 1024]).unwrap();
    std::fs::write(f.path.join(".env"), "SECRET=changed-hidden\n").unwrap();
    std::fs::write(f.path.join("code.txt"), "ordinary change\n").unwrap();
    #[cfg(unix)] std::os::unix::fs::symlink("code.txt", f.path.join("link.txt")).unwrap();
    let overview = manager.read_development_project_git("alice", &project.id, Some(&task)).await.unwrap();
    for path in ["huge.txt", "binary.dat", ".env", "code.txt"] { assert!(overview.files.iter().any(|file| file.path == path)); }
    #[cfg(unix)] assert!(overview.files.iter().any(|file| file.path == "link.txt"));
    assert!(!serde_json::to_string(&overview).unwrap().contains("changed-hidden"));
    assert_eq!(manager.read_development_git("alice", &task).await.unwrap_err(), "git_review_incomplete");
    let index = std::fs::read(f.path.join(".git/index")).unwrap();
    assert_eq!(manager.execute_development_git("alice", stale_write, CancellationToken::new()).await.unwrap_err(), "git_review_incomplete");
    assert_eq!(std::fs::read(f.path.join(".git/index")).unwrap(), index);
    let details = manager.read_development_git_details("alice", GitDetailsInput { project_id: project.id.clone(), task_id: Some(task.clone()), path: None }, CancellationToken::new()).await.unwrap();
    assert_eq!(details.history[0].subject, "protected sources");
    for path in ["huge.txt", "binary.dat", ".env"] {
        let file = details.files.iter().find(|file| file.path == path).unwrap();
        assert_eq!((file.index_added, file.index_deleted, file.worktree_added, file.worktree_deleted), (None, None, None, None));
        assert!(manager.read_development_project_diff("alice", ProjectDiffInput { project_id: project.id.clone(), path: path.into(), source: ProjectDiffSource::Worktree }).await.is_err());
    }
    assert_eq!(details.files.iter().find(|file| file.path == "code.txt").unwrap().worktree_added, Some(1));
    // Only staged blobs enter AI preview: unsupported unrelated worktree entries
    // remain observations, and never become mutation authority.
    git(&f.path, &["add", "code.txt"]);
    let staged = manager.read_development_staged_diff("alice", scope(&project, Some(&task)), CancellationToken::new()).await.unwrap();
    assert!(staged.diff.contains("+ordinary change") && !staged.diff.contains("SECRET="));
    assert_eq!(manager.read_development_git("alice", &task).await.unwrap_err(), "git_review_incomplete");
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn same_status_redacted_raw_edits_index_head_and_modes_change_snapshot_and_reject_stale_writes() {
    let f = Fixture::new(); let (manager, task) = f.manager_task().await;
    let project = manager.task_project("alice", &task).await.unwrap();
    std::fs::write(f.path.join("code.txt"), "api_key=first-private-value\n").unwrap();
    let first = manager.read_development_project_git("alice", &project.id, Some(&task)).await.unwrap();
    let first_review = manager.read_development_git("alice", &task).await.unwrap();
    let first_diff = manager.read_development_project_diff("alice", ProjectDiffInput { project_id: project.id.clone(), path: "code.txt".into(), source: ProjectDiffSource::Worktree }).await.unwrap();
    std::fs::write(f.path.join("code.txt"), "api_key=other-private-value\n").unwrap();
    let second = manager.read_development_project_git("alice", &project.id, Some(&task)).await.unwrap();
    let second_diff = manager.read_development_project_diff("alice", ProjectDiffInput { project_id: project.id.clone(), path: "code.txt".into(), source: ProjectDiffSource::Worktree }).await.unwrap();
    assert_eq!(serde_json::to_value(&first.files).unwrap(), serde_json::to_value(&second.files).unwrap());
    assert_eq!(first_diff.diff, second_diff.diff); assert_ne!(first.snapshot_token, second.snapshot_token);
    assert_eq!(first.snapshot_token, first_review.snapshot_token);
    let stale = request(&task, &first_review, GitAction::Stage { paths: vec!["code.txt".into()] });
    assert_eq!(manager.execute_development_git("alice", stale, CancellationToken::new()).await.unwrap_err(), "git_review_stale");
    let second_review = manager.read_development_git("alice", &task).await.unwrap();
    git(&f.path, &["add", "code.txt"]);
    let staged = manager.read_development_project_git("alice", &project.id, Some(&task)).await.unwrap();
    assert_ne!(second.snapshot_token, staged.snapshot_token);
    assert_eq!(manager.execute_development_git("alice", request(&task, &second_review, GitAction::Unstage { paths: vec!["code.txt".into()] }), CancellationToken::new()).await.unwrap_err(), "git_review_stale");
    let staged_review = manager.read_development_git("alice", &task).await.unwrap();
    git(&f.path, &["commit", "-m", "external commit"]);
    let committed = manager.read_development_project_git("alice", &project.id, Some(&task)).await.unwrap();
    assert_ne!(staged.snapshot_token, committed.snapshot_token);
    assert_eq!(manager.execute_development_git("alice", request(&task, &staged_review, GitAction::Stage { paths: vec!["code.txt".into()] }), CancellationToken::new()).await.unwrap_err(), "git_review_stale");
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(f.path.join("code.txt"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let mode = manager.read_development_project_git("alice", &project.id, Some(&task)).await.unwrap();
        assert_ne!(committed.snapshot_token, mode.snapshot_token);
        assert_eq!(mode.snapshot_token, manager.read_development_git("alice", &task).await.unwrap().snapshot_token);
    }
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn staged_preview_rejects_protected_special_binary_and_oversize_sources_and_checks_scope_lease() {
    let f = Fixture::new(); let (manager, task) = f.manager_task().await;
    let project = manager.task_project("alice", &task).await.unwrap();
    for (path, bytes, expected) in [
        (".env", b"SECRET=never-preview\n".to_vec(), "file_sensitive"),
        ("binary.dat", vec![0, 1, 2], "file_binary"),
        ("large.txt", vec![b'x'; files::FILE_LIMIT + 1], "file_too_large"),
    ] {
        std::fs::write(f.path.join(path), bytes).unwrap(); git(&f.path, &["add", path]);
        assert_eq!(manager.read_development_staged_diff("alice", scope(&project, Some(&task)), CancellationToken::new()).await.unwrap_err(), expected, "{path}");
        assert!(manager.read_development_project_git("alice", &project.id, Some(&task)).await.unwrap().files.iter().any(|file| file.path == path));
        git(&f.path, &["reset", "--", path]); std::fs::remove_file(f.path.join(path)).unwrap();
    }
    #[cfg(unix)] {
        std::os::unix::fs::symlink("code.txt", f.path.join("link.txt")).unwrap(); git(&f.path, &["add", "link.txt"]);
        assert_eq!(manager.read_development_staged_diff("alice", scope(&project, None), CancellationToken::new()).await.unwrap_err(), "file_access_denied");
        git(&f.path, &["reset", "--", "link.txt"]); std::fs::remove_file(f.path.join("link.txt")).unwrap();
    }
    std::fs::write(f.path.join("preview.txt"), "line\n".repeat(14_000)).unwrap(); git(&f.path, &["add", "preview.txt"]);
    let patch_len = git(&f.path, &["diff", "--cached"]).len(); assert!(patch_len > 65536 && patch_len < 1024 * 1024);
    assert_eq!(manager.read_development_staged_diff("alice", scope(&project, None), CancellationToken::new()).await.unwrap_err(), "git_output_limit");
    assert_eq!(manager.read_development_project_git("alice", &project.id, None).await.unwrap().files.len(), 1);
    git(&f.path, &["reset", "--", "preview.txt"]); std::fs::remove_file(f.path.join("preview.txt")).unwrap();
    std::fs::write(f.path.join(".env"), "SECRET=deleted-hidden\n").unwrap(); git(&f.path, &["add", ".env"]); git(&f.path, &["commit", "-m", "protected base"]);
    git(&f.path, &["rm", ".env"]);
    assert_eq!(manager.read_development_staged_diff("alice", scope(&project, None), CancellationToken::new()).await.unwrap_err(), "file_sensitive");
    git(&f.path, &["reset", "--hard", "HEAD"]);
    let empty = manager.read_development_staged_diff("alice", scope(&project, None), CancellationToken::new()).await.unwrap(); assert!(empty.diff.is_empty());
    let mut mismatch = scope(&project, Some(&task)); mismatch.project_id = "another-project".into();
    assert_eq!(manager.read_development_staged_diff("alice", mismatch, CancellationToken::new()).await.unwrap_err(), "task_not_found");
    assert!(manager.read_development_staged_diff("bob", scope(&project, Some(&task)), CancellationToken::new()).await.is_err());
    let cancelled = CancellationToken::new(); cancelled.cancel();
    assert_eq!(manager.read_development_staged_diff("alice", scope(&project, None), cancelled).await.unwrap_err(), "session_invalid");
    assert!(manager.shutdown().await);
}
