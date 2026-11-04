use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE asset_leases(\
                 asset_id TEXT NOT NULL,\
                 user_id TEXT NOT NULL,\
                 lease_id TEXT NOT NULL,\
                 expires_at BIGINT NOT NULL,\
                 PRIMARY KEY(asset_id,lease_id),\
                 FOREIGN KEY(asset_id,user_id) REFERENCES chat_assets(id,user_id) ON DELETE CASCADE);\
                 CREATE INDEX idx_asset_leases_asset ON asset_leases(asset_id,user_id,expires_at);",
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("asset leases are forward-only".into()))
    }
}
