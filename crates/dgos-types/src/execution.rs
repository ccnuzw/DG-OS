use serde::Serialize;

/// Only trusted Rust host code constructs execution provenance; never an IPC DTO.
#[derive(Clone, Debug, Serialize)]
pub struct HostExecutionContext {
    user_id: String,
    source: ExecutionSource,
    request_id: String,
}

#[derive(Clone, Debug)]
pub struct ExecutionProvenance {
    pub caller: BuiltinCaller,
    pub request_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinCaller {
    Settings,
    Workbench,
    Module,
}

/// A module source is only constructed by a validated host publisher. It is
/// deliberately not accepted as an IPC field or renderer-provided token.
#[derive(Clone, Debug, Serialize)]
pub enum ExecutionSource {
    Builtin(BuiltinCaller),
    Module {
        module_id: String,
        version: String,
        integrity: String,
    },
}

impl HostExecutionContext {
    pub fn for_builtin(user_id: &str, caller: BuiltinCaller) -> Self {
        Self {
            user_id: user_id.into(),
            source: ExecutionSource::Builtin(caller),
            request_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub fn for_module(user_id: &str, module_id: &str, version: &str, integrity: &str) -> Self {
        Self {
            user_id: user_id.into(),
            source: ExecutionSource::Module {
                module_id: module_id.into(),
                version: version.into(),
                integrity: integrity.into(),
            },
            request_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn caller(&self) -> BuiltinCaller {
        match &self.source {
            ExecutionSource::Builtin(caller) => *caller,
            ExecutionSource::Module { .. } => BuiltinCaller::Module,
        }
    }

    pub fn source(&self) -> &ExecutionSource {
        &self.source
    }

    pub fn module_identity(&self) -> Option<(&str, &str, &str)> {
        match &self.source {
            ExecutionSource::Module {
                module_id,
                version,
                integrity,
            } => Some((module_id, version, integrity)),
            ExecutionSource::Builtin(_) => None,
        }
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn provenance(&self) -> ExecutionProvenance {
        ExecutionProvenance {
            caller: self.caller(),
            request_id: self.request_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_context_has_fresh_request_provenance() {
        let first = HostExecutionContext::for_builtin("local", BuiltinCaller::Settings);
        let next = HostExecutionContext::for_builtin("local", BuiltinCaller::Workbench);
        assert_eq!(first.user_id(), "local");
        assert_eq!(first.caller(), BuiltinCaller::Settings);
        assert_ne!(first.request_id(), next.request_id());
        assert!(uuid::Uuid::parse_str(first.request_id()).is_ok());
    }
}
