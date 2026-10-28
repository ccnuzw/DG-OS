use crate::{McpError, Result};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Hash, PartialEq, Eq)]
struct CallKey {
    user_id: String,
    server_id: String,
    call_id: String,
}

#[derive(Clone, Default)]
pub(crate) struct ActiveCalls(Arc<Mutex<HashMap<CallKey, CancellationToken>>>);

impl ActiveCalls {
    pub fn register(&self, user: &str, server: &str, call: &str) -> Result<ActiveCall> {
        validate_call_id(call)?;
        let key = Self::key(user, server, call);
        let mut calls = self.0.lock().unwrap();
        if calls.contains_key(&key) {
            return Err(McpError::InvalidConfig("call id is already active".into()));
        }
        if calls.len() >= 1024 {
            return Err(McpError::PermissionDenied);
        }
        let token = CancellationToken::new();
        calls.insert(key.clone(), token.clone());
        Ok(ActiveCall {
            key,
            token,
            calls: self.clone(),
        })
    }

    pub fn cancel(&self, user: &str, server: &str, call: &str) -> Result<bool> {
        validate_call_id(call)?;
        let calls = self.0.lock().unwrap();
        if let Some(token) = calls.get(&Self::key(user, server, call)) {
            token.cancel();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn key(user: &str, server: &str, call: &str) -> CallKey {
        CallKey {
            user_id: user.into(),
            server_id: server.into(),
            call_id: call.into(),
        }
    }
}

pub(crate) struct ActiveCall {
    key: CallKey,
    pub token: CancellationToken,
    calls: ActiveCalls,
}

impl Drop for ActiveCall {
    fn drop(&mut self) {
        self.calls.0.lock().unwrap().remove(&self.key);
    }
}

pub(crate) fn validate_call_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(McpError::InvalidConfig(
            "call id must be 1-128 ASCII letters, digits, hyphens or underscores".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_scoped_and_duplicate_registration_cannot_replace_owner() {
        let calls = ActiveCalls::default();
        let active = calls.register("alice", "server-a", "call-1").unwrap();
        assert!(calls.register("alice", "server-a", "call-1").is_err());
        assert!(!calls.cancel("bob", "server-a", "call-1").unwrap());
        assert!(!calls.cancel("alice", "server-b", "call-1").unwrap());
        assert!(!active.token.is_cancelled());
        assert!(calls.cancel("alice", "server-a", "call-1").unwrap());
        assert!(active.token.is_cancelled());
        drop(active);
        assert!(!calls.cancel("alice", "server-a", "call-1").unwrap());
        let next = calls.register("alice", "server-a", "call-1").unwrap();
        assert!(!next.token.is_cancelled());
    }

    #[tokio::test]
    async fn abandoned_task_releases_registration_without_async_cleanup() {
        let calls = ActiveCalls::default();
        let (ready, started) = tokio::sync::oneshot::channel();
        let task_calls = calls.clone();
        let task = tokio::spawn(async move {
            let _active = task_calls.register("alice", "server", "abandoned").unwrap();
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        started.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!calls.cancel("alice", "server", "abandoned").unwrap());
    }
}
