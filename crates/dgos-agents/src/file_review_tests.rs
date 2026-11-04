use crate::{files::{ListFilesInput, ReadFileInput}, projects::{CreateProjectInput, SaveTaskInput}, review::DiffInput};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn file_access_is_scoped_ignored_bounded_and_symlink_safe() {
    let root = std::env::temp_dir().join(format!("dgos-files-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("code/nested")).unwrap();
    let path = root.join("code");
    std::fs::write(path.join(".gitignore"), "ignored/\n*.log\n").unwrap();
    std::fs::write(path.join("nested/.gitignore"), "/private.txt\n").unwrap();
    std::fs::write(path.join("nested/private.txt"), "not navigated").unwrap();
    std::fs::write(path.join("public.txt"), "safe source\n").unwrap();
    std::fs::write(path.join("binary.dat"), [0, 1, 2]).unwrap();
    std::fs::write(path.join("large.txt"), vec![b'a'; crate::files::FILE_LIMIT + 1]).unwrap();
    std::fs::write(path.join(".env"), "SECRET=must-not-preview").unwrap();
    std::fs::create_dir(path.join("ignored")).unwrap();
    std::fs::write(path.join("ignored/secret.txt"), "ignored").unwrap();
    std::fs::write(root.join("outside.txt"), "outside secret").unwrap();
    #[cfg(unix)] std::os::unix::fs::symlink(root.join("outside.txt"), path.join("link.txt")).unwrap();
    let manager = crate::test_support::manager(&root).await;
    let project = manager.create_project("alice", CreateProjectInput { kind: "import".into(), name: "Files".into(), path: path.to_string_lossy().into(), source: None }, CancellationToken::new()).await.unwrap();
    let list = |include| ListFilesInput { project_id: project.id.clone(), path: "".into(), query: "txt".into(), include_ignored: include };
    let visible = manager.list_development_files("alice", list(false)).await.unwrap();
    assert!(visible.entries.iter().any(|e| e.path == "public.txt"));
    assert!(!visible.entries.iter().any(|e| e.path == "ignored/secret.txt" || e.path == "nested/private.txt"));
    let all = manager.list_development_files("alice", list(true)).await.unwrap();
    assert!(all.entries.iter().any(|e| e.path == "nested/private.txt" && e.ignored));
    for invalid in ["../outside.txt", "/etc/passwd", ".git/config", "nested/../../outside.txt", "link.txt"] {
        let result = manager.read_development_file("alice", ReadFileInput { project_id: project.id.clone(), path: invalid.into() }).await;
        assert!(result.is_err(), "{invalid}");
    }
    for (file, reason) in [("binary.dat", "file_binary"), ("large.txt", "file_too_large"), (".env", "file_sensitive")] {
        let view = manager.read_development_file("alice", ReadFileInput { project_id: project.id.clone(), path: file.into() }).await.unwrap();
        assert_eq!(view.reason.as_deref(), Some(reason)); assert!(view.text.is_none());
    }
    assert!(manager.list_development_files("bob", list(true)).await.is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn baseline_is_immutable_and_external_edits_and_deletions_are_reviewable() {
    let root = std::env::temp_dir().join(format!("dgos-review-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("code")).unwrap(); let path = root.join("code");
    std::fs::write(path.join("one.rs"), "old line\n").unwrap();
    let manager = crate::test_support::manager(&root).await;
    let project = manager.create_project("alice", CreateProjectInput { kind: "import".into(), name: "Review".into(), path: path.to_string_lossy().into(), source: None }, CancellationToken::new()).await.unwrap();
    let task = manager.save_development_task("alice", SaveTaskInput { id: None, project_id: project.id, goal: "Change".into(), requirements: "Review actual content".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
    let original = manager.capture_task_baseline("alice", &task.id).await.unwrap();
    manager.read_development_review("alice", &task.id).await.unwrap();
    std::fs::write(path.join("one.rs"), "external line\n").unwrap();
    std::fs::write(path.join("two.rs"), "new file\n").unwrap();
    assert_eq!(manager.capture_task_baseline("alice", &task.id).await.unwrap(), original);
    let review = manager.read_development_review("alice", &task.id).await.unwrap();
    assert!(review.changes.iter().any(|c| c.path == "one.rs" && c.origin == "external" && c.kind == "modified"));
    let diff = manager.read_development_diff("alice", DiffInput { task_id: task.id.clone(), path: "one.rs".into() }).await.unwrap();
    assert!(diff.diff.contains("-old line") && diff.diff.contains("+external line"));
    std::fs::remove_file(path.join("one.rs")).unwrap();
    let diff = manager.read_development_diff("alice", DiffInput { task_id: task.id.clone(), path: "one.rs".into() }).await.unwrap();
    assert_eq!(diff.before.as_deref(), Some("old line\n")); assert!(diff.after.is_none());
    assert!(manager.read_development_review("bob", &task.id).await.is_err());
    let restarted = crate::test_support::manager(&root).await;
    assert_eq!(restarted.capture_task_baseline("alice", &task.id).await.unwrap(), original);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn large_project_metadata_returns_incomplete_review_instead_of_storage_failure() {
    let root = std::env::temp_dir().join(format!("dgos-review-limit-{}", uuid::Uuid::new_v4()));
    let path = root.join("code");
    std::fs::create_dir_all(&path).unwrap();
    for index in 0..1200 { std::fs::write(path.join(format!("file-{index:04}-{}.txt", "x".repeat(80))), "source\n").unwrap(); }
    let manager = crate::test_support::manager(&root).await;
    let project = manager.create_project("alice", CreateProjectInput { kind: "import".into(), name: "Large".into(), path: path.to_string_lossy().into(), source: None }, CancellationToken::new()).await.unwrap();
    let task = manager.save_development_task("alice", SaveTaskInput { id: None, project_id: project.id, goal: "Review".into(), requirements: "Report limits".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
    manager.capture_task_baseline("alice", &task.id).await.unwrap();
    assert!(manager.read_development_review("alice", &task.id).await.unwrap().truncated);
    let restarted = crate::test_support::manager(&root).await;
    assert!(restarted.read_development_review("alice", &task.id).await.unwrap().truncated);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn save_checks_read_version_account_and_preserves_external_content() {
    use crate::files::SaveFileInput;
    let root = std::env::temp_dir().join(format!("dgos-save-{}", uuid::Uuid::new_v4()));
    let path = root.join("code");
    std::fs::create_dir_all(path.join("nested")).unwrap();
    std::fs::write(path.join("nested/source.txt"), "original\n").unwrap();
    std::fs::write(path.join(".env"), "private").unwrap();
    let manager = crate::test_support::manager(&root).await;
    let project = manager.create_project("alice", CreateProjectInput { kind: "import".into(), name: "Editor".into(), path: path.to_string_lossy().into(), source: None }, CancellationToken::new()).await.unwrap();
    let task = manager.save_development_task("alice", SaveTaskInput { id: None, project_id: project.id.clone(), goal: "Edit".into(), requirements: "Preserve conflicts".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
    let read = manager.read_development_file("alice", ReadFileInput { project_id: project.id.clone(), path: "nested/source.txt".into() }).await.unwrap();
    let input = |hash: &str, text: &str| SaveFileInput { project_id: project.id.clone(), task_id: task.id.clone(), path: "nested/source.txt".into(), expected_hash: hash.into(), text: text.into() };
    let saved = manager.save_development_file("alice", input(read.hash.as_deref().unwrap(), "user edit\n"), CancellationToken::new()).await.unwrap();
    assert_eq!(std::fs::read_to_string(path.join("nested/source.txt")).unwrap(), "user edit\n");
    assert_eq!(manager.read_development_diff("alice", DiffInput { task_id: task.id.clone(), path: "nested/source.txt".into() }).await.unwrap().before.as_deref(), Some("original\n"));
    std::fs::write(path.join("nested/source.txt"), "external edit\n").unwrap();
    assert_eq!(manager.save_development_file("alice", input(saved.hash.as_deref().unwrap(), "stale draft\n"), CancellationToken::new()).await.unwrap_err(), "file_changed");
    assert_eq!(std::fs::read_to_string(path.join("nested/source.txt")).unwrap(), "external edit\n");
    assert!(manager.save_development_file("bob", input(saved.hash.as_deref().unwrap(), "foreign"), CancellationToken::new()).await.is_err());
    let cancelled = CancellationToken::new(); cancelled.cancel();
    assert_eq!(manager.save_development_file("alice", input(saved.hash.as_deref().unwrap(), "revoked"), cancelled).await.unwrap_err(), "session_invalid");
    for blocked in [".env", "../outside.txt", ".git/config"] {
        let mut write = input("unused", "unsafe"); write.path = blocked.into();
        assert!(manager.save_development_file("alice", write, CancellationToken::new()).await.is_err());
    }
    #[cfg(unix)] {
        std::fs::write(root.join("outside.txt"), "outside").unwrap();
        std::os::unix::fs::symlink(root.join("outside.txt"), path.join("link.txt")).unwrap();
        let mut write = input("unused", "unsafe"); write.path = "link.txt".into();
        assert!(manager.save_development_file("alice", write, CancellationToken::new()).await.is_err());
        assert_eq!(std::fs::read_to_string(root.join("outside.txt")).unwrap(), "outside");
    }
    std::fs::remove_dir_all(root).unwrap();
}
