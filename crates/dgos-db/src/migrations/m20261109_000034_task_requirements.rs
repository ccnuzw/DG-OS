//! §13.1 first-send tasks have no completion requirements until explicitly edited.
//! Replace only this non-key column: dropping the parent table would disturb the
//! incoming task foreign keys. Keep enforcement enabled throughout the transaction.
use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::TransactionTrait;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let tx = manager.get_connection().begin().await?;
        let result = tx.execute_unprepared(r#"
ALTER TABLE development_tasks ADD COLUMN requirements_next TEXT NOT NULL DEFAULT ''
 CHECK(length(CAST(requirements_next AS BLOB)) BETWEEN 0 AND 16384);
UPDATE development_tasks SET requirements_next = requirements;
ALTER TABLE development_tasks DROP COLUMN requirements;
ALTER TABLE development_tasks RENAME COLUMN requirements_next TO requirements;
"#).await;
        match result {
            Ok(_) => tx.commit().await,
            Err(error) => { tx.rollback().await?; Err(error) }
        }
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("optional task requirements are forward-only".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{Migrator, MigratorTrait};
    use crate::development_repository::{DevelopmentRepository, DevelopmentTask, Project, Selection, StoredAgentSession};
    use sea_orm_migration::sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};

    async fn before_upgrade(url: &str) -> DatabaseConnection {
        let db = crate::establish_connection(url).await.unwrap();
        let boundary = Migrator::migrations().iter().position(|migration|
            migration.name() == "m20261107_000033_git_pull").unwrap() + 1;
        Migrator::up(&db, Some(boundary as u32)).await.unwrap();
        db
    }

    async fn seed(db: &DatabaseConnection) -> (DevelopmentRepository, DevelopmentTask, StoredAgentSession, Selection) {
        let repo = DevelopmentRepository::new(db.clone());
        let project = Project { id: "project".into(), owner: "alice".into(), name: "Code".into(), path: "/tmp/optional-requirements".into(), directory_identity: "identity".into(), archived: false, created_at: 1, last_opened_at: 1 };
        repo.insert_project(&project).await.unwrap();
        let task = DevelopmentTask { id: "task".into(), owner: project.owner.clone(), project_id: project.id.clone(), goal: "First request".into(), requirements: "Existing requirements\nUnicode 保留".into(), status: "review".into(), created_at: 7 };
        repo.save_task(&task).await.unwrap();
        let session = StoredAgentSession { id: "session".into(), owner: project.owner.clone(), engine: "codex".into(), native_id: Some("original-native".into()), workspace: project.path, executable: None, model: None, project_id: Some(project.id.clone()), task_id: Some(task.id.clone()) };
        repo.save_session(&session).await.unwrap();
        let selection = Selection { project_id: Some(project.id), task_id: Some(task.id.clone()), session_id: Some(session.id.clone()) };
        repo.select("alice", &selection).await.unwrap();
        repo.save_review_baseline("alice", &task.id, &serde_json::json!({"fingerprint":"preserved"})).await.unwrap();
        (repo, task, session, selection)
    }

    async fn assert_foreign_keys(db: &DatabaseConnection) {
        let row = db.query_one(Statement::from_string(DbBackend::Sqlite, "PRAGMA foreign_keys")).await.unwrap().unwrap();
        assert_eq!(row.try_get::<i64>("", "foreign_keys").unwrap(), 1);
        assert!(db.query_all(Statement::from_string(DbBackend::Sqlite, "PRAGMA foreign_key_check")).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn task_requirements_upgrade_preserves_owned_children_and_reopens_empty_requirements() {
        let dir = tempfile::tempdir().unwrap();
        let url = crate::startup::database_url(&dir.path().join("upgrade.sqlite"));
        let db = before_upgrade(&url).await;
        let (repo, task, session, selection) = seed(&db).await;
        let mut empty = task.clone(); empty.id = "empty".into(); empty.requirements.clear();
        // Repository validation is already relaxed, but the old physical schema refuses it.
        assert!(repo.save_task(&empty).await.is_err());
        Migrator::up(&db, None).await.unwrap();
        assert_eq!(repo.task("alice", &task.id).await.unwrap(), task);
        assert_eq!(repo.sessions().await.unwrap(), [session.clone()]);
        assert_eq!(repo.selection("alice").await.unwrap(), selection);
        assert_eq!(repo.review_baseline("alice", &task.id).await.unwrap(), Some(serde_json::json!({"fingerprint":"preserved"})));
        repo.save_task(&empty).await.unwrap();
        let mut edited = task.clone(); edited.requirements.clear();
        repo.save_task(&edited).await.unwrap();
        assert_foreign_keys(&db).await;
        drop(repo);
        db.close().await.unwrap();
        let reopened = crate::establish_connection(&url).await.unwrap();
        Migrator::up(&reopened, None).await.unwrap();
        let repo = DevelopmentRepository::new(reopened.clone());
        assert_eq!(repo.task("alice", &task.id).await.unwrap(), edited);
        assert_eq!(repo.task("alice", &empty.id).await.unwrap(), empty);
        assert_eq!(repo.sessions().await.unwrap(), [session]);
        assert_eq!(repo.selection("alice").await.unwrap(), selection);
        assert!(repo.task("bob", &task.id).await.is_err());
        assert_foreign_keys(&reopened).await;
        drop(repo);
        reopened.close().await.unwrap();
    }

    #[tokio::test]
    async fn task_requirements_empty_does_not_relax_other_fields_or_owner_foreign_keys() {
        let db = crate::establish_connection("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let (repo, task, _, _) = seed(&db).await;
        let mut empty = task.clone(); empty.id = "empty".into(); empty.requirements.clear();
        repo.save_task(&empty).await.unwrap();
        assert_eq!(repo.task("alice", &empty.id).await.unwrap(), empty);
        for (goal, requirements, status) in [
            (String::new(), String::new(), "todo"),
            ("g".repeat(8193), String::new(), "todo"),
            ("goal".into(), "é".repeat(8193), "todo"),
            ("goal".into(), String::new(), "invalid"),
        ] {
            let invalid = DevelopmentTask { id: "invalid".into(), goal, requirements, status: status.into(), ..task.clone() };
            assert_eq!(repo.save_task(&invalid).await.unwrap_err(), "invalid_task");
        }
        let boundary = DevelopmentTask { id: "boundary".into(), requirements: "é".repeat(8192), ..task.clone() };
        repo.save_task(&boundary).await.unwrap();
        assert_eq!(repo.task("alice", &boundary.id).await.unwrap(), boundary);
        for (owner, project_id) in [("bob", "project"), ("alice", "missing"), ("", "project")] {
            let invalid = DevelopmentTask { id: "foreign".into(), owner: owner.into(), project_id: project_id.into(), requirements: String::new(), ..task.clone() };
            assert!(repo.save_task(&invalid).await.is_err());
        }
        // Exercise physical constraints independently of repository input validation.
        for (column, value) in [("goal", "''"), ("goal", "hex(zeroblob(4097))"), ("requirements", "hex(zeroblob(8193))"), ("requirements", "NULL"), ("status", "'invalid'"), ("owner", "''"), ("owner", "'bob'"), ("project_id", "'missing'")] {
            assert!(db.execute_unprepared(&format!("UPDATE development_tasks SET {column}={value} WHERE id='empty'")).await.is_err(), "{column}");
        }
        assert_eq!(repo.task("alice", &empty.id).await.unwrap(), empty);
        assert_foreign_keys(&db).await;
    }

    #[tokio::test]
    async fn task_requirements_failed_column_replacement_rolls_back_without_losing_rows() {
        let db = before_upgrade("sqlite::memory:").await;
        let (repo, task, session, selection) = seed(&db).await;
        // An unknown dependent view must block DROP COLUMN, not be silently removed.
        db.execute_unprepared("CREATE VIEW external_requirements AS SELECT requirements FROM development_tasks").await.unwrap();
        assert!(Migration.up(&SchemaManager::new(&db)).await.is_err());
        assert_eq!(repo.task("alice", &task.id).await.unwrap(), task);
        assert_eq!(repo.sessions().await.unwrap(), [session]);
        assert_eq!(repo.selection("alice").await.unwrap(), selection);
        let columns = db.query_all(Statement::from_string(DbBackend::Sqlite, "PRAGMA table_info(development_tasks)")).await.unwrap();
        assert!(!columns.iter().any(|row| row.try_get::<String>("", "name").unwrap() == "requirements_next"));
        assert_foreign_keys(&db).await;
        db.execute_unprepared("DROP VIEW external_requirements").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let mut empty = task.clone(); empty.requirements.clear();
        repo.save_task(&empty).await.unwrap();
        assert_eq!(repo.task("alice", &task.id).await.unwrap(), empty);
    }
}
