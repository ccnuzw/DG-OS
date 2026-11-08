//! Add explicit pull receipts without rewriting migration 030.
use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::TransactionTrait;
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let tx = manager.get_connection().begin().await?;
        tx.execute_unprepared(r#"
CREATE TABLE development_project_operations_git_pull(
 id TEXT PRIMARY KEY NOT NULL, owner TEXT NOT NULL CHECK(length(owner)>0),
 kind TEXT NOT NULL CHECK(kind IN ('create','clone','workspace_create','workspace_import','workspace_select','workspace_remove','git_stage','git_unstage','git_commit','git_push','git_pull','git_switch','git_merge','git_discard')),
 path TEXT NOT NULL CHECK(length(path)>0), status TEXT NOT NULL CHECK(status IN ('pending','succeeded','failed','unknown')),
 project_id TEXT, task_id TEXT, workspace_id TEXT, request_fingerprint TEXT, details TEXT);
INSERT INTO development_project_operations_git_pull(id,owner,project_id,task_id,workspace_id,kind,path,status,request_fingerprint,details)
 SELECT id,owner,project_id,task_id,workspace_id,kind,path,status,request_fingerprint,details FROM development_project_operations;
DROP TABLE development_project_operations;
ALTER TABLE development_project_operations_git_pull RENAME TO development_project_operations;
CREATE INDEX idx_development_operations_owner ON development_project_operations(owner);
CREATE INDEX idx_development_operations_task ON development_project_operations(owner,task_id);
"#).await?;
        tx.commit().await
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Git pull receipts are forward-only".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm_migration::sea_orm::{ConnectionTrait, Database};

    async fn old_schema() -> sea_orm_migration::sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("CREATE TABLE development_project_operations(id TEXT PRIMARY KEY NOT NULL,owner TEXT NOT NULL CHECK(length(owner)>0),kind TEXT NOT NULL CHECK(kind IN ('create','clone','workspace_create','workspace_import','workspace_select','workspace_remove','git_stage','git_unstage','git_commit','git_push','git_switch','git_merge','git_discard')),path TEXT NOT NULL CHECK(length(path)>0),status TEXT NOT NULL CHECK(status IN ('pending','succeeded','failed','unknown')),project_id TEXT,task_id TEXT,workspace_id TEXT,request_fingerprint TEXT,details TEXT); CREATE INDEX idx_development_operations_owner ON development_project_operations(owner); CREATE INDEX idx_development_operations_task ON development_project_operations(owner,task_id); INSERT INTO development_project_operations VALUES('old','alice','git_push','git','unknown','p','t','w','fingerprint','{\"phase\":\"pending\"}');").await.unwrap();
        db
    }

    #[tokio::test]
    async fn upgrade_preserves_receipt_fields_and_accepts_pull_only_with_original_constraints() {
        let db = old_schema().await;
        Migration.up(&SchemaManager::new(&db)).await.unwrap();
        let row = db.query_one(sea_orm_migration::sea_orm::Statement::from_string(sea_orm_migration::sea_orm::DbBackend::Sqlite, "SELECT details,workspace_id,request_fingerprint FROM development_project_operations WHERE id='old'")).await.unwrap().unwrap();
        assert_eq!(row.try_get::<String>("", "details").unwrap(), "{\"phase\":\"pending\"}");
        assert_eq!(row.try_get::<String>("", "workspace_id").unwrap(), "w");
        assert_eq!(row.try_get::<String>("", "request_fingerprint").unwrap(), "fingerprint");
        db.execute_unprepared("INSERT INTO development_project_operations(id,owner,kind,path,status) VALUES('pull','alice','git_pull','git','pending')").await.unwrap();
        for invalid in ["INSERT INTO development_project_operations(id,owner,kind,path,status) VALUES('bad','','git_pull','git','pending')", "INSERT INTO development_project_operations(id,owner,kind,path,status) VALUES('bad','alice','invalid','git','pending')", "INSERT INTO development_project_operations(id,owner,kind,path,status) VALUES('bad','alice','git_pull','','pending')"] { assert!(db.execute_unprepared(invalid).await.is_err()); }
    }

    #[tokio::test]
    async fn failed_upgrade_rolls_back_and_is_retryable() {
        let db = old_schema().await;
        db.execute_unprepared("CREATE TABLE development_project_operations_git_pull(blocked TEXT)").await.unwrap();
        assert!(Migration.up(&SchemaManager::new(&db)).await.is_err());
        db.execute_unprepared("UPDATE development_project_operations SET status='unknown' WHERE id='old'; DROP TABLE development_project_operations_git_pull").await.unwrap();
        Migration.up(&SchemaManager::new(&db)).await.unwrap();
    }
}
