//! Production WKWebView/PTY recovery acceptance. Build dist first, then use a fresh
//! DGOS_PROJECT_ACCEPTANCE_ROOT and DGOS_PROJECT_ACCEPTANCE_PHASE=terminal-round3
//! cargo test -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture.
//! This scenario deliberately opens xterm in a hidden native window before its
//! first fit. Every transition records actual renderer/a11y DOM and host identity.
use super::*;

type Journey<T> = Result<T, String>;

fn save(fx: &Fx, name: &str, data: &Value) {
    fs::write(fx.root.join(format!("terminal-round3-{name}.json")), serde_json::to_vec_pretty(data).expect("terminal JSON")).expect("terminal evidence");
}

async fn dom(app: &AppHandle, args: Value, body: &str) -> Value {
    eval_long(app, &format!("const stage6UI=true;\n{DOM}\nconst A={args};\n{body}")).await
}

async fn step(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, args: Value, body: &str) -> Journey<Value> {
    let result=dom(app,args,body).await;
    record(ck,ev,name,ui_ok(&result),result.to_string());
    save(fx,"checkpoint",&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev}));
    if ui_ok(&result) { Ok(result["value"].clone()) } else { Err(name.into()) }
}

const SNAPSHOT: &str = r#"
const t=document.querySelector('[data-testid="dev-terminal"]');
const rect=e=>{if(!e)return null;const r=e.getBoundingClientRect();return {width:r.width,height:r.height,clientWidth:e.clientWidth,clientHeight:e.clientHeight};};
const layers={renderer:[...(t?.querySelectorAll('.xterm-rows > div')||[])].map(e=>e.textContent||''),accessibility:[...(t?.querySelectorAll('.xterm-accessibility-tree > div')||[])].map(e=>(e.textContent||'').replaceAll('\u00a0',''))};
return {ok:true,at:performance.now(),hidden:document.hidden,focus:document.hasFocus(),active:document.activeElement?.getAttribute('data-testid'),activeClass:document.activeElement?.className,container:rect(t),screen:rect(t?.querySelector('.xterm-screen')),a11yRows:[...(t?.querySelectorAll('.xterm-accessibility-tree > div')||[])].map(rect),layers};
"#;

const FAILURE_SNAPSHOT: &str = r#"
const t=document.querySelector('[data-testid="dev-terminal"]');
const rect=e=>{if(!e)return null;const r=e.getBoundingClientRect(),s=getComputedStyle(e);return {x:r.x,y:r.y,top:r.top,right:r.right,bottom:r.bottom,left:r.left,width:r.width,height:r.height,clientWidth:e.clientWidth,clientHeight:e.clientHeight,scrollWidth:e.scrollWidth,scrollHeight:e.scrollHeight,scrollTop:e.scrollTop,scrollLeft:e.scrollLeft,overflowX:s.overflowX,overflowY:s.overflowY,display:s.display,visibility:s.visibility};};
const screen=t?.querySelector('.xterm-screen');const ancestors=[];for(let e=t?.parentElement;e;e=e.parentElement)ancestors.push({tag:e.tagName,testId:e.getAttribute('data-testid'),className:e.className,hidden:e.hidden,inert:e.inert,geometry:rect(e)});
const intersection=screen&&'IntersectionObserver' in window?await new Promise(resolve=>{let observer;const timer=setTimeout(()=>{observer?.disconnect();resolve({delivered:false});},300);observer=new IntersectionObserver(entries=>{const e=entries.at(-1);clearTimeout(timer);observer.disconnect();resolve({delivered:true,isIntersecting:e.isIntersecting,ratio:e.intersectionRatio,rootBounds:e.rootBounds?.toJSON(),boundingClientRect:e.boundingClientRect.toJSON(),intersectionRect:e.intersectionRect.toJSON()});});observer.observe(screen);}):null;
const layers={renderer:[...(t?.querySelectorAll('.xterm-rows > div')||[])].map(e=>e.textContent||''),accessibility:[...(t?.querySelectorAll('.xterm-accessibility-tree > div')||[])].map(e=>(e.textContent||'').replaceAll('\u00a0',''))};
return {ok:true,at:performance.now(),hidden:document.hidden,focus:document.hasFocus(),viewport:{width:innerWidth,height:innerHeight,visualViewport:window.visualViewport?{width:visualViewport.width,height:visualViewport.height,offsetTop:visualViewport.offsetTop,offsetLeft:visualViewport.offsetLeft}:null},intersection,ancestors,active:document.activeElement?.getAttribute('data-testid'),activeClass:document.activeElement?.className,container:rect(t),screen:rect(screen),a11yRows:[...(t?.querySelectorAll('.xterm-accessibility-tree > div')||[])].map(rect),layers};
"#;

async fn snapshot(app: &AppHandle, fx: &Fx, name: &str) {
    save(fx,name,&dom(app,json!({}),if name=="failure" { FAILURE_SNAPSHOT } else { SNAPSHOT }).await);
}

pub(super) async fn host(app: &AppHandle, id: &str, needle: &str) -> Journey<Value> {
    let deadline=tokio::time::Instant::now()+Duration::from_secs(25);
    loop {
        let reply=ipc(app,"main","read_development_process",json!({"id":id})).await;
        let row=value(&reply);
        if succeeded(&reply)&&row["output"].as_str().is_some_and(|s|s.contains(needle)) { return Ok(row); }
        if tokio::time::Instant::now()>=deadline { return Err(format!("PTY output missing {needle}: {}",reply.describe())); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(target_os="macos")]
pub(super) async fn background(app: &AppHandle) -> Journey<()> {
    use objc2::{class,msg_send};
    use objc2::runtime::AnyObject;
    let window=app.get_webview_window("main").ok_or("main window absent")?;
    let (send,receive)=oneshot::channel();
    window.run_on_main_thread(move || {
        let state=unsafe {
            let application:*mut AnyObject=msg_send![class!(NSApplication),sharedApplication];
            let _:()=msg_send![application,hide:std::ptr::null_mut::<AnyObject>()];
            let hidden:bool=msg_send![application,isHidden];
            let active:bool=msg_send![application,isActive];
            json!({"native_hidden":hidden,"native_active":active})
        };
        let _=send.send(state);
    }).map_err(|e|e.to_string())?;
    let mut state=tokio::time::timeout(Duration::from_secs(10),receive).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(5);
    loop {
        if state["native_hidden"]==true { return Ok(()); }
        if tokio::time::Instant::now()>=deadline { return Err(format!("native application did not hide: {state}")); }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (send,receive)=oneshot::channel();
        window.run_on_main_thread(move || {
            let state=unsafe {
                let application:*mut AnyObject=msg_send![class!(NSApplication),sharedApplication];
                let hidden:bool=msg_send![application,isHidden];
                let active:bool=msg_send![application,isActive];
                json!({"native_hidden":hidden,"native_active":active})
            };
            let _=send.send(state);
        }).map_err(|e|e.to_string())?;
        state=tokio::time::timeout(Duration::from_secs(10),receive).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
    }
}

#[cfg(not(target_os="macos"))]
pub(super) async fn background(_app: &AppHandle) -> Journey<()> { Err("WKWebView scenario requires macOS".into()) }

pub(super) async fn foreground(app: &AppHandle) -> Journey<()> {
    if let Some(window)=app.get_webview_window("main") { window.show().map_err(|e|e.to_string())?; }
    if super::super::ui_keys::prepare(app).await { Ok(()) } else { Err("native app activation failed".into()) }
}

pub(super) async fn projection(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, label: &str, needle: &str) -> Journey<()> {
    step(app,fx,ck,ev,label,json!({"needle":needle}),r#"
const t=await need('dev-terminal');
const ready=await until(()=>{const rendered=[...t.querySelectorAll('.xterm-rows > div')].map(e=>e.textContent||'');const tree=[...t.querySelectorAll('.xterm-accessibility-tree > div')].map(e=>(e.textContent||'').replaceAll('\u00a0',''));return rendered.join('\n').includes(A.needle)&&tree.join('\n').includes(A.needle)&&rendered.length===tree.length&&rendered.every((line,i)=>line.trimEnd()===tree[i].trimEnd())&&[...t.querySelectorAll('.xterm-accessibility-tree > div')].every(e=>e.getBoundingClientRect().height>0);},5000);
return {ok:!!ready,renderer:[...t.querySelectorAll('.xterm-rows > div')].map(e=>e.textContent),tree:[...t.querySelectorAll('.xterm-accessibility-tree > div')].map(e=>e.textContent),active:document.activeElement?.getAttribute('data-testid')};
"#).await?;
    snapshot(app,fx,label).await;
    Ok(())
}

async fn type_line(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, line: &str) -> Journey<()> {
    step(app,fx,ck,ev,"explicit user terminal input",json!({"line":line}),r#"
const input=(await need('dev-terminal')).querySelector('.xterm-helper-textarea');if(!input)throw Error('xterm textarea absent');input.focus();
for(const key of A.line){const c=key.charCodeAt(0);input.dispatchEvent(new KeyboardEvent('keydown',{key,keyCode:0,which:0,bubbles:true}));input.dispatchEvent(new KeyboardEvent('keypress',{key,charCode:c,keyCode:c,which:c,bubbles:true}));input.dispatchEvent(new KeyboardEvent('keyup',{key,bubbles:true}));}
input.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',code:'Enter',keyCode:13,which:13,bubbles:true}));input.dispatchEvent(new KeyboardEvent('keyup',{key:'Enter',code:'Enter',keyCode:13,which:13,bubbles:true}));return {ok:true};
"#).await?;
    Ok(())
}

async fn journey(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    let path=fx.root.join("terminal-round3-project");make_repo(&path);
    if !ui_import_project(app,ck,ev,"Round3 terminal",&path).await||!ui_new_task(app,ck,ev,"Round3 terminal recovery").await||!ui_select_task(app,ck,ev,"Round3 terminal recovery").await { return Err("terminal project/task setup failed".into()); }
    let selected=value(&ipc(app,"main","read_development_workspace",json!({})).await)["selection"].clone();
    let task=selected["task_id"].as_str().ok_or("selected task absent")?;
    step(app,fx,ck,ev,"configure real interactive shell",json!({}),r#"
const toggle=await need('dev-toggle-runtime');if(toggle.getAttribute('aria-expanded')!=='true')toggle.click();await click('dev-runtime-processes');
const configuration=await need('dev-runtime-configuration');configuration.open=true;
const kind=await need('dev-process-kind');kind.value='terminal';kind.dispatchEvent(new Event('change',{bubbles:true}));await new Promise(r=>setTimeout(r,80));
await fill('dev-process-program','/bin/sh');
while($$('button[aria-label^="移除参数"]').length){$$('button[aria-label^="移除参数"]')[0].click();await new Promise(r=>setTimeout(r,60));}
const trust=await need('dev-process-trust');if(!trust.checked)trust.click();return {ok:!!await until(()=>!tid('dev-process-start')?.disabled)};
"#).await?;
    // Count transport only; no native response, PTY output or instance is mocked.
    step(app,fx,ck,ev,"install passive real IPC observation",json!({}),r#"
const original=window.fetch;const p={original,events:[]};window.__round3Terminal=p;
window.fetch=function(url,options,...rest){const command=String(url).split('/').at(-1);if(['start_development_process','stop_development_process','write_development_terminal','resize_development_terminal'].includes(command))p.events.push({command,args:JSON.parse(options?.body||'{}')});return original.call(this,url,options,...rest);};return {ok:true};
"#).await?;
    let before=value(&ipc(app,"main","list_development_processes",json!({"taskId":task})).await);
    app.get_webview_window("main").ok_or("main window absent")?.hide().map_err(|e|e.to_string())?;
    step(app,fx,ck,ev,"open production xterm while actual native window is hidden",json!({}),r#"
if(!await until(()=>document.hidden))throw Error('native WebView did not hide');
await click('dev-process-start');
const t=await until(()=>document.querySelector('[data-testid="dev-terminal"] .xterm-helper-textarea'));return {ok:!!t,hidden:document.hidden};
"#).await?;
    snapshot(app,fx,"before-first-activation").await;
    let rows=value(&ipc(app,"main","list_development_processes",json!({"taskId":task})).await);
    let fresh:Vec<&Value>=rows.as_array().into_iter().flatten().filter(|row|!before.as_array().into_iter().flatten().any(|old|old["id"]==row["id"])).collect();
    if fresh.len()!=1 { return Err(format!("expected one new real PTY: {rows}")); }
    let id=fresh[0]["id"].as_str().ok_or("PTY id absent")?.to_owned();
    let setup=ipc(app,"main","write_development_terminal",json!({"input":{"id":id,"data":"stty -echo; PS1=''; printf '%s\\n' \"$$\" > round3.pid; printf 'once\\n' >> round3.input; printf 'ROUND3_'; printf 'READY\\n'\r"}})).await;
    if !succeeded(&setup) { return Err(setup.describe()); }
    let baseline=host(app,&id,"ROUND3_READY").await?;
    let pid=fs::read_to_string(path.join("round3.pid")).map_err(|e|e.to_string())?;
    step(app,fx,ck,ev,"place nonterminal focus before native activation",json!({}),r#"const b=await need('dev-toggle-runtime');b.focus();return {ok:document.activeElement===b};"#).await?;
    snapshot(app,fx,"before-first-activation-with-output").await;
    foreground(app).await?;
    projection(app,fx,ck,ev,"after-first-activation","ROUND3_READY").await?;
    step(app,fx,ck,ev,"initial activation preserves nonterminal focus",json!({}),r#"return {ok:!document.activeElement?.classList.contains('xterm-helper-textarea')};"#).await?;
    save(fx,"host-baseline",&json!({"process":baseline,"pid":pid}));
    for cycle in 0..3 {
        let needle=format!("ROUND3_RETURN_{cycle}");
        step(app,fx,ck,ev,"place focus outside terminal before lifecycle transition",json!({}),r#"const b=await need('dev-toggle-runtime');b.focus();return {ok:document.activeElement===b};"#).await?;
        if cycle==0 {
            step(app,fx,ck,ev,"hide runtime through production control",json!({}),r#"await click('dev-toggle-runtime');return {ok:tid('dev-toggle-runtime')?.getAttribute('aria-expanded')==='false'};"#).await?;
        } else {
            background(app).await?;
            step(app,fx,ck,ev,"native hidden application makes WebView hidden before return",json!({}),r#"return {ok:!!await until(()=>document.hidden),hidden:document.hidden,focus:document.hasFocus(),nativeEvidence:'NSApplication.isHidden checked by host; WKWebView document.hasFocus may remain true while hidden'};"#).await?;
        }
        let window=app.get_webview_window("main").ok_or("main window absent")?;
        window.set_size(tauri::LogicalSize::new([1000.0,1350.0,1100.0][cycle],950.0)).map_err(|e|e.to_string())?;
        // Hidden output is produced by the same real shell, independent of rendering.
        let write=ipc(app,"main","write_development_terminal",json!({"input":{"id":id,"data":format!("printf '{needle}\\n'\r")}})).await;
        if !succeeded(&write) { return Err(write.describe()); }
        host(app,&id,&needle).await?;
        snapshot(app,fx,&format!("before-return-{cycle}")).await;
        if cycle==0 {
            step(app,fx,ck,ev,"show runtime through production control",json!({}),r#"await click('dev-toggle-runtime');return {ok:tid('dev-toggle-runtime')?.getAttribute('aria-expanded')==='true'};"#).await?;
        } else { foreground(app).await?; }
        projection(app,fx,ck,ev,&format!("after-return-{cycle}"),&needle).await?;
        step(app,fx,ck,ev,"automatic resume preserves nonterminal focus",json!({}),r#"return {ok:!document.activeElement?.classList.contains('xterm-helper-textarea'),active:document.activeElement?.getAttribute('data-testid')};"#).await?;
        type_line(app,fx,ck,ev,&format!("printf 'once\\n' >> round3.resume.{cycle}; printf 'ROUND3_SIZE_{cycle}:'; stty size; printf 'ROUND3_INPUT_{cycle}\\n'")).await?;
        let returned=host(app,&id,&format!("ROUND3_INPUT_{cycle}")).await?;
        let same=returned["id"]==baseline["id"]&&returned["started_at"]==baseline["started_at"]&&returned["status"]=="running"&&fs::read_to_string(path.join("round3.pid")).unwrap_or_default()==pid&&fs::read_to_string(path.join(format!("round3.resume.{cycle}"))).unwrap_or_default()=="once\n";
        record(ck,ev,"same host process resumes fresh input exactly once",same,returned.to_string());
        if !same { return Err("PTY identity or resumed input count changed".into()); }
        save(fx,&format!("host-return-{cycle}"),&returned);
        let size=step(app,fx,ck,ev,"last fitted dimensions reach unchanged PTY",json!({"id":id,"output":returned["output"],"cycle":cycle}),r#"
        const request=window.__round3Terminal.events.filter(e=>e.command==='resize_development_terminal'&&e.args.input.id===A.id).at(-1)?.args.input;
        return {ok:!!request&&A.output.includes('ROUND3_SIZE_'+A.cycle+':'+request.rows+' '+request.cols),request};
        "#).await?;
        save(fx,&format!("size-return-{cycle}"),&size);
    }
    let observation=step(app,fx,ck,ev,"recovery creates no extra starts stops or input replay",json!({}),r#"
const p=window.__round3Terminal;const starts=p.events.filter(e=>e.command==='start_development_process');const stops=p.events.filter(e=>e.command==='stop_development_process');const writes=p.events.filter(e=>e.command==='write_development_terminal').map(e=>e.args.input.data).join('');return {ok:starts.length===1&&stops.length===0&&(writes.match(/round3\.input/g)||[]).length===1&&[0,1,2].every(i=>(writes.match(new RegExp('round3\\.resume\\.'+i,'g'))||[]).length===1),events:p.events};
"#).await?;
    save(fx,"ipc",&observation);
    if fs::read_to_string(path.join("round3.input")).unwrap_or_default()!="once\n" { return Err("original input was replayed".into()); }
    Ok(())
}

pub(super) async fn run(app: &AppHandle, fx: &Fx, mut ck: Checks, mut ev: Vec<Value>) {
    let old=Path::new("/private/tmp/dgos-layout-round2-20261106-delivery/layout-round2-failure-dom.json");
    if let Ok(bytes)=fs::read(old) { if let Ok(value)=serde_json::from_slice::<Value>(&bytes) { save(fx,"round2-failure-before-fix",&value); } }
    if let Err(error)=journey(app,fx,&mut ck,&mut ev).await { record(&mut ck,&mut ev,"terminal recovery journey completes",false,error); snapshot(app,fx,"failure").await; }
    let _=dom(app,json!({}),r#"const p=window.__round3Terminal;if(p)window.fetch=p.original;return {ok:true};"#).await;
    record(&mut ck,&mut ev,"owned PTY cleanup",app.state::<AgentManager>().shutdown().await,"production AgentManager shutdown");
    save(fx,"result",&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev,"source":"shipped App, actual macOS WKWebView, real shell PTY; native window hide, NSApplication hide/reactivate and native dimension changes; passive IPC counts"}));
    app.exit(if ck.failed.is_empty(){0}else{1});
}
