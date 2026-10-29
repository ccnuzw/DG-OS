//! Typed persistence of appearance preferences in `system_settings` (D065).
//!
//! The only keys this repository can touch are `appearance.theme` and
//! `appearance.density`; callers pass a validated [`AppearancePatch`], never
//! a key or a raw value.

use crate::entities::system_settings;
use dgos_types::appearance::{DENSITY_KEY, THEME_KEY};
use dgos_types::{AppearancePatch, AppearancePreferences, Density, DgosError, DgosResult, Theme};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use serde_json::Value;

#[derive(Clone)]
pub struct AppearanceRepository {
    db: DatabaseConnection,
}

fn db_error(error: sea_orm::DbErr) -> DgosError {
    DgosError::DatabaseError(error.to_string())
}

impl AppearanceRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Both preferences for `user_id`. A missing, unparsable or out-of-set
    /// stored value yields that preference's default (B05-22); it is never
    /// returned or reported.
    pub async fn get(&self, user_id: &str) -> DgosResult<AppearancePreferences> {
        read(&self.db, user_id).await
    }

    /// Persist the fields present in `patch` in one transaction and return the
    /// complete preferences as stored afterwards. Either every present field
    /// is written or none is.
    pub async fn apply(
        &self,
        user_id: &str,
        patch: &AppearancePatch,
    ) -> DgosResult<AppearancePreferences> {
        let txn = self.db.begin().await.map_err(db_error)?;
        if let Some(theme) = patch.theme() {
            upsert(&txn, user_id, THEME_KEY, &encode(&theme)?).await?;
        }
        if let Some(density) = patch.density() {
            upsert(&txn, user_id, DENSITY_KEY, &encode(&density)?).await?;
        }
        let stored = read(&txn, user_id).await?;
        txn.commit().await.map_err(db_error)?;
        Ok(stored)
    }
}

fn encode<T: serde::Serialize>(value: &T) -> DgosResult<String> {
    serde_json::to_string(value).map_err(DgosError::from)
}

async fn upsert(
    conn: &impl ConnectionTrait,
    user_id: &str,
    key: &str,
    json: &str,
) -> DgosResult<()> {
    system_settings::Entity::insert(system_settings::ActiveModel {
        user_id: Set(user_id.to_owned()),
        setting_key: Set(key.to_owned()),
        setting_value: Set(json.to_owned()),
        updated_at: Set(chrono::Utc::now().timestamp()),
        ..Default::default()
    })
    .on_conflict(
        OnConflict::columns([
            system_settings::Column::UserId,
            system_settings::Column::SettingKey,
        ])
        .update_columns([
            system_settings::Column::SettingValue,
            system_settings::Column::UpdatedAt,
        ])
        .to_owned(),
    )
    .exec_without_returning(conn)
    .await
    .map(|_| ())
    .map_err(db_error)
}

async fn read(conn: &impl ConnectionTrait, user_id: &str) -> DgosResult<AppearancePreferences> {
    let rows = system_settings::Entity::find()
        .filter(system_settings::Column::UserId.eq(user_id))
        .filter(system_settings::Column::SettingKey.is_in([THEME_KEY, DENSITY_KEY]))
        .all(conn)
        .await
        .map_err(db_error)?;
    let stored = |key: &str| -> Option<Value> {
        let row = rows.iter().find(|row| row.setting_key == key)?;
        serde_json::from_str(&row.setting_value).ok()
    };
    Ok(AppearancePreferences {
        theme: stored(THEME_KEY)
            .as_ref()
            .and_then(Theme::parse)
            .unwrap_or_default(),
        density: stored(DENSITY_KEY)
            .as_ref()
            .and_then(Density::parse)
            .unwrap_or_default(),
    })
}
