use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Epochs deliberately have no module FK: uninstall must retain tombstones.
        // CHECK rejects integer overflow (SQLite would otherwise promote to REAL).
        manager.get_connection().execute_unprepared(r#"
            CREATE TABLE module_authorization_epochs (
                user_id TEXT NOT NULL, module_id TEXT NOT NULL,
                epoch INTEGER NOT NULL CHECK(typeof(epoch) = 'integer' AND epoch >= 0),
                PRIMARY KEY(user_id, module_id)
            );
            INSERT INTO module_authorization_epochs(user_id,module_id,epoch)
                SELECT DISTINCT user_id,module_id,0 FROM module_permissions;
            -- INSERT OR REPLACE can suppress DELETE triggers in SQLite. The
            -- pre-insert collision check still invalidates the replaced grant.
            CREATE TRIGGER module_epoch_permission_replace BEFORE INSERT ON module_permissions
            WHEN EXISTS(SELECT 1 FROM module_permissions WHERE id=NEW.id OR (user_id=NEW.user_id AND module_id=NEW.module_id AND permission=NEW.permission)) BEGIN
                UPDATE module_authorization_epochs SET epoch=epoch+1
                WHERE (user_id,module_id) IN (SELECT user_id,module_id FROM module_permissions WHERE id=NEW.id OR (user_id=NEW.user_id AND module_id=NEW.module_id AND permission=NEW.permission));
            END;
            CREATE TRIGGER module_epoch_permission_insert AFTER INSERT ON module_permissions BEGIN
                INSERT INTO module_authorization_epochs(user_id,module_id,epoch)
                VALUES(NEW.user_id,NEW.module_id,CASE WHEN NEW.granted = 0 THEN 1 ELSE 0 END)
                ON CONFLICT(user_id,module_id) DO UPDATE SET epoch = epoch + CASE WHEN NEW.granted = 0 THEN 1 ELSE 0 END;
            END;
            CREATE TRIGGER module_epoch_permission_revoke AFTER UPDATE ON module_permissions
            WHEN NEW.granted = 0 OR OLD.user_id IS NOT NEW.user_id OR OLD.module_id IS NOT NEW.module_id OR OLD.permission IS NOT NEW.permission BEGIN
                INSERT INTO module_authorization_epochs(user_id,module_id,epoch) VALUES(OLD.user_id,OLD.module_id,1)
                ON CONFLICT(user_id,module_id) DO UPDATE SET epoch = epoch + 1;
            END;
            CREATE TRIGGER module_epoch_permission_delete AFTER DELETE ON module_permissions BEGIN
                INSERT INTO module_authorization_epochs(user_id,module_id,epoch) VALUES(OLD.user_id,OLD.module_id,1)
                ON CONFLICT(user_id,module_id) DO UPDATE SET epoch = epoch + 1;
            END;
            CREATE TRIGGER module_epoch_module_update AFTER UPDATE OF is_enabled,current_version,integrity,manifest ON modules
            WHEN NEW.is_enabled = 0 OR OLD.current_version IS NOT NEW.current_version OR OLD.integrity IS NOT NEW.integrity OR OLD.manifest IS NOT NEW.manifest BEGIN
                UPDATE module_authorization_epochs SET epoch = epoch + 1 WHERE module_id = OLD.id;
            END;
            CREATE TRIGGER module_epoch_module_delete BEFORE DELETE ON modules BEGIN
                UPDATE module_authorization_epochs SET epoch = epoch + 1 WHERE module_id = OLD.id;
            END;
            CREATE TRIGGER module_epoch_module_replace BEFORE INSERT ON modules
            WHEN EXISTS(SELECT 1 FROM modules WHERE id=NEW.id OR module_name=NEW.module_name) BEGIN
                UPDATE module_authorization_epochs SET epoch=epoch+1
                WHERE module_id IN (SELECT id FROM modules WHERE id=NEW.id OR module_name=NEW.module_name);
            END;
            CREATE TRIGGER module_epoch_account_deleting AFTER UPDATE OF deleting_since ON users
            WHEN NEW.deleting_since IS NOT NULL BEGIN
                UPDATE module_authorization_epochs SET epoch = epoch + 1 WHERE user_id = OLD.id;
            END;
            CREATE UNIQUE INDEX idx_workbench_tasks_id_owner ON workbench_tasks(id,user_id);
            CREATE TABLE ai_run_bindings (
                task_id TEXT PRIMARY KEY NOT NULL REFERENCES workbench_tasks(id) ON DELETE CASCADE,
                user_id TEXT NOT NULL, module_id TEXT NOT NULL,
                module_version TEXT NOT NULL, module_integrity TEXT NOT NULL,
                runtime_id TEXT NOT NULL, request_id TEXT NOT NULL UNIQUE,
                authorization_epoch INTEGER NOT NULL CHECK(typeof(authorization_epoch) = 'integer' AND authorization_epoch >= 0),
                skill_refs TEXT NOT NULL, tool_refs TEXT NOT NULL,
                terminal_status TEXT CHECK(terminal_status IN ('succeeded','failed','cancelled','interrupted')),
                error_domain TEXT, error_code INTEGER, terminal_reason TEXT,
                result_unknown INTEGER NOT NULL DEFAULT 0 CHECK(result_unknown IN (0,1)),
                audit_warning INTEGER NOT NULL DEFAULT 0 CHECK(audit_warning IN (0,1)),
                dispatch_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(dispatch_count) = 'integer' AND dispatch_count >= 0),
                last_dispatch_at BIGINT,
                FOREIGN KEY(task_id,user_id) REFERENCES workbench_tasks(id,user_id) ON DELETE CASCADE
            );
            CREATE INDEX idx_ai_runs_owner_module ON ai_run_bindings(user_id,module_id,task_id);
            CREATE TRIGGER ai_run_binding_source BEFORE INSERT ON ai_run_bindings
            WHEN NOT EXISTS(SELECT 1 FROM workbench_tasks WHERE id=NEW.task_id AND user_id=NEW.user_id AND app_id=NEW.module_id AND kind='single' AND status='running')
            BEGIN SELECT RAISE(ABORT,'invalid AI task binding'); END;
            CREATE TRIGGER ai_run_binding_immutable BEFORE UPDATE OF task_id,user_id,module_id,module_version,module_integrity,runtime_id,request_id,authorization_epoch,skill_refs,tool_refs ON ai_run_bindings
            BEGIN SELECT RAISE(ABORT,'immutable AI run binding'); END;
            CREATE TRIGGER ai_run_task_source_immutable BEFORE UPDATE OF id,user_id,app_id,kind ON workbench_tasks
            WHEN EXISTS(SELECT 1 FROM ai_run_bindings WHERE task_id=OLD.id)
            BEGIN SELECT RAISE(ABORT,'immutable AI task source'); END;
            -- Existing Workbench startup recovery writes task status directly.
            CREATE TRIGGER ai_run_task_recovered AFTER UPDATE OF status ON workbench_tasks
            WHEN OLD.status='running' AND NEW.status='interrupted' BEGIN
                UPDATE ai_run_bindings SET terminal_status='interrupted',
                    error_domain='ai', error_code=NULL, terminal_reason='process_interrupted', result_unknown=1
                WHERE task_id=NEW.id AND terminal_status IS NULL;
            END;
            -- Legacy ref tables have restrictive FKs; remove children before task deletion.
            CREATE TRIGGER ai_run_task_delete_refs BEFORE DELETE ON workbench_tasks
            WHEN EXISTS(SELECT 1 FROM ai_run_bindings WHERE task_id=OLD.id) BEGIN
                DELETE FROM workbench_task_skills WHERE task_id=OLD.id;
                DELETE FROM workbench_task_tools WHERE task_id=OLD.id;
            END;
        "#).await.map(|_| ())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("AI run audit and authorization tombstones are forward-only".into()))
    }
}
