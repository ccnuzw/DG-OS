use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProviderUsageLogs::Table)
                    .add_column(ColumnDef::new(ProviderUsageLogs::Caller).string_len(32))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ProviderUsageLogs::Table)
                    .add_column(ColumnDef::new(ProviderUsageLogs::RequestId).string_len(36))
                    .to_owned(),
            )
            .await
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProviderUsageLogs::Table)
                    .drop_column(ProviderUsageLogs::RequestId)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ProviderUsageLogs::Table)
                    .drop_column(ProviderUsageLogs::Caller)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum ProviderUsageLogs {
    Table,
    Caller,
    RequestId,
}
