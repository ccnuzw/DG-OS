use dgos_db::{migrations::Migrator, sea_orm_migration::MigratorTrait};
use dgos_skills::SkillsManager;
use std::io::{Cursor, Write};
use tempfile::tempdir;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

fn fixture() -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    writer.start_file("SKILL.md", options).unwrap();
    writer
        .write_all(
            b"---\nname: Imported Demo\ndescription: A local package\n---\nUse this as context.\n",
        )
        .unwrap();
    writer.start_file("LICENSE", options).unwrap();
    writer.write_all(b"MIT").unwrap();
    writer.start_file("references/guide.md", options).unwrap();
    writer.write_all(b"Complete reference bytes.\n").unwrap();
    writer.finish().unwrap();
    output.into_inner()
}

#[tokio::test]
async fn local_package_preview_confirm_and_workbench_read_are_atomic() {
    let database_dir = tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database_dir.path().join("skills.db").display()
    );
    let db = dgos_db::establish_connection(&database_url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let skills_dir = tempdir().unwrap();
    let manager = SkillsManager::new(db.clone(), skills_dir.path().to_owned());

    let preview = manager
        .preview_external_package("alice", &fixture())
        .await
        .unwrap();
    assert!(preview.required_confirmation);
    assert_eq!(preview.source_kind, "local_zip");
    assert_eq!(preview.license_status, dgos_skills::LicenseStatus::Present);
    assert!(manager
        .confirm_external_install("bob", &preview.operation_id)
        .await
        .is_err());

    let installed = manager
        .confirm_external_install("alice", &preview.operation_id)
        .await
        .unwrap();
    assert_eq!(installed.skill_id, preview.skill_id);
    assert_eq!(installed.content_hash, preview.content_hash);

    let skills = manager.list_installed_skills("alice").await.unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].skill_id, preview.skill_id);
    assert!(skills[0].enabled);

    let detail = manager.get_skill_detail(&preview.skill_id).await.unwrap();
    assert_eq!(
        detail.content.unwrap().body,
        "---\nname: Imported Demo\ndescription: A local package\n---\nUse this as context.\n"
    );

    // The same preview is idempotent and does not create another staging tree.
    let second = manager
        .preview_external_package("alice", &fixture())
        .await
        .unwrap();
    assert_eq!(second.operation_id, preview.operation_id);
}

#[tokio::test]
async fn pinned_package_load_returns_verified_files_and_requires_current_enabled_version() {
    let database_dir = tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database_dir.path().join("skills.db").display()
    );
    let db = dgos_db::establish_connection(&database_url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let skills_dir = tempdir().unwrap();
    let manager = SkillsManager::new(db, skills_dir.path().to_owned());

    let preview = manager
        .preview_external_package("alice", &fixture())
        .await
        .unwrap();
    let installed = manager
        .confirm_external_install("alice", &preview.operation_id)
        .await
        .unwrap();
    let package = manager
        .load_pinned_package(
            "alice",
            &installed.skill_id,
            &installed.version,
            &installed.content_hash,
        )
        .await
        .unwrap();
    assert!(package.body().contains("Use this as context."));
    assert!(package.files().iter().any(|file| file.path == "SKILL.md"));
    assert_eq!(package.compatibility.scripts, dgos_skills::SkillPackageFeatureSupport::Unsupported);
    assert!(package.files().iter().any(|file| {
        file.path == "references/guide.md"
            && file.kind == dgos_skills::SkillPackageFileKind::Resource
            && file.bytes == b"Complete reference bytes.\n"
    }));
    assert!(!format!("{package:?}").contains("Use this as context."));
    assert_eq!(package.compatibility.hooks, dgos_skills::SkillPackageFeatureSupport::Unsupported);
    assert!(manager
        .load_pinned_package("bob", &installed.skill_id, &installed.version, &installed.content_hash)
        .await
        .is_err());
    assert!(manager
        .load_pinned_package("alice", &installed.skill_id, &installed.version, "wrong-hash")
        .await
        .is_err());
    assert!(manager
        .load_pinned_package("alice", &installed.skill_id, "9.9.9", &installed.content_hash)
        .await
        .is_err());

    manager.toggle_skill(&installed.skill_id, "alice", false).await.unwrap();
    assert!(manager
        .load_pinned_package("alice", &installed.skill_id, &installed.version, &installed.content_hash)
        .await
        .is_err());
}

#[tokio::test]
async fn pinned_package_loader_rejects_modified_snapshot_bytes() {
    let database_dir = tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database_dir.path().join("skills.db").display()
    );
    let db = dgos_db::establish_connection(&database_url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let skills_dir = tempdir().unwrap();
    let manager = SkillsManager::new(db, skills_dir.path().to_owned());
    let preview = manager.preview_external_package("alice", &fixture()).await.unwrap();
    let installed = manager.confirm_external_install("alice", &preview.operation_id).await.unwrap();
    let snapshot = skills_dir.path().join(format!(
        "external/{}/{}",
        installed.skill_id,
        &installed.content_hash[..32]
    ));
    std::fs::write(snapshot.join("SKILL.md"), b"tampered").unwrap();
    assert!(manager
        .load_pinned_package("alice", &installed.skill_id, &installed.version, &installed.content_hash)
        .await
        .is_err());
    let original = dgos_skills::validate_external_zip(&fixture()).unwrap();
    std::fs::write(snapshot.join("SKILL.md"), &original.files["SKILL.md"]).unwrap();
    let resource = snapshot.join("references/guide.md");
    std::fs::write(&resource, vec![b'a'; 256 * 1024 + 1]).unwrap();
    assert!(manager
        .load_pinned_package("alice", &installed.skill_id, &installed.version, &installed.content_hash)
        .await
        .is_err());
    std::fs::write(&resource, &original.files["references/guide.md"]).unwrap();
    std::fs::write(snapshot.join("references/unlisted.md"), b"unexpected").unwrap();
    assert!(manager
        .load_pinned_package("alice", &installed.skill_id, &installed.version, &installed.content_hash)
        .await
        .is_err());
    std::fs::remove_file(snapshot.join("references/unlisted.md")).unwrap();
    #[cfg(unix)]
    {
        std::fs::remove_file(&resource).unwrap();
        let outside = tempdir().unwrap();
        let target = outside.path().join("guide.md");
        std::fs::write(&target, &original.files["references/guide.md"]).unwrap();
        std::os::unix::fs::symlink(&target, &resource).unwrap();
        assert!(manager
            .load_pinned_package("alice", &installed.skill_id, &installed.version, &installed.content_hash)
            .await
            .is_err());
        std::fs::remove_file(&resource).unwrap();
        std::fs::hard_link(&target, &resource).unwrap();
        assert!(manager
            .load_pinned_package("alice", &installed.skill_id, &installed.version, &installed.content_hash)
            .await
            .is_err());
    }
}

#[tokio::test]
async fn external_skill_uninstall_removes_only_user_state_and_preserves_published_content() {
    let database_dir = tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database_dir.path().join("skills.db").display()
    );
    let db = dgos_db::establish_connection(&database_url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let skills_dir = tempdir().unwrap();
    let manager = SkillsManager::new(db.clone(), skills_dir.path().to_owned());

    let preview = manager
        .preview_external_package("alice", &fixture())
        .await
        .unwrap();
    let installed = manager
        .confirm_external_install("alice", &preview.operation_id)
        .await
        .unwrap();
    manager
        .preview_external_package("alice", &fixture())
        .await
        .unwrap();

    manager
        .uninstall_skill(&installed.skill_id, "bob")
        .await
        .unwrap_err();
    assert_eq!(
        manager.list_installed_skills("alice").await.unwrap().len(),
        1
    );
    let prepared = manager
        .prepare_content_skill(&installed.skill_id, "alice")
        .await
        .unwrap();
    assert_eq!(prepared.version, installed.version);
    assert_eq!(prepared.content_hash, installed.content_hash);
    assert!(prepared.body.contains("Use this as context."));
    manager
        .uninstall_skill(&installed.skill_id, "alice")
        .await
        .unwrap();
    assert!(manager
        .list_installed_skills("alice")
        .await
        .unwrap()
        .is_empty());
    assert!(manager
        .list_installed_skills("bob")
        .await
        .unwrap()
        .is_empty());
    let detail = manager.get_skill_detail(&installed.skill_id).await.unwrap();
    assert_eq!(detail.content.unwrap().content_hash, installed.content_hash);
}

#[tokio::test]
async fn uninstall_is_user_scoped_and_retains_immutable_package_files() {
    let database_dir = tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database_dir.path().join("skills.db").display()
    );
    let db = dgos_db::establish_connection(&database_url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let skills_dir = tempdir().unwrap();
    let manager = SkillsManager::new(db, skills_dir.path().to_owned());
    let preview = manager
        .preview_external_package("alice", &fixture())
        .await
        .unwrap();
    let installed = manager
        .confirm_external_install("alice", &preview.operation_id)
        .await
        .unwrap();
    let published_path = skills_dir.path().join(format!(
        "external/{}/{}",
        installed.skill_id,
        &installed.content_hash[..32]
    ));
    assert!(published_path.join("SKILL.md").is_file());

    manager
        .uninstall_skill(&installed.skill_id, "bob")
        .await
        .unwrap_err();
    assert_eq!(
        manager.list_installed_skills("alice").await.unwrap().len(),
        1
    );
    manager
        .uninstall_skill(&installed.skill_id, "alice")
        .await
        .unwrap();
    assert!(manager
        .list_installed_skills("alice")
        .await
        .unwrap()
        .is_empty());
    assert!(manager.get_skill_detail(&installed.skill_id).await.is_ok());
    assert!(published_path.join("SKILL.md").is_file());
}

#[tokio::test]
async fn uninstall_refuses_an_active_task_and_succeeds_after_task_finishes() {
    use dgos_db::sea_orm::{ConnectionTrait, DbBackend, Statement};

    let database_dir = tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database_dir.path().join("skills.db").display()
    );
    let db = dgos_db::establish_connection(&database_url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let skills_dir = tempdir().unwrap();
    let manager = SkillsManager::new(db.clone(), skills_dir.path().to_owned());
    let preview = manager
        .preview_external_package("alice", &fixture())
        .await
        .unwrap();
    let installed = manager
        .confirm_external_install("alice", &preview.operation_id)
        .await
        .unwrap();

    db.execute(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "INSERT INTO workbench_tasks (id, user_id, app_id, provider_id, model_id, status, created_at, kind) \
         VALUES ('task-active', 'alice', 'ai-workbench', 'provider', 'model', 'running', 1, 'single')",
        [],
    )).await.unwrap();
    db.execute(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "INSERT INTO workbench_task_skills (task_id, version_id, skill_id, content_hash) VALUES (?, ?, ?, ?)",
        [
            "task-active".into(),
            installed.version_id.clone().into(),
            installed.skill_id.clone().into(),
            installed.content_hash.clone().into(),
        ],
    )).await.unwrap();

    assert!(manager
        .uninstall_skill(&installed.skill_id, "alice")
        .await
        .is_err());
    assert_eq!(
        manager.list_installed_skills("alice").await.unwrap().len(),
        1
    );
    db.execute(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "UPDATE workbench_tasks SET status = 'succeeded' WHERE id = 'task-active'",
        [],
    ))
    .await
    .unwrap();
    manager
        .uninstall_skill(&installed.skill_id, "alice")
        .await
        .unwrap();
    assert!(manager
        .list_installed_skills("alice")
        .await
        .unwrap()
        .is_empty());
}
