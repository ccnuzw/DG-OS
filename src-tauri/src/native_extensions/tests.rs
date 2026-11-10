use super::*;
use dgos_db::{migrations::{Migrator, MigratorTrait}, DevelopmentRepository, MaintenanceGate};

#[tokio::test]
async fn isolated_service_serializes_same_key_cache_insertion() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let db_path = root.join("metadata.sqlite");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let db = dgos_db::establish_connection(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let repository = DevelopmentRepository::new(db);
    let provider_manager = dgos_providers::ProviderManager::new(&url).await.unwrap();
    let providers = dgos_agents::capabilities::CliProviders::new(repository.clone(), provider_manager).unwrap();
    let gate = MaintenanceGate::new();
    let agents = AgentManager::with_maintenance(root.join("agents.json"), repository.clone(), gate, providers);
    agents.load().await.unwrap();
    let resolver = SessionResolver::no_accounts();
    for name in ["home", "codex", "claude", "journal"] { std::fs::create_dir(root.join(name)).unwrap(); }
    std::fs::write(root.join("codex/config.toml"), "[mcp_servers]\n").unwrap();
    std::fs::write(root.join("claude-user.json"), "{\"mcpServers\":{}}").unwrap();
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.join("journal"), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let generation = resolver.native_session().unwrap().1;
    let host = NativeHost {
        resolver,
        roots: roots::Roots {
            home: root.join("home"),
            codex: root.join("codex"),
            claude: root.join("claude"),
            claude_user: root.join("claude-user.json"),
            journal: root.join("journal"),
        },
        contexts: Mutex::new(HashMap::new()),
        context_init: tokio::sync::Mutex::new(()),
        skill_library: Default::default(),
        mcp_catalog: Default::default(),
    };
    let selection = Selection { target: Target::Codex, scope: Scope::OsUser, project_id: None, task_id: None };
    let ticket = (dgos_types::DEFAULT_USER_ID.to_owned(), generation);
    let permits = MaintenanceGate::new();
    let (left, right) = tokio::join!(
        host.discover_mcp(&agents, selection.clone(), permits.admit().unwrap(), ticket.clone()),
        host.discover_mcp(&agents, selection, permits.admit().unwrap(), ticket.clone()),
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert!(left.instances.is_empty() && !left.incomplete);
    assert!(right.instances.is_empty() && !right.incomplete);
    assert_eq!(host.contexts.lock().unwrap().len(), 1);
    for target in [Target::Codex, Target::Claude] {
        let preview = host.preview_mcp(&agents, PreviewMcp {
            target, scope: Scope::OsUser, project_id: None, task_id: None,
            change: McpChange::Upsert { name: "local-fixture".into(), fields: ServerPatch {
                transport: Field::Set { value: Transport::Stdio },
                program: Field::Set { value: "/usr/bin/true".into() },
                ..Default::default()
            } },
        }, permits.admit().unwrap(), ticket.clone()).await.unwrap();
        let id = preview.receipt.operation_id.as_str().to_owned();
        assert_eq!(preview.receipt.state, core::State::Previewed);
        let receipt = host.operation(&agents, Operation { operation_id: id.clone() }, OperationAction::Commit, Domain::Mcp, permits.admit().unwrap(), ticket.clone()).await.unwrap();
        assert_eq!(receipt.state, core::State::Committed);
        assert!(receipt.pending_activation);
        let found = host.discover_mcp(&agents, Selection { target, scope: Scope::OsUser, project_id: None, task_id: None }, permits.admit().unwrap(), ticket.clone()).await.unwrap();
        assert_eq!(found.instances.iter().map(|instance| instance.native_key.as_str()).collect::<Vec<_>>(), vec!["local-fixture"]);
        let inspected = host.operation(&agents, Operation { operation_id: id }, OperationAction::Inspect, Domain::Mcp, permits.admit().unwrap(), ticket.clone()).await.unwrap();
        assert_eq!(inspected.state, core::State::Committed);
    }
    let source_selection = Selection { target: Target::Codex, scope: Scope::OsUser, project_id: None, task_id: None };
    let source = host.save_skill_source(&agents, skill_library::SaveSource {
        selection: source_selection.clone(), id: None, expected_revision: None, input: "https://github.com/anthropics/skills".into(),
    }, permits.admit().unwrap(), ticket.clone()).await.unwrap();
    let changed = host.save_skill_source(&agents, skill_library::SaveSource {
        selection: source_selection.clone(), id: Some(source.id.clone()), expected_revision: Some(source.revision), input: "https://github.com/anthropics/skills/tree/main/skills".into(),
    }, permits.admit().unwrap(), ticket.clone()).await.unwrap();
    let stale_remove = host.remove_skill_source(&agents, skill_library::SourceRequest {
        selection: source_selection.clone(), source_id: source.id.clone(), expected_revision: source.revision,
    }, permits.admit().unwrap(), ticket.clone()).await.unwrap_err();
    assert_eq!(stale_remove, "native_extension_Conflict");
    let retained = host.list_skill_sources(&agents, source_selection.clone(), permits.admit().unwrap(), ticket.clone()).await.unwrap();
    assert_eq!(retained[0].repository.skill_subpath.as_deref(), Some("skills"));
    assert!(retained[0].commit.is_none(), "saving a repository must not fetch or install it");
    let foreign = ("another-account".to_owned(), ticket.1);
    assert_eq!(host.list_skill_sources(&agents, source_selection.clone(), permits.admit().unwrap(), foreign).await.unwrap_err(), "session_invalid");
    host.remove_skill_source(&agents, skill_library::SourceRequest {
        selection: source_selection.clone(), source_id: changed.id, expected_revision: changed.revision,
    }, permits.admit().unwrap(), ticket.clone()).await.unwrap();
    assert!(host.list_skill_sources(&agents, source_selection, permits.admit().unwrap(), ticket.clone()).await.unwrap().is_empty());
    let rules_selection = Selection { target: Target::Claude, scope: Scope::OsUser, project_id: None, task_id: None };
    assert!(host.discover_skill_rules(&agents, rules_selection.clone(), permits.admit().unwrap(), ticket.clone()).await.unwrap().rules.is_empty());
    let preview = host.preview_visibility(&agents, Visibility {
        target: Target::Claude, scope: Scope::OsUser, project_id: None, task_id: None,
        intent: VisibilityIntent::ClaudeName { name: "future:skill".into(), state: core::skills::RuleState::NameOnly },
    }, permits.admit().unwrap(), ticket.clone()).await.unwrap();
    assert!(preview.affected.is_empty());
    assert_eq!(preview.coverage, "current_observations_only");
    assert_eq!(preview.rule.as_ref().unwrap().name, "future:skill");
    assert_eq!(host.operation(&agents, Operation { operation_id: preview.preview.receipt.operation_id.as_str().into() }, OperationAction::Commit, Domain::Skills, permits.admit().unwrap(), ticket.clone()).await.unwrap().state, core::State::Committed);
    let rules = host.discover_skill_rules(&agents, rules_selection, permits.admit().unwrap(), ticket.clone()).await.unwrap();
    assert_eq!(rules.rules[0].state, core::skills::RuleState::NameOnly);
    for name in ["old-project", "new-project"] {
        std::fs::create_dir_all(root.join(name).join(".codex")).unwrap();
        std::fs::write(root.join(name).join(".codex/config.toml"), format!("[mcp_servers.{name}]\ncommand = '/usr/bin/true'\n")).unwrap();
    }
    let project = agents.create_project(&ticket.0, dgos_agents::projects::CreateProjectInput {
        kind: "import".into(), name: "Relocation".into(), path: root.join("old-project").to_string_lossy().into_owned(), source: None,
    }, tokio_util::sync::CancellationToken::new()).await.unwrap();
    let local = Selection { target: Target::Claude, scope: Scope::ProjectLocal, project_id: Some(project.id.clone()), task_id: None };
    assert!(host.discover_skill_rules(&agents, local, permits.admit().unwrap(), ticket.clone()).await.unwrap().rules.is_empty());
    let local_preview = host.preview_visibility(&agents, Visibility {
        target: Target::Claude, scope: Scope::ProjectLocal, project_id: Some(project.id.clone()), task_id: None,
        intent: VisibilityIntent::ClaudeName { name: "no-instance".into(), state: core::skills::RuleState::Off },
    }, permits.admit().unwrap(), ticket.clone()).await.unwrap();
    assert_eq!(host.operation(&agents, Operation { operation_id: local_preview.preview.receipt.operation_id.as_str().into() }, OperationAction::Cancel, Domain::Skills, permits.admit().unwrap(), ticket.clone()).await.unwrap().state, core::State::Cancelled);
    let denied = host.discover_skills(&agents, DiscoverSkills { target: Target::Claude, scope: Scope::ProjectLocal, project_id: Some(project.id.clone()), task_id: None, query: String::new() }, permits.admit().unwrap(), ticket.clone()).await.unwrap_err();
    assert_eq!(denied, "native_extension_Unsupported");
    let selection = Selection { target: Target::Codex, scope: Scope::ProjectShared, project_id: Some(project.id.clone()), task_id: None };
    let old = host.discover_mcp(&agents, selection.clone(), permits.admit().unwrap(), ticket.clone()).await.unwrap();
    assert_eq!(old.instances[0].native_key, "old-project");
    // Relocation is permitted before a workspace is bound to a task.
    agents.update_project(&ticket.0, dgos_agents::projects::UpdateProjectInput {
        project_id: project.id.clone(), action: "relocate".into(), value: Some(root.join("new-project").to_string_lossy().into_owned()),
    }, tokio_util::sync::CancellationToken::new()).await.unwrap();
    let relocated = host.discover_mcp(&agents, selection.clone(), permits.admit().unwrap(), ticket.clone()).await.unwrap();
    assert_eq!(relocated.instances.iter().map(|item| item.native_key.as_str()).collect::<Vec<_>>(), ["new-project"]);
    agents.update_project(&ticket.0, dgos_agents::projects::UpdateProjectInput {
        project_id: project.id.clone(), action: "relocate".into(), value: Some(root.join("old-project").to_string_lossy().into_owned()),
    }, tokio_util::sync::CancellationToken::new()).await.unwrap();
    // A task-bound native selection must operate in its workspace, never the
    // project's original root; another project's task cannot redirect writes.
    let task = agents.save_development_task(&ticket.0, dgos_agents::projects::SaveTaskInput {
        id: None, project_id: project.id.clone(), goal: "Workspace native configuration".into(), requirements: String::new(), status: "todo".into(),
    }, tokio_util::sync::CancellationToken::new()).await.unwrap();
    let workspace_path = root.join("new-project");
    #[cfg(unix)]
    let directory_identity = { use std::os::unix::fs::MetadataExt; let metadata = std::fs::metadata(&workspace_path).unwrap(); format!("{}:{}", metadata.dev(), metadata.ino()) };
    #[cfg(not(unix))]
    let directory_identity = String::new();
    let workspace = dgos_db::development_repository::StoredWorkspace {
        id: uuid::Uuid::new_v4().to_string(), owner: ticket.0.clone(), project_id: project.id.clone(),
        path: workspace_path.to_string_lossy().into_owned(), directory_identity, common_dir: None, common_identity: None,
        kind: "imported".into(), created_operation: None, base_commit: None,
    };
    repository.insert_workspace(&workspace).await.unwrap();
    repository.bind_task_workspace(&ticket.0, &task.id, &workspace.id).await.unwrap();
    let task_selection = Selection { task_id: Some(task.id.clone()), ..selection.clone() };
    let task_servers = host.discover_mcp(&agents, task_selection.clone(), permits.admit().unwrap(), ticket.clone()).await.unwrap();
    assert_eq!(task_servers.instances[0].native_key, "new-project");
    let preview = host.preview_mcp(&agents, PreviewMcp {
        target: Target::Codex, scope: Scope::ProjectShared, project_id: Some(project.id.clone()), task_id: Some(task.id),
        change: McpChange::Remove { name: "new-project".into() },
    }, permits.admit().unwrap(), ticket.clone()).await.unwrap();
    let receipt = host.operation(&agents, Operation { operation_id: preview.receipt.operation_id.as_str().into() }, OperationAction::Commit, Domain::Mcp, permits.admit().unwrap(), ticket.clone()).await.unwrap();
    assert_eq!(receipt.state, core::State::Committed);
    assert!(host.discover_mcp(&agents, task_selection, permits.admit().unwrap(), ticket.clone()).await.unwrap().instances.is_empty());
    assert_eq!(host.discover_mcp(&agents, selection.clone(), permits.admit().unwrap(), ticket.clone()).await.unwrap().instances[0].native_key, "old-project");
    std::fs::create_dir(root.join("other-project")).unwrap();
    let other_project = agents.create_project(&ticket.0, dgos_agents::projects::CreateProjectInput {
        kind: "import".into(), name: "Other workspace".into(), path: root.join("other-project").to_string_lossy().into_owned(), source: None,
    }, tokio_util::sync::CancellationToken::new()).await.unwrap();
    let other_task = agents.save_development_task(&ticket.0, dgos_agents::projects::SaveTaskInput {
        id: None, project_id: other_project.id, goal: "Foreign task".into(), requirements: String::new(), status: "todo".into(),
    }, tokio_util::sync::CancellationToken::new()).await.unwrap();
    let mismatched = Selection { task_id: Some(other_task.id), ..selection.clone() };
    assert_eq!(host.discover_mcp(&agents, mismatched, permits.admit().unwrap(), ticket.clone()).await.unwrap_err(), "native_extension_AccessDenied");
    let before = std::fs::read(root.join("codex/config.toml")).unwrap();
    let pending = host.preview_mcp(&agents, PreviewMcp {
        target: Target::Codex, scope: Scope::OsUser, project_id: None, task_id: None,
        change: McpChange::Remove { name: "local-fixture".into() },
    }, permits.admit().unwrap(), ticket.clone()).await.unwrap();
    host.resolver.close_agents();
    let rejected = host.operation(&agents, Operation { operation_id: pending.receipt.operation_id.as_str().into() }, OperationAction::Commit, Domain::Mcp, permits.admit().unwrap(), ticket).await;
    assert_eq!(rejected.unwrap_err(), "session_invalid");
    assert_eq!(std::fs::read(root.join("codex/config.toml")).unwrap(), before);
    assert!(agents.shutdown().await);
}

#[test]
fn visibility_dto_requires_matching_discriminated_intent_and_four_states() {
    let input: Visibility = serde_json::from_value(serde_json::json!({"target":"Claude","scope":"ProjectLocal","project_id":"project-a","intent":{"kind":"claude_name","name":"missing:skill","state":"inherit"}})).unwrap();
    assert!(matches!(input.intent, VisibilityIntent::ClaudeName { state: core::skills::RuleState::Inherit, .. }));
    for input in [
        serde_json::json!({"target":"Claude","scope":"OsUser","project_id":null,"instance":{"id":"x","revision":"y"},"visible":false}),
        serde_json::json!({"target":"Claude","scope":"OsUser","project_id":null,"intent":{"kind":"claude_name","name":"x","state":false}}),
        serde_json::json!({"target":"Claude","scope":"OsUser","project_id":null,"intent":{"kind":"claude_name","name":"x","state":"bad"}}),
    ] { assert!(serde_json::from_value::<Visibility>(input).is_err()); }
}
