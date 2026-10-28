use dgos_db::{
    entities::{module_update_events, module_versions, modules},
    sea_orm::{
        ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
        QueryFilter, Set, TransactionTrait,
    },
    ModulePermission, ModulePermissionRepository,
};
use dgos_types::HostExecutionContext;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ModulePackageError {
    #[error("module package I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid module manifest: {0}")]
    Manifest(String),
    #[error("module package integrity mismatch")]
    Integrity,
    #[error("module package database error: {0}")]
    Database(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub entry: String,
    pub artifact: Artifact,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub version: String,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct StagedPackage {
    operation_id: String,
    module_id: String,
    display_name: String,
    version: String,
    integrity: String,
    staging_path: PathBuf,
    entry: String,
    manifest: serde_json::Value,
}

impl StagedPackage {
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
    pub fn module_id(&self) -> &str {
        &self.module_id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn integrity(&self) -> &str {
        &self.integrity
    }
}

#[derive(Clone)]
pub struct ModulePublisher {
    root: PathBuf,
    db: DatabaseConnection,
    permissions: std::sync::Arc<ModulePermissionRepository>,
    publish_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    owner_id: String,
}

impl ModulePublisher {
    pub fn new(root: impl Into<PathBuf>, db: DatabaseConnection) -> Self {
        Self {
            root: root.into(),
            permissions: std::sync::Arc::new(ModulePermissionRepository::new(db.clone())),
            db,
            publish_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            owner_id: Uuid::new_v4().to_string(),
        }
    }

    pub fn permissions(&self) -> &ModulePermissionRepository {
        &self.permissions
    }

    /// Mint a host context only from the currently published and recoverable
    /// module row. Renderer input never participates in this lookup.
    pub async fn execution_context(
        &self,
        user_id: &str,
        module_id: &str,
    ) -> Result<HostExecutionContext, ModulePackageError> {
        let module = modules::Entity::find_by_id(module_id)
            .one(&self.db)
            .await
            .map_err(db_error)?
            .ok_or_else(|| ModulePackageError::Database("module is not registered".into()))?;
        if !module.is_enabled
            || self.recover(module_id)?.as_deref() != Some(module.current_version.as_str())
            || !self.current_package_matches(
                module_id,
                &module.current_version,
                &module.integrity,
            )?
        {
            return Err(ModulePackageError::Manifest(
                "module is not recoverable".into(),
            ));
        }
        Ok(HostExecutionContext::for_module(
            user_id,
            module_id,
            &module.current_version,
            &module.integrity,
        ))
    }

    /// Re-check the package-backed identity immediately before a capability
    /// dispatch. This closes the gap where a package is tampered with after a
    /// context was minted but before an external operation starts.
    pub async fn validate_execution_identity(
        &self,
        module_id: &str,
        version: &str,
        integrity: &str,
    ) -> Result<bool, ModulePackageError> {
        let module = modules::Entity::find_by_id(module_id)
            .one(&self.db)
            .await
            .map_err(db_error)?;
        if !module.is_some_and(|module| {
            module.is_enabled && module.current_version == version && module.integrity == integrity
        }) {
            return Ok(false);
        }
        self.current_package_matches(module_id, version, integrity)
    }

    /// Copy into a staging directory and validate before it can affect the
    /// current-version pointer. Remote URLs are intentionally not accepted.
    pub fn stage(&self, source: &Path) -> Result<StagedPackage, ModulePackageError> {
        self.stage_with_operation_id(source, Uuid::new_v4().to_string())
    }

    /// Stage a package for a caller-provided operation. Retries of one
    /// application operation can therefore retain the same audit identity.
    pub fn stage_with_operation_id(
        &self,
        source: &Path,
        operation_id: impl Into<String>,
    ) -> Result<StagedPackage, ModulePackageError> {
        if !source.is_dir() || fs::symlink_metadata(source)?.file_type().is_symlink() {
            return Err(ModulePackageError::Manifest(
                "package is not a directory".into(),
            ));
        }

        fs::create_dir_all(&self.root)?;
        let source = source.canonicalize()?;
        let store = self.root.canonicalize()?;
        if source.starts_with(&store) || store.starts_with(&source) {
            return Err(ModulePackageError::Manifest(
                "package source and module store cannot overlap".into(),
            ));
        }
        let staging_root = store.join("staging");
        fs::create_dir_all(&staging_root)?;
        let staging = staging_root.join(Uuid::new_v4().to_string());
        if let Err(error) = copy_dir(&source, &staging) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        let validated = self.validate_staged_package(&staging);
        let (manifest, integrity) = match validated {
            Ok(validated) => validated,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
        };
        let final_staging = self.root.join("staging").join(format!(
            "{}-{}-{}",
            manifest.id,
            manifest.version,
            Uuid::new_v4()
        ));
        fs::rename(&staging, &final_staging)?;
        fs::File::open(final_staging.parent().unwrap())?.sync_all()?;
        let manifest_value = serde_json::to_value(&manifest)
            .map_err(|error| ModulePackageError::Manifest(error.to_string()))?;
        Ok(StagedPackage {
            operation_id: operation_id.into(),
            module_id: manifest.id,
            display_name: manifest.name,
            version: manifest.version,
            integrity,
            staging_path: final_staging,
            entry: manifest.entry,
            manifest: manifest_value,
        })
    }

    fn validate_staged_package(
        &self,
        staging: &Path,
    ) -> Result<(ModuleManifest, String), ModulePackageError> {
        let manifest_path = staging.join("manifest.json");
        let manifest: ModuleManifest = serde_json::from_slice(&fs::read(&manifest_path)?)
            .map_err(|e| ModulePackageError::Manifest(e.to_string()))?;
        if manifest.id.is_empty()
            || manifest.version.is_empty()
            || manifest.name.is_empty()
            || !valid_module_id(&manifest.id)
            || !valid_version_path(&manifest.version)
            || manifest.artifact.version != manifest.version
            || manifest.artifact.sha256.len() != 64
            || !manifest
                .artifact
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || manifest.entry.starts_with("http://")
            || manifest.entry.starts_with("https://")
            || !valid_relative_entry(&manifest.entry)
            || !staging.join(&manifest.entry).is_file()
        {
            return Err(ModulePackageError::Manifest(
                "invalid identity, version or local entry".into(),
            ));
        }
        for permission in &manifest.permissions {
            ModulePermission::parse(permission)
                .map_err(|_| ModulePackageError::Manifest("invalid module permission".into()))?;
        }
        let integrity = package_digest(staging)?;
        if !manifest.artifact.sha256.eq_ignore_ascii_case(&integrity) {
            return Err(ModulePackageError::Integrity);
        }
        Ok((manifest, integrity))
    }

    /// Atomically publish files and then update the DB pointer. The old
    /// version remains on disk so startup recovery can roll back safely.
    pub async fn commit(&self, package: StagedPackage) -> Result<(), ModulePackageError> {
        let _publish_guard = self.publish_lock.lock().await;
        self.acquire_release_lock().await?;
        let result = self.commit_locked(package).await;
        if let Err(error) = self.release_release_lock().await {
            if result.is_ok() {
                return Err(error);
            }
        }
        result
    }

    async fn commit_locked(&self, package: StagedPackage) -> Result<(), ModulePackageError> {
        let (validated_manifest, validated_integrity) =
            self.validate_staged_package(&package.staging_path)?;
        if validated_manifest.id != package.module_id
            || validated_manifest.version != package.version
            || validated_manifest.entry != package.entry
            || !validated_integrity.eq_ignore_ascii_case(&package.integrity)
        {
            return Err(ModulePackageError::Integrity);
        }
        let version_dir = self
            .root
            .join("versions")
            .join(&package.module_id)
            .join(&package.version);
        if version_dir.exists() {
            if self
                .existing_release_matches(&package.module_id, &package.version, &package.integrity)
                .await?
            {
                let _ = fs::remove_dir_all(&package.staging_path);
                return Ok(());
            }
            return Err(ModulePackageError::Manifest(
                "version already exists with a different integrity".into(),
            ));
        }
        fs::create_dir_all(version_dir.parent().unwrap())?;
        fs::rename(&package.staging_path, &version_dir)?;
        fs::File::open(version_dir.parent().unwrap())?.sync_all()?;
        let pointer_dir = self.root.join("current").join(&package.module_id);
        let previous = fs::read_to_string(pointer_dir.join("version")).ok();
        let publish_result = async {
            self.write_pointer(&package.module_id, &package.version)?;
            let tx = self.db.begin().await.map_err(db_error)?;
            let row = modules::Entity::find_by_id(&package.module_id)
                .one(&tx)
                .await
                .map_err(db_error)?;
            let from_version = row.as_ref().and_then(|row| {
                Some(row.current_version.clone()).filter(|version| version != &package.version)
            });
            if row.is_none() {
                modules::ActiveModel {
                    id: Set(package.module_id.clone()),
                    module_name: Set(package.module_id.clone()),
                    display_name: Set(package.display_name.clone()),
                    current_version: Set(package.version.clone()),
                    description: Set(None),
                    manifest: Set(package.manifest.clone()),
                    integrity: Set(package.integrity.clone()),
                    entry_url: Set(Some(package.entry.clone())),
                    icon_url: Set(None),
                    is_official: Set(false),
                    is_enabled: Set(true),
                    installed_at: Set(chrono::Utc::now().timestamp()),
                    updated_at: Set(chrono::Utc::now().timestamp()),
                }
                .insert(&tx)
                .await
                .map_err(db_error)?;
            }
            module_versions::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                module_id: Set(package.module_id.clone()),
                version: Set(package.version.clone()),
                integrity: Set(package.integrity.clone()),
                entry_path: Set(package.entry.clone()),
                created_at: Set(chrono::Utc::now().timestamp()),
            }
            .insert(&tx)
            .await
            .map_err(db_error)?;
            if let Some(row) = row {
                let mut model: modules::ActiveModel = row.into();
                model.current_version = Set(package.version.clone());
                model.integrity = Set(package.integrity.clone());
                model.entry_url = Set(Some(package.entry.clone()));
                model.manifest = Set(package.manifest.clone());
                model.updated_at = Set(chrono::Utc::now().timestamp());
                model.update(&tx).await.map_err(db_error)?;
            }
            module_update_events::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                operation_id: Set(package.operation_id.clone()),
                module_id: Set(package.module_id.clone()),
                from_version: Set(from_version),
                to_version: Set(package.version.clone()),
                stage: Set("committed".into()),
                outcome: Set("success".into()),
                error_class: Set(None),
                created_at: Set(chrono::Utc::now().timestamp()),
            }
            .insert(&tx)
            .await
            .map_err(db_error)?;
            tx.commit().await.map_err(db_error)
        }
        .await;
        if let Err(error) = publish_result {
            let restore_result = if pointer_dir.exists() {
                self.restore_pointer(&pointer_dir, previous.as_deref())
            } else {
                Ok(())
            };
            if restore_result.is_err() {
                let _ = self
                    .record_update_event(
                        &package.operation_id,
                        &package.module_id,
                        previous.as_deref().map(str::trim),
                        &package.version,
                        "recovery_pending",
                        "restore_failed",
                        Some("pointer_restore".into()),
                    )
                    .await;
            }
            // A failed transaction must remain visible to startup recovery and
            // operators, while the original error remains the API result.
            let failure_stage = if restore_result.is_ok() {
                "publish_commit"
            } else {
                "recovery_pending"
            };
            let _ = self
                .record_update_event(
                    &package.operation_id,
                    &package.module_id,
                    previous.as_deref().map(str::trim),
                    &package.version,
                    failure_stage,
                    "failure",
                    Some(error_class(&error)),
                )
                .await;
            restore_result?;
            return Err(error);
        }
        Ok(())
    }

    async fn existing_release_matches(
        &self,
        module_id: &str,
        version: &str,
        integrity: &str,
    ) -> Result<bool, ModulePackageError> {
        let package = self.root.join("versions").join(module_id).join(version);
        if !package.is_dir() || !package_digest(&package)?.eq_ignore_ascii_case(integrity) {
            return Ok(false);
        }
        let row = module_versions::Entity::find()
            .filter(module_versions::Column::ModuleId.eq(module_id))
            .filter(module_versions::Column::Version.eq(version))
            .one(&self.db)
            .await
            .map_err(db_error)?;
        Ok(row.is_some_and(|row| row.integrity.eq_ignore_ascii_case(integrity)))
    }

    /// On startup, only keep a pointer whose package still matches its
    /// manifest/integrity. A missing or corrupt pointer is removed so the
    /// caller can disable the module instead of loading unknown code.
    pub fn recover(&self, module_id: &str) -> Result<Option<String>, ModulePackageError> {
        if !valid_module_id(module_id) {
            return Ok(None);
        }
        let pointer = self.root.join("current").join(module_id).join("version");
        let version = match fs::read_to_string(&pointer) {
            Ok(value) => value.trim().to_owned(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !valid_version_path(&version) {
            fs::remove_file(pointer)?;
            return Ok(None);
        }
        let package = self.root.join("versions").join(module_id).join(&version);
        let manifest: ModuleManifest =
            serde_json::from_slice(&fs::read(package.join("manifest.json"))?)
                .map_err(|e| ModulePackageError::Manifest(e.to_string()))?;
        if manifest.id != module_id
            || manifest.version != version
            || !valid_relative_entry(&manifest.entry)
            || !package.join(&manifest.entry).is_file()
            || !package_digest(&package)?.eq_ignore_ascii_case(&manifest.artifact.sha256)
        {
            fs::remove_file(pointer)?;
            return Ok(None);
        }
        Ok(Some(version))
    }

    fn current_package_matches(
        &self,
        module_id: &str,
        version: &str,
        integrity: &str,
    ) -> Result<bool, ModulePackageError> {
        if !valid_module_id(module_id) || !valid_version_path(version) {
            return Ok(false);
        }
        let package = self.root.join("versions").join(module_id).join(version);
        if !package.is_dir() {
            return Ok(false);
        }
        let manifest: ModuleManifest =
            serde_json::from_slice(&fs::read(package.join("manifest.json"))?)
                .map_err(|error| ModulePackageError::Manifest(error.to_string()))?;
        Ok(manifest.id == module_id
            && manifest.version == version
            && valid_relative_entry(&manifest.entry)
            && package.join(&manifest.entry).is_file()
            && manifest.artifact.sha256.eq_ignore_ascii_case(integrity)
            && package_digest(&package)?.eq_ignore_ascii_case(integrity))
    }

    /// Reconcile every registered module before any future loader is allowed
    /// to execute code. Invalid pointers are fail-closed by disabling the row.
    pub async fn recover_registered(&self) -> Result<Vec<String>, ModulePackageError> {
        let _publish_guard = self.publish_lock.lock().await;
        self.acquire_release_lock().await?;
        let result = async {
            let rows = modules::Entity::find()
                .all(&self.db)
                .await
                .map_err(db_error)?;
            let mut ready = Vec::new();
            for row in rows {
                let valid = self
                    .current_package_matches(&row.id, &row.current_version, &row.integrity)
                    .unwrap_or(false);
                if valid {
                    self.write_pointer(&row.id, &row.current_version)?;
                    ready.push(row.id);
                } else {
                    let pointer = self.root.join("current").join(&row.id).join("version");
                    let _ = fs::remove_file(pointer);
                    let mut model: modules::ActiveModel = row.into();
                    model.is_enabled = Set(false);
                    model.updated_at = Set(chrono::Utc::now().timestamp());
                    model.update(&self.db).await.map_err(db_error)?;
                }
            }
            Ok::<_, ModulePackageError>(ready)
        }
        .await;
        if let Err(error) = self.release_release_lock().await {
            if result.is_ok() {
                return Err(error);
            }
        }
        result
    }

    const RELEASE_LOCK_KEY: &'static str = "modules";
    const RELEASE_LOCK_TTL_SECONDS: i64 = 60;

    async fn acquire_release_lock(&self) -> Result<(), ModulePackageError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let now = chrono::Utc::now().timestamp();
            let expires = now + Self::RELEASE_LOCK_TTL_SECONDS;
            let tx = self.db.begin().await.map_err(db_error)?;
            tx.execute_unprepared(&format!(
                "DELETE FROM module_release_locks WHERE expires_at <= {now}"
            ))
            .await
            .map_err(db_error)?;
            let insert = tx
                .execute_unprepared(&format!(
                    "INSERT INTO module_release_locks (lock_key, owner_id, acquired_at, expires_at) VALUES ('{}', '{}', {now}, {expires})",
                    Self::RELEASE_LOCK_KEY,
                    self.owner_id
                ))
                .await;
            match insert {
                Ok(_) => {
                    tx.commit().await.map_err(db_error)?;
                    return Ok(());
                }
                Err(_) => {
                    let _ = tx.rollback().await;
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err(ModulePackageError::Database(
                    "module release lock is busy".into(),
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    async fn release_release_lock(&self) -> Result<(), ModulePackageError> {
        self.db
            .execute_unprepared(&format!(
                "DELETE FROM module_release_locks WHERE lock_key = '{}' AND owner_id = '{}'",
                Self::RELEASE_LOCK_KEY,
                self.owner_id
            ))
            .await
            .map(|_| ())
            .map_err(db_error)
    }

    fn write_pointer(&self, module_id: &str, version: &str) -> Result<(), ModulePackageError> {
        let pointer_dir = self.root.join("current").join(module_id);
        fs::create_dir_all(&pointer_dir)?;
        let pointer_tmp = pointer_dir.join(format!(".next-{}", Uuid::new_v4()));
        let mut file = fs::File::create(&pointer_tmp)?;
        use std::io::Write;
        file.write_all(format!("{version}\n").as_bytes())?;
        file.sync_all()?;
        fs::rename(&pointer_tmp, pointer_dir.join("version"))?;
        fs::File::open(pointer_dir)?.sync_all()?;
        Ok(())
    }

    fn restore_pointer(
        &self,
        pointer_dir: &Path,
        previous: Option<&str>,
    ) -> Result<(), ModulePackageError> {
        match previous {
            Some(version) => {
                let module_id = pointer_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| {
                        ModulePackageError::Manifest("invalid module pointer path".into())
                    })?;
                self.write_pointer(module_id, version.trim())
            }
            None => {
                match fs::remove_file(pointer_dir.join("version")) {
                    Ok(()) => {
                        fs::File::open(pointer_dir)?.sync_all()?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                Ok(())
            }
        }
    }

    async fn record_update_event(
        &self,
        operation_id: &str,
        module_id: &str,
        from_version: Option<&str>,
        to_version: &str,
        stage: &str,
        outcome: &str,
        error_class: Option<String>,
    ) -> Result<(), ModulePackageError> {
        module_update_events::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            operation_id: Set(operation_id.to_owned()),
            module_id: Set(module_id.to_owned()),
            from_version: Set(from_version.map(str::to_owned)),
            to_version: Set(to_version.to_owned()),
            stage: Set(stage.to_owned()),
            outcome: Set(outcome.to_owned()),
            error_class: Set(error_class),
            created_at: Set(chrono::Utc::now().timestamp()),
        }
        .insert(&self.db)
        .await
        .map(|_| ())
        .map_err(db_error)
    }
}

fn valid_module_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'@'))
}

fn valid_version_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains("..")
        && !value.chars().any(char::is_whitespace)
}

fn valid_relative_entry(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && !value.contains("..")
        && !value.chars().any(char::is_whitespace)
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn error_class(error: &ModulePackageError) -> String {
    match error {
        ModulePackageError::Io(_) => "io".into(),
        ModulePackageError::Manifest(_) => "manifest".into(),
        ModulePackageError::Integrity => "integrity".into(),
        ModulePackageError::Database(_) => "database".into(),
    }
}

fn package_digest(root: &Path) -> Result<String, ModulePackageError> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (relative, path) in files {
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update(fs::read(path)?);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), ModulePackageError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(ModulePackageError::Manifest(
                "symbolic links are not allowed".into(),
            ));
        }
        if path.is_dir() {
            collect_files(root, &path, files)?;
        } else if path.file_name().is_some_and(|name| name != "manifest.json") {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            files.push((relative, path));
        }
    }
    Ok(())
}

fn copy_dir(source: &Path, target: &Path) -> Result<(), ModulePackageError> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        if fs::symlink_metadata(&source_path)?.file_type().is_symlink() {
            return Err(ModulePackageError::Manifest(
                "symbolic links are not allowed".into(),
            ));
        }
        let destination = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &destination)?;
        } else {
            fs::copy(source_path, destination)?;
        }
    }
    Ok(())
}

fn db_error(error: dgos_db::sea_orm::DbErr) -> ModulePackageError {
    ModulePackageError::Database(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_db::sea_orm::{ActiveModelTrait, ConnectionTrait, PaginatorTrait, Set};
    use dgos_db::{migrations::Migrator, sea_orm::Database};
    use sea_orm_migration::MigratorTrait;
    use tempfile::tempdir;

    fn write_package(dir: &Path, digest: &str) {
        fs::write(dir.join("entry.js"), "export default {};").unwrap();
        let manifest = serde_json::json!({"id":"module-1","name":"Example","version":"1.0.0","entry":"entry.js","artifact":{"version":"1.0.0","sha256":digest},"permissions":[]});
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn write_versioned_package(dir: &Path, version: &str) -> String {
        fs::write(dir.join("entry.js"), format!("export default '{version}';")).unwrap();
        let digest = package_digest(dir).unwrap();
        let manifest = serde_json::json!({
            "id": "module-1",
            "name": "Example",
            "version": version,
            "entry": "entry.js",
            "artifact": {"version": version, "sha256": digest},
            "permissions": []
        });
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        digest
    }

    async fn insert_module_row(
        db: &dgos_db::sea_orm::DatabaseConnection,
        version: &str,
        integrity: &str,
        manifest: serde_json::Value,
    ) {
        modules::ActiveModel {
            id: Set("module-1".into()),
            module_name: Set("example".into()),
            display_name: Set("Example".into()),
            current_version: Set(version.into()),
            description: Set(None),
            manifest: Set(manifest),
            integrity: Set(integrity.into()),
            entry_url: Set(None),
            icon_url: Set(None),
            is_official: Set(false),
            is_enabled: Set(true),
            installed_at: Set(chrono::Utc::now().timestamp()),
            updated_at: Set(chrono::Utc::now().timestamp()),
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn digest_excludes_manifest_and_rejects_bad_integrity() {
        let root = tempdir().unwrap();
        write_package(root.path(), &"0".repeat(64));
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db);
        assert!(matches!(
            publisher.stage(root.path()),
            Err(ModulePackageError::Integrity)
        ));
    }

    #[tokio::test]
    async fn stage_rejects_store_overlap_in_both_directions() {
        let parent = tempdir().unwrap();
        let store = parent.path().join("store");
        fs::create_dir_all(store.join("staging/source")).unwrap();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let publisher = ModulePublisher::new(&store, db);

        assert!(matches!(
            publisher.stage(parent.path()),
            Err(ModulePackageError::Manifest(_))
        ));
        assert!(matches!(
            publisher.stage(&store.join("staging/source")),
            Err(ModulePackageError::Manifest(_))
        ));
    }

    #[tokio::test]
    async fn stage_rejects_remote_entries_and_unknown_permissions() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("entry.js"), "export default {}; ").unwrap();
        fs::write(
            root.path().join("manifest.json"),
            serde_json::json!({
                "id": "module-1",
                "name": "Example",
                "version": "1.0.0",
                "entry": "https://example.invalid/remote.js",
                "artifact": {"version": "1.0.0", "sha256": "0".repeat(64)},
                "permissions": []
            })
            .to_string(),
        )
        .unwrap();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db);
        assert!(matches!(
            publisher.stage(root.path()),
            Err(ModulePackageError::Manifest(_))
        ));

        let root = tempdir().unwrap();
        fs::write(root.path().join("entry.js"), "export default {}; ").unwrap();
        let digest = package_digest(root.path()).unwrap();
        fs::write(
            root.path().join("manifest.json"),
            serde_json::json!({
                "id": "module-1",
                "name": "Example",
                "version": "1.0.0",
                "entry": "entry.js",
                "artifact": {"version": "1.0.0", "sha256": digest},
                "permissions": ["providers.unknown"]
            })
            .to_string(),
        )
        .unwrap();
        assert!(matches!(
            publisher.stage(root.path()),
            Err(ModulePackageError::Manifest(_))
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stage_rejects_symlinked_package_files() {
        use std::os::unix::fs::symlink;

        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        fs::write(target.path().join("entry.js"), "export default {}; ").unwrap();
        symlink(
            target.path().join("entry.js"),
            source.path().join("entry.js"),
        )
        .unwrap();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db);
        assert!(matches!(
            publisher.stage(source.path()),
            Err(ModulePackageError::Manifest(_))
        ));
    }

    #[tokio::test]
    async fn stage_commit_and_recover_pointer() {
        let source = tempdir().unwrap();
        fs::write(source.path().join("entry.js"), "export default {};").unwrap();
        let digest = package_digest(source.path()).unwrap();
        write_package(source.path(), &digest);
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        insert_module_row(
            &db,
            "0.0.1",
            &"0".repeat(64),
            serde_json::json!({"permissions": []}),
        )
        .await;
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db.clone());
        let staged = publisher.stage(source.path()).unwrap();
        publisher.commit(staged).await.unwrap();
        let version_count = module_versions::Entity::find().count(&db).await.unwrap();
        let event_count = module_update_events::Entity::find()
            .count(&db)
            .await
            .unwrap();
        assert_eq!(version_count, 1);
        assert_eq!(event_count, 1);
        assert_eq!(
            publisher.recover("module-1").unwrap().as_deref(),
            Some("1.0.0")
        );
        let context = publisher
            .execution_context("default-user", "module-1")
            .await
            .unwrap();
        assert_eq!(context.module_identity().unwrap().0, "module-1");

        // Simulate a crash after a file pointer switch but before the DB
        // transaction commits; startup must restore the authoritative DB one.
        publisher.write_pointer("module-1", "0.0.1").unwrap();
        assert_eq!(
            publisher.recover_registered().await.unwrap(),
            vec!["module-1"]
        );
        assert_eq!(
            publisher.recover("module-1").unwrap().as_deref(),
            Some("1.0.0")
        );
    }

    #[tokio::test]
    async fn recovery_accepts_uppercase_manifest_digest_and_rejects_traversal_pointer() {
        let source = tempdir().unwrap();
        fs::write(source.path().join("entry.js"), "export default {};").unwrap();
        let digest = package_digest(source.path()).unwrap();
        write_package(source.path(), &digest);
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db);
        let staged = publisher.stage(source.path()).unwrap();
        publisher.commit(staged).await.unwrap();

        let manifest_path = store.path().join("versions/module-1/1.0.0/manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["artifact"]["sha256"] = digest.to_ascii_uppercase().into();
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(
            publisher.recover("module-1").unwrap().as_deref(),
            Some("1.0.0")
        );

        let sentinel = store.path().join("versions/outside");
        fs::write(&sentinel, "keep").unwrap();
        publisher
            .write_pointer("module-1", "../../outside")
            .unwrap();
        assert_eq!(publisher.recover("module-1").unwrap(), None);
        assert_eq!(fs::read_to_string(sentinel).unwrap(), "keep");
    }

    #[tokio::test]
    async fn database_publish_failure_restores_pointer_and_keeps_old_module_row() {
        let source = tempdir().unwrap();
        fs::write(source.path().join("entry.js"), "export default {};").unwrap();
        let digest = package_digest(source.path()).unwrap();
        write_package(source.path(), &digest);
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        insert_module_row(
            &db,
            "0.9.0",
            &"0".repeat(64),
            serde_json::json!({"permissions": []}),
        )
        .await;
        db.execute_unprepared(
            "CREATE TRIGGER fail_module_version BEFORE INSERT ON module_versions BEGIN SELECT RAISE(ABORT, 'forced failure'); END;",
        )
        .await
        .unwrap();

        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db.clone());
        publisher.write_pointer("module-1", "0.9.0").unwrap();
        let staged = publisher.stage(source.path()).unwrap();
        assert!(publisher.commit(staged).await.is_err());

        assert_eq!(
            fs::read_to_string(store.path().join("current/module-1/version")).unwrap(),
            "0.9.0\n"
        );
        let module = modules::Entity::find_by_id("module-1")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(module.current_version, "0.9.0");
        let events = module_update_events::Entity::find().all(&db).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, "failure");
    }

    #[tokio::test]
    async fn concurrent_commits_keep_database_and_file_pointer_coherent() {
        let source_a = tempdir().unwrap();
        write_versioned_package(source_a.path(), "1.0.0");
        let source_b = tempdir().unwrap();
        write_versioned_package(source_b.path(), "2.0.0");
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher_a = ModulePublisher::new(store.path(), db.clone());
        let publisher_b = ModulePublisher::new(store.path(), db.clone());
        let staged_a = publisher_a.stage(source_a.path()).unwrap();
        let staged_b = publisher_b.stage(source_b.path()).unwrap();
        let (result_a, result_b) =
            tokio::join!(publisher_a.commit(staged_a), publisher_b.commit(staged_b),);
        result_a.unwrap();
        result_b.unwrap();
        let module = modules::Entity::find_by_id("module-1")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            publisher_a.recover("module-1").unwrap().as_deref(),
            Some(module.current_version.as_str())
        );
    }

    #[tokio::test]
    async fn first_install_failure_is_audited_without_creating_module_identity() {
        let source = tempdir().unwrap();
        fs::write(source.path().join("entry.js"), "export default {};").unwrap();
        let digest = package_digest(source.path()).unwrap();
        write_package(source.path(), &digest);
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db.execute_unprepared(
            "CREATE TRIGGER fail_module_version BEFORE INSERT ON module_versions BEGIN SELECT RAISE(ABORT, 'forced failure'); END;",
        )
        .await
        .unwrap();

        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db.clone());
        let staged = publisher.stage(source.path()).unwrap();
        assert!(publisher.commit(staged).await.is_err());
        assert!(modules::Entity::find_by_id("module-1")
            .one(&db)
            .await
            .unwrap()
            .is_none());
        let events = module_update_events::Entity::find().all(&db).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, "failure");
        assert_eq!(events[0].stage, "publish_commit");
    }

    #[tokio::test]
    async fn verified_package_commit_registers_identity_from_manifest() {
        let source = tempdir().unwrap();
        fs::write(source.path().join("entry.js"), "export default {};").unwrap();
        let digest = package_digest(source.path()).unwrap();
        write_package(source.path(), &digest);
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db.clone());
        let staged = publisher.stage(source.path()).unwrap();
        publisher.commit(staged).await.unwrap();
        let module = modules::Entity::find_by_id("module-1")
            .one(&db)
            .await
            .unwrap()
            .expect("publisher registers package identity");
        assert_eq!(module.current_version, "1.0.0");
        assert_eq!(module.integrity, digest);
        assert_eq!(module.manifest["id"], "module-1");
        assert_eq!(
            publisher.recover("module-1").unwrap().as_deref(),
            Some("1.0.0")
        );
    }

    #[tokio::test]
    async fn execution_context_rejects_database_integrity_that_does_not_match_package() {
        let source = tempdir().unwrap();
        fs::write(source.path().join("entry.js"), "export default {};").unwrap();
        let digest = package_digest(source.path()).unwrap();
        write_package(source.path(), &digest);
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        insert_module_row(
            &db,
            "1.0.0",
            &"a".repeat(64),
            serde_json::json!({"permissions": []}),
        )
        .await;
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db.clone());
        let staged = publisher.stage(source.path()).unwrap();
        // The package is published, but the DB row deliberately claims a
        // different digest. Context minting must fail closed.
        publisher.commit(staged).await.unwrap();
        db.execute_unprepared("UPDATE modules SET integrity = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE id = 'module-1';")
            .await
            .unwrap();
        assert!(publisher
            .execution_context("default-user", "module-1")
            .await
            .is_err());
    }

    #[test]
    fn module_ids_cannot_escape_the_package_store() {
        assert!(!valid_module_id("../outside"));
        assert!(!valid_module_id("nested/module"));
        assert!(valid_module_id("@dgos-workbench"));
    }
}
