//! Round-four shipped App/WKWebView acceptance (macOS; one explicit first request per engine).
//! Build once: npm run agents:build && npm run build
//! Real phase (configured provider/model routes and authenticated CLIs; two read-only model requests):
//! DGOS_PROJECT_ACCEPTANCE_ROOT=/private/tmp/dgos-round4-<unique> DGOS_PROJECT_ACCEPTANCE_PHASE=round4-native cargo test -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture
//! Separate synthetic pending phase (protocol fixture, NOT a real engine/model):
//! DGOS_PROJECT_ACCEPTANCE_ROOT=/private/tmp/dgos-round4-pending-<unique> DGOS_PROJECT_ACCEPTANCE_PHASE=round4-pending-fixture cargo test -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture
//! Each root must be fresh. Screenshots require macOS Screen Recording permission.
//! Native keys below are synthesized NSEvents, not physical keyboard evidence. IME
//! composition events are a separate DOM boundary probe, not an installed input method.
use super::*;
use super::super::ui_keys::{self, Key};

type Journey<T> = Result<T, String>;
const SOURCE: &str = "module.exports = { value: 1, manual: false };\n";
const CHANGED: &str = "module.exports = { value: 2, manual: false };\n";
const FIRST_PROMPT: &str = "Read workflow.cjs, verify.cjs and package.json and briefly describe this fixture project. Do not modify any files, run commands, install dependencies or perform Git writes. This is the single explicit read-only first request for Round4 lifecycle acceptance.";

fn save(fx: &Fx, name: &str, data: &Value) {
    fs::write(fx.root.join(format!("round4-{name}.json")), serde_json::to_vec_pretty(data).expect("round4 JSON")).expect("round4 evidence");
}
fn require(fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, ok: bool, detail: impl Display) -> Journey<()> {
    record(ck,ev,name,ok,detail);
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
    if !succeeded(&reply) { return Err(format!("{command}: {}",reply.describe())); }
    let result=value(&reply);
    if result.get("success").is_some() {
        if result["success"]==true { Ok(result["data"].clone()) } else { Err(format!("{command}: {result}")) }
    } else { Ok(result) }
}
async fn session(app: &AppHandle, id: &str) -> Journey<Value> { host(app,"get_agent_session",json!({"sessionId":id})).await }
async fn capture(app: &AppHandle, fx: &Fx, name: &str) -> Journey<String> {
    // Shared capture records actual NSWindow number, process PID and capture command.
    layout_round2::capture(app,fx,&format!("round4-{name}")).await
}
fn prepare(path: &Path) -> Journey<()> {
    if path.exists() { return Err(format!("fresh fixture path required: {}",path.display())); }
    make_repo(path);
    fs::write(path.join("workflow.cjs"),SOURCE).map_err(|e|e.to_string())?;
    fs::write(path.join("verify.cjs"),"const assert = require('node:assert/strict'); const value = require('./workflow.cjs'); assert.deepEqual(value,{value:2,manual:true}); console.log('ROUND3_VERIFY '+JSON.stringify({pid:process.pid,cwd:process.cwd(),value}));\n").map_err(|e|e.to_string())?;
    fs::write(path.join("package.json"),r#"{"scripts":{"test":"node verify.cjs"}}"#).map_err(|e|e.to_string())?;
    if !git(path,&["add","."])||!git(path,&["-c","user.name=dgos","-c","user.email=dgos@example.invalid","commit","-qm","round4 host fixture"]) { return Err("fixture commit failed".into()); }
    Ok(())
}
async fn observe(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    step(app,fx,ck,ev,"passively observe production IPC without replacing host responses",json!({}),r#"
const original=window.fetch,p={original,events:[],keys:[]};window.__round4=p;window.__round3Workflows=p;
window.fetch=function(url,options,...rest){const command=String(url).split('/').at(-1);p.events.push({command,args:JSON.parse(options?.body||'{}'),at:Date.now()});return original.call(this,url,options,...rest);};
document.addEventListener('keydown',e=>p.keys.push({key:e.key,code:e.code,isTrusted:e.isTrusted,isComposing:e.isComposing,target:e.target?.getAttribute?.('data-testid')||e.target?.id||e.target?.className}),true);return {ok:true};
"#).await?;
    Ok(())
}
async fn attach(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, original: &Value) -> Journey<()> {
    step(app,fx,ck,ev,"continue saved live task without native reopen or model send",json!({"sid":sid}),r#"
const p=window.__round4,start=p.events.length;const projects=await need('dev-tool-projects');if(projects.getAttribute('aria-expanded')!=='true')projects.click();const create=document.querySelector('.dev-tree-project-row.is-current [data-testid^="dev-project-new-"]');if(!create||create.disabled)throw Error('new conversation unavailable');create.click();await need('dev-project-conversation');if(projects.getAttribute('aria-expanded')!=='true')projects.click();const row=await need('dev-session-item-'+A.sid),select=row.querySelector('.dev-session-select');if(!select||select.disabled)throw Error('saved live session select absent');select.click();
if(!await until(()=>document.getElementById('agent-message')&&!document.querySelector('[data-testid="dev-project-conversation"]')))throw Error('continued composer absent');if(document.querySelector('.agent-workbench')?.dataset.compact==='true'&&projects.getAttribute('aria-expanded')==='true')projects.click();
await new Promise(r=>setTimeout(r,350));const events=p.events.slice(start);return {ok:events.some(e=>e.command==='get_agent_session'&&e.args.sessionId===A.sid)&&!events.some(e=>['open_agent_session','send_agent_message'].includes(e.command)),events};
"#).await?;
    let current=session(app,sid).await?;
    require(fx,ck,ev,"live continue retains persisted session native ID and history",current["id"]==original["id"]&&current["native_id"]==original["native_id"]&&current["events"]==original["events"]&&current["status"]=="idle",current.to_string())
}

// WCAG relative luminance, alpha compositing through real ancestors, direct-text
// elements only. Gradients/images/filters are explicitly unresolved and fail the
// focused surfaces instead of being reported as a computed passing contrast.
const AUDIT: &str = r#"
const color=s=>{let m=/^rgba?\(([^)]+)\)$/.exec(s),scale=1;if(!m){m=/^color\(srgb ([^)]+)\)$/.exec(s);scale=255;}if(!m)return null;const n=m[1].split(/[ ,/]+/).filter(Boolean).map(Number);return {rgb:n.slice(0,3).map(x=>x*scale),a:n[3]??1};};
const blend=(a,b)=>a.rgb.map((x,i)=>x*a.a+b[i]*(1-a.a));
const background=e=>{const stack=[];for(let n=e;n;n=n.parentElement){const s=getComputedStyle(n);if(s.backgroundImage!=='none'||s.filter!=='none'||Number(s.opacity)<1)return null;const c=color(s.backgroundColor);if(!c)return null;stack.push(c);if(c.a>=.999)break;}if(!stack.some(c=>c.a>=.999))return null;let b=[0,0,0];for(const c of stack.reverse())b=blend(c,b);return b;};
const luminance=c=>c.map(x=>{x/=255;return x<=.04045?x/12.92:((x+.055)/1.055)**2.4;}).reduce((a,x,i)=>a+x*[.2126,.7152,.0722][i],0);
const contrast=(a,b)=>(Math.max(luminance(a),luminance(b))+.05)/(Math.min(luminance(a),luminance(b))+.05);
const rect=e=>{const r=e.getBoundingClientRect();return {x:r.x,y:r.y,right:r.right,bottom:r.bottom,width:r.width,height:r.height};};
const vw=document.documentElement.clientWidth,vh=document.documentElement.clientHeight;
const roots=A.surface==='model-picker'?[tid('composer-model-picker-dialog')]:['dev-panel-agent','dev-panel-code','dev-panel-runtime'].map(tid).filter(e=>visible(e)&&!e.closest('[inert]'));
if(!roots.length||roots.some(e=>!e))throw Error('audit surface absent');
const samples=[],unresolved=[],failures=[];
for(const root of roots)for(const e of [root,...root.querySelectorAll('*')]){if(!visible(e)||e.closest('.sr-only,.xterm-accessibility-tree')||e.matches('script,style,svg,svg *')||e.closest('button:disabled,input:disabled,textarea:disabled,select:disabled,[aria-disabled=true]'))continue;
const own=[...e.childNodes].filter(n=>n.nodeType===Node.TEXT_NODE).map(n=>n.textContent).join('').trim();if(!own&&!e.matches('input,textarea'))continue;
const s=getComputedStyle(e),bg=background(e),fg=color(s.color),large=parseFloat(s.fontSize)>=24||(parseFloat(s.fontSize)>=18.66&&Number(s.fontWeight)>=700),threshold=large?3:4.5;
const item={testid:e.getAttribute('data-testid'),tag:e.tagName,text:(own||e.value||e.placeholder||'').slice(0,120),color:s.color,background:bg,fontSize:s.fontSize,fontWeight:s.fontWeight,threshold,geometry:rect(e)};
if(!bg||!fg){unresolved.push(item);continue;}item.ratio=contrast(blend(fg,bg),bg);samples.push(item);if(item.ratio+.01<threshold)failures.push(item);}
const panels=roots.map(e=>({testid:e.getAttribute('data-testid'),...rect(e)})),clipped=panels.filter(r=>r.x< -1||r.y< -1||r.right>vw+1||r.bottom>vh+1||r.width<1||r.height<1);
const overlaps=[];if(A.surface!=='model-picker')for(let i=0;i<panels.length;i++)for(let j=i+1;j<panels.length;j++){const a=panels[i],b=panels[j];if(Math.min(a.right,b.right)-Math.max(a.x,b.x)>1&&Math.min(a.bottom,b.bottom)-Math.max(a.y,b.y)>1)overlaps.push([a.testid,b.testid]);}
const active=document.activeElement,style=getComputedStyle(active),g=rect(active),outlined=style.outlineStyle!=='none'&&parseFloat(style.outlineWidth)>0,field=active.matches('input,textarea,select,[role=combobox]'),bg=background(outlined&&parseFloat(style.outlineOffset)>=0?active.parentElement||active:active),ink=color(outlined?style.outlineColor:field?style.borderTopColor:''),indicator=outlined||field&&style.borderTopStyle!=='none'&&parseFloat(style.borderTopWidth)>0,ring=ink&&bg?contrast(blend(ink,bg),bg):null;
const focus={tag:active.tagName,id:active.id,testid:active.getAttribute('data-testid'),within:roots.some(e=>e.contains(active)),visible:active.matches(':focus-visible'),geometry:g,indicatorVisible:indicator,contrast:ring,cuts:[]};
const grow=outlined?Math.max(0,parseFloat(style.outlineWidth)+parseFloat(style.outlineOffset)):0;for(let n=active.parentElement;n;n=n.parentElement){const s=getComputedStyle(n),r=rect(n);if(s.overflowX!=='visible'&&(g.x-grow<r.x-1||g.right+grow>r.right+1)||s.overflowY!=='visible'&&(g.y-grow<r.y-1||g.bottom+grow>r.bottom+1))focus.cuts.push({testid:n.getAttribute('data-testid'),tag:n.tagName,geometry:r});}
const focusOK=focus.within&&focus.visible&&indicator&&ring>=3&&g.x>=0&&g.y>=0&&g.right<=vw&&g.bottom<=vh&&!focus.cuts.length;
return {ok:samples.length>0&&!failures.length&&!unresolved.length&&!clipped.length&&!overlaps.length&&document.documentElement.scrollWidth<=vw+1&&focusOK,surface:A.surface,viewport:{width:vw,height:vh},theme:document.documentElement.dataset.theme,uiSize:getComputedStyle(document.documentElement).getPropertyValue('--font-size-ui'),panels,clipped,overlaps,samples,failures,unresolved,focus,focusOK};
"#;

async fn appearance(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, mode: &str, large: bool) -> Journey<()> {
    step(app,fx,ck,ev,"apply production appearance without reloading live renderer",json!({"mode":mode,"large":large}),r#"
const settings=$$('a[href="/settings"]')[0];if(!settings)throw Error('settings entry absent');settings.click();
const category=await until(()=>$$('.workspace-settings-category[role=tab]').find(e=>txt(e)==='外观'));if(category){category.focus();category.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));}else{const c=await until(()=>$$('.workspace-settings-select [role=combobox]')[0]);if(!c)throw Error('settings category control absent');c.click();const o=await until(()=>$$('[role=option]').find(e=>txt(e)==='外观'));if(!o)throw Error('appearance category option absent');o.click();}
const page=await until(()=>$$('.appearance-editor')[0]);if(!page)throw Error('appearance page absent');
const group=$$('[role=radiogroup]',page).find(e=>e.getAttribute('aria-label')==='主题'||txt(document.getElementById(e.getAttribute('aria-labelledby')))==='主题');if(!group)throw Error('theme group absent');const radio=$$('[role=radio]',group).find(e=>labelOf(e)===(A.mode==='light'?'亮色':'暗色'));if(!radio)throw Error('theme radio absent');radio.click();
for(const [label,value] of [['界面字号',A.large?16:12],['阅读字号',A.large?20:13],['代码字号',A.large?18:12]]){const c=await until(()=>$$('[role=combobox]',page).find(e=>labelOf(e)===label));if(!c)throw Error('size control absent '+label);if(txt(c)===value+'px')continue;c.click();const o=await until(()=>$$('[role=option]').find(e=>txt(e)===value+'px'));if(!o)throw Error('size option absent');o.click();if(!await until(()=>!$$('[role=listbox]').length&&visible(c)&&txt(c)===value+'px'))throw Error('font select did not settle '+label);}
const b=$$('button',page).find(e=>txt(e)==='应用外观');if(!b)throw Error('apply absent');if(!b.disabled){b.click();if(!await until(()=>b.disabled&&!b.getAttribute('aria-busy')))throw Error('appearance apply incomplete');}return {ok:true};
"#).await?;
    ui_keys::key(app,Key::Escape).await;
    step(app,fx,ck,ev,"settings closes with original live surface preserved",json!({}),"return {ok:!!await until(()=>!document.querySelector('.workspace-settings-dialog')&&document.getElementById('agent-message'))};").await?;
    let read=host(app,"get_appearance_preferences",json!({})).await?;
    require(fx,ck,ev,"host confirms exact saved appearance snapshot",read["snapshot"]["mode"]==mode&&read["snapshot"]["typography"]["uiSize"]==if large {16}else{12},read.to_string())
}
async fn matrix(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str) -> Journey<()> {
    let window=app.get_webview_window("main").ok_or("main window absent")?;
    let mut cells=Vec::new();
    for (width,height) in [(1440,900),(1280,800),(800,600)] {
        window.set_size(tauri::LogicalSize::new(width as f64,height as f64)).map_err(|e|e.to_string())?;
        for mode in ["light","dark"] { for large in [false,true] {
            appearance(app,fx,ck,ev,mode,large).await?;
            require(fx,ck,ev,"native foreground prepared for synthesized keys",ui_keys::prepare(app).await,"NSApplication + WKWebView first responder")?;
            let name=format!("{width}x{height}-{mode}-{}",if large {"large"}else{"default"});
            for surface in ["agent","code","model-picker"] {
                step(app,fx,ck,ev,"prepare visible responsive audit surface",json!({"surface":surface}),r#"
const compact=document.querySelector('.agent-workbench')?.dataset.compact==='true';const active=$$('[data-testid^="dev-tool-"]').find(e=>e.getAttribute('aria-expanded')==='true');if(compact&&active)active.click();
if(A.surface==='model-picker'){const projects=await need('dev-tool-projects');if(projects.getAttribute('aria-expanded')!=='true')projects.click();const create=document.querySelector('.dev-tree-project-row.is-current [data-testid^="dev-project-new-"]');if(!create||create.disabled)throw Error('new conversation unavailable');create.click();await need('dev-project-conversation');const b=await need('composer-model-picker-trigger');b.focus();return {ok:document.activeElement===b};}
if(A.surface==='agent'){const f=await until(()=>document.getElementById('agent-message'));if(!f)throw Error('composer absent');f.focus();return {ok:document.activeElement===f};}
const files=await need('dev-tool-files');if(files.getAttribute('aria-expanded')!=='true')files.click();const b=await need('dev-review-tab-files');b.focus();return {ok:document.activeElement===b};
"#).await?;
                if surface=="model-picker" {
                    ui_keys::key(app,Key::Enter).await;
                    step(app,fx,ck,ev,"native Enter opens named modal model picker",json!({}),r#"const d=await need('composer-model-picker-dialog');return {ok:d.getAttribute('role')==='dialog'&&d.getAttribute('aria-modal')==='true'&&!!labelOf(d)&&d.contains(document.activeElement),role:d.getAttribute('role'),modal:d.getAttribute('aria-modal'),name:labelOf(d),labelledby:d.getAttribute('aria-labelledby'),focusInside:d.contains(document.activeElement),active:document.activeElement?.outerHTML};"#).await?;
                }
                ui_keys::key(app,Key::Tab).await;
                let _=dom(app,json!({}),"await Promise.allSettled(document.getAnimations().map(a=>a.finished));return {ok:true};").await;
                let measured=dom(app,json!({"surface":if surface=="model-picker" {"model-picker"}else{"panels"}}),AUDIT).await;
                let png=capture(app,fx,&format!("{name}-{surface}")).await?;
                let scale=window.scale_factor().map_err(|e|e.to_string())?;
                let inner=window.inner_size().map_err(|e|e.to_string())?.to_logical::<f64>(scale);
                let native=json!({"requested_logical":{"width":width,"height":height},"inner_logical":{"width":inner.width,"height":inner.height},"scale_factor":scale,"host_pid":std::process::id()});
                cells.push(json!({"cell":name,"surface":surface,"measurement":measured,"native":native,"png":png}));save(fx,"matrix",&json!(cells));
                record(ck,ev,&format!("computed contrast geometry native focus {name} {surface}"),ui_ok(&measured)&&inner.width==width as f64&&inner.height==height as f64&&measured["value"]["theme"]==mode&&measured["value"]["uiSize"].as_str().is_some_and(|s|s.trim()==if large {"16px"}else{"12px"}),measured.to_string());
                save(fx,"checkpoint",&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev}));
                if surface=="model-picker" {
                    for key in [Key::Tab,Key::ShiftTab] { for _ in 0..8 { ui_keys::key(app,key).await;step(app,fx,ck,ev,"native Tab remains trapped in model picker",json!({}),"const d=await need('composer-model-picker-dialog');return {ok:d.contains(document.activeElement)};").await?; } }
                    ui_keys::key(app,Key::Escape).await;
                    step(app,fx,ck,ev,"native Escape dismisses picker and restores opener focus",json!({}),"return {ok:!!await until(()=>!tid('composer-model-picker-dialog')&&document.activeElement===tid('composer-model-picker-trigger'))};").await?;
                    step(app,fx,ck,ev,"return from picker audit to exact live session without another send",json!({"sid":sid}),r#"const p=window.__round4,start=p.events.length;const projects=await need('dev-tool-projects');if(projects.getAttribute('aria-expanded')!=='true')projects.click();const row=await need('dev-session-item-'+A.sid),b=row.querySelector('.dev-session-select');if(!b||b.disabled)throw Error('audited session select absent');b.click();if(!await until(()=>document.getElementById('agent-message')&&!document.querySelector('[data-testid="dev-project-conversation"]')))throw Error('audited session restore absent');if(document.querySelector('.agent-workbench')?.dataset.compact==='true'&&projects.getAttribute('aria-expanded')==='true')projects.click();return {ok:!p.events.slice(start).some(e=>['open_agent_session','send_agent_message'].includes(e.command))};"#).await?;
                }
            }
        } }
    }
    appearance(app,fx,ck,ev,"light",false).await?;
    window.set_size(tauri::LogicalSize::new(1440.0,900.0)).map_err(|e|e.to_string())?;
    Ok(())
}

async fn engine_status(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, engine: &str, status: &str) -> Journey<()> {
    step(app,fx,ck,ev,"active engine exposes one contextual status announcement",json!({"engine":engine,"status":status}),r#"
const announcement=await need('agent-session-announcement'),state=await until(()=>$$('.agent-dialogue-status').find(e=>e.dataset.state===A.status)),name=A.engine==='codex'?'Codex':'Claude';
const announcements=$$('[data-testid="agent-session-announcement"]').filter(e=>e.getAttribute('role')==='status'||e.hasAttribute('aria-live'));
return {ok:!!state&&txt(announcement).includes(name)&&txt(announcement).includes(txt(state))&&announcements.length===1&&state.getAttribute('role')!=='status'&&!state.hasAttribute('aria-live')&&!$$('[data-testid^="agent-event-"][role="alert"],[data-testid^="agent-event-"][aria-live]').length,text:txt(announcement),state:txt(state),engine:A.engine};
"#).await?;
    capture(app,fx,&format!("{engine}-{status}")).await?;
    Ok(())
}
async fn composition(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str) -> Journey<()> {
    let before=session(app,sid).await?;
    step(app,fx,ck,ev,"DOM composition boundary does not send or clear composer draft",json!({}),r#"
const f=document.getElementById('agent-message');if(!f)throw Error('composer absent');setInput(f,'输入法边界验收');f.focus();const p=window.__round4,start=p.events.length;
f.dispatchEvent(new CompositionEvent('compositionstart',{data:'输入',bubbles:true}));f.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',code:'Enter',metaKey:true,isComposing:true,bubbles:true}));f.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',code:'Enter',metaKey:true,keyCode:229,bubbles:true}));f.dispatchEvent(new CompositionEvent('compositionend',{data:'输入',bubbles:true}));
await new Promise(r=>setTimeout(r,300));const events=p.events.slice(start);return {ok:f.value==='输入法边界验收'&&!events.some(e=>e.command==='send_agent_message'),events,evidence:'DOM-synthesized composition boundary only; no installed OS IME or physical typing claim'};
"#).await?;
    require(fx,ck,ev,"composition boundary leaves native session history unchanged",session(app,sid).await?["events"]==before["events"],sid)
}
async fn launch_real(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, engine: &str, path: &Path) -> Journey<(String,String,String)> {
    let (project,task,sid)=round3_workflows::launch(app,fx,ck,ev,engine,path,FIRST_PROMPT).await?;
    let first=stage2::settle(app,&sid,true,0).await?;
    require(fx,ck,ev,"single explicit read-only first request finishes before host baseline",first["status"]=="idle"&&fs::read_to_string(path.join("workflow.cjs")).map_err(|e|e.to_string())?==SOURCE,first.to_string())?;
    // The first request is read-only. Capture the unchanged task baseline through
    // a real explicit host process before the harness introduces its disk change.
    let baseline=layout_round2::start_process(app,fx,ck,ev,&task,"command",&["-c","printf 'ROUND4_BASELINE_HOST:%s\\n' \"$$\""]).await?;
    let receipt=round3_terminal::host(app,&baseline,"ROUND4_BASELINE_HOST:").await?;
    let baseline_done=loop {let row=host(app,"read_development_process",json!({"id":baseline})).await?;if row["finished_at"].is_number(){break row;}tokio::time::sleep(Duration::from_millis(100)).await;};
    require(fx,ck,ev,"baseline host process finishes before explicit external disk change",baseline_done["id"]==baseline&&baseline_done["finished_at"].is_number()&&baseline_done["exit_code"]==0,baseline_done.to_string())?;
    save(fx,&format!("{engine}-baseline-host"),&json!({"output":receipt,"finished":baseline_done,"first_request":first}));
    Ok((project,task,sid))
}
async fn per_engine(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, engine: &str) -> Journey<()> {
    let path=fx.root.join(format!("round4-{engine}"));prepare(&path)?;
    let (project,task,sid)=launch_real(app,fx,ck,ev,engine,&path).await?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(45);
    let original=loop {let s=session(app,&sid).await?;if s["status"]=="idle" {break s;}if tokio::time::Instant::now()>=deadline {return Err(format!("native first-request idle timeout: {s}"));}tokio::time::sleep(Duration::from_millis(100)).await;};
    let workspace=host(app,"read_development_workspace",json!({})).await?;
    let persisted=workspace["sessions"].as_array().and_then(|rs|rs.iter().find(|s|s["id"]==sid)).cloned().ok_or("persisted native session absent")?;
    require(fx,ck,ev,"explicit first request binds persisted task session and workspace exactly once",persisted["task_id"]==task&&persisted["project_id"]==project&&persisted["engine"]==engine&&original["workspace"]==path.to_string_lossy().as_ref()&&original["events"].as_array().is_some_and(|rs|rs.iter().filter(|e|e["kind"]=="userMessage").count()==1),json!({"persisted":persisted,"live":original}).to_string())?;
    save(fx,&format!("{engine}-first-request"),&original);
    engine_status(app,fx,ck,ev,engine,"idle").await?;
    composition(app,fx,ck,ev,&sid).await?;
    attach(app,fx,ck,ev,&sid,&original).await?;
    // An explicit external fixture write supplies disk changes without asking a model
    // to repeat unchanged engine work. It is never presented as a native file event.
    fs::write(path.join("workflow.cjs"),CHANGED).map_err(|e|e.to_string())?;
    step(app,fx,ck,ev,"open actual external disk diff without fabricating native tool events",json!({}),r#"
const tool=await need('dev-tool-git');if(tool.getAttribute('aria-expanded')!=='true')tool.click();const b=await need('dev-review-tab-changes');b.focus();b.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));await click('dev-review-refresh');await click('dev-change-item-workflow.cjs');const d=await need('dev-file-diff');return {ok:!!await until(()=>txt(d).includes('value: 2')),origin:'explicit external harness disk mutation, not model output'};
"#).await?;
    capture(app,fx,&format!("{engine}-external-diff")).await?;
    let review=host(app,"read_development_review",json!({"taskId":task})).await?;
    require(fx,ck,ev,"host review records real disk change for exact persisted task",review["changes"].as_array().is_some_and(|rs|rs.iter().any(|r|r["path"]=="workflow.cjs")),review.to_string())?;
    if engine=="codex" {matrix(app,fx,ck,ev,&sid).await?;}
    step(app,fx,ck,ev,"open exact disk source for manual takeover",json!({"disk":CHANGED}),r#"const tool=await need('dev-tool-files');if(tool.getAttribute('aria-expanded')!=='true')tool.click();const b=await need('dev-review-tab-files');b.focus();b.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));await fill('dev-file-search','workflow.cjs');await click('dev-file-item-workflow.cjs');return {ok:!!await until(()=>tid('dev-file-editor')?.value===A.disk)};"#).await?;
    step(app,fx,ck,ev,"DOM IME composition cannot save the file draft",json!({"disk":CHANGED}),r#"
const f=await need('dev-file-editor'),p=window.__round4,start=p.events.length;setInput(f,A.disk+'// composition draft\n');f.focus();f.dispatchEvent(new CompositionEvent('compositionstart',{data:'输入',bubbles:true}));f.dispatchEvent(new KeyboardEvent('keydown',{key:'s',code:'KeyS',metaKey:true,isComposing:true,bubbles:true}));f.dispatchEvent(new KeyboardEvent('keydown',{key:'s',code:'KeyS',metaKey:true,keyCode:229,bubbles:true}));f.dispatchEvent(new CompositionEvent('compositionend',{data:'输入',bubbles:true}));await new Promise(r=>setTimeout(r,250));const events=p.events.slice(start);const ok=!events.some(e=>e.command==='save_development_file');setInput(f,A.disk);return {ok,events,evidence:'DOM composition/keyCode229 guard only; no OS IME claim'};
"#).await?;
    require(fx,ck,ev,"composition save guard preserves actual disk bytes",fs::read_to_string(path.join("workflow.cjs")).map_err(|e|e.to_string())?==CHANGED,path.display())?;
    round3_workflows::takeover(app,fx,ck,ev,engine,&project,&task,&sid,&path).await?;
    let process=round3_workflows::verify(app,fx,ck,ev,engine,&project,&task,&path).await?;
    // Do not manufacture a native approval: only the separately labeled protocol
    // fixture guarantees a pending request; this read-only first turn need not.
    save(fx,&format!("{engine}-approval-boundary"),&json!({"native_pending":original["pending"],"model_requests":1,"native_approval_exercised":false,"reason":"read-only first request does not guarantee model-generated approval; use round4-pending-fixture separately"}));
    step(app,fx,ck,ev,"focus review tab before native keyboard activation",json!({}),"const tool=await need('dev-tool-tasks');if(tool.getAttribute('aria-expanded')!=='true')tool.click();const b=await need('dev-review-tab-review');b.focus();return {ok:document.activeElement===b};").await?;
    ui_keys::key(app,Key::Enter).await;
    step(app,fx,ck,ev,"explicit review refresh reads current real host verification",json!({}),r#"const s=await need('dev-task-review'),b=await until(()=>$$('button',s).find(e=>txt(e)==='刷新'&&!e.disabled));if(!b)throw Error('review refresh absent');b.click();return {ok:!!await until(()=>tid('dev-task-review-verification')?.dataset.verification==='observed_stable'&&txt(s).includes('workflow.cjs'))};"#).await?;
    round3_workflows::review(app,fx,ck,ev,engine,&task,&process).await?;
    step(app,fx,ck,ev,"lifecycle and picker audits never add to the explicit first request",json!({"sid":sid,"prompt":FIRST_PROMPT}),r#"const events=window.__round4.events.filter(e=>(e.args.input?.session_id??e.args.input?.id??e.args.sessionId)===A.sid),sends=events.filter(e=>e.command==='send_agent_message'),opens=events.filter(e=>e.command==='open_agent_session');return {ok:sends.length===1&&sends[0].args.input?.text===A.prompt&&opens.length===1,sends,opens};"#).await?;
    save(fx,&format!("{engine}-real-host-result"),&json!({"project_id":project,"task_id":task,"session_id":sid,"session":session(app,&sid).await?,"process":process,"disk_path":path.join("workflow.cjs"),"disk":fs::read_to_string(path.join("workflow.cjs")).map_err(|e|e.to_string())?,"model_requests":1}));
    Ok(())
}

async fn terminal(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    step(app,fx,ck,ev,"open navigation for real PTY task",json!({}),"const b=await need('dev-tool-projects');if(b.getAttribute('aria-expanded')==='false')b.click();return {ok:true};").await?;
    let path=fx.root.join("round4-pty");make_repo(&path);
    let imported=ui_import_project(app,ck,ev,"Round4 PTY",&path).await;
    step(app,fx,ck,ev,"open task drawer for original PTY task",json!({}),"const b=await need('dev-tool-tasks');if(b.getAttribute('aria-expanded')!=='true')b.click();return {ok:true};").await?;
    let created=imported&&ui_new_task(app,ck,ev,"Round4 original PTY").await&&ui_select_task(app,ck,ev,"Round4 original PTY").await;
    require(fx,ck,ev,"create persisted PTY task through production forms",created,path.display())?;
    let ws=host(app,"read_development_workspace",json!({})).await?;
    let task=ws["selection"]["task_id"].as_str().ok_or("PTY task absent")?;
    let id=layout_round2::start_process(app,fx,ck,ev,task,"terminal",&[]).await?;
    let setup=host(app,"write_development_terminal",json!({"input":{"id":id,"data":"stty -echo; PS1=''; printf '%s\\n' \"$$\" > round4.pid; printf 'once\\n' >> round4.input; printf 'ROUND4_READY\\n'\r"}})).await?;
    let baseline=round3_terminal::host(app,&id,"ROUND4_READY").await?;
    let pid=fs::read_to_string(path.join("round4.pid")).map_err(|e|e.to_string())?;
    for cycle in 0..2 {
        step(app,fx,ck,ev,"focus nonterminal control before PTY visibility transition",json!({}),"const b=await need('dev-tool-tasks');b.focus();return {ok:document.activeElement===b};").await?;
        if cycle==0 {step(app,fx,ck,ev,"collapse runtime drawer without stopping PTY",json!({}),"await click('dev-tool-tasks');return {ok:tid('dev-tool-tasks')?.getAttribute('aria-expanded')==='false'};").await?;} else {round3_terminal::background(app).await?;}
        host(app,"write_development_terminal",json!({"input":{"id":id,"data":format!("printf 'ROUND4_HIDDEN_{cycle}\\n'\r")}})).await?;
        round3_terminal::host(app,&id,&format!("ROUND4_HIDDEN_{cycle}")).await?;
        if cycle==0 {step(app,fx,ck,ev,"restore original terminal drawer",json!({}),"await click('dev-tool-tasks');return {ok:true};").await?;}
        round3_terminal::foreground(app).await?;
        step(app,fx,ck,ev,"automatic PTY restoration preserves original nonterminal focus",json!({}),"return {ok:document.activeElement===tid('dev-tool-tasks')};").await?;
        round3_terminal::projection(app,fx,ck,ev,"round4 original PTY output projection",&format!("ROUND4_HIDDEN_{cycle}")).await?;
        // Host prepares text without submitting; only native WKWebView Enter may
        // deliver the final CR and execute it. This is labeled, not physical typing.
        host(app,"write_development_terminal",json!({"input":{"id":id,"data":format!("printf 'once\\n' >> round4.resume.{cycle}; printf 'ROUND4_KEY_{cycle}\\n'")}})).await?;
        step(app,fx,ck,ev,"focus actual xterm input for synthesized native Enter",json!({}),"const f=(await need('dev-terminal')).querySelector('.xterm-helper-textarea');if(!f)throw Error('terminal input absent');f.focus();return {ok:document.activeElement===f};").await?;
        ui_keys::key(app,Key::Enter).await;
        let returned=round3_terminal::host(app,&id,&format!("ROUND4_KEY_{cycle}")).await?;
        require(fx,ck,ev,"same original OS PTY executes native UI Enter exactly once",returned["id"]==baseline["id"]&&returned["started_at"]==baseline["started_at"]&&returned["task_id"]==task&&returned["status"]=="running"&&fs::read_to_string(path.join("round4.pid")).map_err(|e|e.to_string())?==pid&&fs::read_to_string(path.join("round4.input")).map_err(|e|e.to_string())?=="once\n"&&fs::read_to_string(path.join(format!("round4.resume.{cycle}"))).map_err(|e|e.to_string())?=="once\n",returned.to_string())?;
        save(fx,&format!("pty-return-{cycle}"),&json!({"baseline":baseline,"returned":returned,"actual_shell_pid":pid,"setup":setup,"input_evidence":"host prepares command text; synthesized native NSEvent Enter through actual xterm submits it once; not physical typing"}));
        capture(app,fx,&format!("pty-return-{cycle}")).await?;
    }
    let stopped=host(app,"stop_development_process",json!({"id":id})).await?;
    require(fx,ck,ev,"original PTY cleanup is confirmed by exact host receipt",stopped["id"]==baseline["id"]&&stopped["started_at"]==baseline["started_at"]&&stopped["finished_at"].is_number()&&stopped["status"]!="cleanup_pending",stopped.to_string())?;
    save(fx,"pty-cleanup",&json!({"baseline":baseline,"stopped":stopped,"shell_pid":pid}));
    Ok(())
}

// A real subprocess and production session transport, with a deliberately fake
// Codex protocol peer. Its pending request is NOT a model/provider/native-CLI claim.
#[cfg(unix)]
async fn pending_fixture(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    use std::os::unix::fs::PermissionsExt;
    let executable=fx.root.join("round4-codex-PROTOCOL-FIXTURE");
    // Production probes accept exactly three numeric semver components. The
    // executable name and evidence retain the synthetic provenance, not a suffix
    // that would make the real model picker reject this protocol-only peer.
    fs::write(&executable,r##"#!/bin/sh
if [ "$1" = "--version" ]; then printf '%s\n' 'codex-cli 0.0.0'; exit 0; fi
while IFS= read -r line; do
case "$line" in
*'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"thread/start"'*|*'"method":"thread/resume"'*) printf '%s\n' '{"id":2,"result":{"thread":{"id":"round4-SYNTHETIC-thread","turns":[]}}}';;
*'"method":"turn/start"'*) printf '%s\n' '{"id":3,"result":{"turn":{"id":"round4-SYNTHETIC-turn"}}}' '{"method":"turn/started","params":{"turn":{"id":"round4-SYNTHETIC-turn"}}}' '{"id":"round4-SYNTHETIC-approval","method":"item/commandExecution/requestApproval","params":{"command":"ROUND4 PROTOCOL FIXTURE ONLY: no command or model execution","availableDecisions":["accept","decline"]}}';;
*'"decision":"decline"'*) printf '%s\n' 'decline' >> round4-fixture-decisions; printf '%s\n' '{"method":"turn/completed","params":{"turn":{"id":"round4-SYNTHETIC-turn","status":"completed"}}}';;
*'"decision":"accept"'*) printf '%s\n' 'accept' >> round4-fixture-decisions; printf '%s\n' '{"method":"turn/completed","params":{"turn":{"id":"round4-SYNTHETIC-turn","status":"completed"}}}';;
esac
done
"##).map_err(|e|e.to_string())?;
    fs::set_permissions(&executable,fs::Permissions::from_mode(0o700)).map_err(|e|e.to_string())?;
    let current=host(app,"get_cli_preferences",json!({})).await?;
    let mut paths=current["paths"].clone();paths["codex"]=json!(executable);
    let saved=host(app,"set_cli_preferences",json!({"paths":paths,"expectedRevision":current["revision"]})).await?;
    require(fx,ck,ev,"isolated account explicitly selects labeled protocol fixture",saved["saved"]==true,saved.to_string())?;
    let path=fx.root.join("round4-synthetic-pending");prepare(&path)?;
    let (project,task,sid)=round3_workflows::launch(app,fx,ck,ev,"codex",&path,"SYNTHETIC protocol request; no real engine/model").await?;
    // The first direct send is the fixture request; never add a second turn.
    let deadline=tokio::time::Instant::now()+Duration::from_secs(20);
    let pending=loop {let v=session(app,&sid).await?;if v["pending"].as_array().is_some_and(|rs|rs.iter().any(|r|r["kind"]=="item/commandExecution/requestApproval"&&r["data"]["command"]=="ROUND4 PROTOCOL FIXTURE ONLY: no command or model execution")){break v;}if tokio::time::Instant::now()>=deadline{return Err(format!("fixture pending timeout: {v}"));}tokio::time::sleep(Duration::from_millis(100)).await;};
    composition(app,fx,ck,ev,&sid).await?;
    engine_status(app,fx,ck,ev,"codex","waiting").await?;
    step(app,fx,ck,ev,"focus explicit deny on labeled synthetic pending request",json!({}),r#"const s=await until(()=>$$('section[aria-label]').find(e=>txt(e).includes('ROUND4 PROTOCOL FIXTURE ONLY')));if(!s)throw Error('fixture approval surface absent');const b=$$('button',s).find(b=>txt(b)==='拒绝'&&!b.disabled);if(!b)throw Error('deny absent');b.focus();return {ok:document.activeElement===b};"#).await?;
    capture(app,fx,"SYNTHETIC-pending-before-decision").await?;
    ui_keys::key(app,Key::Enter).await;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(20);
    let after=loop {let v=session(app,&sid).await?;if v["status"]=="idle"&&v["pending"].as_array().is_some_and(Vec::is_empty){break v;}if tokio::time::Instant::now()>=deadline{return Err(format!("fixture decision timeout: {v}"));}tokio::time::sleep(Duration::from_millis(100)).await;};
    require(fx,ck,ev,"native synthesized Enter sends one scoped explicit fixture denial",fs::read_to_string(path.join("round4-fixture-decisions")).map_err(|e|e.to_string())?=="decline\n",after.to_string())?;
    step(app,fx,ck,ev,"removing resolved fixture approval retains meaningful keyboard focus",json!({}),r#"if(!await until(()=>!$$('section[aria-label]').some(e=>txt(e).includes('ROUND4 PROTOCOL FIXTURE ONLY'))))throw Error('resolved approval remains visible');const a=document.activeElement;return {ok:a!==document.body&&a.isConnected&&visible(a)&&!!a.closest('.agent-workbench'),active:a.outerHTML};"#).await?;
    save(fx,"SYNTHETIC-pending",&json!({"fixture":true,"real_model":false,"real_native_cli":false,"source":"local fake Codex protocol subprocess using production session transport and shipped UI","executable":executable,"project_id":project,"task_id":task,"session_id":sid,"before":pending,"after":after,"decision_file":path.join("round4-fixture-decisions")}));
    let closed=ui_close_run(app,ck,ev).await;
    require(fx,ck,ev,"close synthetic protocol session through UI",closed,sid)
}
#[cfg(not(unix))]
async fn pending_fixture(_app: &AppHandle,_fx: &Fx,_ck: &mut Checks,_ev: &mut Vec<Value>) -> Journey<()> { Err("protocol fixture requires Unix".into()) }

pub(super) async fn run(app: &AppHandle, fx: &Fx, mut ck: Checks, mut ev: Vec<Value>) {
    let fixture=std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").is_ok_and(|p|p=="round4-pending-fixture");
    save(fx,"commands",&json!({"build":["npm run agents:build","npm run build"],"run":format!("DGOS_PROJECT_ACCEPTANCE_ROOT='{}' DGOS_PROJECT_ACCEPTANCE_PHASE={} cargo test -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture",fx.root.display(),if fixture {"round4-pending-fixture"}else{"round4-native"}),"model_requests":if fixture {0}else{2},"explicit_first_sends":if fixture {1}else{2},"prerequisites":["Fresh isolated root","Enabled provider/model catalog with an available route for each requested engine; native CLI authentication alone is insufficient","Authenticated native CLIs for round4-native","macOS Screen Recording permission"],"keyboard":"synthesized NSEvent, not physical","ime":"DOM CompositionEvent guard only, not OS input-method candidate selection","fixture":fixture,"matrix":{"sizes":[[1440,900],[1280,800],[800,600]],"themes":["light","dark"],"fonts":["default UI12/reading13/code12","large UI16/reading20/code18"]}}));
    let provenance=dom(app,json!({}),r#"
const urls=[...new Set([...document.querySelectorAll('script[src],link[rel=stylesheet]')].map(e=>e.src||e.href).concat(performance.getEntriesByType('resource').map(e=>e.name).filter(u=>u.includes('/assets/'))))];const resources=await Promise.all(urls.map(async url=>{try{const r=await fetch(url,{cache:'no-store'});if(!r.ok)throw Error(String(r.status));const bytes=await r.arrayBuffer(),hash=[...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(x=>x.toString(16).padStart(2,'0')).join('');return {url,bytes:bytes.byteLength,sha256:hash};}catch(e){return {url,error:String(e)};}}));return {ok:resources.length>0&&resources.every(r=>r.sha256),url:location.href,userAgent:navigator.userAgent,resources};
"#).await;
    save(fx,"served-assets",&provenance);
    let result:Journey<()>=async {
        require(fx,&mut ck,&mut ev,"round4 evidence stays in a fresh isolated temporary root",fx.root.starts_with("/private/tmp")&&fx.root!=Path::new("/private/tmp"),fx.root.display())?;
        observe(app,fx,&mut ck,&mut ev).await?;
        if fixture {pending_fixture(app,fx,&mut ck,&mut ev).await?;}else{for engine in ENGINES {per_engine(app,fx,&mut ck,&mut ev,engine).await?;}terminal(app,fx,&mut ck,&mut ev).await?;}
        let observation=step(app,fx,&mut ck,&mut ev,"native key receipt and explicit first-send consumption boundaries recorded",json!({"fixture":fixture,"prompt":FIRST_PROMPT}),r#"const p=window.__round4,sends=p.events.filter(e=>e.command==='send_agent_message'),keys=p.keys;return {ok:sends.length===(A.fixture?1:2)&&(A.fixture||sends.every(e=>e.args.input?.text===A.prompt))&&keys.some(e=>e.isTrusted&&e.key==='Enter'),sends,keys,evidence:'trusted WKWebView key events from NSApplication NSEvent synthesis; not physical keyboard'};"#).await?;save(fx,"ipc-and-native-keys",&observation);
        Ok(())
    }.await;
    if let Err(error)=result {record(&mut ck,&mut ev,"round4 focused native phase completes",false,error);save(fx,"failure-dom",&dom(app,json!({}),"return {ok:true,url:location.href,body:document.body.innerText,active:document.activeElement?.outerHTML,events:window.__round4?.events,keys:window.__round4?.keys};").await);let _=capture(app,fx,"failure").await;}
    let _=dom(app,json!({}),"const p=window.__round4;if(p)window.fetch=p.original;return {ok:true};").await;
    record(&mut ck,&mut ev,"owned native sessions and PTYs cleaned up",app.state::<AgentManager>().shutdown().await,"production AgentManager shutdown");
    save(fx,"result",&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev,"fixture":fixture,"model_requests":if fixture {0}else{2},"source":if fixture {"SYNTHETIC Codex protocol subprocess; production App/WKWebView/IPC, not real model/native CLI proof"}else{"real Codex/Claude single explicit read-only first request per engine; production App/WKWebView/host/disk/PID/task/session/process receipts"},"limitations":["No physical keyboard evidence: native keys are synthesized NSEvents","IME composition guard uses DOM events; no OS IME candidate-window proof","Native phase consumes one explicit read-only first model request per engine; all subsequent lifecycle and picker transitions must not send","Real phase does not manufacture pending approvals; separately labeled protocol fixture supplies that UI state","Computed text/focus contrast does not claim a full VoiceOver or platform accessibility-tree audit"]}));
    app.exit(if ck.failed.is_empty(){0}else{1});
}
