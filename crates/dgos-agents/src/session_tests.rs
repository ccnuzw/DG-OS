#[cfg(unix)]
mod unix {
    use crate::{AgentManager, Engine, SessionOptions};
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    use tokio_util::sync::CancellationToken;
    #[tokio::test]
    async fn codex_approval_interrupt_resume_and_owner_isolation() {
        let root = std::env::temp_dir().join(format!("dgos-agent-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("codex");
        std::fs::write(&executable,r##"#!/bin/sh
while IFS= read -r line; do
case "$line" in
*'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"thread/start"'*|*'"method":"thread/resume"'*) printf '%s\n' '{"id":2,"result":{"thread":{"id":"thread-test","turns":[]}}}';;
*'"method":"turn/start"'*) printf '%s\n' '{"id":3,"result":{"turn":{"id":"turn-test"}}}' '{"method":"turn/started","params":{"turn":{"id":"turn-test"}}}' '{"id":"approve","method":"item/commandExecution/requestApproval","params":{"command":"echo allowed","availableDecisions":["accept","decline"]}}';;
*'"decision":"decline"'*) printf '%s\n' '{"method":"turn/completed","params":{"turn":{"id":"turn-test","status":"completed"}}}';;
*'"method":"turn/interrupt"'*) printf '%s\n' '{"id":4,"result":{}}' '{"method":"turn/completed","params":{"turn":{"id":"turn-test","status":"interrupted"}}}';;
esac
done
"##).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
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
            .open(
                "alice",
                Engine::Codex,
                id.clone(),
                options.clone(),
                Some(task_id.clone()),
                lease.clone(),
            )
            .await
            .unwrap();
        assert!(manager.snapshot("bob", &id).await.is_err());
        let project_id = manager.development_workspace("alice").await.unwrap().projects[0].id.clone();
        assert!(matches!(manager.update_project("alice", crate::projects::UpdateProjectInput { project_id: project_id.clone(), action: "unlink".into(), value: None }, lease.clone()).await, Err(e) if e == "project_busy"));
        let bob_project = manager.create_project("bob", crate::projects::CreateProjectInput { kind: "import".into(), name: "Other owner same path".into(), path: root.to_string_lossy().into(), source: None }, CancellationToken::new()).await.unwrap();
        let bob_task = manager.save_development_task("bob", crate::projects::SaveTaskInput { id: None, project_id: bob_project.id, goal: "Other task".into(), requirements: "Isolation".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
        assert!(matches!(manager.open("bob", Engine::Codex, uuid::Uuid::new_v4().to_string(), options.clone(), Some(bob_task.id), CancellationToken::new()).await, Err(e) if e == "workspace_busy"));
        assert!(manager.open("bob", Engine::Codex, id.clone(), options.clone(), None, CancellationToken::new()).await.is_err());
        let request = uuid::Uuid::new_v4().to_string();
        manager
            .send("alice", &id, request.clone(), "task".into())
            .await
            .unwrap();
        manager
            .send("alice", &id, request.clone(), "task".into())
            .await
            .unwrap();
        assert!(manager
            .send("alice", &id, request, "different".into())
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
            .respond("bob", &id, pending.id.clone(), json!({"decision":"accept"}))
            .await
            .is_err());
        assert!(manager
            .respond(
                "alice",
                &id,
                pending.id.clone(),
                json!({"decision":"bypass"})
            )
            .await
            .is_err());
        manager
            .respond(
                "alice",
                &id,
                pending.id.clone(),
                json!({"decision":"decline"}),
            )
            .await
            .unwrap();
        assert!(manager
            .respond("alice", &id, pending.id, json!({"decision":"accept"}))
            .await
            .is_err());
        assert!(manager.close_session("alice", &id).await.unwrap());
        let reloaded = AgentManager::new(root.join("index.json"), crate::test_support::repository(&root).await);
        reloaded.load().await.unwrap();
        assert_eq!(reloaded.list("alice").await.len(), 1);
        assert!(reloaded.list("bob").await.is_empty());
        reloaded
            .open("alice", Engine::Codex, id.clone(), options, Some(task_id), lease.clone())
            .await
            .unwrap();
        reloaded
            .send(
                "alice",
                &id,
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
}
