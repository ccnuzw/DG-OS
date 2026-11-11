use super::*;
use crate::execution::LocalTools;

/// Combines request-scoped read-only web access with the existing host-local tool collection.
/// The shared executor still validates every call and counts all operations in one budget.
pub(super) struct ChatLocalTools<'a, 'b> {
    pub foundation: &'a mut crate::foundation::FoundationTools<'b>,
    pub web: Option<&'a mut crate::web::WebRuntime>,
}

#[async_trait::async_trait]
impl LocalTools for ChatLocalTools<'_, '_> {
    fn definitions(&self) -> Vec<dgos_types::ToolDefinition> {
        let mut definitions = self.foundation.definitions();
        if let Some(web) = &self.web { definitions.extend(web.definitions()); }
        definitions
    }
    fn model_dispatched(&mut self) { self.foundation.model_dispatched(); }
    async fn revalidate(&mut self) -> std::result::Result<(), String> {
        self.foundation.revalidate().await?;
        if let Some(web) = &self.web { web.revalidate().await?; }
        Ok(())
    }
    async fn dispatch(&mut self, name: &str, args: Value, remaining: usize) -> std::result::Result<Value, String> {
        self.revalidate().await?;
        let result = if matches!(name, "dgos_web_search" | "dgos_web_fetch") {
            self.web.as_mut().ok_or("unselected_tool")?.dispatch(name, args, remaining).await
        } else {
            self.foundation.dispatch(name, args, remaining).await
        };
        if result.is_ok() { self.revalidate().await?; }
        result
    }
}

pub(super) fn attach_activities(
    web: &mut crate::web::WebRuntime, repo: ChatRepository, user: String, task: String,
    assistant: String, active: Arc<Active>, sink: Arc<dyn ChatSink>,
) {
    web.set_activity_sink(activity_sink(repo, user, task, assistant, active, sink));
}

fn activity_sink(repo: ChatRepository, user: String, task: String, assistant: String,
    active: Arc<Active>, sink: Arc<dyn ChatSink>) -> crate::web::WebActivitySink {
    Arc::new(move |activity| {
        let (repo, user, task, assistant, active, sink) =
            (repo.clone(), user.clone(), task.clone(), assistant.clone(), active.clone(), sink.clone());
        Box::pin(async move {
            // Running and outcome remain separate immutable facts. A stable web_activity.id lets
            // the view reconcile their display without throwing away the historical dispatch.
            let status = activity.status.clone();
            let step = repo.record_step(&user, &task, "web", "processing_result", &status,
                true, false, Some(json!({"web_activity":activity}))).await;
            match step {
                Ok(step) => {
                    let sent = sink.send(ChatEvent::Activity { seq: 0, task_id: task,
                        assistant_message_id: assistant, step });
                    if !sent { active.stop("task_cancelled"); }
                    sent
                }
                Err(_) => { active.stop("persistence_failed"); false }
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_db::{migrations::Migrator, sea_orm_migration::MigratorTrait};

    #[tokio::test]
    async fn activity_callback_journals_running_and_sources_without_unknown_effects() {
        let db = dgos_db::sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let repo = ChatRepository::new(db);
        let session = repo.create_session("alice", &ChatTarget::default()).await.unwrap();
        repo.begin_generation(&BeginGeneration {
            user: "alice", session_id: &session.id, task_id: "web-task", user_message_id: "question",
            assistant_message_id: "answer", user_content: Some("selected request"), regenerate_of: None,
            provider_id: "provider", model_id: "model", provider_config_revision: None,
            endpoint_fingerprint: "synthetic", skills: &[], generation: &Default::default(), tools: &[], assets: &[],
        }).await.unwrap();
        let active = Arc::new(Active { user: "alice".into(), session_id: session.id, window: "main".into(),
            token: CancellationToken::new(), reason: Mutex::new(None), stream: Arc::new(Mutex::new(StreamState::default())) });
        let events = Arc::new(Mutex::new(Vec::new()));
        let output = events.clone();
        let sink: Arc<dyn ChatSink> = Arc::new(OrderedChatSink::new(Arc::new(move |event| {
            output.lock().unwrap().push(event); true
        }) as Arc<dyn ChatSink>, active.stream.clone()));
        let record = activity_sink(repo.clone(), "alice".into(), "web-task".into(), "answer".into(), active, sink);
        let mut activity = crate::web::WebActivity { id: "operation-1".into(), operation: "fetch".into(),
            status: "running".into(), query: None, url: Some("https://example.com/".into()), sources: vec![],
            error: None, truncated: false };
        assert!(record(activity.clone()).await);
        activity.status = "completed".into();
        activity.sources.push(crate::web::WebSource { id: "source-1".into(), url: "https://example.com/".into(),
            title: "Synthetic source fixture".into(), excerpt: "bounded excerpt".into(), retrieved_at: 1,
            kind: "web_page".into(), truncated: true });
        assert!(record(activity).await);
        let run = repo.get_run("alice", "web-task").await.unwrap();
        assert_eq!(run.steps.len(), 2);
        assert_eq!(run.steps[0].status, "running");
        assert_eq!(run.steps[1].status, "completed");
        assert!(run.steps.iter().all(|step| step.kind == "web" && !step.result_unknown));
        let receipt = run.steps[1].receipt.as_ref().unwrap();
        assert_eq!(receipt["web_activity"]["id"], "operation-1");
        assert_eq!(receipt["web_activity"]["sources"][0]["excerpt"], "bounded excerpt");
        assert_eq!(events.lock().unwrap().len(), 2);
        assert!(repo.get_run("bob", "web-task").await.is_err());
    }
}
