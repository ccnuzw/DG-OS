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
 println!("ACCEPTANCE selected phase: {phase:?}");
 let restart=matches!(phase.as_str(),"restart"|"stage3-restart"|"stage6-restart");
 if !restart {
  for (user,pw) in [ALICE,BOB] {let r=ipc(&app,"main","create_account",json!({"input":{"username":user,"password":pw}})).await;record(&mut ck,&mut ev,&format!("create {user} account"),r.success(),r.describe());}
  let _=ipc(&app,"main","sign_out",json!({})).await;
 }
 if !ui_sign_in(&app,&mut ck,&mut ev,ALICE).await {finish(&app,&fx,ck,ev,"login").await;return;}
 record(&mut ck,&mut ev,"navigate production project workbench",goto(&app,"main","/tasks/agent").await,"navigation");
 if phase == "layout-round2" { super::layout_round2::run(&app, &fx, ck, ev).await; return; }
 if phase == "terminal-round3" { super::round3_terminal::run(&app, &fx, ck, ev).await; return; }
 if phase == "workflows-round3" { super::round3_workflows::run(&app, &fx, ck, ev).await; return; }
 if matches!(phase.as_str(),"round4-native"|"round4-pending-fixture") { super::round4::run(&app, &fx, ck, ev).await; return; }
 if matches!(phase.as_str(),"stage6-first"|"stage6-restart") { super::stage6::run(&app, &fx, ck, ev, restart).await; return; }
 if phase == "terminal-activity" { super::stage3::terminal_activity(&app, &fx, ck, ev).await; return; }
 if phase == "worktree-runtime" { super::stage3::worktree_runtime(&app, &fx, ck, ev).await; return; }
 if matches!(phase.as_str(),"stage3-first"|"stage3-restart") { super::stage3::run(&app, &fx, ck, ev, restart).await; return; }
 if std::env::var_os("DGOS_STAGE2_ACCEPTANCE").is_some() { super::stage2::run(&app, &fx, ck, ev, restart).await; return; }
 if std::env::var_os("DGOS_AGENT_CLOSE_ONLY").is_some() { close_boundary(&app,&fx,&mut ck,&mut ev).await;finish(&app,&fx,ck,ev,"close-boundary").await;return; }
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

// A real CLI handshake and surface close, without a model-answer acceptance run. The pending-open
// window is timing-dependent on the host; report it separately from the deterministic React regression.
async fn close_boundary(app:&AppHandle,fx:&Fx,ck:&mut Checks,ev:&mut Vec<Value>){
 let path=fx.root.join("close-boundary");make_repo(&path);
 if !ui_import_project(app,ck,ev,"Close boundary",&path).await || !ui_new_task(app,ck,ev,"Close initial Agent connection").await {return;}
 let before=ids(&sessions_of(app).await);
 let start=ui(app,json!({}),r#"
const trust=await need('dev-trust');
const box=trust.matches('input,[role=checkbox]')?trust:trust.querySelector('input,[role=checkbox]')||trust;
if(!isChecked(box))box.click();
const start=await until(()=>{const b=tid('dev-session-new-codex');return b&&!b.disabled&&b;});
if(!start)return {ok:false,error:'new session disabled',trusted:isChecked(box),cwd:txt(tid('dev-session-cwd')),alerts:$$('[role=alert]').map(txt),panel:txt(tid('dev-session-list'))};
start.click();
const tab=()=>document.querySelector('[data-workspace-tab="cli-agent"]');
const state=await until(()=>tab()?.querySelector('.workspace-running-dot')?'pending':document.getElementById('agent-message')?'ready':null);
if(!state)throw Error('no pending connection or conversation');
if(state==='ready')return {ok:true,pending:false};
const close=tab()?.parentElement.querySelector('.workspace-tab-close');
if(!close)throw Error('surface close control');
close.click();
const dialog=await until(()=>document.querySelector('[role=dialog]'));
if(!dialog)throw Error('pending connection close has no confirmation');
if(!txt(dialog).includes('停止并关闭'))throw Error('unexpected close confirmation');
const cancel=$$('button',dialog).find(b=>txt(b)==='取消');
if(!cancel)throw Error('cancel close control');
cancel.click();
const dismissed=await until(()=>!document.querySelector('[role=dialog]'));
return {ok:!!dismissed&&!!tab(),pending:true,cancelled:true};
"#).await;
 let pending=start["value"]["pending"]==true;
 record(ck,ev,"initial Agent open close attempt",ui_ok(&start),start.to_string());
 ev.push(json!({"observation":"initial-open pending window captured","captured":pending}));
 if !ui_ok(&start){return;}
 let Some(sid)=new_session_id(app,&before).await else {record(ck,ev,"initial Agent session persisted",false,"no session id");return;};
 if !pending {
  // A fast handshake may already be idle. A real send supplies an active runtime for the same
  // cancel/confirm surface-close contract, without requiring or inspecting a model response.
  let active=ui(app,json!({}),r#"
const composer=await until(()=>document.getElementById('agent-message'));
if(!composer)throw Error('no Agent composer');
setInput(composer,'Reply with OK.');
const send=await until(()=>btn('发送'));if(!send)throw Error('send control');send.click();
const tab=()=>document.querySelector('[data-workspace-tab="cli-agent"]');
if(!await until(()=>tab()?.querySelector('.workspace-running-dot')))throw Error('active Agent window not observed');
tab().parentElement.querySelector('.workspace-tab-close').click();
const dialog=await until(()=>document.querySelector('[role=dialog]'));
if(!dialog)throw Error('active Agent close has no confirmation');
const cancel=$$('button',dialog).find(b=>txt(b)==='取消');if(!cancel)throw Error('cancel close control');cancel.click();
return {ok:!!(await until(()=>!document.querySelector('[role=dialog]')))&&!!tab(),cancelled:true};
"#).await;
  record(ck,ev,"active Agent close cancellation after fast handshake",ui_ok(&active),active.to_string());
  if !ui_ok(&active){return;}
 }
 let current=ipc(app,"main","get_agent_session",json!({"sessionId":sid})).await;
 record(ck,ev,"cancelled surface close preserves host runtime",succeeded(&current)&&value(&current)["status"]!="closed",current.describe());
 let close=ui(app,json!({}),r#"
const tab=()=>document.querySelector('[data-workspace-tab="cli-agent"]');
if(!tab()?.querySelector('.workspace-running-dot')){
 const composer=await until(()=>document.getElementById('agent-message'));if(!composer)throw Error('no Agent composer');
 setInput(composer,'Reply with OK.');const send=await until(()=>btn('发送'));if(!send)throw Error('send control');send.click();
 if(!await until(()=>tab()?.querySelector('.workspace-running-dot')))throw Error('active Agent window not observed');
}
const close=tab()?.parentElement.querySelector('.workspace-tab-close');if(!close)throw Error('surface close control');close.click();
const dialog=await until(()=>document.querySelector('[role=dialog]'));if(!dialog)throw Error('Agent surface close has no confirmation');
const confirm=$$('button',dialog).find(b=>txt(b)==='停止并关闭');if(!confirm)throw Error('stop and close control');confirm.click();
return {ok:!!(await until(()=>!tab())),confirmed:true};
"#).await;
 record(ck,ev,"Agent surface close completes through production UI",ui_ok(&close),close.to_string());
 let closed=ipc(app,"main","get_agent_session",json!({"sessionId":sid})).await;
 record(ck,ev,"closed Agent surface has terminal host session",succeeded(&closed)&&value(&closed)["status"]=="closed",closed.describe());
}
async fn finish(app:&AppHandle,fx:&Fx,mut ck:Checks,ev:Vec<Value>,phase:&str){
 let closed=app.state::<AgentManager>().shutdown().await;ck.check("all Agent processes closed",closed,"shutdown");fs::write(fx.root.join(format!("result-{phase}.json")),serde_json::to_vec_pretty(&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev})).unwrap()).unwrap();println!("PROJECT SMOKE {phase}: {} passed, {} failed; evidence {}",ck.passed,ck.failed.len(),fx.root.display());app.exit(if ck.failed.is_empty(){0}else{1});
}
