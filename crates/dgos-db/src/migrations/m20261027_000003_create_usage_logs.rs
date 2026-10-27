use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(UsageLogs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UsageLogs::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(UsageLogs::ProviderId)
                            .string_len(36)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UsageLogs::ModelId)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(ColumnDef::new(UsageLogs::UserId).string_len(36).not_null())
                    .col(
                        ColumnDef::new(UsageLogs::RequestTokens)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(UsageLogs::ResponseTokens)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(UsageLogs::TotalTokens)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(UsageLogs::LatencyMs)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(UsageLogs::Success).boolean().not_null())
                    .col(ColumnDef::new(UsageLogs::ErrorCode).string_len(50))
                    .col(
                        ColumnDef::new(UsageLogs::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_usage_logs_provider_id")
                            .from(UsageLogs::Table, UsageLogs::ProviderId)
                            .to(Providers::Table, Providers::Id),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index on provider_id for analytics
        manager
            .create_index(
                Index::create()
                    .name("idx_usage_logs_provider_id")
                    .table(UsageLogs::Table)
                    .col(UsageLogs::ProviderId)
                    .to_owned(),
            )
            .await?;

        // Create index on user_id for user-specific queries
        manager
            .create_index(
                Index::create()
                    .name("idx_usage_logs_user_id")
                    .table(UsageLogs::Table)
                    .col(UsageLogs::UserId)
                    .to_owned(),
            )
            .await?;

        // Create index on created_at for time-based queries
        manager
            .create_index(
                Index::create()
                    .name("idx_usage_logs_created_at")
                    .table(UsageLogs::Table)
                    .col(UsageLogs::CreatedAt)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(UsageLogs::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum UsageLogs {
    Table,
    Id,
    ProviderId,
    ModelId,
    UserId,
    RequestTokens,
    ResponseTokens,
    TotalTokens,
    LatencyMs,
    Success,
    ErrorCode,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Providers {
    Table,
    Id,
}
