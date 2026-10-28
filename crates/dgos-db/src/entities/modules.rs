use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "modules")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub module_name: String,
    pub display_name: String,
    pub current_version: String,
    pub description: Option<String>,
    #[sea_orm(column_type = "Json")]
    pub manifest: serde_json::Value,
    pub integrity: String,
    pub entry_url: Option<String>,
    pub icon_url: Option<String>,
    pub is_official: bool,
    pub is_enabled: bool,
    pub installed_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::module_permissions::Entity")]
    Permissions,
}

impl Related<super::module_permissions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Permissions.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
