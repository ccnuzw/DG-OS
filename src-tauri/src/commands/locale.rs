//! Host-level language preference commands (V1-FR-016, D118).
//!
//! These commands are available before account sign-in because the language
//! preference belongs to the host, not to a local user.

use super::CommandResponse;
use crate::host;
use dgos_db::HostSettingsRepository;
use dgos_types::{resolve_locale, DgosError, LanguagePreference, LanguagePreferences};
use tauri::{Manager, WebviewWindow};

const CATALOG_VERSION: &str = "v1";

fn system_hint() -> Option<String> {
    let environment = std::env::var("LC_ALL")
        .ok()
        .or_else(|| std::env::var("LANG").ok());
    if environment.is_some() {
        return environment;
    }

    #[cfg(target_os = "macos")]
    {
        // GUI-launched macOS apps commonly do not inherit LANG/LC_ALL.
        // AppleLocale is the host language source used only for the
        // `system` preference; command failure remains non-fatal.
        std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLocale"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

pub(crate) async fn read(
    repository: &HostSettingsRepository,
) -> Result<LanguagePreferences, DgosError> {
    let preference = repository
        .get_language()
        .await
        .map_err(|error| {
            tracing::error!("language preference read failed: {error}");
            DgosError::InternalError("language preference unavailable".into())
        })?
        .unwrap_or(LanguagePreference::DEFAULT);
    Ok(LanguagePreferences {
        preference,
        locale: resolve_locale(preference, system_hint().as_deref()),
        catalog_version: CATALOG_VERSION,
    })
}

pub(crate) async fn write(
    repository: &HostSettingsRepository,
    preference: LanguagePreference,
) -> Result<LanguagePreferences, DgosError> {
    repository
        .set_language(
            preference,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or_default(),
        )
        .await
        .map_err(|error| {
            tracing::error!("language preference write failed: {error}");
            DgosError::InternalError("language preference unavailable".into())
        })?;
    read(repository).await
}

#[tauri::command]
pub async fn get_language_preferences(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> CommandResponse<LanguagePreferences> {
    if let Err(error) = host::account_window(&window) {
        return CommandResponse::from(Err(error));
    }
    let Some(repository) = app.try_state::<HostSettingsRepository>() else {
        return CommandResponse::from(Err(DgosError::InternalError("language preference unavailable".into())));
    };
    CommandResponse::from(read(&repository).await)
}

#[tauri::command]
pub async fn set_language_preferences(
    app: tauri::AppHandle,
    window: WebviewWindow,
    language: String,
) -> CommandResponse<LanguagePreferences> {
    if let Err(error) = host::account_window(&window) {
        return CommandResponse::from(Err(error));
    }
    let Some(preference) = LanguagePreference::parse(&language) else {
        return CommandResponse::from(Err(DgosError::InvalidLanguagePreference));
    };
    let Some(repository) = app.try_state::<HostSettingsRepository>() else {
        return CommandResponse::from(Err(DgosError::InternalError("language preference unavailable".into())));
    };
    let _permit = match host::admit_activity(&app) { Ok(permit) => permit, Err(error) => return CommandResponse::from(Err(error)) };
    CommandResponse::from(write(&repository, preference).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_db::sea_orm::Database;
    use dgos_db::sea_orm_migration::MigratorTrait;

    async fn repository() -> HostSettingsRepository {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        dgos_db::migrations::Migrator::up(&db, None).await.unwrap();
        HostSettingsRepository::new(db)
    }

    #[tokio::test]
    async fn missing_language_defaults_to_simplified_chinese_without_writing() {
        let repository = repository().await;
        let preferences = read(&repository).await.unwrap();
        assert_eq!(preferences.preference, LanguagePreference::ZhCn);
        assert_eq!(preferences.locale.as_str(), "zh-CN");
        assert_eq!(repository.get("ui.language").await.unwrap(), None);
    }

    #[tokio::test]
    async fn language_round_trip_is_host_scoped() {
        let repository = repository().await;
        let preferences = write(&repository, LanguagePreference::EnUs).await.unwrap();
        assert_eq!(preferences.preference, LanguagePreference::EnUs);
        assert_eq!(preferences.locale.as_str(), "en-US");
        assert_eq!(
            repository.get("ui.language").await.unwrap().as_deref(),
            Some("en-US")
        );
    }
}
