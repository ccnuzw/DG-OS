#![cfg(unix)]
use async_trait::async_trait;
use dgos_db::{sea_orm::{ConnectionTrait, ConnectOptions, Database}, sea_orm_migration::MigratorTrait};
use dgos_mcp::*;
use serde_json::json;
use std::{sync::Arc, time::Duration};
struct Allow;
#[async_trait]
impl McpCallPolicy for Allow {
    async fn allows_call(&self, _: &str, _: &str, _: &str) -> bool { true }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_reaps_stdio_while_call_audit_is_write_locked() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", root.path().join("db.sqlite").display());
    let db = Database::connect(&url).await.unwrap();
    dgos_db::migrations::Migrator::up(&db, None).await.unwrap();
    let manager = Arc::new(McpManager::with_policy(db, Arc::new(Allow)));
    let script = root.path().join("hold.mjs");
    std::fs::write(&script, r#"
import fs from 'node:fs';
import readline from 'node:readline';
fs.writeFileSync('pid', String(process.pid));
const reply=(id,result)=>process.stdout.write(JSON.stringify({jsonrpc:'2.0',id,result})+'\n');
readline.createInterface({input:process.stdin}).on('line',line=>{
 const m=JSON.parse(line);
 if(m.method==='initialize')reply(m.id,{protocolVersion:'2025-03-26',capabilities:{tools:{}},serverInfo:{name:'fixture',version:'1'}});
 if(m.method==='tools/list')reply(m.id,{tools:[{name:'hold',inputSchema:{type:'object',properties:{}}}]});
 if(m.method==='tools/call')fs.writeFileSync('entered','');
});
"#).unwrap();
    let server = manager.create_server("alice", CreateMcpServerRequest {
        name: "locked audit".into(), transport: TransportKind::Stdio,
        stdio: Some(StdioConfig { executable: "node".into(), args: vec![script.to_string_lossy().into()], cwd: Some(root.path().to_string_lossy().into()) }),
        http: None, enabled: Some(true), auto_connect: Some(false),
    }).await.unwrap();
    manager.connect("alice", &server.id).await.unwrap();
    let mut input = CallMcpToolRequest {
        server_id: server.id.clone(), tool_name: "hold".into(), arguments: json!({}),
        call_id: Some(uuid::Uuid::new_v4().to_string()), task_ref: None, timeout_ms: None, confirmation_token: None,
    };
    input.confirmation_token = Some(manager.confirm_call("alice", &input).await.unwrap());
    let worker = manager.clone();
    let call = tokio::spawn(async move { worker.call_tool("alice", input).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !root.path().join("entered").exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
    }).await.unwrap();
    let pid = std::fs::read_to_string(root.path().join("pid")).unwrap();
    let mut options = ConnectOptions::new(url);
    options.max_connections(1);
    let blocker = Database::connect(options).await.unwrap();
    blocker.execute_unprepared("BEGIN IMMEDIATE").await.unwrap();
    let worker = manager.clone();
    let shutdown = tokio::spawn(async move { worker.shutdown().await });
    let reaped = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let alive = std::process::Command::new("kill").args(["-0", &pid]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().unwrap().success();
            if !alive { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.is_ok();
    let draining = !shutdown.is_finished();
    // Release even on a failed assertion, so fixture cleanup cannot strand a child.
    blocker.execute_unprepared("ROLLBACK").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), shutdown).await.unwrap().unwrap().unwrap();
    let response = call.await.unwrap().unwrap();
    assert!(reaped, "stdio child survived while shutdown waited for audit writes");
    assert!(draining, "shutdown reported completion before audit drained");
    assert_eq!(response.outcome, "cancelled");
    assert!(!response.audit_warning);
    assert_eq!(manager.list_usage_logs("alice", Some(&server.id), None).await.unwrap()[0].outcome, "cancelled");
    assert_eq!(manager.list_servers("alice").await.unwrap()[0].status, "configured");
}
