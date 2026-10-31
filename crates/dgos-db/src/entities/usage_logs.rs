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

    #[sea_orm(column_type = "String(StringLen::N(32))", nullable)]
    pub caller: Option<String>,

    #[sea_orm(column_type = "String(StringLen::N(36))", nullable)]
    pub request_id: Option<String>,

    /// D081: `Some(true)` the upstream reported usage; `Some(false)` it did not, so the token
    /// columns are unknown rather than zero; `None` a row older than the column.
    pub usage_reported: Option<bool>,

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
