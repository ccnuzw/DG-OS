//! Local account commands (V1-FR-008, interface contract §9, D087, D090).
//!
//! Six commands behind `host::account_window` (trusted main window only, no
//! signed-in requirement, no caller or capability policy). None takes a user
//! id, session token or account id: `username` is a selector checked
//! together with the password, and change password and delete act on the
//! signed-in user only. Which command may run is decided by the account mode
//! in `AccountService`. Passwords are zeroed `Password` values that never
//! reach a log, error message or DTO.

use super::provider_commands::{CommandError, CommandResponse};
use crate::host;
use dgos_application::{
    AccountService, AccountState, ChangePasswordInput, CreateAccountInput, DeleteAccountInput,
    DeleteAccountResult, SignInInput,
};
use dgos_types::DgosError;
use serde::Serialize;
use tauri::{Manager, WebviewWindow};

/// Like `CommandResponse::from`, plus the error detail (`1008011` carries
/// `retry_after_seconds`).
fn respond<T: Serialize>(result: Result<T, DgosError>) -> CommandResponse<T> {
    match result {
        Ok(data) => CommandResponse {
            success: true,
            data: Some(data),
            error: None,
        },
        Err(error) => CommandResponse {
            success: false,
            data: None,
            error: Some(CommandError {
                code: error.error_code(),
                message: error.to_string(),
                detail: error.detail(),
            }),
        },
    }
}

fn accounts(app: &tauri::AppHandle) -> Result<AccountService, DgosError> {
    app.try_state::<AccountService>()
        .map(|service| service.inner().clone())
        .ok_or(DgosError::SessionInvalid)
}

/// Managed instead of every other state when startup could not open or
/// migrate the database (FR-008 §10.3, D098 item 9, D116). `reason` has no
/// path and no row data; `path` is the database file the user has to look at.
pub struct DatabaseUnavailable {
    pub stage: &'static str,
    pub reason: String,
    pub path: String,
}

impl DatabaseUnavailable {
    fn error(&self) -> DgosError {
        DgosError::DatabaseUnavailable {
            stage: self.stage.to_owned(),
            reason: self.reason.clone(),
            path: self.path.clone(),
        }
    }
}

/// The account gate reads this first; a database failure is reported here so
/// the window can show it. The other account commands stay `1008007`.
#[tauri::command]
pub async fn get_account_state(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> CommandResponse<AccountState> {
    if let Err(error) = host::account_window(&window) {
        return respond(Err(error));
    }
    if let Some(failure) = app.try_state::<DatabaseUnavailable>() {
        return respond(Err(failure.error()));
    }
    respond(async { accounts(&app)?.state().await }.await)
}

#[tauri::command]
pub async fn create_account(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: CreateAccountInput,
) -> CommandResponse<AccountState> {
    if let Err(error) = host::account_window(&window) {
        return respond(Err(error));
    }
    let _permit = match host::admit_activity(&app) { Ok(permit) => permit, Err(error) => return respond(Err(error)) };
    respond(async { accounts(&app)?.create(input).await }.await)
}

#[tauri::command]
pub async fn sign_in(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: SignInInput,
) -> CommandResponse<AccountState> {
    if let Err(error) = host::account_window(&window) {
        return respond(Err(error));
    }
    let _permit = match host::admit_activity(&app) { Ok(permit) => permit, Err(error) => return respond(Err(error)) };
    respond(async { accounts(&app)?.sign_in(input).await }.await)
}

#[tauri::command]
pub async fn sign_out(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> CommandResponse<AccountState> {
    if let Err(error) = host::account_window(&window) {
        return respond(Err(error));
    }
    let _permit = match host::admit_activity(&app) { Ok(permit) => permit, Err(error) => return respond(Err(error)) };
    respond(async { accounts(&app)?.sign_out().await }.await)
}

#[tauri::command]
pub async fn change_password(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: ChangePasswordInput,
) -> CommandResponse<AccountState> {
    if let Err(error) = host::account_window(&window) {
        return respond(Err(error));
    }
    let _permit = match host::admit_activity(&app) { Ok(permit) => permit, Err(error) => return respond(Err(error)) };
    respond(async { accounts(&app)?.change_password(input).await }.await)
}

#[tauri::command]
pub async fn delete_account(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: DeleteAccountInput,
) -> CommandResponse<DeleteAccountResult> {
    if let Err(error) = host::account_window(&window) {
        return respond(Err(error));
    }
    let _permit = match host::admit_activity(&app) { Ok(permit) => permit, Err(error) => return respond(Err(error)) };
    respond(async { accounts(&app)?.delete(input).await }.await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttling_reaches_the_renderer_with_its_retry_seconds_only() {
        let response = serde_json::to_value(respond::<()>(Err(DgosError::SignInThrottled {
            retry_after_seconds: 45,
        })))
        .unwrap();
        assert_eq!(response["success"], false);
        assert_eq!(response["error"]["code"], 1008011);
        assert_eq!(response["error"]["detail"]["retry_after_seconds"], 45);
        let rejected = serde_json::to_value(respond::<()>(Err(DgosError::SignInRejected))).unwrap();
        assert_eq!(rejected["error"]["code"], 1008005);
        assert!(rejected["error"].get("detail").is_none());
    }

    #[test]
    fn a_database_failure_reaches_the_renderer_with_stage_reason_and_path_only_in_detail() {
        let failure = DatabaseUnavailable {
            stage: "migrate",
            reason: "Custom Error: Migration file is missing".into(),
            path: "/data/dgos.db".into(),
        };
        let response = serde_json::to_value(respond::<()>(Err(failure.error()))).unwrap();
        assert_eq!(response["error"]["code"], 1008012);
        assert_eq!(response["error"]["detail"]["stage"], "migrate");
        assert_eq!(
            response["error"]["detail"]["reason"],
            "Custom Error: Migration file is missing"
        );
        assert_eq!(response["error"]["detail"]["path"], "/data/dgos.db");
        assert!(!response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("dgos.db"));
    }
}
