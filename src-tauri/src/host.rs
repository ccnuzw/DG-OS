use crate::session::SessionResolver;
use dgos_db::repositories::maintenance_gate::{ActivityPermit, MaintenanceGate};
use dgos_types::{BuiltinCaller, DgosError, HostExecutionContext, HostSession};
use std::sync::Arc;
use tauri::{Manager, WebviewWindow};

/// Keeps maintenance admission alive through the complete command, including awaited writes.
pub struct Admission<T> {
    value: T,
    _permit: Option<ActivityPermit>,
}
impl<T> std::ops::Deref for Admission<T> {
    type Target = T;
    fn deref(&self) -> &T { &self.value }
}
pub fn admitted<T>(window: &WebviewWindow, value: T) -> Result<Admission<T>, DgosError> {
    let permit = match window.try_state::<MaintenanceGate>() {
        Some(gate) => Some(gate.admit().map_err(|_| DgosError::ProviderRequestRejected)?),
        None if cfg!(any(test, feature = "webview-smoke")) => None,
        None => return Err(DgosError::InternalError("maintenance admission unavailable".into())),
    };
    Ok(Admission { value, _permit: permit })
}

/// The pre-account identity, for tests and smoke fixtures that seed data.
/// Production commands resolve the user through `SessionResolver`.
#[cfg(any(test, feature = "webview-smoke"))]
pub const LOCAL_USER: &str = dgos_types::DEFAULT_USER_ID;

#[derive(Clone, Copy, Debug)]
pub enum Capability {
    ProvidersRead,
    ProvidersManage,
    ProvidersCall,
    SkillsRead,
    SkillsManage,
    SkillsPrepare,
    McpRead,
    McpManage,
    McpCall,
    TasksRun,
    ModulesRead,
    ModulesManage,
    Appearance,
    /// FR-010 host LAN management (contract §14.8). Settings caller only.
    LanManage,
    /// Storage statistics and cleanup (D133 item 5). Settings caller only.
    StorageManage,
}

/// Entry gate of the renderer commands: trusted main window, then the
/// host-resolved session (`1008007` while signed out, switching or deleting;
/// before caller/capability, D090), then caller/capability policy.
pub fn context(
    window: &WebviewWindow,
    caller: BuiltinCaller,
    capability: Capability,
) -> Result<Admission<HostExecutionContext>, DgosError> {
    let url = window.url().map_err(|_| DgosError::PermissionDenied)?;
    if !trusted_main_window(window.label(), &url, cfg!(debug_assertions)) {
        return Err(DgosError::PermissionDenied);
    }
    let session = resolved_session(window)?;
    if !allows(caller, capability) {
        return Err(DgosError::PermissionDenied);
    }
    let value = HostExecutionContext::for_builtin(session.user_id(), caller);
    // Storage owns its exclusive lease; its cleanup writes acquire their own activity permit.
    if matches!(capability, Capability::StorageManage) {
        Ok(Admission { value, _permit: None })
    } else {
        admitted(window, value)
    }
}


/// Admits one write that already passed its own session gate (`workspace_catalog_session` or
/// `module_session`). Same non-blocking `MaintenanceGate::admit`; the caller holds the permit for
/// the whole write.
pub fn admit_activity(app: &tauri::AppHandle) -> Result<ActivityPermit, DgosError> {
    app.state::<MaintenanceGate>().admit().map_err(|_| DgosError::ProviderRequestRejected)
}

/// The resolver's current session. A host without a resolver fails closed.
fn resolved_session(window: &WebviewWindow) -> Result<HostSession, DgosError> {
    window
        .app_handle()
        .try_state::<Arc<SessionResolver>>()
        .ok_or(DgosError::SessionInvalid)?
        .current()
}

/// Bind both lifecycle tokens from managed host state; renderer identifiers select nothing else.
pub fn module_call_leases(window: &WebviewWindow, call: &dgos_modules::BoundModuleCall) -> Result<(HostSession, tokio_util::sync::CancellationToken, tokio_util::sync::CancellationToken), DgosError> {
    module_host_window(window)?;
    let app = window.app_handle();
    let (session, account) = app.try_state::<Arc<SessionResolver>>().ok_or(DgosError::SessionInvalid)?.agent_session()?;
    if session.user_id() != call.context().user_id() || account.is_cancelled() { return Err(DgosError::SessionInvalid); }
    let attachment = app.try_state::<crate::module_host::ModuleHostBridge>().ok_or(DgosError::ModuleCallCancelled)?.attachment_lease(call.attachment_id())?;
    Ok((session, account, attachment))
}

/// Packaged app origins, plus the Vite dev server on port 1420 in debug
/// builds only.
fn trusted_origin(url: &tauri::Url, allow_development: bool) -> bool {
    let packaged = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"));
    let development = allow_development
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
        && url.port() == Some(1420);
    packaged || development
}

/// The only window host pages run in, on a packaged or debug-dev origin.
/// Every renderer-facing gate starts here; route and caller policy are
/// layered on top by the gates that need them.
fn trusted_main_window(label: &str, url: &tauri::Url, allow_development: bool) -> bool {
    label == "main" && trusted_origin(url, allow_development)
}

/// The Settings route is `/settings` or a sub-path of it. This is a route
/// policy for the host's own pages, not a sandbox: script sharing the
/// renderer's realm can change the path.
fn is_settings_route(url: &tauri::Url) -> bool {
    url.path() == "/settings" || url.path().starts_with("/settings/")
}

/// Entry gate for module lifecycle commands (D034/D035). The same window,
/// origin and caller checks as `context`, plus the window must currently be
/// on the Settings route, then the host-derived session. Module commands
/// never receive a user id, runtime handle or execution context from the
/// renderer.
pub fn module_session(
    window: &WebviewWindow,
    capability: Capability,
) -> Result<Admission<HostSession>, DgosError> {
    let context = context(window, BuiltinCaller::Settings, capability)?;
    let url = window.url().map_err(|_| DgosError::PermissionDenied)?;
    if !is_settings_route(&url) {
        return Err(DgosError::PermissionDenied);
    }
    Ok(Admission { value: HostSession::for_user(context.user_id()), _permit: context._permit })
}

/// D128 read-only workspace policy: trusted main plus host session, no Settings route privilege.
pub fn workspace_catalog_session(window: &WebviewWindow) -> Result<Admission<HostSession>, DgosError> {
    module_host_window(window)?;
    admitted(window, resolved_session(window)?)
}

/// Entry gate for the module host bridge commands (D046). Window label and
/// origin only: no caller, capability or route policy, because the page
/// attaches at application start and its modules outlive SPA navigation.
/// It is a page-lifecycle transport check, not authorization; admission
/// (D041) decides what may run.
pub fn module_host_window(window: &WebviewWindow) -> Result<(), DgosError> {
    let url = window.url().map_err(|_| DgosError::PermissionDenied)?;
    if trusted_main_window(window.label(), &url, cfg!(debug_assertions)) {
        Ok(())
    } else {
        Err(DgosError::PermissionDenied)
    }
}

/// Entry gate of the account commands (D090, contract §9.1): the trusted
/// `main` window only. No signed-in session, caller, capability or route
/// check, because the sign-in page calls them; the account service decides
/// by account mode, and password checks are the boundary.
pub fn account_window(window: &WebviewWindow) -> Result<(), DgosError> {
    let url = window.url().map_err(|_| DgosError::PermissionDenied)?;
    if trusted_main_window(window.label(), &url, cfg!(debug_assertions)) {
        Ok(())
    } else {
        Err(DgosError::PermissionDenied)
    }
}

/// Entry gate for the user's answer to a module's MCP confirmation (D093).
/// The dialog is mounted at the App root and the asking module outlives SPA
/// navigation, so like `module_host_window` this checks only the `main`
/// window and a trusted origin, never the route, caller or capability. The
/// user is derived here, never taken from the renderer; the confirmation
/// broker then requires the host-issued id to belong to that user. Not
/// isolation: scripts sharing the page's realm can call it too (D050).
pub fn module_confirmation_session(window: &WebviewWindow) -> Result<Admission<HostSession>, DgosError> {
    module_host_window(window)?;
    admitted(window, resolved_session(window)?)
}

// Internal route policy, not a sandbox for code sharing the host's JavaScript realm.
fn allows(caller: BuiltinCaller, capability: Capability) -> bool {
    use Capability::*;
    match caller {
        BuiltinCaller::Settings => !matches!(capability, TasksRun),
        BuiltinCaller::Workbench => matches!(
            capability,
            ProvidersRead
                | ProvidersCall
                | SkillsRead
                | SkillsPrepare
                | McpRead
                | McpCall
                | TasksRun
        ),
        // Module contexts are minted by the capability service, never by a
        // renderer-facing Tauri command.
        BuiltinCaller::Module => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_routes_have_the_capabilities_their_handlers_need() {
        assert!(allows(BuiltinCaller::Settings, Capability::ProvidersManage));
        assert!(allows(BuiltinCaller::Settings, Capability::SkillsManage));
        assert!(allows(BuiltinCaller::Settings, Capability::McpManage));
        assert!(allows(BuiltinCaller::Workbench, Capability::TasksRun));
        assert!(allows(BuiltinCaller::Workbench, Capability::SkillsPrepare));
        assert!(allows(BuiltinCaller::Workbench, Capability::ProvidersCall));
        assert!(allows(BuiltinCaller::Settings, Capability::ModulesRead));
        assert!(allows(BuiltinCaller::Settings, Capability::ModulesManage));
    }

    #[test]
    fn only_the_settings_route_may_read_or_manage_modules() {
        for capability in [Capability::ModulesRead, Capability::ModulesManage] {
            assert!(!allows(BuiltinCaller::Workbench, capability));
            assert!(!allows(BuiltinCaller::Module, capability));
        }
        assert_eq!(HostSession::local_default().user_id(), LOCAL_USER);
    }

    #[test]
    fn only_the_settings_caller_may_manage_the_lan_service() {
        assert!(allows(BuiltinCaller::Settings, Capability::LanManage));
        assert!(!allows(BuiltinCaller::Workbench, Capability::LanManage));
        assert!(!allows(BuiltinCaller::Module, Capability::LanManage));
    }

    #[test]
    fn only_the_settings_caller_may_read_the_appearance_font_catalogue() {
        assert!(allows(BuiltinCaller::Settings, Capability::Appearance));
        assert!(!allows(BuiltinCaller::Workbench, Capability::Appearance));
        assert!(!allows(BuiltinCaller::Module, Capability::Appearance));
    }

    fn url(value: &str) -> tauri::Url {
        tauri::Url::parse(value).unwrap()
    }

    #[test]
    fn only_packaged_origins_and_the_debug_dev_server_are_trusted() {
        assert!(trusted_origin(&url("tauri://localhost/settings"), false));
        assert!(trusted_origin(&url("http://tauri.localhost/"), false));
        assert!(!trusted_origin(&url("http://localhost:1420/"), false));
        assert!(trusted_origin(&url("http://localhost:1420/"), true));
        assert!(trusted_origin(&url("http://127.0.0.1:1420/"), true));
        for hostile in [
            "http://localhost:1421/",
            "http://localhost/",
            "https://localhost:1420/",
            "tauri://localhost.evil.test/",
            "http://tauri.localhost.evil.test/",
            "https://example.com/settings",
            "file:///settings",
        ] {
            assert!(!trusted_origin(&url(hostile), true), "{hostile}");
        }
    }

    #[test]
    fn bridge_gate_needs_the_main_window_on_a_trusted_origin() {
        let packaged = url("tauri://localhost/");
        assert!(trusted_main_window("main", &packaged, false));
        for label in ["secondary", "Main", "main ", "", "main-2"] {
            assert!(!trusted_main_window(label, &packaged, true), "{label:?}");
        }
        for hostile in [
            "https://example.com/",
            "file:///index.html",
            "http://localhost:1421/",
            "tauri://localhost.evil.test/",
        ] {
            assert!(
                !trusted_main_window("main", &url(hostile), true),
                "{hostile}"
            );
        }
        // The dev server is trusted in debug builds only.
        let dev = url("http://localhost:1420/");
        assert!(trusted_main_window("main", &dev, true));
        assert!(!trusted_main_window("main", &dev, false));
    }

    #[test]
    fn bridge_gate_ignores_the_route_but_lifecycle_commands_do_not() {
        // SPA navigation keeps the page, its attachment and its modules, so
        // the bridge must keep working on every route of a trusted origin.
        for route in [
            "tauri://localhost/",
            "tauri://localhost/workbench",
            "tauri://localhost/settings",
            "tauri://localhost/settings/modules?x=1#y",
            "tauri://localhost/anything/else",
        ] {
            let route = url(route);
            assert!(trusted_main_window("main", &route, false), "{route}");
        }
        // The five lifecycle commands (D037) stay Settings-only.
        assert!(!is_settings_route(&url("tauri://localhost/workbench")));
        assert!(!is_settings_route(&url("tauri://localhost/")));
        assert!(is_settings_route(&url(
            "tauri://localhost/settings/modules"
        )));
    }

    #[test]
    fn module_commands_require_the_settings_route() {
        assert!(is_settings_route(&url("tauri://localhost/settings")));
        assert!(is_settings_route(&url(
            "tauri://localhost/settings/modules?x=1#y"
        )));
        for other in [
            "tauri://localhost/",
            "tauri://localhost/workbench",
            "tauri://localhost/settings-evil",
            "tauri://localhost/settingsx/settings",
            "tauri://localhost/a/settings",
            "tauri://localhost/settings/../workbench",
        ] {
            assert!(!is_settings_route(&url(other)), "{other}");
        }
    }
}
