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

#[tauri::command]
pub async fn get_account_state(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> CommandResponse<AccountState> {
    if let Err(error) = host::account_window(&window) {
        return respond(Err(error));
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
}
