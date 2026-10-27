use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Models::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Models::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Models::ProviderId).string_len(36).not_null())
                    .col(ColumnDef::new(Models::ModelId).string_len(255).not_null())
                    .col(ColumnDef::new(Models::ModelName).string_len(255).not_null())
                    .col(ColumnDef::new(Models::Capabilities).json().not_null())
                    .col(ColumnDef::new(Models::ContextLength).integer())
                    .col(
                        ColumnDef::new(Models::IsAvailable)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(Models::SyncedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_models_provider_id")
                            .from(Models::Table, Models::ProviderId)
                            .to(Providers::Table, Providers::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create unique index on provider_id + model_id
        manager
            .create_index(
                Index::create()
                    .name("idx_models_provider_model")
                    .table(Models::Table)
                    .col(Models::ProviderId)
                    .col(Models::ModelId)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Models::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Models {
    Table,
    Id,
    ProviderId,
    ModelId,
    ModelName,
    Capabilities,
    ContextLength,
    IsAvailable,
    SyncedAt,
}

#[derive(DeriveIden)]
enum Providers {
    Table,
    Id,
}
