//! Local-only reference smoke: real temporary files/SQLite and a deterministic
//! native-protocol child plus production local Git and PTY operations. No native
//! CLI, network, external model or keychain access; only synthetic fixture data.
use dgos_agents::{AgentManager, Engine, SessionOptions, capabilities::{CliProviders, CliProviderRef, CliProviderConfiguration, SaveCliProviderInput, SessionCreate}, files::{ReadFileInput, SaveFileInput}, git::{ExecuteGitInput, GitAction, GitHistoryInput}, projects::{CreateProjectInput, SaveTaskInput}, references::{ConversationReference, ReferenceKind}, runtime::StartProcessInput};
use dgos_db::{DevelopmentRepository, migrations::{Migrator, MigratorTrait}};
use std::{path::Path, sync::Arc};
use tokio_util::sync::CancellationToken;

struct SyntheticVault;
#[async_trait::async_trait]
impl dgos_providers::ProviderCredentialVault for SyntheticVault {
    async fn store(&self, _: &dgos_providers::ProviderCredentialScope, _: dgos_providers::SecretValue) -> dgos_types::DgosResult<()> { Ok(()) }
    async fn load(&self, _: &dgos_providers::ProviderCredentialScope) -> dgos_types::DgosResult<dgos_providers::SecretValue> { dgos_providers::SecretValue::new("synthetic-never-transmitted".into()) }
    async fn remove(&self, _: &dgos_providers::ProviderCredentialScope) -> dgos_types::DgosResult<()> { Ok(()) }
}
fn io(error: impl std::fmt::Display) -> String { error.to_string() }
fn require(value: bool, error: &str) -> Result<(), String> { if value { Ok(()) } else { Err(error.into()) } }
fn git(path: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = std::process::Command::new("git");
    for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_")) { command.env_remove(key); }
    let output = command.current_dir(path).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_TERMINAL_PROMPT", "0")
        .args(["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", "-c", "init.templateDir=", "-c", "commit.gpgSign=false", "-c", "gc.auto=0", "-c", "maintenance.auto=false"]).args(args).output().map_err(io)?;
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).into_owned()); }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn pull_request(manager: &AgentManager, task: &str, trust: bool) -> Result<ExecuteGitInput, String> {
    let preview = manager.preview_development_git_pull("fixture", task, "origin", "main", CancellationToken::new()).await?;
    Ok(ExecuteGitInput { id: uuid::Uuid::new_v4().to_string(), task_id: task.into(), review_token: preview.review_token, action: GitAction::Pull { remote: "origin".into(), branch: "main".into(), preview_token: preview.preview_token, trust_directory: trust, trust_network: trust } })
}

#[cfg(unix)]
async fn fixture(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, r##"#!/bin/sh
if [ "${1-}" = --version ]; then printf 'codex-cli 0.167.0\n'; exit 0; fi
if [ "${1-}" = --fixture-ready ]; then exit 0; fi
fixture_provider=
for argument do
  case "$argument" in model_provider=*) fixture_provider=${argument#*=}; fixture_provider=${fixture_provider#\"}; fixture_provider=${fixture_provider%\"};; esac
done
while IFS= read -r line; do
printf '%s\n' "$line" >> protocol.jsonl
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
*'"method":"config/read"'*) printf '{"id":%s,"result":{"config":{"model_provider":"'"$fixture_provider"'","model_providers":{"'"$fixture_provider"'":{"base_url":"http://127.0.0.1:9/v1","wire_api":"responses","env_key":"DGOS_PROVIDER_KEY","requires_openai_auth":false}}}}}\n' "$request";;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request";;
*'"method":"thread/start"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"'"$fixture_provider"'","thread":{"id":"references-smoke","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"skills/list"'*) printf '{"id":%s,"result":{"data":[{"cwd":"%s","skills":[{"name":"smoke-skill","path":"%s/.agents/skills/smoke-skill/SKILL.md","enabled":true,"scope":"repo"}],"errors":[]}]}}\n' "$request" "$PWD" "$PWD";;
*'"method":"turn/start"'*) printf '{"id":%s,"result":{"turn":{"id":"smoke-turn"}}}\n' "$request"; printf '%s\n' '{"method":"turn/completed","params":{"threadId":"references-smoke","turn":{"id":"smoke-turn","status":"completed"}}}';;
esac
done
"##).map_err(io)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
    let status = tokio::process::Command::new(path).arg("--fixture-ready").status().await.map_err(io)?;
    require(status.success(), "fixture_not_executable")
}

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("dgos-references-smoke-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("code")).map_err(io)?;
    let root = std::fs::canonicalize(root).map_err(io)?;
    std::fs::write(root.join("code/source.txt"), "original\n").map_err(io)?;
    git(&root.join("code"), &["init", "-b", "main"])?;
    git(&root.join("code"), &["config", "user.name", "DG-OS Smoke"])?;
    git(&root.join("code"), &["config", "user.email", "smoke@example.invalid"])?;
    git(&root.join("code"), &["add", "source.txt"])?;
    git(&root.join("code"), &["commit", "-m", "fixture baseline"])?;
    let url = format!("sqlite://{}?mode=rwc", root.join("metadata.sqlite").display());
    let db = dgos_db::establish_connection(&url).await.map_err(io)?;
    Migrator::up(&db, None).await.map_err(io)?;
    let repo = DevelopmentRepository::new(db.clone());
    let providers = dgos_providers::ProviderManager::with_vault(&url, Arc::new(SyntheticVault)).await.map_err(io)?;
    dgos_db::ProviderRepository::new(db).create_with_credential_ref("reference-fixture".into(), dgos_types::ProviderConfig { name: "Local protocol fixture".into(), provider_type: "openai_compatible".into(), base_url: "http://127.0.0.1:9/v1".into(), api_key: String::new(), enabled: true, config: Some(serde_json::json!({"allow_insecure_http":true,"auth_style":"bearer"})) }, "fixture".into(), Some("synthetic-only".into())).await.map_err(io)?;
    providers.add_manual_model("reference-fixture", "fixture", "fixture-model", None).await.map_err(io)?;
    let selected = providers.get_provider("reference-fixture", "fixture").await.map_err(io)?.ok_or("fixture_provider_missing")?;
    let provider = CliProviderRef { provider_id: selected.id, model_id: "fixture-model".into(), provider_revision: selected.config_revision };
    let capabilities = CliProviders::new(repo.clone(), providers)?;
    let manager = AgentManager::new(root.join("legacy.json"), repo, capabilities.clone());
    manager.load().await?;
    let result: Result<(), String> = async {
        let project = manager.create_project("fixture", CreateProjectInput { kind: "import".into(), name: "Reference smoke".into(), path: root.join("code").to_string_lossy().into(), source: None }, CancellationToken::new()).await?;
        let task = manager.save_development_task("fixture", SaveTaskInput { id: None, project_id: project.id.clone(), goal: "Inspect authorized reference bytes".into(), requirements: "Use only the deterministic protocol fixture".into(), status: "todo".into() }, CancellationToken::new()).await?;
        let revision = capabilities.save("fixture", SaveCliProviderInput { project_id: project.id.clone(), expected_revision: 0, configuration: CliProviderConfiguration { engine: Engine::Codex, provider: provider.clone() } }).await?;
        let read = manager.read_development_file("fixture", ReadFileInput { project_id: project.id.clone(), task_id: Some(task.id.clone()), path: "source.txt".into() }).await?;
        let saved = manager.save_development_file("fixture", SaveFileInput { project_id: project.id.clone(), task_id: task.id.clone(), path: "source.txt".into(), expected_hash: read.hash.ok_or("read_hash_missing")?, text: "saved through host\n".into() }, CancellationToken::new()).await?;
        std::fs::write(root.join("code/source.txt"), "external edit\n").map_err(io)?;
        let conflict = manager.save_development_file("fixture", SaveFileInput { project_id: project.id.clone(), task_id: task.id.clone(), path: "source.txt".into(), expected_hash: saved.hash.clone().ok_or("saved_hash_missing")?, text: "must not overwrite\n".into() }, CancellationToken::new()).await;
        require(matches!(&conflict, Err(error) if error == "file_changed"), "external_file_change_not_rejected")?;
        require(std::fs::read_to_string(root.join("code/source.txt")).map_err(io)? == "external edit\n", "external_content_lost")?;
        let current = manager.read_development_file("fixture", ReadFileInput { project_id: project.id.clone(), task_id: Some(task.id.clone()), path: "source.txt".into() }).await?;
        manager.save_development_file("fixture", SaveFileInput { project_id: project.id.clone(), task_id: task.id.clone(), path: "source.txt".into(), expected_hash: current.hash.ok_or("current_hash_missing")?, text: "saved through host\n".into() }, CancellationToken::new()).await?;
        let review = manager.read_development_git("fixture", &task.id).await?;
        let receipt = manager.execute_development_git("fixture", ExecuteGitInput { id: uuid::Uuid::new_v4().to_string(), task_id: task.id.clone(), review_token: review.review_token, action: GitAction::Stage { paths: vec!["source.txt".into()] } }, CancellationToken::new()).await?;
        require(receipt["status"] == "succeeded", "git_stage_failed")?;
        let review = manager.read_development_git("fixture", &task.id).await?;
        let receipt = manager.execute_development_git("fixture", ExecuteGitInput { id: uuid::Uuid::new_v4().to_string(), task_id: task.id.clone(), review_token: review.review_token, action: GitAction::Unstage { paths: vec!["source.txt".into()] } }, CancellationToken::new()).await?;
        require(receipt["status"] == "succeeded", "git_unstage_failed")?;
        let review = manager.read_development_git("fixture", &task.id).await?;
        let receipt = manager.execute_development_git("fixture", ExecuteGitInput { id: uuid::Uuid::new_v4().to_string(), task_id: task.id.clone(), review_token: review.review_token, action: GitAction::Stage { paths: vec!["source.txt".into()] } }, CancellationToken::new()).await?;
        require(receipt["status"] == "succeeded", "git_restage_failed")?;
        let review = manager.read_development_git("fixture", &task.id).await?;
        let receipt = manager.execute_development_git("fixture", ExecuteGitInput { id: uuid::Uuid::new_v4().to_string(), task_id: task.id.clone(), review_token: review.review_token, action: GitAction::Commit { message: "explicit smoke delivery".into(), confirm_full_index: true, confirm_preexisting_staged: true } }, CancellationToken::new()).await?;
        require(receipt["status"] == "succeeded", "git_commit_failed")?;
        let history = manager.read_development_git_history("fixture", GitHistoryInput { task_id: task.id.clone(), start_oid: None, cursor: None, limit: 10 }).await?;
        require(history.entries.iter().any(|entry| entry.subject == "explicit smoke delivery"), "git_history_missing_commit")?;
        // Fixture setup uses only a local bare remote. Every pull below is the
        // production preview/explicit-action path, never a shell `git pull`.
        let code = root.join("code");
        let remote = root.join("remote.git");
        let peer = root.join("peer");
        git(&root, &["init", "--bare", remote.to_str().ok_or("remote_path")?])?;
        git(&code, &["remote", "add", "origin", remote.to_str().ok_or("remote_path")?])?;
        git(&code, &["push", "origin", "main"])?;
        git(&root, &["clone", "--branch", "main", remote.to_str().ok_or("remote_path")?, peer.to_str().ok_or("peer_path")?])?;
        git(&peer, &["config", "user.name", "DG-OS Peer"])?;
        git(&peer, &["config", "user.email", "peer@example.invalid"])?;
        std::fs::write(peer.join("remote.txt"), "remote-only change\n").map_err(io)?;
        git(&peer, &["add", "remote.txt"])?;
        git(&peer, &["commit", "-m", "remote fast forward"])?;
        git(&peer, &["push", "origin", "main"])?;
        let before = git(&code, &["rev-parse", "HEAD"])?;
        let index = git(&code, &["ls-files", "--stage"])?;
        let untrusted = pull_request(&manager, &task.id, false).await?;
        require(matches!(manager.execute_development_git("fixture", untrusted, CancellationToken::new()).await, Err(error) if error == "git_pull_trust_required"), "pull_without_explicit_trust_not_rejected")?;
        let stale = pull_request(&manager, &task.id, true).await?;
        git(&code, &["remote", "set-url", "origin", root.join("absent.git").to_str().ok_or("absent_path")?])?;
        require(matches!(manager.execute_development_git("fixture", stale, CancellationToken::new()).await, Err(error) if error == "git_pull_preview_stale"), "changed_remote_not_rejected")?;
        git(&code, &["remote", "set-url", "origin", remote.to_str().ok_or("remote_path")?])?;
        std::fs::write(code.join("source.txt"), "keep local user modification\n").map_err(io)?;
        std::fs::write(code.join("user-untracked.txt"), "keep untracked bytes\n").map_err(io)?;
        let dirty = pull_request(&manager, &task.id, true).await?;
        require(matches!(manager.execute_development_git("fixture", dirty, CancellationToken::new()).await, Err(error) if error == "git_worktree_dirty"), "dirty_pull_not_rejected")?;
        require(git(&code, &["rev-parse", "HEAD"])? == before && git(&code, &["ls-files", "--stage"])? == index && std::fs::read_to_string(code.join("source.txt")).map_err(io)? == "keep local user modification\n" && std::fs::read_to_string(code.join("user-untracked.txt")).map_err(io)? == "keep untracked bytes\n", "rejected_pull_lost_user_work")?;
        // Preserve the user's modifications by explicitly delivering them, not
        // by stashing/resetting/deleting them to satisfy the clean-tree guard.
        let review = manager.read_development_git("fixture", &task.id).await?;
        let staged = manager.execute_development_git("fixture", ExecuteGitInput { id: uuid::Uuid::new_v4().to_string(), task_id: task.id.clone(), review_token: review.review_token, action: GitAction::Stage { paths: vec!["source.txt".into(), "user-untracked.txt".into()] } }, CancellationToken::new()).await?;
        require(staged["status"] == "succeeded", "user_work_stage_failed")?;
        let review = manager.read_development_git("fixture", &task.id).await?;
        let committed = manager.execute_development_git("fixture", ExecuteGitInput { id: uuid::Uuid::new_v4().to_string(), task_id: task.id.clone(), review_token: review.review_token, action: GitAction::Commit { message: "preserve local user work".into(), confirm_full_index: true, confirm_preexisting_staged: true } }, CancellationToken::new()).await?;
        require(committed["status"] == "succeeded", "user_work_commit_failed")?;
        let local_head = git(&code, &["rev-parse", "HEAD"])?;
        let diverged = pull_request(&manager, &task.id, true).await?;
        let rejected = manager.execute_development_git("fixture", diverged, CancellationToken::new()).await?;
        require(rejected["status"] == "failed" && rejected["details"]["error"] == "git_pull_diverged" && git(&code, &["rev-parse", "HEAD"])? == local_head && std::fs::read_to_string(code.join("source.txt")).map_err(io)? == "keep local user modification\n", "diverged_pull_changed_local_work")?;
        // The peer explicitly incorporates the delivered local commit, making
        // the next request a real fast-forward without destructive local repair.
        git(&code, &["push", "origin", "HEAD:refs/heads/user-work"])?;
        git(&peer, &["fetch", "origin", "user-work"])?;
        git(&peer, &["merge", "--no-edit", "FETCH_HEAD"])?;
        git(&peer, &["push", "origin", "main"])?;
        let final_target = git(&peer, &["rev-parse", "HEAD"])?;
        let input = pull_request(&manager, &task.id, true).await?;
        let pulled = manager.execute_development_git("fixture", input.clone(), CancellationToken::new()).await?;
        require(pulled["status"] == "succeeded" && pulled["details"]["phase"] == "completed" && git(&code, &["rev-parse", "HEAD"])? == final_target && std::fs::read_to_string(code.join("source.txt")).map_err(io)? == "keep local user modification\n" && std::fs::read_to_string(code.join("user-untracked.txt")).map_err(io)? == "keep untracked bytes\n" && std::fs::read_to_string(code.join("remote.txt")).map_err(io)? == "remote-only change\n", "explicit_pull_or_user_preservation_failed")?;
        require(manager.execute_development_git("fixture", input, CancellationToken::new()).await? == pulled, "pull_uuid_replay_changed_receipt")?;
        require(git(&code, &["for-each-ref", "--format=%(refname)", "refs/dgos-pull"])?.is_empty(), "pull_owned_ref_not_cleaned")?;
        // Restore the reference fixture's intended bytes through the host CAS.
        let current = manager.read_development_file("fixture", ReadFileInput { project_id: project.id.clone(), task_id: Some(task.id.clone()), path: "source.txt".into() }).await?;
        let saved = manager.save_development_file("fixture", SaveFileInput { project_id: project.id.clone(), task_id: task.id.clone(), path: "source.txt".into(), expected_hash: current.hash.ok_or("post_pull_hash_missing")?, text: "saved through host\n".into() }, CancellationToken::new()).await?;
        let terminal = StartProcessInput { id: uuid::Uuid::new_v4().to_string(), task_id: task.id.clone(), kind: "terminal".into(), program: "/bin/sh".into(), args: vec!["-c".into(), "stty -echo; printf 'terminal-ready\\n'; read line; stty size; printf 'terminal-input:%s\\n' \"$line\"".into()], cwd: String::new(), env_refs: Default::default(), trust_workspace: true, verification: false, readiness: None, native_run_id: None };
        manager.start_development_process("fixture", terminal.clone(), CancellationToken::new()).await?;
        tokio::time::timeout(std::time::Duration::from_secs(5), async { loop { let view = manager.read_development_process("fixture", &terminal.id).await?; if view.output.contains("terminal-ready") { return Ok::<_, String>(()); } tokio::time::sleep(std::time::Duration::from_millis(20)).await; } }).await.map_err(|_| "terminal_start_timeout")??;
        manager.resize_development_terminal("fixture", &terminal.id, 101, 37).await?;
        manager.write_development_terminal("fixture", &terminal.id, "smoke-input\n".into()).await?;
        let finished = tokio::time::timeout(std::time::Duration::from_secs(5), async { loop { let view = manager.read_development_process("fixture", &terminal.id).await?; if view.finished_at.is_some() { return Ok::<_, String>(view); } tokio::time::sleep(std::time::Duration::from_millis(20)).await; } }).await.map_err(|_| "terminal_exit_timeout")??;
        require(finished.output.contains("37 101") && finished.output.contains("terminal-input:smoke-input"), "terminal_io_or_resize_failed")?;
        require(finished.exit_code == Some(0), "terminal_exit_code_missing")?;
        let sleeper = StartProcessInput { id: uuid::Uuid::new_v4().to_string(), task_id: task.id.clone(), kind: "terminal".into(), program: "/bin/sh".into(), args: vec!["-c".into(), "printf 'stop-ready\\n'; exec sleep 30".into()], cwd: String::new(), env_refs: Default::default(), trust_workspace: true, verification: false, readiness: None, native_run_id: None };
        manager.start_development_process("fixture", sleeper.clone(), CancellationToken::new()).await?;
        tokio::time::timeout(std::time::Duration::from_secs(5), async { loop { let view = manager.read_development_process("fixture", &sleeper.id).await?; if view.output.contains("stop-ready") { return Ok::<_, String>(()); } tokio::time::sleep(std::time::Duration::from_millis(20)).await; } }).await.map_err(|_| "terminal_stop_start_timeout")??;
        let stopped_terminal = manager.stop_development_process("fixture", &sleeper.id).await?;
        require(stopped_terminal.status == "cancelled" && stopped_terminal.finished_at.is_some(), "terminal_stop_unconfirmed")?;
        let executable = root.join("codex-fixture"); fixture(&executable).await?;
        let skill_path = root.join("code/.agents/skills/smoke-skill/SKILL.md");
        std::fs::create_dir_all(skill_path.parent().unwrap()).map_err(io)?;
        std::fs::write(&skill_path, "---\nname: smoke-skill\ndescription: local protocol input proof\n---\nRead only the supplied reference.\n").map_err(io)?;
        let skill = dgos_agents::session::AgentSkillInput { name: "smoke-skill".into(), path: skill_path.to_string_lossy().into_owned() };
        let id = uuid::Uuid::new_v4().to_string();
        let opened = manager.open_session("fixture", Engine::Codex, id.clone(), SessionCreate { options: SessionOptions { workspace: project.path, resume_id: None, executable: Some(executable.to_string_lossy().into()), model: None, trust_workspace: true }, project_revision: revision.revision, provider }, Some(task.id.clone()), CancellationToken::new()).await?;
        let reference = ConversationReference { project_id: project.id, task_id: Some(task.id.clone()), path: "source.txt".into(), kind: ReferenceKind::File, diff_source: None, expected_hash: saved.hash, start_line: None, end_line: None };
        let request = uuid::Uuid::new_v4().to_string();
        manager.send("fixture", &id, &opened.generation, request.clone(), "Review attached host content".into(), None, vec![reference.clone()], vec![skill.clone()], vec![]).await?;
        std::fs::remove_file(root.join("code/source.txt")).map_err(io)?;
        std::fs::remove_file(&skill_path).map_err(io)?;
        manager.send("fixture", &id, &opened.generation, request.clone(), "Review attached host content".into(), None, vec![reference], vec![skill.clone()], vec![]).await?;
        tokio::time::timeout(std::time::Duration::from_secs(5), async { loop { if manager.list_development_runs("fixture", &task.id).await?.iter().any(|run| run.id == request && run.status == "completed") { return Ok::<_, String>(()); } tokio::time::sleep(std::time::Duration::from_millis(20)).await; } }).await.map_err(|_| "run_timeout")??;
        let protocol: Vec<serde_json::Value> = std::fs::read_to_string(root.join("code/protocol.jsonl")).map_err(io)?.lines().map(serde_json::from_str).collect::<Result<_, _>>().map_err(io)?;
        let sends: Vec<_> = protocol.iter().filter(|event| event["method"] == "turn/start").collect();
        require(sends.len() == 1, "duplicate_dispatch")?;
        require(sends[0]["params"]["input"][0]["text"].as_str().is_some_and(|text| text.contains("host-read content follows") && text.ends_with("saved through host\n")), "resolved_content_not_serialized")?;
        require(sends[0]["params"]["input"].as_array().is_some_and(|items| items.iter().any(|item| item["type"] == "skill" && item["name"] == skill.name && item["path"] == skill.path)), "structured_skill_not_dispatched")?;
        let full = manager.snapshot("fixture", &id).await?;
        let last = full.view.events.last().ok_or("missing_terminal_event")?.seq;
        let delta = manager.snapshot_since("fixture", &id, Some(dgos_agents::session::SessionCursor { generation: opened.generation.clone(), after_seq: last - 1 })).await?;
        require(delta.event_sync.as_ref().is_some_and(|sync| !sync.reset) && delta.view.events.len() == 1 && delta.view.events[0].seq == last, "incremental_terminal_missing")?;
        let unchanged = manager.snapshot_since("fixture", &id, Some(dgos_agents::session::SessionCursor { generation: opened.generation.clone(), after_seq: last })).await?;
        require(unchanged.view.events.is_empty() && unchanged.view.status == "idle", "unchanged_snapshot_repeats_history")?;
        let restored = manager.snapshot_since("fixture", &id, Some(dgos_agents::session::SessionCursor { generation: uuid::Uuid::new_v4().to_string(), after_seq: last })).await?;
        require(restored.event_sync.as_ref().is_some_and(|sync| sync.reset) && restored.view.events.len() == full.view.events.len(), "generation_reset_missing")?;
        require(manager.snapshot_since("other-owner", &id, None).await.is_err(), "incremental_snapshot_owner_leak")?;
        println!("structured_skill=true incremental_terminal=true unchanged_history_omitted=true generation_reset=true owner_isolated=true");
        let evidence = serde_json::json!({"kind":"local_file_git_terminal_protocol_smoke","passed":true,"model_calls":0,"network_calls":0,"native_cli":false,"root":root,"request_id":request,"dispatch_count":sends.len(),"git_staged_unstaged_committed_history":true,"file_save_and_external_conflict":true,"pty_resize_input_output_stop":true,"local_bare_remote_pull":{"trust_rejected":true,"changed_remote_rejected":true,"dirty_user_work_preserved":true,"divergence_rejected":true,"explicit_fast_forward":pulled,"uuid_replay":true},"protocol_turn":sends[0]});
        let evidence_path = root.with_extension("json");
        std::fs::write(&evidence_path, serde_json::to_vec_pretty(&evidence).map_err(io)?).map_err(io)?;
        println!("{}", serde_json::json!({"evidence_path":evidence_path,"result":evidence}));
        Ok(())
    }.await;
    let stopped = manager.shutdown().await;
    require(stopped, "fixture_cleanup_unconfirmed")?;
    std::fs::remove_dir_all(&root).map_err(io)?;
    result
}

#[cfg(not(unix))]
fn main() { eprintln!("references_smoke requires a Unix deterministic protocol child"); }
