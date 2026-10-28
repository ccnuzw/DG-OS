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
    let packaged = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"));
    let development = cfg!(debug_assertions)
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
        && url.port() == Some(1420);
    if window.label() != "main" || !(packaged || development) || !allows(caller, capability) {
        return Err(DgosError::PermissionDenied);
    }
    Ok(HostExecutionContext::for_builtin(LOCAL_USER, caller))
}

/// Entry gate for module lifecycle commands (D034). The same window and
/// caller checks as `context`, then the host-derived session. Module
/// commands never receive a user id, runtime handle or execution context
/// from the renderer.
pub fn module_session(
    window: &WebviewWindow,
    capability: Capability,
) -> Result<HostSession, DgosError> {
    let context = context(window, BuiltinCaller::Settings, capability)?;
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
}
