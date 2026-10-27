use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Earlier development builds recorded m004 while using a narrower
        // schema. Reconcile in place; never drop or recreate user data.
        if !manager
            .has_column("skill_catalog", "latest_version")
            .await?
        {
            manager
                .get_connection()
                .execute_unprepared(
                    "ALTER TABLE skill_catalog ADD COLUMN latest_version varchar NOT NULL DEFAULT ''",
                )
                .await?;
        }
        if !manager.has_column("skill_versions", "metadata").await? {
            manager
                .get_connection()
                .execute_unprepared(
                    "ALTER TABLE skill_versions ADD COLUMN metadata json_text NOT NULL DEFAULT '{}'",
                )
                .await?;
        }
        if !manager.has_table("skill_user_states").await? {
            manager
                .get_connection()
                .execute_unprepared(
                    "CREATE TABLE skill_user_states (
                        user_id varchar NOT NULL,
                        skill_id varchar NOT NULL,
                        version_id varchar NOT NULL,
                        enabled boolean NOT NULL,
                        created_at bigint NOT NULL,
                        PRIMARY KEY (user_id, skill_id),
                        FOREIGN KEY (version_id) REFERENCES skill_versions(id)
                    )",
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // This migration repairs deployed schemas and intentionally has no
        // destructive rollback path.
        Ok(())
    }
}
