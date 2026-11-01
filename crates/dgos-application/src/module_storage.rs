//! `storage.read` / `storage.write` operations of a module call (D093).
//!
//! The commands admit the call and run it through
//! `ModuleCapabilityDispatchService::run_bound` with the permission named in
//! the table below; this service performs the operation for the resulting
//! `CapabilityGrant`. The scope of every operation is the `(user, module)` of
//! that grant, which the host derived from the runtime handle; the module
//! supplies only a key and a value. Each method also refuses a grant that was
//! issued for another permission, so a wrong mapping fails closed. Authorization
//! (layers 1-3) has already run, so an unauthorized module learns nothing about
//! keys, sizes or limits. Values never reach logs, audits or error text.
//!
//! | operation | permission |
//! |-----------|------------|
//! | get, list | `storage.read` |
//! | set, delete | `storage.write` |

use crate::CapabilityGrant;
use dgos_db::{ModulePermission, ModuleStorageEntry, ModuleStorageRepository};
use dgos_types::{DgosError, DgosResult};
use std::time::Duration;

/// Host-owned total deadline of one storage call (D093 item 8).
pub const MODULE_STORAGE_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct ModuleStorageService {
    storage: ModuleStorageRepository,
}

fn require(grant: &CapabilityGrant, permission: ModulePermission) -> DgosResult<()> {
    if grant.permission == permission {
        Ok(())
    } else {
        Err(DgosError::PermissionDenied)
    }
}

impl ModuleStorageService {
    pub fn new(storage: ModuleStorageRepository) -> Self {
        Self { storage }
    }

    /// `storage.read`. A missing key is `None`.
    pub async fn get(
        &self,
        grant: &CapabilityGrant,
        key: &str,
    ) -> DgosResult<Option<serde_json::Value>> {
        require(grant, ModulePermission::StorageRead)?;
        self.storage
            .get(&grant.user_id, &grant.module_id, key)
            .await
    }

    /// `storage.read`: keys ascending with their sizes, never values.
    pub async fn list(&self, grant: &CapabilityGrant) -> DgosResult<Vec<ModuleStorageEntry>> {
        require(grant, ModulePermission::StorageRead)?;
        self.storage.list(&grant.user_id, &grant.module_id).await
    }

    /// `storage.write`: insert or replace one key.
    pub async fn set(
        &self,
        grant: &CapabilityGrant,
        key: &str,
        value: &serde_json::Value,
    ) -> DgosResult<()> {
        require(grant, ModulePermission::StorageWrite)?;
        self.storage
            .set(&grant.user_id, &grant.module_id, key, value)
            .await
    }

    /// `storage.write`: idempotent delete.
    pub async fn delete(&self, grant: &CapabilityGrant, key: &str) -> DgosResult<()> {
        require(grant, ModulePermission::StorageWrite)?;
        self.storage
            .delete(&grant.user_id, &grant.module_id, key)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HostCapabilityService, ModuleCapabilityDispatchService, ModuleReleaseService};
    use dgos_db::sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
    use dgos_db::{
        migrations::Migrator, sea_orm_migration::MigratorTrait, ModulePermissionRepository,
    };
    use dgos_modules::BoundModuleCall;
    use dgos_modules::{
        ModuleCallRouter, ModulePageTransport, ModulePublisher, ModuleRuntimeHandle,
        ModuleRuntimeSupervisor, PageInstruction, PageLoaderConfig, PageModuleLoader, PageReceipt,
        PageSendError, PinnedModuleAdmission, RuntimeLost,
    };
    use dgos_types::{DgosError, HostSession};
    use serde_json::json;
    use std::fs;
    use std::sync::Arc;
    use tempfile::{tempdir, TempDir};
    use tokio::sync::mpsc;

    const MODULE: &str = "module-1";
    const USER: &str = "default-user";
    const SECRET: &str = "SECRET-STORED-VALUE-5d1e";

    struct ScriptedPage(mpsc::UnboundedSender<(String, PageInstruction)>);

    impl ModulePageTransport for ScriptedPage {
        fn send(
            &self,
            attachment_id: &str,
            instruction: PageInstruction,
        ) -> Result<(), PageSendError> {
            self.0
                .send((attachment_id.to_owned(), instruction))
                .map_err(|_| PageSendError)
        }
    }

    fn package(permissions: &[&str]) -> TempDir {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("entry.js"), "export default 1;").unwrap();
        // The digest excludes `artifact.sha256`, so write a placeholder first.
        let manifest = |sha256: &str| {
            json!({
                "id": MODULE, "name": "Example", "version": "1.0.0", "entry": "entry.js",
                "artifact": {"version": "1.0.0", "sha256": sha256},
                "permissions": permissions
            })
            .to_string()
        };
        fs::write(dir.path().join("manifest.json"), manifest(&"0".repeat(64))).unwrap();
        let integrity = dgos_modules::package_integrity(dir.path()).unwrap();
        fs::write(
            dir.path().join("manifest.json"),
            manifest(integrity.strip_prefix("v2:").unwrap()),
        )
        .unwrap();
        dir
    }

    /// What the four commands do: run the admitted call through the dispatch
    /// service with the permission of the operation, then call the service.
    struct Commands {
        dispatch: ModuleCapabilityDispatchService,
        service: ModuleStorageService,
    }

    impl Commands {
        async fn get(
            &self,
            call: BoundModuleCall,
            key: &str,
        ) -> DgosResult<Option<serde_json::Value>> {
            self.dispatch
                .run_bound(
                    call,
                    ModulePermission::StorageRead,
                    MODULE_STORAGE_DEADLINE,
                    |grant| async move { self.service.get(&grant, key).await },
                )
                .await
        }

        async fn list(&self, call: BoundModuleCall) -> DgosResult<Vec<ModuleStorageEntry>> {
            self.dispatch
                .run_bound(
                    call,
                    ModulePermission::StorageRead,
                    MODULE_STORAGE_DEADLINE,
                    |grant| async move { self.service.list(&grant).await },
                )
                .await
        }

        async fn set(
            &self,
            call: BoundModuleCall,
            key: &str,
            value: &serde_json::Value,
        ) -> DgosResult<()> {
            self.dispatch
                .run_bound(
                    call,
                    ModulePermission::StorageWrite,
                    MODULE_STORAGE_DEADLINE,
                    |grant| async move { self.service.set(&grant, key, value).await },
                )
                .await
        }

        async fn delete(&self, call: BoundModuleCall, key: &str) -> DgosResult<()> {
            self.dispatch
                .run_bound(
                    call,
                    ModulePermission::StorageWrite,
                    MODULE_STORAGE_DEADLINE,
                    |grant| async move { self.service.delete(&grant, key).await },
                )
                .await
        }
    }

    struct World {
        _db_dir: TempDir,
        _store: TempDir,
        db: DatabaseConnection,
        publisher: Arc<ModulePublisher>,
        permissions: ModulePermissionRepository,
        storage: ModuleStorageRepository,
        service: Commands,
        router: ModuleCallRouter,
        attachment_id: String,
        handle: ModuleRuntimeHandle,
        _lost: mpsc::UnboundedReceiver<RuntimeLost>,
        _supervisor: ModuleRuntimeSupervisor,
        driver: tokio::task::JoinHandle<()>,
    }

    impl Drop for World {
        fn drop(&mut self) {
            self.driver.abort();
        }
    }

    impl World {
        /// A started, pinned runtime of a package declaring `declared`.
        async fn new(declared: &[&str]) -> Self {
            let db_dir = tempdir().unwrap();
            let url = format!(
                "sqlite://{}?mode=rwc",
                db_dir.path().join("fixture.db").display()
            );
            let db = dgos_db::establish_connection(&url).await.unwrap();
            Migrator::up(&db, None).await.unwrap();
            let store = tempdir().unwrap();
            let publisher = Arc::new(ModulePublisher::new(store.path(), db.clone()));
            let release = ModuleReleaseService::new(publisher.clone())
                .publish(package(declared).path())
                .await
                .unwrap();
            let admission =
                PinnedModuleAdmission::new([(MODULE, "1.0.0", release.integrity.as_str())]);

            let (sender, mut receiver) = mpsc::unbounded_channel::<(String, PageInstruction)>();
            let (loader, lost) =
                PageModuleLoader::new(Arc::new(ScriptedPage(sender)), PageLoaderConfig::default());
            let attachment_id = loader.attach("main").attachment_id;
            let router = ModuleCallRouter::new(loader.clone());
            let page = loader.clone();
            let driver = tokio::spawn(async move {
                while let Some((attachment, instruction)) = receiver.recv().await {
                    let (ticket, receipt) = match instruction {
                        PageInstruction::Load { ticket, .. } => (ticket, PageReceipt::Loaded),
                        PageInstruction::Unload { ticket, .. } => {
                            (ticket, PageReceipt::Unloaded { hook_failed: false })
                        }
                    };
                    page.receive(&attachment, &ticket, receipt);
                }
            });

            let permissions = ModulePermissionRepository::new(db.clone());
            let storage = ModuleStorageRepository::new(db.clone());
            let dispatch =
                ModuleCapabilityDispatchService::new(HostCapabilityService::with_publisher(
                    Arc::new(ModulePermissionRepository::new(db.clone())),
                    publisher.clone(),
                ));
            let service = Commands {
                dispatch,
                service: ModuleStorageService::new(storage.clone()),
            };
            let supervisor = ModuleRuntimeSupervisor::new(publisher.clone(), Arc::new(loader))
                .with_admission(Arc::new(admission));
            let handle = supervisor
                .start(&HostSession::local_default(), MODULE)
                .await
                .unwrap();
            // Dropping the supervisor would tear the runtime down; the World keeps it.
            Self {
                _db_dir: db_dir,
                _store: store,
                db,
                publisher,
                permissions,
                storage,
                service,
                router,
                attachment_id,
                handle,
                _lost: lost,
                _supervisor: supervisor,
                driver,
            }
        }

        /// What the channel does for a page call: issue the route, admit one call.
        fn call(&self) -> BoundModuleCall {
            let route = self.router.issue(&self.handle).unwrap();
            self.router
                .admit(route.as_str(), &self.attachment_id)
                .unwrap()
        }

        async fn grant(&self, permission: ModulePermission) {
            self.permissions
                .grant(USER, MODULE, permission)
                .await
                .unwrap();
        }

        async fn revoke(&self, permission: ModulePermission) {
            self.permissions
                .revoke(USER, MODULE, permission)
                .await
                .unwrap();
        }

        async fn rows(&self, sql: &str) -> Vec<dgos_db::sea_orm::QueryResult> {
            self.db
                .query_all(Statement::from_string(self.db.get_database_backend(), sql))
                .await
                .unwrap()
        }

        /// `denied` audit rows as `(permission, request_id)`.
        async fn denials(&self) -> Vec<(String, String)> {
            self.rows(&format!(
                "SELECT permission, request_id FROM module_permission_audits \
                 WHERE action = 'denied' AND module_id = '{MODULE}' ORDER BY rowid"
            ))
            .await
            .into_iter()
            .map(|row| {
                (
                    row.try_get("", "permission").unwrap(),
                    row.try_get::<Option<String>>("", "request_id")
                        .unwrap()
                        .expect("denials carry the host-generated request id"),
                )
            })
            .collect()
        }

        async fn audit_dump(&self) -> String {
            self.rows(
                "SELECT user_id, module_id, permission, action, resource_id, request_id \
                 FROM module_permission_audits",
            )
            .await
            .into_iter()
            .map(|row| {
                [
                    "user_id",
                    "module_id",
                    "permission",
                    "action",
                    "resource_id",
                    "request_id",
                ]
                .map(|name| format!("{:?}", row.try_get::<Option<String>>("", name).unwrap()))
                .join(",")
            })
            .collect::<Vec<_>>()
            .join("\n")
        }
    }

    #[tokio::test]
    async fn ac12_ac04_ac05_reading_without_storage_read_is_denied_and_audited() {
        let world = World::new(&["storage.read", "storage.write"]).await;
        // Writing is granted, reading is not: the two permissions are separate.
        world.grant(ModulePermission::StorageWrite).await;
        world
            .storage
            .set(USER, MODULE, "k", &json!(SECRET))
            .await
            .unwrap();

        let get = world.service.get(world.call(), "k").await;
        assert!(matches!(get, Err(DgosError::PermissionDenied)), "{get:?}");
        assert_eq!(get.unwrap_err().error_code(), 1004004);
        assert!(matches!(
            world.service.list(world.call()).await,
            Err(DgosError::PermissionDenied)
        ));

        let denials = world.denials().await;
        assert_eq!(denials.len(), 2);
        assert!(denials
            .iter()
            .all(|(permission, _)| permission == "storage.read"));
        assert_ne!(
            denials[0].1, denials[1].1,
            "each call has its own request id"
        );
        let stats = world.handle.request_stats();
        assert_eq!((stats.admitted, stats.denied, stats.in_flight), (2, 2, 0));
        assert!(!world.audit_dump().await.contains(SECRET));
    }

    #[tokio::test]
    async fn ac03_a_granted_module_reads_writes_lists_and_deletes_its_own_storage() {
        let world = World::new(&["storage.read", "storage.write"]).await;
        world.grant(ModulePermission::StorageRead).await;
        world.grant(ModulePermission::StorageWrite).await;

        assert_eq!(world.service.get(world.call(), "k").await.unwrap(), None);
        let value = json!({"a": [1, 2, 3], "secret": SECRET});
        world.service.set(world.call(), "k", &value).await.unwrap();
        world
            .service
            .set(world.call(), "a", &json!(1))
            .await
            .unwrap();
        assert_eq!(
            world.service.get(world.call(), "k").await.unwrap(),
            Some(value.clone())
        );

        // The row is scoped to the host-bound user and module.
        assert_eq!(
            world.storage.get(USER, MODULE, "k").await.unwrap(),
            Some(value)
        );
        let listed = world.service.list(world.call()).await.unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|entry| entry.key.as_str())
                .collect::<Vec<_>>(),
            ["a", "k"]
        );
        assert!(!format!("{listed:?}").contains(SECRET));

        world.service.delete(world.call(), "k").await.unwrap();
        world.service.delete(world.call(), "k").await.unwrap();
        assert_eq!(world.service.get(world.call(), "k").await.unwrap(), None);

        let stats = world.handle.request_stats();
        assert_eq!(
            (
                stats.admitted,
                stats.succeeded,
                stats.denied,
                stats.failed,
                stats.in_flight
            ),
            (8, 8, 0, 0, 0)
        );
        assert!(world.denials().await.is_empty());
        assert!(!world.audit_dump().await.contains(SECRET));
    }

    #[tokio::test]
    async fn ac12_writing_needs_storage_write_separately_from_storage_read() {
        let world = World::new(&["storage.read", "storage.write"]).await;
        world.grant(ModulePermission::StorageRead).await;

        assert!(matches!(
            world.service.set(world.call(), "k", &json!(SECRET)).await,
            Err(DgosError::PermissionDenied)
        ));
        assert!(matches!(
            world.service.delete(world.call(), "k").await,
            Err(DgosError::PermissionDenied)
        ));
        // Nothing was written, and reading still works.
        assert_eq!(world.service.get(world.call(), "k").await.unwrap(), None);
        assert!(world.storage.list(USER, MODULE).await.unwrap().is_empty());
        let denials = world.denials().await;
        assert_eq!(denials.len(), 2);
        assert!(denials
            .iter()
            .all(|(permission, _)| permission == "storage.write"));

        world.grant(ModulePermission::StorageWrite).await;
        world
            .service
            .set(world.call(), "k", &json!(1))
            .await
            .unwrap();
        world.revoke(ModulePermission::StorageRead).await;
        assert!(matches!(
            world.service.get(world.call(), "k").await,
            Err(DgosError::PermissionDenied)
        ));
        world
            .service
            .set(world.call(), "k", &json!(2))
            .await
            .unwrap();
        assert_eq!(
            world.storage.get(USER, MODULE, "k").await.unwrap(),
            Some(json!(2))
        );
    }

    #[tokio::test]
    async fn ac07_revoking_takes_effect_on_the_next_call_and_keeps_the_data() {
        let world = World::new(&["storage.read", "storage.write"]).await;
        world.grant(ModulePermission::StorageRead).await;
        world.grant(ModulePermission::StorageWrite).await;
        world
            .service
            .set(world.call(), "k", &json!("kept"))
            .await
            .unwrap();
        assert!(world.service.get(world.call(), "k").await.is_ok());

        world.revoke(ModulePermission::StorageRead).await;
        assert!(matches!(
            world.service.get(world.call(), "k").await,
            Err(DgosError::PermissionDenied)
        ));
        world.revoke(ModulePermission::StorageWrite).await;
        assert!(matches!(
            world
                .service
                .set(world.call(), "k", &json!("changed"))
                .await,
            Err(DgosError::PermissionDenied)
        ));
        // The data is the user's: revoking does not delete it, and a regrant sees it.
        assert_eq!(
            world.storage.get(USER, MODULE, "k").await.unwrap(),
            Some(json!("kept"))
        );
        world.grant(ModulePermission::StorageRead).await;
        assert_eq!(
            world.service.get(world.call(), "k").await.unwrap(),
            Some(json!("kept"))
        );
    }

    #[tokio::test]
    async fn ac12_a_manifest_without_storage_permissions_denies_even_with_a_grant_row() {
        // The package declares only providers.call. `grant` refuses undeclared
        // permissions, so plant the stale rows a past manifest could have left.
        let world = World::new(&["providers.call"]).await;
        world
            .db
            .execute_unprepared(&format!(
                "INSERT INTO module_permissions (id, user_id, module_id, permission, granted, granted_at) \
                 VALUES ('p1', '{USER}', '{MODULE}', 'storage.read', 1, 1), \
                        ('p2', '{USER}', '{MODULE}', 'storage.write', 1, 1)"
            ))
            .await
            .unwrap();

        assert!(matches!(
            world.service.get(world.call(), "k").await,
            Err(DgosError::PermissionDenied)
        ));
        assert!(matches!(
            world.service.set(world.call(), "k", &json!(SECRET)).await,
            Err(DgosError::PermissionDenied)
        ));
        assert!(world.storage.list(USER, MODULE).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn ac12_a_disabled_module_is_denied() {
        let world = World::new(&["storage.read", "storage.write"]).await;
        world.grant(ModulePermission::StorageRead).await;
        world.grant(ModulePermission::StorageWrite).await;
        world
            .service
            .set(world.call(), "k", &json!(1))
            .await
            .unwrap();

        world.publisher.set_enabled(MODULE, false).await.unwrap();
        assert!(matches!(
            world.service.get(world.call(), "k").await,
            Err(DgosError::PermissionDenied)
        ));
        assert!(matches!(
            world.service.set(world.call(), "k", &json!(2)).await,
            Err(DgosError::PermissionDenied)
        ));
        world.publisher.set_enabled(MODULE, true).await.unwrap();
        assert_eq!(
            world.service.get(world.call(), "k").await.unwrap(),
            Some(json!(1))
        );
    }

    #[tokio::test]
    async fn ac12_limit_violations_use_their_own_codes_settle_failed_and_leak_no_value() {
        let world = World::new(&["storage.read", "storage.write"]).await;
        world.grant(ModulePermission::StorageRead).await;
        world.grant(ModulePermission::StorageWrite).await;

        let bad_key = world
            .service
            .set(world.call(), "", &json!(SECRET))
            .await
            .unwrap_err();
        assert!(matches!(bad_key, DgosError::ModuleCapabilityInputInvalid));
        assert_eq!(bad_key.error_code(), 1004024);
        let control = world.service.get(world.call(), "a\nb").await.unwrap_err();
        assert!(matches!(control, DgosError::ModuleCapabilityInputInvalid));
        let too_big = json!(format!("{SECRET}{}", "x".repeat(64 * 1024)));
        let oversized = world
            .service
            .set(world.call(), "k", &too_big)
            .await
            .unwrap_err();
        assert!(matches!(oversized, DgosError::ModuleCapabilityInputInvalid));

        // Fill the 5 MiB quota with 80 full-size values, then one byte more.
        let full = json!("x".repeat(64 * 1024 - 2));
        for index in 0..80 {
            world
                .service
                .set(world.call(), &format!("k{index:02}"), &full)
                .await
                .unwrap();
        }
        let quota = world
            .service
            .set(world.call(), "extra", &json!(SECRET))
            .await
            .unwrap_err();
        assert!(matches!(quota, DgosError::ModuleStorageQuotaExceeded));
        assert_eq!(quota.error_code(), 1004025);

        for error in [&bad_key, &control, &oversized, &quota] {
            let text = format!("{error} {error:?}");
            assert!(!text.contains(SECRET), "error leaked the value: {text}");
        }
        // Refusals are failed calls, not permission denials, and nothing is left in flight.
        let stats = world.handle.request_stats();
        assert_eq!(
            (stats.failed, stats.denied, stats.succeeded, stats.in_flight),
            (4, 0, 80, 0)
        );
        assert!(world.denials().await.is_empty());
        assert!(!world.audit_dump().await.contains(SECRET));
    }

    #[tokio::test]
    async fn ac12_a_grant_for_the_other_storage_permission_is_refused_by_the_service() {
        let repository = ModuleStorageRepository::new(
            dgos_db::establish_connection("sqlite::memory:")
                .await
                .unwrap(),
        );
        let service = ModuleStorageService::new(repository);
        let grant = |permission| CapabilityGrant {
            user_id: USER.into(),
            module_id: MODULE.into(),
            version: "1.0.0".into(),
            integrity: "0".repeat(64),
            permission,
            request_id: "request-1".into(),
        };
        let read = grant(ModulePermission::StorageRead);
        let write = grant(ModulePermission::StorageWrite);
        assert!(matches!(
            service.set(&read, "k", &json!(SECRET)).await,
            Err(DgosError::PermissionDenied)
        ));
        assert!(matches!(
            service.delete(&read, "k").await,
            Err(DgosError::PermissionDenied)
        ));
        assert!(matches!(
            service.get(&write, "k").await,
            Err(DgosError::PermissionDenied)
        ));
        assert!(matches!(
            service.list(&write).await,
            Err(DgosError::PermissionDenied)
        ));
    }
}
