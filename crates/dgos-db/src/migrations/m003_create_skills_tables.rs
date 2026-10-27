use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create skills table
        manager
            .create_table(
                Table::create()
                    .table(Skills::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Skills::Id).string().not_null().primary_key())
                    .col(ColumnDef::new(Skills::SkillId).string().not_null())
                    .col(ColumnDef::new(Skills::Name).string().not_null())
                    .col(ColumnDef::new(Skills::Description).string())
                    .col(ColumnDef::new(Skills::Version).string().not_null())
                    .col(ColumnDef::new(Skills::Author).string())
                    .col(ColumnDef::new(Skills::InstallPath).string().not_null())
                    .col(ColumnDef::new(Skills::CallType).string().not_null())
                    .col(
                        ColumnDef::new(Skills::Enabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(ColumnDef::new(Skills::Config).json())
                    .col(ColumnDef::new(Skills::Manifest).json().not_null())
                    .col(ColumnDef::new(Skills::InstalledAt).big_integer().not_null())
                    .col(ColumnDef::new(Skills::UpdatedAt).big_integer().not_null())
                    .col(ColumnDef::new(Skills::UserId).string().not_null())
                    .to_owned(),
            )
            .await?;

        // Create index on user_id and skill_id
        manager
            .create_index(
                Index::create()
                    .name("idx_skills_user_skill")
                    .table(Skills::Table)
                    .col(Skills::UserId)
                    .col(Skills::SkillId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Create skill_registry_cache table
        manager
            .create_table(
                Table::create()
                    .table(SkillRegistryCache::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SkillRegistryCache::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(SkillRegistryCache::SkillId)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(SkillRegistryCache::Name).string().not_null())
                    .col(ColumnDef::new(SkillRegistryCache::Description).string())
                    .col(
                        ColumnDef::new(SkillRegistryCache::LatestVersion)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(SkillRegistryCache::Author).string())
                    .col(
                        ColumnDef::new(SkillRegistryCache::Category)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(SkillRegistryCache::Tags).json().not_null())
                    .col(
                        ColumnDef::new(SkillRegistryCache::DownloadUrl)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SkillRegistryCache::Manifest)
                            .json()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SkillRegistryCache::SyncedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index on category
        manager
            .create_index(
                Index::create()
                    .name("idx_registry_cache_category")
                    .table(SkillRegistryCache::Table)
                    .col(SkillRegistryCache::Category)
                    .to_owned(),
            )
            .await?;

        // Create skill_usage_logs table
        manager
            .create_table(
                Table::create()
                    .table(SkillUsageLogs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SkillUsageLogs::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(SkillUsageLogs::SkillId).string().not_null())
                    .col(ColumnDef::new(SkillUsageLogs::UserId).string().not_null())
                    .col(
                        ColumnDef::new(SkillUsageLogs::InputParams)
                            .json()
                            .not_null(),
                    )
                    .col(ColumnDef::new(SkillUsageLogs::OutputResult).json())
                    .col(ColumnDef::new(SkillUsageLogs::Success).boolean().not_null())
                    .col(ColumnDef::new(SkillUsageLogs::ErrorMessage).string())
                    .col(
                        ColumnDef::new(SkillUsageLogs::LatencyMs)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SkillUsageLogs::CreatedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index on user_id and created_at for querying logs
        manager
            .create_index(
                Index::create()
                    .name("idx_usage_logs_user_time")
                    .table(SkillUsageLogs::Table)
                    .col(SkillUsageLogs::UserId)
                    .col(SkillUsageLogs::CreatedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SkillUsageLogs::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(SkillRegistryCache::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(Skills::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Skills {
    Table,
    Id,
    SkillId,
    Name,
    Description,
    Version,
    Author,
    InstallPath,
    CallType,
    Enabled,
    Config,
    Manifest,
    InstalledAt,
    UpdatedAt,
    UserId,
}

#[derive(DeriveIden)]
enum SkillRegistryCache {
    Table,
    Id,
    SkillId,
    Name,
    Description,
    LatestVersion,
    Author,
    Category,
    Tags,
    DownloadUrl,
    Manifest,
    SyncedAt,
}

#[derive(DeriveIden)]
enum SkillUsageLogs {
    Table,
    Id,
    SkillId,
    UserId,
    InputParams,
    OutputResult,
    Success,
    ErrorMessage,
    LatencyMs,
    CreatedAt,
}
