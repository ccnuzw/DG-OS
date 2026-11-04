use crate::{migrations::Migrator, DesktopRepository, DesktopSave, DesktopValues};
use sea_orm::{ConnectionTrait, Database};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn desktop_order_survives_reopen_and_cas_cannot_overwrite_another_account() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", root.path().join("preferences.db").display());
    let db = Database::connect(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let first = DesktopRepository::new(db.clone());
    let values = DesktopValues { pinned_module_ids: vec!["second".into(), "first".into()], show_entries: false, show_recent: true, sidebar_collapsed: true };
    assert!(matches!(first.save("alice", values.clone(), 0).await.unwrap(), DesktopSave::Saved { .. }));
    let reopened = DesktopRepository::new(Database::connect(&url).await.unwrap());
    assert_eq!(reopened.get("alice").await.unwrap().values, values);
    assert_eq!(reopened.get("bob").await.unwrap().revision, 0);
    let (a, b) = tokio::join!(first.save("alice", DesktopValues::default(), 1), reopened.save("alice", values, 1));
    assert_eq!([a.unwrap(), b.unwrap()].iter().filter(|result| matches!(result, DesktopSave::Saved { .. })).count(), 1);
    assert_eq!(reopened.get("alice").await.unwrap().revision, 2);
    assert_eq!(reopened.get("bob").await.unwrap().revision, 0);
    let before = reopened.get("alice").await.unwrap();
    db.execute_unprepared("CREATE TRIGGER deny_desktop BEFORE UPDATE ON system_settings BEGIN SELECT RAISE(ABORT,'fixture'); END").await.unwrap();
    assert!(reopened.save("alice", DesktopValues::default(), 2).await.is_err());
    assert_eq!(reopened.get("alice").await.unwrap(), before);
}

#[tokio::test]
async fn desktop_rejects_duplicate_overlimit_and_corrupt_preferences_without_default_write() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let repo = DesktopRepository::new(db.clone());
    for pins in [vec!["a", "a"], vec!["a", "b", "c", "d"], vec!["../escape"]] {
        let value = DesktopValues { pinned_module_ids: pins.into_iter().map(str::to_owned).collect(), ..Default::default() };
        assert!(repo.save("alice", value, 0).await.is_err());
    }
    db.execute_unprepared("INSERT INTO system_settings(user_id,setting_key,setting_value,updated_at) VALUES ('alice','desktop.snapshot.v1','corrupt',0)").await.unwrap();
    assert!(repo.get("alice").await.is_err());
    assert!(repo.save("alice", DesktopValues::default(), 0).await.is_err());
    let row = db.query_one(sea_orm::Statement::from_string(sea_orm::DbBackend::Sqlite, "SELECT setting_value FROM system_settings WHERE user_id='alice'".to_owned())).await.unwrap().unwrap();
    assert_eq!(row.try_get::<String>("", "setting_value").unwrap(), "corrupt");
}
