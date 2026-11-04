//! Storage statistics, cleanup preview and execution (D133 item 5).
//!
//! A preview fixes the account, the cutoff and the exact candidate rows with
//! their versions. Execution deletes only those rows, and each delete repeats
//! its eligibility check against the stored version. A staging directory is
//! removed only when its relative path has the shape `staging/<id>` and
//! resolves inside the skills root. VACUUM is a separate operation.

use dgos_db::repositories::storage_repository::{Candidate, StorageCategory, StorageRepository};
use dgos_db::{AssetRepository, MaintenanceGate, sea_orm};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

/// Preview validity. A preview past this moment is rejected.
pub const PREVIEW_TTL_SECS: i64 = 15 * 60;
/// Default cutoff for old chats (D133 item 5).
pub const OLD_CHAT_DEFAULT_DAYS: i64 = 30;
/// Default cutoff for non-security usage records (D133 item 5).
pub const ACTIVITY_DEFAULT_DAYS: i64 = 90;
/// Terminal install staging is a temp file once it is this old.
pub const TEMP_RETENTION_SECS: i64 = 24 * 60 * 60;
const DAY_SECS: i64 = 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CategoryStatus {
    Ok,
    Empty,
    NotApplicable,
}

#[derive(Clone, Debug)]
pub struct CategoryOverview {
    pub category: StorageCategory,
    pub status: CategoryStatus,
    pub count: i64,
    pub bytes: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct CleanupPreview {
    pub preview_id: String,
    pub category: StorageCategory,
    pub cutoff: i64,
    pub expires_at: i64,
    pub count: i64,
    pub bytes: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CleanupStatus {
    Done,
    Partial,
    Failed,
}

#[derive(Clone, Debug)]
pub struct CleanupResult {
    pub category: StorageCategory,
    pub status: CleanupStatus,
    pub deleted: i64,
    pub failed: i64,
}

#[derive(Clone, Debug)]
pub struct VacuumPreview {
    pub preview_id: String,
    pub expires_at: i64,
}

#[derive(Clone, Debug)]
pub struct VacuumResult {
    pub bytes_before: i64,
    pub bytes_after: i64,
}

#[derive(Debug)]
pub enum StorageError {
    PreviewNotFound,
    PreviewExpired,
    Busy,
    Database,
    InvalidCutoff,
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::PreviewNotFound => "storage preview not found or belongs to another account",
            Self::PreviewExpired => "storage preview expired",
            Self::Busy => "storage maintenance is busy; retry later",
            Self::Database => "storage database error",
            Self::InvalidCutoff => "storage cutoff must be a positive whole number of days within range",
        };
        f.write_str(text)
    }
}

impl From<sea_orm::DbErr> for StorageError {
    fn from(_: sea_orm::DbErr) -> Self {
        Self::Database
    }
}

#[derive(Clone)]
struct StoredPreview {
    user: String,
    category: StorageCategory,
    cutoff: i64,
    expires_at: i64,
    candidates: Vec<Candidate>,
}

pub struct StorageService {
    repo: StorageRepository,
    assets: AssetRepository,
    staging_root: PathBuf,
    gate: MaintenanceGate,
    previews: Mutex<HashMap<String, StoredPreview>>,
    vacuum_previews: Mutex<HashMap<String, (String, i64)>>,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn status_of(category: StorageCategory, count: i64, bytes: Option<i64>) -> CategoryOverview {
    let status = if count == 0 { CategoryStatus::Empty } else { CategoryStatus::Ok };
    CategoryOverview { category, status, count, bytes }
}

fn result(category: StorageCategory, deleted: i64, failed: i64) -> CleanupResult {
    let status = match (deleted, failed) {
        (_, 0) => CleanupStatus::Done,
        (0, _) => CleanupStatus::Failed,
        _ => CleanupStatus::Partial,
    };
    CleanupResult { category, status, deleted, failed }
}

fn cutoff_at(now: i64, days: i64) -> Result<i64, StorageError> {
    if days <= 0 {
        return Err(StorageError::InvalidCutoff);
    }
    days.checked_mul(DAY_SECS)
        .and_then(|seconds| now.checked_sub(seconds))
        .ok_or(StorageError::InvalidCutoff)
}

/// Resolves a staging relpath `staging/<id>` to an absolute path under `root`.
/// Anything else (absolute, `..`, extra components, symlinks) is refused.
fn contained_staging_path(root: &Path, relpath: &str) -> Option<PathBuf> {
    let mut parts = Path::new(relpath).components();
    let (Some(Component::Normal(first)), Some(Component::Normal(second)), None) =
        (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    if first != "staging" {
        return None;
    }
    let meta = std::fs::symlink_metadata(root).ok()?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return None;
    }
    let staging = root.join("staging");
    let meta = std::fs::symlink_metadata(&staging).ok()?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return None;
    }
    let path = staging.join(second);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return None;
    }
    let canonical = path.canonicalize().ok()?;
    let canonical_root = root.canonicalize().ok()?;
    canonical.starts_with(canonical_root.join("staging")).then_some(canonical)
}

impl StorageService {
    pub fn new(
        repo: StorageRepository,
        assets: AssetRepository,
        staging_root: PathBuf,
        gate: MaintenanceGate,
    ) -> Self {
        Self {
            repo,
            assets,
            staging_root,
            gate,
            previews: Mutex::new(HashMap::new()),
            vacuum_previews: Mutex::new(HashMap::new()),
        }
    }

    /// Five cleanup categories for one account. VACUUM is not listed here.
    pub async fn overview(&self, user: &str) -> Result<Vec<CategoryOverview>, StorageError> {
        let _permit = self.gate.admit().map_err(|_| StorageError::Busy)?;
        let now = now();
        let chats = self.repo.old_chat_candidates(user, now - OLD_CHAT_DEFAULT_DAYS * DAY_SECS).await?;
        let chat_bytes = self.repo.old_chat_bytes(user, &chats).await?;
        let assets = self.assets.list_unreferenced(user).await?;
        let asset_bytes: i64 = assets.iter().map(|a| a.bytes_len).sum();
        let temp = self.repo.temp_candidates(user, now - TEMP_RETENTION_SECS).await?;
        let activity = self.repo.activity_candidates(user, now - ACTIVITY_DEFAULT_DAYS * DAY_SECS).await?;

        Ok(vec![
            status_of(StorageCategory::OldChats, chats.len() as i64, Some(chat_bytes)),
            status_of(StorageCategory::SessionAssets, assets.len() as i64, Some(asset_bytes)),
            status_of(StorageCategory::TempFiles, temp.len() as i64, temp.is_empty().then_some(0)),
            CategoryOverview { category: StorageCategory::Cache, status: CategoryStatus::NotApplicable, count: 0, bytes: None },
            status_of(StorageCategory::ActivityRecords, activity.len() as i64, activity.is_empty().then_some(0)),
        ])
    }

    /// Fixes the cutoff and candidate rows for one category.
    pub async fn preview(
        &self,
        user: &str,
        category: StorageCategory,
        cutoff_days: Option<i64>,
    ) -> Result<CleanupPreview, StorageError> {
        let _permit = self.gate.admit().map_err(|_| StorageError::Busy)?;
        let now = now();
        let (cutoff, candidates, bytes) = match category {
            StorageCategory::OldChats => {
                let cutoff = cutoff_at(now, cutoff_days.unwrap_or(OLD_CHAT_DEFAULT_DAYS))?;
                let rows = self.repo.old_chat_candidates(user, cutoff).await?;
                let bytes = self.repo.old_chat_bytes(user, &rows).await?;
                (cutoff, rows, bytes)
            }
            StorageCategory::SessionAssets => {
                let rows = self
                    .assets
                    .list_unreferenced(user)
                    .await?
                    .into_iter()
                    .map(|a| (Candidate { id: a.id, version: a.bytes_len.to_string() }, a.bytes_len))
                    .collect::<Vec<_>>();
                let bytes = rows.iter().map(|(_, b)| *b).sum();
                (0, rows.into_iter().map(|(c, _)| c).collect(), bytes)
            }
            StorageCategory::TempFiles => {
                let cutoff = now - TEMP_RETENTION_SECS;
                (cutoff, self.repo.temp_candidates(user, cutoff).await?, 0)
            }
            StorageCategory::Cache => (0, Vec::new(), 0),
            StorageCategory::ActivityRecords => {
                let cutoff = cutoff_at(now, cutoff_days.unwrap_or(ACTIVITY_DEFAULT_DAYS))?;
                (cutoff, self.repo.activity_candidates(user, cutoff).await?, 0)
            }
        };
        let count = candidates.len() as i64;
        let preview_id = uuid::Uuid::new_v4().to_string();
        let expires_at = now + PREVIEW_TTL_SECS;
        self.previews.lock().expect("storage previews").insert(
            preview_id.clone(),
            StoredPreview { user: user.to_owned(), category, cutoff, expires_at, candidates },
        );
        let bytes = match category { StorageCategory::OldChats | StorageCategory::SessionAssets => Some(bytes), StorageCategory::Cache => None, _ => (count == 0).then_some(0) };
        Ok(CleanupPreview { preview_id, category, cutoff, expires_at, count, bytes })
    }

    /// Executes the previewed candidates only. The preview is consumed only
    /// when its owner matches. Each row is rechecked against its stored version.
    pub async fn execute(&self, user: &str, preview_id: &str) -> Result<CleanupResult, StorageError> {
        let _permit = self.gate.admit().map_err(|_| StorageError::Busy)?;
        let stored = {
            let mut map = self.previews.lock().expect("storage previews");
            match map.get(preview_id) {
                Some(entry) if entry.user == user => {}
                _ => return Err(StorageError::PreviewNotFound),
            }
            map.remove(preview_id).ok_or(StorageError::PreviewNotFound)?
        };
        if now() >= stored.expires_at {
            return Err(StorageError::PreviewExpired);
        }
        let total = stored.candidates.len() as i64;

        match stored.category {
            StorageCategory::OldChats => {
                let mut deleted = 0i64;
                for candidate in &stored.candidates {
                    if let Ok(removed) = self.repo.delete_old_chats(user, stored.cutoff, std::slice::from_ref(candidate)).await {
                        deleted += removed.len() as i64;
                    }
                }
                Ok(result(stored.category, deleted, total - deleted))
            }
            StorageCategory::SessionAssets => {
                let mut deleted = 0i64;
                for candidate in &stored.candidates {
                    if matches!(self.repo.delete_session_asset(user, candidate).await, Ok(true)) {
                        deleted += 1;
                    }
                }
                Ok(result(stored.category, deleted, total - deleted))
            }
            StorageCategory::TempFiles => {
                let mut deleted = 0i64;
                let mut failed = 0i64;
                for candidate in &stored.candidates {
                    let tx = match self.repo.begin_temp_cleanup(user, stored.cutoff, candidate).await {
                        Ok(Some(tx)) => tx,
                        _ => { failed += 1; continue; }
                    };
                    let Ok((_, _, relpath)) = serde_json::from_str::<(i64, String, String)>(&candidate.version) else {
                        failed += 1;
                        continue;
                    };
                    // The installer allocates exactly staging/<operation_id>; no operation may claim another's directory.
                    if relpath != format!("staging/{}", candidate.id) {
                        failed += 1;
                        continue;
                    }
                    let removed = match contained_staging_path(&self.staging_root, &relpath) {
                        Some(path) => match tokio::fs::remove_dir_all(&path).await {
                            Ok(()) => true,
                            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
                        },
                        None => false,
                    };
                    if removed {
                        if matches!(self.repo.forget_temp(tx, user, candidate).await, Ok(true)) {
                            deleted += 1;
                        } else {
                            failed += 1;
                        }
                    } else {
                        let _ = tx.rollback().await;
                        failed += 1;
                    }
                }
                Ok(result(stored.category, deleted, failed))
            }
            StorageCategory::Cache => Ok(result(stored.category, 0, 0)),
            StorageCategory::ActivityRecords => {
                let mut deleted = 0i64;
                for candidate in &stored.candidates {
                    if matches!(self.repo.delete_activity(user, candidate).await, Ok(true)) {
                        deleted += 1;
                    }
                }
                Ok(result(stored.category, deleted, total - deleted))
            }
        }
    }

    /// VACUUM preview bound to the account and an expiry. Admission is decided at execution by the gate,
    /// so a preview never reports a stale idle state.
    pub fn preview_vacuum(&self, user: &str) -> Result<VacuumPreview, StorageError> {
        let preview_id = uuid::Uuid::new_v4().to_string();
        let expires_at = now() + PREVIEW_TTL_SECS;
        self.vacuum_previews
            .lock()
            .expect("vacuum previews")
            .insert(preview_id.clone(), (user.to_owned(), expires_at));
        Ok(VacuumPreview { preview_id, expires_at })
    }

    /// Runs VACUUM only while holding the exclusive maintenance lease. The lease is taken without
    /// waiting: any live activity permit, or another maintenance, returns `Busy` at once. Sizes are read
    /// before and after, and the result reports what the file actually shrank to, nothing more.
    pub async fn execute_vacuum(&self, user: &str, preview_id: &str) -> Result<VacuumResult, StorageError> {
        let (owner, expires_at) = {
            let mut map = self.vacuum_previews.lock().expect("vacuum previews");
            match map.get(preview_id) {
                Some((owner, _)) if owner == user => {}
                _ => return Err(StorageError::PreviewNotFound),
            }
            map.remove(preview_id).ok_or(StorageError::PreviewNotFound)?
        };
        if owner != user {
            return Err(StorageError::PreviewNotFound);
        }
        if now() >= expires_at {
            return Err(StorageError::PreviewExpired);
        }
        let lease = self.gate.begin_maintenance().map_err(|_| StorageError::Busy)?;
        let bytes_before = self.repo.database_bytes().await?;
        let vacuumed = self.repo.vacuum().await;
        let bytes_after = self.repo.database_bytes().await?;
        drop(lease);
        vacuumed?;
        Ok(VacuumResult { bytes_before, bytes_after })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_db::sea_orm::{ConnectionTrait, DbBackend, Statement};

    async fn fixture() -> (tempfile::TempDir, sea_orm::DatabaseConnection, AssetRepository, StorageService, MaintenanceGate) {
        let root = tempfile::tempdir().unwrap();
        let db = dgos_db::establish_connection(&format!("sqlite://{}?mode=rwc", root.path().join("storage.db").display())).await.unwrap();
        use dgos_db::sea_orm_migration::MigratorTrait;
        dgos_db::migrations::Migrator::up(&db, None).await.unwrap();
        let assets = AssetRepository::new(db.clone());
        let gate = MaintenanceGate::new();
        let service = StorageService::new(StorageRepository::new(db.clone()), assets.clone(), root.path().join("skills"), gate.clone());
        (root, db, assets, service, gate)
    }

    #[tokio::test]
    async fn cleanup_rechecks_draft_leases_and_preserves_foreign_assets() {
        let (_root, _db, assets, service, _gate) = fixture().await;
        assets.insert("alice", "a", "a.txt", "text/plain", b"owned", 0).await.unwrap();
        assets.insert("bob", "b", "b.txt", "text/plain", b"foreign", 0).await.unwrap();
        let preview = service.preview("alice", StorageCategory::SessionAssets, None).await.unwrap();
        assert_eq!((preview.count, preview.bytes), (1, Some(5)));
        assert!(matches!(service.execute("bob", &preview.preview_id).await, Err(StorageError::PreviewNotFound)));
        assets.set_lease("alice", "a", "draft", now() + 60).await.unwrap();
        let result = service.execute("alice", &preview.preview_id).await.unwrap();
        assert_eq!((result.deleted, result.failed), (0, 1));
        assert!(assets.bytes("alice", "a").await.unwrap().is_some());
        assert!(assets.bytes("bob", "b").await.unwrap().is_some());
        assert!(matches!(service.execute("alice", &preview.preview_id).await, Err(StorageError::PreviewNotFound)));
    }

    #[tokio::test]
    async fn vacuum_refuses_live_activity_and_runs_after_release() {
        let (_root, _db, _assets, service, gate) = fixture().await;
        let permit = gate.admit().unwrap();
        let preview = service.preview_vacuum("alice").unwrap();
        assert!(matches!(service.execute_vacuum("alice", &preview.preview_id).await, Err(StorageError::Busy)));
        drop(permit);
        let preview = service.preview_vacuum("alice").unwrap();
        assert!(matches!(service.execute_vacuum("bob", &preview.preview_id).await, Err(StorageError::PreviewNotFound)));
        let result = service.execute_vacuum("alice", &preview.preview_id).await.unwrap();
        assert!(result.bytes_before > 0 && result.bytes_after > 0);
    }

    #[tokio::test]
    async fn temp_cleanup_rechecks_state_and_never_follows_a_symlink() {
        let (root, db, _assets, service, _gate) = fixture().await;
        let staging = root.path().join("skills/staging");
        std::fs::create_dir_all(staging.join("op")).unwrap();
        std::fs::write(staging.join("op/keep.txt"), b"keep").unwrap();
        db.execute(Statement::from_string(DbBackend::Sqlite, "INSERT INTO skill_install_operations(operation_id,user_id,idempotency_key,source_kind,source_ref,state,staging_relpath,created_at,updated_at) VALUES ('op','alice','op','local_zip','local','failed','staging/op',0,0)".to_owned())).await.unwrap();
        let preview = service.preview("alice", StorageCategory::TempFiles, None).await.unwrap();
        db.execute(Statement::from_string(DbBackend::Sqlite, "UPDATE skill_install_operations SET state='publishing' WHERE operation_id='op'".to_owned())).await.unwrap();
        let result = service.execute("alice", &preview.preview_id).await.unwrap();
        assert_eq!((result.deleted, result.failed), (0, 1));
        assert!(staging.join("op/keep.txt").exists());
        assert!(contained_staging_path(&root.path().join("skills"), "../safe").is_none());
        #[cfg(unix)] {
            std::os::unix::fs::symlink(staging.join("op"), staging.join("link")).unwrap();
            assert!(contained_staging_path(&root.path().join("skills"), "staging/link").is_none());
        }
    }
    #[tokio::test]
    async fn maintenance_and_expired_previews_never_delete_assets() {
        let (_root, _db, assets, service, gate) = fixture().await;
        assets.insert("alice", "a", "a.txt", "text/plain", b"owned", 0).await.unwrap();
        let preview = service.preview("alice", StorageCategory::SessionAssets, None).await.unwrap();
        let lease = gate.begin_maintenance().unwrap();
        assert!(matches!(service.overview("alice").await, Err(StorageError::Busy)));
        assert!(matches!(service.preview("alice", StorageCategory::TempFiles, None).await, Err(StorageError::Busy)));
        assert!(matches!(service.execute("alice", &preview.preview_id).await, Err(StorageError::Busy)));
        let vacuum = service.preview_vacuum("alice").unwrap();
        assert!(matches!(service.execute_vacuum("alice", &vacuum.preview_id).await, Err(StorageError::Busy)));
        drop(lease);
        service.previews.lock().unwrap().get_mut(&preview.preview_id).unwrap().expires_at = now();
        assert!(matches!(service.execute("alice", &preview.preview_id).await, Err(StorageError::PreviewExpired)));
        assert!(assets.bytes("alice", "a").await.unwrap().is_some());
        let vacuum = service.preview_vacuum("alice").unwrap();
        service.vacuum_previews.lock().unwrap().get_mut(&vacuum.preview_id).unwrap().1 = now();
        assert!(matches!(service.execute_vacuum("alice", &vacuum.preview_id).await, Err(StorageError::PreviewExpired)));
        for days in [0, -1, i64::MAX] {
            assert!(matches!(service.preview("alice", StorageCategory::OldChats, Some(days)).await, Err(StorageError::InvalidCutoff)));
        }
    }

    #[tokio::test]
    async fn asset_cleanup_reports_each_failure_and_rechecks_previewed_size() {
        let (_root, db, assets, service, _gate) = fixture().await;
        for id in ["changed", "fails", "removed"] {
            assets.insert("alice", id, "a.txt", "text/plain", b"owned", 0).await.unwrap();
        }
        let preview = service.preview("alice", StorageCategory::SessionAssets, None).await.unwrap();
        db.execute_unprepared("UPDATE chat_assets SET bytes = X'00' WHERE id = 'changed'; CREATE TRIGGER refuse_asset_delete BEFORE DELETE ON chat_assets WHEN OLD.id = 'fails' BEGIN SELECT RAISE(ABORT, 'refuse'); END;").await.unwrap();
        let result = service.execute("alice", &preview.preview_id).await.unwrap();
        assert_eq!((result.status, result.deleted, result.failed), (CleanupStatus::Partial, 1, 2));
        assert!(assets.bytes("alice", "changed").await.unwrap().is_some());
        assert!(assets.bytes("alice", "fails").await.unwrap().is_some());
        assert!(assets.bytes("alice", "removed").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn temp_cleanup_rechecks_versions_and_shared_path_registration() {
        let (root, db, _assets, service, _gate) = fixture().await;
        let staging = root.path().join("skills/staging");
        for id in ["changed", "shared", "removed"] {
            std::fs::create_dir_all(staging.join(id)).unwrap();
            db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
                "INSERT INTO skill_install_operations(operation_id,user_id,idempotency_key,source_kind,source_ref,state,staging_relpath,created_at,updated_at) VALUES (?,'alice',?,'local_zip','local','failed',?,0,0)",
                [sea_orm::Value::from(id), sea_orm::Value::from(id), sea_orm::Value::from(format!("staging/{id}"))])).await.unwrap();
        }
        let preview = service.preview("alice", StorageCategory::TempFiles, None).await.unwrap();
        assert_eq!((preview.count, preview.bytes), (3, None));
        db.execute_unprepared("UPDATE skill_install_operations SET updated_at = 1 WHERE operation_id = 'changed'; INSERT INTO skill_install_operations(operation_id,user_id,idempotency_key,source_kind,source_ref,state,staging_relpath,created_at,updated_at) VALUES ('foreign','bob','foreign','local_zip','local','publishing','staging/shared',0,0);").await.unwrap();
        let result = service.execute("alice", &preview.preview_id).await.unwrap();
        assert_eq!((result.status, result.deleted, result.failed), (CleanupStatus::Partial, 1, 2));
        assert!(staging.join("changed").exists());
        assert!(staging.join("shared").exists());
        assert!(!staging.join("removed").exists());
        let overview = service.overview("alice").await.unwrap();
        let temp = overview.iter().find(|row| row.category == StorageCategory::TempFiles).unwrap();
        assert_eq!((temp.count, temp.bytes), (1, None));
        let cache = overview.iter().find(|row| row.category == StorageCategory::Cache).unwrap();
        assert_eq!((&cache.status, cache.bytes), (&CategoryStatus::NotApplicable, None));
    }

    #[tokio::test]
    async fn old_chat_cleanup_rechecks_version_and_live_messages() {
        let (_root, db, _assets, service, _gate) = fixture().await;
        db.execute_unprepared("INSERT INTO chat_sessions(id,user_id,title,created_at,updated_at) VALUES ('changed','alice','old',0,0),('running','alice','old',0,0),('removed','alice','old',0,0),('foreign','bob','old',0,0); INSERT INTO chat_messages(id,session_id,user_id,seq,role,content,status,created_at,updated_at) VALUES ('message','removed','alice',1,'user','你好','completed',0,0);").await.unwrap();
        let preview = service.preview("alice", StorageCategory::OldChats, None).await.unwrap();
        assert_eq!((preview.count, preview.bytes), (3, Some(6)));
        db.execute_unprepared("UPDATE chat_sessions SET updated_at=1 WHERE id='changed'; INSERT INTO chat_messages(id,session_id,user_id,seq,role,content,status,created_at,updated_at) VALUES ('generating','running','alice',1,'assistant','','generating',0,0);").await.unwrap();
        let result = service.execute("alice", &preview.preview_id).await.unwrap();
        assert_eq!((result.status, result.deleted, result.failed), (CleanupStatus::Partial, 1, 2));
        let row = db.query_one(Statement::from_string(DbBackend::Sqlite, "SELECT COUNT(*) AS count FROM chat_sessions".to_owned())).await.unwrap().unwrap();
        assert_eq!(row.try_get::<i64>("", "count").unwrap(), 3);
    }

    #[tokio::test]
    async fn asset_link_after_preview_survives_until_its_last_chat_is_removed() {
        let (_root, db, assets, service, _gate) = fixture().await;
        assets.insert("alice", "linked", "owned.txt", "text/plain", b"owned", 0).await.unwrap();
        assets.insert("bob", "foreign", "foreign.txt", "text/plain", b"foreign", 0).await.unwrap();
        let preview = service.preview("alice", StorageCategory::SessionAssets, None).await.unwrap();
        assert_eq!(preview.count, 1);
        db.execute_unprepared("INSERT INTO chat_sessions(id,user_id,title,created_at,updated_at) VALUES ('linked-chat','alice','old',0,0); INSERT INTO chat_messages(id,session_id,user_id,seq,role,content,status,created_at,updated_at) VALUES ('linked-message','linked-chat','alice',1,'user','kept','completed',0,0); INSERT INTO chat_message_assets(message_id,user_id,asset_id,position) VALUES ('linked-message','alice','linked',0);").await.unwrap();
        let protected = service.execute("alice", &preview.preview_id).await.unwrap();
        assert_eq!((protected.deleted, protected.failed), (0, 1));
        assert_eq!(service.preview("alice", StorageCategory::SessionAssets, None).await.unwrap().count, 0);
        assert!(assets.bytes("bob", "linked").await.unwrap().is_none());
        let chats = service.preview("alice", StorageCategory::OldChats, None).await.unwrap();
        assert_eq!(service.execute("alice", &chats.preview_id).await.unwrap().deleted, 1);
        // Removing chat content releases its reference; asset cleanup remains an explicit action.
        assert_eq!(assets.bytes("alice", "linked").await.unwrap().unwrap(), b"owned");
        let unlinked = service.preview("alice", StorageCategory::SessionAssets, None).await.unwrap();
        assert_eq!(unlinked.count, 1);
        assert_eq!(service.execute("alice", &unlinked.preview_id).await.unwrap().deleted, 1);
        assert!(assets.bytes("alice", "linked").await.unwrap().is_none());
        assert_eq!(assets.bytes("bob", "foreign").await.unwrap().unwrap(), b"foreign");
    }
    #[tokio::test]
    async fn activity_cleanup_preserves_foreign_recent_and_changed_rows() {
        let (_root, db, _assets, service, _gate) = fixture().await;
        db.execute_unprepared("INSERT INTO mcp_servers(id,user_id,name,transport,config_json,created_at,updated_at) VALUES ('server','alice','server','stdio','{}',0,0); INSERT INTO mcp_connection_logs(id,user_id,server_id,event,status,created_at) VALUES (1,'alice','server','connect','ok',0),(2,'alice','server','connect','ok',0),(3,'bob','server','connect','ok',0);").await.unwrap();
        db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT INTO mcp_connection_logs(id,user_id,server_id,event,status,created_at) VALUES (4,'alice','server','connect','ok',?)",
            [sea_orm::Value::from(now())])).await.unwrap();
        let preview = service.preview("alice", StorageCategory::ActivityRecords, None).await.unwrap();
        assert_eq!((preview.count, preview.bytes), (2, None));
        db.execute_unprepared("UPDATE mcp_connection_logs SET created_at=1 WHERE id=1;").await.unwrap();
        let result = service.execute("alice", &preview.preview_id).await.unwrap();
        assert_eq!((result.status, result.deleted, result.failed), (CleanupStatus::Partial, 1, 1));
        let row = db.query_one(Statement::from_string(DbBackend::Sqlite, "SELECT COUNT(*) AS count FROM mcp_connection_logs WHERE id IN (1,3,4)".to_owned())).await.unwrap().unwrap();
        assert_eq!(row.try_get::<i64>("", "count").unwrap(), 3);
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn temp_cleanup_refuses_symlinked_directories_and_roots() {
        let (root, db, _assets, service, _gate) = fixture().await;
        let staging = root.path().join("skills/staging");
        let foreign = root.path().join("foreign");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("keep"), b"foreign").unwrap();
        std::os::unix::fs::symlink(&foreign, staging.join("linked")).unwrap();
        db.execute_unprepared("INSERT INTO skill_install_operations(operation_id,user_id,idempotency_key,source_kind,source_ref,state,staging_relpath,created_at,updated_at) VALUES ('linked','alice','linked','local_zip','local','failed','staging/linked',0,0);").await.unwrap();
        let preview = service.preview("alice", StorageCategory::TempFiles, None).await.unwrap();
        let result = service.execute("alice", &preview.preview_id).await.unwrap();
        assert_eq!((result.status, result.deleted, result.failed), (CleanupStatus::Failed, 0, 1));
        assert!(foreign.join("keep").exists());
        let linked_root = root.path().join("linked-root");
        std::os::unix::fs::symlink(root.path().join("skills"), &linked_root).unwrap();
        assert!(contained_staging_path(&linked_root, "staging/linked").is_none());
    }
}
