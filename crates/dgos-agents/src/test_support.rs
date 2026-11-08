use crate::{AgentManager, projects::{CreateProjectInput, SaveTaskInput}};
use dgos_db::{DevelopmentRepository, migrations::{Migrator, MigratorTrait}};
use std::path::Path;
use tokio_util::sync::CancellationToken;
pub async fn repository(root: &Path) -> DevelopmentRepository {
    let db = dgos_db::establish_connection(&format!("sqlite://{}?mode=rwc", root.join("metadata.sqlite").display())).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    DevelopmentRepository::new(db)
}
pub async fn manager(root: &Path) -> AgentManager {
    let repository = repository(root).await;
    let manager = AgentManager::new(root.join("index.json"), repository.clone(), providers(root, repository).await);
    manager.load().await.unwrap();
    manager
}
pub async fn task(manager: &AgentManager, root: &Path) -> String {
    let project = manager.create_project("alice", CreateProjectInput { kind: "import".into(), name: "fixture".into(), path: root.to_string_lossy().into(), source: None }, CancellationToken::new()).await.unwrap();
    manager.save_development_task("alice", SaveTaskInput { id: None, project_id: project.id, goal: "Exercise native lifecycle".into(), requirements: "Keep ownership and shutdown semantics".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap().id
}

#[derive(Default)]
struct MemoryVault(tokio::sync::Mutex<std::collections::HashMap<String, dgos_providers::SecretValue>>);
#[async_trait::async_trait]
impl dgos_providers::ProviderCredentialVault for MemoryVault {
    async fn store(&self, scope: &dgos_providers::ProviderCredentialScope, value: dgos_providers::SecretValue) -> dgos_types::DgosResult<()> { self.0.lock().await.insert(scope.account(), value); Ok(()) }
    async fn load(&self, scope: &dgos_providers::ProviderCredentialScope) -> dgos_types::DgosResult<dgos_providers::SecretValue> { dgos_providers::SecretValue::new(self.0.lock().await.get(&scope.account()).ok_or(dgos_types::DgosError::CredentialUnavailable)?.expose().into()) }
    async fn remove(&self, scope: &dgos_providers::ProviderCredentialScope) -> dgos_types::DgosResult<()> { self.0.lock().await.remove(&scope.account()); Ok(()) }
}

pub async fn providers(root: &Path, repository: DevelopmentRepository) -> crate::capabilities::CliProviders {
    let url = format!("sqlite://{}?mode=rwc", root.join("metadata.sqlite").display());
    let vault = std::sync::Arc::new(MemoryVault::default());
    let providers = dgos_providers::ProviderManager::with_vault(&url, vault.clone()).await.unwrap();
    let db = dgos_db::establish_connection(&url).await.unwrap();
    let repo = dgos_db::ProviderRepository::new(db);
    // Rehydrate only synthetic fixture credentials; never consult a native vault.
    for owner in ["alice", "bob"] {
        let id = format!("fixture-{owner}");
        let credential = format!("fixture-{owner}");
        dgos_providers::ProviderCredentialVault::store(vault.as_ref(), &dgos_providers::ProviderCredentialScope { user_id: owner.into(), provider_id: id.clone(), endpoint: "http://127.0.0.1:9/v1".into(), credential_ref: credential.clone() }, dgos_providers::SecretValue::new("synthetic-fixture-key".into()).unwrap()).await.unwrap();
        if providers.get_provider(&id, owner).await.unwrap().is_none() {
            repo.create_with_credential_ref(id.clone(), dgos_types::ProviderConfig { name: "Fixture".into(), provider_type: "openai_compatible".into(), base_url: "http://127.0.0.1:9/v1".into(), api_key: String::new(), enabled: true, config: Some(serde_json::json!({"allow_insecure_http":true,"auth_style":"bearer"})) }, owner.into(), Some(credential)).await.unwrap();
            providers.add_manual_model(&id, owner, "fixture-model", None).await.unwrap();
        }
    }
    crate::capabilities::CliProviders::new(repository, providers).unwrap()
}

pub async fn create(manager: &AgentManager, owner: &str, task: &str, options: crate::SessionOptions) -> crate::capabilities::SessionCreate {
    let task = manager.inner.repository.task(owner, task).await.unwrap();
    let selected = manager.inner.providers.providers().get_provider(&format!("fixture-{owner}"), owner).await.unwrap().unwrap();
    let provider = crate::capabilities::CliProviderRef { provider_id: selected.id, model_id: "fixture-model".into(), provider_revision: selected.config_revision };
    let service = &manager.inner.providers;
    let revision = if let Some(head) = service.read(owner, &task.project_id).await.unwrap() { head.revision } else {
        service.save(owner, crate::capabilities::SaveCliProviderInput { project_id: task.project_id, expected_revision: 0, configuration: crate::capabilities::CliProviderConfiguration { engine: crate::Engine::Codex, provider: provider.clone() } }).await.unwrap().revision
    };
    crate::capabilities::SessionCreate { options, project_revision: revision, provider }
}

#[cfg(unix)]
pub async fn codex_fixture(path: &Path, body: &str) {
    let prelude = r#"if [ "${1-}" = --version ]; then printf 'codex-cli 0.167.0\n'; exit 0; fi
"#;
    prepare_shell_fixture(path, &format!("{prelude}{body}")).await;
}

pub const CODEX_CONFIG_REPLY: &str = r#"*'"method":"config/read"'*) printf '%s\n' '{"id":2,"result":{"config":{"model_provider":"dgos_project","model_providers":{"dgos_project":{"base_url":"http://127.0.0.1:9/v1","wire_api":"responses","env_key":"DGOS_PROVIDER_KEY","requires_openai_auth":false}}}}}';;
"#;

/// Admit a newly written script before timing its protocol or process cleanup.
/// macOS can hold the first shebang exec before userspace for several seconds
/// when parallel tests create executables. The readiness branch must exit before
/// starting descendants, writing markers, or consuming protocol input.
#[cfg(unix)]
pub(crate) async fn prepare_shell_fixture(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\nif [ \"${{1-}}\" = --dgos-fixture-ready ]; then exit 0; fi\n{body}")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = tokio::process::Command::new(path)
        .arg("--dgos-fixture-ready")
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .kill_on_drop(true).spawn().unwrap();
    match tokio::time::timeout(std::time::Duration::from_secs(10), child.wait()).await {
        Ok(Ok(status)) => assert!(status.success(), "shell fixture failed executable admission"),
        failed => {
            let _ = child.start_kill();
            let reaped = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
            panic!("shell fixture admission failed: {failed:?}; cleanup: {reaped:?}");
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn shell_fixture_preparation_does_not_run_its_protocol_or_side_effects() {
    let root = std::env::temp_dir().join(format!("dgos-fixture-preparation-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let executable = root.join("fixture");
    let marker = root.join("started");
    prepare_shell_fixture(&executable, &format!("printf started > '{}'\nprintf 'codex-cli 0.0.0\\n'\n", marker.display())).await;
    assert!(!marker.exists());
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), crate::cli::probe(crate::cli::CliTool::Codex, executable.to_str())).await.unwrap();
    assert_eq!(result.status, "available");
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "started");
    std::fs::remove_dir_all(root).unwrap();
}

#[derive(Clone, Default)]
pub(crate) struct DiagnosticLog(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for DiagnosticLog {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for DiagnosticLog {
    type Writer = Self;
    fn make_writer(&'a self) -> Self { self.clone() }
}
impl DiagnosticLog {
    pub(crate) fn capture() -> (Self, tracing::Dispatch) {
        // tracing-core's single-dispatch fast path uses the registering thread's
        // current default. A concurrent unsubscribed probe can otherwise cache
        // `never` for the shared event callsite. Keep a second registered dispatch
        // alive so registration considers all test captures, on every thread.
        static REGISTRATION_ANCHOR: std::sync::LazyLock<tracing::Dispatch> = std::sync::LazyLock::new(|| {
            tracing::Dispatch::new(tracing_subscriber::fmt().with_writer(std::io::sink).finish())
        });
        std::sync::LazyLock::force(&REGISTRATION_ANCHOR);
        let log = Self::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false).without_time().with_writer(log.clone()).finish();
        // Async callers attach this dispatch to each polled future instead of
        // retaining a thread-local guard across suspension and runtime tasks.
        (log, tracing::Dispatch::new(subscriber))
    }
    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}
