use super::*;
use crate::git::{GitCommitInput, GitCommitDiffInput};

async fn inspect(manager: &AgentManager, task: &str, oid: &str) -> GitCommitDetails {
    let project = manager.task_project("alice", task).await.unwrap();
    manager.read_development_git_commit("alice", GitCommitInput { project_id: project.id, task_id: Some(task.into()), oid: oid.into() }, CancellationToken::new()).await.unwrap()
}
async fn patch(manager: &AgentManager, task: &str, oid: &str, path: &str) -> crate::review::FileDiff {
    let project = manager.task_project("alice", task).await.unwrap();
    manager.read_development_git_commit_diff("alice", GitCommitDiffInput { project_id: project.id, task_id: Some(task.into()), oid: oid.into(), path: path.into() }, CancellationToken::new()).await.unwrap()
}
#[tokio::test]
async fn initial_add_modify_delete_rename_are_immutable_and_read_only() {
    let f = Fixture::new(); let initial = git(&f.path, &["rev-parse", "HEAD"]);
    std::fs::write(f.path.join("added.txt"), "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n").unwrap();
    git(&f.path, &["add", "added.txt"]); git(&f.path, &["commit", "-m", "subject\n\nFull body fixture@example.invalid\n\ntoken=private-value"]);
    let add = git(&f.path, &["rev-parse", "HEAD"]);
    git(&f.path, &["mv", "added.txt", "renamed file.txt"]);
    std::fs::write(f.path.join("renamed file.txt"), "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nnew ten\n").unwrap();
    git(&f.path, &["add", "--all"]); git(&f.path, &["commit", "-m", "rename and edit"]); let rename = git(&f.path, &["rev-parse", "HEAD"]);
    std::fs::write(f.path.join("code.txt"), "changed\n").unwrap(); git(&f.path, &["add", "code.txt"]); git(&f.path, &["commit", "-m", "modify"]); let modify = git(&f.path, &["rev-parse", "HEAD"]);
    git(&f.path, &["rm", "renamed file.txt"]); git(&f.path, &["commit", "-m", "delete"]); let delete = git(&f.path, &["rev-parse", "HEAD"]);
    // Make the checkout unrelated, with dirty attributes and protected content.
    std::fs::write(f.path.join("code.txt"), "dirty staged\n").unwrap(); git(&f.path, &["add", "code.txt"]);
    std::fs::write(f.path.join("code.txt"), "dirty unstaged\n").unwrap();
    std::fs::write(f.path.join(".env"), "token=must-not-read\n").unwrap();
    std::fs::write(f.path.join(".gitattributes"), "* -diff\n").unwrap();
    let index = std::fs::read(f.path.join(".git/index")).unwrap(); let status = git(&f.path, &["status", "--porcelain=v1"]);
    let (manager, task) = f.manager_task().await;
    let root = inspect(&manager, &task, &initial).await; assert!(root.parents.is_empty()); assert_eq!(root.files[0].status, "A"); assert_eq!(root.files[0].added, Some(1));
    let root_diff = patch(&manager, &task, &initial, "code.txt").await; assert!(root_diff.before.is_none()); assert_eq!(root_diff.after.as_deref(), Some("base\n"));
    let added = inspect(&manager, &task, &add).await; assert_eq!(added.author_email, "fixture@example.invalid"); assert!(added.message.contains("Full body")); assert!(!added.message.contains("private-value")); assert_eq!(added.files[0].added, Some(10));
    let moved = inspect(&manager, &task, &rename).await; assert_eq!(moved.files.len(), 1); assert_eq!(moved.files[0].status, "R"); assert_eq!(moved.files[0].original_path.as_deref(), Some("added.txt"));
    let moved_diff = patch(&manager, &task, &rename, "renamed file.txt").await; assert!(moved_diff.before.unwrap().contains("ten\n")); assert!(moved_diff.after.unwrap().contains("new ten\n"));
    let modified = inspect(&manager, &task, &modify).await; assert_eq!(modified.files[0].status, "M"); assert_eq!((modified.files[0].added, modified.files[0].deleted), (Some(1), Some(1)));
    let removed = inspect(&manager, &task, &delete).await; assert_eq!(removed.files[0].status, "D");
    let removed_diff = patch(&manager, &task, &delete, "renamed file.txt").await; assert!(removed_diff.after.is_none()); assert!(removed_diff.before.unwrap().contains("new ten")); assert!(removed_diff.diff_hash.is_none());
    assert_eq!(std::fs::read(f.path.join(".git/index")).unwrap(), index); assert_eq!(git(&f.path, &["status", "--porcelain=v1"]), status); assert_eq!(std::fs::read_to_string(f.path.join("code.txt")).unwrap(), "dirty unstaged\n");
    assert!(manager.inner.repository.review_baseline("alice", &task).await.unwrap().is_none()); assert!(manager.inner.repository.list_git_operations("alice", &task).await.unwrap().is_empty());
    assert!(manager.shutdown().await);
}
#[tokio::test]
async fn merge_uses_first_parent_and_ignores_replace_refs() {
    let f = Fixture::new(); let root = git(&f.path, &["rev-parse", "HEAD"]);
    git(&f.path, &["checkout", "-b", "topic"]); std::fs::write(f.path.join("topic.txt"), "topic\n").unwrap(); git(&f.path, &["add", "topic.txt"]); git(&f.path, &["commit", "-m", "topic"]); let topic = git(&f.path, &["rev-parse", "HEAD"]);
    git(&f.path, &["checkout", "main"]); std::fs::write(f.path.join("main.txt"), "main\n").unwrap(); git(&f.path, &["add", "main.txt"]); git(&f.path, &["commit", "-m", "main"]); let main = git(&f.path, &["rev-parse", "HEAD"]);
    git(&f.path, &["merge", "--no-ff", "topic", "-m", "merge"]); let merge = git(&f.path, &["rev-parse", "HEAD"]);
    git(&f.path, &["replace", &merge, &root]);
    let (manager, task) = f.manager_task().await;
    let details = inspect(&manager, &task, &merge).await; assert_eq!(details.parents, vec![main, topic]); assert_eq!(details.message.trim(), "merge"); assert_eq!(details.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), vec!["topic.txt"]);
    let diff = patch(&manager, &task, &merge, "topic.txt").await; assert!(diff.before.is_none()); assert_eq!(diff.after.as_deref(), Some("topic\n"));
    assert!(manager.shutdown().await);
}
#[tokio::test]
async fn binary_large_symlink_submodule_and_sensitive_content_fail_closed() {
    let f = Fixture::new();
    std::fs::write(f.path.join("binary.dat"), b"hello\0binary").unwrap(); std::fs::write(f.path.join("large.txt"), vec![b'x'; files::FILE_LIMIT + 1]).unwrap(); std::fs::write(f.path.join(".env"), "token=private-value").unwrap();
    #[cfg(unix)] std::os::unix::fs::symlink("code.txt", f.path.join("link.txt")).unwrap();
    git(&f.path, &["add", "--all"]); let head = git(&f.path, &["rev-parse", "HEAD"]); git(&f.path, &["update-index", "--add", "--cacheinfo", &format!("160000,{head},module")]); git(&f.path, &["commit", "-m", "special entries"]); let oid = git(&f.path, &["rev-parse", "HEAD"]);
    let (manager, task) = f.manager_task().await; let details = inspect(&manager, &task, &oid).await;
    assert_eq!(details.files.iter().find(|f| f.path == "binary.dat").unwrap().added, None);
    assert_eq!(patch(&manager, &task, &oid, "binary.dat").await.reason.as_deref(), Some("file_binary")); assert_eq!(patch(&manager, &task, &oid, "large.txt").await.reason.as_deref(), Some("file_too_large")); assert_eq!(patch(&manager, &task, &oid, "module").await.reason.as_deref(), Some("git_special_entry"));
    #[cfg(unix)] assert_eq!(patch(&manager, &task, &oid, "link.txt").await.reason.as_deref(), Some("git_special_entry"));
    let project = manager.task_project("alice", &task).await.unwrap();
    for path in [".env", "../code.txt", ".git/config"] {
        let err = manager.read_development_git_commit_diff("alice", GitCommitDiffInput { project_id: project.id.clone(), task_id: Some(task.clone()), oid: oid.clone(), path: path.into() }, CancellationToken::new()).await.err().unwrap();
        assert!(matches!(err.as_str(), "file_sensitive" | "file_access_denied"));
    }
    assert!(manager.shutdown().await);
}
#[tokio::test]
async fn ownership_lease_root_and_full_commit_oid_are_enforced() {
    let f = Fixture::new(); let oid = git(&f.path, &["rev-parse", "HEAD"]); let blob = git(&f.path, &["rev-parse", "HEAD:code.txt"]); let tree = git(&f.path, &["rev-parse", "HEAD^{tree}"]);
    let (manager, task) = f.manager_task().await; let project = manager.task_project("alice", &task).await.unwrap();
    let input = GitCommitInput { project_id: project.id.clone(), task_id: Some(task.clone()), oid: oid.clone() };
    assert!(manager.read_development_git_commit("bob", input.clone(), CancellationToken::new()).await.is_err());
    let cancelled = CancellationToken::new(); cancelled.cancel(); assert_eq!(manager.read_development_git_commit("alice", input.clone(), cancelled).await.unwrap_err(), "session_invalid");
    assert!(manager.read_development_git_commit("alice", GitCommitInput { project_id: "foreign".into(), ..input.clone() }, CancellationToken::new()).await.is_err());
    assert!(manager.read_development_git_commit("alice", GitCommitInput { task_id: Some("foreign".into()), ..input.clone() }, CancellationToken::new()).await.is_err());
    for invalid in ["HEAD".into(), "HEAD^{}".into(), "--all".into(), "a".repeat(40), blob, tree, oid[..12].into()] { assert!(manager.read_development_git_commit("alice", GitCommitInput { oid: invalid, ..input.clone() }, CancellationToken::new()).await.is_err()); }
    let project_read = manager.read_development_git_commit("alice", GitCommitInput { task_id: None, ..input.clone() }, CancellationToken::new()).await.unwrap(); assert_eq!(project_read.oid, oid);
    let subdir = f.path.join("nested"); std::fs::create_dir(&subdir).unwrap(); let nested_task = crate::test_support::task(&manager, &subdir).await; let nested = manager.task_project("alice", &nested_task).await.unwrap(); assert_eq!(manager.read_development_git_commit("alice", GitCommitInput { project_id: nested.id, task_id: Some(nested_task), oid }, CancellationToken::new()).await.unwrap_err(), "git_workspace_not_root");
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn bound_workspace_identity_is_used_and_replacement_is_rejected() {
    let f = Fixture::new(); let (manager, task) = f.manager_task().await;
    let project = manager.task_project("alice", &task).await.unwrap();
    let other = f.root.join("isolated"); std::fs::create_dir(&other).unwrap();
    git(&other, &["init", "-b", "isolated"]); git(&other, &["config", "user.name", "Isolated"]); git(&other, &["config", "user.email", "isolated@example.invalid"]);
    std::fs::write(other.join("isolated.txt"), "isolated\n").unwrap(); git(&other, &["add", "isolated.txt"]); git(&other, &["commit", "-m", "isolated"]); let oid = git(&other, &["rev-parse", "HEAD"]);
    let (path, directory_identity) = crate::projects::directory(other.to_str().unwrap()).await.unwrap();
    let workspace = dgos_db::development_repository::StoredWorkspace { id: uuid::Uuid::new_v4().to_string(), owner: "alice".into(), project_id: project.id.clone(), path, directory_identity, common_dir: None, common_identity: None, kind: "imported".into(), created_operation: None, base_commit: None };
    manager.inner.repository.insert_workspace(&workspace).await.unwrap(); manager.inner.repository.bind_task_workspace("alice", &task, &workspace.id).await.unwrap();
    let input = GitCommitInput { project_id: project.id, task_id: Some(task), oid };
    let details = manager.read_development_git_commit("alice", input.clone(), CancellationToken::new()).await.unwrap(); assert_eq!(details.files[0].path, "isolated.txt"); assert_eq!(details.author, "Isolated");
    std::fs::rename(&other, f.root.join("old-isolated")).unwrap(); std::fs::create_dir(&other).unwrap();
    assert!(manager.read_development_git_commit("alice", input, CancellationToken::new()).await.is_err()); assert!(manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn historical_deleted_file_ignores_current_symlink_and_protected_rename_is_denied() {
    let f = Fixture::new(); let original = git(&f.path, &["rev-parse", "HEAD"]);
    git(&f.path, &["rm", "code.txt"]); git(&f.path, &["commit", "-m", "delete"]); let removed = git(&f.path, &["rev-parse", "HEAD"]);
    let outside = f.root.join("outside.txt"); std::fs::write(&outside, "outside-private-value\n").unwrap(); std::os::unix::fs::symlink(&outside, f.path.join("code.txt")).unwrap();
    let (manager, task) = f.manager_task().await;
    let diff = patch(&manager, &task, &removed, "code.txt").await; assert_eq!(diff.before.as_deref(), Some("base\n")); assert!(diff.after.is_none());
    assert_eq!(patch(&manager, &task, &original, "code.txt").await.after.as_deref(), Some("base\n")); assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside-private-value\n");
    std::fs::write(f.path.join(".env"), "private-data\n").unwrap(); git(&f.path, &["add", ".env"]); git(&f.path, &["commit", "-m", "protected"]);
    git(&f.path, &["mv", ".env", "public.txt"]); git(&f.path, &["commit", "-m", "rename protected"]); let rename = git(&f.path, &["rev-parse", "HEAD"]);
    let project = manager.task_project("alice", &task).await.unwrap();
    assert_eq!(manager.read_development_git_commit_diff("alice", GitCommitDiffInput { project_id: project.id, task_id: Some(task), oid: rename, path: "public.txt".into() }, CancellationToken::new()).await.err().unwrap(), "file_sensitive"); assert!(manager.shutdown().await);
}
