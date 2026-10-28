use sea_orm::entity::prelude::*;

#[derive(Clone, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "providers")]
pub struct Model {
    #[sea_orm(
        primary_key,
        auto_increment = false,
        column_type = "String(StringLen::N(36))"
    )]
    pub id: String,

    #[sea_orm(column_type = "String(StringLen::N(255))")]
    pub name: String,

    #[sea_orm(column_type = "String(StringLen::N(50))")]
    pub provider_type: String,

    #[sea_orm(column_type = "String(StringLen::N(512))")]
    pub base_url: String,

    #[sea_orm(column_type = "String(StringLen::N(512))")]
    pub api_key: String,

    /// Stable reference into the host credential vault. Legacy rows may have
    /// this unset while they are waiting for incremental migration.
    #[sea_orm(column_type = "String(StringLen::N(128))", nullable)]
    pub credential_ref: Option<String>,

    pub config_revision: i64,
    pub model_refresh_revision: i64,

    pub enabled: bool,

    pub sort_order: i32,

    #[sea_orm(column_type = "Json", nullable)]
    pub config: Option<serde_json::Value>,

    pub created_at: DateTimeUtc,

    pub updated_at: DateTimeUtc,

    #[sea_orm(column_type = "String(StringLen::N(36))")]
    pub user_id: String,
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRecord")
            .field("id", &self.id)
            .field("credential_ref", &self.credential_ref)
            .finish_non_exhaustive()
    }
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
