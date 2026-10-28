use crate::{
    content_package::{load_bundled_catalog, validate_package},
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
        let path = PathBuf::from(row.install_path);
        // Read from the validated immutable snapshot, never from an upstream
        // repository, legacy install_path or user-supplied resource path.
        let expected = crate::content_package::safe_child(
            &self.skills_dir,
            &format!("bundled/{id}/{version}"),
        )?;
        if path != expected {
            return Err(SkillError::InvalidManifest(
                "content outside snapshot root".into(),
            ));
        }
        let validated = validate_package(&path, &row.content_hash)?;
        let meta = validated.metadata;
        if meta.id != id || meta.version != version {
            return Err(SkillError::InvalidManifest(
                "snapshot identity mismatch".into(),
            ));
        }
        let body = validated.body;
        let license_text = validated.license_text;
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
