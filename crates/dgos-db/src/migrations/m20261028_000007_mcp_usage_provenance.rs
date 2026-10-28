use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(McpUsageLogs::Table)
                    .add_column(ColumnDef::new(McpUsageLogs::Caller).string_len(32))
                    .to_owned(),
            )
            .await
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(McpUsageLogs::Table)
                    .drop_column(McpUsageLogs::Caller)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum McpUsageLogs {
    Table,
    Caller,
}
