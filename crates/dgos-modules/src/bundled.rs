//! Provisioning of the modules that ship with the host (D052, D058, ADR-0002).
//!
//! The shipped directory is checked against the compiled-in pins first
//! (`release_gate`, D049). Only if that check is clean is anything published,
//! so a bundle that was changed after the build never reaches the publisher.
//!
//! Provisioning only *installs* a pinned package that has no row yet. It never
//! replaces, downgrades or re-enables something that is already there: an
//! existing module with another version or another digest is reported and left
//! alone, and version changes go through the update path (D033). Publishing
//! grants nothing and starts nothing; permissions stay a user decision.
//!
//! The first publish creates the row as official and disabled in the same
//! transaction that moves the pointer, so no interruption can leave a bundled
//! module that is enabled or removable (D058).

use crate::{
    admission::BUILTIN_PINS,
    db_error,
    release_gate::{verify_release_bundle, ReleasePin, ReleaseViolation},
    FirstPublish, ModulePackageError, ModulePublisher,
};
use dgos_db::{
    entities::{module_permissions, modules},
    sea_orm::{
        ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
        TransactionTrait,
    },
};
use std::path::Path;
use thiserror::Error;

/// What provisioning did, by `(module_id, version)`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProvisionReport {
    /// Published now.
    pub published: Vec<(String, String)>,
    /// Already published with exactly the pinned version and digest.
    pub present: Vec<(String, String)>,
    /// A row exists for the module with a different version or digest. Left
    /// untouched; the update path owns version changes.
    pub kept_other_release: Vec<(String, String)>,
    /// Present with the pinned content, but a row left by an interrupted
    /// older provisioning was not yet official; repaired now (D058).
    pub repaired: Vec<(String, String)>,
}

#[derive(Debug, Error)]
pub enum ProvisionError {
    /// The shipped directory does not equal the pins. Nothing was published.
    #[error("the shipped module bundle does not match the compiled-in pins")]
    Bundle(Vec<ReleaseViolation>),
    #[error(transparent)]
    Package(#[from] ModulePackageError),
}

impl ModulePublisher {
    /// Provision the modules pinned in this build from `root`
    /// (`<root>/<module_id>/<version>/`).
    pub async fn provision_builtin(&self, root: &Path) -> Result<ProvisionReport, ProvisionError> {
        self.provision(BUILTIN_PINS, root).await
    }

    pub(crate) async fn provision(
        &self,
        pins: &[ReleasePin<'_>],
        root: &Path,
    ) -> Result<ProvisionReport, ProvisionError> {
        let violations = verify_release_bundle(pins, root);
        if !violations.is_empty() {
            return Err(ProvisionError::Bundle(violations));
        }
        // Rows installed under the v1 digest are moved to v2 first, so the
        // comparison with the (v2) pins below sees them as what they are.
        self.migrate_legacy_integrity().await?;
        let mut report = ProvisionReport::default();
        for (module_id, version, integrity) in pins {
            let key = (module_id.to_string(), version.to_string());
            let row = modules::Entity::find_by_id(*module_id)
                .one(&self.db)
                .await
                .map_err(db_error)?;
            match row {
                Some(row) if row.current_version == *version && row.integrity == *integrity => {
                    if self.repair_bundled(&row).await? {
                        report.repaired.push(key);
                    } else {
                        report.present.push(key);
                    }
                }
                Some(_) => report.kept_other_release.push(key),
                None => {
                    let staged = self.stage(&root.join(module_id).join(version))?;
                    if staged.integrity() != *integrity {
                        // The gate just compared these bytes, so this means the
                        // files changed between the check and the copy.
                        self.discard_staged(&staged);
                        return Err(ProvisionError::Package(ModulePackageError::Integrity));
                    }
                    self.commit_with(
                        staged,
                        FirstPublish {
                            official: true,
                            enabled: false,
                        },
                    )
                    .await?;
                    report.published.push(key);
                }
            }
        }
        Ok(report)
    }

    /// A row with exactly the pinned id, version and integrity *is* the
    /// bundled module; if an interrupted older provisioning left it
    /// unofficial, make it official, and disabled when the user never granted
    /// it anything (D058). Returns whether anything changed.
    async fn repair_bundled(&self, row: &modules::Model) -> Result<bool, ModulePackageError> {
        if row.is_official {
            return Ok(false);
        }
        let _guard = self.publish_lock.lock().await;
        let tx = self.db.begin().await.map_err(db_error)?;
        let Some(current) = modules::Entity::find_by_id(&row.id)
            .one(&tx)
            .await
            .map_err(db_error)?
            .filter(|current| !current.is_official)
        else {
            return Ok(false);
        };
        let granted = module_permissions::Entity::find()
            .filter(module_permissions::Column::ModuleId.eq(&current.id))
            .filter(module_permissions::Column::Granted.eq(true))
            .count(&tx)
            .await
            .map_err(db_error)?;
        let mut model: modules::ActiveModel = current.into();
        model.is_official = Set(true);
        if granted == 0 {
            model.is_enabled = Set(false);
        }
        model.updated_at = Set(chrono::Utc::now().timestamp());
        model.update(&tx).await.map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_integrity;
    use dgos_db::sea_orm::ConnectionTrait;
    use dgos_db::{migrations::Migrator, sea_orm::Database};
    use sea_orm_migration::MigratorTrait;
    use std::fs;
    use tempfile::{tempdir, TempDir};

    async fn publisher() -> (TempDir, ModulePublisher) {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher = ModulePublisher::new(store.path(), db);
        (store, publisher)
    }

    /// A bundle directory with one package; returns its v2 integrity.
    fn bundle(root: &Path, id: &str, version: &str, body: &str) -> String {
        let dir = root.join(id).join(version);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("entry.js"), body).unwrap();
        let write_manifest = |sha256: &str| {
            fs::write(
                dir.join("manifest.json"),
                serde_json::json!({
                    "id": id, "name": "Bundled", "version": version, "entry": "entry.js",
                    "artifact": {"version": version, "sha256": sha256},
                    "permissions": ["providers.call"]
                })
                .to_string(),
            )
            .unwrap();
        };
        write_manifest(&"0".repeat(64));
        let integrity = package_integrity(&dir).unwrap();
        write_manifest(integrity.strip_prefix("v2:").unwrap());
        integrity
    }

    #[tokio::test]
    async fn a_clean_bundle_is_published_once_and_a_rerun_changes_nothing() {
        let (_store, publisher) = publisher().await;
        let root = tempdir().unwrap();
        let digest = bundle(root.path(), "mod-a", "1.0.0", "export default 1;");
        let pins = [("mod-a", "1.0.0", digest.as_str())];

        let first = publisher.provision(&pins, root.path()).await.unwrap();
        assert_eq!(first.published, [("mod-a".to_owned(), "1.0.0".to_owned())]);
        let listed = publisher.published_modules("default-user").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (listed[0].version.as_str(), listed[0].integrity.as_str()),
            ("1.0.0", digest.as_str())
        );
        // Bundled modules start official and disabled (D055).
        assert!(listed[0].official && !listed[0].enabled);

        let second = publisher.provision(&pins, root.path()).await.unwrap();
        assert!(second.published.is_empty());
        assert_eq!(second.present, [("mod-a".to_owned(), "1.0.0".to_owned())]);
    }

    #[tokio::test]
    async fn a_changed_bundle_publishes_nothing_even_for_its_intact_packages() {
        let (_store, publisher) = publisher().await;
        let root = tempdir().unwrap();
        let good = bundle(root.path(), "mod-a", "1.0.0", "export default 1;");
        let bad = bundle(root.path(), "mod-b", "1.0.0", "export default 2;");
        fs::write(
            root.path().join("mod-b/1.0.0/entry.js"),
            "export default 'changed after the build';",
        )
        .unwrap();
        let pins = [
            ("mod-a", "1.0.0", good.as_str()),
            ("mod-b", "1.0.0", bad.as_str()),
        ];

        let error = publisher.provision(&pins, root.path()).await.unwrap_err();
        assert!(matches!(
            error,
            ProvisionError::Bundle(v) if v == [ReleaseViolation::DigestMismatch {
                module_id: "mod-b".into(),
                version: "1.0.0".into(),
            }]
        ));
        assert!(publisher.release_snapshot("mod-a").await.unwrap().is_none());
        assert!(publisher.release_snapshot("mod-b").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn an_unpinned_package_in_the_bundle_blocks_provisioning() {
        let (_store, publisher) = publisher().await;
        let root = tempdir().unwrap();
        let digest = bundle(root.path(), "mod-a", "1.0.0", "export default 1;");
        bundle(root.path(), "smuggled", "1.0.0", "export default 'x';");

        let error = publisher
            .provision(&[("mod-a", "1.0.0", digest.as_str())], root.path())
            .await
            .unwrap_err();
        assert!(matches!(error, ProvisionError::Bundle(_)));
        assert!(publisher.release_snapshot("mod-a").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn an_existing_module_with_another_release_is_never_replaced() {
        let (_store, publisher) = publisher().await;
        let root = tempdir().unwrap();
        let v2 = bundle(root.path(), "mod-a", "2.0.0", "export default 2;");

        // A user-visible release 1.0.0 is already installed.
        let installed = tempdir().unwrap();
        bundle(installed.path(), "mod-a", "1.0.0", "export default 1;");
        let staged = publisher
            .stage(&installed.path().join("mod-a/1.0.0"))
            .unwrap();
        publisher.commit(staged).await.unwrap();

        let report = publisher
            .provision(&[("mod-a", "2.0.0", v2.as_str())], root.path())
            .await
            .unwrap();
        assert_eq!(
            report.kept_other_release,
            [("mod-a".to_owned(), "2.0.0".to_owned())]
        );
        assert!(report.published.is_empty());
        let snapshot = publisher.release_snapshot("mod-a").await.unwrap().unwrap();
        assert_eq!(snapshot.version(), "1.0.0");
    }

    /// The real shipped bundle and the real pins: provisioning publishes the
    /// first-party module with the digest the host recomputes.
    #[tokio::test]
    async fn the_real_bundle_provisions_against_the_real_pins() {
        let (_store, publisher) = publisher().await;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bundled-modules");
        let report = publisher.provision_builtin(&root).await.unwrap();
        assert_eq!(report.published.len(), BUILTIN_PINS.len());
        for (id, version, integrity) in BUILTIN_PINS {
            let listed = publisher.published_modules("default-user").await.unwrap();
            let module = listed.iter().find(|m| m.module_id == *id).unwrap();
            assert_eq!(module.version, *version);
            assert_eq!(module.integrity, *integrity);
            // Shipped with the host, and off until the user enables it (D055).
            assert!(module.official && !module.enabled);
        }
    }

    async fn row(publisher: &ModulePublisher, id: &str) -> modules::Model {
        modules::Entity::find_by_id(id)
            .one(&publisher.db)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn an_interrupted_first_publish_leaves_no_row_and_the_retry_is_official_and_disabled() {
        let (store, publisher) = publisher().await;
        let root = tempdir().unwrap();
        let digest = bundle(root.path(), "mod-a", "1.0.0", "export default 1;");
        let pins = [("mod-a", "1.0.0", digest.as_str())];

        // Fault injection: the transaction aborts after the package was moved
        // into `versions/`, exactly the window an interruption would hit.
        publisher
            .db
            .execute_unprepared(
                "CREATE TRIGGER fail_module_version BEFORE INSERT ON module_versions \
                 BEGIN SELECT RAISE(ABORT, 'forced failure'); END;",
            )
            .await
            .unwrap();
        assert!(publisher.provision(&pins, root.path()).await.is_err());
        // Never a half-state: no row at all, hence no enabled or removable
        // bundled module.
        assert!(modules::Entity::find_by_id("mod-a")
            .one(&publisher.db)
            .await
            .unwrap()
            .is_none());
        assert!(store.path().join("versions/mod-a/1.0.0").exists());

        publisher
            .db
            .execute_unprepared("DROP TRIGGER fail_module_version")
            .await
            .unwrap();
        let report = publisher.provision(&pins, root.path()).await.unwrap();
        assert_eq!(report.published, [("mod-a".to_owned(), "1.0.0".to_owned())]);
        let row = row(&publisher, "mod-a").await;
        assert!(row.is_official && !row.is_enabled);
        assert!(matches!(
            publisher.uninstall("mod-a").await,
            Err(crate::ManagementError::Official)
        ));
    }

    #[tokio::test]
    async fn a_row_left_unofficial_and_enabled_by_an_older_interrupted_run_is_repaired() {
        let (_store, publisher) = publisher().await;
        let root = tempdir().unwrap();
        let digest = bundle(root.path(), "mod-a", "1.0.0", "export default 1;");
        let pins = [("mod-a", "1.0.0", digest.as_str())];
        // The old two-step provisioning: a plain commit, then a crash before
        // the row was marked official and disabled.
        let staged = publisher.stage(&root.path().join("mod-a/1.0.0")).unwrap();
        publisher.commit(staged).await.unwrap();
        let before = row(&publisher, "mod-a").await;
        assert!(!before.is_official && before.is_enabled);

        let report = publisher.provision(&pins, root.path()).await.unwrap();
        assert_eq!(report.repaired, [("mod-a".to_owned(), "1.0.0".to_owned())]);
        let after = row(&publisher, "mod-a").await;
        assert!(after.is_official && !after.is_enabled);

        let again = publisher.provision(&pins, root.path()).await.unwrap();
        assert_eq!(again.present, [("mod-a".to_owned(), "1.0.0".to_owned())]);
    }

    #[tokio::test]
    async fn repair_keeps_a_module_the_user_already_granted_permissions_enabled() {
        let (_store, publisher) = publisher().await;
        let root = tempdir().unwrap();
        let digest = bundle(root.path(), "mod-a", "1.0.0", "export default 1;");
        let pins = [("mod-a", "1.0.0", digest.as_str())];
        let staged = publisher.stage(&root.path().join("mod-a/1.0.0")).unwrap();
        publisher.commit(staged).await.unwrap();
        publisher
            .permissions
            .grant(
                "default-user",
                "mod-a",
                dgos_db::ModulePermission::ProvidersCall,
            )
            .await
            .unwrap();

        publisher.provision(&pins, root.path()).await.unwrap();
        let after = row(&publisher, "mod-a").await;
        assert!(after.is_official, "never removable");
        assert!(after.is_enabled, "the user's decision is kept");
    }

    #[tokio::test]
    async fn a_legacy_install_is_migrated_before_the_pins_are_compared() {
        let (store, publisher) = publisher().await;
        let root = tempdir().unwrap();
        let digest = bundle(root.path(), "mod-a", "1.0.0", "export default 1;");
        let pins = [("mod-a", "1.0.0", digest.as_str())];
        // Install the same files under the v1 digest, as the previous host did.
        let dir = store.path().join("versions/mod-a/1.0.0");
        crate::copy_dir(&root.path().join("mod-a/1.0.0"), &dir).unwrap();
        let legacy = crate::digest::read_package(&dir, true)
            .unwrap()
            .legacy_digest
            .unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        modules::ActiveModel {
            id: Set("mod-a".into()),
            module_name: Set("mod-a".into()),
            display_name: Set("Bundled".into()),
            current_version: Set("1.0.0".into()),
            description: Set(None),
            manifest: Set(manifest),
            integrity: Set(legacy),
            entry_url: Set(Some("entry.js".into())),
            icon_url: Set(None),
            is_official: Set(true),
            is_enabled: Set(false),
            installed_at: Set(0),
            updated_at: Set(0),
        }
        .insert(&publisher.db)
        .await
        .unwrap();

        let report = publisher.provision(&pins, root.path()).await.unwrap();
        assert_eq!(report.present, [("mod-a".to_owned(), "1.0.0".to_owned())]);
        assert!(report.kept_other_release.is_empty());
        assert_eq!(row(&publisher, "mod-a").await.integrity, digest);
    }
}
