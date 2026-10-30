use dgos_db::{
    migrations::Migrator, sea_orm::Database, sea_orm_migration::MigratorTrait, ModelRepository,
};
use dgos_providers::{
    test_support::{seed_provider, MemoryCredentialVault},
    ProviderManager,
};
use dgos_skills::SkillsManager;
use dgos_types::{Model, ProviderConfig};
use dgos_workbench::{TaskInput, WorkbenchManager};

// Explicit opt-in, fixed non-sensitive material, temporary database. Never read
// the user's normal DGOS database or print credentials/base URLs.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_url =
        std::env::var("DGOS_SMOKE_BASE_URL").map_err(|_| "Set DGOS_SMOKE_BASE_URL explicitly")?;
    let api_key =
        std::env::var("DGOS_SMOKE_API_KEY").map_err(|_| "Set DGOS_SMOKE_API_KEY explicitly")?;
    let model_id =
        std::env::var("DGOS_SMOKE_MODEL").map_err(|_| "Set DGOS_SMOKE_MODEL explicitly")?;
    let dir = tempfile::tempdir()?;
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("smoke.db").display()
    );
    let db = Database::connect(&url).await?;
    Migrator::up(&db, None).await?;
    let vault = std::sync::Arc::new(MemoryCredentialVault::default());
    let provider = seed_provider(
        db.clone(),
        &vault,
        ProviderConfig {
            name: "Explicit workbench smoke".into(),
            provider_type: "openai_compatible".into(),
            base_url,
            api_key,
            enabled: true,
            config: None,
        },
        "smoke-user",
    )
    .await?;
    ModelRepository::new(db.clone())
        .sync_models(
            &provider.id,
            vec![Model {
                id: uuid::Uuid::new_v4().to_string(),
                provider_id: provider.id.clone(),
                model_id: model_id.clone(),
                model_name: model_id.clone(),
                model_type: "chat".into(),
                context_window: None,
                max_output_tokens: None,
                supports_streaming: false,
                supports_vision: false,
                cached_at: String::new(),
                ..Default::default()
            }],
        )
        .await?;
    let skills = SkillsManager::new(db.clone(), dir.path().join("skills"));
    skills
        .register_bundled_catalog(
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills"),
        )
        .await?;
    let skill_id = "dgos.official.writing.summarize";
    skills.install_skill(skill_id, "smoke-user").await?;
    let providers = ProviderManager::with_vault(&url, vault).await?;
    let workbench = WorkbenchManager::new(db);
    let outcome = workbench.run(&skills, &providers, TaskInput {
        provider_id: provider.id, model_id, skill_id: Some(skill_id.into()), allow_without_skill: false, client_task_id: None,
        prompt: "请用中文总结以下材料，区分已确认事项和待确认事项。材料：团队会议决定周五发布第一版；负责人为林。预算尚未批准。不要补充材料外的信息。".into(),
    }, "smoke-user").await.map_err(std::io::Error::other)?;
    if outcome.record.task.status != "succeeded" {
        return Err(format!("Model task failed: {:?}", outcome.record.task.error_code).into());
    }
    println!(
        "Task {} succeeded; bound version: {}; hash: {}",
        outcome.record.task.id,
        outcome.record.skills[0].version_id,
        outcome.record.skills[0].content_hash
    );
    println!(
        "Model output (fixed non-sensitive input; review quality manually):\n{}",
        outcome.content.unwrap_or_default()
    );
    Ok(())
}
