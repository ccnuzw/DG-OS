//! D136 lifecycle evidence from an isolated production WebView host.
//! Loopback responses and local MCP are protocol fixtures, never model evidence.
use super::{eval, goto, ipc, poll_operation, wait_load, wait_until, write_package_with, Checks, LoadEvents, Pending, WATCHDOG};
use crate::{commands, module_host::{self, ChannelTransport, ModuleHostBridge, RoutePublisher}, session::{HostSessionPort, SessionResolver}};
use dgos_application::{AccountPorts, AccountService, HostCapabilityService, ModuleCapabilityDispatchService, ModuleCrashService, ModuleManagementService, ModuleOperationManager, ModuleReleaseService};
use dgos_db::{sea_orm::{ConnectionTrait, DatabaseConnection, Statement}, sea_orm_migration::MigratorTrait, ModuleCrashRepository, ModulePermission, ModulePermissionRepository};
use dgos_modules::{LocalModuleSources, ModulePublisher, ModuleRuntimeSupervisor, PageModuleLoader, PageLoaderConfig, PinnedModuleAdmission, ModuleCallRouter};
use dgos_providers::{ProviderManager, test_support::{MemoryCredentialVault, seed_provider}};
use serde_json::{json, Value};
use std::{fs, io::Write, path::{Path, PathBuf}, process::{Command, Stdio}, sync::{Arc, Mutex}, time::Duration};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener, sync::broadcast};

const USER: &str = "default-user";
const MODULE: &str = "lifecycle-consumer";
const SOURCE: &str = r#"export default function(host) {
 const React = host.React;
 globalThis.lifecycleHost = host;
 return { Component: () => React.createElement('div', {'data-testid':'lifecycle-consumer'}, 'lifecycle'),
   async onUnload() { globalThis.lifecycleUnloadEntered = true; await new Promise(r => setTimeout(r, 1500)); throw new Error('local lifecycle unload failure'); } };
}"#;

struct NoMcpVault;
#[async_trait::async_trait]
impl dgos_mcp::McpCredentialVault for NoMcpVault {
    async fn store(&self, _: &dgos_mcp::CredentialScope, _: dgos_mcp::SecretToken) -> dgos_mcp::Result<()> { Err(dgos_mcp::McpError::CredentialUnavailable) }
    async fn load(&self, _: &dgos_mcp::CredentialScope) -> dgos_mcp::Result<dgos_mcp::SecretToken> { Err(dgos_mcp::McpError::CredentialUnavailable) }
    async fn remove(&self, _: &dgos_mcp::CredentialScope) -> dgos_mcp::Result<()> { Err(dgos_mcp::McpError::CredentialUnavailable) }
}

/// Imports and execution use the production loader; only its stop boundary fails.
struct SlowStop { loader: PageModuleLoader, root: PathBuf }
#[async_trait::async_trait]
impl dgos_modules::ModuleLoader for SlowStop {
    fn executes_module_code(&self) -> bool { true }
    async fn smoke(&self, artifact: &dgos_modules::VerifiedModuleArtifact) -> Result<(),dgos_modules::ModuleLoadError> { self.loader.smoke(artifact).await }
    async fn start(&self, id: &str, artifact: &dgos_modules::VerifiedModuleArtifact) -> Result<(),dgos_modules::ModuleLoadError> { self.loader.start(id,artifact).await }
    async fn stop(&self, id: &str, artifact: &dgos_modules::VerifiedModuleArtifact) -> Result<(),dgos_modules::ModuleLoadError> {
        save(&self.root,"module-stop-entered.json",&json!({"runtime":id,"host_pid":std::process::id(),"fault":"delayed failing loader stop"}));
        tokio::time::sleep(Duration::from_secs(4)).await;
        let actual = self.loader.stop(id,artifact).await;
        save(&self.root,"module-stop-returned.json",&json!({"production_loader_result":format!("{actual:?}")}));
        Err(dgos_modules::ModuleLoadError::Rejected("isolated lifecycle stop failure".into()))
    }
}

struct World {
    root: PathBuf, phase: String, db: DatabaseConnection, manager: ModuleOperationManager,
    router: Arc<ModuleCallRouter>, routes: Arc<RoutePublisher>, management: ModuleManagementService,
    dispatch: ModuleCapabilityDispatchService, maintenance: dgos_db::MaintenanceGate,
    providers: ProviderManager, vault: Arc<MemoryCredentialVault>, ai: dgos_application::AiService,
    confirmations: Arc<dgos_application::ModuleMcpConfirmations>, mcp: Arc<dgos_mcp::McpManager>,
    agents: dgos_agents::AgentManager, cli: dgos_agents::capabilities::CliProviders,
    provider_id: String, resolver: Arc<SessionResolver>,
}

fn save(root: &Path, name: &str, value: &Value) {
    let temporary=root.join(format!(".{name}.tmp"));
    fs::write(&temporary, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    fs::rename(temporary,root.join(name)).unwrap();
}
fn ps(root: &Path, label: &str) -> String {
    let mut ids = vec![std::process::id().to_string()];
    for phase in ["account","shutdown","absent","crash"] {
        let log = fs::read_to_string(root.join(format!("process-{phase}.log"))).unwrap_or_default();
        for line in log.lines().filter(|line|line.contains("stage=\"spawned\"")) {
            if let Some(pid) = line.split("child_pid=Some(").nth(1).and_then(|v|v.split(')').next()).filter(|v|v.parse::<u32>().is_ok()) {ids.push(pid.into());}
        }
    }
    if let Ok(wire) = fs::read_to_string(root.join("mcp-wire.jsonl")) {
        for row in wire.lines().filter_map(|line|serde_json::from_str::<Value>(line).ok()).filter(|row|row["kind"]=="spawn") {
            if let Some(pid) = row["pid"].as_u64() { ids.push(pid.to_string()); }
        }
    }
    ids.sort(); ids.dedup();
    let output = Command::new("/bin/ps").args(["-p",&ids.join(","),"-o","pid=,ppid=,pgid=,stat=,comm=,lstart="]).output().expect("owned identity snapshot");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    fs::write(root.join(format!("ps-{label}.txt")), &text).unwrap(); text
}

fn provider_hits(root: &Path) -> usize { fs::read_to_string(root.join("provider-wire.jsonl")).unwrap_or_default().lines().count() }

fn agent_spawns(root: &Path, phase: &str, snapshot: &str) -> Vec<Value> {
    let log = fs::read_to_string(root.join(format!("process-{phase}.log"))).unwrap_or_default();
    log.lines().filter(|line|line.contains("stage=\"spawned\"")).filter_map(|line| {
        let pid = line.split("child_pid=Some(").nth(1)?.split(')').next()?.parse::<u32>().ok()?;
        let role = if line.contains("operation=\"version_probe\"") { "version_probe" } else if line.contains("tool=\"claude\"") { "claude_bridge" } else { "codex_app_server" };
        let row = snapshot.lines().find(|row|row.split_whitespace().next()==Some(pid.to_string().as_str()));
        let columns: Vec<_> = row.unwrap_or_default().split_whitespace().collect();
        Some(json!({"pid":pid,"role":role,"spawn_diagnostic":line,"ppid":columns.get(1),"pgid":columns.get(2),"state":columns.get(3),"comm":columns.get(4),"observed_in_ps":row.is_some()}))
    }).collect()
}

// Evidence only: no signals or wait calls. Birth times come from the native
// microsecond-resolution process identity, never a command-line heuristic.
#[cfg(target_os = "macos")]
mod process_tree {
    use super::*;
    use std::{collections::{BTreeMap, BTreeSet}, ffi::{c_int, c_void}};

    #[repr(C)]
    #[derive(Default)]
    struct BsdInfo {
        flags: u32, status: u32, xstatus: u32, pid: u32, ppid: u32,
        uid: u32, gid: u32, ruid: u32, rgid: u32, svuid: u32, svgid: u32, reserved: u32,
        comm: [u8; 16], name: [u8; 32], nfiles: u32, pgid: u32, jobc: u32,
        tty: u32, tty_pgid: u32, nice: i32, start_sec: u64, start_usec: u64,
    }
    #[link(name = "proc")]
    extern "C" {
        fn proc_listallpids(buffer: *mut c_void, size: c_int) -> c_int;
        fn proc_pidinfo(pid: c_int, flavor: c_int, arg: u64, buffer: *mut c_void, size: c_int) -> c_int;
        fn proc_pidpath(pid: c_int, buffer: *mut c_void, size: u32) -> c_int;
    }
    #[derive(Clone, serde::Serialize)]
    pub(super) struct Identity {
        pid: u32, ppid: u32, pgid: u32, state: u32,
        start_sec: u64, start_usec: u64, executable: Option<String>, executable_error: Option<String>,
        role: String, ancestry: Vec<u32>,
    }
    impl Identity {
        fn birth(&self) -> (u64,u64) { (self.start_sec,self.start_usec) }
    }
    fn info(pid: u32) -> Result<Identity,String> {
        let mut row = BsdInfo::default();
        let size = std::mem::size_of::<BsdInfo>() as c_int;
        // SAFETY: the SDK's PROC_PIDTBSDINFO (3) writes exactly proc_bsdinfo.
        let read = unsafe { proc_pidinfo(pid as c_int,3,0,(&mut row as *mut BsdInfo).cast(),size) };
        if read != size { return Err(if read == 0 && std::io::Error::last_os_error().raw_os_error()==Some(3) { "absent".into() } else { format!("proc_pidinfo bytes={read}: {}",std::io::Error::last_os_error()) }); }
        Ok(Identity {pid:row.pid,ppid:row.ppid,pgid:row.pgid,state:row.status,start_sec:row.start_sec,start_usec:row.start_usec,executable:None,executable_error:None,role:String::new(),ancestry:Vec::new()})
    }
    fn executable(row: &mut Identity) {
        let mut buffer = [0u8;4096]; // PROC_PIDPATHINFO_MAXSIZE from the SDK.
        // SAFETY: the buffer is writable for the advertised length.
        let read = unsafe { proc_pidpath(row.pid as c_int,buffer.as_mut_ptr().cast(),buffer.len() as u32) };
        if read > 0 && (read as usize) < buffer.len() {
            let end = buffer.iter().position(|b|*b==0).unwrap_or(read as usize);
            match std::str::from_utf8(&buffer[..end]) {
                Ok(path) => row.executable=Some(path.into()),
                Err(error) => row.executable_error=Some(format!("non-UTF8 executable: {error}; bytes={:?}",&buffer[..end])),
            }
        } else { row.executable_error=Some(format!("proc_pidpath bytes={read}: {}",std::io::Error::last_os_error())); }
        if !info(row.pid).is_ok_and(|current|current.birth()==row.birth()) {
            row.executable=None; row.executable_error=Some("birth identity changed during executable lookup".into());
        }
    }
    pub(super) struct Capture {
        roots: BTreeMap<u32,String>, tracked: BTreeMap<(u32,u64,u64),Identity>, samples: Vec<Value>,
    }
    impl Capture {
        pub(super) fn new(root: &Path, phase: &str) -> Self {
            let mut roots = BTreeMap::new();
            for row in agent_spawns(root,phase,"") {
                if row["role"]!="version_probe" { roots.insert(row["pid"].as_u64().unwrap() as u32,row["role"].as_str().unwrap().into()); }
            }
            Self {roots,tracked:BTreeMap::new(),samples:Vec::new()}
        }
        fn sample(&mut self, root: &Path, stage: &str, index: usize) {
            // A hard cap bounds enumeration and each phase contains only three samples.
            let mut pids = vec![0i32;65536];
            // SAFETY: the buffer is writable for its full byte length.
            let count = unsafe { proc_listallpids(pids.as_mut_ptr().cast(),(pids.len()*std::mem::size_of::<i32>()) as c_int) };
            let complete = count>0 && (count as usize)<pids.len();
            let mut rows = BTreeMap::new(); let mut lookup_errors = 0;
            for pid in pids.into_iter().take(count.max(0) as usize).filter(|pid|*pid>0) {
                match info(pid as u32) { Ok(row)=>{rows.insert(row.pid,row);}, Err(_)=>lookup_errors+=1 }
            }
            let mut selected = BTreeSet::new();
            for (&pid,role) in &self.roots {
                if let Some(row) = rows.get_mut(&pid) {
                    // First observation is accepted only while this is our direct child.
                    let owned = if self.tracked.values().any(|old|old.pid==pid) { self.tracked.contains_key(&(pid,row.start_sec,row.start_usec)) } else { row.ppid==std::process::id() && row.pgid==pid };
                    if owned { row.role=role.clone();row.ancestry=vec![std::process::id()];selected.insert(pid); }
                }
            }
            // Preserve previously observed identities even after reparenting.
            for old in self.tracked.values() {
                if let Some(row)=rows.get_mut(&old.pid).filter(|row|row.birth()==old.birth()) {row.role=old.role.clone();row.ancestry=old.ancestry.clone();selected.insert(old.pid);}
            }
            loop {
                let descendants: Vec<_> = rows.values().filter(|row|!selected.contains(&row.pid)).filter_map(|row| {
                    let parent=rows.get(&row.ppid)?;
                    (selected.contains(&parent.pid) && parent.birth()<=row.birth() && info(parent.pid).is_ok_and(|current|current.birth()==parent.birth())).then(||(row.pid,parent.role.clone(),parent.ancestry.iter().copied().chain([parent.pid]).collect::<Vec<_>>()))
                }).collect();
                if descendants.is_empty() {break;}
                for (pid,parent_role,ancestry) in descendants {let row=rows.get_mut(&pid).unwrap();row.role=format!("{parent_role}/descendant");row.ancestry=ancestry;selected.insert(pid);}
            }
            let mut observed = Vec::new();
            let native_spawns:Vec<_>=["codex","claude"].into_iter().flat_map(|name|fs::read_to_string(root.join(format!("native-mcp-{name}-wire.jsonl"))).unwrap_or_default().lines().filter_map(|line|serde_json::from_str::<Value>(line).ok()).filter(|row|row["kind"]=="spawn").collect::<Vec<_>>()).collect();
            for pid in selected {
                let row=rows.get_mut(&pid).unwrap();executable(row);
                // Fixture logging labels an already ancestry-selected child; it never
                // establishes ownership or imports the manager-owned MCP process.
                if let Some(native)=native_spawns.iter().find(|native|native["pid"].as_u64()==Some(pid as u64) && native["ppid"].as_u64()==Some(row.ppid as u64)) {
                    row.role=native["role"].as_str().unwrap_or("cli-native-mcp").into();
                }
                let key=(pid,row.start_sec,row.start_usec);
                self.tracked.entry(key).or_insert_with(||row.clone());
                // A missing executable on an earlier sample may be resolved later.
                if let Some(old)=self.tracked.get_mut(&key).filter(|old|old.executable.is_none()) {old.executable=row.executable.clone();old.executable_error=row.executable_error.clone();}
                observed.push(row.clone());
            }
            let mut comparisons=Vec::new();
            for old in self.tracked.values() {
                let current=info(old.pid);
                let comparison=match current {
                    Err(error) if error=="absent" => json!({"status":"absent","basis":"native ESRCH"}),
                    Err(error) => json!({"status":"unobserved","reason":error}),
                    Ok(mut row) => {
                        executable(&mut row);
                        if row.birth()!=old.birth() {json!({"status":"absent","basis":"PID reused with different birth identity","current":row})}
                        else if row.executable.is_none() || old.executable.is_none() {json!({"status":"unobserved","reason":"full executable identity unavailable","current":row})}
                        else if row.executable!=old.executable {json!({"status":"unobserved","reason":"same birth identity changed executable","current":row})}
                        else {json!({"status":"present","current":row})}
                    }
                };
                comparisons.push(json!({"original":old,"observation":comparison}));
            }
            let unobserved_roots:Vec<_>=self.roots.iter().filter(|(pid,_)|!self.tracked.values().any(|row|row.pid==**pid)).map(|(pid,role)|json!({"pid":pid,"role":role,"status":"unobserved","reason":"no owned birth identity captured"})).collect();
            self.samples.push(json!({"stage":stage,"sample":index,"unix_time_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),"enumeration_complete":complete,"enumeration_count":count,"metadata_lookup_errors":lookup_errors,"observed":observed,"comparisons":comparisons,"unobserved_roots":unobserved_roots}));
        }
        pub(super) async fn capture(&mut self, root: &Path, stage: &str) {
            for index in 0..3 { self.sample(root,stage,index);if index<2 {tokio::time::sleep(Duration::from_millis(150)).await;} }
            save(root,"cli-process-tree.json",&json!({"source":"macOS libproc PROC_PIDTBSDINFO + proc_pidpath","scope":"feature-driver process observations; roles distinguish local helper fixture from real CLI idle with MemoryCredentialVault","sample_limit_per_stage":3,"interval_ms":150,"pid_enumeration_cap":65536,"descendant_discovery":"observed PPID ancestry with parent birth preceding child birth; only descendants of owned transport roots","limitations":"bounded observations can miss short-lived or already reparented descendants; absence is not wait proof; no wait for grandchildren","wait_proof_for_descendants":false,"tracked_identities":self.tracked.values().collect::<Vec<_>>(),"samples":self.samples}));
        }
        pub(super) async fn fixture_smoke(root: &Path) {
            use std::os::unix::process::CommandExt;
            // Natural exit; wait is for our direct shell child only.
            let mut child=Command::new("/bin/sh").args(["-c","/bin/sleep 1 & wait"]).process_group(0).spawn().unwrap();
            let mut capture=Self {roots:BTreeMap::from([(child.id(),"owned-local-fixture".into())]),tracked:BTreeMap::new(),samples:Vec::new()};
            capture.capture(root,"fixture-live").await;
            let had_descendant=capture.tracked.values().any(|row|row.role.ends_with("/descendant") && row.executable.is_some());
            let status=child.wait().unwrap();
            capture.capture(root,"fixture-after-owned-child-wait").await;
            let absent=capture.samples.last().unwrap()["comparisons"].as_array().unwrap().iter().all(|row|row["observation"]["status"]=="absent");
            save(root,"capture-fixture-result.json",&json!({"exit_success":status.success(),"descendant_observed_with_full_identity":had_descendant,"all_tracked_birth_identities_absent":absent,"owned_direct_child_wait_completed":true,"wait_for_grandchildren":false,"native_cli_executed":false}));
            assert!(status.success() && had_descendant && absent,"local fixture process capture failed");
        }
    }
}
fn record(c: &mut Checks, evidence: &mut Vec<Value>, name: &str, ok: bool, detail: impl ToString) {
    let detail = detail.to_string(); c.check(name, ok, &detail);
    evidence.push(json!({"check":name,"ok":ok,"detail":detail}));
}
async fn count(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one(Statement::from_string(db.get_database_backend(), sql.to_owned())).await.unwrap().unwrap().try_get_by_index(0).unwrap()
}
async fn provider(listener: TcpListener, root: PathBuf) {
    loop {
        let (mut socket, _) = listener.accept().await.unwrap(); let root = root.clone();
        tauri::async_runtime::spawn(async move {
            let mut bytes = Vec::new(); let mut buffer = [0; 4096];
            let (start, length) = loop {
                let n = socket.read(&mut buffer).await.unwrap_or(0); if n == 0 { return; } bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length = head.lines().find_map(|x| x.strip_prefix("content-length:")).and_then(|x| x.trim().parse::<usize>().ok()).unwrap_or(0);
                    break (end + 4, length);
                }
            };
            while bytes.len() < start + length { let n = socket.read(&mut buffer).await.unwrap_or(0); if n == 0 { return; } bytes.extend_from_slice(&buffer[..n]); }
            let mut log = fs::OpenOptions::new().create(true).append(true).open(root.join("provider-wire.jsonl")).unwrap();
            writeln!(log,"{{\"role\":\"loopback-provider-fixture\",\"driver_pid\":{}}}",std::process::id()).unwrap();
            // Match the public tool stream while retaining the one-call/replay assertion.
            let request: Value = serde_json::from_slice(&bytes[start..start + length]).unwrap();
            let message = json!({"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"local-tool-1","type":"function","function":{"name":"dgos_tool_0","arguments":"{\"text\":\"lifecycle-controlled\",\"delay\":60000}"}}]});
            let (content_type, body) = if request["stream"] == true {
                let delta = json!({"id":"lifecycle-fixture","object":"chat.completion.chunk","model":"fixture-chat","choices":[{"index":0,"delta":message,"finish_reason":null}]});
                let terminal = json!({"id":"lifecycle-fixture","object":"chat.completion.chunk","model":"fixture-chat","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}});
                ("text/event-stream", format!("data: {delta}\n\ndata: {terminal}\n\ndata: [DONE]\n\n"))
            } else {
                ("application/json", json!({"id":"lifecycle-fixture","object":"chat.completion","model":"fixture-chat","choices":[{"index":0,"message":message,"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}).to_string())
            };
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            let _ = socket.write_all(response.as_bytes()).await; let _ = socket.shutdown().await;
        });
    }
}

async fn build(root: PathBuf, phase: String) -> (World, ModuleHostBridge) {
    let url = format!("sqlite://{}?mode=rwc", root.join("dgos.db").display());
    let db = dgos_db::establish_connection(&url).await.unwrap();
    dgos_db::migrations::Migrator::up(&db, None).await.unwrap();
    let maintenance = dgos_db::MaintenanceGate::new();
    let resolver = SessionResolver::starting();
    let endpoint = std::env::var("DGOS_LIFECYCLE_ENDPOINT").expect("driver-owned loopback provider");
    let vault = Arc::new(MemoryCredentialVault::default());
    // Restart uses a fresh synthetic provider. Persisted runs retain their original provider identity.
    let selected = seed_provider(db.clone(), &vault, dgos_types::ProviderConfig {
        name: "Lifecycle local protocol fixture".into(), provider_type: "openai_compatible".into(), base_url: endpoint,
        api_key: "local-fixture-only".into(), enabled: true, config: Some(json!({"allow_insecure_http":true,"auth_style":"bearer"})),
    }, USER).await.unwrap();
    let providers = ProviderManager::with_vault(&url, vault.clone()).await.unwrap().with_maintenance(maintenance.clone());
    providers.add_manual_model(&selected.id, USER, "fixture-chat", Some("text-generation")).await.unwrap();
    let publisher = Arc::new(ModulePublisher::new(root.join("modules"), db.clone()));
    let package = root.join("package"); write_package_with(&package, MODULE, "1.0.0", SOURCE);
    // Explicit MCP permission is part of this fixture's pinned package identity.
    let mut manifest: Value = serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap()).unwrap();
    manifest["permissions"] = json!(["providers.call","mcp.call"]);
    fs::write(package.join("manifest.json"), manifest.to_string()).unwrap();
    let digest = dgos_modules::package_integrity(&package).unwrap();
    manifest["artifact"]["sha256"] = digest.strip_prefix("v2:").unwrap().into();
    fs::write(package.join("manifest.json"), manifest.to_string()).unwrap();
    if !phase.ends_with("restart") { ModuleReleaseService::new(publisher.clone()).publish(&package).await.unwrap(); }
    let transport = Arc::new(ChannelTransport::default());
    let (loader, lost) = PageModuleLoader::new(transport.clone(), PageLoaderConfig { window_label:"main".into(), hook_timeout:Duration::from_secs(3) });
    let router = Arc::new(ModuleCallRouter::new(loader.clone()));
    let routes = Arc::new(RoutePublisher::new(router.clone(), transport.clone()));
    let supervisor = ModuleRuntimeSupervisor::new(publisher.clone(), Arc::new(SlowStop {loader:loader.clone(),root:root.clone()}))
        .with_admission(Arc::new(PinnedModuleAdmission::new([(MODULE,"1.0.0",digest.as_str())])))
        .with_ready_observer(routes.clone());
    let permissions = Arc::new(ModulePermissionRepository::new(db.clone()));
    let capabilities = HostCapabilityService::with_publisher(permissions.clone(), publisher.clone());
    let manager = ModuleOperationManager::with_supervisor(publisher, supervisor, LocalModuleSources::new(root.join("sources"))).with_maintenance(maintenance.clone());
    manager.recover().await.expect("production module manager recovery");
    manager.spawn_lost_runtime_reporter(lost).unwrap();
    let management = ModuleManagementService::new(permissions.clone(), manager.clone(), ModuleCrashService::new(ModuleCrashRepository::new(db.clone()), manager.clone()));
    let mcp = Arc::new(dgos_mcp::McpManager::with_policy_and_vault(db.clone(), Arc::new(commands::DesktopMcpPolicy(resolver.clone())), Arc::new(NoMcpVault)).with_maintenance(maintenance.clone()));
    let confirmations = Arc::new(dgos_application::ModuleMcpConfirmations::new(transport.clone()));
    let ai = dgos_application::AiService::new(capabilities.clone(), dgos_db::AiRunRepository::new(db.clone()), Arc::new(providers.clone()), Arc::new(tokio::sync::RwLock::new(dgos_skills::SkillsManager::new(db.clone(),root.join("skills")))), mcp.clone(), confirmations.clone(), maintenance.clone());
    ai.recover_interrupted().await.unwrap();
    let repository = dgos_db::DevelopmentRepository::new(db.clone());
    let cli = dgos_agents::capabilities::CliProviders::new(repository.clone(), providers.clone()).unwrap();
    let agents = dgos_agents::AgentManager::with_maintenance(root.join("agents.json"), repository, maintenance.clone(), cli.clone()); agents.load().await.unwrap();
    let dispatch = ModuleCapabilityDispatchService::new(capabilities);
    (World {root,phase,db,manager,router,routes,management,dispatch,maintenance,providers,vault,ai,confirmations:confirmations.clone(),mcp,agents,cli,provider_id:selected.id,resolver}, ModuleHostBridge::new(loader, transport).with_mcp_confirmations(confirmations))
}

// Only driver-owned file configuration; it does not isolate the macOS keychain.
fn prepare_native_mcp(w: &World, name: &str) {
    let Some(node)=std::env::var_os("DGOS_LIFECYCLE_NODE").map(PathBuf::from) else {
        save(&w.root,&format!("native-mcp-{name}-config.json"),&json!({"status":"unavailable","reason":"explicit Node executable unavailable"}));return;
    };
    assert!(node.is_absolute() && node.is_file());
    let fixture=w.root.join(format!("native-mcp-{name}.mjs"));
    let log=serde_json::to_string(&w.root.join(format!("native-mcp-{name}-wire.jsonl"))).unwrap();
    let role=serde_json::to_string(&format!("cli-native-mcp-{name}")).unwrap();
    // Empty catalog; refuses dispatch. No model turn.
    let source=r#"import fs from 'node:fs';
import readline from 'node:readline';
const log=__LOG__, role=__ROLE__;
const record=row=>fs.appendFileSync(log,JSON.stringify({role,pid:process.pid,ppid:process.ppid,...row})+'\n');
record({kind:'spawn'});
process.stdin.on('end',()=>process.exit(0));
readline.createInterface({input:process.stdin}).on('line',line=>{
 const request=JSON.parse(line);record({kind:'request',method:request.method});
 const reply=result=>process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:request.id,result})+'\n');
 if(request.method==='initialize')reply({protocolVersion:request.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:role,version:'1'}});
 else if(request.method==='tools/list')reply({tools:[]});
 else if(request.method==='ping')reply({});
 else if(request.id!==undefined)process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:request.id,error:{code:-32601,message:'Idle evidence fixture: no dispatch'}})+'\n');
});
"#;
    fs::write(&fixture,source.replace("__LOG__",&log).replace("__ROLE__",&role)).unwrap();
    let config=if name=="codex" {
        let path=w.root.join("codex/config.toml");
        let command=serde_json::to_string(&node.to_string_lossy()).unwrap();
        let args=serde_json::to_string(&vec![fixture.to_string_lossy()]).unwrap();
        fs::write(&path,format!("[mcp_servers.lifecycle_idle]\ncommand = {command}\nargs = {args}\nenabled = true\n")).unwrap();path
    } else {
        let path=w.root.join("claude/.claude.json");
        fs::write(&path,json!({"mcpServers":{"lifecycle_idle":{"type":"stdio","command":node,"args":[fixture]}}}).to_string()).unwrap();path
    };
    save(&w.root,&format!("native-mcp-{name}-config.json"),&json!({"status":"configured","config":config,"fixture":fixture,"role":format!("cli-native-mcp-{name}"),"model_turns_sent":0,"dispatch":"not attempted","keychain_isolation":false}));
}

async fn open_idle(w: &World, checks: &mut Checks, evidence: &mut Vec<Value>) -> Vec<(String,String)> {
    use dgos_agents::{Engine, SessionOptions, capabilities::{CliProviderRef, CliProviderConfiguration, SaveCliProviderInput, SessionCreate}, projects::{CreateProjectInput, SaveTaskInput}};
    let mut ids = Vec::new();
    for (name, engine, variable) in [("codex",Engine::Codex,"DGOS_LIFECYCLE_CODEX"),("claude",Engine::Claude,"DGOS_LIFECYCLE_CLAUDE")] {
        let Some(path) = std::env::var_os(variable).map(PathBuf::from) else {
            evidence.push(json!({"coverage":"real_cli_idle","engine":name,"status":"unavailable","reason":format!("{variable} explicit executable not supplied")})); continue;
        };
        assert!(path.is_absolute() && path.is_file(), "explicit executable must be an existing absolute path");
        let directory = w.root.join(format!("workspace-{name}")); fs::create_dir_all(&directory).unwrap();
        prepare_native_mcp(w,name);
        let (_, lease) = w.resolver.agent_session().unwrap();
        let project = w.agents.create_project(USER, CreateProjectInput {kind:"import".into(),name:name.into(),path:directory.to_string_lossy().into(),source:None},lease.clone()).await.unwrap();
        let task = w.agents.save_development_task(USER, SaveTaskInput {id:None,project_id:project.id.clone(),goal:"Idle lifecycle only".into(),requirements:"No model turn".into(),status:"todo".into()},lease.clone()).await.unwrap();
        let provider = if name == "claude" {
            seed_provider(w.db.clone(), &w.vault, dgos_types::ProviderConfig {name:"Claude local fixture".into(),provider_type:"anthropic".into(),base_url:"http://127.0.0.1:9".into(),api_key:"local-fixture-only".into(),enabled:true,config:Some(json!({"allow_insecure_http":true}))},USER).await.unwrap()
        } else { w.providers.get_provider(&w.provider_id,USER).await.unwrap().unwrap() };
        if name == "claude" { w.providers.add_manual_model(&provider.id,USER,"fixture-chat",Some("text-generation")).await.unwrap(); }
        let provider = w.providers.get_provider(&provider.id,USER).await.unwrap().unwrap();
        let selected = CliProviderRef {provider_id:provider.id,model_id:"fixture-chat".into(),provider_revision:provider.config_revision};
        let revision = w.cli.save(USER,SaveCliProviderInput {project_id:project.id,expected_revision:0,configuration:CliProviderConfiguration {engine,provider:selected.clone()}}).await.unwrap().revision;
        let id = uuid::Uuid::new_v4().to_string();
        let result = w.agents.open_session(USER,engine,id.clone(),SessionCreate {options:SessionOptions {workspace:directory.to_string_lossy().into(),resume_id:None,executable:Some(path.to_string_lossy().into()),model:None,trust_workspace:true},project_revision:revision,provider:selected},Some(task.id),lease).await;
        let detail = match &result { Ok(view) => serde_json::to_value(view).unwrap(), Err(error) => json!({"startup_error":error}) };
        save(&w.root,&format!("idle-{name}.json"),&json!({"executable":path,"result":detail,"model_turns_sent":0}));
        record(checks,evidence,&format!("real {name} isolated idle startup"),result.as_ref().is_ok_and(|v| v.view.status=="idle"),detail);
        if let Ok(view) = result { ids.push((id,view.generation)); }
    }
    ids
}

async fn scenario(app: AppHandle, w: World, ready: bool) {
    let mut checks = Checks::default(); let mut evidence = Vec::new();
    record(&mut checks,&mut evidence,"bundled production WebView loaded",ready,"real WebView; protocol fixtures only");
    let port = Arc::new(HostSessionPort::new(w.resolver.clone(),w.mcp.clone(),app.clone()));
    let accounts = AccountService::new(w.db.clone(),AccountPorts::new(&w.db,port.clone())); accounts.recover().await.unwrap(); app.manage(accounts);
    let restart = w.phase.ends_with("restart");
    let signed = if restart { ipc(&app,"main","sign_in",json!({"input":{"username":"lifecycle","password":"local-lifecycle-password"}})).await }
        else { ipc(&app,"main","create_account",json!({"input":{"username":"lifecycle","password":"local-lifecycle-password"}})).await };
    record(&mut checks,&mut evidence,"production account session established",signed.success(),signed.describe());
    let refreshed = goto(&app,"main","/settings").await;
    record(&mut checks,&mut evidence,"account gate reloads signed-in production shell",refreshed,"same production dist");
    let attached = wait_until(|| app.state::<ModuleHostBridge>().loader().has_attachment("main"),"module host attachment").await;
    record(&mut checks,&mut evidence,"production module host attached",attached.is_ok(),format!("{attached:?}"));
    let repository = ModulePermissionRepository::new(w.db.clone());
    for permission in [ModulePermission::ProvidersCall,ModulePermission::McpCall] { repository.grant(USER,MODULE,permission).await.unwrap(); }
    let start = ipc(&app,"main","start_module",json!({"moduleId":MODULE})).await;
    let operation = if let Some(id) = start.data()["operation_id"].as_str() { poll_operation(&app,"main",id).await } else { json!({"start":start.describe()}) };
    let loaded = eval(&app,"main","for(let i=0;i<100;i++){if(globalThis.lifecycleHost) return true; await new Promise(r=>setTimeout(r,50));} return false;").await;
    record(&mut checks,&mut evidence,"pinned module loaded by bundled module loader",loaded["value"]==true && operation["state"]=="succeeded",operation);
    if loaded["value"]!=true || !checks.failed.is_empty() {
        crate::lifecycle::shutdown(&app).await;
        save(&w.root,&format!("result-{}.json",w.phase),&json!({"phase":w.phase,"passed":checks.passed,"failed":checks.failed,"checks":evidence,"stopped_at":"module startup","installed_app_evidence":false}));
        std::process::exit(1);
    }
    let _ = wait_until(|| !w.routes.issued().is_empty(),"ready route").await;
    let route = w.routes.issued().first().cloned();
    if restart {
        let previous: Value = serde_json::from_slice(&fs::read(w.root.join("first.json")).unwrap()).unwrap();
        let receipt = eval(&app,"main",&format!("return await lifecycleHost.ai.getRun({});",previous["run_id"])).await;
        let queried = eval(&app,"main","return await lifecycleHost.ai.listRuns();").await;
        record(&mut checks,&mut evidence,"separate host preserves interrupted unknown receipt",receipt["value"]["status"]=="interrupted" && receipt["value"]["result_unknown"]==true,receipt);
        record(&mut checks,&mut evidence,"restart uses a different host process",previous["host_pid"].as_u64()!=Some(std::process::id() as u64),format!("old={} new={}",previous["host_pid"],std::process::id()));
        let refused = ipc(&app,"main","module_list_ai_providers",json!({"attachmentId":previous["attachment"],"route":previous["route"],"callId":"previous-process-route"})).await;
        record(&mut checks,&mut evidence,"previous process route refused",!refused.success(),refused.describe());
        tokio::time::sleep(Duration::from_millis(750)).await;
        let tool_calls = fs::read_to_string(w.root.join("mcp-wire.jsonl")).unwrap_or_default().lines().filter(|line|line.contains("tools/call")).count();
        record(&mut checks,&mut evidence,"recovery and receipt lookup do not replay external work",provider_hits(&w.root)==previous["provider_hits"].as_u64().unwrap_or(u64::MAX) as usize && tool_calls==1 && count(&w.db,"SELECT COUNT(*) FROM mcp_usage_logs WHERE caller='module'").await==previous["tool_usage"].as_i64().unwrap_or(-1),queried);
        crate::lifecycle::shutdown(&app).await;
    } else {
        let sessions = if matches!(w.phase.as_str(),"absent"|"crash") { Vec::new() } else { open_idle(&w,&mut checks,&mut evidence).await };
        #[cfg(target_os = "macos")]
        let mut process_capture = process_tree::Capture::new(&w.root,&w.phase);
        #[cfg(target_os = "macos")]
        process_capture.capture(&w.root,"real-cli-idle").await;
        let (old_session, old_lease) = w.resolver.agent_session().unwrap();
        // Instrument the existing local protocol fixture at its dispatch boundary.
        let fixture = include_str!("../../../crates/dgos-mcp/tests/fixtures/mcp-server.mjs");
        let log_path = serde_json::to_string(&w.root.join("mcp-wire.jsonl")).unwrap();
        let fixture = format!("import fs from 'node:fs';\nprocess.stdin.on('end',()=>process.exit(0));\nfs.appendFileSync({log_path},JSON.stringify({{kind:'spawn',pid:process.pid,ppid:process.ppid,role:'local-mcp-fixture'}})+'\\n');\n{}",fixture.replace("const request = JSON.parse(line);",&format!("const request = JSON.parse(line); fs.appendFileSync({log_path},JSON.stringify({{kind:'request',method:request.method,pid:process.pid}})+'\\n');")));
        fs::write(w.root.join("mcp-fixture.mjs"),fixture).unwrap();
        let node = dgos_agents::session::resolve_executable("node",std::env::var("DGOS_LIFECYCLE_NODE").ok().as_deref()).unwrap();
        let server = w.mcp.create_server(USER,dgos_mcp::CreateMcpServerRequest {name:"Lifecycle local MCP".into(),transport:dgos_mcp::TransportKind::Stdio,stdio:Some(dgos_mcp::StdioConfig {executable:node.to_string_lossy().into(),args:vec![w.root.join("mcp-fixture.mjs").to_string_lossy().into(),"cancellation-observable".into()],cwd:Some(w.root.to_string_lossy().into())}),http:None,enabled:Some(true),auto_connect:Some(false)}).await.unwrap();
        w.mcp.connect(USER,&server.id).await.unwrap();
        let launched = eval(&app,"main",&format!(r#"const host = globalThis.lifecycleHost; const ps = await host.ai.listProviders(); const p = ps.find(p=>p.provider_id==={:?}); const tools = await host.ai.listTools(); const tool = tools.find(t=>t.server_id==={:?} && t.tool_name==='echo'); if(!p||!tool) return {{missing:true,ps,tools}};
            globalThis.lifecycleEvents=[]; globalThis.lifecyclePromise = host.ai.run({{provider_id:p.provider_id,model_id:'fixture-chat',provider_revision:p.config_revision,prompt:'local lifecycle tool proposal',tools:[{{server_id:tool.server_id,tool_name:tool.tool_name,schema_hash:tool.schema_hash}}]}},e=>lifecycleEvents.push(e)).then(v=>{{globalThis.lifecycleTerminal=v;return v;}},e=>{{globalThis.lifecycleError=String(e);return null;}}); return true;"#,w.provider_id,server.id)).await;
        record(&mut checks,&mut evidence,"public module host.ai run launched",launched["value"]==true,launched);
        let approved = eval(&app,"main",r#"for(let i=0;i<150;i++){const dialog=[...document.querySelectorAll('[role=dialog]')].find(d=>d.querySelector('pre')?.textContent.includes('lifecycle-controlled')); if(dialog){const b=[...dialog.querySelectorAll('button')].find(b=>/^(Approve|批准)$/.test(b.textContent));if(b){b.click();return true;}} await new Promise(r=>setTimeout(r,50));} return false;"#).await;
        record(&mut checks,&mut evidence,"local tool approved through production confirmation UI",approved["value"]==true,approved);
        let dispatched = wait_until(|| fs::read_to_string(w.root.join("mcp-wire.jsonl")).unwrap_or_default().contains("tools/call"),"local tool dispatch").await;
        record(&mut checks,&mut evidence,"MCP fixture observed actual tools/call before lifecycle end",dispatched.is_ok(),format!("{dispatched:?}"));
        let started = eval(&app,"main","return lifecycleEvents.find(e=>e.kind==='started') || null;").await;
        #[cfg(target_os = "macos")]
        process_capture.capture(&w.root,"before-production-cleanup").await;
        let before = ps(&w.root,"before-end");
        let identities = agent_spawns(&w.root,&w.phase,&before);
        save(&w.root,"agent-spawn-identities.json",&json!(identities));
        record(&mut checks,&mut evidence,"every opened real CLI has a recorded transport spawn",identities.iter().filter(|row|row["role"]!="version_probe").count()==sessions.len(),format!("sessions={} identities={}",sessions.len(),identities.len()));
        for identity in identities.iter().filter(|row|row["role"]!="version_probe") {
            let pid = identity["pid"].as_u64().unwrap().to_string();
            record(&mut checks,&mut evidence,"real CLI transport has exact dedicated process group",identity["observed_in_ps"]==true && identity["pgid"]==pid,identity);
        }
        let tool_spawn = fs::read_to_string(w.root.join("mcp-wire.jsonl")).unwrap().lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).find(|row|row["kind"]=="spawn").unwrap();
        let tool_pid = tool_spawn["pid"].as_u64().unwrap();
        record(&mut checks,&mut evidence,"local MCP exact spawn identity observed",before.lines().any(|line|line.split_whitespace().next()==Some(tool_pid.to_string().as_str())),tool_spawn);
        save(&w.root,"mcp-spawn-identity.json",&json!({"pid":tool_pid,"role":"local-mcp-fixture","ps_row":before.lines().find(|line|line.split_whitespace().next()==Some(tool_pid.to_string().as_str()))}));
        record(&mut checks,&mut evidence,"exactly one controlled Provider request reached the fixture",provider_hits(&w.root)==1,provider_hits(&w.root));
        if w.phase=="crash" {
            let (runtime,attachment,route) = route.clone().unwrap_or_default();
            save(&w.root,"first.json",&json!({"host_pid":std::process::id(),"run_id":started["value"]["run_id"],"runtime":runtime,"attachment":attachment,"route":route,"tool_usage":count(&w.db,"SELECT COUNT(*) FROM mcp_usage_logs WHERE caller='module'").await,"provider_hits":provider_hits(&w.root),"tool_pid":tool_pid,"crash_ready":true}));
            save(&w.root,"crash-ready.json",&json!({"host_pid":std::process::id(),"tool_pid":tool_pid,"checks":evidence,"failed":checks.failed}));
            loop {tokio::time::sleep(Duration::from_secs(1)).await;}
        }
        let began = std::time::Instant::now();
        let end = async {
            if w.phase=="account" {
                let ended = ipc(&app,"main","sign_out",json!({})).await;
                (ended.success(),ended.describe())
            } else { crate::lifecycle::shutdown(&app).await; (true,"shared production shutdown returned".into()) }
        };
        let independent = async {
            let entered = wait_until(|| w.root.join("module-stop-entered.json").exists(),"delayed module stop entry").await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            let receipt = dgos_db::AiRunRepository::new(w.db.clone()).list(USER,MODULE,dgos_types::AiRunListInput::default()).await.unwrap();
            let child_absent = !ps(&w.root,"during-delayed-stop").lines().any(|line|line.split_whitespace().next()==Some(tool_pid.to_string().as_str()));
            let module_pending = !w.root.join("module-stop-returned.json").exists();
            json!({"entered":entered.is_ok(),"module_pending":module_pending,"mcp_child_absent":child_absent,"ai_interrupted_unknown":receipt.items.iter().any(|r|r.status==dgos_types::AiRunStatus::Interrupted && r.result_unknown)})
        };
        let ((ended,detail),independent) = tokio::join!(end,independent);
        #[cfg(target_os = "macos")]
        process_capture.capture(&w.root,"after-production-cleanup").await;
        record(&mut checks,&mut evidence,"production lifecycle end completes",ended,detail);
        record(&mut checks,&mut evidence,"MCP and AI settle while module cleanup remains delayed",independent["entered"]==true && independent["module_pending"]==true && independent["mcp_child_absent"]==true && independent["ai_interrupted_unknown"]==true,independent);
        record(&mut checks,&mut evidence,"delayed module cleanup fault actually returned",w.root.join("module-stop-returned.json").exists(),"feature-gated loader stop failure; production module manager still invoked");
        let elapsed = began.elapsed().as_millis();
        record(&mut checks,&mut evidence,"old account lease revoked",old_lease.is_cancelled(),format!("elapsed_ms={elapsed}; owner={}",old_session.user_id()));
        record(&mut checks,&mut evidence,"old agent action admission refused",w.agents.admit_native_extensions(&old_lease).await.is_err(),"old lease checked by real AgentManager");
        let refused = eval(&app,"main","try {await lifecycleHost.ai.listProviders();return {allowed:true};}catch(e){return {allowed:false,code:e.code};}").await;
        record(&mut checks,&mut evidence,"old module closure refused",refused["value"]["allowed"]==false,refused);
        for (id,generation) in sessions {
            record(&mut checks,&mut evidence,"old real CLI action refused",w.agents.respond(USER,&id,&generation,uuid::Uuid::new_v4().to_string(),json!({"behavior":"deny"})).await.is_err(),&id);
            let repository = dgos_db::DevelopmentRepository::new(w.db.clone());
            let binding = repository.read_cli_session_configuration(USER,&id).await.unwrap().binding.unwrap();
            record(&mut checks,&mut evidence,"CLI cleanup records owned child wait proof",binding.cleanup_proof.as_deref()==Some("owned_exit"),serde_json::to_value(binding).unwrap());
        }
        let run_id = started["value"]["run_id"].as_str().unwrap_or("");
        let receipts = dgos_db::AiRunRepository::new(w.db.clone()).list(USER,MODULE,dgos_types::AiRunListInput::default()).await.unwrap();
        let receipt = receipts.items.iter().find(|r|r.run_id==run_id);
        record(&mut checks,&mut evidence,"actual in-flight AI receipt persists interrupted unknown",receipt.is_some_and(|r|r.status==dgos_types::AiRunStatus::Interrupted && r.result_unknown),serde_json::to_value(&receipts).unwrap());
        let after = ps(&w.root,"after-end");
        for name in ["codex","claude"] {
            let wire=fs::read_to_string(w.root.join(format!("native-mcp-{name}-wire.jsonl"))).unwrap_or_default();
            let rows:Vec<_>=wire.lines().filter_map(|line|serde_json::from_str::<Value>(line).ok()).collect();
            let spawned=rows.iter().any(|row|row["kind"]=="spawn");
            save(&w.root,&format!("native-mcp-{name}-observation.json"),&json!({"engine":name,"status":if spawned {"spawn_observed"} else {"unobserved_at_idle"},"reason":if spawned {"owned fixture wrote spawn record; consult cli-process-tree.json for ancestry and birth identity"} else {"CLI did not launch fixture during bounded idle observation, native config not loaded, unsupported, or CLI unavailable"},"initialize_observed":rows.iter().any(|row|row["method"]=="initialize"),"tools_list_observed":rows.iter().any(|row|row["method"]=="tools/list"),"tools_call_observed":rows.iter().any(|row|row["method"]=="tools/call"),"dispatch_attempted_by_driver":false,"model_turns_sent":0,"manager_mcp_fixture":false,"wait_proof_for_descendants":false}));
        }
        record(&mut checks,&mut evidence,"ps no longer observes exact MCP child after manager cleanup",!after.lines().any(|line|line.split_whitespace().next()==Some(tool_pid.to_string().as_str())),format!("pid={tool_pid}; ps absence is observation, not wait proof"));
        for identity in identities.iter().filter(|row|row["role"]!="version_probe") {
            let pid = identity["pid"].as_u64().unwrap().to_string();
            record(&mut checks,&mut evidence,"ps no longer observes exact real CLI transport child",!after.lines().any(|row|row.split_whitespace().next()==Some(pid.as_str())),identity);
        }
        let (runtime,attachment,route) = route.unwrap_or_default();
        let refused = ipc(&app,"main","module_list_ai_providers",json!({"attachmentId":attachment,"route":route,"callId":"after-lifecycle-end"})).await;
        record(&mut checks,&mut evidence,"captured old route refused",!refused.success(),refused.describe());
        save(&w.root,"first.json",&json!({"host_pid":std::process::id(),"run_id":run_id,"runtime":runtime,"attachment":attachment,"route":route,"tool_usage":count(&w.db,"SELECT COUNT(*) FROM mcp_usage_logs WHERE caller='module'").await,"provider_hits":provider_hits(&w.root),"elapsed_ms":elapsed,"real_model_turns":0}));
        // Account end leaves the process alive; the same production shutdown still drains remaining services.
        if w.phase=="account" { crate::lifecycle::shutdown(&app).await; }
    }
    save(&w.root,&format!("result-{}.json",w.phase),&json!({"host_pid":std::process::id(),"phase":w.phase,"passed":checks.passed,"failed":checks.failed,"checks":evidence,"provider_fixture_hits":provider_hits(&w.root),"installed_app_evidence":false,"real_model_turns":0}));
    println!("LIFECYCLE ROOT: {}",w.root.display());
    std::io::stdout().flush().ok(); std::process::exit(if checks.failed.is_empty() {0}else{1});
}

/// Entry point used only by the feature-gated standalone smoke executable.
pub fn run_lifecycle() {
    let root = PathBuf::from(std::env::var_os("DGOS_LIFECYCLE_ROOT").expect("isolated lifecycle root"));
    let phase = std::env::var("DGOS_LIFECYCLE_PHASE").expect("lifecycle child phase");
    // The driver installs these before this process (and all runtime threads) starts.
    for key in ["HOME","CODEX_HOME","CLAUDE_CONFIG_DIR"] { assert!(PathBuf::from(std::env::var_os(key).unwrap()).starts_with(&root)); }
    let log = fs::OpenOptions::new().create(true).append(true).open(root.join(format!("process-{}.log",phase))).unwrap();
    tracing_subscriber::fmt().with_ansi(false).with_writer(Mutex::new(log)).with_max_level(tracing::Level::INFO).init();
    std::thread::spawn(|| {std::thread::sleep(WATCHDOG);std::process::exit(2);});
    let (w,bridge) = tauri::async_runtime::block_on(build(root,phase));
    let (sender,_) = broadcast::channel(64); let mut context = tauri::generate_context!();context.config_mut().app.windows.clear();
    let mcp_service = dgos_application::ModuleMcpService::new(w.confirmations.clone(),w.mcp.clone(),w.db.clone());
    let absent = w.phase.starts_with("absent");
    let mut builder = tauri::Builder::default().manage(Pending::default()).manage(LoadEvents(sender))
        .manage(w.resolver.clone()).manage(w.maintenance.clone()).manage(w.manager.clone()).manage(w.router.clone()).manage(bridge)
        .manage(w.dispatch.clone()).manage(w.management.clone()).manage(w.providers.clone()).manage(w.ai.clone())
        .manage(w.confirmations.clone()).manage(mcp_service).manage(commands::McpState {manager:w.mcp.clone()});
    // This real configuration omits AgentManager, chat and workbench managed state.
    // Local MCP and AI remain active and must drain independently.
    if !absent { builder = builder.manage(w.agents.clone()); }
    let slot = Mutex::new(Some(w));
    builder.on_page_load(|webview,payload| { module_host::handle_page_load(webview,payload);if matches!(payload.event(),tauri::webview::PageLoadEvent::Finished){let _=webview.app_handle().state::<LoadEvents>().0.send((webview.label().into(),payload.url().path().into()));} })
        .on_window_event(module_host::handle_window_event)
        .setup(move |app| {
            let events=app.state::<LoadEvents>().0.subscribe();
            WebviewWindowBuilder::new(app,"main",WebviewUrl::App("settings".into())).visible(true).build()?;
            let handle=app.handle().clone();let w=slot.lock().unwrap().take().unwrap();
            tauri::async_runtime::spawn(async move {
                use futures::FutureExt;
                let root=w.root.clone();let phase=w.phase.clone();let cleanup=handle.clone();
                let ready=wait_load(events,"main","/settings").await;
                let result=tokio::time::timeout(WATCHDOG-Duration::from_secs(30),std::panic::AssertUnwindSafe(scenario(handle,w,ready)).catch_unwind()).await;
                let reason=match result {Ok(Err(_))=>"scenario panicked",Err(_)=>"scenario timed out",Ok(Ok(()))=>"scenario returned without result"};
                crate::lifecycle::shutdown(&cleanup).await;
                save(&root,&format!("result-{phase}.json"),&json!({"phase":phase,"failed":[reason],"cleanup":"production shutdown awaited","installed_app_evidence":false}));
                std::process::exit(1);
            });Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::create_account,commands::sign_in,commands::sign_out,commands::get_account_state,
            commands::start_module,commands::stop_module,commands::get_module_operation,commands::get_module_runtime_status,commands::read_workspace_app_catalog,
            commands::attach_module_host,commands::report_module_host,commands::module_run_ai,commands::module_list_ai_providers,commands::module_list_ai_tools,
            commands::module_list_ai_skills,commands::module_list_ai_runs,commands::module_get_ai_run,commands::module_cancel_ai_run,commands::resolve_module_mcp_confirmation,super::smoke_report])
        .run(context).expect("lifecycle WebView host");
}

/// Separate processes are essential: page reload cannot prove host restart behavior.
pub fn drive_lifecycle() {
    let base = std::env::var_os("DGOS_LIFECYCLE_ROOT").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join(format!("dgos-lifecycle-{}",uuid::Uuid::new_v4())));
    assert!(!base.exists(),"lifecycle driver requires a fresh root");fs::create_dir_all(&base).unwrap();
    #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;fs::set_permissions(&base,fs::Permissions::from_mode(0o700)).unwrap();}
    let base = base.canonicalize().unwrap();let executable=std::env::current_exe().unwrap();let mut failures=0;
    // Safe helper-only exercise: before executable discovery, DB, WebView or CLI.
    if std::env::var_os("DGOS_LIFECYCLE_CAPTURE_FIXTURE").is_some() {
        #[cfg(target_os = "macos")]
        tauri::async_runtime::block_on(process_tree::Capture::fixture_smoke(&base));
        #[cfg(not(target_os = "macos"))]
        panic!("native capture fixture requires macOS");
        println!("LIFECYCLE CAPTURE FIXTURE ROOT: {}",base.display());return;
    }
    // Discovery only: pass absolute paths to children before replacing HOME. No CLI executes here.
    let discovered: Vec<_> = [("codex","DGOS_LIFECYCLE_CODEX"),("claude","DGOS_LIFECYCLE_CLAUDE"),("node","DGOS_LIFECYCLE_NODE")].into_iter().filter_map(|(name,key)| {
        let explicit = std::env::var(key).ok();
        dgos_agents::session::resolve_executable(name,explicit.as_deref()).ok().and_then(|path|path.canonicalize().ok()).map(|path|(key,path))
    }).collect();
    // Smoke driver only; default still executes every original scenario and restart.
    let modes = std::env::var("DGOS_LIFECYCLE_MODES").unwrap_or_else(|_|"account,shutdown,absent,crash".into());
    let modes:Vec<_>=modes.split(',').collect();
    assert!(!modes.is_empty() && modes.iter().all(|mode|matches!(*mode,"account"|"shutdown"|"absent"|"crash")),"DGOS_LIFECYCLE_MODES must be comma-separated account,shutdown,absent,crash");
    for mode in modes {
        let root=base.join(mode);fs::create_dir_all(&root).unwrap();
        for folder in ["home","codex","claude","sources"] {fs::create_dir_all(root.join(folder)).unwrap();}
        let endpoint = tauri::async_runtime::block_on(async {
            let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let endpoint=format!("http://{}/v1",listener.local_addr().unwrap());
            tauri::async_runtime::spawn(provider(listener,root.clone()));endpoint
        });
        for phase in [mode.to_owned(),format!("{mode}-restart")] {
            let output=fs::File::create(root.join(format!("stdout-{phase}.log"))).unwrap();let errors=output.try_clone().unwrap();
            let mut command=Command::new(&executable);command.arg("--lifecycle-child").env("DGOS_LIFECYCLE_ROOT",&root).env("DGOS_LIFECYCLE_PHASE",&phase)
                .env("HOME",root.join("home")).env("CODEX_HOME",root.join("codex")).env("CLAUDE_CONFIG_DIR",root.join("claude"))
                .env("XDG_CONFIG_HOME",root.join("home/.config")).stdout(Stdio::from(output)).stderr(Stdio::from(errors));
            command.env("DGOS_LIFECYCLE_ENDPOINT",&endpoint);
            for (key,path) in &discovered { command.env(key,path); }
            for (key,_) in std::env::vars_os() {let name=key.to_string_lossy();if name.starts_with("OPENAI_")||name.starts_with("ANTHROPIC_")||name.starts_with("CLAUDE_CODE_USE_")||matches!(name.as_ref(),"CODEX_API_KEY"|"AZURE_OPENAI_API_KEY"|"AWS_BEARER_TOKEN_BEDROCK"|"GOOGLE_API_KEY"|"GOOGLE_APPLICATION_CREDENTIALS"){command.env_remove(key);}}
            let mut child=command.spawn().unwrap();let pid=child.id();
            let mut crash_killed = false;
            if phase=="crash" {
                let began=std::time::Instant::now();
                while !root.join("crash-ready.json").exists() && began.elapsed()<WATCHDOG {
                    if child.try_wait().unwrap().is_some(){break;}
                    std::thread::sleep(Duration::from_millis(50));
                }
                if root.join("crash-ready.json").exists() {
                    let ready: Value=serde_json::from_slice(&fs::read(root.join("crash-ready.json")).unwrap()).unwrap();
                    assert_eq!(ready["host_pid"].as_u64(),Some(pid as u64));
                    if !ready["failed"].as_array().is_some_and(|failures|failures.is_empty()) {failures+=1;}
                    child.kill().expect("kill exact owned host while tool dispatch is active");crash_killed=true;
                } else if child.try_wait().unwrap().is_none(){child.kill().unwrap();}
            }
            let status=child.wait().unwrap();
            if crash_killed {
                let first:Value=serde_json::from_slice(&fs::read(root.join("first.json")).unwrap()).unwrap();
                let tool_pid=first["tool_pid"].as_u64().unwrap().to_string();
                let began=std::time::Instant::now();
                let mut absent=false;
                while began.elapsed()<Duration::from_secs(10) {
                    absent=!ps(&root,"after-crash").lines().any(|line|line.split_whitespace().next()==Some(tool_pid.as_str()));
                    if absent {break;} std::thread::sleep(Duration::from_millis(50));
                }
                save(&root,"crash-outcome.json",&json!({"host_pid":pid,"host_killed":true,"owned_host_wait_completed":true,"exit":status.code(),"tool_pid":tool_pid,"tool_absent_ps":absent,"provider_hits":provider_hits(&root),"wait_proof_for_tool":false}));
                if !absent {failures+=1;}
                if !absent {
                    // The exact fixture belongs to this run; compare executable + spawn time
                    // before fallback cleanup so a recycled PID is never treated as ours.
                    let identity:Value=serde_json::from_slice(&fs::read(root.join("mcp-spawn-identity.json")).unwrap()).unwrap();
                    let original=identity["ps_row"].as_str().unwrap_or("").split_whitespace().skip(4).collect::<Vec<_>>().join(" ");
                    let now=ps(&root,"before-orphan-cleanup");
                    let current=now.lines().find(|line|line.split_whitespace().next()==Some(tool_pid.as_str())).map(|line|line.split_whitespace().skip(4).collect::<Vec<_>>().join(" "));
                    if current.as_deref()==Some(original.as_str()) && !original.is_empty() {
                        Command::new("/bin/kill").args(["-KILL",&tool_pid]).status().expect("owned fixture cleanup");
                        for _ in 0..100 {if !ps(&root,"after-orphan-cleanup").lines().any(|line|line.split_whitespace().next()==Some(tool_pid.as_str())){break;}std::thread::sleep(Duration::from_millis(50));}
                    }
                }
            }
            save(&root,&format!("driver-{phase}.json"),&json!({"role":"isolated-production-webview-host","executable":executable,"pid":pid,"exit":status.code(),"wait_completed":true,"installed_app":false}));
            println!("LIFECYCLE {phase}: pid={pid} wait={status} root={}",root.display());
            if !status.success() && !crash_killed {failures+=1;}
            if !root.join("first.json").exists(){break;}
        }
    }
    println!("LIFECYCLE ROOT: {}",base.display());std::process::exit(if failures==0{0}else{1});
}
