use super::*;

fn remote(f: &Fixture) -> (PathBuf, PathBuf) {
    let remote = f.root.join("remote.git");
    git(&f.root, &["init", "--bare", remote.to_str().unwrap()]);
    git(&f.path, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&f.path, &["push", "origin", "main"]);
    let peer = f.root.join("peer");
    git(&f.root, &["clone", "--branch", "main", remote.to_str().unwrap(), peer.to_str().unwrap()]);
    git(&peer, &["config", "user.name", "Peer"]); git(&peer, &["config", "user.email", "peer@example.invalid"]);
    (remote, peer)
}
fn advance(peer: &Path) -> String {
    std::fs::write(peer.join("code.txt"), "remote change\n").unwrap();
    git(peer, &["commit", "-am", "remote change"]); git(peer, &["push", "origin", "main"]);
    git(peer, &["rev-parse", "HEAD"])
}
async fn pull_request(manager: &AgentManager, task: &str) -> ExecuteGitInput {
    let preview = manager.preview_development_git_pull("alice", task, "origin", "main", CancellationToken::new()).await.unwrap();
    ExecuteGitInput { id: uuid::Uuid::new_v4().to_string(), task_id: task.into(), review_token: preview.review_token, action: GitAction::Pull { remote: "origin".into(), branch: "main".into(), preview_token: preview.preview_token, trust_directory: true, trust_network: true } }
}

// Smoke plan: this permanent test creates a temporary bare remote plus two local
// work directories, uses the production manager/ledger, and removes all fixtures.
// Run centrally: cargo test -p dgos-agents git::tests::pull -- --nocapture
#[tokio::test]
async fn temporary_remote_smoke_fast_forward_already_current_and_uuid_replay() {
    let f = Fixture::new(); let (remote, peer) = remote(&f); let target = advance(&peer);
    let (manager, task) = f.manager_task().await;
    let input = pull_request(&manager, &task).await;
    let receipt = manager.execute_development_git("alice", input.clone(), CancellationToken::new()).await.unwrap();
    assert_eq!(receipt["status"], "succeeded"); assert_eq!(receipt["details"]["phase"], "completed");
    assert_eq!(git(&f.path, &["rev-parse", "HEAD"]), target);
    assert!(git(&f.path, &["for-each-ref", "--format=%(refname)", "refs/dgos-pull"]).is_empty());
    std::fs::rename(&remote, f.root.join("offline.git")).unwrap();
    assert_eq!(manager.execute_development_git("alice", input, CancellationToken::new()).await.unwrap(), receipt);
    std::fs::rename(f.root.join("offline.git"), &remote).unwrap();
    let result = manager.execute_development_git("alice", pull_request(&manager, &task).await, CancellationToken::new()).await.unwrap();
    assert_eq!(result["status"], "succeeded"); assert_eq!(result["details"]["result"], "already_up_to_date");
    assert!(git(&f.path, &["for-each-ref", "--format=%(refname)", "refs/dgos-pull"]).is_empty());
}

#[tokio::test]
async fn dirty_diverged_and_remote_change_preserve_local_work() {
    let f = Fixture::new(); let (_, peer) = remote(&f); advance(&peer);
    let (manager, task) = f.manager_task().await;
    let stale = pull_request(&manager, &task).await;
    git(&f.path, &["remote", "set-url", "origin", "/nonexistent/changed.git"]);
    assert_eq!(manager.execute_development_git("alice", stale, CancellationToken::new()).await.unwrap_err(), "git_pull_preview_stale");
    git(&f.path, &["remote", "set-url", "origin", f.root.join("remote.git").to_str().unwrap()]);
    std::fs::write(f.path.join("code.txt"), "keep local\n").unwrap();
    let input = pull_request(&manager, &task).await;
    assert_eq!(manager.execute_development_git("alice", input, CancellationToken::new()).await.unwrap_err(), "git_worktree_dirty");
    git(&f.path, &["commit", "-am", "local branch"]);
    let before = git(&f.path, &["rev-parse", "HEAD"]);
    let input = pull_request(&manager, &task).await;
    let result = manager.execute_development_git("alice", input, CancellationToken::new()).await.unwrap();
    assert_eq!(result["status"], "failed"); assert_eq!(result["details"]["error"], "git_pull_diverged");
    assert_eq!(git(&f.path, &["rev-parse", "HEAD"]), before);
    assert_eq!(std::fs::read_to_string(f.path.join("code.txt")).unwrap(), "keep local\n");
}

#[tokio::test]
async fn fixed_history_anchor_survives_head_change_and_empty_repository() {
    let f = Fixture::new();
    git(&f.path, &["commit", "--allow-empty", "-m", "second"]);
    let (manager, task) = f.manager_task().await;
    let first = manager.read_development_git_history("alice", GitHistoryInput { task_id: task.clone(), start_oid: None, cursor: None, limit: 1 }).await.unwrap();
    assert_eq!(first.entries.len(), 1); assert_eq!(first.entries[0].subject, "second");
    git(&f.path, &["commit", "--allow-empty", "-m", "later"]);
    let second = manager.read_development_git_history("alice", GitHistoryInput { task_id: task.clone(), start_oid: Some(first.start_oid.clone()), cursor: first.next_cursor, limit: 1 }).await.unwrap();
    assert_eq!(second.start_oid, first.start_oid); assert_eq!(second.entries[0].subject, "base"); assert!(second.next_cursor.is_none());
    let empty = f.root.join("empty"); std::fs::create_dir(&empty).unwrap(); git(&empty, &["init", "-b", "main"]);
    let task = crate::test_support::task(&manager, &empty).await;
    let page = manager.read_development_git_history("alice", GitHistoryInput { task_id: task, start_oid: None, cursor: None, limit: 30 }).await.unwrap();
    assert!(page.entries.is_empty());
}

#[tokio::test]
async fn prepared_restart_remains_unknown_and_never_fetches() {
    let f = Fixture::new(); let (remote, _) = remote(&f); let (manager, task) = f.manager_task().await;
    let input = pull_request(&manager, &task).await;
    let project = manager.task_project("alice", &task).await.unwrap();
    let before = manager.read_development_git("alice", &task).await.unwrap();
    let mut details = json!({"request":input,"before":before,"initial_index":{"entries":before.index,"staged_paths":[]},"phase":"prepared"});
    manager.prepare_git_action("alice", &project, &input, &before, &mut details, &CancellationToken::new()).await.unwrap();
    manager.inner.repository.start_git_operation("alice", &GitOperationIntent { id: &input.id, project_id: &project.id, task_id: &task, workspace_id: None, kind: "git_pull", fingerprint: &digest(&serde_json::to_vec(&input).unwrap()), details: &details }).await.unwrap();
    std::fs::remove_dir_all(remote).unwrap();
    let restarted = crate::test_support::manager(&f.root).await;
    let result = restarted.reconcile_development_git("alice", ReconcileGitInput { id: input.id.clone(), trust_directory: false, trust_network: false }, CancellationToken::new()).await.unwrap();
    assert_eq!(result["status"], "unknown");
    assert_eq!(restarted.execute_development_git("alice", input, CancellationToken::new()).await.unwrap()["status"], "unknown");
}

#[tokio::test]
async fn fetch_ignores_configured_extra_refspec_uploadpack_and_hooks() {
    let f = Fixture::new(); let (_, peer) = remote(&f); let target = advance(&peer);
    let marker = f.root.join("executed");
    git(&f.path, &["config", "remote.origin.uploadpack", &format!("touch {}", marker.display())]);
    git(&f.path, &["config", "--add", "remote.origin.fetch", "+refs/heads/*:refs/heads/injected/*"]);
    git(&f.path, &["config", "maintenance.auto", "true"]);
    let hook = f.path.join(".git/hooks/post-merge");
    std::fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o755)).unwrap(); }
    let (manager, task) = f.manager_task().await;
    let input = pull_request(&manager, &task).await;
    assert_eq!(manager.execute_development_git("alice", input, CancellationToken::new()).await.unwrap()["status"], "succeeded");
    assert_eq!(git(&f.path, &["rev-parse", "HEAD"]), target); assert!(!marker.exists());
    assert!(git(&f.path, &["for-each-ref", "--format=%(refname)", "refs/heads/injected"]).is_empty());
}

#[test]
fn remote_display_never_projects_credentials_query_fragment_or_path() {
    let display = redact_remote("https://user:synthetic@host/private?token=synthetic#secret");
    assert_eq!(display, "https://host/…");
    assert_eq!(valid_url("https://host/repo?token=synthetic#secret").unwrap_err(), "unsafe_git_remote");
    assert_eq!(valid_url("ext::sh -c anything").unwrap_err(), "unsafe_git_remote");
}

#[tokio::test]
async fn post_fetch_external_edit_and_cancel_preserve_unknown_without_replay() {
    for cancel in [false, true] {
        let f = Fixture::new(); let (remote, peer) = remote(&f); advance(&peer);
        let (manager, task) = f.manager_task().await; let input = pull_request(&manager, &task).await;
        let (reached_tx, reached) = tokio::sync::oneshot::channel();
        let (release, release_rx) = tokio::sync::oneshot::channel();
        PULL_FETCH_BARRIERS.lock().unwrap().insert(input.id.clone(), (reached_tx, release_rx));
        let lease = CancellationToken::new(); let running = manager.clone(); let request = input.clone(); let running_lease = lease.clone();
        let job = tokio::spawn(async move { running.execute_development_git("alice", request, running_lease).await.unwrap() });
        reached.await.unwrap();
        if cancel { lease.cancel(); } else { std::fs::write(f.path.join("code.txt"), "external edit\n").unwrap(); }
        release.send(()).unwrap();
        let receipt = job.await.unwrap();
        assert_eq!(receipt["status"], "unknown"); assert_eq!(receipt["details"]["phase"], "fetched");
        let old_head = git(&f.path, &["rev-parse", "HEAD"]);
        std::fs::remove_dir_all(remote).unwrap();
        let restarted = crate::test_support::manager(&f.root).await;
        let observed = restarted.reconcile_development_git("alice", ReconcileGitInput { id: input.id.clone(), trust_directory: false, trust_network: false }, CancellationToken::new()).await.unwrap();
        assert_eq!(observed["status"], "unknown");
        assert_eq!(restarted.execute_development_git("alice", input, CancellationToken::new()).await.unwrap()["status"], "unknown");
        assert_eq!(git(&f.path, &["rev-parse", "HEAD"]), old_head);
        if !cancel { assert_eq!(std::fs::read_to_string(f.path.join("code.txt")).unwrap(), "external edit\n"); }
    }
}

#[tokio::test]
async fn detached_unborn_and_revoked_preview_reject_before_fetch() {
    let f = Fixture::new(); remote(&f); let (manager, task) = f.manager_task().await;
    let lease = CancellationToken::new();
    let preview = manager.preview_development_git_pull("alice", &task, "origin", "main", lease.clone()).await.unwrap();
    lease.cancel();
    let input = ExecuteGitInput { id: uuid::Uuid::new_v4().to_string(), task_id: task.clone(), review_token: preview.review_token, action: GitAction::Pull { remote: "origin".into(), branch: "main".into(), preview_token: preview.preview_token, trust_directory: true, trust_network: true } };
    assert_eq!(manager.execute_development_git("alice", input, CancellationToken::new()).await.unwrap_err(), "git_pull_preview_stale");
    git(&f.path, &["checkout", "--detach"]);
    let input = pull_request(&manager, &task).await;
    assert_eq!(manager.execute_development_git("alice", input, CancellationToken::new()).await.unwrap_err(), "git_detached_head");
    let empty = f.root.join("empty"); std::fs::create_dir(&empty).unwrap(); git(&empty, &["init", "-b", "main"]);
    git(&empty, &["remote", "add", "origin", f.root.join("remote.git").to_str().unwrap()]);
    let task = crate::test_support::task(&manager, &empty).await;
    let input = pull_request(&manager, &task).await;
    assert_eq!(manager.execute_development_git("alice", input, CancellationToken::new()).await.unwrap_err(), "git_unborn_head");
}

#[tokio::test]
async fn preexisting_pull_ref_is_rejected_without_overwriting_or_deleting_it() {
    for symbolic in [false, true] {
        let f = Fixture::new(); let (_, peer) = remote(&f); advance(&peer);
        let (manager, task) = f.manager_task().await;
        let input = pull_request(&manager, &task).await;
        let reference = format!("refs/dgos-pull/{}", input.id);
        let original = git(&f.path, &["rev-parse", "HEAD"]);
        if symbolic { git(&f.path, &["symbolic-ref", &reference, "refs/heads/main"]); }
        else { git(&f.path, &["update-ref", &reference, &original]); }
        let receipt = manager.execute_development_git("alice", input, CancellationToken::new()).await.unwrap();
        assert_eq!(receipt["status"], "failed");
        assert_eq!(receipt["details"]["error"], "git_pull_ref_conflict");
        assert!(receipt["details"]["owned_ref_oid"].is_null());
        assert_eq!(git(&f.path, &["rev-parse", &reference]), original);
        assert_eq!(git(&f.path, &["rev-parse", "HEAD"]), original);
        if symbolic { assert_eq!(git(&f.path, &["symbolic-ref", &reference]), "refs/heads/main"); }
    }
}

#[tokio::test]
async fn external_replacement_before_fetch_or_install_defeats_ownership_cas() {
    for before_fetch in [true, false] {
        let f = Fixture::new(); let (_, peer) = remote(&f); advance(&peer);
        let (manager, task) = f.manager_task().await;
        let input = pull_request(&manager, &task).await;
        let reference = format!("refs/dgos-pull/{}", input.id);
        let original = git(&f.path, &["rev-parse", "HEAD"]);
        let (reached_tx, reached) = tokio::sync::oneshot::channel();
        let (release, release_rx) = tokio::sync::oneshot::channel();
        let barriers = if before_fetch { &PULL_RESERVED_BARRIERS } else { &PULL_INSTALL_BARRIERS };
        barriers.lock().unwrap().insert(input.id.clone(), (reached_tx, release_rx));
        let running = manager.clone(); let request = input.clone();
        let job = tokio::spawn(async move { running.execute_development_git("alice", request, CancellationToken::new()).await.unwrap() });
        reached.await.unwrap();
        let reserved = git(&f.path, &["rev-parse", &reference]);
        assert_ne!(reserved, original);
        git(&f.path, &["update-ref", &reference, &original, &reserved]);
        release.send(()).unwrap();
        let receipt = job.await.unwrap();
        assert_eq!(receipt["status"], "unknown");
        assert_eq!(git(&f.path, &["rev-parse", &reference]), original);
        assert_eq!(git(&f.path, &["rev-parse", "HEAD"]), original);
        let restarted = crate::test_support::manager(&f.root).await;
        let reconciled = restarted.reconcile_development_git("alice", ReconcileGitInput { id: input.id, trust_directory: false, trust_network: false }, CancellationToken::new()).await.unwrap();
        assert_eq!(reconciled["status"], "unknown");
        assert_eq!(git(&f.path, &["rev-parse", &reference]), original);
    }
}

#[tokio::test]
async fn failed_or_cancelled_pull_never_deletes_a_replaced_owned_ref() {
    for cancel in [false, true] {
        let f = Fixture::new(); let (_, peer) = remote(&f); let target = advance(&peer);
        if !cancel {
            std::fs::write(f.path.join("code.txt"), "local commit\n").unwrap();
            git(&f.path, &["commit", "-am", "local divergence"]);
        }
        let (manager, task) = f.manager_task().await;
        let input = pull_request(&manager, &task).await;
        let reference = format!("refs/dgos-pull/{}", input.id);
        let original = git(&f.path, &["rev-parse", "HEAD"]);
        let (reached_tx, reached) = tokio::sync::oneshot::channel();
        let (release, release_rx) = tokio::sync::oneshot::channel();
        PULL_FETCH_BARRIERS.lock().unwrap().insert(input.id.clone(), (reached_tx, release_rx));
        let lease = CancellationToken::new(); let running = manager.clone(); let request = input.clone(); let running_lease = lease.clone();
        let job = tokio::spawn(async move { running.execute_development_git("alice", request, running_lease).await.unwrap() });
        reached.await.unwrap();
        let owned = git(&f.path, &["rev-parse", &reference]);
        assert_ne!(owned, target);
        assert_eq!(git(&f.path, &["rev-parse", &format!("{reference}^")]), target);
        git(&f.path, &["update-ref", &reference, &original, &owned]);
        if cancel { lease.cancel(); }
        release.send(()).unwrap();
        let receipt = job.await.unwrap();
        assert_eq!(receipt["status"], if cancel { "unknown" } else { "failed" });
        if !cancel { assert_eq!(receipt["details"]["error"], "git_pull_diverged"); }
        assert_eq!(git(&f.path, &["rev-parse", &reference]), original);
        assert_eq!(git(&f.path, &["rev-parse", "HEAD"]), original);
    }
}

#[tokio::test]
async fn completed_pull_reconciliation_requires_owned_anchor_and_cleans_only_it() {
    for replace in [false, true] {
        let f = Fixture::new(); let (_, peer) = remote(&f); let target = advance(&peer);
        let (manager, task) = f.manager_task().await;
        let input = pull_request(&manager, &task).await;
        let project = manager.task_project("alice", &task).await.unwrap();
        let before = manager.read_development_git("alice", &task).await.unwrap();
        let mut details = json!({"request":input,"before":before,"initial_index":{"entries":before.index,"staged_paths":[]},"phase":"prepared"});
        manager.prepare_git_action("alice", &project, &input, &before, &mut details, &CancellationToken::new()).await.unwrap();
        manager.inner.repository.start_git_operation("alice", &GitOperationIntent { id: &input.id, project_id: &project.id, task_id: &task, workspace_id: None, kind: "git_pull", fingerprint: &digest(&serde_json::to_vec(&input).unwrap()), details: &details }).await.unwrap();
        manager.run_git_action("alice", &project, &input, &mut details, &CancellationToken::new()).await.unwrap();
        let reference = format!("refs/dgos-pull/{}", input.id);
        let owned = details["owned_ref_oid"].as_str().unwrap();
        assert_eq!(git(&f.path, &["rev-parse", &reference]), owned);
        assert_eq!(git(&f.path, &["rev-parse", &format!("{reference}^")]), target);
        if replace { git(&f.path, &["update-ref", &reference, &target, owned]); }
        let restarted = crate::test_support::manager(&f.root).await;
        let result = restarted.reconcile_development_git("alice", ReconcileGitInput { id: input.id, trust_directory: false, trust_network: false }, CancellationToken::new()).await.unwrap();
        assert_eq!(result["status"], if replace { "unknown" } else { "succeeded" });
        if replace { assert_eq!(git(&f.path, &["rev-parse", &reference]), target); }
        else { assert!(git(&f.path, &["for-each-ref", "--format=%(refname)", &reference]).is_empty()); }
        assert_eq!(git(&f.path, &["rev-parse", "HEAD"]), target);
    }
}
