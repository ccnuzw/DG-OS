use super::*;
use super::projects::require_owner;

pub const DESKTOP_STATE_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DesktopState { pub revision: i64, pub value: serde_json::Value }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceTrust {
    pub path: String, pub directory_identity: String,
    pub agent: bool, pub agent_revoked: bool, pub terminal: bool, pub service: bool, pub revision: i64,
}
impl WorkspaceTrust {
    pub fn permits(&self, scope: &str) -> bool {
        match scope { "agent" => self.agent, "terminal" => self.terminal, "service" => self.service, _ => false }
    }
}
fn desktop_value(value: &serde_json::Value, depth: usize) -> Result<(), String> {
    if depth > 64 { return Err("invalid_desktop_state".into()); }
    match value {
        serde_json::Value::Object(object) => for (key, value) in object {
            let key: String = key.chars().filter(|c| *c != '_' && *c != '-').flat_map(char::to_lowercase).collect();
            if matches!(key.as_str(), "apikey"|"accesstoken"|"refreshtoken"|"password"|"credentials"|"secret"|"authorization"|"approval"|"approvals"|"runtime"|"trust"|"workspacetrust"|"pid") {
                return Err("invalid_desktop_state".into());
            }
            desktop_value(value, depth + 1)?;
        },
        serde_json::Value::Array(array) => for value in array { desktop_value(value, depth + 1)?; },
        _ => {},
    }
    Ok(())
}
fn live(valid: &impl Fn() -> bool) -> Result<(), String> {
    if valid() { Ok(()) } else { Err("session_invalid".into()) }
}
impl DevelopmentRepository {
    pub async fn desktop_state(&self, owner: &str) -> Result<DesktopState, String> {
        require_owner(owner)?;
        match fetch_one(&self.db, "SELECT revision,value FROM development_desktop_state WHERE owner=?", vec![v(owner)]).await? {
            Some(row) => Ok(DesktopState { revision: row.try_get("", "revision").map_err(db)?, value: serde_json::from_str(&row.try_get::<String>("", "value").map_err(db)?).map_err(|_| "storage_failed")? }),
            None => Ok(DesktopState { revision: 0, value: serde_json::json!({}) }),
        }
    }
    /// An exact revision CAS; the account lease is checked again before commit.
    pub async fn save_desktop_state(&self, owner: &str, expected: i64, value: serde_json::Value, valid: impl Fn() -> bool) -> Result<DesktopState, String> {
        require_owner(owner)?; live(&valid)?;
        if expected < 0 || expected == i64::MAX || !value.is_object() { return Err("invalid_desktop_state".into()); }
        desktop_value(&value, 0)?;
        let text = serde_json::to_string(&value).map_err(|_| "invalid_desktop_state")?;
        if text.len() > DESKTOP_STATE_LIMIT { return Err("desktop_state_limit".into()); }
        let tx = self.db.begin().await.map_err(db)?;
        let changed = if expected == 0 {
            execute(&tx, "INSERT INTO development_desktop_state(owner,revision,value) VALUES(?,1,?) ON CONFLICT(owner) DO NOTHING", vec![v(owner),v(text)]).await?
        } else {
            execute(&tx, "UPDATE development_desktop_state SET revision=revision+1,value=? WHERE owner=? AND revision=?", vec![v(text),v(owner),v(expected)]).await?
        };
        if changed != 1 { tx.rollback().await.map_err(db)?; return Err("desktop_state_conflict".into()); }
        if let Err(error) = live(&valid) { tx.rollback().await.map_err(db)?; return Err(error); }
        tx.commit().await.map_err(db)?;
        Ok(DesktopState { revision: expected + 1, value })
    }
    pub async fn workspace_trust(&self, owner: &str, project: &Project) -> Result<WorkspaceTrust, String> {
        project_row(&self.db, owner, &project.id).await?;
        let mut trust = WorkspaceTrust { path: project.path.clone(), directory_identity: project.directory_identity.clone(), agent: false, agent_revoked: false, terminal: false, service: false, revision: 0 };
        if let Some(row) = fetch_one(&self.db, "SELECT agent,agent_revoked,terminal,service,revision FROM development_workspace_trust WHERE owner=? AND project_id=? AND path=? AND directory_identity=?", vec![v(owner),v(project.id.clone()),v(project.path.clone()),v(project.directory_identity.clone())]).await? {
            trust.agent = row.try_get::<i64>("", "agent").map_err(db)? != 0;
            trust.agent_revoked = row.try_get::<i64>("", "agent_revoked").map_err(db)? != 0;
            trust.terminal = row.try_get::<i64>("", "terminal").map_err(db)? != 0;
            trust.service = row.try_get::<i64>("", "service").map_err(db)? != 0;
            trust.revision = row.try_get("", "revision").map_err(db)?;
        }
        Ok(trust)
    }
    pub async fn set_workspace_trust(&self, owner: &str, project: &Project, expected: i64, scope: &str, granted: bool, valid: impl Fn() -> bool) -> Result<WorkspaceTrust, String> {
        if !matches!(scope, "agent"|"terminal"|"service") || expected < 0 || expected == i64::MAX { return Err("invalid_workspace_trust".into()); }
        live(&valid)?;
        let tx = self.db.begin().await.map_err(db)?;
        project_row(&tx, owner, &project.id).await?;
        let denied = if scope == "agent" { ",agent_revoked" } else { "" };
        let changed = if expected == 0 {
            // Scope is an enum allow-list above, never renderer SQL.
            let mut values = vec![v(owner),v(project.id.clone()),v(project.path.clone()),v(project.directory_identity.clone()),v(granted as i64)];
            if scope == "agent" { values.push(v((!granted) as i64)); }
            let marker = if scope == "agent" { ",?" } else { "" };
            execute(&tx, &format!("INSERT INTO development_workspace_trust(owner,project_id,path,directory_identity,{scope}{denied},revision) VALUES(?,?,?,?,?{marker},1) ON CONFLICT DO NOTHING"), values).await?
        } else {
            let mut values = vec![v(granted as i64)];
            let marker = if scope == "agent" { values.push(v((!granted) as i64)); ",agent_revoked=?" } else { "" };
            values.extend([v(owner),v(project.id.clone()),v(project.path.clone()),v(project.directory_identity.clone()),v(expected)]);
            execute(&tx, &format!("UPDATE development_workspace_trust SET {scope}=?{marker},revision=revision+1 WHERE owner=? AND project_id=? AND path=? AND directory_identity=? AND revision=?"), values).await?
        };
        if changed != 1 { tx.rollback().await.map_err(db)?; return Err("workspace_trust_conflict".into()); }
        if let Err(error) = live(&valid) { tx.rollback().await.map_err(db)?; return Err(error); }
        tx.commit().await.map_err(db)?;
        self.workspace_trust(owner, project).await
    }
    /// Activation is an explicit host action. Reads never call this method.
    /// The admission owner validates the selected project/task/directory first.
    pub async fn activate_workspace_agent(&self, owner: &str, project: &Project, expected: i64, valid: impl Fn() -> bool) -> Result<WorkspaceTrust, String> {
        live(&valid)?;
        let trust = self.workspace_trust(owner, project).await?;
        if expected < 0 || expected == i64::MAX { return Err("invalid_workspace_trust".into()); }
        if trust.revision != expected { return Err("workspace_trust_conflict".into()); }
        if trust.agent || trust.agent_revoked { live(&valid)?; return Ok(trust); }
        // Explicit revocation and activation share the revision CAS, so a
        // revocation racing this action cannot be overwritten.
        self.set_workspace_trust(owner, project, expected, "agent", true, valid).await
    }
}
