use dgos_db::{
    entities::{module_update_events, module_versions, modules},
    sea_orm::{
        ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
        QueryFilter, Set, TransactionTrait,
    },
    ModulePermission, ModulePermissionRepository,
};
use dgos_types::{HostExecutionContext, HostSession};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use thiserror::Error;
use uuid::Uuid;

mod admission;
mod bundled;
mod call_channel;
mod compat;
mod digest;
mod loader;
mod management;
mod migration;
mod page_loader;
mod release_gate;
mod runtime;
mod sources;
mod supervisor;

pub use admission::{ModuleAdmission, PinnedModuleAdmission};
pub use bundled::{ProvisionError, ProvisionReport};
pub use call_channel::{
    BoundModuleCall, CallRejection, ModuleCallRoute, ModuleCallRouter, MAX_IN_FLIGHT_CALLS,
};
pub use compat::{Requires, HOST_API_VERSION, HOST_REACT_VERSION};
pub use digest::package_integrity;
pub use loader::{
    LoadPhase, LocalArtifactLoader, ModuleLoadError, ModuleLoader, PageFailureStage,
    VerifiedModuleArtifact,
};
pub use management::{ManagementError, PublishedModule};
pub use migration::IntegrityMigrationReport;
pub use page_loader::{
    Attachment, ModulePageTransport, PageInstruction, PageLoaderConfig, PageModuleLoader,
    PageReceipt, PageSendError, ReceiptDisposition, RuntimeLost,
};
pub use release_gate::{
    verify_builtin_release_bundle, verify_release_bundle, ReleasePin, ReleaseViolation,
};
pub use runtime::{
    ModuleRequest, ModuleRequestOutcome, ModuleRequestStats, ModuleRuntimeError,
    ModuleRuntimeHandle, ModuleRuntimeState,
};
pub use sources::{LocalModuleSources, ModuleSourceError};
pub use supervisor::{
    LoaderTimeouts, ModuleReadyObserver, ModuleRuntimeSupervisor, ModuleSupervisorError,
    ModuleUpdateOutcome, ModuleUpdateReport,
};

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
    /// No row is registered for this module id: it was never published.
    #[error("module is not published")]
    NotPublished,
    /// The module is registered but disabled (D035).
    #[error("module is disabled")]
    Disabled,
    /// The manifest's `requires` is not satisfied by this host (D055).
    #[error("module is not compatible with this host version")]
    Incompatible,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Requires>,
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

struct ReleaseLease {
    db: DatabaseConnection,
    owner_id: String,
    lost: Arc<AtomicBool>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    heartbeat: Option<tokio::task::JoinHandle<()>>,
}

impl ReleaseLease {
    fn ensure_valid(&self) -> Result<(), ModulePackageError> {
        if self.lost.load(Ordering::Acquire) {
            Err(ModulePackageError::Database(
                "module release lease lost".into(),
            ))
        } else {
            Ok(())
        }
    }
}

impl Drop for ReleaseLease {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(handle) = self.heartbeat.take() {
            handle.abort();
        }
        let db = self.db.clone();
        let owner = self.owner_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = db
                    .execute_unprepared(&format!(
                        "DELETE FROM module_release_locks WHERE lock_key = '{}' AND owner_id = '{}'",
                        ModulePublisher::RELEASE_LOCK_KEY,
                        owner
                    ))
                    .await;
            });
        }
    }
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
        session: &HostSession,
        module_id: &str,
    ) -> Result<HostExecutionContext, ModulePackageError> {
        let module = modules::Entity::find_by_id(module_id)
            .one(&self.db)
            .await
            .map_err(db_error)?
            .ok_or_else(|| ModulePackageError::Database("module is not registered".into()))?;
        if !module.is_enabled
            || self.pointer_version(module_id).as_deref() != Some(module.current_version.as_str())
            || !self.current_package_matches(
                module_id,
                &module.current_version,
                &module.integrity,
                &module.manifest,
            )?
        {
            return Err(ModulePackageError::Manifest(
                "module is not recoverable".into(),
            ));
        }
        Ok(HostExecutionContext::for_module_session(
            session,
            module_id,
            &module.current_version,
            &module.integrity,
        ))
    }

    /// Issue a runtime handle only for an enabled release whose DB row, file
    /// pointer, committed version row and on-disk artifact agree. The handle
    /// starts in `Starting`; only the supervisor marks it ready after a load.
    pub(crate) async fn runtime_handle(
        &self,
        session: &HostSession,
        module_id: &str,
    ) -> Result<ModuleRuntimeHandle, ModulePackageError> {
        if !valid_module_id(module_id) {
            return Err(ModulePackageError::Manifest("invalid module id".into()));
        }
        let module = modules::Entity::find_by_id(module_id)
            .one(&self.db)
            .await
            .map_err(db_error)?
            .ok_or(ModulePackageError::NotPublished)?;
        if !module.is_enabled {
            return Err(ModulePackageError::Disabled);
        }
        let pointer =
            fs::read_to_string(self.root.join("current").join(module_id).join("version")).ok();
        if pointer.as_deref().map(str::trim) != Some(module.current_version.as_str()) {
            return Err(ModulePackageError::Manifest(
                "module pointer does not match the published release".into(),
            ));
        }
        // Only versions that went through `commit` are loadable; a directory
        // and DB row written by any other path is not a verified release.
        let committed = module_versions::Entity::find()
            .filter(module_versions::Column::ModuleId.eq(module_id))
            .filter(module_versions::Column::Version.eq(&module.current_version))
            .one(&self.db)
            .await
            .map_err(db_error)?
            .is_some_and(|row| row.integrity.eq_ignore_ascii_case(&module.integrity));
        if !committed {
            return Err(ModulePackageError::Manifest(
                "module release was not committed by the publisher".into(),
            ));
        }
        let artifact = self.verify_artifact(
            &self
                .root
                .join("versions")
                .join(module_id)
                .join(&module.current_version),
            module_id,
            &module.current_version,
            &module.integrity,
            true,
            Some(&module.manifest),
        )?;
        let context = HostExecutionContext::for_module_session(
            session,
            module_id,
            &module.current_version,
            &module.integrity,
        );
        Ok(ModuleRuntimeHandle::issue(context, artifact))
    }

    /// The single artifact check used before any loader sees a package. The
    /// package is read once (D057): identity, manifest/release agreement, the
    /// v2 digest and the entry bytes all come from that one read, so what is
    /// verified is exactly what a loader executes. `integrity` must be a v2
    /// value; legacy values never execute. `published_manifest` is the
    /// manifest recorded for the release (the `modules` row or a rollback
    /// snapshot) and must agree with the one on disk. `require_compatible`
    /// also enforces the manifest's host requirements; rollback to a
    /// previously published release passes `false`.
    fn verify_artifact(
        &self,
        package_root: &Path,
        module_id: &str,
        version: &str,
        integrity: &str,
        require_compatible: bool,
        published_manifest: Option<&serde_json::Value>,
    ) -> Result<VerifiedModuleArtifact, ModulePackageError> {
        if !valid_module_id(module_id) || !valid_version_path(version) {
            return Err(ModulePackageError::Manifest(
                "invalid module identity".into(),
            ));
        }
        if !digest::is_v2_integrity(integrity) {
            return Err(ModulePackageError::Integrity);
        }
        let snapshot = digest::read_package(package_root, false)?;
        let manifest = &snapshot.manifest;
        if manifest.id != module_id
            || manifest.version != version
            || manifest.artifact.version != version
        {
            return Err(ModulePackageError::Manifest(
                "manifest does not match the release".into(),
            ));
        }
        if snapshot.integrity != integrity {
            return Err(ModulePackageError::Integrity);
        }
        if let Some(published) = published_manifest {
            if !manifest_matches(published, manifest) {
                return Err(ModulePackageError::Manifest(
                    "manifest does not match the published record".into(),
                ));
            }
        }
        if require_compatible && compat::check(manifest.requires.as_ref()).is_err() {
            return Err(ModulePackageError::Incompatible);
        }
        Ok(VerifiedModuleArtifact::new(
            module_id.to_owned(),
            version.to_owned(),
            integrity.to_owned(),
            snapshot.entry,
        ))
    }

    /// Re-verify a staged package so a load smoke can run before the
    /// current-version pointer moves.
    pub(crate) fn verify_staged(
        &self,
        package: &StagedPackage,
    ) -> Result<VerifiedModuleArtifact, ModulePackageError> {
        self.verify_artifact(
            &package.staging_path,
            &package.module_id,
            &package.version,
            &package.integrity,
            true,
            None,
        )
    }

    /// Remove a staged copy that will not be committed. Already-committed or
    /// already-removed staging paths are ignored.
    pub(crate) fn discard_staged(&self, package: &StagedPackage) {
        let _ = fs::remove_dir_all(&package.staging_path);
    }

    /// Authoritative state of an enabled release, captured before a hot
    /// update so a failed activation can be rolled back exactly.
    pub(crate) async fn release_snapshot(
        &self,
        module_id: &str,
    ) -> Result<Option<ReleaseSnapshot>, ModulePackageError> {
        Ok(modules::Entity::find_by_id(module_id)
            .one(&self.db)
            .await
            .map_err(db_error)?
            .filter(|row| row.is_enabled)
            .map(|row| ReleaseSnapshot {
                module_id: row.id,
                version: row.current_version,
                integrity: row.integrity,
                manifest: row.manifest,
                entry_url: row.entry_url,
            }))
    }

    /// Roll a failed activation back to `snapshot`. The DB update is a CAS on
    /// the failed release, so a concurrent publish is never overwritten.
    pub(crate) async fn reactivate(
        &self,
        operation_id: &str,
        snapshot: &ReleaseSnapshot,
        failed_version: &str,
        failed_integrity: &str,
    ) -> Result<(), ModulePackageError> {
        let _publish_guard = self.publish_lock.lock().await;
        let lease = self.acquire_release_lock().await?;
        lease.ensure_valid()?;
        self.verify_artifact(
            &self
                .root
                .join("versions")
                .join(&snapshot.module_id)
                .join(&snapshot.version),
            &snapshot.module_id,
            &snapshot.version,
            &snapshot.integrity,
            false,
            Some(&snapshot.manifest),
        )?;
        // If the DB step below fails, the caller disables the module, which
        // removes this pointer again; startup recovery follows the DB row.
        self.write_pointer(&snapshot.module_id, &snapshot.version)?;
        let tx = self.db.begin().await.map_err(db_error)?;
        let row = modules::Entity::find_by_id(&snapshot.module_id)
            .one(&tx)
            .await
            .map_err(db_error)?
            .filter(|row| {
                row.current_version == failed_version && row.integrity == failed_integrity
            })
            .ok_or_else(|| {
                ModulePackageError::Database("module release changed during rollback".into())
            })?;
        let mut model: modules::ActiveModel = row.into();
        model.current_version = Set(snapshot.version.clone());
        model.integrity = Set(snapshot.integrity.clone());
        model.manifest = Set(snapshot.manifest.clone());
        model.entry_url = Set(snapshot.entry_url.clone());
        model.updated_at = Set(chrono::Utc::now().timestamp());
        model.update(&tx).await.map_err(db_error)?;
        insert_update_event(
            &tx,
            operation_id,
            &snapshot.module_id,
            Some(failed_version),
            &snapshot.version,
            "rolled_back",
            "success",
            None,
        )
        .await?;
        lease.ensure_valid()?;
        tx.commit().await.map_err(db_error)
    }

    /// Fail-closed end state after a failed rollback: the module row is
    /// disabled first (capability checks read it), then the pointer removed.
    pub(crate) async fn disable_module(
        &self,
        operation_id: &str,
        module_id: &str,
        from_version: Option<&str>,
        to_version: &str,
        error_class: &str,
    ) -> Result<(), ModulePackageError> {
        let _publish_guard = self.publish_lock.lock().await;
        let lease = self.acquire_release_lock().await?;
        lease.ensure_valid()?;
        let tx = self.db.begin().await.map_err(db_error)?;
        if let Some(row) = modules::Entity::find_by_id(module_id)
            .one(&tx)
            .await
            .map_err(db_error)?
        {
            let mut model: modules::ActiveModel = row.into();
            model.is_enabled = Set(false);
            model.updated_at = Set(chrono::Utc::now().timestamp());
            model.update(&tx).await.map_err(db_error)?;
        }
        insert_update_event(
            &tx,
            operation_id,
            module_id,
            from_version,
            to_version,
            "disabled",
            "failure",
            Some(error_class.to_owned()),
        )
        .await?;
        lease.ensure_valid()?;
        tx.commit().await.map_err(db_error)?;
        let pointer_dir = self.root.join("current").join(module_id);
        match fs::remove_file(pointer_dir.join("version")) {
            Ok(()) => fs::File::open(&pointer_dir)?.sync_all()?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
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
        let Some(module) = modules::Entity::find_by_id(module_id)
            .one(&self.db)
            .await
            .map_err(db_error)?
            .filter(|module| {
                module.is_enabled
                    && module.current_version == version
                    && module.integrity == integrity
            })
        else {
            return Ok(false);
        };
        self.current_package_matches(module_id, version, integrity, &module.manifest)
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
        let snapshot = digest::read_package(staging, false)?;
        let manifest = snapshot.manifest;
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
            || !valid_relative_entry(&manifest.entry)
        {
            return Err(ModulePackageError::Manifest(
                "invalid identity, version or local entry".into(),
            ));
        }
        for permission in &manifest.permissions {
            ModulePermission::parse(permission)
                .map_err(|_| ModulePackageError::Manifest("invalid module permission".into()))?;
        }
        // The declared digest is a build-tool consistency check for new
        // packages. It is not part of the digest and is never read at run
        // time (D057).
        let declared_matches = digest::v2_hex(&snapshot.integrity)
            .is_some_and(|hex| manifest.artifact.sha256.eq_ignore_ascii_case(hex));
        if !declared_matches {
            return Err(ModulePackageError::Integrity);
        }
        if compat::check(manifest.requires.as_ref()).is_err() {
            return Err(ModulePackageError::Incompatible);
        }
        Ok((manifest, snapshot.integrity))
    }

    /// Atomically publish files and then update the DB pointer. The old
    /// version remains on disk so startup recovery can roll back safely.
    pub async fn commit(&self, package: StagedPackage) -> Result<(), ModulePackageError> {
        self.commit_with(package, FirstPublish::default()).await
    }

    /// `commit` with the state a first-time row is created in (D058).
    pub(crate) async fn commit_with(
        &self,
        package: StagedPackage,
        first: FirstPublish,
    ) -> Result<(), ModulePackageError> {
        let _publish_guard = self.publish_lock.lock().await;
        let lease = self.acquire_release_lock().await?;
        self.commit_locked(package, &lease, first).await
    }

    async fn commit_locked(
        &self,
        package: StagedPackage,
        lease: &ReleaseLease,
        first: FirstPublish,
    ) -> Result<(), ModulePackageError> {
        lease.ensure_valid()?;
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
        fs::create_dir_all(version_dir.parent().unwrap())?;
        if version_dir.exists() {
            if self
                .existing_release_matches(&package.module_id, &package.version, &package.integrity)
                .await?
            {
                let _ = fs::remove_dir_all(&package.staging_path);
                return Ok(());
            }
            if self
                .release_is_registered(&package.module_id, &package.version)
                .await?
            {
                return Err(ModulePackageError::Manifest(
                    "version already exists with a different integrity".into(),
                ));
            }
            // D057: no committed row refers to this directory, so it is the
            // leftover of an interrupted commit. Redo it from the staged copy
            // that was just validated instead of trusting the leftover.
            fs::remove_dir_all(&version_dir)?;
        }
        fs::rename(&package.staging_path, &version_dir)?;
        fs::File::open(version_dir.parent().unwrap())?.sync_all()?;
        // The staged copy was validated before the rename. Read what is now
        // in place once more, so a change in between never becomes a release.
        let renamed = digest::read_package(&version_dir, false).map(|read| read.integrity);
        if !matches!(&renamed, Ok(integrity) if integrity.eq_ignore_ascii_case(&package.integrity))
        {
            let _ = fs::remove_dir_all(&version_dir);
            return Err(ModulePackageError::Integrity);
        }
        let pointer_dir = self.root.join("current").join(&package.module_id);
        let previous = fs::read_to_string(pointer_dir.join("version")).ok();
        let publish_result = async {
            lease.ensure_valid()?;
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
                    is_official: Set(first.official),
                    is_enabled: Set(first.enabled),
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
            lease.ensure_valid()?;
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

    /// A committed release: a `module_versions` row, or the version a
    /// module row currently points at.
    async fn release_is_registered(
        &self,
        module_id: &str,
        version: &str,
    ) -> Result<bool, ModulePackageError> {
        let version_row = module_versions::Entity::find()
            .filter(module_versions::Column::ModuleId.eq(module_id))
            .filter(module_versions::Column::Version.eq(version))
            .one(&self.db)
            .await
            .map_err(db_error)?;
        if version_row.is_some() {
            return Ok(true);
        }
        Ok(modules::Entity::find_by_id(module_id)
            .one(&self.db)
            .await
            .map_err(db_error)?
            .is_some_and(|row| row.current_version == version))
    }

    async fn existing_release_matches(
        &self,
        module_id: &str,
        version: &str,
        integrity: &str,
    ) -> Result<bool, ModulePackageError> {
        let package = self.root.join("versions").join(module_id).join(version);
        match digest::read_package(&package, false) {
            Ok(read) if read.integrity.eq_ignore_ascii_case(integrity) => {}
            Ok(_) | Err(ModulePackageError::Manifest(_)) => return Ok(false),
            Err(ModulePackageError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(false)
            }
            Err(error) => return Err(error),
        }
        let row = module_versions::Entity::find()
            .filter(module_versions::Column::ModuleId.eq(module_id))
            .filter(module_versions::Column::Version.eq(version))
            .one(&self.db)
            .await
            .map_err(db_error)?;
        Ok(row.is_some_and(|row| row.integrity.eq_ignore_ascii_case(integrity)))
    }

    /// The version the current-version pointer names, if any.
    fn pointer_version(&self, module_id: &str) -> Option<String> {
        if !valid_module_id(module_id) {
            return None;
        }
        fs::read_to_string(self.root.join("current").join(module_id).join("version"))
            .ok()
            .map(|value| value.trim().to_owned())
    }

    /// On startup, only keep a pointer whose package still equals the
    /// integrity committed for that version. A missing or corrupt pointer is
    /// removed so the caller can disable the module instead of loading
    /// unknown code.
    pub async fn recover(&self, module_id: &str) -> Result<Option<String>, ModulePackageError> {
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
        let committed = module_versions::Entity::find()
            .filter(module_versions::Column::ModuleId.eq(module_id))
            .filter(module_versions::Column::Version.eq(&version))
            .one(&self.db)
            .await
            .map_err(db_error)?
            .map(|row| row.integrity);
        let package = self.root.join("versions").join(module_id).join(&version);
        let valid = match committed {
            Some(integrity) => {
                match self.verify_artifact(&package, module_id, &version, &integrity, false, None) {
                    Ok(_) => true,
                    Err(ModulePackageError::Io(error))
                        if error.kind() != io::ErrorKind::NotFound =>
                    {
                        return Err(error.into())
                    }
                    Err(_) => false,
                }
            }
            None => false,
        };
        if !valid {
            fs::remove_file(pointer)?;
            return Ok(None);
        }
        Ok(Some(version))
    }

    /// Whether the package on disk is exactly the release the `modules` row
    /// records: integrity (v2) and manifest, from one read.
    fn current_package_matches(
        &self,
        module_id: &str,
        version: &str,
        integrity: &str,
        manifest: &serde_json::Value,
    ) -> Result<bool, ModulePackageError> {
        let package = self.root.join("versions").join(module_id).join(version);
        match self.verify_artifact(
            &package,
            module_id,
            version,
            integrity,
            false,
            Some(manifest),
        ) {
            Ok(_) => Ok(true),
            Err(ModulePackageError::Integrity | ModulePackageError::Manifest(_)) => Ok(false),
            Err(ModulePackageError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    /// Reconcile every registered module before any future loader is allowed
    /// to execute code. Legacy integrity values are migrated first (D057);
    /// invalid pointers are fail-closed by disabling the row.
    pub async fn recover_registered(&self) -> Result<Vec<String>, ModulePackageError> {
        let _publish_guard = self.publish_lock.lock().await;
        let lease = self.acquire_release_lock().await?;
        let result = async {
            lease.ensure_valid()?;
            self.migrate_legacy_locked(&lease).await?;
            let rows = modules::Entity::find()
                .all(&self.db)
                .await
                .map_err(db_error)?;
            let mut ready = Vec::new();
            for row in rows {
                lease.ensure_valid()?;
                let valid = self
                    .current_package_matches(
                        &row.id,
                        &row.current_version,
                        &row.integrity,
                        &row.manifest,
                    )
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
            self.cleanup_orphans().await?;
            Ok::<_, ModulePackageError>(ready)
        }
        .await;
        result
    }

    /// Remove package directories that were moved out of staging but never
    /// became part of a committed database version. This is deliberately
    /// allow-list based: only versions represented by the DB or current
    /// module rows survive startup recovery.
    pub async fn cleanup_orphans(&self) -> Result<usize, ModulePackageError> {
        let version_rows = module_versions::Entity::find()
            .all(&self.db)
            .await
            .map_err(db_error)?;
        let module_rows = modules::Entity::find()
            .all(&self.db)
            .await
            .map_err(db_error)?;
        let mut allowed = HashSet::new();
        for row in version_rows {
            allowed.insert((row.module_id, row.version));
        }
        for row in module_rows {
            allowed.insert((row.id, row.current_version));
        }

        let mut removed = 0;
        let staging = self.root.join("staging");
        if staging.is_dir() {
            for entry in fs::read_dir(&staging)? {
                let path = entry?.path();
                if path.is_dir() {
                    fs::remove_dir_all(path)?;
                    removed += 1;
                }
            }
        }
        let versions_root = self.root.join("versions");
        if versions_root.is_dir() {
            for module_entry in fs::read_dir(&versions_root)? {
                let module_path = module_entry?.path();
                let Some(module_id) = module_path.file_name().and_then(|value| value.to_str())
                else {
                    continue;
                };
                if !module_path.is_dir() {
                    continue;
                }
                for version_entry in fs::read_dir(&module_path)? {
                    let version_path = version_entry?.path();
                    let Some(version) = version_path.file_name().and_then(|value| value.to_str())
                    else {
                        continue;
                    };
                    if version_path.is_dir()
                        && !allowed.contains(&(module_id.to_owned(), version.to_owned()))
                    {
                        fs::remove_dir_all(version_path)?;
                        removed += 1;
                    }
                }
            }
        }
        Ok(removed)
    }

    const RELEASE_LOCK_KEY: &'static str = "modules";
    const RELEASE_LOCK_TTL_SECONDS: i64 = 60;

    async fn acquire_release_lock(&self) -> Result<ReleaseLease, ModulePackageError> {
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
                    let lost = Arc::new(AtomicBool::new(false));
                    let (stop, mut stop_rx) = tokio::sync::oneshot::channel();
                    let db = self.db.clone();
                    let owner = self.owner_id.clone();
                    let lost_for_task = lost.clone();
                    let heartbeat = tokio::spawn(async move {
                        let mut interval =
                            tokio::time::interval(std::time::Duration::from_secs(15));
                        loop {
                            tokio::select! {
                                _ = interval.tick() => {
                                    let now = chrono::Utc::now().timestamp();
                                    let expires = now + ModulePublisher::RELEASE_LOCK_TTL_SECONDS;
                                    let result = db.execute_unprepared(&format!(
                                        "UPDATE module_release_locks SET expires_at = {expires} WHERE lock_key = '{}' AND owner_id = '{}' AND expires_at > {now}",
                                        ModulePublisher::RELEASE_LOCK_KEY, owner
                                    )).await;
                                    if !matches!(result, Ok(result) if result.rows_affected() == 1) {
                                        lost_for_task.store(true, Ordering::Release);
                                        break;
                                    }
                                }
                                _ = &mut stop_rx => break,
                            }
                        }
                    });
                    return Ok(ReleaseLease {
                        db: self.db.clone(),
                        owner_id: self.owner_id.clone(),
                        lost,
                        stop: Some(stop),
                        heartbeat: Some(heartbeat),
                    });
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
        insert_update_event(
            &self.db,
            operation_id,
            module_id,
            from_version,
            to_version,
            stage,
            outcome,
            error_class,
        )
        .await
    }
}

/// State a module row is created in by its first publish (D058). Only used
/// when no row exists; updates never change `official` or `enabled`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FirstPublish {
    pub(crate) official: bool,
    pub(crate) enabled: bool,
}

impl Default for FirstPublish {
    fn default() -> Self {
        Self {
            official: false,
            enabled: true,
        }
    }
}

/// Whether the manifest recorded for a release equals the one on disk,
/// ignoring `artifact.sha256` (D057). A record that does not parse as a
/// manifest never matches.
fn manifest_matches(recorded: &serde_json::Value, on_disk: &ModuleManifest) -> bool {
    let Ok(recorded) = serde_json::from_value::<ModuleManifest>(recorded.clone()) else {
        return false;
    };
    digest::manifest_canon(&recorded) == digest::manifest_canon(on_disk)
}

/// Authoritative release fields captured before a hot update (D033).
#[derive(Clone, Debug)]
pub(crate) struct ReleaseSnapshot {
    module_id: String,
    version: String,
    integrity: String,
    manifest: serde_json::Value,
    entry_url: Option<String>,
}

impl ReleaseSnapshot {
    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    pub(crate) fn integrity(&self) -> &str {
        &self.integrity
    }
}

async fn insert_update_event(
    db: &impl ConnectionTrait,
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
    .insert(db)
    .await
    .map(|_| ())
    .map_err(db_error)
}

/// The single module-id rule shared by the publisher, source resolver and
/// host operation manager: a short ASCII name that is never `.` or `..`.
pub fn valid_module_id(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
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

/// Entries are package-relative paths over a conservative ASCII alphabet, so
/// URL schemes (`https:`, `data:`, `file:`), query/fragment, percent-encoded
/// traversal, backslashes and drive letters can never reach a loader.
fn valid_relative_entry(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && value.len() <= 255
        && !path.is_absolute()
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn error_class(error: &ModulePackageError) -> String {
    match error {
        ModulePackageError::Io(_) => "io".into(),
        ModulePackageError::Manifest(_) => "manifest".into(),
        ModulePackageError::Integrity => "integrity".into(),
        ModulePackageError::Incompatible => "incompatible".into(),
        ModulePackageError::Database(_) => "database".into(),
        ModulePackageError::NotPublished => "not_published".into(),
        ModulePackageError::Disabled => "disabled".into(),
    }
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

    /// Write `manifest` with its digest sealed in: placeholder first, then
    /// the v2 value of the resulting package (the manifest is part of the
    /// digest, the declared `sha256` is not). Returns the integrity, or an
    /// empty string when the manifest cannot be digested.
    fn seal_manifest(dir: &Path, manifest: &serde_json::Value) -> String {
        let mut manifest = manifest.clone();
        manifest["artifact"]["sha256"] = "0".repeat(64).into();
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let Ok(integrity) = package_integrity(dir) else {
            return String::new();
        };
        manifest["artifact"]["sha256"] = digest::v2_hex(&integrity).unwrap().into();
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        integrity
    }

    fn example_manifest(version: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "module-1", "name": "Example", "version": version, "entry": "entry.js",
            "artifact": {"version": version, "sha256": "0".repeat(64)},
            "permissions": []
        })
    }

    /// An entry plus a sealed manifest for `module-1@1.0.0`.
    fn write_package(dir: &Path) -> String {
        fs::write(dir.join("entry.js"), "export default {};").unwrap();
        seal_manifest(dir, &example_manifest("1.0.0"))
    }

    fn write_versioned_package(dir: &Path, version: &str) -> String {
        fs::write(dir.join("entry.js"), format!("export default '{version}';")).unwrap();
        seal_manifest(dir, &example_manifest(version))
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
    async fn a_declared_digest_that_is_not_the_v2_digest_is_rejected() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("entry.js"), "export default {};").unwrap();
        fs::write(
            root.path().join("manifest.json"),
            serde_json::to_vec(&example_manifest("1.0.0")).unwrap(),
        )
        .unwrap();
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
        let mut manifest = example_manifest("1.0.0");
        manifest["permissions"] = serde_json::json!(["providers.unknown"]);
        seal_manifest(root.path(), &manifest);
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
        write_package(source.path());
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
            publisher.recover("module-1").await.unwrap().as_deref(),
            Some("1.0.0")
        );
        let context = publisher
            .execution_context(&dgos_types::HostSession::local_default(), "module-1")
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
            publisher.recover("module-1").await.unwrap().as_deref(),
            Some("1.0.0")
        );
    }

    #[tokio::test]
    async fn recovery_does_not_read_the_declared_digest_and_rejects_traversal_pointer() {
        let source = tempdir().unwrap();
        let digest = write_package(source.path());
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db);
        let staged = publisher.stage(source.path()).unwrap();
        publisher.commit(staged).await.unwrap();

        // `artifact.sha256` is not authoritative at run time (D057): any
        // value there leaves the package and its recovery unchanged.
        let manifest_path = store.path().join("versions/module-1/1.0.0/manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["artifact"]["sha256"] =
            digest::v2_hex(&digest).unwrap().to_ascii_uppercase().into();
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(
            publisher.recover("module-1").await.unwrap().as_deref(),
            Some("1.0.0")
        );

        let sentinel = store.path().join("versions/outside");
        fs::write(&sentinel, "keep").unwrap();
        publisher
            .write_pointer("module-1", "../../outside")
            .unwrap();
        assert_eq!(publisher.recover("module-1").await.unwrap(), None);
        assert_eq!(fs::read_to_string(sentinel).unwrap(), "keep");
    }

    fn write_package_requiring(dir: &Path, requires: Option<serde_json::Value>) {
        fs::write(dir.join("entry.js"), "export default {};").unwrap();
        let mut manifest = example_manifest("1.0.0");
        if let Some(requires) = requires {
            manifest["requires"] = requires;
        }
        seal_manifest(dir, &manifest);
    }

    #[tokio::test]
    async fn stage_enforces_declared_host_requirements() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db);

        for requires in [
            serde_json::json!({"react": "^20"}),
            serde_json::json!({"api": HOST_API_VERSION + 1}),
        ] {
            let source = tempdir().unwrap();
            write_package_requiring(source.path(), Some(requires));
            assert!(matches!(
                publisher.stage(source.path()),
                Err(ModulePackageError::Incompatible)
            ));
        }
        assert!(fs::read_dir(store.path().join("staging"))
            .unwrap()
            .next()
            .is_none());

        for requires in [
            None,
            Some(serde_json::json!({})),
            Some(serde_json::json!({"react": "^19", "api": HOST_API_VERSION})),
        ] {
            let source = tempdir().unwrap();
            write_package_requiring(source.path(), requires);
            publisher.stage(source.path()).unwrap();
        }

        let source = tempdir().unwrap();
        write_package_requiring(
            source.path(),
            Some(serde_json::json!({"react": "not a range"})),
        );
        assert!(matches!(
            publisher.stage(source.path()),
            Err(ModulePackageError::Manifest(_))
        ));
        assert_eq!(
            error_class(&ModulePackageError::Incompatible),
            "incompatible"
        );
    }

    #[tokio::test]
    async fn recovery_keeps_an_already_published_incompatible_module_enabled() {
        // Compatibility is checked at stage and start, never at recovery
        // (D055). Such a release cannot be staged any more, so it is placed
        // the way an older host published it.
        let source = tempdir().unwrap();
        write_package_requiring(source.path(), Some(serde_json::json!({"react": "^20"})));
        let integrity = package_integrity(source.path()).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(source.path().join("manifest.json")).unwrap())
                .unwrap();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let version_dir = store.path().join("versions/module-1/1.0.0");
        copy_dir(source.path(), &version_dir).unwrap();
        insert_module_row(&db, "1.0.0", &integrity, manifest).await;
        module_versions::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            module_id: Set("module-1".into()),
            version: Set("1.0.0".into()),
            integrity: Set(integrity),
            entry_path: Set("entry.js".into()),
            created_at: Set(0),
        }
        .insert(&db)
        .await
        .unwrap();
        let publisher = ModulePublisher::new(store.path(), db.clone());

        assert_eq!(
            publisher.recover_registered().await.unwrap(),
            vec!["module-1"]
        );
        assert_eq!(
            publisher.recover("module-1").await.unwrap().as_deref(),
            Some("1.0.0")
        );
        let row = modules::Entity::find_by_id("module-1")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(row.is_enabled);
    }

    #[tokio::test]
    async fn database_publish_failure_restores_pointer_and_keeps_old_module_row() {
        let source = tempdir().unwrap();
        write_package(source.path());
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
            publisher_a.recover("module-1").await.unwrap().as_deref(),
            Some(module.current_version.as_str())
        );
    }

    #[tokio::test]
    async fn expired_release_lease_is_reclaimed_but_wrong_owner_cannot_delete_it() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let first = ModulePublisher::new(store.path(), db.clone());
        let second = ModulePublisher::new(store.path(), db.clone());

        let lease = first.acquire_release_lock().await.unwrap();
        db.execute_unprepared(
            "DELETE FROM module_release_locks WHERE lock_key = 'modules' AND owner_id = 'not-the-owner'",
        )
        .await
        .unwrap();
        db.execute_unprepared(
            "UPDATE module_release_locks SET expires_at = 0 WHERE lock_key = 'modules'",
        )
        .await
        .unwrap();
        drop(lease);
        let _reclaimed = second.acquire_release_lock().await.unwrap();
    }

    #[tokio::test]
    async fn identical_release_is_idempotent_and_keeps_one_version_row() {
        let source = tempdir().unwrap();
        write_versioned_package(source.path(), "1.0.0");
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db.clone());
        publisher
            .commit(
                publisher
                    .stage_with_operation_id(source.path(), "op-1")
                    .unwrap(),
            )
            .await
            .unwrap();
        publisher
            .commit(
                publisher
                    .stage_with_operation_id(source.path(), "op-1")
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(module_versions::Entity::find().count(&db).await.unwrap(), 1);
        assert_eq!(
            module_update_events::Entity::find()
                .count(&db)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn first_install_failure_is_audited_without_creating_module_identity() {
        let source = tempdir().unwrap();
        write_package(source.path());
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
        assert_eq!(publisher.cleanup_orphans().await.unwrap(), 1);
        assert!(!store.path().join("versions/module-1/1.0.0").exists());
    }

    #[tokio::test]
    async fn verified_package_commit_registers_identity_from_manifest() {
        let source = tempdir().unwrap();
        let digest = write_package(source.path());
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
            publisher.recover("module-1").await.unwrap().as_deref(),
            Some("1.0.0")
        );
    }

    #[tokio::test]
    async fn execution_context_rejects_database_integrity_that_does_not_match_package() {
        let source = tempdir().unwrap();
        write_package(source.path());
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
        db.execute_unprepared(&format!(
            "UPDATE modules SET integrity = 'v2:{}' WHERE id = 'module-1';",
            "a".repeat(64)
        ))
        .await
        .unwrap();
        assert!(publisher
            .execution_context(&dgos_types::HostSession::local_default(), "module-1")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn runtime_handle_is_publisher_issued_and_requires_loader_ready_transition() {
        let source = tempdir().unwrap();
        write_versioned_package(source.path(), "1.0.0");
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db);
        publisher
            .commit(publisher.stage(source.path()).unwrap())
            .await
            .unwrap();

        let handle = publisher
            .runtime_handle(&dgos_types::HostSession::local_default(), "module-1")
            .await
            .unwrap();
        assert_eq!(handle.state(), ModuleRuntimeState::Starting);
        assert!(handle.begin_draining().is_err());
        handle.mark_ready().unwrap();
        handle.begin_draining().unwrap();
        handle.stop().unwrap();
        assert_eq!(handle.state(), ModuleRuntimeState::Stopped);
        assert!(handle.fail().is_err());
    }

    /// Publish `module-1@1.0.0` with the given permissions and return the
    /// publisher, the store and the db.
    async fn published(
        permissions: &[&str],
    ) -> (
        ModulePublisher,
        tempfile::TempDir,
        dgos_db::sea_orm::DatabaseConnection,
    ) {
        let source = tempdir().unwrap();
        fs::write(source.path().join("entry.js"), "export default 1;").unwrap();
        let mut manifest = example_manifest("1.0.0");
        manifest["permissions"] = serde_json::json!(permissions);
        seal_manifest(source.path(), &manifest);
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db.clone());
        publisher
            .commit(publisher.stage(source.path()).unwrap())
            .await
            .unwrap();
        (publisher, store, db)
    }

    fn installed_manifest(store: &tempfile::TempDir) -> std::path::PathBuf {
        store.path().join("versions/module-1/1.0.0/manifest.json")
    }

    #[tokio::test]
    async fn widening_the_declared_permissions_in_an_installed_manifest_cannot_execute() {
        let (publisher, store, db) = published(&["providers.call"]).await;
        let session = dgos_types::HostSession::local_default();
        publisher
            .runtime_handle(&session, "module-1")
            .await
            .unwrap();

        // Only the manifest changes; every other file is byte-identical, which
        // the v1 digest could not see.
        let path = installed_manifest(&store);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["permissions"] = serde_json::json!(["providers.call", "providers.list"]);
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();

        assert!(matches!(
            publisher.runtime_handle(&session, "module-1").await,
            Err(ModulePackageError::Integrity)
        ));
        assert!(publisher
            .execution_context(&session, "module-1")
            .await
            .is_err());
        assert!(!publisher
            .validate_execution_identity(
                "module-1",
                "1.0.0",
                &modules::Entity::find_by_id("module-1")
                    .one(&db)
                    .await
                    .unwrap()
                    .unwrap()
                    .integrity,
            )
            .await
            .unwrap());
        // Startup recovery fails closed: the module is disabled, not run.
        assert!(publisher.recover_registered().await.unwrap().is_empty());
        assert!(
            !modules::Entity::find_by_id("module-1")
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .is_enabled
        );
    }

    #[tokio::test]
    async fn the_executed_bytes_are_the_verified_bytes_when_the_entry_changes_afterwards() {
        let (publisher, store, _db) = published(&[]).await;
        let session = dgos_types::HostSession::local_default();
        let handle = publisher
            .runtime_handle(&session, "module-1")
            .await
            .unwrap();
        assert_eq!(handle.artifact().entry_bytes(), b"export default 1;");

        // A change between verification and use does not reach the artifact;
        // it only makes the next verification fail.
        fs::write(
            store.path().join("versions/module-1/1.0.0/entry.js"),
            "export default 'evil';",
        )
        .unwrap();
        assert_eq!(handle.artifact().entry_bytes(), b"export default 1;");
        assert!(matches!(
            publisher.runtime_handle(&session, "module-1").await,
            Err(ModulePackageError::Integrity)
        ));
    }

    #[tokio::test]
    async fn a_legacy_integrity_value_is_never_executable() {
        let (publisher, _store, db) = published(&[]).await;
        db.execute_unprepared(&format!(
            "UPDATE modules SET integrity = '{0}' WHERE id = 'module-1'; \
             UPDATE module_versions SET integrity = '{0}' WHERE module_id = 'module-1'",
            "a".repeat(64)
        ))
        .await
        .unwrap();
        assert!(matches!(
            publisher
                .runtime_handle(&dgos_types::HostSession::local_default(), "module-1")
                .await,
            Err(ModulePackageError::Integrity)
        ));
    }

    #[tokio::test]
    async fn a_staged_copy_changed_before_commit_is_refused() {
        let source = tempdir().unwrap();
        write_package(source.path());
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db.clone());
        let staged = publisher.stage(source.path()).unwrap();
        fs::write(staged.staging_path.join("entry.js"), "export default 'x';").unwrap();
        assert!(publisher.commit(staged).await.is_err());
        assert!(modules::Entity::find_by_id("module-1")
            .one(&db)
            .await
            .unwrap()
            .is_none());
        assert!(!store.path().join("versions/module-1/1.0.0").exists());
    }

    #[tokio::test]
    async fn a_leftover_version_directory_from_an_interrupted_commit_is_redone() {
        let source = tempdir().unwrap();
        let integrity = write_package(source.path());
        for leftover in ["garbage", "same-content"] {
            let db = Database::connect("sqlite::memory:").await.unwrap();
            Migrator::up(&db, None).await.unwrap();
            let store = tempdir().unwrap();
            let publisher = ModulePublisher::new(store.path(), db.clone());
            // The directory was renamed into `versions/` but no row was ever
            // written: exactly what a crash between the two steps leaves.
            let version_dir = store.path().join("versions/module-1/1.0.0");
            if leftover == "same-content" {
                copy_dir(source.path(), &version_dir).unwrap();
            } else {
                fs::create_dir_all(&version_dir).unwrap();
                fs::write(version_dir.join("entry.js"), "leftover").unwrap();
            }
            publisher
                .commit(publisher.stage(source.path()).unwrap())
                .await
                .unwrap();
            let row = modules::Entity::find_by_id("module-1")
                .one(&db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.integrity, integrity, "{leftover}");
            assert_eq!(
                package_integrity(&version_dir).unwrap(),
                integrity,
                "{leftover}"
            );
        }
    }

    #[tokio::test]
    async fn a_registered_version_with_different_content_is_never_replaced() {
        let (publisher, store, _db) = published(&[]).await;
        let other = tempdir().unwrap();
        fs::write(other.path().join("entry.js"), "export default 2;").unwrap();
        seal_manifest(other.path(), &example_manifest("1.0.0"));
        let staged = publisher.stage(other.path()).unwrap();
        assert!(matches!(
            publisher.commit(staged).await,
            Err(ModulePackageError::Manifest(reason)) if reason.contains("different integrity")
        ));
        assert_eq!(
            fs::read(store.path().join("versions/module-1/1.0.0/entry.js")).unwrap(),
            b"export default 1;"
        );
    }

    #[tokio::test]
    async fn the_first_publish_state_is_written_in_the_same_transaction() {
        let source = tempdir().unwrap();
        write_package(source.path());
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db.clone());
        publisher
            .commit_with(
                publisher.stage(source.path()).unwrap(),
                FirstPublish {
                    official: true,
                    enabled: false,
                },
            )
            .await
            .unwrap();
        let row = modules::Entity::find_by_id("module-1")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(row.is_official && !row.is_enabled);

        // A later release changes neither flag.
        let next = tempdir().unwrap();
        write_versioned_package(next.path(), "1.1.0");
        publisher
            .commit_with(
                publisher.stage(next.path()).unwrap(),
                FirstPublish::default(),
            )
            .await
            .unwrap();
        let row = modules::Entity::find_by_id("module-1")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.current_version, "1.1.0");
        assert!(row.is_official && !row.is_enabled);
    }

    #[test]
    fn module_ids_cannot_escape_the_package_store() {
        assert!(!valid_module_id("../outside"));
        assert!(!valid_module_id("."));
        assert!(!valid_module_id(".."));
        assert!(!valid_module_id("nested/module"));
        assert!(valid_module_id("@dgos-workbench"));
    }

    #[test]
    fn entries_are_local_package_relative_paths_only() {
        for entry in ["entry.js", "dist/remote-entry.js", "assets/app_v2-min.mjs"] {
            assert!(valid_relative_entry(entry), "{entry}");
        }
        for entry in [
            "https://example.invalid/entry.js",
            "data:text/javascript,1",
            "file:///etc/passwd",
            "C:/entry.js",
            "/abs/entry.js",
            "../entry.js",
            "./entry.js",
            "dist\\entry.js",
            "%2e%2e/entry.js",
            "entry.js?x=1",
            "entry.js#x",
            "entry js",
            "",
        ] {
            assert!(!valid_relative_entry(entry), "{entry}");
        }
    }
}
