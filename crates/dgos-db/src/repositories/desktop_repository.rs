//! D133: fixed-key, account-scoped desktop preferences; reads never overwrite stored data.
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopValues {
    pub pinned_module_ids: Vec<String>,
    pub show_entries: bool,
    pub show_recent: bool,
    pub sidebar_collapsed: bool,
}
impl Default for DesktopValues {
    fn default() -> Self { Self { pinned_module_ids: vec![], show_entries: true, show_recent: true, sidebar_collapsed: false } }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopSnapshot { pub revision: u64, pub values: DesktopValues }
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopSave { Saved { snapshot: DesktopSnapshot }, Conflict { snapshot: DesktopSnapshot } }
#[derive(Clone)]
pub struct DesktopRepository { db: DatabaseConnection }
const KEY: &str = "desktop.snapshot.v1";
fn statement(sql: &str, values: Vec<sea_orm::Value>) -> Statement { Statement::from_sql_and_values(DbBackend::Sqlite, sql, values) }
impl DesktopRepository {
    pub fn new(db: DatabaseConnection) -> Self { Self { db } }
    pub async fn get(&self, user: &str) -> Result<DesktopSnapshot, String> {
        let row = self.db.query_one(statement("SELECT setting_value FROM system_settings WHERE user_id=? AND setting_key=?", vec![user.into(), KEY.into()])).await.map_err(|_| "desktop_storage_failed")?;
        match row {
            None => Ok(DesktopSnapshot::default()),
            Some(row) => {
                let raw: String = row.try_get("", "setting_value").map_err(|_| "desktop_storage_failed")?;
                let snapshot: DesktopSnapshot = serde_json::from_str(&raw).map_err(|_| "desktop_preferences_invalid")?;
                validate(&snapshot.values)?;
                Ok(snapshot)
            }
        }
    }
    pub async fn save(&self, user: &str, values: DesktopValues, expected: u64) -> Result<DesktopSave, String> {
        validate(&values)?;
        let current = self.get(user).await?;
        if current.revision != expected { return Ok(DesktopSave::Conflict { snapshot: current }); }
        let revision = expected.checked_add(1).filter(|n| *n <= 9_007_199_254_740_991).ok_or("desktop_revision_exhausted")?;
        let snapshot = DesktopSnapshot { revision, values };
        let json = serde_json::to_string(&snapshot).map_err(|_| "desktop_preferences_invalid")?;
        let changed = if expected == 0 {
            self.db.execute(statement("INSERT INTO system_settings(user_id,setting_key,setting_value,updated_at) VALUES (?,?,?,unixepoch()) ON CONFLICT(user_id,setting_key) DO NOTHING", vec![user.into(),KEY.into(),json.clone().into()])).await.map_err(|_| "desktop_storage_failed")?.rows_affected()
        } else {
            self.db.execute(statement("UPDATE system_settings SET setting_value=?,updated_at=unixepoch() WHERE user_id=? AND setting_key=? AND json_extract(setting_value,'$.revision')=?", vec![json.into(),user.into(),KEY.into(),(expected as i64).into()])).await.map_err(|_| "desktop_storage_failed")?.rows_affected()
        };
        if changed == 1 { Ok(DesktopSave::Saved { snapshot }) } else { Ok(DesktopSave::Conflict { snapshot: self.get(user).await? }) }
    }
}
fn validate(values: &DesktopValues) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    if values.pinned_module_ids.len() > 3 || values.pinned_module_ids.iter().any(|id| id.is_empty() || id.len() > 128 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) || !seen.insert(id)) { return Err("desktop_preferences_invalid".into()); }
    Ok(())
}
