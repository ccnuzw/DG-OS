#[cfg(unix)]
mod unix {
    use crate::{Engine, SessionOptions};
    use serde_json::json;
    use tokio_util::sync::CancellationToken;
    #[tokio::test]
    async fn codex_approval_interrupt_resume_and_owner_isolation() {
        let root = std::env::temp_dir().join(format!("dgos-agent-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("codex");
        crate::test_support::codex_fixture(&executable, &r##"
while IFS= read -r line; do
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"thread/start"'*|*'"method":"thread/resume"'*) printf '{"id":3,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"thread-test","turns":[]}}}\n' "$PWD";;
*'"method":"turn/start"'*) printf '%s\n' '{"id":4,"result":{"turn":{"id":"turn-test"}}}' '{"method":"turn/started","params":{"threadId":"thread-test","turn":{"id":"turn-test"}}}' '{"id":"approve","method":"item/commandExecution/requestApproval","params":{"threadId":"thread-test","turnId":"turn-test","itemId":"command","command":"echo allowed","availableDecisions":["accept","decline"]}}';;
*'"decision":"decline"'*) printf '%s\n' '{"method":"turn/completed","params":{"threadId":"thread-test","turn":{"id":"turn-test","status":"completed"}}}';;
*'"method":"turn/interrupt"'*) printf '%s\n' '{"id":5,"result":{}}' '{"method":"turn/completed","params":{"threadId":"thread-test","turn":{"id":"turn-test","status":"interrupted"}}}';;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
        let manager = crate::test_support::manager(&root).await;
        let task_id = crate::test_support::task(&manager, &root).await;
        let lease = CancellationToken::new();
        let id = uuid::Uuid::new_v4().to_string();
        let options = SessionOptions {
            workspace: root.to_string_lossy().into(),
            resume_id: None,
            executable: Some(executable.to_string_lossy().into()),
            model: None,
            trust_workspace: true,
        };
        manager
            .open_session(
                "alice",
                Engine::Codex,
                id.clone(),
                crate::test_support::create(&manager, "alice", &task_id, options.clone()).await,
                Some(task_id.clone()),
                lease.clone(),
            )
            .await
            .unwrap();
        let generation = manager.snapshot("alice", &id).await.unwrap().generation;
        assert!(manager.snapshot("bob", &id).await.is_err());
        let project_id = manager.development_workspace("alice").await.unwrap().projects[0].id.clone();
        assert!(matches!(manager.update_project("alice", crate::projects::UpdateProjectInput { project_id: project_id.clone(), action: "unlink".into(), value: None }, lease.clone()).await, Err(e) if e == "project_busy"));
        let bob_project = manager.create_project("bob", crate::projects::CreateProjectInput { kind: "import".into(), name: "Other owner same path".into(), path: root.to_string_lossy().into(), source: None }, CancellationToken::new()).await.unwrap();
        let bob_task = manager.save_development_task("bob", crate::projects::SaveTaskInput { id: None, project_id: bob_project.id, goal: "Other task".into(), requirements: "Isolation".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
        assert!(matches!(manager.open_session("bob", Engine::Codex, uuid::Uuid::new_v4().to_string(), crate::test_support::create(&manager, "bob", &bob_task.id, options.clone()).await, Some(bob_task.id.clone()), CancellationToken::new()).await, Err(e) if e == "workspace_busy"));
        assert!(manager.open_session("bob", Engine::Codex, id.clone(), crate::test_support::create(&manager, "bob", &bob_task.id, options.clone()).await, None, CancellationToken::new()).await.is_err());
        let request = uuid::Uuid::new_v4().to_string();
        manager
            .send("alice", &id, &generation, request.clone(), "task".into())
            .await
            .unwrap();
        manager
            .send("alice", &id, &generation, request.clone(), "task".into())
            .await
            .unwrap();
        assert!(manager
            .send("alice", &id, &generation, request, "different".into())
            .await
            .is_err());
        let pending = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let view = manager.snapshot("alice", &id).await.unwrap();
                if !view.view.pending.is_empty() {
                    break view.view.pending[0].clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(manager
            .respond("bob", &id, &generation, pending.id.clone(), json!({"decision":"accept"}))
            .await
            .is_err());
        assert!(manager
            .respond(
                "alice",
                &id,
                &generation,
                pending.id.clone(),
                json!({"decision":"bypass"})
            )
            .await
            .is_err());
        manager
            .respond(
                "alice",
                &id,
                &generation,
                pending.id.clone(),
                json!({"decision":"decline"}),
            )
            .await
            .unwrap();
        assert!(manager
            .respond("alice", &id, &generation, pending.id, json!({"decision":"accept"}))
            .await
            .is_err());
        assert!(manager.close_session("alice", &id).await.unwrap());
        let reloaded = crate::test_support::manager(&root).await;
        assert_eq!(reloaded.list("alice").await.len(), 1);
        assert!(reloaded.list("bob").await.is_empty());
        reloaded
            .resume_session("alice", &id, true, lease.clone())
            .await
            .unwrap();
        let old_generation = generation;
        let current_generation = reloaded.snapshot("alice", &id).await.unwrap().generation;
        assert_ne!(old_generation, current_generation);
        let stale_request = uuid::Uuid::new_v4().to_string();
        assert_eq!(reloaded.send("alice", &id, &old_generation, stale_request.clone(), "must not send".into()).await.unwrap_err(), "stale_generation");
        assert!(reloaded.inner.repository.run("alice", &stale_request).await.unwrap().is_none());
        assert_eq!(reloaded.respond("alice", &id, &old_generation, "old-request".into(), json!({"decision":"accept"})).await.unwrap_err(), "stale_generation");
        let generation = reloaded.snapshot("alice", &id).await.unwrap().generation;
        reloaded
            .send(
                "alice",
                &id,
                &generation,
                uuid::Uuid::new_v4().to_string(),
                "interrupt me".into(),
            )
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if !reloaded
                    .snapshot("alice", &id)
                    .await
                    .unwrap()
                    .view
                    .pending
                    .is_empty()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        reloaded.interrupt("alice", &id).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if reloaded.snapshot("alice", &id).await.unwrap().view.status == "idle" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        lease.cancel();
        assert!(reloaded.snapshot("alice", &id).await.is_err());
        assert!(reloaded.shutdown().await);
        assert!(manager.shutdown().await);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn codex_successful_streaming_rounds_restore_history_without_text_deduplication() {
        let root = std::env::temp_dir().join(format!("dgos-codex-rounds-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("codex");
        crate::test_support::codex_fixture(&executable, &r##"
round=0
while IFS= read -r line; do
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
*'"method":"thread/start"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"rounds-thread","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"thread/resume"'*)
 round=2
 printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"rounds-thread","turns":[{"id":"turn-1","status":"completed","items":[{"type":"userMessage","id":"user-1","content":[{"type":"text","text":"same prompt"}]},{"type":"agentMessage","id":"answer-1","text":"hello world"}]},{"id":"turn-2","status":"completed","items":[{"type":"userMessage","id":"user-2","content":[{"type":"text","text":"same prompt"}]},{"type":"agentMessage","id":"answer-2","text":"hello world"}]}]}}}\n' "$request" "$PWD";;
*'"method":"turn/start"'*)
 round=$((round+1))
 printf '{"method":"turn/completed","params":{"threadId":"rounds-thread","turn":{"id":"old-turn","status":"completed"}}}\n'
 printf '{"method":"turn/started","params":{"threadId":"rounds-thread","turn":{"id":"turn-%s"}}}\n' "$round"
 printf '{"method":"item/completed","params":{"threadId":"rounds-thread","turnId":"turn-%s","item":{"id":"user-%s","type":"userMessage","content":[{"type":"text","text":"same prompt"}]}}}\n' "$round" "$round"
 printf '{"method":"item/agentMessage/delta","params":{"threadId":"rounds-thread","turnId":"turn-%s","itemId":"answer-%s","delta":"hello "}}\n' "$round" "$round"
 printf '{"method":"item/agentMessage/delta","params":{"threadId":"rounds-thread","turnId":"turn-%s","itemId":"answer-%s","delta":"world"}}\n' "$round" "$round"
 printf '{"method":"item/completed","params":{"threadId":"rounds-thread","turnId":"turn-%s","item":{"id":"answer-%s","type":"agentMessage","text":"hello world"}}}\n' "$round" "$round"
 printf '{"method":"turn/completed","params":{"threadId":"rounds-thread","turn":{"id":"turn-%s","status":"completed"}}}\n' "$round"
 printf '{"id":%s,"result":{"turn":{"id":"turn-%s"}}}\n' "$request" "$round";;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
        let manager = crate::test_support::manager(&root).await;
        let task = crate::test_support::task(&manager, &root).await;
        let lease = CancellationToken::new();
        let id = uuid::Uuid::new_v4().to_string();
        let options = SessionOptions { workspace: root.to_string_lossy().into(), resume_id: None, executable: Some(executable.to_string_lossy().into()), model: None, trust_workspace: true };
        manager.open_session("alice", Engine::Codex, id.clone(), crate::test_support::create(&manager, "alice", &task, options).await, Some(task.clone()), lease.clone()).await.unwrap();
        let generation = manager.snapshot("alice", &id).await.unwrap().generation;
        for _ in 0..2 {
            let request = uuid::Uuid::new_v4().to_string();
            manager.send("alice", &id, &generation, request.clone(), "same prompt".into()).await.unwrap();
            let view = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    let view = manager.snapshot("alice", &id).await.unwrap().view;
                    if view.status == "idle" { break view; }
                    tokio::task::yield_now().await;
                }
            }).await.unwrap();
            assert_eq!(view.events.iter().filter(|event| event.run.as_deref() == Some(request.as_str()) && event.kind == "item/agentMessage/delta").count(), 2);
            assert_eq!(view.events.iter().filter(|event| event.run.as_deref() == Some(request.as_str()) && event.kind == "turn/completed").count(), 1);
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    if manager.list_development_runs("alice", &task).await.unwrap().iter().any(|run| run.id == request && run.status == "completed") { break; }
                    tokio::task::yield_now().await;
                }
            }).await.unwrap();
        }
        let before_close = manager.snapshot("alice", &id).await.unwrap().view;
        assert_eq!(before_close.events.iter().filter(|event| event.kind == "userMessage").count(), 2);
        assert!(manager.close_session("alice", &id).await.unwrap());
        assert!(manager.shutdown().await);
        let restored = crate::test_support::manager(&root).await;
        restored.resume_session("alice", &id, true, CancellationToken::new()).await.unwrap();
        let generation = restored.snapshot("alice", &id).await.unwrap().generation;
        let history = restored.snapshot("alice", &id).await.unwrap().view;
        assert_eq!(history.native_id.as_deref(), Some("rounds-thread"));
        assert_eq!(history.events.iter().filter(|event| event.kind == "userMessage").count(), 2);
        assert_eq!(history.events.iter().filter(|event| event.kind == "agentMessage").count(), 2);
        restored.send("alice", &id, &generation, uuid::Uuid::new_v4().to_string(), "same prompt".into()).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let view = restored.snapshot("alice", &id).await.unwrap().view;
                if view.status == "idle" {
                    assert_eq!(view.events.iter().filter(|event| event.kind == "userMessage").count(), 3);
                    break;
                }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        assert!(restored.shutdown().await);
        std::fs::remove_dir_all(root).unwrap();
    }
}
