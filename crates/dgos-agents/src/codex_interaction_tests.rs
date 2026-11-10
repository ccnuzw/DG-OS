use super::*;
use crate::session::{SteeringState, AgentTurnSettings, CollaborationMode};
use serde_json::{json, Value};

#[cfg(unix)]
#[tokio::test]
async fn guarded_steering_is_single_dispatch_generation_bound_and_supervised() {
    let root = std::env::temp_dir().join(format!("dgos-steering-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let executable = root.join("codex");
    crate::test_support::codex_fixture(&executable, &r##"
while IFS= read -r line; do
printf '%s\n' "$line" >> protocol
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
*'"method":"collaborationMode/list"'*) printf '{"id":%s,"result":{"data":[{"name":"Default","mode":"default","model":"untrusted-preset"},{"name":"Plan","mode":"plan","model":"untrusted-preset","reasoning_effort":"high"}]}}\n' "$request";;
CONFIG_REPLY
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
*'"method":"thread/start"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","approvalPolicy":"on-request","sandbox":{"type":"workspaceWrite"},"reasoningEffort":"medium","thread":{"id":"steering-thread","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[{"model":"fixture-model","defaultReasoningEffort":"medium","supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"medium"}]}],"nextCursor":null}}\n' "$request";;
*'"method":"turn/start"'*) printf '{"id":%s,"result":{"turn":{"id":"steering-turn"}}}\n' "$request";;
*'"method":"turn/steer"'*)
case "$line" in
*'"text":"ambiguous"'*) printf '{"id":%s,"result":{"turnId":"foreign-turn"}}\n' "$request";;
*'"text":"reject"'*) printf '{"id":%s,"error":{"code":-32000,"message":"refused"}}\n' "$request";;
*'"text":"unsupported"'*) printf '{"id":%s,"error":{"code":-32601,"message":"unsupported"}}\n' "$request";;
*'"text":"delayed"'*) printf dispatched > dispatched; sleep 1; printf '{"id":%s,"result":{"turnId":"steering-turn"}}\n' "$request";;
*) printf '{"id":%s,"result":{"turnId":"steering-turn"}}\n' "$request";;
esac;;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
    let manager = crate::test_support::manager(&root).await;
    let task = crate::test_support::task(&manager, &root).await;
    let id = Uuid::new_v4().to_string();
    let options = SessionOptions { workspace: root.to_string_lossy().into_owned(), resume_id: None, executable: Some(executable.to_string_lossy().into_owned()), model: None, trust_workspace: true };
    let opened = manager.open_session("alice", Engine::Codex, id.clone(), crate::test_support::create(&manager, "alice", &task, options).await, Some(task), CancellationToken::new()).await.unwrap();
    let generation = opened.generation;
    assert!(opened.view.controls.unwrap().steering);
    let run = Uuid::new_v4().to_string();
    manager.send("alice", &id, &generation, run.clone(), "start".into(), Some(AgentTurnSettings { reasoning_effort: Some(Some("low".into())), permission_mode: None, collaboration_mode: Some(CollaborationMode::Plan) }), vec![], vec![], vec![]).await.unwrap();
    assert_eq!(manager.steer("bob", &id, &generation, run.clone(), Uuid::new_v4().to_string(), "wrong owner".into()).await.unwrap_err(), "session_not_found");
    assert_eq!(manager.steer("alice", &id, &Uuid::new_v4().to_string(), run.clone(), Uuid::new_v4().to_string(), "old generation".into()).await.unwrap_err(), "stale_generation");
    assert_eq!(manager.steer("alice", &id, &generation, Uuid::new_v4().to_string(), Uuid::new_v4().to_string(), "old run".into()).await.unwrap_err(), "not_running");
    assert_eq!(manager.steer("alice", &id, &generation, run.clone(), "not-a-uuid".into(), "invalid".into()).await.unwrap_err(), "invalid_request_id");
    let request = Uuid::new_v4().to_string();
    let receipt = manager.steer("alice", &id, &generation, run.clone(), request.clone(), "applied".into()).await.unwrap();
    assert_eq!(receipt.state, SteeringState::Applied);
    assert_eq!(manager.steer("alice", &id, &generation, run.clone(), request.clone(), "applied".into()).await.unwrap(), receipt);
    assert_eq!(manager.steer("alice", &id, &generation, run.clone(), request.clone(), "different".into()).await.unwrap_err(), "request_conflict");
    let ambiguous = Uuid::new_v4().to_string();
    let receipt = manager.steer("alice", &id, &generation, run.clone(), ambiguous.clone(), "ambiguous".into()).await.unwrap();
    assert_eq!(receipt.state, SteeringState::Unknown);
    assert_eq!(manager.steer("alice", &id, &generation, run.clone(), ambiguous.clone(), "ambiguous".into()).await.unwrap(), receipt);
    let rejected = Uuid::new_v4().to_string();
    assert_eq!(manager.steer("alice", &id, &generation, run.clone(), rejected.clone(), "reject".into()).await.unwrap_err(), "engine_rejected");
    assert_eq!(manager.steer("alice", &id, &generation, run.clone(), rejected, "reject".into()).await.unwrap_err(), "engine_rejected");
    let delayed = Uuid::new_v4().to_string();
    let worker = {
        let manager = manager.clone(); let id = id.clone(); let generation = generation.clone(); let run = run.clone(); let request = delayed.clone();
        tokio::spawn(async move { manager.steer("alice", &id, &generation, run, request, "delayed".into()).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !root.join("dispatched").exists() { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
    }).await.unwrap();
    worker.abort();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let view = manager.snapshot("alice", &id).await.unwrap().view;
            if view.events.iter().any(|event| event.kind == "userSteering" && event.data["requestId"] == delayed) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    let unsupported = Uuid::new_v4().to_string();
    assert_eq!(manager.steer("alice", &id, &generation, run.clone(), unsupported.clone(), "unsupported".into()).await.unwrap_err(), "steering_unsupported");
    assert_eq!(manager.steer("alice", &id, &generation, run.clone(), unsupported, "unsupported".into()).await.unwrap_err(), "steering_unsupported");
    assert_eq!(manager.steer("alice", &id, &generation, run.clone(), Uuid::new_v4().to_string(), "disabled".into()).await.unwrap_err(), "steering_unsupported");
    let view = manager.snapshot("alice", &id).await.unwrap().view;
    assert_eq!(view.status, "running");
    assert_eq!(view.active_run.as_deref(), Some(run.as_str()));
    assert!(!view.controls.unwrap().steering);
    assert_eq!(view.events.iter().filter(|event| event.kind == "userSteering").count(), 2);
    assert!(view.events.iter().any(|event| event.kind == "steeringReceipt" && event.data == json!({"state":"unknown","requestId":ambiguous,"run":run}) && event.run.as_deref() == Some(run.as_str())));
    let protocol: Vec<Value> = std::fs::read_to_string(root.join("protocol")).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    let steers: Vec<_> = protocol.iter().filter(|message| message["method"] == "turn/steer").collect();
    assert_eq!(steers.len(), 5);
    assert!(steers.iter().all(|message| message["params"]["expectedTurnId"] == "steering-turn" && message["params"]["threadId"] == "steering-thread"));
    let start = protocol.iter().find(|message| message["method"] == "turn/start").unwrap();
    assert_eq!(start["params"]["collaborationMode"], json!({"mode":"plan","settings":{"model":"fixture-model","reasoning_effort":"low","developer_instructions":null}}));
    assert!(protocol.iter().any(|message| message["method"] == "initialize" && message["params"]["capabilities"]["experimentalApi"] == true));
    assert!(manager.shutdown().await);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn terminal_projection_waits_for_late_confirmed_steering() {
    let root = std::env::temp_dir().join(format!("dgos-steering-terminal-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let executable = root.join("codex");
    crate::test_support::codex_fixture(&executable, &r##"
while IFS= read -r line; do
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
*'"method":"thread/start"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"terminal-thread","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request";;
*'"method":"turn/start"'*) printf '{"id":%s,"result":{"turn":{"id":"terminal-turn"}}}\n' "$request";;
*'"method":"turn/steer"'*) printf '%s\n' '{"method":"turn/completed","params":{"threadId":"terminal-thread","turn":{"id":"terminal-turn","status":"completed"}}}'; sleep 2; printf '{"id":%s,"result":{"turnId":"terminal-turn"}}\n' "$request";;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
    let manager = crate::test_support::manager(&root).await;
    let task = crate::test_support::task(&manager, &root).await;
    let id = Uuid::new_v4().to_string();
    let options = SessionOptions { workspace: root.to_string_lossy().into_owned(), resume_id: None, executable: Some(executable.to_string_lossy().into_owned()), model: None, trust_workspace: true };
    let opened = manager.open_session("alice", Engine::Codex, id.clone(), crate::test_support::create(&manager, "alice", &task, options).await, Some(task), CancellationToken::new()).await.unwrap();
    assert!(opened.view.controls.as_ref().unwrap().collaboration.is_none());
    assert!(opened.view.controls.as_ref().unwrap().steering);
    let generation = opened.generation;
    let run = Uuid::new_v4().to_string();
    manager.send("alice", &id, &generation, run.clone(), "start".into(), None, vec![], vec![], vec![]).await.unwrap();
    let request = Uuid::new_v4().to_string();
    let worker = {
        let manager = manager.clone(); let id = id.clone(); let generation = generation.clone(); let run = run.clone(); let request = request.clone();
        tokio::spawn(async move { manager.steer("alice", &id, &generation, run, request, "late confirmed".into()).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while manager.snapshot("alice", &id).await.unwrap().view.status != "idle" { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
    }).await.unwrap();
    assert!(manager.inner.run_projections.lock().await.contains_key(&(id.clone(), generation.clone())));
    assert!(!matches!(manager.inner.repository.run("alice", &run).await.unwrap().unwrap().status.as_str(), "completed" | "failed" | "cancelled" | "unknown"));
    assert_eq!(worker.await.unwrap().unwrap().state, SteeringState::Applied);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let stored = manager.inner.repository.run("alice", &run).await.unwrap().unwrap();
            if stored.status == "completed" {
                let events: Vec<crate::session::SessionEvent> = serde_json::from_str(&stored.events).unwrap();
                assert!(events.iter().any(|event| event.kind == "userSteering" && event.text.as_deref() == Some("late confirmed") && event.data["requestId"] == request));
                assert!(events.iter().any(|event| event.kind == "steeringReceipt" && event.data["state"] == "applied" && event.data["requestId"] == request));
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    assert!(manager.shutdown().await);
    std::fs::remove_dir_all(root).unwrap();
}
