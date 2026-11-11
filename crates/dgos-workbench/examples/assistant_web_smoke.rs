//! Opt-in D147 acceptance using one configured Provider and real public HTTPS.
//! The formal DB is opened read-only; credentials remain in memory and all writes
//! go to a temporary DB. macOS vault lookup requires explicit --allow-configured-provider.
//! cargo run -p dgos-workbench --example assistant_web_smoke -- --allow-configured-provider --db PATH --provider NAME --model MODEL
//! Add --search to verify Brave with the configured account's separate search credential.
use dgos_db::{migrations::Migrator, sea_orm::{Database, ConnectionTrait, DbBackend, Statement}, sea_orm_migration::MigratorTrait, ModelRepository, ProviderRepository};
use dgos_providers::{test_support::{seed_provider, MemoryCredentialVault}, ProviderCredentialScope, SecretValue, ProviderManager};
use dgos_workbench::{ChatManager, ChatEvent, ChatSink, CreateChatSessionInput, ChatTargetInput, SendChatInput, GetChatRunInput};
use dgos_types::ProviderConfig;
use serde_json::{json, Value};
use std::{sync::{Arc, Mutex}, time::Duration};

#[derive(Default)]
struct Events(Mutex<Vec<Value>>);
impl ChatSink for Events {
    fn send(&self, event: ChatEvent) -> bool {
        self.0.lock().unwrap().push(serde_json::to_value(event).expect("event serializes"));
        true
    }
}

async fn secret(scope: &ProviderCredentialScope) -> Result<SecretValue, Box<dyn std::error::Error>> {
    if !cfg!(target_os="macos") { return Err("this opt-in smoke uses the macOS credential adapter".into()); }
    let mut command=tokio::process::Command::new("/usr/bin/security");
    command.args(["find-generic-password", "-s", "com.dgos.app.providers.dev", "-a", &scope.account(), "-w"]);
    command.kill_on_drop(true);
    let mut output=tokio::time::timeout(Duration::from_secs(15),command.output()).await.map_err(|_|"credential read timed out")??;
    if !output.status.success() { output.stdout.fill(0); return Err("configured credential unavailable in development vault".into()); }
    let value=String::from_utf8(output.stdout.clone()).map_err(|_|"credential encoding invalid")?;
    output.stdout.fill(0);
    Ok(SecretValue::new(value)?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args:Vec<String>=std::env::args().skip(1).collect();
    if !args.iter().any(|v|v=="--allow-configured-provider") { return Err("explicit --allow-configured-provider is required; this makes a paid model request".into()); }
    let arg=|key:&str|args.iter().position(|v|v==key).and_then(|i|args.get(i+1)).cloned().ok_or_else(||format!("missing {key}"));
    let formal_path=arg("--db")?;
    let name=arg("--provider")?;
    let model_id=arg("--model")?;
    let search=args.iter().any(|v|v=="--search");
    let formal=Database::connect(format!("sqlite://{formal_path}?mode=ro")).await?;
    let source=ProviderRepository::new(formal.clone()).list("default-user").await?.into_iter().find(|p|p.name==name&&p.enabled).ok_or("configured enabled Provider not found")?;
    let mut model=ModelRepository::new(formal.clone()).get(&source.id,&model_id).await?.ok_or("configured model not found")?;
    let scope=ProviderCredentialScope{user_id:source.user_id.clone(),provider_id:source.id.clone(),endpoint:source.base_url.clone(),credential_ref:source.credential_ref.clone().ok_or("Provider has no credential reference")?};
    let provider_secret=secret(&scope).await?;
    let dir=tempfile::tempdir()?;
    let db_url=format!("sqlite://{}?mode=rwc",dir.path().join("acceptance.db").display());
    let db=dgos_db::establish_connection(&db_url).await?;
    Migrator::up(&db,None).await?;
    let vault=Arc::new(MemoryCredentialVault::default());
    let provider=seed_provider(db.clone(),&vault,ProviderConfig{name:"D147 isolated acceptance".into(),provider_type:source.provider_type,base_url:source.base_url,api_key:provider_secret.expose().to_owned(),enabled:true,config:source.config},"default-user").await?;
    model.id=uuid::Uuid::new_v4().to_string();model.provider_id=provider.id.clone();
    ModelRepository::new(db.clone()).sync_models(&provider.id,vec![model]).await?;
    let providers=ProviderManager::with_vault(&db_url,vault.clone()).await?;
    let chat=ChatManager::new(db.clone(),dgos_db::MaintenanceGate::new()).with_web_vault(vault.clone()).with_web_credential_lock(providers.credential_mutation_lock());
    let skills=Arc::new(tokio::sync::RwLock::new(dgos_skills::SkillsManager::new(db.clone(),dir.path().join("skills"))));
    let mcp=dgos_mcp::McpManager::new(db.clone());
    if search {
        use dgos_workbench::web::{BRAVE_CREDENTIAL_SETTING,BRAVE_ENDPOINT,BRAVE_PROVIDER_ID};
        let row=formal.query_one(Statement::from_sql_and_values(DbBackend::Sqlite,"SELECT setting_value FROM system_settings WHERE user_id = ? AND setting_key = ?",["default-user".into(),BRAVE_CREDENTIAL_SETTING.into()])).await?.ok_or("Brave search credential is not configured; no live search attempted")?;
        let raw:String=row.try_get("","setting_value")?;
        let reference:String=serde_json::from_str(&raw)?;
        let key=secret(&ProviderCredentialScope{user_id:"default-user".into(),provider_id:BRAVE_PROVIDER_ID.into(),endpoint:BRAVE_ENDPOINT.into(),credential_ref:reference}).await?;
        chat.web_service()?.configure("default-user",Some(key.expose().to_owned()),false).await?;
    }
    formal.close().await?;
    let session=chat.create_session(&providers,&*skills.read().await,"default-user",CreateChatSessionInput{target:Some(ChatTargetInput{provider_id:Some(provider.id),model_id:Some(model_id),skill_ids:vec![],no_skill:true,generation:serde_json::from_value(json!({"max_tokens":512}))?})}).await.map_err(|error|format!("chat session rejected: {} {}",error.code(),error.message()))?;
    let prompt=if search { "Use dgos_web_search exactly once to find the official Rust programming language website. Then use dgos_web_fetch on that official HTTPS website once. Answer in under 60 words stating what Rust is, with one source citation using the exact dgos-source ID returned by the tool. Do not use other tools or make more than these two calls." } else { "Use dgos_web_fetch exactly once on https://example.com/ . Then answer in under 40 words with the page's title and what it says the domain is for, citing the returned source using its exact dgos-source ID. Do not answer from memory or call other tools." };
    let request:SendChatInput=serde_json::from_value(json!({"session_id":session.id,"content":prompt,"client_request_id":uuid::Uuid::new_v4().to_string(),"web_access":{"search":search,"fetch":true}}))?;
    let replay=request.clone();
    let sent=chat.send_with_mcp(&providers,&*skills.read().await,Some(&mcp),"default-user","d147-isolated-smoke",request).await.map_err(|error|format!("chat send rejected: {} {}",error.code(),error.message()))?;
    let task=sent.ack.task_id.clone();
    let events=Arc::new(Events::default());
    let job=sent.job.ok_or("new request unexpectedly replayed")?;
    if !job.is_tool_turn(){return Err("web permission did not enable actual shared tool loop".into())}
    tokio::time::timeout(Duration::from_secs(150),chat.run_tool_turn(&providers,&mcp,skills.clone(),job,events.clone())).await.map_err(|_|"real Provider journey exceeded acceptance deadline")?;
    let run=chat.get_run("default-user",GetChatRunInput{task_id:task}).await.map_err(|error|format!("chat snapshot rejected: {} {}",error.code(),error.message()))?;
    let all=events.0.lock().unwrap();
    let ordered=all.iter().enumerate().all(|(i,e)|e["seq"].as_u64()==Some((i+1)as u64));
    let final_message=all.iter().find(|e|e["kind"]=="done").map(|e|&e["message"]);
    let source_ids:Vec<&str>=run.steps.iter().filter_map(|s|s.receipt.as_ref()).filter_map(|r|r.get("web_activity")).filter_map(|a|a.get("sources").and_then(Value::as_array)).flatten().filter_map(|s|s["id"].as_str()).collect();
    let content=final_message.and_then(|m|m["content"].as_str()).unwrap_or("");
    let cited=source_ids.iter().any(|id|content.contains(&format!("dgos-source:{id}")));
    let exact=if search {run.steps.iter().filter_map(|s|s.receipt.as_ref()).filter_map(|r|r.get("web_activity")).any(|a|a["operation"]=="search"&&a["status"]=="completed")}else{content.to_lowercase().contains("example")&&run.steps.iter().filter_map(|s|s.receipt.as_ref()).any(|r|r.to_string().contains("Example Domain"))};
    let web_completed=run.steps.iter().filter(|s|s.kind=="web"&&s.receipt.as_ref().and_then(|r|r.get("web_activity")).is_some_and(|a|a["status"]=="completed")).count();
    let terminal_count=all.iter().filter(|e|e["kind"]=="done"||e["kind"]=="error").count();
    let completed = run.status == "completed" && final_message.is_some();
    println!("{}",json!({"real_provider":true,"public_https_attempted":true,"public_https_completed":web_completed>0,"live_search_requested":search,"ordered_events":ordered,"terminal_count":terminal_count,"run_status":run.status,"web_completed":web_completed,"source_count":source_ids.len(),"bound_citation":cited,"page_fact_matches":exact,"event_count":all.len(),"tokens":run.total_tokens}));
    let failures=all.iter().filter(|e|e["kind"]=="error").map(|e|json!({"error_code":e["error_code"],"provider_code":e["provider_code"]})).collect::<Vec<_>>();
    if !failures.is_empty(){println!("{}",json!({"terminal_errors":failures}));}
    drop(all);
    let repeated=chat.send_with_mcp(&providers,&*skills.read().await,Some(&mcp),"default-user","d147-isolated-smoke",replay).await.map_err(|error|format!("chat replay rejected: {} {}",error.code(),error.message()))?;
    println!("{}",json!({"same_request_replayed_without_execution":repeated.ack.replayed&&repeated.job.is_none()}));
    if !(completed&&ordered&&terminal_count==1&&web_completed >= (if search {2}else{1})&&cited&&exact&&repeated.ack.replayed&&repeated.job.is_none()) { return Err("real journey did not satisfy the expected evidence; no automatic retry".into()); }
    Ok(())
}
