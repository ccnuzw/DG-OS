//! Bounded, read-only metadata for either the selected task workspace or project.
use super::*;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitDetailsInput { pub project_id: String, pub task_id: Option<String>, pub path: Option<String> }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitFileStats { pub path: String, pub index_added: Option<u64>, pub index_deleted: Option<u64>, pub worktree_added: Option<u64>, pub worktree_deleted: Option<u64> }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitDetails { pub branches: Vec<String>, pub remotes: Vec<String>, pub remote_branches: BTreeMap<String, Vec<String>>, pub remote_push_targets: BTreeMap<String, Vec<String>>, pub remote_fetch_targets: BTreeMap<String, Vec<String>>, pub upstream: Option<String>, pub ahead: Option<u64>, pub behind: Option<u64>, pub files: Vec<GitFileStats>, pub history: Vec<GitHistoryEntry>, pub history_truncated: bool }

fn numstat(bytes: &[u8]) -> Result<BTreeMap<String, (Option<u64>, Option<u64>)>, String> {
    let mut result = BTreeMap::new();
    for record in bytes.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let record = std::str::from_utf8(record).map_err(|_| "git_non_utf8_path")?;
        let mut fields = record.splitn(3, '\t');
        let mut number = || -> Result<Option<u64>, String> { let v = fields.next().ok_or("git_invalid_output")?; if v == "-" { Ok(None) } else { Ok(Some(v.parse().map_err(|_| "git_invalid_output")?)) } };
        let added = number()?; let deleted = number()?;
        result.insert(fields.next().ok_or("git_invalid_output")?.into(), (added, deleted));
    }
    Ok(result)
}
impl AgentManager {
    pub async fn read_development_git_details(&self, owner: &str, input: GitDetailsInput, lease: CancellationToken) -> Result<GitDetails, String> {
        let _admission = self.inner.admission.lock().await;
        self.ready().await?; check_lease(&lease)?;
        let project = if let Some(task) = &input.task_id {
            let project = self.task_project(owner, task).await?;
            if project.id != input.project_id { return Err("task_not_found".into()); }
            project
        } else { self.file_project(owner, &input.project_id).await? };
        let status = self.git_status_at(&project, &lease).await?;
        let mut result = GitDetails { branches: vec![], remotes: vec![], remote_branches: BTreeMap::new(), remote_push_targets: BTreeMap::new(), remote_fetch_targets: BTreeMap::new(), upstream: None, ahead: None, behind: None, files: vec![], history: vec![], history_truncated: false };
        if let Some(selected) = &input.path {
            files::relative(selected, false)?;
            if files::sensitive(selected) { return Err("file_sensitive".into()); }
            let root = files::open_root(&project)?;
            let mut prefix = PathBuf::new();
            for part in Path::new(selected).components() {
                prefix.push(part);
                match root.symlink_metadata(&prefix) {
                    Ok(m) if m.file_type().is_symlink() => return Err("file_access_denied".into()),
                    Ok(_) => {},
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                    Err(_) => return Err("file_access_denied".into()),
                }
            }
        }
        if status.overview.kind != "git" { check_lease(&lease)?; return Ok(result); }
        let path = Path::new(&project.path); let stop = &self.inner.stop;
        result.branches = text(output(path, &["for-each-ref", "--format=%(refname:strip=2)", "refs/heads/"], &lease, stop, false).await?)?.lines().map(str::to_owned).collect();
        result.remotes = text(output(path, &["remote"], &lease, stop, false).await?)?.lines().map(str::to_owned).collect();
        for remote in &result.remotes {
            valid_remote(remote)?;
            let targets = text(output(path, &["remote", "get-url", "--push", "--all", remote], &lease, stop, false).await?)?;
            result.remote_push_targets.insert(remote.clone(), targets.lines().map(redact_remote).collect());
            let targets = text(output(path, &["remote", "get-url", "--all", remote], &lease, stop, false).await?)?;
            result.remote_fetch_targets.insert(remote.clone(), targets.lines().map(redact_remote).collect());
            let prefix = format!("refs/remotes/{remote}/");
            let refs = text(output(path, &["for-each-ref", "--format=%(refname)%09%(symref)", &prefix], &lease, stop, false).await?)?;
            result.remote_branches.insert(remote.clone(), refs.lines().filter_map(|line| { let (name, symref) = line.split_once('\t')?; if symref.is_empty() { name.strip_prefix(&prefix).map(str::to_owned) } else { None } }).collect());
        }
        result.upstream = optional_ref(path, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"], &lease, stop).await?;
        if result.upstream.is_some() && status.overview.head.is_some() {
            let counts = text(output(path, &["rev-list", "--left-right", "--count", "HEAD...@{upstream}", "--"], &lease, stop, false).await?)?;
            let counts: Vec<_> = counts.split_whitespace().collect();
            if counts.len() != 2 { return Err("git_invalid_output".into()); }
            result.ahead = Some(counts[0].parse().map_err(|_| "git_invalid_output")?); result.behind = Some(counts[1].parse().map_err(|_| "git_invalid_output")?);
        }
        let staged = numstat(&output(path, &["diff", "--cached", "--numstat", "-z", "--no-ext-diff", "--no-textconv", "--no-renames", "--"], &lease, stop, false).await?)?;
        let worktree = numstat(&output(path, &["diff", "--numstat", "-z", "--no-ext-diff", "--no-textconv", "--no-renames", "--"], &lease, stop, false).await?)?;
        let root = files::open_root(&project)?;
        let mut authorized = status.index.clone();
        if status.overview.files.iter().any(|file| file.index == "D") {
            if let Some(head) = &status.overview.head {
                authorized.extend(head_tree(path, head, &lease, stop).await?.into_iter().filter(|entry| status.overview.files.iter().any(|file| file.path == entry.path && file.index == "D")));
            }
        }
        for file in &status.overview.files {
            // Never load protected paths or follow repository-controlled links.
            validate_paths(&project, std::slice::from_ref(&file.path), &authorized)?;
            let (index_added, index_deleted) = staged.get(&file.path).copied().unwrap_or((Some(0), Some(0)));
            let (worktree_added, worktree_deleted) = if file.index == "?" {
                let view = files::read_at(&root, &file.path)?;
                match (view.reason.as_deref(), view.text) { (None, Some(text)) => (Some(text.lines().count() as u64), Some(0)), (Some("file_binary"), _) => (None, None), (Some(reason), _) => return Err(reason.into()), _ => return Err("git_review_incomplete".into()) }
            } else { worktree.get(&file.path).copied().unwrap_or((Some(0), Some(0))) };
            result.files.push(GitFileStats { path: file.path.clone(), index_added, index_deleted, worktree_added, worktree_deleted });
        }
        if let Some(head) = status.overview.head {
            let mut args = vec!["log", "-z", "--format=%H%x00%an%x00%at%x00%s%x00%P", "--topo-order", "--no-decorate", "--no-show-signature", "--no-notes", "--max-count=51", &head, "--"];
            if let Some(selected) = &input.path { args.push(selected); }
            let raw = text(output(path, &args, &lease, stop, false).await?)?;
            let fields: Vec<_> = raw.split_terminator('\0').collect();
            if fields.len() % 5 != 0 { return Err("git_invalid_output".into()); }
            for row in fields.chunks_exact(5) {
                let parents: Vec<_> = row[4].split_whitespace().map(str::to_owned).collect();
                if !valid_oid(row[0]) || parents.iter().any(|p| !valid_oid(p)) { return Err("git_invalid_output".into()); }
                result.history.push(GitHistoryEntry { oid: row[0].into(), author: crate::executions::sanitize_text(row[1]), authored_at: row[2].parse().map_err(|_| "git_invalid_output")?, subject: crate::executions::sanitize_text(&row[3].chars().take(512).collect::<String>()), parents });
            }
            result.history_truncated = result.history.len() > 50; result.history.truncate(50);
        }
        crate::projects::validate_directory(&project).await?; check_lease(&lease)?;
        Ok(result)
    }
}
