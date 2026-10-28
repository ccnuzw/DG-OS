use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "mcp_connection_logs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: String,
    pub server_id: String,
    pub event: String,
    pub status: String,
    pub error_class: Option<String>,
    pub duration_ms: Option<i32>,
    pub reconnect_attempt: Option<i32>,
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::mcp_servers::Entity",
        from = "Column::ServerId",
        to = "super::mcp_servers::Column::Id",
        on_delete = "Cascade"
    )]
    Server,
}

impl Related<super::mcp_servers::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Server.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
