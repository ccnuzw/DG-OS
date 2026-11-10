use super::*;
use crate::{AgentManager, Engine, session::SessionOptions};
use tokio_util::sync::CancellationToken;

struct Fixture { root: std::path::PathBuf, manager: AgentManager, id: String, session: CodexSession }
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-codex-operations-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let executable = root.join("codex");
        crate::test_support::codex_fixture(&executable, &r##"
while IFS= read -r line; do
printf '%s\n' "$line" >> protocol.log
rpc=${line#*\"id\":}; rpc=${rpc%%,*}
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$rpc";;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$rpc";;
*'"method":"thread/start"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"root","turns":[]}}}\n' "$rpc" "$PWD";;
*'"method":"thread/read"'*)
 case "$line" in
 *'"threadId":"child"'*) printf '{"id":%s,"result":{"thread":{"id":"child","parentThreadId":"root","status":{"type":"active"},"canAcceptDirectInput":true}}}\n' "$rpc";;
 *'"threadId":"grandchild"'*) printf '{"id":%s,"result":{"thread":{"id":"grandchild","parentThreadId":"child","status":{"type":"idle"},"canAcceptDirectInput":true}}}\n' "$rpc";;
 *) printf '{"id":%s,"error":{"code":-1,"message":"unobserved"}}\n' "$rpc";;
 esac;;
*'"method":"thread/turns/list"'*)
 case "$line" in
 *'"threadId":"grandchild"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$rpc";;
 *'"threadId":"child"'*)
  case "$line" in
  *'"limit":1,'*) printf '{"id":%s,"result":{"data":[{"id":"child-turn","status":"inProgress","items":[]}],"nextCursor":null}}\n' "$rpc";;
  *) printf '{"id":%s,"result":{"data":[{"id":"child-turn","status":"inProgress","items":[{"id":"nested","type":"collabAgentToolCall","tool":"spawnAgent","senderThreadId":"child","receiverThreadIds":["grandchild"],"agentsStates":{}}]}],"nextCursor":null}}\n' "$rpc";;
  esac;;
 *)
  if [ -f reverted ]; then
   if [ -f revert-auto-on-refresh ]; then
    rm revert-auto-on-refresh
    printf '%s\n' '{"method":"turn/started","params":{"threadId":"root","turn":{"id":"revert-race-turn","status":"inProgress"}}}' '{"method":"item/agentMessage/delta","params":{"threadId":"root","turnId":"revert-race-turn","itemId":"live-output","delta":"live during hydration"}}'
    sleep 0.05
   fi
   printf '{"id":%s,"result":{"data":[{"id":"t1","status":"completed","items":[{"id":"command","type":"commandExecution","status":"failed","command":"false","aggregatedOutput":"native failure","exitCode":1}]}],"nextCursor":null}}\n' "$rpc"
  else
   printf '{"id":%s,"result":{"data":[{"id":"t2","status":"completed","items":[{"id":"spawn","type":"collabAgentToolCall","tool":"spawnAgent","senderThreadId":"root","receiverThreadIds":["child"],"agentsStates":{}}]},{"id":"t1","status":"completed","items":[{"id":"answer","type":"agentMessage","text":"first"}]}],"nextCursor":null}}\n' "$rpc"
  fi;;
 esac;;
*'"method":"thread/queue/list"'*)
 if [ -f queue-cycle ]; then printf '{"id":%s,"result":{"data":[],"nextCursor":"repeat"}}\n' "$rpc";
 else printf '{"id":%s,"result":{"data":[{"id":"a","input":[{"type":"text","text":"queued"}]},{"id":"b","input":[{"type":"text","text":"second"}]}],"nextCursor":null}}\n' "$rpc"; fi;;
*'"method":"thread/queue/add"'*)
 printf '%s\n' '{"method":"turn/started","params":{"threadId":"root","turn":{"id":"automatic-turn","status":"inProgress"}}}'
 printf '%s\n' '{"method":"turn/completed","params":{"threadId":"root","turn":{"id":"automatic-turn","status":"completed","items":[]}}}'
 sleep 0.05
 printf '{"id":%s,"result":{"queuedSubmission":{"id":"added"}}}\n' "$rpc";;
*'"method":"thread/queue/update"'*)
 submission=${line#*\"queuedSubmissionId\":\"}; submission=${submission%%\"*}
 printf '{"id":%s,"result":{"queuedSubmission":{"id":"%s"}}}\n' "$rpc" "$submission";;
*'"method":"thread/queue/delete"'*) printf '{"id":%s,"result":{"deleted":true}}\n' "$rpc";;
*'"method":"thread/resume"'*) printf '{"id":%s,"result":{"thread":{"id":"grandchild"}}}\n' "$rpc";;
*'"method":"turn/start"'*) printf '{"id":%s,"result":{"turn":{"id":"grandchild-turn"}}}\n' "$rpc";;
*'"method":"thread/backgroundTerminals/list"'*)
 if [ -f terminals-unsupported ]; then printf '{"id":%s,"error":{"code":-32601,"message":"absent"}}\n' "$rpc";
 else printf '{"id":%s,"result":{"data":[{"processId":"pty","command":"sleep 60"}],"nextCursor":null}}\n' "$rpc"; fi;;
*'"method":"thread/backgroundTerminals/terminate"'*) printf '{"id":%s,"result":{"terminated":true}}\n' "$rpc";;
*'"method":"thread/queue/reorder"'*|*'"method":"turn/interrupt"'*) printf '{"id":%s,"result":{}}\n' "$rpc";;
*'"method":"turn/steer"'*) printf '{"id":%s,"result":{"turnId":"child-turn"}}\n' "$rpc";;
*'"method":"thread/fork"'*) printf '{"id":%s,"result":{"thread":{"id":"fork"}}}\n' "$rpc";;
*'"method":"thread/revert"'*) printf yes > reverted; printf '{"id":%s,"result":{"thread":{"id":"root"}}}\n' "$rpc";;
*'"method":"review/start"'*|*'"method":"thread/queue/start"'*|*'"method":"thread/compact/start"'*)
 case "$line" in *'"method":"review/start"'*) native=review-turn;; *'"method":"thread/queue/start"'*) native=queue-turn;; *) native=compact-turn;; esac
 printf '{"method":"turn/started","params":{"threadId":"root","turn":{"id":"%s","status":"inProgress"}}}\n' "$native"
 printf '{"method":"item/agentMessage/delta","params":{"threadId":"root","turnId":"%s","itemId":"reply","delta":"native output"}}\n' "$native"
 printf '{"method":"turn/completed","params":{"threadId":"root","turn":{"id":"%s","status":"completed","items":[]}}}\n' "$native"
 sleep 0.05
 case "$line" in
 *'"method":"thread/compact/start"'*)
  if [ -f misleading-compact-ack ]; then printf '{"id":%s,"result":{"turn":{"id":"not-the-native-compact-turn"}}}\n' "$rpc";
  else printf '{"id":%s,"result":{}}\n' "$rpc"; fi;;
 *) printf '{"id":%s,"result":{"reviewThreadId":"root","turn":{"id":"%s"}}}\n' "$rpc" "$native";;
 esac;;
*) case "$line" in *'"id":'*) printf '{"id":%s,"error":{"code":-32601,"message":"absent"}}\n' "$rpc";; esac;;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
        let manager = crate::test_support::manager(&root).await;
        let task = crate::test_support::task(&manager, &root).await;
        let id = uuid::Uuid::new_v4().to_string();
        let options = SessionOptions { workspace: root.to_string_lossy().into(), resume_id: None, executable: Some(executable.to_string_lossy().into()), model: None, trust_workspace: true };
        let create = crate::test_support::create(&manager, "alice", &task, options).await;
        manager.open_session("alice", Engine::Codex, id.clone(), create, Some(task), CancellationToken::new()).await.unwrap();
        let (_, crate::manager::Backend::Codex(session)) = manager.owned("alice", &id).await.unwrap() else { panic!("Codex fixture") };
        Self { root, manager, id, session }
    }
    fn requests(&self) -> Vec<Value> { std::fs::read_to_string(self.root.join("protocol.log")).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect() }
    async fn finish(self) { assert!(self.manager.close_session("alice", &self.id).await.unwrap()); assert!(self.manager.shutdown().await); std::fs::remove_dir_all(self.root).unwrap(); }
}
#[tokio::test]
async fn native_observation_discovers_nested_children_and_rejects_forged_controls() {
    let f = Fixture::new().await;
    let observed = f.session.observe_operations().await.unwrap();
    assert_eq!(observed.children.len(), 2);
    assert_eq!(observed.children.iter().find(|child| child.thread_id == "grandchild").unwrap().parent_thread_id, "child");
    assert_eq!(observed.queue[0].id, "a");
    assert_eq!(observed.terminals[0].process_id, "pty");
    assert!(observed.unsupported.contains(&"close_child".into()));
    let before = f.requests().len();
    assert_eq!(f.session.read_child_history("foreign").await.unwrap_err(), "codex_child_unobserved");
    assert_eq!(f.session.execute_operation(CodexAction::InterruptChild { child_thread_id: "foreign".into(), turn_id: "child-turn".into() }, "r".into()).await.unwrap_err(), "codex_child_unobserved");
    assert_eq!(f.requests().len(), before);
    assert_eq!(f.session.execute_operation(CodexAction::InterruptChild { child_thread_id: "child".into(), turn_id: "stale".into() }, "r".into()).await.unwrap_err(), "not_running");
    f.session.execute_operation(CodexAction::SendChild { child_thread_id: "child".into(), text: "continue".into() }, "r".into()).await.unwrap();
    f.session.execute_operation(CodexAction::InterruptChild { child_thread_id: "child".into(), turn_id: "child-turn".into() }, "r".into()).await.unwrap();
    let requests = f.requests();
    let steer = requests.iter().find(|request| request["method"] == "turn/steer").unwrap();
    assert_eq!(steer["params"]["expectedTurnId"], "child-turn");
    assert!(!requests.iter().any(|request| request["method"] == "turn/interrupt" && request["params"]["turnId"] == "stale"));
    f.finish().await;
}
#[tokio::test]
async fn run_operations_bind_pre_ack_output_and_terminal_to_the_supplied_run() {
    let f = Fixture::new().await;
    std::fs::write(f.root.join("misleading-compact-ack"), "yes").unwrap();
    for action in [CodexAction::Review { target: ReviewTarget::UncommittedChanges }, CodexAction::QueueStart { submission_id: Some("a".into()) }, CodexAction::Compact {}] {
        let run = uuid::Uuid::new_v4().to_string();
        f.session.execute_operation(action, run.clone()).await.unwrap();
        let view = f.session.snapshot().await;
        assert_eq!(view.status, "idle");
        assert_eq!(view.terminal_run.as_ref().unwrap().run_id, run);
        assert_eq!(view.terminal_run.as_ref().unwrap().status, "completed");
        assert!(view.events.iter().any(|event| event.run.as_deref() == Some(run.as_str()) && event.text.as_deref() == Some("native output")));
    }
    f.finish().await;
}
#[tokio::test]
async fn native_controls_enforce_queue_membership_and_terminal_observations() {
    let f = Fixture::new().await;
    assert_eq!(f.session.execute_operation(CodexAction::QueueReorder { submission_ids: vec!["a".into()] }, "r".into()).await.unwrap_err(), "codex_queue_membership_changed");
    assert_eq!(f.session.execute_operation(CodexAction::TerminateTerminal { process_id: "foreign".into() }, "r".into()).await.unwrap_err(), "codex_terminal_unobserved");
    f.session.execute_operation(CodexAction::QueueReorder { submission_ids: vec!["b".into(), "a".into()] }, "r".into()).await.unwrap();
    f.session.execute_operation(CodexAction::TerminateTerminal { process_id: "pty".into() }, "r".into()).await.unwrap();
    let requests = f.requests();
    assert_eq!(requests.iter().filter(|request| request["method"] == "thread/queue/reorder").count(), 1);
    assert_eq!(requests.iter().find(|request| request["method"] == "thread/queue/reorder").unwrap()["params"]["queuedSubmissionIds"], json!(["b","a"]));
    std::fs::write(f.root.join("queue-cycle"), "").unwrap();
    std::fs::write(f.root.join("terminals-unsupported"), "").unwrap();
    let snapshot = f.session.observe_operations().await.unwrap();
    assert!(snapshot.incomplete);
    assert!(snapshot.unsupported.contains(&"terminals".into()));
    assert!(!snapshot.unsupported.contains(&"queue".into()));
    f.finish().await;
}
#[tokio::test]
async fn fork_preserves_source_and_revert_refreshes_rich_history_with_cursor_reset() {
    let f = Fixture::new().await;
    let native = f.session.snapshot().await.native_id;
    let fork = f.session.execute_operation(CodexAction::Fork { new_session_id: uuid::Uuid::new_v4().to_string(), last_turn_id: Some("t1".into()) }, "r".into()).await.unwrap();
    assert_eq!(fork.as_deref(), Some("fork"));
    assert_eq!(f.session.snapshot().await.native_id, native);
    f.session.view.lock().await.push("agentMessage", Some("obsolete suffix".into()), json!({"id":"old"}));
    let old_seq = f.session.snapshot().await.events.last().unwrap().seq;
    f.session.execute_operation(CodexAction::Revert { before_turn_id: "t2".into() }, "r".into()).await.unwrap();
    let view = f.session.snapshot().await;
    let command = view.events.iter().find(|event| event.kind == "commandExecution").unwrap();
    assert_eq!(command.data["aggregatedOutput"], "native failure");
    assert_eq!(command.data["exitCode"], 1);
    assert!(!view.events.iter().any(|event| event.text.as_deref() == Some("obsolete suffix")));
    assert!(view.snapshot_since(Some(old_seq)).1.reset);
    let requests = f.requests();
    let fork = requests.iter().find(|request| request["method"] == "thread/fork").unwrap();
    assert_eq!(fork["params"]["deferGoalContinuation"], true);
    assert_eq!(fork["params"]["threadId"], "root");
    f.finish().await;
}
#[tokio::test]
async fn compact_dispatch_requires_current_native_evidence_and_refusals_take_precedence() {
    let f = Fixture::new().await;
    for version in [None, Some("codex-cli 0.166.0"), Some("codex-cli 0.168.0")] {
        f.session.view.lock().await.cli_version = version.map(str::to_owned);
        assert!(!f.session.compact_supported().await);
        assert_eq!(f.session.execute_operation(CodexAction::Compact {}, "not-dispatched".into()).await.unwrap_err(), "codex_operation_unsupported");
    }
    assert!(!f.requests().iter().any(|request| request["method"] == "thread/compact/start"));
    assert_eq!(f.session.snapshot().await.status, "idle");
    f.session.turn.lock().await.operations.compact_confirmed = true;
    assert!(f.session.compact_supported().await);
    f.session.turn.lock().await.operations.unsupported.insert("compact".into());
    assert!(!f.session.compact_supported().await);
    assert_eq!(f.session.execute_operation(CodexAction::Compact {}, "refused".into()).await.unwrap_err(), "codex_operation_unsupported");
    assert!(!f.requests().iter().any(|request| request["method"] == "thread/compact/start"));
    f.finish().await;
}

#[test]
fn compact_without_started_event_expires_as_unknown_not_success_or_forever_running() {
    let mut view = crate::session::SessionView::new("/fixture".into());
    view.status = "running".into(); view.active_run = Some("run".into());
    let mut turn = super::super::TurnState { operation_binding: true, ..Default::default() };
    expire_binding(&mut view, &mut turn, "stale-run");
    assert_eq!(view.status, "running");
    expire_binding(&mut view, &mut turn, "run");
    assert_eq!(view.status, "unknown");
    assert_eq!(view.terminal_run.as_ref().unwrap().status, "unknown");
    assert_eq!(view.terminal_run.as_ref().unwrap().run_id, "run");
    assert!(view.active_run.is_none());
}

#[tokio::test]
async fn automatic_queue_start_before_add_ack_retains_a_supervisable_terminal() {
    let f = Fixture::new().await;
    f.session.execute_operation(CodexAction::QueueAdd { text: "native automatically runs this".into() }, "queue-receipt".into()).await.unwrap();
    let view = f.session.snapshot().await;
    let marker = view.events.iter().find(|event| event.kind == "nativeRun/observed").unwrap();
    let run = marker.run.as_deref().unwrap();
    assert_ne!(run, "queue-receipt");
    assert_eq!(marker.data["run_id"].as_str(), Some(run));
    assert!(view.events.iter().any(|event| event.kind == "turn/completed" && event.run.as_deref() == Some(run) && event.data["run_status"] == "completed"));
    if let Some(status) = f.session.automatic_run_status(run).await { assert_eq!(status, "completed"); }
    // The manager registrar owns settlement; never race it by clearing the
    // pending identity ourselves before its durable transaction completes.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !f.session.automatic_runs().await.is_empty() { tokio::time::sleep(std::time::Duration::from_millis(20)).await; }
    }).await.unwrap();
    let durable = f.manager.inner.repository.run("alice", run).await.unwrap().unwrap();
    assert_eq!(durable.status, "completed");
    f.finish().await;
}
#[tokio::test]
async fn queue_crud_and_child_resume_use_only_observed_native_ids() {
    let f = Fixture::new().await;
    f.session.observe_operations().await.unwrap();
    assert_eq!(f.session.execute_operation(CodexAction::QueueUpdate { submission_id: "foreign".into(), text: "replace".into() }, "r".into()).await.unwrap_err(), "codex_queue_unobserved");
    f.session.execute_operation(CodexAction::QueueUpdate { submission_id: "a".into(), text: "replace".into() }, "r".into()).await.unwrap();
    f.session.execute_operation(CodexAction::QueueDelete { submission_id: "b".into() }, "r".into()).await.unwrap();
    assert_eq!(f.session.execute_operation(CodexAction::ResumeChild { child_thread_id: "child".into() }, "r".into()).await.unwrap_err(), "busy");
    f.session.execute_operation(CodexAction::ResumeChild { child_thread_id: "grandchild".into() }, "r".into()).await.unwrap();
    f.session.execute_operation(CodexAction::SendChild { child_thread_id: "grandchild".into(), text: "start work".into() }, "r".into()).await.unwrap();
    assert_eq!(f.session.execute_operation(CodexAction::CloseChild { child_thread_id: "grandchild".into() }, "r".into()).await.unwrap_err(), "codex_operation_unsupported");
    let requests = f.requests();
    assert_eq!(requests.iter().filter(|request| request["method"] == "thread/queue/update").count(), 1);
    assert_eq!(requests.iter().filter(|request| request["method"] == "thread/resume").count(), 1);
    assert!(!requests.iter().any(|request| request["method"] == "thread/unsubscribe"));
    f.finish().await;
}

#[tokio::test]
async fn reverting_history_preserves_automatic_turn_started_during_hydration_and_interrupt_identity() {
    let f = Fixture::new().await;
    f.session.view.lock().await.push("agentMessage", Some("obsolete suffix".into()), json!({"id":"obsolete"}));
    std::fs::write(f.root.join("revert-auto-on-refresh"), "yes").unwrap();
    f.session.execute_operation(CodexAction::Revert { before_turn_id: "t2".into() }, "revert-request".into()).await.unwrap();
    let view = f.session.snapshot().await;
    assert_eq!(view.status, "running");
    let run = view.active_run.clone().expect("automatic run survives transcript replacement");
    assert!(view.events.iter().any(|event| event.text.as_deref() == Some("live during hydration") && event.run.as_deref() == Some(run.as_str())));
    assert!(!view.events.iter().any(|event| event.text.as_deref() == Some("obsolete suffix")));
    f.session.interrupt(&run).await.unwrap();
    let requests = f.requests();
    let interrupt = requests.iter().find(|request| request["method"] == "turn/interrupt").unwrap();
    assert_eq!(interrupt["params"], json!({"threadId":"root","turnId":"revert-race-turn"}));
    f.session.event(json!({"method":"turn/completed","params":{"threadId":"root","turn":{"id":"revert-race-turn","status":"interrupted","items":[]}}})).await;
    f.finish().await;
}

#[test]
fn history_refresh_failure_preserves_live_runtime_and_notifications() {
    let mut view = crate::session::SessionView::new("/fixture".into());
    view.native_id = Some("root".into());
    view.push("agentMessage", Some("obsolete suffix".into()), Value::Null);
    let before = view.events.last().unwrap().seq;
    view.status = "waiting".into();
    view.active_run = Some("live-run".into());
    view.request(crate::session::PendingRequest { id: "approval".into(), kind: "approval".into(), title: "Approve".into(), data: Value::Null });
    view.push("nativeRun/observed", None, json!({"turn_id":"live-turn"}));
    view.push("item/agentMessage/delta", Some("live output".into()), Value::Null);
    replace_history(&mut view, "root", vec![], true, before);
    assert_eq!(view.status, "waiting");
    assert_eq!(view.active_run.as_deref(), Some("live-run"));
    assert_eq!(view.pending[0].id, "approval");
    assert!(view.history_truncated);
    assert!(view.events.iter().any(|event| event.text.as_deref() == Some("live output") && event.run.as_deref() == Some("live-run")));
    assert!(!view.events.iter().any(|event| event.text.as_deref() == Some("obsolete suffix")));
}

#[test]
fn mutation_boolean_ack_distinguishes_explicit_refusal_from_unknown_effect() {
    for field in ["deleted", "terminated"] {
        assert_eq!(confirmed_boolean(&json!({(field):true}), field), Ok(()));
        assert_eq!(confirmed_boolean(&json!({(field):false}), field).unwrap_err(), "engine_rejected");
        for reply in [json!({}), json!({(field):null}), json!({(field):"false"}), json!({(field):1})] {
            assert_eq!(confirmed_boolean(&reply, field).unwrap_err(), "result_unknown");
        }
    }
}

#[tokio::test]
async fn native_started_subagent_activity_establishes_nested_ancestry_without_path_authority() {
    let f = Fixture::new().await;
    f.session.event(json!({"method":"item/started","params":{"threadId":"foreign","item":{"id":"foreign-start","type":"subAgentActivity","kind":"started","agentThreadId":"forged","agentPath":"/untrusted"}}})).await;
    f.session.event(json!({"method":"item/completed","params":{"threadId":"root","item":{"id":"reference","type":"subAgentActivity","kind":"interacted","agentThreadId":"forged","agentPath":"/untrusted"}}})).await;
    assert_eq!(f.session.confirmed_child("forged").await.unwrap_err(), "codex_child_unobserved");
    f.session.event(json!({"method":"item/started","params":{"threadId":"root","item":{"id":"native-start","type":"subAgentActivity","kind":"started","agentThreadId":"child","agentPath":"../../not-authority"}}})).await;
    f.session.event(json!({"method":"item/completed","params":{"threadId":"child","item":{"id":"native-nested","type":"subAgentActivity","kind":"started","agentThreadId":"grandchild","agentPath":"/also-not-authority"}}})).await;
    let child = f.session.confirmed_child("child").await.unwrap();
    assert_eq!(child["id"], "child");
    let history = f.session.read_child_history("grandchild").await.unwrap();
    assert_eq!(history.thread_id, "grandchild");
    let state = f.session.turn.lock().await;
    assert_eq!(state.operations.children["child"].parent_thread_id, "root");
    assert_eq!(state.operations.children["grandchild"].parent_thread_id, "child");
    assert!(!state.operations.children.contains_key("forged"));
    drop(state);
    f.finish().await;
}

#[test]
fn hydrated_subagent_creation_history_uses_same_scoped_ancestry_parser() {
    let mut state = OperationsState::default();
    state.item("root", "root", &json!({"id":"activity","type":"subAgentActivity","kind":"started","agentThreadId":"child","agentPath":"unused"}));
    state.item("root", "child", &json!({"id":"nested","type":"subAgentActivity","kind":"started","agentThreadId":"grandchild","agentPath":"unused"}));
    state.item("root", "root", &json!({"id":"conflict","type":"subAgentActivity","kind":"started","agentThreadId":"grandchild","agentPath":"unused"}));
    assert_eq!(state.children.len(), 2);
    assert_eq!(state.children["grandchild"].parent_thread_id, "child");
    assert!(state.incomplete);
    state.item("root", "root", &json!({"id":"completed","type":"subAgentActivity","kind":"completed","agentThreadId":"unobserved","agentPath":"unused"}));
    assert!(!state.children.contains_key("unobserved"));
}
