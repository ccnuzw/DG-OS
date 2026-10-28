use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "module_permissions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub user_id: String,
    pub module_id: String,
    pub permission: String,
    pub granted: bool,
    pub granted_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::modules::Entity",
        from = "Column::ModuleId",
        to = "super::modules::Column::Id"
    )]
    Module,
}

impl Related<super::modules::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Module.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
