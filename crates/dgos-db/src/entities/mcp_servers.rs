use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "mcp_servers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub transport: String,
    pub config_json: String,
    pub credential_ref: Option<String>,
    pub enabled: bool,
    pub auto_connect: bool,
    pub status: String,
    pub last_error_class: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::mcp_server_tools::Entity")]
    Tools,
    #[sea_orm(has_many = "super::mcp_usage_logs::Entity")]
    UsageLogs,
}

impl Related<super::mcp_server_tools::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tools.def()
    }
}

impl Related<super::mcp_usage_logs::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UsageLogs.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
