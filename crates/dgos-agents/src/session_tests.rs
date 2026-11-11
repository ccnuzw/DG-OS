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
if [ -f native-provider ]; then IFS= read -r native_provider < native-provider; else native_provider=$fixture_provider; printf '%s\n' "$native_provider" > native-provider; fi
while IFS= read -r line; do
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request";;
*'"method":"thread/start"'*|*'"method":"thread/resume"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"'"$native_provider"'","thread":{"id":"thread-test","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"turn/start"'*) printf '{"id":%s,"result":{"turn":{"id":"turn-test"}}}\n' "$request"; printf '%s\n' '{"method":"turn/started","params":{"threadId":"thread-test","turn":{"id":"turn-test"}}}' '{"id":"approve","method":"item/commandExecution/requestApproval","params":{"threadId":"thread-test","turnId":"turn-test","itemId":"command","command":"echo allowed","availableDecisions":["accept","decline"]}}';;
*'"decision":"decline"'*) printf '%s\n' '{"method":"turn/completed","params":{"threadId":"thread-test","turn":{"id":"turn-test","status":"completed"}}}';;
*'"method":"turn/interrupt"'*) printf '{"id":%s,"result":{}}\n' "$request"; printf '%s\n' '{"method":"turn/completed","params":{"threadId":"thread-test","turn":{"id":"turn-test","status":"interrupted"}}}';;
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
        assert_eq!(manager.snapshot("alice", &id).await.unwrap().view.cli_version.as_deref(), Some("codex-cli 0.167.0"));
        assert!(manager.snapshot("bob", &id).await.is_err());
        let project_id = manager.development_workspace("alice").await.unwrap().projects[0].id.clone();
        assert!(matches!(manager.update_project("alice", crate::projects::UpdateProjectInput { project_id: project_id.clone(), action: "archive".into(), value: None }, lease.clone()).await, Err(e) if e == "project_busy"));
        let bob_project = manager.create_project("bob", crate::projects::CreateProjectInput { kind: "import".into(), name: "Other owner same path".into(), path: root.to_string_lossy().into(), source: None }, CancellationToken::new()).await.unwrap();
        let bob_task = manager.save_development_task("bob", crate::projects::SaveTaskInput { id: None, project_id: bob_project.id, goal: "Other task".into(), requirements: "Isolation".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
        assert!(matches!(manager.open_session("bob", Engine::Codex, uuid::Uuid::new_v4().to_string(), crate::test_support::create(&manager, "bob", &bob_task.id, options.clone()).await, Some(bob_task.id.clone()), CancellationToken::new()).await, Err(e) if e == "workspace_busy"));
        assert!(manager.open_session("bob", Engine::Codex, id.clone(), crate::test_support::create(&manager, "bob", &bob_task.id, options.clone()).await, None, CancellationToken::new()).await.is_err());
        let request = uuid::Uuid::new_v4().to_string();
        manager.send("alice", &id, &generation, request.clone(), "task".into(), None, vec![], vec![], vec![])
            .await
            .unwrap();
        manager.send("alice", &id, &generation, request.clone(), "task".into(), None, vec![], vec![], vec![])
            .await
            .unwrap();
        assert!(manager
            .send("alice", &id, &generation, request, "different".into(), None, vec![], vec![], vec![])
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
        // A restart must settle the old supervisor before another manager recovers its DB.
        assert!(manager.shutdown().await);
        let reloaded = crate::test_support::manager(&root).await;
        let lease = CancellationToken::new();
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
        assert_eq!(reloaded.send("alice", &id, &old_generation, stale_request.clone(), "must not send".into(), None, vec![], vec![], vec![]).await.unwrap_err(), "stale_generation");
        assert!(reloaded.inner.repository.run("alice", &stale_request).await.unwrap().is_none());
        assert_eq!(reloaded.respond("alice", &id, &old_generation, "old-request".into(), json!({"decision":"accept"})).await.unwrap_err(), "stale_generation");
        let generation = reloaded.snapshot("alice", &id).await.unwrap().generation;
        reloaded.send("alice", &id, &generation, uuid::Uuid::new_v4().to_string(), "interrupt me".into(), None, vec![], vec![], vec![])
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
        let active = reloaded.snapshot("alice", &id).await.unwrap();
        let run = active.view.active_run.as_deref().unwrap();
        assert_eq!(reloaded.interrupt("alice", &id, &old_generation, run).await.unwrap_err(), "stale_generation");
        assert_eq!(reloaded.interrupt("alice", &id, &active.generation, "previous-run").await.unwrap_err(), "not_running");
        reloaded.interrupt("alice", &id, &active.generation, run).await.unwrap();
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
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request";;
*'"method":"thread/start"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"'"$fixture_provider"'","thread":{"id":"rounds-thread","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"thread/resume"'*)
 round=2
 printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"'"$fixture_provider"'","thread":{"id":"rounds-thread","turns":[{"id":"turn-1","status":"completed","items":[{"type":"userMessage","id":"user-1","content":[{"type":"text","text":"same prompt"}]},{"type":"agentMessage","id":"answer-1","text":"hello world"}]},{"id":"turn-2","status":"completed","items":[{"type":"userMessage","id":"user-2","content":[{"type":"text","text":"same prompt"}]},{"type":"agentMessage","id":"answer-2","text":"hello world"}]}]}}}\n' "$request" "$PWD";;
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
            manager.send("alice", &id, &generation, request.clone(), "same prompt".into(), None, vec![], vec![], vec![]).await.unwrap();
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
        restored.send("alice", &id, &generation, uuid::Uuid::new_v4().to_string(), "same prompt".into(), None, vec![], vec![], vec![]).await.unwrap();
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
    #[tokio::test]
    async fn explicit_turn_settings_reach_codex_without_retry_or_permission_bypass() {
        use crate::session::{AgentTurnSettings, PermissionMode};
        let root = std::env::temp_dir().join(format!("dgos-turn-controls-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("codex");
        crate::test_support::codex_fixture(&executable, &r##"
while IFS= read -r line; do
printf '%s\n' "$line" >> protocol
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
*'"method":"thread/start"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"'"$fixture_provider"'","approvalPolicy":"on-request","approvalsReviewer":"user","sandbox":{"type":"workspaceWrite","networkAccess":false,"writableRoots":[]},"reasoningEffort":"medium","thread":{"id":"controls-thread","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[{"model":"other-model","defaultReasoningEffort":"max","supportedReasoningEfforts":[{"reasoningEffort":"max"}]},{"model":"fixture-model","defaultReasoningEffort":"medium","supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"medium"}]}],"nextCursor":null}}\n' "$request";;
*'"method":"turn/start"'*) printf '{"id":%s,"result":{"turn":{"id":"controls-turn"}}}\n' "$request"; printf '%s\n' '{"method":"turn/completed","params":{"threadId":"controls-thread","turn":{"id":"controls-turn","status":"completed"}}}';;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
        let manager = crate::test_support::manager(&root).await;
        let task = crate::test_support::task(&manager, &root).await;
        let options = SessionOptions { workspace: root.to_string_lossy().into_owned(), resume_id: None, executable: Some(executable.to_string_lossy().into_owned()), model: None, trust_workspace: true };
        let id = uuid::Uuid::new_v4().to_string();
        let opened = manager.open_session("alice", Engine::Codex, id.clone(), crate::test_support::create(&manager, "alice", &task, options).await, Some(task), CancellationToken::new()).await.unwrap();
        assert_eq!(opened.view.controls.as_ref().unwrap().reasoning.options, ["low", "medium"]);
        let invalid = uuid::Uuid::new_v4().to_string();
        assert_eq!(manager.send("alice", &id, &opened.generation, invalid.clone(), "never dispatch".into(), Some(AgentTurnSettings { reasoning_effort: Some(Some("max".into())), permission_mode: Some(PermissionMode::ReadOnly), collaboration_mode: None, response_preferences: None }), vec![], vec![], vec![]).await.unwrap_err(), "unsupported_reasoning_effort");
        assert!(manager.inner.repository.run("alice", &invalid).await.unwrap().is_none());
        let request = uuid::Uuid::new_v4().to_string();
        let settings = AgentTurnSettings { reasoning_effort: Some(Some("low".into())), permission_mode: Some(PermissionMode::ReadOnly), collaboration_mode: None, response_preferences: None };
        let literal_settings = format!("once\0{}", serde_json::to_string(&settings).unwrap());
        manager.send("alice", &id, &opened.generation, request.clone(), "once".into(), Some(settings.clone()), vec![], vec![], vec![]).await.unwrap();
        manager.send("alice", &id, &opened.generation, request.clone(), "once".into(), Some(settings), vec![], vec![], vec![]).await.unwrap();
        assert_eq!(manager.send("alice", &id, &opened.generation, request.clone(), literal_settings.clone(), None, vec![], vec![], vec![]).await.unwrap_err(), "request_conflict");
        let (_, crate::manager::Backend::Codex(backend)) = manager.owned("alice", &id).await.unwrap() else { panic!("expected Codex backend"); };
        assert_eq!(backend.send(request.clone(), literal_settings, None, vec![], vec![]).await.unwrap_err(), "request_conflict");
        assert_eq!(manager.send("alice", &id, &opened.generation, request, "once".into(), Some(AgentTurnSettings { reasoning_effort: Some(None), permission_mode: Some(PermissionMode::Default), collaboration_mode: None, response_preferences: None }), vec![], vec![], vec![]).await.unwrap_err(), "request_conflict");
        let protocol: Vec<serde_json::Value> = std::fs::read_to_string(root.join("protocol")).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        let sends: Vec<_> = protocol.iter().filter(|value| value["method"] == "turn/start").collect();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0]["params"]["effort"], "low");
        assert_eq!(sends[0]["params"]["approvalPolicy"], "never");
        assert_eq!(sends[0]["params"]["approvalsReviewer"], "user");
        assert_eq!(sends[0]["params"]["sandboxPolicy"], json!({"type":"readOnly","networkAccess":false}));
        assert!(manager.shutdown().await);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn startup_diagnostics_preserve_preflight_spawn_handshake_and_binding_boundaries() {
        use std::os::unix::fs::PermissionsExt;
        for (mode, expected, spawned) in [
            ("missing", "cli_not_installed", false),
            ("not_executable", "cli_not_executable", false),
            ("older", "handshake_success", true),
            ("newer", "handshake_success", true),
            ("invalid_version", "cli_probe_failed", false),
            ("spawn_failure", "cli_start_failed", false),
            ("handshake_rejected", "cli_handshake_failed", true),
            ("handshake_disconnected", "cli_handshake_failed", true),
            ("provider_binding", "provider_binding_unconfirmed", true),
        ] {
            let root = std::env::temp_dir().join(format!("dgos-startup-diagnostic-{mode}-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let executable = root.join("codex");
            let version = match mode { "older" => "codex-cli 0.166.0", "newer" => "codex-cli 0.168.0", "invalid_version" => "sensitive-version-output", _ => "codex-cli 0.167.0" };
            let version_effect = if mode == "spawn_failure" { "chmod 600 \"$0\"" } else { ":" };
            let initialize = match mode {
                "handshake_rejected" => "printf '{\"id\":%s,\"error\":{\"code\":-1,\"message\":\"sensitive-handshake-output\"}}\\n' \"$request\"",
                "handshake_disconnected" => "exit 0",
                _ => "printf '{\"id\":%s,\"result\":{}}\\n' \"$request\"",
            };
            let workspace = root.to_string_lossy().into_owned();
            let thread_cwd = if matches!(mode, "older" | "newer") { workspace.as_str() } else { "/dgos-missing-workspace" };
            crate::test_support::prepare_shell_fixture(&executable, &format!(r##"
if [ "${{1-}}" = --version ]; then printf '%s\n' '{version}'; {version_effect}; exit 0; fi
fixture_provider=
for argument do
  case "$argument" in model_provider=*) fixture_provider=${{argument#*=}}; fixture_provider=${{fixture_provider#\"}}; fixture_provider=${{fixture_provider%\"}};; esac
done
printf x >> started
while IFS= read -r line; do
printf '%s\n' "$line" >> protocol
request=${{line#*\"id\":}}; request=${{request%%,*}}
case "$line" in
{config}
*'"method":"initialize"'*) {initialize};;
*'"method":"thread/start"'*) printf '{{"id":%s,"result":{{"cwd":"%s","model":"fixture-model","modelProvider":"'"$fixture_provider"'","thread":{{"id":"diagnostic-thread","turns":[]}}}}}}\n' "$request" '{thread_cwd}';;
*'"method":"model/list"'*) printf '{{"id":%s,"result":{{"data":[{{"model":"fixture-model"}}],"nextCursor":null}}}}\n' "$request";;
esac
done
"##, config = crate::test_support::CODEX_CONFIG_REPLY)).await;
            if mode == "missing" { std::fs::remove_file(&executable).unwrap(); }
            if mode == "not_executable" { std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o600)).unwrap(); }
            let manager = crate::test_support::manager(&root).await;
            let task = crate::test_support::task(&manager, &root).await;
            let options = SessionOptions { workspace, executable: Some(executable.to_string_lossy().into_owned()), resume_id: None, model: None, trust_workspace: true };
            let id = uuid::Uuid::new_v4().to_string();
            let create = crate::test_support::create(&manager, "alice", &task, options).await;
            let opened = manager.open_session("alice", Engine::Codex, id.clone(), create, Some(task.clone()), CancellationToken::new()).await;
            assert_eq!(root.join("started").exists(), spawned, "{mode}");
            if expected == "handshake_success" {
                assert!(opened.is_ok(), "{mode}: {:?}", opened.as_ref().err());
                let session = manager.snapshot("alice", &id).await.unwrap();
                assert_eq!(session.view.cli_version.as_deref(), Some(version), "{mode}");
                assert_eq!(session.view.status, "idle", "{mode}");
            } else {
                assert_eq!(opened.err().as_deref(), Some(expected), "{mode}");
                // Failed startup remains readable history, without a live generation.
                let session = manager.snapshot("alice", &id).await.unwrap();
                assert_eq!(session.view.status, "closed", "{mode}");
                assert_eq!(session.generation, uuid::Uuid::nil().to_string(), "{mode}");
                assert!(session.view.pending.is_empty(), "{mode}");
                assert!(session.view.active_run.is_none(), "{mode}");
                assert!(matches!(session.configuration, crate::capabilities::SessionConfigurationView::ProviderBound { read_only: true, .. }), "{mode}");
                assert!(manager.snapshot("bob", &id).await.is_err(), "{mode}");
                let binding = manager.inner.repository.read_cli_session_configuration("alice", &id).await.unwrap().binding.unwrap();
                assert_eq!(binding.state, "closed", "{mode}");
                assert_eq!(binding.cleanup_state, "stopped", "{mode}");
                assert_eq!(binding.cleanup_proof.as_deref(), Some(if spawned { "owned_exit" } else { "no_execution" }), "{mode}");
            }
            if spawned {
                assert_eq!(std::fs::read(&root.join("started")).unwrap(), b"x", "startup was replayed: {mode}");
                let protocol = std::fs::read_to_string(root.join("protocol")).unwrap();
                assert!(!protocol.contains("turn/start"), "a prompt was sent during startup: {mode}");
            }
            assert!(manager.list_development_runs("alice", &task).await.unwrap().is_empty());
            assert!(manager.shutdown().await);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
