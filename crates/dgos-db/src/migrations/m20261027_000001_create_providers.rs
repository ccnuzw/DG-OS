use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Providers::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Providers::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Providers::Name).string_len(255).not_null())
                    .col(
                        ColumnDef::new(Providers::ProviderType)
                            .string_len(50)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Providers::BaseUrl)
                            .string_len(512)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Providers::ApiKey)
                            .string_len(512)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Providers::Enabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(Providers::SortOrder)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(Providers::Config).json())
                    .col(
                        ColumnDef::new(Providers::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Providers::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Providers::UserId)
                            .string_len(36)
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index on user_id for faster queries
        manager
            .create_index(
                Index::create()
                    .name("idx_providers_user_id")
                    .table(Providers::Table)
                    .col(Providers::UserId)
                    .to_owned(),
            )
            .await?;

        // Create index on sort_order for ordering
        manager
            .create_index(
                Index::create()
                    .name("idx_providers_sort_order")
                    .table(Providers::Table)
                    .col(Providers::SortOrder)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Providers::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Providers {
    Table,
    Id,
    Name,
    ProviderType,
    BaseUrl,
    ApiKey,
    Enabled,
    SortOrder,
    Config,
    CreatedAt,
    UpdatedAt,
    UserId,
}
