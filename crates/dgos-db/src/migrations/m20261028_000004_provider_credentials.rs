use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Nullable keeps old installations readable while the host performs
        // an explicit, retryable vault migration per provider.
        manager
            .alter_table(
                Table::alter()
                    .table(Providers::Table)
                    .add_column(ColumnDef::new(Providers::CredentialRef).string_len(128))
                    .to_owned(),
            )
            .await?;
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE provider_credential_cleanup (
                credential_ref TEXT PRIMARY KEY NOT NULL,
                user_id TEXT NOT NULL, provider_id TEXT NOT NULL, endpoint TEXT NOT NULL
            );",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let _ = manager;
        // A rollback cannot reconstruct secrets from the OS vault, and dropping
        // the reference/outbox would strand them. Require an explicit restore.
        Err(DbErr::Custom(
            "Provider credential migration is forward-only".into(),
        ))
    }
}

#[derive(DeriveIden)]
enum Providers {
    Table,
    CredentialRef,
}
