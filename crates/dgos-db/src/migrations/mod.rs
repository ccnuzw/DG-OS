pub use sea_orm_migration::prelude::*;

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

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
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
        ]
    }
}
