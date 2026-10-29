//! Enable, disable, uninstall and listing for published modules (D055).
//!
//! These change what the host will run, never what a module may do: grants
//! stay with the permission repository, and nothing here starts a runtime.
//! Callers stop the runtime first (`ModuleOperationManager` does).

use crate::{db_error, ModulePackageError, ModulePublisher};
use dgos_db::{
    entities::{module_permissions, module_versions, modules},
    sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait},
};
use std::fs;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManagementError {
    #[error("module is not published")]
    NotPublished,
    /// The module ships with the host: it can be disabled, not removed.
    #[error("bundled module cannot be uninstalled")]
    Official,
    #[error(transparent)]
    Package(#[from] ModulePackageError),
}

/// One published module as the management page needs it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedModule {
    pub module_id: String,
    pub name: String,
    pub version: String,
    pub integrity: String,
    pub official: bool,
    pub enabled: bool,
    /// Permission strings the manifest declares.
    pub declared: Vec<String>,
    /// Declared permissions the given user currently has granted.
    pub granted: Vec<String>,
}

impl ModulePublisher {
    /// Published modules, with which declared permissions `user_id` granted.
    pub async fn published_modules(
        &self,
        user_id: &str,
    ) -> Result<Vec<PublishedModule>, ManagementError> {
        let rows = modules::Entity::find()
            .all(&self.db)
            .await
            .map_err(db_error)?;
        let grants = module_permissions::Entity::find()
            .filter(module_permissions::Column::UserId.eq(user_id))
            .filter(module_permissions::Column::Granted.eq(true))
            .all(&self.db)
            .await
            .map_err(db_error)?;
        let mut published: Vec<_> = rows
            .into_iter()
            .map(|row| PublishedModule {
                declared: row
                    .manifest
                    .get("permissions")
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                granted: grants
                    .iter()
                    .filter(|grant| grant.module_id == row.id)
                    .map(|grant| grant.permission.clone())
                    .collect(),
                module_id: row.id,
                name: row.display_name,
                version: row.current_version,
                integrity: row.integrity,
                official: row.is_official,
                enabled: row.is_enabled,
            })
            .collect();
        published.sort_by(|a, b| a.module_id.cmp(&b.module_id));
        Ok(published)
    }

    /// Flip the enabled flag. Serialized with publishing, so it cannot
    /// interleave with a pointer switch.
    pub async fn set_enabled(&self, module_id: &str, enabled: bool) -> Result<(), ManagementError> {
        let _guard = self.publish_lock.lock().await;
        let row = modules::Entity::find_by_id(module_id)
            .one(&self.db)
            .await
            .map_err(db_error)?
            .ok_or(ManagementError::NotPublished)?;
        if row.is_enabled != enabled {
            let mut model: modules::ActiveModel = row.into();
            model.is_enabled = Set(enabled);
            model.updated_at = Set(chrono::Utc::now().timestamp());
            model.update(&self.db).await.map_err(db_error)?;
        }
        Ok(())
    }

    /// Remove a module: grants, version rows, the module row and its files.
    /// Update history and permission audits stay as the audit trail. A module
    /// that ships with the host is refused.
    pub async fn uninstall(&self, module_id: &str) -> Result<(), ManagementError> {
        let _guard = self.publish_lock.lock().await;
        let row = modules::Entity::find_by_id(module_id)
            .one(&self.db)
            .await
            .map_err(db_error)?
            .ok_or(ManagementError::NotPublished)?;
        if row.is_official {
            return Err(ManagementError::Official);
        }
        let tx = self.db.begin().await.map_err(db_error)?;
        module_permissions::Entity::delete_many()
            .filter(module_permissions::Column::ModuleId.eq(module_id))
            .exec(&tx)
            .await
            .map_err(db_error)?;
        module_versions::Entity::delete_many()
            .filter(module_versions::Column::ModuleId.eq(module_id))
            .exec(&tx)
            .await
            .map_err(db_error)?;
        modules::Entity::delete_by_id(module_id)
            .exec(&tx)
            .await
            .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        // Rows are gone, so the files are unreachable even if removal fails;
        // startup recovery cleans orphans.
        for dir in ["current", "versions"] {
            let _ = fs::remove_dir_all(self.root.join(dir).join(module_id));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_integrity;
    use dgos_db::{migrations::Migrator, sea_orm::Database};
    use sea_orm_migration::MigratorTrait;
    use tempfile::{tempdir, TempDir};

    async fn published(official: bool) -> (TempDir, ModulePublisher) {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db.clone());
        let source = tempdir().unwrap();
        fs::write(source.path().join("entry.js"), "export default 1;").unwrap();
        let write_manifest = |sha256: &str| {
            fs::write(
                source.path().join("manifest.json"),
                serde_json::json!({
                    "id": "mod-a", "name": "Mod A", "version": "1.0.0", "entry": "entry.js",
                    "artifact": {"version": "1.0.0", "sha256": sha256},
                    "permissions": ["providers.call"]
                })
                .to_string(),
            )
            .unwrap();
        };
        write_manifest(&"0".repeat(64));
        let integrity = package_integrity(source.path()).unwrap();
        write_manifest(integrity.strip_prefix("v2:").unwrap());
        let staged = publisher.stage(source.path()).unwrap();
        publisher.commit(staged).await.unwrap();
        if official {
            let row = modules::Entity::find_by_id("mod-a")
                .one(&db)
                .await
                .unwrap()
                .unwrap();
            let mut model: modules::ActiveModel = row.into();
            model.is_official = Set(true);
            model.update(&db).await.unwrap();
        }
        (store, publisher)
    }

    #[tokio::test]
    async fn enable_and_disable_change_only_the_flag_and_are_idempotent() {
        let (_store, publisher) = published(false).await;
        let list = publisher.published_modules("default-user").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].declared, ["providers.call"]);
        assert!(list[0].enabled);

        publisher.set_enabled("mod-a", false).await.unwrap();
        publisher.set_enabled("mod-a", false).await.unwrap();
        assert!(!publisher.published_modules("default-user").await.unwrap()[0].enabled);
        assert!(publisher.release_snapshot("mod-a").await.unwrap().is_none());
        publisher.set_enabled("mod-a", true).await.unwrap();
        assert!(publisher.release_snapshot("mod-a").await.unwrap().is_some());
        assert!(matches!(
            publisher.set_enabled("nope", true).await,
            Err(ManagementError::NotPublished)
        ));
    }

    #[tokio::test]
    async fn uninstall_removes_rows_grants_and_files_but_keeps_the_audit_trail() {
        let (store, publisher) = published(false).await;
        publisher
            .permissions
            .grant(
                "default-user",
                "mod-a",
                dgos_db::ModulePermission::ProvidersCall,
            )
            .await
            .unwrap();
        assert!(store.path().join("versions/mod-a").exists());

        publisher.uninstall("mod-a").await.unwrap();
        assert!(publisher
            .published_modules("default-user")
            .await
            .unwrap()
            .is_empty());
        assert!(!store.path().join("versions/mod-a").exists());
        assert!(!store.path().join("current/mod-a").exists());
        let grants = module_permissions::Entity::find()
            .all(&publisher.db)
            .await
            .unwrap();
        assert!(grants.is_empty());
        let events = dgos_db::entities::module_update_events::Entity::find()
            .all(&publisher.db)
            .await
            .unwrap();
        assert!(!events.is_empty(), "the release history is kept");
        assert!(matches!(
            publisher.uninstall("mod-a").await,
            Err(ManagementError::NotPublished)
        ));
    }

    #[tokio::test]
    async fn a_bundled_module_cannot_be_uninstalled() {
        let (store, publisher) = published(true).await;
        assert!(matches!(
            publisher.uninstall("mod-a").await,
            Err(ManagementError::Official)
        ));
        assert_eq!(
            publisher
                .published_modules("default-user")
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(store.path().join("versions/mod-a").exists());
    }
}
