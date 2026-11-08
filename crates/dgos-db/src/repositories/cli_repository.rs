//! Account-scoped CLI preferences and durable manual detection, using one CAS snapshot.
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_REVISION: u64 = 9_007_199_254_740_991;
const TOOLS: [&str; 4] = ["codex", "claude", "gemini", "node"];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CliPaths { pub codex: Option<String>, pub claude: Option<String>, pub gemini: Option<String>, pub node: Option<String> }
impl CliPaths {
    fn path(&self, tool: &str) -> Result<&Option<String>, String> {
        match tool { "codex" => Ok(&self.codex), "claude" => Ok(&self.claude), "gemini" => Ok(&self.gemini), "node" => Ok(&self.node), _ => Err("cli_invalid_tool".into()) }
    }
    fn path_mut(&mut self, tool: &str) -> Result<&mut Option<String>, String> {
        match tool { "codex" => Ok(&mut self.codex), "claude" => Ok(&mut self.claude), "gemini" => Ok(&mut self.gemini), "node" => Ok(&mut self.node), _ => Err("cli_invalid_tool".into()) }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CliUpdateMetadata {
    pub status: String,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub method: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CliDetection {
    pub status: String,
    pub path: Option<String>,
    pub version: Option<String>,
    pub update: Option<CliUpdateMetadata>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CliSnapshot {
    pub revision: u64,
    pub paths: CliPaths,
    #[serde(default)]
    pub detections: BTreeMap<String, CliDetection>,
}
#[derive(Serialize)]
pub struct CliSave { pub saved: bool, pub snapshot: CliSnapshot }
#[derive(Clone)]
pub struct CliRepository { db: DatabaseConnection }
fn sql(query: &str, values: Vec<sea_orm::Value>) -> Statement { Statement::from_sql_and_values(DbBackend::Sqlite, query, values) }
fn validate_path(path: &Option<String>) -> Result<(), String> {
    if path.as_ref().is_some_and(|path| path.is_empty() || path.len() > 4096 || path.contains('\0') || !std::path::Path::new(path).is_absolute()) { return Err("cli_invalid_path".into()); }
    Ok(())
}
impl CliRepository {
    pub fn new(db: DatabaseConnection) -> Self { Self { db } }
    pub async fn get(&self, user: &str) -> Result<CliSnapshot, String> {
        let row = self.db.query_one(sql("SELECT setting_value FROM system_settings WHERE user_id=? AND setting_key='cli.paths.v1'", vec![user.into()])).await.map_err(|_| "cli_storage_failed")?;
        let snapshot: CliSnapshot = match row {
            None => CliSnapshot::default(),
            Some(row) => serde_json::from_str(&row.try_get::<String>("", "setting_value").map_err(|_| "cli_storage_failed")?).map_err(|_| "cli_preferences_invalid")?,
        };
        if snapshot.revision > MAX_REVISION { return Err("cli_preferences_invalid".into()); }
        Ok(snapshot)
    }
    pub async fn save(&self, user: &str, paths: CliPaths, expected: u64) -> Result<CliSave, String> {
        self.save_verified(user, paths, expected, None, || true).await
    }
    /// The host supplies a fresh Node probe only when changing its selected executable.
    /// Paths and the verified version are committed in the same revision, or neither is.
    pub async fn save_verified(&self, user: &str, paths: CliPaths, expected: u64, node: Option<CliDetection>, can_write: impl Fn() -> bool) -> Result<CliSave, String> {
        for tool in TOOLS { validate_path(paths.path(tool)?)?; }
        let mut snapshot = self.get(user).await?;
        if snapshot.revision != expected { return Ok(CliSave { saved: false, snapshot }); }
        snapshot.detections.retain(|tool, record| paths.path(tool).is_ok_and(|path| path == &record.path));
        if let Some(mut node) = node {
            if node.status != "available" || node.version.is_none() || node.path.is_none() || node.path != paths.node { return Err("cli_invalid_node".into()); }
            node.update = None;
            snapshot.detections.insert("node".into(), node);
        }
        snapshot.paths = paths;
        self.commit(user, snapshot, expected, &can_write).await
    }
    /// Retry only the per-tool mutation, never an old whole snapshot, after a CAS loss.
    pub async fn record_detection(&self, user: &str, tool: &str, mut record: CliDetection, can_write: impl Fn() -> bool) -> Result<CliSnapshot, String> {
        validate_path(&record.path)?;
        record.update = None;
        loop {
            let mut snapshot = self.get(user).await?;
            let expected = snapshot.revision;
            if snapshot.paths.path(tool)? == &record.path && snapshot.detections.get(tool) == Some(&record) {
                if !can_write() { return Err("session_invalid".into()); }
                return Ok(snapshot);
            }
            *snapshot.paths.path_mut(tool)? = record.path.clone();
            snapshot.detections.insert(tool.to_owned(), record.clone());
            let saved = self.commit(user, snapshot, expected, &can_write).await?;
            if saved.saved { return Ok(saved.snapshot); }
        }
    }
    /// A check is only a hint about the exact persisted detection that it examined.
    /// This type deliberately has no update ticket; tickets are process-local authority.
    pub async fn record_update(&self, user: &str, tool: &str, path: Option<&str>, version: Option<&str>, update: CliUpdateMetadata, can_write: impl Fn() -> bool) -> Result<bool, String> {
        loop {
            let mut snapshot = self.get(user).await?;
            let expected = snapshot.revision;
            if snapshot.paths.path(tool)?.as_deref() != path { return Ok(false); }
            let Some(record) = snapshot.detections.get_mut(tool) else { return Ok(false); };
            if record.path.as_deref() != path || record.version.as_deref() != version { return Ok(false); }
            if record.update.as_ref() == Some(&update) {
                if !can_write() { return Err("session_invalid".into()); }
                return Ok(true);
            }
            record.update = Some(update.clone());
            if self.commit(user, snapshot, expected, &can_write).await?.saved { return Ok(true); }
        }
    }
    async fn commit(&self, user: &str, mut snapshot: CliSnapshot, expected: u64, can_write: &impl Fn() -> bool) -> Result<CliSave, String> {
        snapshot.revision = expected.checked_add(1).filter(|v| *v <= MAX_REVISION).ok_or("cli_revision_exhausted")?;
        let raw = serde_json::to_string(&snapshot).map_err(|_| "cli_preferences_invalid")?;
        if !can_write() { return Err("session_invalid".into()); }
        let changed = if expected == 0 {
            self.db.execute(sql("INSERT INTO system_settings(user_id,setting_key,setting_value,updated_at) SELECT ?,'cli.paths.v1',?,unixepoch() WHERE EXISTS(SELECT 1 FROM users WHERE id=? AND deleting_since IS NULL) ON CONFLICT(user_id,setting_key) DO UPDATE SET setting_value=excluded.setting_value,updated_at=excluded.updated_at WHERE json_extract(system_settings.setting_value,'$.revision')=0 AND EXISTS(SELECT 1 FROM users WHERE id=? AND deleting_since IS NULL)", vec![user.into(), raw.into(), user.into(), user.into()])).await
        } else {
            self.db.execute(sql("UPDATE system_settings SET setting_value=?,updated_at=unixepoch() WHERE user_id=? AND setting_key='cli.paths.v1' AND json_extract(setting_value,'$.revision')=? AND EXISTS(SELECT 1 FROM users WHERE id=? AND deleting_since IS NULL)", vec![raw.into(), user.into(), (expected as i64).into(), user.into()])).await
        }.map_err(|_| "cli_storage_failed")?.rows_affected();
        if changed != 1 && !self.user_can_write(user).await? { return Err("session_invalid".into()); }
        Ok(CliSave { saved: changed == 1, snapshot: if changed == 1 { snapshot } else { self.get(user).await? } })
    }
    async fn user_can_write(&self, user: &str) -> Result<bool, String> {
        let row = self.db.query_one(sql("SELECT 1 AS active FROM users WHERE id=? AND deleting_since IS NULL", vec![user.into()])).await.map_err(|_| "cli_storage_failed")?;
        Ok(row.is_some())
    }

}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::Migrator;
    use sea_orm::Database;
    use sea_orm_migration::MigratorTrait;

    async fn memory() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db.execute_unprepared("INSERT INTO users(id,username,username_key,password_hash,created_at,updated_at) VALUES('alice','alice','alice','fixture',0,0)").await.unwrap();
        db
    }
    fn detected(path: &str, version: &str) -> CliDetection {
        CliDetection { status: "available".into(), path: Some(path.into()), version: Some(version.into()), update: None }
    }
    fn offer(version: &str) -> CliUpdateMetadata {
        CliUpdateMetadata { status: "available".into(), current_version: Some(version.into()), latest_version: Some("9.0.0".into()), method: Some("npm".into()) }
    }
    async fn raw(db: &DatabaseConnection, user: &str) -> String {
        db.query_one(sql("SELECT setting_value FROM system_settings WHERE user_id=? AND setting_key='cli.paths.v1'", vec![user.into()])).await.unwrap().unwrap().try_get("", "setting_value").unwrap()
    }
    async fn seed(db: &DatabaseConnection, value: &str) {
        db.execute(sql("INSERT INTO system_settings(user_id,setting_key,setting_value,updated_at) VALUES('alice','cli.paths.v1',?,0)", vec![value.into()])).await.unwrap();
    }

    #[tokio::test]
    async fn cli_legacy_paths_load_read_only_without_migration() {
        let db = memory().await;
        let legacy = r#"{"revision":7,"paths":{"codex":"/opt/codex","claude":null,"gemini":null,"node":null}}"#;
        seed(&db, legacy).await;
        let repo = CliRepository::new(db.clone());
        let snapshot = repo.get("alice").await.unwrap();
        assert_eq!(snapshot.revision, 7);
        assert_eq!(snapshot.paths.codex.as_deref(), Some("/opt/codex"));
        assert!(snapshot.detections.is_empty());
        assert_eq!(raw(&db, "alice").await, legacy);
        assert_eq!(repo.get("bob").await.unwrap(), CliSnapshot::default());
    }
    #[tokio::test]
    async fn cli_writes_cannot_recreate_settings_for_deleting_or_missing_users() {
        let db = memory().await;
        let repo = CliRepository::new(db.clone());
        db.execute_unprepared("UPDATE users SET deleting_since=1 WHERE id='alice'").await.unwrap();
        assert_eq!(repo.record_detection("alice", "codex", detected("/opt/codex", "1.0"), || true).await.unwrap_err(), "session_invalid");
        assert_eq!(repo.get("alice").await.unwrap(), CliSnapshot::default());
        db.execute_unprepared("DELETE FROM users WHERE id='alice'").await.unwrap();
        db.execute_unprepared("INSERT INTO users(id,username,username_key,password_hash,created_at,updated_at) VALUES('bob','bob','bob','fixture',0,0)").await.unwrap();
        let before = repo.record_detection("bob", "node", detected("/opt/node", "v1"), || true).await.unwrap();
        db.execute_unprepared("UPDATE users SET deleting_since=1 WHERE id='bob'").await.unwrap();
        assert_eq!(repo.record_detection("bob", "node", detected("/opt/node", "v2"), || true).await.unwrap_err(), "session_invalid");
        assert_eq!(repo.get("bob").await.unwrap(), before);
        assert_eq!(repo.record_detection("alice", "codex", detected("/opt/codex", "1.0"), || true).await.unwrap_err(), "session_invalid");
        assert_eq!(repo.get("alice").await.unwrap(), CliSnapshot::default());
    }


    #[tokio::test]
    async fn cli_manual_detection_survives_reopen_and_isolates_accounts() {
        let root = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}?mode=rwc", root.path().join("cli.db").display());
        let db = Database::connect(&url).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db.execute_unprepared("INSERT INTO users(id,username,username_key,password_hash,created_at,updated_at) VALUES('alice','alice','alice','fixture',0,0)").await.unwrap();
        let record = detected("/opt/codex", "codex-cli 1.0.0");
        {
            let repo = CliRepository::new(db.clone());
            repo.record_detection("alice", "codex", record.clone(), || true).await.unwrap();
            assert!(repo.record_update("alice", "codex", record.path.as_deref(), record.version.as_deref(), offer("codex-cli 1.0.0"), || true).await.unwrap());
        }
        drop(db);
        let reopened = CliRepository::new(Database::connect(&url).await.unwrap());
        let snapshot = reopened.get("alice").await.unwrap();
        assert_eq!(snapshot.paths.codex, record.path);
        assert_eq!(snapshot.detections["codex"].version, record.version);
        assert_eq!(snapshot.detections["codex"].update, Some(offer("codex-cli 1.0.0")));
        assert_eq!(reopened.get("bob").await.unwrap(), CliSnapshot::default());
    }

    #[tokio::test]
    async fn cli_parallel_tool_saves_merge_without_lost_detections() {
        let db = memory().await;
        let first = CliRepository::new(db.clone());
        let second = CliRepository::new(db);
        let (codex, claude, gemini, node) = tokio::join!(
            first.record_detection("alice", "codex", detected("/opt/codex", "codex-cli 1.0.0"), || true),
            second.record_detection("alice", "claude", detected("/opt/claude", "1.0.0 (Claude Code)"), || true),
            first.record_detection("alice", "gemini", detected("/opt/gemini", "1.0.0"), || true),
            second.record_detection("alice", "node", detected("/opt/node", "v22.0.0"), || true),
        );
        codex.unwrap(); claude.unwrap(); gemini.unwrap(); node.unwrap();
        let snapshot = first.get("alice").await.unwrap();
        assert_eq!(snapshot.revision, 4);
        assert_eq!(snapshot.detections.len(), 4);
        for tool in TOOLS { assert_eq!(snapshot.paths.path(tool).unwrap(), &snapshot.detections[tool].path); }
        let (codex, claude) = tokio::join!(
            first.record_update("alice", "codex", Some("/opt/codex"), Some("codex-cli 1.0.0"), offer("codex-cli 1.0.0"), || true),
            second.record_update("alice", "claude", Some("/opt/claude"), Some("1.0.0 (Claude Code)"), offer("1.0.0 (Claude Code)"), || true),
        );
        assert!(codex.unwrap() && claude.unwrap());
        let snapshot = first.get("alice").await.unwrap();
        assert_eq!(snapshot.revision, 6);
        assert!(snapshot.detections["codex"].update.is_some());
        assert!(snapshot.detections["claude"].update.is_some());
    }

    #[tokio::test]
    async fn cli_preference_cas_preserves_matching_records_and_other_tool_callers() {
        let repo = CliRepository::new(memory().await);
        repo.record_detection("alice", "codex", detected("/opt/codex", "codex-cli 1.0.0"), || true).await.unwrap();
        repo.record_detection("alice", "claude", detected("/opt/claude", "1.0.0 (Claude Code)"), || true).await.unwrap();
        let before = repo.get("alice").await.unwrap();
        let mut paths = before.paths.clone();
        paths.codex = Some("/new/codex".into());
        let stale = repo.save("alice", paths.clone(), 1).await.unwrap();
        assert!(!stale.saved);
        assert_eq!(stale.snapshot, before);
        let saved = repo.save("alice", paths, before.revision).await.unwrap();
        assert!(saved.saved);
        assert!(!saved.snapshot.detections.contains_key("codex"));
        assert_eq!(saved.snapshot.detections["claude"], before.detections["claude"]);
    }

    #[tokio::test]
    async fn cli_stale_update_rejected_and_manual_probe_clears_offer() {
        let repo = CliRepository::new(memory().await);
        let first = repo.record_detection("alice", "codex", detected("/opt/codex", "codex-cli 1.0.0"), || true).await.unwrap();
        assert!(repo.record_update("alice", "codex", Some("/opt/codex"), Some("codex-cli 1.0.0"), offer("codex-cli 1.0.0"), || true).await.unwrap());
        let hinted = repo.get("alice").await.unwrap();
        assert_eq!(hinted.revision, first.revision + 1);
        assert!(repo.record_update("alice", "codex", Some("/opt/codex"), Some("codex-cli 1.0.0"), offer("codex-cli 1.0.0"), || true).await.unwrap());
        assert_eq!(repo.get("alice").await.unwrap(), hinted);
        let fresh = repo.record_detection("alice", "codex", detected("/opt/codex", "codex-cli 2.0.0"), || true).await.unwrap();
        assert!(fresh.detections["codex"].update.is_none());
        for (path, version) in [("/opt/codex", "codex-cli 1.0.0"), ("/old/codex", "codex-cli 2.0.0")] {
            assert!(!repo.record_update("alice", "codex", Some(path), Some(version), offer(version), || true).await.unwrap());
        }
        assert_eq!(repo.get("alice").await.unwrap(), fresh);
        let repeated = repo.record_detection("alice", "codex", detected("/opt/codex", "codex-cli 2.0.0"), || true).await.unwrap();
        assert_eq!(repeated.revision, fresh.revision);
    }

    #[tokio::test]
    async fn cli_update_metadata_cannot_persist_tickets() {
        let db = memory().await;
        let repo = CliRepository::new(db.clone());
        repo.record_detection("alice", "codex", detected("/opt/codex", "codex-cli 1.0.0"), || true).await.unwrap();
        repo.record_update("alice", "codex", Some("/opt/codex"), Some("codex-cli 1.0.0"), offer("codex-cli 1.0.0"), || true).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw(&db, "alice").await).unwrap();
        let metadata = value["detections"]["codex"]["update"].as_object().unwrap();
        assert_eq!(metadata.len(), 4);
        assert!(!raw(&db, "alice").await.contains("ticket"));
        let mut forged = serde_json::to_value(offer("codex-cli 1.0.0")).unwrap();
        forged["ticket"] = "must-never-be-stored".into();
        assert!(serde_json::from_value::<CliUpdateMetadata>(forged).is_err());
    }

    #[tokio::test]
    async fn cli_node_cas_commits_version_and_path_as_one_pair() {
        let db = memory().await;
        let repo = CliRepository::new(db.clone());
        let before = repo.record_detection("alice", "node", detected("/old/node", "v20.0.0"), || true).await.unwrap();
        let selected = detected("/new/node", "v22.0.0");
        let mut paths = before.paths.clone();
        paths.node = selected.path.clone();
        // Directory probing itself has no repository write; only Save carries this record.
        assert_eq!(repo.get("alice").await.unwrap(), before);
        repo.record_detection("alice", "codex", detected("/opt/codex", "codex-cli 1.0.0"), || true).await.unwrap();
        let stale = repo.save_verified("alice", paths, before.revision, Some(selected.clone()), || true).await.unwrap();
        assert!(!stale.saved);
        assert_eq!(stale.snapshot.paths.node, before.paths.node);
        assert_eq!(stale.snapshot.detections["node"], before.detections["node"]);
        let mut paths = stale.snapshot.paths.clone();
        paths.node = selected.path.clone();
        let saved = repo.save_verified("alice", paths.clone(), stale.snapshot.revision, Some(selected.clone()), || true).await.unwrap();
        assert!(saved.saved);
        assert_eq!(saved.snapshot.paths.node, selected.path);
        assert_eq!(saved.snapshot.detections["node"], selected);
        assert!(saved.snapshot.detections.contains_key("codex"));
        assert!(repo.save_verified("alice", paths, saved.snapshot.revision, Some(detected("/wrong/node", "v99.0.0")), || true).await.is_err());
        assert_eq!(repo.get("alice").await.unwrap(), saved.snapshot);
        db.execute_unprepared("CREATE TRIGGER fail_cli BEFORE UPDATE ON system_settings BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
        let mut paths = saved.snapshot.paths.clone();
        paths.node = Some("/failed/node".into());
        assert!(repo.save_verified("alice", paths, saved.snapshot.revision, Some(detected("/failed/node", "v24.0.0")), || true).await.is_err());
        assert_eq!(repo.get("alice").await.unwrap(), saved.snapshot);
    }

    #[tokio::test]
    async fn cli_cancelled_writes_and_revision_overflow_leave_storage_unchanged() {
        let db = memory().await;
        let repo = CliRepository::new(db.clone());
        assert_eq!(repo.record_detection("alice", "codex", detected("/opt/codex", "codex-cli 1.0.0"), || false).await.unwrap_err(), "session_invalid");
        assert_eq!(repo.get("alice").await.unwrap(), CliSnapshot::default());
        let before = repo.record_detection("alice", "node", detected("/opt/node", "v22.0.0"), || true).await.unwrap();
        assert!(repo.record_update("alice", "node", Some("/opt/node"), Some("v22.0.0"), offer("v22.0.0"), || false).await.is_err());
        assert!(repo.save_verified("alice", before.paths.clone(), before.revision, None, || false).await.is_err());
        assert_eq!(repo.get("alice").await.unwrap(), before);
        db.execute(sql("UPDATE system_settings SET setting_value=json_set(setting_value,'$.revision',?) WHERE user_id='alice' AND setting_key='cli.paths.v1'", vec![(MAX_REVISION as i64).into()])).await.unwrap();
        let exhausted = raw(&db, "alice").await;
        assert_eq!(repo.record_detection("alice", "node", detected("/opt/node", "v24.0.0"), || true).await.unwrap_err(), "cli_revision_exhausted");
        assert_eq!(raw(&db, "alice").await, exhausted);
    }

    #[tokio::test]
    async fn cli_failed_detection_replaces_version_and_clears_update_hint() {
        let repo = CliRepository::new(memory().await);
        repo.record_detection("alice", "codex", detected("/opt/codex", "codex-cli 1.0.0"), || true).await.unwrap();
        repo.record_update("alice", "codex", Some("/opt/codex"), Some("codex-cli 1.0.0"), offer("codex-cli 1.0.0"), || true).await.unwrap();
        let failure = CliDetection { status: "not_installed".into(), path: None, version: None, update: Some(offer("codex-cli 1.0.0")) };
        let snapshot = repo.record_detection("alice", "codex", failure, || true).await.unwrap();
        assert_eq!(snapshot.revision, 3);
        assert!(snapshot.paths.codex.is_none());
        let record = &snapshot.detections["codex"];
        assert_eq!(record.status, "not_installed");
        assert!(record.path.is_none() && record.version.is_none() && record.update.is_none());
        assert_eq!(repo.get("alice").await.unwrap(), snapshot);
    }
}
