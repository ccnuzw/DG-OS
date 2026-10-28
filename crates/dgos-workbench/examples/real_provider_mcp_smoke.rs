//! Explicit opt-in smoke for a configured local Provider plus a real MCP stdio
//! server. The formal database is read only; all task, usage and MCP records
//! are written to a temporary database. The key must be supplied explicitly and
//! stays in a temporary in-memory vault, never copied out of the formal DB.

use async_trait::async_trait;
use dgos_db::{
    migrations::Migrator, sea_orm::Database, sea_orm_migration::MigratorTrait, ModelRepository,
    ProviderRepository,
};
use dgos_mcp::{CreateMcpServerRequest, McpCallPolicy, McpManager, StdioConfig, TransportKind};
use dgos_providers::{
    test_support::{seed_provider, MemoryCredentialVault},
    ProviderManager,
};
use dgos_skills::SkillsManager;
use dgos_types::ProviderConfig;
use dgos_workbench::{TaskInput, ToolAction, ToolSelection, ToolTaskInput, WorkbenchManager};
use std::sync::Arc;

struct Allow;

#[async_trait]
impl McpCallPolicy for Allow {
    async fn allows_call(&self, user: &str, _: &str, _: &str) -> bool {
        user == "default-user"
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let formal_db = std::env::var("DGOS_FORMAL_DB")
        .map_err(|_| "Set DGOS_FORMAL_DB to the existing local DGOS database")?;
    let provider_name = std::env::var("DGOS_REAL_PROVIDER_NAME")
        .map_err(|_| "Set DGOS_REAL_PROVIDER_NAME to an enabled configured Provider")?;
    let model_id = std::env::var("DGOS_REAL_MODEL").unwrap_or_else(|_| "gpt-6.1-sol".into());
    let api_key = std::env::var("DGOS_SMOKE_API_KEY").map_err(|_| {
        "Set DGOS_SMOKE_API_KEY explicitly; Provider DTOs no longer expose credentials"
    })?;
    let formal = dgos_db::establish_connection(&formal_db).await?;
    let source = ProviderRepository::new(formal.clone())
        .list("default-user")
        .await?
        .into_iter()
        .find(|provider| provider.name == provider_name && provider.enabled)
        .ok_or("Configured Provider was not found or is disabled")?;
    let source_model = ModelRepository::new(formal)
        .get(&source.id, &model_id)
        .await?
        .ok_or("Requested model is not cached for the configured Provider")?;

    let dir = tempfile::tempdir()?;
    let db_url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("smoke.db").display()
    );
    let db = Database::connect(&db_url).await?;
    Migrator::up(&db, None).await?;
    let vault = Arc::new(MemoryCredentialVault::default());
    let provider = seed_provider(
        db.clone(),
        &vault,
        ProviderConfig {
            name: format!("{} (isolated smoke)", source.name),
            provider_type: source.provider_type,
            base_url: source.base_url,
            api_key,
            enabled: true,
            config: source.config,
        },
        "default-user",
    )
    .await?;
    ModelRepository::new(db.clone())
        .sync_models(
            &provider.id,
            vec![dgos_types::Model {
                id: uuid::Uuid::new_v4().to_string(),
                provider_id: provider.id.clone(),
                model_id: source_model.model_id.clone(),
                model_name: source_model.model_name,
                model_type: "chat".into(),
                context_window: source_model.context_window,
                max_output_tokens: source_model.max_output_tokens,
                supports_streaming: source_model.supports_streaming,
                supports_vision: source_model.supports_vision,
                cached_at: String::new(),
            }],
        )
        .await?;

    let mcp = McpManager::with_policy(db.clone(), Arc::new(Allow));
    let node = std::env::var("DGOS_MCP_NODE").unwrap_or_else(|_| "node".into());
    let fixture = format!(
        "{}/../dgos-mcp/tests/fixtures/mcp-server.mjs",
        env!("CARGO_MANIFEST_DIR")
    );
    let server = mcp
        .create_server(
            "default-user",
            CreateMcpServerRequest {
                name: "real-provider-smoke-server".into(),
                transport: TransportKind::Stdio,
                stdio: Some(StdioConfig {
                    executable: node,
                    args: vec![fixture, "normal".into()],
                    cwd: None,
                }),
                http: None,
                enabled: Some(true),
                auto_connect: Some(false),
            },
        )
        .await?;
    mcp.connect("default-user", &server.id).await?;
    let tool = mcp
        .list_tools("default-user", &server.id)
        .await?
        .into_iter()
        .next()
        .ok_or("Smoke MCP server exposed no tools")?;

    let providers = ProviderManager::with_vault(&db_url, vault).await?;
    let skills = SkillsManager::new(db.clone(), dir.path().join("skills"));
    let workbench = WorkbenchManager::new(db.clone());
    let input = ToolTaskInput {
        task: TaskInput {
            provider_id: provider.id,
            model_id,
            prompt: "Use the selected echo tool exactly once with {\"text\":\"DGOS-REAL-PROVIDER-MCP-20261028\"}. After the tool result, reply with a short confirmation. Do not answer before calling the tool.".into(),
            skill_id: None,
            allow_without_skill: true,
            client_task_id: None,
        },
        tools: vec![ToolSelection {
            server_id: tool.server_id.clone(),
            tool_name: tool.tool_name.clone(),
            schema_hash: tool.schema_hash.clone(),
        }],
    };
    let started = workbench
        .start_tool_task(&skills, &providers, &mcp, input, "default-user")
        .await?;
    let proposed = workbench
        .advance_tool_task(
            &providers,
            &mcp,
            &started.record.task.id,
            "default-user",
            ToolAction::RunModel,
        )
        .await?;
    let proposal = proposed
        .proposal
        .ok_or("Real Provider did not return a structured tool proposal")?;
    let done = workbench
        .advance_tool_task(
            &providers,
            &mcp,
            &started.record.task.id,
            "default-user",
            ToolAction::Approve {
                proposal_id: proposal.id,
            },
        )
        .await?;
    if done.record.task.status != "succeeded" || done.content.as_deref().unwrap_or("").is_empty() {
        return Err("Real Provider MCP task did not complete with final content".into());
    }
    let logs = mcp
        .list_usage_logs("default-user", Some(&server.id), None)
        .await?;
    if logs.len() != 1 || logs[0].outcome != "success" {
        return Err("Expected exactly one successful redacted MCP usage audit".into());
    }
    let audit = serde_json::to_string(&workbench.list("default-user").await?)?;
    if audit.contains("DGOS-REAL-PROVIDER-MCP-20261028") {
        return Err("Workbench audit unexpectedly retained tool prompt/result content".into());
    }
    println!(
        "Real Provider tools smoke passed: task={}, status={}, tokens={}, audits={}",
        done.record.task.id,
        done.record.task.status,
        done.record.task.total_tokens.unwrap_or_default(),
        logs.len()
    );
    mcp.shutdown().await?;
    workbench.shutdown().await?;
    Ok(())
}
