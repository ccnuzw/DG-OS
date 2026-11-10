use super::*;
use crate::foundation::ResolvedContext;


pub(super) fn foundation_fingerprint(options: &dgos_types::AiFoundationOptions) -> Result<String> {
    Ok(format!("{:x}",Sha256::digest(serde_json::to_vec(options).map_err(|_|ChatError::Internal)?)))
}
fn foundation_error(_: String) -> ChatError { ChatError::ToolTurnInvalid }

pub(super) fn validate_chat_foundation(options: &dgos_types::AiFoundationOptions) -> Result<()> {
    // Chat's explicit summary/continuation commands retain their existing source and effect checks.
    if options.source_run_id.is_some() || options.operation != dgos_types::AiOperation::Generate {
        return Err(ChatError::InvalidMessageContent);
    }
    crate::foundation::validate_output_contract(&options.output).map_err(foundation_error)
}

pub(super) fn effective_skill_mode(mode: ChatSkillMode, options: &dgos_types::AiFoundationOptions) -> ChatSkillMode {
    if options.skill_mode == dgos_types::AiSkillMode::OnDemand { ChatSkillMode::OnDemand } else { mode }
}

pub(super) fn inline_context(contexts: &[ResolvedContext], model: &Model, output: &dgos_types::AiOutputContract) -> Result<crate::foundation::InlineContext> {
    let mut inline = crate::foundation::assemble_inline(contexts, model.capabilities.vision == dgos_types::Hint::Supported).map_err(foundation_error)?;
    if let dgos_types::AiOutputContract::JsonSchema { schema } = output {
        inline.text.push_str("\nRequested output: return one JSON value satisfying this schema. The host validates the final output after generation; this does not grant permissions.\n");
        inline.text.push_str(&schema.to_string());
    }
    Ok(inline)
}

pub(super) fn has_context_tools(contexts: &[ResolvedContext]) -> bool {
    contexts.iter().any(|context| context.reference.mode == dgos_types::AiContextMode::OnDemand)
}

pub(super) fn validate_budget(messages: &[WireMessage], generation: &dgos_types::GenerationParameters, reserve: usize, window: Option<i32>, has_tools: bool) -> Result<()> {
    let budget = crate::foundation::request_context_budget(messages, generation, reserve, window, has_tools).map_err(|_|ChatError::ContextTooLong)?;
    if budget.can_send { Ok(()) } else { Err(ChatError::ContextTooLong) }
}

impl ChatManager {
    pub(super) async fn check_foundation_replay(&self, user: &str, task: &str, options: &dgos_types::AiFoundationOptions) -> Result<()> {
        let run=self.repo.get_run(user,task).await?;
        let expected=foundation_fingerprint(options)?;
        let recorded=run.steps.iter().filter_map(|step|step.receipt.as_ref()).find_map(|receipt|receipt.get("foundation_fingerprint").and_then(Value::as_str));
        if recorded.map_or(options==&dgos_types::AiFoundationOptions::default(),|value|value==expected) {Ok(())} else {Err(ChatError::RequestIdConflict)}
    }

    pub(super) async fn revalidate_source_runs(&self, user: &str, session: &str, sources: &[String]) -> std::result::Result<(), String> {
        self.check_lease().map_err(|_|"session_invalid")?;
        let repo=dgos_db::AiContentRepository::new(self.db.clone());
        let scope=format!("chat:{session}");
        for source in sources {
            let run=self.repo.get_run(user,source).await.map_err(|_|"source_unavailable")?;
            if run.session_id!=session || run.status=="running" || run.audit_warning.is_some() || run.steps.iter().any(|step|step.kind=="tool"&&(step.result_unknown||step.status=="dispatched")) {return Err("source_unavailable".into())}
            for step in run.steps {
                let Some(mut receipt) = step.receipt else { continue };
                let Some(receipts) = receipt.get_mut("context_receipts") else { continue };
                let receipts: Vec<dgos_types::AiContextReceipt> = serde_json::from_value(receipts.take()).map_err(|_|"source_unavailable")?;
                for receipt in receipts {
                    let resource=repo.resource_metadata(user,&scope,&receipt.resource_id).await.map_err(|_|"source_unavailable")?.ok_or("source_unavailable")?;
                    if resource.sha256!=receipt.sha256 {return Err("source_unavailable".into())}
                }
            }
        }
        self.check_lease().map_err(|_|"session_invalid".into())
    }
    pub async fn import_ai_resource(&self, user: &str, session: &str, input: dgos_types::AiResourceImport) -> Result<dgos_types::AiResource> {
        self.check_lease()?;
        self.repo.get_session(user, session).await?;
        let bytes = crate::foundation::validate_resource(&input).map_err(foundation_error)?;
        self.check_lease()?;
        let resource = dgos_db::AiContentRepository::new(self.db.clone())
            .import_resource(user, &format!("chat:{session}"), &input.name, &input.mime, &bytes).await?;
        self.check_lease()?;
        Ok(resource)
    }

    pub async fn delete_ai_resource(&self, user: &str, session: &str, resource: &str) -> Result<bool> {
        self.check_lease()?;
        self.repo.get_session(user, session).await?;
        Ok(dgos_db::AiContentRepository::new(self.db.clone()).delete_resource(user, &format!("chat:{session}"), resource).await?)
    }

    pub(super) async fn resolve_contexts(&self, user: &str, session: &str, options: &dgos_types::AiFoundationOptions) -> Result<Vec<ResolvedContext>> {
        self.check_lease()?;
        self.repo.get_session(user, session).await?;
        validate_chat_foundation(options)?;
        if options.context_refs.len() > 16 { return Err(ChatError::ContextTooLong); }
        let repo = dgos_db::AiContentRepository::new(self.db.clone());
        let scope = format!("chat:{session}");
        let mut resolved = Vec::with_capacity(options.context_refs.len());
        for reference in &options.context_refs {
            if resolved.iter().any(|r: &ResolvedContext| r.reference.resource_id == reference.resource_id) {
                return Err(ChatError::ToolTurnInvalid);
            }
            let (resource, bytes) = repo.resource(user, &scope, &reference.resource_id).await?
                .ok_or(ChatError::ToolTurnInvalid)?;
            if resource.sha256 != reference.sha256 { return Err(ChatError::ToolTurnInvalid); }
            resolved.push(ResolvedContext { resource, bytes, reference: reference.clone() });
        }
        self.check_lease()?;
        Ok(resolved)
    }

    pub(super) async fn revalidate_contexts(&self, user: &str, session: &str, contexts: &[ResolvedContext]) -> std::result::Result<(), String> {
        self.check_lease().map_err(|_| "session_invalid")?;
        self.repo.get_session(user, session).await.map_err(|_| "session_invalid")?;
        let repo = dgos_db::AiContentRepository::new(self.db.clone());
        let scope = format!("chat:{session}");
        for context in contexts {
            let resource = repo.resource_metadata(user, &scope, &context.resource.id).await
                .map_err(|_| "context_unavailable")?.ok_or("context_unavailable")?;
            if resource.sha256 != context.reference.sha256 { return Err("context_unavailable".into()); }
        }
        self.check_lease().map_err(|_| "session_invalid".into())
    }

    pub(super) fn local_schema_bytes(runtime: Option<crate::skill_runtime::SkillRuntime>, contexts: &[ResolvedContext])
        -> Result<(Option<crate::skill_runtime::SkillRuntime>, usize)> {
        let definitions=crate::foundation::local_definitions(runtime.as_ref(),contexts);
        let bytes=crate::foundation::tool_schema_bytes(&definitions).map_err(foundation_error)?;
        Ok((runtime,bytes))
    }

    pub(super) async fn record_inline_context(&self, user: &str, task: &str, receipts: &[dgos_types::AiContextReceipt]) -> Result<()> {
        if !receipts.is_empty() {
            self.repo.record_step(user, task, "context", "processing_result", "supplied", false, false,
                Some(json!({"context_receipts": receipts}))).await?;
        }
        Ok(())
    }
}
