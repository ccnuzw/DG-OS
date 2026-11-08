//! Opt-in production-dist MCP page acceptance: DGOS_MCP_SMOKE_MODE=page.
//! Fresh /tmp root only; retained JSON, native target-window screenshots, IPC timing and asset hashes.
//! The original W4 compatibility journey is not run in this mode. No dev/preview URL is accepted.
//! Run after building dist: DGOS_MCP_SMOKE_MODE=page DGOS_MCP_PAGE_ROOT=/private/tmp/<fresh>
//! DGOS_OFFICIAL_MCP_SDK_PREFIX=/private/tmp/dgos-mcp-official DGOS_OFFICIAL_FILESYSTEM_SERVER=<entry>
//! cargo test --offline -p dgos --features mcp-smoke --test webview_smoke_mcp -- --nocapture
//! Requires foreground macOS WKWebView and Screen Recording permission. Synthesized NSEvents are
//! not physical typing; DOM only supplies text and establishes known starting focus/pointer shortcuts.
//! Fault evidence is explicitly smoke IPC: pre-dispatch rejection; gated real update; real refresh
//! completes/publishes before its receipt is lost. None is a preview fixture or a real storage outage.
use super::*;
use super::super::ui_keys::{self, Key};
use dgos_db::sea_orm::{ConnectionTrait, Statement};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::State;

const FS_NAME: &str = "MCPFilesystemABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789LongNameWrapCheck";
const URL_NAME: &str = "MCPLongURLABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789ReadOnlyConfig";
const JS: &str = include_str!("mcp_page.js");
type Journey<T> = Result<T, String>;

pub(super) fn selected() -> bool { std::env::var("DGOS_MCP_SMOKE_MODE").is_ok_and(|mode| mode == "page") }
pub(super) fn root() -> Journey<PathBuf> {
    let root = std::env::var_os("DGOS_MCP_PAGE_ROOT").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/private/tmp").join(format!("dgos-mcp-page-{}", uuid::Uuid::new_v4())));
    let parent = root.parent().ok_or("page root needs a parent")?.canonicalize().map_err(|e|e.to_string())?;
    if !parent.starts_with("/private/tmp") && !parent.starts_with("/tmp") { return Err("page root must be a fresh directory under /tmp".into()); }
    if root.exists() { return Err(format!("refusing existing page evidence root: {}", root.display())); }
    std::fs::create_dir(&root).map_err(|e|e.to_string())?;
    println!("INFO MCP page retained evidence {}",root.display());
    Ok(root)
}
pub(super) async fn install(app: &mut tauri::App, fx: &Fixture) {
    use dgos_application::{ModuleCrashService, ModuleManagementService, ModuleOperationManager};
    use dgos_db::{ModuleCrashRepository, ModulePermissionRepository};
    use dgos_modules::{LocalModuleSources, ModulePublisher, PageLoaderConfig, PageModuleLoader};
    let publisher=Arc::new(ModulePublisher::new(fx.root.join("modules"),fx.db.clone()));
    let transport=Arc::new(crate::module_host::ChannelTransport::default());
    let (loader,lost)=PageModuleLoader::new(transport.clone(),PageLoaderConfig {window_label:MAIN.into(),hook_timeout:Duration::from_secs(2)});
    let operations=ModuleOperationManager::new(publisher,Arc::new(loader.clone()),LocalModuleSources::new(fx.root.join("module-sources")))
        .with_maintenance(app.state::<dgos_db::MaintenanceGate>().inner().clone());
    operations.spawn_lost_runtime_reporter(lost).expect("MCP page module reporter");
    operations.recover().await.expect("MCP page empty module catalog recovery");
    let crashes=ModuleCrashService::new(ModuleCrashRepository::new(fx.db.clone()),operations.clone());
    app.manage(ModuleManagementService::new(Arc::new(ModulePermissionRepository::new(fx.db.clone())),operations.clone(),crashes.clone()));
    app.manage(operations);
    app.manage(crashes);
    app.manage(crate::module_host::ModuleHostBridge::new(loader,transport));
}
#[derive(Default)]
pub(super) struct Control {
    reject: Mutex<HashSet<String>>,
    gate_update: AtomicBool,
    release_update: AtomicBool,
    lose_refresh_receipt: AtomicBool,
    counts: Mutex<HashMap<String, usize>>,
    events: Mutex<Vec<Value>>,
}
impl Control {
    fn event(&self, command: &str, phase: &str, detail: Value) {
        let at=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
        self.events.lock().unwrap().push(json!({"at_ms":at,"command":command,"phase":phase,"detail":detail}));
    }
    fn before(&self, command: &str) -> Journey<()> {
        *self.counts.lock().unwrap().entry(command.into()).or_default()+=1;
        self.event(command,"received",json!({}));
        if self.reject.lock().unwrap().contains(command) {
            self.event(command,"smoke_pre_dispatch_rejection",json!({"production_handler_called":false}));
            return Err(format!("MCP page smoke controlled read failure: {command}"));
        }
        Ok(())
    }
    fn reject(&self, command: &str, enabled: bool) {
        let mut set=self.reject.lock().unwrap();
        if enabled { set.insert(command.into()); } else { set.remove(command); }
    }
    fn count(&self, command: &str) -> usize { *self.counts.lock().unwrap().get(command).unwrap_or(&0) }
}
#[tauri::command]
async fn list_mcp_servers(window: tauri::WebviewWindow, state: State<'_,commands::McpState>, control: State<'_,Control>) -> Result<Vec<dgos_mcp::McpServer>,String> {
    control.before("list_mcp_servers")?; commands::list_mcp_servers(window,state).await
}
#[tauri::command]
async fn get_mcp_server_config(window: tauri::WebviewWindow, server_id: String, state: State<'_,commands::McpState>, control: State<'_,Control>) -> Result<CreateMcpServerRequest,String> {
    control.before("get_mcp_server_config")?; commands::get_mcp_server_config(window,server_id,state).await
}
#[tauri::command]
async fn list_mcp_tools(window: tauri::WebviewWindow, server_id: String, state: State<'_,commands::McpState>, control: State<'_,Control>) -> Result<Vec<dgos_mcp::McpTool>,String> {
    control.before("list_mcp_tools")?; commands::list_mcp_tools(window,server_id,state).await
}
#[tauri::command]
async fn list_mcp_usage_logs(window: tauri::WebviewWindow, server_id: Option<String>, limit: Option<u64>, state: State<'_,commands::McpState>, control: State<'_,Control>) -> Result<Vec<dgos_mcp::McpUsageLog>,String> {
    control.before("list_mcp_usage_logs")?; commands::list_mcp_usage_logs(window,server_id,limit,state).await
}
#[tauri::command]
async fn list_mcp_connection_logs(window: tauri::WebviewWindow, server_id: Option<String>, limit: Option<u64>, state: State<'_,commands::McpState>, control: State<'_,Control>) -> Result<Vec<dgos_mcp::McpConnectionLog>,String> {
    control.before("list_mcp_connection_logs")?; commands::list_mcp_connection_logs(window,server_id,limit,state).await
}
#[tauri::command]
async fn update_mcp_server(window: tauri::WebviewWindow, server_id: String, request: dgos_mcp::UpdateMcpServerRequest, state: State<'_,commands::McpState>, control: State<'_,Control>) -> Result<dgos_mcp::McpServer,String> {
    control.before("update_mcp_server")?;
    if control.gate_update.swap(false,Ordering::SeqCst) {
        control.event("update_mcp_server","smoke_gate_entered",json!({"server_id":server_id,"production_handler_called":false}));
        let deadline=tokio::time::Instant::now()+Duration::from_secs(15);
        while !control.release_update.load(Ordering::SeqCst) {
            if tokio::time::Instant::now()>=deadline { return Err("controlled save gate was not released in 15s".into()); }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        control.event("update_mcp_server","smoke_gate_released",json!({"server_id":server_id}));
    }
    let result=commands::update_mcp_server(window,server_id,request,state).await;
    control.event("update_mcp_server","real_handler_settled",json!({"success":result.is_ok()}));
    result
}
#[tauri::command]
async fn refresh_mcp_tools(window: tauri::WebviewWindow, server_id: String, state: State<'_,commands::McpState>, control: State<'_,Control>) -> Result<Vec<dgos_mcp::McpTool>,String> {
    control.before("refresh_mcp_tools")?;
    let result=commands::refresh_mcp_tools(window,server_id.clone(),state).await;
    control.event("refresh_mcp_tools","real_handler_settled",json!({"server_id":server_id,"success":result.is_ok(),"tool_count":result.as_ref().ok().map(Vec::len)}));
    if result.is_ok() && control.lose_refresh_receipt.swap(false,Ordering::SeqCst) {
        control.event("refresh_mcp_tools","smoke_receipt_lost_after_publication",json!({"real_handler_succeeded":true}));
        Err("MCP page smoke lost reply after real discovery publication".into())
    } else { result }
}
pub(super) fn handle(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
    let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![list_mcp_servers, get_mcp_server_config, update_mcp_server, refresh_mcp_tools, list_mcp_tools, list_mcp_usage_logs, list_mcp_connection_logs];
    handler(invoke)
}
fn save(fx: &Fixture, name: &str, value: &Value) -> Journey<()> {
    std::fs::write(fx.root.join(format!("{name}.json")),serde_json::to_vec_pretty(value).map_err(|e|e.to_string())?).map_err(|e|e.to_string())
}
async fn dom(app: &AppHandle, args: Value, body: &str) -> Value {
    eval(app,MAIN,&format!("{JS}\nconst A={args};\n{body}")).await
}
async fn step(app: &AppHandle, fx: &Fixture, checks: &mut Checks, name: &str, args: Value, body: &str) -> Journey<Value> {
    let value=dom(app,args,body).await;
    let passed=value["ok"]==true && value["value"]["ok"]==true;
    app.state::<Control>().event("page_step",name,json!({"passed":passed,"result":value}));
    save(fx,"ipc-timeline",&json!(*app.state::<Control>().events.lock().unwrap()))?;
    checks.check(name,passed,&value);
    if passed { Ok(value["value"].clone()) } else { Err(format!("{name}: {value}")) }
}
#[cfg(target_os="macos")]
async fn capture(app: &AppHandle, fx: &Fixture, name: &str) -> Journey<()> {
    use objc2::{msg_send,runtime::AnyObject};
    let window=app.get_webview_window(MAIN).ok_or("main window absent")?;
    let (send,receive)=tokio::sync::oneshot::channel();
    let target=window.clone();
    window.run_on_main_thread(move || {
        let result=target.ns_window().map_err(|e|e.to_string()).and_then(|pointer| {
            let native=pointer as *mut AnyObject;
            if native.is_null() { return Err("native target absent".into()); }
            let number:isize=unsafe {msg_send![native,windowNumber]};
            if number>0 {Ok(number)} else {Err("native window number absent".into())}
        }); let _=send.send(result);
    }).map_err(|e|e.to_string())?;
    let number=tokio::time::timeout(Duration::from_secs(10),receive).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())??;
    tokio::time::sleep(Duration::from_millis(350)).await;
    let path=fx.root.join(format!("{name}.png"));
    let status=std::process::Command::new("/usr/sbin/screencapture").args(["-x","-o","-l"]).arg(number.to_string()).arg(&path).status().map_err(|e|e.to_string())?;
    save(fx,&format!("{name}-capture"),&json!({"path":path,"window_number":number,"pid":std::process::id(),"source":"actual NSWindow.windowNumber, screencapture -x -o -l","success":status.success()}))?;
    if !status.success() { return Err(format!("native window capture failed: {status}; Screen Recording permission required")); }
    Ok(())
}
#[cfg(not(target_os="macos"))]
async fn capture(_: &AppHandle, _: &Fixture, _: &str) -> Journey<()> { Err("MCP page acceptance requires macOS".into()) }
async fn screenshot(app: &AppHandle, fx: &Fixture, name: &str) -> Journey<()> {
    save(fx,&format!("{name}-dom"),&dom(app,json!({}),"return snapshot();").await)?;
    capture(app,fx,name).await
}
async fn navigate(app: &AppHandle, fx: &Fixture, checks: &mut Checks, path: &str) -> Journey<()> {
    step(app,fx,checks,&format!("navigate {path}"),json!({"path":path}),"go(A.path);await frame();return {ok:!!await until(()=>location.pathname===A.path&&(A.path!=='/mcp'||visible(page())))};").await?; Ok(())
}
async fn appearance(app: &AppHandle, fx: &Fixture, checks: &mut Checks, mode: &str, large: bool) -> Journey<()> {
    let read=ipc(app,MAIN,"get_appearance_preferences",json!({})).await;
    let mut snapshot=read.data()["snapshot"].clone();
    let revision=snapshot.as_object_mut().and_then(|s|s.remove("revision")).ok_or("appearance snapshot absent")?;
    snapshot["mode"]=json!(mode); snapshot["density"]=json!(if large {150}else{100});
    snapshot["typography"]["uiSize"]=json!(if large {16}else{12});
    snapshot["typography"]["readingSize"]=json!(if large {20}else{13});
    snapshot["typography"]["codeSize"]=json!(if large {18}else{12});
    snapshot["effects"]["reducedMotion"]=json!("reduce");
    let saved=ipc(app,MAIN,"set_appearance_preferences",json!({"snapshot":snapshot,"expectedRevision":revision})).await;
    if !saved.success() || saved.data()["kind"]!="saved" { return Err(saved.describe()); }
    // Apply through the production bootstrap on a real reload, never patch theme DOM/CSS in the driver.
    app.get_webview_window(MAIN).ok_or("main window absent")?.eval("location.reload()").map_err(|e|e.to_string())?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    step(app,fx,checks,"production appearance bootstrap applies saved host snapshot",json!({"mode":mode,"density":if large{150}else{100}}),"return {ok:!!await until(()=>document.documentElement.dataset.theme===A.mode&&document.documentElement.dataset.density===String(A.density)&&!!document.querySelector('.workspace-tabbar'))};").await?;
    navigate(app,fx,checks,"/mcp").await?;
    step(app,fx,checks,"MCP hydrates after production reload",json!({}),"return {ok:!!await until(()=>rows().length===3)};").await?;
    Ok(())
}
async fn seed(fx: &Fixture) -> Journey<String> {
    fx.manager.update_server(USER,&fx.filesystem_id,dgos_mcp::UpdateMcpServerRequest{name:Some(FS_NAME.into()),..Default::default()}).await.map_err(|e|e.to_string())?;
    // Renaming disconnects by production policy; restore a genuine filesystem connection once.
    fx.manager.connect(USER,&fx.filesystem_id).await.map_err(|e|e.to_string())?;
    let url=format!("https://example.invalid/{}", "longURLsegmentABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".repeat(7));
    let server=fx.manager.create_server(USER,CreateMcpServerRequest{name:URL_NAME.into(),transport:TransportKind::Http,stdio:None,http:Some(HttpConfig{url,credential_ref:None,allow_insecure_http:false}),enabled:Some(false),auto_connect:Some(false)}).await.map_err(|e|e.to_string())?;
    // Synthetic long log rows only exercise wrapping. Real SDK/filesystem lifecycle and usage records
    // remain present and are read independently. This marker must never be called a genuine MCP event.
    fx.db.execute(Statement::from_sql_and_values(fx.db.get_database_backend(),"INSERT INTO mcp_connection_logs (user_id,server_id,event,status,error_class,created_at) VALUES (?,?,?,?,?,?)",vec![USER.into(),fx.http_id.clone().into(),format!("SYNTHETIC_LAYOUT_ONLY_{}","LongLogToken".repeat(24)).into(),"configured".into(),format!("SYNTHETIC_LAYOUT_ONLY_{}","LongErrorClass".repeat(16)).into(),1793948903_i64.into()])).await.map_err(|e|e.to_string())?;
    dgos_db::HostSettingsRepository::new(fx.db.clone()).set_language(dgos_types::LanguagePreference::parse("en-US").ok_or("English locale absent")?,1793948903).await.map_err(|e|e.to_string())?;
    Ok(server.id)
}
async fn failures(app: &AppHandle, fx: &Fixture, checks: &mut Checks) -> Journey<()> {
    let control=app.state::<Control>();
    control.reject("list_mcp_servers",true);
    navigate(app,fx,checks,"/mcp").await?;
    step(app,fx,checks,"initial server read failure is visible without empty success",json!({}),"return {ok:!!await until(()=>text(page()).includes('controlled read failure'))&&rows().length===0&&!text(page()).includes('No MCP servers yet')};").await?;
    screenshot(app,fx,"initial-read-failure").await?;
    control.reject("list_mcp_servers",false);
    step(app,fx,checks,"explicit retry recovers real server catalog",json!({}),"button('Retry',page()).click();return {ok:!!await until(()=>rows().length===3&&!text(page()).includes('controlled read failure'))};").await?;
    control.reject("list_mcp_servers",true);
    step(app,fx,checks,"failed refresh retains old server rows and announces stale content",json!({}),"button('Refresh status',page()).click();return {ok:!!await until(()=>text(page()).includes('controlled read failure'))&&rows().length===3};").await?;
    screenshot(app,fx,"server-refresh-failure-retains").await?;
    control.reject("list_mcp_servers",false);
    step(app,fx,checks,"server refresh failure recovers explicitly",json!({}),"button('Refresh status',page()).click();return {ok:!!await until(()=>rows().length===3&&!text(page()).includes('controlled read failure'))};").await?;
    for command in ["list_mcp_tools","list_mcp_usage_logs","list_mcp_connection_logs"] { control.reject(command,true); }
    step(app,fx,checks,"initial selected detail reads fail visibly",json!({"name":"Official SDK Echo"}),"row(A.name).click();return {ok:!!await until(()=>['list_mcp_tools','list_mcp_usage_logs','list_mcp_connection_logs'].every(c=>text(detail()).includes(c)))};").await?;
    screenshot(app,fx,"detail-initial-read-failures").await?;
    for command in ["list_mcp_tools","list_mcp_usage_logs","list_mcp_connection_logs"] { control.reject(command,false); }
    step(app,fx,checks,"detail retry restores genuine tools logs and connections",json!({}),"for(const b of all('button',detail()).filter(e=>text(e)==='Retry'))b.click();return {ok:!!await until(()=>text(detail()).includes('echo')&&text(detail()).includes('SYNTHETIC_LAYOUT_ONLY_')&&!text(detail()).includes('controlled read failure'))};").await?;
    let called=super::call_in_browser(app,"Official SDK Echo","echo",r#"{"text":"MCP-PAGE-REAL-AUDIT"}"#).await;
    if called["result"]!=true || !called["text"].as_str().is_some_and(|s|s.contains("MCP-PAGE-REAL-AUDIT")) { return Err(format!("real page echo audit prerequisite failed: {called}")); }
    let logs=fx.manager.list_usage_logs(USER,Some(&fx.http_id),Some(50)).await.map_err(|e|e.to_string())?;
    if logs.len()!=1 || logs[0].outcome!="success" { return Err(format!("real echo audit missing: {logs:?}")); }
    save(fx,"real-echo-audit",&json!({"page":called,"host_usage":logs,"source":"actual production page confirmation, SDK echo and manager terminal usage read"}))?;
    step(app,fx,checks,"usage refresh reads real confirmed echo audit",json!({}),"button('Refresh usage',detail()).click();return {ok:!!await until(()=>all('.mcp-log-list',detail()).some(e=>text(e).includes('echo')&&text(e).includes('success')))};").await?;
    // Force refreshed detail reads by deactivation/reactivation while the successful old snapshots exist.
    for command in ["list_mcp_tools","list_mcp_usage_logs","list_mcp_connection_logs"] { control.reject(command,true); }
    navigate(app,fx,checks,"/").await?; navigate(app,fx,checks,"/mcp").await?;
    step(app,fx,checks,"failed detail refresh keeps old tools usage and connection content",json!({}),"return {ok:!!await until(()=>['list_mcp_tools','list_mcp_usage_logs','list_mcp_connection_logs'].every(c=>text(detail()).includes(c)))&&text(detail()).includes('echo')&&all('.mcp-log-list',detail()).some(e=>text(e).includes('echo')&&text(e).includes('success'))&&text(detail()).includes('SYNTHETIC_LAYOUT_ONLY_')};").await?;
    screenshot(app,fx,"detail-refresh-failures-retain").await?;
    for command in ["list_mcp_tools","list_mcp_usage_logs","list_mcp_connection_logs"] { control.reject(command,false); }
    step(app,fx,checks,"detail refresh read failures recover",json!({}),"for(const b of all('button',detail()).filter(e=>text(e)==='Retry'))b.click();return {ok:!!await until(()=>!text(detail()).includes('controlled read failure'))};").await?;
    control.reject("get_mcp_server_config",true);
    step(app,fx,checks,"edit configuration first read failure freezes blank fields",json!({}),"button('Edit',detail()).click();return {ok:!!await until(()=>text(form()).includes('get_mcp_server_config'))&&all('input',form()).every(e=>e.disabled)&&form().querySelector('button[type=submit]').disabled};").await?;
    screenshot(app,fx,"config-initial-read-failure").await?;
    control.reject("get_mcp_server_config",false);
    step(app,fx,checks,"configuration retry restores real HTTP config",json!({}),"button('Retry',form()).click();return {ok:!!await until(()=>field('Name').value==='Official SDK Echo'&&!field('Name').disabled)};").await?;
    step(app,fx,checks,"close unchanged config",json!({}),"button('Cancel',form()).click();return {ok:!!await until(()=>!form())};").await?;
    Ok(())
}
async fn discovery(app: &AppHandle, fx: &Fixture, checks: &mut Checks) -> Journey<()> {
    let control=app.state::<Control>();
    let before=control.count("refresh_mcp_tools");
    control.lose_refresh_receipt.store(true,Ordering::SeqCst);
    step(app,fx,checks,"real discovery publication with lost receipt stays unconfirmed",json!({}),"button('Refresh tools',detail()).click();return {ok:!!await until(()=>text(feedback()).includes('is unconfirmed'))&&button('Refresh tools',detail()).disabled};").await?;
    let real=fx.manager.list_tools(USER,&fx.http_id).await.map_err(|e|e.to_string())?;
    save(fx,"discovery-real-publication",&json!({"source":"real McpManager read after real refresh handler succeeded, reply loss is smoke-only","tools":real,"refresh_before":before,"refresh_after":control.count("refresh_mcp_tools")}))?;
    if control.count("refresh_mcp_tools")!=before+1 || real.is_empty() { return Err("real discovery call/publication count mismatch".into()); }
    screenshot(app,fx,"discovery-unconfirmed").await?;
    step(app,fx,checks,"read saved tools does not certify discovery success",json!({}),"button('Read saved tools',detail()).click();return {ok:!!await until(()=>text(detail()).includes('echo'))&&text(feedback()).includes('is unconfirmed')&&button('Refresh tools',detail()).disabled};").await?;
    step(app,fx,checks,"edit saved config does not certify discovery success",json!({}),"button('Edit',detail()).click();await until(()=>form()&&field('Name')?.value==='Official SDK Echo');button('Cancel',form()).click();return {ok:!!await until(()=>!form())&&text(feedback()).includes('is unconfirmed')};").await?;
    navigate(app,fx,checks,"/").await?; navigate(app,fx,checks,"/mcp").await?;
    step(app,fx,checks,"inactive return and read refresh do not replay discovery",json!({}),"return {ok:!!await until(()=>text(feedback()).includes('is unconfirmed'))&&button('Refresh tools',detail()).disabled};").await?;
    if control.count("refresh_mcp_tools")!=before+1 { return Err("unconfirmed discovery was replayed".into()); }
    save(fx,"discovery-no-replay",&json!({"before":before,"after":control.count("refresh_mcp_tools"),"tools_reads":control.count("list_mcp_tools"),"unconfirmed_remains":true,"scope":"same mounted resource surface including hide/show, saved tool/config reads; full process restart is not asserted"}))?;
    Ok(())
}
async fn native(app: &AppHandle, fx: &Fixture, checks: &mut Checks) -> Journey<()> {
    if !ui_keys::prepare(app).await { return Err("native key foreground preflight failed".into()); }
    step(app,fx,checks,"establish known row focus for native list selection",json!({"name":FS_NAME}),"const b=row(A.name);b.focus();return {ok:document.activeElement===b};").await?;
    ui_keys::key(app,Key::Enter).await;
    step(app,fx,checks,"native Enter selects actual MCP server row",json!({"name":FS_NAME}),"return {ok:!!await until(()=>row(A.name).getAttribute('aria-pressed')==='true'&&text(detail()).includes('list_directory'))};").await?;
    step(app,fx,checks,"establish real tool opener focus",json!({}),"const a=all('article',detail()).find(e=>text(e.querySelector('code'))==='list_directory');const b=button('Inspect / call',a);window.__mcpOpener=b;b.focus();return {ok:document.activeElement===b};").await?;
    ui_keys::key(app,Key::Enter).await;
    step(app,fx,checks,"native Enter opens production tool Modal with focus inside",json!({}),"return {ok:!!await until(()=>dialog()?.contains(document.activeElement))};").await?;
    for key in [Key::Tab,Key::ShiftTab] { for _ in 0..8 { ui_keys::key(app,key).await; step(app,fx,checks,"native Tab keeps focus trapped in tool Modal",json!({}),"return {ok:dialog()?.contains(document.activeElement)};").await?; } }
    screenshot(app,fx,"native-tool-modal").await?;
    ui_keys::key(app,Key::Escape).await;
    step(app,fx,checks,"native Escape closes tool Modal and returns exact opener focus",json!({}),"return {ok:!!await until(()=>!dialog()&&document.activeElement===window.__mcpOpener)};").await?;
    step(app,fx,checks,"establish editor opener focus",json!({}),"const b=button('Edit',detail());b.focus();return {ok:document.activeElement===b};").await?;
    ui_keys::key(app,Key::Enter).await;
    step(app,fx,checks,"native Enter opens real editor and selects transport starting focus",json!({"name":FS_NAME}),"if(!await until(()=>form()&&field('Name')?.value===A.name))return {ok:false};const s=form().querySelector('[role=combobox]');window.__mcpSelect=s;s.focus();return {ok:document.activeElement===s};").await?;
    ui_keys::key(app,Key::Space).await;
    step(app,fx,checks,"native Space opens editor Select",json!({}),"return {ok:!!await until(()=>document.querySelector('[role=listbox]'))};").await?;
    ui_keys::key(app,Key::End).await; ui_keys::key(app,Key::Enter).await;
    step(app,fx,checks,"native End Enter changes transport to HTTP",json!({}),"return {ok:!!await until(()=>field('Server URL'))&&!document.querySelector('[role=listbox]')&&document.activeElement===window.__mcpSelect};").await?;
    step(app,fx,checks,"focus cancel before native dirty-form Modal",json!({}),"const b=button('Cancel',form());window.__mcpCancel=b;b.focus();return {ok:document.activeElement===b};").await?;
    ui_keys::key(app,Key::Enter).await;
    step(app,fx,checks,"native editor cancel opens discard Modal",json!({}),"return {ok:!!await until(()=>dialog()&&text(dialog()).includes('Unsaved changes')&&dialog().contains(document.activeElement))};").await?;
    ui_keys::key(app,Key::Escape).await;
    step(app,fx,checks,"native Escape preserves editor draft and returns cancel focus",json!({}),"return {ok:!!await until(()=>!dialog()&&field('Server URL')&&document.activeElement===window.__mcpCancel)};").await?;
    ui_keys::key(app,Key::Enter).await;
    // A known dialog control is focused; acceptance of the destructive choice is still a native Enter.
    step(app,fx,checks,"focus explicit discard choice",json!({}),"const b=await until(()=>button('Discard and close',dialog()));b.focus();return {ok:document.activeElement===b};").await?;
    ui_keys::key(app,Key::Enter).await;
    step(app,fx,checks,"native explicit discard closes editor",json!({}),"return {ok:!!await until(()=>!dialog()&&!form())};").await?;
    Ok(())
}
async fn drafts(app: &AppHandle, fx: &Fixture, checks: &mut Checks) -> Journey<()> {
    step(app,fx,checks,"create server form draft through actual input handler",json!({}),"button('Add MCP server',page()).click();await until(()=>form());set(field('Name'),'MCP-DRAFT-RETAINED');await frame();set(field('Executable'),'/usr/bin/true');return {ok:!!await until(()=>field('Name').value==='MCP-DRAFT-RETAINED'&&field('Executable').value==='/usr/bin/true')};").await?;
    for path in ["/","/settings/appearance"] {
        navigate(app,fx,checks,path).await?;
        if path.starts_with("/settings") { step(app,fx,checks,"close settings overlay with actual control",json!({}),"button('Close settings').click();return {ok:!!await until(()=>!document.querySelector('.workspace-settings-dialog'))};").await?; }
        navigate(app,fx,checks,"/mcp").await?;
        step(app,fx,checks,"MCP configuration draft survives inactive work surface",json!({}),"return {ok:!!await until(()=>field('Name')?.value==='MCP-DRAFT-RETAINED')&&field('Executable').value==='/usr/bin/true'};").await?;
    }
    step(app,fx,checks,"surface close requests unsaved confirmation",json!({}),"document.querySelector('[data-workspace-tab=\"/mcp\"]').closest('.workspace-tab').querySelector('.workspace-tab-close').click();return {ok:!!await until(()=>dialog()&&text(dialog()).includes('Unsaved changes'))};").await?;
    screenshot(app,fx,"surface-close-cancel").await?;
    ui_keys::key(app,Key::Escape).await;
    step(app,fx,checks,"surface close cancellation retains same configuration draft",json!({}),"return {ok:!!await until(()=>!dialog())&&!!document.querySelector('[data-workspace-tab=\"/mcp\"]')&&field('Name').value==='MCP-DRAFT-RETAINED'};").await?;
    step(app,fx,checks,"surface explicit discard removes only target MCP surface",json!({}),"document.querySelector('[data-workspace-tab=\"/mcp\"]').closest('.workspace-tab').querySelector('.workspace-tab-close').click();const b=await until(()=>button('Discard and close',dialog()));b.click();return {ok:!!await until(()=>!document.querySelector('[data-workspace-tab=\"/mcp\"]'))};").await?;
    navigate(app,fx,checks,"/mcp").await?;
    step(app,fx,checks,"discarded surface reopens without old draft",json!({}),"return {ok:!!await until(()=>rows().length===3)&&!form()};").await?;
    let control=app.state::<Control>();
    control.release_update.store(false,Ordering::SeqCst); control.gate_update.store(true,Ordering::SeqCst);
    let before=control.count("update_mcp_server");
    step(app,fx,checks,"begin gated real save for server A",json!({"name":FS_NAME}),"row(A.name).click();await frame();if(!await until(()=>row(A.name).getAttribute('aria-pressed')==='true'))return {ok:false};button('Edit',detail()).click();await until(()=>field('Name')?.value===A.name);set(field('Name'),'MCP-SLOW-SAVED-A');await until(()=>field('Name').value==='MCP-SLOW-SAVED-A');form().querySelector('button[type=submit]').click();return {ok:!!await until(()=>field('Name')?.disabled)};").await?;
    let gate_deadline=tokio::time::Instant::now()+Duration::from_secs(5);
    loop {
        let entered=control.events.lock().unwrap().iter().any(|event|event["command"]=="update_mcp_server"&&event["phase"]=="smoke_gate_entered");
        if entered { break; }
        if tokio::time::Instant::now()>=gate_deadline { return Err("slow-save real command never entered the controlled gate".into()); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    step(app,fx,checks,"select B during pending A save without replacing the saving editor",json!({}),"row('Official SDK Echo').click();return {ok:!!await until(()=>row('Official SDK Echo').getAttribute('aria-pressed')==='true')&&field('Name').value==='MCP-SLOW-SAVED-A'&&field('Name').disabled&&button('Edit',detail()).disabled&&button('Add MCP server',page()).disabled};").await?;
    screenshot(app,fx,"slow-save-A-selected-B-before-receipt").await?;
    control.release_update.store(true,Ordering::SeqCst);
    step(app,fx,checks,"late A save receipt never steals B selection",json!({}),"return {ok:!!await until(()=>!form()&&row('MCP-SLOW-SAVED-A'))&&row('Official SDK Echo').getAttribute('aria-pressed')==='true'&&text(document.querySelector('.mcp-detail-heading')).includes('Official SDK Echo')};").await?;
    let stored=fx.manager.server_config(USER,&fx.filesystem_id).await.map_err(|e|e.to_string())?;
    if stored.name!="MCP-SLOW-SAVED-A" || control.count("update_mcp_server")!=before+1 { return Err("slow real save persistence/count mismatch".into()); }
    save(fx,"slow-save-host-readback",&json!({"source":"real manager after one gated real production update","server":stored,"update_before":before,"update_after":control.count("update_mcp_server")}))?;
    step(app,fx,checks,"new B draft created only after A receipt remains intact",json!({}),"button('Edit',detail()).click();await until(()=>field('Name')?.value==='Official SDK Echo');set(field('Name'),'MCP-NEW-B-DRAFT');return {ok:!!await until(()=>field('Name').value==='MCP-NEW-B-DRAFT')};").await?;
    navigate(app,fx,checks,"/").await?;navigate(app,fx,checks,"/mcp").await?;
    step(app,fx,checks,"B draft survives subsequent asynchronous presentation reads",json!({}),"return {ok:!!await until(()=>field('Name')?.value==='MCP-NEW-B-DRAFT')&&row('Official SDK Echo').getAttribute('aria-pressed')==='true'};").await?;
    step(app,fx,checks,"discard B draft explicitly",json!({}),"button('Cancel',form()).click();(await until(()=>button('Discard and close',dialog()))).click();return {ok:!!await until(()=>!form()&&!dialog())};").await?;
    // Password text is a disposable sentinel; it is never submitted and never included in snapshots.
    step(app,fx,checks,"seed unsaved Bearer sentinel",json!({}),"set(document.querySelector('.mcp-server-detail input[type=password]'),'synthetic-unsaved-bearer');return {ok:!!await until(()=>document.querySelector('.mcp-server-detail input[type=password]').value.length>0)};").await?;
    navigate(app,fx,checks,"/").await?;navigate(app,fx,checks,"/mcp").await?;
    step(app,fx,checks,"inactive work surface clears unsaved Bearer",json!({}),"return {ok:!!await until(()=>document.querySelector('.mcp-server-detail input[type=password]'))&&document.querySelector('.mcp-server-detail input[type=password]').value===''};").await?;
    step(app,fx,checks,"seed Bearer before settings overlay",json!({}),"set(document.querySelector('.mcp-server-detail input[type=password]'),'synthetic-unsaved-bearer');return {ok:!!await until(()=>document.querySelector('.mcp-server-detail input[type=password]').value.length>0)};").await?;
    navigate(app,fx,checks,"/settings/appearance").await?;
    step(app,fx,checks,"close settings after Bearer deactivation",json!({}),"button('Close settings').click();return {ok:!!await until(()=>!document.querySelector('.workspace-settings-dialog'))};").await?;
    navigate(app,fx,checks,"/mcp").await?;
    step(app,fx,checks,"settings overlay inactivity clears unsaved Bearer",json!({}),"return {ok:!!await until(()=>document.querySelector('.mcp-server-detail input[type=password]'))&&document.querySelector('.mcp-server-detail input[type=password]').value===''};").await?;
    Ok(())
}
async fn matrix(app: &AppHandle, fx: &Fixture, checks: &mut Checks) -> Journey<()> {
    let window=app.get_webview_window(MAIN).ok_or("main window absent")?;
    for mode in ["light","dark"] { for large in [false,true] { for width in [1440,800,520] {
        window.set_size(tauri::LogicalSize::new(width as f64,900.0)).map_err(|e|e.to_string())?;
        appearance(app,fx,checks,mode,large).await?;
        let name=format!("matrix-{mode}-{}-{width}",if large{"relaxed115"}else{"standard100"});
        step(app,fx,checks,"wide or compact server list wraps long names",json!({"width":width}),"if(document.querySelector('.mcp-back-list')&&!document.querySelector('.mcp-server-list').hidden){}else if(document.querySelector('.mcp-back-list'))document.querySelector('.mcp-back-list').click();await frame();return audit('list',A.width);").await?;
        screenshot(app,fx,&format!("{name}-list")).await?;
        step(app,fx,checks,"actual long URL config is editable without page overflow",json!({"name":URL_NAME,"width":width}),"row(A.name).click();await frame();await until(()=>!detail().hidden&&row(A.name).getAttribute('aria-pressed')==='true');button('Edit',detail()).click();await until(()=>field('Name')?.value===A.name);await frame();return {...audit('form',A.width),urlLength:field('Server URL')?.value.length,ok:audit('form',A.width).ok&&field('Server URL')?.value.length>300};").await?;
        screenshot(app,fx,&format!("{name}-url-form")).await?;
        step(app,fx,checks,"close unchanged long URL form",json!({}),"button('Cancel',form()).click();return {ok:!!await until(()=>!form())};").await?;
        // Selecting SDK is a DOM pointer shortcut; keyboard modal/Select checks are a separate slice.
        step(app,fx,checks,"detail shows long diagnostic log without page overflow",json!({"width":width}),"if(document.querySelector('.mcp-back-list')){document.querySelector('.mcp-back-list').click();await frame();}row('Official SDK Echo').click();await until(()=>text(detail()).includes('echo'));document.querySelector('.mcp-diagnostics').open=true;await until(()=>text(detail()).includes('SYNTHETIC_LAYOUT_ONLY_'));await frame();return audit('detail',A.width);").await?;
        screenshot(app,fx,&format!("{name}-detail-log")).await?;
        step(app,fx,checks,"long JSON tool Modal fits native viewport",json!({"width":width,"long":"LongJSONValueABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".repeat(18)}),"button('Inspect / call',detail()).click();await until(()=>dialog()?.querySelector('textarea'));set(dialog().querySelector('textarea'),JSON.stringify({text:A.long}));await until(()=>dialog().querySelector('textarea').value.includes(A.long));button('Review tool call',dialog()).click();if(!await until(()=>all('section[aria-label] pre',dialog()).some(p=>text(p).includes(A.long))))return {ok:false,error:'long JSON review absent'};await frame();return audit('modal',A.width);").await?;
        screenshot(app,fx,&format!("{name}-json-modal")).await?;
        ui_keys::key(app,Key::Escape).await;
        step(app,fx,checks,"long JSON Modal closes without dispatch",json!({}),"return {ok:!!await until(()=>!dialog())};").await?;
    } } }
    // Focused native resize semantics: detail/editor focus must not disappear into hidden list.
    window.set_size(tauri::LogicalSize::new(1440.0,900.0)).map_err(|e|e.to_string())?;
    appearance(app,fx,checks,"light",false).await?;
    step(app,fx,checks,"focus server editor before native resize",json!({"name":URL_NAME}),"row(A.name).click();await frame();button('Edit',detail()).click();await until(()=>field('Name')?.value===A.name);field('Name').focus();return {ok:document.activeElement===field('Name')};").await?;
    window.set_size(tauri::LogicalSize::new(520.0,900.0)).map_err(|e|e.to_string())?;
    step(app,fx,checks,"native shrink retains visible editor and valid focus",json!({}),"return {ok:!!await until(()=>document.querySelector('.mcp-columns.is-compact')&&!detail().hidden)&&!!form()&&!document.activeElement.closest('[hidden],[inert]')};").await?;
    screenshot(app,fx,"resize-wide-editor-to-narrow").await?;
    step(app,fx,checks,"compact back returns focus to server search",json!({}),"document.querySelector('.mcp-back-list').click();return {ok:!!await until(()=>document.activeElement===document.querySelector('.mcp-server-list input'))};").await?;
    step(app,fx,checks,"compact resume restores editor detail focus",json!({}),"button('Return to configuration',page()).click();return {ok:!!await until(()=>!detail().hidden&&document.activeElement===detail())};").await?;
    window.set_size(tauri::LogicalSize::new(1440.0,900.0)).map_err(|e|e.to_string())?;
    step(app,fx,checks,"native grow restores both columns with original editor",json!({}),"return {ok:!!await until(()=>!document.querySelector('.mcp-columns.is-compact'))&&!!form()&&field('Name').value.startsWith('MCPLongURL')};").await?;
    step(app,fx,checks,"close resize editor",json!({}),"button('Cancel',form()).click();return {ok:!!await until(()=>!form())};").await?;
    // Exercise the existing shell surface menu; MCP has no page-specific menu.
    step(app,fx,checks,"focus existing work-surface menu opener",json!({}),"const b=document.querySelector('button[aria-label=\"Open work surfaces\"]');window.__mcpMenu=b;b.focus();return {ok:document.activeElement===b};").await?;
    ui_keys::key(app,Key::Enter).await;
    step(app,fx,checks,"native Enter opens surface menu at current MCP entry",json!({}),"return {ok:!!await until(()=>document.querySelector('.workspace-tab-list [aria-current=true]')===document.activeElement)};").await?;
    ui_keys::key(app,Key::Home).await;
    step(app,fx,checks,"native Home navigates surface menu",json!({}),"return {ok:document.activeElement===document.querySelector('.workspace-tab-list [data-tab-choice]')};").await?;
    ui_keys::key(app,Key::Escape).await;
    step(app,fx,checks,"native Escape returns surface menu focus to opener",json!({}),"return {ok:!!await until(()=>!document.querySelector('.workspace-tab-list')&&document.activeElement===window.__mcpMenu)};").await?;
    window.set_size(tauri::LogicalSize::new(800.0,900.0)).map_err(|e|e.to_string())?;
    appearance(app,fx,checks,"dark",true).await?;
    step(app,fx,checks,"prepare long modal for independent scroll and focus checks",json!({}),"if(document.querySelector('.mcp-back-list'))document.querySelector('.mcp-back-list').click();await frame();row('Official SDK Echo').click();await until(()=>text(detail()).includes('echo'));button('Inspect / call',detail()).click();await until(()=>dialog()?.querySelector('textarea'));set(dialog().querySelector('textarea'),JSON.stringify({text:'LongJSONValue'.repeat(150)}));await frame();button('Review tool call',dialog()).click();return {ok:!!await until(()=>button('Confirm and call tool',dialog()))};").await?;
    step(app,fx,checks,"modal and JSON scroll independently while footer stays reachable",json!({}),"const d=dialog(),pre=d.querySelector('section[aria-label] pre');const ports=all('*',d).filter(e=>e.scrollHeight>e.clientHeight+4&&/auto|scroll/.test(getComputedStyle(e).overflowY));const outer=ports.find(e=>e!==pre&&e.contains(pre));if(!outer||!pre)return {ok:false,ports:ports.map(e=>e.className)};const before=page().scrollTop;outer.scrollTop=outer.scrollHeight;pre.scrollTop=pre.scrollHeight;await frame();const b=button('Confirm and call tool',d),r=rect(b);return {ok:outer.scrollTop>0&&pre.scrollTop>0&&page().scrollTop===before&&r.y>=0&&r.bottom<=innerHeight,outer:{top:outer.scrollTop,height:outer.clientHeight,total:outer.scrollHeight},json:{top:pre.scrollTop,height:pre.clientHeight,total:pre.scrollHeight},pageTop:before,footer:r};").await?;
    for _ in 0..8 {
        ui_keys::key(app,Key::Tab).await;
        step(app,fx,checks,"native modal focus has visible ring and unclipped control",json!({}),"await frame();const e=document.activeElement,r=rect(e),s=getComputedStyle(e);const clips=all('*',dialog()).filter(p=>p!==e&&p.contains(e)&&/auto|scroll|hidden/.test(getComputedStyle(p).overflowY)).map(rect);return {ok:dialog().contains(e)&&e.matches(':focus-visible')&&(parseFloat(s.outlineWidth)>0||s.boxShadow!=='none')&&r.x>=0&&r.right<=innerWidth&&r.y>=0&&r.bottom<=innerHeight&&clips.every(c=>r.y>=c.y-2&&r.bottom<=c.bottom+2),target:e.tagName,rect:r,outline:s.outline,shadow:s.boxShadow,clips};").await?;
    }
    screenshot(app,fx,"native-scroll-focus-dark-relaxed115-800").await?;
    ui_keys::key(app,Key::Escape).await;
    step(app,fx,checks,"supplemental modal closes without dispatch",json!({}),"return {ok:!!await until(()=>!dialog())};").await?;
    Ok(())
}
async fn journey(app: &AppHandle, fx: &Fixture, checks: &mut Checks) -> Journey<()> {
    seed(fx).await?;
    // Navigation probes await animation frames; background WKWebView can suspend them.
    if !ui_keys::prepare(app).await { return Err("native foreground preflight before page probes failed".into()); }
    // The shell recovery consumer reads the shared catalog before MCP mounts.
    // Reject before reload so this is a genuinely empty first-read state.
    app.state::<Control>().reject("list_mcp_servers", true);
    // Reload before first MCP mount so production locale/bootstrap read the isolated English preference.
    app.get_webview_window(MAIN).ok_or("main absent")?.eval("location.reload()").map_err(|e|e.to_string())?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    step(app,fx,checks,"production shell loads English from isolated host",json!({}),"return {ok:!!await until(()=>document.documentElement.dataset.locale==='en-US'&&document.querySelector('.workspace-tabbar'))};").await?;
    let catalog=ipc(app,MAIN,"read_workspace_app_catalog",json!({})).await;
    if !catalog.success() || !catalog.data()["entries"].as_array().is_some_and(Vec::is_empty) { return Err(format!("real isolated module catalog unavailable: {}",catalog.describe())); }
    save(fx,"shell-host-scope",&json!({"catalog":catalog.0,"registered":"production desktop/CLI/appearance/locale preferences, empty ModuleManagementService and ModuleHostBridge over this same temporary DB","account_gate":"existing smoke no-account-command fallback; SessionResolver::no_accounts uses default-user","uncovered":"Provider/chat/skills/Agent management and non-appearance settings categories are not installed or certified","window_widths":{"1440":"wide desktop","520":"stress below the public 800px minimum window; tests content response only"}}))?;
    failures(app,fx,checks).await?;
    discovery(app,fx,checks).await?;
    native(app,fx,checks).await?;
    drafts(app,fx,checks).await?;
    matrix(app,fx,checks).await?;
    Ok(())
}
pub(super) async fn run(app: AppHandle, fx: Fixture) {
    let mut checks=Checks::default();
    let execution=journey(&app,&fx,&mut checks).await;
    if let Err(error)=execution {
        checks.check("MCP page journey completes",false,&error);
        let _=save(&fx,"failure",&json!({"error":error,"dom":dom(&app,json!({}),"return snapshot();").await}));
        let capture=capture(&app,&fx,"failure").await;
        checks.check("failure native screenshot retained",capture.is_ok(),format!("{capture:?}"));
    }
    let assets=dom(&app,json!({}),"return await assets();").await;
    let _=save(&fx,"asset-identity",&assets);
    checks.check("shipped dist asset identity recorded",assets["ok"]==true&&assets["value"]["ok"]==true,&assets);
    let control=app.state::<Control>();
    let _=save(&fx,"ipc-timeline",&json!(*control.events.lock().unwrap()));
    let _=save(&fx,"ipc-counts",&json!(*control.counts.lock().unwrap()));
    let shutdown=fx.manager.shutdown().await;
    checks.check("owned MCP sessions shut down",shutdown.is_ok(),format!("{shutdown:?}"));
    let child={fx.sdk_child.lock().unwrap().take()};
    if let Some(mut child)=child { let _=child.kill().await; }
    let _=save(&fx,"result",&json!({"passed":checks.passed,"failed":checks.failed,"pid":std::process::id(),"executable":std::env::current_exe().ok(),"source":"production dist in visible macOS WKWebView; real isolated SQLite/McpManager/official SDK and filesystem","fault_evidence":"smoke IPC rejects read before production dispatch; one real update gated before dispatch; one real refresh published before deliberate receipt loss","layout_fixture":"SYNTHETIC_LAYOUT_ONLY long connection event; disabled example.invalid URL is config-only; no network request","input":"synthesized native NSEvent keyDown/keyUp via NSApplication; known focus/text and pointer shortcuts use DOM","limits":["not physical typing or real mouse hit testing","not VoiceOver or other platforms","in-memory credential vault, not OS keychain","not real disk/network failure","new B editor is prohibited until A save settles; after-settlement B draft is checked, no fabricated overlapping editor"]}));
    println!("SMOKE RESULT: {} passed, {} failed; retained MCP page evidence {}",checks.passed,checks.failed.len(),fx.root.display());
    std::process::exit(if checks.failed.is_empty(){0}else{1});
}
