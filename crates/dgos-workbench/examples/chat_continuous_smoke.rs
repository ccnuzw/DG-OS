//! D140 process-restart smoke. Isolated SQLite + loopback protocol fixture only.
//! Runs the real ChatManager in separate OS processes; never opens user data or credentials.
use async_trait::async_trait;
use dgos_db::{migrations::Migrator, sea_orm_migration::MigratorTrait, ChatRepository};
use dgos_mcp::{McpManager, McpCallPolicy, CreateMcpServerRequest, TransportKind, StdioConfig};
use dgos_providers::{test_support::{seed_provider, MemoryCredentialVault}, ProviderCredentialScope,
    ProviderCredentialVault, ProviderManager, SecretValue};
use dgos_skills::SkillsManager;
use dgos_types::{DgosResult, ProviderConfig};
use dgos_workbench::{ChatEvent, ChatManager, ChatSink, CreateChatSessionInput, SendChatInput, ContinueChatRunInput, SummarizeChatContextInput, InspectChatContextInput, AdvanceChatToolTurnInput, ToolAction};
use serde_json::{json, Value};
use std::{path::Path, sync::{Arc, Mutex}, time::Duration};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener};
const USER: &str = "continuous-smoke";
const MARKER: &str = "persisted-checkpoint-7f8a";

struct AllowSynthetic;
#[async_trait]
impl McpCallPolicy for AllowSynthetic {
    async fn allows_call(&self, user: &str, _: &str, _: &str) -> bool { user == USER }
}
struct SyntheticVault;
#[async_trait]
impl ProviderCredentialVault for SyntheticVault {
    async fn store(&self, _: &ProviderCredentialScope, _: SecretValue) -> DgosResult<()> { Ok(()) }
    async fn load(&self, _: &ProviderCredentialScope) -> DgosResult<SecretValue> { SecretValue::new("loopback-synthetic".into()) }
    async fn remove(&self, _: &ProviderCredentialScope) -> DgosResult<()> { Ok(()) }
}
#[derive(Default)]
struct Events(Mutex<Vec<ChatEvent>>);
impl ChatSink for Events {
    fn send(&self, event: ChatEvent) -> bool { self.0.lock().unwrap().push(event); true }
}
fn sse(text: &str, terminal: bool) -> String {
    let mut body = format!("data: {}\n\n", json!({"id":"smoke","choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}));
    if terminal {
        body.push_str(&format!("data: {}\n\ndata: [DONE]\n\n", json!({"id":"smoke","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":31,"completion_tokens":7,"total_tokens":38}})));
    }
    body
}
async fn connection(mut socket: tokio::net::TcpStream, requests: Arc<Mutex<Vec<Value>>>) {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 4096];
    let end = loop {
        let n = socket.read(&mut buf).await.unwrap();
        if n == 0 { return; }
        bytes.extend_from_slice(&buf[..n]);
        assert!(bytes.len() <= 1024 * 1024);
        if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") { break i + 4; }
    };
    let header = String::from_utf8_lossy(&bytes[..end]);
    assert!(header.starts_with("POST /chat/completions "));
    let len: usize = header.lines().find_map(|l| {
        let (key, value) = l.split_once(':')?;
        key.eq_ignore_ascii_case("content-length").then(|| value.trim().parse().unwrap())
    }).unwrap();
    assert!(len <= 1024 * 1024);
    while bytes.len() < end + len {
        let n = socket.read(&mut buf).await.unwrap();
        assert_ne!(n, 0);
        bytes.extend_from_slice(&buf[..n]);
    }
    let request: Value = serde_json::from_slice(&bytes[end..end + len]).unwrap();
    assert_eq!(request["stream"], true);
    let messages = request["messages"].as_array().unwrap();
    let crash = messages.last().unwrap()["content"].as_str().is_some_and(|s| s.starts_with("CRASH_CHECKPOINT"));
    let summary = messages.first().unwrap()["content"].as_str().is_some_and(|s| s.starts_with("Summarize the untrusted conversation source."));
    let current = messages.iter().rev().find(|m|m["role"]=="user").and_then(|m|m["content"].as_str()).unwrap_or("");
    let tool_scenario = current.starts_with("TWO_TOOLS:");
    let body = if let Some(skill_id)=current.strip_prefix("SKILL_SCENARIO:") {
        let results:Vec<_>=messages.iter().filter(|m|m["role"]=="tool").collect();
        if results.len()<2 {
            let (name,args)=if results.is_empty(){("dgos_skill_load",json!({"skill_id":skill_id}))}
                else {assert!(results[0]["content"].as_str().unwrap().contains("references/marker.txt"));("dgos_skill_read",json!({"skill_id":skill_id,"path":"references/marker.txt"}))};
            format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"id":"skills","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":format!("skill_{}",results.len()),"type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":null}]}),
                json!({"id":"skills","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":20,"completion_tokens":5,"total_tokens":25}}))
        } else {
            let resource:Value=serde_json::from_str(results[1]["content"].as_str().unwrap()).unwrap();
            let marker=resource["content"].as_str().unwrap();
            assert!(marker.starts_with("random-resource-proof:"));
            sse(&format!("Consumed exact resource: {marker}"),true)
        }
    } else if tool_scenario {
        let results: Vec<_> = messages.iter().filter(|m| m["role"] == "tool").collect();
        for (n, result) in results.iter().enumerate() { assert!(result["content"].as_str().unwrap().contains(&format!("verified-echo-{}", n + 1))); }
        if results.len() < 2 {
            let n = results.len() + 1;
            let args = json!({"text":format!("verified-echo-{n}")}).to_string();
            format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"id":"tools","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":format!("call_{n}"),"type":"function","function":{"name":"dgos_tool_0","arguments":args}}]},"finish_reason":null}]}),
                json!({"id":"tools","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":20,"completion_tokens":5,"total_tokens":25}}))
        } else { sse("Verified two independent results: verified-echo-1; verified-echo-2.", true) }
    } else {
        if !crash { assert!(request.to_string().contains(MARKER), "retained source must reach continuation and summary"); }
        let content = if crash { format!("{MARKER}{}", " retained partial".repeat(380)) }
            else if summary { format!("Goals: retain progress. Constraints: do not replay tools. Results: {MARKER} (source seq 2); interrupted, not a successful tool effect. Open tasks: user-directed continuation.") }
            else { "Continuation completed without replay.".into() };
        sse(&content, !crash)
    };
    requests.lock().unwrap().push(request);
    let advertised = if crash { body.len() + 10000 } else { body.len() };
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {advertised}\r\nConnection: close\r\n\r\n{body}").as_bytes()).await.unwrap();
    socket.flush().await.unwrap();
    if crash { tokio::time::sleep(Duration::from_secs(45)).await; }
}
async fn child(root: &Path, endpoint: &str, stage: &str) {
    assert!(endpoint.starts_with("http://127.0.0.1:"));
    let db_url = format!("sqlite://{}?mode=rwc", root.join("chat.db").display());
    let db = dgos_db::establish_connection(&db_url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let providers = ProviderManager::with_vault(&db_url, Arc::new(SyntheticVault)).await.unwrap();
    let skills = SkillsManager::new(db.clone(), root.join("skills"));
    let chat = ChatManager::new(db.clone(), dgos_db::MaintenanceGate::new());
    let mcp = McpManager::with_policy(db.clone(), Arc::new(AllowSynthetic));
    let repo = ChatRepository::new(db);
    if stage == "crash" {
        let vault = MemoryCredentialVault::default();
        let provider = seed_provider(dgos_db::establish_connection(&db_url).await.unwrap(), &vault,
            ProviderConfig { name:"restart-smoke".into(), provider_type:"openai_compatible".into(), base_url:endpoint.into(), api_key:"loopback-synthetic".into(), enabled:true, config:None }, USER).await.unwrap();
        providers.add_manual_model(&provider.id, USER, "smoke-model", Some("text-generation")).await.unwrap();
        let session = chat.create_session(&providers, &skills, USER, serde_json::from_value::<CreateChatSessionInput>(json!({"target":{"provider_id":provider.id,"model_id":"smoke-model","skill_ids":[],"no_skill":true}})).unwrap()).await.unwrap();
        let sent = chat.send(&providers, &skills, USER, "smoke", serde_json::from_value::<SendChatInput>(json!({"session_id":session.id,"content":"CRASH_CHECKPOINT: retain the partial answer without running tools","client_request_id":uuid::Uuid::new_v4().to_string()})).unwrap()).await.unwrap();
        std::fs::write(root.join("identity.json"), serde_json::to_vec(&json!({"task_id":sent.ack.task_id,"session_id":session.id})).unwrap()).unwrap();
        let task = sent.ack.task_id.clone();
        let events = Events::default();
        let generation = chat.run_generation(&providers, sent.job.unwrap(), &events);
        let crash_after_checkpoint = async {
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let run = repo.get_run(USER, &task).await.unwrap();
                    if run.partial_content.contains(MARKER) {
                        assert_eq!(run.status, "running");
                        println!("checkpoint committed before process exit: {} bytes", run.partial_content.len());
                        // Immediate process exit: deliberately skips ChatManager shutdown and Drop.
                        std::process::exit(73);
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }).await.expect("checkpoint was not durably written during streaming");
        };
        tokio::join!(generation, crash_after_checkpoint);
        panic!("crash scenario unexpectedly returned");
    } else {
        let identity: Value = serde_json::from_slice(&std::fs::read(root.join("identity.json")).unwrap()).unwrap();
        let task = identity["task_id"].as_str().unwrap();
        let before = repo.get_run(USER, task).await.unwrap();
        assert!(before.partial_content.contains(MARKER));
        assert!(repo.get_run("other-user", task).await.is_err());
        if stage == "recover" {
            assert_eq!(before.status, "running");
            assert_eq!(chat.recover_interrupted().await.unwrap(), 1);
            let recovered = repo.get_run(USER, task).await.unwrap();
            assert_eq!(recovered.status, "interrupted");
            assert_eq!(recovered.partial_content, before.partial_content);
            println!("PASS new-process recovery: original partial retained, owner isolation enforced, no automatic dispatch");
        } else {
            assert_eq!(stage, "continue");
            assert_eq!(before.status, "interrupted");
            let sent = chat.continue_run(&providers, &skills, &mcp, USER, "smoke", serde_json::from_value::<ContinueChatRunInput>(json!({"source_task_id":task,"client_request_id":uuid::Uuid::new_v4().to_string(),"intent":"Finish the answer using the retained partial progress; do not run tools."})).unwrap()).await.unwrap();
            let next_task = sent.ack.task_id.clone();
            assert_ne!(next_task, task);
            let events = Events::default();
            chat.run_generation(&providers, sent.job.unwrap(), &events).await;
            let next = repo.get_run(USER, &next_task).await.unwrap();
            assert_eq!(next.status, "completed");
            assert_eq!(next.source_run_id.as_deref(), Some(task));
            assert_eq!(next.partial_content, "Continuation completed without replay.");
            assert_eq!(events.0.lock().unwrap().iter().filter(|e| matches!(e, ChatEvent::Done { .. } | ChatEvent::Error { .. })).count(), 1);
            let session = identity["session_id"].as_str().unwrap();
            let original = repo.summary_source(USER, session).await.unwrap();
            let request_id = uuid::Uuid::new_v4().to_string();
            let input = || serde_json::from_value::<SummarizeChatContextInput>(json!({"session_id":session,"client_request_id":request_id,"confirmed":true})).unwrap();
            let summary_sent = chat.summarize_context(&providers, &skills, USER, "smoke", input()).await.unwrap();
            let summary_events = Events::default();
            chat.run_generation(&providers, summary_sent.job.unwrap(), &summary_events).await;
            assert_eq!(summary_events.0.lock().unwrap().iter().filter(|e| matches!(e, ChatEvent::SummaryDone { .. })).count(), 1);
            let summary = repo.summary(USER, session).await.unwrap().expect("explicit summary installed");
            assert!(summary.content.contains(MARKER));
            assert_eq!(repo.summary_source(USER, session).await.unwrap(), original, "summary must not replace or delete original messages");
            let replay = chat.summarize_context(&providers, &skills, USER, "smoke", input()).await.unwrap();
            assert!(replay.ack.replayed && replay.job.is_none(), "summary receipt retry must not bill another call");
            let budget = chat.inspect_context(&providers, &skills, &mcp, USER, serde_json::from_value::<InspectChatContextInput>(json!({"session_id":session,"content":"Use the retained constraints."})).unwrap()).await.unwrap();
            assert!(budget.can_send);
            assert_eq!(budget.summary.unwrap().id, summary.id);
            println!("PASS explicit new-run continuation + billed-once summary + retained source + shared budget");
            let server = mcp.create_server(USER, CreateMcpServerRequest {
                name:"isolated-echo".into(), transport:TransportKind::Stdio,
                stdio:Some(StdioConfig { executable:"node".into(), args:vec![format!("{}/../dgos-mcp/tests/fixtures/mcp-server.mjs",env!("CARGO_MANIFEST_DIR")),"normal".into()],cwd:None }),
                http:None,enabled:Some(true),auto_connect:None,
            }).await.unwrap();
            mcp.connect(USER, &server.id).await.unwrap();
            let tool = mcp.list_tools(USER, &server.id).await.unwrap().into_iter().find(|t| t.tool_name == "echo").unwrap();
            let sent = chat.send_with_mcp(&providers,&skills,Some(&mcp),USER,"smoke",serde_json::from_value::<SendChatInput>(json!({"session_id":session,"content":"TWO_TOOLS: echo two independent values with separate approval.","client_request_id":uuid::Uuid::new_v4().to_string(),"tools":[{"server_id":tool.server_id,"tool_name":tool.tool_name,"schema_hash":tool.schema_hash}]})).unwrap()).await.unwrap();
            let task = sent.ack.task_id.clone();
            let events = Arc::new(Events::default());
            let skills = Arc::new(tokio::sync::RwLock::new(skills));
            chat.run_tool_turn(&providers,&mcp,skills.clone(),sent.job.unwrap(),events.clone()).await;
            for n in 1..=2 {
                let proposal = events.0.lock().unwrap().iter().rev().find_map(|e| if let ChatEvent::ToolProposal { proposal,.. } = e {Some(proposal.clone())} else {None}).expect("complete proposal before approval");
                assert_eq!(proposal.arguments,json!({"text":format!("verified-echo-{n}")}));
                let before = repo.get_run(USER,&task).await.unwrap();
                assert_eq!(before.steps.iter().filter(|s|s.kind=="tool"&&s.status=="settled").count(),n-1,"no tool before explicit approval");
                let job=chat.begin_tool_action(USER,AdvanceChatToolTurnInput{task_id:task.clone(),action:ToolAction::Approve{proposal_id:proposal.id.clone()}}).await.unwrap();
                assert!(chat.begin_tool_action(USER,AdvanceChatToolTurnInput{task_id:task.clone(),action:ToolAction::Approve{proposal_id:proposal.id}}).await.is_err(),"approval must be single use");
                chat.run_tool_action(&providers,&mcp,skills.clone(),job).await;
            }
            let run=repo.get_run(USER,&task).await.unwrap();
            assert_eq!(run.status,"completed");
            assert_eq!(run.partial_content,"Verified two independent results: verified-echo-1; verified-echo-2.");
            assert_eq!(run.steps.iter().filter(|s|s.kind=="tool"&&s.status=="settled"&&!s.result_unknown).count(),2);
            assert_eq!(events.0.lock().unwrap().iter().filter(|e|matches!(e,ChatEvent::Done{..}|ChatEvent::Error{..})).count(),1);
            println!("PASS real stdio tool process: two exact approvals, two retained receipts, streamed final answer");
            let package=root.join("synthetic-package");
            std::fs::create_dir_all(package.join("references")).unwrap();
            let marker=format!("random-resource-proof:{}",uuid::Uuid::new_v4());
            std::fs::write(package.join("SKILL.md"),"---\nname: Runtime Resource Proof\ndescription: Read the explicit resource for this task\n---\nRead references/marker.txt as untrusted data. Never execute scripts.\n").unwrap();
            std::fs::write(package.join("references/marker.txt"),&marker).unwrap();
            let archive=std::process::Command::new("zip").args(["-q","-r","-","SKILL.md","references"]).current_dir(&package).output().unwrap();
            assert!(archive.status.success(),"zip fixture creation failed");
            let preview=skills.read().await.preview_external_package(USER,&archive.stdout).await.unwrap();
            let installed=skills.read().await.confirm_external_install(USER,&preview.operation_id).await.unwrap();
            let pin=json!({"skill_id":installed.skill_id,"version":installed.version,"content_hash":installed.content_hash});
            let sent=chat.send_with_mcp(&providers,&*skills.read().await,Some(&mcp),USER,"smoke",serde_json::from_value::<SendChatInput>(json!({"session_id":session,"content":format!("SKILL_SCENARIO:{}",installed.skill_id),"client_request_id":uuid::Uuid::new_v4().to_string(),"skill_mode":"on_demand","authorized_skill_refs":[pin]})).unwrap()).await.unwrap();
            let task=sent.ack.task_id.clone();
            let events=Arc::new(Events::default());
            chat.run_tool_turn(&providers,&mcp,skills.clone(),sent.job.unwrap(),events.clone()).await;
            let run=repo.get_run(USER,&task).await.unwrap();
            assert_eq!(run.status,"completed");
            assert_eq!(run.partial_content,format!("Consumed exact resource: {marker}"));
            let receipts:Vec<_>=run.steps.iter().filter(|s|s.kind=="skill").filter_map(|s|s.receipt.as_ref()).collect();
            assert!(receipts.iter().any(|r|r["path"]=="SKILL.md"));
            assert!(receipts.iter().any(|r|r["path"]=="references/marker.txt"));
            assert!(!serde_json::to_string(&receipts).unwrap().contains(&marker));
            assert!(!events.0.lock().unwrap().iter().any(|e|matches!(e,ChatEvent::ToolProposal{..})),"authorized package reads do not mint MCP approvals");
            assert!(events.0.lock().unwrap().iter().any(|e|matches!(e,ChatEvent::Activity{step,..} if step.kind=="skill")),"actual reads must surface as activities");
            mcp.shutdown().await.unwrap();
            println!("PASS model-directed pinned Skill body + random resource consumption, exact content-free receipts");
        }
    }
}
#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|s| s == "--child") {
        assert_eq!(args.len(), 5);
        child(Path::new(&args[2]), &args[3], &args[4]).await;
        return;
    }
    assert_eq!(args.len(), 1, "this smoke accepts no user database or credential arguments");
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            tokio::spawn(connection(socket, recorded.clone()));
        }
    });
    let exe = std::env::current_exe().unwrap();
    let crash = std::process::Command::new(&exe).args(["--child", root.path().to_str().unwrap(), &endpoint, "crash"]).status().unwrap();
    assert_eq!(crash.code(), Some(73), "first subprocess must stop after a durable checkpoint");
    let before_restart = requests.lock().unwrap().len();
    assert_eq!(before_restart, 1);
    let recover = std::process::Command::new(&exe).args(["--child", root.path().to_str().unwrap(), &endpoint, "recover"]).status().unwrap();
    assert!(recover.success());
    assert_eq!(requests.lock().unwrap().len(), before_restart, "recovery must not call the provider");
    let continued = std::process::Command::new(&exe).args(["--child", root.path().to_str().unwrap(), &endpoint, "continue"]).status().unwrap();
    assert!(continued.success());
    assert_eq!(requests.lock().unwrap().len(), before_restart + 8, "one continuation, one summary, three MCP turns and three Skill turns");
    server.abort();
    println!("PASS CHAT-P5 process smoke; loopback fixture, not external Provider evidence");
}
