use super::*;
use super::history::{History, Page};

fn item(id: &str) -> Value { json!({"id":id,"type":"agentMessage","text":"identical text"}) }
fn turn(id: &str, items: &[&str]) -> Value { json!({"id":id,"itemsView":"full","items":items.iter().map(|id| item(id)).collect::<Vec<_>>(),"status":"completed"}) }
fn page(descending: bool) -> Page { Page { method:"thread/turns/list", turn:None, cursor:Some("anchor".into()), descending } }
fn ids(view: &SessionView) -> Vec<&str> { view.events.iter().map(|event| event.data["id"].as_str().unwrap()).collect() }

#[test]
fn history_protocol_orders_overlap_by_identity_not_text() {
    let mut view = SessionView::new("/fixture".into());
    let mut history = History::default();
    history.seed(&mut view, &json!({"thread":{"turns":[turn("t3", &["d","e"])]},"initialTurnsPage":{"data":[turn("t3", &["d","e"]),turn("t2", &["c"])],"nextCursor":null,"backwardsCursor":null},"itemsBackwardsCursor":"items"}));
    let items = Page { method:"thread/items/list", turn:None, cursor:Some("items".into()), descending:true };
    history.page(&mut view, &items, &json!({"data":[{"turnId":"t2","item":item("c")},{"turnId":"t1","item":item("b")},{"turnId":"t1","item":item("a")}],"nextCursor":null,"backwardsCursor":null}));
    history.page(&mut view, &page(true), &json!({"data":[turn("t2", &["c"]),turn("t1", &["a","b"])],"nextCursor":null,"backwardsCursor":null}));
    history.finish(&mut view);
    assert_eq!(ids(&view), ["a","b","c","d","e"]);
    assert_eq!(view.events.iter().map(|event| event.seq).collect::<Vec<_>>(), [1,2,3,4,5]);
    assert!(!view.history_truncated);
    assert!(view.events.iter().all(|event| event.run.is_none()));
}

#[test]
fn history_protocol_identity_is_scoped_to_its_turn() {
    let mut view = SessionView::new("/fixture".into());
    let mut history = History::default();
    history.seed(&mut view, &json!({"thread":{"turns":[turn("first", &["same-id"]),turn("second", &["same-id"])]}}));
    history.finish(&mut view);
    assert_eq!(ids(&view), ["same-id", "same-id"]);
}

#[test]
fn history_protocol_handles_empty_malformed_and_repeated_cursors() {
    let mut view = SessionView::new("/fixture".into());
    let mut history = History::default();
    history.seed(&mut view, &json!({"thread":{"turns":[turn("t", &["kept"])]},"turnsBackwardsCursor":"repeat"}));
    let first = history.pages.pop_front().unwrap();
    history.page(&mut view, &first, &json!({"data":[],"nextCursor":"after-empty","backwardsCursor":null}));
    assert!(!view.history_truncated);
    let next = history.pages.pop_front().unwrap();
    assert_eq!(next.cursor.as_deref(), Some("after-empty"));
    history.page(&mut view, &next, &json!({"data":[{"id":"malformed"},turn("t", &["kept"])],"nextCursor":"repeat","backwardsCursor":42}));
    assert!(view.history_truncated);
    assert!(history.pages.is_empty());
    history.finish(&mut view);
    assert_eq!(ids(&view), ["kept"]);
}

#[test]
fn history_protocol_overlapping_cursor_streams_may_converge_without_a_cycle() {
    let mut view = SessionView::new("/fixture".into());
    let mut history = History::default();
    history.seed(&mut view, &json!({"thread":{"turns":[]},"initialTurnsPage":{"data":[],"nextCursor":"shared","backwardsCursor":null},"turnsBackwardsCursor":"inclusive"}));
    let shared = history.pages.pop_front().unwrap();
    history.page(&mut view, &shared, &json!({"data":[turn("t", &["kept"])],"nextCursor":null,"backwardsCursor":null}));
    let inclusive = history.pages.pop_front().unwrap();
    history.page(&mut view, &inclusive, &json!({"data":[turn("t", &["kept"])],"nextCursor":"shared","backwardsCursor":null}));
    assert!(history.pages.is_empty());
    assert!(!view.history_truncated);
    history.finish(&mut view);
    assert_eq!(ids(&view), ["kept"]);
}

#[test]
fn history_protocol_item_summary_fetches_turn_scoped_full_items() {
    let mut history = History::default();
    let mut view = SessionView::new("/fixture".into());
    history.seed(&mut view, &json!({"thread":{"turns":[{"id":"t","items":[],"itemsView":"notLoaded"}]}}));
    let page = history.pages.pop_front().unwrap();
    assert_eq!(page.params("thread"), json!({"threadId":"thread","turnId":"t","cursor":null,"limit":128,"sortDirection":"asc"}));
    history.page(&mut view, &page, &json!({"data":[{"turnId":"foreign","item":item("bad")},{"turnId":"t","item":item("good")}],"nextCursor":null,"backwardsCursor":null}));
    history.finish(&mut view);
    assert_eq!(ids(&view), ["good"]);
    assert!(view.history_truncated);
}

#[test]
fn history_protocol_full_items_upgrade_summaries_and_preserve_entry_timestamps() {
    let mut history = History::default();
    let mut view = SessionView::new("/fixture".into());
    history.seed(&mut view, &json!({"thread":{"turns":[{"id":"t","itemsView":"summary","items":[{"id":"command","type":"commandExecution","command":"false"}]}]}}));
    let page = history.pages.pop_front().unwrap();
    history.page(&mut view, &page, &json!({"data":[{"turnId":"t","startedAtMs":10,"completedAtMs":20,"item":{"id":"command","type":"commandExecution","command":"false","status":"failed","aggregatedOutput":"native failure","startedAtMs":11}}],"nextCursor":null,"backwardsCursor":null}));
    history.finish(&mut view);
    assert_eq!(view.events.len(), 1);
    let item = &view.events[0].data;
    assert_eq!(item["status"], "failed");
    assert_eq!(item["aggregatedOutput"], "native failure");
    assert_eq!(item["startedAtMs"], 11);
    assert_eq!(item["completedAtMs"], 20);
    assert!(!view.history_truncated);
    view.push("agentMessage", Some("after upgrade".into()), Value::Null);
    assert_eq!(view.events.len(), 2, "summary upgrades keep SessionView byte accounting intact");
}

#[test]
fn history_protocol_event_and_byte_budgets_keep_partial_history() {
    let mut history = History::default();
    let mut view = SessionView::new("/fixture".into());
    let items: Vec<Value> = (0..=history::MAX_EVENTS).map(|index| item(&index.to_string())).collect();
    history.seed(&mut view, &json!({"thread":{"turns":[{"id":"t","items":items}]}}));
    assert_eq!(view.events.len(), history::MAX_EVENTS);
    assert!(history.exhausted && view.history_truncated);
    let mut history = History::default();
    let mut view = SessionView::new("/fixture".into());
    let huge = json!({"id":"huge","type":"agentMessage","text":"x".repeat(history::MAX_BYTES / 2)});
    history.seed(&mut view, &json!({"thread":{"turns":[{"id":"t","items":[item("kept"), huge]}]}}));
    assert_eq!(ids(&view), ["kept"]);
    assert!(history.exhausted && view.history_truncated);
}

#[test]
fn permissions_adapter_expires_foreign_and_conflicting_native_requests() {
    let mut view = SessionView::new("/fixture".into());
    view.native_id = Some("thread".into());
    view.status = "running".into();
    view.active_run = Some("run".into());
    let mut state = TurnState { id:Some("turn".into()), ..Default::default() };
    let request = json!({"id":7,"method":"item/permissions/requestApproval","params":{"threadId":"thread","turnId":"turn","itemId":"item","cwd":"/fixture","startedAtMs":1,"permissions":{"network":{"enabled":true}}}});
    for (thread, turn) in [("old-thread","turn"),("thread","old-turn")] {
        let mut expired = request.clone();
        expired["params"]["threadId"] = json!(thread);
        expired["params"]["turnId"] = json!(turn);
        assert_eq!(CodexSession::apply_event(&mut view, &mut state, expired).unwrap()["error"]["code"], -32600);
    }
    let mut malformed = request.clone();
    malformed["params"]["permissions"] = json!({"future":true});
    assert_eq!(CodexSession::apply_event(&mut view, &mut state, malformed).unwrap()["error"]["code"], -32602);
    assert!(view.pending.is_empty());
    assert!(CodexSession::apply_event(&mut view, &mut state, request.clone()).is_none());
    let public = view.pending[0].id.clone();
    assert!(CodexSession::valid_response(&view.pending[0], &json!({"scope":"turn","permissions":{}})));
    assert!(!CodexSession::valid_response(&view.pending[0], &json!({"decision":"accept"})));
    state.requests.get_mut("7").unwrap().answered = true;
    view.pending.clear();
    CodexSession::apply_event(&mut view, &mut state, request.clone());
    assert!(view.pending.is_empty(), "a repeated answered native ID cannot recreate a control");
    assert_eq!(state.requests["7"].public_id, public);
    let mut conflict = request;
    conflict["params"]["permissions"] = json!({"network":{"enabled":false}});
    CodexSession::apply_event(&mut view, &mut state, conflict);
    assert_eq!(view.status, "unknown");
    assert_eq!(view.error.as_deref(), Some("request_conflict"));
}

#[cfg(unix)]
mod fixture {
    use super::*;
    use crate::{AgentManager, Engine};
    use tokio_util::sync::CancellationToken;

    struct Fixture { root: std::path::PathBuf, manager:AgentManager, id:String, task:String }
    impl Fixture {
        async fn new() -> Self {
            let root = std::env::temp_dir().join(format!("dgos-codex-history-{}",uuid::Uuid::new_v4()));
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
*'"method":"thread/start"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"history-thread","turns":[]}}}\n' "$rpc" "$PWD";;
*'"method":"thread/resume"'*)
 printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"history-thread","turns":[]},"initialTurnsPage":{"data":[{"id":"t3","itemsView":"full","items":[{"id":"d","type":"agentMessage","text":"same"},{"id":"e","type":"agentMessage","text":"same"}]}],"nextCursor":"more-turns","backwardsCursor":"newer-turns"},"turnsBackwardsCursor":"older-turns","itemsBackwardsCursor":"older-items"}}\n' "$rpc" "$PWD";;
*'"method":"thread/turns/list"'*|*'"method":"thread/items/list"'*)
 if [ -f close-history ]; then exit 0; fi
 if [ -f hold-history ]; then printf waiting > history-waiting; continue; fi
 if [ -f page-error ]; then printf '{"id":%s,"error":{"code":-1,"message":"fixture history failure"}}\n' "$rpc"; continue; fi
 if [ -f page-budget ]; then
  number=${line#*\"cursor\":\"}; number=${number%%\"*}
  case "$number" in budget-*) number=${number#budget-};; *) number=0;; esac
  number=$((number + 1))
  printf '{"id":%s,"result":{"data":[],"nextCursor":"budget-%s","backwardsCursor":null}}\n' "$rpc" "$number"
  continue
 fi
 case "$line" in
 *'"cursor":"more-turns"'*) printf '{"id":%s,"result":{"data":[{"id":"t3","items":[{"id":"d","type":"agentMessage","text":"same"},{"id":"e","type":"agentMessage","text":"same"}]},{"id":"t2","items":[{"id":"c","type":"agentMessage","text":"same"}]}],"nextCursor":"empty-turns","backwardsCursor":null}}\n' "$rpc";;
 *'"cursor":"empty-turns"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":"oldest-turns","backwardsCursor":null}}\n' "$rpc";;
 *'"cursor":"oldest-turns"'*) printf '{"id":%s,"result":{"data":[{"id":"t1","items":[{"id":"a","type":"agentMessage","text":"same"},{"id":"b","type":"agentMessage","text":"same"}]}],"nextCursor":null,"backwardsCursor":null}}\n' "$rpc";;
 *'"cursor":"older-items"'*) printf '{"id":%s,"result":{"data":[{"turnId":"t1","item":{"id":"b","type":"agentMessage","text":"same"}},{"turnId":"t1","item":{"id":"a","type":"agentMessage","text":"same"}}],"nextCursor":null,"backwardsCursor":null}}\n' "$rpc";;
 *) printf '{"id":%s,"result":{"data":[],"nextCursor":null,"backwardsCursor":null}}\n' "$rpc";;
 esac;;
*'"method":"turn/start"'*)
 printf '{"id":%s,"result":{"turn":{"id":"permission-turn"}}}\n' "$rpc"
 printf '%s\n' '{"id":"permission","method":"item/permissions/requestApproval","params":{"threadId":"history-thread","turnId":"permission-turn","itemId":"permission-item","cwd":"/fixture","startedAtMs":1,"permissions":{"network":{"enabled":true}}}}';;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
            let manager = crate::test_support::manager(&root).await;
            let task = crate::test_support::task(&manager,&root).await;
            let id = uuid::Uuid::new_v4().to_string();
            let options = SessionOptions { workspace:root.to_string_lossy().into(), resume_id:None, executable:Some(executable.to_string_lossy().into()), model:None, trust_workspace:true };
            let create = crate::test_support::create(&manager,"alice",&task,options).await;
            manager.open_session("alice",Engine::Codex,id.clone(),create,Some(task.clone()),CancellationToken::new()).await.unwrap();
            Self { root,manager,id,task }
        }
        fn protocol(&self) -> Vec<Value> { std::fs::read_to_string(self.root.join("protocol.log")).unwrap().lines().map(|line|serde_json::from_str(line).unwrap()).collect() }
        async fn close(self) {
            assert!(self.manager.shutdown().await);
            std::fs::remove_dir_all(self.root).unwrap();
        }
    }

    #[tokio::test]
    async fn codex_history_protocol_smoke_existing_adapter_fixture() {
        let f = Fixture::new().await;
        let old = f.manager.snapshot("alice",&f.id).await.unwrap().generation;
        let (_, old_backend) = f.manager.owned("alice", &f.id).await.unwrap();
        let crate::manager::Backend::Codex(old_backend) = old_backend else { unreachable!() };
        assert!(f.manager.close_session("alice",&f.id).await.unwrap());
        let restored = f.manager.resume_session("alice",&f.id,true,CancellationToken::new()).await.unwrap();
        assert_ne!(restored.generation, old);
        assert_eq!(restored.view.native_id.as_deref(), Some("history-thread"));
        assert_eq!(std::fs::canonicalize(&restored.view.workspace).unwrap(), std::fs::canonicalize(&f.root).unwrap());
        assert_eq!(restored.view.status,"idle");
        assert_eq!(ids(&restored.view), ["a","b","c","d","e"]);
        assert!(!restored.view.history_truncated);
        old_backend.event(json!({"method":"item/completed","params":{"threadId":"history-thread","turnId":"stale-turn","item":item("stale")}})).await;
        old_backend.event(json!({"method":"host/closed"})).await;
        let current = f.manager.snapshot("alice", &f.id).await.unwrap();
        assert_eq!(current.generation, restored.generation);
        assert_eq!(ids(&current.view), ["a","b","c","d","e"]);
        assert_eq!(current.view.status, "idle");
        assert_eq!(f.manager.send("alice",&f.id,&old,uuid::Uuid::new_v4().to_string(),"never replay".into(),None,vec![]).await.unwrap_err(),"stale_generation");
        let protocol = f.protocol();
        assert!(!protocol.iter().any(|request|request["method"] == "turn/start"));
        let resume = protocol.iter().find(|request|request["method"] == "thread/resume").unwrap();
        assert_eq!(resume["params"]["threadId"],"history-thread");
        assert_eq!(resume["params"]["excludeTurns"],true);
        assert_eq!(resume["params"]["initialTurnsPage"],json!({"limit":128,"sortDirection":"desc","itemsView":"full"}));
        for (cursor, direction, method) in [("older-turns","desc","thread/turns/list"),("older-items","desc","thread/items/list"),("newer-turns","asc","thread/turns/list"),("oldest-turns","desc","thread/turns/list")] {
            let request = protocol.iter().find(|request|request["params"]["cursor"] == cursor).unwrap();
            assert_eq!(request["method"],method);
            assert_eq!(request["params"]["sortDirection"],direction);
            assert_eq!(request["params"]["threadId"],"history-thread");
            assert_eq!(request["params"]["limit"],128);
            if method == "thread/turns/list" { assert_eq!(request["params"]["itemsView"],"full"); }
        }
        f.close().await;
    }

    #[tokio::test]
    async fn codex_history_protocol_page_errors_and_page_budget_are_not_native_restore_failures() {
        for mode in ["page-error","page-budget"] {
            let f = Fixture::new().await;
            assert!(f.manager.close_session("alice",&f.id).await.unwrap());
            std::fs::write(f.root.join(mode),"").unwrap();
            let restored = f.manager.resume_session("alice",&f.id,true,CancellationToken::new()).await.unwrap();
            assert_eq!(restored.view.status,"idle");
            assert_eq!(ids(&restored.view),["d","e"]);
            assert!(restored.view.history_truncated);
            let pages = f.protocol().iter().filter(|request|matches!(request["method"].as_str(),Some("thread/turns/list" | "thread/items/list"))).count();
            assert!(pages <= history::MAX_PAGES);
            if mode == "page-budget" { assert_eq!(pages,history::MAX_PAGES); }
            assert!(f.manager.list_development_runs("alice",&f.task).await.unwrap().is_empty());
            f.close().await;
        }
    }

    #[tokio::test]
    async fn codex_history_protocol_interrupted_open_cannot_publish_or_replace_saved_thread() {
        let f = Fixture::new().await;
        assert!(f.manager.close_session("alice",&f.id).await.unwrap());
        std::fs::write(f.root.join("close-history"),"").unwrap();
        assert!(f.manager.resume_session("alice",&f.id,true,CancellationToken::new()).await.is_err());
        assert_eq!(f.manager.list("alice").await[0].native_id.as_deref(),Some("history-thread"));
        std::fs::remove_file(f.root.join("close-history")).unwrap();
        let current = f.manager.resume_session("alice",&f.id,true,CancellationToken::new()).await.unwrap();
        assert_eq!(ids(&current.view),["a","b","c","d","e"]);
        assert_eq!(f.manager.snapshot("alice",&f.id).await.unwrap().generation,current.generation);
        assert!(!f.protocol().iter().any(|request|request["method"] == "turn/start"));
        f.close().await;
    }

    #[tokio::test]
    async fn codex_history_protocol_revoked_opening_lease_cannot_publish_a_generation() {
        let f = Fixture::new().await;
        assert!(f.manager.close_session("alice", &f.id).await.unwrap());
        std::fs::write(f.root.join("hold-history"), "").unwrap();
        let lease = CancellationToken::new();
        let manager = f.manager.clone();
        let id = f.id.clone();
        let opening_lease = lease.clone();
        let opening = tokio::spawn(async move { manager.resume_session("alice", &id, true, opening_lease).await });
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !f.root.join("history-waiting").exists() { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
        }).await.unwrap();
        lease.cancel();
        assert!(tokio::time::timeout(std::time::Duration::from_secs(5), opening).await.unwrap().unwrap().is_err());
        assert_eq!(f.manager.list("alice").await[0].native_id.as_deref(), Some("history-thread"));
        std::fs::remove_file(f.root.join("hold-history")).unwrap();
        let current = f.manager.resume_session("alice", &f.id, true, CancellationToken::new()).await.unwrap();
        assert_eq!(ids(&current.view), ["a","b","c","d","e"]);
        assert_eq!(f.manager.snapshot("alice", &f.id).await.unwrap().generation, current.generation);
        assert!(!f.protocol().iter().any(|request| request["method"] == "turn/start"));
        f.close().await;
    }

    #[tokio::test]
    async fn codex_history_protocol_expired_deadline_never_issues_a_page() {
        let f = Fixture::new().await;
        let (_, backend) = f.manager.owned("alice", &f.id).await.unwrap();
        let crate::manager::Backend::Codex(session) = backend else { unreachable!() };
        session.view.lock().await.status = "starting".into();
        session.restore_history_until("history-thread", &json!({"thread":{"turns":[turn("t", &["kept"])]},"turnsBackwardsCursor":"never-request"}), tokio::time::Instant::now()).await;
        let view = session.snapshot().await;
        assert_eq!(ids(&view), ["kept"]);
        assert!(view.history_truncated);
        assert!(!f.protocol().iter().any(|request|request["method"] == "thread/turns/list"));
        session.view.lock().await.status = "idle".into();
        f.close().await;
    }

    #[tokio::test]
    async fn permissions_adapter_duplicate_answer_never_writes_twice() {
        let f = Fixture::new().await;
        let view = f.manager.snapshot("alice",&f.id).await.unwrap();
        f.manager.send("alice", &f.id, &view.generation, uuid::Uuid::new_v4().to_string(), "request permissions".into(), None, vec![]).await.unwrap();
        let pending = tokio::time::timeout(std::time::Duration::from_secs(3),async {
            loop {
                if let Some(pending) = f.manager.snapshot("alice",&f.id).await.unwrap().view.pending.first().cloned() { break pending; }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        assert_eq!(pending.kind,"item/permissions/requestApproval");
        let answer = json!({"permissions":{},"scope":"turn"});
        f.manager.respond("alice",&f.id,&view.generation,pending.id.clone(),answer.clone()).await.unwrap();
        assert!(f.manager.respond("alice",&f.id,&view.generation,pending.id,answer).await.is_err());
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !f.protocol().iter().any(|request|request["id"] == "permission" && request.get("result").is_some()) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        assert!(f.manager.close_session("alice",&f.id).await.unwrap());
        assert_eq!(f.protocol().iter().filter(|request|request["id"] == "permission" && request.get("result").is_some()).count(),1);
        f.close().await;
    }
}
