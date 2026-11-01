//! The remote chat's only way into the host (technical design §4): the FR-006
//! `ChatManager` methods behind [`ChatBackend`], plus the read-only target list.
//!
//! This is the one file of the crate that touches the Provider and Skills
//! managers, and only through their list/read methods. No account, module,
//! MCP or management call exists here; there is no tool selection (`tools`
//! stays empty) and no `accept_destination` (D099 item 8).

use async_trait::async_trait;
use dgos_db::ChatSession;
use dgos_providers::ProviderManager;
use dgos_skills::SkillsManager;
use dgos_workbench::{
    ChatError, ChatEvent, ChatManager, ChatMessagePage, ChatSink, ChatTargetInput,
    CreateChatSessionInput, ListChatMessagesInput, RegenerateChatInput, SendChatInput, Sent,
};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LanModel {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LanProvider {
    pub id: String,
    pub name: String,
    pub models: Vec<LanModel>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LanSkill {
    pub id: String,
    pub name: String,
    pub version: String,
}

/// Interface contract §14.6: no endpoint, `credential_ref`, configuration,
/// usage or connection state.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LanTargets {
    pub providers: Vec<LanProvider>,
    pub skills: Vec<LanSkill>,
}

/// Remote chat operations, always for the session's bound user and the
/// window `lan:<session_id>`.
#[async_trait]
pub trait ChatBackend: Send + Sync + 'static {
    async fn targets(&self, user: &str) -> Result<LanTargets, ChatError>;
    async fn create_session(
        &self,
        user: &str,
        target: ChatTargetInput,
    ) -> Result<ChatSession, ChatError>;
    async fn get_session(&self, user: &str, session_id: &str) -> Result<ChatSession, ChatError>;
    async fn list_messages(
        &self,
        user: &str,
        input: ListChatMessagesInput,
    ) -> Result<ChatMessagePage, ChatError>;
    async fn session_of_message(
        &self,
        user: &str,
        message_id: &str,
    ) -> Result<Option<String>, ChatError>;
    async fn session_of_task(&self, user: &str, task_id: &str)
        -> Result<Option<String>, ChatError>;
    async fn send(&self, user: &str, window: &str, input: SendChatInput)
        -> Result<Sent, ChatError>;
    async fn regenerate(
        &self,
        user: &str,
        window: &str,
        input: RegenerateChatInput,
    ) -> Result<Sent, ChatError>;
    /// Run a started generation in the background; `done` runs after its terminal event.
    fn drive(
        &self,
        job: dgos_workbench::GenerationJob,
        sink: Arc<dyn ChatSink>,
        done: Box<dyn FnOnce() + Send>,
    );
    fn cancel(&self, user: &str, task_id: &str) -> bool;
    fn cancel_window(&self, window: &str) -> usize;
}

/// Where the host keeps its managers. The Tauri host reads them from managed
/// state; tests own them. The `ChatManager` must be the host's instance (it
/// carries the per-user generation registry shared with the desktop).
pub trait ChatHost: Send + Sync + 'static {
    fn chat(&self) -> ChatManager;
    fn providers(&self) -> &ProviderManager;
    fn skills(&self) -> Arc<RwLock<SkillsManager>>;
}

pub struct ManagedChat<H: ChatHost>(pub Arc<H>);

impl<H: ChatHost> ManagedChat<H> {
    pub fn new(host: H) -> Self {
        Self(Arc::new(host))
    }
}

fn text_models(models: Vec<dgos_types::Model>) -> Vec<LanModel> {
    models
        .into_iter()
        .filter(|model| matches!(model.model_type.as_str(), "text-generation" | "chat"))
        .map(|model| LanModel {
            name: if model.model_name.is_empty() {
                model.model_id.clone()
            } else {
                model.model_name
            },
            id: model.model_id,
        })
        .collect()
}

#[async_trait]
impl<H: ChatHost> ChatBackend for ManagedChat<H> {
    async fn targets(&self, user: &str) -> Result<LanTargets, ChatError> {
        let providers = self.0.providers();
        let mut out = Vec::new();
        for provider in providers.list_providers(user).await? {
            if !provider.enabled {
                continue;
            }
            let models = text_models(providers.list_models(&provider.id, user).await?);
            out.push(LanProvider {
                id: provider.id,
                name: provider.name,
                models,
            });
        }
        let skills = self.0.skills();
        let skills = skills.read().await;
        let skills = skills
            .list_installed_skills(user)
            .await
            .map_err(|_| ChatError::Internal)?
            .into_iter()
            .filter(|skill| skill.enabled && skill.call_type == dgos_skills::CallType::Content)
            .map(|skill| LanSkill {
                id: skill.skill_id,
                name: skill.name,
                version: skill.version,
            })
            .collect();
        Ok(LanTargets {
            providers: out,
            skills,
        })
    }

    async fn create_session(
        &self,
        user: &str,
        target: ChatTargetInput,
    ) -> Result<ChatSession, ChatError> {
        let skills = self.0.skills();
        let skills = skills.read().await;
        self.0
            .chat()
            .create_session(
                self.0.providers(),
                &skills,
                user,
                CreateChatSessionInput {
                    target: Some(target),
                },
            )
            .await
    }

    async fn get_session(&self, user: &str, session_id: &str) -> Result<ChatSession, ChatError> {
        self.0.chat().get_session(user, session_id).await
    }

    async fn list_messages(
        &self,
        user: &str,
        input: ListChatMessagesInput,
    ) -> Result<ChatMessagePage, ChatError> {
        self.0.chat().list_messages(user, input).await
    }

    async fn session_of_message(
        &self,
        user: &str,
        message_id: &str,
    ) -> Result<Option<String>, ChatError> {
        self.0.chat().session_of_message(user, message_id).await
    }

    async fn session_of_task(
        &self,
        user: &str,
        task_id: &str,
    ) -> Result<Option<String>, ChatError> {
        self.0.chat().session_of_task(user, task_id).await
    }

    async fn send(
        &self,
        user: &str,
        window: &str,
        input: SendChatInput,
    ) -> Result<Sent, ChatError> {
        let skills = self.0.skills();
        let skills = skills.read().await;
        self.0
            .chat()
            .send(self.0.providers(), &skills, user, window, input)
            .await
    }

    async fn regenerate(
        &self,
        user: &str,
        window: &str,
        input: RegenerateChatInput,
    ) -> Result<Sent, ChatError> {
        let skills = self.0.skills();
        let skills = skills.read().await;
        self.0
            .chat()
            .regenerate(self.0.providers(), &skills, user, window, input)
            .await
    }

    fn drive(
        &self,
        job: dgos_workbench::GenerationJob,
        sink: Arc<dyn ChatSink>,
        done: Box<dyn FnOnce() + Send>,
    ) {
        let host = self.0.clone();
        tokio::spawn(async move {
            host.chat()
                .run_generation(host.providers(), job, sink.as_ref())
                .await;
            done();
        });
    }

    fn cancel(&self, user: &str, task_id: &str) -> bool {
        self.0.chat().cancel(user, task_id)
    }

    fn cancel_window(&self, window: &str) -> usize {
        self.0.chat().cancel_window(window)
    }
}

/// Kinds the remote stream never carries: the first whitelist has no MCP
/// tool turns, so tool proposals are dropped.
pub fn forwardable(event: &ChatEvent) -> bool {
    !matches!(event, ChatEvent::ToolProposal { .. })
}
