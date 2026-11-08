//! Configuration presence is not evidence of native credentials or a live session.
//! This adapter never opens a credential store, invokes a helper, or starts OAuth.
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum OperationSupport { Unsupported }

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct Projection {
    pub configured: bool,
    pub credential_present: Option<bool>,
    pub authenticated: Option<bool>,
    pub connected: Option<bool>,
    pub approved: Option<bool>,
    pub active_session: Option<bool>,
    pub login: OperationSupport,
    pub logout: OperationSupport,
    pub delete_credentials: OperationSupport,
}
impl Projection {
    pub(super) fn configuration_only(configured: bool) -> Self {
        Self {
            configured,
            credential_present: None,
            authenticated: None,
            connected: None,
            approved: None,
            active_session: None,
            login: OperationSupport::Unsupported,
            logout: OperationSupport::Unsupported,
            delete_credentials: OperationSupport::Unsupported,
        }
    }
}
