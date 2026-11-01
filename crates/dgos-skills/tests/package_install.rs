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
