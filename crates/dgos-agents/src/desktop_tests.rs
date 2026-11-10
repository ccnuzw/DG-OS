#![cfg(unix)]
use crate::{AgentManager, Engine, SessionOptions, desktop::{ActivateWorkspaceAgentInput, ReadWorkspaceTrustInput, SetWorkspaceTrustInput, SaveDesktopStateInput}, projects::{CreateProjectInput, SaveTaskInput}, runtime::{StartProcessInput, ReadinessInput}};
use tokio_util::sync::CancellationToken;

struct Fixture { root: tempfile::TempDir, manager: AgentManager, project: String, task: String }
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("code")).unwrap();
        std::fs::write(root.path().join("code/source"), "original\n").unwrap();
        crate::test_support::codex_fixture(&root.path().join("codex"), &r#"
while IFS= read -r line; do
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request";;
*'"method":"thread/start"'*|*'"method":"thread/resume"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"desktop-thread","turns":[]}}}\n' "$request" "$PWD";;
esac
done
"#.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
        let manager = crate::test_support::manager(root.path()).await;
        let task = crate::test_support::task(&manager, &root.path().join("code")).await;
        let project = manager.inner.repository.task("alice", &task).await.unwrap().project_id;
        Self {root, manager, project, task}
    }
    async fn scope(&self, scope: &str, granted: bool) {
        let trust = self.manager.read_development_workspace_trust("alice", ReadWorkspaceTrustInput {project_id:self.project.clone(),task_id:Some(self.task.clone())}, CancellationToken::new()).await.unwrap();
        self.manager.set_development_workspace_trust("alice", SetWorkspaceTrustInput {project_id:self.project.clone(),task_id:Some(self.task.clone()),expected_directory_identity:trust.directory_identity,expected_revision:trust.revision,scope:scope.into(),granted}, CancellationToken::new()).await.unwrap();
    }
    async fn open(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let options = SessionOptions { workspace: String::new(), executable:Some(self.root.path().join("codex").to_string_lossy().into()), model:None,resume_id:None,trust_workspace:false };
        let create = crate::test_support::create(&self.manager,"alice",&self.task,options).await;
        self.manager.open_session("alice",Engine::Codex,id.clone(),create,Some(self.task.clone()),CancellationToken::new()).await.unwrap();
        id
    }
    fn process(&self, service: bool) -> StartProcessInput {
        let port = {let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();listener.local_addr().unwrap().port()};
        StartProcessInput {id:uuid::Uuid::new_v4().to_string(),task_id:self.task.clone(),kind:if service {"service"}else{"command"}.into(),program:"/bin/sh".into(),args:vec!["-c".into(),"printf 'desktop-owned\\n'; exec sleep 30".into()],cwd:String::new(),env_refs:Default::default(),trust_workspace:false,verification:false,readiness:service.then_some(ReadinessInput {port,path:"/".into(),timeout_ms:120000}),native_run_id:None}
    }
}
#[tokio::test]
async fn persistent_service_coexists_in_both_orders_but_commands_terminal_and_foreign_owner_do_not() {
    let f=Fixture::new().await; f.scope("agent",true).await; f.scope("service",true).await; f.scope("terminal",true).await;
    let service=f.process(true);f.manager.start_development_process("alice",service.clone(),CancellationToken::new()).await.unwrap();
    let first=f.open().await;
    for kind in ["command","terminal"] {
        let mut command=f.process(false);command.kind=kind.into();command.trust_workspace=true;
        assert_eq!(f.manager.start_development_process("alice",command,CancellationToken::new()).await.unwrap_err(),"workspace_busy");
    }
    f.scope("agent",false).await;
    assert_eq!(f.manager.snapshot("alice",&first).await.unwrap().view.status,"closed");
    assert!(f.manager.read_development_process("alice",&service.id).await.unwrap().finished_at.is_none());
    assert_eq!(f.manager.resume_session("alice",&first,false,CancellationToken::new()).await.err().unwrap(),"workspace_untrusted");
    f.manager.stop_development_process("alice",&service.id).await.unwrap();
    f.scope("agent",true).await;let second=f.open().await;
    let reverse=f.process(true);f.manager.start_development_process("alice",reverse.clone(),CancellationToken::new()).await.unwrap();
    let bob=f.manager.create_project("bob",CreateProjectInput {kind:"import".into(),name:"Foreign owner".into(),path:f.root.path().join("code").to_string_lossy().into(),source:None},CancellationToken::new()).await.unwrap();
    let task=f.manager.save_development_task("bob",SaveTaskInput {id:None,project_id:bob.id.clone(),goal:"Foreign process".into(),requirements:"Do not affect Alice".into(),status:"todo".into()},CancellationToken::new()).await.unwrap();
    f.manager.set_development_workspace_trust("bob",SetWorkspaceTrustInput {project_id:bob.id,task_id:Some(task.id.clone()),expected_directory_identity:bob.directory_identity,expected_revision:0,scope:"service".into(),granted:true},CancellationToken::new()).await.unwrap();
    let mut foreign=f.process(true);foreign.task_id=task.id;
    assert_eq!(f.manager.start_development_process("bob",foreign,CancellationToken::new()).await.unwrap_err(),"workspace_busy");
    f.scope("service",false).await;
    assert!(f.manager.read_development_process("alice",&reverse.id).await.unwrap().finished_at.is_some());
    assert_ne!(f.manager.snapshot("alice",&second).await.unwrap().view.status,"closed");
    assert!(f.manager.shutdown().await);
}
#[tokio::test]
async fn explicit_trust_never_persists_and_exact_directory_replacement_invalidates_grants() {
    let f=Fixture::new().await;
    let mut command=f.process(false);command.trust_workspace=true;command.args=vec!["-c".into(),"exit 0".into()];
    f.manager.start_development_process("alice",command,CancellationToken::new()).await.unwrap();
    let project=f.manager.task_project("alice",&f.task).await.unwrap();
    assert!(!f.manager.inner.repository.workspace_trust("alice",&project).await.unwrap().terminal);
    f.scope("agent",true).await;
    std::fs::rename(f.root.path().join("code"),f.root.path().join("original-code")).unwrap();
    std::fs::create_dir(f.root.path().join("code")).unwrap();
    assert_eq!(f.manager.read_development_workspace_trust("alice",ReadWorkspaceTrustInput {project_id:f.project.clone(),task_id:Some(f.task.clone())},CancellationToken::new()).await.unwrap_err(),"workspace_changed");
    assert!(f.manager.require_workspace_trust("alice",&project,"agent",false).await.is_err());
    assert!(f.manager.shutdown().await);
}
#[tokio::test]
async fn cancelled_account_cannot_save_desktop_or_change_trust() {
    let f=Fixture::new().await;let lease=CancellationToken::new();lease.cancel();
    assert_eq!(f.manager.save_development_desktop_state("alice",SaveDesktopStateInput {expected_revision:0,value:serde_json::json!({"draft":"retained only locally"})},lease.clone()).await.unwrap_err(),"session_invalid");
    let project=f.manager.task_project("alice",&f.task).await.unwrap();
    assert_eq!(f.manager.set_development_workspace_trust("alice",SetWorkspaceTrustInput {project_id:f.project,task_id:Some(f.task),expected_directory_identity:project.directory_identity,expected_revision:0,scope:"agent".into(),granted:true},lease).await.unwrap_err(),"session_invalid");
    assert_eq!(f.manager.inner.repository.desktop_state("alice").await.unwrap().revision,0);
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn one_operation_service_trust_cannot_relax_native_directory_exclusion() {
    let f=Fixture::new().await;f.scope("agent",true).await;
    let mut service=f.process(true);service.trust_workspace=true;
    f.manager.start_development_process("alice",service.clone(),CancellationToken::new()).await.unwrap();
    let project=f.manager.task_project("alice",&f.task).await.unwrap();
    assert_eq!(f.manager.agent_host_workspace_available("alice",&project).await.unwrap_err(),"workspace_busy");
    assert!(!f.manager.inner.repository.workspace_trust("alice",&project).await.unwrap().service);
    f.manager.stop_development_process("alice",&service.id).await.unwrap();
    let _agent=f.open().await;
    service.id=uuid::Uuid::new_v4().to_string();
    assert_eq!(f.manager.start_development_process("alice",service,CancellationToken::new()).await.unwrap_err(),"workspace_busy");
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn revocation_stops_recorded_execution_even_when_directory_is_missing_or_replaced() {
    for scope in ["agent", "terminal", "service"] {
        let f=Fixture::new().await; f.scope(scope,true).await;
        let project=f.manager.task_project("alice",&f.task).await.unwrap();
        let trust=f.manager.inner.repository.workspace_trust("alice",&project).await.unwrap();
        let id=if scope=="agent" {f.open().await} else {
            let process=f.process(scope=="service"); let id=process.id.clone();
            f.manager.start_development_process("alice",process,CancellationToken::new()).await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5),async {
                loop {
                    let view=f.manager.read_development_process("alice",&id).await.unwrap();
                    if view.output.contains("desktop-owned") {break;}
                    assert!(view.finished_at.is_none(),"process failed before directory replacement");
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            id
        };
        std::fs::rename(f.root.path().join("code"),f.root.path().join("old-code")).unwrap();
        if scope!="terminal" {std::fs::create_dir(f.root.path().join("code")).unwrap();}
        let revoked=f.manager.set_development_workspace_trust("alice",SetWorkspaceTrustInput {
            project_id:f.project.clone(),task_id:Some(f.task.clone()),expected_directory_identity:trust.directory_identity,
            expected_revision:trust.revision,scope:scope.into(),granted:false,
        },CancellationToken::new()).await.unwrap();
        assert!(!revoked.permits(scope));
        assert_eq!(revoked.revision,trust.revision+1);
        assert!(!f.manager.inner.repository.workspace_trust("alice",&project).await.unwrap().permits(scope));
        if scope=="agent" {assert_eq!(f.manager.snapshot("alice",&id).await.unwrap().view.status,"closed");}
        else {assert!(f.manager.read_development_process("alice",&id).await.unwrap().finished_at.is_some());}
        assert!(f.manager.shutdown().await);
    }
}

#[tokio::test]
async fn service_coexistence_never_bypasses_a_cancelled_native_execution() {
    let f=Fixture::new().await; f.scope("agent",true).await; f.scope("service",true).await;
    let id=f.open().await;
    // Cancellation precedes asynchronous native cleanup; the directory remains
    // occupied until that owned process has actually stopped.
    f.manager.session_cancellation(&id).await.unwrap().0.cancel();
    assert_eq!(f.manager.start_development_process("alice",f.process(true),CancellationToken::new()).await.unwrap_err(),"workspace_busy");
    assert!(f.manager.close_session("alice",&id).await.unwrap());
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn entering_selected_workspace_activates_only_agent_and_preserves_explicit_revocation() {
    let f=Fixture::new().await;
    let project=f.manager.inner.repository.project("alice",&f.project).await.unwrap();
    let input=|task_id: Option<String>, revision| ActivateWorkspaceAgentInput {
        project_id:f.project.clone(), task_id, expected_path:project.path.clone(), expected_directory_identity:project.directory_identity.clone(), expected_revision:revision,
    };
    assert_eq!(f.manager.activate_development_workspace_agent("alice",input(None,0),CancellationToken::new()).await.unwrap_err(),"invalid_selection");
    f.manager.select_development_context("alice",dgos_db::development_repository::Selection {project_id:Some(f.project.clone()),task_id:None,session_id:None},CancellationToken::new()).await.unwrap();
    // Existing project metadata and ordinary reads must not implicitly grant.
    assert_eq!(f.manager.read_development_workspace_trust("alice",ReadWorkspaceTrustInput {project_id:f.project.clone(),task_id:None},CancellationToken::new()).await.unwrap().revision,0);
    let active=f.manager.activate_development_workspace_agent("alice",input(None,0),CancellationToken::new()).await.unwrap();
    assert!(active.agent && !active.agent_revoked && !active.terminal && !active.service);
    assert_eq!(f.manager.activate_development_workspace_agent("alice",input(None,1),CancellationToken::new()).await.unwrap().revision,1);
    f.manager.select_development_context("alice",dgos_db::development_repository::Selection {project_id:Some(f.project.clone()),task_id:Some(f.task.clone()),session_id:None},CancellationToken::new()).await.unwrap();
    f.scope("agent",false).await;
    let denied=f.manager.activate_development_workspace_agent("alice",input(Some(f.task.clone()),2),CancellationToken::new()).await.unwrap();
    assert!(denied.agent_revoked && !denied.agent && denied.revision==2);
    f.scope("agent",true).await;
    let enabled=f.manager.activate_development_workspace_agent("alice",input(Some(f.task.clone()),3),CancellationToken::new()).await.unwrap();
    assert!(enabled.agent && !enabled.agent_revoked);
    assert_eq!(f.manager.activate_development_workspace_agent("alice",input(None,3),CancellationToken::new()).await.unwrap_err(),"invalid_selection");
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn activation_rejects_foreign_owner_invalid_task_stale_identity_and_cancelled_account() {
    let f=Fixture::new().await;
    let project=f.manager.task_project("alice",&f.task).await.unwrap();
    let input=|| ActivateWorkspaceAgentInput {project_id:f.project.clone(),task_id:Some(f.task.clone()),expected_path:project.path.clone(),expected_directory_identity:project.directory_identity.clone(),expected_revision:0};
    f.manager.select_development_context("alice",dgos_db::development_repository::Selection {project_id:Some(f.project.clone()),task_id:Some(f.task.clone()),session_id:None},CancellationToken::new()).await.unwrap();
    assert_eq!(f.manager.activate_development_workspace_agent("bob",input(),CancellationToken::new()).await.unwrap_err(),"project_not_found");
    let mut invalid=input();invalid.task_id=Some("missing".into());
    assert_eq!(f.manager.activate_development_workspace_agent("alice",invalid,CancellationToken::new()).await.unwrap_err(),"task_not_found");
    let mut stale=input();stale.expected_directory_identity="other-directory".into();
    assert_eq!(f.manager.activate_development_workspace_agent("alice",stale,CancellationToken::new()).await.unwrap_err(),"workspace_changed");
    let mut stale_path=input();stale_path.expected_path.push_str("-foreign");
    assert_eq!(f.manager.activate_development_workspace_agent("alice",stale_path,CancellationToken::new()).await.unwrap_err(),"workspace_changed");
    let lease=CancellationToken::new();lease.cancel();
    assert_eq!(f.manager.activate_development_workspace_agent("alice",input(),lease).await.unwrap_err(),"session_invalid");
    std::fs::rename(f.root.path().join("code"),f.root.path().join("previous-code")).unwrap();
    std::fs::create_dir(f.root.path().join("code")).unwrap();
    assert_eq!(f.manager.activate_development_workspace_agent("alice",input(),CancellationToken::new()).await.unwrap_err(),"workspace_changed");
    assert_eq!(f.manager.inner.repository.workspace_trust("alice",&project).await.unwrap().revision,0);
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn selected_task_worktree_gets_its_own_grant_without_inheriting_project_directory() {
    let f=Fixture::new().await;
    let source=f.root.path().join("code");
    for args in [vec!["init","-b","main"],vec!["add","source"],vec!["-c","user.name=Fixture","-c","user.email=fixture@example.invalid","commit","-m","base"]] {
        let output=std::process::Command::new("git").current_dir(&source).args(args).output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    }
    let head=std::process::Command::new("git").current_dir(&source).args(["rev-parse","HEAD"]).output().unwrap();
    assert!(head.status.success());
    let head=String::from_utf8(head.stdout).unwrap().trim().to_owned();
    let target=f.root.path().join("isolated");
    f.manager.save_development_workspaces("alice",crate::workspaces::SaveWorkspaceInput {
        request_id:uuid::Uuid::new_v4().to_string(),task_id:f.task.clone(),action:"create".into(),workspace_id:None,path:Some(target.to_string_lossy().into()),branch:Some("task/trust".into()),base_commit:Some(head),retain_branch:None,
    },CancellationToken::new()).await.unwrap();
    let original=f.manager.inner.repository.project("alice",&f.project).await.unwrap();
    let worktree=f.manager.task_project("alice",&f.task).await.unwrap();
    assert_ne!(worktree.directory_identity,original.directory_identity);
    f.manager.select_development_context("alice",dgos_db::development_repository::Selection {project_id:Some(f.project.clone()),task_id:Some(f.task.clone()),session_id:None},CancellationToken::new()).await.unwrap();
    let wrong=ActivateWorkspaceAgentInput {project_id:f.project.clone(),task_id:Some(f.task.clone()),expected_path:original.path.clone(),expected_directory_identity:original.directory_identity.clone(),expected_revision:0};
    assert_eq!(f.manager.activate_development_workspace_agent("alice",wrong,CancellationToken::new()).await.unwrap_err(),"workspace_changed");
    let active=f.manager.activate_development_workspace_agent("alice",ActivateWorkspaceAgentInput {project_id:f.project.clone(),task_id:Some(f.task.clone()),expected_path:worktree.path.clone(),expected_directory_identity:worktree.directory_identity.clone(),expected_revision:0},CancellationToken::new()).await.unwrap();
    assert!(active.agent && !active.terminal && !active.service);
    assert_eq!(active.path,worktree.path);
    assert_eq!(f.manager.inner.repository.workspace_trust("alice",&original).await.unwrap().revision,0);
    let other_path=f.root.path().join("other-project");std::fs::create_dir(&other_path).unwrap();
    let other=f.manager.create_project("alice",CreateProjectInput {kind:"import".into(),name:"Another project".into(),path:other_path.to_string_lossy().into(),source:None},CancellationToken::new()).await.unwrap();
    let mismatched=ActivateWorkspaceAgentInput {project_id:other.id,task_id:Some(f.task.clone()),expected_path:worktree.path,expected_directory_identity:worktree.directory_identity,expected_revision:1};
    assert_eq!(f.manager.activate_development_workspace_agent("alice",mismatched,CancellationToken::new()).await.unwrap_err(),"task_not_found");
    assert!(f.manager.shutdown().await);
}
