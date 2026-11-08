//! Isolated production App/WKWebView round-two layout acceptance; no provider/model calls.
//! Build dist first. Use a fresh DGOS_PROJECT_ACCEPTANCE_ROOT under /private/tmp and
//! DGOS_PROJECT_ACCEPTANCE_PHASE=layout-round2 cargo test -p dgos --features webview-smoke
//! --test webview_smoke_agents -- --nocapture
//! Optional DGOS_LAYOUT_ROUND2_INTERACTIVE=1 waits BEFORE assertions for control.json.
//! Commands: {"id":"unique","action":"run|eval|capture|finish","script":"return {ok:true}"}.
//! Replies: layout-round2-control-result.json; final evidence: layout-round2-result.json.
use super::*;

type Journey<T> = Result<T, String>;
const LAYOUT_KEY: &str = "dgos.development.layout.v2";

// The renderer is the shipped full App. Install the real catalog service rather than
// fabricating catalog responses or hiding the stripped-harness error banner.
pub(super) async fn install(app: &mut tauri::App) {
    use dgos_application::{ModuleCrashService, ModuleManagementService, ModuleOperationManager};
    use dgos_db::{ModuleCrashRepository, ModulePermissionRepository};
    use dgos_modules::{LocalModuleSources, ModulePublisher, PageLoaderConfig, PageModuleLoader};
    let fx = app.state::<Fx>();
    let publisher = Arc::new(ModulePublisher::new(fx.root.join("modules"), fx.db.clone()));
    let (loader, lost) = PageModuleLoader::new(Arc::new(crate::module_host::ChannelTransport::default()), PageLoaderConfig {
        window_label: "main".into(), hook_timeout: Duration::from_secs(2),
    });
    let operations = ModuleOperationManager::new(publisher, Arc::new(loader), LocalModuleSources::new(fx.root.join("module-sources")))
        .with_maintenance(app.state::<dgos_db::MaintenanceGate>().inner().clone());
    operations.spawn_lost_runtime_reporter(lost).expect("layout module reporter");
    operations.recover().await.expect("layout module recovery");
    let crashes = ModuleCrashService::new(ModuleCrashRepository::new(fx.db.clone()), operations.clone());
    let management = ModuleManagementService::new(Arc::new(ModulePermissionRepository::new(fx.db.clone())), operations, crashes);
    app.manage(management);
}

fn write_json(path: impl AsRef<Path>, data: &Value) {
    fs::write(path, serde_json::to_vec_pretty(data).expect("layout JSON")).expect("layout evidence");
}

fn require(fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, ok: bool, detail: impl Display) -> Journey<()> {
    record(ck, ev, name, ok, detail);
    write_json(fx.root.join("layout-round2-checkpoint.json"), &json!({"step":name,"passed":ck.passed,"failed":ck.failed,"checks":ev}));
    if ok { Ok(()) } else { Err(name.into()) }
}

// New-phase helpers filter hidden duplicate workspace surfaces without changing old scenarios.
async fn dom(app: &AppHandle, args: Value, body: &str) -> Value {
    eval_long(app, &format!("const stage6UI=true;\n{DOM}\nconst A={args};\n{body}")).await
}

async fn step(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, args: Value, body: &str) -> Journey<Value> {
    let result = dom(app, args, body).await;
    require(fx, ck, ev, name, ui_ok(&result), result.to_string())?;
    Ok(result["value"].clone())
}

pub(super) async fn capture(app: &AppHandle, fx: &Fx, name: &str) -> Journey<String> {
    super::super::capture_native(app, &fx.root, &format!("layout-round2-{name}")).await
}

async fn navigation(app: &AppHandle) -> Journey<()> {
    let result = dom(app, json!({}), r#"
const toggle=await need('dev-toggle-navigation');
const details=tid('dev-context-details');if(details?.tagName==='DETAILS')details.open=false;
if(toggle.getAttribute('aria-expanded')==='false')toggle.click();
return {ok:!!await until(()=>tid('dev-project-create-open'))};
"#).await;
    if ui_ok(&result) { Ok(()) } else { Err(result.to_string()) }
}

async fn selection(app: &AppHandle) -> Value {
    value(&ipc(app,"main","read_development_workspace",json!({})).await)["selection"].clone()
}

async fn import(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, path: &Path) -> Journey<()> {
    navigation(app).await?;
    let ok=ui_import_project(app,ck,ev,name,path).await;
    require(fx,ck,ev,name,ok,"production project form")
}

async fn new_task(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, goal: &str) -> Journey<()> {
    navigation(app).await?;
    let ok=ui_new_task(app,ck,ev,goal).await;
    require(fx,ck,ev,goal,ok,"production task form")?;
    let row=step(app,fx,ck,ev,"explicitly select newly saved task through production navigation",json!({"goal":goal}),r#"
const row=await until(()=>$$('[data-testid^="dev-task-item-"]').find(e=>e.querySelector('[title]')?.getAttribute('title')===A.goal&&!e.disabled));
if(!row)throw Error('saved task row absent');
if(row.getAttribute('aria-pressed')!=='true')row.click();
const id=row.getAttribute('data-testid').slice('dev-task-item-'.length);
return {ok:!!await until(()=>tid('dev-task-item-'+id)?.getAttribute('aria-pressed')==='true'),id};
"#).await?;
    let id=row["id"].as_str().ok_or("new task selected ID absent")?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(20);
    loop {
        let selected=selection(app).await;
        if selected["task_id"]==id { return require(fx,ck,ev,"host confirms newly created task selection",true,selected.to_string()); }
        if tokio::time::Instant::now()>=deadline { return require(fx,ck,ev,"host confirms newly created task selection",false,selected.to_string()); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn select_task(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, id: &str) -> Journey<()> {
    navigation(app).await?;
    step(app,fx,ck,ev,"select exact original task via production navigation",json!({"id":id}),r#"await click('dev-task-item-'+A.id);return {ok:true};"#).await?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(20);
    loop {
        let selected=selection(app).await;
        if selected["task_id"]==id { return require(fx,ck,ev,"host confirms exact selected task",true,selected.to_string()); }
        if tokio::time::Instant::now()>=deadline { return require(fx,ck,ev,"host confirms exact selected task",false,selected.to_string()); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn process(app: &AppHandle, id: &str, needle: &str) -> Journey<Value> {
    let deadline = tokio::time::Instant::now()+Duration::from_secs(30);
    loop {
        let reply=ipc(app,"main","read_development_process",json!({"id":id})).await;
        let v=value(&reply);
        if succeeded(&reply) && v["output"].as_str().is_some_and(|s|s.contains(needle)) { return Ok(v); }
        if tokio::time::Instant::now()>=deadline { return Err(format!("process output timeout: {}",reply.describe())); }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

async fn runtime(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    step(app,fx,ck,ev,"open independent bottom runtime",json!({}),r#"
const t=await need('dev-toggle-runtime');if(t.getAttribute('aria-expanded')==='false')t.click();
return {ok:!!await until(()=>tid('dev-development-runtime')&&tid('dev-panel-code'))};
"#).await?;
    Ok(())
}

pub(super) async fn start_process(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, task: &str, kind: &str, args: &[&str]) -> Journey<String> {
    runtime(app,fx,ck,ev).await?;
    let before=value(&ipc(app,"main","list_development_processes",json!({"taskId":task})).await);
    step(app,fx,ck,ev,&format!("start real host {kind} via production form"),json!({"kind":kind,"args":args}),r#"
const s=await need('dev-development-runtime');const pause=()=>new Promise(r=>setTimeout(r,70));
const configuration=await need('dev-runtime-configuration');configuration.open=true;
const select=await need('dev-process-kind');select.value=A.kind;select.dispatchEvent(new Event('change',{bubbles:true}));await pause();
while($$('button[aria-label^="移除参数"]',s).length){$$('button[aria-label^="移除参数"]',s)[0].click();await pause();}
await fill('dev-process-program','/bin/sh');
for(let i=0;i<A.args.length;i++){const add=$$('button',s).find(e=>txt(e)==='添加参数');if(!add)throw Error('add argument absent');add.click();await pause();const field=await until(()=>$$('input',s).find(e=>labelOf(e)==='参数 '+(i+1)));if(!field)throw Error('argument absent');setInput(field,A.args[i]);await pause();}
const trust=await need('dev-process-trust');if(!trust.checked)trust.click();await pause();
const start=await until(()=>{const b=tid('dev-process-start');return b&&!b.disabled&&b;});if(!start)throw Error('start disabled');start.click();return {ok:true};
"#).await?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(30);
    loop {
        let rows=value(&ipc(app,"main","list_development_processes",json!({"taskId":task})).await);
        let new:Vec<&Value>=rows.as_array().into_iter().flatten().filter(|r|!before.as_array().into_iter().flatten().any(|old|old["id"]==r["id"])).collect();
        if !new.is_empty() {
            require(fx,ck,ev,"one form submission creates one task-bound host receipt",new.len()==1&&new[0]["task_id"]==task&&new[0]["kind"]==kind,rows.to_string())?;
            return new[0]["id"].as_str().map(str::to_owned).ok_or("process ID absent".into());
        }
        if tokio::time::Instant::now()>=deadline { return Err("new host process receipt timeout".into()); }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

async fn history(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, id: &str, needle: &str) -> Journey<()> {
    runtime(app,fx,ck,ev).await?;
    step(app,fx,ck,ev,"select actual task-bound process history",json!({"id":id,"needle":needle}),r#"
const configuration=await need('dev-runtime-configuration');configuration.open=false;
await click('dev-process-history-'+A.id);
return {ok:!!await until(()=>tid('dev-process-history-'+A.id)?.getAttribute('aria-pressed')==='true'&&txt(tid('dev-process-detail')).includes(A.needle)),text:txt(tid('dev-process-detail'))};
"#).await?;
    Ok(())
}

async fn terminal_foreground(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    let _ = super::super::ui_keys::prepare(app).await;
    let state=dom(app,json!({}),r#"
const ready=!!await until(()=>document.visibilityState==='visible'&&document.hasFocus());
return {ok:ready,visibilityState:document.visibilityState,hidden:document.hidden,hasFocus:document.hasFocus(),activeElement:document.activeElement?.className};
"#).await;
    require(fx,ck,ev,"native foreground enables live WKWebView terminal projection",ui_ok(&state),state.to_string())
}

async fn terminal_input(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, line: &str) -> Journey<()> {
    terminal_foreground(app,fx,ck,ev).await?;
    step(app,fx,ck,ev,"type into production xterm connected to real PTY",json!({"line":line}),r#"
const t=await need('dev-terminal');t.scrollIntoView({block:'center'});
const input=await until(()=>t.querySelector('.xterm-helper-textarea'));if(!input)throw Error('xterm input absent');input.focus();
for(const key of A.line){const c=key.charCodeAt(0);input.dispatchEvent(new KeyboardEvent('keydown',{key,keyCode:0,which:0,bubbles:true}));input.dispatchEvent(new KeyboardEvent('keypress',{key,charCode:c,keyCode:c,which:c,bubbles:true}));input.dispatchEvent(new KeyboardEvent('keyup',{key,bubbles:true}));}
input.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',code:'Enter',keyCode:13,which:13,bubbles:true}));input.dispatchEvent(new KeyboardEvent('keyup',{key:'Enter',code:'Enter',keyCode:13,which:13,bubbles:true}));return {ok:true};
"#).await?;
    Ok(())
}

async fn reload(app: &AppHandle) -> Journey<()> {
    if goto(app,"main","/tasks/agent").await { Ok(()) } else { Err("production App reload failed".into()) }
}

async fn appearance(app: &AppHandle, mode: &str, large: bool, density: u16) -> Journey<()> {
    let read=ipc(app,"main","get_appearance_preferences",json!({})).await;
    let mut snapshot=read.data()["snapshot"].clone();
    let revision=snapshot.as_object_mut().and_then(|s|s.remove("revision")).ok_or("appearance revision absent")?;
    snapshot["mode"]=json!(mode);
    snapshot["density"]=json!(density);
    snapshot["typography"]["uiSize"]=json!(if large {16} else {12});
    snapshot["typography"]["readingSize"]=json!(if large {20} else {13});
    snapshot["typography"]["codeSize"]=json!(if large {18} else {12});
    let saved=ipc(app,"main","set_appearance_preferences",json!({"snapshot":snapshot,"expectedRevision":revision})).await;
    if !saved.success() || saved.data()["kind"]!="saved" { return Err(saved.describe()); }
    reload(app).await
}

const MEASURE: &str = r#"
if(!await until(()=>!document.documentElement.hasAttribute('data-appearance-pending')&&tid('dev-panel-code')))throw Error('production layout not ready');
await new Promise(r=>setTimeout(r,500));
const rect=e=>{const r=e.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom}};
const panels=['dev-panel-agent','dev-panel-code','dev-panel-runtime'].map(id=>({id,el:tid(id)})).filter(p=>visible(p.el)).map(p=>({id:p.id,...rect(p.el)}));
const vw=document.documentElement.clientWidth,vh=document.documentElement.clientHeight;
const clipped=panels.filter(p=>p.x< -1||p.right>vw+1||p.y< -1||p.bottom>vh+1||p.width<1||p.height<1);
const overlaps=[];for(let i=0;i<panels.length;i++)for(let j=i+1;j<panels.length;j++){const a=panels[i],b=panels[j];if(Math.min(a.right,b.right)-Math.max(a.x,b.x)>1&&Math.min(a.bottom,b.bottom)-Math.max(a.y,b.y)>1)overlaps.push([a.id,b.id]);}
const root=document.querySelector('.agent-workbench'),g=parseFloat(getComputedStyle(document.documentElement).fontSize),u=parseFloat(getComputedStyle(root).getPropertyValue('--font-size-ui'));
const minimum=Math.max(16*g,16*u)+Math.max(32*g,32*u)+Math.max(20*g,22*u)+.5*g+2;
const compact=root.getAttribute('data-compact')==='true',saved=JSON.parse(localStorage.getItem(A.key)||'null');
const code=tid('dev-panel-code'),agent=tid('dev-panel-agent'),split=tid('dev-agent-resizer');
const available=visible(split)?root.querySelector('.dev-collaboration').clientWidth-split.getBoundingClientRect().width:0;
const expectedAgent=available?Math.max(Math.max(20*g,22*u),Math.min(available-Math.max(32*g,32*u),available*saved.agentWidth/100)):null;
const splitFits=expectedAgent===null||Math.abs(agent.getBoundingClientRect().width-expectedAgent)<=2;
return {ok:!!document.querySelector('[data-workspace-surface="cli-agent"]')&&document.documentElement.scrollWidth<=vw+1&&!clipped.length&&!overlaps.length&&compact===(root.clientWidth<minimum)&&splitFits,viewport:{width:vw,height:vh},theme:document.documentElement.dataset.theme,density:document.documentElement.dataset.density,uiSize:getComputedStyle(document.documentElement).getPropertyValue('--font-size-ui'),budget:{width:root.clientWidth,minimum,compact},split:{available,expectedAgent,actualAgent:agent.getBoundingClientRect().width,preference:saved.agentWidth,splitFits},lineHeights:{code:getComputedStyle(code).lineHeight,agent:getComputedStyle(agent).lineHeight},panels,clipped,overlaps,scrollWidth:document.documentElement.scrollWidth,layout:localStorage.getItem(A.key),alerts:$$('[role=alert]').map(txt)};
"#;

async fn matrix(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    let window=app.get_webview_window("main").ok_or("main window absent")?;
    let mut cells=vec![];
    for (width,height) in [(1440,900),(1280,800),(800,600)] {
        window.set_size(tauri::LogicalSize::new(width as f64,height as f64)).map_err(|e|e.to_string())?;
        for mode in ["light","dark"] {
            for large in [false,true] {
            for density in [60,100,150] {
                appearance(app,mode,large,density).await?;
                step(app,fx,ck,ev,"prepare responsive production panel visibility",json!({"width":width}),r#"
if(!await until(()=>{const e=document.querySelector('.agent-workbench');if(!e)return false;const g=parseFloat(getComputedStyle(document.documentElement).fontSize),u=parseFloat(getComputedStyle(e).getPropertyValue('--font-size-ui')),minimum=Math.max(16*g,16*u)+Math.max(32*g,32*u)+Math.max(20*g,22*u)+.5*g+2;return e.getAttribute('data-compact')===String(e.clientWidth<minimum)}))throw Error('responsive content budget unsettled');
const compact=document.querySelector('.agent-workbench')?.getAttribute('data-compact')==='true';
const nav=await need('dev-toggle-navigation');if(compact&&nav.getAttribute('aria-expanded')==='true')nav.click();
if(compact&&!visible(tid('dev-panel-code'))){const code=await until(()=>$$('button',document.querySelector('.dev-view-switch')).find(e=>e.getAttribute('aria-pressed')==='false'&&!e.hasAttribute('data-testid')));if(!code)throw Error('compact code view toggle absent');code.click();}
if(!compact){for(const name of ['agent','runtime']){const t=await need('dev-toggle-'+name);if(t.getAttribute('aria-expanded')==='false')t.click();}}
return {ok:!!await until(()=>visible(tid('dev-panel-code')))};
"#).await?;
                let name=format!("{width}x{height}-{mode}-{}-density{density}",if large {"large"} else {"default"});
                let measured=step(app,fx,ck,ev,&format!("full App layout {name}"),json!({"key":LAYOUT_KEY}),MEASURE).await?;
                let scale=window.scale_factor().map_err(|e|e.to_string())?;
                let inner=window.inner_size().map_err(|e|e.to_string())?;
                let outer=window.outer_size().map_err(|e|e.to_string())?;
                let logical_inner=inner.to_logical::<f64>(scale);
                let webview: &tauri::Webview<tauri::Wry> = window.as_ref();
                let bounds=webview.bounds().map_err(|e|e.to_string())?;
                let client=bounds.size.to_logical::<f64>(scale);
                let position=bounds.position.to_logical::<f64>(scale);
                let dom_width=measured["viewport"]["width"].as_f64().ok_or("DOM viewport width absent")?;
                let dom_height=measured["viewport"]["height"].as_f64().ok_or("DOM viewport height absent")?;
                let native=json!({"requested_logical":{"width":width,"height":height},"scale_factor":scale,"inner_physical":{"width":inner.width,"height":inner.height},"outer_physical":{"width":outer.width,"height":outer.height},"inner_logical":{"width":logical_inner.width,"height":logical_inner.height},"webview_bounds_logical":{"x":position.x,"y":position.y,"width":client.width,"height":client.height},"window_to_dom_delta_logical":{"width":logical_inner.width-dom_width,"height":logical_inner.height-dom_height},"webview_to_dom_delta_logical":{"width":client.width-dom_width,"height":client.height-dom_height},"viewport_source":"DOM clientWidth/clientHeight; native window and WebView bounds recorded independently"});
                require(fx,ck,ev,"native window exactly matches requested size and shipped appearance",logical_inner.width==width as f64&&logical_inner.height==height as f64&&measured["theme"]==mode&&measured["density"].as_str().is_some_and(|v|v==density.to_string())&&measured["uiSize"].as_str().is_some_and(|v|v.trim()==if large {"16px"} else {"12px"}),json!({"native":native,"measurement":measured}))?;
                let png=capture(app,fx,&name).await?;
                if width==800 {
                    step(app,fx,ck,ev,"compact agent and code views switch without losing runtime",json!({}),r#"
const runtimeBefore=tid('dev-toggle-runtime')?.getAttribute('aria-expanded');await click('dev-toggle-agent');
if(!await until(()=>visible(tid('dev-panel-agent'))&&!visible(tid('dev-panel-code'))))throw Error('compact agent view absent');
const code=$$('button',document.querySelector('.dev-view-switch')).find(e=>e.getAttribute('aria-pressed')==='false'&&!e.hasAttribute('data-testid'));if(!code)throw Error('code view toggle absent');code.click();
return {ok:!!await until(()=>visible(tid('dev-panel-code'))&&!visible(tid('dev-panel-agent')))&&tid('dev-toggle-runtime')?.getAttribute('aria-expanded')===runtimeBefore};
"#).await?;
                }
                cells.push(json!({"cell":name,"native":native,"measurement":measured,"png":png}));
                write_json(fx.root.join("layout-round2-matrix.json"),&json!(cells));
            }
            }
        }
    }
    appearance(app,"light",false,100).await?;
    window.set_size(tauri::LogicalSize::new(1440.0,900.0)).map_err(|e|e.to_string())?;
    Ok(())
}

async fn geometry(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    step(app,fx,ck,ev,"resize agent and runtime with accessible separators",json!({}),r#"
for(const name of ['agent','runtime']){const toggle=await need('dev-toggle-'+name);if(toggle.getAttribute('aria-expanded')==='false')toggle.click();}
const changes=[];
for(const name of ['agent','runtime']){const s=await need('dev-'+name+'-resizer');const before=s.getAttribute('aria-valuenow');s.focus();s.dispatchEvent(new KeyboardEvent('keydown',{key:name==='agent'?'ArrowRight':'ArrowUp',bubbles:true}));const changed=await until(()=>s.getAttribute('aria-valuenow')!==before);if(!changed)throw Error('separator did not resize '+name);changes.push({name,before,after:s.getAttribute('aria-valuenow')});}
return {ok:true,changes};
"#).await?;
    for panel in ["agent","runtime","navigation"] {
        step(app,fx,ck,ev,&format!("collapse {panel} without losing code panel"),json!({"name":panel}),r#"
const t=await need('dev-toggle-'+A.name);if(t.getAttribute('aria-expanded')!=='false')t.click();
return {ok:!!await until(()=>t.getAttribute('aria-expanded')==='false'&&visible(tid('dev-panel-code')))};
"#).await?;
    }
    let before=step(app,fx,ck,ev,"persist only presentation geometry",json!({"key":LAYOUT_KEY}),r#"const saved=localStorage.getItem(A.key),v=JSON.parse(saved||'null');return {ok:!!v&&Object.keys(v).every(k=>['navigation','agent','runtime','agentWidth','runtimeHeight','codeView','compactView','runtimeView'].includes(k))&&Number.isInteger(v.agentWidth)&&Number.isInteger(v.runtimeHeight),saved};"#).await?;
    reload(app).await?;
    step(app,fx,ck,ev,"restore collapsed resized layout after full App reload",json!({"key":LAYOUT_KEY,"saved":before["saved"]}),r#"
await need('dev-panel-code');return {ok:localStorage.getItem(A.key)===A.saved&&['agent','runtime','navigation'].every(n=>tid('dev-toggle-'+n)?.getAttribute('aria-expanded')==='false'),saved:localStorage.getItem(A.key)};
"#).await?;
    capture(app,fx,"collapsed-restored").await?;
    navigation(app).await?;
    runtime(app,fx,ck,ev).await?;
    step(app,fx,ck,ev,"restore resized separator values when panels reopen",json!({"saved":before["saved"]}),r#"
const v=JSON.parse(A.saved);const agent=await need('dev-toggle-agent');if(agent.getAttribute('aria-expanded')==='false')agent.click();
return {ok:!!await until(()=>Number(tid('dev-agent-resizer')?.getAttribute('aria-valuenow'))===v.agentWidth&&Number(tid('dev-runtime-resizer')?.getAttribute('aria-valuenow'))===v.runtimeHeight)};
"#).await?;
    Ok(())
}

async fn journey(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    require(fx,ck,ev,"Git projects and all writes stay under isolated temporary root",fx.root.starts_with("/private/tmp")&&fx.root!=Path::new("/private/tmp"),fx.root.display())?;
    app.get_webview_window("main").ok_or("main window absent")?.set_size(tauri::LogicalSize::new(1440.0,900.0)).map_err(|e|e.to_string())?;
    step(app,fx,ck,ev,"initialize isolated phase presentation defaults",json!({"key":LAYOUT_KEY}),r#"
const previous=localStorage.getItem(A.key);localStorage.removeItem(A.key);return {ok:localStorage.getItem(A.key)===null,previous};
"#).await?;
    appearance(app,"light",false,100).await?;
    let a=fx.root.join("layout-project-a");let b=fx.root.join("layout-project-b");
    require(fx,ck,ev,"layout phase requires fresh project paths",!a.exists()&&!b.exists(),fx.root.display())?;
    make_repo(&a);make_repo(&b);
    fs::write(a.join("layout.txt"),"ROUND2_BASELINE\n").map_err(|e|e.to_string())?;
    require(fx,ck,ev,"commit real Git fixture baseline",git(&a,&["add","layout.txt"])&&git(&a,&["-c","user.name=dgos","-c","user.email=dgos@example.invalid","commit","-qm","layout baseline"]),a.display())?;
    let catalog=ipc(app,"main","read_workspace_app_catalog",json!({})).await;
    require(fx,ck,ev,"full App catalog reads actual production service",catalog.success(),catalog.describe())?;
    navigation(app).await?;
    import(app,fx,ck,ev,"Layout project A",&a).await?;
    new_task(app,fx,ck,ev,"Round2 task A").await?;
    let selected=selection(app).await;
    let task=selected["task_id"].as_str().ok_or("task A absent")?.to_owned();
    let project=selected["project_id"].as_str().ok_or("project A absent")?.to_owned();
    step(app,fx,ck,ev,"inspect full resolved context details",json!({"path":a}),r#"
const d=await need('dev-context-details');if(d.tagName==='DETAILS')d.open=true;else d.click();
const ready=!!await until(()=>txt(tid('dev-context-details')).includes(A.path));const text=txt(tid('dev-context-details'));if(d.tagName==='DETAILS')d.open=false;
return {ok:ready,text};
"#).await?;
    step(app,fx,ck,ev,"inspect real file and type unsaved editor draft",json!({}),r#"
(await need('dev-review-tab-files')).click();await fill('dev-file-search','layout.txt');await click('dev-file-item-layout.txt');
if(!await until(()=>tid('dev-file-editor')?.value==='ROUND2_BASELINE\n'))throw Error('file baseline absent');
await fill('dev-file-editor','ROUND2_EDITED_DRAFT\n');return {ok:tid('dev-file-editor').value==='ROUND2_EDITED_DRAFT\n'};
"#).await?;
    step(app,fx,ck,ev,"same-task panel collapse and restore retains file draft",json!({}),r#"
for(const name of ['agent','runtime']){const b=await need('dev-toggle-'+name);b.click();await new Promise(r=>setTimeout(r,80));b.click();}
(await need('dev-review-tab-changes')).click();await need('dev-review-tab-files');(await need('dev-review-tab-files')).click();
if(!await until(()=>tid('dev-file-editor')?.value==='ROUND2_EDITED_DRAFT\n'))throw Error('same-task review tab switch lost draft');
return {ok:tid('dev-file-editor')?.value==='ROUND2_EDITED_DRAFT\n'};
"#).await?;
    step(app,fx,ck,ev,"save reviewed draft through real editor",json!({}),r#"
const save=await until(()=>{const b=tid('dev-file-save');return b&&!b.disabled&&b});if(!save)throw Error('save disabled');save.click();
return {ok:!!await until(()=>$$('[role=status]',tid('dev-file-source')).some(e=>txt(e)==='没有未保存修改'))};
"#).await?;
    require(fx,ck,ev,"saved editor draft reaches actual project bytes",fs::read_to_string(a.join("layout.txt")).unwrap_or_default()=="ROUND2_EDITED_DRAFT\n",a.display())?;
    let command=start_process(app,fx,ck,ev,&task,"command",&["-c","printf 'ROUND2_COMMAND_OUTPUT\\n'; pwd"]).await?;
    let receipt=process(app,&command,"ROUND2_COMMAND_OUTPUT").await?;
    require(fx,ck,ev,"real command output belongs to selected task and cwd",receipt["task_id"]==task&&receipt["output"].as_str().is_some_and(|s|s.contains(a.to_string_lossy().as_ref())),receipt.to_string())?;
    history(app,fx,ck,ev,&command,"ROUND2_COMMAND_OUTPUT").await?;
    let terminal=start_process(app,fx,ck,ev,&task,"terminal",&[]).await?;
    history(app,fx,ck,ev,&terminal,"/bin/sh").await?;
    terminal_input(app,fx,ck,ev,"stty -echo; PS1=''; printf '%s%s\\n' 'ROUND2_' 'PTY_CONTEXT'; pwd; printf '%s\\n' $$").await?;
    let baseline=process(app,&terminal,"ROUND2_PTY_CONTEXT").await?;
    write_json(fx.root.join("layout-round2-host-baseline.json"),&json!({"selection":selected,"command":receipt,"terminal":baseline}));
    new_task(app,fx,ck,ev,"Round2 task A sibling").await?;
    step(app,fx,ck,ev,"task switch clears previous task terminal and receipts",json!({"id":terminal,"command":command}),r#"
return {ok:!!await until(()=>!tid('dev-process-history-'+A.id)&&!tid('dev-process-history-'+A.command)&&!tid('dev-terminal'))};
"#).await?;
    select_task(app,fx,ck,ev,&task).await?;
    history(app,fx,ck,ev,&terminal,"/bin/sh").await?;
    import(app,fx,ck,ev,"Layout project B",&b).await?;
    new_task(app,fx,ck,ev,"Round2 task B").await?;
    step(app,fx,ck,ev,"project switch clears previous project process context",json!({"id":terminal,"command":command}),r#"
return {ok:!!await until(()=>!tid('dev-process-history-'+A.id)&&!tid('dev-process-history-'+A.command)&&!tid('dev-terminal'))};
"#).await?;
    step(app,fx,ck,ev,"return to original project using navigation",json!({"id":project}),r#"await click('dev-project-item-'+A.id);return {ok:true};"#).await?;
    select_task(app,fx,ck,ev,&task).await?;
    history(app,fx,ck,ev,&command,"ROUND2_COMMAND_OUTPUT").await?;
    history(app,fx,ck,ev,&terminal,"/bin/sh").await?;
    terminal_input(app,fx,ck,ev,"printf 'ROUND2_RETURNED_SAME_PTY\\n'; pwd").await?;
    let returned=process(app,&terminal,"ROUND2_RETURNED_SAME_PTY").await?;
    require(fx,ck,ev,"return preserves original terminal identity receipt and output",returned["id"]==baseline["id"]&&returned["started_at"]==baseline["started_at"]&&returned["status"]=="running"&&returned["task_id"]==task&&returned["output"].as_str().is_some_and(|s|s.contains("ROUND2_PTY_CONTEXT")&&s.contains(a.to_string_lossy().as_ref())),returned.to_string())?;
    terminal_foreground(app,fx,ck,ev).await?;
    step(app,fx,ck,ev,"restored actual xterm viewport displays new shell output",json!({}),r#"
const terminal=await need('dev-terminal');terminal.scrollIntoView({block:'center'});
const ready=!!await until(()=>terminal.querySelector('.xterm-accessibility-tree')?.innerText.includes('ROUND2_RETURNED_SAME_PTY'));
return {ok:ready,viewport:terminal.querySelector('.xterm-accessibility-tree')?.innerText,historyEvidence:'original output and same process identity verified independently through real host baseline and returned snapshots'};
"#).await?;
    let actual_sessions=sessions_of(app).await;
    require(fx,ck,ev,"layout host journey consumes no model or Agent sessions",actual_sessions.is_empty(),json!(actual_sessions))?;
    geometry(app,fx,ck,ev).await?;
    matrix(app,fx,ck,ev).await?;
    history(app,fx,ck,ev,&terminal,"/bin/sh").await?;
    terminal_input(app,fx,ck,ev,"printf 'ROUND2_FINAL_PTY\\n'; exit 0").await?;
    let final_terminal=process(app,&terminal,"ROUND2_FINAL_PTY").await?;
    write_json(fx.root.join("layout-round2-host-returned.json"),&json!({"selection":selection(app).await,"terminal":final_terminal}));
    Ok(())
}

// A local, opt-in harness control protocol. It executes only in this test binary;
// renderer invoke commands and production handlers remain untouched.
async fn controls(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, before_run: bool) -> Journey<()> {
    let path=fx.root.join("layout-round2-control.json");
    write_json(fx.root.join("layout-round2-ready.json"),&json!({"state":if before_run {"waiting-to-run"} else if ck.failed.is_empty() {"assertions-complete"} else {"assertions-failed-awaiting-diagnostics"},"control":path,"result":fx.root.join("layout-round2-control-result.json")}));
    let mut last=String::new();
    let deadline=tokio::time::Instant::now()+Duration::from_secs(3000);
    loop {
        if let Ok(bytes)=fs::read(&path) {
            if let Ok(request)=serde_json::from_slice::<Value>(&bytes) {
                let id=request["id"].as_str().unwrap_or_default();
                if !id.is_empty() && id!=last {
                    last=id.into();
                    let action=request["action"].as_str().unwrap_or_default();
                    let result=match action {
                        "run" if before_run => json!({"ok":true,"action":"run"}),
                        "finish" if !before_run => json!({"ok":true,"action":"finish"}),
                        "capture" => match capture(app,fx,&format!("control-{}",uuid::Uuid::new_v4().simple())).await {Ok(path)=>json!({"ok":true,"png":path}),Err(e)=>json!({"ok":false,"error":e})},
                        "eval" => {
                            let result=dom(app,request["args"].clone(),request["script"].as_str().unwrap_or("throw Error('script absent')")).await;
                            record(ck,ev,&format!("interactive assertion {id}"),ui_ok(&result),result.to_string());
                            result
                        },
                        _ => json!({"ok":false,"error":"expected unique id and eval/capture, run before assertions, finish afterward"}),
                    };
                    write_json(fx.root.join("layout-round2-control-result.json"),&json!({"id":id,"result":result}));
                    if result["ok"]==true && ((before_run&&action=="run")||(!before_run&&action=="finish")) { return Ok(()); }
                }
            }
        }
        if tokio::time::Instant::now()>=deadline { return Err("interactive control deadline exceeded".into()); }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

pub(super) async fn run(app: &AppHandle, fx: &Fx, mut ck: Checks, mut ev: Vec<Value>) {
    let interactive=std::env::var_os("DGOS_LAYOUT_ROUND2_INTERACTIVE").is_some();
    let result:Journey<()>=async {
        if interactive { controls(app,fx,&mut ck,&mut ev,true).await?; }
        journey(app,fx,&mut ck,&mut ev).await?;
        Ok(())
    }.await;
    if let Err(error)=result {
        record(&mut ck,&mut ev,"round-two continuous production journey completes",false,error);
        let diagnostic=dom(app,json!({}),r#"
const geometry=el=>{if(!el)return null;const r=el.getBoundingClientRect(),s=getComputedStyle(el);return {x:r.x,y:r.y,width:r.width,height:r.height,clientWidth:el.clientWidth,clientHeight:el.clientHeight,scrollWidth:el.scrollWidth,scrollHeight:el.scrollHeight,scrollTop:el.scrollTop,scrollLeft:el.scrollLeft,display:s.display,visibility:s.visibility,overflow:s.overflow,hidden:el.hidden,inert:!!el.closest('[inert]')};};
const terminal=document.querySelector('[data-testid="dev-terminal"]');const active=document.activeElement;
return {ok:true,url:location.href,body:document.body.innerText,alerts:$$('[role=alert]').map(txt),layout:localStorage.getItem('dgos.development.layout.v2'),document:{hidden:document.hidden,visibilityState:document.visibilityState,hasFocus:document.hasFocus(),activeElement:active?{tag:active.tagName,id:active.id,className:active.className,testId:active.getAttribute('data-testid'),geometry:geometry(active)}:null},xterm:{terminal:geometry(terminal),text:terminal?.innerText,accessibilityText:terminal?.querySelector('.xterm-accessibility-tree')?.innerText,accessibilityRows:[...(terminal?.querySelectorAll('.xterm-accessibility-tree > *')||[])].map(e=>({text:e.textContent,geometry:geometry(e)})),renderedRows:[...(terminal?.querySelectorAll('.xterm-rows > *')||[])].map(e=>e.textContent),elements:[...(terminal?.querySelectorAll('.xterm,.xterm-screen,.xterm-viewport,.xterm-scroll-area,.xterm-accessibility-tree')||[])].map(e=>({className:e.className,geometry:geometry(e)})),canvases:[...(terminal?.querySelectorAll('canvas')||[])].map(e=>({width:e.width,height:e.height,geometry:geometry(e)}))}};
"#).await;
        write_json(fx.root.join("layout-round2-failure-dom.json"),&diagnostic);
        let _=capture(app,fx,"failure").await;
    }
    if interactive {
        if let Err(error)=controls(app,fx,&mut ck,&mut ev,false).await {
            record(&mut ck,&mut ev,"interactive diagnostics conclude",false,error);
        }
    }
    record(&mut ck,&mut ev,"owned host processes shut down through real AgentManager",app.state::<AgentManager>().shutdown().await,"production shutdown");
    write_json(fx.root.join("layout-round2-result.json"),&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev,"execution_source":"shipped full production App in real Tauri WKWebView; DOM handlers and actual host IPC, Git, command and PTY","screenshots":"native /usr/sbin/screencapture -x -o -l; exact target NSWindow.windowNumber via run_on_main_thread; per-image capture.json records target PID and window number","approval_evidence":{"source":"actual host state only; no model request or approval fixture injected","fixture":false},"matrix":"layout-round2-matrix.json"}));
    println!("LAYOUT ROUND2: {} passed, {} failed; evidence {}",ck.passed,ck.failed.len(),fx.root.display());
    app.exit(if ck.failed.is_empty(){0}else{1});
}
