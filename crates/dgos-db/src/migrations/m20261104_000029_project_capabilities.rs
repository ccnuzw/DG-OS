//! Stage-4 project configuration revisions and inert session/run capability snapshots.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "CREATE TABLE development_capability_revisions(
                owner TEXT NOT NULL CHECK(length(owner)>0),
                project_id TEXT NOT NULL,
                revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>0),
                configuration TEXT NOT NULL CHECK(length(CAST(configuration AS BLOB))<=65536 AND json_valid(configuration)),
                created_at BIGINT NOT NULL,
                PRIMARY KEY(owner,project_id,revision),
                FOREIGN KEY(project_id,owner) REFERENCES development_projects(id,owner) ON DELETE CASCADE);
            CREATE TABLE development_capability_heads(
                owner TEXT NOT NULL CHECK(length(owner)>0),
                project_id TEXT NOT NULL,
                revision INTEGER CHECK(revision IS NULL OR (typeof(revision)='integer' AND revision>0)),
                permission_epoch INTEGER NOT NULL CHECK(typeof(permission_epoch)='integer' AND permission_epoch>=0),
                PRIMARY KEY(owner,project_id),
                FOREIGN KEY(project_id,owner) REFERENCES development_projects(id,owner) ON DELETE CASCADE,
                FOREIGN KEY(owner,project_id,revision) REFERENCES development_capability_revisions(owner,project_id,revision));
            CREATE TRIGGER development_capability_epoch_monotonic BEFORE UPDATE OF permission_epoch ON development_capability_heads
                WHEN NEW.permission_epoch<OLD.permission_epoch
                BEGIN SELECT RAISE(ABORT,'capability_epoch_regression'); END;
            CREATE TABLE development_session_capabilities(
                owner TEXT NOT NULL CHECK(length(owner)>0),
                session_id TEXT NOT NULL,
                project_id TEXT NOT NULL,
                revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>=0),
                permission_epoch INTEGER NOT NULL CHECK(typeof(permission_epoch)='integer' AND permission_epoch>=0),
                snapshot TEXT NOT NULL CHECK(length(CAST(snapshot AS BLOB))<=1048576 AND json_valid(snapshot)),
                PRIMARY KEY(owner,session_id),
                FOREIGN KEY(session_id,owner) REFERENCES development_agent_sessions(id,owner) ON DELETE CASCADE,
                FOREIGN KEY(project_id,owner) REFERENCES development_projects(id,owner) ON DELETE CASCADE);
            CREATE TRIGGER development_session_capabilities_insert BEFORE INSERT ON development_session_capabilities
                WHEN NOT EXISTS(SELECT 1 FROM development_agent_sessions s JOIN development_tasks t ON t.id=s.task_id AND t.project_id=s.project_id AND t.owner=s.owner WHERE s.id=NEW.session_id AND s.owner=NEW.owner AND s.project_id=NEW.project_id)
                OR (NEW.revision>0 AND NOT EXISTS(SELECT 1 FROM development_capability_revisions r WHERE r.owner=NEW.owner AND r.project_id=NEW.project_id AND r.revision=NEW.revision))
                BEGIN SELECT RAISE(ABORT,'invalid_capabilities'); END;
            CREATE TRIGGER development_session_capabilities_update BEFORE UPDATE ON development_session_capabilities
                WHEN NOT EXISTS(SELECT 1 FROM development_agent_sessions s JOIN development_tasks t ON t.id=s.task_id AND t.project_id=s.project_id AND t.owner=s.owner WHERE s.id=NEW.session_id AND s.owner=NEW.owner AND s.project_id=NEW.project_id)
                OR (NEW.revision>0 AND NOT EXISTS(SELECT 1 FROM development_capability_revisions r WHERE r.owner=NEW.owner AND r.project_id=NEW.project_id AND r.revision=NEW.revision))
                BEGIN SELECT RAISE(ABORT,'invalid_capabilities'); END;
            CREATE TRIGGER development_session_capabilities_unlink BEFORE UPDATE OF project_id,task_id ON development_agent_sessions
                WHEN NEW.project_id IS NOT OLD.project_id OR NEW.task_id IS NOT OLD.task_id
                BEGIN DELETE FROM development_session_capabilities WHERE owner=OLD.owner AND session_id=OLD.id; END;
            CREATE TABLE development_run_capabilities(
                owner TEXT NOT NULL CHECK(length(owner)>0),
                run_id TEXT NOT NULL,
                snapshot TEXT NOT NULL CHECK(length(CAST(snapshot AS BLOB))<=1048576 AND json_valid(snapshot)),
                PRIMARY KEY(owner,run_id),
                FOREIGN KEY(run_id,owner) REFERENCES development_runs(id,owner) ON DELETE CASCADE);
            CREATE TRIGGER development_run_capabilities_insert BEFORE INSERT ON development_run_capabilities
                WHEN NOT EXISTS(SELECT 1 FROM development_runs r JOIN development_tasks t ON t.id=r.task_id AND t.project_id=r.project_id AND t.owner=r.owner JOIN development_projects p ON p.id=t.project_id AND p.owner=t.owner WHERE r.id=NEW.run_id AND r.owner=NEW.owner AND (r.session_id IS NULL OR EXISTS(SELECT 1 FROM development_agent_sessions s WHERE s.id=r.session_id AND s.owner=r.owner AND s.project_id=r.project_id AND s.task_id=r.task_id)))
                BEGIN SELECT RAISE(ABORT,'invalid_capabilities'); END;
            CREATE TRIGGER development_run_capabilities_update BEFORE UPDATE ON development_run_capabilities
                WHEN NOT EXISTS(SELECT 1 FROM development_runs r JOIN development_tasks t ON t.id=r.task_id AND t.project_id=r.project_id AND t.owner=r.owner JOIN development_projects p ON p.id=t.project_id AND p.owner=t.owner WHERE r.id=NEW.run_id AND r.owner=NEW.owner AND (r.session_id IS NULL OR EXISTS(SELECT 1 FROM development_agent_sessions s WHERE s.id=r.session_id AND s.owner=r.owner AND s.project_id=r.project_id AND s.task_id=r.task_id)))
                BEGIN SELECT RAISE(ABORT,'invalid_capabilities'); END;"
        ).await.map(|_| ())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("project capabilities are forward-only".into()))
    }
}
