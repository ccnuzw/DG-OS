use super::*;

async fn workspace(app:&AppHandle)->Value { value(&ipc(app,"main","read_development_workspace",json!({})).await) }
async fn selected(app:&AppHandle)->Value { workspace(app).await["selection"].clone() }
pub(super) async fn close_ui(app:&AppHandle,ck:&mut Checks,ev:&mut Vec<Value>,sid:&str){
 ui_close_run(app,ck,ev).await;
 let mut closed=false; for _ in 0..100 { let v=ipc(app,"main","get_agent_session",json!({"sessionId":sid})).await; if value(&v)["status"]=="closed" {closed=true;break;} tokio::time::sleep(Duration::from_millis(100)).await; }
 record(ck,ev,"UI close confirmed by host",closed,sid);
}
pub(super) async fn pick_engine(app:&AppHandle,name:&str)->Value { ui(app,json!({"name":name}),r#"const c=$$('[role=combobox]').find(e=>labelOf(e)==='智能体');if(!c)throw Error('engine control');c.click();const o=await until(()=>$$('[role=option]').find(e=>txt(e)===A.name));if(!o)throw Error('engine option');o.click();return {ok:true};"#).await }

pub(super) async fn scenario(app:AppHandle,loaded:bool){
 let fx=app.state::<Fx>().inner().clone();let mut ck=Checks::default();let mut ev=vec![];
 record(&mut ck,&mut ev,"production WebView loaded",loaded,"/workbench");
 let phase=std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").unwrap_or_default();
 let restart=matches!(phase.as_str(),"restart"|"stage3-restart");
 if !restart {
  for (user,pw) in [ALICE,BOB] {let r=ipc(&app,"main","create_account",json!({"input":{"username":user,"password":pw}})).await;record(&mut ck,&mut ev,&format!("create {user} account"),r.success(),r.describe());}
  let _=ipc(&app,"main","sign_out",json!({})).await;
 }
 if !ui_sign_in(&app,&mut ck,&mut ev,ALICE).await {finish(&app,&fx,ck,ev,"login").await;return;}
 record(&mut ck,&mut ev,"navigate production project workbench",goto(&app,"main","/tasks/agent").await,"navigation");
 if matches!(phase.as_str(),"stage3-first"|"stage3-restart") { super::stage3::run(&app, &fx, ck, ev, restart).await; return; }
 if std::env::var_os("DGOS_STAGE2_ACCEPTANCE").is_some() { super::stage2::run(&app, &fx, ck, ev, restart).await; return; }
 if restart {
  let first:Value=serde_json::from_slice(&fs::read(fx.root.join("evidence-first.json")).expect("first evidence")).unwrap();
  let ws=workspace(&app).await;record(&mut ck,&mut ev,"last selection restored",ws["selection"]==first["selection"],ws["selection"].to_string());
  for s in first["sessions"].as_array().unwrap(){
   let sid=s["id"].as_str().unwrap();let goal=s["goal"].as_str().unwrap();let engine=s["engine"].as_str().unwrap();
   let row=ws["sessions"].as_array().unwrap().iter().find(|r|r["id"]==sid);record(&mut ck,&mut ev,&format!("{engine} association survives process restart"),row.is_some_and(|r|r["task_id"]==s["task"]&&r["project_id"]==first["project"]),format!("{row:?}"));
   ui_select_task(&app,&mut ck,&mut ev,goal).await;
   let reply=ui(&app,json!({"sid":sid}),r#"const trust=await need('dev-trust');if(!trust.checked)trust.click();const row=await need('dev-session-item-'+A.sid);const b=await until(()=>{const b=row.querySelector('button');return b&&!b.disabled&&b});if(!b)throw Error('resume disabled');b.click();const composer=await until(()=>document.getElementById('agent-message'));return {ok:!!composer};"#).await;
   record(&mut ck,&mut ev,&format!("{engine} explicit UI restore"),ui_ok(&reply),reply.to_string());
   let v=ipc(&app,"main","get_agent_session",json!({"sessionId":sid})).await;record(&mut ck,&mut ev,&format!("{engine} native id unchanged"),value(&v)["native_id"]==s["native_id"],v.describe());
   let seq=last_seq(&app,sid).await;ui_send(&app,&mut ck,&mut ev,RECALL).await;let r=await_reply(&app,sid,RECALL,s["token"].as_str().unwrap(),seq).await;record(&mut ck,&mut ev,&format!("{engine} recalls pre-exit token"),r.is_ok(),format!("{r:?}"));close_ui(&app,&mut ck,&mut ev,sid).await;
  }
  let pid=first["project"].as_str().unwrap();let current=workspace(&app).await;let path=PathBuf::from(current["projects"].as_array().unwrap().iter().find(|p|p["id"]==pid).unwrap()["path"].as_str().unwrap());let moved=fx.root.join(format!("alpha-relocated-{}",uuid::Uuid::new_v4()));fs::rename(&path,&moved).unwrap();
  let inspect=ipc(&app,"main","inspect_development_project",json!({"projectId":pid})).await;record(&mut ck,&mut ev,"missing path detected",value(&inspect)["path_state"]=="missing",inspect.describe());
  let r=ui(&app,json!({"path":moved.to_string_lossy()}),r#"await fill('dev-project-relocate-input',A.path);await click('dev-project-relocate');const ok=await until(()=>txt(tid('dev-project-detail')).includes(A.path));return {ok:!!ok};"#).await;record(&mut ck,&mut ev,"relocate through production UI",ui_ok(&r),r.to_string());
  let inspect=ipc(&app,"main","inspect_development_project",json!({"projectId":pid})).await;record(&mut ck,&mut ev,"relocated directory available",value(&inspect)["path_state"]=="available",inspect.describe());
  if !ui_sign_out(&app,&mut ck,&mut ev,ALICE.0).await || !ui_sign_in(&app,&mut ck,&mut ev,BOB).await {finish(&app,&fx,ck,ev,"restart").await;return;}goto(&app,"main","/tasks/agent").await;
  let bob=workspace(&app).await;record(&mut ck,&mut ev,"Bob cannot list Alice projects/tasks/sessions",["projects","tasks","sessions"].iter().all(|k|bob[k].as_array().is_some_and(|v|v.is_empty())),bob.to_string());
  let sid=first["sessions"][0]["id"].as_str().unwrap();for (cmd,args) in [("inspect_development_project",json!({"projectId":pid})),("get_agent_session",json!({"sessionId":sid})),("select_development_context",json!({"input":{"project_id":pid,"task_id":null,"session_id":null}}))]{let r=ipc(&app,"main",cmd,args).await;record(&mut ck,&mut ev,&format!("Bob denied {cmd}"),!succeeded(&r),r.describe());}
  finish(&app,&fx,ck,ev,"restart").await;
 } else {
  let path=fx.root.join("alpha");make_repo(&path);fs::write(path.join("README.md"),"dirty user code\n").unwrap();
  ui_import_project(&app,&mut ck,&mut ev,"Alpha",&path).await;let pid=selected(&app).await["project_id"].as_str().unwrap_or_default().to_owned();
  let inspect=ipc(&app,"main","inspect_development_project",json!({"projectId":pid})).await;record(&mut ck,&mut ev,"dirty Git status visible",value(&inspect)["kind"]=="git"&&value(&inspect)["changes"].as_array().is_some_and(|v|!v.is_empty()),inspect.describe());
  let mut rows=vec![];
  for engine in ENGINES {
   let goal=format!("Verify {engine} session");ui_new_task(&app,&mut ck,&mut ev,&goal).await;let task=selected(&app).await["task_id"].clone();
   let name=if engine=="codex"{"Codex"}else{"Claude"};let r=pick_engine(&app,name).await;record(&mut ck,&mut ev,&format!("choose {name}"),ui_ok(&r),r.to_string());
   let before=ids(&sessions_of(&app).await);ui_start_session(&app,&mut ck,&mut ev,engine).await;
   let Some(sid)=new_session_id(&app,&before).await else {record(&mut ck,&mut ev,"new session persisted",false,engine);continue;};
   let code=token("DGOS");let prompt=format!("Remember the exact code {code}. Reply with only that code. Do not use tools.");let seq=last_seq(&app,&sid).await;ui_send(&app,&mut ck,&mut ev,&prompt).await;
   let result=await_reply(&app,&sid,&prompt,&code,seq).await;record(&mut ck,&mut ev,&format!("{engine} real model reply"),result.is_ok(),format!("{result:?}"));
   let v=ipc(&app,"main","get_agent_session",json!({"sessionId":sid})).await;
   rows.push(json!({"engine":engine,"id":sid,"task":task,"goal":goal,"native_id":value(&v)["native_id"],"token":code}));
   // Switch away while the runtime exists, then attach without starting another process.
   let r=ui(&app,json!({}),r#"await click('dev-task-new');return {ok:true};"#).await;record(&mut ck,&mut ev,"switch task without closing runtime",ui_ok(&r),r.to_string());
   let still=ipc(&app,"main","get_agent_session",json!({"sessionId":sid})).await;record(&mut ck,&mut ev,"switched runtime remains alive",succeeded(&still)&&value(&still)["status"]!="closed",still.describe());
   ui_select_task(&app,&mut ck,&mut ev,&goal).await;
   let r=ui(&app,json!({"sid":sid}),r#"const row=await need('dev-session-item-'+A.sid);row.querySelector('button').click();const composer=await until(()=>document.getElementById('agent-message'));return {ok:!!composer};"#).await;record(&mut ck,&mut ev,"reattach existing runtime through UI",ui_ok(&r),r.to_string());
   close_ui(&app,&mut ck,&mut ev,&sid).await;
  }
  // A second ordinary directory exercises project switching, then return to the saved selection.
  let second=fx.root.join("plain");fs::create_dir(&second).unwrap();ui_import_project(&app,&mut ck,&mut ev,"Plain directory",&second).await;
  let r=ui(&app,json!({"pid":pid}),r#"await click('dev-project-item-'+A.pid);return {ok:true};"#).await;record(&mut ck,&mut ev,"switch project through UI",ui_ok(&r),r.to_string());
  if let Some(s)=rows.last(){ui_select_task(&app,&mut ck,&mut ev,s["goal"].as_str().unwrap()).await;}
  let selection=selected(&app).await;fs::write(fx.root.join("evidence-first.json"),serde_json::to_vec_pretty(&json!({"project":pid,"sessions":rows,"selection":selection,"checks":ev})).unwrap()).unwrap();
  finish(&app,&fx,ck,ev,"first").await;
 }
}
async fn finish(app:&AppHandle,fx:&Fx,mut ck:Checks,ev:Vec<Value>,phase:&str){
 let closed=app.state::<AgentManager>().shutdown().await;ck.check("all Agent processes closed",closed,"shutdown");fs::write(fx.root.join(format!("result-{phase}.json")),serde_json::to_vec_pretty(&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev})).unwrap()).unwrap();println!("PROJECT SMOKE {phase}: {} passed, {} failed; evidence {}",ck.passed,ck.failed.len(),fx.root.display());app.exit(if ck.failed.is_empty(){0}else{1});
}
