use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
 async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
  manager.get_connection().execute_unprepared("CREATE TABLE chat_assets(id TEXT PRIMARY KEY NOT NULL,user_id TEXT NOT NULL,name TEXT NOT NULL,mime TEXT NOT NULL,bytes BLOB NOT NULL,created_at BIGINT NOT NULL,UNIQUE(id,user_id)); CREATE TABLE chat_message_assets(message_id TEXT NOT NULL,user_id TEXT NOT NULL,asset_id TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(message_id,asset_id),FOREIGN KEY(message_id) REFERENCES chat_messages(id) ON DELETE CASCADE,FOREIGN KEY(asset_id,user_id) REFERENCES chat_assets(id,user_id)); CREATE INDEX idx_chat_assets_user ON chat_assets(user_id);").await.map(|_| ())
 }
 async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> { Err(DbErr::Custom("chat assets are forward-only".into())) }
}
