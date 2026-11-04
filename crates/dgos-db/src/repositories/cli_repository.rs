//! D133 device/account CLI path preferences, compare-and-swap writes.
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliPaths { pub codex: Option<String>, pub claude: Option<String>, pub gemini: Option<String>, pub node: Option<String> }
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliSnapshot { pub revision: u64, pub paths: CliPaths }
#[derive(Serialize)]
pub struct CliSave { pub saved: bool, pub snapshot: CliSnapshot }
#[derive(Clone)]
pub struct CliRepository { db: DatabaseConnection }
fn sql(query: &str, values: Vec<sea_orm::Value>) -> Statement { Statement::from_sql_and_values(DbBackend::Sqlite, query, values) }
impl CliRepository {
    pub fn new(db: DatabaseConnection) -> Self { Self { db } }
    pub async fn get(&self, user: &str) -> Result<CliSnapshot, String> {
        let row = self.db.query_one(sql("SELECT setting_value FROM system_settings WHERE user_id=? AND setting_key='cli.paths.v1'", vec![user.into()])).await.map_err(|_| "cli_storage_failed")?;
        match row { None => Ok(CliSnapshot::default()), Some(row) => serde_json::from_str(&row.try_get::<String>("", "setting_value").map_err(|_| "cli_storage_failed")?).map_err(|_| "cli_preferences_invalid".into()) }
    }
    pub async fn save(&self, user: &str, paths: CliPaths, expected: u64) -> Result<CliSave, String> {
        for path in [&paths.codex, &paths.claude, &paths.gemini, &paths.node].into_iter().flatten() {
            if path.is_empty() || path.len() > 4096 || path.contains('\0') || !std::path::Path::new(path).is_absolute() { return Err("cli_invalid_path".into()); }
        }
        let revision = expected.checked_add(1).filter(|v| *v <= 9_007_199_254_740_991).ok_or("cli_revision_exhausted")?;
        let snapshot = CliSnapshot { revision, paths };
        let raw = serde_json::to_string(&snapshot).map_err(|_| "cli_preferences_invalid")?;
        let changed = if expected == 0 {
            self.db.execute(sql("INSERT INTO system_settings(user_id,setting_key,setting_value,updated_at) VALUES(?,'cli.paths.v1',?,unixepoch()) ON CONFLICT(user_id,setting_key) DO NOTHING", vec![user.into(),raw.into()])).await
        } else {
            self.db.execute(sql("UPDATE system_settings SET setting_value=?,updated_at=unixepoch() WHERE user_id=? AND setting_key='cli.paths.v1' AND json_extract(setting_value,'$.revision')=?", vec![raw.into(),user.into(),(expected as i64).into()])).await
        }.map_err(|_| "cli_storage_failed")?.rows_affected();
        Ok(CliSave { saved: changed == 1, snapshot: if changed == 1 { snapshot } else { self.get(user).await? } })
    }
}
