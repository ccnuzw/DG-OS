//! Explicit trusted local execution. Environment is cleared, then safe defaults and named
//! host references are applied. References are capability requests, never persisted values.
//! Redaction recognizes known formats; it cannot promise to recognize arbitrary secrets.
use crate::AgentManager;
use dgos_db::development_repository::StoredRun;
use serde::{Deserialize,Serialize};
use sha2::{Digest,Sha256};
use std::{collections::BTreeMap, sync::{Arc,Mutex as StdMutex}, time::{Duration,SystemTime,UNIX_EPOCH}};
use tokio::sync::{Mutex,Notify};
use tokio_util::sync::CancellationToken;

const OUTPUT_LIMIT:usize=1024*1024;
fn now()->i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 }
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessInput { pub port:u16,pub path:String,pub timeout_ms:u64 }
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartProcessInput {
    pub id:String,pub task_id:String,pub kind:String,pub program:String,pub args:Vec<String>,pub cwd:String,
    #[serde(default)] pub env_refs:BTreeMap<String,String>,pub trust_workspace:bool,
    #[serde(default)] pub verification:bool,pub readiness:Option<ReadinessInput>,
    pub native_run_id:Option<String>,
}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandConfiguration {pub id:String,pub name:String,pub program:String,pub args:Vec<String>,pub cwd:String,#[serde(default)] pub env_refs:BTreeMap<String,String>}
#[derive(Serialize)]
pub struct CommandCatalog {pub commands:Vec<CommandConfiguration>,pub discovered:Vec<CommandConfiguration>}
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct ProcessSnapshot {
    pub id:String,pub project_id:String,pub task_id:String,pub kind:String,pub source:String,pub status:String,
    pub output:String,pub truncated:bool,pub exit_code:Option<i32>,pub reason:Option<String>,pub started_at:i64,pub finished_at:Option<i64>,
    pub program:String,pub args:Vec<String>,pub cwd:String,pub fingerprint:String,pub start_fingerprint:String,
    pub verification_status:String,pub verification_complete:bool,pub mutation_observed:bool,pub preview_url:Option<String>,
    pub native_run_id:Option<String>,
}
pub(crate) struct HostProcess {
    pub owner:String,pub project_id:String,pub workspace:String,
    directory_identity:String,service_coexist:bool,
    state:Mutex<ProcessSnapshot>,cancel:CancellationToken,done:Notify,
    terminal:StdMutex<Option<std::fs::File>>,
    retained_permit:StdMutex<Option<dgos_db::ActivityPermit>>,
}
fn validate_command(program:&str,args:&[String],cwd:&str,refs:&BTreeMap<String,String>)->Result<(),String> {
    if program.is_empty() || program.len()>4096 || program.contains('\0') || args.len()>256 || args.iter().map(String::len).sum::<usize>()>32768 || args.iter().any(|s|s.contains('\0')) || refs.len()>64 {return Err("invalid_command".into());}
    crate::files::relative(cwd,true)?;
    fn variable(v:&str)->bool { !v.is_empty() && v.len()<=128 && v.bytes().enumerate().all(|(i,c)|c==b'_' || c.is_ascii_alphabetic() || (i>0 && c.is_ascii_digit())) }
    if refs.iter().any(|(a,b)|!variable(a)||!variable(b)) {return Err("invalid_command".into());}
    // Known credential forms must not enter durable command metadata.
    let arguments=args.join(" ");
    if [program,arguments.as_str(),cwd].into_iter().any(|text|crate::executions::sanitize_text(text)!=text) {return Err("secret_argument_denied".into());}
    Ok(())
}
fn receipt(input:&StartProcessInput)->Result<String,String> {Ok(format!("{:x}",Sha256::digest(serde_json::to_vec(input).map_err(|_|"invalid_command")?)))}
fn process_view(run:StoredRun)->Result<ProcessSnapshot,String> {
    if run.engine!="host" {return Err("process_not_found".into());}
    let mut view:ProcessSnapshot=serde_json::from_str(run.runtime.as_deref().ok_or("storage_failed")?).map_err(|_|"storage_failed")?;
    if run.status=="unknown" {view.status="unknown".into();view.reason=run.reason;view.finished_at=run.finished_at;view.preview_url=None;view.verification_status=if view.verification_status=="not_requested" {"not_requested"} else {"unknown"}.into();}
    Ok(view)
}
impl AgentManager {
    pub(crate) async fn host_workspace_available(&self,path:&str)->Result<(),String> {
        if self.inner.host.lock().await.values().any(|p|p.workspace==path) {Err("workspace_busy".into())} else {Ok(())}
    }
    /// Only the explicitly granted, same-account/project service scope relaxes
    /// native-vs-host admission. Unknown cleanup and arbitrary Shell never do.
    pub(crate) async fn agent_host_workspace_available(&self,owner:&str,project:&dgos_db::development_repository::Project)->Result<(),String> {
        let processes:Vec<_>=self.inner.host.lock().await.values().filter(|p|p.workspace==project.path).cloned().collect();
        if processes.is_empty(){return Ok(());}
        let granted=self.inner.repository.workspace_trust(owner,project).await?.service;
        for process in processes {
            let state=process.state.lock().await;
            if !granted || !process.service_coexist || process.owner!=owner || process.project_id!=project.id || process.directory_identity!=project.directory_identity || state.kind!="service" || !matches!(state.status.as_str(),"starting"|"ready"|"running") || process.cancel.is_cancelled() {return Err("workspace_busy".into());}
        }
        Ok(())
    }
    pub(crate) async fn revoke_directory_processes(&self,owner:&str,project_id:&str,path:&str,scope:&str)->Result<(),String> {
        let processes:Vec<_>=self.inner.host.lock().await.iter().filter(|(_,p)|p.owner==owner && p.project_id==project_id && p.workspace==path).map(|(id,p)|(id.clone(),p.clone())).collect();
        let mut ids=Vec::new();
        for (id,p) in processes {let service=p.state.lock().await.kind=="service";if (scope=="service")==service {p.cancel.cancel();ids.push(id);}}
        let mut clean=true;
        for id in ids {let view=self.stop_development_process(owner,&id).await?;clean &= view.finished_at.is_some() && !matches!(view.status.as_str(),"cleanup_pending"|"unknown") && !self.inner.host.lock().await.contains_key(&id);}
        if clean {Ok(())}else{Err("cleanup_pending".into())}
    }
    /// Caller already holds admission. Saves remain permitted during host execution.
    pub(crate) async fn mark_development_mutation(&self,path:&str) {
        let processes:Vec<_>=self.inner.host.lock().await.values().filter(|p|p.workspace==path).cloned().collect();
        for process in processes {process.state.lock().await.mutation_observed=true;}
    }
    pub async fn read_development_commands(&self,owner:&str,task_id:&str)->Result<CommandCatalog,String> {
        self.ready().await?;
        let project=self.task_project(owner,task_id).await?;
        let commands=self.inner.repository.development_commands(owner,&project.id).await?.into_iter().map(|v|serde_json::from_value(v).map_err(|_|"storage_failed".into())).collect::<Result<Vec<_>,String>>()?;
        let discovered=tokio::task::spawn_blocking(move || {
            let root=crate::files::open_root(&project)?;
            let mut discovered=Vec::new();
            if let Ok(file)=crate::files::read_at(&root,"package.json") {
                if let Some(text)=file.text {if let Ok(value)=serde_json::from_str::<serde_json::Value>(&text) {if let Some(scripts)=value.get("scripts").and_then(|v|v.as_object()) {
                    for key in scripts.keys().take(100) {if key.starts_with('-') || key.len()>200 {continue;} discovered.push(CommandConfiguration{id:format!("package:{key}"),name:format!("npm run {key}"),program:"npm".into(),args:vec!["run".into(),key.clone()],cwd:String::new(),env_refs:BTreeMap::new()});}
                }}}
            }
            if crate::files::read_at(&root,"Cargo.toml").is_ok() {for action in ["test","build","check"] {discovered.push(CommandConfiguration{id:format!("cargo:{action}"),name:format!("cargo {action}"),program:"cargo".into(),args:vec![action.into()],cwd:String::new(),env_refs:BTreeMap::new()});}}
            Ok::<_,String>(discovered)
        }).await.map_err(|_|"file_access_denied")??;
        Ok(CommandCatalog{commands,discovered})
    }
    pub async fn save_development_commands(&self,owner:&str,task_id:&str,commands:Vec<CommandConfiguration>)->Result<CommandCatalog,String> {
        let _admission=self.inner.admission.lock().await;self.ready().await?;
        let project=self.task_project(owner,task_id).await?;if project.archived {return Err("project_archived".into());}
        let mut ids=std::collections::BTreeSet::new();let mut encoded=Vec::new();
        for c in &commands {validate_command(&c.program,&c.args,&c.cwd,&c.env_refs)?;if c.name.trim().is_empty() || c.name.len()>200 || crate::executions::sanitize_text(&c.name)!=c.name || !ids.insert(&c.id) {return Err("invalid_command".into());}encoded.push((c.id.clone(),serde_json::to_string(c).map_err(|_|"invalid_command")?));}
        self.inner.repository.save_development_commands(owner,&project.id,&encoded).await?;
        self.read_development_commands(owner,task_id).await
    }
    pub async fn start_development_process(&self,owner:&str,input:StartProcessInput,lease:CancellationToken)->Result<ProcessSnapshot,String> {
        let _admission=self.inner.admission.lock().await;self.ready().await?;
        if lease.is_cancelled() {return Err("cancelled".into());}
        uuid::Uuid::parse_str(&input.id).map_err(|_|"invalid_request_id")?;
        validate_command(&input.program,&input.args,&input.cwd,&input.env_refs)?;
        let request_fingerprint=receipt(&input)?;
        if let Some(old)=self.inner.repository.run(owner,&input.id).await? {
            if old.engine!="host" || old.request_fingerprint!=request_fingerprint {return Err("request_conflict".into());}return self.read_development_process(owner,&input.id).await;
        }
        if !matches!(input.kind.as_str(),"terminal"|"command"|"service") || (input.verification && input.kind!="command") || (input.kind=="service")!=input.readiness.is_some() {return Err("invalid_command".into());}
        if let Some(r)=&input.readiness {if r.port==0 || !r.path.starts_with('/') || r.path.starts_with("//") || r.path.len()>2048 || r.path.chars().any(|c|c.is_control() || c==' ' || c=='#') || !(100..=120000).contains(&r.timeout_ms) {return Err("invalid_readiness".into());}}
        let task=self.inner.repository.task(owner,&input.task_id).await?;let project=self.task_project(owner,&input.task_id).await?;
        if project.archived {return Err("project_archived".into());}
        let scope=if input.kind=="service" {"service"}else{"terminal"};
        self.require_workspace_trust(owner,&project,scope,input.trust_workspace).await?;
        let service_coexist=input.kind=="service" && self.inner.repository.workspace_trust(owner,&project).await?.service;
        if let Some(id)=&input.native_run_id {let native=self.inner.repository.run(owner,id).await?.ok_or("run_not_found")?;if native.source!="native_agent" || native.task_id!=input.task_id {return Err("run_not_found".into());}}
        self.workspace_available_with_service(&project.path,None,if service_coexist {Some((owner,project.id.as_str()))}else{None}).await?;
        if self.inner.host.lock().await.len()>=16 {return Err("process_capacity".into());}
        let permit=self.inner.gate.admit().map_err(|_|"maintenance_active")?;
        self.capture_task_baseline(owner,&input.task_id).await?;
        let (fingerprint,complete)=self.verification_snapshot(owner,&input.task_id).await?;
        crate::projects::validate_directory(&project).await?;
        if lease.is_cancelled() || self.inner.stop.is_cancelled(){return Err("session_invalid".into());}
        let state=ProcessSnapshot{id:input.id.clone(),project_id:project.id.clone(),task_id:task.id.clone(),kind:input.kind.clone(),source:format!("host_{}",input.kind),status:if input.kind=="service" {"starting"} else {"running"}.into(),output:String::new(),truncated:false,exit_code:None,reason:None,started_at:now(),finished_at:None,program:input.program.clone(),args:input.args.clone(),cwd:input.cwd.clone(),fingerprint:fingerprint.clone(),start_fingerprint:fingerprint.clone(),verification_status:if input.verification {"running"} else {"not_requested"}.into(),verification_complete:complete,mutation_observed:false,preview_url:None,native_run_id:input.native_run_id.clone()};
        let run=StoredRun{id:input.id.clone(),owner:owner.into(),session_id:None,task_id:task.id,project_id:project.id.clone(),engine:"host".into(),source:state.source.clone(),status:"running".into(),started_at:state.started_at,finished_at:None,summary:String::new(),reason:None,events:"[]".into(),fingerprint:fingerprint.clone(),start_fingerprint:fingerprint,truncated:false,request_fingerprint,runtime:Some(serde_json::to_string(&state).map_err(|_|"storage_failed")?)};
        // Reserve registration before the durable receipt. Once start_run commits,
        // cancellation must not strand a running row without its owned supervisor.
        let mut watchers=self.inner.watchers.lock().await;
        self.reap_watchers(&mut watchers).await;
        let mut host=self.inner.host.lock().await;
        self.inner.repository.start_run(&run).await?;
        let process=Arc::new(HostProcess{owner:owner.into(),project_id:project.id.clone(),workspace:project.path.clone(),directory_identity:project.directory_identity.clone(),service_coexist,state:Mutex::new(state),cancel:lease.child_token(),done:Notify::new(),terminal:StdMutex::new(None),retained_permit:StdMutex::new(Some(permit))});
        host.insert(input.id.clone(),process.clone());
        let manager=self.clone();let watched=process.clone();
        let handle=tokio::spawn(async move {supervise(manager,watched,input,project,run).await});
        watchers.push(handle);
        drop(host);drop(watchers);
        let view=process.state.lock().await.clone();Ok(view)
    }
    pub async fn read_development_process(&self,owner:&str,id:&str)->Result<ProcessSnapshot,String> {
        self.ready().await?;
        let active=self.inner.host.lock().await.get(id).filter(|p|p.owner==owner).cloned();
        let mut view=if let Some(active)=active {let view=active.state.lock().await.clone();view} else {process_view(self.inner.repository.run(owner,id).await?.ok_or("process_not_found")?)?};
        if view.verification_status=="observed_stable" {match self.verification_snapshot(owner,&view.task_id).await {Ok((fp,true)) if fp==view.fingerprint=>{},Ok((_,false))=>view.verification_status="incomplete".into(),_=>view.verification_status="stale".into()}}
        Ok(view)
    }
    pub async fn list_development_processes(&self,owner:&str,task_id:&str)->Result<Vec<ProcessSnapshot>,String> {
        self.ready().await?;self.inner.repository.task(owner,task_id).await?;
        let mut result=Vec::new();for run in self.inner.repository.runs(owner,task_id).await? {if run.engine=="host" {result.push(self.read_development_process(owner,&run.id).await?);}}Ok(result)
    }
    pub async fn stop_development_process(&self,owner:&str,id:&str)->Result<ProcessSnapshot,String> {
        self.ready().await?;let process=self.inner.host.lock().await.get(id).filter(|p|p.owner==owner).cloned();
        if let Some(process)=process {let done=process.done.notified();tokio::pin!(done);done.as_mut().enable();process.cancel.cancel();if process.state.lock().await.finished_at.is_none() {let _=tokio::time::timeout(Duration::from_secs(8),done).await;}}
        self.read_development_process(owner,id).await
    }
    pub(crate) async fn stop_host_processes(&self,owner:Option<&str>)->bool {
        let processes:Vec<_>=self.inner.host.lock().await.values().filter(|p|owner.is_none_or(|o|o==p.owner)).cloned().collect();
        for p in &processes {p.cancel.cancel();}
        let deadline=tokio::time::Instant::now()+Duration::from_secs(8);
        for p in &processes {
            let done=p.done.notified();tokio::pin!(done);done.as_mut().enable();
            let registered=self.inner.host.lock().await.values().any(|active|Arc::ptr_eq(active,p));
            if registered {let _=tokio::time::timeout_at(deadline,done).await;}
        }
        !self.inner.host.lock().await.values().any(|p|owner.is_none_or(|o|o==p.owner))
    }
    pub async fn write_development_terminal(&self,owner:&str,id:&str,input:String)->Result<(),String> {
        use std::io::Write;
        if input.len()>65536 {return Err("terminal_input_limit".into());}self.ready().await?;
        let p=self.inner.host.lock().await.get(id).filter(|p|p.owner==owner).cloned().ok_or("process_not_found")?;
        if p.cancel.is_cancelled() || p.state.lock().await.kind!="terminal" {return Err("terminal_closed".into());}
        let result=p.terminal.lock().map_err(|_|"terminal_closed")?.as_mut().ok_or("terminal_starting")?.write_all(input.as_bytes()).map_err(|_|"terminal_closed".into());result
    }
    pub async fn resize_development_terminal(&self,owner:&str,id:&str,cols:u16,rows:u16)->Result<(),String> {
        self.ready().await?;if rows==0 || cols==0 || rows>1000 || cols>1000 {return Err("invalid_terminal_size".into());}
        let p=self.inner.host.lock().await.get(id).filter(|p|p.owner==owner).cloned().ok_or("process_not_found")?;
        if p.cancel.is_cancelled() || p.state.lock().await.kind!="terminal" {return Err("terminal_closed".into());}
        resize(&p,cols,rows)
    }
}

#[cfg(unix)]
fn resize(p:&HostProcess,cols:u16,rows:u16)->Result<(),String> {
    use std::os::fd::AsRawFd;
    let terminal=p.terminal.lock().map_err(|_|"terminal_closed")?;let fd=terminal.as_ref().ok_or("terminal_starting")?.as_raw_fd();
    let size=libc::winsize{ws_row:rows,ws_col:cols,ws_xpixel:0,ws_ypixel:0};
    if unsafe{libc::ioctl(fd,libc::TIOCSWINSZ,&size)}<0 {Err("terminal_closed".into())}else{Ok(())}
}
#[cfg(not(unix))]
fn resize(_: &HostProcess,_:u16,_:u16)->Result<(),String>{Err("terminal_unsupported".into())}

/// Keep the bounded raw window in memory so redaction spans arbitrary read chunks.
/// The trailing unfinished token is withheld until a delimiter or confirmed EOF.
#[derive(Default)]
struct Output {raw:Vec<u8>,truncated:bool,secrets:Vec<Vec<u8>>}
impl Output {
    fn push(&mut self,bytes:&[u8]) {if self.truncated {return;}let keep=bytes.len().min(OUTPUT_LIMIT-self.raw.len());self.raw.extend_from_slice(&bytes[..keep]);self.truncated=keep<bytes.len();}
    fn view(&self,finished:bool)->String {
        // Redact byte-for-byte before UTF-8 conversion, including non-UTF8 host
        // values. Withhold any suffix that could become a secret on the next read.
        let complete=finished && !self.truncated;
        let mut end=self.raw.len();
        if !complete {for secret in &self.secrets {for n in 1..=secret.len().min(self.raw.len()) {if self.raw.ends_with(&secret[..n]) {end=end.min(self.raw.len()-n);}}}}
        let mut redacted=Vec::with_capacity(end);let mut i=0;
        while i<end {
            if let Some(secret)=self.secrets.iter().filter(|secret|!secret.is_empty() && self.raw[i..].starts_with(secret)).max_by_key(|secret|secret.len()) {redacted.extend_from_slice(b"[REDACTED]");i+=secret.len();}
            else {redacted.push(self.raw[i]);i+=1;}
        }
        let text=String::from_utf8_lossy(&redacted);
        let end=if complete {text.len()}else{text.rfind(char::is_whitespace).map_or(0,|i|i+text[i..].chars().next().unwrap().len_utf8())};
        crate::executions::sanitize_text(&text[..end])
    }
}

#[cfg(target_os="macos")]
fn session_members(session:i32)->Result<Vec<i32>,String> {
    #[link(name="proc")]
    extern "C" {fn proc_listallpids(buffer:*mut libc::c_void,buffersize:libc::c_int)->libc::c_int;}
    let count=unsafe{proc_listallpids(std::ptr::null_mut(),0)};
    if count<=0 || count>1_000_000 {return Err("cleanup_pending".into());}
    let mut pids=vec![0i32;count as usize+1024];let size=(pids.len()*std::mem::size_of::<i32>()) as i32;
    let count=unsafe{proc_listallpids(pids.as_mut_ptr().cast(),size)};
    if count<0 || count as usize>=pids.len() {return Err("cleanup_pending".into());}
    Ok(pids.into_iter().take(count as usize).filter(|pid|*pid>0 && unsafe{libc::getsid(*pid)}==session).collect())
}
#[cfg(all(unix,not(target_os="macos")))]
fn session_members(session:i32)->Result<Vec<i32>,String> {
    let entries=std::fs::read_dir("/proc").map_err(|_|"cleanup_pending")?;let mut pids=Vec::new();
    for entry in entries {let entry=entry.map_err(|_|"cleanup_pending")?;if let Some(pid)=entry.file_name().to_str().and_then(|s|s.parse::<i32>().ok()) {if unsafe{libc::getsid(pid)}==session {pids.push(pid);}}}
    Ok(pids)
}
/// The unreaped session leader pins the session identity until cleanup completes.
#[cfg(unix)]
pub(crate) struct OwnedChild {pub(crate) child:std::process::Child,pub(crate) group:i32,pub(crate) reaped:bool}
#[cfg(unix)]
impl OwnedChild {
    pub(crate) fn exited(&self)->Result<bool,String> {
        let mut info:libc::siginfo_t=unsafe{std::mem::zeroed()};
        let rc=unsafe{libc::waitid(libc::P_PID,self.group as libc::id_t,&mut info,libc::WEXITED|libc::WNOHANG|libc::WNOWAIT)};
        if rc<0 {Err("process_wait_failed".into())} else {Ok(info.si_signo!=0)}
    }
    pub(crate) async fn cleanup(&mut self)->(Option<i32>,bool) {
        self.cleanup_owned(false).await
    }
    pub(crate) async fn cleanup_auth(&mut self)->(Option<i32>,bool) {
        self.cleanup_owned(true).await
    }
    async fn cleanup_owned(&mut self, retain_uncertain: bool)->(Option<i32>,bool) {
        // Never signal a persisted PID or a cached descendant group: descendants
        // can leave and their group IDs can be recycled during the grace period.
        if self.reaped {return (None,false);}
        let mut enumeration_ok=true;
        for signal in [libc::SIGTERM,libc::SIGKILL] {
            match session_members(self.group) {
                Ok(pids)=>for pid in pids {
                    // Signal current members individually, including job-control
                    // groups whose group leader has already exited.
                    if pid!=self.group && unsafe{libc::getsid(pid)}==self.group {unsafe{libc::kill(pid,signal);}}
                },
                Err(_)=>enumeration_ok=false,
            }
            unsafe{libc::kill(-self.group,signal);}
            if signal==libc::SIGTERM {tokio::time::sleep(Duration::from_millis(200)).await;}
        }
        // Confirm no live session members remain before reaping the leader.
        // Keep ownership conservatively if enumeration cannot establish this.
        let mut clean=false;
        for _ in 0..30 {
            if session_members(self.group).is_ok_and(|pids|pids.iter().all(|pid|*pid==self.group)) {clean=enumeration_ok;break;}
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if retain_uncertain && !clean { return (None, false); }
        let result=self.child.wait();self.reaped=result.is_ok();
        (result.ok().and_then(|s|s.code()),clean&&self.reaped)
    }
}
#[cfg(unix)]
impl Drop for OwnedChild {fn drop(&mut self){if !self.reaped {unsafe{libc::kill(-self.group,libc::SIGKILL);}let _=self.child.wait();}}}

#[cfg(unix)]
type ProcessLaunch = (OwnedChild, std::sync::mpsc::Receiver<Vec<u8>>, Vec<Vec<u8>>);

#[cfg(unix)]
fn launch(input:&StartProcessInput,project:&dgos_db::development_repository::Project,process:&HostProcess)->Result<ProcessLaunch,String> {
    use std::{io::Read,os::{fd::{FromRawFd,AsRawFd},unix::process::CommandExt},process::Stdio};
    // Capability-open cwd and pass the live directory descriptor through pre_exec.
    let root=crate::files::open_root(project)?;
    let mut relative=std::path::PathBuf::new();for part in std::path::Path::new(&input.cwd).components(){relative.push(part);if root.symlink_metadata(&relative).map_err(|_|"invalid_workspace")?.file_type().is_symlink(){return Err("file_access_denied".into());}}
    let cwd=if input.cwd.is_empty(){root}else{root.open_dir(&input.cwd).map_err(|_|"invalid_workspace")?};
    let cwd=cwd.into_std_file();let cwd_fd=cwd.as_raw_fd();
    let mut command=std::process::Command::new(&input.program);command.args(&input.args).env_clear();
    for key in ["PATH","HOME","USER","LOGNAME","TMPDIR","LANG","LC_ALL"] {if let Some(value)=std::env::var_os(key) {command.env(key,value);}}
    // Shell startup files can reintroduce secrets; suppress implicit interactive startup.
    command.env("TERM","xterm-256color").env("BASH_ENV","/dev/null").env("ENV","/dev/null").env("ZDOTDIR","/dev/null");
    let mut secrets=Vec::new();
    for (target,reference) in &input.env_refs {
        use std::os::unix::ffi::OsStrExt;
        let value=std::env::var_os(reference).ok_or("environment_reference_missing")?;
        if !value.is_empty(){secrets.push(value.as_bytes().to_vec());}command.env(target,value);
    }
    let mut master=None;
    if input.kind=="terminal" {
        let mut m=-1;let mut s=-1;let mut size=libc::winsize{ws_row:24,ws_col:80,ws_xpixel:0,ws_ypixel:0};
        if unsafe{libc::openpty(&mut m,&mut s,std::ptr::null_mut(),std::ptr::null_mut(),&mut size)}<0 {return Err("terminal_unavailable".into());}
        let m=unsafe{std::fs::File::from_raw_fd(m)};let slave=unsafe{std::fs::File::from_raw_fd(s)};
        unsafe{libc::fcntl(m.as_raw_fd(),libc::F_SETFD,libc::FD_CLOEXEC);}
        command.stdin(Stdio::from(slave.try_clone().map_err(|_|"terminal_unavailable")?)).stdout(Stdio::from(slave.try_clone().map_err(|_|"terminal_unavailable")?)).stderr(Stdio::from(slave));
        unsafe{command.pre_exec(move || {if libc::setsid()<0 || libc::ioctl(0,libc::TIOCSCTTY as libc::c_ulong,0)<0 || libc::fchdir(cwd_fd)<0 {return Err(std::io::Error::last_os_error());}Ok(())});}
        *process.terminal.lock().map_err(|_|"terminal_unavailable")?=Some(m.try_clone().map_err(|_|"terminal_unavailable")?);master=Some(m);
    } else {
        command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        unsafe{command.pre_exec(move || {if libc::setsid()<0 || libc::fchdir(cwd_fd)<0 {return Err(std::io::Error::last_os_error());}Ok(())});}
    }
    let mut child=command.spawn().map_err(|e|if e.kind()==std::io::ErrorKind::NotFound {"program_unavailable"}else{"process_spawn_failed"})?;
    let group=child.id() as i32;
    let (tx,rx)=std::sync::mpsc::sync_channel(64);
    let mut readers:Vec<Box<dyn Read+Send>>=Vec::new();
    if let Some(master)=master {
        readers.push(Box::new(master));
    } else {
        if let Some(out)=child.stdout.take(){readers.push(Box::new(out));}
        if let Some(err)=child.stderr.take(){readers.push(Box::new(err));}
    }
    for mut reader in readers {let tx=tx.clone();std::thread::spawn(move || {
        let mut buffer=[0u8;8192];let mut remaining=OUTPUT_LIMIT+1;
        loop {match reader.read(&mut buffer) {
            Ok(0)|Err(_)=>break,
            Ok(n)=>{
                // Preserve the prefix under backpressure. Once the capture budget
                // is exhausted, keep draining the pipe without allocating chunks.
                let keep=n.min(remaining);
                if keep>0 {if tx.send(buffer[..keep].to_vec()).is_err(){break;}remaining-=keep;}
            },
        }}
    });}
    Ok((OwnedChild{child,group,reaped:false},rx,secrets))
}

#[cfg(target_os="macos")]
async fn owns_listener(session:i32,port:u16)->bool {
    // OS listener inspection never executes a workspace command. Refuse preview
    // when ownership cannot be established; port preflight alone has a race.
    let selector=format!("-iTCP:{port}");
    let mut query=tokio::process::Command::new("/usr/sbin/lsof");
    query.args(["-nP","-a",&selector,"-sTCP:LISTEN","-Fp"]).env_clear().stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    let Ok(Ok(output))=tokio::time::timeout(Duration::from_millis(500),query.output()).await else{return false;};
    if !output.status.success() {return false;}
    let Ok(text)=std::str::from_utf8(&output.stdout) else{return false;};
    let pids:Vec<_>=text.lines().filter_map(|line|line.strip_prefix('p')).map(str::parse::<i32>).collect();
    !pids.is_empty() && pids.into_iter().all(|pid|pid.is_ok_and(|pid|unsafe{libc::getsid(pid)}==session))
}
#[cfg(all(unix,not(target_os="macos")))]
async fn owns_listener(session:i32,port:u16)->bool {
    tokio::task::spawn_blocking(move || {
        let mut sockets=std::collections::BTreeSet::new();
        for table in ["/proc/net/tcp","/proc/net/tcp6"] {
            let Ok(text)=std::fs::read_to_string(table) else{return false;};
            for line in text.lines().skip(1) {
                let fields:Vec<_>=line.split_whitespace().collect();
                if fields.len()>9 && fields[3]=="0A" && fields[1].rsplit_once(':').is_some_and(|(_,value)|u16::from_str_radix(value,16)==Ok(port)) {sockets.insert(format!("socket:[{}]",fields[9]));}
            }
        }
        if sockets.is_empty(){return false;}
        let Ok(pids)=session_members(session) else{return false;};
        for pid in pids {
            let Ok(fds)=std::fs::read_dir(format!("/proc/{pid}/fd")) else{continue;};
            for fd in fds.flatten() {if let Ok(target)=std::fs::read_link(fd.path()) {if let Some(target)=target.to_str(){sockets.remove(target);}}}
        }
        sockets.is_empty()
    }).await.unwrap_or(false)
}
#[cfg(unix)]
async fn http_ready(readiness:&ReadinessInput,session:i32)->bool {
    use tokio::io::{AsyncReadExt,AsyncWriteExt};
    if !owns_listener(session,readiness.port).await {return false;}
    let request=async {
        let mut stream=tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST,readiness.port)).await.ok()?;
        stream.write_all(format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",readiness.path,readiness.port).as_bytes()).await.ok()?;
        let mut response=[0u8;512];let n=stream.read(&mut response).await.ok()?;let line=std::str::from_utf8(&response[..n]).ok()?.lines().next()?;
        let status=line.split_whitespace().nth(1)?.parse::<u16>().ok()?;Some(line.starts_with("HTTP/1.")&&(200..400).contains(&status))
    };
    matches!(tokio::time::timeout(Duration::from_millis(300),request).await,Ok(Some(true))) && owns_listener(session,readiness.port).await
}
async fn persist(manager:&AgentManager,run:&mut StoredRun,view:&ProcessSnapshot)->bool {
    run.status=match view.status.as_str(){"starting"|"ready"|"running"=>"running","cancelled"=>"cancelled","unknown"|"cleanup_pending"=>"unknown","failed"=>"failed",_=>if view.exit_code==Some(0){"completed"}else{"failed"}}.into();
    run.finished_at=view.finished_at;run.reason=view.reason.clone();run.fingerprint=view.fingerprint.clone();run.truncated=view.truncated;
    run.runtime=serde_json::to_string(view).ok();manager.inner.repository.checkpoint_host_run(run).await.is_ok()
}
/// Clean process exit alone cannot discard a terminal projection that SQLite
/// has not accepted. Keep its permit and retry without executing the command again.
async fn settle_process(manager:&AgentManager,process:&Arc<HostProcess>,run:&mut StoredRun,clean:bool)->bool {
    process.terminal.lock().ok().map(|mut terminal|terminal.take());
    let mut backoff=Duration::from_millis(250);
    loop {
        let persisted={let view=process.state.lock().await;persist(manager,run,&view).await};
        if !clean {
            process.done.notify_waiters();
            return false;
        }
        if persisted {
            let mut host=manager.inner.host.lock().await;
            if host.get(&run.id).is_some_and(|active|Arc::ptr_eq(active,process)) {host.remove(&run.id);}
            process.retained_permit.lock().unwrap_or_else(|error|error.into_inner()).take();
            process.done.notify_waiters();
            return true;
        }
        tokio::time::sleep(backoff).await;
        backoff=(backoff*2).min(Duration::from_secs(2));
    }
}
#[cfg(unix)]
async fn supervise(manager:AgentManager,process:Arc<HostProcess>,input:StartProcessInput,project:dgos_db::development_repository::Project,mut run:StoredRun)->bool {
    let mut launch_error=None;
    if let Some(r)=&input.readiness {if tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,r.port)).await.is_err(){launch_error=Some("port_conflict".to_string());}}
    let launched=if process.cancel.is_cancelled() || manager.inner.stop.is_cancelled() {Err("cancelled".into())}else if let Some(error)=launch_error {Err(error)}else{launch(&input,&project,&process)};
    let (mut child,rx,secrets)=match launched {Ok(v)=>v,Err(error)=>{
        let mut view=process.state.lock().await;let cancelled=error=="cancelled";view.status=if cancelled {"cancelled"}else{"failed"}.into();view.reason=Some(error);view.finished_at=Some(now());if input.verification {view.verification_status=if cancelled {"unknown"}else{"failed"}.into();}
        drop(view);return settle_process(&manager,&process,&mut run,true).await;
    }};
    let started=std::time::Instant::now();let mut sample=std::time::Instant::now();let mut output=Output{secrets,..Output::default()};let mut failure=None;let mut cancelled=false;let mut persisted_mutation=false;
    loop {
        let previous=(output.raw.len(),output.truncated);
        while let Ok(bytes)=rx.try_recv(){output.push(&bytes);}
        let mut changed=previous!=(output.raw.len(),output.truncated);
        if changed {let mut view=process.state.lock().await;view.output=output.view(false);view.truncated=output.truncated;}
        if input.verification && sample.elapsed()>=Duration::from_millis(400) {observe(&manager,&process).await;sample=std::time::Instant::now();changed=true;}
        if process.cancel.is_cancelled() || manager.inner.stop.is_cancelled(){cancelled=true;break;}
        match child.exited(){Ok(true)=>{if input.kind=="service" && process.state.lock().await.status=="starting"{failure=Some("service_exited_before_ready".into());}break;},Err(error)=>{failure=Some(error);break;},_=>{}}
        if let Some(readiness)=&input.readiness {
            let starting=process.state.lock().await.status=="starting";
            if starting && http_ready(readiness,child.group).await && child.exited()==Ok(false) {let mut view=process.state.lock().await;view.status="ready".into();view.preview_url=Some(format!("http://127.0.0.1:{}{}",readiness.port,readiness.path));changed=true;}
            else if starting && started.elapsed()>=Duration::from_millis(readiness.timeout_ms) {failure=Some("service_readiness_timeout".into());break;}
        }
        {let view=process.state.lock().await;if changed || view.mutation_observed!=persisted_mutation {if !persist(&manager,&mut run,&view).await {failure=Some("storage_failed".into());break;}persisted_mutation=view.mutation_observed;}}
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (exit_code,clean)=child.cleanup().await;
    // Reader threads can still be publishing bytes after waitid/cleanup. Wait
    // for EOF before exposing a trailing token as complete; bound escaped pipes.
    let drain=std::time::Instant::now();let mut eof=false;
    loop {
        match rx.try_recv() {
            Ok(bytes)=>output.push(&bytes),
            Err(std::sync::mpsc::TryRecvError::Disconnected)=>{eof=true;break;},
            Err(std::sync::mpsc::TryRecvError::Empty)=>{
                if drain.elapsed()>=Duration::from_millis(500){break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            },
        }
    }
    if !eof {output.truncated=true;}
    if input.verification {observe(&manager,&process).await;}
    let mut view=process.state.lock().await;view.output=output.view(true);view.truncated|=output.truncated;view.exit_code=exit_code;view.finished_at=Some(now());view.preview_url=None;
    view.status=if !clean {"cleanup_pending"}else if cancelled {"cancelled"}else if failure.is_some() || exit_code!=Some(0) {"failed"}else{"exited"}.into();
    view.reason=if !clean {Some("cleanup_pending".into())}else if cancelled {Some("cancelled".into())}else{failure};
    if input.verification {view.verification_status=verification_result(&view,cancelled,clean).into();}
    drop(view);
    settle_process(&manager,&process,&mut run,clean).await
}
fn verification_result(view:&ProcessSnapshot,cancelled:bool,clean:bool)->&'static str {
    if !clean || cancelled || view.exit_code.is_none() {"unknown"}else if view.exit_code!=Some(0) || view.reason.is_some(){"failed"}else if !view.verification_complete {"incomplete"}else if view.mutation_observed || view.fingerprint!=view.start_fingerprint {"changed"}else{"observed_stable"}
}
async fn observe(manager:&AgentManager,process:&HostProcess) {
    let task=process.state.lock().await.task_id.clone();let snapshot=manager.verification_snapshot(&process.owner,&task).await;
    let mut view=process.state.lock().await;
    match snapshot {Ok((fingerprint,complete))=>{view.verification_complete&=complete;view.mutation_observed|=fingerprint!=view.start_fingerprint;view.fingerprint=fingerprint;},Err(_)=>view.verification_complete=false}
}
#[cfg(not(unix))]
async fn supervise(manager:AgentManager,process:Arc<HostProcess>,_input:StartProcessInput,_:dgos_db::development_repository::Project,mut run:StoredRun)->bool {
    let mut view=process.state.lock().await;view.status="failed".into();view.reason=Some("process_platform_unsupported".into());view.finished_at=Some(now());drop(view);settle_process(&manager,&process,&mut run,true).await
}

#[cfg(test)]
mod tests;
