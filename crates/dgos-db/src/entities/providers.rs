use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "providers")]
pub struct Model {
    #[sea_orm(primary_key, column_type = "String(StringLen::N(36))")]
    pub id: String,
    
    #[sea_orm(column_type = "String(StringLen::N(255))")]
    pub name: String,
    
    #[sea_orm(column_type = "String(StringLen::N(50))")]
    pub provider_type: String,
    
    #[sea_orm(column_type = "String(StringLen::N(512))")]
    pub base_url: String,
    
    #[sea_orm(column_type = "String(StringLen::N(512))")]
    pub api_key: String,
    
    pub enabled: bool,
    
    pub sort_order: i32,
    
    #[sea_orm(column_type = "Json", nullable)]
    pub config: Option<serde_json::Value>,
    
    pub created_at: DateTimeUtc,
    
    pub updated_at: DateTimeUtc,
    
    #[sea_orm(column_type = "String(StringLen::N(36))")]
    pub user_id: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::models::Entity")]
    Models,
    #[sea_orm(has_many = "super::usage_logs::Entity")]
    UsageLogs,
}

impl Related<super::models::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Models.def()
    }
}

impl Related<super::usage_logs::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UsageLogs.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
