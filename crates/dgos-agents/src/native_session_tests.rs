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
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"thread/start"'*|*'"method":"thread/resume"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"fixture-thread","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"skills/list"'*)
 if [ -f disconnect ]; then exit 0; fi
 if [ -f block ]; then touch entered; while [ ! -f release ]; do sleep 0.01; done; fi
 case "$line" in *'"forceReload":true'*) printf x >> reloads;; esac
 if [ -f empty ]; then printf '{"id":%s,"result":{"data":[]}}\n' "$request"; continue; fi
 if [ -f missing-errors ]; then printf '{"id":%s,"result":{"data":[{"skills":[]}]}}\n' "$request"; continue; fi
 if [ -f malformed ]; then printf '{"id":%s,"result":{"data":null}}\n' "$request";
 else printf '{"id":%s,"result":{"data":[{"cwd":"private","skills":[{"name":"fixture","enabled":true,"description":"secret-SENTINEL","path":"/fixture/plugin/skills/fixture/SKILL.md","scope":"repo","pluginId":"fixture@marketplace"}],"errors":[]}]}}\n' "$request"; fi;;
*'"method":"mcpServerStatus/list"'*) printf '{"id":%s,"result":{"data":[{"name":"fixture","runtimeStatus":"authenticationRequired","authStatus":"notLoggedIn","tools":{"echo":{"description":"secret-SENTINEL","inputSchema":{"secret":"secret-SENTINEL"}}},"httpOrigin":"https://secret-SENTINEL","config":{"env":{"TOKEN":"secret-SENTINEL"}}}],"nextCursor":"repeat"}}\n' "$request";;
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
    assert!(!handle.capabilities.native_login && !handle.capabilities.reconnect_mcp);
    let auth = fixture.manager.native_session_action("alice", &fixture.id, &handle.generation, &uuid::Uuid::new_v4().to_string(), NativeSessionAction::NativeLogin { server_name: "fixture".into() }, &fixture.lease).await.unwrap_err();
    assert_eq!(auth, "native_extension_Unsupported");
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
    assert_eq!(fixture.manager.send("alice", &fixture.id, &handle.generation, uuid::Uuid::new_v4().to_string(), "must not dispatch".into()).await.unwrap_err(), "result_unknown");
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
    assert_eq!(fixture.manager.send("alice", &fixture.id, &handle.generation, uuid::Uuid::new_v4().to_string(), "must not dispatch".into()).await.unwrap_err(), "session_busy");
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
