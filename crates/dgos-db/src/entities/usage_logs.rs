use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "provider_usage_logs")]
pub struct Model {
    #[sea_orm(primary_key, column_type = "String(StringLen::N(36))")]
    pub id: String,
    
    #[sea_orm(column_type = "String(StringLen::N(36))")]
    pub provider_id: String,
    
    #[sea_orm(column_type = "String(StringLen::N(255))")]
    pub model_id: String,
    
    #[sea_orm(column_type = "String(StringLen::N(36))")]
    pub user_id: String,
    
    pub request_tokens: i32,
    
    pub response_tokens: i32,
    
    pub total_tokens: i32,
    
    pub latency_ms: i32,
    
    pub success: bool,
    
    #[sea_orm(column_type = "String(StringLen::N(50))", nullable)]
    pub error_code: Option<String>,
    
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::providers::Entity",
        from = "Column::ProviderId",
        to = "super::providers::Column::Id"
    )]
    Providers,
}

impl Related<super::providers::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Providers.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
