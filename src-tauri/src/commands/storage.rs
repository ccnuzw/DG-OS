//! Storage statistics and cleanup commands (D133 item 5).
//!
//! The account is resolved by the host from the trusted session. The renderer
//! supplies only the category, an optional cutoff, and a preview id. VACUUM is
//! not exposed here; it is a separate maintenance command.

use super::provider_commands::{CommandError, CommandResponse};
use crate::host::{self, Capability};
use dgos_application::storage::{
    CategoryOverview, CategoryStatus, CleanupPreview, CleanupResult, CleanupStatus,
    StorageError, StorageService,
};
use dgos_db::repositories::storage_repository::StorageCategory;
use dgos_types::DgosError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{Manager, WebviewWindow};

/// Storage failures use the existing generic `DatabaseError` code. The error
/// table has no dedicated storage entries, and 1008012 belongs to recovery.
fn storage_error_code() -> u32 {
    DgosError::DatabaseError(String::new()).error_code()
}

#[derive(Serialize)]
pub struct StorageCategoryDto {
    pub category: &'static str,
    pub status: &'static str,
    pub count: i64,
    pub bytes: Option<i64>,
}

#[derive(Serialize)]
pub struct StorageOverviewDto {
    pub categories: Vec<StorageCategoryDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewStorageCleanupInput {
    pub category: String,
    pub cutoff_days: Option<i64>,
}

#[derive(Serialize)]
pub struct StorageCleanupPreviewDto {
    pub preview_id: String,
    pub category: &'static str,
    pub cutoff: i64,
    pub expires_at: i64,
    pub count: i64,
    pub bytes: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteStorageCleanupInput {
    pub preview_id: String,
}

#[derive(Serialize)]
pub struct StorageCleanupResultDto {
    pub category: &'static str,
    pub status: &'static str,
    pub deleted: i64,
    pub failed: i64,
}

#[derive(Serialize)]
pub struct ExecuteStorageCleanupDto {
    pub result: StorageCleanupResultDto,
    pub overview: StorageOverviewDto,
}

fn respond<T: Serialize>(result: Result<T, StorageError>) -> CommandResponse<T> {
    match result {
        Ok(data) => CommandResponse { success: true, data: Some(data), error: None },
        Err(error) => CommandResponse {
            success: false,
            data: None,
            error: Some(CommandError {
                code: storage_error_code(),
                message: error.to_string(),
                detail: None,
            }),
        },
    }
}

fn denied<T: Serialize>(error: DgosError) -> CommandResponse<T> {
    CommandResponse {
        success: false,
        data: None,
        error: Some(CommandError {
            code: error.error_code(),
            message: error.to_string(),
            detail: None,
        }),
    }
}

fn service(app: &tauri::AppHandle) -> Result<Arc<StorageService>, StorageError> {
    app.try_state::<Arc<StorageService>>()
        .map(|state| state.inner().clone())
        .ok_or(StorageError::Database)
}

fn parse_category(value: &str) -> Result<StorageCategory, StorageError> {
    match value {
        "old_chats" => Ok(StorageCategory::OldChats),
        "session_assets" => Ok(StorageCategory::SessionAssets),
        "temp_files" => Ok(StorageCategory::TempFiles),
        "cache" => Ok(StorageCategory::Cache),
        "activity_records" => Ok(StorageCategory::ActivityRecords),
        _ => Err(StorageError::PreviewNotFound),
    }
}

fn category_key(category: StorageCategory) -> &'static str {
    category.key()
}

fn status_key(status: &CategoryStatus) -> &'static str {
    match status {
        CategoryStatus::Ok => "ok",
        CategoryStatus::Empty => "empty",
        CategoryStatus::NotApplicable => "not_applicable",
    }
}

fn overview_dto(rows: Vec<CategoryOverview>) -> StorageOverviewDto {
    StorageOverviewDto {
        categories: rows
            .into_iter()
            .map(|row| StorageCategoryDto {
                category: category_key(row.category),
                status: status_key(&row.status),
                count: row.count,
                bytes: row.bytes,
            })
            .collect(),
    }
}

fn cleanup_status_key(status: &CleanupStatus) -> &'static str {
    match status {
        CleanupStatus::Done => "done",
        CleanupStatus::Partial => "partial",
        CleanupStatus::Failed => "failed",
    }
}

fn result_dto(result: CleanupResult) -> StorageCleanupResultDto {
    StorageCleanupResultDto {
        category: category_key(result.category),
        status: cleanup_status_key(&result.status),
        deleted: result.deleted,
        failed: result.failed,
    }
}

fn preview_dto(preview: CleanupPreview) -> StorageCleanupPreviewDto {
    StorageCleanupPreviewDto {
        preview_id: preview.preview_id,
        category: category_key(preview.category),
        cutoff: preview.cutoff,
        expires_at: preview.expires_at,
        count: preview.count,
        bytes: preview.bytes,
    }
}

#[tauri::command]
pub async fn get_storage_overview(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> CommandResponse<StorageOverviewDto> {
    let context = match host::module_session(&window, Capability::StorageManage) {
        Ok(context) => context,
        Err(error) => return denied(error),
    };
    let service = match service(&app) {
        Ok(service) => service,
        Err(error) => return respond(Err(error)),
    };
    respond(service.overview(context.user_id()).await.map(overview_dto))
}

#[tauri::command]
pub async fn preview_storage_cleanup(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: PreviewStorageCleanupInput,
) -> CommandResponse<StorageCleanupPreviewDto> {
    let context = match host::module_session(&window, Capability::StorageManage) {
        Ok(context) => context,
        Err(error) => return denied(error),
    };
    let category = match parse_category(&input.category) {
        Ok(category) => category,
        Err(error) => return respond(Err(error)),
    };
    let service = match service(&app) {
        Ok(service) => service,
        Err(error) => return respond(Err(error)),
    };
    respond(
        service
            .preview(context.user_id(), category, input.cutoff_days)
            .await
            .map(preview_dto),
    )
}

#[tauri::command]
pub async fn execute_storage_cleanup(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: ExecuteStorageCleanupInput,
) -> CommandResponse<ExecuteStorageCleanupDto> {
    let context = match host::module_session(&window, Capability::StorageManage) {
        Ok(context) => context,
        Err(error) => return denied(error),
    };
    let _permit = match host::admit_activity(&app) {
        Ok(permit) => permit,
        Err(error) => return denied(error),
    };
    let service = match service(&app) {
        Ok(service) => service,
        Err(error) => return respond(Err(error)),
    };
    let user = context.user_id().to_owned();
    let result = match service.execute(&user, &input.preview_id).await {
        Ok(result) => result,
        Err(error) => return respond(Err(error)),
    };
    // Statistics are read again after execution so the renderer never shows
    // the pre-delete totals as the result.
    match service.overview(&user).await {
        Ok(rows) => respond(Ok(ExecuteStorageCleanupDto {
            result: result_dto(result),
            overview: overview_dto(rows),
        })),
        Err(error) => respond(Err(error)),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteVacuumInput {
    pub preview_id: String,
    pub confirmed: bool,
}

#[derive(Serialize)]
pub struct VacuumPreviewDto {
    pub preview_id: String,
    pub expires_at: i64,
}

#[derive(Serialize)]
pub struct VacuumResultDto {
    pub bytes_before: i64,
    pub bytes_after: i64,
}

#[tauri::command]
pub async fn preview_vacuum(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> CommandResponse<VacuumPreviewDto> {
    let context = match host::module_session(&window, Capability::StorageManage) {
        Ok(context) => context,
        Err(error) => return denied(error),
    };
    let service = match service(&app) {
        Ok(service) => service,
        Err(error) => return respond(Err(error)),
    };
    respond(service.preview_vacuum(context.user_id()).map(|preview| VacuumPreviewDto {
        preview_id: preview.preview_id,
        expires_at: preview.expires_at,
    }))
}

/// VACUUM runs only after an explicit confirmation. The service takes the exclusive maintenance lease
/// without waiting and returns `Busy` while any account has activity in progress.
#[tauri::command]
pub async fn execute_vacuum(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: ExecuteVacuumInput,
) -> CommandResponse<VacuumResultDto> {
    let context = match host::module_session(&window, Capability::StorageManage) {
        Ok(context) => context,
        Err(error) => return denied(error),
    };
    if !input.confirmed {
        return respond(Err(StorageError::PreviewNotFound));
    }
    let service = match service(&app) {
        Ok(service) => service,
        Err(error) => return respond(Err(error)),
    };
    respond(
        service
            .execute_vacuum(context.user_id(), &input.preview_id)
            .await
            .map(|result| VacuumResultDto {
                bytes_before: result.bytes_before,
                bytes_after: result.bytes_after,
            }),
    )
}
