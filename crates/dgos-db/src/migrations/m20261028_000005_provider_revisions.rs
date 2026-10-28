use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in [Providers::ConfigRevision, Providers::ModelRefreshRevision] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Providers::Table)
                        .add_column(ColumnDef::new(column).big_integer().not_null().default(0))
                        .to_owned(),
                )
                .await?;
        }
        manager
            .alter_table(
                Table::alter()
                    .table(WorkbenchTasks::Table)
                    .add_column(
                        ColumnDef::new(WorkbenchTasks::ProviderConfigRevision).big_integer(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(WorkbenchTasks::Table)
                    .drop_column(WorkbenchTasks::ProviderConfigRevision)
                    .to_owned(),
            )
            .await?;
        for column in [Providers::ModelRefreshRevision, Providers::ConfigRevision] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Providers::Table)
                        .drop_column(column)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Providers {
    Table,
    ConfigRevision,
    ModelRefreshRevision,
}

#[derive(DeriveIden)]
enum WorkbenchTasks {
    Table,
    ProviderConfigRevision,
}
