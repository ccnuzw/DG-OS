pub use sea_orm_migration::prelude::*;

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
        ]
    }
}
