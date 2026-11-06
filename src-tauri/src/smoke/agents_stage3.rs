//! Real desktop stage3 acceptance. No CLI, filesystem, PTY, HTTP or opener mocks.
//! Run the two phases in separate processes with the same fresh acceptance root.
//!
//! Prerequisites: built dist and tools/agent-bridge, authenticated real codex/claude
//! CLIs on PATH, node/npm/git, a desktop session and its normal browser opener.
//! DGOS_PROJECT_ACCEPTANCE_ROOT=/absolute/fresh/path \
//! DGOS_PROJECT_ACCEPTANCE_PHASE=stage3-first \
//! cargo test -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture
//! Repeat with stage3-restart and the identical root after the first process exits.
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn write_json(path: impl AsRef<Path>, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn prepare(path: &Path) {
    make_repo(path);
    fs::write(path.join("package.json"), r#"{"type":"module","scripts":{"test":"node --test"}}"#).unwrap();
    fs::write(path.join("price.js"), "export function total(items) { return items.reduce((n, i) => n + i.price * i.quantity, 0); }\n").unwrap();
    fs::write(path.join("price.test.js"), "import {test} from 'node:test';\nimport assert from 'node:assert/strict';\nimport {total} from './price.js';\ntest('quantity and empty basket',()=>{assert.equal(total([{price:7,quantity:3},{price:2,quantity:1}]),23);assert.equal(total([]),0);});\n").unwrap();
    // A real project HTTP service also starts a descendant, so lifecycle checks examine
    // the owned tree rather than inferring cleanup from the leader or port alone.
    fs::write(path.join("service.cjs"), r#"const fs=require('node:fs');
const http=require('node:http');
const {spawn}=require('node:child_process');
const [port,pids,marker,body]=process.argv.slice(2);
const child=spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'});
fs.writeFileSync(pids,JSON.stringify([process.pid,child.pid]));
fs.appendFileSync(marker,'start\n');
http.createServer((req,res)=>{res.writeHead(req.url==='/health'?200:404,{'Content-Type':'text/plain','Content-Length':Buffer.byteLength(body)});res.end(body);}).listen(Number(port),'127.0.0.1');
"#).unwrap();
    fs::write(path.join(".gitignore"), "*.pids.json\n*.marker\n").unwrap();
    assert!(git(path, &["add", "."]));
    assert!(git(path, &["-c", "user.name=dgos", "-c", "user.email=dgos@example.invalid", "commit", "-q", "-m", "stage3 fixture"]));
}

fn free_port() -> u16 {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port()
}

async fn http(port: u16) -> Result<String, String> {
    let request = async {
        let mut stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await.map_err(|e|e.to_string())?;
        stream.write_all(format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes()).await.map_err(|e|e.to_string())?;
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.map_err(|e|e.to_string())?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    };
    tokio::time::timeout(Duration::from_secs(3), request).await.map_err(|_|"HTTP timeout".to_string())?
}

fn alive(pid: i64) -> bool {
    Command::new("/bin/kill").args(["-0", &pid.to_string()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s|s.success())
}

fn pids(path: &Path) -> Value {
    fs::read(path).ok().and_then(|b|serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
}

async fn gone(port: u16, owned: &Value) -> bool {
    for _ in 0..100 {
        let port_gone = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await.is_err();
        let tree_gone = owned.as_array().is_some_and(|rows|rows.len() >= 2 && rows.iter().all(|pid|pid.as_i64().is_some_and(|p|!alive(p))));
        if port_gone && tree_gone { return true; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

fn input(task: &str, kind: &str, program: &str, args: Vec<String>) -> Value {
    json!({"id":uuid::Uuid::new_v4().to_string(),"task_id":task,"kind":kind,"program":program,"args":args,"cwd":"","env_refs":{},"trust_workspace":true,"verification":false,"readiness":null,"native_run_id":null})
}

async fn start(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, request: &Value) -> Value {
    let reply = ipc(app,"main","start_development_process",json!({"input":request})).await;
    record(ck,ev,"explicit production host start",succeeded(&reply),reply.describe());
    value(&reply)
}

async fn process(app: &AppHandle, id: &str) -> Value {
    value(&ipc(app,"main","read_development_process",json!({"id":id})).await)
}

async fn wait_process(app: &AppHandle, id: &str, predicate: impl Fn(&Value)->bool) -> Value {
    let deadline = tokio::time::Instant::now()+Duration::from_secs(120);
    loop {
        let view = process(app,id).await;
        if predicate(&view) || tokio::time::Instant::now() >= deadline { return view; }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

async fn terminal(app: &AppHandle, id: &str) -> Value {
    wait_process(app,id,|v|v["finished_at"].is_number()).await
}

async fn stop(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, id: &str) -> Value {
    let reply = ipc(app,"main","stop_development_process",json!({"id":id})).await;
    record(ck,ev,"explicit host stop confirms terminal cleanup",succeeded(&reply) && value(&reply)["finished_at"].is_number() && value(&reply)["status"]!="cleanup_pending",reply.describe());
    value(&reply)
}

async fn tab(app: &AppHandle, name: &str) -> Value {
    ui(app,json!({"name":name}),r#"const t=await need('dev-review-tab-'+A.name);t.focus();t.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));return {ok:!!(await until(()=>t.getAttribute('data-state')==='active'))};"#).await
}

async fn open_file(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, path: &str) {
    tab(app,"files").await;
    let workspace=value(&ipc(app,"main","read_development_workspace",json!({})).await);
    let disk=value(&ipc(app,"main","read_development_file",json!({"input":{"project_id":workspace["selection"]["project_id"],"path":path}})).await);
    let reply=ui(app,json!({"path":path,"expected":disk["text"]}),r#"await fill('dev-file-search',A.path);const f=await need('dev-file-item-'+A.path);f.click();const editor=await until(()=>{const e=tid('dev-file-editor');return e&&e.value===A.expected&&txt(tid('dev-file-source')?.querySelector('strong'))===A.path&&e});return {ok:!!editor,text:editor?.value};"#).await;
    record(ck,ev,"real file opened in editor",ui_ok(&reply),reply.to_string());
}

async fn edit(app: &AppHandle, text: &str) -> Value {
    ui(app,json!({"text":text}),r#"await fill('dev-file-editor',A.text);return {ok:!!(await until(()=>tid('dev-file-editor')?.value===A.text))};"#).await
}

async fn save(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, path: &Path, text: &str) {
    let reply=edit(app,text).await;
    record(ck,ev,"user edit through actual textarea",ui_ok(&reply),reply.to_string());
    let reply=ui(app,json!({}),r#"const b=await until(()=>{const b=tid('dev-file-save');return b&&!b.disabled&&b});if(!b)throw Error('save unavailable');b.click();return {ok:!!(await until(()=>tid('dev-file-save')?.disabled&&!tid('dev-file-source')?.querySelector('[role=alert]')))};"#).await;
    let mut disk=String::new();
    for _ in 0..100 {disk=fs::read_to_string(path).unwrap_or_default();if disk==text {break;}tokio::time::sleep(Duration::from_millis(50)).await;}
    record(ck,ev,"UI save inspected on real disk",ui_ok(&reply)&&disk==text,format!("{reply}; disk={disk}"));
}

async fn conflict(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, path: &Path) {
    open_file(app,ck,ev,"README.md").await;
    let draft="unsaved local stage3 draft\n";
    edit(app,draft).await;
    let reply=ui(app,json!({"draft":draft}),r#"const t=await need('dev-review-tab-develop');t.focus();t.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));const dialog=await until(()=>document.querySelector('[role=alertdialog],[role=dialog]'));if(!dialog)throw Error('no discard confirmation');const cancel=$$('button',dialog).find(b=>['取消','Cancel'].includes(txt(b)));if(!cancel)throw Error('cancel absent');cancel.click();return {ok:!!(await until(()=>tid('dev-file-editor')?.value===A.draft&&!document.querySelector('[role=alertdialog],[role=dialog]')))};"#).await;
    record(ck,ev,"unsaved leave cancel retains editor and draft",ui_ok(&reply),reply.to_string());
    let disk="external writer stage3 content\n";
    fs::write(path.join("README.md"),disk).unwrap();
    let reply=ui(app,json!({"draft":draft,"disk":disk}),r#"await click('dev-file-save');const c=await need('dev-file-conflict');const compared=await until(()=>{const ps=$$('pre',c);return ps.length===2&&ps[0].textContent===A.draft&&ps[1].textContent===A.disk});return {ok:!!compared&&tid('dev-file-editor')?.value===A.draft&&tid('dev-file-save')?.disabled,comparison:txt(c)};"#).await;
    record(ck,ev,"stale save refuses overwrite and compares both real versions",ui_ok(&reply)&&fs::read_to_string(path.join("README.md")).unwrap()==disk,reply.to_string());
    let reply=ui(app,json!({"disk":disk}),r#"await click('dev-file-reload');const dialog=await until(()=>document.querySelector('[role=alertdialog],[role=dialog]'));if(!dialog)throw Error('reload must confirm discard');const b=$$('button',dialog).find(b=>['放弃并关闭','Discard and close'].includes(txt(b)));if(!b)throw Error('discard absent: '+txt(dialog));b.click();return {ok:!!(await until(()=>tid('dev-file-editor')?.value===A.disk&&!tid('dev-file-conflict')))};"#).await;
    record(ck,ev,"explicit discard reloads actual disk version",ui_ok(&reply),reply.to_string());
}

fn service_input(task: &str, port: u16, prefix: &str, body: &str) -> Value {
    let mut request=input(task,"service","node",vec!["service.cjs".into(),port.to_string(),format!("{prefix}.pids.json"),format!("{prefix}.marker"),body.into()]);
    request["readiness"]=json!({"port":port,"path":"/health","timeout_ms":10000});
    request
}

async fn ready_service(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, task: &str, port: u16, prefix: &str, body: &str) -> Value {
    let request=service_input(task,port,prefix,body);
    start(app,ck,ev,&request).await;
    let view=wait_process(app,request["id"].as_str().unwrap(),|v|v["status"]=="ready"||v["finished_at"].is_number()).await;
    let response=http(port).await;
    record(ck,ev,"real service HTTP readiness and GET body",view["status"]=="ready"&&view["preview_url"]==format!("http://127.0.0.1:{port}/health")&&response.as_ref().is_ok_and(|r|r.starts_with("HTTP/1.1 200")&&r.ends_with(body)),format!("{view}; HTTP={response:?}"));
    view
}

async fn preview(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, project: &str, id: &str, allowed: bool) {
    let snapshot=process(app,id).await;
    let reply=ipc(app,"main","open_development_external",json!({"input":{"project_id":project,"task_id":snapshot["task_id"],"process_id":id,"target":"preview"}})).await;
    record(ck,ev,if allowed {"ready preview opens external browser via production IPC"} else {"non-ready preview denied by production IPC"},succeeded(&reply)==allowed,reply.describe());
}

async fn native_repair(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, engine: &str, task: &str, path: &Path, fx: &Fx) -> (String,Value) {
    let choice=acceptance::pick_engine(app,if engine=="codex"{"Codex"}else{"Claude"}).await;
    record(ck,ev,"select real repair engine",ui_ok(&choice),choice.to_string());
    let _=ui(app,json!({}),r#"const f=$$('input').find(e=>labelOf(e).startsWith('模型'));if(f)setInput(f,'');return {ok:true};"#).await;
    let before=ids(&sessions_of(app).await);
    ui_start_session(app,ck,ev,engine).await;
    let Some(sid)=new_session_id(app,&before).await else {record(ck,ev,"real Agent created",false,engine);return (String::new(),Value::Null);};
    let original_tests=fs::read(path.join("price.test.js")).unwrap();
    let original_service=fs::read(path.join("service.cjs")).unwrap();
    // Keep the existing real-CLI first-turn warmup; no simulated messages or tool results.
    stage2::exercise_turn(app,ck,ev,&sid,"Hello. We will repair this quantity project in the next request. Do not modify files yet.").await;
    let before_runs=value(&ipc(app,"main","list_development_runs",json!({"taskId":task})).await);
    let before_ids:Vec<String>=before_runs.as_array().into_iter().flatten().filter_map(|r|r["id"].as_str().map(str::to_owned)).collect();
    let view=stage2::exercise_turn(app,ck,ev,&sid,"The user saved a regression in price.js and the host npm test failed. Run npm test to observe it, repair price.js so total sums price * quantity (and returns 0 for an empty basket), then run npm test again. You MUST modify price.js and execute the actual test command. Keep price.test.js and service.cjs unchanged; do not weaken tests, install dependencies or perform git writes.").await;
    write_json(fx.root.join(format!("stage3-{engine}-native-events.json")),&view);
    let deadline=tokio::time::Instant::now()+Duration::from_secs(10);
    let native=loop {
        let runs=value(&ipc(app,"main","list_development_runs",json!({"taskId":task})).await);
        let found=runs.as_array().into_iter().flatten().find(|r|r["source"]=="native_agent"&&!before_ids.iter().any(|id|r["id"]==id.as_str())).cloned().unwrap_or(Value::Null);
        if found["status"]=="completed" || tokio::time::Instant::now()>=deadline {break found;}
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let observed=view["events"].as_array().is_some_and(|es|es.iter().any(|e|{
        if matches!(e["kind"].as_str(),Some("userMessage"|"text_delta"|"agentMessage"|"result")){return false;}
        let data=e["data"].to_string();data.contains("npm test")&&(data.contains("command")||data.contains("tool_use")||data.contains("tool_result"))
    }));
    record(ck,ev,"real engine modifies code and executes native test tool",native["status"]=="completed"&&observed&&fs::read_to_string(path.join("price.js")).unwrap_or_default()!="export function total(items) { return -999; }\n",format!("native={native}; tool_observed={observed}"));
    record(ck,ev,"Agent preserves meaningful user tests and actual service",fs::read(path.join("price.test.js")).unwrap()==original_tests&&fs::read(path.join("service.cjs")).unwrap()==original_service,engine);
    acceptance::close_ui(app,ck,ev,&sid).await;
    (sid,native)
}

async fn per_engine(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, engine: &str) -> Value {
    let path=fx.root.join(format!("stage3-{engine}"));prepare(&path);
    ui_import_project(app,ck,ev,&format!("Stage3 {engine}"),&path).await;
    let goal=format!("Stage3 repair and serve {engine}");ui_new_task(app,ck,ev,&goal).await;
    let selected=value(&ipc(app,"main","read_development_workspace",json!({})).await)["selection"].clone();
    let task=selected["task_id"].as_str().unwrap_or_default();let project=selected["project_id"].as_str().unwrap_or_default();
    conflict(app,ck,ev,&path).await;
    open_file(app,ck,ev,"price.js").await;
    save(app,ck,ev,&path.join("price.js"),"export function total(items) { return -999; }\n").await;
    let mut request=input(task,"command","npm",vec!["test".into()]);request["verification"]=json!(true);
    start(app,ck,ev,&request).await;
    let failed=terminal(app,request["id"].as_str().unwrap()).await;
    record(ck,ev,"user saved regression fails actual host test",failed["exit_code"].as_i64().is_some_and(|n|n!=0)&&failed["verification_status"]=="failed"&&failed["output"].as_str().is_some_and(|s|s.contains("-999")),failed.to_string());
    // A terminal host record releases directory admission before opening the real Agent.
    stop(app,ck,ev,request["id"].as_str().unwrap()).await;
    let (sid,native)=native_repair(app,ck,ev,engine,task,&path,fx).await;
    let mut verify=input(task,"command","npm",vec!["test".into()]);verify["verification"]=json!(true);verify["native_run_id"]=native["id"].clone();
    start(app,ck,ev,&verify).await;
    let verified=terminal(app,verify["id"].as_str().unwrap()).await;
    record(ck,ev,"independent host verification linked to real native run",!native["id"].is_null()&&verified["native_run_id"]==native["id"]&&verified["source"]=="host_command"&&verified["exit_code"]==0&&verified["verification_status"]=="observed_stable"&&verified["output"].as_str().is_some_and(|s|s.contains("quantity and empty basket")),verified.to_string());
    stop(app,ck,ev,verify["id"].as_str().unwrap()).await;
    let port=free_port();let body=format!("stage3-{engine}-actual-response");
    let service=ready_service(app,ck,ev,task,port,"preview",&body).await;
    let owned=pids(&path.join("preview.pids.json"));
    preview(app,ck,ev,project,service["id"].as_str().unwrap_or_default(),true).await;
    stop(app,ck,ev,service["id"].as_str().unwrap_or_default()).await;
    record(ck,ev,"preview stop removes port and owned descendant tree",gone(port,&owned).await,format!("port={port}; pids={owned}"));
    let replay=input(task,"command","/bin/sh",vec!["-c".into(),"printf 'executed-once\\n' >> once.marker".into()]);
    start(app,ck,ev,&replay).await;
    let once=terminal(app,replay["id"].as_str().unwrap()).await;
    let replayed=ipc(app,"main","start_development_process",json!({"input":replay})).await;
    record(ck,ev,"same request UUID returns receipt without command replay",once["exit_code"]==0&&succeeded(&replayed)&&fs::read_to_string(path.join("once.marker")).unwrap_or_default()=="executed-once\n",replayed.describe());
    json!({"engine":engine,"project":project,"task":task,"goal":goal,"path":path,"sid":sid,"native_run":native["id"],"failed":failed,"verified":verified,"service":service,"marker":path.join("preview.marker"),"marker_contents":fs::read_to_string(path.join("preview.marker")).unwrap_or_default(),"once_request":replay})
}

async fn select_row(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, row: &Value) {
    let reply=ui(app,json!({"project":row["project"]}),r#"await click('dev-project-item-'+A.project);return {ok:true};"#).await;
    record(ck,ev,"project switch through UI",ui_ok(&reply),reply.to_string());
    ui_select_task(app,ck,ev,row["goal"].as_str().unwrap()).await;
}

async fn pty_and_switch(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, a: &Value, b: &Value) {
    select_row(app,ck,ev,a).await;
    let request=input(a["task"].as_str().unwrap(),"terminal","/bin/sh",vec![]);
    start(app,ck,ev,&request).await;let id=request["id"].as_str().unwrap();
    // Wait for the real PTY before issuing input, never replay input across selection changes.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let resize=ipc(app,"main","resize_development_terminal",json!({"input":{"id":id,"cols":113,"rows":37}})).await;
    record(ck,ev,"actual PTY resize IPC",succeeded(&resize),resize.describe());
    let nonce=token("PTY");
    let write=ipc(app,"main","write_development_terminal",json!({"input":{"id":id,"data":format!("stty size; printf '{}\\n'\n",nonce)}})).await;
    record(ck,ev,"actual PTY input IPC",succeeded(&write),write.describe());
    let output=wait_process(app,id,|v|v["output"].as_str().is_some_and(|o|o.contains("37 113")&&o.contains(&nonce))).await;
    record(ck,ev,"PTY output reports actual input and dimensions",output["output"].as_str().is_some_and(|o|o.contains("37 113")&&o.contains(&nonce)),output.to_string());
    tab(app,"develop").await;
    let shown=ui(app,json!({"nonce":nonce}),r#"const row=await until(()=>$$('.dev-process-history button').find(b=>txt(b).includes('/bin/sh')));if(!row)throw Error('terminal history missing');row.click();return {ok:!!(await until(()=>tid('dev-terminal')?.innerText.includes(A.nonce)))};"#).await;
    record(ck,ev,"real terminal output rendered in desktop",ui_ok(&shown),shown.to_string());
    select_row(app,ck,ev,b).await;tab(app,"develop").await;
    let other=input(b["task"].as_str().unwrap(),"command","/bin/sh",vec!["-c".into(),"printf 'OTHER_PROJECT_OUTPUT\\n'".into()]);start(app,ck,ev,&other).await;terminal(app,other["id"].as_str().unwrap()).await;
    let shown=ui(app,json!({"nonce":nonce,"id":other["id"]}),r#"const row=await until(()=>document.querySelector('[data-testid="dev-process-history-'+A.id+'"]'));if(!row)throw Error('other command missing');row.click();const ok=await until(()=>tid('dev-process-detail')?.innerText.includes('OTHER_PROJECT_OUTPUT'));return {ok:!!ok&&!tid('dev-process-detail').innerText.includes(A.nonce)&&!tid('dev-terminal'),text:txt(tid('dev-process-detail'))};"#).await;
    record(ck,ev,"project switch never bleeds previous PTY output",ui_ok(&shown),shown.to_string());
    let still=process(app,id).await;record(ck,ev,"project switching retains owned PTY without stopping",still["status"]=="running",still.to_string());
    select_row(app,ck,ev,a).await;tab(app,"develop").await;
    let shown=ui(app,json!({"nonce":nonce}),r#"const row=await until(()=>$$('.dev-process-history button').find(b=>txt(b).includes('/bin/sh')));if(!row)throw Error('terminal history missing on return');row.click();return {ok:!!(await until(()=>tid('dev-terminal')?.innerText.includes(A.nonce)))};"#).await;
    record(ck,ev,"PTY reconnect rereads same output instance",ui_ok(&shown),shown.to_string());
    let exit=ipc(app,"main","write_development_terminal",json!({"input":{"id":id,"data":"printf 'PTY_EXIT_CONFIRMED\\n'; exit 7\n"}})).await;
    let exited=terminal(app,id).await;
    record(ck,ev,"PTY actual exit output and exit code retained",succeeded(&exit)&&exited["exit_code"]==7&&exited["output"].as_str().is_some_and(|s|s.contains("PTY_EXIT_CONFIRMED")),exited.to_string());
    let request=input(a["task"].as_str().unwrap(),"command","node",vec!["-e".into(),"process.stdout.write('bounded output line\\n'.repeat(120000))".into()]);start(app,ck,ev,&request).await;
    let bounded=terminal(app,request["id"].as_str().unwrap()).await;
    record(ck,ev,"real excessive output has explicit bounded truncation",bounded["finished_at"].is_number()&&bounded["truncated"]==true&&bounded["output"].as_str().is_some_and(|s|s.starts_with("bounded output line\n")&&s.len()<=1024*1024),format!("truncated={}, output_bytes={}",bounded["truncated"],bounded["output"].as_str().map_or(0,str::len)));
}

async fn service_boundaries(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, row: &Value) {
    let task=row["task"].as_str().unwrap();let project=row["project"].as_str().unwrap();
    // This is a real external HTTP server, not an AgentManager-owned process. Keep its
    // Child handle, and demonstrate both HTTP liveness and try_wait after the refusal.
    let port=free_port();let body=token("EXTERNAL");
    let mut external=Command::new("node").args(["-e","require('node:http').createServer((q,r)=>r.end(process.argv[2])).listen(Number(process.argv[1]),'127.0.0.1')",&port.to_string(),&body]).spawn().unwrap();
    for _ in 0..100 {if http(port).await.as_ref().is_ok_and(|s|s.ends_with(&body)){break;}tokio::time::sleep(Duration::from_millis(50)).await;}
    let request=service_input(task,port,"occupied","must not start");start(app,ck,ev,&request).await;
    let refused=terminal(app,request["id"].as_str().unwrap()).await;
    let response=http(port).await;
    record(ck,ev,"live occupied port refused without killing external owner",refused["status"]=="failed"&&matches!(refused["reason"].as_str(),Some("port_conflict"|"service_port_in_use"))&&external.try_wait().unwrap().is_none()&&response.as_ref().is_ok_and(|s|s.ends_with(&body)),format!("{refused}; external_pid={}; HTTP={response:?}",external.id()));
    preview(app,ck,ev,project,request["id"].as_str().unwrap(),false).await;
    external.kill().unwrap();external.wait().unwrap();
    let timeout_port=free_port();
    let mut request=input(task,"service","/bin/sh",vec!["-c".into(),"sleep 30".into()]);request["readiness"]=json!({"port":timeout_port,"path":"/health","timeout_ms":600});
    start(app,ck,ev,&request).await;
    let timeout=terminal(app,request["id"].as_str().unwrap()).await;
    record(ck,ev,"readiness timeout never publishes preview",timeout["status"]=="failed"&&timeout["reason"]=="service_readiness_timeout"&&timeout["preview_url"].is_null()&&tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST,timeout_port)).await.is_err(),timeout.to_string());
    preview(app,ck,ev,project,request["id"].as_str().unwrap(),false).await;
}

async fn verification_boundaries(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, row: &Value) {
    let task=row["task"].as_str().unwrap();let path=Path::new(row["path"].as_str().unwrap());
    fs::write(path.join("external-change.txt"),"a new external mutation\n").unwrap();
    let old=process(app,row["verified"]["id"].as_str().unwrap()).await;
    record(ck,ev,"old successful evidence becomes stale against actual disk",old["verification_status"]=="stale",old.to_string());
    let mut request=input(task,"command","/bin/sh",vec!["-c".into(),"sleep 2; printf 'observed mutation command done\\n'".into()]);request["verification"]=json!(true);
    start(app,ck,ev,&request).await;
    tokio::time::sleep(Duration::from_millis(650)).await;fs::write(path.join("external-change.txt"),"changed while command ran\n").unwrap();
    let changed=terminal(app,request["id"].as_str().unwrap()).await;
    record(ck,ev,"zero exit with actual execution-time mutation cannot pass",changed["exit_code"]==0&&changed["verification_status"]=="changed"&&changed["mutation_observed"]==true,changed.to_string());
    fs::write(path.join("oversized.txt"),vec![b'a';262145]).unwrap();
    let mut request=input(task,"command","/bin/sh",vec!["-c".into(),"printf 'incomplete snapshot command done\\n'".into()]);request["verification"]=json!(true);
    start(app,ck,ev,&request).await;
    let incomplete=terminal(app,request["id"].as_str().unwrap()).await;
    record(ck,ev,"incomplete actual scan cannot confirm a zero-exit pass",incomplete["exit_code"]==0&&incomplete["verification_status"]=="incomplete"&&incomplete["verification_complete"]==false,incomplete.to_string());
    // Only fixture-owned data is removed. No repository/user files are touched.
    fs::remove_file(path.join("oversized.txt")).unwrap();
    let request=input(task,"command","/bin/sh",vec!["-c".into(),"printf 'CANCEL_READY\\n'; sleep 30".into()]);start(app,ck,ev,&request).await;
    wait_process(app,request["id"].as_str().unwrap(),|v|v["output"].as_str().is_some_and(|s|s.contains("CANCEL_READY"))).await;
    let cancelled=stop(app,ck,ev,request["id"].as_str().unwrap()).await;
    record(ck,ev,"actual host command cancellation retained",cancelled["status"]=="cancelled"&&cancelled["output"].as_str().is_some_and(|s|s.contains("CANCEL_READY")),cancelled.to_string());
}

async fn account_cleanup(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, row: &Value) {
    let task=row["task"].as_str().unwrap();let path=Path::new(row["path"].as_str().unwrap());let port=free_port();
    let service=ready_service(app,ck,ev,task,port,"revoke","revoke-response").await;
    let mut owned=pids(&path.join("revoke.pids.json"));
    let terminal_request=input(task,"terminal","/bin/sh",vec!["-c".into(),"printf '%s' \"$$\" > revoke-pty.marker; printf 'REVOKE_PTY_READY\\n'; sleep 30".into()]);
    start(app,ck,ev,&terminal_request).await;
    wait_process(app,terminal_request["id"].as_str().unwrap(),|v|v["output"].as_str().is_some_and(|s|s.contains("REVOKE_PTY_READY"))).await;
    if let Ok(pid)=fs::read_to_string(path.join("revoke-pty.marker")) {if let Ok(pid)=pid.parse::<i64>() {if let Some(rows)=owned.as_array_mut() {rows.push(json!(pid));}}}
    ui_sign_out(app,ck,ev,ALICE.0).await;
    record(ck,ev,"account lease revocation removes actual service and PTY tree and port",owned.as_array().is_some_and(|p|p.len()>=3)&&gone(port,&owned).await,format!("port={port}; pids={owned}"));
    ui_sign_in(app,ck,ev,BOB).await;goto(app,"main","/tasks/agent").await;
    for (command,args) in [("read_development_process",json!({"id":service["id"]})),("stop_development_process",json!({"id":service["id"]})),("write_development_terminal",json!({"input":{"id":terminal_request["id"],"data":"echo forbidden\n"}})),("list_development_processes",json!({"taskId":task}))] {
        let reply=ipc(app,"main",command,args).await;record(ck,ev,&format!("other account cannot {command}"),!succeeded(&reply),reply.describe());
    }
    let workspace=value(&ipc(app,"main","read_development_workspace",json!({})).await);
    record(ck,ev,"account switch clears previous projects and output projection",workspace["projects"].as_array().is_some_and(Vec::is_empty),workspace.to_string());
    ui_sign_out(app,ck,ev,BOB.0).await;ui_sign_in(app,ck,ev,ALICE).await;goto(app,"main","/tasks/agent").await;
    let service=process(app,service["id"].as_str().unwrap()).await;let terminal=process(app,terminal_request["id"].as_str().unwrap()).await;
    record(ck,ev,"revoked service and PTY terminal histories survive switch",service["status"]=="cancelled"&&terminal["status"]=="cancelled"&&service["finished_at"].is_number()&&terminal["finished_at"].is_number(),format!("service={service}; PTY={terminal}"));
}

async fn restart(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) {
    let saved:Value=serde_json::from_slice(&fs::read(fx.root.join("stage3-state.json")).expect("stage3-first state required")).unwrap();
    let before=value(&ipc(app,"main","read_development_workspace",json!({})).await);
    let history=value(&ipc(app,"main","list_development_processes",json!({"taskId":saved["exit"]["task"]})).await);
    let before_count=history.as_array().map_or(0,Vec::len);
    tokio::time::sleep(Duration::from_secs(2)).await;
    for row in saved["engines"].as_array().unwrap() {
        select_row(app,ck,ev,row).await;
        if let Ok(editor)=std::env::var("DGOS_ACCEPTANCE_EDITOR") {
            let reply=ipc(app,"main","open_development_external",json!({"input":{"project_id":row["project"],"task_id":row["task"],"process_id":null,"target":"editor","editor":editor}})).await;
            record(ck,ev,"validated project opens in explicit external editor",succeeded(&reply),reply.describe());
        }
        let runs=value(&ipc(app,"main","list_development_runs",json!({"taskId":row["task"]})).await);
        let processes=value(&ipc(app,"main","list_development_processes",json!({"taskId":row["task"]})).await);
        let expected=row["history_ids"].as_array().unwrap();
        record(ck,ev,"native and host histories retained across app process",runs.as_array().is_some_and(|runs|expected.iter().all(|id|runs.iter().any(|r|r["id"]==*id)))&&runs.as_array().is_some_and(|runs|runs.iter().any(|r|r["id"]==row["native_run"]&&r["source"]=="native_agent")),runs.to_string());
        record(ck,ev,"restart never automatically starts host processes or services",processes.as_array().is_some_and(|ps|ps.iter().all(|p|!matches!(p["status"].as_str(),Some("running"|"ready"|"starting")))),processes.to_string());
        let marker=Path::new(row["marker"].as_str().unwrap());
        record(ck,ev,"restart does not replay per-engine service marker",fs::read_to_string(marker).unwrap_or_default()==row["marker_contents"].as_str().unwrap_or_default(),marker.display());
        let replayed=ipc(app,"main","start_development_process",json!({"input":row["once_request"]})).await;
        let once_path=Path::new(row["path"].as_str().unwrap()).join("once.marker");
        record(ck,ev,"restart command marker and explicit repeated UUID never reexecute",succeeded(&replayed)&&fs::read_to_string(&once_path).unwrap_or_default()=="executed-once\n",replayed.describe());
        let reply=tab(app,"develop").await;
        record(ck,ev,"retained host history visible in actual restarted desktop",ui_ok(&reply)&&ui_ok(&ui(app,json!({}),r#"return {ok:!!(await until(()=>$$('.dev-process-history button').length>0))};"#).await),reply.to_string());
    }
    let exit=&saved["exit"];let owned=&exit["pids"];let port=exit["port"].as_u64().unwrap() as u16;
    record(ck,ev,"new app process confirms prior app exit removed owned tree",gone(port,owned).await,format!("pids={owned}; port={port}"));
    record(ck,ev,"exit/restart marker proves no automatic reexecution",fs::read_to_string(exit["marker"].as_str().unwrap()).unwrap_or_default()==exit["marker_contents"].as_str().unwrap_or_default(),exit.to_string());
    let after=value(&ipc(app,"main","list_development_processes",json!({"taskId":exit["task"]})).await);
    record(ck,ev,"startup creates no additional process receipts",after.as_array().map_or(0,Vec::len)==before_count,format!("before={history}; after={after}; selection={}",before["selection"]));
}

pub(super) async fn run(app: &AppHandle, fx: &Fx, mut ck: Checks, mut ev: Vec<Value>, restarting: bool) {
    if restarting {
        restart(app,fx,&mut ck,&mut ev).await;
        finish(app,fx,ck,ev,"restart");
        return;
    }
    let mut rows=Vec::new();
    for engine in ENGINES {rows.push(per_engine(app,fx,&mut ck,&mut ev,engine).await);}
    pty_and_switch(app,&mut ck,&mut ev,&rows[0],&rows[1]).await;
    select_row(app,&mut ck,&mut ev,&rows[0]).await;
    service_boundaries(app,&mut ck,&mut ev,&rows[0]).await;
    verification_boundaries(app,&mut ck,&mut ev,&rows[0]).await;
    account_cleanup(app,&mut ck,&mut ev,&rows[0]).await;
    select_row(app,&mut ck,&mut ev,&rows[0]).await;
    let port=free_port();let path=PathBuf::from(rows[0]["path"].as_str().unwrap());let task=rows[0]["task"].as_str().unwrap().to_owned();
    let exit_service=ready_service(app,&mut ck,&mut ev,&task,port,"app-exit","app-exit-response").await;
    let mut owned=pids(&path.join("app-exit.pids.json"));
    let exit_terminal=input(&task,"terminal","/bin/sh",vec!["-c".into(),"printf '%s' \"$$\" > app-exit-pty.marker; printf 'APP_EXIT_PTY_READY\\n'; sleep 60".into()]);start(app,&mut ck,&mut ev,&exit_terminal).await;
    let live_terminal=wait_process(app,exit_terminal["id"].as_str().unwrap(),|v|v["output"].as_str().is_some_and(|s|s.contains("APP_EXIT_PTY_READY"))).await;
    if let Ok(pid)=fs::read_to_string(path.join("app-exit-pty.marker")) {if let Ok(pid)=pid.parse::<i64>() {if let Some(rows)=owned.as_array_mut() {rows.push(json!(pid));}}}
    record(&mut ck,&mut ev,"app exit begins with live owned service and PTY",exit_service["status"]=="ready"&&live_terminal["status"]=="running"&&owned.as_array().is_some_and(|p|p.len()>=3&&p.iter().all(|pid|pid.as_i64().is_some_and(alive))),format!("service={exit_service}; terminal={live_terminal}; pids={owned}"));
    for row in &mut rows {
        let runs=value(&ipc(app,"main","list_development_runs",json!({"taskId":row["task"]})).await);
        row["history_ids"]=json!(runs.as_array().into_iter().flatten().map(|r|r["id"].clone()).collect::<Vec<_>>());
    }
    let marker=path.join("app-exit.marker");
    write_json(fx.root.join("stage3-state.json"),&json!({"engines":rows,"exit":{"id":exit_service["id"],"terminal_id":exit_terminal["id"],"task":task,"port":port,"pids":owned,"marker":marker,"marker_contents":fs::read_to_string(&marker).unwrap_or_default()}}));
    // Do not pre-shutdown here: ExitRequested must exercise the application's owned
    // process cleanup path while these processes are still live.
    finish(app,fx,ck,ev,"first");
}

// Focused production-dist/WKWebView/PTY acceptance. This phase never opens an Agent
// session, executes a Provider request, or replaces a production invoke result.
// DGOS_PROJECT_ACCEPTANCE_PHASE=terminal-activity DGOS_PROJECT_ACCEPTANCE_ROOT=/private/tmp/<fresh>
// cargo test -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture
const TERMINAL_ACTIVITY_PROBE: &str = r#"
if(window.__terminalActivity)throw Error('probe already installed');
// Tauri's invoke property is non-writable. Observe its real fetch transport instead.
const original=window.fetch;
const p={events:[],marks:[],original};window.__terminalActivity=p;
window.fetch=function(url,options,...rest){
 const command=String(url).split('/').filter(Boolean).at(-1);
 const watched=['start_development_process','write_development_terminal','resize_development_terminal','stop_development_process','read_development_process'].includes(command);
 const event=watched?{command,args:JSON.parse(options?.body||'{}'),at:performance.now(),phase:p.phase||'setup'}:null;
 if(event)p.events.push(event);
 return original.call(this,url,options,...rest).then(response=>{if(event){event.done=performance.now();event.ok=response.headers.get('Tauri-Response')==='ok';}return response;},error=>{if(event){event.done=performance.now();event.ok=false;}throw error;});
};
p.mark=name=>{p.phase=name;const mark={name,index:p.events.length,at:performance.now()};p.marks.push(mark);return mark;};
p.send=(line,focus=true)=>{
 const textarea=document.querySelector('[data-testid="dev-terminal"] .xterm-helper-textarea');
 if(!textarea)throw Error('real xterm textarea missing');
 if(focus)textarea.focus();
 for(const key of line){const code=key.charCodeAt(0);textarea.dispatchEvent(new KeyboardEvent('keydown',{key,keyCode:0,which:0,bubbles:true}));textarea.dispatchEvent(new KeyboardEvent('keypress',{key,charCode:code,keyCode:code,which:code,bubbles:true}));textarea.dispatchEvent(new KeyboardEvent('keyup',{key,keyCode:0,which:0,bubbles:true}));}
 textarea.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',code:'Enter',keyCode:13,which:13,bubbles:true}));
 textarea.dispatchEvent(new KeyboardEvent('keyup',{key:'Enter',code:'Enter',keyCode:13,which:13,bubbles:true}));
};
return {ok:true};
"#;

async fn activity_input(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, line: &str) -> bool {
    let reply=ui(app,json!({"line":line}),r#"const p=window.__terminalActivity;const index=p.events.length;p.send(A.line);const writes=await until(()=>{const ws=p.events.slice(index).filter(e=>e.command==='write_development_terminal');return ws.map(e=>e.args.input.data).join('')===A.line+'\r'&&ws.every(e=>e.ok)&&ws;});return {ok:!!writes};"#).await;
    record(ck,ev,"real xterm textarea keyboard input",ui_ok(&reply),reply.to_string());
    ui_ok(&reply)
}

async fn activity_show(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, id: &str, needle: &str) -> bool {
    let selected=tab(app,"develop").await;
    let reply=ui(app,json!({"id":id,"needle":needle}),r#"
const row=await need('dev-process-history-'+A.id);row.click();
// xterm pauses offscreen rendering; its accessibility rows can still describe
// the pre-reflow viewport. Observe the actual accessible rows after revealing it.
const mounted=await until(()=>tid('dev-terminal'));mounted?.scrollIntoView({block:'center'});
const ticks=rows=>rows.flatMap(row=>{const m=(row.textContent||'').match(/^TICK:(\d+)\s*$/);return m?[Number(m[1])]:[];});
let layers=null;
const terminal=await until(()=>{const t=tid('dev-terminal');const tree=t?.querySelector('.xterm-accessibility-tree');const text=tree?.innerText;const observed=/^TICK:\d+\s*$/.test(A.needle)?text?.split('\n').some(line=>line.trim()===A.needle.trim()):text?.includes(A.needle);const accessible=[...(tree?.children||[])];const rendered=[...(t?.querySelectorAll('.xterm-rows > div')||[])];layers={accessible:ticks(accessible),rendered:ticks(rendered)};return observed&&accessible.length&&rendered.length&&JSON.stringify(layers.accessible)===JSON.stringify(layers.rendered)&&t;});
return {ok:!!terminal,layers,focused:document.activeElement?.className,text:terminal?.querySelector('.xterm-accessibility-tree')?.innerText.slice(-1500)};
"#).await;
    record(ck,ev,"same PTY history renders after activation",ui_ok(&selected)&&ui_ok(&reply),reply.to_string());
    ui_ok(&selected)&&ui_ok(&reply)
}

async fn activity_start_ui(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, task: &str) -> String {
    let before=value(&ipc(app,"main","list_development_processes",json!({"taskId":task})).await);
    // A DOM click works even when WKWebView rendering is suspended. Establish
    // the visible desktop scenario before requiring its first animation frame.
    if let Some(window)=app.get_webview_window("main") {let _=window.show();let _=window.set_focus();}
    let reply=ui(app,json!({}),r#"
const foreground=await until(()=>document.visibilityState==='visible'&&document.hasFocus());if(!foreground)throw Error('terminal desktop window is not foreground');
const toggle=await need('dev-toggle-runtime');if(toggle.getAttribute('aria-expanded')!=='true')toggle.click();
const processes=await need('dev-runtime-processes');processes.click();
const configuration=await need('dev-runtime-configuration');if(!configuration.open)configuration.querySelector('summary').click();
const kind=await need('dev-process-kind');kind.value='terminal';kind.dispatchEvent(new Event('change',{bubbles:true}));
await fill('dev-process-program','/bin/sh');const trust=await need('dev-process-trust');if(!trust.checked)trust.click();
const start=await until(()=>{const b=tid('dev-process-start');return b&&!b.disabled&&b;});if(!start)throw Error('terminal start disabled');
const p=window.__terminalActivity;const index=p.events.length;start.click();
const request=await until(()=>p.events.slice(index).find(e=>e.command==='start_development_process'));if(!request)throw Error('real start invocation missing');
const chain=[];let previous='';const inspect=()=>{const t=tid('dev-terminal');const r=t?.getBoundingClientRect();const state={ack:request.ok===true,selected:tid('dev-process-history-'+request.args.input.id)?.getAttribute('aria-pressed')==='true',mounted:!!t,textarea:!!t?.querySelector('.xterm-helper-textarea'),width:t?.clientWidth,height:t?.clientHeight,top:r?.top,bottom:r?.bottom,visibility:document.visibilityState,focus:document.hasFocus(),enabled:!start.disabled,sized:p.events.slice(index).some(e=>e.command==='resize_development_terminal'&&e.args.input.id===request.args.input.id&&e.ok)};const key=JSON.stringify(state);if(key!==previous){chain.push({at:performance.now(),...state});previous=key;}return state;};
let frameAt=null;requestAnimationFrame(()=>{frameAt=performance.now();});
const ready=await until(()=>{const s=inspect();return s.ack&&s.enabled&&s.selected&&s.sized&&s.textarea;});
return {ok:!!ready,chain,frameAt,at:performance.now()};
"#).await;
    record(ck,ev,"start real /bin/sh PTY through production UI",ui_ok(&reply),reply.to_string());
    let after=value(&ipc(app,"main","list_development_processes",json!({"taskId":task})).await);
    let rows:Vec<&Value>=after.as_array().into_iter().flatten().filter(|row|!before.as_array().into_iter().flatten().any(|old|old["id"]==row["id"])).collect();
    record(ck,ev,"one UI action creates exactly one terminal receipt",rows.len()==1&&rows[0]["kind"]=="terminal"&&rows[0]["program"]=="/bin/sh",after.to_string());
    rows.first().and_then(|row|row["id"].as_str()).unwrap_or_default().to_owned()
}

async fn activity_mark(app: &AppHandle, name: &str) {
    ui(app,json!({"name":name}),r#"return {ok:true,mark:window.__terminalActivity.mark(A.name)};"#).await;
}

async fn activity_window(app: &AppHandle, width: f64) -> bool {
    app.get_webview_window("main").is_some_and(|window|window.set_size(tauri::LogicalSize::new(width,1000.0)).is_ok())
}

async fn activity_interval(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, quiet: bool) {
    let reply=ui(app,json!({"name":name,"quiet":quiet}),r#"
const p=window.__terminalActivity;const mark=p.marks.findLast(m=>m.name===A.name);if(!mark)throw Error('interval mark missing');
const events=p.events.slice(mark.index);const resize=events.filter(e=>e.command==='resize_development_terminal');
const writes=events.filter(e=>e.command==='write_development_terminal');const starts=events.filter(e=>e.command==='start_development_process');
const valid=resize.every(e=>e.args.input.rows>=1&&e.args.input.rows<=1000&&e.args.input.cols>=1&&e.args.input.cols<=1000&&e.ok===true);
return {ok:!starts.length&&!writes.length&&valid&&(A.quiet?!resize.length:resize.length>0),from:mark.at,to:performance.now(),resize,writes:writes.length,starts:starts.length,detailReads:events.filter(e=>e.command==='read_development_process').length};
"#).await;
    record(ck,ev,&format!("measured {name}: {} resize, no duplicate start/input",if quiet {"zero"} else {"valid real"}),ui_ok(&reply),reply.to_string());
}

// Dependency diagnostic only; intentionally reports tree mismatches as failures.
// DGOS_TERMINAL_ACCESSIBILITY_ONLY=1 DGOS_PROJECT_ACCEPTANCE_PHASE=terminal-activity
// DGOS_PROJECT_ACCEPTANCE_ROOT=/private/tmp/<fresh> cargo test --offline -p dgos \
// --features webview-smoke --test webview_smoke_agents -- --nocapture
async fn terminal_accessibility(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, id: &str) {
    let first_write = ipc(app,"main","write_development_terminal",json!({"input":{"id":id,"data":"stty -echo; PS1=''; printf '\\033c'; printf '%0148d\\nMARK\\nTICK:1\\n' 0\n"}})).await;
    record(ck,ev,"focused accessibility writes once to real PTY",succeeded(&first_write),first_write.describe());
    let first = wait_process(app,id,|v|v["output"].as_str().is_some_and(|s|s.contains("\r\nTICK:1\r\n"))).await;
    let second_write = ipc(app,"main","write_development_terminal",json!({"input":{"id":id,"data":"printf 'TICK:2\\n'\n"}})).await;
    record(ck,ev,"focused accessibility appends once to same real PTY",succeeded(&second_write),second_write.describe());
    let second = wait_process(app,id,|v|v["output"].as_str().is_some_and(|s|s.contains("\r\nTICK:2\r\n"))).await;
    let prefix = first["output"].as_str().zip(second["output"].as_str()).is_some_and(|(a,b)|b.starts_with(a));
    record(ck,ev,"real host snapshots retain exact output prefix",prefix,format!("first={first}; second={second}"));
    let root=Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("repository root");
    let source=fs::read_to_string(root.join("node_modules/@xterm/xterm/lib/xterm.js"));
    let css=fs::read_to_string(root.join("node_modules/@xterm/xterm/css/xterm.css"));
    let package=fs::read_to_string(root.join("node_modules/@xterm/xterm/package.json"));
    let probe=fs::read_to_string(root.join("tests/fixtures/terminal-accessibility-probe.js"));
    match (source,css,package,probe) {
        (Ok(source),Ok(css),Ok(package),Ok(probe)) => {
            let installed:Value=serde_json::from_str(&package).unwrap_or(Value::Null);
            let body=format!(r#"
const style=document.createElement('style');style.textContent=A.css;document.head.appendChild(style);
const previousTerminal=globalThis.Terminal;const previousProbe=globalThis.runTerminalAccessibilityProbe;
try {{
{source}
;
{probe}
const result=await globalThis.runTerminalAccessibilityProbe({{Terminal:globalThis.Terminal,snapshots:A.snapshots}});
const production=tid('dev-terminal');
return {{ok:true,result,production:{{scope:'production DOM only; public instance and actual application write chunks are unavailable',renderer:[...(production?.querySelectorAll('.xterm-rows > div')||[])].map(e=>e.textContent),tree:[...(production?.querySelectorAll('.xterm-accessibility-tree > div')||[])].map(e=>e.textContent)}}}};
}} finally {{style.remove();if(previousProbe===undefined)delete globalThis.runTerminalAccessibilityProbe;else globalThis.runTerminalAccessibilityProbe=previousProbe;if(previousTerminal===undefined)delete globalThis.Terminal;else globalThis.Terminal=previousTerminal;}}
"#);
            let reply=ui(app,json!({"css":css,"snapshots":[first["output"],second["output"]]}),&body).await;
            write_json(fx.root.join("terminal-accessibility-layers.json"),&json!({"installed_version":installed["version"],"host":{"first":first,"second":second},"probe":reply}));
            record(ck,ev,"focused WK dependency probe completes without dropping host bytes",ui_ok(&reply)&&reply["value"]["result"]["chunksEqualLatestSnapshot"]==true,"terminal-accessibility-layers.json");
            record(ck,ev,"offscreen accessibility tree never invents duplicate output ticks",ui_ok(&reply)&&reply["value"]["result"]["treeIntroducedDuplicateTicks"]==false,"terminal-accessibility-layers.json; fixed-interval dependency observation, not a product fix or real screen-reader observation");
        }
        _ => record(ck,ev,"focused accessibility dependency sources are available",false,"npm-installed xterm UMD/CSS/package and tests/fixtures/terminal-accessibility-probe.js are required"),
    }
    let cleanup=ui(app,json!({}),r#"const p=window.__terminalActivity;if(p)window.fetch=p.original;return {ok:true};"#).await;
    record(ck,ev,"focused accessibility restores invoke observation",ui_ok(&cleanup),cleanup.to_string());
    stop(app,ck,ev,id).await;
}

pub(super) async fn terminal_activity(app: &AppHandle, fx: &Fx, mut ck: Checks, mut ev: Vec<Value>) {
    let path=fx.root.join("terminal-activity");make_repo(&path);
    if !ui_import_project(app,&mut ck,&mut ev,"Terminal activity",&path).await || !ui_new_task(app,&mut ck,&mut ev,"Terminal activity isolated PTY").await {
        finish(app,fx,ck,ev,"terminal-activity");return;
    }
    if !ui_select_task(app,&mut ck,&mut ev,"Terminal activity isolated PTY").await { finish(app,fx,ck,ev,"terminal-activity");return; }
    let selected=value(&ipc(app,"main","read_development_workspace",json!({})).await)["selection"].clone();
    let task=selected["task_id"].as_str().unwrap_or_default();
    let runtime=ui(app,json!({}),r#"const toggle=await need('dev-toggle-runtime');if(toggle.getAttribute('aria-expanded')!=='true')toggle.click();const processes=await need('dev-runtime-processes');processes.click();return {ok:!!(await until(()=>tid('dev-development-runtime')))};"#).await;
    record(&mut ck,&mut ev,"open current runtime panel through production controls",ui_ok(&runtime),runtime.to_string());
    let probe=ui(app,json!({}),TERMINAL_ACTIVITY_PROBE).await;
    record(&mut ck,&mut ev,"count-only invoke probe preserves all real host calls",ui_ok(&probe),probe.to_string());
    let id=activity_start_ui(app,&mut ck,&mut ev,task).await;
    if id.is_empty() {finish(app,fx,ck,ev,"terminal-activity");return;}
    if std::env::var_os("DGOS_TERMINAL_ACCESSIBILITY_ONLY").is_some() {
        terminal_accessibility(app,fx,&mut ck,&mut ev,&id).await;
        finish(app,fx,ck,ev,"terminal-activity");return;
    }
    let initial_size=ui(app,json!({}),r#"const e=window.__terminalActivity.events.find(e=>e.command==='resize_development_terminal'&&e.ok);return {ok:!!e,rows:e?.args.input.rows,cols:e?.args.input.cols};"#).await;
    activity_input(app,&mut ck,&mut ev,"stty -echo; printf 'INITIAL_SIZE:'; stty size").await;
    let initial_view=wait_process(app,&id,|v|v["output"].as_str().is_some_and(|s|s.contains(&format!("INITIAL_SIZE:{} {}",initial_size["value"]["rows"],initial_size["value"]["cols"])))).await;
    record(&mut ck,&mut ev,"first fit reaches actual PTY stty dimensions",ui_ok(&initial_size)&&initial_view["output"].as_str().is_some_and(|s|s.contains(&format!("INITIAL_SIZE:{} {}",initial_size["value"]["rows"],initial_size["value"]["cols"]))),initial_size.to_string());
    let nonce=token("ACTIVITY");
    activity_input(app,&mut ck,&mut ev,&format!("stty -echo; PS1=''; printf '%s\\n' \"$$\" > shell.pid; printf 'once\\n' >> input.marker; printf '%s%s\\n' '{}' '{}'; (i=0; while [ ! -f output.stop ]; do i=$((i+1)); printf 'TICK:%s\\n' \"$i\"; sleep 1; done) &",&nonce[..nonce.len()/2],&nonce[nonce.len()/2..])).await;
    let baseline=wait_process(app,&id,|view|view["output"].as_str().is_some_and(|s|s.contains(&nonce)&&s.contains("TICK:2"))).await;
    let pid=fs::read_to_string(path.join("shell.pid")).unwrap_or_default();
    record(&mut ck,&mut ev,"xterm input executes once in live owned shell",baseline["status"]=="running"&&pid.trim().parse::<i64>().is_ok_and(alive)&&fs::read_to_string(path.join("input.marker")).unwrap_or_default()=="once\n",format!("id={id}; pid={pid}; output={}",baseline["output"]));
    activity_show(app,&mut ck,&mut ev,&id,&nonce).await;
    activity_mark(app,"active-resize").await;
    record(&mut ck,&mut ev,"native active window resize",activity_window(app,1200.0).await,"1200x1000");
    tokio::time::sleep(Duration::from_secs(2)).await;
    activity_interval(app,&mut ck,&mut ev,"active-resize",false).await;
    activity_mark(app,"active-idle").await;
    tokio::time::sleep(Duration::from_millis(4600)).await;
    activity_interval(app,&mut ck,&mut ev,"active-idle",true).await;
    for cycle in 0..3 {
        let away=ui(app,json!({}),r#"const a=document.querySelector('a[href="/workbench"]');if(!a)throw Error('AI navigation absent');a.click();return {ok:!!(await until(()=>document.querySelector('[data-workspace-surface="cli-agent"]')?.hidden))};"#).await;
        record(&mut ck,&mut ev,"switch away keeps terminal surface mounted and hidden",ui_ok(&away),away.to_string());
        // Let an already-dispatched resize settle before measuring strictly hidden work.
        tokio::time::sleep(Duration::from_millis(250)).await;
        let interval=format!("hidden-{cycle}");activity_mark(app,&interval).await;
        record(&mut ck,&mut ev,"native hidden window resize",activity_window(app,[1000.0,1100.0,900.0][cycle]).await,cycle);
        tokio::time::sleep(Duration::from_millis(4600)).await;
        activity_interval(app,&mut ck,&mut ev,&interval,true).await;
        let hidden=process(app,&id).await;
        record(&mut ck,&mut ev,"hidden PTY continues producing host output with same identity",hidden["id"]==baseline["id"]&&hidden["started_at"]==baseline["started_at"]&&hidden["status"]=="running"&&hidden["output"].as_str().map_or(0,str::len)>baseline["output"].as_str().map_or(0,str::len)&&fs::read_to_string(path.join("shell.pid")).unwrap_or_default()==pid,format!("id={}; bytes={}; pid={pid}",hidden["id"],hidden["output"].as_str().map_or(0,str::len)));
        let rejected=ui(app,json!({}),r#"const p=window.__terminalActivity;const index=p.events.length;p.send('printf forbidden >> forbidden.marker',false);await new Promise(r=>setTimeout(r,100));return {ok:!p.events.slice(index).some(e=>e.command==='write_development_terminal')};"#).await;
        record(&mut ck,&mut ev,"hidden terminal rejects adversarial textarea input",ui_ok(&rejected)&&!path.join("forbidden.marker").exists(),rejected.to_string());
        let back=ui(app,json!({}),r#"const tab=document.querySelector('[data-workspace-tab="cli-agent"]');if(!tab)throw Error('Agent tab absent');tab.click();return {ok:!!(await until(()=>!document.querySelector('[data-workspace-surface="cli-agent"]')?.hidden))};"#).await;
        record(&mut ck,&mut ev,"restore terminal surface through workspace tab",ui_ok(&back),back.to_string());
        let latest=hidden["output"].as_str().unwrap_or_default().lines().rev().find(|line|line.starts_with("TICK:")).unwrap_or("TICK:").to_owned();
        activity_show(app,&mut ck,&mut ev,&id,&latest).await;
        let order=ui(app,json!({}),r#"const screen=tid('dev-terminal')?.querySelector('.xterm-accessibility-tree');const text=screen?.innerText||'';const ticks=[...text.matchAll(/^TICK:(\d+)\s*$/gm)].map(m=>Number(m[1]));return {ok:!!screen&&ticks.length>=2&&ticks.every((n,i)=>!i||n===ticks[i-1]+1),ticks};"#).await;
        record(&mut ck,&mut ev,"restored xterm exposes ordered nonduplicated output ticks",ui_ok(&order),order.to_string());
        if cycle==2 {
            ui(app,json!({}),r#"tid('dev-terminal')?.scrollIntoView({block:'center'});return {ok:true};"#).await;
            if let Some(window)=app.get_webview_window("main") {let _=window.set_focus();}
            tokio::time::sleep(Duration::from_millis(300)).await;
            let capture=Command::new("/usr/sbin/screencapture").arg("-x").arg(fx.root.join("terminal-activity-restored.png")).status();
            record(&mut ck,&mut ev,"capture restored real WKWebView terminal",capture.is_ok_and(|s|s.success()),"terminal-activity-restored.png");
        }
    }
    let dimensions=ui(app,json!({"id":id}),r#"
const p=window.__terminalActivity;const mark=p.marks.findLast(m=>m.name==='hidden-2');
const e=await until(()=>p.events.slice(mark.index).filter(e=>e.command==='resize_development_terminal'&&e.args.input.id===A.id&&e.ok).at(-1));
const t=tid('dev-terminal');return {ok:!!e&&e.args.input.rows>0&&e.args.input.cols>0,request:e?.args.input,width:t?.clientWidth,height:t?.clientHeight};
"#).await;
    let rows=dimensions["value"]["request"]["rows"].as_u64().unwrap_or(0);let cols=dimensions["value"]["request"]["cols"].as_u64().unwrap_or(0);
    activity_input(app,&mut ck,&mut ev,"printf 'SIZE:'; stty size").await;
    let sized=wait_process(app,&id,|v|v["output"].as_str().is_some_and(|s|s.contains(&format!("SIZE:{rows} {cols}")))).await;
    record(&mut ck,&mut ev,"restored fit reaches actual PTY stty dimensions",ui_ok(&dimensions)&&rows>0&&cols>0&&sized["output"].as_str().is_some_and(|s|s.contains(&format!("SIZE:{rows} {cols}"))),dimensions.to_string());
    let covered=ui(app,json!({}),r#"document.querySelector('[data-workspace-tab="cli-agent"]').focus();window.dispatchEvent(new KeyboardEvent('keydown',{key:'k',metaKey:true,bubbles:true}));return {ok:!!(await until(()=>document.querySelector('.workspace-command')))&&!!tid('dev-terminal')&&!document.querySelector('[data-workspace-surface="cli-agent"]').hidden};"#).await;
    record(&mut ck,&mut ev,"command overlay covers still-visible terminal",ui_ok(&covered),covered.to_string());
    tokio::time::sleep(Duration::from_millis(250)).await;activity_mark(app,"covered").await;
    let blocked=ui(app,json!({}),r#"window.__terminalActivity.send("printf forbidden >> forbidden.marker",false);return {ok:true};"#).await;
    record(&mut ck,&mut ev,"covered terminal receives adversarial textarea keys",ui_ok(&blocked),blocked.to_string());
    record(&mut ck,&mut ev,"native covered window resize",activity_window(app,1400.0).await,"1400x1000");
    tokio::time::sleep(Duration::from_millis(3300)).await;
    activity_interval(app,&mut ck,&mut ev,"covered",false).await;
    record(&mut ck,&mut ev,"covered terminal rejects queued shell input",!path.join("forbidden.marker").exists(),"no forbidden.marker");
    let uncovered=ui(app,json!({}),r#"window.dispatchEvent(new KeyboardEvent('keydown',{key:'k',metaKey:true,bubbles:true}));return {ok:!!(await until(()=>!document.querySelector('.workspace-command')))&&!document.activeElement?.classList.contains('xterm-helper-textarea')};"#).await;
    record(&mut ck,&mut ev,"overlay restore never focuses terminal unsolicited",ui_ok(&uncovered),uncovered.to_string());
    activity_input(app,&mut ck,&mut ev,"printf 'COVER_SIZE:'; stty size").await;
    let cover_size=ui(app,json!({"id":id}),r#"const p=window.__terminalActivity;const m=p.marks.findLast(m=>m.name==='covered');const e=p.events.slice(m.index).filter(e=>e.command==='resize_development_terminal'&&e.args.input.id===A.id&&e.ok).at(-1);return {ok:!!e,request:e?.args.input};"#).await;
    let cover_rows=cover_size["value"]["request"]["rows"].as_u64().unwrap_or(0);let cover_cols=cover_size["value"]["request"]["cols"].as_u64().unwrap_or(0);
    let cover_view=wait_process(app,&id,|v|v["output"].as_str().is_some_and(|s|s.contains(&format!("COVER_SIZE:{cover_rows} {cover_cols}")))).await;
    record(&mut ck,&mut ev,"covered visible fit reaches actual PTY dimensions",ui_ok(&cover_size)&&cover_view["output"].as_str().is_some_and(|s|s.contains(&format!("COVER_SIZE:{cover_rows} {cover_cols}"))),cover_size.to_string());
    let zero=ui(app,json!({}),r#"const t=await need('dev-terminal');window.__terminalActivity.originalStyle=t.getAttribute('style');t.style.setProperty('height','0px','important');t.style.setProperty('min-height','0px','important');t.style.setProperty('padding','0px','important');t.style.setProperty('border','0px','important');return {ok:true};"#).await;
    record(&mut ck,&mut ev,"collapse real terminal to zero height",ui_ok(&zero),zero.to_string());
    tokio::time::sleep(Duration::from_millis(250)).await;activity_mark(app,"zero-height").await;
    activity_window(app,1500.0).await;tokio::time::sleep(Duration::from_millis(1800)).await;
    activity_interval(app,&mut ck,&mut ev,"zero-height",true).await;
    let uncollapse=ui(app,json!({}),r#"const t=await need('dev-terminal');const s=window.__terminalActivity.originalStyle;if(s===null)t.removeAttribute('style');else t.setAttribute('style',s);return {ok:true};"#).await;
    record(&mut ck,&mut ev,"restore terminal container geometry",ui_ok(&uncollapse),uncollapse.to_string());
    let snapshot=ui(app,json!({}),r#"return {ok:true,events:window.__terminalActivity.events,marks:window.__terminalActivity.marks};"#).await;
    write_json(fx.root.join("terminal-activity-ipc-before-reopen.json"),&snapshot);
    let files=tab(app,"files").await;
    record(&mut ck,&mut ev,"internal files tab unmounts terminal",ui_ok(&files)&&ui_ok(&ui(app,json!({}),r#"return {ok:!tid('dev-terminal')};"#).await),files.to_string());
    activity_mark(app,"internal-files").await;tokio::time::sleep(Duration::from_millis(1800)).await;
    activity_interval(app,&mut ck,&mut ev,"internal-files",true).await;
    activity_show(app,&mut ck,&mut ev,&id,"TICK:").await;
    activity_input(app,&mut ck,&mut ev,"touch output.stop; wait; printf 'NORMAL_EXIT_HISTORY\\n'; exit 7").await;
    let exited=terminal(app,&id).await;
    record(&mut ck,&mut ev,"normal shell exit retains actual output and exit code",exited["exit_code"]==7&&exited["output"].as_str().is_some_and(|s|s.contains("NORMAL_EXIT_HISTORY"))&&fs::read_to_string(path.join("input.marker")).unwrap_or_default()=="once\n",format!("id={id}; exit={}",exited["exit_code"]));
    activity_show(app,&mut ck,&mut ev,&id,"NORMAL_EXIT_HISTORY").await;
    let stopped_id=activity_start_ui(app,&mut ck,&mut ev,task).await;
    if !stopped_id.is_empty() {
        activity_input(app,&mut ck,&mut ev,"stty -echo; printf 'STOP_HISTORY\\n'").await;
        wait_process(app,&stopped_id,|v|v["output"].as_str().is_some_and(|s|s.contains("STOP_HISTORY"))).await;
        let stopped=ui(app,json!({}),r#"const b=await need('dev-process-stop');if(b.disabled)throw Error('stop disabled');b.click();return {ok:!!(await until(()=>tid('dev-process-stop')?.disabled))};"#).await;
        let stopped_view=terminal(app,&stopped_id).await;
        record(&mut ck,&mut ev,"explicit UI stop keeps terminal history",ui_ok(&stopped)&&stopped_view["status"]=="cancelled"&&stopped_view["output"].as_str().is_some_and(|s|s.contains("STOP_HISTORY")),stopped.to_string());
    }
    let capacity_id=activity_start_ui(app,&mut ck,&mut ev,task).await;
    if !capacity_id.is_empty() {
        activity_input(app,&mut ck,&mut ev,"stty -echo; sleep 2; printf 'CAPTURE_PREFIX\\n'; node -e 'let s=\"\";for(let i=0;i<90000;i++)s+=\"CAPACITY:\"+String(i).padStart(8,\"0\")+\"\\n\";process.stdout.write(s)'; touch capacity.done; exit 0").await;
        let hidden_capacity=tab(app,"files").await;
        record(&mut ck,&mut ev,"large PTY output starts after files unmounts terminal",ui_ok(&hidden_capacity)&&ui_ok(&ui(app,json!({}),r#"return {ok:!tid('dev-terminal')};"#).await),hidden_capacity.to_string());
        let bounded=terminal(app,&capacity_id).await;
        let bytes=bounded["output"].as_str().map_or(0,str::len);
        record(&mut ck,&mut ev,"actual PTY output exceeds and preserves 1MiB capture prefix",bounded["exit_code"]==0&&bounded["truncated"]==true&&bytes<=1024*1024&&bytes>=1024*1024-256&&bounded["output"].as_str().is_some_and(|s|s.contains("CAPTURE_PREFIX")&&!s.contains("CAPACITY:00089999"))&&path.join("capacity.done").exists(),format!("id={capacity_id}; bytes={bytes}; truncated={}; exit={}",bounded["truncated"],bounded["exit_code"]));
        activity_show(app,&mut ck,&mut ev,&capacity_id,"CAPACITY:").await;
        let sequence:Vec<u64>=bounded["output"].as_str().unwrap_or_default().lines().filter_map(|line|line.strip_prefix("CAPACITY:").and_then(|n|n.trim().parse().ok())).collect();
        record(&mut ck,&mut ev,"hidden capacity output retains original contiguous prefix order",sequence.first()==Some(&0)&&sequence.len()>1000&&sequence.windows(2).all(|pair|pair[1]==pair[0]+1),format!("lines={}; first={:?}; last={:?}",sequence.len(),sequence.first(),sequence.last()));
    }
    // Surface close/reopen deliberately remains a real UI action: preserve the known
    // module-reopen timeout risk rather than route around it with a host-only assertion.
    let reopen=ui(app,json!({}),r#"
const t=document.querySelector('[data-workspace-tab="cli-agent"]');if(!t)throw Error('terminal workspace tab missing');t.parentElement.querySelector('.workspace-tab-close').click();
if(!await until(()=>!document.querySelector('[data-workspace-tab="cli-agent"]')))throw Error('workspace close did not complete');
const command=$$('.workspace-tab-actions button').find(b=>b.querySelector('svg')&&/命令|command/i.test(b.getAttribute('aria-label')||''));
if(command)command.click();else window.dispatchEvent(new KeyboardEvent('keydown',{key:'k',metaKey:true,bubbles:true}));
const target=await until(()=>$$('.workspace-command-results button').find(b=>/Agent|智能体/.test(txt(b))));if(!target)throw Error('reopen Agent command missing');target.click();
return {ok:!!(await until(()=>tid('dev-project-list')))};
"#).await;
    record(&mut ck,&mut ev,"workspace close and explicit reopen (known timeout boundary)",ui_ok(&reopen),reopen.to_string());
    if ui_ok(&reopen) {activity_show(app,&mut ck,&mut ev,&id,"NORMAL_EXIT_HISTORY").await;}
    let history=value(&ipc(app,"main","list_development_processes",json!({"taskId":task})).await);
    record(&mut ck,&mut ev,"reopen retains all receipts without restarting PTYs",history.as_array().is_some_and(|ps|ps.len()==3&&ps.iter().all(|p|p["finished_at"].is_number())&&ps.iter().any(|p|p["id"]==id)),history.to_string());
    let counts=ui(app,json!({}),r#"const p=window.__terminalActivity;const starts=p.events.filter(e=>e.command==='start_development_process');return {ok:starts.length===3&&new Set(starts.map(e=>e.args.input.id)).size===3,starts:starts.map(e=>({id:e.args.input.id,at:e.at,ok:e.ok}))};"#).await;
    record(&mut ck,&mut ev,"all switches and reopen create no additional starts",ui_ok(&counts),counts.to_string());
    let trace=ui(app,json!({}),r#"const p=window.__terminalActivity;const trace={events:p.events,marks:p.marks};window.fetch=p.original;return {ok:true,trace};"#).await;
    write_json(fx.root.join("terminal-activity-ipc.json"),&trace);
    record(&mut ck,&mut ev,"persist real invocation timing and measurement intervals",ui_ok(&trace),"terminal-activity-ipc.json");
    finish(app,fx,ck,ev,"terminal-activity");
}

// No model account or CLI is involved: these are production IPC invocations from WKWebView.
pub(super) async fn worktree_runtime(app: &AppHandle, fx: &Fx, mut ck: Checks, mut ev: Vec<Value>) {
    let source=fx.root.join("runtime-original");make_repo(&source);
    fs::write(source.join("package.json"),r#"{"scripts":{"original-only":"node -e \"throw Error('wrong original workspace')\""}}"#).unwrap();
    assert!(git(&source,&["add","."]));
    assert!(git(&source,&["-c","user.name=dgos","-c","user.email=dgos@example.invalid","commit","-q","-m","original command fixture"]));
    if !ui_import_project(app,&mut ck,&mut ev,"Runtime worktree acceptance",&source).await || !ui_new_task(app,&mut ck,&mut ev,"Run commands in isolated task worktree").await {
        finish(app,fx,ck,ev,"worktree-runtime");return;
    }
    ui_select_task(app,&mut ck,&mut ev,"Run commands in isolated task worktree").await;
    let mut selected=Value::Null;
    for _ in 0..100 {
        selected=value(&ipc(app,"main","read_development_workspace",json!({})).await)["selection"].clone();
        if selected["task_id"].is_string(){break;}
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    record(&mut ck,&mut ev,"task selection committed before runtime IPC",selected["task_id"].is_string(),selected.to_string());
    let task=selected["task_id"].as_str().unwrap_or_default();let project=selected["project_id"].as_str().unwrap_or_default();
    let original=value(&ipc(app,"main","read_development_commands",json!({"taskId":task})).await);
    record(&mut ck,&mut ev,"original task command catalog reads original package",original["discovered"].as_array().is_some_and(|rows|rows.iter().any(|r|r["id"]=="package:original-only")),original.to_string());
    let head=Command::new("git").arg("-C").arg(&source).args(["rev-parse","HEAD"]).output().unwrap();
    let head=String::from_utf8(head.stdout).unwrap();let target=fx.root.join("runtime-worktree");
    let created=ipc(app,"main","save_development_workspaces",json!({"input":{"request_id":uuid::Uuid::new_v4().to_string(),"task_id":task,"action":"create","path":target,"branch":"task/runtime-acceptance","base_commit":head.trim()}})).await;
    record(&mut ck,&mut ev,"production IPC creates and binds actual Git worktree",succeeded(&created)&&target.join(".git").is_file(),created.describe());
    if !succeeded(&created) {finish(app,fx,ck,ev,"worktree-runtime");return;}
    fs::write(target.join("package.json"),r#"{"scripts":{"worktree-only":"node -e \"require('fs').writeFileSync('npm-executed.marker',process.cwd());console.log('WORKTREE_NPM_EXECUTED')\""}}"#).unwrap();
    fs::write(target.join("Cargo.toml"),"[package]\nname = \"dgos-runtime-worktree-fixture\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[workspace]\n").unwrap();
    fs::create_dir(target.join("src")).unwrap();
    fs::write(target.join("src/lib.rs"),"#[test]\nfn actual_worktree_execution() { let path = std::env::current_dir().unwrap(); assert!(path.ends_with(\"runtime-worktree\")); std::fs::write(\"cargo-executed.marker\", path.to_str().unwrap()).unwrap(); println!(\"WORKTREE_CARGO_EXECUTED\"); }\n").unwrap();
    let catalog=value(&ipc(app,"main","read_development_commands",json!({"taskId":task})).await);
    let discovered=catalog["discovered"].as_array();
    record(&mut ck,&mut ev,"task discovery uses selected worktree npm and Cargo manifests",discovered.is_some_and(|rows|rows.iter().any(|r|r["id"]=="package:worktree-only")&&rows.iter().any(|r|r["id"]=="cargo:test")&&!rows.iter().any(|r|r["id"]=="package:original-only")),catalog.to_string());
    for id in ["package:worktree-only","cargo:test"] {
        if let Some(command)=discovered.and_then(|rows|rows.iter().find(|r|r["id"]==id)) {
            let mut request=input(task,"command",command["program"].as_str().unwrap(),command["args"].as_array().unwrap().iter().map(|a|a.as_str().unwrap().to_owned()).collect());
            if id=="cargo:test" {request["args"]=json!(["test","--offline","--","--nocapture"]);}
            start(app,&mut ck,&mut ev,&request).await;
            let result=terminal(app,request["id"].as_str().unwrap()).await;
            let marker=if id=="cargo:test" {"cargo-executed.marker"}else{"npm-executed.marker"};
            record(&mut ck,&mut ev,&format!("{id} executes real process in actual task worktree"),result["exit_code"]==0&&result["cwd"]==""&&fs::read_to_string(target.join(marker)).unwrap_or_default()==target.to_string_lossy()&&!source.join(marker).exists(),result.to_string());
        } else {record(&mut ck,&mut ev,&format!("{id} available for actual execution"),false,catalog.to_string());}
    }
    let capture=fx.root.join("editor-opened.txt");let script=fx.root.join("CaptureEditor.applescript");let editor=fx.root.join("CaptureEditor.app");
    fs::write(&script,format!(r#"on open openedItems
    set destination to POSIX file "{}"
    set stream to open for access destination with write permission
    repeat with openedItem in openedItems
        write ((POSIX path of openedItem) & linefeed) to stream starting at eof
    end repeat
    close access stream
end open
"#,capture.display())).unwrap();
    let compiled=Command::new("/usr/bin/osacompile").arg("-o").arg(&editor).arg(&script).output();
    record(&mut ck,&mut ev,"compile harmless local editor capture application",compiled.as_ref().is_ok_and(|o|o.status.success()),format!("{compiled:?}"));
    let open_args=json!({"input":{"project_id":project,"task_id":task,"target":"editor","editor":editor}});
    let opened=ipc(app,"main","open_development_external",open_args.clone()).await;
    for _ in 0..100 {if fs::read_to_string(&capture).is_ok_and(|text|text.ends_with('\n')){break;}tokio::time::sleep(Duration::from_millis(100)).await;}
    let captured=fs::read_to_string(&capture).unwrap_or_default();
    let actual=captured.lines().next().and_then(|p|fs::canonicalize(p).ok());
    record(&mut ck,&mut ev,"real macOS editor open event receives task worktree rather than original",succeeded(&opened)&&actual.as_ref()==Some(&target),format!("{}; captured={captured:?}",opened.describe()));
    let mismatched=ipc(app,"main","open_development_external",json!({"input":{"project_id":"unrelated-project","task_id":task,"target":"editor","editor":editor}})).await;
    record(&mut ck,&mut ev,"real opener IPC rejects mismatched task project",!succeeded(&mismatched)&&mismatched.describe().contains("task_not_found"),mismatched.describe());
    // A real running process holds Alice's lease until sign-out revokes it.
    let running=input(task,"command","/bin/sh",vec!["-c".into(),"printf 'LEASE_READY\\n'; sleep 120".into()]);
    start(app,&mut ck,&mut ev,&running).await;
    let running_id=running["id"].as_str().unwrap();
    let ready=wait_process(app,running_id,|v|v["output"].as_str().is_some_and(|s|s.contains("LEASE_READY"))).await;
    record(&mut ck,&mut ev,"actual process is running before lease revocation",ready["status"]=="running",ready.to_string());
    ui_sign_out(app,&mut ck,&mut ev,ALICE.0).await;
    for (command,args) in [("open_development_external",open_args.clone()),("read_development_commands",json!({"taskId":task})),("start_development_process",json!({"input":input(task,"command","/bin/sh",vec!["-c".into(),"touch forbidden.marker".into()])}))] {
        let denied=ipc(app,"main",command,args).await;
        record(&mut ck,&mut ev,&format!("signed-out real IPC rejects {command}"),!succeeded(&denied),denied.describe());
    }
    ui_sign_in(app,&mut ck,&mut ev,BOB).await;goto(app,"main","/tasks/agent").await;
    for (command,args) in [("open_development_external",open_args.clone()),("read_development_commands",json!({"taskId":task})),("save_development_commands",json!({"input":{"task_id":task,"commands":[]}})),("read_development_process",json!({"id":running_id}))] {
        let denied=ipc(app,"main",command,args).await;
        record(&mut ck,&mut ev,&format!("other owner real IPC rejects {command}"),!succeeded(&denied),denied.describe());
    }
    record(&mut ck,&mut ev,"denied IPC creates no editor events or command markers",fs::read_to_string(&capture).unwrap_or_default()==captured&&!target.join("forbidden.marker").exists()&&!source.join("forbidden.marker").exists(),"capture unchanged; forbidden.marker absent");
    ui_sign_out(app,&mut ck,&mut ev,BOB.0).await;ui_sign_in(app,&mut ck,&mut ev,ALICE).await;
    let revoked=process(app,running_id).await;
    record(&mut ck,&mut ev,"revoked account lease terminates actual running command",revoked["status"]=="cancelled"&&revoked["finished_at"].is_number(),revoked.to_string());
    // LaunchServices has finished the open event; remove only the temporary capture app/source.
    if editor.exists(){fs::remove_dir_all(&editor).unwrap();}fs::remove_file(&script).unwrap();
    finish(app,fx,ck,ev,"worktree-runtime");
}

fn finish(app: &AppHandle, fx: &Fx, ck: Checks, ev: Vec<Value>, phase: &str) {
    write_json(fx.root.join(format!("stage3-result-{phase}.json")),&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev}));
    println!("STAGE3 {phase}: {} passed, {} failed; {}",ck.passed,ck.failed.len(),fx.root.display());
    app.exit(if ck.failed.is_empty(){0}else{1});
}

/// Called by the real Tauri ExitRequested callback, after shutdown has been awaited.
/// Evidence is persisted before the app event loop returns, independently of restart.
pub(super) fn exit_evidence(fx: &Fx, clean: bool) -> bool {
    let phase=std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").unwrap_or_default();
    let suffix=if phase=="stage3-first" {"first"} else if phase=="stage3-restart" {"restart"} else if phase=="terminal-activity" {"terminal-activity"} else if phase=="worktree-runtime" {"worktree-runtime"} else {return clean;};
    let result_path=fx.root.join(format!("stage3-result-{suffix}.json"));
    let Ok(bytes)=fs::read(&result_path) else {return false;};
    let mut result:Value=serde_json::from_slice(&bytes).unwrap();
    let mut ok=clean;let mut details=json!({"shutdown":clean});
    if suffix=="first" {
        if let Ok(bytes)=fs::read(fx.root.join("stage3-state.json")) {
            let saved:Value=serde_json::from_slice(&bytes).unwrap();
            let owned=saved["exit"]["pids"].clone();let port=saved["exit"]["port"].as_u64().unwrap() as u16;
            let gone=tauri::async_runtime::block_on(gone(port,&owned));ok &= gone;
            details=json!({"shutdown":clean,"owned_tree_and_port_gone":gone,"pids":owned,"port":port});
        } else {ok=false;}
    }
    let name="real application ExitRequested awaits owned-process shutdown";
    result["checks"].as_array_mut().unwrap().push(json!({"check":name,"ok":ok,"detail":details}));
    if ok {result["passed"]=json!(result["passed"].as_u64().unwrap_or(0)+1);} else {result["failed"].as_array_mut().unwrap().push(json!(name));}
    write_json(result_path,&result);
    println!("STAGE3 EXIT: {details}; ok={ok}");
    ok
}
