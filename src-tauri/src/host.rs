use dgos_types::{BuiltinCaller, DgosError, HostExecutionContext, HostSession};
use tauri::WebviewWindow;

// Preserve the identity used by existing desktop data until FR-008 supplies sessions.
pub const LOCAL_USER: &str = "default-user";

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
}

pub fn context(
    window: &WebviewWindow,
    caller: BuiltinCaller,
    capability: Capability,
) -> Result<HostExecutionContext, DgosError> {
    let url = window.url().map_err(|_| DgosError::PermissionDenied)?;
    let trusted = trusted_origin(&url, cfg!(debug_assertions));
    if window.label() != "main" || !trusted || !allows(caller, capability) {
        return Err(DgosError::PermissionDenied);
    }
    Ok(HostExecutionContext::for_builtin(LOCAL_USER, caller))
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
) -> Result<HostSession, DgosError> {
    let context = context(window, BuiltinCaller::Settings, capability)?;
    let url = window.url().map_err(|_| DgosError::PermissionDenied)?;
    if !is_settings_route(&url) {
        return Err(DgosError::PermissionDenied);
    }
    debug_assert_eq!(context.user_id(), LOCAL_USER);
    Ok(HostSession::local_default())
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
