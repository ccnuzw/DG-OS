#![cfg(unix)]
use crate::{native_session::*, AgentManager, Engine, SessionOptions};
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

struct Fixture { root: PathBuf, manager: AgentManager, id: String, project: String, lease: CancellationToken }
impl Fixture {
    async fn open() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-native-session-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        crate::test_support::codex_fixture(&root.join("codex"), &r##"
while IFS= read -r line; do
request=${line#*\"id\":}; request=${request%%,*}
printf '%s\n' "$line" >> protocol
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request";;
*'"method":"thread/start"'*|*'"method":"thread/resume"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"fixture-thread","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"skills/list"'*)
 if [ -f unsupported-skills ]; then printf '{"id":%s,"error":{"code":-32601,"message":"Method not found"}}\n' "$request"; continue; fi
 if [ -f disconnect ]; then exit 0; fi
 if [ -f block ]; then touch entered; while [ ! -f release ]; do sleep 0.01; done; fi
 case "$line" in *'"forceReload":true'*) printf x >> reloads;; esac
 if [ -f empty ]; then printf '{"id":%s,"result":{"data":[]}}\n' "$request"; continue; fi
 if [ -f missing-errors ]; then printf '{"id":%s,"result":{"data":[{"skills":[]}]}}\n' "$request"; continue; fi
 if [ -f disabled ]; then printf '{"id":%s,"result":{"data":[{"skills":[{"name":"fixture","enabled":false,"path":"/fixture/plugin/skills/fixture/SKILL.md","scope":"repo"}],"errors":[]}]}}\n' "$request"; continue; fi
 if [ -f duplicate ]; then printf '{"id":%s,"result":{"data":[{"skills":[{"name":"fixture","enabled":true,"path":"/fixture/plugin/skills/fixture/SKILL.md","scope":"repo"},{"name":"fixture","enabled":true,"path":"/fixture/plugin/skills/fixture/SKILL.md","scope":"user"}],"errors":[]}]}}\n' "$request"; continue; fi
 if [ -f malformed ]; then printf '{"id":%s,"result":{"data":null}}\n' "$request";
 else printf '{"id":%s,"result":{"data":[{"cwd":"private","skills":[{"name":"fixture","enabled":true,"description":"secret-SENTINEL","path":"/fixture/plugin/skills/fixture/SKILL.md","scope":"repo","pluginId":"fixture@marketplace"}],"errors":[]}]}}\n' "$request"; fi;;
*'"method":"mcpServerStatus/list"'*) printf '{"id":%s,"result":{"data":[{"name":"fixture","runtimeStatus":"authenticationRequired","authStatus":"notLoggedIn","tools":{"echo":{"description":"secret-SENTINEL","inputSchema":{"secret":"secret-SENTINEL"}}},"httpOrigin":"https://secret-SENTINEL","config":{"env":{"TOKEN":"secret-SENTINEL"}}}],"nextCursor":"repeat"}}\n' "$request";;
*'"method":"turn/start"'*) printf '{"id":%s,"result":{"turn":{"id":"skill-turn"}}}\n' "$request";;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
        let manager = crate::test_support::manager(&root).await;
        let task = crate::test_support::task(&manager, &root).await;
        let project = manager.development_workspace("alice").await.unwrap().projects[0].id.clone();
        let id = uuid::Uuid::new_v4().to_string();
        let lease = CancellationToken::new();
        let options = SessionOptions { workspace: root.to_string_lossy().into(), executable: Some(root.join("codex").to_string_lossy().into()), resume_id: None, model: None, trust_workspace: true };
        let create = crate::test_support::create(&manager, "alice", &task, options).await;
        manager.open_session("alice", Engine::Codex, id.clone(), create, Some(task), lease.clone()).await.unwrap();
        Self { root, manager, id, project, lease }
    }
    async fn handle(&self) -> NativeSessionHandle {
        self.manager.native_session_handles("alice", &self.project, &self.lease).await.unwrap().remove(0)
    }
    async fn action(&self, handle: &NativeSessionHandle, request: &str) -> Result<NativeSessionReceipt, String> {
        self.manager.native_session_action("alice", &self.id, &handle.generation, request, NativeSessionAction::ReloadSkills {}, &self.lease).await
    }
    async fn inspect(&self, handle: &NativeSessionHandle, request: &str) -> Result<Option<NativeSessionReceipt>, String> {
        self.manager.inspect_native_session_request("alice", &self.id, &handle.generation, request, SessionDomain::Skills, &self.lease).await
    }
    async fn finish(self) {
        assert!(self.manager.shutdown().await);
        std::fs::remove_dir_all(self.root).unwrap();
    }
}

#[tokio::test]
async fn missing_optional_skill_method_disables_only_skills_and_keeps_session_usable() {
    let fixture = Fixture::open().await;
    let handle = fixture.handle().await;
    std::fs::write(fixture.root.join("unsupported-skills"), "").unwrap();
    assert_eq!(fixture.manager.observe_native_session("alice", &fixture.id, &handle.generation, SessionDomain::Skills, &fixture.lease).await.unwrap_err(), "native_extension_Unsupported");
    let current = fixture.handle().await;
    assert!(!current.capabilities.observe_skills && !current.capabilities.reload_skills);
    assert!(current.capabilities.observe_mcp);
    assert_eq!(fixture.manager.snapshot("alice", &fixture.id).await.unwrap().view.status, "idle");
    let observed = std::fs::read_to_string(fixture.root.join("protocol")).unwrap();
    assert_eq!(fixture.manager.observe_native_session("alice", &fixture.id, &handle.generation, SessionDomain::Skills, &fixture.lease).await.unwrap_err(), "native_extension_Unsupported");
    assert_eq!(std::fs::read_to_string(fixture.root.join("protocol")).unwrap(), observed);
    fixture.manager.send("alice", &fixture.id, &handle.generation, uuid::Uuid::new_v4().to_string(), "plain message".into(), None, vec![], vec![]).await.unwrap();
    assert!(std::fs::read_to_string(fixture.root.join("protocol")).unwrap().contains("turn/start"));
    fixture.finish().await;
}

#[tokio::test]
async fn reload_before_observation_of_missing_skill_method_keeps_plain_send_usable() {
    let fixture = Fixture::open().await;
    let handle = fixture.handle().await;
    let request = uuid::Uuid::new_v4().to_string();
    std::fs::write(fixture.root.join("unsupported-skills"), "").unwrap();
    assert_eq!(fixture.action(&handle, &request).await.unwrap_err(), "native_extension_Unsupported");
    assert!(fixture.inspect(&handle, &request).await.unwrap().is_none());
    let current = fixture.handle().await;
    assert!(!current.capabilities.observe_skills && !current.capabilities.reload_skills);
    assert!(current.capabilities.observe_mcp);
    let protocol = std::fs::read_to_string(fixture.root.join("protocol")).unwrap();
    assert_eq!(fixture.action(&handle, &request).await.unwrap_err(), "native_extension_Unsupported");
    assert_eq!(fixture.manager.observe_native_session("alice", &fixture.id, &handle.generation, SessionDomain::Skills, &fixture.lease).await.unwrap_err(), "native_extension_Unsupported");
    assert_eq!(std::fs::read_to_string(fixture.root.join("protocol")).unwrap(), protocol);
    assert!(!fixture.root.join("reloads").exists());
    assert_eq!(fixture.manager.snapshot("alice", &fixture.id).await.unwrap().view.status, "idle");
    fixture.manager.send("alice", &fixture.id, &handle.generation, uuid::Uuid::new_v4().to_string(), "plain message".into(), None, vec![], vec![]).await.unwrap();
    assert!(std::fs::read_to_string(fixture.root.join("protocol")).unwrap().contains("turn/start"));
    fixture.finish().await;
}

#[tokio::test]
async fn blocked_skill_send_allows_other_session_admission_and_revocation_cancels_dispatch() {
    for revoke_provider in [false, true] {
        let fixture = Fixture::open().await;
        let handle = fixture.handle().await;
        let other_root = fixture.root.join("other-session");
        std::fs::create_dir(&other_root).unwrap();
        let task = crate::test_support::task(&fixture.manager, &other_root).await;
        let other_id = uuid::Uuid::new_v4().to_string();
        let options = SessionOptions { workspace: other_root.to_string_lossy().into(), executable: Some(fixture.root.join("codex").to_string_lossy().into()), resume_id: None, model: None, trust_workspace: true };
        let create = crate::test_support::create(&fixture.manager, "alice", &task, options).await;
        std::fs::write(fixture.root.join("block"), "").unwrap();
        let request = uuid::Uuid::new_v4().to_string();
        let operation = fixture.manager.send("alice", &fixture.id, &handle.generation, request.clone(), "Use this skill".into(), None, vec![], vec![crate::session::AgentSkillInput { name: "fixture".into(), path: "/fixture/plugin/skills/fixture/SKILL.md".into() }]);
        tokio::pin!(operation);
        tokio::select! {
            result = &mut operation => panic!("skill send completed before fixture release: {result:?}"),
            entered = tokio::time::timeout(Duration::from_secs(5), async {
                while !fixture.root.join("entered").exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
            }) => { entered.expect("fixture never entered skill observation"); },
        }
        assert_eq!(fixture.manager.close_session("alice", &fixture.id).await.unwrap_err(), "session_busy");
        tokio::time::timeout(Duration::from_secs(5), fixture.manager.open_session("alice", Engine::Codex, other_id.clone(), create, Some(task), CancellationToken::new())).await.expect("blocked skill observation retained manager admission").unwrap();
        if revoke_provider {
            fixture.manager.inner.providers.providers().toggle_provider("fixture-alice", false, "alice").await.unwrap();
        } else {
            fixture.lease.cancel();
        }
        let error = tokio::time::timeout(Duration::from_secs(1), &mut operation).await.expect("revocation did not cancel skill observation").unwrap_err();
        assert!(matches!(error.as_str(), "session_closed" | "provider_revision_changed"));
        assert!(fixture.manager.inner.repository.run("alice", &request).await.unwrap().is_none());
        assert!(!std::fs::read_to_string(fixture.root.join("protocol")).unwrap().contains("turn/start"));
        std::fs::write(fixture.root.join("release"), "").unwrap();
        assert!(fixture.manager.shutdown().await);
        std::fs::remove_dir_all(&fixture.root).unwrap();
    }
}

#[tokio::test]
async fn native_controls_are_bounded_owned_generation_bound_and_never_replay() {
    let fixture = Fixture::open().await;
    let handle = fixture.handle().await;
    assert!(fixture.manager.native_session_handles("bob", &fixture.project, &fixture.lease).await.is_err());
    assert!(fixture.manager.observe_native_session("bob", &fixture.id, &handle.generation, SessionDomain::Skills, &fixture.lease).await.is_err());
    let skills = fixture.manager.observe_native_session("alice", &fixture.id, &handle.generation, SessionDomain::Skills, &fixture.lease).await.unwrap();
    assert_eq!(skills.skills[0].name, "fixture");
    assert_eq!(skills.skills[0].classification, NativeSkillClassification::Skill);
    assert_eq!(skills.skills[0].path.as_deref(), Some("/fixture/plugin/skills/fixture/SKILL.md"));
    assert_eq!(skills.skills[0].scope.as_deref(), Some("repo"));
    assert_eq!(skills.skills[0].plugin_id.as_deref(), Some("fixture@marketplace"));
    assert!(!skills.incomplete);
    assert!(!fixture.root.join("reloads").exists(), "observation never changes native state");
    assert!(!serde_json::to_string(&skills).unwrap().contains("secret-SENTINEL"));
    let mcp = fixture.manager.observe_native_session("alice", &fixture.id, &handle.generation, SessionDomain::Mcp, &fixture.lease).await.unwrap();
    assert!(mcp.incomplete, "repeated opaque cursor must stop pagination");
    assert_eq!(mcp.mcp_servers[0].status, "authenticationRequired");
    assert!(mcp.mcp_servers[0].tools.is_empty(), "auth-required must not retain a stale tool catalog");
    assert!(!mcp.mcp_servers[0].schema_observed);
    assert!(!serde_json::to_string(&mcp).unwrap().contains("secret-SENTINEL"));
    assert!(!handle.capabilities.native_login && !handle.capabilities.native_logout && !handle.capabilities.reconnect_mcp && !handle.capabilities.toggle_mcp);
    let guidance = handle.capabilities.mcp_control_guidance.as_ref().unwrap();
    assert_eq!(guidance.verified_version, crate::cli::codex_protocol_reference().version);
    assert_eq!(guidance.reason, "missing_session_scoped_mcp_control_contract");
    assert_eq!(guidance.inspect_argv, ["codex", "mcp", "get", "<NAME>"]);
    assert_eq!(handle.capabilities.native_auth_guidance.as_ref().unwrap().reason, "missing_mcp_login_cancel_and_logout_contract");
    for action in [
        NativeSessionAction::ReconnectMcp { server_name: "fixture".into() },
        NativeSessionAction::ToggleMcp { server_name: "fixture".into(), enabled: true },
        NativeSessionAction::ToggleMcp { server_name: "fixture".into(), enabled: false },
        NativeSessionAction::NativeLogin { server_name: "fixture".into() },
        NativeSessionAction::NativeLogout { server_name: "fixture".into() },
    ] {
        let request = uuid::Uuid::new_v4().to_string();
        let refusal = fixture.manager.native_session_action("alice", &fixture.id, &handle.generation, &request, action, &fixture.lease).await.unwrap_err();
        assert_eq!(refusal, "native_extension_Unsupported");
        assert!(fixture.manager.inspect_native_session_request("alice", &fixture.id, &handle.generation, &request, SessionDomain::Mcp, &fixture.lease).await.unwrap().is_none(), "unsupported controls are refused before reserving a mutation");
    }
    let request = uuid::Uuid::new_v4().to_string();
    assert!(fixture.inspect(&handle, &request).await.unwrap().is_none());
    assert!(fixture.manager.inspect_native_session_request("bob", &fixture.id, &handle.generation, &request, SessionDomain::Skills, &fixture.lease).await.is_err());
    assert_eq!(fixture.inspect(&handle, "invalid").await.unwrap_err(), "invalid_request_id");
    assert_eq!(fixture.action(&handle, &request).await.unwrap().state, NativeActionState::Applied);
    assert_eq!(fixture.action(&handle, &request).await.unwrap().state, NativeActionState::Applied);
    let inspected = fixture.inspect(&handle, &request).await.unwrap().unwrap();
    assert_eq!(inspected.request_id, request);
    assert_eq!(inspected.generation, handle.generation);
    assert_eq!(inspected.state, NativeActionState::Applied);
    assert!(fixture.manager.inspect_native_session_request("alice", &fixture.id, &handle.generation, &request, SessionDomain::Mcp, &fixture.lease).await.unwrap().is_none());
    assert_eq!(std::fs::read(fixture.root.join("reloads")).unwrap(), b"x");
    assert_eq!(fixture.manager.native_session_action("alice", &fixture.id, &handle.generation, &request, NativeSessionAction::ReloadPlugins { hold_on_cache_impact: true }, &fixture.lease).await.unwrap_err(), "request_conflict");
    assert!(fixture.manager.close_session("alice", &fixture.id).await.unwrap());
    assert!(fixture.inspect(&handle, &request).await.is_err());
    assert!(fixture.manager.native_session_handles("alice", &fixture.project, &fixture.lease).await.unwrap().is_empty());
    fixture.manager.resume_session("alice", &fixture.id, true, fixture.lease.clone()).await.unwrap();
    let resumed = fixture.handle().await;
    assert_ne!(resumed.generation, handle.generation);
    assert_eq!(fixture.action(&handle, &request).await.unwrap_err(), "native_extension_StaleGeneration");
    assert_eq!(fixture.inspect(&handle, &request).await.unwrap_err(), "native_extension_StaleGeneration");
    assert!(fixture.inspect(&resumed, &request).await.unwrap().is_none());
    fixture.finish().await;
}

#[tokio::test]
async fn unknown_native_result_blocks_new_controls_and_turns_but_allows_read_and_close() {
    let fixture = Fixture::open().await;
    let handle = fixture.handle().await;
    std::fs::write(fixture.root.join("malformed"), "").unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    assert_eq!(fixture.action(&handle, &request).await.unwrap().state, NativeActionState::Unknown);
    assert_eq!(fixture.action(&handle, &request).await.unwrap().state, NativeActionState::Unknown);
    assert_eq!(fixture.inspect(&handle, &request).await.unwrap().unwrap().state, NativeActionState::Unknown);
    assert!(fixture.inspect(&handle, &uuid::Uuid::new_v4().to_string()).await.unwrap().is_none());
    assert_eq!(fixture.action(&handle, &uuid::Uuid::new_v4().to_string()).await.unwrap_err(), "result_unknown");
    assert_eq!(fixture.manager.send("alice", &fixture.id, &handle.generation, uuid::Uuid::new_v4().to_string(), "must not dispatch".into(), None, vec![], vec![]).await.unwrap_err(), "result_unknown");
    assert_eq!(std::fs::read(fixture.root.join("reloads")).unwrap(), b"x");
    assert!(fixture.manager.observe_native_session("alice", &fixture.id, &handle.generation, SessionDomain::Mcp, &fixture.lease).await.is_ok());
    assert!(fixture.manager.close_session("alice", &fixture.id).await.unwrap());
    fixture.finish().await;
}

#[tokio::test]
async fn cancelled_control_never_replays_or_blocks_admission_and_late_results_are_discarded() {
    let fixture = Fixture::open().await;
    let handle = fixture.handle().await;
    std::fs::write(fixture.root.join("block"), "").unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    let operation = fixture.action(&handle, &request);
    tokio::pin!(operation);
    tokio::select! {
        result = &mut operation => panic!("control completed before fixture release: {result:?}"),
        entered = tokio::time::timeout(Duration::from_secs(5), async {
            while !fixture.root.join("entered").exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
        }) => { entered.expect("fixture never entered native control"); },
    }
    assert_eq!(fixture.manager.close_session("alice", &fixture.id).await.unwrap_err(), "session_busy");
    assert_eq!(fixture.manager.send("alice", &fixture.id, &handle.generation, uuid::Uuid::new_v4().to_string(), "must not dispatch".into(), None, vec![], vec![]).await.unwrap_err(), "session_busy");
    assert_eq!(fixture.manager.native_session_action("alice", &fixture.id, &handle.generation, &request, NativeSessionAction::ReloadSkills {}, &fixture.lease).await.unwrap().state, NativeActionState::Unknown);
    assert_eq!(fixture.inspect(&handle, &request).await.unwrap().unwrap().state, NativeActionState::Unknown);
    let other = CancellationToken::new();
    let admission = tokio::time::timeout(Duration::from_secs(1), fixture.manager.admit_native_extensions(&other)).await.unwrap().unwrap();
    drop(admission);
    fixture.lease.cancel();
    assert_eq!(operation.await.unwrap_err(), "session_invalid");
    std::fs::write(fixture.root.join("release"), "").unwrap();
    assert!(fixture.manager.observe_native_session("alice", &fixture.id, &handle.generation, SessionDomain::Skills, &fixture.lease).await.is_err());
    assert!(fixture.inspect(&handle, &request).await.is_err());
    assert!(fixture.manager.shutdown().await);
    std::fs::remove_dir_all(&fixture.root).unwrap();
}

#[tokio::test]
async fn provider_revision_revocation_denies_native_reads_and_actions() {
    let fixture = Fixture::open().await;
    let handle = fixture.handle().await;
    let request = uuid::Uuid::new_v4().to_string();
    fixture.action(&handle, &request).await.unwrap();
    fixture.manager.inner.providers.providers().toggle_provider("fixture-alice", false, "alice").await.unwrap();
    fixture.manager.inner.providers.providers().toggle_provider("fixture-alice", true, "alice").await.unwrap();
    assert!(fixture.manager.observe_native_session("alice", &fixture.id, &handle.generation, SessionDomain::Skills, &fixture.lease).await.is_err());
    assert!(fixture.action(&handle, &uuid::Uuid::new_v4().to_string()).await.is_err());
    assert!(fixture.inspect(&handle, &request).await.is_err());
    assert_eq!(std::fs::read(fixture.root.join("reloads")).unwrap(), b"x");
    fixture.finish().await;
}

#[tokio::test]
async fn empty_or_unproven_skill_observations_do_not_claim_completeness() {
    let fixture = Fixture::open().await;
    let handle = fixture.handle().await;
    for marker in ["empty", "missing-errors"] {
        std::fs::write(fixture.root.join(marker), "").unwrap();
        let observation = fixture.manager.observe_native_session("alice", &fixture.id, &handle.generation, SessionDomain::Skills, &fixture.lease).await.unwrap();
        assert!(observation.incomplete);
        assert!(observation.skills.is_empty());
        std::fs::remove_file(fixture.root.join(marker)).unwrap();
    }
    assert!(!fixture.root.join("reloads").exists());
    fixture.finish().await;
}

#[tokio::test]
async fn lost_native_transport_rejects_observations_and_receipt_reads() {
    let fixture = Fixture::open().await;
    let handle = fixture.handle().await;
    let request = uuid::Uuid::new_v4().to_string();
    fixture.action(&handle, &request).await.unwrap();
    std::fs::write(fixture.root.join("disconnect"), "").unwrap();
    assert!(fixture.manager.observe_native_session("alice", &fixture.id, &handle.generation, SessionDomain::Skills, &fixture.lease).await.is_err());
    assert!(fixture.manager.native_session_handles("alice", &fixture.project, &fixture.lease).await.unwrap().is_empty());
    assert!(fixture.inspect(&handle, &request).await.is_err());
    assert!(fixture.action(&handle, &uuid::Uuid::new_v4().to_string()).await.is_err());
    assert_eq!(std::fs::read(fixture.root.join("reloads")).unwrap(), b"x");
    fixture.finish().await;
}

#[tokio::test]
async fn explicit_skills_are_revalidated_fingerprinted_and_sent_as_native_inputs() {
    use crate::session::AgentSkillInput;
    let fixture = Fixture::open().await;
    let generation = fixture.handle().await.generation;
    let skill = AgentSkillInput { name: "fixture".into(), path: "/fixture/plugin/skills/fixture/SKILL.md".into() };
    for (selection, expected) in [(vec![AgentSkillInput { path: "/forged/SKILL.md".into(), ..skill.clone() }], "agent_skill_unavailable"), (vec![skill.clone(); 9], "agent_skill_limit"), (vec![skill.clone(); 2], "agent_skill_invalid")] {
        let request = uuid::Uuid::new_v4().to_string();
        assert_eq!(fixture.manager.send("alice", &fixture.id, &generation, request.clone(), "Use this skill".into(), None, vec![], selection).await.unwrap_err(), expected);
        assert!(fixture.manager.inner.repository.run("alice", &request).await.unwrap().is_none());
    }
    for (marker, expected) in [("disabled", "agent_skill_unavailable"), ("duplicate", "agent_skill_unavailable"), ("missing-errors", "agent_skills_unavailable")] {
        std::fs::write(fixture.root.join(marker), "").unwrap();
        let request = uuid::Uuid::new_v4().to_string();
        assert_eq!(fixture.manager.send("alice", &fixture.id, &generation, request.clone(), "Use this skill".into(), None, vec![], vec![skill.clone()]).await.unwrap_err(), expected);
        assert!(fixture.manager.inner.repository.run("alice", &request).await.unwrap().is_none());
        std::fs::remove_file(fixture.root.join(marker)).unwrap();
    }
    assert_eq!(fixture.manager.send("alice", &fixture.id, "stale", uuid::Uuid::new_v4().to_string(), "Use this skill".into(), None, vec![], vec![skill.clone()]).await.unwrap_err(), "stale_generation");
    let request = uuid::Uuid::new_v4().to_string();
    fixture.manager.send("alice", &fixture.id, &generation, request.clone(), "Use this skill".into(), None, vec![], vec![skill.clone()]).await.unwrap();
    std::fs::write(fixture.root.join("disabled"), "").unwrap();
    fixture.manager.send("alice", &fixture.id, &generation, request.clone(), "Use this skill".into(), None, vec![], vec![skill.clone()]).await.unwrap();
    assert_eq!(fixture.manager.send("alice", &fixture.id, &generation, request.clone(), "Use this skill".into(), None, vec![], vec![]).await.unwrap_err(), "request_conflict");
    let (_, crate::manager::Backend::Codex(backend)) = fixture.manager.owned("alice", &fixture.id).await.unwrap() else { panic!("expected Codex"); };
    assert_eq!(backend.send(request, "Use this skill".into(), None, vec![]).await.unwrap_err(), "request_conflict");
    let protocol: Vec<serde_json::Value> = std::fs::read_to_string(fixture.root.join("protocol")).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    let sends: Vec<_> = protocol.iter().filter(|value| value["method"] == "turn/start").collect();
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0]["params"]["input"], serde_json::json!([{"type":"text","text":"Use this skill"},{"type":"skill","name":skill.name,"path":skill.path}]));
    fixture.finish().await;
}
