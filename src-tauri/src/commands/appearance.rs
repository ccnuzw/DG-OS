//! Appearance preference commands (V1-FR-005 §12.1, D065).
//!
//! Two typed commands, no generic configuration access. The only parameters
//! are the Tauri context and, for writes, a `patch` object that the host
//! validates against the closed theme/density sets. The user is always the
//! host-derived one; the renderer can neither name a user nor a settings key.

use super::CommandResponse;
use crate::host::{self, Capability};
use dgos_db::AppearanceRepository;
use dgos_types::{AppearancePatch, AppearancePreferences, BuiltinCaller, DgosError};
use tauri::{Manager, WebviewWindow};

/// Storage failures reach the renderer as a fixed message; the driver text
/// (SQL, paths) goes to the log only.
fn storage_unavailable(operation: &str, error: DgosError) -> DgosError {
    tracing::error!("appearance {operation} failed: {error}");
    DgosError::InternalError("appearance storage unavailable".into())
}

fn is_storage_failure(error: &DgosError) -> bool {
    matches!(error, DgosError::DatabaseError(_) | DgosError::JsonError(_))
}

pub(crate) async fn read_preferences(
    repository: &AppearanceRepository,
    user_id: &str,
) -> Result<AppearancePreferences, DgosError> {
    repository
        .get(user_id)
        .await
        .map_err(|error| storage_unavailable("read", error))
}

pub(crate) async fn write_preferences(
    repository: &AppearanceRepository,
    user_id: &str,
    raw_patch: &serde_json::Value,
) -> Result<AppearancePreferences, DgosError> {
    // Validation first: a rejected patch never reaches storage.
    let patch = AppearancePatch::parse(raw_patch)?;
    repository.apply(user_id, &patch).await.map_err(|error| {
        if is_storage_failure(&error) {
            storage_unavailable("write", error)
        } else {
            error
        }
    })
}

/// Both preferences of the host-derived user; missing or invalid stored values read as defaults.
#[tauri::command]
pub async fn get_appearance_preferences(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> CommandResponse<AppearancePreferences> {
    let context = match host::context(&window, BuiltinCaller::Settings, Capability::Appearance) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let repository = app.state::<AppearanceRepository>();
    CommandResponse::from(read_preferences(&repository, context.user_id()).await)
}

/// Persists the fields present in `patch` in one transaction and returns the complete preferences.
#[tauri::command]
pub async fn set_appearance_preferences(
    app: tauri::AppHandle,
    window: WebviewWindow,
    patch: serde_json::Value,
) -> CommandResponse<AppearancePreferences> {
    let context = match host::context(&window, BuiltinCaller::Settings, Capability::Appearance) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let repository = app.state::<AppearanceRepository>();
    CommandResponse::from(write_preferences(&repository, context.user_id(), &patch).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_db::sea_orm::{ConnectionTrait, Database};
    use dgos_db::sea_orm_migration::MigratorTrait;
    use serde_json::json;

    async fn repository() -> (dgos_db::sea_orm::DatabaseConnection, AppearanceRepository) {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        dgos_db::migrations::Migrator::up(&db, None).await.unwrap();
        (db.clone(), AppearanceRepository::new(db))
    }

    fn envelope<T: serde::Serialize>(result: Result<T, DgosError>) -> serde_json::Value {
        serde_json::to_value(CommandResponse::from(result)).unwrap()
    }

    #[tokio::test]
    async fn host_commands_round_trip_partial_and_pair_patches_for_the_host_user() {
        let (_db, repo) = repository().await;
        let user = host::LOCAL_USER;

        let first = envelope(read_preferences(&repo, user).await);
        assert_eq!(first["data"], json!({ "theme": "light", "density": 100 }));

        let themed = envelope(write_preferences(&repo, user, &json!({ "theme": "dark" })).await);
        assert_eq!(themed["success"], true);
        assert_eq!(themed["data"], json!({ "theme": "dark", "density": 100 }));

        let paired = envelope(
            write_preferences(&repo, user, &json!({ "theme": "auto", "density": 150 })).await,
        );
        assert_eq!(paired["data"], json!({ "theme": "auto", "density": 150 }));

        let read = envelope(read_preferences(&repo, user).await);
        assert_eq!(read["data"], paired["data"]);
    }

    #[tokio::test]
    async fn invalid_patches_return_their_codes_and_write_nothing() {
        let (db, repo) = repository().await;
        let user = host::LOCAL_USER;
        write_preferences(&repo, user, &json!({ "theme": "dark", "density": 125 }))
            .await
            .unwrap();

        for (patch, code) in [
            (json!({ "theme": "neon" }), 1005001),
            (json!({ "density": 90 }), 1005006),
            (json!({ "theme": "light", "density": "100" }), 1005006),
            (json!({}), 1005002),
            (json!({ "user_id": "mallory", "theme": "light" }), 1005002),
            (
                json!({ "setting_key": "http_port", "theme": "light" }),
                1005002,
            ),
            (json!("light"), 1005002),
        ] {
            let response = envelope(write_preferences(&repo, user, &patch).await);
            assert_eq!(response["success"], false, "{patch}");
            assert_eq!(response["error"]["code"], code, "{patch}");
            assert!(response["data"].is_null());
        }

        assert_eq!(
            envelope(read_preferences(&repo, user).await)["data"],
            json!({ "theme": "dark", "density": 125 })
        );
        let count = db
            .query_one(dgos_db::sea_orm::Statement::from_string(
                db.get_database_backend(),
                "SELECT COUNT(*) AS n FROM system_settings".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(count.try_get::<i64>("", "n").unwrap(), 2);
    }

    #[tokio::test]
    async fn database_failures_return_a_fixed_message_without_driver_text() {
        let (db, repo) = repository().await;
        db.execute_unprepared("DROP TABLE system_settings")
            .await
            .unwrap();

        let read = envelope(read_preferences(&repo, host::LOCAL_USER).await);
        let write =
            envelope(write_preferences(&repo, host::LOCAL_USER, &json!({ "theme": "dark" })).await);
        for response in [read, write] {
            assert_eq!(response["success"], false);
            assert_eq!(response["error"]["code"], 1000000);
            let message = response["error"]["message"].as_str().unwrap();
            assert!(!message.contains("system_settings"), "{message}");
            assert!(!message.to_lowercase().contains("sql"), "{message}");
        }
    }
}
