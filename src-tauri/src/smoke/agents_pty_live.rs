//! Same-round production UI/PTTY acceptance, selected by DGOS_PTY_LIVE_ACCEPTANCE.
use super::*;

fn evidence(fx: &Fx, name: &str, data: &Value) {
    let text=serde_json::to_string_pretty(data).expect("PTY evidence");
    fs::write(fx.root.join(format!("pty-live-{name}.json")),text.replace(fx.root.to_string_lossy().as_ref(),"<acceptance-root>")).expect("PTY evidence file");
}
async fn action(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, args: Value, body: &str) -> Journey<Value> {
    let result=dom(app,args,body).await;
    record(ck,ev,name,ui_ok(&result),result.to_string());
    evidence(fx,"checkpoint",&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev}));
    if ui_ok(&result) {Ok(result["value"].clone())} else {Err(format!("{name}: {result}"))}
}
fn alive(pid: i64) -> bool {
    Command::new("/bin/kill").args(["-0",&pid.to_string()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s|s.success())
}
async fn preferences(app: &AppHandle) -> Value {
    value(&ipc(app,"main","get_appearance_preferences",json!({})).await)["data"]["snapshot"].clone()
}
async fn identity(app: &AppHandle, baseline: &Value, pid: i64) -> Journey<Value> {
    let row=value(&ipc(app,"main","read_development_process",json!({"id":baseline["id"]})).await);
    if ["id","started_at","task_id","project_id"].iter().any(|key|row[*key]!=baseline[*key])||row["status"]!="running"||!alive(pid){return Err(format!("live identity changed: {row}"));}
    Ok(row)
}
#[cfg(target_os="macos")]
async fn native_line(app: &AppHandle, line: &str) -> Journey<()> {
    use objc2::encode::{Encode,Encoding};
    use objc2::runtime::{AnyObject,Bool};
    use objc2::{class,msg_send};
    use std::ffi::CString;
    #[repr(C)] #[derive(Clone,Copy)] struct Point {x:f64,y:f64}
    unsafe impl Encode for Point {const ENCODING:Encoding=Encoding::Struct("CGPoint",&[Encoding::Double,Encoding::Double]);}
    let window=app.get_webview_window("main").ok_or("main window absent")?;
    for ch in line.chars().chain(std::iter::once('\r')) {
        let text=CString::new(ch.to_string()).map_err(|e|e.to_string())?;
        let (send,receive)=oneshot::channel();
        window.with_webview(move |webview| unsafe {
            let win=webview.ns_window() as *mut AnyObject;
            let application:*mut AnyObject=msg_send![class!(NSApplication),sharedApplication];
            let chars:*mut AnyObject=msg_send![class!(NSString),stringWithUTF8String:text.as_ptr()];
            let info:*mut AnyObject=msg_send![class!(NSProcessInfo),processInfo];
            let time:f64=msg_send![info,systemUptime];let number:isize=msg_send![win,windowNumber];
            let code=if ch=='\r'{36u16}else{0u16};
            for kind in [10usize,11usize] {
                let event:*mut AnyObject=msg_send![class!(NSEvent),keyEventWithType:kind,location:Point{x:0.0,y:0.0},modifierFlags:0usize,timestamp:time,windowNumber:number,context:std::ptr::null_mut::<AnyObject>(),characters:chars,charactersIgnoringModifiers:chars,isARepeat:Bool::NO,keyCode:code];
                let _:()=msg_send![application,sendEvent:event];
            }
            let _=send.send(());
        }).map_err(|e|e.to_string())?;
        receive.await.map_err(|e|e.to_string())?;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}
#[cfg(not(target_os="macos"))]
async fn native_line(_: &AppHandle, _: &str) -> Journey<()> {Err("AppKit input requires macOS".into())}

async fn measure(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, path: &Path, baseline: &Value, pid: i64, label: &str) -> Journey<()> {
    let front=action(app,fx,ck,ev,&format!("{label}: nonzero frontend dimensions acknowledged by host"),json!({"id":baseline["id"]}),r#"
const p=window.__livePty;await document.fonts.ready;
await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
let stable='',since=performance.now();
const ready=await until(()=>{
const last=p.events.filter(e=>e.command==='resize_development_terminal'&&e.args.input.id===A.id).at(-1);
const t=document.querySelector('[data-testid="dev-terminal"]'),screen=t?.querySelector('.xterm-screen'),rows=[...(t?.querySelectorAll('.xterm-rows > div')||[])];
const size=last?.args.input,geometry={container:t?.getBoundingClientRect().toJSON(),screen:screen?.getBoundingClientRect().toJSON(),rowHeight:rows[0]?.getBoundingClientRect().height,renderRows:rows.length};
const signature=JSON.stringify({size,geometry,done:last?.done});if(signature!==stable){stable=signature;since=performance.now();}
return last?.ok&&size.rows>0&&size.cols>0&&size.rows<=1000&&size.cols<=1000&&geometry.screen?.width>0&&geometry.screen?.height>0&&geometry.screen.width<=geometry.container.width&&geometry.screen.height<=geometry.container.height&&rows.length===size.rows&&t.querySelector('.xterm')===p.terminal&&performance.now()-since>250&&{size,geometry};
},5000);
return {ok:!!ready,...ready};
"#).await?;
    fs::write(path.join("size.query"),label).map_err(|e|e.to_string())?;
    if !Command::new("/bin/kill").args(["-USR1",&pid.to_string()]).status().map_err(|e|e.to_string())?.success(){return Err("child measurement signal failed".into());}
    let deadline=tokio::time::Instant::now()+Duration::from_secs(5);
    let actual=loop {
        let text=fs::read_to_string(path.join("size.receipt")).unwrap_or_default();
        if text.starts_with(&format!("{label} ")){break text;}
        if tokio::time::Instant::now()>=deadline{return Err(format!("child stty receipt absent: {label}: {text}"));}
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    let expected=format!("{label} {} {}",front["size"]["rows"],front["size"]["cols"]);
    let host=identity(app,baseline,pid).await?;
    evidence(fx,&format!("size-{label}"),&json!({"frontend":front,"child_stty":actual,"host":host,"pid":pid}));
    let equal=actual.trim()==expected;
    record(ck,ev,&format!("{label}: child stty equals host acknowledged frontend fit"),equal,&actual);
    if equal {Ok(())} else {Err(format!("child stty {actual:?} != {expected:?}"))}
}
async fn input(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, path: &Path, id: &str, token: &str) -> Journey<()> {
    action(app,fx,ck,ev,"focus existing xterm for AppKit input",json!({}),r#"const input=(await need('dev-terminal')).querySelector('.xterm-helper-textarea');if(!input)throw Error('xterm input absent');input.focus();window.__livePty.keys=[];return {ok:document.activeElement===input};"#).await?;
    native_line(app,token).await?;
    host(app,id,&format!("RECEIVED:{token}")).await?;
    let receipts=fs::read_to_string(path.join("input.receipts")).map_err(|e|e.to_string())?;
    let count=receipts.lines().filter(|line|line.split_whitespace().next()==Some(token)).count();
    let keys=action(app,fx,ck,ev,"trusted WebKit input comes from AppKit NSEvents",json!({"length":token.len()}),r#"const keys=window.__livePty.keys;return {ok:keys.length===A.length+1&&keys.every(e=>e.trusted)&&keys.at(-1)?.key==='Enter',keys,evidence:'NSEvent -> NSApplication.sendEvent -> WKWebView -> xterm -> production host -> child receipt; not physical keyboard'};"#).await?;
    evidence(fx,token,&json!({"token":token,"child_received_count":count,"child_receipts":receipts,"native_input":keys}));
    record(ck,ev,"child records unique input token exactly once independently of terminal echo",count==1,format!("{token}: {count}"));
    if count==1 {Ok(())} else {Err("duplicate/missing child input".into())}
}
const OPEN: &str=r#"document.querySelector('a[href="/settings"]').click();const category=await until(()=>document.querySelector('.workspace-settings-category[data-settings-category="appearance"]'));if(!category)throw Error('appearance category absent');category.click();return {ok:!!await until(()=>document.querySelector('.appearance-editor'))};"#;

async fn journey(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    foreground(app).await?;
    let path=fx.root.join("pty-live-project");make_repo(&path);
    fs::write(path.join("pty-acceptance.sh"),r#"#!/bin/sh
stty -echo
printf '%s\n' "$$" > child.pid
measure() { printf '%s ' "$(cat size.query)" > size.tmp; stty size >> size.tmp; mv size.tmp size.receipt; }
trap measure USR1
printf 'PTY_LIVE_READY\nWIDTH:I中W文0\n'
while :; do
  if IFS= read -r line; then
    printf '%s ' "$line" >> input.receipts
    stty size >> input.receipts
    printf 'RECEIVED:%s\n' "$line"
  fi
done
"#).map_err(|e|e.to_string())?;
    if !ui_import_project(app,ck,ev,"Live PTY isolated project",&path).await||!ui_new_task(app,ck,ev,"Live PTY acceptance").await||!ui_select_task(app,ck,ev,"Live PTY acceptance").await{return Err("production project/task UI setup failed".into());}
    let workspace=value(&ipc(app,"main","read_development_workspace",json!({})).await);
    let task=workspace["selection"]["task_id"].as_str().ok_or("selected task absent")?.to_owned();
    let resolver=app.state::<Arc<crate::session::SessionResolver>>();
    let session=resolver.current().map_err(|e|e.to_string())?;
    let project=workspace["projects"].as_array().and_then(|rows|rows.iter().find(|p|p["id"]==workspace["selection"]["project_id"])).ok_or("project absent")?;
    if project["owner"]!=session.user_id(){return Err("project owner differs from formal account session".into());}
    let assets=action(app,fx,ck,ev,"record same-round production bundle identity",json!({}),r#"return {ok:true,location:location.href,resources:performance.getEntriesByType('resource').map(e=>e.name).filter(url=>/\.(js|css)(\?|$)/.test(url)),scripts:[...document.scripts].map(e=>e.src).filter(Boolean),styles:[...document.querySelectorAll('link[rel=stylesheet]')].map(e=>e.href),runtime:navigator.userAgent};"#).await?;
    evidence(fx,"runtime",&json!({"assets":assets,"host_pid":std::process::id(),"host_version":env!("CARGO_PKG_VERSION"),"project_owner_matches_signed_in_session":true,"account":"isolated alice","project":project,"selection":workspace["selection"]}));
    action(app,fx,ck,ev,"open production runtime configuration",json!({}),r#"const toggle=await need('dev-toggle-runtime');if(toggle.getAttribute('aria-expanded')!=='true')toggle.click();await click('dev-runtime-processes');const details=await need('dev-runtime-configuration');if(!details.open)details.querySelector('summary').click();return {ok:!!await need('dev-process-program')};"#).await?;
    action(app,fx,ck,ev,"commit terminal kind before other fields",json!({}),r#"const kind=await need('dev-process-kind');kind.value='terminal';kind.dispatchEvent(new Event('change',{bubbles:true}));await new Promise(r=>setTimeout(r,50));return {ok:kind.value==='terminal'};"#).await?;
    action(app,fx,ck,ev,"commit harmless shell program before submit",json!({}),r#"await fill('dev-process-program','/bin/sh');return {ok:!!await until(()=>tid('dev-runtime-draft-summary')?.textContent.includes('/bin/sh'))};"#).await?;
    action(app,fx,ck,ev,"commit isolated child script argument",json!({}),r#"const form=(await need('dev-process-program')).closest('form'),fields=form.querySelector('fieldset');while(fields.querySelector('input')){fields.querySelector('button').click();await new Promise(r=>setTimeout(r,50));}fields.querySelector('button').click();const field=await until(()=>fields.querySelector('input'));if(!field)throw Error('argument input absent');setInput(field,'pty-acceptance.sh');return {ok:!!await until(()=>tid('dev-runtime-draft-summary')?.textContent.includes('pty-acceptance.sh'))};"#).await?;
    action(app,fx,ck,ev,"confirm trust on settled project cwd",json!({}),r#"await fill('dev-process-cwd','');const trust=await need('dev-process-trust');if(!trust.checked)trust.click();await new Promise(r=>setTimeout(r,50));return {ok:trust.checked};"#).await?;
    action(app,fx,ck,ev,"observe real transport without replacing host results",json!({}),r#"
const original=window.fetch,p={original,events:[],keys:[],terminal:null};window.__livePty=p;
p.listener=e=>{if(e.target?.classList?.contains('xterm-helper-textarea'))p.keys.push({key:e.key,trusted:e.isTrusted,repeat:e.repeat});};document.addEventListener('keydown',p.listener,true);
window.fetch=function(url,options,...rest){const command=String(url).split('/').at(-1),watched=['start_development_process','stop_development_process','write_development_terminal','resize_development_terminal','set_appearance_preferences'].includes(command);const e=watched?{command,args:JSON.parse(options?.body||'{}'),at:performance.now()}:null;if(e)p.events.push(e);return original.call(this,url,options,...rest).then(response=>{if(e){e.ok=response.headers.get('Tauri-Response')==='ok';e.done=performance.now();}return response;},error=>{if(e){e.ok=false;e.done=performance.now();}throw error;});};return {ok:true};
"#).await?;
    let before=value(&ipc(app,"main","list_development_processes",json!({"taskId":task})).await);
    let request=action(app,fx,ck,ev,"submit production UI start and observe actual host request",json!({}),r#"const start=await need('dev-process-start');if(start.disabled)throw Error('start disabled');start.click();const e=await until(()=>window.__livePty.events.find(e=>e.command==='start_development_process'),5000);return {ok:!!e,request:e?.args.input,alerts:$$('[role=alert]').map(txt),trust:tid('dev-process-trust')?.checked,draft:tid('dev-runtime-draft-summary')?.textContent};"#).await?;
    let id=request["request"]["id"].as_str().ok_or("real process ID absent")?.to_owned();
    let baseline=host(app,&id,"PTY_LIVE_READY").await?;
    let after=value(&ipc(app,"main","list_development_processes",json!({"taskId":task})).await);
    let fresh:Vec<&Value>=after.as_array().into_iter().flatten().filter(|row|!before.as_array().into_iter().flatten().any(|old|old["id"]==row["id"])).collect();
    if fresh.len()!=1||fresh[0]["id"]!=id||!baseline["started_at"].is_number(){return Err(format!("one start must create one process: {after}"));}
    let pid=fs::read_to_string(path.join("child.pid")).map_err(|e|e.to_string())?.trim().parse::<i64>().map_err(|e|e.to_string())?;
    action(app,fx,ck,ev,"first output renders in existing production terminal",json!({}),r#"const t=await need('dev-terminal');t.scrollIntoView({block:'center'});const ready=await until(()=>[...t.querySelectorAll('.xterm-rows > div')].some(e=>e.textContent.includes('PTY_LIVE_READY')));window.__livePty.terminal=t.querySelector('.xterm');return {ok:!!ready};"#).await?;
    identity(app,&baseline,pid).await?;
    let ps=Command::new("/bin/ps").args(["-p",&pid.to_string(),"-o","pid=,ppid=,lstart=,comm="]).output().map_err(|e|e.to_string())?;
    evidence(fx,"identity",&json!({"process":baseline,"pid":pid,"os_process":String::from_utf8_lossy(&ps.stdout),"run_id":id,"started_at":baseline["started_at"]}));
    super::super::super::capture_native(app,&fx.root,"pty-live-first-output").await?;
    measure(app,fx,ck,ev,&path,&baseline,pid,"initial").await?;
    let first=format!("input_{}",uuid::Uuid::new_v4().simple());let second=format!("resume_{}",uuid::Uuid::new_v4().simple());
    input(app,fx,ck,ev,&path,&id,&first).await?;
    let confirmed=preferences(app).await;
    let appearance_before=action(app,fx,ck,ev,"record terminal font before preview",json!({}),r#"const s=getComputedStyle(document.querySelector('[data-testid=\"dev-terminal\"] .xterm-rows'));return {ok:true,family:s.fontFamily,size:s.fontSize};"#).await?;
    action(app,fx,ck,ev,"open existing appearance settings",json!({}),OPEN).await?;
    let size=if confirmed["typography"]["codeSize"]==24 {18}else{24};
    action(app,fx,ck,ev,"preview code size through production number input",json!({"size":size}),r#"await fill('appearance-code-size-input',String(A.size));return {ok:!!await until(()=>getComputedStyle(document.querySelector('[data-testid="dev-terminal"] .xterm-rows')).fontSize===A.size+'px')};"#).await?;
    measure(app,fx,ck,ev,&path,&baseline,pid,"code-size").await?;
    let font=action(app,fx,ck,ev,"select different installed monospace through production picker",json!({}),r#"
const response=await window.__TAURI_INTERNALS__.invoke('list_local_fonts');if(!response?.success)throw Error('native catalogue unavailable');const current=getComputedStyle(document.querySelector('[data-testid="dev-terminal"] .xterm-rows')).fontFamily;
const family=response.data.find(f=>f.monospace&&!current.includes(f.family)&&document.fonts.check('16px "'+f.family.replaceAll('"','\\"')+'"'))?.family;if(!family)throw Error('different installed monospace absent');
await click('appearance-code-font');const search=await until(()=>document.querySelector('input[type=search]'));if(!search)throw Error('font search absent');setInput(search,family);const option=await until(()=>document.querySelector('button[role=option][data-font-family="'+CSS.escape(family)+'"]'));if(!option)throw Error('installed font option absent');option.click();await document.fonts.ready;return {ok:!!await until(()=>getComputedStyle(document.querySelector('[data-testid="dev-terminal"] .xterm-rows')).fontFamily.includes(family)),family};
"#).await?;
    measure(app,fx,ck,ev,&path,&baseline,pid,"local-font").await?;
    for density in [60,100,150] {
        action(app,fx,ck,ev,&format!("preview production layout Slider {density}%"),json!({"density":density}),r#"const slider=await until(()=>document.querySelector('.appearance-layout-scale [role=slider]'));if(!slider)throw Error('Slider absent');slider.focus();slider.dispatchEvent(new KeyboardEvent('keydown',{key:'Home',bubbles:true}));await new Promise(r=>setTimeout(r,50));for(let n=60;n<A.density;n+=10){slider.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true}));await new Promise(r=>setTimeout(r,30));}return {ok:!!await until(()=>document.documentElement.dataset.density===String(A.density)),density:document.documentElement.dataset.density};"#).await?;
        measure(app,fx,ck,ev,&path,&baseline,pid,&format!("scale-{density}")).await?;
        if preferences(app).await!=confirmed{return Err("preview wrote confirmed preferences".into());}
        super::super::super::capture_native(app,&fx.root,&format!("pty-live-scale-{density}")).await?;
    }
    action(app,fx,ck,ev,"cancel restores appearance while business process survives",json!({"density":confirmed["density"],"size":appearance_before["size"],"family":appearance_before["family"]}),r#"const cancel=document.querySelector('.appearance-save-actions button');if(!cancel)throw Error('cancel absent');cancel.click();return {ok:!!await until(()=>{const s=getComputedStyle(document.querySelector('[data-testid=\"dev-terminal\"] .xterm-rows'));return document.documentElement.dataset.density===String(A.density)&&s.fontSize===A.size&&s.fontFamily===A.family})};"#).await?;
    if preferences(app).await!=confirmed{return Err("cancel wrote stored preferences".into());}
    action(app,fx,ck,ev,"close cancelled settings",json!({}),r#"document.querySelector('.workspace-settings-header button').click();return {ok:!!await until(()=>!document.querySelector('.workspace-settings-dialog'))};"#).await?;
    measure(app,fx,ck,ev,&path,&baseline,pid,"cancel-restored").await?;
    action(app,fx,ck,ev,"resize actual runtime panel using production separator",json!({}),r#"const s=await need('dev-runtime-resizer');const before=s.getAttribute('aria-valuenow');s.focus();s.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowUp',bubbles:true}));return {ok:!!await until(()=>s.getAttribute('aria-valuenow')!==before),before,after:s.getAttribute('aria-valuenow')};"#).await?;
    measure(app,fx,ck,ev,&path,&baseline,pid,"panel-resize").await?;
    action(app,fx,ck,ev,"hide runtime using production toggle",json!({}),r#"await click('dev-toggle-runtime');return {ok:tid('dev-toggle-runtime')?.getAttribute('aria-expanded')==='false'};"#).await?;
    identity(app,&baseline,pid).await?;
    action(app,fx,ck,ev,"restore runtime and switch actual runs panel",json!({}),r#"await click('dev-toggle-runtime');await click('dev-runtime-runs');return {ok:tid('dev-runtime-runs')?.getAttribute('aria-pressed')==='true'};"#).await?;
    identity(app,&baseline,pid).await?;
    action(app,fx,ck,ev,"restore processes with old output and same xterm",json!({"first":first}),r#"await click('dev-runtime-processes');const t=await need('dev-terminal');t.scrollIntoView({block:'center'});const text=()=>[...t.querySelectorAll('.xterm-rows > div')].map(e=>e.textContent).join('\n');return {ok:!!await until(()=>text().includes('RECEIVED:'+A.first)&&text().includes('PTY_LIVE_READY'))&&t.querySelector('.xterm')===window.__livePty.terminal};"#).await?;
    measure(app,fx,ck,ev,&path,&baseline,pid,"hidden-restored").await?;
    input(app,fx,ck,ev,&path,&id,&second).await?;
    identity(app,&baseline,pid).await?;
    super::super::super::capture_native(app,&fx.root,"pty-live-resumed").await?;
    action(app,fx,ck,ev,"stop real PTY through production UI",json!({}),r#"const b=await need('dev-process-stop');if(b.disabled)throw Error('stop disabled');b.click();return {ok:!!await until(()=>window.__livePty.events.some(e=>e.command==='stop_development_process'&&e.ok))};"#).await?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(10);
    let stopped=loop {let row=value(&ipc(app,"main","read_development_process",json!({"id":id})).await);if row["finished_at"].is_number()&&!alive(pid){break row;}if tokio::time::Instant::now()>=deadline{return Err(format!("UI stop did not release PID {pid}: {row}"));}tokio::time::sleep(Duration::from_millis(50)).await;};
    let receipts=fs::read_to_string(path.join("input.receipts")).map_err(|e|e.to_string())?;
    if receipts.lines().count()!=2||![&first,&second].iter().all(|token|receipts.lines().filter(|line|line.split_whitespace().next()==Some(token.as_str())).count()==1){return Err(format!("child detected input replay: {receipts}"));}
    let rejected=ipc(app,"main","write_development_terminal",json!({"input":{"id":id,"data":"after-stop\r"}})).await;
    if succeeded(&rejected)||stopped["status"]=="cleanup_pending"{return Err("stopped PTY retained input resource".into());}
    let trace=action(app,fx,ck,ev,"one start one UI stop zero preference writes and no replay",json!({"first":first,"second":second}),r#"const events=window.__livePty.events,starts=events.filter(e=>e.command==='start_development_process'),stops=events.filter(e=>e.command==='stop_development_process'),writes=events.filter(e=>e.command==='write_development_terminal'&&e.ok).map(e=>e.args.input.data).join('');return {ok:starts.length===1&&stops.length===1&&!events.some(e=>e.command==='set_appearance_preferences')&&writes===A.first+'\r'+A.second+'\r',events};"#).await?;
    evidence(fx,"complete",&json!({"process":stopped,"pid":pid,"pid_exited":true,"post_stop_input_rejected":true,"child_input_receipts":receipts,"font":font,"preferences_unchanged":preferences(app).await==confirmed,"trace":trace,"input_evidence":"AppKit NSEvent -> trusted WKWebView -> xterm -> production host -> real child receipts; not physical keyboard"}));
    super::super::super::capture_native(app,&fx.root,"pty-live-stopped").await?;
    Ok(())
}
pub(super) async fn run(app: &AppHandle, fx: &Fx, mut ck: Checks, mut ev: Vec<Value>) {
    if let Err(error)=journey(app,fx,&mut ck,&mut ev).await {
        record(&mut ck,&mut ev,"same-round live PTY journey completes",false,error);
        evidence(fx,"failure",&dom(app,json!({}),FAILURE_SNAPSHOT).await);
        let _=super::super::super::capture_native(app,&fx.root,"pty-live-failure").await;
        // Cleanup still traverses the production UI; manager shutdown is the final guard.
        let cleanup=dom(app,json!({}),r#"document.querySelector('.appearance-save-actions button')?.click();document.querySelector('.workspace-settings-header button')?.click();await until(()=>!document.querySelector('.workspace-settings-dialog'),2000);const toggle=tid('dev-toggle-runtime');if(toggle?.getAttribute('aria-expanded')==='false')toggle.click();tid('dev-runtime-processes')?.click();const stop=await until(()=>tid('dev-process-stop'),2000);if(stop&&!stop.disabled)stop.click();return {ok:true,stopRequested:!!stop};"#).await;
        evidence(fx,"failure-ui-cleanup",&cleanup);
        let workspace=value(&ipc(app,"main","read_development_workspace",json!({})).await);
        if let Some(task)=workspace["selection"]["task_id"].as_str() {
            let rows=value(&ipc(app,"main","list_development_processes",json!({"taskId":task})).await);
            for row in rows.as_array().into_iter().flatten().filter(|row|row["finished_at"].is_null()) {
                let stop=ipc(app,"main","stop_development_process",json!({"id":row["id"]})).await;
                evidence(fx,"failure-host-stop",&json!({"id":row["id"],"reply":stop.0}));
            }
        }
    }
    let _=dom(app,json!({}),r#"const p=window.__livePty;if(p){window.fetch=p.original;document.removeEventListener('keydown',p.listener,true);}return {ok:true};"#).await;
    record(&mut ck,&mut ev,"owned PTY cleanup completes",app.state::<AgentManager>().shutdown().await,"production shutdown after UI stop assertions");
    evidence(fx,"result",&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev,"source":"production dist/WKWebView, isolated signed-in account, real shell PID/stty/input receipts, AppKit NSEvent input, native window screenshots"}));
    app.exit(if ck.failed.is_empty(){0}else{1});
}
