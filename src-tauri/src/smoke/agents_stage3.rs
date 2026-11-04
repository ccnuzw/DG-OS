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
    let reply=ipc(app,"main","open_development_external",json!({"input":{"project_id":project,"process_id":id,"target":"preview"}})).await;
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
            let reply=ipc(app,"main","open_development_external",json!({"input":{"project_id":row["project"],"process_id":null,"target":"editor","editor":editor}})).await;
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

fn finish(app: &AppHandle, fx: &Fx, ck: Checks, ev: Vec<Value>, phase: &str) {
    write_json(fx.root.join(format!("stage3-result-{phase}.json")),&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev}));
    println!("STAGE3 {phase}: {} passed, {} failed; {}",ck.passed,ck.failed.len(),fx.root.display());
    app.exit(if ck.failed.is_empty(){0}else{1});
}

/// Called by the real Tauri ExitRequested callback, after shutdown has been awaited.
/// Evidence is persisted before the app event loop returns, independently of restart.
pub(super) fn exit_evidence(fx: &Fx, clean: bool) -> bool {
    let phase=std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").unwrap_or_default();
    let suffix=if phase=="stage3-first" {"first"} else if phase=="stage3-restart" {"restart"} else {return clean;};
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
