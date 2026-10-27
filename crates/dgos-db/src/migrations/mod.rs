pub use sea_orm_migration::prelude::*;

mod m003_create_skills_tables;
mod m004_create_skill_content_tables;
mod m005_create_workbench_tasks;
mod m006_align_provider_usage_logs;
mod m007_reconcile_skill_content_schema;
mod m20261027_000001_create_providers;
mod m20261027_000002_create_models;
mod m20261027_000003_create_usage_logs;

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
        ]
    }
}
