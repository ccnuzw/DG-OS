//! Admit the explicit index/worktree restoration receipts without changing old migrations.
use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::TransactionTrait;
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let tx = manager.get_connection().begin().await?;
        tx.execute_unprepared(r#"
CREATE TABLE development_project_operations_git_restore(
 id TEXT PRIMARY KEY NOT NULL, owner TEXT NOT NULL CHECK(length(owner)>0),
 kind TEXT NOT NULL CHECK(kind IN ('create','clone','workspace_create','workspace_import','workspace_select','workspace_remove','git_stage','git_unstage','git_untrack','git_restore','git_restore_hunk','git_commit','git_push','git_pull','git_switch','git_merge','git_discard')),
 path TEXT NOT NULL CHECK(length(path)>0), status TEXT NOT NULL CHECK(status IN ('pending','succeeded','failed','unknown')),
 project_id TEXT, task_id TEXT, workspace_id TEXT, request_fingerprint TEXT, details TEXT);
INSERT INTO development_project_operations_git_restore(id,owner,project_id,task_id,workspace_id,kind,path,status,request_fingerprint,details)
 SELECT id,owner,project_id,task_id,workspace_id,kind,path,status,request_fingerprint,details FROM development_project_operations;
DROP TABLE development_project_operations;
ALTER TABLE development_project_operations_git_restore RENAME TO development_project_operations;
CREATE INDEX idx_development_operations_owner ON development_project_operations(owner);
CREATE INDEX idx_development_operations_task ON development_project_operations(owner,task_id);
"#).await?;
        tx.commit().await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Git restoration receipts are forward-only".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm_migration::sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement, DbBackend};

    async fn old_schema() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        // Start at the existing pull schema, including all receipt fields/indexes.
        db.execute_unprepared("CREATE TABLE development_project_operations(id TEXT PRIMARY KEY NOT NULL,owner TEXT NOT NULL CHECK(length(owner)>0),kind TEXT NOT NULL CHECK(kind IN ('git_pull','git_stage')),path TEXT NOT NULL CHECK(length(path)>0),status TEXT NOT NULL CHECK(status IN ('pending','succeeded','failed','unknown')),project_id TEXT,task_id TEXT,workspace_id TEXT,request_fingerprint TEXT,details TEXT); CREATE INDEX idx_development_operations_owner ON development_project_operations(owner); CREATE INDEX idx_development_operations_task ON development_project_operations(owner,task_id); INSERT INTO development_project_operations VALUES('old','alice','git_pull','git','unknown','p','t','w','fingerprint','{\"phase\":\"fetched\"}');").await.unwrap();
        db
    }
    #[tokio::test]
    async fn upgrade_preserves_unknown_receipt_and_admits_only_valid_restore_kinds() {
        let db = old_schema().await;
        Migration.up(&SchemaManager::new(&db)).await.unwrap();
        let row = db.query_one(Statement::from_string(DbBackend::Sqlite, "SELECT owner,kind,status,project_id,task_id,workspace_id,request_fingerprint,details FROM development_project_operations WHERE id='old'")).await.unwrap().unwrap();
        for (field, value) in [("owner","alice"),("kind","git_pull"),("status","unknown"),("project_id","p"),("task_id","t"),("workspace_id","w"),("request_fingerprint","fingerprint"),("details","{\"phase\":\"fetched\"}")] { assert_eq!(row.try_get::<String>("", field).unwrap(), value); }
        for kind in ["git_untrack", "git_restore", "git_restore_hunk"] {
            db.execute_unprepared(&format!("INSERT INTO development_project_operations(id,owner,kind,path,status) VALUES('{kind}','alice','{kind}','git','pending')")).await.unwrap();
        }
        for invalid in ["INSERT INTO development_project_operations(id,owner,kind,path,status) VALUES('bad','','git_restore','git','pending')", "INSERT INTO development_project_operations(id,owner,kind,path,status) VALUES('bad','alice','invalid','git','pending')", "INSERT INTO development_project_operations(id,owner,kind,path,status) VALUES('bad','alice','git_restore','','pending')", "INSERT INTO development_project_operations(id,owner,kind,path,status) VALUES('bad','alice','git_restore','git','invalid')"] { assert!(db.execute_unprepared(invalid).await.is_err()); }
    }
    #[tokio::test]
    async fn failed_upgrade_rolls_back_without_losing_receipts() {
        let db = old_schema().await;
        db.execute_unprepared("CREATE TABLE development_project_operations_git_restore(blocked TEXT)").await.unwrap();
        assert!(Migration.up(&SchemaManager::new(&db)).await.is_err());
        db.execute_unprepared("UPDATE development_project_operations SET status='pending' WHERE id='old'; DROP TABLE development_project_operations_git_restore").await.unwrap();
        Migration.up(&SchemaManager::new(&db)).await.unwrap();
    }
}
