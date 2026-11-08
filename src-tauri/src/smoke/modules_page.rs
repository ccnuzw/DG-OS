//! Focused shipped-module acceptance; DGOS_MODULE_SMOKE_MODE=page.
//! No fixture modules, test pins, model requests, injected grants or simulated IPC results.
use super::*;
use crate::session::{HostSessionPort, SessionResolver};
use dgos_application::{AccountPorts, AccountService, AiService, HostCapabilityService, ModuleCapabilityDispatchService, ModuleMcpConfirmations};
use dgos_db::sea_orm::DatabaseConnection;

type Journey<T> = Result<T, String>;
struct PageWorld { root: PathBuf, db: DatabaseConnection, routes: Arc<RoutePublisher> }

fn save(root: &Path, name: &str, value: &Value) -> Journey<()> {
    fs::write(root.join(format!("{name}.json")), serde_json::to_vec_pretty(value).map_err(|e|e.to_string())?).map_err(|e|e.to_string())
}
fn root() -> Journey<PathBuf> {
    let root = std::env::var_os("DGOS_MODULE_PAGE_ROOT").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join(format!("dgos-module-page-{}", uuid::Uuid::new_v4())));
    let parent = root.parent().ok_or("temporary evidence root needs parent")?.canonicalize().map_err(|e|e.to_string())?;
    if !parent.starts_with("/private/tmp") && !parent.starts_with("/tmp") { return Err("module page root must be under /tmp".into()); }
    if root.exists() { return Err(format!("fresh evidence root required; retained {}", root.display())); }
    fs::create_dir(&root).map_err(|e|e.to_string())?;
    root.canonicalize().map_err(|e|e.to_string())
}
async fn install(app: &mut tauri::App, root: PathBuf) -> Journey<PageWorld> {
    let url = format!("sqlite://{}?mode=rwc", root.join("dgos.db").display());
    let db = dgos_db::establish_connection(&url).await.map_err(|e|e.to_string())?;
    dgos_db::migrations::Migrator::up(&db,None).await.map_err(|e|e.to_string())?;
    let maintenance = dgos_db::MaintenanceGate::new();
    let resolver = SessionResolver::starting();
    let publisher = Arc::new(ModulePublisher::new(root.join("modules"), db.clone()));
    let bundle = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bundled-modules");
    publisher.provision_builtin(&bundle).await.map_err(|e|e.to_string())?;
    let transport = Arc::new(ChannelTransport::default());
    let (loader,lost) = PageModuleLoader::new(transport.clone(),PageLoaderConfig { window_label:MAIN.into(), hook_timeout:Duration::from_secs(5) });
    let router = Arc::new(ModuleCallRouter::new(loader.clone()));
    let routes = Arc::new(RoutePublisher::new(router.clone(),transport.clone()));
    let supervisor = ModuleRuntimeSupervisor::new(publisher.clone(),Arc::new(loader.clone()))
        .with_admission(Arc::new(PinnedModuleAdmission::builtin())).with_ready_observer(routes.clone());
    let manager = ModuleOperationManager::with_supervisor(publisher.clone(),supervisor,LocalModuleSources::new(root.join("module-sources")))
        .with_builtin_sources(BuiltinModuleSources::new(&bundle).map_err(|e|e.to_string())?).map_err(|e|format!("{e:?}"))?.with_maintenance(maintenance.clone());
    manager.spawn_lost_runtime_reporter(lost).map_err(|e|format!("{e:?}"))?;
    manager.recover().await.map_err(|e|format!("{e:?}"))?;
    let permissions = Arc::new(ModulePermissionRepository::new(db.clone()));
    let capabilities = HostCapabilityService::with_publisher(permissions.clone(),publisher);
    let crashes = ModuleCrashService::new(ModuleCrashRepository::new(db.clone()),manager.clone());
    let management = ModuleManagementService::new(permissions,manager.clone(),crashes.clone());
    let providers = dgos_providers::ProviderManager::with_vault(&url,Arc::new(dgos_providers::test_support::MemoryCredentialVault::default())).await.map_err(|e|e.to_string())?.with_maintenance(maintenance.clone());
    let skills = Arc::new(tokio::sync::RwLock::new(dgos_skills::SkillsManager::new(db.clone(),root.join("skills"))));
    let mcp = Arc::new(dgos_mcp::McpManager::with_policy_and_vault(db.clone(),Arc::new(commands::DesktopMcpPolicy(resolver.clone())),Arc::new(commands::OsCredentialVault)).with_maintenance(maintenance.clone()));
    let confirmations = Arc::new(ModuleMcpConfirmations::new(transport.clone()));
    let ai = AiService::new(capabilities.clone(),dgos_db::AiRunRepository::new(db.clone()),Arc::new(providers.clone()),skills,mcp.clone(),confirmations.clone(),maintenance.clone());
    ai.recover_interrupted().await.map_err(|e|e.to_string())?;
    app.manage(resolver.clone()); app.manage(maintenance); app.manage(providers); app.manage(ai);
    app.manage(ModuleCapabilityDispatchService::new(capabilities.clone())); app.manage(capabilities);
    app.manage(ModuleHostBridge::new(loader,transport).with_mcp_confirmations(confirmations.clone()));
    app.manage(confirmations); app.manage(router); app.manage(manager); app.manage(crashes); app.manage(management); app.manage(StartupLoad);
    app.manage(dgos_db::AppearanceRepository::new(db.clone()));
    app.manage(dgos_db::HostSettingsRepository::new(db.clone()));
    app.manage(dgos_db::DesktopRepository::new(db.clone()));
    app.manage(dgos_db::CliRepository::new(db.clone()));
    let accounts = AccountService::new(db.clone(),AccountPorts::new(&db,Arc::new(HostSessionPort::new(resolver,mcp,app.handle().clone()))));
    accounts.recover().await.map_err(|e|e.to_string())?; app.manage(accounts);
    Ok(PageWorld { root, db, routes })
}

const DOM: &str = r#"
const until=async(f,ms=6000)=>{const end=Date.now()+ms;while(Date.now()<end){const v=f();if(v)return v;await new Promise(r=>setTimeout(r,50));}throw Error('UI condition timed out: '+f.toString())};
const visible=e=>!!e&&e.isConnected&&!e.closest('[hidden],[inert],[aria-hidden=true]')&&getComputedStyle(e).display!=='none'&&e.getClientRects().length>0;
const all=(s,r=document)=>[...r.querySelectorAll(s)];const txt=e=>e?.textContent?.trim()||'';
const button=(re,r=document)=>all('button',r).find(e=>visible(e)&&!e.disabled&&re.test(txt(e)));
const set=(e,v)=>{if(!e)throw Error('input absent');Object.getOwnPropertyDescriptor(e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(e,v);e.dispatchEvent(new Event('input',{bubbles:true}));};
const click=async(re,r=document)=>{const b=await until(()=>button(re,r));b.scrollIntoView({block:'center'});b.click();return b;};
const category=async(name)=>{if(!document.querySelector('[data-settings-shell]')){const a=document.querySelector('a[href="/settings"]');if(!a)throw Error('Settings link absent');a.click();await until(()=>location.pathname==='/settings/appearance');}const c=await until(()=>document.querySelector('button.workspace-settings-category[data-settings-category="'+name+'"]'));c.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,button:0,ctrlKey:false}));c.dispatchEvent(new MouseEvent('mouseup',{bubbles:true,button:0}));await until(()=>c.getAttribute('aria-selected')==='true'&&location.pathname==='/settings/'+name);};
const closeSettings=async()=>{const b=document.querySelector('.workspace-settings-header button');if(!b)throw Error('close Settings absent');b.click();await until(()=>!document.querySelector('[data-settings-shell]'));};
const article=()=>document.querySelector('[data-testid="module-provider-console"]');
const consoleRoot=()=>all('[data-module-view] [data-testid="provider-console"]').find(visible);
const openConsole=async()=>{const launcher=await until(()=>all('button.workspace-nav-item[data-tone="app"]').find(visible),6000);launcher.click();const item=await until(()=>all('.workspace-launcher-item').find(e=>visible(e)&&txt(e.querySelector('.workspace-launcher-name'))==='Provider Console'&&e.getAttribute('aria-disabled')!=='true'),6000);item.click();};
const frame=()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
"#;
async fn dom(app: &AppHandle, args: Value, body: &str) -> Value {
    eval(app,MAIN,&format!("{DOM}\nconst A={args};\n{body}")).await
}
async fn step(app: &AppHandle, world: &PageWorld, checks: &mut Checks, events: &mut Vec<Value>, name: &str, args: Value, body: &str) -> Journey<Value> {
    let reply=dom(app,args,body).await; let ok=reply["ok"]==true&&reply["value"]["ok"]==true;
    checks.check(name,ok,reply.to_string()); events.push(json!({"check":name,"ok":ok,"result":reply}));
    save(&world.root,"module-page-checkpoint",&json!({"passed":checks.passed,"failed":checks.failed,"checks":events}))?;
    if ok {Ok(reply["value"].clone())} else {Err(format!("{name}: {reply}"))}
}
async fn shot(app: &AppHandle, world: &PageWorld, name: &str) -> Journey<()> {
    save(&world.root,&format!("{name}-dom"),&dom(app,json!({}),"return {ok:true,url:location.href,body:document.body.innerText,theme:document.documentElement.dataset.theme,density:document.documentElement.dataset.density};").await)?;
    super::super::capture_native(app,&world.root,name).await.map(|_|())
}
async fn protected(app: &AppHandle, route: &(String,String,String), call: &str) -> super::super::Reply {
    ipc(app,MAIN,"module_list_provider_catalog",json!({"attachmentId":route.1,"route":route.2,"callId":call})).await
}
fn require(checks: &mut Checks, events: &mut Vec<Value>, name: &str, ok: bool, evidence: Value) -> Journey<()> {
    checks.check(name,ok,evidence.to_string()); events.push(json!({"check":name,"ok":ok,"evidence":evidence}));
    if ok {Ok(())} else {Err(name.into())}
}

async fn journey(app: &AppHandle, world: &PageWorld, checks: &mut Checks, events: &mut Vec<Value>) -> Journey<()> {
    if !super::super::ui_keys::prepare(app).await { return Err("native module window is not foreground/focused".into()); }
    let assets=dom(app,json!({}),r#"const urls=[...new Set(all('script[src],link[rel=stylesheet]').map(e=>e.src||e.href).concat(performance.getEntriesByType('resource').map(e=>e.name).filter(u=>u.includes('/assets/'))))];const resources=await Promise.all(urls.map(async url=>{const response=await fetch(url,{cache:'no-store'});if(!response.ok)throw Error('asset read '+response.status);const bytes=await response.arrayBuffer();const sha256=[...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(x=>x.toString(16).padStart(2,'0')).join('');return {url,bytes:bytes.byteLength,sha256}}));return {ok:resources.some(r=>/\.js$/.test(new URL(r.url).pathname))&&resources.some(r=>/\.css$/.test(new URL(r.url).pathname)),location:location.href,resources};"#).await;
    require(checks,events,"embedded production assets carry real resource hashes",assets["ok"]==true&&assets["value"]["ok"]==true,assets.clone())?;
    save(&world.root,"module-page-assets",&json!({"host_pid":std::process::id(),"executable":std::env::current_exe().map_err(|e|e.to_string())?,"host_version":env!("CARGO_PKG_VERSION"),"bundle":{"id":CONSOLE,"version":"1.1.0","manifest":serde_json::from_str::<Value>(include_str!("../../../bundled-modules/provider-console/1.1.0/manifest.json")).map_err(|e|e.to_string())?},"page":assets}))?;
    let account_reload=app.state::<LoadEvents>().0.subscribe();
    step(app,world,checks,events,"create isolated account through production welcome form",json!({}),r#"
const username=await until(()=>document.querySelector('input[autocomplete=username]'));set(username,'module-acceptance');const passwords=all('input[type=password]');if(passwords.length!==2)throw Error('create account confirmation absent');set(passwords[0],'isolated-module-pass-2026');set(passwords[1],'isolated-module-pass-2026');await frame();const submit=button(/^(创建账户|Create account)$/);if(!submit)throw Error('create account submit absent');setTimeout(()=>submit.click(),100);return {ok:true};
"#).await?;
    if !wait_load(account_reload,MAIN,"/workbench").await { return Err("account reload did not finish".into()); }
    step(app,world,checks,events,"await reloaded signed-in production document",json!({}),"await until(()=>document.querySelector('a[href=\"/settings\"]'));return {ok:true};").await?;
    let state=ipc(app,MAIN,"get_account_state",json!({})).await;
    require(checks,events,"isolated production account is signed in",state.success()&&state.data()["mode"]=="signed_in",json!(state.describe()))?;
    // Passive transport observer: original fetch and responses always flow unchanged.
    step(app,world,checks,events,"observe real IPC without replacing results",json!({}),r#"
const original=window.fetch;window.__modulePage={original,events:[]};window.fetch=function(url,options,...rest){const command=typeof url==='string'&&url.startsWith('ipc://localhost/')?url.slice(16).split('?')[0]:null;const watched=command&&(command.startsWith('module_')||command.endsWith('_module')||command.includes('module_permission')||command.includes('appearance_preferences'));const event=watched?{command,args:JSON.parse(options?.body||'{}')}:null;if(event)window.__modulePage.events.push(event);return original.call(this,url,options,...rest).then(response=>{if(event)response.clone().json().then(value=>{event.reply=value;event.done=true}).catch(error=>{event.error=String(error)});return response;});};return {ok:true};
"#).await?;
    step(app,world,checks,events,"review shipped module permissions in Settings",json!({}),r#"
await category('modules');const a=await until(article);await click(/^(启用|Enable)$/,a);const d=await until(()=>a.querySelector('[role=dialog]'));const c=d.querySelector('[role=checkbox],input[type=checkbox]');if(!c)throw Error('permission acknowledgement absent');c.click();return {ok:true,permissions:txt(d)};
"#).await?;
    step(app,world,checks,events,"confirm authorization and await pinned module runtime",json!({}),r#"
const a=article(),d=a.querySelector('[role=dialog]');await click(/确认启用|Confirm enable/i,d);await until(()=>txt(article()).includes('ready'));return {ok:txt(article()).includes('1.1.0'),permissions:txt(article())};
"#).await?;
    step(app,world,checks,events,"open unpinned module through production Apps Launcher",json!({}),r#"
await closeSettings();await openConsole();return {ok:true,path:location.pathname};
"#).await?;
    step(app,world,checks,events,"await actionable console and retain document identity",json!({}),r#"
const root=await until(consoleRoot);await until(()=>root.querySelector('[role=combobox]')?.disabled===false);await until(()=>button(/刷新选择目录|Refresh selections/,root));set(root.querySelector('textarea'),'MODULE_ACCEPTANCE_DRAFT 中文 Aa 0123');await frame();window.__modulePage.root=root;window.__modulePage.view=root.closest('[data-module-view]');window.__modulePage.refresh=button(/刷新选择目录|Refresh selections/,root);return {ok:root.querySelector('textarea').value.includes('MODULE_ACCEPTANCE_DRAFT'),path:location.pathname,view:window.__modulePage.view.dataset.moduleView,runtime:root.closest('[data-runtime-id]')?.dataset.runtimeId};
"#).await?;
    let route=world.routes.issued().last().cloned().ok_or("production runtime route absent")?;
    let initial=protected(app,&route,&uuid::Uuid::new_v4().to_string()).await;
    require(checks,events,"protected production metadata read succeeds with real grants",initial.success()&&initial.data().is_array(),json!({"runtime":route.0,"reply":initial.describe(),"meaning":"real empty provider catalogue; no model request"}))?;
    shot(app,world,"module-page-ready").await?;
    for (mode,size,density,local) in [("light",12,60,false),("dark",16,100,true),("light",20,150,false),("dark",14,60,true),("light",16,100,false),("dark",12,150,true)] {
        let tag=format!("module-page-{mode}-{size}-{density}");
        step(app,world,checks,events,&format!("open production appearance editor {tag}"),json!({}),"await category('appearance');await until(()=>document.querySelector('.appearance-editor'));return {ok:true};").await?;
        let font=step(app,world,checks,events,&format!("select theme size and real font {tag}"),json!({"mode":mode,"size":size,"local":local}),r#"
const page=document.querySelector('.appearance-editor');const theme=all('[role=radiogroup]',page).find(e=>/^(主题|Theme)$/.test(e.getAttribute('aria-label')||txt(document.getElementById(e.getAttribute('aria-labelledby')))));const radio=theme&&all('[role=radio]',theme).find(e=>new RegExp(A.mode==='light'?'亮色|浅色|Light':'暗色|深色|Dark','i').test(txt(e)));if(!radio)throw Error('theme control absent');radio.click();await until(()=>document.documentElement.dataset.theme===A.mode);await frame();set(page.querySelector('[data-testid="appearance-ui-size-input"]'),String(A.size));await frame();page.querySelector('[data-testid="appearance-ui-font"]').click();const menu=await until(()=>document.querySelector('.appearance-font-menu'),6000);let font=null;if(A.local){set(menu.querySelector('input'),'Helvetica');font=await until(()=>all('[data-font-family]',menu).find(e=>e.dataset.fontFamily.toLowerCase().includes('helvetica')),6000);}else font=menu.querySelector('[role=option]');if(!font)throw Error('font option absent');const family=font.dataset.fontFamily||'system-ui';font.click();await frame();return {ok:true,family};
"#).await?;
        let appearance=step(app,world,checks,events,&format!("save appearance through production UI {tag}"),json!({"density":density,"family":font["family"]}),r#"
const page=document.querySelector('.appearance-editor');const scale=page.querySelector('.appearance-layout-scale [role=slider]');for(let i=0;i<30&&Number(scale.getAttribute('aria-valuenow'))!==A.density;i++){const n=Number(scale.getAttribute('aria-valuenow'));const b=all('.appearance-scale-inputs button',page).find(e=>txt(e)===(n<A.density?'+':'−'));if(!b)throw Error('layout scale step button absent');b.click();await new Promise(r=>setTimeout(r,30));}if(Number(scale.getAttribute('aria-valuenow'))!==A.density)throw Error('layout scale mismatch');const before=window.__modulePage.events.length;await click(/^(保存更改|Save changes)$/i,page);await until(()=>window.__modulePage.events.slice(before).some(e=>e.command==='set_appearance_preferences'&&e.reply?.success&&e.reply?.data?.kind==='saved'));let fontTimer;try{await Promise.race([document.fonts.ready,new Promise((_,reject)=>{fontTimer=setTimeout(()=>reject(Error('native font readiness timed out')),6000)})]);}finally{clearTimeout(fontTimer)}const persisted=await window.__TAURI_INTERNALS__.invoke('get_appearance_preferences',{});if(!persisted.success)throw Error('host appearance read failed');return {ok:true,family:A.family,persisted:persisted.data};
"#).await?;
        step(app,world,checks,events,&format!("return to retained module after appearance save {tag}"),json!({}),"await closeSettings();await until(()=>consoleRoot()===window.__modulePage.root);await frame();return {ok:true};").await?;
        let persisted=&appearance["persisted"]["snapshot"];
        require(checks,events,&format!("host stores exact schema6 appearance {tag}"),persisted["schemaVersion"]==6&&persisted["mode"]==mode&&persisted["density"]==density&&persisted["typography"]["uiSize"]==size,json!({"snapshot":persisted,"selected":appearance}))?;
        step(app,world,checks,events,&format!("host.ui inherits tokens and retains actionable document {tag}"),json!({"mode":mode,"size":size,"density":density,"family":appearance["family"],"local":local}),r#"
const p=window.__modulePage,root=consoleRoot(),view=root?.closest('[data-module-view]'),input=root?.querySelector('textarea'),refresh=button(/刷新选择目录|Refresh selections/,root),css=getComputedStyle(document.documentElement);if(!root||!refresh)throw Error('module operation absent');refresh.scrollIntoView({block:'center'});await frame();const r=refresh.getBoundingClientRect(),hit=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2),style=getComputedStyle(refresh),reading=getComputedStyle(input),before=p.events.length,normalize=s=>s.replace(/["'\s]/g,'');refresh.click();await until(()=>p.events.slice(before).some(e=>e.command==='module_list_ai_providers'&&e.reply?.success));await until(()=>!refresh.disabled);return {ok:root===p.root&&view===p.view&&input.value==='MODULE_ACCEPTANCE_DRAFT 中文 Aa 0123'&&document.documentElement.dataset.theme===A.mode&&document.documentElement.dataset.density===String(A.density)&&css.getPropertyValue('--font-size-ui').trim()===A.size+'px'&&normalize(style.fontFamily)===normalize(css.getPropertyValue('--font-family-ui'))&&normalize(reading.fontFamily)===normalize(css.getPropertyValue('--font-family-reading'))&&(!A.local||style.fontFamily.includes(A.family))&&!!hit&&(hit===refresh||refresh.contains(hit)),theme:document.documentElement.dataset.theme,density:document.documentElement.dataset.density,uiSize:css.getPropertyValue('--font-size-ui'),font:style.fontFamily,readingFont:reading.fontFamily,button:{fontSize:style.fontSize,background:style.backgroundColor,color:style.color,rect:r.toJSON()},view:view.dataset.moduleView,scrollWidth:root.scrollWidth,clientWidth:root.clientWidth};
"#).await?;
        shot(app,world,&tag).await?;
    }
    step(app,world,checks,events,"navigation hides live module without disposal",json!({}),r#"
const p=window.__modulePage,path=location.pathname;const nav=all('a[href="/workbench"],a[href="/tasks/agent"]').find(visible);if(!nav)throw Error('workspace destination absent');nav.click();await until(()=>location.pathname!==path&&!visible(p.root));return {ok:p.root.isConnected&&!visible(p.root),path:location.pathname};
"#).await?;
    step(app,world,checks,events,"Apps Launcher returns to identical module document",json!({}),r#"
await openConsole();const p=window.__modulePage;await until(()=>consoleRoot()===p.root);return {ok:p.root.querySelector('textarea').value==='MODULE_ACCEPTANCE_DRAFT 中文 Aa 0123',path:location.pathname,view:p.view.dataset.moduleView};
"#).await?;
    step(app,world,checks,events,"revoke providers.call through formal Settings control",json!({}),r#"
await category('modules');const row=await until(()=>all('li',article()).find(e=>e.querySelector('code')?.textContent==='providers.call'));await click(/^(撤销|撤权|Revoke)$/i,row);await until(()=>button(/^(授予|授权|Grant)$/i,all('li',article()).find(e=>e.querySelector('code')?.textContent==='providers.call')));return {ok:true,permissions:txt(article())};
"#).await?;
    let old_call=uuid::Uuid::new_v4().to_string();
    let denied=protected(app,&route,&old_call).await;
    require(checks,events,"revoked runtime protected call rejects with 1004004",denied.code()==Some(1004004),json!(denied.describe()))?;
    step(app,world,checks,events,"revocation disposes document and shows unavailable surface",json!({}),r#"
await closeSettings();const p=window.__modulePage;await until(()=>!p.root.isConnected&&button(/打开应用管理|Open app management/i));const before=p.events.length;p.refresh.click();await new Promise(r=>setTimeout(r,150));if(p.events.length!==before)throw Error('revoked detached handler sent IPC');await category('modules');return {ok:true,oldDocumentReleased:true};
"#).await?;
    step(app,world,checks,events,"reauthorize only providers.call through production UI",json!({}),r#"
const row=all('li',article()).find(e=>e.querySelector('code')?.textContent==='providers.call');await click(/^(授予|授权|Grant)$/i,row);await until(()=>button(/^(撤销|撤权|Revoke)$/i,all('li',article()).find(e=>e.querySelector('code')?.textContent==='providers.call')));return {ok:true,permissions:txt(article())};
"#).await?;
    let replay=protected(app,&route,&old_call).await;
    let fresh=protected(app,&route,&uuid::Uuid::new_v4().to_string()).await;
    require(checks,events,"regrant permits fresh calls without reviving settled old call handle",replay.code()==Some(1004004)&&fresh.success(),json!({"old_call":replay.describe(),"fresh_call":fresh.describe(),"scope":"capabilities bind runtime, not individual view; settled call IDs never revive"}))?;
    step(app,world,checks,events,"reauthorization opens fresh consumer document with actionable catalogue",json!({}),r#"
await closeSettings();const root=await until(consoleRoot);await until(()=>root.querySelector('[role=combobox]')?.disabled===false);if(root===window.__modulePage.root||root.querySelector('textarea').value!=='')throw Error('revoked document resurrected');const p=window.__modulePage,before=p.events.length;await click(/刷新选择目录|Refresh selections/,root);await until(()=>p.events.slice(before).some(e=>e.command==='module_list_ai_providers'&&e.reply?.success));set(root.querySelector('textarea'),'MODULE_ACCEPTANCE_DRAFT 中文 Aa 0123');await frame();p.root=root;p.view=root.closest('[data-module-view]');p.refresh=button(/刷新选择目录|Refresh selections/,root);await category('modules');return {ok:true};
"#).await?;
    step(app,world,checks,events,"close dirty module document through discard confirmation",json!({}),r#"
await closeSettings();const p=window.__modulePage;const close=all('button[aria-label]').find(e=>visible(e)&&/^(Close|关闭) Provider Console$/.test(e.getAttribute('aria-label')));if(!close)throw Error('module close control absent');close.click();const dialog=await until(()=>all('[role=alertdialog],[role=dialog]').find(e=>visible(e)&&button(/放弃并关闭|Discard and close/i,e)));await click(/放弃并关闭|Discard and close/i,dialog);await until(()=>!p.view.isConnected);const before=p.events.length;p.refresh.click();await frame();await new Promise(r=>setTimeout(r,150));return {ok:!p.root.isConnected&&p.events.length===before,oldView:p.view.dataset.moduleView,oldHandlerSentCalls:p.events.length-before};
"#).await?;
    shot(app,world,"module-page-document-released").await?;
    step(app,world,checks,events,"disable module and release runtime through Settings",json!({}),r#"
await category('modules');await click(/^(禁用|Disable)$/i,await until(article));await until(()=>button(/^(启用|Enable)$/i,article()));return {ok:true,module:txt(article())};
"#).await?;
    let closed=protected(app,&route,&uuid::Uuid::new_v4().to_string()).await;
    require(checks,events,"released runtime rejects previously issued capability route",closed.code()==Some(1004004),json!({"runtime":route.0,"reply":closed.describe(),"tracked_instances":app.state::<ModuleHostBridge>().loader().tracked_instances()}))?;
    shot(app,world,"module-page-runtime-released").await?;
    step(app,world,checks,events,"review reenable permissions through formal acknowledgement",json!({}),r#"
const a=article();await click(/^(启用|Enable)$/i,a);const d=await until(()=>a.querySelector('[role=dialog]'));d.querySelector('[role=checkbox],input[type=checkbox]').click();return {ok:true};
"#).await?;
    step(app,world,checks,events,"confirm reenable and await fresh runtime",json!({}),r#"
await click(/确认启用|Confirm enable/i,article().querySelector('[role=dialog]'));await until(()=>txt(article()).includes('ready'));return {ok:true};
"#).await?;
    step(app,world,checks,events,"reopen module through Apps Launcher",json!({}),"await closeSettings();await openConsole();return {ok:true};").await?;
    step(app,world,checks,events,"fresh document has independent identity and empty draft",json!({}),r#"
const r=await until(consoleRoot);await until(()=>r.querySelector('[role=combobox]')?.disabled===false);return {ok:r!==window.__modulePage.root&&r.querySelector('textarea').value==='',view:r.closest('[data-module-view]').dataset.moduleView};
"#).await?;
    let next=world.routes.issued().last().cloned().ok_or("fresh route absent")?;
    let dead=protected(app,&route,&uuid::Uuid::new_v4().to_string()).await;
    let live=protected(app,&next,&uuid::Uuid::new_v4().to_string()).await;
    require(checks,events,"new runtime is distinct and never resurrects old runtime route",next.0!=route.0&&next.2!=route.2&&dead.code()==Some(1004004)&&live.success(),json!({"old_runtime":route.0,"new_runtime":next.0,"old":dead.describe(),"new":live.describe()}))?;
    step(app,world,checks,events,"read permission audit from formal module Settings page",json!({}),"await category('modules');await until(article);return {ok:true};").await?;
    let audit=ipc(app,MAIN,"list_module_permission_audits",json!({"moduleId":CONSOLE,"before":null,"limit":100})).await;
    require(checks,events,"host audit preserves grant revoke and protected denial",audit.success()&&audit.data()["entries"].as_array().is_some_and(|rows|["granted","revoked","denied"].iter().all(|action|rows.iter().any(|row|row["permission"]=="providers.call"&&row["action"]==*action))),json!(audit.describe()))?;
    save(&world.root,"module-page-permission-audits",&json!({"reply":audit.describe()}))?;
    step(app,world,checks,events,"return to fresh module for final native evidence",json!({}),"await closeSettings();await until(consoleRoot);return {ok:true};").await?;
    shot(app,world,"module-page-reopened").await?;
    Ok(())
}

async fn scenario(app: AppHandle, world: PageWorld, ready: bool) {
    let mut checks=Checks::default();let mut events=Vec::new();
    let result=if ready {journey(&app,&world,&mut checks,&mut events).await} else {Err("production dist page did not load".into())};
    if let Err(error)=result {checks.check("focused production module journey completes",false,&error);let _=save(&world.root,"module-page-failure",&json!({"error":error,"dom":dom(&app,json!({}),"return {ok:true,url:location.href,body:document.body.innerText};").await}));let _=shot(&app,&world,"module-page-failure").await;}
    let trace=dom(&app,json!({}),r#"const p=window.__modulePage;const events=p?.events||[];if(p)window.fetch=p.original;delete window.__modulePage;return {ok:true,events};"#).await;
    let _=save(&world.root,"module-page-ipc",&trace);
    let no_model=trace["value"]["events"].as_array().is_some_and(|rows|rows.iter().all(|e|e["command"]!="module_run_ai"&&e["command"]!="module_call_provider"));
    checks.check("no model execution requested",no_model,"actual observed IPC commands");
    if let Ok((session,_))=app.state::<Arc<SessionResolver>>().agent_session() {
        checks.check("all owned module runtimes release through production stop_all",app.state::<ModuleOperationManager>().stop_all(&session).await.is_empty(),"formal shutdown");
    }
    let _=save(&world.root,"module-page-result",&json!({"passed":checks.passed,"failed":checks.failed,"checks":events,"source":"embedded production dist in macOS WKWebView; compiled-in admission pin provider-console@1.1.0; real authorization/session/attachment leases","model_requests":0,"document_close_contract":"document disposal does not unload shared runtime; old DOM handlers must not submit IPC; disable releases runtime and old route permanently","evidence_root":world.root,"database":world.root.join("dgos.db"),"screenshot_source":"capture_native exact NSWindow.windowNumber"}));
    drop(world.db);
    println!("MODULE PAGE: {} passed, {} failed; evidence {}",checks.passed,checks.failed.len(),world.root.display());
    app.exit(if checks.failed.is_empty(){0}else{1});
}

pub(super) fn run() {
    let root=root().expect("isolated module page root");
    println!("INFO module page retained evidence {}",root.display());
    std::thread::spawn(||{std::thread::sleep(Duration::from_secs(600));eprintln!("MODULE PAGE watchdog expired");std::process::exit(2)});
    let (sender,_)=broadcast::channel(64);let mut context=tauri::generate_context!();context.config_mut().app.windows.clear();
    let app=tauri::Builder::default().manage(Pending::default()).manage(LoadEvents(sender))
        .on_page_load(|webview,payload|{module_host::handle_page_load(webview,payload);if matches!(payload.event(),tauri::webview::PageLoadEvent::Finished){let _=webview.app_handle().state::<LoadEvents>().0.send((webview.label().into(),payload.url().path().into()));}})
        .on_window_event(module_host::handle_window_event)
        .setup(move|app|{
            let world=tauri::async_runtime::block_on(install(app,root)).map_err(std::io::Error::other)?;
            let events=app.state::<LoadEvents>().0.subscribe();
            WebviewWindowBuilder::new(app,MAIN,WebviewUrl::App("workbench".into())).inner_size(1440.,1000.).visible(true).build()?;
            let handle=app.handle().clone();tauri::async_runtime::spawn(async move{let ready=wait_load(events,MAIN,"/workbench").await;scenario(handle,world,ready).await;});Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::get_account_state,commands::create_account,commands::sign_in,commands::sign_out,
            commands::get_appearance_preferences,commands::set_appearance_preferences,commands::recover_appearance_preferences,commands::list_local_fonts,
            commands::get_language_preferences,commands::set_language_preferences,commands::get_desktop_preferences,commands::set_desktop_preferences,commands::get_cli_preferences,
            commands::start_module,commands::stop_module,commands::update_module,commands::get_module_operation,commands::get_module_runtime_status,commands::enable_module,commands::disable_module,
            commands::read_workspace_app_catalog,commands::list_modules,commands::uninstall_module,commands::grant_module_permission,commands::revoke_module_permission,commands::list_module_crashes,commands::list_module_permission_audits,
            commands::attach_module_host,commands::report_module_host,commands::report_module_crash,commands::module_list_provider_catalog,commands::cancel_module_call,
            commands::module_list_ai_providers,commands::module_list_ai_skills,commands::module_list_ai_tools,commands::module_list_ai_runs,commands::module_get_ai_run,
            super::super::smoke_report]).build(context).expect("module production page app");
    let code=Arc::new(std::sync::atomic::AtomicI32::new(3));let captured=code.clone();
    app.run(move|_,event|{if let tauri::RunEvent::ExitRequested{code:Some(code),..}=event{captured.store(code,std::sync::atomic::Ordering::SeqCst);}});
    std::process::exit(code.load(std::sync::atomic::Ordering::SeqCst));
}
