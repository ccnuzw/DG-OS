//! Real dual-engine production App/WKWebView workflow acceptance; no fixture CLI or provider.
//! Build: npm run agents:build && npm run build
//! Run with a NEW absolute root (both native CLIs must already be authenticated):
//! DGOS_PROJECT_ACCEPTANCE_ROOT=/private/tmp/dgos-workflows-round3-<unique> 
//! DGOS_PROJECT_ACCEPTANCE_PHASE=workflows-round3 cargo test -p dgos --features webview-smoke 
//! --test webview_smoke_agents -- --nocapture
//! Two explicit model requests total, one per engine; no warmup, repair retry or resumed turn.
//! Fixture preparation alone writes directly to disk. Every user mutation uses the shipped UI.
use super::*;

type Journey<T> = Result<T, String>;
const SOURCE: &str = "module.exports = { value: 1, manual: false };\n";
const MANUAL: &str = "module.exports = { value: 2, manual: true };\n";
const TAKEOVER: &str = "关闭相关智能体会话后手工操作";

fn save(fx: &Fx, name: &str, data: &Value) {
    fs::write(fx.root.join(format!("workflows-round3-{name}.json")), serde_json::to_vec_pretty(data).expect("workflow JSON")).expect("workflow evidence");
}

fn require(fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, ok: bool, detail: impl Display) -> Journey<()> {
    record(ck, ev, name, ok, detail);
    save(fx,"checkpoint",&json!({"step":name,"passed":ck.passed,"failed":ck.failed,"checks":ev}));
    if ok { Ok(()) } else { Err(name.into()) }
}

async fn dom(app: &AppHandle, args: Value, body: &str) -> Value {
    eval_long(app,&format!("const stage6UI=true;\n{DOM}\nconst A={args};\n{body}")).await
}

async fn step(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, args: Value, body: &str) -> Journey<Value> {
    let reply=dom(app,args,body).await;
    require(fx,ck,ev,name,ui_ok(&reply),reply.to_string())?;
    Ok(reply["value"].clone())
}

async fn host(app: &AppHandle, command: &str, args: Value) -> Journey<Value> {
    let reply=ipc(app,"main",command,args).await;
    if succeeded(&reply) { Ok(value(&reply)) } else { Err(format!("{command}: {}",reply.describe())) }
}

async fn workspace(app: &AppHandle) -> Journey<Value> { host(app,"read_development_workspace",json!({})).await }
async fn session(app: &AppHandle, sid: &str) -> Journey<Value> { host(app,"get_agent_session",json!({"sessionId":sid})).await }
fn rows<'a>(v: &'a Value, key: &str) -> Journey<&'a Vec<Value>> { v[key].as_array().ok_or_else(||format!("missing {key}: {v}")) }

#[cfg(target_os="macos")]
async fn capture(app: &AppHandle, fx: &Fx, name: &str) -> Journey<()> {
    use objc2::{msg_send,runtime::AnyObject};
    let window=app.get_webview_window("main").ok_or("main window absent")?;
    window.show().map_err(|e|e.to_string())?;
    let (send,receive)=oneshot::channel();
    let target=window.clone();
    window.run_on_main_thread(move || {
        let number=target.ns_window().map_err(|e|e.to_string()).and_then(|pointer| {
            let native=pointer as *mut AnyObject;
            if native.is_null() { return Err("native window absent".to_owned()); }
            let number:isize=unsafe { msg_send![native,windowNumber] };
            if number>0 { Ok(number) } else { Err("native window number absent".into()) }
        });
        let _=send.send(number);
    }).map_err(|e|e.to_string())?;
    let number=tokio::time::timeout(Duration::from_secs(10),receive).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())??;
    tokio::time::sleep(Duration::from_millis(350)).await;
    let path=fx.root.join(format!("workflows-round3-{name}.png"));
    let status=Command::new("/usr/sbin/screencapture").args(["-x","-o","-l"]).arg(number.to_string()).arg(&path).status().map_err(|e|e.to_string())?;
    if !status.success() { return Err(format!("target-window capture: {status}")); }
    save(fx,&format!("{name}-capture"),&json!({"png":path,"window_label":"main","window_number":number,"process_id":std::process::id(),"source":"actual NSWindow.windowNumber"}));
    Ok(())
}
#[cfg(not(target_os="macos"))]
async fn capture(_app: &AppHandle,_fx: &Fx,_name: &str) -> Journey<()> { Err("requires macOS WKWebView".into()) }

fn prepare(path: &Path) {
    make_repo(path);
    fs::write(path.join("workflow.cjs"),SOURCE).expect("fixture source");
    // Verification emits its actual PID and cwd but writes nothing during observation.
    fs::write(path.join("verify.cjs"),"const assert = require('node:assert/strict');\nconst value = require('./workflow.cjs');\nassert.equal(value.value, 2);\nassert.equal(typeof value.manual, 'boolean');\nconsole.log('ROUND3_VERIFY ' + JSON.stringify({pid:process.pid,cwd:process.cwd(),value}));\n").expect("fixture verification");
    fs::write(path.join("package.json"),r#"{"scripts":{"test":"node verify.cjs"}}"#).expect("fixture commands");
    assert!(git(path,&["add","."]));
    assert!(git(path,&["-c","user.name=dgos","-c","user.email=dgos@example.invalid","commit","-q","-m","workflow fixture"]));
}

async fn navigation(app: &AppHandle) -> Journey<()> {
    let result=dom(app,json!({}),r#"const t=await need('dev-toggle-navigation');if(t.getAttribute('aria-expanded')==='false')t.click();return {ok:!!await until(()=>tid('dev-project-create-open'))};"#).await;
    if ui_ok(&result) { Ok(()) } else { Err(result.to_string()) }
}

async fn observe(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    step(app,fx,ck,ev,"passively observe production mutations without replacing responses",json!({}),r#"
const original=window.fetch;const p={original,events:[]};window.__round3Workflows=p;
const names=['save_development_task','open_agent_session','send_agent_message','close_agent_session','get_agent_session','save_development_file','close_development_takeover','start_development_process'];
window.fetch=function(url,options,...rest){const command=String(url).split('/').at(-1);if(names.includes(command))p.events.push({command,args:JSON.parse(options?.body||'{}'),at:Date.now()});return original.call(this,url,options,...rest);};return {ok:true};
"#).await?;
    Ok(())
}

pub(super) async fn launch(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, engine: &str, path: &Path) -> Journey<(String,String,String)> {
    navigation(app).await?;
    let imported=ui_import_project(app,ck,ev,&format!("Round3 {engine}"),path).await;
    require(fx,ck,ev,"import actual workflow directory through project form",imported,path.display())?;
    let before=workspace(app).await?;
    let before_ids=ids(&host(app,"list_agent_sessions",json!({})).await?.as_array().cloned().ok_or("session list absent")?);
    let project=before["selection"]["project_id"].as_str().ok_or("selected project absent")?.to_owned();
    step(app,fx,ck,ev,"start new task and chosen native engine through TaskLaunch",json!({"engine":engine,"goal":format!("Round3 real {engine} workflow")}),r#"
await click('dev-start-task');await need('dev-task-launch');const goal=document.getElementById('task-launch-goal');const requirements=document.getElementById('task-launch-requirements');if(!goal||!requirements)throw Error('new launch fields absent');setInput(goal,A.goal);setInput(requirements,'Change workflow.cjs only; preserve verify.cjs and Git index.');
const c=document.getElementById('task-launch-engine');if(!c)throw Error('launch engine absent');c.click();const option=await until(()=>$$('[role=option]').find(e=>txt(e)===(A.engine==='codex'?'Codex':'Claude')));if(!option)throw Error('engine option absent');option.click();
const trust=await until(()=>{const e=document.getElementById('task-launch-trust');return e&&!e.disabled&&e;});if(!trust)throw Error('launch trust absent');const box=trust.matches('input,[role=checkbox]')?trust:trust.querySelector('input,[role=checkbox]');if(!box)throw Error('trust checkbox absent');if(!isChecked(box))box.click();
const b=await until(()=>{const b=tid('task-launch-start');return b&&!b.disabled&&b});if(!b)throw Error('TaskLaunch start disabled');b.click();return {ok:!!await until(()=>document.getElementById('agent-message'))};
"#).await?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(30);
    let after=loop {
        let after=workspace(app).await?;
        if rows(&after,"sessions")?.iter().any(|r|!before_ids.iter().any(|id|r["id"]==id.as_str())) { break after; }
        if tokio::time::Instant::now()>=deadline { return Err(format!("TaskLaunch host session timeout: {after}")); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let tasks:Vec<_>=rows(&after,"tasks")?.iter().filter(|r|!rows(&before,"tasks").unwrap().iter().any(|old|old["id"]==r["id"])).collect();
    let sessions:Vec<_>=rows(&after,"sessions")?.iter().filter(|r|!before_ids.iter().any(|id|r["id"]==id.as_str())).collect();
    require(fx,ck,ev,"one TaskLaunch submission persists exactly one task and one engine session",tasks.len()==1&&sessions.len()==1&&sessions[0]["engine"]==engine&&sessions[0]["task_id"]==tasks[0]["id"]&&sessions[0]["project_id"]==project&&sessions[0]["workspace"]==path.to_string_lossy().as_ref(),after.to_string())?;
    let task=tasks[0]["id"].as_str().ok_or("task id absent")?.to_owned();
    let sid=sessions[0]["id"].as_str().ok_or("session id absent")?.to_owned();
    save(fx,&format!("{engine}-launch"),&json!({"before":before,"after":after,"host_process_id":std::process::id()}));
    Ok((project,task,sid))
}

fn file_event(event: &Value, engine: &str) -> bool {
    let d=&event["data"];
    if engine=="codex" { return event["kind"]=="fileChange"&&d["type"]=="fileChange"&&d["status"]=="completed"&&d["changes"].as_array().is_some_and(|rows|rows.iter().any(|r|r["path"].as_str().is_some_and(|p|p.ends_with("workflow.cjs")))); }
    event["kind"]=="message"&&d["type"]=="assistant"&&d["content"].as_array().is_some_and(|rows|rows.iter().any(|r|r["type"]=="tool_use"&&matches!(r["name"].as_str(),Some("Edit"|"Write"|"MultiEdit"))&&r["input"]["file_path"].as_str().is_some_and(|p|p.ends_with("workflow.cjs"))))
}
fn failed_event(event: &Value) -> bool {
    let d=&event["data"];
    d["exitCode"].as_i64().is_some_and(|n|n!=0)||d["status"]=="failed"||d["is_error"]==true||d["content"].as_array().is_some_and(|rows|rows.iter().any(|r|r["type"]=="tool_result"&&r["is_error"]==true))
}

fn command_receipt(data: &Value) -> Option<Value> {
    match data {
        Value::String(text) => text.lines().find_map(|line|line.strip_prefix("ROUND3_VERIFY ").and_then(|json|serde_json::from_str(json).ok())),
        Value::Array(values) => values.iter().find_map(command_receipt),
        Value::Object(values) => values.values().find_map(command_receipt),
        _ => None,
    }
}

async fn references(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, engine: &str, sid: &str, task: &str, path: &Path) -> Journey<Value> {
    let tests=fs::read(path.join("verify.cjs")).map_err(|e|e.to_string())?;
    let prompt="Use native file tools now: change only workflow.cjs so module.exports is { value: 2, manual: false }. For Claude use Edit or Write (not Bash redirection); for Codex use apply_patch. Execute exactly node -e 'process.exit(7)' ONCE as its own native command tool invocation. Do not append echo, ||, ;, pipes, wrappers or anything that changes its exit status: the command tool must observe exit 7 as a failure. This intentional failure is expected and must not be retried. Then execute exactly node verify.cjs ONCE as a separate command tool invocation. Do not edit verify.cjs, package.json or README.md, install anything or perform Git writes. Report the two exit codes briefly.";
    let view=stage2::exercise_turn(app,ck,ev,sid,prompt).await;
    save(fx,&format!("{engine}-native-events"),&json!({"session.id":sid,"session":view}));
    require(fx,ck,ev,"single real model turn finishes idle and preserves verification program",view["status"]=="idle"&&fs::read(path.join("verify.cjs")).map_err(|e|e.to_string())?==tests,view.to_string())?;
    let events=rows(&view,"events")?;
    let receipt=events.iter().filter(|event|matches!(event["kind"].as_str(),Some("commandExecution"|"message"))).find_map(|event|command_receipt(&event["data"])).ok_or("native tool did not return actual verification output")?;
    require(fx,ck,ev,"native tool executes verification in exact workspace with actual OS PID",receipt["cwd"]==path.to_string_lossy().as_ref()&&receipt["pid"].as_u64().is_some_and(|pid|pid>0)&&receipt["value"]["value"]==2&&receipt["value"]["manual"]==false,receipt.to_string())?;
    save(fx,&format!("{engine}-native-process"),&receipt);
    let event=events.iter().find(|e|file_event(e,engine)).ok_or_else(||format!("no actual structured {engine} file event"))?;
    require(fx,ck,ev,"native file event carries actual sequence and run identity",event["seq"].is_u64()&&event["run"].is_string(),event.to_string())?;
    save(fx,&format!("{engine}-file-event"),&json!({"session":{"id":sid},"seq":event["seq"],"run":event["run"],"data":event["data"],"kind":event["kind"]}));
    step(app,fx,ck,ev,"actual structured native event opens production disk diff",json!({"seq":event["seq"],"run":event["run"]}),r#"
const a=await need('agent-event-'+A.seq);if(a.dataset.eventRun!==A.run)throw Error('event run mismatch');
const reference=await until(()=>$$('[data-reference-path]',a).find(e=>e.dataset.referencePath.endsWith('workflow.cjs')));if(!reference)throw Error('native file reference absent');
const link=reference.querySelector('[data-reference-mode=diff]');if(!link)throw Error('diff link absent');link.click();const diff=await until(()=>{const d=tid('dev-file-diff');return txt(d?.querySelector('strong'))==='workflow.cjs'&&txt(d).includes('+')&&d;});return {ok:!!diff,text:txt(diff)};
"#).await?;
    capture(app,fx,&format!("{engine}-native-diff")).await?;
    let failure=events.iter().find(|e|failed_event(e)&&e["run"].is_string()).ok_or("intentional native command failure event absent")?;
    save(fx,&format!("{engine}-failure-event"),&json!({"session":{"id":sid},"seq":failure["seq"],"run":failure["run"],"data":failure["data"],"kind":failure["kind"]}));
    let runs=host(app,"list_development_runs",json!({"taskId":task})).await?;
    require(fx,ck,ev,"native failure link identifies a persisted run belonging to exact task",runs.as_array().is_some_and(|rs|rs.iter().any(|r|r["id"]==failure["run"]&&r["task_id"]==task&&r["source"]=="native_agent")),runs.to_string())?;
    step(app,fx,ck,ev,"failed tool event opens its exact persisted run without another model turn",json!({"seq":failure["seq"],"run":failure["run"]}),r#"
const b=await need('agent-reference-'+A.seq+'-run');if(b.dataset.runId!==A.run)throw Error('failure run mismatch');b.click();const row=await until(()=>{const r=tid('dev-run-history')?.querySelector('[data-run-id="'+A.run+'"]');return r&&document.activeElement===r&&r;});return {ok:!!row,text:txt(row)};
"#).await?;
    capture(app,fx,&format!("{engine}-failure-run")).await?;
    step(app,fx,ck,ev,"native file event opens exact production editable source",json!({"seq":event["seq"],"disk":fs::read_to_string(path.join("workflow.cjs")).map_err(|e|e.to_string())?}),r#"
const a=await need('agent-event-'+A.seq);const r=$$('[data-reference-path]',a).find(e=>e.dataset.referencePath.endsWith('workflow.cjs'));const b=r?.querySelector('[data-reference-mode=file]');if(!b)throw Error('file link absent');b.click();return {ok:!!await until(()=>txt(tid('dev-file-source')?.querySelector('strong'))==='workflow.cjs'&&tid('dev-file-editor')?.value===A.disk)};
"#).await?;
    Ok(view)
}

async fn attach(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, native: &Value) -> Journey<()> {
    // Re-enter the saved run while still live. Only a snapshot may be requested.
    step(app,fx,ck,ev,"attach saved live run through TaskLaunch without native reopen",json!({"sid":sid}),r#"
await click('dev-start-task');await need('dev-task-launch');const p=window.__round3Workflows;const index=p.events.length;await click('task-launch-continue-'+A.sid);
if(!await until(()=>document.getElementById('agent-message')&&!document.getElementById('agent-message').disabled))throw Error('attached composer absent');
await new Promise(r=>setTimeout(r,600));const events=p.events.slice(index);return {ok:events.some(e=>e.command==='get_agent_session'&&e.args.sessionId===A.sid)&&!events.some(e=>['open_agent_session','send_agent_message'].includes(e.command)),events};
"#).await?;
    let live=session(app,sid).await?;
    require(fx,ck,ev,"live attach retains host native identity and event history",live["native_id"]==native["native_id"]&&live["events"]==native["events"]&&live["status"]=="idle",live.to_string())?;
    // onContinue leaves the launch drawer open in current production integration.
    step(app,fx,ck,ev,"dismiss launch drawer without closing attached Agent",json!({}),r#"const b=await need('dev-start-task');if(b.getAttribute('aria-expanded')==='true')b.click();return {ok:true};"#).await?;
    Ok(())
}

pub(super) async fn takeover(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, engine: &str, project: &str, task: &str, sid: &str, path: &Path) -> Journey<()> {
    let original=fs::read_to_string(path.join("workflow.cjs")).map_err(|e|e.to_string())?;
    let processes=host(app,"list_development_processes",json!({"taskId":task})).await?;
    step(app,fx,ck,ev,"manual edit refuses busy directory without implicit close or save replay",json!({"text":MANUAL}),r#"
await fill('dev-file-editor',A.text);const b=await until(()=>{const b=tid('dev-file-save');return b&&!b.disabled&&b});if(!b)throw Error('save disabled');b.click();return {ok:!!await until(()=>tid('dev-manual-takeover')),draft:tid('dev-file-editor')?.value};
"#).await?;
    let scope=json!({"input":{"project_id":project,"task_id":task}});
    let blocked=host(app,"read_development_takeover",scope.clone()).await?;
    require(fx,ck,ev,"busy refusal preserves disk and lists actual same-directory session",fs::read_to_string(path.join("workflow.cjs")).map_err(|e|e.to_string())?==original&&blocked["directory"]==path.to_string_lossy().as_ref()&&blocked["can_close"]==true&&blocked["sessions"].as_array().is_some_and(|rs|rs.len()==1&&rs[0]["id"]==sid&&rs[0]["task_id"]==task),blocked.to_string())?;
    step(app,fx,ck,ev,"explicit manual takeover confirmation identifies directory and exact session",json!({"label":TAKEOVER,"sid":sid,"path":path}),r#"
const s=await need('dev-manual-takeover');const b=await until(()=>$$('button',s).find(b=>txt(b)===A.label&&!b.disabled));if(!b)throw Error('takeover close disabled');b.click();const d=await until(()=>$$('[role=dialog],[role=alertdialog]').find(e=>txt(e).includes(A.sid)&&txt(e).includes(A.path)));if(!d)throw Error('scoped takeover confirmation absent');const confirm=$$('button',d).find(b=>txt(b)===A.label);if(!confirm)throw Error('takeover confirm absent');return {ok:true,dialog:txt(d)};
"#).await?;
    capture(app,fx,&format!("{engine}-takeover-confirmation")).await?;
    step(app,fx,ck,ev,"confirm scoped takeover through actual Modal",json!({"label":TAKEOVER}),r#"
const d=$$('[role=dialog],[role=alertdialog]').find(e=>$$('button',e).some(b=>txt(b)===A.label));if(!d)throw Error('takeover dialog disappeared');$$('button',d).find(b=>txt(b)===A.label).click();return {ok:!!await until(()=>!d.isConnected&&tid('dev-manual-takeover')?.getAttribute('aria-busy')==='false')};
"#).await?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(30);
    let released=loop {
        let view=host(app,"read_development_takeover",scope.clone()).await?;
        if view["sessions"].as_array().is_some_and(Vec::is_empty)&&view["host_busy"]==false { break view; }
        if tokio::time::Instant::now()>=deadline { return Err(format!("takeover not released: {view}")); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    require(fx,ck,ev,"confirmed takeover closes exact native session and does not save or start a command",session(app,sid).await?["status"]=="closed"&&fs::read_to_string(path.join("workflow.cjs")).map_err(|e|e.to_string())?==original&&host(app,"list_development_processes",json!({"taskId":task})).await?==processes,released.to_string())?;
    step(app,fx,ck,ev,"explicit user resubmission saves retained manual draft",json!({"text":MANUAL}),r#"
if(tid('dev-file-editor')?.value!==A.text)throw Error('draft lost during takeover');await click('dev-file-save');return {ok:!!await until(()=>tid('dev-file-save')?.disabled&&txt(tid('dev-file-source')?.querySelector('[role=status]'))==='没有未保存修改'&&!tid('dev-file-source')?.querySelector('[role=alert]'))};
"#).await?;
    require(fx,ck,ev,"manual UI save writes intended exact file",fs::read_to_string(path.join("workflow.cjs")).map_err(|e|e.to_string())?==MANUAL,path.display())?;
    save(fx,&format!("{engine}-takeover"),&json!({"before":blocked,"after":released,"closed_session":session(app,sid).await?,"file":fs::read_to_string(path.join("workflow.cjs")).map_err(|e|e.to_string())?}));
    Ok(())
}

pub(super) async fn verify(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, engine: &str, project: &str, task: &str, path: &Path) -> Journey<Value> {
    let before=host(app,"list_development_processes",json!({"taskId":task})).await?;
    step(app,fx,ck,ev,"explicitly configure and run real host verification after save",json!({}),r#"
const t=await need('dev-toggle-runtime');if(t.getAttribute('aria-expanded')==='false')t.click();await click('dev-runtime-processes');
const configuration=await need('dev-runtime-configuration');configuration.open=true;const pause=()=>new Promise(r=>setTimeout(r,80));
const kind=await need('dev-process-kind');kind.value='command';kind.dispatchEvent(new Event('change',{bubbles:true}));await pause();await fill('dev-process-program','node');await fill('dev-process-cwd','');
const s=await need('dev-development-runtime');while($$('button[aria-label^="移除参数"]',s).length){$$('button[aria-label^="移除参数"]',s)[0].click();await pause();}
const add=$$('button',s).find(b=>txt(b)==='添加参数');if(!add)throw Error('add argument absent');add.click();await pause();const f=$$('input',s).find(e=>labelOf(e)==='参数 1');if(!f)throw Error('argument field absent');setInput(f,'verify.cjs');await pause();
const check=$$('input[type=checkbox]',s).find(e=>labelOf(e)==='将本命令记录为验证证据');if(!check)throw Error('verification checkbox absent');if(!check.checked)check.click();
const trust=await need('dev-process-trust');if(!trust.checked)trust.click();await pause();const b=await until(()=>{const b=tid('dev-process-start');return b&&!b.disabled&&b});if(!b)throw Error('host start disabled');b.click();return {ok:true};
"#).await?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(45);
    let process=loop {
        let all=host(app,"list_development_processes",json!({"taskId":task})).await?;
        let new:Vec<_>=all.as_array().ok_or("process list absent")?.iter().filter(|r|!before.as_array().into_iter().flatten().any(|old|old["id"]==r["id"])).collect();
        if new.len()>1 { return Err(format!("one verification click created multiple receipts: {all}")); }
        if let Some(row)=new.first() { if row["finished_at"].is_number() { break host(app,"read_development_process",json!({"id":row["id"]})).await?; } }
        if tokio::time::Instant::now()>=deadline { return Err(format!("verification timeout: {all}")); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let actual=process["output"].as_str().and_then(|s|s.lines().find_map(|l|l.strip_prefix("ROUND3_VERIFY "))).and_then(|s|serde_json::from_str::<Value>(s).ok()).ok_or_else(||format!("real verification PID/cwd absent: {process}"))?;
    require(fx,ck,ev,"one real process verifies exact task directory and saved file",process["source"]=="host_command"&&process["task_id"]==task&&process["project_id"]==project&&process["program"]=="node"&&process["args"]==json!(["verify.cjs"])&&process["exit_code"]==0&&process["verification_status"]=="observed_stable"&&process["verification_complete"]==true&&process["mutation_observed"]==false&&actual["pid"].as_u64().is_some_and(|pid|pid>0)&&actual["cwd"]==path.to_string_lossy().as_ref()&&actual["value"]["manual"]==true,process.to_string())?;
    save(fx,&format!("{engine}-verification"),&json!({"process":process,"actual_os_process":actual,"saved_file":MANUAL}));
    capture(app,fx,&format!("{engine}-verification")).await?;
    Ok(process)
}

async fn resume(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, original: &Value) -> Journey<()> {
    let before=workspace(app).await?;
    step(app,fx,ck,ev,"return to saved Agent using explicit native resume after manual takeover",json!({"sid":sid}),r#"
await click('dev-start-task');await need('dev-task-launch');const trust=await until(()=>{const e=document.getElementById('task-launch-trust');return e&&!e.disabled&&e;});if(!trust)throw Error('resume trust absent');const box=trust.matches('input,[role=checkbox]')?trust:trust.querySelector('input,[role=checkbox]');if(!box)throw Error('resume trust absent');if(!isChecked(box))box.click();
const p=window.__round3Workflows;const index=p.events.length;const b=await until(()=>{const b=tid('task-launch-continue-'+A.sid);return b&&!b.disabled&&b});if(!b)throw Error('saved resume disabled');b.click();
const opened=await until(()=>p.events.slice(index).filter(e=>e.command==='open_agent_session').length===1);if(!opened)throw Error('explicit native resume absent');
if(!await until(()=>document.getElementById('agent-message')&&!document.getElementById('agent-message').disabled))throw Error('resumed composer absent');const events=p.events.slice(index);return {ok:events.filter(e=>e.command==='open_agent_session').length===1&&!events.some(e=>e.command==='send_agent_message'),events};
"#).await?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(30);
    let resumed=loop {
        let view=session(app,sid).await?;
        if view["status"]=="idle" { break view; }
        if tokio::time::Instant::now()>=deadline { return Err(format!("native resume timeout: {view}")); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let after=workspace(app).await?;
    require(fx,ck,ev,"native resume preserves saved session task and native thread IDs without creating another task",original["native_id"].is_string()&&resumed["native_id"]==original["native_id"]&&resumed["workspace"]==original["workspace"]&&rows(&after,"sessions")?.len()==rows(&before,"sessions")?.len()&&rows(&after,"tasks")?.len()==rows(&before,"tasks")?.len()&&after["selection"]["session_id"]==sid,json!({"before":before,"after":after,"resumed":resumed}).to_string())?;
    step(app,fx,ck,ev,"dismiss launch after native resume",json!({}),r#"const b=await need('dev-start-task');if(b.getAttribute('aria-expanded')==='true')b.click();return {ok:true};"#).await?;
    // No additional model request is needed to demonstrate native resume.
    let closed=ui_close_run(app,ck,ev).await;
    require(fx,ck,ev,"close resumed Agent through production control",closed,sid)?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(30);
    loop { let view=session(app,sid).await?;if view["status"]=="closed" { break; }if tokio::time::Instant::now()>=deadline { return Err(format!("resumed Agent close timeout: {view}")); }tokio::time::sleep(Duration::from_millis(100)).await; }
    Ok(())
}

pub(super) async fn review(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, engine: &str, task: &str, process: &Value) -> Journey<()> {
    step(app,fx,ck,ev,"review actual task result and current passing host verification",json!({}),r#"
const tab=await need('dev-review-tab-review');tab.focus();tab.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));const v=await until(()=>tid('dev-task-review-verification')?.dataset.verification==='observed_stable');const s=await need('dev-task-review');return {ok:!!v&&txt(s).includes('workflow.cjs'),text:txt(s)};
"#).await?;
    let current=host(app,"read_development_review",json!({"taskId":task})).await?;
    require(fx,ck,ev,"review host fingerprint still matches independently executed saved-file verification",process["fingerprint"]==current["fingerprint"]&&current["changes"].as_array().is_some_and(|rs|rs.iter().any(|r|r["path"]=="workflow.cjs")),current.to_string())?;
    capture(app,fx,&format!("{engine}-review")).await?;
    step(app,fx,ck,ev,"explicit review completion requires user confirmation",json!({}),r#"
const c=await need('dev-task-review-confirm');if(!c.checked)c.click();const b=await until(()=>{const b=tid('dev-task-review-complete');return b&&!b.disabled&&b});if(!b)throw Error('review completion disabled');b.click();return {ok:!!await until(()=>!tid('dev-task-review-complete'))};
"#).await?;
    let after=workspace(app).await?;
    require(fx,ck,ev,"host task status is completed only after explicit reviewed completion",rows(&after,"tasks")?.iter().any(|r|r["id"]==task&&r["status"]=="completed"),after.to_string())?;
    save(fx,&format!("{engine}-completed"),&json!({"workspace":after,"review":current,"process":process}));
    Ok(())
}

async fn per_engine(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, engine: &str) -> Journey<()> {
    let path=fx.root.join(format!("workflows-round3-{engine}"));prepare(&path);
    let (project,task,sid)=launch(app,fx,ck,ev,engine,&path).await?;
    let native=references(app,fx,ck,ev,engine,&sid,&task,&path).await?;
    attach(app,fx,ck,ev,&sid,&native).await?;
    let event=rows(&native,"events")?.iter().find(|e|file_event(e,engine)).ok_or("native file event lost")?;
    step(app,fx,ck,ev,"reopen native source after saved live attach",json!({"seq":event["seq"]}),r#"
const a=await need('agent-event-'+A.seq);const r=$$('[data-reference-path]',a).find(e=>e.dataset.referencePath.endsWith('workflow.cjs'));const b=r?.querySelector('[data-reference-mode=file]');if(!b)throw Error('file link absent after attach');b.click();await new Promise(resolve=>setTimeout(resolve,100));return {ok:!!await until(()=>!b.disabled&&tid('dev-file-editor'))};
"#).await?;
    takeover(app,fx,ck,ev,engine,&project,&task,&sid,&path).await?;
    let process=verify(app,fx,ck,ev,engine,&project,&task,&path).await?;
    resume(app,fx,ck,ev,&sid,&native).await?;
    review(app,fx,ck,ev,engine,&task,&process).await?;
    let counts=step(app,fx,ck,ev,"exact workflow mutation counts include attach versus native resume",json!({"sid":sid,"task":task}),r#"
const events=window.__round3Workflows.events;const sessionEvents=events.filter(e=>(e.args.sessionId??e.args.input?.session_id??e.args.input?.id)===A.sid);const opens=sessionEvents.filter(e=>e.command==='open_agent_session');const sends=sessionEvents.filter(e=>e.command==='send_agent_message');const saves=events.filter(e=>e.command==='save_development_file'&&e.args.input?.task_id===A.task);const starts=events.filter(e=>e.command==='start_development_process'&&e.args.input?.task_id===A.task);const takeovers=events.filter(e=>e.command==='close_development_takeover'&&e.args.input?.task_id===A.task);
return {ok:opens.length===2&&sends.length===1&&saves.length===2&&starts.length===1&&takeovers.length===1,opens,sends,saves,starts,takeovers};
"#).await?;
    let final_workspace=workspace(app).await?;
    let runs=host(app,"list_development_runs",json!({"taskId":task})).await?;
    require(fx,ck,ev,"host retains exactly one task session native run and explicit command receipt",rows(&final_workspace,"tasks")?.iter().filter(|r|r["project_id"]==project).count()==1&&rows(&final_workspace,"sessions")?.iter().filter(|r|r["task_id"]==task).count()==1&&runs.as_array().is_some_and(|rs|rs.iter().filter(|r|r["source"]=="native_agent").count()==1)&&host(app,"list_development_processes",json!({"taskId":task})).await?.as_array().is_some_and(|rs|rs.len()==1),runs.to_string())?;
    save(fx,&format!("{engine}-counts"),&json!({"ipc":counts,"workspace":final_workspace,"runs":runs}));
    Ok(())
}

pub(super) async fn run(app: &AppHandle, fx: &Fx, mut ck: Checks, mut ev: Vec<Value>) {
    save(fx,"commands",&json!({"build":["npm run agents:build","npm run build"],"run":format!("DGOS_PROJECT_ACCEPTANCE_ROOT='{}' DGOS_PROJECT_ACCEPTANCE_PHASE=workflows-round3 cargo test -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture",fx.root.display()),"prerequisites":["Fresh root for each run","Authenticated native Codex and Claude CLIs","macOS Screen Recording permission for target-window screenshots"],"model_requests":2,"implicit_retries":0}));
    let mut completed=Vec::new();
    let started=observe(app,fx,&mut ck,&mut ev).await;
    if let Err(error)=started { record(&mut ck,&mut ev,"workflow transport observation installed",false,error); }
    else {
        for engine in ENGINES {
            if let Err(error)=per_engine(app,fx,&mut ck,&mut ev,engine).await {
                record(&mut ck,&mut ev,&format!("{engine} full real workflow completes"),false,error);
                save(fx,&format!("{engine}-failure-dom"),&dom(app,json!({}),r#"return {ok:true,url:location.href,body:document.body.innerText,dialogs:$$('[role=dialog],[role=alertdialog]').map(txt),events:window.__round3Workflows?.events};"#).await);
                if let Err(error)=capture(app,fx,&format!("{engine}-failure")).await { record(&mut ck,&mut ev,"failure screenshot captured",false,error); }
                // Cleanup is explicit host shutdown, never a model retry or softened assertion.
                record(&mut ck,&mut ev,"failed journey owned runtime cleanup",app.state::<AgentManager>().shutdown().await,engine);
                break;
            }
            completed.push(engine);
        }
    }
    save(fx,"ipc",&dom(app,json!({}),r#"const p=window.__round3Workflows;if(p)window.fetch=p.original;return {ok:true,events:p?.events};"#).await);
    record(&mut ck,&mut ev,"all workflow native and host processes cleaned up",app.state::<AgentManager>().shutdown().await,"production AgentManager shutdown");
    save(fx,"result",&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev,"source":"production App/WKWebView and IPC; real Codex/Claude model file and command tools; actual disk, native run IDs and verification PID/cwd","requested_engines":["codex","claude"],"completed_engines":completed}));
    app.exit(if ck.failed.is_empty(){0}else{1});
}
