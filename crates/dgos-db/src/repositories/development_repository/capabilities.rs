//! Bounded JSON configuration and inert audit snapshots. Epochs are live revocation
//! counters; reading a historical revision or snapshot never grants permission.
use super::*;

const MAX_CONFIGURATION: usize = 65_536;
const MAX_SNAPSHOT: usize = 1_048_576;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCapabilityRevision {
    pub revision: i64,
    /// Current project epoch, including when reading a historical revision.
    pub permission_epoch: i64,
    pub configuration: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSessionCapabilities {
    pub project_id: String,
    /// Zero records native capabilities without a project configuration revision.
    pub revision: i64,
    pub permission_epoch: i64,
    pub snapshot: String,
}

impl DevelopmentRepository {
    /// Read the current configuration, or an immutable historical revision, for this owner.
    pub async fn capability_revision(&self, owner: &str, project_id: &str, revision: Option<i64>) -> Result<Option<StoredCapabilityRevision>, String> {
        project_row(&self.db, owner, project_id).await?;
        let row = fetch_one(&self.db,
            "SELECT r.revision,h.permission_epoch,r.configuration,r.created_at FROM development_capability_revisions r JOIN development_capability_heads h ON h.owner=r.owner AND h.project_id=r.project_id WHERE r.owner=? AND r.project_id=? AND r.revision=COALESCE(?,h.revision) AND r.format='legacy_mixed_v1'",
            vec![v(owner), v(project_id), v(revision)]).await?;
        row.as_ref().map(revision_from).transpose()
    }

    /// Read only Provider configurations; legacy snapshots never enter this decoder.
    pub async fn cli_provider_revision(&self, owner: &str, project_id: &str, revision: Option<i64>) -> Result<Option<StoredCapabilityRevision>, String> {
        project_row(&self.db, owner, project_id).await?;
        let row = fetch_one(&self.db,
            "SELECT r.revision,h.permission_epoch,r.configuration,r.created_at FROM development_capability_revisions r JOIN development_capability_heads h ON h.owner=r.owner AND h.project_id=r.project_id WHERE r.owner=? AND r.project_id=? AND r.revision=COALESCE(?,h.revision) AND r.format='cli_provider_v1'",
            vec![v(owner), v(project_id), v(revision)]).await?;
        row.as_ref().map(revision_from).transpose()
    }

    /// Compare-and-swap the head. Initial writes expect zero; every accepted write appends
    /// one revision without changing the revocation epoch. No external work is done here.
    pub async fn save_capability_revision(&self, owner: &str, project_id: &str, expected_revision: i64, configuration: &str) -> Result<StoredCapabilityRevision, String> {
        check_json(configuration, MAX_CONFIGURATION)?;
        if expected_revision < 0 { return Err("invalid_capabilities".into()); }
        let revision = expected_revision.checked_add(1).ok_or("capability_conflict")?;
        let tx = self.db.begin().await.map_err(db)?;
        // Acquire SQLite's writer lock before reading the head, so concurrent CAS writes
        // observe the committed head instead of failing a deferred read-to-write upgrade.
        execute(&tx, "UPDATE development_capability_heads SET permission_epoch=permission_epoch WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?;
        require_legacy(&tx).await?;
        active_project(&tx, owner, project_id).await?;
        let head = head(&tx, owner, project_id).await?;
        let (current, epoch) = head.unwrap_or((0, 0));
        if current != expected_revision { return Err("capability_conflict".into()); }
        execute(&tx, "INSERT INTO development_capability_revisions(owner,project_id,revision,configuration,created_at,format) VALUES(?,?,?,?,unixepoch(),'legacy_mixed_v1')", vec![v(owner), v(project_id), v(revision), v(configuration)]).await?;
        let changed = execute(&tx,
            "INSERT INTO development_capability_heads(owner,project_id,revision,permission_epoch) VALUES(?,?,?,?) ON CONFLICT(owner,project_id) DO UPDATE SET revision=excluded.revision WHERE COALESCE(development_capability_heads.revision,0)=?",
            vec![v(owner), v(project_id), v(revision), v(epoch), v(expected_revision)]).await?;
        if changed != 1 { return Err("capability_conflict".into()); }
        let row = fetch_one(&tx, "SELECT revision,configuration,created_at,? AS permission_epoch FROM development_capability_revisions WHERE owner=? AND project_id=? AND revision=?", vec![v(epoch), v(owner), v(project_id), v(revision)]).await?.ok_or("storage_failed")?;
        let stored = revision_from(&row)?;
        tx.commit().await.map_err(db)?;
        Ok(stored)
    }

    /// Revoke previously issued live grants, including before the first configuration.
    /// Archived projects can still revoke. Revision history is unchanged.
    pub async fn revoke_project_capabilities(&self, owner: &str, project_id: &str) -> Result<i64, String> {
        let tx = self.db.begin().await.map_err(db)?;
        execute(&tx, "UPDATE development_capability_heads SET permission_epoch=permission_epoch WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?;
        require_legacy(&tx).await?;
        project_row(&tx, owner, project_id).await?;
        let epoch = head(&tx, owner, project_id).await?.map(|(_, epoch)| epoch).unwrap_or(0)
            .checked_add(1).ok_or("storage_failed")?;
        execute(&tx, "INSERT INTO development_capability_heads(owner,project_id,revision,permission_epoch) VALUES(?,?,NULL,?) ON CONFLICT(owner,project_id) DO UPDATE SET permission_epoch=excluded.permission_epoch", vec![v(owner), v(project_id), v(epoch)]).await?;
        tx.commit().await.map_err(db)?;
        Ok(epoch)
    }

    pub async fn capability_epoch(&self, owner: &str, project_id: &str) -> Result<i64, String> {
        project_row(&self.db, owner, project_id).await?;
        Ok(head(&self.db, owner, project_id).await?.map(|(_, epoch)| epoch).unwrap_or(0))
    }

    /// Store the creation snapshot of a classified session. Native sessions use revision zero.
    /// The captured epoch is audit data and may precede the current live epoch.
    pub async fn save_session_capabilities(&self, owner: &str, session_id: &str, project_id: &str, revision: i64, permission_epoch: i64, snapshot: &str) -> Result<(), String> {
        check_json(snapshot, MAX_SNAPSHOT)?;
        if revision < 0 || permission_epoch < 0 { return Err("invalid_capabilities".into()); }
        let tx = self.db.begin().await.map_err(db)?;
        require_legacy(&tx).await?;
        active_project(&tx, owner, project_id).await?;
        let session = session_row(&tx, owner, session_id).await?;
        if session.project_id.as_deref() != Some(project_id) { return Err("invalid_capabilities".into()); }
        let task = task_row(&tx, owner, session.task_id.as_deref().ok_or("invalid_capabilities")?).await?;
        if task.project_id != project_id { return Err("invalid_capabilities".into()); }
        if revision > 0 && fetch_one(&tx, "SELECT revision FROM development_capability_revisions WHERE owner=? AND project_id=? AND revision=? AND format='legacy_mixed_v1'", vec![v(owner), v(project_id), v(revision)]).await?.is_none() {
            return Err("invalid_capabilities".into());
        }
        execute(&tx, "INSERT INTO development_session_capabilities(owner,session_id,project_id,revision,permission_epoch,snapshot) VALUES(?,?,?,?,?,?) ON CONFLICT(owner,session_id) DO UPDATE SET project_id=excluded.project_id,revision=excluded.revision,permission_epoch=excluded.permission_epoch,snapshot=excluded.snapshot", vec![v(owner), v(session_id), v(project_id), v(revision), v(permission_epoch), v(snapshot)]).await?;
        tx.commit().await.map_err(db)
    }

    pub async fn read_session_capabilities(&self, owner: &str, session_id: &str) -> Result<Option<StoredSessionCapabilities>, String> {
        let row = fetch_one(&self.db,
            "SELECT c.project_id,c.revision,c.permission_epoch,c.snapshot FROM development_session_capabilities c JOIN development_agent_sessions s ON s.id=c.session_id AND s.owner=c.owner AND s.project_id=c.project_id JOIN development_tasks t ON t.id=s.task_id AND t.project_id=s.project_id AND t.owner=s.owner WHERE c.owner=? AND c.session_id=?",
            vec![v(owner), v(session_id)]).await?;
        row.map(|r| Ok(StoredSessionCapabilities {
            project_id: r.try_get("", "project_id").map_err(db)?,
            revision: r.try_get("", "revision").map_err(db)?,
            permission_epoch: r.try_get("", "permission_epoch").map_err(db)?,
            snapshot: r.try_get("", "snapshot").map_err(db)?,
        })).transpose()
    }

    /// Attach evidence to the existing native/host run ID, never a parallel run ledger.
    pub async fn save_run_capabilities(&self, owner: &str, run_id: &str, snapshot: &str) -> Result<(), String> {
        check_json(snapshot, MAX_SNAPSHOT)?;
        require_legacy(&self.db).await?;
        let changed = execute(&self.db,
            "INSERT INTO development_run_capabilities(owner,run_id,snapshot) SELECT r.owner,r.id,? FROM development_runs r JOIN development_tasks t ON t.id=r.task_id AND t.project_id=r.project_id AND t.owner=r.owner JOIN development_projects p ON p.id=t.project_id AND p.owner=t.owner WHERE r.owner=? AND r.id=? AND (r.session_id IS NULL OR EXISTS(SELECT 1 FROM development_agent_sessions s WHERE s.id=r.session_id AND s.owner=r.owner AND s.project_id=r.project_id AND s.task_id=r.task_id)) ON CONFLICT(owner,run_id) DO UPDATE SET snapshot=excluded.snapshot",
            vec![v(snapshot), v(owner), v(run_id)]).await?;
        if changed == 0 { Err("run_not_found".into()) } else { Ok(()) }
    }

    pub async fn run_capabilities(&self, owner: &str, run_id: &str) -> Result<Option<String>, String> {
        fetch_one(&self.db, "SELECT c.snapshot FROM development_run_capabilities c JOIN development_runs r ON r.id=c.run_id AND r.owner=c.owner JOIN development_tasks t ON t.id=r.task_id AND t.project_id=r.project_id AND t.owner=r.owner JOIN development_projects p ON p.id=t.project_id AND p.owner=t.owner WHERE c.owner=? AND c.run_id=? AND (r.session_id IS NULL OR EXISTS(SELECT 1 FROM development_agent_sessions s WHERE s.id=r.session_id AND s.owner=r.owner AND s.project_id=r.project_id AND s.task_id=r.task_id))", vec![v(owner), v(run_id)]).await?
            .map(|row| row.try_get("", "snapshot").map_err(db)).transpose()
    }
}

async fn require_legacy<C: ConnectionTrait>(c: &C) -> Result<(),String> {
    if fetch_one(c,"SELECT 1 AS complete FROM development_migrations WHERE key='d136-cli-provider-v1'",vec![]).await?.is_some() { return Err("legacy_session_read_only".into()); }
    Ok(())
}

fn check_json(value: &str, maximum: usize) -> Result<(), String> {
    if value.len() > maximum || serde_json::from_str::<serde::de::IgnoredAny>(value).is_err() {
        Err("invalid_capabilities".into())
    } else { Ok(()) }
}

pub(super) async fn active_project<C: ConnectionTrait>(c: &C, owner: &str, project_id: &str) -> Result<(), String> {
    if project_row(c, owner, project_id).await?.archived { Err("project_archived".into()) } else { Ok(()) }
}

pub(super) async fn head<C: ConnectionTrait>(c: &C, owner: &str, project_id: &str) -> Result<Option<(i64, i64)>, String> {
    fetch_one(c, "SELECT COALESCE(revision,0) AS revision,permission_epoch FROM development_capability_heads WHERE owner=? AND project_id=?", vec![v(owner), v(project_id)]).await?
        .map(|r| Ok((r.try_get("", "revision").map_err(db)?, r.try_get("", "permission_epoch").map_err(db)?))).transpose()
}

fn revision_from(r: &QueryResult) -> Result<StoredCapabilityRevision, String> {
    Ok(StoredCapabilityRevision {
        revision: r.try_get("", "revision").map_err(db)?,
        permission_epoch: r.try_get("", "permission_epoch").map_err(db)?,
        configuration: r.try_get("", "configuration").map_err(db)?,
        created_at: r.try_get("", "created_at").map_err(db)?,
    })
}
