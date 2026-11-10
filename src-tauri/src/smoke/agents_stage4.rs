//! Provider-only desktop acceptance. Does not claim native Skill or MCP fixture coverage.
//! The CLI acceptance driver owns native Skill/MCP resource and tool evidence.
//! Optional DGOS_STAGE4_CLAUDE_AUTH_STYLE preserves an explicit bearer/both/x-api-key
//! Provider override; absence retains the Provider profile's existing authentication semantics.
use super::*;
use dgos_agents::Engine;
use dgos_providers::{ProviderManager,ProviderCredentialVault,ProviderCredentialScope,SecretValue,new_credential_ref};
use dgos_types::ProviderConfig;
use tokio::sync::Mutex;
struct Seed { engine:Engine,endpoint:String,model:String,key:SecretValue,auth_style:Option<String> }
struct Services(Mutex<Vec<Seed>>);
pub(super) async fn install(app:&AppHandle,fx:&Fx)->Arc<McpManager>{
 let mut seeds=Vec::new();
 for(engine,label)in[(Engine::Codex,"CODEX"),(Engine::Claude,"CLAUDE")]{
  let take=|suffix:&str|{let name=format!("DGOS_STAGE4_{label}_{suffix}");let value=std::env::var(&name).unwrap_or_default();if suffix=="KEY"{std::env::remove_var(name);}value};
  let auth_style=if engine==Engine::Claude { std::env::var("DGOS_STAGE4_CLAUDE_AUTH_STYLE").ok() } else { None };
  if let Some(style)=auth_style.as_deref(){assert!(matches!(style,"bearer"|"both"|"x-api-key"),"unsupported explicit stage4 Claude authentication style");}
  seeds.push(Seed{engine,endpoint:take("URL"),model:take("MODEL"),key:SecretValue::new(take("KEY")).expect("explicit stage4 key"),auth_style});
 }
 app.manage(Services(Mutex::new(seeds)));
 Arc::new(McpManager::new(fx.db.clone()))
}
async fn seed(app:&AppHandle,fx:&Fx,owner:&str)->Result<Vec<Value>,String>{
 let providers=app.state::<ProviderManager>();let vault=app.state::<Arc<dgos_providers::test_support::MemoryCredentialVault>>();let mut rows=Vec::new();
 for seed in app.state::<Services>().0.lock().await.drain(..){
  let id=uuid::Uuid::new_v4().to_string();let credential=new_credential_ref();
  vault.store(&ProviderCredentialScope{user_id:owner.into(),provider_id:id.clone(),endpoint:seed.endpoint.clone(),credential_ref:credential.clone()},seed.key).await.map_err(|_|"fixture_vault_failed")?;
  dgos_db::ProviderRepository::new(fx.db.clone()).create_with_credential_ref(id.clone(),ProviderConfig{name:format!("Stage4 {:?}",seed.engine),provider_type:if seed.engine==Engine::Codex{"openai_compatible"}else{"anthropic"}.into(),base_url:seed.endpoint,api_key:String::new(),enabled:true,config:seed.auth_style.map(|style|json!({"auth_style":style}))},owner.into(),Some(credential)).await.map_err(|_|"fixture_provider_failed")?;
  providers.add_manual_model(&id,owner,&seed.model,None).await.map_err(|_|"fixture_model_failed")?;
  let provider=providers.get_provider(&id,owner).await.map_err(|_|"fixture_read_failed")?.ok_or("provider_missing")?;
  rows.push(json!({"engine":seed.engine,"provider_id":id,"provider_revision":provider.config_revision,"model":seed.model}));
 }
 Ok(rows)
}
fn write_json(path:impl AsRef<Path>,v:&Value){fs::write(path,serde_json::to_vec_pretty(v).unwrap()).expect("stage4 evidence");}
async fn per_engine(app:&AppHandle,fx:&Fx,ck:&mut Checks,ev:&mut Vec<Value>,row:&Value)->Result<Value,String>{
 let engine=row["engine"].as_str().ok_or("engine_missing")?;let path=fx.root.join(format!("stage4-ui-{engine}"));make_repo(&path);
 if !ui_import_project(app,ck,ev,&format!("Stage4 {engine}"),&path).await{return Err("native_project_import_failed".into());}
 if !ui_new_task(app,ck,ev,&format!("Stage4 {engine}")).await{return Err("task_creation_failed".into());}
 let selection=value(&ipc(app,"main","read_development_workspace",json!({})).await)["selection"].clone();let project=selection["project_id"].as_str().ok_or("project_missing")?;let task=selection["task_id"].as_str().ok_or("task_missing")?;
 let configured=ui(app,row.clone(),r#"await chooseConversationModel(A.engine,'Stage4 '+(A.engine==='codex'?'Codex':'Claude'),A.model);return {ok:true};"#).await;record(ck,ev,"explicit Provider and model selected through production composer",ui_ok(&configured),configured.to_string());
 // Grant only after route selection so automatic preparation persists that exact revision.
 let before=ids(&sessions_of(app).await);
 let authorized=ui(app,json!({}),r#"await authorizeConversation();return {ok:!!await until(()=>!tid('dev-project-conversation')&&document.getElementById('agent-message'))};"#).await;record(ck,ev,"agent workspace permission confirmed and native session automatically prepared",ui_ok(&authorized),authorized.to_string());
 let saved=value(&ipc(app,"main","read_development_cli_provider",json!({"projectId":project})).await);record(ck,ev,"persisted exact Provider selection",saved["configuration"]["provider"]["provider_id"]==row["provider_id"]&&saved["configuration"]["provider"]["model_id"]==row["model"],saved.to_string());
 let stale=ipc(app,"main","save_development_cli_provider",json!({"input":{"project_id":project,"expected_revision":0,"configuration":saved["configuration"]}})).await;record(ck,ev,"stale CAS refuses",!succeeded(&stale),stale.describe());
 let sid=new_session_id(app,&before).await.ok_or("session_missing")?;
 let actual=value(&ipc(app,"main","get_development_session_configuration",json!({"sessionId":sid})).await);record(ck,ev,"session contains Provider-only snapshot",actual["format"]=="provider_bound"&&actual["snapshot"]["provider_id"]==row["provider_id"],actual.to_string());
 let first=stage2::exercise_turn(app,ck,ev,&sid,"Acknowledge this session without tools or file changes.").await;record(ck,ev,"ready then first native turn reaches terminal",first["status"]=="idle",first.to_string());
 let request=uuid::Uuid::new_v4().to_string();let sent=ipc(app,"main","send_agent_message",json!({"input":{"session_id":sid,"generation":first["generation"],"request_id":request,"text":"Use your native command tool to print STAGE4_NATIVE_APPROVAL. Request approval if required. Preserve all files."}})).await;record(ck,ev,"second turn admitted after ready",succeeded(&sent),sent.describe());
 let deadline=tokio::time::Instant::now()+Duration::from_secs(90);let mut approvals=Vec::new();
 loop{
  let view=value(&ipc(app,"main","get_agent_session",json!({"sessionId":sid})).await);
  for pending in view["pending"].as_array().into_iter().flatten(){let response=match pending["kind"].as_str().unwrap_or(""){"item/commandExecution/requestApproval"|"item/fileChange/requestApproval"=>json!({"decision":"decline"}),"claude/toolPermission"=>json!({"behavior":"deny","message":"Acceptance cancellation"}),_=>continue};let reply=ipc(app,"main","respond_agent_request",json!({"input":{"session_id":sid,"generation":view["generation"],"request_id":pending["id"],"response":response}})).await;record(ck,ev,"native approval response crosses production IPC",succeeded(&reply),reply.describe());approvals.push(pending.clone());}
  if matches!(view["status"].as_str(),Some("idle"|"error"|"unknown")){break;}if tokio::time::Instant::now()>=deadline{return Err("native_turn_timeout".into());}tokio::time::sleep(Duration::from_millis(100)).await;
 }
 let revoke=ipc(app,"main","revoke_development_cli_provider",json!({"projectId":project})).await;record(ck,ev,"active binding revocation persists",succeeded(&revoke),revoke.describe());
 let denied=ipc(app,"main","send_agent_message",json!({"input":{"session_id":sid,"generation":first["generation"],"request_id":uuid::Uuid::new_v4(),"text":"must not execute"}})).await;record(ck,ev,"revoked session rejects further sends",!succeeded(&denied),denied.describe());
 if let Some(old)=approvals.first(){let late=ipc(app,"main","respond_agent_request",json!({"input":{"session_id":sid,"generation":first["generation"],"request_id":old["id"],"response":{"decision":"accept"}}})).await;record(ck,ev,"revoked native approval cannot be reused",!succeeded(&late),late.describe());}
 let runs=value(&ipc(app,"main","list_development_runs",json!({"taskId":task})).await);let run=runs.as_array().into_iter().flatten().find(|r|r["id"]==request).cloned().unwrap_or(Value::Null);record(ck,ev,"persisted run retains original Provider snapshot",run["configuration"]["snapshot"]==actual["snapshot"],run.to_string());
 let result=json!({"engine":engine,"project":project,"task":task,"session":sid,"saved":saved,"actual":actual,"native_approvals":approvals,"run":run});write_json(fx.root.join(format!("stage4-ui-{engine}.json")),&result);Ok(result)
}
pub(super) async fn run(app:AppHandle,loaded:bool){
 let fx=app.state::<Fx>().inner().clone();let mut ck=Checks::default();let mut ev=Vec::new();record(&mut ck,&mut ev,"main WebView loaded",loaded,"/workbench");
 let execution=async{
  for(user,password)in[ALICE,BOB]{let r=ipc(&app,"main","create_account",json!({"input":{"username":user,"password":password}})).await;if !r.success(){return Err("account_fixture_failed".to_string());}}
  ipc(&app,"main","sign_out",json!({})).await;if !ui_sign_in(&app,&mut ck,&mut ev,ALICE).await{return Err("sign_in_failed".into());}let resolver=app.state::<Arc<SessionResolver>>();let(session,_)=resolver.agent_session().map_err(|_|"session_missing")?;let owner=session.user_id().to_owned();let seeds=seed(&app,&fx,&owner).await?;goto(&app,"main","/tasks/agent").await;let mut rows=Vec::new();for row in seeds{rows.push(per_engine(&app,&fx,&mut ck,&mut ev,&row).await?);}
  let row=rows.first().ok_or("engine_evidence_missing")?;let events=app.state::<LoadEvents>().0.subscribe();let window=WebviewWindowBuilder::new(&app,"stage4-untrusted",WebviewUrl::App("workbench".into())).visible(false).build().map_err(|_|"window_failed")?;wait_load(events,"stage4-untrusted","/workbench").await;
  for label in["stage4-untrusted","main"]{if label=="main"{ui_sign_out(&app,&mut ck,&mut ev,ALICE.0).await;}for(command,args)in[("read_development_cli_provider",json!({"projectId":row["project"]})),("get_development_session_configuration",json!({"sessionId":row["session"]})),("revoke_development_cli_provider",json!({"projectId":row["project"]}))]{let r=ipc(&app,label,command,args).await;record(&mut ck,&mut ev,&format!("{label} boundary refuses {command}"),!succeeded(&r),r.describe());}}
  window.close().map_err(|_|"window_close_failed")?;if !ui_sign_in(&app,&mut ck,&mut ev,BOB).await{return Err("bob_sign_in_failed".into());}let foreign=ipc(&app,"main","get_development_session_configuration",json!({"sessionId":row["session"]})).await;record(&mut ck,&mut ev,"foreign owner cannot read Provider session",!succeeded(&foreign),foreign.describe());write_json(fx.root.join("stage4-ui-state.json"),&json!({"engines":rows}));Ok::<_,String>(())
 }.await;
 if let Err(error)=execution{record(&mut ck,&mut ev,"desktop scenario completes",false,error);}record(&mut ck,&mut ev,"owned CLI child wait completes",app.state::<AgentManager>().shutdown().await,"production shutdown and notification drain");write_json(fx.root.join("stage4-ui-result.json"),&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev,"runtime":"Provider-bound AgentManager, native approvals"}));app.exit(if ck.failed.is_empty(){0}else{1});
}
