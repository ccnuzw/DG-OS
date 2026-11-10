use dgos_db::{
    entities::{skill_usage_logs, skill_user_states, skill_versions},
    migrations::Migrator,
    sea_orm::{ActiveModelTrait, Database, EntityTrait, Set},
    sea_orm_migration::MigratorTrait,
};
use dgos_skills::{recommend_skills, validate_skill_read_path, SkillCatalogEntry, SkillsManager};
use dgos_types::AiSkillRef;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use zip::{write::SimpleFileOptions, ZipWriter};

fn archive(marker: &str) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (path, text) in [
        ("SKILL.md", "---\nname: Meeting Notes\ndescription: Summarize meeting notes\n---\nRead references/marker.txt as data only.\n"),
        ("references/marker.txt", marker), ("templates/outline.md", "# Meeting\n"),
        ("examples/result.json", "{\"example\":true}"),
    ] {
        writer.start_file(path, SimpleFileOptions::default()).unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

async fn setup() -> (SkillsManager, dgos_db::sea_orm::DatabaseConnection, tempfile::TempDir, AiSkillRef, String) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let manager = SkillsManager::new(db.clone(), dir.path().to_path_buf());
    let marker = format!("skill-resource-proof:{}", uuid::Uuid::new_v4());
    let preview = manager.preview_external_package("alice", &archive(&marker)).await.unwrap();
    let installed = manager.confirm_external_install("alice", &preview.operation_id).await.unwrap();
    let pin = AiSkillRef { skill_id: installed.skill_id, version: installed.version, content_hash: installed.content_hash };
    (manager, db, dir, pin, marker)
}

#[tokio::test]
async fn catalogue_is_metadata_only_and_recommendations_are_deterministic() {
    let (manager, db, dir, pin, _) = setup().await;
    let catalog = manager.chat_skill_catalog("alice").await.unwrap();
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].pin, pin);
    assert!(manager.chat_skill_catalog("bob").await.unwrap().is_empty());
    // A removed body proves discovery never performs eager package preparation.
    let snapshot = dir.path().join(format!("external/{}/{}", pin.skill_id, &pin.content_hash[..32]));
    std::fs::remove_file(snapshot.join("SKILL.md")).unwrap();
    assert_eq!(manager.chat_skill_catalog("alice").await.unwrap()[0].pin, pin);
    assert_eq!(manager.ai_content_catalog("alice").await.unwrap().len(), 1);
    assert!(manager.read_pinned_skill_text("alice", &pin, "references/marker.txt", 1024).await.is_err());
    assert!(skill_usage_logs::Entity::find().all(&db).await.unwrap().is_empty());
    let suggestions = recommend_skills(&catalog, "meeting notes", 10);
    assert_eq!(suggestions.len(), 1);
    assert!(suggestions[0].reasons.iter().all(|reason| reason.starts_with("lexical_")));
    assert_eq!(serde_json::to_value(&suggestions).unwrap(), serde_json::to_value(recommend_skills(&catalog, "meeting notes", 10)).unwrap());
    assert!(recommend_skills(&catalog, "zzzz-unrelated", 10).is_empty());
    manager.toggle_skill(&pin.skill_id, "alice", false).await.unwrap();
    assert!(manager.chat_skill_catalog("alice").await.unwrap().is_empty());
}

#[tokio::test]
async fn actual_marker_read_has_exact_content_free_receipt_and_audit() {
    let (manager, db, _dir, pin, marker) = setup().await;
    let loaded = manager.read_pinned_skill_text("alice", &pin, "references/marker.txt", 4096).await.unwrap();
    assert_eq!(loaded.content, marker);
    assert_eq!(loaded.receipt.pin, pin);
    assert_eq!(loaded.receipt.bytes, marker.len());
    assert_eq!(loaded.receipt.sha256, format!("{:x}", Sha256::digest(marker.as_bytes())));
    assert!(!format!("{loaded:?}").contains(&marker));
    assert!(!serde_json::to_string(&loaded.receipt).unwrap().contains(&marker));
    let logs = skill_usage_logs::Entity::find().all(&db).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].output_result, Some(serde_json::to_value(&loaded.receipt).unwrap()));
    assert!(!format!("{logs:?}").contains(&marker));
    assert!(manager.read_pinned_skill_text("alice", &pin, "templates/outline.md", 4096).await.is_ok());
    assert!(manager.read_pinned_skill_text("alice", &pin, "examples/result.json", 4096).await.is_ok());
    assert!(manager.read_pinned_skill_text("alice", &pin, "SKILL.md", 4096).await.unwrap().content.contains("references/marker.txt"));
}

#[tokio::test]
async fn reads_reject_authorization_pin_mutation_and_byte_budget() {
    let (manager, db, _dir, pin, marker) = setup().await;
    assert!(manager.read_pinned_skill_text("bob", &pin, "references/marker.txt", 4096).await.is_err());
    assert!(manager.read_pinned_skill_text("alice", &pin, "references/marker.txt", marker.len() - 1).await.is_err());
    let mut changed = pin.clone();
    changed.content_hash = "0".repeat(64);
    assert!(manager.read_pinned_skill_text("alice", &changed, "SKILL.md", 4096).await.is_err());
    changed = pin.clone();
    changed.version = "different".into();
    assert!(manager.read_pinned_skill_text("alice", &changed, "SKILL.md", 4096).await.is_err());
    manager.toggle_skill(&pin.skill_id, "alice", false).await.unwrap();
    assert!(manager.read_pinned_skill_text("alice", &pin, "SKILL.md", 4096).await.is_err());
    manager.toggle_skill(&pin.skill_id, "alice", true).await.unwrap();
    let version = skill_versions::Entity::find_by_id(format!("{}@{}", pin.skill_id, pin.version)).one(&db).await.unwrap().unwrap();
    let mut replacement: skill_versions::ActiveModel = version.clone().into();
    replacement.id = Set(format!("{}@replacement", pin.skill_id));
    replacement.version = Set("replacement".into());
    replacement.insert(&db).await.unwrap();
    let mut state: skill_user_states::ActiveModel = skill_user_states::Entity::find_by_id(("alice".to_string(), pin.skill_id.clone())).one(&db).await.unwrap().unwrap().into();
    state.version_id = Set(format!("{}@replacement", pin.skill_id));
    state.update(&db).await.unwrap();
    assert!(manager.read_pinned_skill_text("alice", &pin, "SKILL.md", 4096).await.is_err());
    assert!(skill_usage_logs::Entity::find().all(&db).await.unwrap().is_empty());
}

#[tokio::test]
async fn resources_reject_traversal_cross_package_tampering_oversize_and_links() {
    let (manager, _, dir, pin, marker) = setup().await;
    for path in ["../SKILL.md", "/SKILL.md", "references/../SKILL.md", "references//marker.txt", "references/./marker.txt", "references\\marker.txt", "references/%2e%2e/secret.txt", "references/marker.txt/", "other/references/marker.txt", "skill.json", "LICENSE", "scripts/run.sh", "references/no.txt"] {
        assert!(manager.read_pinned_skill_text("alice", &pin, path, 4096).await.is_err(), "accepted {path}");
    }
    let file = dir.path().join(format!("external/{}/{}/references/marker.txt", pin.skill_id, &pin.content_hash[..32]));
    std::fs::write(&file, "tampered resource").unwrap();
    assert!(manager.read_pinned_skill_text("alice", &pin, "references/marker.txt", 4096).await.is_err());
    std::fs::write(&file, vec![b'x'; 256 * 1024 + 1]).unwrap();
    assert!(manager.read_pinned_skill_text("alice", &pin, "references/marker.txt", usize::MAX).await.is_err());
    #[cfg(unix)] {
        std::fs::remove_file(&file).unwrap();
        let outside = dir.path().join("outside.txt");
        std::fs::write(&outside, &marker).unwrap();
        std::os::unix::fs::symlink(&outside, &file).unwrap();
        assert!(manager.read_pinned_skill_text("alice", &pin, "references/marker.txt", 4096).await.is_err());
        std::fs::remove_file(&file).unwrap();
        std::fs::hard_link(&outside, &file).unwrap();
        assert!(manager.read_pinned_skill_text("alice", &pin, "references/marker.txt", 4096).await.is_err());
    }
}

#[test]
fn recommendations_are_lexical_stably_ordered_and_support_chinese_metadata() {
    let entry = |id: &str| SkillCatalogEntry {
        pin: AiSkillRef { skill_id: id.into(), version: "1".into(), content_hash: "h".into() },
        name: "会议纪要".into(), description: "整理会议资料".into(), category: "writing".into(), tags: vec!["会议".into()],
    };
    let results = recommend_skills(&[entry("b"), entry("a")], "请整理会议记录", 2);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].entry.pin.skill_id, "a");
    assert!(results[0].reasons.iter().any(|reason| reason.contains("会议")));
    for path in ["references/ok.md", "templates/ok.txt", "examples/ok.json", "SKILL.md"] { assert!(validate_skill_read_path(path).is_ok()); }
}

#[tokio::test]
async fn changed_installed_hash_or_required_tools_invalidates_consumption() {
    let (manager, db, _dir, pin, _) = setup().await;
    let id = format!("{}@{}", pin.skill_id, pin.version);
    let row = skill_versions::Entity::find_by_id(&id).one(&db).await.unwrap().unwrap();
    let mut changed: skill_versions::ActiveModel = row.clone().into();
    changed.content_hash = Set("0".repeat(64));
    changed.update(&db).await.unwrap();
    assert!(manager.validate_chat_skill_pin("alice", &pin).await.is_err());
    assert!(manager.read_pinned_skill_text("alice", &pin, "SKILL.md", 4096).await.is_err());
    let mut metadata = row.metadata.clone();
    metadata["required_tools"] = serde_json::json!(["unavailable_tool"]);
    let mut changed: skill_versions::ActiveModel = row.into();
    changed.content_hash = Set(pin.content_hash.clone());
    changed.metadata = Set(metadata);
    changed.update(&db).await.unwrap();
    assert!(manager.chat_skill_catalog("alice").await.unwrap().is_empty());
    assert!(manager.validate_chat_skill_pin("alice", &pin).await.is_err());
    assert!(manager.read_pinned_skill_text("alice", &pin, "SKILL.md", 4096).await.is_err());
}
