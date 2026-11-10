use super::*;
use crate::projects::{CreateProjectInput,SaveTaskInput,UpdateProjectInput};

async fn setup()->(std::path::PathBuf,AgentManager,String,String) {
    let root=std::env::temp_dir().join(format!("dgos-runtime-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(root.join("code")).unwrap();let root=root.canonicalize().unwrap();std::fs::write(root.join("code/source.txt"),"original\n").unwrap();
    let manager=crate::test_support::manager(&root).await;
    let project=manager.create_project("alice",CreateProjectInput{kind:"import".into(),name:"Runtime".into(),path:root.join("code").to_string_lossy().into_owned(),source:None},CancellationToken::new()).await.unwrap();
    let task=manager.save_development_task("alice",SaveTaskInput{id:None,project_id:project.id.clone(),goal:"Verify code".into(),requirements:"Observed process result".into(),status:"todo".into()},CancellationToken::new()).await.unwrap();
    (root,manager,project.id,task.id)
}
fn command(task:&str,script:&str)->StartProcessInput {StartProcessInput{id:uuid::Uuid::new_v4().to_string(),task_id:task.into(),kind:"command".into(),program:"/bin/sh".into(),args:vec!["-c".into(),script.into()],cwd:String::new(),env_refs:BTreeMap::new(),trust_workspace:true,verification:false,readiness:None,native_run_id:None}}
async fn until(manager:&AgentManager,id:&str,predicate:impl Fn(&ProcessSnapshot)->bool)->ProcessSnapshot {
    tokio::time::timeout(Duration::from_secs(12),async {loop {let view=manager.read_development_process("alice",id).await.unwrap();if predicate(&view){return view;}tokio::time::sleep(Duration::from_millis(40)).await;}}).await.expect("runtime did not converge")
}
async fn finished(manager:&AgentManager,id:&str)->ProcessSnapshot {until(manager,id,|s|s.finished_at.is_some()).await}

#[test]
fn redaction_spans_reads_and_bounded_partial_tokens_do_not_leak() {
    let mut output=Output::default();output.push(b"safe sk-");assert!(!output.view(false).contains("sk-"));output.push(b"private\nAuthorization: Bear");assert!(!output.view(false).contains("private"));output.push(b"er secret-value\n");assert!(!output.view(true).contains("secret-value"));
    output.push(&vec![b'x';OUTPUT_LIMIT]);assert!(output.truncated);assert!(!output.view(true).contains("private"));
}

#[test]
fn bounded_output_keeps_the_prefix_of_the_chunk_crossing_the_limit() {
    let mut output=Output::default();output.push(b"first\n");
    output.push(&vec![b'x';OUTPUT_LIMIT]);
    assert_eq!(output.raw.len(),OUTPUT_LIMIT);assert!(output.raw.starts_with(b"first\n"));assert!(output.truncated);
    output.push(b"must not replace the captured prefix\n");
    assert_eq!(output.raw.len(),OUTPUT_LIMIT);assert_eq!(output.view(true),"first\n");
}

#[cfg(unix)]
#[tokio::test]
async fn saturated_output_retains_its_prefix_and_drains_to_actual_exit() {
    let(root,manager,_,task)=setup().await;
    let input=command(&task,"printf 'captured-prefix\\n'; i=0; while [ \"$i\" -lt 20000 ]; do printf 'captured-output captured-output captured-output captured-output\\n'; i=$((i+1)); done");
    manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();
    let view=finished(&manager,&input.id).await;
    assert_eq!(view.exit_code,Some(0));assert_eq!(view.status,"exited");assert!(view.truncated);
    assert!(view.output.starts_with("captured-prefix\n"));assert!(view.output.len()<=OUTPUT_LIMIT);
    let stored=manager.inner.repository.run("alice",&input.id).await.unwrap().unwrap();
    assert!(stored.truncated);assert_eq!(process_view(stored).unwrap().output,view.output);
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn command_receipts_isolation_configuration_and_restart_never_replay() {
    let(root,manager,project,task)=setup().await;
    let configured=CommandConfiguration{id:"check".into(),name:"Check".into(),program:"/bin/sh".into(),args:vec!["-c".into(),"exit 0".into()],cwd:String::new(),env_refs:BTreeMap::new()};
    manager.save_development_commands("alice",&task,vec![configured]).await.unwrap();assert!(manager.read_development_commands("bob",&task).await.is_err());
    let input=command(&task,"printf 'once\\n' >> invocations; printf 'result\\n'; exit 7");manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();
    assert!(manager.read_development_process("bob",&input.id).await.is_err());assert!(manager.write_development_terminal("bob",&input.id,"x".into()).await.is_err());
    let view=finished(&manager,&input.id).await;assert_eq!(view.exit_code,Some(7));assert_eq!(view.status,"failed");assert!(view.output.contains("result"));
    manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();let mut conflict=input.clone();conflict.args=vec!["-c".into(),"exit 0".into()];assert_eq!(manager.start_development_process("alice",conflict,CancellationToken::new()).await.unwrap_err(),"request_conflict");
    let restarted=crate::test_support::manager(&root).await;let history=restarted.read_development_process("alice",&input.id).await.unwrap();assert_eq!(history.exit_code,Some(7));assert_eq!(restarted.read_development_commands("alice",&task).await.unwrap().commands.len(),1);assert_eq!(std::fs::read_to_string(root.join("code/invocations")).unwrap(),"once\n");
    manager.update_project("alice",UpdateProjectInput{project_id:project.clone(),action:"unlink".into(),value:None},CancellationToken::new()).await.unwrap();assert!(manager.read_development_process("alice",&input.id).await.is_err());std::fs::remove_dir_all(root).unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn real_pty_input_resize_and_explicit_exit_are_bound_to_instance() {
    let(root,manager,_,task)=setup().await;let mut input=command(&task,"stty -echo; printf 'ready\\n'; read line; stty size; printf 'received:%s\\n' \"$line\"; exit 3");input.kind="terminal".into();
    manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();until(&manager,&input.id,|v|v.output.contains("ready")).await;
    manager.resize_development_terminal("alice",&input.id,101,37).await.unwrap();manager.write_development_terminal("alice",&input.id,"hello\n".into()).await.unwrap();let view=finished(&manager,&input.id).await;assert_eq!(view.exit_code,Some(3));assert!(view.output.contains("37 101"));assert!(view.output.contains("received:hello"));assert!(manager.write_development_terminal("alice",&input.id,"late".into()).await.is_err());std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn verification_tracks_stable_stale_incomplete_and_reverted_manual_mutation() {
    let(root,manager,_,task)=setup().await;let mut input=command(&task,"sleep 1; exit 0");input.verification=true;
    manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();let stable=finished(&manager,&input.id).await;assert_eq!(stable.verification_status,"observed_stable");
    std::fs::write(root.join("code/source.txt"),"changed\n").unwrap();assert_eq!(manager.read_development_process("alice",&input.id).await.unwrap().verification_status,"stale");std::fs::write(root.join("code/source.txt"),"original\n").unwrap();
    input.id=uuid::Uuid::new_v4().to_string();manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();manager.mark_development_mutation(root.join("code").to_str().unwrap()).await;assert_eq!(finished(&manager,&input.id).await.verification_status,"changed");
    std::fs::write(root.join("code/binary"),[0,1,2]).unwrap();input.id=uuid::Uuid::new_v4().to_string();manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();assert_eq!(finished(&manager,&input.id).await.verification_status,"incomplete");std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn periodic_snapshot_observes_mutation_and_retains_it_after_restore() {
    let(root,manager,_,task)=setup().await;let mut input=command(&task,"printf 'changed\\n' > source.txt; sleep 2; printf 'original\\n' > source.txt; sleep 1; exit 0");input.verification=true;
    manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();let view=finished(&manager,&input.id).await;assert!(view.mutation_observed);assert_eq!(view.fingerprint,view.start_fingerprint);assert_eq!(view.verification_status,"changed");std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn service_conflict_never_kills_other_listener_and_readiness_failure_cleans_owned_process() {
    let(root,manager,_,task)=setup().await;let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let port=listener.local_addr().unwrap().port();let mut input=command(&task,"sleep 30");input.kind="service".into();input.readiness=Some(ReadinessInput{port,path:"/".into(),timeout_ms:300});
    manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();let view=finished(&manager,&input.id).await;assert_eq!(view.reason.as_deref(),Some("port_conflict"));assert!(tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST,port)).await.is_ok());drop(listener);
    input.id=uuid::Uuid::new_v4().to_string();manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();let view=finished(&manager,&input.id).await;assert_eq!(view.reason.as_deref(),Some("service_readiness_timeout"));assert!(view.preview_url.is_none());assert!(manager.inner.host.lock().await.is_empty());std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn ready_requires_http_and_only_ready_has_loopback_preview() {
    let(root,manager,_,task)=setup().await;let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();drop(listener);
    let mut input=command(&task,"");input.program="/usr/bin/python3".into();input.args=vec!["-c".into(),format!("import http.server;http.server.HTTPServer(('127.0.0.1',{port}),http.server.SimpleHTTPRequestHandler).serve_forever()")];input.kind="service".into();input.readiness=Some(ReadinessInput{port,path:"/".into(),timeout_ms:5000});
    manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();let view=until(&manager,&input.id,|v|v.status=="ready"||v.finished_at.is_some()).await;assert_eq!(view.status,"ready");assert_eq!(view.preview_url,Some(format!("http://127.0.0.1:{port}/")));let stopped=manager.stop_development_process("alice",&input.id).await.unwrap();assert_eq!(stopped.status,"cancelled");assert!(stopped.preview_url.is_none());std::fs::remove_dir_all(root).unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn lease_revocation_stops_descendant_and_blocks_agent_and_project_mutation() {
    let(root,manager,project,task)=setup().await;let lease=CancellationToken::new();let input=command(&task,"sleep 30 & printf 'child:%s\\n' $!; wait");manager.start_development_process("alice",input.clone(),lease.clone()).await.unwrap();let view=until(&manager,&input.id,|v|v.output.contains("child:")).await;let pid=view.output.trim().strip_prefix("child:").unwrap().parse::<i32>().unwrap();
    assert_eq!(manager.host_workspace_available(root.join("code").to_str().unwrap()).await.unwrap_err(),"workspace_busy");assert!(manager.update_project("alice",UpdateProjectInput{project_id:project,action:"archive".into(),value:None},CancellationToken::new()).await.is_err());
    lease.cancel();let stopped=finished(&manager,&input.id).await;assert_ne!(stopped.status,"running");
    // A killed descendant may briefly remain a system-owned zombie; it must never execute.
    let alive=unsafe{libc::kill(pid,0)}==0;assert!(!alive || stopped.status=="cleanup_pending");
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn simulated_restart_marks_live_history_unknown_without_replaying_or_claiming_child() {
    let(root,manager,_,task)=setup().await;let input=command(&task,"printf 'once\\n' >> invocations; sleep 30");manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();until(&manager,&input.id,|_|root.join("code/invocations").exists()).await;
    let restarted=crate::test_support::manager(&root).await;let history=restarted.read_development_process("alice",&input.id).await.unwrap();assert_eq!(history.status,"unknown");assert_eq!(history.reason.as_deref(),Some("interrupted_by_restart"));restarted.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();assert!(restarted.inner.host.lock().await.is_empty());assert_eq!(std::fs::read_to_string(root.join("code/invocations")).unwrap(),"once\n");manager.stop_development_process("alice",&input.id).await.unwrap();std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn account_cleanup_preserves_other_account_child_and_environment_is_explicit() {
    let(root,manager,_,task)=setup().await;
    std::fs::write(root.join("code/package.json"),r#"{"scripts":{"test":"exit 0","--danger":"exit 1"}}"#).unwrap();std::fs::write(root.join("code/Cargo.toml"),"[package]\nname='sample'\nversion='0.1.0'\n").unwrap();
    let catalog=manager.read_development_commands("alice",&task).await.unwrap();assert!(catalog.discovered.iter().any(|c|c.id=="package:test"));assert!(catalog.discovered.iter().any(|c|c.id=="cargo:test"));assert!(!catalog.discovered.iter().any(|c|c.id=="package:--danger"));
    let input=command(&task,"printf 'alive\\n'; sleep 30");manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();until(&manager,&input.id,|v|v.output.contains("alive")).await;
    assert!(manager.cancel_user("bob").await);assert_eq!(manager.read_development_process("alice",&input.id).await.unwrap().status,"running");assert!(manager.cancel_user("alice").await);assert_eq!(manager.read_development_process("alice",&input.id).await.unwrap().status,"cancelled");
    let input=command(&task,"printf 'environment:%s\\n' \"${DGOS_UNREFERENCED_SECRET-unset}\"");manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();assert!(finished(&manager,&input.id).await.output.contains("environment:unset"));
    assert!(manager.shutdown().await);std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn readiness_rejects_unrelated_http_listener_after_preflight() {
    use tokio::io::{AsyncReadExt,AsyncWriteExt};
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let port=listener.local_addr().unwrap().port();
    let responder=tokio::spawn(async move {loop {let Ok((mut stream,_))=listener.accept().await else{break;};tokio::spawn(async move {let mut request=[0;1024];let _=stream.read(&mut request).await;let _=stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;});}});
    // A different, live owned session must not claim this process's listener.
    let(root,manager,_,task)=setup().await;let input=command(&task,"printf 'started:%s\\n' $$; sleep 30");
    manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();let running=until(&manager,&input.id,|v|v.output.contains("started:")).await;let session=running.output.trim().strip_prefix("started:").unwrap().parse().unwrap();
    assert!(!http_ready(&ReadinessInput{port,path:"/".into(),timeout_ms:500},session).await);
    manager.stop_development_process("alice",&input.id).await.unwrap();responder.abort();std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn command_cleanup_includes_descendants_in_different_job_groups() {
    let(root,manager,_,task)=setup().await;
    let mut input=command(&task,"");input.program="/usr/bin/python3".into();input.args=vec!["-u".into(),"-c".into(),"import os,signal,time\npid=os.fork()\nif pid==0:\n os.setpgid(0,0)\n signal.signal(signal.SIGTERM,signal.SIG_IGN)\n print('child:%s'%os.getpid(),flush=True)\n while True: time.sleep(1)\nwhile True: time.sleep(1)".into()];
    manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();
    let running=until(&manager,&input.id,|v|v.output.contains("child:")).await;let pid=running.output.trim().strip_prefix("child:").unwrap().parse::<i32>().unwrap();
    manager.stop_development_process("alice",&input.id).await.unwrap();
    // Even when a system-owned zombie prevents complete reclamation, the child
    // must have received SIGKILL rather than continuing its own job group.
    let gone=unsafe{libc::kill(pid,0)}<0;
    #[cfg(target_os="linux")]
    let gone=gone || std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s|s.rsplit_once(')').is_some_and(|(_,tail)|tail.trim_start().starts_with('Z')));
    #[cfg(target_os="macos")]
    let gone=gone || {
        let mut info:libc::proc_bsdinfo=unsafe{std::mem::zeroed()};
        let n=unsafe{libc::proc_pidinfo(pid,libc::PROC_PIDTBSDINFO,0,(&mut info as *mut libc::proc_bsdinfo).cast(),std::mem::size_of_val(&info) as i32)};
        n==std::mem::size_of_val(&info) as i32 && info.pbi_status==libc::SZOMB
    };
    assert!(gone,"descendant in a separate job group kept executing");std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn signalled_verification_without_exit_code_is_unknown_and_persisted() {
    let(root,manager,_,task)=setup().await;let mut input=command(&task,"kill -TERM $$");input.verification=true;
    manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();let view=finished(&manager,&input.id).await;
    assert_eq!(view.exit_code,None);assert_eq!(view.verification_status,"unknown");
    let restarted=crate::test_support::manager(&root).await;let persisted=restarted.read_development_process("alice",&input.id).await.unwrap();assert_eq!(persisted.verification_status,"unknown");assert_eq!(persisted.exit_code,None);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn private_key_and_auth_output_stay_redacted_at_every_chunk_boundary() {
    let bytes=b"safe\n-----BEGIN PRIVATE KEY-----\nsecret material\n-----END PRIVATE KEY-----\nAuthorization: Bearer secret-value\n";
    let mut output=Output::default();
    for byte in bytes {output.push(&[*byte]);let view=output.view(false);assert!(!view.contains("secret"));}
    assert!(!output.view(true).contains("secret"));
}

#[test]
fn resolved_environment_values_are_redacted_including_whitespace_and_partial_suffixes() {
    let mut output=Output{secrets:vec![b"unusual value with spaces".to_vec(),vec![0xff,b'x']],..Output::default()};
    output.push(b"safe\nunusual value ");assert!(!output.view(false).contains("unusual"));
    output.push(b"with spaces\n");output.push(&[0xff,b'x',b'\n']);
    let view=output.view(true);assert!(!view.contains("unusual"));assert!(!view.contains('\u{fffd}'));assert!(view.contains("[REDACTED]"));
}

#[tokio::test]
async fn environment_reference_values_never_enter_snapshots_or_durable_history() {
    let(root,manager,_,task)=setup().await;let mut input=command(&task,"printf 'prefix:'; printf '%s' \"$RESOLVED\"; printf '\\n' ");input.env_refs.insert("RESOLVED".into(),"HOME".into());
    let secret=std::env::var("HOME").unwrap();assert!(!secret.is_empty());
    manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();let view=finished(&manager,&input.id).await;assert!(!view.output.contains(&secret));assert!(view.output.contains("[REDACTED]"));
    let run=manager.inner.repository.run("alice",&input.id).await.unwrap().unwrap();assert!(!run.runtime.unwrap().contains(&secret));std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn known_credentials_are_rejected_in_command_names_and_working_directories() {
    let(root,manager,_,task)=setup().await;
    let configuration=CommandConfiguration{id:"check".into(),name:"sk-private".into(),program:"/bin/sh".into(),args:vec![],cwd:String::new(),env_refs:BTreeMap::new()};
    assert!(manager.save_development_commands("alice",&task,vec![configuration]).await.is_err());
    assert_eq!(validate_command("/bin/sh",&[],"ghp_private",&BTreeMap::new()).unwrap_err(),"secret_argument_denied");
    assert!(manager.read_development_commands("alice",&task).await.unwrap().commands.is_empty());std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn sqlite_checkpoint_failure_reaps_child_but_retains_ownership_until_terminal_is_durable() {
    use dgos_db::sea_orm::ConnectionTrait;
    let(root,manager,_,task)=setup().await;
    let db=dgos_db::establish_connection(&format!("sqlite://{}?mode=rwc",root.join("metadata.sqlite").display())).await.unwrap();
    db.execute_unprepared("CREATE TRIGGER deny_host_checkpoint BEFORE UPDATE ON development_runs WHEN NEW.engine='host' BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
    let input=command(&task,"printf 'once\\n' >> invocations; printf 'retained code\\n' > source.txt; printf '%s' $$ > child-pid; printf 'checkpoint output\\n'; sleep 30");
    manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(8),async {
        loop {
            let view=manager.read_development_process("alice",&input.id).await.unwrap();
            if view.finished_at.is_some() {break;}
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    let pid=std::fs::read_to_string(root.join("code/child-pid")).unwrap().parse::<i32>().unwrap();
    assert_ne!(unsafe{libc::kill(pid,0)},0,"storage failure left the owned command executing");
    assert!(manager.inner.host.lock().await.contains_key(&input.id));
    assert!(manager.inner.gate.begin_maintenance().is_err());
    assert_eq!(manager.inner.watchers.lock().await.len(),1);
    assert!(!manager.inner.watchers.lock().await[0].is_finished());
    assert_eq!(manager.inner.repository.run("alice",&input.id).await.unwrap().unwrap().status,"running");
    db.execute_unprepared("DROP TRIGGER deny_host_checkpoint").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5),async {
        while manager.inner.host.lock().await.contains_key(&input.id) {tokio::time::sleep(Duration::from_millis(20)).await;}
    }).await.unwrap();
    assert!(manager.inner.gate.begin_maintenance().is_ok());
    assert_eq!(manager.inner.repository.run("alice",&input.id).await.unwrap().unwrap().status,"failed");
    assert!(manager.shutdown().await);
    let restarted=crate::test_support::manager(&root).await;
    let history=restarted.read_development_process("alice",&input.id).await.unwrap();
    assert_eq!(history.status,"failed");assert_eq!(history.reason.as_deref(),Some("storage_failed"));assert!(history.preview_url.is_none());
    assert_eq!(restarted.start_development_process("alice",input,CancellationToken::new()).await.unwrap().status,"failed");
    assert!(restarted.inner.host.lock().await.is_empty());
    assert_eq!(std::fs::read_to_string(root.join("code/invocations")).unwrap(),"once\n");
    assert_eq!(std::fs::read_to_string(root.join("code/source.txt")).unwrap(),"retained code\n");
    assert_eq!(restarted.inner.repository.task("alice",&task).await.unwrap().status,"blocked");
    assert!(restarted.shutdown().await);std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn task_worktree_catalog_and_execution_share_project_configuration_but_use_task_files() {
    use crate::workspaces::SaveWorkspaceInput;
    fn git(path:&std::path::Path,args:&[&str])->String {
        let output=std::process::Command::new("git").current_dir(path).args(args).output().unwrap();
        assert!(output.status.success(),"git {args:?}: {}",String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
    fn scripts(path:&std::path::Path,label:&str) {
        std::fs::write(path.join("package.json"),format!(r#"{{"scripts":{{"{label}":"printf '{label}\\n' > npm-executed"}}}}"#)).unwrap();
        std::fs::write(path.join("Cargo.toml"),format!("[package]\nname='{label}'\nversion='0.1.0'\nedition='2021'\n[workspace]\n")).unwrap();
        std::fs::create_dir_all(path.join("src")).unwrap();
        std::fs::write(path.join("src/lib.rs"),format!("#[test]\nfn records_package_root() {{ std::fs::write(\"cargo-executed\", \"{label}\\n\").unwrap(); }}\n")).unwrap();
    }
    async fn run_configuration(manager:&AgentManager,task:&str,configuration:&CommandConfiguration) {
        let mut input=command(task,"");input.program=configuration.program.clone();input.args=configuration.args.clone();input.cwd=configuration.cwd.clone();input.env_refs=configuration.env_refs.clone();
        manager.start_development_process("alice",input.clone(),CancellationToken::new()).await.unwrap();
        let view=finished(manager,&input.id).await;
        assert_eq!(view.exit_code,Some(0),"{}: {:?}",configuration.id,view);
    }
    let(root,manager,project,original_task)=setup().await;let source=root.join("code");
    scripts(&source,"original");git(&source,&["init","-b","main"]);git(&source,&["add","."]);
    git(&source,&["-c","user.name=Fixture","-c","user.email=fixture@example.invalid","commit","-m","original scripts"]);
    let base=git(&source,&["rev-parse","HEAD"]);
    let task=manager.save_development_task("alice",SaveTaskInput{id:None,project_id:project.clone(),goal:"Run isolated scripts".into(),requirements:"Use task files".into(),status:"todo".into()},CancellationToken::new()).await.unwrap().id;
    let target=root.join("task-worktree");
    manager.save_development_workspaces("alice",SaveWorkspaceInput{request_id:uuid::Uuid::new_v4().to_string(),task_id:task.clone(),action:"create".into(),workspace_id:None,path:Some(target.to_string_lossy().into_owned()),branch:Some("task/scripts".into()),base_commit:Some(base),retain_branch:None},CancellationToken::new()).await.unwrap();
    scripts(&target,"isolated");
    let saved=CommandConfiguration{id:"saved".into(),name:"Saved project command".into(),program:"/bin/sh".into(),args:vec!["-c".into(),"cat source.txt > saved-executed".into()],cwd:String::new(),env_refs:BTreeMap::new()};
    manager.save_development_commands("alice",&original_task,vec![saved.clone()]).await.unwrap();
    let original=manager.read_development_commands("alice",&original_task).await.unwrap();
    let isolated=manager.read_development_commands("alice",&task).await.unwrap();
    for (catalog,present,absent) in [(&original,"package:original","package:isolated"),(&isolated,"package:isolated","package:original")] {
        assert!(catalog.discovered.iter().any(|c|c.id==present));assert!(!catalog.discovered.iter().any(|c|c.id==absent));
        assert!(catalog.discovered.iter().any(|c|c.id=="cargo:test"));
        assert_eq!(serde_json::to_value(&catalog.commands).unwrap(),serde_json::json!([saved]));
    }
    std::fs::write(source.join("source.txt"),"original saved configuration\n").unwrap();
    std::fs::write(target.join("source.txt"),"isolated saved configuration\n").unwrap();
    for (task,catalog,path,label) in [(&original_task,&original,&source,"original"),(&task,&isolated,&target,"isolated")] {
        for id in [format!("package:{label}"),"cargo:test".into()] {
            run_configuration(&manager,task,catalog.discovered.iter().find(|c|c.id==id).unwrap()).await;
        }
        run_configuration(&manager,task,&catalog.commands[0]).await;
        assert_eq!(std::fs::read_to_string(path.join("npm-executed")).unwrap(),format!("{label}\n"));
        assert_eq!(std::fs::read_to_string(path.join("cargo-executed")).unwrap(),format!("{label}\n"));
        assert_eq!(std::fs::read_to_string(path.join("saved-executed")).unwrap(),format!("{label} saved configuration\n"));
    }
    let mut updated=saved;updated.name="Updated from task worktree".into();
    manager.save_development_commands("alice",&task,vec![updated.clone()]).await.unwrap();
    let restarted=crate::test_support::manager(&root).await;
    for task in [&original_task,&task] {
        assert_eq!(serde_json::to_value(restarted.read_development_commands("alice",task).await.unwrap().commands).unwrap(),serde_json::json!([updated]));
    }
    assert!(manager.shutdown().await);assert!(restarted.shutdown().await);std::fs::remove_dir_all(root).unwrap();
}
