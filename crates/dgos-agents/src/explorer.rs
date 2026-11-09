//! Taskless explorer projections. No model, network, task creation or authority issuance.
use crate::AgentManager;
pub use dgos_db::development_repository::SessionMessageSummary as SessionSummary;

impl AgentManager {
    pub async fn read_development_session_summaries(&self, owner: &str) -> Result<Vec<SessionSummary>, String> {
        self.ready().await?;
        let mut summaries = self.inner.repository.session_message_summaries(owner).await?;
        for summary in &mut summaries {
            summary.preview = summary.preview.take().and_then(|text| {
                let safe = crate::executions::sanitize_text(&text);
                let mut preview = String::new();
                for character in safe.split_whitespace().enumerate().flat_map(|(index, word)| {
                    (index > 0).then_some(' ').into_iter().chain(word.chars())
                }).take(240) { preview.push(character); }
                if preview.is_empty() { None } else { Some(preview) }
            });
        }
        Ok(summaries)
    }
}

#[cfg(test)]
mod tests;
