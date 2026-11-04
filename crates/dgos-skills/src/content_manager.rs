use crate::{
    content_hash,
    content_package::{load_bundled_catalog, safe_child, validate_package},
    ContentPreview, ContentSkillMetadata, Result, SkillDetail, SkillError, SkillUsageLog,
    SkillsManager,
};
use dgos_types::{BuiltinCaller, ExecutionProvenance};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Instant;

// Check each directory before descending so a legacy/local symlink cannot
// redirect snapshot publication to an unrelated location.
fn content_directory(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut current = root.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            std::path::Component::Normal(part) => current.push(part),
            _ => {
                return Err(SkillError::InvalidManifest(
                    "invalid snapshot directory".into(),
                ))
            }
        }
        match std::fs::create_dir(&current) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let metadata = std::fs::symlink_metadata(&current)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(SkillError::InvalidManifest(
                "unsafe snapshot directory".into(),
            ));
        }
    }
    Ok(current)
}

async fn secure_snapshot_child(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = safe_child(root, relative)?;
    let mut current = root.to_path_buf();
    for component in Path::new(relative).components() {
        let std::path::Component::Normal(part) = component else {
            return Err(SkillError::InvalidManifest("unsafe snapshot path".into()));
        };
        current.push(part);
        if tokio::fs::symlink_metadata(&current)
            .await
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(SkillError::InvalidManifest(
                "snapshot path contains a symlink".into(),
            ));
        }
    }
    Ok(path)
}

impl SkillsManager {
    pub async fn register_bundled_catalog(&self, root: &Path) -> Result<usize> {
        let root = root.to_path_buf();
        let mut packages = tokio::task::spawn_blocking(move || load_bundled_catalog(&root))
            .await
            .map_err(|e| SkillError::Internal(e.to_string()))??;
        tokio::fs::create_dir_all(&self.skills_dir).await?;
        if std::fs::symlink_metadata(&self.skills_dir)?
            .file_type()
            .is_symlink()
        {
            return Err(SkillError::InvalidManifest("symlink content root".into()));
        }
        for package in &mut packages {
            let source = PathBuf::from(&package.install_path);
            let parent = content_directory(
                &self.skills_dir,
                &format!("bundled/{}", package.metadata.id),
            )?;
            let dest = parent.join(&package.metadata.version);
            if dest.exists() {
                validate_package(&dest, &package.content_hash)?;
            } else {
                let staging = content_directory(&self.skills_dir, "staging")?
                    .join(format!("bundled-{}", uuid::Uuid::new_v4()));
                tokio::fs::create_dir(&staging).await?;
                let result: Result<()> = async {
                    for name in package.files.keys() {
                        let target = staging.join(name);
                        if let Some(parent) = target.parent() {
                            tokio::fs::create_dir_all(parent).await?;
                        }
                        tokio::fs::copy(source.join(name), &target).await?;
                    }
                    validate_package(&staging, &package.content_hash)?;
                    if tokio::fs::rename(&staging, &dest).await.is_err() {
                        // Another initializer may have published the same bytes.
                        validate_package(&dest, &package.content_hash)?;
                    }
                    Ok(())
                }
                .await;
                let _ = tokio::fs::remove_dir_all(&staging).await;
                result?;
            }
            package.install_path = dest.to_string_lossy().into_owned();
        }
        self.content_repo.register(&packages).await?;
        Ok(packages.len())
    }

    async fn read_content(
        &self,
        id: &str,
        version: &str,
    ) -> Result<(ContentSkillMetadata, ContentPreview)> {
        let row = self.content_repo.get_version(id, version).await?;
        let path = if row.is_bundled {
            PathBuf::from(row.install_path.clone())
        } else {
            safe_child(&self.skills_dir, &row.install_path)?
        };
        // Read from the validated immutable snapshot, never from an upstream
        // repository, legacy install_path or user-supplied resource path.
        if row.is_bundled {
            let expected = safe_child(&self.skills_dir, &format!("bundled/{id}/{version}"))?;
            if path != expected {
                return Err(SkillError::InvalidManifest(
                    "content outside snapshot root".into(),
                ));
            }
        }
        let (meta, body, license_text) = if row.is_bundled {
            let validated = validate_package(&path, &row.content_hash)?;
            (validated.metadata, validated.body, validated.license_text)
        } else {
            let root_metadata = tokio::fs::symlink_metadata(&path).await?;
            if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
                return Err(SkillError::InvalidManifest(
                    "unsafe external snapshot root".into(),
                ));
            }
            let mut metadata = row.metadata.clone();
            if let Some(object) = metadata.as_object_mut() {
                // Provenance and the file manifest belong to the immutable
                // version record, not to the strict content-package DTO.
                object.remove("provenance");
                object.remove("file_manifest");
            }
            let meta: ContentSkillMetadata = serde_json::from_value(metadata)?;
            let manifest = row.file_manifest.as_object().ok_or_else(|| {
                SkillError::InvalidManifest("external file manifest is invalid".into())
            })?;
            let mut files = std::collections::BTreeMap::new();
            for relative in manifest.keys() {
                let file = secure_snapshot_child(&path, relative).await?;
                let bytes = tokio::fs::read(file).await?;
                files.insert(relative.clone(), bytes);
            }
            let (actual_hash, _) = content_hash(&files);
            if actual_hash != row.content_hash {
                return Err(SkillError::InvalidManifest(
                    "external Skill snapshot hash mismatch".into(),
                ));
            }
            let body = files.get("SKILL.md").ok_or_else(|| {
                SkillError::InvalidManifest("external Skill is missing SKILL.md".into())
            })?;
            let body = String::from_utf8(body.clone()).map_err(|_| {
                SkillError::InvalidManifest("external SKILL.md is not UTF-8".into())
            })?;
            let license_text = files
                .get("LICENSE")
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .unwrap_or_default();
            (meta, body, license_text)
        };
        if meta.id != id || meta.version != version {
            return Err(SkillError::InvalidManifest(
                "snapshot identity mismatch".into(),
            ));
        }
        Ok((
            meta.clone(),
            ContentPreview {
                body,
                license_text,
                content_hash: row.content_hash,
                source_revision: meta.source.revision,
                verification: meta.verification,
                required_tools: meta.required_tools,
            },
        ))
    }

    /// Load the user's exact enabled immutable version after verifying its hash.
    /// The returned files are data only and must be published under host-owned
    /// application data, never into a project or CLI configuration directory.
    ///
    /// Missing user state/version returns `SkillNotFound`; disabled user state
    /// returns `SkillDisabled`. A changed installed version, hash mismatch or
    /// unsafe package returns `InvalidManifest`. Catalog updates never select a
    /// replacement: the host must ask the user to reselect an invalid pin.
    /// `body()` borrows SKILL.md; `into_files()` transfers every validated file
    /// to a publisher without copying. Compatibility describes content and tool
    /// requirements only; the host still owns CLI/tool execution authorization.
    pub async fn load_pinned_package(
        &self,
        user: &str,
        id: &str,
        version: &str,
        expected_hash: &str,
    ) -> Result<crate::PinnedSkillPackage> {
        self.load_pinned_package_inner(user, id, version, expected_hash)
            .await
            .map_err(|error| match error {
                SkillError::SkillNotFound(_) => SkillError::SkillNotFound("pinned version".into()),
                SkillError::SkillDisabled(_) => SkillError::SkillDisabled("pinned Skill".into()),
                SkillError::InvalidManifest(message)
                    if message == "pinned Skill version is no longer installed for this user" =>
                {
                    SkillError::InvalidManifest(message)
                }
                _ => SkillError::InvalidManifest("pinned Skill package verification failed".into()),
            })
    }

    async fn load_pinned_package_inner(
        &self,
        user: &str,
        id: &str,
        version: &str,
        expected_hash: &str,
    ) -> Result<crate::PinnedSkillPackage> {
        use crate::{SkillPackageCompatibility, SkillPackageFeatureSupport, SkillPackageFile, SkillPackageFileKind};

        let row = self.content_repo.enabled_version(id, user, version).await?;
        if row.content_hash != expected_hash {
            return Err(SkillError::InvalidManifest(
                "pinned Skill content hash mismatch".into(),
            ));
        }
        let root_metadata = tokio::fs::symlink_metadata(&self.skills_dir).await?;
        if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
            return Err(SkillError::InvalidManifest("unsafe Skill storage root".into()));
        }
        let path = if row.is_bundled {
            let path = PathBuf::from(&row.install_path);
            let expected = safe_child(&self.skills_dir, &format!("bundled/{id}/{version}"))?;
            if path != expected {
                return Err(SkillError::InvalidManifest(
                    "content outside snapshot root".into(),
                ));
            }
            path
        } else {
            let hash_prefix = expected_hash.get(..32).ok_or_else(|| {
                SkillError::InvalidManifest("invalid pinned Skill hash".into())
            })?;
            if row.install_path != format!("external/{id}/{hash_prefix}") {
                return Err(SkillError::InvalidManifest("content outside snapshot root".into()));
            }
            safe_child(&self.skills_dir, &row.install_path)?
        };

        let (files, compatible_apps, required_tools) = if row.is_bundled {
            let (meta, files, _) =
                crate::content_package::validate_package_files(&path, expected_hash)?;
            if meta.id != id || meta.version != version {
                return Err(SkillError::InvalidManifest("snapshot identity mismatch".into()));
            }
            (files, meta.compatible_apps, meta.required_tools)
        } else {
            // Scan the complete tree using bounded reads and link checks.
            let files = crate::content_package::read_package_files(&path)?;
            let manifest = row.file_manifest.as_object().ok_or_else(|| {
                SkillError::InvalidManifest("external file manifest is invalid".into())
            })?;
            if manifest.is_empty() {
                return Err(SkillError::InvalidManifest(
                    "external file manifest is invalid".into(),
                ));
            }
            let validated = crate::package_validation::validate_external_files(files)?;
            if validated.content_hash != expected_hash
                || validated.file_manifest.len() != manifest.len()
                || validated.file_manifest.iter().any(|(path, hash)| {
                    manifest.get(path).and_then(serde_json::Value::as_str) != Some(hash.as_str())
                })
            {
                return Err(SkillError::InvalidManifest(
                    "external Skill snapshot hash mismatch".into(),
                ));
            }
            if validated.skill_id != id || validated.version_label != version {
                return Err(SkillError::InvalidManifest(
                    "snapshot identity mismatch".into(),
                ));
            }
            let mut metadata = row.metadata.clone();
            if let Some(object) = metadata.as_object_mut() {
                object.remove("provenance");
                object.remove("file_manifest");
            }
            let meta: ContentSkillMetadata = serde_json::from_value(metadata)?;
            if meta.id != id || meta.version != version {
                return Err(SkillError::InvalidManifest("snapshot identity mismatch".into()));
            }
            let compatible_apps = meta.compatible_apps;
            let required_tools = meta.required_tools;
            (validated.files, compatible_apps, required_tools)
        };
        let body_index = files
            .keys()
            .position(|relative| relative == "SKILL.md")
            .ok_or_else(|| SkillError::InvalidManifest("missing SKILL.md".into()))?;
        let mut package_files = Vec::with_capacity(files.len());
        for (relative, bytes) in files {
            let kind = match relative.as_str() {
                "SKILL.md" => SkillPackageFileKind::Body,
                "skill.json" => SkillPackageFileKind::Manifest,
                "LICENSE" => SkillPackageFileKind::License,
                _ => SkillPackageFileKind::Resource,
            };
            package_files.push(SkillPackageFile {
                path: relative,
                kind,
                bytes,
            });
        }
        // Recheck after disk I/O so revocation during loading cannot publish a
        // newly loaded package using an earlier enabled-state observation.
        self.content_repo.enabled_version(id, user, version).await?;
        Ok(crate::PinnedSkillPackage {
            skill_id: id.into(),
            version: version.into(),
            content_hash: expected_hash.into(),
            compatibility: SkillPackageCompatibility {
                compatible_apps,
                required_tools,
                scripts: SkillPackageFeatureSupport::Unsupported,
                hooks: SkillPackageFeatureSupport::Unsupported,
                reason: "scripts and Hooks are not accepted by the installed package format and are never executed",
            },
            files: package_files,
            body_index,
        })
    }

    pub(crate) async fn content_detail(
        &self,
        row: dgos_db::entities::skill_catalog::Model,
    ) -> Result<SkillDetail> {
        let (meta, content) = self
            .read_content(&row.skill_id, &row.latest_version)
            .await?;
        Ok(SkillDetail {
            skill_id: meta.id.clone(),
            name: meta.name.clone(),
            description: Some(meta.description.clone()),
            latest_version: meta.version.clone(),
            author: Some(meta.author.name.clone()),
            category: meta.category.clone(),
            tags: meta.tags.clone(),
            manifest: meta.compatibility_manifest(),
            download_url: String::new(),
            identity: meta.identity,
            source_kind: meta.source.kind,
            license_spdx: meta.license.spdx,
            compatible_apps: meta.compatible_apps,
            format: "skill_md".into(),
            entry: Some(meta.entry),
            content: Some(content),
        })
    }

    /// Context preparation is not model execution or tool authorization.
    pub async fn prepare_content_skill(
        &self,
        id: &str,
        user: &str,
    ) -> Result<crate::PreparedContent> {
        self.prepare_content_skill_with_provenance(id, user, None)
            .await
    }

    /// Prepare immutable content and optionally record the trusted host source.
    /// Audit persistence is best-effort: a local content request must not be
    /// turned into a retryable task merely because its audit row was unavailable.
    pub async fn prepare_content_skill_with_provenance(
        &self,
        id: &str,
        user: &str,
        provenance: Option<&ExecutionProvenance>,
    ) -> Result<crate::PreparedContent> {
        let started = Instant::now();
        let result = self.prepare_content_skill_inner(id, user).await;
        if let Some(provenance) = provenance {
            let (success, output_result, error_message) = match &result {
                Ok(content) => (
                    true,
                    Some(json!({
                        "version": content.version,
                        "content_hash": content.content_hash,
                    })),
                    None,
                ),
                Err(error) => (false, None, Some(error.error_code().to_string())),
            };
            let task_ref = (provenance.caller == BuiltinCaller::Workbench)
                .then(|| provenance.request_id.clone());
            let log = SkillUsageLog {
                id: uuid::Uuid::new_v4().to_string(),
                skill_id: id.to_string(),
                user_id: user.to_string(),
                caller: Some(format!("{:?}", provenance.caller).to_lowercase()),
                request_id: Some(provenance.request_id.clone()),
                task_ref,
                input_params: json!({"operation": "prepare_content_skill"}),
                output_result,
                success,
                error_message,
                latency_ms: started.elapsed().as_millis().min(i32::MAX as u128) as i32,
                created_at: chrono::Utc::now().timestamp(),
            };
            if let Err(audit_error) = self.skill_repo.insert_usage_log(&log).await {
                tracing::warn!(skill_id = id, error = %audit_error, "Skill preparation audit write failed");
            }
        }
        result
    }

    /// Non-secret installed version metadata for project capability selection.
    pub async fn installed_content_pins(&self, user: &str) -> Result<Vec<(crate::Skill, String)>> {
        let mut pins = Vec::new();
        for skill in self.content_repo.user_skills(user).await? {
            let version = self.content_repo.get_version(&skill.skill_id, &skill.version).await?;
            pins.push((skill, version.content_hash));
        }
        Ok(pins)
    }
    async fn prepare_content_skill_inner(
        &self,
        id: &str,
        user: &str,
    ) -> Result<crate::PreparedContent> {
        let skill = self
            .content_repo
            .user_skills(user)
            .await?
            .into_iter()
            .find(|skill| skill.skill_id == id)
            .ok_or_else(|| SkillError::SkillNotFound(id.into()))?;
        if !skill.enabled {
            return Err(SkillError::SkillDisabled(id.into()));
        }
        let (meta, content) = self.read_content(id, &skill.version).await?;
        if !meta.compatible_apps.iter().any(|app| app == "ai-workbench")
            || !meta.required_tools.is_empty()
        {
            return Err(SkillError::DependencyNotSatisfied(
                "workbench compatibility or tools unavailable".into(),
            ));
        }
        Ok(crate::PreparedContent {
            skill_id: id.into(),
            version: skill.version,
            content_hash: content.content_hash,
            body: content.body,
        })
    }
}
