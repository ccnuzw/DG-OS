//! Batch B installation coordinator.
//!
//! The coordinator deliberately separates source acquisition and package
//! validation from publication.  A validated package is first written below
//! an application-owned staging directory; only an explicit confirmation can
//! rename that directory into the immutable content tree and register the
//! corresponding database rows.

use crate::{
    content_hash, GithubAdapter, LicenseStatus, SkillError, SkillSource, ValidatedExternalPackage,
    MAX_EXTERNAL_ARCHIVE_BYTES,
};
use dgos_db::{
    entities::{skill_catalog, skill_user_states, skill_versions},
    repositories::{OperationPatch, SkillInstallOperation, SkillInstallRepository},
    sea_orm::{
        sea_query::OnConflict, ActiveModelTrait, DatabaseConnection, EntityTrait, Set,
        TransactionTrait,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, OwnedMutexGuard};

const MAX_OPERATION_AGE_SECS: i64 = 30 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInstallPreview {
    pub operation_id: String,
    pub skill_id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub content_hash: String,
    pub source_kind: String,
    pub source_ref: String,
    pub resolved_commit: Option<String>,
    pub license_status: LicenseStatus,
    pub file_count: usize,
    pub required_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkill {
    pub operation_id: String,
    pub skill_id: String,
    pub version_id: String,
    pub version: String,
    pub content_hash: String,
}

#[derive(Clone)]
pub struct InstallCoordinator {
    db: DatabaseConnection,
    skills_dir: PathBuf,
    operations: SkillInstallRepository,
    source_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl InstallCoordinator {
    pub fn new(db: DatabaseConnection, skills_dir: PathBuf) -> Self {
        Self {
            operations: SkillInstallRepository::new(db.clone()),
            db,
            skills_dir,
            source_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Preview a source and create a resumable operation.  No package body or
    /// absolute path is written to SQLite.
    pub async fn preview(
        &self,
        user_id: &str,
        source: SkillSource,
    ) -> crate::Result<SkillInstallPreview> {
        let (package, source_kind, source_ref, resolved_commit) = match source {
            SkillSource::LocalZip { path } => {
                let metadata = tokio::fs::symlink_metadata(&path).await?;
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || metadata.len() > MAX_EXTERNAL_ARCHIVE_BYTES as u64
                {
                    return Err(SkillError::InvalidManifest(
                        "local Skill archive exceeds 8 MiB or is not a file".into(),
                    ));
                }
                let bytes = tokio::fs::read(&path).await?;
                let package = crate::validate_external_zip(&bytes)?;
                let source_ref = format!("content:{}", package.content_hash);
                (package, "local_zip".into(), source_ref, None)
            }
            SkillSource::Github(source) => {
                let adapter = GithubAdapter::new()?;
                let (resolved, package) = adapter.fetch_and_validate(source).await?;
                (
                    package,
                    "github".into(),
                    resolved.source.normalized_source,
                    Some(resolved.resolved_commit),
                )
            }
        };
        self.preview_package(user_id, package, source_kind, source_ref, resolved_commit)
            .await
    }

    pub async fn preview_package(
        &self,
        user_id: &str,
        package: ValidatedExternalPackage,
        source_kind: String,
        source_ref: String,
        resolved_commit: Option<String>,
    ) -> crate::Result<SkillInstallPreview> {
        let idempotency_key =
            idempotency_key(user_id, &source_kind, &source_ref, &package.content_hash);
        if let Some(existing) = self
            .operations
            .find_by_idempotency(user_id, &idempotency_key)
            .await?
        {
            if matches!(
                existing.state.as_str(),
                "previewed" | "publishing" | "committed"
            ) {
                return preview_from_operation(existing);
            }
        }

        let operation_id = uuid::Uuid::new_v4().to_string();
        let stage_relpath = format!("staging/{operation_id}");
        let stage = self.safe_path(&stage_relpath)?;
        write_staging(&stage, &package).await?;

        let metadata = package_metadata(
            &package,
            &source_kind,
            &source_ref,
            resolved_commit.as_deref(),
        )?;
        let now = chrono::Utc::now().timestamp();
        let operation = SkillInstallOperation {
            operation_id: operation_id.clone(),
            user_id: user_id.to_owned(),
            idempotency_key,
            source_kind,
            source_ref,
            state: "previewed".into(),
            skill_id: Some(package.skill_id.clone()),
            version_id: Some(version_id(&package.skill_id, &package.version_label)),
            content_hash: Some(package.content_hash.clone()),
            staging_relpath: Some(stage_relpath),
            published_relpath: Some(published_relpath(&package.skill_id, &package.content_hash)),
            metadata,
            last_error_key: None,
            created_at: now,
            updated_at: now,
        };
        if let Err(error) = self.operations.insert(operation.clone()).await {
            let _ = tokio::fs::remove_dir_all(&stage).await;
            if let Some(existing) = self
                .operations
                .find_by_idempotency(&operation.user_id, &operation.idempotency_key)
                .await?
            {
                return preview_from_operation(existing);
            }
            return Err(SkillError::DatabaseError(error.to_string()));
        }
        preview_from_operation(operation)
    }

    /// Publish a previously previewed operation.  The user id is always an
    /// argument supplied by the host, never by an untrusted page.
    pub async fn confirm(
        &self,
        user_id: &str,
        operation_id: &str,
    ) -> crate::Result<InstalledSkill> {
        let operation = self
            .operations
            .find_for_user(user_id, operation_id)
            .await?
            .ok_or_else(|| SkillError::SkillNotFound(operation_id.to_owned()))?;
        if operation.state == "committed" {
            return installed_from_operation(&operation);
        }
        if !matches!(operation.state.as_str(), "previewed" | "publishing") {
            return Err(SkillError::InvalidManifest(
                "Skill install operation is not confirmable".into(),
            ));
        }
        if operation.updated_at + MAX_OPERATION_AGE_SECS < chrono::Utc::now().timestamp() {
            let _ = self
                .operations
                .update(
                    user_id,
                    operation_id,
                    OperationPatch {
                        state: Some("abandoned".into()),
                        last_error_key: Some(Some("preview_expired".into())),
                        ..Default::default()
                    },
                )
                .await;
            return Err(SkillError::InvalidManifest("Skill preview expired".into()));
        }
        let skill_id = operation
            .skill_id
            .clone()
            .ok_or_else(|| SkillError::InvalidManifest("preview has no Skill identity".into()))?;
        let content_hash = operation
            .content_hash
            .clone()
            .ok_or_else(|| SkillError::InvalidManifest("preview has no content hash".into()))?;
        let _lock = self.lock_for(&format!("{skill_id}:{content_hash}")).await;
        let stage_relpath = operation
            .staging_relpath
            .clone()
            .ok_or_else(|| SkillError::InvalidManifest("preview has no staging path".into()))?;
        let stage = self.safe_path(&stage_relpath)?;
        verify_staging(&stage, &content_hash).await?;
        let published_relpath = operation
            .published_relpath
            .clone()
            .ok_or_else(|| SkillError::InvalidManifest("preview has no publication path".into()))?;
        let target = self.safe_path(&published_relpath)?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        self.operations
            .update(
                user_id,
                operation_id,
                OperationPatch {
                    state: Some("publishing".into()),
                    ..Default::default()
                },
            )
            .await?;

        if tokio::fs::symlink_metadata(&target).await.is_err() {
            tokio::fs::rename(&stage, &target).await?;
        } else {
            verify_staging(&target, &content_hash).await?;
            let _ = tokio::fs::remove_dir_all(&stage).await;
        }
        let installed = self.register_published(user_id, &operation).await?;
        self.operations
            .update(
                user_id,
                operation_id,
                OperationPatch {
                    state: Some("committed".into()),
                    ..Default::default()
                },
            )
            .await?;
        Ok(installed)
    }

    pub async fn cancel(&self, user_id: &str, operation_id: &str) -> crate::Result<()> {
        let operation = self
            .operations
            .find_for_user(user_id, operation_id)
            .await?
            .ok_or_else(|| SkillError::SkillNotFound(operation_id.to_owned()))?;
        if operation.state == "committed" {
            return Err(SkillError::InvalidManifest(
                "committed Skill cannot be cancelled".into(),
            ));
        }
        if let Some(path) = operation.staging_relpath.as_deref() {
            let _ = tokio::fs::remove_dir_all(self.safe_path(path)?).await;
        }
        self.operations
            .update(
                user_id,
                operation_id,
                OperationPatch {
                    state: Some("abandoned".into()),
                    last_error_key: Some(Some("cancelled".into())),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    /// Finish or abandon interrupted operations after a process restart.
    pub async fn recover_operations(&self) -> crate::Result<usize> {
        let mut recovered = 0;
        let rows = self.operations.list_all_active().await?;
        for operation in rows {
            let Some(hash) = operation.content_hash.as_deref() else {
                continue;
            };
            let target = operation
                .published_relpath
                .as_deref()
                .map(|path| self.safe_path(path))
                .transpose()?;
            if let Some(target) = target {
                if verify_staging(&target, hash).await.is_ok() {
                    self.register_published(&operation.user_id, &operation)
                        .await?;
                    self.operations
                        .update(
                            &operation.user_id,
                            &operation.operation_id,
                            OperationPatch {
                                state: Some("committed".into()),
                                ..Default::default()
                            },
                        )
                        .await?;
                    recovered += 1;
                    continue;
                }
            }
            if operation.updated_at + MAX_OPERATION_AGE_SECS < chrono::Utc::now().timestamp() {
                if let Some(path) = operation.staging_relpath.as_deref() {
                    let _ = tokio::fs::remove_dir_all(self.safe_path(path)?).await;
                }
                self.operations
                    .update(
                        &operation.user_id,
                        &operation.operation_id,
                        OperationPatch {
                            state: Some("abandoned".into()),
                            last_error_key: Some(Some("recovery_expired".into())),
                            ..Default::default()
                        },
                    )
                    .await?;
            }
        }
        Ok(recovered)
    }

    fn safe_path(&self, relative: &str) -> crate::Result<PathBuf> {
        let path = Path::new(relative);
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(SkillError::InvalidManifest(
                "unsafe Skill storage path".into(),
            ));
        }
        Ok(self.skills_dir.join(path))
    }

    async fn lock_for(&self, key: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.source_locks.lock().await;
            locks
                .entry(key.to_owned())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }

    async fn register_published(
        &self,
        user_id: &str,
        operation: &SkillInstallOperation,
    ) -> crate::Result<InstalledSkill> {
        let skill_id = operation
            .skill_id
            .as_deref()
            .ok_or_else(|| SkillError::InvalidManifest("operation has no Skill identity".into()))?;
        let content_hash = operation
            .content_hash
            .as_deref()
            .ok_or_else(|| SkillError::InvalidManifest("operation has no content hash".into()))?;
        let version_id = operation.version_id.as_deref().ok_or_else(|| {
            SkillError::InvalidManifest("operation has no version identity".into())
        })?;
        let metadata = operation.metadata.clone();
        let version = metadata
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| SkillError::InvalidManifest("operation metadata has no version".into()))?
            .to_owned();
        let name = metadata
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(skill_id)
            .to_owned();
        let description = metadata
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let category = metadata
            .get("category")
            .and_then(Value::as_str)
            .unwrap_or("imported")
            .to_owned();
        let tags = metadata.get("tags").cloned().unwrap_or_else(|| json!([]));
        let compatible_apps = metadata
            .get("compatible_apps")
            .cloned()
            .unwrap_or_else(|| json!(["ai-workbench"]));
        let author = metadata
            .get("author")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let license_spdx = metadata
            .get("license")
            .and_then(|v| v.get("spdx").or(Some(v)))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let source_kind = operation.source_kind.clone();
        let source_ref = Some(operation.source_ref.clone());
        let verification_status = "unverified".to_owned();
        let now = chrono::Utc::now().timestamp();
        let tx = self.db.begin().await.map_err(db_error)?;
        if let Some(existing) = skill_versions::Entity::find_by_id(version_id)
            .one(&tx)
            .await
            .map_err(db_error)?
        {
            if existing.content_hash != content_hash {
                return Err(SkillError::InvalidManifest(
                    "published version content cannot change".into(),
                ));
            }
        } else {
            skill_versions::ActiveModel {
                id: Set(version_id.to_owned()),
                skill_id: Set(skill_id.to_owned()),
                version: Set(version.to_owned()),
                content_hash: Set(content_hash.to_owned()),
                install_path: Set(operation.published_relpath.clone().unwrap_or_default()),
                is_bundled: Set(false),
                verification_status: Set(verification_status.clone()),
                file_manifest: Set(metadata
                    .get("file_manifest")
                    .cloned()
                    .unwrap_or_else(|| json!({}))),
                installed_at: Set(now),
                metadata: Set(metadata.clone()),
            }
            .insert(&tx)
            .await
            .map_err(db_error)?;
        }
        skill_catalog::Entity::insert(skill_catalog::ActiveModel {
            skill_id: Set(skill_id.to_owned()),
            identity: Set(metadata
                .get("identity")
                .and_then(Value::as_str)
                .unwrap_or(skill_id)
                .to_owned()),
            name: Set(name),
            latest_version: Set(version.clone()),
            description: Set(description),
            category: Set(category),
            tags: Set(tags),
            compatible_apps: Set(compatible_apps),
            author: Set(author),
            license_spdx: Set(license_spdx),
            source_kind: Set(source_kind),
            source_ref: Set(source_ref),
            verification_status: Set(verification_status),
            metadata: Set(metadata),
            synced_at: Set(now),
        })
        .on_conflict(
            OnConflict::column(skill_catalog::Column::SkillId)
                .update_columns([
                    skill_catalog::Column::Identity,
                    skill_catalog::Column::Name,
                    skill_catalog::Column::LatestVersion,
                    skill_catalog::Column::Description,
                    skill_catalog::Column::Category,
                    skill_catalog::Column::Tags,
                    skill_catalog::Column::CompatibleApps,
                    skill_catalog::Column::Author,
                    skill_catalog::Column::LicenseSpdx,
                    skill_catalog::Column::SourceKind,
                    skill_catalog::Column::SourceRef,
                    skill_catalog::Column::VerificationStatus,
                    skill_catalog::Column::Metadata,
                    skill_catalog::Column::SyncedAt,
                ])
                .to_owned(),
        )
        .exec(&tx)
        .await
        .map_err(db_error)?;
        skill_user_states::Entity::insert(skill_user_states::ActiveModel {
            user_id: Set(user_id.to_owned()),
            skill_id: Set(skill_id.to_owned()),
            version_id: Set(version_id.to_owned()),
            enabled: Set(true),
            created_at: Set(now),
        })
        .on_conflict(
            OnConflict::columns([
                skill_user_states::Column::UserId,
                skill_user_states::Column::SkillId,
            ])
            .update_column(skill_user_states::Column::VersionId)
            .to_owned(),
        )
        .exec(&tx)
        .await
        .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(InstalledSkill {
            operation_id: operation.operation_id.clone(),
            skill_id: skill_id.to_owned(),
            version_id: version_id.to_owned(),
            version,
            content_hash: content_hash.to_owned(),
        })
    }
}

fn db_error(error: dgos_db::sea_orm::DbErr) -> SkillError {
    SkillError::DatabaseError(error.to_string())
}

fn idempotency_key(
    user_id: &str,
    source_kind: &str,
    source_ref: &str,
    content_hash: &str,
) -> String {
    let mut hash = Sha256::new();
    hash.update(user_id.as_bytes());
    hash.update([0]);
    hash.update(source_kind.as_bytes());
    hash.update([0]);
    hash.update(source_ref.as_bytes());
    hash.update([0]);
    hash.update(content_hash.as_bytes());
    format!("install:{:x}", hash.finalize())
}

fn version_id(skill_id: &str, version: &str) -> String {
    format!("{skill_id}@{version}")
}

fn published_relpath(skill_id: &str, content_hash: &str) -> String {
    format!("external/{skill_id}/{}", &content_hash[..32])
}

fn package_metadata(
    package: &ValidatedExternalPackage,
    source_kind: &str,
    source_ref: &str,
    resolved_commit: Option<&str>,
) -> crate::Result<Value> {
    let author = package
        .metadata
        .author
        .clone()
        .unwrap_or_else(|| "External author".into());
    let version = package.version_label.clone();
    let metadata = json!({
        "schema_version": "1.0",
        "id": package.skill_id.clone(),
        "slug": package.skill_id.clone(),
        "name": package.metadata.name.clone(),
        "description": package.metadata.description.clone(),
        "version": version,
        "identity": package.metadata.identity.clone(),
        "category": package.metadata.category.clone().unwrap_or_else(|| "imported".into()),
        "tags": package.metadata.tags.clone(),
        "entry": "SKILL.md",
        "locales": [],
        "compatible_apps": ["ai-workbench"],
        "required_tools": [],
        "author": {"name": author, "type": "external"},
        "license": {"spdx": package.metadata.license_spdx.clone(), "redistributable": false},
        "source": {"kind": source_kind, "path": source_ref, "revision": resolved_commit},
        "verification": {"status": "unverified", "scope": ["package_validation"]},
        "provenance": {
            "source_kind": source_kind,
            "normalized_source": source_ref,
            "requested_ref": source_ref,
            "resolved_commit": resolved_commit,
            "license_status": package.license_status.clone(),
            "verification_scope": ["package_validation"],
            "file_manifest_version": 1,
            "version_label_kind": if package.metadata.version.is_some() { "declared" } else { "content_revision" }
        },
        "file_manifest": package.file_manifest.clone(),
    });
    // Ensure JSON serialization itself cannot smuggle package bytes into the
    // operation record; only the generated metadata and hash are persisted.
    if serde_json::to_vec(&metadata)
        .map_err(SkillError::JsonError)?
        .len()
        > 256 * 1024
    {
        return Err(SkillError::InvalidManifest(
            "Skill metadata is too large".into(),
        ));
    }
    Ok(metadata)
}

async fn write_staging(root: &Path, package: &ValidatedExternalPackage) -> crate::Result<()> {
    if let Some(parent) = root.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::create_dir(root).await?;
    for (relative, bytes) in &package.files {
        let target = safe_child(root, relative)?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(target, bytes).await?;
    }
    Ok(())
}

async fn verify_staging(root: &Path, expected_hash: &str) -> crate::Result<()> {
    let mut files = std::collections::BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let metadata = tokio::fs::symlink_metadata(entry.path()).await?;
            if metadata.file_type().is_symlink() {
                return Err(SkillError::InvalidManifest(
                    "published Skill contains a symlink".into(),
                ));
            }
            if metadata.is_dir() {
                stack.push(entry.path());
            } else {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|_| SkillError::InvalidManifest("invalid published path".into()))?
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/");
                files.insert(relative, tokio::fs::read(entry.path()).await?);
            }
        }
    }
    let (actual, _) = content_hash(&files);
    if actual != expected_hash {
        return Err(SkillError::InvalidManifest(
            "published Skill hash mismatch".into(),
        ));
    }
    Ok(())
}

fn safe_child(root: &Path, relative: &str) -> crate::Result<PathBuf> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(SkillError::InvalidManifest("unsafe package path".into()));
    }
    Ok(root.join(relative))
}

fn preview_from_operation(operation: SkillInstallOperation) -> crate::Result<SkillInstallPreview> {
    let metadata = &operation.metadata;
    let package = metadata;
    let license_status = package
        .get("provenance")
        .and_then(|v| v.get("license_status"))
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(SkillError::JsonError)?
        .unwrap_or(LicenseStatus::Unknown);
    Ok(SkillInstallPreview {
        operation_id: operation.operation_id,
        skill_id: operation.skill_id.unwrap_or_default(),
        name: package
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        description: package
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        version: package
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        content_hash: operation.content_hash.unwrap_or_default(),
        source_kind: operation.source_kind,
        source_ref: operation.source_ref,
        resolved_commit: package
            .get("source")
            .and_then(|v| v.get("revision"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        license_status,
        file_count: package
            .get("file_manifest")
            .and_then(Value::as_object)
            .map(|v| v.len())
            .unwrap_or_default(),
        required_confirmation: true,
    })
}

fn installed_from_operation(operation: &SkillInstallOperation) -> crate::Result<InstalledSkill> {
    Ok(InstalledSkill {
        operation_id: operation.operation_id.clone(),
        skill_id: operation
            .skill_id
            .clone()
            .ok_or_else(|| SkillError::InvalidManifest("operation has no Skill identity".into()))?,
        version_id: operation.version_id.clone().ok_or_else(|| {
            SkillError::InvalidManifest("operation has no version identity".into())
        })?,
        version: operation
            .metadata
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        content_hash: operation.content_hash.clone().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

    fn fixture() -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(&mut output);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file("SKILL.md", options).unwrap();
        writer
            .write_all(b"---\nname: Demo\ndescription: Fixture\n---\n")
            .unwrap();
        writer.start_file("references/readme.txt", options).unwrap();
        writer.write_all(b"reference").unwrap();
        writer.finish().unwrap();
        output.into_inner()
    }

    #[tokio::test]
    async fn staging_round_trip_preserves_the_content_hash() {
        let package = crate::validate_external_zip(&fixture()).unwrap();
        let root = tempfile::tempdir().unwrap();
        let staging = root.path().join("staging/op");
        write_staging(&staging, &package).await.unwrap();
        verify_staging(&staging, &package.content_hash)
            .await
            .unwrap();
        assert!(staging.join("SKILL.md").is_file());
        assert!(staging.join("references/readme.txt").is_file());
    }

    #[test]
    fn storage_paths_reject_absolute_and_parent_components() {
        let error = safe_child(Path::new("/tmp/root"), "../escape").unwrap_err();
        assert!(matches!(error, SkillError::InvalidManifest(_)));
        let error = safe_child(Path::new("/tmp/root"), "/absolute").unwrap_err();
        assert!(matches!(error, SkillError::InvalidManifest(_)));
    }
}
