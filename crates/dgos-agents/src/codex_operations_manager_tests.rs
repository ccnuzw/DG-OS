use super::*;
use serde_json::json;

struct Fixture { root: PathBuf, manager: AgentManager, id: String, task: String, generation: String }
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-codex-operations-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("codex");
        crate::test_support::codex_fixture(&executable, &r##"
while IFS= read -r line; do
printf '%s\n' "$line" >> protocol
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
*'"method":"initialized"'*) :;;
*'"method":"thread/start"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"'"$fixture_provider"'","thread":{"id":"ops-thread","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request";;
*'"method":"thread/turns/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request";;
*'"method":"thread/queue/add"'*)
case "$line" in
*'"text":"delayed"'*) printf yes > dispatched; sleep 1; printf '{"id":%s,"result":{"queuedSubmission":{"id":"queued-one"}}}\n' "$request";;
*'"text":"unknown"'*) printf '{"id":%s,"result":{"unexpected":true}}\n' "$request";;
*'"text":"auto"'*) printf '%s\n' '{"method":"turn/started","params":{"threadId":"ops-thread","turn":{"id":"automatic-turn","status":"inProgress"}}}' '{"method":"item/completed","params":{"threadId":"ops-thread","turnId":"automatic-turn","item":{"id":"automatic-output","type":"agentMessage","text":"automatic queue result"}}}' '{"method":"turn/completed","params":{"threadId":"ops-thread","turn":{"id":"automatic-turn","status":"completed","items":[]}}}'; printf '{"id":%s,"result":{"queuedSubmission":{"id":"automatic-submission"}}}\n' "$request";;
*) printf '{"id":%s,"result":{"queuedSubmission":{"id":"queued-one"}}}\n' "$request";;
esac;;
*'"method":"review/start"'*) printf '%s\n' '{"method":"turn/started","params":{"threadId":"ops-thread","turn":{"id":"review-turn","status":"inProgress"}}}' '{"method":"item/completed","params":{"threadId":"ops-thread","turnId":"review-turn","item":{"id":"review-output","type":"agentMessage","text":"native review result"}}}' '{"method":"turn/completed","params":{"threadId":"ops-thread","turn":{"id":"review-turn","status":"completed","items":[]}}}'; printf '{"id":%s,"result":{"turn":{"id":"review-turn"},"reviewThreadId":"ops-thread"}}\n' "$request";;
*'"method":"thread/compact/start"'*)
 if [ -f malformed-compact-ack ]; then printf '{"id":%s,"result":null}\n' "$request";
 else printf '%s\n' '{"method":"turn/started","params":{"threadId":"ops-thread","turn":{"id":"compact-turn","status":"inProgress"}}}' '{"method":"thread/tokenUsage/updated","params":{"threadId":"ops-thread","tokenUsage":{"last":{"inputTokens":12,"outputTokens":1},"total":{"inputTokens":120,"outputTokens":10},"modelContextWindow":1000}}}' '{"method":"turn/completed","params":{"threadId":"ops-thread","turn":{"id":"compact-turn","status":"completed","items":[]}}}'; printf '{"id":%s,"result":{}}\n' "$request"; fi;;
*'"method":"thread/fork"'*) printf '{"id":%s,"result":{"thread":{"id":"fork-native","turns":[]}}}\n' "$request";;
*'"method":"thread/queue/list"'*) printf '{"id":%s,"result":{"data":[{"id":"queued-one","input":[]}],"nextCursor":null}}\n' "$request";;
*'"method":"thread/backgroundTerminals/list"'*) printf '{"id":%s,"result":{"data":[{"processId":"pty","command":"sleep 60"}],"nextCursor":null}}\n' "$request";;
*'"method":"thread/queue/delete"'*|*'"method":"thread/backgroundTerminals/terminate"'*)
 if [ -f malformed-ack ]; then printf '{"id":%s,"result":{"deleted":"false","terminated":null}}\n' "$request";
 else printf '{"id":%s,"result":{}}\n' "$request"; fi;;
*) printf '{"id":%s,"error":{"code":-32601,"message":"unsupported"}}\n' "$request";;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
        let manager = crate::test_support::manager(&root).await;
        let task = crate::test_support::task(&manager, &root).await;
        let id = Uuid::new_v4().to_string();
        let options = SessionOptions { workspace: root.to_string_lossy().into_owned(), resume_id: None, executable: Some(executable.to_string_lossy().into_owned()), model: None, trust_workspace: true };
        let opened = manager.open_session("alice", Engine::Codex, id.clone(), crate::test_support::create(&manager, "alice", &task, options).await, Some(task.clone()), CancellationToken::new()).await.unwrap();
        Self { root, manager, id, task, generation: opened.generation }
    }
    fn input(&self, action: CodexAction) -> ControlCodexOperation { ControlCodexOperation { session_id: self.id.clone(), generation: self.generation.clone(), request_id: Uuid::new_v4().to_string(), action } }
    fn calls(&self, method: &str) -> Vec<Value> { std::fs::read_to_string(self.root.join("protocol")).unwrap().lines().map(|line| serde_json::from_str::<Value>(line).unwrap()).filter(|value| value["method"] == method).collect() }
    async fn finish(self) { assert!(self.manager.shutdown().await); std::fs::remove_dir_all(self.root).unwrap(); }
}

#[cfg(unix)]
#[tokio::test]
async fn operation_receipts_are_owner_generation_bound_and_never_replayed() {
    let f = Fixture::new().await;
    let input = f.input(CodexAction::QueueAdd { text: "first".into() });
    assert_eq!(f.manager.control_codex_operation("bob", input.clone()).await.unwrap_err(), "session_not_found");
    let mut stale = input.clone(); stale.generation = Uuid::new_v4().to_string();
    assert_eq!(f.manager.control_codex_operation("alice", stale).await.unwrap_err(), "stale_generation");
    let receipt = f.manager.control_codex_operation("alice", input.clone()).await.unwrap();
    assert_eq!(receipt.state, CodexOperationState::Applied);
    assert_eq!(f.manager.control_codex_operation("alice", input.clone()).await.unwrap().request_id, receipt.request_id);
    let mut conflict = input.clone(); conflict.action = CodexAction::QueueAdd { text: "second".into() };
    assert_eq!(f.manager.control_codex_operation("alice", conflict).await.unwrap_err(), "request_conflict");
    assert_eq!(f.calls("thread/queue/add").len(), 1);
    assert!(f.manager.close_session("alice", &f.id).await.unwrap());
    assert_eq!(f.manager.control_codex_operation("alice", input).await.unwrap().state, CodexOperationState::Applied);
    assert_eq!(f.calls("thread/queue/add").len(), 1);
    f.finish().await;
}
#[cfg(unix)]
#[tokio::test]
async fn review_is_a_durable_run_even_when_terminal_precedes_ack() {
    let f = Fixture::new().await;
    let input = f.input(CodexAction::Review { target: ReviewTarget::UncommittedChanges });
    let receipt = f.manager.control_codex_operation("alice", input.clone()).await.unwrap();
    assert_eq!(receipt.run_id.as_deref(), Some(input.request_id.as_str()));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let run = f.manager.inner.repository.run("alice", &input.request_id).await.unwrap().unwrap();
            if run.status == "completed" && f.manager.inner.run_projections.lock().await.is_empty() {
                assert!(run.finished_at.is_some());
                assert!(run.events.contains("native review result"));
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert_eq!(f.manager.list_development_runs("alice", &f.task).await.unwrap().len(), 1);
    assert_eq!(f.manager.control_codex_operation("alice", input).await.unwrap().state, CodexOperationState::Applied);
    assert_eq!(f.calls("review/start").len(), 1);
    f.finish().await;
}
#[tokio::test]
async fn compact_is_generation_bound_and_never_replayed_after_native_receipt() {
    let f = Fixture::new().await;
    assert!(f.manager.observe_agent_compression("alice", CodexScope { session_id: f.id.clone(), generation: f.generation.clone() }).await.unwrap());
    assert!(f.calls("thread/compact/start").is_empty());
    let input = f.input(CodexAction::Compact {});
    assert_eq!(f.manager.control_codex_operation("bob", input.clone()).await.unwrap_err(), "session_not_found");
    let mut stale = input.clone(); stale.generation = Uuid::new_v4().to_string();
    assert_eq!(f.manager.control_codex_operation("alice", stale).await.unwrap_err(), "stale_generation");
    let receipt = f.manager.control_codex_operation("alice", input.clone()).await.unwrap();
    assert_eq!(receipt.run_id.as_deref(), Some(input.request_id.as_str()));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let run = f.manager.inner.repository.run("alice", &input.request_id).await.unwrap().unwrap();
            if run.status == "completed" && f.manager.inner.run_projections.lock().await.is_empty() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert_eq!(f.manager.control_codex_operation("alice", input).await.unwrap().state, CodexOperationState::Applied);
    assert_eq!(f.calls("thread/compact/start").len(), 1);
    let view = f.manager.snapshot("alice", &f.id).await.unwrap();
    assert!(view.view.events.iter().any(|event| event.kind == "thread/tokenUsage/updated" && event.data["tokenUsage"]["last"]["inputTokens"] == 12));
    f.finish().await;
}

#[tokio::test]
async fn malformed_compact_ack_is_durable_unknown_and_never_replayed() {
    let f = Fixture::new().await;
    std::fs::write(f.root.join("malformed-compact-ack"), "yes").unwrap();
    let input = f.input(CodexAction::Compact {});
    let receipt = f.manager.control_codex_operation("alice", input.clone()).await.unwrap();
    assert_eq!(receipt.state, CodexOperationState::Unknown);
    assert_eq!(f.manager.control_codex_operation("alice", input.clone()).await.unwrap().state, CodexOperationState::Unknown);
    let inspected = f.manager.inspect_codex_operation("alice", InspectCodexOperation { session_id: input.session_id, generation: input.generation, request_id: input.request_id }).await.unwrap().unwrap();
    assert_eq!(inspected.state, CodexOperationState::Unknown);
    assert_eq!(f.calls("thread/compact/start").len(), 1);
    assert_eq!(f.manager.control_codex_operation("alice", f.input(CodexAction::Compact {})).await.unwrap_err(), "result_unknown");
    let view = f.manager.snapshot("alice", &f.id).await.unwrap();
    assert!(!view.view.events.iter().any(|event| event.kind == "thread/tokenUsage/updated"));
    f.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn aborting_ipc_cannot_release_control_or_cancel_receipt_settlement() {
    let f = Fixture::new().await;
    let input = f.input(CodexAction::QueueAdd { text: "delayed".into() });
    let worker = { let manager = f.manager.clone(); let input = input.clone(); tokio::spawn(async move { manager.control_codex_operation("alice", input).await }) };
    tokio::time::timeout(std::time::Duration::from_secs(3), async { while !f.root.join("dispatched").exists() { tokio::time::sleep(std::time::Duration::from_millis(10)).await; } }).await.unwrap();
    worker.abort();
    assert_eq!(f.manager.control_codex_operation("alice", f.input(CodexAction::QueueAdd { text: "other".into() })).await.unwrap_err(), "session_busy");
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        loop { let journal = f.manager.inner.repository.codex_operation("alice", &input.request_id).await.unwrap().unwrap(); if journal.state == "applied" { break; } tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
    }).await.unwrap();
    assert_eq!(f.manager.control_codex_operation("alice", input).await.unwrap().state, CodexOperationState::Applied);
    assert_eq!(f.calls("thread/queue/add").len(), 1);
    f.finish().await;
}
#[cfg(unix)]
#[tokio::test]
async fn ambiguous_queue_ack_blocks_new_mutations_and_never_replays() {
    let f = Fixture::new().await;
    let input = f.input(CodexAction::QueueAdd { text: "unknown".into() });
    assert_eq!(f.manager.control_codex_operation("alice", input.clone()).await.unwrap().state, CodexOperationState::Unknown);
    assert_eq!(f.manager.control_codex_operation("alice", input).await.unwrap().state, CodexOperationState::Unknown);
    assert_eq!(f.manager.control_codex_operation("alice", f.input(CodexAction::QueueAdd { text: "next".into() })).await.unwrap_err(), "result_unknown");
    assert_eq!(f.calls("thread/queue/add").len(), 1);
    f.finish().await;
}
#[cfg(unix)]
#[tokio::test]
async fn fork_is_persisted_independently_and_does_not_alias_running_backend() {
    let f = Fixture::new().await;
    let new_id = Uuid::new_v4().to_string();
    let input = f.input(CodexAction::Fork { new_session_id: new_id.clone(), last_turn_id: None });
    let receipt = f.manager.control_codex_operation("alice", input.clone()).await.unwrap();
    assert_eq!(receipt.fork_session_id.as_deref(), Some(new_id.as_str()));
    assert!(receipt.run_id.is_none());
    let fork = f.manager.snapshot("alice", &new_id).await.unwrap();
    assert_eq!(fork.view.status, "closed"); assert_eq!(fork.view.native_id.as_deref(), Some("fork-native"));
    assert!(!f.manager.inner.active.lock().await.contains_key(&new_id));
    assert_eq!(f.manager.snapshot("alice", &f.id).await.unwrap().view.native_id.as_deref(), Some("ops-thread"));
    assert_eq!(f.manager.control_codex_operation("alice", input).await.unwrap().fork_session_id.as_deref(), Some(new_id.as_str()));
    assert_eq!(f.calls("thread/fork").len(), 1);
    assert_eq!(f.calls("thread/fork")[0]["params"]["deferGoalContinuation"], json!(true));
    f.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn native_automatic_queue_turn_enters_durable_supervision_without_start_replay() {
    let f = Fixture::new().await;
    let input = f.input(CodexAction::QueueAdd { text: "auto".into() });
    let receipt = f.manager.control_codex_operation("alice", input.clone()).await.unwrap();
    assert_eq!(receipt.state, CodexOperationState::Applied);
    assert!(receipt.run_id.is_none());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let runs = f.manager.list_development_runs("alice", &f.task).await.unwrap();
            if let Some(run) = runs.iter().find(|run| run.reason.as_deref() == Some("observed_native_automatic_turn")) {
                if run.status == "completed" {
                    assert!(run.events.iter().any(|event| event.text.as_deref() == Some("automatic queue result")));
                    assert!(run.start_fingerprint.is_empty());
                    assert!(run.configuration.is_some());
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert_eq!(f.calls("thread/queue/add").len(), 1);
    assert!(f.calls("thread/queue/start").is_empty());
    assert!(f.calls("turn/start").is_empty());
    assert_eq!(f.manager.control_codex_operation("alice", input).await.unwrap().state, CodexOperationState::Applied);
    f.finish().await;
}

#[tokio::test]
async fn malformed_delete_and_terminate_acknowledgements_are_unknown_immutable_and_never_replayed() {
    for malformed in [false, true] {
        for (action, method) in [
            (CodexAction::QueueDelete { submission_id: "queued-one".into() }, "thread/queue/delete"),
            (CodexAction::TerminateTerminal { process_id: "pty".into() }, "thread/backgroundTerminals/terminate"),
        ] {
            let f = Fixture::new().await;
            if malformed { std::fs::write(f.root.join("malformed-ack"), "yes").unwrap(); }
            let input = f.input(action);
            let receipt = f.manager.control_codex_operation("alice", input.clone()).await.unwrap();
            assert_eq!(receipt.state, CodexOperationState::Unknown);
            let journal = f.manager.inner.repository.codex_operation("alice", &input.request_id).await.unwrap().unwrap();
            assert_eq!(journal.state, "unknown");
            assert_eq!(f.manager.control_codex_operation("alice", input.clone()).await.unwrap().state, CodexOperationState::Unknown);
            let inspected = f.manager.inspect_codex_operation("alice", InspectCodexOperation { session_id: input.session_id, generation: input.generation, request_id: input.request_id }).await.unwrap().unwrap();
            assert_eq!(inspected.state, CodexOperationState::Unknown);
            assert_eq!(f.calls(method).len(), 1);
            f.finish().await;
        }
    }
}
