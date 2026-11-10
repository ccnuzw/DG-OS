use super::*;
use crate::{Engine, SessionOptions, files::ReadFileInput};
use tokio_util::sync::CancellationToken;

struct Fixture {
    root: std::path::PathBuf,
    manager: AgentManager,
    session: SessionRecord,
}
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-references-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("code/sub")).unwrap();
        std::fs::write(root.join("code/source.txt"), "first\nsecond\nthird\n").unwrap();
        let manager = crate::test_support::manager(&root).await;
        let task = crate::test_support::task(&manager, &root.join("code")).await;
        let project = manager.task_project("alice", &task).await.unwrap();
        let session = SessionRecord { id: uuid::Uuid::new_v4().to_string(), owner: "alice".into(), engine: Engine::Codex, native_id: None, workspace: project.path, executable: None, model: None, project_id: Some(project.id), task_id: Some(task) };
        Self { root, manager, session }
    }
    async fn reference(&self, kind: ReferenceKind) -> ConversationReference {
        let view = self.manager.read_development_file("alice", ReadFileInput { project_id: self.session.project_id.clone().unwrap(), task_id: self.session.task_id.clone(), path: "source.txt".into() }).await.unwrap();
        ConversationReference { project_id: self.session.project_id.clone().unwrap(), task_id: self.session.task_id.clone(), path: "source.txt".into(), kind, diff_source: None, expected_hash: if kind == ReferenceKind::Path { None } else { view.hash }, start_line: None, end_line: None }
    }
    async fn resolve(&self, references: Vec<ConversationReference>) -> Result<String, String> {
        self.manager.resolve_conversation_references("alice", &self.session, "Review these references".into(), references).await
    }
    #[cfg(unix)]
    async fn open(&mut self) -> String {
        let executable = self.root.join("codex");
        crate::test_support::codex_fixture(&executable, &r##"
while IFS= read -r line; do
printf '%s\n' "$line" >> protocol
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request";;
*'"method":"thread/start"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"reference-thread","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"turn/start"'*) printf '{"id":%s,"result":{"turn":{"id":"reference-turn"}}}\n' "$request"; printf '%s\n' '{"method":"turn/completed","params":{"threadId":"reference-thread","turn":{"id":"reference-turn","status":"completed"}}}';;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
        let options = SessionOptions { workspace: self.session.workspace.clone(), executable: Some(executable.to_string_lossy().into()), resume_id: None, model: None, trust_workspace: true };
        self.manager.open_session("alice", Engine::Codex, self.session.id.clone(), crate::test_support::create(&self.manager, "alice", self.session.task_id.as_deref().unwrap(), options).await, self.session.task_id.clone(), CancellationToken::new()).await.unwrap().generation
    }
    async fn close(self) {
        assert!(self.manager.shutdown().await);
        std::fs::remove_dir_all(self.root).unwrap();
    }
}

#[tokio::test]
async fn host_reads_full_file_selection_and_path_without_renderer_bodies() {
    let f = Fixture::new().await;
    let file = f.reference(ReferenceKind::File).await;
    let result = f.resolve(vec![file.clone()]).await.unwrap();
    assert!(result.contains("host-read content follows"));
    assert!(result.ends_with("first\nsecond\nthird\n"));
    let mut project_file = file.clone(); project_file.task_id = None;
    assert_eq!(f.resolve(vec![project_file]).await.unwrap(), result);
    let mut selection = file;
    selection.kind = ReferenceKind::Selection;
    selection.start_line = Some(2); selection.end_line = Some(2);
    let result = f.resolve(vec![selection]).await.unwrap();
    assert!(result.contains("selection lines 2-2"));
    assert!(result.ends_with("second\n"));
    assert!(!result.contains("first")); assert!(!result.contains("third"));
    let mut path = f.reference(ReferenceKind::Path).await;
    path.task_id = None; // First-send attachment survives task/session creation.
    let result = f.resolve(vec![path.clone()]).await.unwrap();
    assert!(result.ends_with("path only, no content attached]"));
    assert!(!result.contains("first"));
    path.path = "sub".into();
    assert!(f.resolve(vec![path]).await.is_ok());
    let mut body = serde_json::to_value(f.reference(ReferenceKind::File).await).unwrap();
    body["text"] = "renderer supplied content".into();
    assert!(serde_json::from_value::<ConversationReference>(body).is_err());
    f.close().await;
}

#[tokio::test]
async fn references_reject_changed_deleted_binary_sensitive_oversize_and_invalid_ranges() {
    let f = Fixture::new().await;
    let file = f.reference(ReferenceKind::File).await;
    std::fs::write(f.root.join("code/source.txt"), "other\nsecond\nthird\n").unwrap();
    assert_eq!(f.resolve(vec![file.clone()]).await.unwrap_err(), "file_changed");
    std::fs::remove_file(f.root.join("code/source.txt")).unwrap();
    assert_eq!(f.resolve(vec![file.clone()]).await.unwrap_err(), "file_not_found");
    std::fs::write(f.root.join("code/source.txt"), b"binary\0content").unwrap();
    assert_eq!(f.resolve(vec![file.clone()]).await.unwrap_err(), "file_binary");
    std::fs::write(f.root.join("code/source.txt"), vec![b'x'; files::FILE_LIMIT + 1]).unwrap();
    assert_eq!(f.resolve(vec![file.clone()]).await.unwrap_err(), "file_too_large");
    std::fs::write(f.root.join("code/source.txt"), "first\nsecond\nthird\n").unwrap();
    let mut sensitive = file.clone(); sensitive.path = ".env".into();
    std::fs::write(f.root.join("code/.env"), "SECRET=not attached").unwrap();
    assert_eq!(f.resolve(vec![sensitive]).await.unwrap_err(), "file_sensitive");
    for (start, end) in [(0, 1), (3, 2), (1, 4)] {
        let mut selection = file.clone(); selection.kind = ReferenceKind::Selection;
        selection.start_line = Some(start); selection.end_line = Some(end);
        assert_eq!(f.resolve(vec![selection]).await.unwrap_err(), "invalid_reference");
    }
    let mut missing_hash = file.clone(); missing_hash.expected_hash = None;
    assert_eq!(f.resolve(vec![missing_hash]).await.unwrap_err(), "invalid_reference");
    let mut stray_range = file; stray_range.start_line = Some(1);
    assert_eq!(f.resolve(vec![stray_range]).await.unwrap_err(), "invalid_reference");
    f.close().await;
}

#[tokio::test]
async fn identity_path_and_reference_budgets_are_enforced() {
    let f = Fixture::new().await;
    let file = f.reference(ReferenceKind::File).await;
    let mut other = file.clone(); other.project_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(f.resolve(vec![other]).await.unwrap_err(), "file_access_denied");
    let mut other = file.clone(); other.task_id = Some(uuid::Uuid::new_v4().to_string());
    assert_eq!(f.resolve(vec![other]).await.unwrap_err(), "file_access_denied");
    for path in ["../source.txt", "/etc/passwd", ".git/config", "sub/../../source.txt"] {
        let mut unsafe_path = file.clone(); unsafe_path.path = path.into();
        assert_eq!(f.resolve(vec![unsafe_path]).await.unwrap_err(), "file_access_denied");
    }
    #[cfg(unix)] {
        std::os::unix::fs::symlink(f.root.join("code/source.txt"), f.root.join("code/link")).unwrap();
        let mut link = file.clone(); link.path = "link".into();
        assert_eq!(f.resolve(vec![link]).await.unwrap_err(), "file_access_denied");
    }
    assert!(f.manager.resolve_conversation_references("bob", &f.session, "review".into(), vec![file.clone()]).await.is_err());
    let mut worktree_session = f.session.clone(); worktree_session.workspace = f.root.join("different-worktree").to_string_lossy().into();
    let mut original = file.clone(); original.task_id = None;
    assert_eq!(f.manager.resolve_conversation_references("alice", &worktree_session, "review".into(), vec![original]).await.unwrap_err(), "file_access_denied");
    let attached = f.manager.resolve_conversation_references("alice", &f.session, String::new(), vec![file.clone()]).await.unwrap();
    let exact = "é".repeat((PROMPT_BYTES_LIMIT - attached.len()) / 2);
    let exact = format!("{exact}{}", "x".repeat(PROMPT_BYTES_LIMIT - attached.len() - exact.len()));
    assert_eq!(f.manager.resolve_conversation_references("alice", &f.session, exact.clone(), vec![file.clone()]).await.unwrap().len(), PROMPT_BYTES_LIMIT);
    assert_eq!(f.manager.resolve_conversation_references("alice", &f.session, format!("{exact}x"), vec![file.clone()]).await.unwrap_err(), "reference_limit");
    assert!(f.resolve(vec![f.reference(ReferenceKind::Path).await; REFERENCE_COUNT_LIMIT]).await.is_ok());
    assert_eq!(f.resolve(vec![file.clone(); REFERENCE_COUNT_LIMIT + 1]).await.unwrap_err(), "reference_limit");
    let long = "x".repeat(PROMPT_BYTES_LIMIT - 50);
    assert_eq!(f.manager.resolve_conversation_references("alice", &f.session, long, vec![file.clone()]).await.unwrap_err(), "reference_limit");
    std::fs::write(f.root.join("code/source.txt"), "x".repeat(33_000)).unwrap();
    let file = f.reference(ReferenceKind::File).await;
    assert_eq!(f.resolve(vec![file.clone(), file]).await.unwrap_err(), "reference_limit");
    f.close().await;
}

#[tokio::test]
async fn diff_hash_requires_current_complete_supported_content() {
    let f = Fixture::new().await;
    f.manager.capture_task_baseline("alice", f.session.task_id.as_deref().unwrap()).await.unwrap();
    std::fs::write(f.root.join("code/source.txt"), "first\nnew second\nthird\n").unwrap();
    let diff = f.manager.read_development_diff("alice", DiffInput { task_id: f.session.task_id.clone().unwrap(), path: "source.txt".into(), source: crate::review::DiffSource::Task }).await.unwrap();
    let mut reference = f.reference(ReferenceKind::File).await;
    reference.kind = ReferenceKind::Diff;
    reference.expected_hash = Some(format!("{:x}", Sha256::digest(diff.diff.as_bytes())));
    let result = f.resolve(vec![reference.clone()]).await.unwrap();
    assert!(result.contains("task baseline diff")); assert!(result.ends_with(&diff.diff));
    std::fs::write(f.root.join("code/source.txt"), "external change\n").unwrap();
    assert_eq!(f.resolve(vec![reference.clone()]).await.unwrap_err(), "file_changed");
    std::fs::remove_file(f.root.join("code/source.txt")).unwrap();
    assert_eq!(f.resolve(vec![reference.clone()]).await.unwrap_err(), "file_not_found");
    std::fs::write(f.root.join("code/source.txt"), b"binary\0content").unwrap();
    assert_eq!(f.resolve(vec![reference]).await.unwrap_err(), "file_binary");
    f.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn send_admission_resolves_before_receipt_and_retries_never_reread_or_dispatch() {
    let mut f = Fixture::new().await;
    let generation = f.open().await;
    let file = f.reference(ReferenceKind::File).await;
    let expected = f.resolve(vec![file.clone()]).await.unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    f.manager.send("alice", &f.session.id, &generation, request.clone(), "Review these references".into(), None, vec![file.clone()], vec![], vec![]).await.unwrap();
    // Removal after dispatch cannot invalidate the durable receipt or replay it.
    std::fs::remove_file(f.root.join("code/source.txt")).unwrap();
    f.manager.send("alice", &f.session.id, &generation, request.clone(), "Review these references".into(), None, vec![file.clone()], vec![], vec![]).await.unwrap();
    let literal_references = format!("Review these references\0DG-OS references\0{}", serde_json::to_string(&vec![file.clone()]).unwrap());
    assert_eq!(f.manager.send("alice", &f.session.id, &generation, request.clone(), literal_references, None, vec![], vec![], vec![]).await.unwrap_err(), "request_conflict");
    let mut conflict = file.clone(); conflict.kind = ReferenceKind::Path; conflict.expected_hash = None;
    assert_eq!(f.manager.send("alice", &f.session.id, &generation, request.clone(), "Review these references".into(), None, vec![conflict], vec![], vec![]).await.unwrap_err(), "request_conflict");
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        loop {
            let runs = f.manager.list_development_runs("alice", f.session.task_id.as_deref().unwrap()).await.unwrap();
            if runs.iter().any(|run| run.id == request && run.status == "completed") { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    let invalid = uuid::Uuid::new_v4().to_string();
    assert_eq!(f.manager.send("alice", &f.session.id, &generation, invalid.clone(), "Review these references".into(), None, vec![file.clone()], vec![], vec![]).await.unwrap_err(), "file_not_found");
    assert!(f.manager.inner.repository.run("alice", &invalid).await.unwrap().is_none());
    std::fs::write(f.root.join("code/source.txt"), "external edit\n").unwrap();
    assert_eq!(f.manager.send("alice", &f.session.id, &generation, invalid.clone(), "Review these references".into(), None, vec![file.clone()], vec![], vec![]).await.unwrap_err(), "file_changed");
    assert!(f.manager.inner.repository.run("alice", &invalid).await.unwrap().is_none());
    std::fs::write(f.root.join("code/source.txt"), vec![b'x'; files::FILE_LIMIT + 1]).unwrap();
    assert_eq!(f.manager.send("alice", &f.session.id, &generation, invalid.clone(), "Review these references".into(), None, vec![file.clone()], vec![], vec![]).await.unwrap_err(), "file_too_large");
    assert!(f.manager.inner.repository.run("alice", &invalid).await.unwrap().is_none());
    std::fs::write(f.root.join("code/source.txt"), "first\nsecond\nthird\n").unwrap();
    assert_eq!(f.manager.send("alice", &f.session.id, &generation, invalid.clone(), "x".repeat(PROMPT_BYTES_LIMIT - 1), None, vec![file], vec![], vec![]).await.unwrap_err(), "reference_limit");
    assert!(f.manager.inner.repository.run("alice", &invalid).await.unwrap().is_none());
    let protocol: Vec<serde_json::Value> = std::fs::read_to_string(f.root.join("code/protocol")).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    let sends: Vec<_> = protocol.iter().filter(|value| value["method"] == "turn/start").collect();
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0]["params"]["input"][0]["text"], expected);
    assert!(sends[0]["params"].get("approvalPolicy").is_none());
    assert!(sends[0]["params"].get("effort").is_none());
    // Restart lookup uses the same original intent fingerprint, not current bytes.
    assert!(f.manager.shutdown().await);
    let restarted = crate::test_support::manager(&f.root).await;
    let original = ConversationReference { project_id: f.session.project_id.clone().unwrap(), task_id: f.session.task_id.clone(), path: "source.txt".into(), kind: ReferenceKind::File, diff_source: None, expected_hash: Some(format!("{:x}", Sha256::digest(b"first\nsecond\nthird\n"))), start_line: None, end_line: None };
    restarted.send("alice", &f.session.id, &generation, request, "Review these references".into(), None, vec![original], vec![], vec![]).await.unwrap();
    assert!(restarted.shutdown().await);
    f.close().await;
}

fn git(path: &std::path::Path, args: &[&str]) {
    let result = std::process::Command::new("git").current_dir(path).args(args).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
}

#[tokio::test]
async fn index_and_worktree_diffs_are_not_task_baseline_and_hash_bind_the_source() {
    use crate::review::DiffSource;
    let f = Fixture::new().await;
    let code = f.root.join("code");
    git(&code, &["init", "-b", "main"]);
    git(&code, &["config", "user.name", "Reference Fixture"]);
    git(&code, &["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(code.join("literal[1].txt"), "base\n").unwrap();
    std::fs::write(code.join("literal1.txt"), "unrelated base\n").unwrap();
    git(&code, &["add", "source.txt", "literal[1].txt", "literal1.txt"]);
    git(&code, &["commit", "-m", "fixture"]);
    std::fs::write(code.join("source.txt"), "task baseline\n").unwrap();
    f.manager.capture_task_baseline("alice", f.session.task_id.as_deref().unwrap()).await.unwrap();
    std::fs::write(code.join("source.txt"), "staged version\n").unwrap();
    git(&code, &["add", "source.txt"]);
    std::fs::write(code.join("source.txt"), "worktree version\n").unwrap();
    for source in [DiffSource::Index, DiffSource::Worktree, DiffSource::Task] {
        let diff = f.manager.read_development_diff("alice", DiffInput { task_id: f.session.task_id.clone().unwrap(), path: "source.txt".into(), source }).await.unwrap();
        let expected_before = match source { DiffSource::Index => "first\nsecond\nthird\n", DiffSource::Worktree => "staged version\n", DiffSource::Task => "task baseline\n" };
        let expected_after = if source == DiffSource::Index { "staged version\n" } else { "worktree version\n" };
        assert_eq!(diff.before.as_deref(), Some(expected_before));
        assert_eq!(diff.after.as_deref(), Some(expected_after));
        let mut reference = f.reference(ReferenceKind::File).await;
        reference.kind = ReferenceKind::Diff; reference.diff_source = Some(source);
        reference.expected_hash = Some(format!("{:x}", Sha256::digest(diff.diff.as_bytes())));
        assert!(f.resolve(vec![reference.clone()]).await.unwrap().ends_with(&diff.diff));
        reference.diff_source = Some(if source == DiffSource::Index { DiffSource::Worktree } else { DiffSource::Index });
        assert_eq!(f.resolve(vec![reference]).await.unwrap_err(), "file_changed");
    }
    std::fs::write(code.join("literal[1].txt"), "selected change\n").unwrap();
    std::fs::write(code.join("literal1.txt"), "unrelated secret marker\n").unwrap();
    let diff = f.manager.read_development_diff("alice", DiffInput { task_id: f.session.task_id.clone().unwrap(), path: "literal[1].txt".into(), source: DiffSource::Worktree }).await.unwrap();
    assert!(diff.diff.contains("selected change")); assert!(!diff.diff.contains("unrelated"));
    std::fs::write(code.join("untracked.txt"), "new untracked text\n").unwrap();
    let diff = f.manager.read_development_diff("alice", DiffInput { task_id: f.session.task_id.clone().unwrap(), path: "untracked.txt".into(), source: DiffSource::Worktree }).await.unwrap();
    assert!(diff.before.is_none()); assert!(diff.diff.contains("+new untracked text"));
    std::fs::remove_file(code.join("source.txt")).unwrap();
    let diff = f.manager.read_development_diff("alice", DiffInput { task_id: f.session.task_id.clone().unwrap(), path: "source.txt".into(), source: DiffSource::Worktree }).await.unwrap();
    assert!(diff.after.is_none()); assert!(diff.diff.contains("-staged version"));
    assert!(f.manager.read_development_diff("bob", DiffInput { task_id: f.session.task_id.clone().unwrap(), path: "source.txt".into(), source: DiffSource::Index }).await.is_err());
    f.close().await;
}

#[tokio::test]
async fn incomplete_baseline_cannot_masquerade_as_new_file_diff() {
    let f = Fixture::new().await;
    let baseline = serde_json::json!({"captured_at":1,"baseline_late":false,"truncated":true,"files":{},"initial_changes":[],"fingerprint":"incomplete"});
    f.manager.inner.repository.save_review_baseline("alice", f.session.task_id.as_deref().unwrap(), &baseline).await.unwrap();
    let diff = f.manager.read_development_diff("alice", DiffInput { task_id: f.session.task_id.clone().unwrap(), path: "source.txt".into(), source: crate::review::DiffSource::Task }).await.unwrap();
    assert_eq!(diff.reason.as_deref(), Some("scan_limit"));
    assert!(diff.diff.is_empty());
    let mut reference = f.reference(ReferenceKind::File).await;
    reference.kind = ReferenceKind::Diff;
    reference.expected_hash = Some(format!("{:x}", Sha256::digest(b"")));
    assert_eq!(f.resolve(vec![reference]).await.unwrap_err(), "scan_limit");
    f.close().await;
}
