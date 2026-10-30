use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "models")]
pub struct Model {
    #[sea_orm(primary_key, column_type = "String(StringLen::N(36))")]
    pub id: String,

    #[sea_orm(column_type = "String(StringLen::N(36))")]
    pub provider_id: String,

    #[sea_orm(column_type = "String(StringLen::N(255))")]
    pub model_id: String,

    #[sea_orm(column_type = "String(StringLen::N(255))")]
    pub model_name: String,

    #[sea_orm(column_type = "Json")]
    pub capabilities: serde_json::Value,

    #[sea_orm(nullable)]
    pub context_length: Option<i32>,

    pub is_available: bool,

    pub synced_at: DateTimeUtc,

    /// `remote` or `manual` (V1-FR-001 §4.3).
    #[sea_orm(column_type = "Text")]
    pub source: String,

    /// NULL on rows written before the kind migration; the reader derives it.
    #[sea_orm(column_type = "Text", nullable)]
    pub kind: Option<String>,

    #[sea_orm(column_type = "Text", nullable)]
    pub kind_source: Option<String>,

    #[sea_orm(nullable)]
    pub max_output_tokens: Option<i32>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::providers::Entity",
        from = "Column::ProviderId",
        to = "super::providers::Column::Id",
        on_delete = "Cascade"
    )]
    Providers,
}

impl Related<super::providers::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Providers.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
