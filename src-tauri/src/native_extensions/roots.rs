use dgos_agents::native_extensions::{Error, Target, TrustedRoots};
use std::path::{Component, Path, PathBuf};
#[derive(Clone)]
pub struct Roots { pub home: PathBuf, pub codex: PathBuf, pub claude: PathBuf, pub claude_user: PathBuf, pub journal: PathBuf }
fn absolute(path: PathBuf) -> Result<PathBuf, Error> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) { return Err(Error::UnsafePath); }
    Ok(path)
}
impl Roots {
    /// Captures trusted process startup paths; performs no configuration reads.
    pub fn from_environment(private_root: PathBuf) -> Result<Self, Error> {
        let home = absolute(std::env::var_os("HOME").map(PathBuf::from).ok_or(Error::Unavailable)?)?;
        let codex = absolute(std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex")))?;
        let claude_override = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
        let claude = absolute(claude_override.clone().unwrap_or_else(|| home.join(".claude")))?;
        // Coordinator's isolated installed-CLI evidence: override/.claude.json.
        let claude_user = claude_override.map(|_| claude.join(".claude.json")).unwrap_or_else(|| home.join(".claude.json"));
        Ok(Self { home, codex, claude, claude_user, journal: absolute(private_root)? })
    }
    pub fn trusted(&self, project: Option<PathBuf>) -> TrustedRoots {
        TrustedRoots { os_home: self.home.clone(), codex_home: self.codex.clone(), claude_home: self.claude.clone(), claude_user_file: self.claude_user.clone(), project, journal: self.journal.clone() }
    }
    pub fn skill_path(&self, target: Target, project: Option<&Path>, slug: &str) -> PathBuf {
        match (target, project) { (Target::Codex, None) => self.home.join(".agents/skills"), (Target::Codex, Some(p)) => p.join(".agents/skills"), (Target::Claude, None) => self.claude.join("skills"), (Target::Claude, Some(p)) => p.join(".claude/skills") }.join(slug).join("SKILL.md")
    }
}
pub fn validate_project(project: &dgos_db::development_repository::Project) -> Result<PathBuf, String> {
    if project.archived { return Err("project_archived".into()); }
    let path = std::fs::canonicalize(&project.path).map_err(|_| "workspace_changed")?;
    let meta = std::fs::metadata(&path).map_err(|_| "workspace_changed")?;
    #[cfg(unix)]
    let identity = { use std::os::unix::fs::MetadataExt; format!("{}:{}", meta.dev(), meta.ino()) };
    #[cfg(not(unix))]
    return Err("native_extension_Unsupported".into());
    #[cfg(unix)]
    if !meta.is_dir() || path.to_str() != Some(project.path.as_str()) || identity != project.directory_identity { return Err("workspace_changed".into()); }
    Ok(path)
}
