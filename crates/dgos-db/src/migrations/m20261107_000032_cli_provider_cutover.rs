//! Persistent Provider-only CLI bindings and one-time legacy archive.
use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::TransactionTrait;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let tx = manager.get_connection().begin().await?;
        tx.execute_unprepared(
            "ALTER TABLE development_capability_revisions ADD COLUMN format TEXT NOT NULL DEFAULT 'legacy_mixed_v1';
             ALTER TABLE development_session_capabilities ADD COLUMN format TEXT NOT NULL DEFAULT 'legacy_mixed_v1';
             ALTER TABLE development_run_capabilities ADD COLUMN format TEXT NOT NULL DEFAULT 'legacy_mixed_v1';
             ALTER TABLE development_migrations ADD COLUMN cleanup_boot_identity TEXT;
             CREATE TABLE development_cli_session_bindings(
               owner TEXT NOT NULL CHECK(length(owner)>0), session_id TEXT NOT NULL,
               binding_id TEXT NOT NULL CHECK(length(binding_id)>0),
               execution_policy TEXT NOT NULL CHECK(execution_policy IN ('legacy_read_only','provider_bound')),
               project_id TEXT, task_id TEXT, revision INTEGER,
               permission_epoch INTEGER NOT NULL CHECK(typeof(permission_epoch)='integer' AND permission_epoch>=0),
               provider_id TEXT, model_id TEXT, provider_revision INTEGER,
               state TEXT NOT NULL CHECK(state IN ('opening','active','closed','unknown','revoked')),
               cleanup_state TEXT NOT NULL CHECK(cleanup_state IN ('not_required','pending','stopped','unknown')),
               cleanup_reason TEXT, host_instance_id TEXT, boot_identity TEXT,
               cleanup_proof TEXT CHECK(cleanup_proof IS NULL OR cleanup_proof IN ('owned_exit','boot_changed','no_execution')),
               request_digest TEXT CHECK(request_digest IS NULL OR length(CAST(request_digest AS BLOB))<=256),
               updated_at BIGINT NOT NULL,
               PRIMARY KEY(owner,session_id),
               FOREIGN KEY(session_id,owner) REFERENCES development_agent_sessions(id,owner) ON DELETE CASCADE,
               CHECK((execution_policy='legacy_read_only' AND revision IS NULL AND provider_id IS NULL AND model_id IS NULL AND provider_revision IS NULL AND state IN ('closed','unknown','revoked'))
                  OR (execution_policy='provider_bound' AND project_id IS NOT NULL AND task_id IS NOT NULL AND typeof(revision)='integer' AND revision>0 AND provider_id IS NOT NULL AND length(provider_id)>0 AND model_id IS NOT NULL AND length(model_id)>0 AND typeof(provider_revision)='integer' AND provider_revision>=0))
             );
             CREATE INDEX development_cli_provider_cleanup ON development_cli_session_bindings(owner,provider_id,provider_revision,state);
             CREATE INDEX development_cli_binding_state ON development_cli_session_bindings(owner,state,cleanup_state);
             CREATE TABLE development_cli_legacy_history(
               owner TEXT NOT NULL, session_id TEXT NOT NULL, original_project_id TEXT, original_task_id TEXT,
               session_record TEXT NOT NULL CHECK(json_valid(session_record) AND length(CAST(session_record AS BLOB))<=1048576),
               capability_snapshot TEXT CHECK(capability_snapshot IS NULL OR length(CAST(capability_snapshot AS BLOB))<=1048576),
               referenced_configuration TEXT CHECK(referenced_configuration IS NULL OR length(CAST(referenced_configuration AS BLOB))<=65536),
               original_revision INTEGER, original_permission_epoch INTEGER, archived_at BIGINT NOT NULL,
               PRIMARY KEY(owner,session_id),
               FOREIGN KEY(session_id,owner) REFERENCES development_agent_sessions(id,owner) ON DELETE CASCADE
             );
             CREATE TRIGGER development_cli_binding_insert_guard BEFORE INSERT ON development_cli_session_bindings
             WHEN NEW.execution_policy='provider_bound' AND NEW.state IN ('opening','active') AND NOT EXISTS(
               SELECT 1 FROM development_agent_sessions s JOIN development_tasks t ON t.id=s.task_id AND t.project_id=s.project_id AND t.owner=s.owner
               JOIN development_capability_revisions r ON r.owner=s.owner AND r.project_id=s.project_id AND r.revision=NEW.revision AND r.format='cli_provider_v1'
               WHERE s.id=NEW.session_id AND s.owner=NEW.owner AND s.project_id=NEW.project_id AND s.task_id=NEW.task_id AND t.id=NEW.task_id
                 AND s.engine=json_extract(r.configuration,'$.engine') AND json_extract(r.configuration,'$.provider_id')=NEW.provider_id AND json_extract(r.configuration,'$.model_id')=NEW.model_id AND json_extract(r.configuration,'$.provider_revision')=NEW.provider_revision
             )
             BEGIN SELECT RAISE(ABORT,'invalid_cli_binding'); END;
             CREATE TRIGGER development_cli_binding_update_guard BEFORE UPDATE ON development_cli_session_bindings
             WHEN (OLD.execution_policy='legacy_read_only' AND NEW.execution_policy<>'legacy_read_only')
               OR (OLD.state='revoked' AND NEW.state<>'revoked')
               OR (OLD.binding_id<>NEW.binding_id AND NOT (OLD.execution_policy='provider_bound' AND OLD.state='closed' AND OLD.cleanup_state IN ('stopped','not_required') AND NEW.execution_policy='provider_bound' AND NEW.state='opening' AND NEW.cleanup_state='pending'))
               OR (OLD.owner<>NEW.owner OR OLD.session_id<>NEW.session_id)
               OR (NEW.permission_epoch<OLD.permission_epoch)
               OR (OLD.execution_policy='provider_bound' AND (NEW.execution_policy<>OLD.execution_policy OR NEW.project_id IS NOT OLD.project_id OR NEW.task_id IS NOT OLD.task_id OR NEW.revision IS NOT OLD.revision OR NEW.permission_epoch<>OLD.permission_epoch OR NEW.provider_id IS NOT OLD.provider_id OR NEW.model_id IS NOT OLD.model_id OR NEW.provider_revision IS NOT OLD.provider_revision))
               OR (OLD.state IN ('closed','unknown','revoked') AND NEW.state IN ('opening','active') AND NOT (OLD.state='closed' AND NEW.state='opening' AND OLD.cleanup_state IN ('stopped','not_required') AND NEW.binding_id<>OLD.binding_id AND NEW.cleanup_state='pending'))
               OR (OLD.state='active' AND NEW.state='opening')
               OR (NEW.execution_policy='provider_bound' AND NEW.state IN ('opening','active') AND NOT EXISTS(
                 SELECT 1 FROM development_agent_sessions s JOIN development_tasks t ON t.id=s.task_id AND t.project_id=s.project_id AND t.owner=s.owner
                 JOIN development_capability_revisions r ON r.owner=s.owner AND r.project_id=s.project_id AND r.revision=NEW.revision AND r.format='cli_provider_v1'
                 WHERE s.id=NEW.session_id AND s.owner=NEW.owner AND s.project_id=NEW.project_id AND s.task_id=NEW.task_id AND s.engine=json_extract(r.configuration,'$.engine')
                   AND json_extract(r.configuration,'$.provider_id')=NEW.provider_id AND json_extract(r.configuration,'$.model_id')=NEW.model_id AND json_extract(r.configuration,'$.provider_revision')=NEW.provider_revision
               ))
             BEGIN SELECT RAISE(ABORT,'invalid_cli_binding'); END;
             CREATE TRIGGER development_cli_binding_unlink_guard BEFORE UPDATE OF project_id,task_id ON development_agent_sessions
             WHEN (NEW.project_id IS NOT OLD.project_id OR NEW.task_id IS NOT OLD.task_id) AND EXISTS(
               SELECT 1 FROM development_cli_session_bindings b WHERE b.owner=OLD.owner AND b.session_id=OLD.id AND b.state IN ('opening','active')
             )
             BEGIN SELECT RAISE(ABORT,'cli_binding_active'); END;
             CREATE TRIGGER development_cli_session_identity_guard BEFORE UPDATE OF native_id,workspace,executable,model,engine,owner ON development_agent_sessions
             WHEN EXISTS(SELECT 1 FROM development_cli_session_bindings b WHERE b.owner=OLD.owner AND b.session_id=OLD.id)
               AND (NEW.workspace IS NOT OLD.workspace OR NEW.executable IS NOT OLD.executable OR NEW.model IS NOT OLD.model OR NEW.engine IS NOT OLD.engine OR NEW.owner IS NOT OLD.owner
                 OR (NEW.native_id IS NOT OLD.native_id AND NOT (OLD.native_id IS NULL AND NEW.native_id IS NOT NULL AND EXISTS(SELECT 1 FROM development_cli_session_bindings b WHERE b.owner=OLD.owner AND b.session_id=OLD.id AND b.execution_policy='provider_bound' AND b.state='opening'))))
             BEGIN SELECT RAISE(ABORT,'invalid_cli_binding'); END;
             CREATE TRIGGER development_cli_session_snapshot_guard BEFORE INSERT ON development_session_capabilities
             WHEN NEW.format='cli_provider_v1' AND NOT EXISTS(
               SELECT 1 FROM development_cli_session_bindings b JOIN development_agent_sessions s ON s.id=b.session_id AND s.owner=b.owner
               WHERE b.owner=NEW.owner AND b.session_id=NEW.session_id AND b.binding_id=json_extract(NEW.snapshot,'$.binding_id') AND b.execution_policy='provider_bound' AND b.project_id=NEW.project_id AND b.task_id=s.task_id AND b.revision=NEW.revision AND b.permission_epoch=NEW.permission_epoch
                 AND s.engine=json_extract(NEW.snapshot,'$.engine') AND b.provider_id=json_extract(NEW.snapshot,'$.provider_id') AND b.model_id=json_extract(NEW.snapshot,'$.model_id') AND b.provider_revision=json_extract(NEW.snapshot,'$.provider_revision')
                 AND json_extract(NEW.snapshot,'$.revision')=b.revision AND json_extract(NEW.snapshot,'$.permission_epoch')=b.permission_epoch
             )
             BEGIN SELECT RAISE(ABORT,'invalid_cli_snapshot'); END;
             CREATE TRIGGER development_cli_session_snapshot_update_guard BEFORE UPDATE ON development_session_capabilities
             WHEN NEW.format='cli_provider_v1' AND NOT EXISTS(
               SELECT 1 FROM development_cli_session_bindings b JOIN development_agent_sessions s ON s.id=b.session_id AND s.owner=b.owner
               WHERE b.owner=NEW.owner AND b.session_id=NEW.session_id AND b.binding_id=json_extract(NEW.snapshot,'$.binding_id') AND b.execution_policy='provider_bound' AND b.project_id=NEW.project_id AND b.task_id=s.task_id AND b.revision=NEW.revision AND b.permission_epoch=NEW.permission_epoch
                 AND s.engine=json_extract(NEW.snapshot,'$.engine') AND b.provider_id=json_extract(NEW.snapshot,'$.provider_id') AND b.model_id=json_extract(NEW.snapshot,'$.model_id') AND b.provider_revision=json_extract(NEW.snapshot,'$.provider_revision')
                 AND json_extract(NEW.snapshot,'$.revision')=b.revision AND json_extract(NEW.snapshot,'$.permission_epoch')=b.permission_epoch
             )
             BEGIN SELECT RAISE(ABORT,'invalid_cli_snapshot'); END;
             CREATE TRIGGER development_cli_session_format_guard BEFORE UPDATE ON development_session_capabilities
             WHEN OLD.format='cli_provider_v1' AND NEW.format<>'cli_provider_v1'
             BEGIN SELECT RAISE(ABORT,'legacy_session_read_only'); END;
             CREATE TRIGGER development_cli_run_immutable BEFORE UPDATE ON development_run_capabilities
             WHEN OLD.format='cli_provider_v1'
             BEGIN SELECT RAISE(ABORT,'invalid_cli_snapshot'); END;
             CREATE TRIGGER development_cli_run_snapshot_guard BEFORE INSERT ON development_run_capabilities
             WHEN NEW.format='cli_provider_v1' AND NOT EXISTS(
               SELECT 1 FROM development_runs r JOIN development_cli_session_bindings b ON b.owner=r.owner AND b.session_id=r.session_id AND b.binding_id=json_extract(NEW.snapshot,'$.binding_id') AND b.state='active' AND b.execution_policy='provider_bound'
                 JOIN development_agent_sessions s ON s.owner=b.owner AND s.id=b.session_id
               WHERE r.owner=NEW.owner AND r.id=NEW.run_id AND r.source='native_agent' AND r.engine=json_extract(NEW.snapshot,'$.engine') AND r.project_id=b.project_id AND r.task_id=b.task_id AND s.engine=r.engine
                 AND b.revision=json_extract(NEW.snapshot,'$.revision') AND b.permission_epoch=json_extract(NEW.snapshot,'$.permission_epoch') AND b.provider_id=json_extract(NEW.snapshot,'$.provider_id') AND b.model_id=json_extract(NEW.snapshot,'$.model_id') AND b.provider_revision=json_extract(NEW.snapshot,'$.provider_revision')
             )
             BEGIN SELECT RAISE(ABORT,'invalid_cli_snapshot'); END;
             CREATE TRIGGER development_cli_run_snapshot_update_guard BEFORE UPDATE ON development_run_capabilities
             WHEN NEW.format='cli_provider_v1' AND NOT EXISTS(
               SELECT 1 FROM development_runs r JOIN development_cli_session_bindings b ON b.owner=r.owner AND b.session_id=r.session_id AND b.binding_id=json_extract(NEW.snapshot,'$.binding_id') AND b.state='active' AND b.execution_policy='provider_bound'
                 JOIN development_agent_sessions s ON s.owner=b.owner AND s.id=b.session_id
               WHERE r.owner=NEW.owner AND r.id=NEW.run_id AND r.source='native_agent' AND r.engine=json_extract(NEW.snapshot,'$.engine') AND r.project_id=b.project_id AND r.task_id=b.task_id AND s.engine=r.engine
                 AND b.revision=json_extract(NEW.snapshot,'$.revision') AND b.permission_epoch=json_extract(NEW.snapshot,'$.permission_epoch') AND b.provider_id=json_extract(NEW.snapshot,'$.provider_id') AND b.model_id=json_extract(NEW.snapshot,'$.model_id') AND b.provider_revision=json_extract(NEW.snapshot,'$.provider_revision')
             )
             BEGIN SELECT RAISE(ABORT,'invalid_cli_snapshot'); END;"
        ).await?;
        for table in ["development_capability_revisions", "development_session_capabilities", "development_run_capabilities"] {
            for event in ["INSERT", "UPDATE"] {
                tx.execute_unprepared(&format!("CREATE TRIGGER {table}_format_{event} BEFORE {event} ON {table} WHEN NEW.format NOT IN ('legacy_mixed_v1','cli_provider_v1') BEGIN SELECT RAISE(ABORT,'invalid_capability_format'); END;")).await?;
            }
        }
        tx.commit().await
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("CLI Provider cutover is forward-only".into()))
    }
}
