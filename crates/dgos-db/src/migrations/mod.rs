pub use sea_orm_migration::prelude::*;
use sea_orm::TransactionTrait;

#[cfg(test)]
mod atomic_tests;

mod m003_create_skills_tables;
mod m004_create_skill_content_tables;
mod m005_create_workbench_tasks;
mod m006_align_provider_usage_logs;
mod m007_reconcile_skill_content_schema;
mod m008_create_mcp_tables;
mod m009_create_mcp_connection_logs;
mod m010_create_workbench_task_tools;
mod m20261027_000001_create_providers;
mod m20261027_000002_create_models;
mod m20261027_000003_create_usage_logs;
mod m20261028_000004_provider_credentials;
mod m20261028_000005_provider_revisions;
mod m20261028_000006_provider_usage_provenance;
mod m20261028_000007_mcp_usage_provenance;
mod m20261028_000008_skill_usage_provenance;
mod m20261028_000009_module_permissions;
mod m20261028_000010_module_release_history;
mod m20261028_000011_module_release_locks;
mod m20261028_000012_module_release_operation;
mod m20261029_000013_module_crash_logs;
mod m20261029_000014_system_settings;
mod m20261030_000015_model_kind_and_source;
mod m20261030_000016_chat_sessions_and_messages;
mod m20261031_000017_usage_reported;
mod m20261101_000018_module_storage;
mod m20261101_000019_local_accounts;
mod m20261101_000020_lan_audit_events;
mod m20261102_000021_skill_install_operations;
mod m20261104_000022_generation_snapshots;
mod m20261104_000023_chat_assets;
mod m20261104_000024_asset_leases;
mod m20261104_000026_project_tasks;
mod m20261104_000027_executions_review;
mod m20261104_000028_development_runtime;
mod m20261104_000029_project_capabilities;
mod m20261104_000030_task_workspaces;
mod m20261107_000031_ai_runs;
mod m20261107_000032_cli_provider_cutover;
mod m20261107_000033_git_pull;
mod m20261109_000034_task_requirements;
mod m20261109_000035_git_restore;
mod m20261109_000036_chat_continuous;
mod m20261110_000037_ai_foundation;
mod m20261110_000038_codex_operations;
mod m20261110_000039_development_desktop;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    async fn up<'c, C>(db: C, steps: Option<u32>) -> Result<(), DbErr>
    where
        C: IntoSchemaManagerConnection<'c>,
    {
        // SQLite upgrades must commit schema changes and version receipts together.
        let db = db.into_schema_manager_connection();
        let tx = db.begin().await?;
        match MigrationRunner::up(&tx, steps).await {
            Ok(()) => tx.commit().await,
            Err(error) => {
                tx.rollback().await?;
                Err(error)
            }
        }
    }

    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261027_000001_create_providers::Migration),
            Box::new(m20261027_000002_create_models::Migration),
            Box::new(m20261027_000003_create_usage_logs::Migration),
            Box::new(m003_create_skills_tables::Migration),
            Box::new(m004_create_skill_content_tables::Migration),
            Box::new(m005_create_workbench_tasks::Migration),
            Box::new(m006_align_provider_usage_logs::Migration),
            Box::new(m007_reconcile_skill_content_schema::Migration),
            Box::new(m008_create_mcp_tables::Migration),
            Box::new(m009_create_mcp_connection_logs::Migration),
            Box::new(m010_create_workbench_task_tools::Migration),
            Box::new(m20261028_000004_provider_credentials::Migration),
            Box::new(m20261028_000005_provider_revisions::Migration),
            Box::new(m20261028_000006_provider_usage_provenance::Migration),
            Box::new(m20261028_000007_mcp_usage_provenance::Migration),
            Box::new(m20261028_000008_skill_usage_provenance::Migration),
            Box::new(m20261028_000009_module_permissions::Migration),
            Box::new(m20261028_000010_module_release_history::Migration),
            Box::new(m20261028_000011_module_release_locks::Migration),
            Box::new(m20261028_000012_module_release_operation::Migration),
            Box::new(m20261029_000013_module_crash_logs::Migration),
            Box::new(m20261029_000014_system_settings::Migration),
            Box::new(m20261030_000015_model_kind_and_source::Migration),
            Box::new(m20261030_000016_chat_sessions_and_messages::Migration),
            Box::new(m20261031_000017_usage_reported::Migration),
            Box::new(m20261101_000018_module_storage::Migration),
            Box::new(m20261101_000019_local_accounts::Migration),
            Box::new(m20261101_000020_lan_audit_events::Migration),
            Box::new(m20261102_000021_skill_install_operations::Migration),
            Box::new(m20261104_000022_generation_snapshots::Migration),
            Box::new(m20261104_000023_chat_assets::Migration),
            Box::new(m20261104_000024_asset_leases::Migration),
            Box::new(m20261104_000026_project_tasks::Migration),
            Box::new(m20261104_000027_executions_review::Migration),
            Box::new(m20261104_000028_development_runtime::Migration),
            Box::new(m20261104_000029_project_capabilities::Migration),
            Box::new(m20261104_000030_task_workspaces::Migration),
            Box::new(m20261107_000031_ai_runs::Migration),
            Box::new(m20261107_000032_cli_provider_cutover::Migration),
            Box::new(m20261107_000033_git_pull::Migration),
            Box::new(m20261109_000034_task_requirements::Migration),
            Box::new(m20261109_000035_git_restore::Migration),
            Box::new(m20261109_000036_chat_continuous::Migration),
            Box::new(m20261110_000037_ai_foundation::Migration),
            Box::new(m20261110_000038_codex_operations::Migration),
            Box::new(m20261110_000039_development_desktop::Migration),
        ]
    }
}

// Reuse SeaORM ordering/history without recursively calling the override.
// Transactions inside individual migrations use savepoints under this boundary.
struct MigrationRunner;
#[async_trait::async_trait]
impl MigratorTrait for MigrationRunner {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        Migrator::migrations()
    }
}
