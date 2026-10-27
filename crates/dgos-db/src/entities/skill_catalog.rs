//! Catalog entries for the SKILL.md based content model.
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "skill_catalog")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub skill_id: String,
    pub identity: String,
    pub name: String,
    pub latest_version: String,
    pub description: Option<String>,
    pub category: String,
    #[sea_orm(column_type = "Json")]
    pub tags: serde_json::Value,
    #[sea_orm(column_type = "Json")]
    pub compatible_apps: serde_json::Value,
    pub author: Option<String>,
    pub license_spdx: Option<String>,
    pub source_kind: String,
    pub source_ref: Option<String>,
    pub verification_status: String,
    pub metadata: serde_json::Value,
    pub synced_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
