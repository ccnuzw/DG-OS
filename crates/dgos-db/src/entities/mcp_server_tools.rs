use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "mcp_server_tools")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub server_id: String,
    pub tool_name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema_json: String,
    pub output_schema_json: Option<String>,
    pub schema_hash: String,
    pub discovered_at: i64,
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
