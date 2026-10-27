use dgos_db::{
    migrations::Migrator,
    sea_orm::{Database, EntityTrait, PaginatorTrait},
    sea_orm_migration::MigratorTrait,
};
use dgos_skills::{
    content_package::{load_bundled_catalog, safe_child, validate_package},
    SkillsManager,
};
use std::path::{Path, PathBuf};

fn source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills")
}
async fn setup() -> (
    SkillsManager,
    dgos_db::sea_orm::DatabaseConnection,
    tempfile::TempDir,
) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    (
        SkillsManager::new(db.clone(), dir.path().join("skills")),
        db,
        dir,
    )
}
fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[tokio::test]
async fn ac16_offline_initialization_search_and_safe_preview() {
    let (manager, _, _dir) = setup().await;
    assert_eq!(
        manager.register_bundled_catalog(&source()).await.unwrap(),
        6
    );
    let snapshot = manager.fetch_registry().await.unwrap();
    assert_eq!(snapshot.registry_url, "bundled://official");
    assert!(!snapshot.offline);
    assert_eq!(snapshot.skills.len(), 6);
    assert!(snapshot
        .skills
        .iter()
        .all(|s| s.identity == "dgos_official"));
    assert_eq!(manager.search_skills("meeting").await.unwrap().len(), 1);
    let detail = manager
        .get_skill_detail("dgos.official.writing.summarize")
        .await
        .unwrap();
    assert_eq!(detail.format, "skill_md");
    assert!(detail.content.unwrap().body.contains("name: summarize"));
}

#[tokio::test]
async fn ac19_initialization_preserves_disabled_user_state_and_user_isolation() {
    let (manager, db, _dir) = setup().await;
    manager.register_bundled_catalog(&source()).await.unwrap();
    let id = "dgos.official.writing.summarize";
    manager.install_skill(id, "alice").await.unwrap();
    manager.install_skill(id, "alice").await.unwrap();
    manager.toggle_skill(id, "alice", false).await.unwrap();
    manager.register_bundled_catalog(&source()).await.unwrap();
    assert_eq!(
        dgos_db::entities::skill_versions::Entity::find()
            .count(&db)
            .await
            .unwrap(),
        6
    );
    assert_eq!(
        dgos_db::entities::skills::Entity::find()
            .count(&db)
            .await
            .unwrap(),
        0
    );
    assert!(!manager.list_installed_skills("alice").await.unwrap()[0].enabled);
    assert!(manager
        .list_installed_skills("bob")
        .await
        .unwrap()
        .is_empty());
    assert!(manager.prepare_content_skill(id, "bob").await.is_err());
    assert!(manager.prepare_content_skill(id, "alice").await.is_err());
    manager.toggle_skill(id, "alice", true).await.unwrap();
    assert!(manager
        .prepare_content_skill(id, "alice")
        .await
        .unwrap()
        .body
        .contains("执行步骤"));
    assert!(manager.uninstall_skill(id, "alice").await.is_err());
    assert!(manager
        .call_cli_skill(id, "alice", serde_json::json!({}))
        .await
        .is_err());
}

#[tokio::test]
async fn ac23_tampered_snapshot_is_not_loaded_or_replaced_on_restart() {
    let (manager, _, _dir) = setup().await;
    manager.register_bundled_catalog(&source()).await.unwrap();
    let id = "dgos.official.writing.summarize";
    let installed = manager.install_skill(id, "alice").await.unwrap();
    std::fs::write(
        PathBuf::from(&installed.install_path).join("SKILL.md"),
        "tampered",
    )
    .unwrap();
    assert!(manager.prepare_content_skill(id, "alice").await.is_err());
    assert!(manager.register_bundled_catalog(&source()).await.is_err());
}

#[test]
fn ac23_rejects_tampered_metadata_and_unsupported_resources() {
    let packages = load_bundled_catalog(&source()).unwrap();
    let package = &packages[0];
    let dir = tempfile::tempdir().unwrap();
    copy_tree(Path::new(&package.install_path), dir.path());
    validate_package(dir.path(), &package.content_hash).unwrap();
    std::fs::write(dir.path().join("skill.json"), "{}").unwrap();
    assert!(validate_package(dir.path(), &package.content_hash).is_err());
    copy_tree(Path::new(&package.install_path), dir.path());
    std::fs::create_dir(dir.path().join("scripts")).unwrap();
    std::fs::write(dir.path().join("scripts/run.sh"), "echo unsafe").unwrap();
    assert!(validate_package(dir.path(), &package.content_hash).is_err());
}

#[test]
fn ac23_rejects_path_traversal_and_changed_catalog() {
    let dir = tempfile::tempdir().unwrap();
    assert!(safe_child(dir.path(), "../SKILL.md").is_err());
    assert!(safe_child(dir.path(), "/etc/passwd").is_err());
    assert!(safe_child(dir.path(), "..\\SKILL.md").is_err());
    std::fs::write(dir.path().join("catalog.v1.json"), "{\"skills\": []}").unwrap();
    assert!(load_bundled_catalog(dir.path()).is_err());

    let package = load_bundled_catalog(&source()).unwrap().remove(0);
    let deep = tempfile::tempdir().unwrap();
    copy_tree(Path::new(&package.install_path), deep.path());
    std::fs::create_dir_all(deep.path().join("references/a/b/c/d/e/f/g/h")).unwrap();
    assert!(validate_package(deep.path(), &package.content_hash).is_err());
    let wide = tempfile::tempdir().unwrap();
    copy_tree(Path::new(&package.install_path), wide.path());
    for index in 0..256 {
        std::fs::create_dir_all(wide.path().join(format!("references/dir-{index}"))).unwrap();
    }
    assert!(validate_package(wide.path(), &package.content_hash).is_err());
}

#[cfg(unix)]
#[test]
fn ac23_rejects_symlinks_and_oversized_resources() {
    let package = load_bundled_catalog(&source()).unwrap().remove(0);
    let dir = tempfile::tempdir().unwrap();
    copy_tree(Path::new(&package.install_path), dir.path());
    std::fs::remove_file(dir.path().join("SKILL.md")).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", dir.path().join("SKILL.md")).unwrap();
    assert!(validate_package(dir.path(), &package.content_hash).is_err());
    std::fs::remove_file(dir.path().join("SKILL.md")).unwrap();
    std::fs::write(dir.path().join("SKILL.md"), vec![b'a'; 256 * 1024 + 1]).unwrap();
    assert!(validate_package(dir.path(), &package.content_hash).is_err());
}

#[tokio::test]
async fn migration_keeps_legacy_rows_and_old_data_is_not_executed() {
    use dgos_db::sea_orm::{ActiveModelTrait, Set};
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, Some(4)).await.unwrap();
    dgos_db::entities::skill_registry_cache::ActiveModel {
        id: Set("old".into()),
        skill_id: Set("old".into()),
        name: Set("Legacy".into()),
        description: Set(None),
        latest_version: Set("1".into()),
        author: Set(None),
        category: Set("old".into()),
        tags: Set(serde_json::json!([])),
        download_url: Set("https://invalid.test".into()),
        manifest: Set(serde_json::json!({})),
        synced_at: Set(1),
    }
    .insert(&db)
    .await
    .unwrap();
    Migrator::up(&db, None).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let manager = SkillsManager::new(db.clone(), dir.path().into());
    manager.register_bundled_catalog(&source()).await.unwrap();
    assert_eq!(
        dgos_db::entities::skill_registry_cache::Entity::find()
            .count(&db)
            .await
            .unwrap(),
        1
    );
    assert!(manager.install_skill("old", "alice").await.is_err());
    assert!(manager
        .call_http_skill("old", "alice", serde_json::json!({}))
        .await
        .is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn ac23_snapshot_parent_symlinks_cannot_redirect_publication() {
    let (manager, db, dir) = setup().await;
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("skills")).unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("skills/bundled")).unwrap();
    assert!(manager.register_bundled_catalog(&source()).await.is_err());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    assert_eq!(
        dgos_db::entities::skill_versions::Entity::find()
            .count(&db)
            .await
            .unwrap(),
        0
    );
}
