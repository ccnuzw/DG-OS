use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::TransactionTrait;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let tx = manager.get_connection().begin().await?;
        tx.execute_unprepared(r#"
CREATE TABLE development_codex_operations (
 owner TEXT NOT NULL CHECK(length(owner)>0),
 request_id TEXT NOT NULL CHECK(length(request_id)>0),
 session_id TEXT NOT NULL,
 generation TEXT NOT NULL CHECK(length(generation)>0),
 fingerprint TEXT NOT NULL CHECK(length(fingerprint)>0),
 state TEXT NOT NULL CHECK(state IN ('pending','applied','unknown','failed')),
 result TEXT NOT NULL CHECK(json_valid(result)),
 PRIMARY KEY(owner,request_id),
 FOREIGN KEY(session_id,owner) REFERENCES development_agent_sessions(id,owner) ON DELETE CASCADE
);
CREATE INDEX development_codex_operations_session ON development_codex_operations(owner,session_id);
CREATE TRIGGER development_codex_operations_identity_guard BEFORE UPDATE ON development_codex_operations
WHEN NEW.owner IS NOT OLD.owner OR NEW.request_id IS NOT OLD.request_id
 OR NEW.session_id IS NOT OLD.session_id OR NEW.generation IS NOT OLD.generation
 OR NEW.fingerprint IS NOT OLD.fingerprint
 OR (OLD.state<>'pending' AND (NEW.state IS NOT OLD.state OR NEW.result IS NOT OLD.result))
BEGIN SELECT RAISE(ABORT,'codex_operation_conflict'); END;
-- Observed turns are audit evidence, not admission to execute. Keep the normal
-- active-binding run guard intact and allow this format only for exact stored
-- Codex identity/snapshot, including after its execution authority is revoked.
DROP TRIGGER development_run_capabilities_format_INSERT;
DROP TRIGGER development_run_capabilities_format_UPDATE;
CREATE TRIGGER development_run_capabilities_format_INSERT BEFORE INSERT ON development_run_capabilities
WHEN NEW.format NOT IN ('legacy_mixed_v1','cli_provider_v1','observed_codex_v1')
BEGIN SELECT RAISE(ABORT,'invalid_capability_format'); END;
CREATE TRIGGER development_run_capabilities_format_UPDATE BEFORE UPDATE ON development_run_capabilities
WHEN NEW.format NOT IN ('legacy_mixed_v1','cli_provider_v1','observed_codex_v1')
BEGIN SELECT RAISE(ABORT,'invalid_capability_format'); END;
CREATE TRIGGER development_observed_codex_run_immutable BEFORE UPDATE ON development_run_capabilities
WHEN OLD.format='observed_codex_v1' OR NEW.format='observed_codex_v1'
BEGIN SELECT RAISE(ABORT,'invalid_cli_snapshot'); END;
CREATE TRIGGER development_observed_codex_run_guard BEFORE INSERT ON development_run_capabilities
WHEN NEW.format='observed_codex_v1' AND NOT EXISTS(
 SELECT 1 FROM development_runs r
 JOIN development_agent_sessions s ON s.owner=r.owner AND s.id=r.session_id
 JOIN development_cli_session_bindings b ON b.owner=s.owner AND b.session_id=s.id
 JOIN development_session_capabilities c ON c.owner=b.owner AND c.session_id=b.session_id
 WHERE r.owner=NEW.owner AND r.id=NEW.run_id AND r.source='native_agent' AND r.engine='codex' AND s.engine=r.engine
 AND r.project_id=s.project_id AND r.task_id=s.task_id AND b.project_id=s.project_id AND b.task_id=s.task_id
 AND b.execution_policy='provider_bound' AND c.format='cli_provider_v1' AND c.project_id=b.project_id
 AND c.revision=b.revision AND c.permission_epoch=b.permission_epoch AND c.snapshot=NEW.snapshot
 AND b.binding_id=json_extract(NEW.snapshot,'$.binding_id') AND json_extract(NEW.snapshot,'$.engine')='codex'
 AND b.revision=json_extract(NEW.snapshot,'$.revision') AND b.permission_epoch=json_extract(NEW.snapshot,'$.permission_epoch')
 AND b.provider_id=json_extract(NEW.snapshot,'$.provider_id') AND b.model_id=json_extract(NEW.snapshot,'$.model_id')
 AND b.provider_revision=json_extract(NEW.snapshot,'$.provider_revision')
)
BEGIN SELECT RAISE(ABORT,'invalid_cli_snapshot'); END;
"#).await?;
        tx.commit().await
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Codex operation receipts are forward-only".into()))
    }
}
