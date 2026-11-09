//! Real stage5 acceptance in disposable Git repositories. Never operates on DG-OS Git state.
use async_trait::async_trait;
use dgos_agents::{AgentManager,Engine,SessionOptions,capabilities::{CliProviders,CliProviderConfiguration,CliProviderRef,SaveCliProviderInput,SessionCreate},projects::{CreateProjectInput,SaveTaskInput},workspaces::SaveWorkspaceInput,git::{ExecuteGitInput,GitAction,ReconcileGitInput},handoffs::{PrepareHandoffInput,AcceptHandoffInput},runtime::StartProcessInput};
use dgos_db::{DevelopmentRepository,ProviderRepository,migrations::Migrator,sea_orm_migration::MigratorTrait};
use dgos_providers::{ProviderManager,ProviderCredentialVault,ProviderCredentialScope,SecretValue,new_credential_ref};
use dgos_types::{DgosResult,DgosError,ProviderConfig};
use serde_json::{json,Value};
use std::{collections::{HashMap,BTreeMap},path::{Path,PathBuf},sync::Arc,time::Duration};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
type R<T> = Result<T,String>;
const OWNER:&str="stage5-owner";
#[derive(Default)]struct Vault(Mutex<HashMap<String,SecretValue>>);
#[async_trait]impl ProviderCredentialVault for Vault{
 async fn store(&self,s:&ProviderCredentialScope,v:SecretValue)->DgosResult<()>{self.0.lock().await.insert(s.account(),v);Ok(())}
 async fn load(&self,s:&ProviderCredentialScope)->DgosResult<SecretValue>{SecretValue::new(self.0.lock().await.get(&s.account()).ok_or(DgosError::CredentialUnavailable)?.expose().into())}
 async fn remove(&self,s:&ProviderCredentialScope)->DgosResult<()>{self.0.lock().await.remove(&s.account());Ok(())}
}
fn err(e:impl std::fmt::Display)->String{e.to_string()}
fn need(v:bool,s:&str)->R<()>{if v {Ok(())}else{Err(s.into())}}
fn uid()->String{Uuid::new_v4().to_string()}
fn lease()->CancellationToken{CancellationToken::new()}
fn artifact(root:&Path,name:&str,value:Value)->R<()>{std::fs::write(root.join(name),serde_json::to_vec_pretty(&value).map_err(err)?).map_err(err)}
async fn git(path:&Path,args:&[&str])->R<String>{let o=tokio::process::Command::new("git").current_dir(path).args(["-c","core.hooksPath=/dev/null","-c","commit.gpgsign=false"]).args(args).output().await.map_err(err)?;need(o.status.success(),&format!("fixture_git_failed:{args:?}"))?;String::from_utf8(o.stdout).map_err(err).map(|s|s.trim().into())}
async fn action(a:&AgentManager,task:&str,action:GitAction)->R<Value>{let review=a.read_development_git(OWNER,task).await?;a.execute_development_git(OWNER,ExecuteGitInput{id:uid(),task_id:task.into(),review_token:review.review_token,action},lease()).await}
async fn task(a:&AgentManager,p:&str,goal:&str)->R<String>{Ok(a.save_development_task(OWNER,SaveTaskInput{id:None,project_id:p.into(),goal:goal.into(),requirements:"Modify only requested source; run node --test; preserve all user changes".into(),status:"todo".into()},lease()).await?.id)}
fn options()->SessionOptions{SessionOptions{workspace:String::new(),resume_id:None,executable:None,model:None,trust_workspace:true}}
async fn wait_run(a:&AgentManager,root:&Path,task:&str,run:&str,label:&str)->R<()> {
 let until=tokio::time::Instant::now()+Duration::from_secs(360);
 loop {
  let all=a.list_development_runs(OWNER,task).await?;
  if let Some(r)=all.iter().find(|r|r.id==run) {
   if !matches!(r.status.as_str(),"running"|"waiting") {artifact(root,&format!("{label}-run.json"),serde_json::to_value(r).map_err(err)?)?;return need(r.status=="completed",&format!("native_failed:{label}"));}
   if let Some(session)=r.session_id.as_deref() {
    let view=a.snapshot(OWNER,session).await?;
    for pending in view.view.pending {
     let answer=match pending.kind.as_str(){"item/commandExecution/requestApproval"|"item/fileChange/requestApproval"=>json!({"decision":"accept"}),"claude/toolPermission" if pending.title!="AskUserQuestion"=>json!({"behavior":"allow"}),_=>return Err("unexpected_native_prompt".into())};
     a.respond(OWNER,session,&view.generation,pending.id,answer).await?;
    }
   }
  }
  need(tokio::time::Instant::now()<until,"native_timeout")?;tokio::time::sleep(Duration::from_millis(150)).await;
 }
}
async fn verify(a:&AgentManager,task:&str)->R<Value>{let id=uid();a.start_development_process(OWNER,StartProcessInput{id:id.clone(),task_id:task.into(),kind:"command".into(),program:"node".into(),args:vec!["--test".into()],cwd:String::new(),env_refs:BTreeMap::new(),trust_workspace:true,verification:true,readiness:None,native_run_id:None},lease()).await?;loop{let s=a.read_development_process(OWNER,&id).await?;if s.finished_at.is_some(){need(s.exit_code==Some(0)&&s.verification_status=="observed_stable","host_verification_failed")?;return serde_json::to_value(s).map_err(err);}tokio::time::sleep(Duration::from_millis(100)).await;}}
#[tokio::main]async fn main(){if let Err(e)=run().await{eprintln!("stage5 failed: {e}");std::process::exit(1)}}
async fn run()->R<()> {
 let args:Vec<_>=std::env::args().collect();let root=PathBuf::from(args.get(1).ok_or("root_required")?).canonicalize().map_err(err)?;let restart=args.get(2).is_some_and(|s|s=="--restart");
 need(root.join("stage5-isolated-root").is_file(),"isolated_marker_required")?;
 let url=format!("sqlite://{}?mode=rwc",root.join("acceptance.sqlite").display());let db=dgos_db::establish_connection(&url).await.map_err(err)?;Migrator::up(&db,None).await.map_err(err)?;
 let repo=DevelopmentRepository::new(db.clone());let vault=Arc::new(Vault::default());let providers=ProviderManager::with_vault(&url,vault.clone()).await.map_err(err)?;let cap=CliProviders::new(repo.clone(),providers.clone())?;let a=AgentManager::new(root.join("legacy.json"),repo.clone(),cap.clone());
 if restart {a.load().await?;let state:Value=serde_json::from_slice(&std::fs::read(root.join("state.json")).map_err(err)?).map_err(err)?;let id=state["push_operation"].as_str().ok_or("state_missing")?;let before=git(&root.join("remote.git"),&["rev-parse","refs/heads/codex-work"]).await?;let receipt=a.reconcile_development_git(OWNER,ReconcileGitInput{id:id.into(),trust_directory:true,trust_network:true},lease()).await?;need(receipt["status"]=="succeeded","restart_reconcile_failed")?;need(before==git(&root.join("remote.git"),&["rev-parse","refs/heads/codex-work"]).await?,"restart_changed_remote")?;artifact(&root,"restart.json",json!({"receipt":receipt,"remote_oid":before,"no_replay":true,"workspace":a.development_workspace(OWNER).await?}))?;need(a.shutdown().await,"shutdown_failed")?;println!("stage5 restart evidence={}",root.display());return Ok(());}
 a.load().await?;
 let result: R<()> = async {
 let original=root.join("original");std::fs::create_dir(&original).map_err(err)?;git(&original,&["init","-b","main"]).await?;git(&original,&["config","user.name","Stage5 Acceptance"]).await?;git(&original,&["config","user.email","stage5@example.invalid"]).await?;
 std::fs::write(original.join("math.cjs"),"exports.sum=(a,b)=>a-b;\nexports.product=(a,b)=>a+b;\n").map_err(err)?;
 std::fs::write(original.join("math.test.cjs"),"const t=require('node:test'),a=require('node:assert/strict'),m=require('./math.cjs');t('sum',()=>a.equal(m.sum(4,3),7));t('product',()=>a.equal(m.product(4,3),12));\n").map_err(err)?;
 std::fs::write(original.join("README.md"),"base\n").map_err(err)?;git(&original,&["add","--","math.cjs","math.test.cjs","README.md"]).await?;git(&original,&["commit","-m","fixture initial"]).await?;let base=git(&original,&["rev-parse","HEAD"]).await?;
 git(&root,&["init","--bare","remote.git"]).await?;git(&original,&["remote","add","origin",root.join("remote.git").to_str().unwrap()]).await?;
 std::fs::write(original.join("README.md"),"user staged\n").map_err(err)?;git(&original,&["add","--","README.md"]).await?;std::fs::write(original.join("README.md"),"user staged\nuser unstaged\n").map_err(err)?;std::fs::write(original.join("notes.txt"),"user untracked\n").map_err(err)?;let original_index=git(&original,&["write-tree"]).await?;
 let p=a.create_project(OWNER,CreateProjectInput{kind:"import".into(),name:"Stage5 Real".into(),path:original.to_string_lossy().into(),source:None},lease()).await?;
 let codex=task(&a,&p.id,"Codex isolation and Claude continuation").await?;let claude=task(&a,&p.id,"Claude isolation and Codex continuation").await?;
 let mut revisions=Vec::new();let mut selections=Vec::new();for (n,engine) in [Engine::Codex,Engine::Claude].into_iter().enumerate(){let pre=if engine==Engine::Codex{"DGOS_STAGE5_CODEX"}else{"DGOS_STAGE5_CLAUDE"};let endpoint=std::env::var(format!("{pre}_URL")).map_err(err)?;let key=std::env::var(format!("{pre}_KEY")).map_err(err)?;let model=std::env::var(format!("{pre}_MODEL")).map_err(err)?;let id=uid();let credential=new_credential_ref();vault.store(&ProviderCredentialScope{user_id:OWNER.into(),provider_id:id.clone(),endpoint:endpoint.clone(),credential_ref:credential.clone()},SecretValue::new(key).map_err(err)?).await.map_err(err)?;ProviderRepository::new(db.clone()).create_with_credential_ref(id.clone(),ProviderConfig{name:format!("real-{n}"),provider_type:if engine==Engine::Codex{"openai_compatible"}else{"anthropic"}.into(),base_url:endpoint,api_key:String::new(),enabled:true,config:None},OWNER.into(),Some(credential)).await.map_err(err)?;providers.add_manual_model(&id,OWNER,&model,None).await.map_err(err)?;let rev=providers.get_provider(&id,OWNER).await.map_err(err)?.ok_or("provider_missing")?.config_revision;let provider=CliProviderRef{provider_id:id,model_id:model,provider_revision:rev};let c=cap.save(OWNER,SaveCliProviderInput{project_id:p.id.clone(),expected_revision:n as i64,configuration:CliProviderConfiguration{engine,provider:provider.clone()}}).await?;selections.push(provider);revisions.push(c.revision);}
 for(name,t)in[("codex",&codex),("claude",&claude)]{a.save_development_workspaces(OWNER,SaveWorkspaceInput{request_id:uid(),task_id:t.clone(),action:"create".into(),workspace_id:None,path:Some(root.join(format!("{name}-work")).to_string_lossy().into()),branch:Some(format!("{name}-work")),base_commit:Some(base.clone()),retain_branch:None},lease()).await?;need(!root.join(format!("{name}-work/notes.txt")).exists(),"worktree_copied_untracked")?;}
 let sid1=uid();let sid2=uid();a.open_session(OWNER,Engine::Codex,sid1.clone(),SessionCreate{options:options(),project_revision:revisions[0],provider:selections[0].clone()},Some(codex.clone()),lease()).await?;a.open_session(OWNER,Engine::Claude,sid2.clone(),SessionCreate{options:options(),project_revision:revisions[1],provider:selections[1].clone()},Some(claude.clone()),lease()).await?;
 let same=a.open_session(OWNER,Engine::Claude,uid(),SessionCreate{options:options(),project_revision:revisions[1],provider:selections[1].clone()},Some(codex.clone()),lease()).await;need(matches!(&same,Err(e)if e=="workspace_busy"),"same_directory_not_blocked")?;
 let r1=uid();let r2=uid();let prompt="Edit only math.cjs: fix both sum and product to compute arithmetic correctly. Run node --test and inspect actual exit status. Preserve math.test.cjs and README.md byte-for-byte. Do not run Git mutations. No need for other tools or external files.";
 a.send(OWNER, &sid1, &a.snapshot(OWNER,&sid1).await?.generation, r1.clone(), prompt.into(), None, vec![], vec![]).await?;a.send(OWNER, &sid2, &a.snapshot(OWNER,&sid2).await?.generation, r2.clone(), prompt.into(), None, vec![], vec![]).await?;let (x,y)=tokio::join!(wait_run(&a,&root,&codex,&r1,"codex-initial"),wait_run(&a,&root,&claude,&r2,"claude-initial"));x?;y?;need(a.close_session(OWNER,&sid1).await?&&a.close_session(OWNER,&sid2).await?,"close_initial_failed")?;artifact(&root,"initial-verification.json",json!([verify(&a,&codex).await?,verify(&a,&claude).await?]))?;
 for(t,source,target,revision,label)in[(&codex,&sid1,Engine::Claude,revisions[1],"codex-to-claude"),(&claude,&sid2,Engine::Codex,revisions[0],"claude-to-codex")]{let h=a.prepare_development_handoff(OWNER,PrepareHandoffInput{task_id:t.clone(),source_session_id:source.clone(),target_engine:target,remaining:"Continue by adding exports.difference=(a,b)=>a-b to math.cjs, and add a consumer test to math.test.cjs checking difference(9,4)==5. Run node --test. Keep previous sum and product fixes and do not mutate Git.".into()},lease()).await?;let summary=h.summary.clone();let accepted=a.accept_development_handoff(OWNER,AcceptHandoffInput{handoff_id:h.id.clone(),summary:summary.clone(),confirmed:true,project_revision:revision,provider:selections[usize::from(target==Engine::Claude)].clone(),options:options()},lease()).await?;need(accepted.session.id!=*source,"handoff_reused_session")?;let run=uid();a.send(OWNER, &accepted.session.id, &accepted.session.generation, run.clone(), summary, None, vec![], vec![]).await?;wait_run(&a,&root,t,&run,label).await?;need(a.close_session(OWNER,&accepted.session.id).await?,"close_handoff_failed")?;artifact(&root,&format!("{label}.json"),json!({"handoff":accepted.handoff,"new_native_id":accepted.session.view.native_id,"verification":verify(&a,t).await?}))?;need(std::fs::read_to_string(root.join(if target==Engine::Claude{"codex-work/math.cjs"}else{"claude-work/math.cjs"})).map_err(err)?.contains("difference"),"receiver_did_not_modify")?;}
 need(git(&original,&["write-tree"]).await?==original_index,"original_index_changed")?;need(std::fs::read_to_string(original.join("README.md")).map_err(err)?=="user staged\nuser unstaged\n","original_user_changes_lost")?;need(std::fs::read_to_string(original.join("notes.txt")).map_err(err)?=="user untracked\n","untracked_lost")?;
 action(&a,&codex,GitAction::Stage{paths:vec!["math.cjs".into()]}).await?;action(&a,&codex,GitAction::Unstage{paths:vec!["math.cjs".into()]}).await?;action(&a,&codex,GitAction::Stage{paths:vec!["math.cjs".into(),"math.test.cjs".into()]}).await?;let commit=action(&a,&codex,GitAction::Commit{message:"Deliver corrected math and continuation".into(),confirm_full_index:true,confirm_preexisting_staged:false}).await?;let push=action(&a,&codex,GitAction::Push{remote:"origin".into(),trust_directory:true,trust_network:true}).await?;let remote=git(&root.join("remote.git"),&["rev-parse","refs/heads/codex-work"]).await?;need(remote==git(&root.join("codex-work"),&["rev-parse","HEAD"]).await?,"remote_ref_mismatch")?;
 let stale=a.read_development_git(OWNER,&codex).await?;std::fs::write(root.join("codex-work/external.txt"),"external\n").map_err(err)?;let refused=a.execute_development_git(OWNER,ExecuteGitInput{id:uid(),task_id:codex.clone(),review_token:stale.review_token,action:GitAction::Stage{paths:vec!["math.cjs".into()]}},lease()).await;need(refused.is_err(),"external_edit_not_rechecked")?;
 let revoke=a.prepare_development_handoff(OWNER,PrepareHandoffInput{task_id:codex.clone(),source_session_id:sid1.clone(),target_engine:Engine::Claude,remaining:"must be rejected after revocation".into()},lease()).await?;cap.revoke(OWNER,&p.id).await?;let denied=a.accept_development_handoff(OWNER,AcceptHandoffInput{handoff_id:revoke.id,summary:revoke.summary,confirmed:true,project_revision:revisions[1],provider:selections[1].clone(),options:options()},lease()).await;need(matches!(&denied,Err(e)if e=="capability_revoked"),"revoked_handoff_accepted")?;
 artifact(&root,"state.json",json!({"project":p.id,"codex_task":codex,"claude_task":claude,"push_operation":push["id"],"commit":commit,"push":push,"remote_oid":remote,"original_index":original_index}))?;
 // Emulate durable intent surviving a crash after remote success; restart may only inspect.
 db.execute(dgos_db::sea_orm::Statement::from_sql_and_values(dgos_db::sea_orm::DbBackend::Sqlite,"UPDATE development_project_operations SET status='pending' WHERE id=?",[push["id"].as_str().ok_or("push_id_missing")?.into()])).await.map_err(err)?;
 println!("stage5 real engine and Git evidence={}",root.display());Ok(())
 }.await;
 let clean=a.shutdown().await;need(clean,"shutdown_failed")?;result
}
use dgos_db::sea_orm::ConnectionTrait;
