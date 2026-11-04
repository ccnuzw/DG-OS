//! Window-owned transcription cancellation. Account leases are additionally checked by the command.
use std::{collections::HashMap, sync::Mutex};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct Transcriptions(Mutex<HashMap<(String, String), (String, CancellationToken)>>);
impl Transcriptions {
    pub fn begin(&self, user: &str, id: &str, owner: &str) -> Result<CancellationToken, dgos_types::DgosError> {
        let mut entries = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if entries.keys().any(|(account, _)| account == user) { return Err(dgos_types::DgosError::ProviderRequestRejected); }
        let token = CancellationToken::new();
        entries.insert((user.into(), id.into()), (owner.into(), token.clone()));
        Ok(token)
    }
    pub fn cancel(&self, user: &str, id: &str) -> bool {
        let entries = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((_, token)) = entries.get(&(user.into(), id.into())) { token.cancel(); true } else { false }
    }
    pub fn cancel_owner(&self, owner: &str) {
        for (_, (window, token)) in self.0.lock().unwrap_or_else(|p| p.into_inner()).iter() { if window == owner { token.cancel(); } }
    }
    pub fn finish(&self, user: &str, id: &str) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).remove(&(user.into(), id.into())).is_some_and(|(_, token)| token.is_cancelled())
    }
}
