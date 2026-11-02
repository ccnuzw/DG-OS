use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "skill_install_operations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub operation_id: String,
    pub user_id: String,
    pub idempotency_key: String,
    pub source_kind: String,
    pub source_ref: String,
    pub state: String,
    pub skill_id: Option<String>,
    pub version_id: Option<String>,
    pub content_hash: Option<String>,
    pub staging_relpath: Option<String>,
    pub published_relpath: Option<String>,
    #[sea_orm(column_type = "Json")]
    pub metadata: serde_json::Value,
    pub last_error_key: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
