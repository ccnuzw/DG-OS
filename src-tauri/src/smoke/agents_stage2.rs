//! Opt-in real model development acceptance; all project Git writes below are fixture
//! preparation only. Production file/review commands remain read-only.
use super::*;

fn prepare(path: &Path, engine: &str) {
    make_repo(path);
    if engine == "codex" {
        fs::write(path.join("package.json"), r#"{"type":"module","scripts":{"test":"node --test"}}"#).unwrap();
        fs::write(path.join("prices.js"), "export function total(items) { return items.reduce((sum, item) => sum + item.price, 0); }\n").unwrap();
        fs::write(path.join("receipt.js"), "import { total } from './prices.js';\nexport function receipt(items) { return `Total: ${total(items).toFixed(2)}`; }\n").unwrap();
        fs::write(path.join("prices.test.js"), "import { test } from 'node:test';\nimport assert from 'node:assert/strict';\nimport { total } from './prices.js';\nimport { receipt } from './receipt.js';\ntest('basic', () => { assert.equal(total([{price:10}]), 10); assert.equal(receipt([{price:10}]), 'Total: 10.00'); });\n").unwrap();
    } else {
        fs::create_dir(path.join("src")).unwrap();
        fs::write(path.join("Cargo.toml"), "[package]\nname='receipt-proof'\nversion='0.1.0'\nedition='2021'\n").unwrap();
        fs::write(path.join("src/lib.rs"), "pub mod price;\npub mod receipt;\n#[cfg(test)] mod tests { #[test] fn basic() { assert_eq!(crate::price::total(&[10]),10); assert_eq!(crate::receipt::format(&[10]),\"Total: 10\"); } }\n").unwrap();
        fs::write(path.join("src/price.rs"), "pub fn total(items: &[i64]) -> i64 { items.iter().sum() }\n").unwrap();
        fs::write(path.join("src/receipt.rs"), "pub fn format(items: &[i64]) -> String { format!(\"Total: {}\", crate::price::total(items)) }\n").unwrap();
    }
    fs::write(path.join(".gitignore"), "target/\nignored/\n").unwrap();
    assert!(git(path, &["add", "."]));
    assert!(git(path, &["-c", "user.name=dgos", "-c", "user.email=dgos@example.invalid", "commit", "-q", "-m", "fixture sources"]));
    fs::write(path.join("README.md"), "user staged content must survive\n").unwrap();
    assert!(git(path, &["add", "README.md"]));
    fs::write(path.join("README.md"), "user staged content must survive\nuser unstaged note\n").unwrap();
    fs::write(path.join("user-notes.txt"), "pre-existing untracked user notes\n").unwrap();
    fs::write(path.join("binary.dat"), [0,1,2]).unwrap();
    fs::write(path.join("large.txt"), vec![b'a'; 262145]).unwrap();
    fs::create_dir(path.join("ignored")).unwrap(); fs::write(path.join("ignored/hidden.txt"), "ignored source\n").unwrap();
}
fn index_content(path: &Path) -> Vec<u8> { Command::new("git").arg("-C").arg(path).args(["show", ":README.md"]).output().unwrap().stdout }
async fn state(app: &AppHandle, sid: &str) -> Value { value(&ipc(app,"main","get_agent_session",json!({"sessionId":sid})).await) }
pub(super) async fn settle(app: &AppHandle, sid: &str, allow: bool, from_seq: u64) -> Result<Value,String> {
    let deadline = tokio::time::Instant::now()+Duration::from_secs(300);
    let mut seen_running = false;
    loop {
        let view = state(app,sid).await;
        if let Some(pending)=view["pending"].as_array().filter(|p|!p.is_empty()) {
            seen_running=true;
            let r=ui(app,json!({"allow":allow}),r#"const b=await until(()=>A.allow?btn('允许本次'):(btn('拒绝')||btn('取消')));if(!b)return {ok:false,buttons:$$('button').map(txt),body:document.body.innerText};b.click();return {ok:true};"#).await;
            if !ui_ok(&r) { return Err(format!("approval DOM {r}; pending {pending:?}")); }
            if !allow { return Ok(json!({"denied":true,"pending":pending})); }
        }
        let status=view["status"].as_str().unwrap_or("");
        seen_running |= matches!(status,"running"|"waiting");
        if matches!(status,"idle"|"error"|"unknown"|"closed") && (seen_running || view["events"].as_array().is_some_and(|e|e.iter().any(|v|v["seq"].as_u64().unwrap_or(0)>from_seq&&matches!(v["kind"].as_str(),Some("turn/completed"|"result"|"failure"|"error"))))) { return Ok(view); }
        if tokio::time::Instant::now()>deadline { return Err(format!("terminal timeout: {view}")); }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}
pub(super) async fn exercise_turn(app:&AppHandle,ck:&mut Checks,ev:&mut Vec<Value>,sid:&str,prompt:&str)->Value {
    let from_seq=last_seq(app,sid).await;
    ui_send(app,ck,ev,prompt).await;
    match settle(app,sid,true,from_seq).await { Ok(v)=>{record(ck,ev,"real native turn terminal",v["status"]=="idle",v["status"].to_string());v},Err(e)=>{record(ck,ev,"real native turn terminal",false,e);Value::Null} }
}
async fn resume_ui(app:&AppHandle,sid:&str)->Value { ui(app,json!({"sid":sid}),r#"const t=await need('dev-trust');if(!t.checked)t.click();const row=await need('dev-session-item-'+A.sid);const b=await until(()=>{const b=row.querySelector('button');return b&&!b.disabled&&b});if(!b)throw Error('resume disabled');b.click();return {ok:!!(await until(()=>document.getElementById('agent-message')))};"#).await }
async fn inspect_ui(app:&AppHandle,ck:&mut Checks,ev:&mut Vec<Value>,path:&str){
    let r=ui(app,json!({"path":path}),r#"const t=await need('dev-review-tab-files');t.focus();t.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));await fill('dev-file-search',A.path);const f=await until(()=>tid('dev-file-item-'+A.path));if(!f)throw Error('file absent '+A.path);f.click();const s=await until(()=>tid('dev-file-source')?.querySelector('textarea,code'));return {ok:!!s,text:s?.value??txt(s)};"#).await;
    record(ck,ev,"actual source rendered",ui_ok(&r)&&!r["value"]["text"].as_str().unwrap_or("").is_empty(),r.to_string());
    let r=ui(app,json!({"path":path}),r#"const t=await need('dev-review-tab-changes');t.focus();t.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));const f=await until(()=>tid('dev-change-item-'+A.path));if(!f)throw Error('change absent');f.click();const d=await until(()=>tid('dev-file-diff')?.querySelector('pre'));return {ok:!!d,text:txt(d)};"#).await;
    record(ck,ev,"actual disk diff rendered",ui_ok(&r)&&r["value"]["text"].as_str().is_some_and(|t|t.contains("+")),r.to_string());
    let r=ui(app,json!({}),r#"const t=await need('dev-review-tab-runs');t.focus();t.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));const d=await until(()=>tid('dev-run-history')?.querySelector('.dev-run'));return {ok:!!d,text:txt(tid('dev-run-history'))};"#).await;
    record(ck,ev,"persisted execution records visible",ui_ok(&r),r.to_string());
    let r=ui(app,json!({}),r#"for(const d of $$('[data-testid=dev-run-history] details'))d.open=true;const d=tid('dev-run-history');const text=txt(d);return {ok:/npm test|cargo test/.test(text)&&/passed|test result|Tests:/.test(text),text};"#).await;
    record(ck,ev,"actual command and result rendered",ui_ok(&r),r.to_string());
}
async fn finish(app:&AppHandle,fx:&Fx,mut ck:Checks,ev:Vec<Value>,phase:&str){
    ck.check("all Agent processes closed",app.state::<AgentManager>().shutdown().await,"shutdown");
    fs::write(fx.root.join(format!("stage2-result-{phase}.json")),serde_json::to_vec_pretty(&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev})).unwrap()).unwrap();
    println!("STAGE2 {phase}: {} passed, {} failed; {}",ck.passed,ck.failed.len(),fx.root.display());app.exit(if ck.failed.is_empty(){0}else{1});
}
pub(super) async fn run(app:&AppHandle,fx:&Fx,mut ck:Checks,mut ev:Vec<Value>,restart:bool){
    if restart {
        let saved:Value=serde_json::from_slice(&fs::read(fx.root.join("stage2-evidence.json")).unwrap()).unwrap();
        for row in saved.as_array().unwrap() {
            let sid=row["sid"].as_str().unwrap();let task=row["task"].as_str().unwrap();let goal=row["goal"].as_str().unwrap();
            let runs=value(&ipc(app,"main","list_development_runs",json!({"taskId":task})).await);
            record(&mut ck,&mut ev,"execution history survives new process",runs.as_array().is_some_and(|r|r.iter().any(|r|r["status"]=="completed")),runs.to_string());
            let r=ui(app,json!({"pid":row["project"]}),r#"await click('dev-project-item-'+A.pid);return {ok:true};"#).await;record(&mut ck,&mut ev,"restore selected project",ui_ok(&r),r.to_string());
            ui_select_task(app,&mut ck,&mut ev,goal).await;
            let r=resume_ui(app,sid).await;record(&mut ck,&mut ev,"explicit native restore through UI",ui_ok(&r),r.to_string());
            let view=state(app,sid).await;record(&mut ck,&mut ev,"native id restored",view["native_id"]==row["native_id"],view["native_id"].to_string());
            exercise_turn(app,&mut ck,&mut ev,sid,"Read the project and run its existing test command once. Report command and result. Do not change files.").await;
            let path=row["file"].as_str().unwrap();inspect_ui(app,&mut ck,&mut ev,path).await;
            ui_close_run(app,&mut ck,&mut ev).await;
        }
        finish(app,fx,ck,ev,"restart").await;return;
    }
    let mut saved=vec![];
    for engine in ENGINES {
        let path=fx.root.join(format!("stage2-{engine}"));prepare(&path,engine);
        let original_index=index_content(&path);
        ui_import_project(app,&mut ck,&mut ev,&format!("Stage2 {engine}"),&path).await;
        let goal=format!("Stage2 multi-file {engine}");ui_new_task(app,&mut ck,&mut ev,&goal).await;
        let selected=value(&ipc(app,"main","read_development_workspace",json!({})).await)["selection"].clone();
        let _=ui(app,json!({}),r#"const f=$$('input').find(e=>labelOf(e).startsWith('模型'));if(f)setInput(f,'');await new Promise(r=>setTimeout(r,100));return {ok:true};"#).await;
        let task=selected["task_id"].as_str().unwrap_or_default().to_owned();let pid=selected["project_id"].as_str().unwrap_or_default().to_owned();
        let r=super::acceptance::pick_engine(app,if engine=="codex"{"Codex"}else{"Claude"}).await;record(&mut ck,&mut ev,"choose engine",ui_ok(&r),r.to_string());
        let before=ids(&sessions_of(app).await);ui_start_session(app,&mut ck,&mut ev,engine).await;
        let Some(sid)=new_session_id(app,&before).await else {record(&mut ck,&mut ev,"session created",false,engine);continue;};
        // Claude's known first-turn service behavior gets an ordinary greeting, not an implicit retry.
        exercise_turn(app,&mut ck,&mut ev,&sid,"Hello. We will work on this small receipt project in the next request. Do not modify files yet.").await;
        let prompt=if engine=="codex" {"Implement quantity support across prices.js and receipt.js. total must sum price*quantity (default quantity=1); receipt returns exactly one line formatted 'Items: <sum quantities>; Total: <two decimals>' using semicolon+space as separator. Update meaningful tests in prices.test.js. Execute npm test. Preserve README.md, user-notes.txt and all existing staged changes. Do not run git writes. Implement now."} else {"Implement quantity support across src/price.rs and src/receipt.rs: introduce public Item { price: i64, quantity: i64 }, total(&[Item]) sums price*quantity; receipt::format returns exactly one line 'Items: <sum quantity>; Total: <sum>' using semicolon+space as separator, no newline. Update src/lib.rs tests. Run cargo test --offline. Preserve README.md, user-notes.txt and all staged changes; no git writes. Implement now."};
        let view=exercise_turn(app,&mut ck,&mut ev,&sid,prompt).await;
        fs::write(fx.root.join(format!("stage2-{engine}-feature-events.json")),serde_json::to_vec_pretty(&view).unwrap()).unwrap();
        let files=if engine=="codex"{["prices.js","receipt.js"]}else{["src/price.rs","src/receipt.rs"]};
        record(&mut ck,&mut ev,"real multi-file feature on disk",files.iter().all(|f|fs::read_to_string(path.join(f)).unwrap_or_default().contains("quantity")),engine);
        // Inject a concrete consumer-visible regression into two files, then ask the engine to fix it.
        for file in files { let f=path.join(file);let mut text=fs::read_to_string(&f).unwrap();text.push_str(if engine=="codex" { "\nTHIS_IS_A_REAL_SYNTAX_REGRESSION !!!\n" } else { "\nTHIS_IS_A_REAL_SYNTAX_REGRESSION !!!\n" });fs::write(f,text).unwrap(); }
        let prompt="A regression has just been introduced into the quantity implementation in both source files. Run the tests to observe the failure, repair both files without changing intended behavior or weakening tests, then run the tests again. Preserve all user notes and Git index. Do not perform git writes.";
        let view=exercise_turn(app,&mut ck,&mut ev,&sid,prompt).await;
        fs::write(fx.root.join(format!("stage2-{engine}-fix-events.json")),serde_json::to_vec_pretty(&view).unwrap()).unwrap();
        let output=if engine=="codex"{Command::new("npm").arg("test").current_dir(&path).output()}else{Command::new("cargo").args(["test","--offline"]).current_dir(&path).output()}.unwrap();
        record(&mut ck,&mut ev,"independent actual project verification",output.status.success(),format!("{}{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr)));
        let probe=if engine=="codex" { Command::new("node").args(["--input-type=module","-e","import assert from 'node:assert/strict'; import {total} from './prices.js'; import {receipt} from './receipt.js'; assert.equal(total([{price:7,quantity:3},{price:2}]),23); assert.equal(receipt([{price:7,quantity:3},{price:2}]),'Items: 4; Total: 23.00');"]).current_dir(&path).output() } else {
            let probes=path.join("tests");fs::create_dir_all(&probes).unwrap();let proof=probes.join("independent_probe.rs");
            fs::write(&proof,"use receipt_proof::{price::{Item,total},receipt}; #[test] fn quantity_consumer(){let items=[Item{price:7,quantity:3},Item{price:2,quantity:1}];assert_eq!(total(&items),23);assert_eq!(receipt::format(&items),\"Items: 4; Total: 23\");}").unwrap();
            let result=Command::new("cargo").args(["test","--offline","--test","independent_probe"]).current_dir(&path).output();fs::remove_file(proof).unwrap();let _=fs::remove_dir(probes);result
        }.unwrap();
        record(&mut ck,&mut ev,"independent consumer quantity boundary",probe.status.success(),format!("{}{}",String::from_utf8_lossy(&probe.stdout),String::from_utf8_lossy(&probe.stderr)));
        record(&mut ck,&mut ev,"dirty user files and native index preserved",fs::read_to_string(path.join("README.md")).unwrap()=="user staged content must survive\nuser unstaged note\n"&&fs::read_to_string(path.join("user-notes.txt")).unwrap()=="pre-existing untracked user notes\n"&&index_content(&path)==original_index,engine);
        inspect_ui(app,&mut ck,&mut ev,files[0]).await;
        let review=value(&ipc(app,"main","read_development_review",json!({"taskId":task})).await);
        record(&mut ck,&mut ev,"initial dirty content identified",review["changes"].as_array().is_some_and(|r|r.iter().any(|r|r["path"]=="README.md"&&r["pre_existing"]==true)),review.to_string());
        fs::write(path.join("external.txt"),"written outside Agent while idle\n").unwrap();
        let external=value(&ipc(app,"main","read_development_review",json!({"taskId":task})).await);
        record(&mut ck,&mut ev,"external edit explicitly classified",external["changes"].as_array().is_some_and(|r|r.iter().any(|r|r["path"]=="external.txt"&&r["origin"]=="external")),external.to_string());
        // Mid-command interruption is an explicit user action. It cannot revert prior changes.
        ui_send(app,&mut ck,&mut ev,"Execute sleep 30 in the shell now. Do not modify files.").await;
        let deadline=tokio::time::Instant::now()+Duration::from_secs(90);let mut tool_started=false;
        while tokio::time::Instant::now()<deadline {let s=state(app,&sid).await;tool_started=s["events"].as_array().is_some_and(|es|es.iter().rev().take(12).any(|e|serde_json::to_string(e).unwrap_or_default().contains("sleep 30")&&e["kind"]!="userMessage"));if tool_started {break;}tokio::time::sleep(Duration::from_millis(300)).await;}
        record(&mut ck,&mut ev,"native command observed before interrupt",tool_started,engine);
        let r=ui(app,json!({}),r#"const b=await until(()=>btn('中断当前任务'));if(!b)throw Error('interrupt control');b.click();return {ok:true};"#).await;record(&mut ck,&mut ev,"interrupt through UI",ui_ok(&r),r.to_string());
        let _=settle(app,&sid,true,0).await;
        exercise_turn(app,&mut ck,&mut ev,&sid,"Continue after the interruption. Execute the existing tests, don't change files, then report the result.").await;
        let denied_path=fx.root.join(format!("denied-{engine}.txt"));
        let prompt=if engine=="codex" {format!("Request approval using a shell command with sandbox_permissions=require_escalated to write the word denied to {}. Do not bypass approval or use another tool. If approval is denied, stop and report denial; do not retry.",denied_path.display())} else {format!("Use the Write tool to write the word denied to {}. Do not use Bash or another tool. If permission is denied, stop and report denial; do not retry.",denied_path.display())};
        ui_send(app,&mut ck,&mut ev,&prompt).await;
        tokio::time::sleep(Duration::from_millis(700)).await;
        let denied=settle(app,&sid,false,0).await;
        record(&mut ck,&mut ev,"native approval rejected through UI",denied.as_ref().is_ok_and(|v|v["denied"]==true),format!("{denied:?}"));
        let terminal=settle(app,&sid,false,0).await;
        record(&mut ck,&mut ev,"denial reaches native terminal",terminal.as_ref().is_ok_and(|v|v["status"]=="idle"),format!("{terminal:?}"));
        record(&mut ck,&mut ev,"denied write absent from disk",!denied_path.exists(),denied_path.display());
        let r=ui(app,json!({}),r#"await click('dev-task-new');const cleared=await until(()=>!document.getElementById('agent-message')&&!document.querySelector('[aria-label=\"待处理请求\"]'));return {ok:!!cleared};"#).await;record(&mut ck,&mut ev,"task switch removes old conversation and approvals",ui_ok(&r),r.to_string());
        ui_select_task(app,&mut ck,&mut ev,&goal).await;let _=resume_ui(app,&sid).await;
        let view=state(app,&sid).await;
        saved.push(json!({"engine":engine,"sid":sid,"task":task,"project":pid,"goal":goal,"file":files[0],"native_id":view["native_id"]}));
        ui_close_run(app,&mut ck,&mut ev).await;
        let mut closed=false;for _ in 0..100 {if state(app,&sid).await["status"]=="closed" {closed=true;break;}tokio::time::sleep(Duration::from_millis(100)).await;}
        record(&mut ck,&mut ev,"UI close confirmed by host",closed,&sid);
        // An unavailable model exercises a real CLI failure without changing native settings.
        let before=ids(&sessions_of(app).await);
        let mut restore_preferences=None;
        let pid_file=fx.root.join("claude-failure.pid");
        if engine=="claude" {
            use std::os::unix::fs::PermissionsExt;
            let native=dgos_agents::session::resolve_executable("claude",None).unwrap();
            let launcher=fx.root.join("claude-failure-launcher");
            let quote=|s:&str|format!("'{}'",s.replace('\'',"'\\''"));
            fs::write(&launcher,format!("#!/bin/sh\nprintf '%s' \"$$\" > {}\nexec {} \"$@\"\n",quote(&pid_file.to_string_lossy()),quote(&native.to_string_lossy()))).unwrap();
            fs::set_permissions(&launcher,fs::Permissions::from_mode(0o700)).unwrap();
            let prefs=ipc(app,"main","get_cli_preferences",json!({})).await.data().clone();
            let mut paths=prefs["paths"].clone();paths["claude"]=json!(launcher);
            let result=ipc(app,"main","set_cli_preferences",json!({"paths":paths,"expectedRevision":prefs["revision"]})).await;
            record(&mut ck,&mut ev,"isolated real CLI launcher configured",result.success()&&result.data()["saved"]==true,result.describe());
            restore_preferences=Some(prefs);
        }
        let r=ui(app,json!({"model":if engine=="codex"{"dgos-intentionally-missing-model"}else{""}}),r#"const f=$$('input').find(e=>labelOf(e).startsWith('模型'));if(!f)throw Error('model field');setInput(f,A.model);return {ok:true};"#).await;record(&mut ck,&mut ev,"explicit failure configuration selected",ui_ok(&r),r.to_string());
        let r=ui(app,json!({"engine":engine}),r#"const start=await until(()=>{const b=tid('dev-session-new-'+A.engine);return b&&!b.disabled&&b});if(!start)throw Error('new session disabled');start.click();const outcome=await until(()=>document.querySelector('[role=alert]')||document.getElementById('agent-message'));return {ok:!!outcome,text:txt(outcome)};"#).await;
        record(&mut ck,&mut ev,"CLI failure attempt returned visible outcome",ui_ok(&r),r.to_string());
        if let Some(failed_sid)=new_session_id(app,&before).await {
            ui_send(app,&mut ck,&mut ev,if engine=="claude" {"Execute sleep 30, then report the result."} else {"Reply with HELLO."}).await;
            tokio::time::sleep(Duration::from_millis(700)).await;
            if engine=="claude" {
                let pid=fs::read_to_string(&pid_file).unwrap().parse::<i32>().unwrap();
                let killed=Command::new("/bin/kill").args(["-KILL",&pid.to_string()]).status().is_ok_and(|status|status.success());
                record(&mut ck,&mut ev,"real Claude CLI process terminated",killed,pid);
            }
            let failure=settle(app,&failed_sid,true,0).await;
            record(&mut ck,&mut ev,"real CLI error retained",failure.as_ref().is_ok_and(|v|v["status"]=="error"||v["status"]=="unknown"),format!("{failure:?}"));
            let r=ui(app,json!({}),r#"const d=await until(()=>document.querySelector('[data-run-status=failed], [role=alert]'));return {ok:!!d,text:txt(d)};"#).await;record(&mut ck,&mut ev,"CLI failure visible in interface",ui_ok(&r),r.to_string());
            ui_close_run(app,&mut ck,&mut ev).await;
            for _ in 0..100 {if state(app,&failed_sid).await["status"]=="closed" {break;}tokio::time::sleep(Duration::from_millis(100)).await;}
        } else {
            let r=ui(app,json!({}),r#"const d=await until(()=>document.querySelector('[role=alert]'));return {ok:!!d,text:txt(d)};"#).await;
            record(&mut ck,&mut ev,"CLI failure startup visible",ui_ok(&r),r.to_string());
        }
        if let Some(prefs)=restore_preferences {
            let current=ipc(app,"main","get_cli_preferences",json!({})).await.data().clone();
            let result=ipc(app,"main","set_cli_preferences",json!({"paths":prefs["paths"],"expectedRevision":current["revision"]})).await;
            record(&mut ck,&mut ev,"isolated CLI preferences restored",result.success()&&result.data()["saved"]==true,result.describe());
        }
        let _=ui(app,json!({}),r#"const f=$$('input').find(e=>labelOf(e).startsWith('模型'));if(f)setInput(f,'');await new Promise(r=>setTimeout(r,100));return {ok:true};"#).await;
    }
    fs::write(fx.root.join("stage2-evidence.json"),serde_json::to_vec_pretty(&saved).unwrap()).unwrap();
    finish(app,fx,ck,ev,"first").await;
}
