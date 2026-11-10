//! Isolated production-service composition shared only by the foundation example.
use dgos_application::{AiEventSink, AiInvocation, AiService, HostCapabilityService,
    McpConfirmationDeliveryError, McpConfirmationNotifier, McpConfirmationPrompt,
    ModuleMcpConfirmations, ModuleReleaseService};
use dgos_db::{migrations::Migrator, sea_orm_migration::MigratorTrait, AiRunRepository,
    MaintenanceGate, ModulePermission, ModulePermissionRepository, NewUser, UserRepository};
use dgos_mcp::{CreateMcpServerRequest, McpCallPolicy, StdioConfig, TransportKind};
use dgos_modules::{ModuleCallRouter, ModulePageTransport, ModulePublisher, ModuleRuntimeSupervisor,
    PageInstruction, PageLoaderConfig, PageModuleLoader, PageReceipt, PageSendError, PinnedModuleAdmission};
use dgos_providers::{test_support::{seed_provider, MemoryCredentialVault}, ProviderManager};
use dgos_skills::SkillsManager;
use dgos_types::*;
use serde_json::{json, Value};
use std::{path::Path, sync::{Arc, Mutex}};
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

pub const USER: &str = "foundation-smoke-user";
pub const MODULE: &str = "foundation-smoke-consumer";
pub const SKILL: &str = "dgos.official.writing.summarize";
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

struct Allow;
#[async_trait::async_trait]
impl McpCallPolicy for Allow {
    async fn allows_call(&self, user: &str, _: &str, _: &str) -> bool { user == USER }
}
pub struct Prompt { pub id: String, pub attachment: String, pub module: String, pub server: String, pub tool: String, pub args: Value }
struct Confirm(mpsc::UnboundedSender<Prompt>);
impl McpConfirmationNotifier for Confirm {
    fn open(&self, attachment: &str, prompt: McpConfirmationPrompt<'_>) -> std::result::Result<(), McpConfirmationDeliveryError> {
        self.0.send(Prompt { id: prompt.confirmation_id.into(), attachment: attachment.into(), module: prompt.module_name.into(),
            server: prompt.server_id.into(), tool: prompt.tool_name.into(), args: prompt.arguments.clone() }).map_err(|_| McpConfirmationDeliveryError)
    }
    fn close(&self, _: &str, _: &str) {}
}
struct Page(mpsc::UnboundedSender<(String, PageInstruction)>);
impl ModulePageTransport for Page {
    fn send(&self, attachment: &str, instruction: PageInstruction) -> std::result::Result<(), PageSendError> {
        self.0.send((attachment.into(), instruction)).map_err(|_| PageSendError)
    }
}

pub struct Fixture {
    pub db: dgos_db::sea_orm::DatabaseConnection,
    pub service: AiService,
    pub mcp: Arc<dgos_mcp::McpManager>,
    pub skills: Arc<RwLock<SkillsManager>>,
    pub permissions: Arc<ModulePermissionRepository>,
    pub repository: AiRunRepository,
    pub router: ModuleCallRouter,
    pub route: String,
    pub attachment: String,
    pub account: CancellationToken,
    pub page: CancellationToken,
    pub input: AiRunInput,
    pub prompts: mpsc::UnboundedReceiver<Prompt>,
    _supervisor: ModuleRuntimeSupervisor,
    _runtime: dgos_modules::ModuleRuntimeHandle,
    _lost: mpsc::UnboundedReceiver<dgos_modules::RuntimeLost>,
    driver: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture { fn drop(&mut self) { self.driver.abort(); } }
impl Fixture {
    pub async fn open(root: &Path, base_url: &str, model: &str, key: &str) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let url = format!("sqlite://{}?mode=rwc", root.join("smoke.sqlite").display());
        let db = dgos_db::establish_connection(&url).await?;
        Migrator::up(&db, None).await?;
        let publisher = Arc::new(ModulePublisher::new(root.join("modules"), db.clone()));
        // This local non-authority file only lets the recovery process reopen the exact pin.
        let pin_path = root.join("fixture-pin.json");
        let integrity: String = if pin_path.exists() {
            serde_json::from_slice(&std::fs::read(&pin_path)?)?
        } else {
            for (id, name) in [(USER, "Foundation smoke"), ("foundation-other-user", "Other smoke")] {
                UserRepository::new(db.clone()).insert(NewUser { id, username: name,
                    username_key: id, password_hash: "isolated-fixture", now: 1 }).await?;
            }
            let package = root.join("package");
            std::fs::create_dir(&package)?;
            std::fs::write(package.join("entry.js"), "export default () => ({Component: () => null});")?;
            let manifest = |hash: &str| json!({"id":MODULE,"name":"Foundation smoke consumer",
                "version":"1.0.0","entry":"entry.js","artifact":{"version":"1.0.0","sha256":hash},
                "permissions":["providers.call","skills.execute","mcp.call"]}).to_string();
            std::fs::write(package.join("manifest.json"), manifest(&"0".repeat(64)))?;
            let digest = dgos_modules::package_integrity(&package)?;
            std::fs::write(package.join("manifest.json"), manifest(digest.strip_prefix("v2:").ok_or("unexpected package hash format")?))?;
            let release = ModuleReleaseService::new(publisher.clone()).publish(&package).await?;
            std::fs::write(&pin_path, serde_json::to_vec(&release.integrity)?)?;
            release.integrity
        };
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let loader_config = PageLoaderConfig::default();
        let host_window = loader_config.window_label.clone();
        let (loader, lost) = PageModuleLoader::new(Arc::new(Page(sender)), loader_config);
        let attachment = loader.attach(&host_window).attachment_id;
        let router = ModuleCallRouter::new(loader.clone());
        let responder = loader.clone();
        let driver = tokio::spawn(async move {
            while let Some((attachment, instruction)) = receiver.recv().await {
                let (ticket, receipt) = match instruction {
                    PageInstruction::Load { ticket, .. } => (ticket, PageReceipt::Loaded),
                    PageInstruction::Unload { ticket, .. } => (ticket, PageReceipt::Unloaded { hook_failed: false }),
                };
                responder.receive(&attachment, &ticket, receipt);
            }
        });
        let supervisor = ModuleRuntimeSupervisor::new(publisher.clone(), Arc::new(loader))
            .with_admission(Arc::new(PinnedModuleAdmission::new([(MODULE, "1.0.0", integrity.as_str())])));
        let runtime = supervisor.start(&HostSession::for_user(USER), MODULE).await?;
        let route = router.issue(&runtime).map_err(|e| format!("route issue: {e:?}"))?.as_str().to_owned();
        let permissions = Arc::new(ModulePermissionRepository::new(db.clone()));
        for permission in [ModulePermission::ProvidersCall, ModulePermission::SkillsExecute, ModulePermission::McpCall] {
            permissions.grant(USER, MODULE, permission).await?;
        }
        let vault = Arc::new(MemoryCredentialVault::default());
        let provider = seed_provider(db.clone(), &vault, ProviderConfig { name: "Isolated foundation smoke".into(),
            provider_type: "openai_compatible".into(), base_url: base_url.into(), api_key: key.into(), enabled: true,
            config: Some(json!({"max_retries":0,"initial_delay_ms":1,"max_delay_ms":1})) }, USER).await?;
        let providers = Arc::new(ProviderManager::with_vault(&url, vault).await?);
        providers.add_manual_model(&provider.id, USER, model, None).await?;
        let revision = providers.get_provider(&provider.id, USER).await?.ok_or("missing seeded provider")?.config_revision;
        let skills = SkillsManager::new(db.clone(), root.join("skills"));
        skills.register_bundled_catalog(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills")).await?;
        if skills.ai_content_catalog(USER).await?.is_empty() { skills.install_skill(SKILL, USER).await?; }
        let skills = Arc::new(RwLock::new(skills));
        let mcp = Arc::new(dgos_mcp::McpManager::with_policy(db.clone(), Arc::new(Allow)));
        let repository = AiRunRepository::new(db.clone());
        let (sender, prompts) = mpsc::unbounded_channel();
        let confirmations = Arc::new(ModuleMcpConfirmations::new(Arc::new(Confirm(sender))));
        let service = AiService::new(HostCapabilityService::with_publisher(permissions.clone(), publisher),
            repository.clone(), providers, skills.clone(), mcp.clone(), confirmations, MaintenanceGate::new());
        // The host normally initializes this on first admission. Inspect remains read-only.
        repository.authorization_epoch(USER, MODULE).await?;
        Ok(Self { db, service, mcp, skills, permissions, repository, router, route, attachment,
            account: CancellationToken::new(), page: CancellationToken::new(),
            input: AiRunInput { provider_id: provider.id, model_id: model.into(), provider_revision: revision,
                prompt: String::new(), skill_refs: vec![], tools: vec![], generation: GenerationParameters::default(),
                foundation: AiFoundationOptions::default() }, prompts, _supervisor: supervisor, _runtime: runtime, _lost: lost, driver })
    }
    pub fn invoke(&self, control: bool) -> AiInvocation {
        let id = uuid::Uuid::new_v4().to_string();
        let call = if control { self.router.admit_control_once(&self.route, &self.attachment, &id) }
            else { self.router.admit_once(&self.route, &self.attachment, &id) }.expect("fixture admission");
        AiInvocation::bind(call, &HostSession::for_user(USER), self.account.clone(), self.page.clone()).expect("fixture identity")
    }
    pub fn input(&self, marker: &str) -> AiRunInput {
        let mut input = self.input.clone();
        input.prompt = marker.into();
        input
    }
    pub async fn import(&self, name: &str, text: &str) -> Result<AiResource> {
        use base64::Engine;
        Ok(self.service.import_resource(self.invoke(false), AiResourceImport { name: name.into(), mime: "text/plain".into(),
            base64: base64::engine::general_purpose::STANDARD.encode(text) }).await?)
    }
    pub async fn pin(&self) -> Result<AiSkillRef> {
        Ok(self.service.list_skills(self.invoke(true)).await?.into_iter().find(|entry| entry.skill.skill_id == SKILL)
            .ok_or("missing bundled Skill")?.skill)
    }
    pub async fn tool(&self) -> Result<AiToolSelection> {
        let server = self.mcp.create_server(USER, CreateMcpServerRequest { name: "Existing local MCP fixture".into(),
            transport: TransportKind::Stdio, stdio: Some(StdioConfig { executable: "node".into(),
                args: vec![format!("{}/../dgos-mcp/tests/fixtures/mcp-server.mjs", env!("CARGO_MANIFEST_DIR")), "normal".into()], cwd: None }),
            http: None, enabled: Some(true), auto_connect: None }).await?;
        self.mcp.connect(USER, &server.id).await?;
        Ok(self.service.list_tools(self.invoke(true)).await?.into_iter().find(|entry| entry.tool.tool_name == "echo")
            .ok_or("missing existing MCP echo tool")?.tool)
    }
}

pub struct Run {
    pub terminal: AiTerminal,
    pub id: String,
    pub events: Vec<AiEvent>,
}
pub async fn run(f: &Fixture, input: AiRunInput) -> Result<Run> {
    run_service(&f.service, f.invoke(false), input).await
}
pub async fn run_service(service: &AiService, invocation: AiInvocation, input: AiRunInput) -> Result<Run> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let received = events.clone();
    let sink: Arc<dyn AiEventSink> = Arc::new(move |event: AiEvent| { received.lock().unwrap().push(event); true });
    let terminal = service.run(invocation, input, sink).await?;
    let events = events.lock().unwrap().clone();
    let id = events.first().ok_or("missing Started")?.run_id.clone();
    assert!(matches!(events[0].payload, AiEventPayload::Started { .. }));
    assert_eq!(events.iter().filter(|event| matches!(event.payload, AiEventPayload::Terminal { .. })).count(), 1);
    assert!(events.windows(2).all(|pair| pair[0].seq < pair[1].seq));
    Ok(Run { terminal, id, events })
}
pub fn reference(resource: &AiResource, mode: AiContextMode, start: u32, max_bytes: u32) -> AiContextRef {
    AiContextRef { resource_id: resource.id.clone(), sha256: resource.sha256.clone(), mode, start, max_bytes }
}
pub fn text_sse(text: &str) -> String {
    format!("data: {}\n\ndata: [DONE]\n\n", json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":12,"completion_tokens":8,"total_tokens":20}}))
}
pub fn tool_sse(name: &str, id: &str, args: Value) -> String {
    format!("data: {}\n\ndata: [DONE]\n\n", json!({"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{
        "index":0,"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]}))
}
pub type Requests = Arc<Mutex<Vec<Value>>>;
pub async fn response(server: &mut mockito::ServerGuard, marker: &str, after_tool: Option<&str>, body: String) -> (mockito::Mock, Requests) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let marker = marker.to_owned();
    let after_tool = after_tool.map(str::to_owned);
    let mock = server.mock("POST", "/chat/completions").match_request(move |request| {
        let Ok(bytes) = request.body() else { return false; };
        let Ok(value) = serde_json::from_slice::<Value>(bytes) else { return false; };
        let Some(messages) = value["messages"].as_array() else { return false; };
        let matching_prompt = messages.iter().any(|message| message["role"] == "user" && message["content"].as_str() == Some(marker.as_str()));
        let matching_round = match &after_tool {
            Some(id) => messages.last().is_some_and(|message| message["tool_call_id"] == id.as_str()),
            None => !messages.iter().any(|message| message["role"] == "tool"),
        };
        if matching_prompt && matching_round { captured.lock().unwrap().push(value); true } else { false }
    }).with_header("content-type", "text/event-stream").with_body(body).expect(1).create_async().await;
    (mock, requests)
}
pub fn request(requests: &Requests) -> Value {
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "exactly one observed Provider request");
    requests[0].clone()
}
pub fn tool_result(request: &Value, id: &str) -> Value {
    let message = request["messages"].as_array().unwrap().iter().find(|message| message["tool_call_id"] == id).expect("actual local/MCP tool result");
    let text = message["content"].as_str().expect("textual tool wire result");
    serde_json::from_str(text).expect("structured host tool result")
}
