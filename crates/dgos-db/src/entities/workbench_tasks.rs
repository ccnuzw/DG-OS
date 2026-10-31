use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "workbench_tasks")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub user_id: String,
    pub app_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub status: String,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    pub error_code: Option<String>,
    pub total_tokens: Option<i64>,
    pub provider_config_revision: Option<i64>,
    /// Chat session this generation belongs to; cleared when the session is deleted (D072).
    pub session_id: Option<String>,
    /// `single` (D018/D022 task page) or `chat` (D073). Survives session deletion.
    pub kind: String,
    /// Hash of provider type + scheme/host/port; never the raw URL (D075).
    pub endpoint_fingerprint: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
