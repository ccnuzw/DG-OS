//! Installed CLI smoke through production AgentManager; local HTTP only, synthetic keys.
//! Entry: bun scripts/stage4-provider-errors.ts. No paid provider calls.
use async_trait::async_trait;
use axum::{extract::State, http::{HeaderMap, StatusCode, Uri}, response::IntoResponse, Json, Router};
use dgos_agents::{capabilities::{CliProviders,CliProviderRef,CliProviderConfiguration,SaveCliProviderInput,SessionCreate,SessionConfigurationView},projects::{CreateProjectInput,SaveTaskInput},AgentManager,Engine,SessionOptions};
use dgos_db::{DevelopmentRepository,ProviderRepository,migrations::{Migrator,MigratorTrait}};
use dgos_providers::{ProviderManager,ProviderCredentialVault,ProviderCredentialScope,SecretValue,new_credential_ref};
use dgos_types::{DgosResult,DgosError,ProviderConfig};
use serde_json::{json,Value};
use std::{collections::HashMap,path::{Path,PathBuf},sync::Arc,time::Duration};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
type R<T> = Result<T,String>;
const OWNER:&str="d136-local-smoke";
#[derive(Default)]struct Vault(Mutex<HashMap<String,SecretValue>>);
#[async_trait]impl ProviderCredentialVault for Vault {
 async fn store(&self,s:&ProviderCredentialScope,v:SecretValue)->DgosResult<()>{self.0.lock().await.insert(s.account(),v);Ok(())}
 async fn load(&self,s:&ProviderCredentialScope)->DgosResult<SecretValue>{SecretValue::new(self.0.lock().await.get(&s.account()).ok_or(DgosError::CredentialUnavailable)?.expose().into())}
 async fn remove(&self,s:&ProviderCredentialScope)->DgosResult<()>{self.0.lock().await.remove(&s.account());Ok(())}
}
#[derive(Clone)]struct Fixture { key:Arc<String>, observations:Arc<Mutex<Vec<Value>>> }
async fn http(State(f):State<Fixture>,uri:Uri,headers:HeaderMap)->impl IntoResponse {
 let authenticated=headers.get("authorization").and_then(|h|h.to_str().ok()).is_some_and(|h|h==format!("Bearer {}",f.key))||headers.get("x-api-key").and_then(|h|h.to_str().ok()).is_some_and(|h|h==f.key.as_str());
 let status=if uri.path().contains("/400/"){400}else{401};
 f.observations.lock().await.push(json!({"path":uri.path(),"status":status,"selected_key":authenticated}));
 (StatusCode::from_u16(status).unwrap(),[("x-should-retry","false")],Json(json!({"type":"error","error":{"type":if status==401{"authentication_error"}else{"invalid_request_error"},"message":"local fixture rejection","code":"invalid_api_key"}})))
}
fn opaque(_:impl std::fmt::Display)->String{"fixture_operation_failed".into()}
fn need(v:bool,r:&str)->R<()>{if v{Ok(())}else{Err(r.into())}}
fn failure_class(error:&str)->&'static str { match error { "provider_binding_unconfirmed"=>"provider_binding_unconfirmed", "provider_model_mismatch"=>"provider_model_mismatch", "provider_auth_conflict"=>"provider_auth_conflict", "cli_handshake_failed"=>"cli_handshake_failed", "cli_handshake_timeout"=>"cli_handshake_timeout", "invalid_output"=>"invalid_output", "invalid_cli_binding"=>"invalid_cli_binding", "engine_rejected"=>"engine_rejected", "resume_identity_changed"=>"resume_identity_changed", "resume_child_wait_required"=>"resume_child_wait_required", "cleanup_pending"=>"cleanup_pending", "terminal_deadline"=>"terminal_deadline", "selected_route_not_observed"=>"selected_route_not_observed", "responses_route_or_no_fallback_failed"=>"responses_route_or_no_fallback_failed", _=>"other" } }
fn label(e:Engine)->&'static str{match e{Engine::Codex=>"codex",Engine::Claude=>"claude"}}
fn config(e:Engine,url:&str,enabled:bool,key:String)->ProviderConfig{ProviderConfig{name:"Local fixture".into(),provider_type:"openai_compatible".into(),base_url:url.into(),api_key:key,enabled,config:Some(json!({"allow_insecure_http":true,"auth_style":"bearer","cli_routes":if e==Engine::Codex{json!({"responses":{"enabled":true,"base_url":format!("{url}/cli")}})}else{json!({"messages":{"enabled":true,"base_url":format!("{url}/cli"),"auth_style":"x-api-key"}})}}))}}
fn artifact(root:&Path,name:&str,v:Value)->R<()>{std::fs::write(root.join(name),serde_json::to_vec_pretty(&v).map_err(opaque)?).map_err(opaque)}
fn options(root:&Path,e:Engine,workspace:&str)->SessionOptions{SessionOptions{workspace:workspace.into(),resume_id:None,executable:Some(root.join(format!("{}-cli",label(e))).to_string_lossy().into()),model:None,trust_workspace:true}}
#[tokio::main]async fn main(){let result=run().await;let failed=result.is_err();println!("{}",json!({"result":if failed{"failed"}else{"passed"},"reason":result.err()}));if failed{std::process::exit(1)}}
async fn run()->R<()> {
 let args:Vec<_>=std::env::args().collect();need(args.len()==3&&args[1]=="--root","isolated_root_required")?;
 let root=PathBuf::from(&args[2]).canonicalize().map_err(opaque)?;
 need(root.join("stage4-provider-errors-isolated").is_file(),"launcher_required")?;
 for(name,sub)in[("HOME","home"),("CODEX_HOME","codex-home"),("CLAUDE_CONFIG_DIR","claude-home")]{need(PathBuf::from(std::env::var(name).map_err(opaque)?).canonicalize().map_err(opaque)?==root.join(sub).canonicalize().map_err(opaque)?,"native_config_not_isolated")?;}
 let engines = match std::env::var("DGOS_STAGE4_ENGINE").as_deref() { Ok("claude") => vec![Engine::Claude], Ok("codex") => vec![Engine::Codex], Ok("both") | Err(_) => vec![Engine::Codex, Engine::Claude], _ => return Err("invalid_engine".into()) };
 let url=format!("sqlite://{}?mode=rwc",root.join("acceptance.sqlite").display());let db=dgos_db::establish_connection(&url).await.map_err(opaque)?;Migrator::up(&db,None).await.map_err(opaque)?;
 let repo=DevelopmentRepository::new(db.clone());let vault=Arc::new(Vault::default());let providers=ProviderManager::with_vault(&url,vault.clone()).await.map_err(opaque)?;let service=CliProviders::new(repo.clone(),providers.clone())?;let agents=AgentManager::new(root.join("index.json"),repo.clone(),service.clone());agents.load().await?;
 let fixture=Fixture{key:Arc::new(Uuid::new_v4().to_string()),observations:Arc::default()};let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(opaque)?;let address=format!("http://{}",listener.local_addr().map_err(opaque)?);let stop=CancellationToken::new();let stopped=stop.clone();let server=tokio::spawn(axum::serve(listener,Router::new().fallback(http).with_state(fixture.clone())).with_graceful_shutdown(stopped.cancelled_owned()).into_future());
 let execution=async {
 let mut versions=Vec::new();
 for engine in engines {
  let probe=dgos_agents::cli::probe(if engine==Engine::Codex{dgos_agents::cli::CliTool::Codex}else{dgos_agents::cli::CliTool::Claude},options(&root,engine,"").executable.as_deref()).await;
  need(probe.status=="available","cli_probe_failed")?;versions.push(json!({"engine":engine,"version":probe.version}));
  for status in[401,400]{
   let workspace=root.join(format!("{}-{status}",label(engine)));std::fs::create_dir(&workspace).map_err(opaque)?;
   let project=agents.create_project(OWNER,CreateProjectInput{kind:"import".into(),name:format!("{}-{status}",label(engine)),path:workspace.to_string_lossy().into(),source:None},CancellationToken::new()).await?;
   let task=agents.save_development_task(OWNER,SaveTaskInput{id:None,project_id:project.id.clone(),goal:"Observe selected local route failure".into(),requirements:"No native authentication fallback".into(),status:"todo".into()},CancellationToken::new()).await?;
   let endpoint=format!("{address}/{}/{status}{}",label(engine),if engine==Engine::Codex{"/v1"}else{""});let id=Uuid::new_v4().to_string();let credential=new_credential_ref();vault.store(&ProviderCredentialScope{user_id:OWNER.into(),provider_id:id.clone(),endpoint:endpoint.clone(),credential_ref:credential.clone()},SecretValue::new(fixture.key.as_str().into()).map_err(opaque)?).await.map_err(opaque)?;
   ProviderRepository::new(db.clone()).create_with_credential_ref(id.clone(),config(engine,&endpoint,true,String::new()),OWNER.into(),Some(credential)).await.map_err(opaque)?;let model=if engine==Engine::Codex{"fixture-model"}else{"claude-sonnet-4-6"};providers.add_manual_model(&id,OWNER,model,None).await.map_err(opaque)?;let revision=providers.get_provider(&id,OWNER).await.map_err(opaque)?.ok_or("provider_missing")?.config_revision;let provider=CliProviderRef{provider_id:id.clone(),model_id:model.into(),provider_revision:revision};let head=service.save(OWNER,SaveCliProviderInput{project_id:project.id,expected_revision:0,configuration:CliProviderConfiguration{engine,provider:provider.clone()}}).await?;
   let ordinary_start=fixture.observations.lock().await.len();
   let ordinary_result=providers.call_provider(&id,model,dgos_types::ChatRequest{messages:vec![dgos_types::ChatMessage{role:"user".into(),content:"Local route check".into(),images:vec![]}],max_tokens:Some(1),..Default::default()},OWNER).await;
   let ordinary_requests=fixture.observations.lock().await[ordinary_start..].to_vec();
   need(ordinary_result.is_err()&&ordinary_requests.len()==1&&ordinary_requests[0]["path"]==format!("/{}/{status}{}/chat/completions",label(engine),if engine==Engine::Codex{"/v1"}else{""})&&ordinary_requests[0]["selected_key"]==true,"ordinary_chat_route_changed")?;
   let native_start=fixture.observations.lock().await.len();
   let session=Uuid::new_v4().to_string();let opened=agents.open_session(OWNER,engine,session.clone(),SessionCreate{options:options(&root,engine,&workspace.to_string_lossy()),project_revision:head.revision,provider:provider.clone()},Some(task.id.clone()),CancellationToken::new()).await?;
   need(opened.view.status=="idle","ready_required_before_send")?;let native=opened.view.native_id.clone();let run=Uuid::new_v4().to_string();agents.send(OWNER, &session, &opened.generation, run.clone(), "Reply with one word using only the selected provider.".into(), None, vec![], vec![], vec![]).await?;
   let terminal=tokio::time::timeout(Duration::from_secs(75),async{loop{if let Some(r)=agents.list_development_runs(OWNER,&task.id).await?.into_iter().find(|r|r.id==run&&!matches!(r.status.as_str(),"running"|"waiting")){return Ok::<_,String>(r)}tokio::time::sleep(Duration::from_millis(100)).await;}}).await.map_err(|_|"terminal_deadline")??;
   let requests:Vec<_>=fixture.observations.lock().await[native_start..].iter().filter(|r|r["path"].as_str().is_some_and(|p|p.starts_with(&format!("/{}/{status}/",label(engine))))).cloned().collect();
   need(!requests.is_empty()&&requests.iter().all(|r|r["selected_key"]==true),"selected_route_not_observed")?;need(terminal.status=="failed"&&terminal.finished_at.is_some(),"native_failure_required")?;need(terminal.configuration.is_some(),"run_provider_snapshot_required")?;
   need(requests.iter().all(|r|r["path"].as_str().is_some_and(|p|p.ends_with(if engine==Engine::Codex{"/cli/responses"}else{"/cli/v1/messages"}))),"responses_route_or_no_fallback_failed")?;
   need(agents.close_session(OWNER,&session).await?,"child_wait_required")?;
   if status==400{let resumed=agents.resume_session(OWNER,&session,true,CancellationToken::new()).await?;need(resumed.view.native_id==native,"resume_identity_changed")?;need(agents.close_session(OWNER,&session).await?,"resume_child_wait_required")?;}
   else{providers.update_provider(&id,config(engine,&endpoint,false,String::new()),OWNER).await.map_err(opaque)?;need(agents.resume_session(OWNER,&session,true,CancellationToken::new()).await.is_err(),"disabled_resume_admitted")?;providers.update_provider(&id,config(engine,&endpoint,true,Uuid::new_v4().to_string()),OWNER).await.map_err(opaque)?;need(agents.resume_session(OWNER,&session,true,CancellationToken::new()).await.is_err(),"reenable_resurrected_binding")?;}
   let binding=repo.read_cli_session_configuration(OWNER,&session).await?.binding.ok_or("binding_missing")?;need(binding.cleanup_state=="stopped","cleanup_receipt_missing")?;
   artifact(&root,&format!("{}-http-{status}.json",label(engine)),json!({"passed":true,"ordinary_requests":ordinary_requests,"requests":requests,"terminal_status":terminal.status,"finished_at":terminal.finished_at,"child_wait":true,"cleanup_proof":binding.cleanup_proof,"snapshot":opened.configuration,"paid_external_models":"not covered"}))?;
  }
 }
 artifact(&root,"versions.json",json!(versions))?;
 // Legacy history is never resumed, even when a plausible native ID survives.
 let legacy_root=root.join("legacy");std::fs::create_dir(&legacy_root).map_err(opaque)?;
 let legacy_project=agents.create_project(OWNER,CreateProjectInput{kind:"import".into(),name:"Legacy history".into(),path:legacy_root.to_string_lossy().into(),source:None},CancellationToken::new()).await?;
 let legacy_task=agents.save_development_task(OWNER,SaveTaskInput{id:None,project_id:legacy_project.id.clone(),goal:"Preserve legacy history".into(),requirements:"Never resume native execution".into(),status:"todo".into()},CancellationToken::new()).await?;
 let legacy_url=format!("sqlite://{}?mode=rwc",legacy_root.join("legacy.sqlite").display());let legacy_db=dgos_db::establish_connection(&legacy_url).await.map_err(opaque)?;Migrator::up(&legacy_db,None).await.map_err(opaque)?;
 let legacy_repo=DevelopmentRepository::new(legacy_db);legacy_repo.insert_project(&legacy_project).await?;legacy_repo.save_task(&legacy_task).await?;
 let legacy_id=Uuid::new_v4().to_string();legacy_repo.save_session(&dgos_db::development_repository::StoredAgentSession{id:legacy_id.clone(),owner:OWNER.into(),engine:"codex".into(),native_id:Some("never-resume".into()),workspace:legacy_root.to_string_lossy().into(),executable:None,model:None,project_id:Some(legacy_project.id),task_id:Some(legacy_task.id)}).await?;
 let legacy_providers=ProviderManager::with_vault(&legacy_url,Arc::new(Vault::default())).await.map_err(opaque)?;let legacy_service=CliProviders::new(legacy_repo.clone(),legacy_providers)?;let legacy=AgentManager::new(legacy_root.join("index.json"),legacy_repo,legacy_service.clone());legacy.load().await?;
 let legacy_rejected=matches!(legacy.resume_session(OWNER,&legacy_id,true,CancellationToken::new()).await,Err(e)if e=="legacy_session_read_only");
 let legacy_history=matches!(legacy_service.read_session(OWNER,&legacy_id).await?,SessionConfigurationView::LegacyHistory{..});
 let legacy_clean=legacy.shutdown().await;
 need(legacy_rejected,"legacy_resume_admitted")?;need(legacy_history,"legacy_projection_missing")?;need(legacy_clean,"legacy_shutdown_failed")?;
 Ok::<_,String>(())
 }.await;
 let clean=agents.shutdown().await;stop.cancel();let http_stopped=matches!(server.await,Ok(Ok(())));artifact(&root,"result.json",json!({"passed":execution.is_ok()&&clean&&http_stopped,"failure_class":execution.as_ref().err().map(|error|failure_class(error)),"agents_child_wait":clean,"http_stopped":http_stopped,"paid_external_models":"not covered"}))?;execution?;need(clean&&http_stopped,"shutdown_failed")
}
use std::future::IntoFuture;
