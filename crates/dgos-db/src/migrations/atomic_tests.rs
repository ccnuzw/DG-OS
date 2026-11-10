use super::*;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};

async fn before_foundation() -> DatabaseConnection {
    let db = crate::establish_connection("sqlite::memory:").await.unwrap();
    let before = Migrator::migrations().iter()
        .position(|migration| migration.name() == "m20261110_000037_ai_foundation").unwrap();
    Migrator::up(&db, Some(before as u32)).await.unwrap();
    db
}

async fn foundation_objects(db: &DatabaseConnection) -> i64 {
    db.query_one(Statement::from_string(DbBackend::Sqlite,
        "SELECT COUNT(*) AS n FROM sqlite_master WHERE name LIKE 'ai_content_%'"))
        .await.unwrap().unwrap().try_get("", "n").unwrap()
}

#[tokio::test]
async fn failed_content_schema_upgrade_rolls_back_and_can_retry() {
    let db = before_foundation().await;
    db.execute_unprepared("CREATE TABLE ai_content_runs(marker TEXT); INSERT INTO ai_content_runs VALUES('preserve existing data');").await.unwrap();
    assert!(Migrator::up(&db, None).await.is_err());
    assert_eq!(foundation_objects(&db).await, 1, "failed migration must not leave earlier DDL behind");
    let row = db.query_one(Statement::from_string(DbBackend::Sqlite, "SELECT marker FROM ai_content_runs")).await.unwrap().unwrap();
    assert_eq!(row.try_get::<String>("", "marker").unwrap(), "preserve existing data");
    // Only the fixture removes its deliberately injected conflicting table.
    db.execute_unprepared("DROP TABLE ai_content_runs").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    assert_eq!(Migrator::get_pending_migrations(&db).await.unwrap().len(), 0);
}

#[tokio::test]
async fn failed_version_registration_rolls_back_schema_and_can_retry() {
    let db = before_foundation().await;
    db.execute_unprepared("CREATE TRIGGER reject_foundation_registration BEFORE INSERT ON seaql_migrations WHEN NEW.version='m20261110_000037_ai_foundation' BEGIN SELECT RAISE(ABORT,'injected registration failure'); END;").await.unwrap();
    assert!(Migrator::up(&db, None).await.is_err());
    assert_eq!(foundation_objects(&db).await, 0, "schema and migration receipt must commit together");
    db.execute_unprepared("DROP TRIGGER reject_foundation_registration").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    assert_eq!(Migrator::get_pending_migrations(&db).await.unwrap().len(), 0);
}
