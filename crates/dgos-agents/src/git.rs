//! Task-scoped Git delivery. UUID receipts describe external actions; replay never
//! executes Git again. The admission lock coordinates only DGOS, not external Git.
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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscardFile { pub path: String, pub expected_hash: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GitAction {
    Stage { paths: Vec<String> },
    Unstage { paths: Vec<String> },
    Commit { message: String, confirm_full_index: bool, confirm_preexisting_staged: bool },
    Push { remote: String, trust_directory: bool, trust_network: bool },
    Switch { branch: String },
    Merge { branch: String },
    Discard { baseline_version: String, files: Vec<DiscardFile>, confirmed: bool },
}
impl GitAction {
    fn operation_kind(&self) -> &'static str { match self { Self::Stage { .. } => "git_stage", Self::Unstage { .. } => "git_unstage", Self::Commit { .. } => "git_commit", Self::Push { .. } => "git_push", Self::Switch { .. } => "git_switch", Self::Merge { .. } => "git_merge", Self::Discard { .. } => "git_discard" } }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteGitInput { pub id: String, pub task_id: String, pub review_token: String, pub action: GitAction }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileGitInput { pub id: String, pub trust_directory: bool, pub trust_network: bool }

fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
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
    if value.is_empty() || value.starts_with('-') || value.bytes().any(|b| b <= 32 || b == 127) || value.contains('\\') { return Err("unsafe_git_remote".into()); }
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
fn hardened_command(path: &Path) -> Command {
    let mut command = git_command(path);
    for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_")) { command.env_remove(key); }
    command.args(["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", "-c", "core.untrackedCache=false", "-c", "core.pager=cat", "-c", "core.editor=false", "-c", "core.askPass=", "-c", "core.attributesFile=/dev/null", "-c", "core.sshCommand=ssh -F /dev/null -oBatchMode=yes", "-c", "credential.helper=", "-c", "credential.interactive=false", "-c", "commit.gpgSign=false", "-c", "tag.gpgSign=false", "-c", "push.gpgSign=false", "-c", "push.recurseSubmodules=no", "-c", "submodule.recurse=false", "-c", "diff.external=", "-c", "diff.renames=false", "-c", "merge.renames=false", "-c", "gc.auto=0", "-c", "maintenance.auto=false"])
        .args(["-c", "merge.verifySignatures=false", "-c", "commit.status=false", "-c", "core.quotePath=true", "-c", "merge.default=text", "-c", "rerere.enabled=false", "-c", "rerere.autoUpdate=false"])
        .env("GIT_OPTIONAL_LOCKS", "0").env("GIT_TERMINAL_PROMPT", "0").env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_ATTR_NOSYSTEM", "1").env("GIT_LITERAL_PATHSPECS", "1").env("GIT_MERGE_AUTOEDIT", "no").env_remove("SSH_ASKPASS").env_remove("GIT_ASKPASS");
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
        if path.contains(['\r', '\n']) { return Err("invalid_git_paths".into()); }
        let mut current = PathBuf::new();
        for part in Path::new(path).components() {
            current.push(part);
            match root.symlink_metadata(&current) {
                Ok(m) if m.file_type().is_symlink() || (!m.is_dir() && current != Path::new(path)) || (current == Path::new(path) && !m.is_file()) => return Err("file_access_denied".into()),
                Ok(_) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && current == Path::new(path) && index.iter().any(|e| e.path == *path && matches!(e.mode.as_str(), "100644" | "100755")) => {},
                Err(_) => return Err("file_access_denied".into()),
            }
        }
        if index.iter().any(|entry| (entry.path == *path || path.starts_with(&format!("{}/", entry.path))) && !matches!(entry.mode.as_str(), "100644" | "100755")) { return Err("git_special_entry".into()); }
    }
    Ok(())
}

impl AgentManager {
    async fn git_snapshot(&self, owner: &str, task_id: &str, lease: &CancellationToken) -> Result<GitReview, String> {
        let project = self.task_project(owner, task_id).await?;
        let baseline_version = self.inner.repository.review_baseline(owner, task_id).await?.and_then(|v| v["fingerprint"].as_str().map(str::to_owned)).unwrap_or_default();
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
                return Ok(GitReview { task_id: task_id.into(), kind: "directory".into(), path: project.path, common_dir: None, common_identity: None, branch: None, head: None, merge_heads: vec![], files: vec![], index: vec![], conflicts: vec![], staged_diff: String::new(), worktree_diff: String::new(), review_token: self.task_fingerprint(owner, task_id).await?, initial_staged_paths: vec![], file_versions: BTreeMap::new(), baseline_version });
            },
        }
        let (common_dir, common_identity) = git_common(&project.path).await?;
        let branch = optional_ref(path, &["symbolic-ref", "--quiet", "--short", "HEAD"], lease, &self.inner.stop).await?;
        let head = optional_ref(path, &["rev-parse", "--verify", "HEAD"], lease, &self.inner.stop).await?;
        if head.as_deref().is_some_and(|h| !valid_oid(h)) { return Err("git_invalid_output".into()); }
        let merge_heads = optional_ref(path, &["rev-parse", "--verify", "MERGE_HEAD"], lease, &self.inner.stop).await?.into_iter().collect();
        let index = parse_index(&output(path, &["ls-files", "--stage", "-z"], lease, &self.inner.stop, false).await?)?;
        let flags = output(path, &["ls-files", "-v", "-z"], lease, &self.inner.stop, false).await?;
        if flags.split(|b| *b == 0).filter(|v| !v.is_empty()).any(|v| v.first().is_some_and(|b| b.is_ascii_lowercase() || *b == b'S')) { return Err("git_index_hidden_files".into()); }
        let files = parse_status(&output(path, &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--ignore-submodules=none"], lease, &self.inner.stop, false).await?)?;
        let staged_diff = text(output(path, &["diff", "--cached", "--no-ext-diff", "--no-textconv", "--no-renames", "--binary", "--"], lease, &self.inner.stop, false).await?)?;
        let worktree_diff = text(output(path, &["diff", "--no-ext-diff", "--no-textconv", "--no-renames", "--binary", "--"], lease, &self.inner.stop, false).await?)?;
        let conflicts = index.iter().filter(|e| e.stage != 0).cloned().collect();
        let initial_staged_paths = self.initial_git_index(owner, task_id).await?.and_then(|v| v.get("staged_paths").cloned()).map(serde_json::from_value).transpose().map_err(|_| "storage_failed")?.unwrap_or_else(|| files.iter().filter(|f| f.index != " " && f.index != "?").map(|f| f.path.clone()).collect());
        let mut review = GitReview { task_id: task_id.into(), kind: "git".into(), path: project.path.clone(), common_dir: Some(common_dir), common_identity: Some(common_identity), branch, head, merge_heads, files, index, conflicts, staged_diff, worktree_diff, review_token: String::new(), initial_staged_paths, file_versions: BTreeMap::new(), baseline_version };
        // Diff omits untracked contents. Include exact bounded file hashes.
        let root = files::open_root(&project)?;
        for entry in &review.files {
            files::relative(&entry.path, false)?;
            let version = match files::read_at(&root, &entry.path) {
                Ok(view) => { if view.reason.is_some() || view.hash.is_none() { return Err("git_review_incomplete".into()); } view.hash.unwrap() },
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
        Ok(review)
    }
    async fn initial_git_index(&self, owner: &str, task_id: &str) -> Result<Option<Value>, String> {
        for op in self.inner.repository.list_git_operations(owner, task_id).await? {
            if let Some(value) = op.get("details").and_then(|d| d.get("initial_index")) { return Ok(Some(value.clone())); }
        }
        Ok(None)
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
            details["error"] = json!(error);
        }
        if let Ok(after) = self.git_snapshot(owner, &input.task_id, &CancellationToken::new()).await {
            if status == "unknown" && details["error"] == "git_failed" && matches!(input.action, GitAction::Merge { .. }) && !after.conflicts.is_empty() { status = "failed"; }
            details["after"] = serde_json::to_value(after).map_err(|_| "storage_failed")?;
        }
        self.inner.repository.finish_git_operation(owner, &input.id, status, &details).await?;
        self.inner.repository.git_operation(owner, &input.id).await?.ok_or_else(|| "storage_failed".into())
    }
}
fn same_version(a: &GitReview, b: &GitReview) -> bool {
    if a.kind == "directory" || b.kind == "directory" { return a.kind == b.kind && a.path == b.path && a.review_token == b.review_token; }
    let mut a = a.clone(); let mut b = b.clone();
    a.initial_staged_paths.clear(); b.initial_staged_paths.clear(); a.review_token.clear(); b.review_token.clear(); a.baseline_version.clear(); b.baseline_version.clear();
    serde_json::to_vec(&a).ok() == serde_json::to_vec(&b).ok()
}
