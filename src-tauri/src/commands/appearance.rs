//! Full appearance snapshot IPC (D126). The host derives the user and owns revision.
use super::CommandResponse;
use crate::host::{self, Capability};
use dgos_db::AppearanceRepository;
use dgos_types::{AppearanceRead, AppearanceSave, AppearanceSnapshot, AppearanceValues, BuiltinCaller, DgosError};
use tauri::{Manager, WebviewWindow};

fn storage_error(error: DgosError) -> DgosError {
    if matches!(error, DgosError::DatabaseError(_) | DgosError::JsonError(_)) {
        tracing::error!("appearance storage operation failed");
        DgosError::InternalError("appearance storage unavailable".into())
    } else { error }
}
pub(crate) async fn read_preferences(repository: &AppearanceRepository, user: &str) -> Result<AppearanceRead,DgosError> {
    repository.get(user).await.map_err(storage_error)
}
pub(crate) async fn write_preferences(repository: &AppearanceRepository, user: &str, raw: &serde_json::Value, expected_revision: u64) -> Result<AppearanceSave,DgosError> {
    let values = AppearanceValues::parse(raw)?;
    repository.apply(user,&values,expected_revision).await.map_err(storage_error)
}
#[tauri::command]
pub async fn get_appearance_preferences(app: tauri::AppHandle, window: WebviewWindow) -> CommandResponse<AppearanceRead> {
    let context = match host::context(&window,BuiltinCaller::Settings,Capability::Appearance) { Ok(c) => c, Err(e) => return CommandResponse::from(Err(e)) };
    let repository = app.state::<AppearanceRepository>();
    CommandResponse::from(read_preferences(&repository,context.user_id()).await)
}
#[tauri::command]
pub async fn set_appearance_preferences(app: tauri::AppHandle, window: WebviewWindow, snapshot: serde_json::Value, expected_revision: u64) -> CommandResponse<AppearanceSave> {
    let context = match host::context(&window,BuiltinCaller::Settings,Capability::Appearance) { Ok(c) => c, Err(e) => return CommandResponse::from(Err(e)) };
    let _permit = match host::admit_activity(&app) { Ok(permit) => permit, Err(error) => return CommandResponse::from(Err(error)) };
    let repository = app.state::<AppearanceRepository>();
    CommandResponse::from(write_preferences(&repository,context.user_id(),&snapshot,expected_revision).await)
}
#[tauri::command]
pub async fn recover_appearance_preferences(app: tauri::AppHandle, window: WebviewWindow, recovery_token: String) -> CommandResponse<AppearanceSnapshot> {
    let context = match host::context(&window,BuiltinCaller::Settings,Capability::Appearance) { Ok(c) => c, Err(e) => return CommandResponse::from(Err(e)) };
    let repository = app.state::<AppearanceRepository>();
    CommandResponse::from(repository.recover(context.user_id(),&recovery_token).await.map_err(storage_error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_db::{sea_orm::{Database,ConnectionTrait},sea_orm_migration::MigratorTrait};
    #[tokio::test]
    async fn storage_failure_does_not_expose_sql_or_paths() {
        let db=Database::connect("sqlite::memory:").await.unwrap();
        dgos_db::migrations::Migrator::up(&db,None).await.unwrap();
        let repo=AppearanceRepository::new(db.clone());
        db.execute_unprepared("DROP TABLE system_settings").await.unwrap();
        let error=read_preferences(&repo,"default-user").await.unwrap_err();
        assert_eq!(error.error_code(),1000000);
        assert_eq!(error.to_string(),"Internal error: appearance storage unavailable");
    }
}
