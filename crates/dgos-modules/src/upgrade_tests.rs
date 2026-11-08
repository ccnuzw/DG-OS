// Included in supervisor::tests to reuse the real publisher and scripted loader.

/// D136 diagnostics use real SQLite files. When the coordinator supplies its private
/// evidence root, retain the directory even after a failed assertion.
fn d136_directory(label: &str) -> TempDir {
    let mut builder = tempfile::Builder::new();
    builder.prefix(label);
    match std::env::var_os("DGOS_D136_EVIDENCE_DIR") {
        Some(root) => builder.tempdir_in(root).unwrap(),
        None => builder.tempdir().unwrap(),
    }
}

async fn d136_console_fixture(enabled: bool) -> Fixture {
    let mut store = d136_directory("d136-console-upgrade-");
    if std::env::var_os("DGOS_D136_EVIDENCE_DIR").is_some() {
        store.disable_cleanup(true);
    }
    eprintln!("D136 console SQLite/store: {}", store.path().display());
    d136_console_fixture_in(store, enabled).await
}

async fn d136_console_fixture_in(store: TempDir, enabled: bool) -> Fixture {
    let url = format!("sqlite://{}?mode=rwc", store.path().join("dgos.db").display());
    let db = dgos_db::establish_connection(&url).await.unwrap();
    let boundary = Migrator::migrations().iter()
        .position(|migration| migration.name() == "m20261104_000030_task_workspaces")
        .unwrap() + 1;
    Migrator::up(&db, Some(boundary as u32)).await.unwrap();
    let publisher = Arc::new(ModulePublisher::new(store.path(), db.clone()));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bundled-modules");
    let staged = publisher.stage(&root.join("provider-console/1.0.0")).unwrap();
    publisher.commit_with(staged, crate::FirstPublish { official: true, enabled: true }).await.unwrap();
    publisher.permissions().grant("default-user", "provider-console", "providers.call").await.unwrap();
    publisher.set_enabled("provider-console", enabled).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let loader = Arc::new(ScriptedLoader::default());
    let supervisor = ModuleRuntimeSupervisor::new(publisher.clone(), loader.clone());
    Fixture { store, db, publisher, loader, supervisor }
}
#[tokio::test]
async fn stopped_unpinned_release_upgrades_and_recovers_without_loading_old_code() {
    let mut fixture=Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
    fixture.publish("1.0.0", &["providers.call"]).await;
    fixture.publisher.permissions().grant("default-user",MODULE,"providers.call").await.unwrap();
    let source=package(PackageSpec { version:"2.0.0", permissions:&["providers.call","mcp.call"], ..PackageSpec::default() });
    let integrity=package_integrity(source.path()).unwrap();
    fixture.loader.executes.store(true,Ordering::SeqCst);
    fixture.supervisor=fixture.supervisor.with_admission(Arc::new(crate::PinnedModuleAdmission::new([(MODULE,"2.0.0",integrity.as_str())])));
    assert!(matches!(fixture.supervisor.start(&HostSession::local_default(),MODULE).await,Err(ModuleSupervisorError::NotAdmitted)));
    fs::remove_file(fixture.store.path().join("current").join(MODULE).join("version")).unwrap();
    fixture.publisher.recover_registered().await.unwrap();
    assert_eq!(fixture.pointer().as_deref(),Some("1.0.0"));
    assert_eq!(fixture.update(&source).await.unwrap().outcome,ModuleUpdateOutcome::Switched { previous_stopped:true });
    assert_eq!(fixture.supervisor.runtime(MODULE).unwrap().module_identity().unwrap().1,"2.0.0");
    let module=fixture.publisher.published_modules("default-user").await.unwrap().remove(0);
    assert!(module.enabled);
    assert_eq!(module.granted,vec!["providers.call"]);
    fixture.supervisor.stop(MODULE).await.unwrap();
    fixture.publisher.recover_registered().await.unwrap();
    assert_eq!(fixture.pointer().as_deref(),Some("2.0.0"));
    assert_eq!(fixture.update(&source).await.unwrap().outcome,ModuleUpdateOutcome::Unchanged);
}

#[tokio::test]
async fn disabled_update_load_checks_candidate_and_preserves_disabled_and_grants() {
    let fixture=Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
    fixture.publish("1.0.0", &["providers.call"]).await;
    fixture.publisher.permissions().grant("default-user",MODULE,"providers.call").await.unwrap();
    fixture.publisher.set_enabled(MODULE,false).await.unwrap();
    let source=package(PackageSpec { version:"2.0.0", permissions:&["providers.call","mcp.call"], ..PackageSpec::default() });
    fixture.update(&source).await.unwrap();
    assert!(!fixture.module_row().await.is_enabled);
    assert_eq!(fixture.pointer().as_deref(),Some("2.0.0"));
    assert!(fixture.supervisor.runtime(MODULE).is_none());
    assert_eq!(fixture.loader.starts.load(Ordering::SeqCst),1);
    assert_eq!(fixture.loader.stops().len(),1);
    assert_eq!(fixture.supervisor.pending_release_count(MODULE),0);
    let module=fixture.publisher.published_modules("default-user").await.unwrap().remove(0);
    assert_eq!(module.granted,vec!["providers.call"]);
    fixture.publisher.recover_registered().await.unwrap();
    assert!(!fixture.module_row().await.is_enabled);
}

#[tokio::test]
async fn stopped_activation_failure_restores_disk_enabled_and_dropped_grants_and_can_retry() {
    let fixture=Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
    fixture.publish("1.0.0", &["providers.call"]).await;
    fixture.publisher.permissions().grant("default-user",MODULE,"providers.call").await.unwrap();
    fixture.loader.fail_start(Some("2.0.0"));
    fixture.loader.fail_next_stops(1);
    let source=package(PackageSpec { version:"2.0.0", ..PackageSpec::default() });
    assert!(matches!(fixture.update(&source).await,Err(ModuleSupervisorError::RolledBack(_))));
    assert_eq!(fixture.pointer().as_deref(),Some("1.0.0"));
    assert!(fixture.module_row().await.is_enabled);
    assert!(fixture.publisher.permissions().check("default-user",MODULE,"providers.call").await.unwrap());
    assert_eq!(fixture.supervisor.pending_release_count(MODULE),1);
    fixture.supervisor.stop(MODULE).await.unwrap();
    fixture.loader.fail_start(None);
    fixture.update(&source).await.unwrap();
    assert_eq!(fixture.pointer().as_deref(),Some("2.0.0"));
    assert!(!fixture.publisher.permissions().check("default-user",MODULE,"providers.call").await.unwrap());
}

#[tokio::test]
async fn disabled_candidate_cleanup_failure_keeps_old_release_and_pending_resource() {
    let fixture=Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
    fixture.publish("1.0.0", &[]).await;
    fixture.publisher.set_enabled(MODULE,false).await.unwrap();
    fixture.loader.fail_next_stops(1);
    let source=package(PackageSpec { version:"2.0.0", ..PackageSpec::default() });
    assert!(matches!(fixture.update(&source).await,Err(ModuleSupervisorError::Load(_))));
    assert_eq!(fixture.pointer().as_deref(),Some("1.0.0"));
    assert!(!fixture.module_row().await.is_enabled);
    assert_eq!(fixture.supervisor.pending_release_count(MODULE),1);
    fixture.supervisor.stop(MODULE).await.unwrap();
    assert_eq!(fixture.supervisor.pending_release_count(MODULE),0);
    fixture.update(&source).await.unwrap();
    assert!(!fixture.module_row().await.is_enabled);
}

#[tokio::test]
async fn stopped_update_commit_failure_leaves_pointer_row_and_grants_intact() {
    use dgos_db::sea_orm::ConnectionTrait;
    let fixture=Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
    fixture.publish("1.0.0", &["providers.call"]).await;
    fixture.publisher.permissions().grant("default-user",MODULE,"providers.call").await.unwrap();
    fixture.db.execute_unprepared("CREATE TRIGGER fail_upgrade BEFORE UPDATE ON modules WHEN NEW.current_version='2.0.0' BEGIN SELECT RAISE(ABORT,'fixture'); END;").await.unwrap();
    let source=package(PackageSpec { version:"2.0.0", ..PackageSpec::default() });
    assert!(fixture.update(&source).await.is_err());
    assert_eq!(fixture.pointer().as_deref(),Some("1.0.0"));
    assert_eq!(fixture.module_row().await.current_version,"1.0.0");
    assert!(fixture.publisher.permissions().check("default-user",MODULE,"providers.call").await.unwrap());
    assert!(fixture.supervisor.runtime(MODULE).is_none());
    fixture.db.execute_unprepared("DROP TRIGGER fail_upgrade").await.unwrap();
    fixture.update(&source).await.unwrap();
    assert_eq!(fixture.pointer().as_deref(),Some("2.0.0"));
}

#[tokio::test]
async fn postcommit_identity_conflict_does_not_disable_or_replace_concurrent_release() {
    use dgos_db::sea_orm::ConnectionTrait;
    for enabled in [false, true] {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        fixture.publisher.set_enabled(MODULE, enabled).await.unwrap();
        let previous = if enabled { Some(fixture.start().await) } else { None };
        fixture.db.execute_unprepared(
            "CREATE TRIGGER conflict_after_upgrade AFTER UPDATE OF current_version ON modules WHEN NEW.current_version='2.0.0' BEGIN UPDATE modules SET current_version='3.0.0' WHERE id=NEW.id; END;"
        ).await.unwrap();
        let source = package(PackageSpec { version: "2.0.0", ..PackageSpec::default() });

        assert!(matches!(fixture.supervisor.update_with_operation_id(
            &HostSession::local_default(), MODULE, source.path(), "conflict-op"
        ).await, Err(ModuleSupervisorError::RollbackFailed { module_disabled: false })));
        let row = fixture.module_row().await;
        assert_eq!(row.current_version, "3.0.0");
        assert_eq!(row.is_enabled, enabled);
        assert_eq!(fixture.pointer().as_deref(), Some("2.0.0"));
        match previous {
            Some(previous) => {
                assert_eq!(previous.state(), ModuleRuntimeState::Ready);
                assert!(previous.begin_request().is_ok());
                assert!(fixture.supervisor.runtime(MODULE).unwrap().is_same_runtime(&previous));
            }
            None => assert!(fixture.supervisor.runtime(MODULE).is_none()),
        }
        assert!(fixture.events().await.iter().all(|(stage, _)| stage != "disabled"));
    }
}

#[tokio::test]
async fn postcommit_snapshot_read_failure_rolls_back_and_keeps_old_runtime() {
    use dgos_db::sea_orm::ConnectionTrait;
    for enabled in [false, true] {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &["providers.call"]).await;
        fixture.publisher.permissions().grant("default-user", MODULE, "providers.call").await.unwrap();
        fixture.publisher.set_enabled(MODULE, enabled).await.unwrap();
        let previous = if enabled { Some(fixture.start().await) } else { None };
        fixture.db.execute_unprepared(
            "CREATE TRIGGER corrupt_upgrade_manifest AFTER INSERT ON module_update_events WHEN NEW.operation_id='read-failure-op' AND NEW.stage='committed' BEGIN UPDATE modules SET manifest=x'FF' WHERE id=NEW.module_id; END;"
        ).await.unwrap();
        let source = package(PackageSpec { version: "2.0.0", permissions: &["mcp.call"], ..PackageSpec::default() });

        let result = fixture.supervisor.update_with_operation_id(
            &HostSession::local_default(), MODULE, source.path(), "read-failure-op"
        ).await;
        assert!(matches!(result, Err(ModuleSupervisorError::RolledBack(_))), "unexpected update result: {result:?}");
        assert_eq!(fixture.pointer().as_deref(), Some("1.0.0"));
        let row = fixture.module_row().await;
        assert_eq!(row.current_version, "1.0.0");
        assert_eq!(row.is_enabled, enabled);
        assert_eq!(fixture.publisher.published_modules("default-user").await.unwrap()[0].granted, vec!["providers.call"]);
        match previous {
            Some(previous) => {
                assert_eq!(previous.state(), ModuleRuntimeState::Ready);
                assert!(previous.begin_request().is_ok());
                assert!(fixture.supervisor.runtime(MODULE).unwrap().is_same_runtime(&previous));
            }
            None => assert!(fixture.supervisor.runtime(MODULE).is_none()),
        }
    }
}

#[tokio::test]
async fn official_bundle_is_rechecked_and_cannot_stage_replaced_bytes() {
    let fixture=Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
    fixture.publish("1.0.0", &[]).await;
    let root=tempdir().unwrap();
    let source=package(PackageSpec { version:"2.0.0", ..PackageSpec::default() });
    crate::copy_dir(source.path(),&root.path().join(MODULE).join("2.0.0")).unwrap();
    let digest=package_integrity(source.path()).unwrap();
    let sources=crate::BuiltinModuleSources::for_test(root.path().into(),&[(MODULE,"2.0.0",digest.as_str())]).unwrap();
    fs::write(root.path().join(MODULE).join("2.0.0/entry.js"),"tampered").unwrap();
    assert!(matches!(fixture.supervisor.update_builtin_with_operation_id(&HostSession::local_default(),MODULE,&sources,"bundle-op").await,Err(ModuleSupervisorError::NotAdmitted)));
    assert_eq!(fixture.pointer().as_deref(),Some("1.0.0"));
    crate::copy_dir(source.path(),&root.path().join(MODULE).join("2.0.0")).unwrap();
    let report=fixture.supervisor.update_builtin_with_operation_id(&HostSession::local_default(),MODULE,&sources,"bundle-op").await.unwrap();
    assert_eq!(report.operation_id,"bundle-op");
    assert_eq!(fixture.pointer().as_deref(),Some("2.0.0"));
}

#[tokio::test]
async fn full_bundle_rejects_extra_package_even_when_selected_package_is_valid() {
    let root=tempdir().unwrap();
    let source=package(PackageSpec { version:"2.0.0", ..PackageSpec::default() });
    crate::copy_dir(source.path(),&root.path().join(MODULE).join("2.0.0")).unwrap();
    let digest=package_integrity(source.path()).unwrap();
    let sources=crate::BuiltinModuleSources::for_test(root.path().into(),&[(MODULE,"2.0.0",digest.as_str())]).unwrap();
    crate::copy_dir(source.path(),&root.path().join("extra").join("2.0.0")).unwrap();
    assert!(matches!(sources.resolve(MODULE),Err(crate::ModuleSourceError::Rejected)));
    assert!(crate::BuiltinModuleSources::for_test(root.path().into(),&[(MODULE,"2.0.0",digest.as_str())]).is_err());
}

#[tokio::test]
async fn disabled_load_failure_never_publishes_or_enables_candidate() {
    let fixture=Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
    fixture.publish("1.0.0", &["providers.call"]).await;
    fixture.publisher.permissions().grant("default-user",MODULE,"providers.call").await.unwrap();
    fixture.publisher.set_enabled(MODULE,false).await.unwrap();
    fixture.loader.fail_start(Some("2.0.0"));
    let source=package(PackageSpec { version:"2.0.0", permissions:&["mcp.call"], ..PackageSpec::default() });
    assert!(fixture.update(&source).await.is_err());
    assert_eq!(fixture.pointer().as_deref(),Some("1.0.0"));
    assert_eq!(fixture.module_row().await.current_version,"1.0.0");
    assert!(!fixture.module_row().await.is_enabled);
    assert!(fixture.supervisor.runtime(MODULE).is_none());
    assert_eq!(fixture.supervisor.pending_release_count(MODULE),0);
    assert_eq!(fixture.publisher.published_modules("default-user").await.unwrap()[0].granted,vec!["providers.call"]);
}

#[tokio::test]
async fn rollback_restores_unpinned_old_release_on_disk_without_executing_it() {
    let mut fixture=Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
    fixture.publish("1.0.0", &[]).await;
    let source=package(PackageSpec { version:"2.0.0", ..PackageSpec::default() });
    let digest=package_integrity(source.path()).unwrap();
    fixture.supervisor=fixture.supervisor.with_admission(Arc::new(crate::PinnedModuleAdmission::new([(MODULE,"2.0.0",digest.as_str())])));
    fixture.loader.executes.store(true,Ordering::SeqCst);
    fixture.loader.fail_start(Some("2.0.0"));
    assert!(matches!(fixture.update(&source).await,Err(ModuleSupervisorError::RolledBack(_))));
    assert_eq!(fixture.pointer().as_deref(),Some("1.0.0"));
    assert!(fixture.module_row().await.is_enabled);
    assert_eq!(fixture.loader.starts.load(Ordering::SeqCst),0);
    assert!(fixture.supervisor.runtime(MODULE).is_none());
    assert!(matches!(fixture.supervisor.start(&HostSession::local_default(),MODULE).await,Err(ModuleSupervisorError::NotAdmitted)));
}

#[tokio::test]
async fn real_console_upgrade_preserves_enabled_state_and_permissions_without_running_retired_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bundled-modules");
    for enabled in [false, true] {
        let fixture = d136_console_fixture(enabled).await;
        let id = "provider-console";
        fixture.loader.executes.store(true, Ordering::SeqCst);
        let supervisor = ModuleRuntimeSupervisor::new(fixture.publisher.clone(), fixture.loader.clone())
            .with_admission(Arc::new(crate::PinnedModuleAdmission::builtin()));
        let source = crate::BuiltinModuleSources::new(&root).unwrap();
        assert_eq!(source.resolve(id).unwrap().1, "1.1.0");
        let provision = fixture.publisher.provision_builtin(&root).await.unwrap();
        assert_eq!(provision.kept_other_release, [(id.to_owned(), "1.1.0".to_owned())]);
        assert_eq!(fixture.publisher.published_modules("default-user").await.unwrap()[0].version, "1.0.0");
        let retained = fixture.publisher.published_modules("default-user").await.unwrap().remove(0);
        assert_eq!(retained.enabled, enabled);
        assert_eq!(retained.granted, ["providers.call"]);
        let rejected = supervisor.start(&HostSession::local_default(), id).await;
        if enabled { assert!(matches!(rejected, Err(ModuleSupervisorError::NotAdmitted))); }
        else { assert!(rejected.is_err()); }
        assert_eq!(fixture.loader.starts.load(Ordering::SeqCst), 0);
        let report = supervisor.update_builtin_with_operation_id(&HostSession::local_default(), id, &source, "console-upgrade").await.unwrap();
        assert_eq!(report.outcome, ModuleUpdateOutcome::Switched { previous_stopped: true });
        let row = fixture.publisher.published_modules("default-user").await.unwrap().remove(0);
        assert_eq!(row.version, "1.1.0");
        assert_eq!(row.enabled, enabled);
        assert_eq!(row.granted, ["providers.call"]);
        assert_eq!(row.declared, ["providers.call", "skills.execute", "mcp.call"]);
        assert_eq!(fixture.loader.starts.load(Ordering::SeqCst), 1);
        assert_eq!(supervisor.runtime(id).is_some(), enabled);
        if enabled { supervisor.stop(id).await.unwrap(); }
        fixture.publisher.recover_registered().await.unwrap();
        assert_eq!(fs::read_to_string(fixture.store.path().join("current").join(id).join("version")).unwrap().trim(), "1.1.0");
        assert!(fixture.store.path().join("versions").join(id).join("1.0.0/entry.js").exists());
        assert_eq!(supervisor.update_builtin_with_operation_id(&HostSession::local_default(), id, &source, "console-idempotent").await.unwrap().outcome, ModuleUpdateOutcome::Unchanged);
    }
}

#[tokio::test]
async fn real_console_activation_failure_restores_old_release_and_grants_without_execution() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bundled-modules");
    let fixture = d136_console_fixture(true).await;
    let id = "provider-console";
    fixture.loader.executes.store(true, Ordering::SeqCst);
    fixture.loader.fail_start(Some("1.1.0"));
    let supervisor = ModuleRuntimeSupervisor::new(fixture.publisher.clone(), fixture.loader.clone())
        .with_admission(Arc::new(crate::PinnedModuleAdmission::builtin()));
    let source = crate::BuiltinModuleSources::new(&root).unwrap();
    let failure = supervisor.update_builtin_with_operation_id(&HostSession::local_default(), id, &source, "console-fail").await.unwrap_err();
    fs::write(fixture.store.path().join("upgrade-failure.txt"), failure.to_string()).unwrap();
    assert!(matches!(failure, ModuleSupervisorError::RolledBack(_)));
    {
        use dgos_db::sea_orm::{ConnectionTrait, DbBackend, Statement};
        fixture.db.execute(Statement::from_sql_and_values(DbBackend::Sqlite, "VACUUM INTO ?",
            [fixture.store.path().join("failed-upgrade.sqlite").to_string_lossy().into_owned().into()])).await.unwrap();
    }
    let row = fixture.publisher.published_modules("default-user").await.unwrap().remove(0);
    assert_eq!(row.version, "1.0.0");
    assert!(row.enabled && row.official);
    assert_eq!(row.granted, ["providers.call"]);
    assert_eq!(fixture.loader.starts.load(Ordering::SeqCst), 0);
    fs::write(fixture.store.path().join("failed-upgrade-state.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "version": row.version, "enabled": row.enabled, "official": row.official,
        "granted": row.granted, "starts": fixture.loader.starts.load(Ordering::SeqCst),
        "pointer": fs::read_to_string(fixture.store.path().join("current").join(id).join("version")).unwrap()
    })).unwrap()).unwrap();
    fixture.publisher.recover_registered().await.unwrap();
    assert_eq!(fs::read_to_string(fixture.store.path().join("current").join(id).join("version")).unwrap().trim(), "1.0.0");
    assert!(matches!(supervisor.start(&HostSession::local_default(), id).await, Err(ModuleSupervisorError::NotAdmitted)));
    fixture.loader.fail_start(None);
    supervisor.update_builtin_with_operation_id(&HostSession::local_default(), id, &source, "console-retry").await.unwrap();
    assert_eq!(fixture.publisher.published_modules("default-user").await.unwrap()[0].version, "1.1.0");
}
