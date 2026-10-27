//! Immutable content versions for the SKILL.md based content model.
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "skill_versions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub skill_id: String,
    pub version: String,
    pub content_hash: String,
    pub install_path: String,
    pub is_bundled: bool,
    pub verification_status: String,
    pub file_manifest: serde_json::Value,
    pub metadata: serde_json::Value,
    pub installed_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
