//! Fixed-key appearance snapshots with database compare-and-swap (D126).
use crate::entities::system_settings;
use dgos_types::appearance::{MAX_REVISION, SNAPSHOT_KEY};
use dgos_types::{AppearanceRead, AppearanceSave, AppearanceSnapshot, AppearanceValues, DgosError, DgosResult};
use sea_orm::{sea_query::OnConflict, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set, TransactionTrait};
use serde_json::Value;
use std::{collections::HashMap, sync::{Arc, Mutex}};

#[derive(Clone)]
pub struct AppearanceRepository {
    db: DatabaseConnection,
    recovery: Arc<Mutex<HashMap<String, (String, String)>>>,
}
fn db_error(error: sea_orm::DbErr) -> DgosError { DgosError::DatabaseError(error.to_string()) }
impl AppearanceRepository {
    pub fn new(db: DatabaseConnection) -> Self { Self { db, recovery: Arc::new(Mutex::new(HashMap::new())) } }
    pub fn invalidate_recovery(&self, user: &str) {
        self.recovery.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, (owner, _)| owner != user);
    }
    async fn raw(&self, user: &str) -> DgosResult<String> {
        let txn = self.db.begin().await.map_err(db_error)?;
        let row = system_settings::Entity::find().filter(system_settings::Column::UserId.eq(user))
            .filter(system_settings::Column::SettingKey.eq(SNAPSHOT_KEY))
            .one(&txn).await.map_err(db_error)?;
        if let Some(row) = row {
            let raw = row.setting_value.clone();
            txn.commit().await.map_err(db_error)?;
            return Ok(raw);
        }
        let defaults = AppearanceSnapshot { revision: 0, values: AppearanceValues::default() };
        system_settings::Entity::insert(system_settings::ActiveModel {
            user_id: Set(user.to_owned()), setting_key: Set(SNAPSHOT_KEY.to_owned()),
            setting_value: Set(serde_json::to_string(&defaults)?), updated_at: Set(chrono::Utc::now().timestamp()), ..Default::default()
        }).on_conflict(OnConflict::columns([system_settings::Column::UserId,system_settings::Column::SettingKey]).do_nothing().to_owned())
            .exec_without_returning(&txn).await.map_err(db_error)?;
        let raw = system_settings::Entity::find().filter(system_settings::Column::UserId.eq(user)).filter(system_settings::Column::SettingKey.eq(SNAPSHOT_KEY))
            .one(&txn).await.map_err(db_error)?.map(|row| row.setting_value).ok_or_else(|| DgosError::InternalError("appearance unavailable".into()))?;
        txn.commit().await.map_err(db_error)?;
        Ok(raw)
    }
    pub async fn get(&self, user: &str) -> DgosResult<AppearanceRead> {
        let raw = self.raw(user).await?;
        match decode(&raw) {
            Ok((snapshot, repair_warnings)) => Ok(AppearanceRead::Ready { snapshot, repair_warnings }),
            Err(DgosError::AppearanceSchemaUnsupported) => {
                let version = serde_json::from_str::<Value>(&raw).ok().and_then(|v| v["schemaVersion"].as_u64()).unwrap_or(3);
                Ok(AppearanceRead::UnsupportedSchema { safe_defaults: AppearanceValues::default(), schema_version: version })
            }
            Err(_) => {
                let token = uuid::Uuid::new_v4().to_string();
                let mut tokens = self.recovery.lock().unwrap_or_else(|e| e.into_inner());
                tokens.retain(|_, (owner, _)| owner != user);
                tokens.insert(token.clone(), (user.to_owned(), raw));
                Ok(AppearanceRead::Corrupt { safe_defaults: AppearanceValues::default(), recovery_token: token })
            }
        }
    }
    pub async fn apply(&self, user: &str, values: &AppearanceValues, expected_revision: u64) -> DgosResult<AppearanceSave> {
        let raw = self.raw(user).await?;
        let (current, _) = decode(&raw)?;
        if current.revision != expected_revision {
            return Ok(AppearanceSave::Conflict { code:1005007, snapshot:current });
        }
        if current.revision >= MAX_REVISION { return Err(DgosError::AppearanceRevisionConflict); }
        let snapshot = AppearanceSnapshot { revision: current.revision + 1, values: values.clone() };
        if self.replace(user, &raw, &snapshot).await? {
            self.invalidate_recovery(user);
            Ok(AppearanceSave::Saved { snapshot })
        } else {
            let (snapshot, _) = decode(&self.raw(user).await?)?;
            Ok(AppearanceSave::Conflict { code:1005007, snapshot })
        }
    }
    pub async fn recover(&self, user: &str, token: &str) -> DgosResult<AppearanceSnapshot> {
        let raw = {
            let mut tokens = self.recovery.lock().unwrap_or_else(|e| e.into_inner());
            if !tokens.get(token).is_some_and(|(owner, _)| owner == user) { return Err(DgosError::AppearanceRecoveryInvalid); }
            tokens.remove(token).expect("checked token").1
        };
        if matches!(decode(&raw), Err(DgosError::AppearanceSchemaUnsupported)) { return Err(DgosError::AppearanceSchemaUnsupported); }
        let snapshot = AppearanceSnapshot { revision:0, values:AppearanceValues::default() };
        if !self.replace(user, &raw, &snapshot).await? { return Err(DgosError::AppearanceRecoveryInvalid); }
        Ok(snapshot)
    }
    async fn replace(&self, user: &str, raw: &str, snapshot: &AppearanceSnapshot) -> DgosResult<bool> {
        let txn = self.db.begin().await.map_err(db_error)?;
        let update = system_settings::Entity::update_many()
            .col_expr(system_settings::Column::SettingValue, sea_orm::sea_query::Expr::value(serde_json::to_string(snapshot)?))
            .col_expr(system_settings::Column::UpdatedAt, sea_orm::sea_query::Expr::value(chrono::Utc::now().timestamp()))
            .filter(system_settings::Column::UserId.eq(user)).filter(system_settings::Column::SettingKey.eq(SNAPSHOT_KEY))
            .filter(system_settings::Column::SettingValue.eq(raw)).exec(&txn).await.map_err(db_error)?;
        txn.commit().await.map_err(db_error)?;
        Ok(update.rows_affected == 1)
    }
}

fn decode(raw: &str) -> DgosResult<(AppearanceSnapshot, Vec<String>)> {
    let raw: Value = serde_json::from_str(raw).map_err(|_| DgosError::AppearanceRecoveryInvalid)?;
    if raw["schemaVersion"].as_u64().is_some_and(|v| v > 2) { return Err(DgosError::AppearanceSchemaUnsupported); }
    let revision = raw["revision"].as_u64().filter(|r| *r <= MAX_REVISION).ok_or(DgosError::AppearanceRecoveryInvalid)?;
    if raw["schemaVersion"].as_u64() != Some(2) { return Err(DgosError::AppearanceRecoveryInvalid); }
    if let Ok(snapshot) = AppearanceSnapshot::parse(&raw) { return Ok((snapshot, Vec::new())); }
    let mut values = serde_json::to_value(AppearanceValues::default())?;
    let mut warnings = Vec::new();
    // Every leaf is accepted only if the whole typed candidate stays valid. Invalid fields inherit defaults.
    fn merge(target: &mut Value, source: &Value, path: &mut Vec<String>, warnings: &mut Vec<String>) {
        let Some(object) = source.as_object() else { warnings.push(path.join(".")); return; };
        if !path.is_empty() {
            let mut candidate = target.clone();
            let pointer = format!("/{}", path.join("/"));
            if let Some(slot) = candidate.pointer_mut(&pointer) {
                *slot = source.clone();
                if let Ok(normalized) = AppearanceValues::parse(&candidate) {
                    *target = serde_json::to_value(normalized).expect("serializable appearance");
                    return;
                }
            }
        }
        for (key, value) in object {
            if path.is_empty() && key == "revision" { continue; }
            path.push(key.clone());
            let pointer = format!("/{}", path.join("/"));
            if value.is_object() {
                merge(target, value, path, warnings);
            } else {
                let mut candidate = target.clone();
                let parent = format!("/{}", path[..path.len()-1].join("/"));
                let holder = if path.len() == 1 { candidate.as_object_mut() } else { candidate.pointer_mut(&parent).and_then(Value::as_object_mut) };
                if let Some(holder) = holder { holder.insert(key.clone(),value.clone()); }
                match AppearanceValues::parse(&candidate) {
                    Ok(normalized) => { *target = serde_json::to_value(normalized).expect("serializable appearance"); }
                    Err(_) => warnings.push(pointer),
                }
            }
            path.pop();
        }
    }
    merge(&mut values, &raw, &mut Vec::new(), &mut warnings);
    Ok((AppearanceSnapshot { revision, values:AppearanceValues::parse(&values)? },warnings))
}
