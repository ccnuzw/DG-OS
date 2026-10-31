use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // D081. The token columns are NOT NULL DEFAULT 0, so a call whose upstream never reported
        // usage (cancelled, interrupted, or simply silent) was indistinguishable from a free one.
        // `usage_reported` says which it was: 1 the upstream reported usage, 0 it did not (the
        // zeros are unknown, not free), NULL a row written before this column existed. Additive
        // and forward-only; rebuilding the table to make the token columns nullable is avoided.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE provider_usage_logs ADD COLUMN usage_reported INTEGER \
                 CHECK (usage_reported IN (0,1));",
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "provider_usage_logs.usage_reported is forward-only".into(),
        ))
    }
}
