//! Stage6 production DOM journey. All Git writes target repositories under Fx.root.
//! DOM controls run each full dual-engine, delivery and restart criterion in order. A failed
//! prerequisite stops the journey; checkpoints and real model/process snapshots survive exit.
//! No fixture messages or direct delivery IPC writes: DOM actions invoke production handlers.
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn write_json(path: impl AsRef<Path>, data: &Value) { fs::write(path, serde_json::to_vec_pretty(data).unwrap()).unwrap(); }
fn command(path: &Path, args: &[&str]) -> String {
    let o=Command::new("git").arg("-C").arg(path).args(["-c","core.hooksPath=/dev/null","-c","commit.gpgsign=false"]).args(args).output().unwrap();
    assert!(o.status.success(), "acceptance git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().into()
}
type Journey<T> = Result<T, String>;

fn checkpoint(app: &AppHandle, ck: &Checks, ev: &[Value], name: &str, state: &str) {
    let fx = app.state::<Fx>();
    write_json(fx.root.join("stage6-checkpoint.json"), &json!({"step":name,"state":state,"passed":ck.passed,"failed":ck.failed,"checks":ev}));
    println!("STAGE6 {state}: {name}");
}

fn require(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, ok: bool, detail: impl Display) -> Journey<()> {
    record(ck, ev, name, ok, detail);
    checkpoint(app, ck, ev, name, if ok { "complete" } else { "failed" });
    if ok { Ok(()) } else { Err(name.into()) }
}

const HELPERS: &str=r#"
const pause=()=>new Promise(r=>setTimeout(r,50));
const select=(e,v)=>{if(!e||![...e.options].some(o=>o.value===v))throw Error('select option absent '+v);Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(e,v);e.dispatchEvent(new Event('change',{bubbles:true}));};
const field=(root,label)=>{const f=$$('input,textarea,select',root).find(e=>labelOf(e)===label);if(!f)throw Error('field absent '+label);return f};
const button=async(root,label)=>{const b=await until(()=>$$('button',root).find(e=>txt(e)===label&&!e.disabled));if(!b)throw Error('button absent/disabled '+label+' '+txt(root));b.click();await pause();};
const check=(root,label)=>{const l=$$('label',root).find(e=>{const c=e.querySelector('input');return c&&labelOf(c)===label;});if(!l)throw Error('checkbox '+label);const c=l.querySelector('input');if(!c.checked)c.click();};
const delivery=()=>tid('dev-delivery');
const settled=async()=>{if(!await until(()=>delivery()&&!$$('[role=status]',delivery()).some(e=>txt(e)==='读取中…')))throw Error('delivery busy');await pause();};
const operation=()=>$$('[role=status]',delivery()).map(txt).find(t=>t.startsWith('已完成 ·'));
const completed=async(before)=>{window.__stage6Receipt={before,started:Date.now(),phase:'new operation'};if(!await until(()=>{const current=operation();window.__stage6Receipt.current=current;return current&&current!==before;}))return false;window.__stage6Receipt.phase='refresh settled';await settled();window.__stage6Receipt.phase='refresh enabled';const ready=!!await until(()=>$$('button',delivery()).some(b=>txt(b)==='刷新'&&!b.disabled));window.__stage6Receipt.phase=ready?'complete':'refresh timeout';window.__stage6Receipt.finished=Date.now();return ready;};
const confirm=async(label)=>{const d=await until(()=>$$('[role=alertdialog],[role=dialog]').find(e=>$$('button',e).some(b=>txt(b)===label)));if(!d)throw Error('confirmation absent '+label);await button(d,label);};
"#;
async fn step(app:&AppHandle,ck:&mut Checks,ev:&mut Vec<Value>,name:&str,args:Value,body:&str)->Journey<Value> {
    checkpoint(app,ck,ev,name,"running");
    let r=ui(app,args,&format!("{HELPERS}\ntry {{ {body} }} catch(error) {{ return {{ok:false,error:String(error),activeTabs:$$('[role=tab][data-state=active]').map(txt),dialogs:$$('[role=alertdialog],[role=dialog]').map(txt),alerts:$$('[role=alert]').map(txt),buttons:$$('button').map(b=>({{text:txt(b),disabled:b.disabled}}))}}; }}")).await;
    require(app,ck,ev,name,ui_ok(&r),r.to_string())?;
    Ok(r)
}
async fn tab(app:&AppHandle,ck:&mut Checks,ev:&mut Vec<Value>,name:&str)->Journey<()> {
    step(app,ck,ev,&format!("activate visible {name} review tab"),json!({"name":name}),r#"const t=await need('dev-review-tab-'+A.name);t.focus();t.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));return {ok:!!await until(()=>{const current=tid('dev-review-tab-'+A.name);return current?.getAttribute('data-state')==='active'&&visible(document.getElementById(current.getAttribute('aria-controls')));})};"#).await?;
    Ok(())
}
async fn selection(app:&AppHandle)->Value{value(&ipc(app,"main","read_development_workspace",json!({})).await)["selection"].clone()}
async fn screenshot(fx:&Fx,name:&str){let path=fx.root.join(format!("{name}.png"));let status=Command::new("/usr/sbin/screencapture").args(["-x",path.to_str().unwrap()]).status();println!("STAGE6 screenshot {name}: {status:?}");}
async fn edit(app:&AppHandle,ck:&mut Checks,ev:&mut Vec<Value>,path:&Path,text:&str)->Journey<()> {
    tab(app,ck,ev,"files").await?;
    let expected=fs::read_to_string(path).map_err(|e|format!("editor prerequisite {}: {e}",path.display()))?;
    step(app,ck,ev,"open task-bound file via production tree",json!({"path":path.file_name().unwrap().to_str().unwrap(),"expected":expected}),r#"await fill('dev-file-search',A.path);await pause();await click('dev-file-item-'+A.path);return {ok:!!await until(()=>{const source=tid('dev-file-source'),editor=tid('dev-file-editor');return txt(source?.querySelector('strong'))===A.path&&editor?.value===A.expected&&!$$('[role=alert]',source).length&&editor;})};"#).await?;
    step(app,ck,ev,"manual edit and save through production editor",json!({"text":text}),r#"await fill('dev-file-editor',A.text);const save=await until(()=>{const b=tid('dev-file-save');return b&&!b.disabled&&b});if(!save)throw Error('editor save disabled');save.click();return {ok:!!await until(()=>tid('dev-file-editor')?.value===A.text&&$$('[role=status]',tid('dev-file-source')).some(e=>txt(e)==='没有未保存修改')&&!$$('[role=alert]',tid('dev-file-source')).length)};"#).await?;
    require(app,ck,ev,"saved bytes match task worktree",fs::read_to_string(path).unwrap_or_default()==text,path.display())
}
async fn close(app:&AppHandle,ck:&mut Checks,ev:&mut Vec<Value>,sid:&str)->Journey<()> {
    step(app,ck,ev,"close selected native runtime through visible conversation",json!({}),r#"const root=await until(()=>$$('.agent-conversation').find(e=>e.querySelector('#agent-message')));if(!root)throw Error('conversation absent');await button(root,'关闭运行');return {ok:true};"#).await?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(15);
    loop {
        let view=value(&ipc(app,"main","get_agent_session",json!({"sessionId":sid})).await);
        if view["status"]=="closed" { return require(app,ck,ev,"UI close confirmed by host",true,sid); }
        if tokio::time::Instant::now()>=deadline { return require(app,ck,ev,"UI close confirmed by host",false,view.to_string()); }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}
async fn resume(app:&AppHandle,ck:&mut Checks,ev:&mut Vec<Value>,sid:&str)->Journey<()> {
    step(app,ck,ev,"explicit resume through selected visible session row",json!({"sid":sid}),r#"await need('dev-session-item-'+A.sid);await pause();const t=await need('dev-trust');if(!t.checked)t.click();await pause();const b=await until(()=>{const trust=tid('dev-trust');if(trust&&!trust.checked)trust.click();const button=tid('dev-session-item-'+A.sid)?.querySelector('button');return button&&!button.disabled&&button;});if(!b)throw Error('resume disabled');b.click();return {ok:!!await until(()=>tid('dev-session-item-'+A.sid)?.querySelector('[aria-current=true]')&&visible(document.getElementById('agent-message')))};"#).await?;
    Ok(())
}
async fn turn(app:&AppHandle,ck:&mut Checks,ev:&mut Vec<Value>,sid:&str,name:&str,prompt:&str)->Journey<Value> {
    let before=value(&ipc(app,"main","get_agent_session",json!({"sessionId":sid})).await);
    let previous_runs:std::collections::HashSet<String>=before["events"].as_array().into_iter().flatten().filter_map(|e|e["run"].as_str().map(str::to_owned)).collect();
    let sent=ui_send(app,ck,ev,prompt).await;
    if !sent { return Err(format!("{name}: prompt not sent")); }
    checkpoint(app,ck,ev,name,"waiting for real native turn");
    let deadline=tokio::time::Instant::now()+Duration::from_secs(300);
    loop {
        let reply=ipc(app,"main","get_agent_session",json!({"sessionId":sid})).await;
        let view=value(&reply);
        write_json(app.state::<Fx>().root.join(format!("{name}.json")),&view);
        if !succeeded(&reply) { require(app,ck,ev,name,false,reply.describe())?; }
        if view["pending"].as_array().is_some_and(|p|!p.is_empty()) {
            step(app,ck,ev,"approve real native request through visible conversation",json!({}),r#"const root=$$('.agent-conversation').find(e=>e.querySelector('#agent-message'));if(!root)throw Error('conversation absent');await button(root,'允许本次');return {ok:true};"#).await?;
        }
        let terminal=view["events"].as_array().is_some_and(|events|events.iter().any(|e|e["run"].as_str().is_some_and(|run|!previous_runs.contains(run))&&matches!(e["kind"].as_str(),Some("turn/completed"|"result"|"failure"|"error"))));
        let status=view["status"].as_str().unwrap_or("");
        if matches!(status,"error"|"unknown"|"closed")||(status=="idle"&&terminal) {
            require(app,ck,ev,name,status=="idle"&&terminal,view["status"].to_string())?;
            return Ok(view);
        }
        if tokio::time::Instant::now()>=deadline { require(app,ck,ev,name,false,format!("native turn timeout; last snapshot retained in {name}.json"))?; }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}
async fn host(app:&AppHandle,ck:&mut Checks,ev:&mut Vec<Value>,task:&str,program:&str,args:&[&str],service:Option<u16>)->Journey<Value>{
    tab(app,ck,ev,"develop").await?;
    let before=value(&ipc(app,"main","list_development_processes",json!({"taskId":task})).await);
    step(app,ck,ev,"configure and start actual host command through form",json!({"program":program,"args":args,"port":service}),r#"
const s=await need('dev-development-runtime');select(tid('dev-process-kind'),A.port?'service':'command');await pause();
while($$('button[aria-label^="移除参数"]',s).length){$$('button[aria-label^="移除参数"]',s)[0].click();await pause();}
await fill('dev-process-program',A.program);
for(let i=0;i<A.args.length;i++){await button(s,'添加参数');const f=await until(()=>$$('input',s).find(e=>labelOf(e)==='参数 '+(i+1)));if(!f)throw Error('argument field');setInput(f,A.args[i]);await pause();}
if(A.port){setInput(field(s,'回环端口'),String(A.port));setInput(field(s,'HTTP 路径'),'/health');}else{const l=$$('label',s).find(e=>txt(e).includes('验证')&&e.querySelector('input[type=checkbox]'));if(l&&!l.querySelector('input').checked)l.querySelector('input').click();}
if(!tid('dev-process-trust').checked)tid('dev-process-trust').click();await pause();const start=await until(()=>{const b=tid('dev-process-start');return b&&!b.disabled&&b});if(!start)throw Error('process start disabled');start.click();return {ok:true};"#).await?;
    let old=before.as_array().cloned().unwrap_or_default();let deadline=tokio::time::Instant::now()+Duration::from_secs(120);
    loop {
        let rows=value(&ipc(app,"main","list_development_processes",json!({"taskId":task})).await);
        if let Some(v)=rows.as_array().into_iter().flatten().find(|v|!old.iter().any(|o|o["id"]==v["id"])&&v["source"].as_str().is_some_and(|s|s.starts_with("host_"))) {
            let v=value(&ipc(app,"main","read_development_process",json!({"id":v["id"]})).await);
            write_json(app.state::<Fx>().root.join("stage6-host-process.json"),&v);
            if v["finished_at"].is_number()||v["status"]=="ready" {
                step(app,ck,ev,"select actual host process in production history",json!({"id":v["id"]}),r#"await click('dev-process-history-'+A.id);return {ok:!!await until(()=>txt(tid('dev-process-detail')).includes(A.id))};"#).await?;
                return Ok(v);
            }
        }
        if tokio::time::Instant::now()>=deadline { require(app,ck,ev,"actual host command reaches terminal or ready state",false,rows.to_string())?; }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}
async fn git_action(app:&AppHandle,ck:&mut Checks,ev:&mut Vec<Value>,label:&str,paths:&[&str],message:&str)->Journey<()> {
    tab(app,ck,ev,"delivery").await?;
    step(app,ck,ev,&format!("delivery DOM {label}"),json!({"label":label,"paths":paths,"message":message}),r#"
window.__stage6Action='waiting delivery';await need('dev-delivery');await settled();const d=delivery(),before=operation();
window.__stage6Action='clearing selection';for(const l of $$('label',d)){const c=l.querySelector('input[type=checkbox]');if(c&&/ \(/.test(txt(l))&&c.checked)c.click();}await pause();
for(const path of A.paths){window.__stage6Action='selecting '+path;const get=()=>$$('label',delivery()).find(e=>e.querySelector('input[type=checkbox]')&&txt(e).startsWith(path+' ('))?.querySelector('input');const c=await until(get,3000);if(!c)throw Error('changed path absent '+path+' labels='+$$('label',delivery()).map(txt).join('|'));if(!c.checked)c.click();await pause();if(!get()?.checked)throw Error('path selection lost '+path);}
if(A.label==='提交已审查索引'){setInput(field(d,'提交信息'),A.message);check(d,'我已审查完整索引并确认本次提交的确切内容');}
if(A.label==='推送当前分支')setInput(field(d,'已配置的远端名'),'origin');
window.__stage6Action='settling selection';await settled();await pause();for(const path of A.paths){const c=$$('label',delivery()).find(e=>txt(e).startsWith(path+' ('))?.querySelector('input');if(!c?.checked)throw Error('selection reset before action '+path);}window.__stage6Action='clicking '+A.label;await button(delivery(),A.label);window.__stage6Action='waiting receipt';
if(A.label==='推送当前分支')await confirm(A.label);
return {ok:await completed(before),alerts:$$('[role=alert]',delivery()).map(txt)};"#).await?;
    Ok(())
}
fn prepare(root:&Path,engine:&str)->(PathBuf,String){
    let p=root.join(format!("original-{engine}"));fs::create_dir_all(&p).unwrap();command(&p,&["init","-b","main"]);command(&p,&["config","user.name","Stage6 Acceptance"]);command(&p,&["config","user.email","stage6@example.invalid"]);
    for(name,text)in[("math.cjs","exports.total=(items)=>items.reduce((n,i)=>n+i.price,0);\n"),("receipt.cjs","const {total}=require('./math.cjs');exports.receipt=items=>'Total: '+total(items);\n"),("test.cjs","const {test}=require('node:test'),a=require('node:assert/strict'),{total}=require('./math.cjs'),{receipt}=require('./receipt.cjs');test('basic',()=>{a.equal(total([{price:3}]),3);a.equal(receipt([{price:3}]),'Total: 3')});\n"),("package.json","{\"scripts\":{\"test\":\"node --test test.cjs\",\"build\":\"node --check math.cjs && node --check receipt.cjs\"}}\n"),("service.cjs","const http=require('node:http');const {receipt}=require('./receipt.cjs');http.createServer((q,s)=>s.end(receipt([{price:7,quantity:3},{price:2}]))).listen(Number(process.argv[2]),'127.0.0.1');\n"),("README.md","base\n"),(".gitignore","*.marker\n")]{fs::write(p.join(name),text).unwrap();}
    command(&p,&["add","--","math.cjs","receipt.cjs","test.cjs","package.json","service.cjs","README.md",".gitignore"]);command(&p,&["commit","-m","initial project"]);let base=command(&p,&["rev-parse","HEAD"]);
    command(root,&["init","--bare",&format!("remote-{engine}.git")]);command(&p,&["remote","add","origin",root.join(format!("remote-{engine}.git")).to_str().unwrap()]);
    fs::write(p.join("README.md"),"user staged\n").unwrap();command(&p,&["add","--","README.md"]);fs::write(p.join("README.md"),"user staged\nuser unstaged\n").unwrap();fs::write(p.join("user-notes.txt"),"user notes\n").unwrap();(p,base)
}
async fn configure(app:&AppHandle,ck:&mut Checks,ev:&mut Vec<Value>,engine:&str)->Journey<()> {
    tab(app,ck,ev,"capabilities").await?;
    let resolver=app.state::<Arc<SessionResolver>>();let(session,_)=resolver.agent_session().map_err(|_|"account_session_missing")?;let owner=session.user_id().to_owned();
    let manager=app.state::<dgos_providers::ProviderManager>();let vault=app.state::<Arc<dgos_providers::test_support::MemoryCredentialVault>>();
    let prefix=format!("DGOS_STAGE6_{}",engine.to_uppercase());let endpoint=std::env::var(format!("{prefix}_URL")).map_err(|_|"explicit_stage6_provider_required")?;let key=std::env::var(format!("{prefix}_KEY")).map_err(|_|"explicit_stage6_key_required")?;let model=std::env::var(format!("{prefix}_MODEL")).map_err(|_|"explicit_stage6_model_required")?;
    let provider=dgos_providers::test_support::seed_provider(app.state::<Fx>().db.clone(),vault.inner(),dgos_types::ProviderConfig{name:format!("Stage6 {engine}"),provider_type:if engine=="codex"{"openai_compatible"}else{"anthropic"}.into(),base_url:endpoint,api_key:key,enabled:true,config:None},&owner).await.map_err(|_|"stage6_provider_seed_failed")?;
    manager.add_manual_model(&provider.id,&owner,&model,None).await.map_err(|_|"stage6_model_seed_failed")?;
    step(app,ck,ev,"save explicit Provider project revision through UI",json!({"engine":engine,"provider":provider.id,"model":model}),r#"const s=await until(()=>$$('section[aria-label="项目能力"]')[0]);if(!s)throw Error('Provider section absent');const c=$$('[role=combobox]',s).find(e=>labelOf(e)==='智能体');if(!c)throw Error('engine selector absent');c.click();const o=await until(()=>$$('[role=option]').find(e=>txt(e)===(A.engine==='codex'?'Codex':'Claude')));o.click();const p=await until(()=>$$('select',s).find(e=>[...e.options].some(o=>o.value===A.provider)));if(!p)throw Error('explicit Provider absent');Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(p,A.provider);p.dispatchEvent(new Event('change',{bubbles:true}));await until(()=>$$('select',s).some(e=>e.value===A.model));await button(s,'保存新修订');return {ok:!!await until(()=>txt(s).includes('已保存'))};"#).await?;
    Ok(())
}

async fn per_engine(app:&AppHandle,fx:&Fx,ck:&mut Checks,ev:&mut Vec<Value>,engine:&str)->Journey<Value> {
    checkpoint(app,ck,ev,engine,"preparing isolated fixture");
    let(original,base)=prepare(&fx.root,engine);
    let index=command(&original,&["write-tree"]);
    if !ui_import_project(app,ck,ev,&format!("Stage6 {engine}"),&original).await { return Err(format!("{engine}: project import")); }
    let goal=format!("Stage6 complete {engine}");
    if !ui_new_task(app,ck,ev,&goal).await { return Err(format!("{engine}: task creation")); }
    let sel=selection(app).await;
    let task=sel["task_id"].as_str().ok_or("selected task missing")?;
    let project=sel["project_id"].as_str().ok_or("selected project missing")?;
    let work=fx.root.join(format!("work-{engine}"));
    tab(app,ck,ev,"delivery").await?;
    step(app,ck,ev,"create task worktree using production delivery form",json!({"path":work,"base":base,"branch":format!("delivery-{engine}")}),r#"const d=await need('dev-delivery');await settled();setInput(field(d,'目录'),A.path);setInput(field(d,'新分支'),A.branch);setInput(field(d,'基准提交哈希'),A.base);await pause();await button(d,'创建 worktree');return {ok:!!await until(()=>txt(tid('dev-session-cwd')).includes(A.path)),alerts:$$('[role=alert]',d).map(txt)};"#).await?;
    require(app,ck,ev,"created task worktree exists before file operations",work.join("math.cjs").is_file(),work.display())?;
    configure(app,ck,ev,engine).await?;
    let before=ids(&sessions_of(app).await);
    if !ui_start_session(app,ck,ev,engine).await { return Err(format!("{engine}: session startup")); }
    let sid=new_session_id(app,&before).await.ok_or("new session absent")?;
    turn(app,ck,ev,&sid,&format!("{engine}-warmup"),"Hello. Acknowledge readiness for a separate project development request. Do not change files yet.").await?;
    let source=turn(app,ck,ev,&sid,&format!("{engine}-feature"),"Implement quantity support across math.cjs and receipt.cjs. total sums price * quantity with default quantity 1; receipt must return exactly 'Items: 4; Total: 23' for [{price:7,quantity:3},{price:2}]. Update meaningful tests in test.cjs, run npm test and npm run build. Preserve README.md and service.cjs. Do not write Git or install dependencies. Implement now.").await?;
    let changed=command(&work,&["diff","--name-only",&base,"--","math.cjs","receipt.cjs"]);
    let consumer=Command::new("node").args(["-e","const a=require('node:assert/strict'),m=require('./math.cjs'),r=require('./receipt.cjs');a.equal(m.total([{price:7,quantity:3},{price:2}]),23);a.equal(m.total([{price:7,quantity:0}]),0);a.equal(m.total([]),0);a.equal(r.receipt([{price:7,quantity:3},{price:2}]),'Items: 4; Total: 23');"]).current_dir(&work).output().map_err(|e|e.to_string())?;
    require(app,ck,ev,"actual cross-file feature consumer behavior",["math.cjs","receipt.cjs"].iter().all(|p|changed.lines().any(|line|line==*p))&&consumer.status.success(),format!("{engine}: {changed}; consumer={:?}; {}",consumer.status,String::from_utf8_lossy(&consumer.stderr)))?;
    close(app,ck,ev,&sid).await?;
    edit(app,ck,ev,&work.join("math.cjs"),"exports.total=()=>-999;\n").await?;
    let failed=host(app,ck,ev,task,"npm",&["test"],None).await?;
    require(app,ck,ev,"manual regression has actual nonzero test exit",failed["exit_code"].as_i64().is_some_and(|n|n!=0),failed.to_string())?;
    resume(app,ck,ev,&sid).await?;
    turn(app,ck,ev,&sid,&format!("{engine}-repair"),"The user manually saved a regression in math.cjs and npm test failed. Run npm test to see the failure, repair total's quantity support without weakening tests, then run npm test and npm run build. Do not write Git.").await?;
    close(app,ck,ev,&sid).await?;
    for args in [&["test"][..],&["run","build"][..]] {
        let r=host(app,ck,ev,task,"npm",args,None).await?;
        require(app,ck,ev,"repaired project real test/build exit zero",r["exit_code"]==0,r.to_string())?;
    }
    let port=std::net::TcpListener::bind("127.0.0.1:0").map_err(|e|e.to_string())?.local_addr().map_err(|e|e.to_string())?.port();
    let port_s=port.to_string();
    let service=host(app,ck,ev,task,"node",&["service.cjs",&port_s],Some(port)).await?;
    require(app,ck,ev,"actual project service reaches ready state",service["status"]=="ready",service.to_string())?;
    let response=tokio::time::timeout(Duration::from_secs(10),async {
        let mut socket=tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST,port)).await?;
        socket.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await?;
        let mut b=vec![];socket.read_to_end(&mut b).await?;
        Ok::<_,std::io::Error>(String::from_utf8_lossy(&b).into_owned())
    }).await;
    require(app,ck,ev,"actual project service HTTP consumer response",response.as_ref().is_ok_and(|r|r.as_ref().is_ok_and(|r|r.contains("Items: 4; Total: 23"))),format!("{response:?}"))?;
    step(app,ck,ev,"open ready preview through production UI",json!({}),r#"const b=await until(()=>{const b=tid('dev-service-preview');return b&&!b.disabled&&b});if(!b)throw Error('preview disabled');b.click();return {ok:true};"#).await?;
    step(app,ck,ev,"stop owned service through production UI",json!({}),r#"const b=await until(()=>{const b=tid('dev-process-stop');return b&&!b.disabled&&b});if(!b)throw Error('stop disabled');b.click();return {ok:!!await until(()=>tid('dev-process-stop')?.disabled&&!$$('[role=status]',tid('dev-process-detail')).some(e=>txt(e)==='已就绪'))};"#).await?;
    tab(app,ck,ev,"delivery").await?;
    screenshot(fx,&format!("{engine}-delivery-review")).await;
    git_action(app,ck,ev,"暂存所选文件",&["math.cjs","receipt.cjs","test.cjs"],"").await?;
    git_action(app,ck,ev,"取消暂存所选文件",&["math.cjs"],"").await?;
    git_action(app,ck,ev,"暂存所选文件",&["math.cjs"],"").await?;
    git_action(app,ck,ev,"提交已审查索引",&[],"Deliver quantity support").await?;
    git_action(app,ck,ev,"推送当前分支",&[],"").await?;
    let oid=command(&work,&["rev-parse","HEAD"]);
    let remote=command(&fx.root.join(format!("remote-{engine}.git")),&["rev-parse",&format!("refs/heads/delivery-{engine}")]);
    require(app,ck,ev,"UI commit and push verified by Git objects and bare reference",oid==remote,oid)?;
    require(app,ck,ev,"original staged unstaged and untracked user bytes preserved",command(&original,&["write-tree"])==index&&fs::read_to_string(original.join("README.md")).unwrap_or_default()=="user staged\nuser unstaged\n"&&fs::read_to_string(original.join("user-notes.txt")).unwrap_or_default()=="user notes\n",original.display())?;
    let target=if engine=="codex" { "claude" } else { "codex" };
    let before=ids(&sessions_of(app).await);
    step(app,ck,ev,"prepare cross-engine handoff using production controls",json!({"sid":sid}),r#"const d=await need('dev-delivery');select(field(d,'来源会话'),A.sid);await pause();setInput(field(d,'剩余问题与续作要求'),'Add exports.difference=(a,b)=>a-b to math.cjs and a meaningful consumer test difference(9,4)==5 to test.cjs. Run npm test and npm run build. Preserve quantity support. No Git writes.');await pause();await button(d,'生成交接摘要');return {ok:!!await until(()=>$$('select',d).some(e=>labelOf(e)==='目标新会话能力配置'))};"#).await?;
    step(app,ck,ev,"review and accept cross-engine handoff using production controls",json!({}),r#"const d=await need('dev-delivery');select(field(d,'目标新会话能力配置'),'0');await pause();check(d,'我信任新会话的目标目录');await pause();await button(d,'创建带此草稿的新会话');return {ok:!!await until(()=>visible(document.getElementById('agent-message'))&&document.getElementById('agent-message').value.includes('difference'))};"#).await?;
    let target_sid=new_session_id(app,&before).await.ok_or("handoff receiving session absent")?;
    let sessions=sessions_of(app).await;
    require(app,ck,ev,"handoff receiver belongs to task and opposite engine",sessions.iter().any(|s|s["id"]==target_sid&&s["task_id"]==task&&s["engine"]==target),&target_sid)?;
    let summary=step(app,ck,ev,"read reviewed handoff draft from visible composer",json!({}),r#"const c=document.getElementById('agent-message');return {ok:visible(c)&&c.value.includes('difference'),text:c?.value};"#).await?;
    let summary=summary["value"]["text"].as_str().ok_or("handoff summary absent")?;
    let v=turn(app,ck,ev,&target_sid,&format!("{engine}-handoff"),summary).await?;
    require(app,ck,ev,"handoff receiver uses distinct native history",v["native_id"].is_string()&&source["native_id"].is_string()&&v["native_id"]!=source["native_id"],format!("source={}; target={}",source["native_id"],v["native_id"]))?;
    close(app,ck,ev,&target_sid).await?;
    let r=host(app,ck,ev,task,"node",&["-e","const a=require('node:assert/strict'),m=require('./math.cjs');a.equal(m.difference(9,4),5);a.equal(m.total([{price:7,quantity:3},{price:2}]),23)"],None).await?;
    require(app,ck,ev,"cross-engine receiving edits independently verified",r["exit_code"]==0,r.to_string())?;
    Ok(json!({"engine":engine,"project":project,"task":task,"goal":goal,"path":work,"source":sid,"sid":target_sid,"target":target,"native_id":v["native_id"]}))
}

async fn delivery_boundaries(app:&AppHandle,fx:&Fx,ck:&mut Checks,ev:&mut Vec<Value>)->Journey<()> {
    let (original,_)=prepare(&fx.root,"conflict");
    // Independent fixture branches are created before DG-OS imports the repository.
    let clean=fx.root.join("conflict-seed");command(&original,&["worktree","add","-b","conflict-ours",clean.to_str().unwrap(),"HEAD"]);
    fs::write(clean.join("conflict.txt"),"base\n").unwrap();command(&clean,&["add","--","conflict.txt"]);command(&clean,&["commit","-m","conflict base"]);let base=command(&clean,&["rev-parse","HEAD"]);
    command(&clean,&["switch","-c","conflict-theirs"]);fs::write(clean.join("conflict.txt"),"theirs\n").unwrap();command(&clean,&["add","--","conflict.txt"]);command(&clean,&["commit","-m","theirs"]);command(&clean,&["switch","conflict-ours"]);fs::write(clean.join("conflict.txt"),"ours\n").unwrap();command(&clean,&["add","--","conflict.txt"]);command(&clean,&["commit","-m","ours"]);let ours=command(&clean,&["rev-parse","HEAD"]);
    if !ui_import_project(app,ck,ev,"Stage6 Git boundaries",&original).await { return Err("conflict project import".into()); }
    if !ui_new_task(app,ck,ev,"Stage6 conflict and safe cleanup").await { return Err("conflict task creation".into()); }
    let work=fx.root.join("conflict-work");tab(app,ck,ev,"delivery").await?;
    step(app,ck,ev,"create conflict task worktree through UI",json!({"path":work,"base":ours}),r#"const d=await need('dev-delivery');await settled();setInput(field(d,'目录'),A.path);setInput(field(d,'新分支'),'conflict-delivery');setInput(field(d,'基准提交哈希'),A.base);await pause();await button(d,'创建 worktree');return {ok:!!await until(()=>txt(tid('dev-session-cwd')).includes(A.path))};"#).await?;
    require(app,ck,ev,"conflict worktree exists before file operations",work.join("README.md").is_file(),work.display())?;
    edit(app,ck,ev,&work.join("README.md"),"host-owned temporary change\n").await?;
    tab(app,ck,ev,"delivery").await?;
    step(app,ck,ev,"safe restore explicit host-owned file through UI",json!({}),r#"const d=await need('dev-delivery');await settled();const before=operation();const l=await until(()=>$$('label',d).find(e=>txt(e).startsWith('README.md (')));if(!l)throw Error('restore path absent');if(!l.querySelector('input').checked)l.querySelector('input').click();await pause();await button(d,'安全恢复所选文件');await confirm('恢复所选文件');return {ok:await completed(before)};"#).await?;
    require(app,ck,ev,"safe restore preserves actual baseline and index",fs::read_to_string(work.join("README.md")).unwrap_or_default()=="base\n"&&command(&work,&["diff","--cached"]).is_empty(),&base)?;
    step(app,ck,ev,"merge conflict preserved and rendered through UI",json!({}),r#"const d=await need('dev-delivery');const f=$$('input',d).filter(e=>labelOf(e)==='新分支').at(-1);if(!f)throw Error('merge branch field absent');setInput(f,'conflict-theirs');await pause();await button(d,'合并分支');return {ok:!!await until(()=>txt(delivery()).includes('未合并冲突')&&txt(delivery()).includes('阶段 3'))};"#).await?;
    let stages=command(&work,&["ls-files","-u"]);
    require(app,ck,ev,"real Git conflict stages retained",[" 1\tconflict.txt"," 2\tconflict.txt"," 3\tconflict.txt"].iter().all(|s|stages.contains(s)),stages)?;
    screenshot(fx,"conflict-three-stages").await;
    edit(app,ck,ev,&work.join("conflict.txt"),"resolved both sides\n").await?;
    git_action(app,ck,ev,"暂存所选文件",&["conflict.txt"],"").await?;
    git_action(app,ck,ev,"提交已审查索引",&[],"Resolve actual merge conflict").await?;
    let parents=command(&work,&["rev-list","--parents","-n","1","HEAD"]);
    require(app,ck,ev,"resolution produces actual two-parent commit",parents.split_whitespace().count()==3,parents)?;
    step(app,ck,ev,"mark task complete through UI",json!({}),r#"const f=await need('dev-task-form');const c=$$('[role=combobox]',f)[0];if(!c)throw Error('task status absent');c.click();const o=await until(()=>$$('[role=option]').find(e=>txt(e)==='已完成'));if(!o)throw Error('completed option absent');o.click();await pause();await click('dev-task-save');return {ok:!!await until(()=>!tid('dev-task-save')?.disabled)};"#).await?;
    tab(app,ck,ev,"delivery").await?;
    step(app,ck,ev,"safe managed worktree cleanup through UI",json!({"path":work}),r#"const d=await need('dev-delivery');await settled();const before=operation();setInput(field(d,'移除 worktree 时保留成果的分支'),'conflict-delivery');await pause();await button(d,'移除工作区');await confirm('移除工作区');return {ok:await completed(before)&&txt(tid('dev-session-cwd')).includes(A.path)&&$$('[role=alert]',delivery()).some(e=>txt(e)==='目录不存在或不可用，请重新定位。')};"#).await?;
    require(app,ck,ev,"removed worktree retains committed branch and user original",!work.exists()&&command(&original,&["rev-parse","refs/heads/conflict-delivery"]).len()==40&&fs::read_to_string(original.join("README.md")).unwrap_or_default()=="user staged\nuser unstaged\n",original.display())
}

async fn restart_journey(app:&AppHandle,fx:&Fx,ck:&mut Checks,ev:&mut Vec<Value>)->Journey<()> {
    let bytes=fs::read(fx.root.join("stage6-state.json")).map_err(|e|format!("restart state: {e}"))?;
    let rows:Value=serde_json::from_slice(&bytes).map_err(|e|format!("restart state: {e}"))?;
    let rows=rows.as_array().ok_or("restart state is not an array")?;
    require(app,ck,ev,"restart requires complete dual-engine first phase",rows.len()==ENGINES.len()&&ENGINES.iter().all(|engine|rows.iter().any(|r|r["engine"]==*engine&&["project","task","goal","sid","native_id"].iter().all(|key|r[*key].as_str().is_some_and(|s|!s.is_empty())))),json!(rows).to_string())?;
    for row in rows {
        step(app,ck,ev,"select persisted project after independent restart",row.clone(),r#"await click('dev-project-item-'+A.project);return {ok:!!await until(()=>tid('dev-task-summary')||tid('dev-task-new'))};"#).await?;
        if !ui_select_task(app,ck,ev,row["goal"].as_str().unwrap()).await { return Err("restart task selection".into()); }
        let sid=row["sid"].as_str().unwrap();
        resume(app,ck,ev,sid).await?;
        let v=turn(app,ck,ev,sid,&format!("{}-restart",row["engine"].as_str().unwrap()),"Continue development after the independent application restart. Add exports.square=x=>x*x in math.cjs and test square(5)==25 in test.cjs. Run npm test and npm run build. Do not mutate Git.").await?;
        require(app,ck,ev,"restart retains receiving native session identity",v["native_id"]==row["native_id"],v["native_id"].to_string())?;
        close(app,ck,ev,sid).await?;
        let r=host(app,ck,ev,row["task"].as_str().unwrap(),"node",&["-e","require('node:assert/strict').equal(require('./math.cjs').square(5),25)"],None).await?;
        require(app,ck,ev,"independent restarted receiver modified and verified real code",r["exit_code"]==0,r.to_string())?;
    }
    Ok(())
}

pub(super) async fn run(app:&AppHandle,fx:&Fx,mut ck:Checks,mut ev:Vec<Value>,restart:bool) {
    let phase=if restart { "restart" } else { "first" };
    let result:Journey<()>=async {
        require(app,&mut ck,&mut ev,"stage6 Git fixtures are isolated under private temporary root",fx.root.starts_with("/private/tmp")&&fx.root!=Path::new("/private/tmp"),fx.root.display())?;
        let denied=ipc(app,"main","respond_agent_request",json!({"input":{"session_id":"missing-session","generation":uuid::Uuid::new_v4(),"request_id":"expired-approval","response":{"decision":"accept"}}})).await;
        require(app,&mut ck,&mut ev,"Provider-only approval IPC rejects absent session and expired authority",!succeeded(&denied),denied.describe())?;
        let guard=app.state::<dgos_db::MaintenanceGate>().begin_maintenance().map_err(|_|"maintenance_fixture_busy")?;
        let blocked=ipc(app,"main","save_development_cli_provider",json!({"input":{"project_id":"missing-project","expected_revision":0,"configuration":{"engine":"codex","provider":{"provider_id":"missing-provider","model_id":"missing-model","provider_revision":0}}}})).await;
        drop(guard);
        require(app,&mut ck,&mut ev,"Provider IPC retains maintenance admission",!succeeded(&blocked),blocked.describe())?;
        if restart { restart_journey(app,fx,&mut ck,&mut ev).await?; }
        else if std::env::var_os("DGOS_STAGE6_BOUNDARIES_ONLY").is_some() { delivery_boundaries(app,fx,&mut ck,&mut ev).await?; }
        else if ENGINES.iter().any(|engine|fx.root.join(format!("original-{engine}")).exists()||fx.root.join(format!("work-{engine}")).exists()||fx.root.join(format!("remote-{engine}.git")).exists()) {
            return Err("first phase requires a fresh acceptance root; existing evidence and repositories are retained".into());
        }
        else {
            let mut rows=vec![];
            for engine in ENGINES {
                rows.push(per_engine(app,fx,&mut ck,&mut ev,engine).await?);
                write_json(fx.root.join("stage6-state.json"),&json!(rows));
            }
            delivery_boundaries(app,fx,&mut ck,&mut ev).await?;
        }
        Ok(())
    }.await;
    if let Err(error)=result {
        record(&mut ck,&mut ev,"stage6 stops at first unmet prerequisite",false,&error);
        let diagnostic=ui(app,json!({}),r#"return {ok:true,action:window.__stage6Action,receipt:window.__stage6Receipt,url:location.href,activeTabs:$$('[role=tab][data-state=active]').map(txt),dialogs:$$('[role=alertdialog],[role=dialog]').map(txt),alerts:$$('[role=alert]').map(txt),buttons:$$('button').map(b=>({text:txt(b),disabled:b.disabled})),body:document.body.innerText};"#).await;
        write_json(fx.root.join(format!("stage6-{phase}-failure-dom.json")),&diagnostic);
        screenshot(fx,&format!("stage6-{phase}-failure")).await;
    }
    // This stripped agent shell does not install ModuleManagementService. Its catalog banner
    // is independent of the production development controls; no catalog success is fabricated.
    record(&mut ck,&mut ev,"all owned Agent and host processes cleanly shut down",app.state::<AgentManager>().shutdown().await,"shutdown");
    write_json(fx.root.join(format!("stage6-result-{phase}.json")),&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev,"interaction":"DOM-driven production WKWebView; read-only IPC observations; local real CLI/model; native config reused","shell_limitation":"read_workspace_app_catalog unavailable: stripped-shell ModuleManagementService is not installed; unrelated catalog banner retained"}));
    println!("STAGE6 {phase}: {} passed, {} failed; {}",ck.passed,ck.failed.len(),fx.root.display());
    app.exit(if ck.failed.is_empty(){0}else{1});
}
