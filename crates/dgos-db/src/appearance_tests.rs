//! AC02 / AC04 / AC21 host side: appearance preferences against real SQLite.

use crate::{migrations::Migrator, AppearanceRepository};
use dgos_types::{AppearancePatch, AppearancePreferences, Density, Theme};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement};
use sea_orm_migration::MigratorTrait;
use serde_json::{json, Value};

const USER: &str = "default-user";

async fn memory() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

fn patch(raw: Value) -> AppearancePatch {
    AppearancePatch::parse(&raw).unwrap()
}

fn prefs(theme: Theme, density: u16) -> AppearancePreferences {
    AppearancePreferences {
        theme,
        density: Density::new(density).unwrap(),
    }
}

async fn rows(db: &DatabaseConnection, sql: &str) -> Vec<sea_orm::QueryResult> {
    db.query_all(Statement::from_string(
        db.get_database_backend(),
        sql.to_owned(),
    ))
    .await
    .unwrap()
}

#[tokio::test]
async fn appearance_migration_fixes_the_schema_decision() {
    let db = memory().await;
    let columns = rows(&db, "PRAGMA table_info(system_settings)").await;
    let column = |name: &str| {
        let row = columns
            .iter()
            .find(|row| row.try_get::<String>("", "name").unwrap() == name)
            .unwrap_or_else(|| panic!("missing column {name}"));
        (
            row.try_get::<String>("", "type").unwrap(),
            row.try_get::<i32>("", "notnull").unwrap(),
        )
    };
    assert_eq!(column("user_id"), ("TEXT".into(), 1));
    assert_eq!(column("setting_key"), ("TEXT".into(), 1));
    assert_eq!(column("setting_value"), ("TEXT".into(), 1));
    assert_eq!(column("updated_at"), ("INTEGER".into(), 1));

    let unique = rows(&db, "PRAGMA index_list(system_settings)").await;
    assert!(unique.iter().any(|row| {
        row.try_get::<String>("", "name").unwrap() == "idx_system_settings_unique"
            && row.try_get::<i32>("", "unique").unwrap() == 1
    }));
    // No default rows are seeded: defaults live in the host.
    assert!(rows(&db, "SELECT 1 FROM system_settings").await.is_empty());
}

#[tokio::test]
async fn appearance_migration_upgrades_an_existing_database_without_touching_it() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let before = Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m20261029_000014_system_settings")
        .unwrap();
    Migrator::up(&db, Some(before as u32)).await.unwrap();
    assert!(rows(
        &db,
        "SELECT name FROM sqlite_master WHERE name='system_settings'"
    )
    .await
    .is_empty());
    db.execute_unprepared("INSERT INTO providers (id, name, provider_type, base_url, api_key, enabled, sort_order, user_id, created_at, updated_at) VALUES ('legacy-provider', 'Legacy', 'openai_compatible', 'https://example.invalid/v1', '', 1, 0, 'alice', '2026-10-27T00:00:00+00:00', '2026-10-27T00:00:00+00:00');")
        .await
        .unwrap();

    Migrator::up(&db, None).await.unwrap();

    assert_eq!(
        rows(&db, "SELECT id FROM providers WHERE id='legacy-provider'")
            .await
            .len(),
        1
    );
    let repo = AppearanceRepository::new(db);
    assert_eq!(
        repo.get(USER).await.unwrap(),
        AppearancePreferences::default()
    );
}

#[tokio::test]
async fn appearance_defaults_to_light_and_100_when_nothing_is_stored() {
    let repo = AppearanceRepository::new(memory().await);
    assert_eq!(repo.get(USER).await.unwrap(), prefs(Theme::Light, 100));
}

#[tokio::test]
async fn appearance_partial_updates_leave_the_other_field_alone() {
    let repo = AppearanceRepository::new(memory().await);

    let after_theme = repo
        .apply(USER, &patch(json!({ "theme": "dark" })))
        .await
        .unwrap();
    assert_eq!(after_theme, prefs(Theme::Dark, 100));

    let after_density = repo
        .apply(USER, &patch(json!({ "density": 150 })))
        .await
        .unwrap();
    assert_eq!(after_density, prefs(Theme::Dark, 150));

    let after_auto = repo
        .apply(USER, &patch(json!({ "theme": "auto" })))
        .await
        .unwrap();
    assert_eq!(after_auto, prefs(Theme::Auto, 150));
    assert_eq!(repo.get(USER).await.unwrap(), after_auto);
}

#[tokio::test]
async fn appearance_repeated_writes_keep_one_row_per_key() {
    let db = memory().await;
    let repo = AppearanceRepository::new(db.clone());
    for theme in ["dark", "light", "dark"] {
        repo.apply(USER, &patch(json!({ "theme": theme, "density": 75 })))
            .await
            .unwrap();
    }
    let stored = rows(
        &db,
        "SELECT setting_key, setting_value FROM system_settings ORDER BY setting_key",
    )
    .await;
    let pairs: Vec<(String, String)> = stored
        .iter()
        .map(|row| {
            (
                row.try_get("", "setting_key").unwrap(),
                row.try_get("", "setting_value").unwrap(),
            )
        })
        .collect();
    // Values are JSON: a quoted string and a bare number.
    assert_eq!(
        pairs,
        vec![
            ("appearance.density".into(), "75".into()),
            ("appearance.theme".into(), "\"dark\"".into()),
        ]
    );
}

#[tokio::test]
async fn appearance_pair_patch_commits_both_or_neither() {
    let db = memory().await;
    let repo = AppearanceRepository::new(db.clone());
    repo.apply(USER, &patch(json!({ "theme": "dark", "density": 125 })))
        .await
        .unwrap();

    // The density write fails after the theme write already succeeded in the same transaction.
    db.execute_unprepared(
        "CREATE TRIGGER fail_density BEFORE UPDATE ON system_settings \
         WHEN NEW.setting_key = 'appearance.density' \
         BEGIN SELECT RAISE(ABORT, 'scripted failure'); END;",
    )
    .await
    .unwrap();

    let error = repo
        .apply(USER, &patch(json!({ "theme": "light", "density": 175 })))
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1000000);

    // Neither half of the pair was persisted.
    assert_eq!(repo.get(USER).await.unwrap(), prefs(Theme::Dark, 125));
}

#[tokio::test]
async fn appearance_first_write_failure_leaves_no_partial_rows() {
    let db = memory().await;
    db.execute_unprepared(
        "CREATE TRIGGER fail_density BEFORE INSERT ON system_settings \
         WHEN NEW.setting_key = 'appearance.density' \
         BEGIN SELECT RAISE(ABORT, 'scripted failure'); END;",
    )
    .await
    .unwrap();
    let repo = AppearanceRepository::new(db.clone());

    assert!(repo
        .apply(USER, &patch(json!({ "theme": "dark", "density": 150 })))
        .await
        .is_err());

    assert!(rows(&db, "SELECT 1 FROM system_settings").await.is_empty());
    assert_eq!(
        repo.get(USER).await.unwrap(),
        AppearancePreferences::default()
    );
}

#[tokio::test]
async fn appearance_survives_closing_and_reopening_the_database_file() {
    let dir = std::env::temp_dir().join(format!("dgos-appearance-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.join("dgos.db").display());

    {
        let db = crate::establish_connection(&url).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        AppearanceRepository::new(db.clone())
            .apply(USER, &patch(json!({ "theme": "dark", "density": 175 })))
            .await
            .unwrap();
        db.close().await.unwrap();
    }

    // A fresh connection and a second migration run, as at the next application start.
    let db = crate::establish_connection(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let repo = AppearanceRepository::new(db.clone());
    assert_eq!(repo.get(USER).await.unwrap(), prefs(Theme::Dark, 175));

    db.close().await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn appearance_invalid_stored_values_fall_back_per_field() {
    let db = memory().await;
    let repo = AppearanceRepository::new(db.clone());
    let cases: [(&str, &str, AppearancePreferences); 6] = [
        ("\"sepia\"", "125", prefs(Theme::Light, 125)),
        ("not json", "125", prefs(Theme::Light, 125)),
        ("\"dark\"", "\"125\"", prefs(Theme::Dark, 100)),
        ("\"dark\"", "90", prefs(Theme::Dark, 100)),
        ("\"dark\"", "125.5", prefs(Theme::Dark, 100)),
        ("null", "null", prefs(Theme::Light, 100)),
    ];
    for (theme, density, expected) in cases {
        db.execute_unprepared("DELETE FROM system_settings")
            .await
            .unwrap();
        for (key, value) in [("appearance.theme", theme), ("appearance.density", density)] {
            db.execute(Statement::from_sql_and_values(
                db.get_database_backend(),
                "INSERT INTO system_settings (user_id, setting_key, setting_value, updated_at) VALUES (?, ?, ?, 0)",
                [USER.into(), key.into(), value.into()],
            ))
            .await
            .unwrap();
        }
        assert_eq!(
            repo.get(USER).await.unwrap(),
            expected,
            "{theme} / {density}"
        );
    }
}

#[tokio::test]
async fn appearance_is_scoped_to_the_user_and_ignores_other_keys() {
    let db = memory().await;
    let repo = AppearanceRepository::new(db.clone());
    repo.apply("alice", &patch(json!({ "theme": "dark", "density": 75 })))
        .await
        .unwrap();
    db.execute_unprepared(
        "INSERT INTO system_settings (user_id, setting_key, setting_value, updated_at) VALUES ('default-user', 'http_port', '3000', 0)",
    )
    .await
    .unwrap();

    assert_eq!(
        repo.get(USER).await.unwrap(),
        AppearancePreferences::default()
    );
    assert_eq!(repo.get("alice").await.unwrap(), prefs(Theme::Dark, 75));
    repo.apply(USER, &patch(json!({ "theme": "auto" })))
        .await
        .unwrap();
    assert_eq!(repo.get("alice").await.unwrap(), prefs(Theme::Dark, 75));
    // The unrelated key is untouched.
    assert_eq!(
        rows(
            &db,
            "SELECT setting_value FROM system_settings WHERE setting_key='http_port'"
        )
        .await
        .len(),
        1
    );
}

#[tokio::test]
async fn appearance_database_failure_surfaces_as_an_error_not_defaults() {
    let db = memory().await;
    let repo = AppearanceRepository::new(db.clone());
    repo.apply(USER, &patch(json!({ "theme": "dark" })))
        .await
        .unwrap();
    db.execute_unprepared("DROP TABLE system_settings")
        .await
        .unwrap();

    assert_eq!(repo.get(USER).await.unwrap_err().error_code(), 1000000);
    assert_eq!(
        repo.apply(USER, &patch(json!({ "density": 100 })))
            .await
            .unwrap_err()
            .error_code(),
        1000000
    );
}
