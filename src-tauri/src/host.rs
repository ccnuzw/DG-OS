use dgos_types::{BuiltinCaller, DgosError, HostExecutionContext};
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
    }
}
