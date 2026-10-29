//! One-time migration of v1 (bare SHA-256) integrity values to v2 (D057).
//!
//! A legacy row is migrated only when the files on disk still hash to the
//! recorded v1 value *and* the manifest on disk equals the manifest the row
//! recorded at publish time. The v1 digest never covered the manifest, so the
//! second check is what brings the manifest under the new digest. Package
//! files are never rewritten: the module row and its current version row are
//! updated in one transaction, so an interruption leaves the row on v1 and the
//! next start retries. A row that fails either check is disabled
//! (fail-closed), keeps its old value and never executes.

use crate::{
    db_error, digest, insert_update_event, manifest_matches, valid_module_id, valid_version_path,
    ModulePackageError, ModulePublisher, ReleaseLease,
};
use dgos_db::{
    entities::{module_versions, modules},
    sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait},
};
use std::fs;
use uuid::Uuid;

/// What one migration pass did, by module id.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IntegrityMigrationReport {
    /// Moved from v1 to v2.
    pub migrated: Vec<String>,
    /// Failed verification and was disabled; the old value is kept.
    pub disabled: Vec<String>,
}

impl ModulePublisher {
    /// Migrate every module row that still has a legacy integrity value.
    /// Idempotent; safe to call at every start.
    pub async fn migrate_legacy_integrity(
        &self,
    ) -> Result<IntegrityMigrationReport, ModulePackageError> {
        let _publish_guard = self.publish_lock.lock().await;
        let lease = self.acquire_release_lock().await?;
        self.migrate_legacy_locked(&lease).await
    }

    /// The migration with the publish lock and the release lease held.
    pub(crate) async fn migrate_legacy_locked(
        &self,
        lease: &ReleaseLease,
    ) -> Result<IntegrityMigrationReport, ModulePackageError> {
        let mut report = IntegrityMigrationReport::default();
        let rows = modules::Entity::find()
            .all(&self.db)
            .await
            .map_err(db_error)?;
        for row in rows
            .into_iter()
            .filter(|row| !digest::is_v2_integrity(&row.integrity))
        {
            lease.ensure_valid()?;
            match self.legacy_row_snapshot(&row) {
                Some(new_integrity) => {
                    let tx = self.db.begin().await.map_err(db_error)?;
                    let old = row.integrity.clone();
                    let (id, version) = (row.id.clone(), row.current_version.clone());
                    let mut model: modules::ActiveModel = row.into();
                    model.integrity = Set(new_integrity.clone());
                    model.updated_at = Set(chrono::Utc::now().timestamp());
                    model.update(&tx).await.map_err(db_error)?;
                    if let Some(version_row) = module_versions::Entity::find()
                        .filter(module_versions::Column::ModuleId.eq(&id))
                        .filter(module_versions::Column::Version.eq(&version))
                        .one(&tx)
                        .await
                        .map_err(db_error)?
                        .filter(|version_row| version_row.integrity.eq_ignore_ascii_case(&old))
                    {
                        let mut model: module_versions::ActiveModel = version_row.into();
                        model.integrity = Set(new_integrity);
                        model.update(&tx).await.map_err(db_error)?;
                    }
                    insert_update_event(
                        &tx,
                        &Uuid::new_v4().to_string(),
                        &id,
                        Some(&version),
                        &version,
                        "integrity_migrated",
                        "success",
                        None,
                    )
                    .await?;
                    lease.ensure_valid()?;
                    tx.commit().await.map_err(db_error)?;
                    report.migrated.push(id);
                }
                None => {
                    let pointer = self.root.join("current").join(&row.id).join("version");
                    let _ = fs::remove_file(pointer);
                    if row.is_enabled {
                        let tx = self.db.begin().await.map_err(db_error)?;
                        let (id, version) = (row.id.clone(), row.current_version.clone());
                        let mut model: modules::ActiveModel = row.into();
                        model.is_enabled = Set(false);
                        model.updated_at = Set(chrono::Utc::now().timestamp());
                        model.update(&tx).await.map_err(db_error)?;
                        insert_update_event(
                            &tx,
                            &Uuid::new_v4().to_string(),
                            &id,
                            Some(&version),
                            &version,
                            "integrity_migration",
                            "failure",
                            Some("integrity".into()),
                        )
                        .await?;
                        lease.ensure_valid()?;
                        tx.commit().await.map_err(db_error)?;
                        report.disabled.push(id);
                    }
                }
            }
        }
        Ok(report)
    }

    /// The v2 integrity of the row's current package, if and only if the
    /// package still equals what the legacy row recorded: files hash to the
    /// recorded v1 value and the manifest equals the recorded manifest.
    fn legacy_row_snapshot(&self, row: &modules::Model) -> Option<String> {
        if !valid_module_id(&row.id) || !valid_version_path(&row.current_version) {
            return None;
        }
        let package = self
            .root
            .join("versions")
            .join(&row.id)
            .join(&row.current_version);
        let snapshot = digest::read_package(&package, true).ok()?;
        let manifest = &snapshot.manifest;
        let consistent = manifest.id == row.id
            && manifest.version == row.current_version
            && manifest.artifact.version == row.current_version
            && snapshot
                .legacy_digest
                .as_deref()
                .is_some_and(|legacy| legacy.eq_ignore_ascii_case(&row.integrity))
            && manifest_matches(&row.manifest, manifest);
        consistent.then_some(snapshot.integrity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_db::sea_orm::ConnectionTrait;
    use dgos_db::{migrations::Migrator, sea_orm::Database};
    use sea_orm_migration::MigratorTrait;
    use std::path::Path;
    use tempfile::{tempdir, TempDir};

    async fn publisher() -> (TempDir, ModulePublisher) {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db);
        (store, publisher)
    }

    fn manifest(permissions: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "id": "mod-a", "name": "A", "version": "1.0.0", "entry": "entry.js",
            "artifact": {"version": "1.0.0", "sha256": "0".repeat(64)},
            "permissions": permissions,
        })
    }

    /// Install a package the way the pre-v2 host did: files under
    /// `versions/`, a pointer, and rows holding the v1 digest.
    async fn install_legacy(publisher: &ModulePublisher, permissions: &[&str]) -> String {
        let dir = publisher.root.join("versions/mod-a/1.0.0");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("entry.js"), "export default 1;").unwrap();
        let mut manifest = manifest(permissions);
        fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        let legacy = digest::read_package(&dir, true)
            .unwrap()
            .legacy_digest
            .unwrap();
        manifest["artifact"]["sha256"] = legacy.clone().into();
        fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        publisher.write_pointer("mod-a", "1.0.0").unwrap();
        modules::ActiveModel {
            id: Set("mod-a".into()),
            module_name: Set("mod-a".into()),
            display_name: Set("A".into()),
            current_version: Set("1.0.0".into()),
            description: Set(None),
            manifest: Set(manifest),
            integrity: Set(legacy.clone()),
            entry_url: Set(Some("entry.js".into())),
            icon_url: Set(None),
            is_official: Set(false),
            is_enabled: Set(true),
            installed_at: Set(0),
            updated_at: Set(0),
        }
        .insert(&publisher.db)
        .await
        .unwrap();
        module_versions::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            module_id: Set("mod-a".into()),
            version: Set("1.0.0".into()),
            integrity: Set(legacy.clone()),
            entry_path: Set("entry.js".into()),
            created_at: Set(0),
        }
        .insert(&publisher.db)
        .await
        .unwrap();
        legacy
    }

    async fn row(publisher: &ModulePublisher) -> modules::Model {
        modules::Entity::find_by_id("mod-a")
            .one(&publisher.db)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn an_untouched_legacy_install_is_migrated_to_the_v2_value_of_the_same_files() {
        let (_store, publisher) = publisher().await;
        install_legacy(&publisher, &["providers.call"]).await;

        let report = publisher.migrate_legacy_integrity().await.unwrap();
        assert_eq!(report.migrated, ["mod-a"]);
        assert!(report.disabled.is_empty());

        let migrated = row(&publisher).await;
        let on_disk =
            digest::package_integrity(&publisher.root.join("versions/mod-a/1.0.0")).unwrap();
        assert_eq!(migrated.integrity, on_disk);
        assert!(digest::is_v2_integrity(&migrated.integrity));
        assert!(migrated.is_enabled);
        let version_row = module_versions::Entity::find()
            .filter(module_versions::Column::ModuleId.eq("mod-a"))
            .one(&publisher.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(version_row.integrity, on_disk);
        // The migrated module can now be recovered and executed.
        assert_eq!(publisher.recover_registered().await.unwrap(), ["mod-a"]);
        // A second pass has nothing to do.
        assert_eq!(
            publisher.migrate_legacy_integrity().await.unwrap(),
            IntegrityMigrationReport::default()
        );
    }

    #[tokio::test]
    async fn a_manifest_edited_after_publishing_is_not_migrated_and_the_module_is_disabled() {
        let (_store, publisher) = publisher().await;
        let legacy = install_legacy(&publisher, &["providers.call"]).await;
        // The v1 digest never covered the manifest: widening the declared
        // permissions left the old digest valid. Migration must notice.
        let path = publisher.root.join("versions/mod-a/1.0.0/manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["permissions"] = serde_json::json!(["providers.call", "providers.list"]);
        fs::write(&path, manifest.to_string()).unwrap();

        let report = publisher.migrate_legacy_integrity().await.unwrap();
        assert_eq!(report.disabled, ["mod-a"]);
        assert!(report.migrated.is_empty());
        let after = row(&publisher).await;
        assert!(!after.is_enabled);
        assert_eq!(after.integrity, legacy, "the old value is kept");
        assert!(publisher.pointer_version("mod-a").is_none());
        // Nothing legacy ever executes.
        assert!(publisher.recover_registered().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn changed_files_are_not_migrated() {
        let (_store, publisher) = publisher().await;
        install_legacy(&publisher, &[]).await;
        fs::write(
            publisher.root.join("versions/mod-a/1.0.0/entry.js"),
            "export default 'tampered';",
        )
        .unwrap();
        let report = publisher.migrate_legacy_integrity().await.unwrap();
        assert_eq!(report.disabled, ["mod-a"]);
        assert!(!digest::is_v2_integrity(&row(&publisher).await.integrity));
    }

    #[tokio::test]
    async fn an_interrupted_migration_leaves_the_row_legacy_and_the_retry_completes_it() {
        let (_store, publisher) = publisher().await;
        let legacy = install_legacy(&publisher, &[]).await;
        // Fault injection: the version-row update cannot run, so the
        // transaction fails after the module row was written.
        publisher
            .db
            .execute_unprepared("DROP TABLE module_update_events")
            .await
            .unwrap();
        assert!(publisher.migrate_legacy_integrity().await.is_err());
        let after = row(&publisher).await;
        assert_eq!(after.integrity, legacy, "rolled back, still legacy");
        assert!(after.is_enabled);

        publisher
            .db
            .execute_unprepared(
                "CREATE TABLE module_update_events (id TEXT PRIMARY KEY NOT NULL, \
                 operation_id TEXT NOT NULL, module_id TEXT NOT NULL, from_version TEXT, \
                 to_version TEXT NOT NULL, stage TEXT NOT NULL, outcome TEXT NOT NULL, \
                 error_class TEXT, created_at BIGINT NOT NULL)",
            )
            .await
            .unwrap();
        let report = publisher.migrate_legacy_integrity().await.unwrap();
        assert_eq!(report.migrated, ["mod-a"]);
    }

    #[allow(dead_code)]
    fn _unused(_: &Path) {}
}
