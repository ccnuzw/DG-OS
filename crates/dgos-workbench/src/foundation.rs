//! Shared, host-only assistant foundation. None of these helpers authorize a user or scope.
use async_trait::async_trait;
use base64::Engine as _;
use dgos_skills::SkillsManager;
use dgos_types::{AiContextBudget, AiContextMode, AiContextReceipt, AiContextRef, AiOutputContract, AiResource, AiResourceImport, ImageInput, ToolDefinition, ToolFunction};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{borrow::Cow, collections::HashSet, sync::{Arc, Mutex}};
use tokio::sync::RwLock;
use crate::{execution::LocalTools, skill_runtime::SkillRuntime};

pub const MAX_CONTEXT_BYTES: usize = 24 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_SCHEMA_BYTES: usize = 16 * 1024;
pub const MAX_CONTEXT_REFS: usize = 16;
pub const RESOURCE_LIST_TOOL: &str = "dgos_resource_list";
pub const RESOURCE_READ_TOOL: &str = "dgos_resource_read";

/// Conservative estimate: one UTF-8 byte per token, 1024 safety tokens and 2048 per image.
pub fn text_ceiling(window: Option<i32>, output_tokens: u32, image_count: usize) -> Option<usize> {
    match window {
        Some(n) if n <= 0 => None,
        Some(n) => (n as usize).checked_sub(1024usize.saturating_add(output_tokens as usize).saturating_add(image_count.saturating_mul(2048))).map(|v| v.min(MAX_CONTEXT_BYTES)),
        None => Some(MAX_CONTEXT_BYTES),
    }
}

/// `text_bytes` includes message/serialization overhead; schemas are accounted separately.
pub fn context_budget(text_bytes: usize, image_count: usize, tool_schema_bytes: usize, window: Option<i32>, output_tokens: u32) -> AiContextBudget {
    let total = text_bytes.saturating_add(tool_schema_bytes);
    let ceiling = text_ceiling(window, output_tokens, image_count);
    let can_send = ceiling.is_some_and(|limit| total <= limit);
    AiContextBudget {
        text_bytes, image_reserve_tokens: image_count.saturating_mul(2048), tool_schema_bytes,
        host_text_limit_bytes: MAX_CONTEXT_BYTES, model_window_tokens: window, reserved_output_tokens: output_tokens,
        estimated_input_tokens: total.saturating_add(image_count.saturating_mul(2048)), can_send,
        rejection: (!can_send).then(|| "context_budget_exceeded".into()),
    }
}

/// Exact tool-array contribution, reserving the existing result reader for external tools.
/// Empty array brackets belong to the message envelope rather than schema bytes.
pub fn tool_schema_bytes(definitions: &[ToolDefinition]) -> Result<usize, String> {
    let mut bytes = serde_json::to_vec(definitions).map_err(|_| "context_encoding_failed")?.len().saturating_sub(2);
    let has_external = definitions.iter().any(|definition| definition.function.name.starts_with("dgos_tool_") && definition.function.name != "dgos_tool_result_read");
    if has_external && !definitions.iter().any(|definition| definition.function.name == "dgos_tool_result_read") {
        bytes = bytes.saturating_add(1).saturating_add(serde_json::to_vec(&crate::execution::result_read_definition()).map_err(|_| "context_encoding_failed")?.len());
    }
    Ok(bytes)
}

pub async fn selected_tool_schema_bytes(mcp: &dgos_mcp::McpManager, user: &str, selections: &[dgos_types::AiToolSelection], local: &[ToolDefinition]) -> Result<usize, String> {
    if selections.len() > 8 { return Err("tool_call_limit".into()); }
    let mut definitions = if selections.is_empty() { vec![] } else {
        crate::tools::resolve_selected_tools(mcp, user, selections.to_vec()).await?.into_iter().map(|tool| tool.definition).collect()
    };
    definitions.extend_from_slice(local);
    let mut names = HashSet::new();
    if definitions.iter().any(|definition| !names.insert(&definition.function.name)) { return Err("duplicate_tool_definition".into()); }
    tool_schema_bytes(&definitions)
}

/// Same envelope used by execution; inspect must not just sum tool-message text.
pub fn request_context_budget(messages: &[dgos_types::ChatMessage], generation: &dgos_types::GenerationParameters, tool_schema_bytes: usize, window: Option<i32>, has_tools: bool) -> Result<AiContextBudget, String> {
    let images = messages.iter().map(|message| message.images.len()).sum();
    let bytes = if has_tools {
        let request = dgos_types::ToolChatRequest {
            messages: messages.iter().map(|message| dgos_types::ToolMessage { role: message.role.clone(), content: Some(message.content.clone()),
                tool_call_id: None, tool_calls: vec![], reasoning: None, images: vec![] }).collect(),
            tools: vec![], max_tokens: generation.output_tokens(), temperature: generation.temperature, reasoning: generation.reasoning.clone(),
        };
        serde_json::to_vec(&request).map_err(|_| "context_encoding_failed")?.len()
    } else { messages.iter().map(|message| message.content.len()).sum() };
    Ok(context_budget(bytes, images, tool_schema_bytes, window, generation.output_tokens()))
}

fn bounded_json(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), String> {
    *nodes += 1;
    if depth > 32 || *nodes > 2048 { return Err("output_schema_limit".into()); }
    match value {
        Value::Object(values) => for value in values.values() { bounded_json(value, depth + 1, nodes)?; },
        Value::Array(values) => for value in values { bounded_json(value, depth + 1, nodes)?; },
        _ => {}
    }
    Ok(())
}

// Traverse resolved references too, so cyclic schemas or expansion bombs never reach the validator.
fn bounded_schema(root: &Value, value: &Value, depth: usize, nodes: &mut usize, references: &mut HashSet<String>) -> Result<(), String> {
    *nodes += 1;
    if depth > 32 || *nodes > 2048 { return Err("output_schema_limit".into()); }
    match value {
        Value::Object(object) => {
            for key in ["$dynamicRef", "$recursiveRef", "$dynamicAnchor", "$recursiveAnchor"] {
                if object.contains_key(key) { return Err("unsupported_output_schema_reference".into()); }
            }
            if object.get("$schema").is_some_and(|dialect| !matches!(dialect.as_str(),
                Some("http://json-schema.org/draft-04/schema#" | "http://json-schema.org/draft-06/schema#" | "http://json-schema.org/draft-07/schema#" |
                    "https://json-schema.org/draft/2019-09/schema" | "https://json-schema.org/draft/2020-12/schema"))) {
                return Err("unsupported_output_schema_dialect".into());
            }
            if object.get("$id").is_some_and(|id| id.as_str().is_none_or(|id| !id.starts_with('#'))) {
                return Err("external_output_schema_reference".into());
            }
            if let Some(reference) = object.get("$ref") {
                let reference = reference.as_str().ok_or("invalid_output_schema")?;
                if !reference.starts_with("#/") { return Err("unsupported_output_schema_reference".into()); }
                if !references.insert(reference.into()) { return Err("recursive_output_schema".into()); }
                let target = root.pointer(&reference[1..]).ok_or("invalid_output_schema_reference")?;
                bounded_schema(root, target, depth + 1, nodes, references)?;
                references.remove(reference);
            }
            if object.get("pattern").and_then(Value::as_str).is_some_and(|pattern| pattern.len() > 512) {
                return Err("output_schema_limit".into());
            }
            for keyword in ["$defs", "definitions", "properties", "patternProperties", "dependentSchemas"] {
                if let Some(Value::Object(schemas)) = object.get(keyword) {
                    for (name, schema) in schemas {
                        if keyword == "patternProperties" && name.len() > 512 { return Err("output_schema_limit".into()); }
                        bounded_schema(root, schema, depth + 1, nodes, references)?;
                    }
                }
            }
            for keyword in ["additionalProperties", "unevaluatedProperties", "propertyNames", "contains", "additionalItems", "unevaluatedItems", "not", "if", "then", "else", "contentSchema"] {
                if let Some(schema) = object.get(keyword) { bounded_schema(root, schema, depth + 1, nodes, references)?; }
            }
            for keyword in ["allOf", "anyOf", "oneOf", "prefixItems"] {
                if let Some(Value::Array(schemas)) = object.get(keyword) {
                    for schema in schemas { bounded_schema(root, schema, depth + 1, nodes, references)?; }
                }
            }
            if let Some(items) = object.get("items") {
                match items {
                    Value::Array(schemas) => for schema in schemas { bounded_schema(root, schema, depth + 1, nodes, references)?; },
                    schema => bounded_schema(root, schema, depth + 1, nodes, references)?,
                }
            }
            if let Some(Value::Object(dependencies)) = object.get("dependencies") {
                for schema in dependencies.values().filter(|schema| !schema.is_array()) { bounded_schema(root, schema, depth + 1, nodes, references)?; }
            }
        }
        _ => {}
    }
    Ok(())
}

fn compile_output_contract(contract: &AiOutputContract) -> Result<Option<jsonschema::Validator>, String> {
    if let AiOutputContract::JsonSchema { schema } = contract {
        if !schema.is_object() && !schema.is_boolean() { return Err("invalid_output_schema".into()); }
        if serde_json::to_vec(schema).map_err(|_| "invalid_output_schema")?.len() > MAX_SCHEMA_BYTES { return Err("output_schema_limit".into()); }
        bounded_json(schema, 0, &mut 0)?;
        bounded_schema(schema, schema, 0, &mut 0, &mut HashSet::new())?;
        return jsonschema::validator_for(schema).map(Some).map_err(|_| "invalid_output_schema".into());
    }
    Ok(None)
}

pub fn validate_output_contract(contract: &AiOutputContract) -> Result<(), String> {
    compile_output_contract(contract).map(|_| ())
}

/// Post-generation validation only: never calls a provider or silently repairs paid output.
pub fn validate_output(contract: &AiOutputContract, output: &str) -> Result<(), String> {
    if output.len() > MAX_OUTPUT_BYTES { return Err("output_limit".into()); }
    if let Some(validator) = compile_output_contract(contract)? {
        let instance: Value = serde_json::from_str(output).map_err(|_| "output_contract_failed")?;
        if !validator.is_valid(&instance) {
            return Err("output_contract_failed".into());
        }
    }
    Ok(())
}

/// Same UTF-8 and image-signature checks as controlled chat attachments; no parser or execution.
pub fn validate_resource(input: &AiResourceImport) -> Result<Vec<u8>, String> {
    if input.name.trim().is_empty() || input.name.chars().count() > 255 || input.name.chars().any(char::is_control)
        || input.name.contains(['/', '\\']) || matches!(input.name.as_str(), "." | "..") {
        return Err("invalid_resource_name".into());
    }
    let limit: usize = match input.mime.as_str() {
        "text/plain" | "text/markdown" => 1024 * 1024,
        "image/png" | "image/jpeg" | "image/webp" => 8 * 1024 * 1024,
        _ => return Err("invalid_resource_mime".into()),
    };
    if input.base64.len() > limit.div_ceil(3) * 4 { return Err("resource_limit".into()); }
    let bytes = base64::engine::general_purpose::STANDARD.decode(&input.base64).map_err(|_| "invalid_resource_encoding")?;
    if bytes.len() > limit { return Err("resource_limit".into()); }
    validate_resource_bytes(&input.mime, &bytes)?;
    Ok(bytes)
}

fn validate_resource_bytes(mime: &str, bytes: &[u8]) -> Result<(), String> {
    let valid = match mime {
        "text/plain" | "text/markdown" => bytes.len() <= 1024 * 1024 && !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok(),
        "image/png" => bytes.len() <= 8 * 1024 * 1024 && bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.len() <= 8 * 1024 * 1024 && bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/webp" => bytes.len() <= 8 * 1024 * 1024 && bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        _ => false,
    };
    if valid { Ok(()) } else { Err("invalid_resource_content".into()) }
}

/// Construct only from an owned immutable resource resolved by the host repository.
#[derive(Clone)]
pub struct ResolvedContext { pub resource: AiResource, pub bytes: Vec<u8>, pub reference: AiContextRef }

fn validate_contexts(contexts: &[ResolvedContext]) -> Result<(), String> {
    if contexts.len() > MAX_CONTEXT_REFS { return Err("context_reference_limit".into()); }
    let mut ids = HashSet::new();
    let mut total = 0usize;
    for context in contexts {
        let resource = &context.resource;
        let reference = &context.reference;
        if !ids.insert(&resource.id) || resource.id != reference.resource_id || resource.sha256 != reference.sha256
            || resource.bytes_len != context.bytes.len() as u64 || resource.expires_at <= chrono::Utc::now().timestamp()
            || reference.max_bytes == 0 || reference.max_bytes as usize > 8 * 1024 * 1024 {
            return Err("context_reference_changed".into());
        }
        if format!("{:x}", Sha256::digest(&context.bytes)) != resource.sha256 { return Err("context_reference_changed".into()); }
        validate_resource_bytes(&resource.mime, &context.bytes)?;
        total = total.saturating_add(context.bytes.len());
        if total > 24 * 1024 * 1024 { return Err("context_reference_limit".into()); }
        if resource.mime.starts_with("text/") { text_range(context, reference.start, reference.max_bytes)?; }
        else if reference.start != 0 { return Err("invalid_context_range".into()); }
    }
    Ok(())
}

fn text_range(context: &ResolvedContext, start: u32, max_bytes: u32) -> Result<(&str, AiContextReceipt), String> {
    let text = std::str::from_utf8(&context.bytes).map_err(|_| "invalid_resource_content")?;
    let start = start as usize;
    if max_bytes == 0 || start > text.len() || !text.is_char_boundary(start) { return Err("invalid_context_range".into()); }
    let mut end = start.saturating_add(max_bytes as usize).min(text.len());
    while !text.is_char_boundary(end) { end -= 1; }
    let receipt = AiContextReceipt { resource_id: context.resource.id.clone(), sha256: context.resource.sha256.clone(), start: start as u32,
        bytes: (end - start) as u32, total_bytes: context.resource.bytes_len };
    Ok((&text[start..end], receipt))
}

#[derive(Default)]
pub struct InlineContext { pub text: String, pub images: Vec<ImageInput>, pub receipts: Vec<AiContextReceipt> }

/// Images are supplied whole as image parts (start must be zero); max_bytes selects text only.
/// Returned receipts cover assembly, and must only be persisted if that assembly is dispatched.
pub fn assemble_inline(contexts: &[ResolvedContext], supports_vision: bool) -> Result<InlineContext, String> {
    validate_contexts(contexts)?;
    let mut result = InlineContext::default();
    for context in contexts.iter().filter(|context| context.reference.mode == AiContextMode::Inline) {
        if context.resource.mime.starts_with("text/") {
            let (text, receipt) = text_range(context, context.reference.start, context.reference.max_bytes)?;
            let header = format!("\n\n[Untrusted source id={} sha256={} start={} bytes={}]\n", context.resource.id, context.resource.sha256, receipt.start, receipt.bytes);
            if result.text.len().saturating_add(header.len()).saturating_add(text.len()) > MAX_CONTEXT_BYTES { return Err("context_budget_exceeded".into()); }
            result.text.push_str(&header);
            result.text.push_str(text);
            result.receipts.push(receipt);
        } else {
            if !supports_vision { return Err("vision_not_supported".into()); }
            result.images.push(ImageInput { mime: context.resource.mime.clone(), data: context.bytes.clone() });
            result.receipts.push(AiContextReceipt { resource_id: context.resource.id.clone(), sha256: context.resource.sha256.clone(), start: 0,
                bytes: context.bytes.len() as u32, total_bytes: context.resource.bytes_len });
        }
    }
    Ok(result)
}

/// Implement in the host: check current account, lease, configuration, resource ownership/hash/expiry.
/// The fixed resource slice is borrowed, never copied; implementation can query metadata only.
#[async_trait]
pub trait FoundationGuard: Send + Sync {
    async fn revalidate(&self, contexts: &[ResolvedContext]) -> Result<(), String>;
}

#[derive(Clone, Debug)]
pub struct FoundationReceipts {
    pub context_receipts: Vec<AiContextReceipt>,
    pub pending_context_receipts: Vec<AiContextReceipt>,
    pub skill_receipts: Value,
    pub pending_skill_receipts: Value,
}
impl Default for FoundationReceipts {
    fn default() -> Self { Self { context_receipts: vec![], pending_context_receipts: vec![], skill_receipts: json!([]), pending_skill_receipts: json!([]) } }
}

enum SkillAccess<'a> { Shared(Arc<RwLock<SkillsManager>>), Borrowed(&'a SkillsManager) }

pub struct FoundationTools<'a> {
    skills: SkillAccess<'a>, runtime: Option<SkillRuntime>, contexts: Cow<'a, [ResolvedContext]>, guard: &'a dyn FoundationGuard,
    receipts: Arc<Mutex<FoundationReceipts>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceRead { resource_id: String, start: u32, max_bytes: u32 }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

impl<'a> FoundationTools<'a> {
    pub fn new(skills: Arc<RwLock<SkillsManager>>, runtime: Option<SkillRuntime>, contexts: Vec<ResolvedContext>, guard: &'a dyn FoundationGuard) -> Result<Self, String> {
        Self::create(SkillAccess::Shared(skills), runtime, Cow::Owned(contexts), guard)
    }
    pub fn new_borrowed(skills: &'a SkillsManager, runtime: Option<SkillRuntime>, contexts: Vec<ResolvedContext>, guard: &'a dyn FoundationGuard) -> Result<Self, String> {
        Self::create(SkillAccess::Borrowed(skills), runtime, Cow::Owned(contexts), guard)
    }
    pub fn new_borrowed_contexts(skills: &'a SkillsManager, runtime: Option<SkillRuntime>, contexts: &'a [ResolvedContext], guard: &'a dyn FoundationGuard) -> Result<Self, String> {
        Self::create(SkillAccess::Borrowed(skills), runtime, Cow::Borrowed(contexts), guard)
    }
    pub fn new_shared_contexts(skills: Arc<RwLock<SkillsManager>>, runtime: Option<SkillRuntime>, contexts: &'a [ResolvedContext], guard: &'a dyn FoundationGuard) -> Result<Self, String> {
        Self::create(SkillAccess::Shared(skills), runtime, Cow::Borrowed(contexts), guard)
    }
    fn create(skills: SkillAccess<'a>, runtime: Option<SkillRuntime>, contexts: Cow<'a, [ResolvedContext]>, guard: &'a dyn FoundationGuard) -> Result<Self, String> {
        validate_contexts(&contexts)?;
        let skill_receipts = runtime.as_ref().map(|runtime| serde_json::to_value(runtime.receipts())).transpose().map_err(|_| "receipt_encoding_failed")?.unwrap_or_else(|| json!([]));
        Ok(Self { skills, runtime, contexts, guard, receipts: Arc::new(Mutex::new(FoundationReceipts { skill_receipts, ..Default::default() })) })
    }
    pub fn receipts(&self) -> Arc<Mutex<FoundationReceipts>> { self.receipts.clone() }
    pub fn context_receipts(&self) -> Vec<AiContextReceipt> { self.receipts.lock().unwrap_or_else(|e| e.into_inner()).context_receipts.clone() }
    pub fn skill_receipts(&self) -> Value { self.receipts.lock().unwrap_or_else(|e| e.into_inner()).skill_receipts.clone() }
    pub fn into_runtime(self) -> Option<SkillRuntime> { self.runtime }
    pub async fn inline(&mut self, supports_vision: bool) -> Result<InlineContext, String> {
        self.revalidate().await?;
        let inline = assemble_inline(&self.contexts, supports_vision)?;
        self.revalidate().await?;
        self.receipts.lock().unwrap_or_else(|e| e.into_inner()).pending_context_receipts.extend(inline.receipts.iter().cloned());
        Ok(inline)
    }
    async fn validate_live(&self) -> Result<(), String> {
        // Immutable bytes/hash were checked once at construction. Never rehash megabytes per step.
        if self.contexts.iter().any(|context| context.resource.expires_at <= chrono::Utc::now().timestamp()) { return Err("context_reference_expired".into()); }
        self.guard.revalidate(&self.contexts).await?;
        if let Some(runtime) = &self.runtime {
            match &self.skills {
                SkillAccess::Shared(skills) => runtime.revalidate(&*skills.read().await).await?,
                SkillAccess::Borrowed(skills) => runtime.revalidate(skills).await?,
            }
        }
        Ok(())
    }
    fn resources(&self) -> impl Iterator<Item = &ResolvedContext> { self.contexts.iter().filter(|context| context.reference.mode == AiContextMode::OnDemand) }
    fn resource_result(&self, name: &str, args: Value, remaining: usize) -> Result<(Value, Option<AiContextReceipt>), String> {
        match name {
            RESOURCE_LIST_TOOL => {
                serde_json::from_value::<Empty>(args).map_err(|_| "invalid_resource_arguments")?;
                let value = Value::Array(self.resources().map(|context| json!({"resource":context.resource,"start":context.reference.start,"max_bytes":context.reference.max_bytes,"text_readable":context.resource.mime.starts_with("text/")})).collect());
                if value.to_string().len() > remaining { return Err("context_budget_exceeded".into()); }
                Ok((value, None))
            }
            RESOURCE_READ_TOOL => {
                let args: ResourceRead = serde_json::from_value(args).map_err(|_| "invalid_resource_arguments")?;
                let context = self.resources().find(|context| context.resource.id == args.resource_id).ok_or("unselected_resource")?;
                if !context.resource.mime.starts_with("text/") { return Err("image_requires_inline_vision".into()); }
                let allowed_end = (context.reference.start as usize).saturating_add(context.reference.max_bytes as usize).min(context.bytes.len());
                if args.start < context.reference.start || args.start as usize > allowed_end || args.max_bytes == 0 || args.max_bytes as usize > MAX_CONTEXT_BYTES { return Err("invalid_context_range".into()); }
                let maximum = (args.max_bytes as usize).min(allowed_end.saturating_sub(args.start as usize));
                if maximum == 0 { return Err("invalid_context_range".into()); }
                // Count the outer ToolMessage content string too: JSON escaping is applied twice.
                let (mut low, mut high) = (1usize, maximum.min(remaining));
                let mut best = None;
                while low <= high {
                    let limit = low + (high - low) / 2;
                    let (text, receipt) = text_range(context, args.start, limit as u32)?;
                    let value = json!({"content":text,"receipt":receipt,"selected_end":allowed_end,"untrusted":true});
                    if tool_result_content_bytes(&value)? <= remaining {
                        if receipt.bytes > 0 { best = Some((value, Some(receipt))); }
                        low = limit + 1;
                    } else { high = limit - 1; }
                }
                best.ok_or_else(|| "context_budget_exceeded".into())
            }
            _ => Err("unselected_tool".into()),
        }
    }
}

/// Serialized contribution of a tool result inside ToolMessage.content, including outer quotes.
pub fn tool_result_content_bytes(value: &Value) -> Result<usize, String> {
    serialized_tool_result_content_bytes(value)
}

pub(crate) fn serialized_tool_result_content_bytes<T: serde::Serialize + ?Sized>(value: &T) -> Result<usize, String> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.0 += bytes.len(); Ok(bytes.len()) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let content = serde_json::to_string(value).map_err(|_| "context_encoding_failed")?;
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, &content).map_err(|_| "context_encoding_failed")?;
    Ok(counter.0)
}

pub fn local_definitions(runtime: Option<&SkillRuntime>, contexts: &[ResolvedContext]) -> Vec<ToolDefinition> {
    let mut definitions = runtime.map(SkillRuntime::tools).unwrap_or_default();
    if contexts.iter().any(|context| context.reference.mode == AiContextMode::OnDemand) {
        let definition = |name: &str, description: &str, properties: Value, required: Value| ToolDefinition { kind: "function".into(), function: ToolFunction {
            name: name.into(), description: description.into(), parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}) } };
        definitions.push(definition(RESOURCE_LIST_TOOL, "List only host-selected resource metadata. Does not read content or grant authority.", json!({}), json!([])));
        definitions.push(definition(RESOURCE_READ_TOOL, "Read a bounded UTF-8 range within an explicitly selected immutable text source. Returns exact byte coverage, not understanding. Images require inline vision input.",
            json!({"resource_id":{"type":"string","minLength":1},"start":{"type":"integer","minimum":0,"maximum":u32::MAX},"max_bytes":{"type":"integer","minimum":1,"maximum":MAX_CONTEXT_BYTES}}), json!(["resource_id","start","max_bytes"])));
    }
    definitions
}

#[async_trait]
impl LocalTools for FoundationTools<'_> {
    fn definitions(&self) -> Vec<ToolDefinition> { local_definitions(self.runtime.as_ref(), &self.contexts) }
    fn model_dispatched(&mut self) {
        let mut receipts = self.receipts.lock().unwrap_or_else(|e| e.into_inner());
        let pending = std::mem::take(&mut receipts.pending_context_receipts);
        receipts.context_receipts.extend(pending);
        if receipts.pending_skill_receipts.as_array().is_some_and(|receipts| !receipts.is_empty()) {
            receipts.skill_receipts = std::mem::replace(&mut receipts.pending_skill_receipts, json!([]));
        }
    }
    async fn revalidate(&mut self) -> Result<(), String> { self.validate_live().await }
    async fn dispatch(&mut self, name: &str, args: Value, remaining_context_bytes: usize) -> Result<Value, String> {
        self.validate_live().await?;
        let (value, receipt) = if SkillRuntime::is_tool(name) {
            let runtime = self.runtime.as_mut().ok_or("unselected_tool")?;
            let value = match &self.skills {
                SkillAccess::Shared(skills) => runtime.dispatch(&*skills.read().await, name, args, remaining_context_bytes).await?,
                SkillAccess::Borrowed(skills) => runtime.dispatch(skills, name, args, remaining_context_bytes).await?,
            };
            (value, None)
        } else { self.resource_result(name, args, remaining_context_bytes)? };
        self.validate_live().await?;
        if tool_result_content_bytes(&value)? > remaining_context_bytes { return Err("context_budget_exceeded".into()); }
        let mut receipts = self.receipts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(receipt) = receipt { receipts.pending_context_receipts.push(receipt); }
        if let Some(runtime) = &self.runtime { receipts.pending_skill_receipts = serde_json::to_value(runtime.receipts()).map_err(|_| "receipt_encoding_failed")?; }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn source(text: &str, mode: AiContextMode, start: u32, max_bytes: u32) -> ResolvedContext {
        let sha256 = format!("{:x}", Sha256::digest(text.as_bytes()));
        ResolvedContext {
            resource: AiResource { id: "selected".into(), name: "source.txt".into(), mime: "text/plain".into(), bytes_len: text.len() as u64,
                sha256: sha256.clone(), expires_at: chrono::Utc::now().timestamp() + 3600 },
            bytes: text.as_bytes().to_vec(), reference: AiContextRef { resource_id: "selected".into(), sha256, start, max_bytes, mode },
        }
    }

    struct Guard { checks: AtomicUsize, fail_at: usize }
    impl Guard { fn live() -> Self { Self { checks: AtomicUsize::new(0), fail_at: usize::MAX } } }
    #[async_trait]
    impl FoundationGuard for Guard {
        async fn revalidate(&self, _: &[ResolvedContext]) -> Result<(), String> {
            if self.checks.fetch_add(1, Ordering::SeqCst) + 1 >= self.fail_at { Err("revoked".into()) } else { Ok(()) }
        }
    }

    async fn manager() -> (SkillsManager, tempfile::TempDir) {
        let db = dgos_db::sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        (SkillsManager::new(db, dir.path().join("skills")), dir)
    }

    #[test]
    fn budgets_preserve_host_window_image_and_output_reserves() {
        assert_eq!(text_ceiling(None, 1024, 2), Some(24 * 1024));
        assert_eq!(text_ceiling(Some(8192), 1024, 2), Some(2048));
        assert_eq!(text_ceiling(Some(2047), 1024, 0), None);
        assert_eq!(text_ceiling(Some(0), 0, 0), None);
        assert!(context_budget(1000, 2, 1048, Some(8192), 1024).can_send);
        let denied = context_budget(1001, 2, 1048, Some(8192), 1024);
        assert!(!denied.can_send);
        assert_eq!(denied.image_reserve_tokens, 4096);
        assert_eq!(denied.rejection.as_deref(), Some("context_budget_exceeded"));
    }

    #[test]
    fn schema_uses_real_validator_and_post_generation_does_not_repair() {
        let contract = AiOutputContract::JsonSchema { schema: json!({"type":"object","properties":{"amount":{"type":"integer","minimum":1}},"required":["amount"],"additionalProperties":false}) };
        assert!(validate_output_contract(&contract).is_ok());
        assert!(validate_output(&contract, r#"{"amount":1}"#).is_ok());
        for invalid in [r#"{"amount":0}"#, r#"{"amount":"1"}"#, r#"{"amount":1,"extra":true}"#, "```json\n{}\n```", "{} trailing"] {
            assert_eq!(validate_output(&contract, invalid).unwrap_err(), "output_contract_failed");
        }
        assert!(validate_output(&AiOutputContract::Text, "ordinary text").is_ok());
        assert!(validate_output(&AiOutputContract::Text, &"x".repeat(MAX_OUTPUT_BYTES + 1)).is_err());
    }

    #[test]
    fn schemas_reject_external_recursive_excessive_or_invalid_before_dispatch() {
        for schema in [json!({"$ref":"https://example.invalid/private"}), json!({"$ref":"file:///etc/passwd"}),
            json!({"$ref":"#"}), json!({"$dynamicRef":"#node"}), json!({"type":"not-a-type"}),
            json!({"$defs":{"cycle":{"$ref":"#/$defs/cycle"}},"$ref":"#/$defs/cycle"})] {
            assert!(validate_output_contract(&AiOutputContract::JsonSchema { schema }).is_err());
        }
        let local = AiOutputContract::JsonSchema { schema: json!({"$defs":{"n":{"type":"integer"}},"$ref":"#/$defs/n"}) };
        assert!(validate_output(&local, "123").is_ok());
        assert!(validate_output(&local, "\"123\"").is_err());
        let mut deep = json!({"type":"string"});
        for _ in 0..33 { deep = json!({"allOf":[deep]}); }
        assert!(validate_output_contract(&AiOutputContract::JsonSchema { schema: deep }).is_err());
        assert!(validate_output_contract(&AiOutputContract::JsonSchema { schema: json!({"description":"x".repeat(MAX_SCHEMA_BYTES)}) }).is_err());
    }

    #[test]
    fn imports_validate_utf8_signatures_mime_names_and_limits() {
        let input = |name: &str, mime: &str, bytes: &[u8]| AiResourceImport { name: name.into(), mime: mime.into(), base64: base64::engine::general_purpose::STANDARD.encode(bytes) };
        assert_eq!(validate_resource(&input("notes.md", "text/markdown", "正文".as_bytes())).unwrap(), "正文".as_bytes());
        assert!(validate_resource(&input("bad.txt", "text/plain", &[0xff])).is_err());
        assert!(validate_resource(&input("bad.txt", "text/plain", b"a\0b")).is_err());
        assert!(validate_resource(&input("bad.png", "image/png", b"pretend image")).is_err());
        assert!(validate_resource(&input("valid.png", "image/png", b"\x89PNG\r\n\x1a\n")).is_ok());
        assert!(validate_resource(&input("file.svg", "image/svg+xml", b"<svg/>")).is_err());
        assert!(validate_resource(&input("../escape.txt", "text/plain", b"text")).is_err());
        assert!(validate_resource(&input("a.txt", "text/plain", &vec![b'x'; 1024 * 1024 + 1])).is_err());
    }

    #[test]
    fn inline_utf8_ranges_and_hash_expiry_are_exact() {
        let selected = source("A中B文C", AiContextMode::Inline, 1, 5);
        let inline = assemble_inline(&[selected.clone()], false).unwrap();
        assert!(inline.text.ends_with("中B"));
        assert_eq!(inline.receipts[0].start, 1);
        assert_eq!(inline.receipts[0].bytes, 4);
        let mut invalid = selected.clone(); invalid.reference.start = 2;
        assert!(assemble_inline(&[invalid], false).is_err());
        let mut invalid = selected.clone(); invalid.bytes[0] = b'Z';
        assert!(assemble_inline(&[invalid], false).is_err());
        let mut invalid = selected; invalid.resource.expires_at = 0;
        assert!(assemble_inline(&[invalid], false).is_err());
    }

    #[test]
    fn images_use_real_image_parts_not_textual_claims() {
        let bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut selected = source("", AiContextMode::Inline, 0, 8192);
        selected.resource.mime = "image/png".into(); selected.resource.bytes_len = bytes.len() as u64;
        selected.resource.sha256 = format!("{:x}", Sha256::digest(&bytes));
        selected.reference.sha256 = selected.resource.sha256.clone(); selected.bytes = bytes;
        assert!(assemble_inline(&[selected.clone()], false).is_err());
        let inline = assemble_inline(&[selected], true).unwrap();
        assert!(inline.text.is_empty());
        assert_eq!(inline.images.len(), 1);
        assert_eq!(inline.receipts[0].bytes as usize, inline.images[0].data.len());
    }

    #[tokio::test]
    async fn local_reads_cannot_escape_fixed_subset_or_selected_range() {
        let (manager, _dir) = manager().await;
        let guard = Guard::live();
        let mut tools = FoundationTools::new_borrowed(&manager, None, vec![source("hidden selected tail", AiContextMode::OnDemand, 7, 8)], &guard).unwrap();
        let listed = tools.dispatch(RESOURCE_LIST_TOOL, json!({}), 4096).await.unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert!(tools.context_receipts().is_empty());
        assert!(tools.dispatch(RESOURCE_READ_TOOL, json!({"resource_id":"other","start":7,"max_bytes":8}), 4096).await.is_err());
        assert!(tools.dispatch(RESOURCE_READ_TOOL, json!({"resource_id":"selected","start":0,"max_bytes":8}), 4096).await.is_err());
        assert!(tools.dispatch(RESOURCE_READ_TOOL, json!({"resource_id":"selected","start":7,"max_bytes":8,"scope":"other"}), 4096).await.is_err());
        let read = tools.dispatch(RESOURCE_READ_TOOL, json!({"resource_id":"selected","start":7,"max_bytes":100}), 4096).await.unwrap();
        assert_eq!(read["content"], "selected");
        assert!(tools.context_receipts().is_empty());
        assert_eq!(tools.receipts().lock().unwrap().pending_context_receipts.len(), 1);
        tools.model_dispatched();
        tools.model_dispatched(); // Idempotent: a later model round cannot duplicate coverage.
        assert_eq!(tools.context_receipts().len(), 1);
        assert_eq!(tools.context_receipts()[0].bytes, 8);
        assert_eq!(tools.context_receipts()[0].total_bytes, 20);
        assert!(tools.skill_receipts().as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn revocation_after_read_and_budget_rejection_publish_no_receipt() {
        let (manager, _dir) = manager().await;
        let guard = Guard { checks: AtomicUsize::new(0), fail_at: 2 };
        let mut tools = FoundationTools::new_borrowed(&manager, None, vec![source("source", AiContextMode::OnDemand, 0, 6)], &guard).unwrap();
        assert_eq!(tools.dispatch(RESOURCE_READ_TOOL, json!({"resource_id":"selected","start":0,"max_bytes":6}), 4096).await.unwrap_err(), "revoked");
        assert!(tools.context_receipts().is_empty());
        let live = Guard::live();
        let mut tools = FoundationTools::new_borrowed(&manager, None, vec![source("source", AiContextMode::OnDemand, 0, 6)], &live).unwrap();
        assert!(tools.dispatch(RESOURCE_READ_TOOL, json!({"resource_id":"selected","start":0,"max_bytes":6}), 2).await.is_err());
        assert!(tools.context_receipts().is_empty());
    }

    #[tokio::test]
    async fn byte_budget_counts_json_escaping_and_only_reports_actual_utf8_bytes() {
        let (manager, _dir) = manager().await;
        let guard = Guard::live();
        let text = "中\n\"".repeat(1024);
        let mut tools = FoundationTools::new_borrowed(&manager, None, vec![source(&text, AiContextMode::OnDemand, 0, text.len() as u32)], &guard).unwrap();
        let value = tools.dispatch(RESOURCE_READ_TOOL, json!({"resource_id":"selected","start":0,"max_bytes":4096}), 512).await.unwrap();
        assert!(tool_result_content_bytes(&value).unwrap() <= 512);
        let content = value["content"].as_str().unwrap();
        assert!(!content.is_empty());
        assert!(tools.context_receipts().is_empty());
        tools.model_dispatched();
        assert_eq!(tools.context_receipts()[0].bytes as usize, content.len());
        assert!(text.starts_with(content));
    }

    #[tokio::test]
    async fn shared_tools_do_not_hold_manager_lock_between_steps() {
        let (manager, _dir) = manager().await;
        let skills = Arc::new(RwLock::new(manager));
        let guard = Guard::live();
        let mut tools = FoundationTools::new(skills.clone(), None, vec![source("source", AiContextMode::Inline, 0, 6)], &guard).unwrap();
        assert!(skills.try_write().is_ok());
        tools.revalidate().await.unwrap();
        assert!(skills.try_write().is_ok());
        tools.inline(false).await.unwrap();
        assert!(tools.context_receipts().is_empty());
        assert_eq!(tools.receipts().lock().unwrap().pending_context_receipts[0].bytes, 6);
        tools.model_dispatched();
        assert_eq!(tools.context_receipts()[0].bytes, 6);
        assert!(skills.try_write().is_ok());
    }

    #[test]
    fn inspection_and_dispatch_count_identical_tool_schema_envelopes() {
        let messages = vec![dgos_types::ChatMessage { role: "user".into(), content: "request".into(), images: vec![] }];
        let generation = dgos_types::GenerationParameters::default();
        let definitions = local_definitions(None, &[source("source", AiContextMode::OnDemand, 0, 6)]);
        let schema_bytes = tool_schema_bytes(&definitions).unwrap();
        let budget = request_context_budget(&messages, &generation, schema_bytes, None, true).unwrap();
        let mut wire = dgos_types::ToolChatRequest { messages: vec![dgos_types::ToolMessage { role: "user".into(), content: Some("request".into()), tool_call_id: None,
            tool_calls: vec![], reasoning: None, images: vec![] }], tools: definitions, max_tokens: generation.output_tokens(), temperature: generation.temperature, reasoning: generation.reasoning };
        assert_eq!(budget.estimated_input_tokens, serde_json::to_vec(&wire).unwrap().len());
        let boundary = budget.estimated_input_tokens + 1024 + wire.max_tokens as usize;
        assert!(crate::tools::validate_budget(&wire, Some(boundary as i32)).is_ok());
        assert!(crate::tools::validate_budget(&wire, Some(boundary as i32 - 1)).is_err());
        wire.tools[0].function.name = "dgos_tool_0".into();
        let reserved = tool_schema_bytes(&wire.tools).unwrap();
        wire.tools.push(crate::execution::result_read_definition());
        assert_eq!(reserved, tool_schema_bytes(&wire.tools).unwrap());
    }

    #[tokio::test]
    async fn pinned_skill_reads_publish_metadata_receipts_and_preserve_runtime_budget() {
        use dgos_db::{migrations::Migrator, sea_orm_migration::MigratorTrait};
        let db = dgos_db::sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let manager = SkillsManager::new(db, dir.path().join("skills"));
        manager.register_bundled_catalog(&std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills")).await.unwrap();
        manager.install_skill("dgos.official.writing.summarize", "alice").await.unwrap();
        let pins: Vec<_> = manager.chat_skill_catalog("alice").await.unwrap().into_iter().map(|entry| entry.pin).collect();
        let runtime = SkillRuntime::authorize(&manager, "alice", &pins, 65536, 2).await.unwrap();
        let guard = Guard::live();
        let mut tools = FoundationTools::new_borrowed(&manager, Some(runtime), vec![], &guard).unwrap();
        let content = tools.dispatch(crate::skill_runtime::SKILL_LOAD_TOOL, json!({"skill_id":pins[0].skill_id}), 24000).await.unwrap();
        assert!(tools.skill_receipts().as_array().unwrap().is_empty());
        assert_eq!(tools.receipts().lock().unwrap().pending_skill_receipts.as_array().unwrap().len(), 1);
        tools.model_dispatched();
        let receipt = tools.skill_receipts();
        assert_eq!(receipt[0]["bytes"].as_u64().unwrap() as usize, content["content"].as_str().unwrap().len());
        assert_eq!(receipt[0]["path"], "SKILL.md");
        assert!(!receipt.to_string().contains("name:"));
        manager.toggle_skill(&pins[0].skill_id, "alice", false).await.unwrap();
        assert!(tools.dispatch(crate::skill_runtime::SKILL_LOAD_TOOL, json!({"skill_id":pins[0].skill_id}), 24000).await.is_err());
        assert_eq!(tools.skill_receipts(), receipt);
        let runtime = tools.into_runtime().unwrap();
        assert_eq!(runtime.receipts().len(), 1);
        assert!(runtime.consumed_bytes() > 0);
    }

    struct NoApproval;
    #[async_trait]
    impl crate::execution::ExecutionGuard for NoApproval {
        async fn before_model(&self) -> Result<(), String> { Ok(()) }
        async fn before_tool(&self) -> Result<(), String> { Ok(()) }
        async fn approve(&self, _: &crate::ToolProposal) -> Result<bool, String> { panic!("host resource reads never use MCP approval") }
    }

    #[test]
    fn schema_literal_keywords_and_property_names_are_not_interpreted_as_schemas() {
        let literal = json!({"$ref":"https://literal.invalid/data","$schema":"not-a-dialect"});
        let contract = AiOutputContract::JsonSchema { schema: json!({
            "type":"object", "properties": {
                "$ref":{"const":literal}, "$schema":{"enum":[literal]}, "examples":{"type":"string"}
            }, "required":["$ref","$schema"], "examples":[literal], "additionalProperties":false
        }) };
        assert!(validate_output_contract(&contract).is_ok());
        assert!(validate_output(&contract, &json!({"$ref":literal,"$schema":literal}).to_string()).is_ok());
        let external = AiOutputContract::JsonSchema { schema: json!({"properties":{"safe":{"$ref":"https://external.invalid/schema"}}}) };
        assert_eq!(validate_output_contract(&external).unwrap_err(), "unsupported_output_schema_reference");
    }

    #[tokio::test]
    async fn backslash_heavy_sixteen_kib_source_shrinks_to_final_tool_message_budget() {
        let (manager, _dir) = manager().await;
        let guard = Guard::live();
        let text = "\\".repeat(16 * 1024);
        let mut tools = FoundationTools::new_borrowed(&manager, None, vec![source(&text, AiContextMode::OnDemand, 0, text.len() as u32)], &guard).unwrap();
        let value = tools.dispatch(RESOURCE_READ_TOOL, json!({"resource_id":"selected","start":0,"max_bytes":16384}), 16384).await.unwrap();
        assert!(tool_result_content_bytes(&value).unwrap() <= 16384);
        let bytes = value["content"].as_str().unwrap().len();
        assert!(bytes > 0 && bytes < text.len() / 2);
        assert!(tools.context_receipts().is_empty());
        let request = dgos_types::ToolChatRequest {
            messages: vec![dgos_types::ToolMessage { role: "tool".into(), content: Some(value.to_string()), tool_call_id: Some("call".into()), tool_calls: vec![], reasoning: None, images: vec![] }],
            tools: tools.definitions(), max_tokens: 1024, temperature: None, reasoning: None,
        };
        assert!(crate::tools::validate_budget(&request, None).is_ok());
        tools.model_dispatched();
        assert_eq!(tools.context_receipts()[0].bytes as usize, bytes);
    }

    async fn executor_fixture(base_url: &str) -> (tempfile::TempDir, dgos_providers::ProviderManager, dgos_mcp::McpManager, SkillsManager, String) {
        use dgos_db::{migrations::Migrator, sea_orm_migration::MigratorTrait};
        use dgos_providers::test_support::{seed_provider, MemoryCredentialVault};
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("executor.db").display());
        let db = dgos_db::sea_orm::Database::connect(&url).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let vault = Arc::new(MemoryCredentialVault::default());
        let provider = seed_provider(db.clone(), &vault, dgos_types::ProviderConfig { name: "local fixture".into(), provider_type: "openai_compatible".into(),
            base_url: base_url.into(), api_key: "synthetic-only".into(), enabled: true, config: None }, "alice").await.unwrap();
        dgos_db::ModelRepository::new(db.clone()).sync_models(&provider.id, vec![dgos_types::Model {
            id: uuid::Uuid::new_v4().to_string(), provider_id: provider.id.clone(), model_id: "chat".into(), model_name: "chat".into(), model_type: "chat".into(),
            context_window: Some(65536), max_output_tokens: Some(2048), ..Default::default()
        }]).await.unwrap();
        let providers = dgos_providers::ProviderManager::with_vault(&url, vault).await.unwrap();
        let mcp = dgos_mcp::McpManager::new(db.clone());
        let manager = SkillsManager::new(db, dir.path().join("skills"));
        (dir, providers, mcp, manager, provider.id)
    }

    #[tokio::test]
    async fn executor_runs_local_resource_call_and_preserves_usage_if_final_phase_delivery_fails() {
        for refuse_final in [false, true] {
        let mut server = mockito::Server::new_async().await;
        let call = json!({"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_local","type":"function","function":{"name":RESOURCE_READ_TOOL,"arguments":json!({"resource_id":"selected","start":0,"max_bytes":6}).to_string()}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}});
        let first = server.mock("POST", "/chat/completions").with_status(200).with_header("content-type", "text/event-stream")
            .with_body(format!("data: {call}\n\ndata: [DONE]\n\n")).expect(1).create_async().await;
        let final_chunk = json!({"choices":[{"index":0,"delta":{"content":"source observed"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}});
        let second = server.mock("POST", "/chat/completions").match_body(mockito::Matcher::Regex("\"role\":\"tool\"".into()))
            .with_status(200).with_header("content-type", "text/event-stream").with_body(format!("data: {final_chunk}\n\ndata: [DONE]\n\n")).expect(1).create_async().await;
        let (_dir, providers, mcp, manager, provider_id) = executor_fixture(&server.url()).await;
        let guard = Guard::live();
        let mut local = FoundationTools::new_borrowed(&manager, None, vec![source("source", AiContextMode::OnDemand, 0, 6)], &guard).unwrap();
        let receipts = local.receipts();
        let request = crate::execution::ExecutionRequest {
            user: "alice".into(), provider_id, model_id: "chat".into(), revision: None,
            messages: vec![dgos_types::ChatMessage { role: "user".into(), content: "Read the selected source".into(), images: vec![] }], tools: vec![],
            generation: Default::default(), context_window: Some(65536),
            provenance: dgos_types::ExecutionProvenance { caller: dgos_types::BuiltinCaller::Workbench, request_id: "foundation-loop".into() },
            cancellation: tokio_util::sync::CancellationToken::new(), deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(10),
        };
        let cancellation = request.cancellation.clone();
        let sink = |event| !(refuse_final && matches!(event, crate::execution::CoreEvent::Phase(dgos_types::AiPhase::FinalAnswer)));
        let result = crate::execution::AiExecutor::run_with_local(&providers, &mcp, request, &NoApproval, &sink, Some(&mut local)).await;
        assert_eq!(result.error.as_deref(), if refuse_final { Some("task_cancelled") } else { None });
        assert_eq!(result.total_tokens, Some(10));
        assert!(!result.result_unknown);
        assert_eq!(cancellation.is_cancelled(), refuse_final);
        assert_eq!(result.content.as_deref(), Some("source observed"));
        assert_eq!(receipts.lock().unwrap().context_receipts[0].bytes, 6);
        assert!(guard.checks.load(Ordering::SeqCst) >= 6);
        assert!(mcp.list_usage_logs("alice", None, None).await.unwrap().is_empty());
        first.assert_async().await;
        second.assert_async().await;
        }
    }

    #[tokio::test]
    async fn plain_stream_preserves_progress_and_known_usage_when_final_revalidation_stops() {
        for (cancel, reject_before_dispatch) in [(false, false), (true, false), (false, true)] {
            let mut server = mockito::Server::new_async().await;
            let chunk = json!({"choices":[{"index":0,"delta":{"content":"retained progress"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}});
            let response = server.mock("POST", "/chat/completions").with_status(200).with_header("content-type", "text/event-stream")
                .with_body(format!("data: {chunk}\n\ndata: [DONE]\n\n")).expect(usize::from(!reject_before_dispatch)).create_async().await;
            let (_dir, providers, mcp, manager, provider_id) = executor_fixture(&server.url()).await;
            let guard = Guard { checks: AtomicUsize::new(0), fail_at: if reject_before_dispatch { 1 } else { 2 } };
            let contexts = vec![source("source", AiContextMode::Inline, 0, 6)];
            let inline = assemble_inline(&contexts, false).unwrap();
            let mut local = FoundationTools::new_borrowed_contexts(&manager, None, &contexts, &guard).unwrap();
            let receipts = local.receipts();
            receipts.lock().unwrap().pending_context_receipts = inline.receipts;
            let request = crate::execution::ExecutionRequest {
                user: "alice".into(), provider_id, model_id: "chat".into(), revision: None,
                messages: vec![dgos_types::ChatMessage { role: "user".into(), content: inline.text, images: vec![] }], tools: vec![],
                generation: Default::default(), context_window: Some(65536),
                provenance: dgos_types::ExecutionProvenance { caller: dgos_types::BuiltinCaller::Workbench, request_id: "plain-final-revalidation".into() },
                cancellation: tokio_util::sync::CancellationToken::new(), deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(10),
            };
            let sink = |event| !(cancel && matches!(event, crate::execution::CoreEvent::Delta(_)));
            let result = crate::execution::AiExecutor::run_with_local(&providers, &mcp, request, &NoApproval, &sink, Some(&mut local)).await;
            if reject_before_dispatch {
                assert!(result.content.is_none());
                assert!(receipts.lock().unwrap().context_receipts.is_empty());
                assert_eq!(receipts.lock().unwrap().pending_context_receipts.len(), 1);
            } else {
                assert_eq!(result.content.as_deref(), Some("retained progress"));
                assert_eq!(receipts.lock().unwrap().context_receipts.len(), 1);
                assert!(receipts.lock().unwrap().pending_context_receipts.is_empty());
                if cancel { assert_eq!(result.error.as_deref(), Some("task_cancelled")); }
                else { assert_eq!(result.error.as_deref(), Some("revoked")); assert_eq!(result.total_tokens, Some(7)); }
            }
            response.assert_async().await;
        }
    }
}
