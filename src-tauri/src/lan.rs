//! FR-010 host wiring: the LAN service over the host's own managers.
//!
//! - remote chat uses the managed `ChatManager` (one generation registry for
//!   desktop and remote), `ProviderManager` and Skills manager;
//! - the remote session's user is checked against `SessionResolver` on every
//!   request; codes are issued for the resolver's current user;
//! - FR-008's `RemoteSessionRevoker` is [`LanSessionRevoker`] (replaces
//!   `NoRemoteSessions` in this host).

use crate::commands::SkillsState;
use crate::session::SessionResolver;
use dgos_application::RemoteSessionRevoker;
use dgos_db::sea_orm::DatabaseConnection;
use dgos_db::{HostSettingsRepository, LanAuditRepository};
use dgos_lan::{
    ChatHost, DbAuditSink, Issuer, LanConfig, LanCore, LanService, ManagedChat, SpaFiles,
    SystemClock, SystemNetwork,
};
use dgos_providers::ProviderManager;
use dgos_skills::SkillsManager;
use dgos_workbench::ChatManager;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;

struct TauriChatHost(tauri::AppHandle);

struct HostLocaleSource(HostSettingsRepository);

#[async_trait::async_trait]
impl dgos_lan::locale::LocaleSource for HostLocaleSource {
    async fn bootstrap(&self) -> Result<dgos_lan::locale::LocaleBootstrap, ()> {
        let preferences = crate::commands::locale::read(&self.0)
            .await
            .map_err(|_| ())?;
        Ok(dgos_lan::locale::LocaleBootstrap {
            locale: preferences.locale,
            catalog_version: preferences.catalog_version,
        })
    }
}

impl ChatHost for TauriChatHost {
    fn chat(&self) -> ChatManager {
        self.0.state::<ChatManager>().inner().clone()
    }

    fn providers(&self) -> &ProviderManager {
        self.0.state::<ProviderManager>().inner()
    }

    fn skills(&self) -> Arc<RwLock<SkillsManager>> {
        self.0.state::<SkillsState>().manager.clone()
    }
}

/// The resolver's user: signed in, or `default-user` while no account exists.
struct ResolverIssuer(Arc<SessionResolver>);

impl Issuer for ResolverIssuer {
    fn current_user(&self) -> Option<String> {
        self.0.state().user_id().map(str::to_owned)
    }
}

/// FR-008 §9.4 step 4 (D088, D099 item 7).
pub struct LanSessionRevoker(pub Arc<LanService>);

#[async_trait::async_trait]
impl RemoteSessionRevoker for LanSessionRevoker {
    async fn revoke_issued_by(&self, user_id: &str) {
        self.0.revoke_issued_by(user_id).await;
    }
}

/// The remote SPA build: `dist-lan/` next to the sources in debug builds,
/// the bundled `dist-lan` resource in release builds.
fn spa_files(app: &tauri::AppHandle) -> Option<SpaFiles> {
    #[cfg(debug_assertions)]
    let root = {
        let _ = app;
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist-lan")
    };
    #[cfg(not(debug_assertions))]
    let root = app.path().resource_dir().ok()?.join("dist-lan");
    if !root.join("index.html").is_file() {
        tracing::warn!("Remote SPA build not found; LAN pages will answer 404");
    }
    Some(SpaFiles::new(root))
}

/// Built inside the Tokio runtime (the audit writer is spawned here).
pub fn build(
    app: &tauri::AppHandle,
    db: &DatabaseConnection,
    resolver: Arc<SessionResolver>,
) -> Arc<LanService> {
    let maintenance = app.state::<dgos_db::MaintenanceGate>().inner().clone();
    let core = Arc::new(LanCore::new(
        Arc::new(SystemClock),
        Arc::new(ResolverIssuer(resolver)),
        Arc::new(DbAuditSink::spawn_with_maintenance(LanAuditRepository::new(db.clone()), maintenance.clone())),
        Arc::new(ManagedChat::new(TauriChatHost(app.clone()))),
    ));
    LanService::new(
        core,
        Arc::new(HostSettingsRepository::new(db.clone())),
        Arc::new(SystemNetwork),
        Some(LanAuditRepository::new(db.clone())),
        LanConfig {
            spa: spa_files(app),
            locale: Arc::new(HostLocaleSource(HostSettingsRepository::new(db.clone()))),
            ..LanConfig::default()
        },
    )
}
