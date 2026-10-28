use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(SkillUsageLogs::Table)
                    .add_column(ColumnDef::new(SkillUsageLogs::Caller).string_len(32))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(SkillUsageLogs::Table)
                    .add_column(ColumnDef::new(SkillUsageLogs::RequestId).string_len(64))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(SkillUsageLogs::Table)
                    .add_column(ColumnDef::new(SkillUsageLogs::TaskRef).string_len(64))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(SkillUsageLogs::Table)
                    .drop_column(SkillUsageLogs::TaskRef)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(SkillUsageLogs::Table)
                    .drop_column(SkillUsageLogs::RequestId)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(SkillUsageLogs::Table)
                    .drop_column(SkillUsageLogs::Caller)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum SkillUsageLogs {
    Table,
    Caller,
    RequestId,
    TaskRef,
}
