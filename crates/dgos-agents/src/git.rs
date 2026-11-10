//! Read-only project Git status and task-scoped delivery. UUID receipts describe
//! external actions; replay never executes Git again. The admission lock coordinates
//! only DGOS, not external Git. Mutation reviews require complete file versions.
use crate::{files, projects::{git_command, git_output}, AgentManager};
use dgos_db::development_repository::{GitOperationIntent, Project};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, BTreeSet}, path::{Path, PathBuf}, time::Duration};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
mod operations;
mod reconcile;
mod file_diff;
mod details;
mod restore;
mod commit;
pub use commit::{GitCommitDetails, GitCommitDiffInput, GitCommitFile, GitCommitInput};
pub use details::{GitDetails, GitDetailsInput, GitFileStats};
pub use file_diff::{ProjectDiffInput, ProjectDiffSource};
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitIndexEntry { pub path: String, pub mode: String, pub oid: String, pub stage: u8 }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitFileStatus { pub path: String, pub original_path: Option<String>, pub index: String, pub worktree: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitReview {
    pub task_id: String, pub kind: String, pub path: String,
    pub common_dir: Option<String>, pub common_identity: Option<String>,
    pub branch: Option<String>, pub head: Option<String>, pub merge_heads: Vec<String>,
    pub files: Vec<GitFileStatus>, pub index: Vec<GitIndexEntry>, pub conflicts: Vec<GitIndexEntry>,
    pub staged_diff: String, pub worktree_diff: String, pub review_token: String,
    pub initial_staged_paths: Vec<String>,
    pub file_versions: BTreeMap<String, String>,
    pub baseline_version: String,
}
/// Original project Git status without mutation authority or file-content requirements.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectGitOverview {
    pub kind: String, pub path: String, pub branch: Option<String>, pub head: Option<String>,
    pub files: Vec<GitFileStatus>, pub conflicts: Vec<GitIndexEntry>,
    pub staged_diff: String, pub worktree_diff: String,
}
struct GitStatus {
    overview: ProjectGitOverview,
    index: Vec<GitIndexEntry>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectGitHistoryInput {
    pub project_id: String, pub start_oid: Option<String>, pub cursor: Option<String>,
    #[serde(default = "history_default_limit")] pub limit: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscardFile { pub path: String, pub expected_hash: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GitAction {
    Stage { paths: Vec<String> },
    Unstage { paths: Vec<String> },
    Untrack { paths: Vec<String> },
    Restore { paths: Vec<String>, confirmed: bool },
    RestoreHunk { path: String, diff_hash: String, hunk_index: usize, confirmed: bool },
    Commit { message: String, confirm_full_index: bool, confirm_preexisting_staged: bool },
    Push { remote: String, trust_directory: bool, trust_network: bool },
    Switch { branch: String },
    Merge { branch: String },
    Discard { baseline_version: String, files: Vec<DiscardFile>, confirmed: bool },
    Pull { remote: String, branch: String, preview_token: String, trust_directory: bool, trust_network: bool },
}
impl GitAction {
    fn operation_kind(&self) -> &'static str { match self { Self::Stage { .. } => "git_stage", Self::Unstage { .. } => "git_unstage", Self::Untrack { .. } => "git_untrack", Self::Restore { .. } => "git_restore", Self::RestoreHunk { .. } => "git_restore_hunk", Self::Commit { .. } => "git_commit", Self::Push { .. } => "git_push", Self::Pull { .. } => "git_pull", Self::Switch { .. } => "git_switch", Self::Merge { .. } => "git_merge", Self::Discard { .. } => "git_discard" } }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteGitInput { pub id: String, pub task_id: String, pub review_token: String, pub action: GitAction }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileGitInput { pub id: String, pub trust_directory: bool, pub trust_network: bool }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitHistoryInput { pub task_id: String, pub start_oid: Option<String>, pub cursor: Option<String>, #[serde(default = "history_default_limit")] pub limit: usize }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitHistoryEntry { pub oid: String, pub author: String, #[serde(rename = "timestamp")] pub authored_at: i64, pub subject: String, pub parents: Vec<String> }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitHistoryPage { #[serde(rename = "anchor")] pub start_oid: String, pub next_cursor: Option<String>, #[serde(rename = "commits")] pub entries: Vec<GitHistoryEntry> }
fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitPullPreview { pub review_token: String, pub remote: String, pub branch: String, pub remote_ref: String, pub remote_display: String, pub preview_token: String }
fn history_default_limit() -> usize { 30 }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitPullPreviewInput { pub task_id: String, pub remote: String, pub branch: String }
fn text(bytes: Vec<u8>) -> Result<String, String> { String::from_utf8(bytes).map_err(|_| "git_non_utf8_path".into()) }
fn check_lease(lease: &CancellationToken) -> Result<(), String> { if lease.is_cancelled() { Err("session_invalid".into()) } else { Ok(()) } }
fn valid_oid(value: &str) -> bool { matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit()) }
fn valid_branch(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 1024 || value.starts_with(['-', '/']) || value.ends_with(['.', '/']) || value.contains("..") || value.contains("@{") || value.contains(['\\', '~', '^', ':', '?', '*', '[']) || value.bytes().any(|b| b <= 32 || b == 127) || value.split('/').any(|p| p.is_empty() || p.starts_with('.') || p.ends_with(".lock")) || value == "@" { return Err("invalid_git_ref".into()); }
    Ok(())
}
fn valid_remote(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 || !value.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) || value.starts_with(['-', '.']) { return Err("invalid_git_remote".into()); }
    Ok(())
}
fn valid_url(value: &str) -> Result<(), String> {
    if value.is_empty() || value.starts_with('-') || value.bytes().any(|b| b <= 32 || b == 127) || value.contains(['\\', '?', '#']) { return Err("unsafe_git_remote".into()); }
    if let Some((scheme, remainder)) = value.split_once("://") {
        if !matches!(scheme, "https" | "http" | "ssh" | "file") || remainder.is_empty() { return Err("unsafe_git_remote".into()); }
        if matches!(scheme, "http" | "https") && remainder.split('/').next().is_some_and(|host| host.contains('@')) { return Err("unsafe_git_remote".into()); }
    } else if !Path::new(value).is_absolute() && !(value.contains(':') && !value.contains("::") && !value.split(':').next().unwrap_or_default().starts_with('-')) { return Err("unsafe_git_remote".into()); }
    Ok(())
}

/// Repository settings are data, never permission to execute project programs.
/// Every filter is disabled; a required filter makes the operation unavailable.
pub(crate) async fn safe_command(path: &Path) -> Result<Command, String> {
    let mut config = hardened_command(path);
    config.args(["config", "--null", "--list"]);
    let bytes = complete(git_output(config, Duration::from_secs(10), &CancellationToken::new(), &CancellationToken::new()).await?)?;
    let config = text(bytes)?;
    let mut command = hardened_command(path);
    for item in config.split('\0').filter(|s| !s.is_empty()) {
        let (key, value) = item.split_once('\n').unwrap_or((item, ""));
        if key.contains(['\0', '\n', '\r', '=']) { return Err("unsafe_git_config".into()); }
        let lower = key.to_ascii_lowercase();
        if lower.starts_with("filter.") {
            if lower.ends_with(".required") && !matches!(value.to_ascii_lowercase().as_str(), "false" | "no" | "off" | "0") { return Err("git_required_filter".into()); }
            if [".clean", ".smudge", ".process", ".required"].iter().any(|suffix| lower.ends_with(suffix)) { command.arg("-c").arg(format!("{key}={}", if lower.ends_with(".required") { "false" } else { "" })); }
        }
        if lower.starts_with("url.") && (lower.ends_with(".insteadof") || lower.ends_with(".pushinsteadof")) { return Err("git_url_rewrite_unsupported".into()); }
        if lower.starts_with("merge.") && lower.ends_with(".driver") { command.arg("-c").arg(format!("{key}=false")); }
        if lower.starts_with("branch.") && lower.ends_with(".mergeoptions") { command.arg("-c").arg(format!("{key}=")); }
    }
    Ok(command)
}
struct PullPreviewBinding { owner: String, task: String, review: String, remote: String, branch: String, url_hash: String, expires: std::time::Instant, lease: CancellationToken }
#[cfg(test)]
type PullFetchBarrier = (tokio::sync::oneshot::Sender<()>, tokio::sync::oneshot::Receiver<()>);
#[cfg(test)]
static PULL_FETCH_BARRIERS: std::sync::LazyLock<std::sync::Mutex<BTreeMap<String, PullFetchBarrier>>> = std::sync::LazyLock::new(Default::default);
#[cfg(test)]
static PULL_RESERVED_BARRIERS: std::sync::LazyLock<std::sync::Mutex<BTreeMap<String, PullFetchBarrier>>> = std::sync::LazyLock::new(Default::default);
#[cfg(test)]
static PULL_INSTALL_BARRIERS: std::sync::LazyLock<std::sync::Mutex<BTreeMap<String, PullFetchBarrier>>> = std::sync::LazyLock::new(Default::default);
#[cfg(test)]
async fn pull_reserved_barrier(id: &str) {
    let barrier = PULL_RESERVED_BARRIERS.lock().unwrap().remove(id);
    if let Some((reached, release)) = barrier { let _ = reached.send(()); let _ = release.await; }
}
#[cfg(test)]
async fn pull_install_barrier(id: &str) {
    let barrier = PULL_INSTALL_BARRIERS.lock().unwrap().remove(id);
    if let Some((reached, release)) = barrier { let _ = reached.send(()); let _ = release.await; }
}
#[cfg(test)]
async fn pull_fetch_barrier(id: &str) {
    let barrier = PULL_FETCH_BARRIERS.lock().unwrap().remove(id);
    if let Some((reached, release)) = barrier { let _ = reached.send(()); let _ = release.await; }
}
static PULL_PREVIEWS: std::sync::LazyLock<std::sync::Mutex<BTreeMap<String, PullPreviewBinding>>> = std::sync::LazyLock::new(Default::default);
fn history_binding(owner: &str, task: &str, path: &str, identity: &str, common: &(String, String), anchor: &str, offset: usize) -> Result<String, String> {
    Ok(digest(&serde_json::to_vec(&json!([owner, task, path, identity, common, anchor, offset])).map_err(|_| "storage_failed")?))
}
fn redact_remote(value: &str) -> String {
    let value = value.split(['?', '#']).next().unwrap_or_default();
    if let Some((scheme, rest)) = value.split_once("://") {
        let (authority, _) = rest.split_once('/').map_or((rest, ""), |(host, path)| (host, path));
        format!("{scheme}://{}/…", authority.rsplit('@').next().unwrap_or(authority))
    } else { "[local or SSH remote]".into() }
}
fn hardened_command(path: &Path) -> Command {
    let mut command = git_command(path);
    for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_")) { command.env_remove(key); }
    command.args(["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", "-c", "core.untrackedCache=false", "-c", "core.pager=cat", "-c", "core.editor=false", "-c", "core.askPass=", "-c", "core.attributesFile=/dev/null", "-c", "core.sshCommand=ssh -F /dev/null -oBatchMode=yes", "-c", "credential.helper=", "-c", "credential.interactive=false", "-c", "commit.gpgSign=false", "-c", "tag.gpgSign=false", "-c", "push.gpgSign=false", "-c", "push.recurseSubmodules=no", "-c", "submodule.recurse=false", "-c", "diff.external=", "-c", "diff.renames=false", "-c", "merge.renames=false", "-c", "gc.auto=0", "-c", "maintenance.auto=false"])
        .args(["-c", "merge.verifySignatures=false", "-c", "commit.status=false", "-c", "core.quotePath=true", "-c", "merge.default=text", "-c", "rerere.enabled=false", "-c", "rerere.autoUpdate=false"])
        .args(["-c", "protocol.allow=never", "-c", "protocol.file.allow=always", "-c", "protocol.http.allow=always", "-c", "protocol.https.allow=always", "-c", "protocol.ssh.allow=always", "-c", "merge.autoStash=false", "-c", "fetch.recurseSubmodules=false"])
        .env("GIT_OPTIONAL_LOCKS", "0").env("GIT_TERMINAL_PROMPT", "0").env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_ATTR_NOSYSTEM", "1").env("GIT_LITERAL_PATHSPECS", "1").env("GIT_MERGE_AUTOEDIT", "no").env_remove("SSH_ASKPASS").env_remove("GIT_ASKPASS")
        .env("GIT_NO_REPLACE_OBJECTS", "1").env("GIT_GRAFT_FILE", "/dev/null");
    command
}
async fn output(path: &Path, args: &[&str], lease: &CancellationToken, stop: &CancellationToken, write: bool) -> Result<Vec<u8>, String> {
    let mut command = safe_command(path).await?;
    command.args(args);
    complete(git_output(command, Duration::from_secs(if write { 120 } else { 10 }), lease, stop).await?)
}
fn complete((bytes, truncated): (Vec<u8>, bool)) -> Result<Vec<u8>, String> {
    if truncated { Err("git_output_limit".into()) } else { Ok(bytes) }
}
async fn optional_ref(path: &Path, args: &[&str], lease: &CancellationToken, stop: &CancellationToken) -> Result<Option<String>, String> {
    match output(path, args, lease, stop, false).await { Ok(bytes) => Ok(Some(text(bytes)?.trim().into())), Err(error) if error == "git_failed" => Ok(None), Err(error) => Err(error) }
}
pub(crate) async fn git_common(path: &str) -> Result<(String, String), String> {
    let bytes = output(Path::new(path), &["rev-parse", "--path-format=absolute", "--git-common-dir"], &CancellationToken::new(), &CancellationToken::new(), false).await?;
    crate::projects::directory(text(bytes)?.trim()).await
}
fn parse_index(bytes: &[u8]) -> Result<Vec<GitIndexEntry>, String> {
    let mut result = Vec::new();
    for part in bytes.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let value = std::str::from_utf8(part).map_err(|_| "git_non_utf8_path")?;
        let (header, path) = value.split_once('\t').ok_or("git_invalid_output")?;
        let fields: Vec<_> = header.split(' ').collect();
        if fields.len() != 3 || !valid_oid(fields[1]) { return Err("git_invalid_output".into()); }
        result.push(GitIndexEntry { path: path.into(), mode: fields[0].into(), oid: fields[1].into(), stage: fields[2].parse().map_err(|_| "git_invalid_output")? });
    }
    Ok(result)
}
fn parse_status(bytes: &[u8]) -> Result<Vec<GitFileStatus>, String> {
    let mut result = Vec::new();
    let mut parts = bytes.split(|b| *b == 0).filter(|p| !p.is_empty());
    while let Some(part) = parts.next() {
        if part.len() < 4 || part[2] != b' ' { return Err("git_invalid_output".into()); }
        let original_path = if matches!(part[0], b'R' | b'C') || matches!(part[1], b'R' | b'C') { Some(text(parts.next().ok_or("git_invalid_output")?.to_vec())?) } else { None };
        result.push(GitFileStatus { path: text(part[3..].to_vec())?, original_path, index: (part[0] as char).to_string(), worktree: (part[1] as char).to_string() });
    }
    Ok(result)
}
async fn head_tree(path: &Path, head: &str, lease: &CancellationToken, stop: &CancellationToken) -> Result<Vec<GitIndexEntry>, String> {
    if !valid_oid(head) { return Err("invalid_git_ref".into()); }
    let bytes = output(path, &["ls-tree", "-r", "-z", head], lease, stop, false).await?;
    let mut entries = Vec::new();
    for part in bytes.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let value = std::str::from_utf8(part).map_err(|_| "git_non_utf8_path")?;
        let (header, path) = value.split_once('\t').ok_or("git_invalid_output")?;
        let fields: Vec<_> = header.split(' ').collect();
        if fields.len() != 3 || !valid_oid(fields[2]) { return Err("git_invalid_output".into()); }
        entries.push(GitIndexEntry { path: path.into(), mode: fields[0].into(), oid: fields[2].into(), stage: 0 });
    }
    Ok(entries)
}
fn sorted(mut entries: Vec<GitIndexEntry>) -> Vec<GitIndexEntry> { entries.sort_by(|a, b| (&a.path, a.stage).cmp(&(&b.path, b.stage))); entries }
fn validate_paths(project: &Project, paths: &[String], index: &[GitIndexEntry]) -> Result<(), String> {
    if paths.is_empty() || paths.len() > 1000 || paths.iter().collect::<BTreeSet<_>>().len() != paths.len() { return Err("invalid_git_paths".into()); }
    let root = files::open_root(project)?;
    for path in paths {
        files::relative(path, false)?;
        if files::sensitive(path) { return Err("file_sensitive".into()); }
        if path.contains(['\r', '\n']) { return Err("invalid_git_paths".into()); }
        let mut current = PathBuf::new();
        for part in Path::new(path).components() {
            current.push(part);
            match root.symlink_metadata(&current) {
                Ok(m) if m.file_type().is_symlink() || (!m.is_dir() && current != Path::new(path)) || (current == Path::new(path) && !m.is_file()) => return Err("file_access_denied".into()),
                Ok(_) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && index.iter().any(|e| e.path == *path && matches!(e.mode.as_str(), "100644" | "100755")) => {},
                Err(_) => return Err("file_access_denied".into()),
            }
        }
        if index.iter().any(|entry| (entry.path == *path || path.starts_with(&format!("{}/", entry.path))) && !matches!(entry.mode.as_str(), "100644" | "100755")) { return Err("git_special_entry".into()); }
    }
    Ok(())
}

impl AgentManager {
    async fn git_status_at(&self, project: &Project, lease: &CancellationToken) -> Result<GitStatus, String> {
        let path = Path::new(&project.path);
        // Distinguish an ordinary directory from a damaged/inaccessible repository.
        let mut probe = git_command(path); probe.args(["rev-parse", "--show-toplevel"]);
        match git_output(probe, Duration::from_secs(10), lease, &self.inner.stop).await {
            Ok(result) => {
                let bytes = complete(result)?;
                let actual = tokio::fs::canonicalize(text(bytes)?.trim()).await.map_err(|_| "workspace_changed")?;
                if actual != path { return Err("git_workspace_not_root".into()); }
            },
            Err(error) => {
                let has_git = path.ancestors().any(|p| p.join(".git").symlink_metadata().is_ok());
                if has_git || error != "git_failed" { return Err(error); }
                crate::projects::validate_directory(project).await?;
                return Ok(GitStatus { overview: ProjectGitOverview { kind: "directory".into(), path: project.path.clone(), branch: None, head: None, files: vec![], conflicts: vec![], staged_diff: String::new(), worktree_diff: String::new() }, index: vec![] });
            },
        }
        let branch = optional_ref(path, &["symbolic-ref", "--quiet", "--short", "HEAD"], lease, &self.inner.stop).await?;
        let head = optional_ref(path, &["rev-parse", "--verify", "HEAD"], lease, &self.inner.stop).await?;
        if head.as_deref().is_some_and(|h| !valid_oid(h)) { return Err("git_invalid_output".into()); }
        let index = parse_index(&output(path, &["ls-files", "--stage", "-z"], lease, &self.inner.stop, false).await?)?;
        let files = parse_status(&output(path, &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--ignore-submodules=none"], lease, &self.inner.stop, false).await?)?;
        let mut display_paths = BTreeSet::new();
        for file in &files {
            for selected in std::iter::once(&file.path).chain(file.original_path.iter()) {
                if !files::sensitive(selected) && !index.iter().any(|e| e.path == *selected && !matches!(e.mode.as_str(), "100644" | "100755")) { display_paths.insert(selected.as_str()); }
            }
        }
        let mut diffs = Vec::new();
        for cached in [true, false] {
            if display_paths.is_empty() { diffs.push(String::new()); continue; }
            let mut args = vec!["diff", "--no-ext-diff", "--no-textconv", "--no-renames", "--binary"];
            if cached { args.push("--cached"); }
            args.push("--"); args.extend(display_paths.iter().copied());
            diffs.push(crate::executions::sanitize_text(&text(output(path, &args, lease, &self.inner.stop, false).await?)?));
        }
        let worktree_diff = diffs.pop().ok_or("git_invalid_output")?;
        let staged_diff = diffs.pop().ok_or("git_invalid_output")?;
        let conflicts = index.iter().filter(|e| e.stage != 0).cloned().collect();
        crate::projects::validate_directory(project).await?;
        Ok(GitStatus { overview: ProjectGitOverview { kind: "git".into(), path: project.path.clone(), branch, head, files, conflicts, staged_diff, worktree_diff }, index })
    }
    async fn git_snapshot(&self, owner: &str, task_id: &str, lease: &CancellationToken) -> Result<GitReview, String> {
        let project = self.task_project(owner, task_id).await?;
        let baseline_version = self.inner.repository.review_baseline(owner, task_id).await?.and_then(|v| v["fingerprint"].as_str().map(str::to_owned)).unwrap_or_default();
        let GitStatus { overview, index } = self.git_status_at(&project, lease).await?;
        let mut review = GitReview { task_id: task_id.into(), kind: overview.kind, path: overview.path, common_dir: None, common_identity: None, branch: overview.branch, head: overview.head, merge_heads: vec![], files: overview.files, index, conflicts: overview.conflicts, staged_diff: overview.staged_diff, worktree_diff: overview.worktree_diff, review_token: String::new(), initial_staged_paths: vec![], file_versions: BTreeMap::new(), baseline_version };
        if review.kind == "directory" {
            review.review_token = self.task_fingerprint(owner, task_id).await?;
            return Ok(review);
        }
        let path = Path::new(&project.path);
        let (common_dir, common_identity) = git_common(&project.path).await?;
        review.common_dir = Some(common_dir); review.common_identity = Some(common_identity);
        review.merge_heads = optional_ref(path, &["rev-parse", "--verify", "MERGE_HEAD"], lease, &self.inner.stop).await?.into_iter().collect();
        let flags = output(path, &["ls-files", "-v", "-z"], lease, &self.inner.stop, false).await?;
        if flags.split(|b| *b == 0).filter(|v| !v.is_empty()).any(|v| v.first().is_some_and(|b| b.is_ascii_lowercase() || *b == b'S')) { return Err("git_index_hidden_files".into()); }
        review.initial_staged_paths = self.initial_git_index(owner, task_id).await?.and_then(|v| v.get("staged_paths").cloned()).map(serde_json::from_value).transpose().map_err(|_| "storage_failed")?.unwrap_or_else(|| review.files.iter().filter(|f| f.index != " " && f.index != "?").map(|f| f.path.clone()).collect());
        // Diff omits untracked contents. Include exact bounded file hashes.
        let root = files::open_root(&project)?;
        for entry in &review.files {
            files::relative(&entry.path, false)?;
            if files::sensitive(&entry.path) { return Err("git_review_incomplete".into()); }
            let version = match files::read_at(&root, &entry.path) {
                Ok(view) => { if view.hash.is_none() || view.reason.as_deref().is_some_and(|r| r != "file_binary") { return Err("git_review_incomplete".into()); } view.hash.unwrap() },
                Err(error) if error == "file_not_found" => "deleted".into(),
                Err(_) => return Err("git_review_incomplete".into()),
            };
            let mut version = version;
            #[cfg(unix)] if let Ok(metadata) = root.symlink_metadata(&entry.path) {
                use cap_std::fs::MetadataExt;
                version.push_str(&format!(":{:o}", metadata.mode()));
            }
            review.file_versions.insert(entry.path.clone(), version);
        }
        let mut version = review.clone(); version.initial_staged_paths.clear(); version.baseline_version.clear();
        review.review_token = digest(&serde_json::to_vec(&version).map_err(|_| "storage_failed")?);
        crate::projects::validate_directory(&project).await?;
        Ok(review)
    }
    pub async fn read_development_git_history(&self, owner: &str, input: GitHistoryInput) -> Result<GitHistoryPage, String> {
        let _admission = self.inner.admission.lock().await;
        self.ready().await?;
        if input.limit == 0 || input.limit > 100 { return Err("invalid_git_history_limit".into()); }
        let project = self.task_project(owner, &input.task_id).await?;
        let scope = input.task_id.clone();
        self.git_history_at(owner, &scope, project, input.start_oid, input.cursor, input.limit).await
    }
    pub async fn read_development_project_git(&self, owner: &str, project_id: &str) -> Result<ProjectGitOverview, String> {
        let _admission = self.inner.admission.lock().await;
        let project = self.file_project(owner, project_id).await?;
        Ok(self.git_status_at(&project, &CancellationToken::new()).await?.overview)
    }
    pub async fn read_development_project_git_history(&self, owner: &str, input: ProjectGitHistoryInput) -> Result<GitHistoryPage, String> {
        let _admission = self.inner.admission.lock().await;
        self.ready().await?;
        if input.limit == 0 || input.limit > 100 { return Err("invalid_git_history_limit".into()); }
        let project = self.file_project(owner, &input.project_id).await?;
        let scope = format!("project:{}", input.project_id);
        self.git_history_at(owner, &scope, project, input.start_oid, input.cursor, input.limit).await
    }
    async fn git_history_at(&self, owner: &str, scope: &str, project: Project, start_oid: Option<String>, cursor: Option<String>, limit: usize) -> Result<GitHistoryPage, String> {
        let path = Path::new(&project.path);
        let root = text(output(path, &["rev-parse", "--show-toplevel"], &CancellationToken::new(), &self.inner.stop, false).await?)?;
        if tokio::fs::canonicalize(root.trim_end_matches('\n')).await.map_err(|_| "workspace_changed")? != path { return Err("git_workspace_not_root".into()); }
        let (_, identity) = crate::projects::directory(&project.path).await?;
        let common = git_common(&project.path).await?;
        let current = optional_ref(path, &["rev-parse", "--verify", "HEAD"], &CancellationToken::new(), &self.inner.stop).await?;
        let cursor_anchor = cursor.as_deref().and_then(|cursor| cursor.split(':').next()).map(str::to_owned);
        let Some(start_oid) = start_oid.or(cursor_anchor).or(current) else { return Ok(GitHistoryPage { start_oid: String::new(), next_cursor: None, entries: vec![] }); };
        if !valid_oid(&start_oid) { return Err("invalid_git_ref".into()); }
        let offset = if let Some(cursor) = cursor.as_deref() {
            let fields: Vec<_> = cursor.split(':').collect();
            if fields.len() != 3 || fields[0] != start_oid { return Err("invalid_git_history_cursor".into()); }
            let offset: usize = fields[1].parse().map_err(|_| "invalid_git_history_cursor")?;
            if offset == 0 || offset >= 10_000 { return Err("git_history_limit".into()); }
            if fields[2] != history_binding(owner, scope, &project.path, &identity, &common, &start_oid, offset)? { return Err("invalid_git_history_cursor".into()); }
            offset
        } else { 0 };
        if output(path, &["cat-file", "-e", &format!("{start_oid}^{{commit}}")], &CancellationToken::new(), &self.inner.stop, false).await.is_err() { return Err("invalid_git_ref".into()); }
        let format = "--format=%H%x00%an%x00%at%x00%s%x00%P";
        let max = (limit + 1).to_string();
        let skip = offset.to_string();
        let bytes = output(path, &["log", "-z", format, "--topo-order", "--no-decorate", "--no-show-signature", "--no-notes", "--max-count", &max, "--skip", &skip, &start_oid, "--"], &CancellationToken::new(), &self.inner.stop, false).await?;
        let raw = text(bytes)?;
        let fields: Vec<_> = raw.split_terminator('\0').collect();
        if fields.len() % 5 != 0 { return Err("git_invalid_output".into()); }
        let mut entries = Vec::new();
        for fields in fields.chunks_exact(5) {
            let oid = fields[0];
            let parents = fields[4].split_whitespace().map(str::to_owned).collect::<Vec<_>>();
            if !valid_oid(oid) || parents.iter().any(|parent| !valid_oid(parent)) { return Err("git_invalid_output".into()); }
            entries.push(GitHistoryEntry { oid: oid.into(), author: fields[1].into(), authored_at: fields[2].parse().map_err(|_| "git_invalid_output")?, subject: fields[3].chars().take(512).collect(), parents });
        }
        let has_more = entries.len() > limit;
        entries.truncate(limit);
        let next_cursor = if has_more { let next = offset + entries.len(); if next >= 10_000 { return Err("git_history_limit".into()); } Some(format!("{start_oid}:{next}:{}", history_binding(owner, scope, &project.path, &identity, &common, &start_oid, next)?)) } else { None };
        crate::projects::validate_directory(&project).await?;
        Ok(GitHistoryPage { start_oid, next_cursor, entries })
    }
    async fn initial_git_index(&self, owner: &str, task_id: &str) -> Result<Option<Value>, String> {
        for op in self.inner.repository.list_git_operations(owner, task_id).await? {
            if let Some(value) = op.get("details").and_then(|d| d.get("initial_index")) { return Ok(Some(value.clone())); }
        }
        Ok(None)
    }
    pub async fn preview_development_git_pull(&self, owner: &str, task_id: &str, remote: &str, branch: &str, lease: CancellationToken) -> Result<GitPullPreview, String> {
        let review = self.read_development_git(owner, task_id).await?;
        if review.kind != "git" { return Err("not_git_repository".into()); }
        valid_remote(remote)?; valid_branch(branch)?;
        let path = Path::new(&review.path);
        let urls = text(output(path, &["remote", "get-url", "--all", remote], &CancellationToken::new(), &self.inner.stop, false).await?)?;
        let urls: Vec<_> = urls.lines().collect();
        if urls.len() != 1 { return Err("unsafe_git_remote".into()); }
        valid_url(urls[0])?;
        let remote_ref = format!("refs/heads/{branch}");
        check_lease(&lease)?;
        let preview_token = uuid::Uuid::new_v4().to_string();
        let mut previews = PULL_PREVIEWS.lock().map_err(|_| "storage_failed")?;
        previews.retain(|_, value| value.expires > std::time::Instant::now() && !value.lease.is_cancelled());
        if previews.len() >= 1024 { return Err("workspace_busy".into()); }
        previews.insert(preview_token.clone(), PullPreviewBinding { owner: owner.into(), task: task_id.into(), review: review.review_token.clone(), remote: remote.into(), branch: branch.into(), url_hash: digest(urls[0].as_bytes()), expires: std::time::Instant::now() + Duration::from_secs(600), lease });
        Ok(GitPullPreview { review_token: review.review_token, remote: remote.into(), branch: branch.into(), remote_ref, remote_display: redact_remote(urls[0]), preview_token })
    }
    pub async fn read_development_git(&self, owner: &str, task_id: &str) -> Result<GitReview, String> {
        let _admission = self.inner.admission.lock().await;
        self.ready().await?;
        self.git_snapshot(owner, task_id, &CancellationToken::new()).await
    }
    pub async fn execute_development_git(&self, owner: &str, input: ExecuteGitInput, lease: CancellationToken) -> Result<Value, String> {
        let _admission = self.inner.admission.lock().await;
        self.ready().await?; check_lease(&lease)?;
        uuid::Uuid::parse_str(&input.id).map_err(|_| "invalid_request_id")?;
        let request_fingerprint = digest(&serde_json::to_vec(&input).map_err(|_| "invalid_request")?);
        if let Some(receipt) = self.inner.repository.git_operation(owner, &input.id).await? {
            if receipt.get("request_fingerprint").and_then(Value::as_str) != Some(request_fingerprint.as_str()) { return Err("request_conflict".into()); }
            return Ok(receipt);
        }
        let project = self.task_project(owner, &input.task_id).await?;
        if project.archived { return Err("project_archived".into()); }
        let before = self.git_snapshot(owner, &input.task_id, &lease).await?;
        if before.review_token != input.review_token { return Err("git_review_stale".into()); }
        self.workspace_available(&project.path, None).await?;
        self.host_workspace_available(&project.path).await?;
        if let Some(common) = &before.common_dir { self.repository_git_available(&project.path, common).await?; }
        if before.kind != "git" && !matches!(input.action, GitAction::Discard { .. }) { return Err("not_git_repository".into()); }
        let staged_paths: Vec<_> = before.files.iter().filter(|f| f.index != " " && f.index != "?").map(|f| f.path.clone()).collect();
        let initial_index = self.initial_git_index(owner, &input.task_id).await?.unwrap_or_else(|| json!({"entries":before.index,"staged_paths":staged_paths}));
        let mut details = json!({"request":input,"before":before,"initial_index":initial_index});
        self.prepare_git_action(owner, &project, &input, &before, &mut details, &lease).await?;
        if matches!(input.action, GitAction::Pull { .. }) { details["phase"] = json!("prepared"); }
        self.capture_task_baseline(owner, &input.task_id).await?;
        let workspace = self.inner.repository.task_workspace(owner, &input.task_id).await?;
        check_lease(&lease)?;
        self.inner.repository.start_git_operation(owner, &GitOperationIntent {
            id: &input.id,
            project_id: &project.id,
            task_id: &input.task_id,
            workspace_id: workspace.as_ref().map(|w| w.id.as_str()),
            kind: input.action.operation_kind(),
            fingerprint: &request_fingerprint,
            details: &details,
        }).await?;
        // Recheck after durable intent, immediately before any external mutation.
        let checked = self.git_snapshot(owner, &input.task_id, &lease).await;
        // Persisting the first snapshot changes only the displayed initial-stage
        // annotation, never the reviewed Git version.
        let checked = checked.and_then(|v| if same_version(&before, &v) { Ok(v) } else { Err("git_review_stale".into()) });
        let result = match checked {
            Err(error) => Err(error),
            Ok(_) => { details["action_started"] = json!(true); self.run_git_action(owner, &project, &input, &mut details, &lease).await },
        };
        let mut status = "succeeded";
        if let Err(error) = result {
            status = if matches!(error.as_str(), "cancelled" | "session_invalid" | "git_timeout" | "git_output_limit" | "file_save_unknown" | "git_unknown") || (error == "git_failed" && details["action_started"].as_bool() == Some(true)) { "unknown" } else { "failed" };
            if matches!(input.action, GitAction::Pull { .. }) && matches!(details["phase"].as_str(), Some("fetch_started" | "fetched" | "ff_started" | "completed")) && error != "git_pull_diverged" { status = "unknown"; }
            details["error"] = json!(error);
        }
        if let Ok(after) = self.git_snapshot(owner, &input.task_id, &CancellationToken::new()).await {
            if status == "unknown" && details["error"] == "git_failed" && matches!(input.action, GitAction::Merge { .. }) && !after.conflicts.is_empty() { status = "failed"; }
            details["after"] = serde_json::to_value(after).map_err(|_| "storage_failed")?;
        }
        self.inner.repository.finish_git_operation(owner, &input.id, status, &details).await?;
        if matches!(input.action, GitAction::Pull { .. }) && status != "unknown" {
            operations::cleanup_pull_ref(Path::new(&project.path), &input.id, &details, &self.inner.stop).await;
        }
        self.inner.repository.git_operation(owner, &input.id).await?.ok_or_else(|| "storage_failed".into())
    }
}
fn same_version(a: &GitReview, b: &GitReview) -> bool {
    if a.kind == "directory" || b.kind == "directory" { return a.kind == b.kind && a.path == b.path && a.review_token == b.review_token; }
    let mut a = a.clone(); let mut b = b.clone();
    a.initial_staged_paths.clear(); b.initial_staged_paths.clear(); a.review_token.clear(); b.review_token.clear(); a.baseline_version.clear(); b.baseline_version.clear();
    serde_json::to_vec(&a).ok() == serde_json::to_vec(&b).ok()
}
