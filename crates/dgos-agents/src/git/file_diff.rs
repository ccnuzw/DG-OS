//! Per-file index/worktree review, using the same hardened Git executor as delivery.
use super::*;
use crate::review::{DiffInput, DiffSource, FileDiff};

async fn blob(path: &Path, entry: Option<&GitIndexEntry>, stop: &CancellationToken) -> Result<Option<String>, String> {
    let Some(entry) = entry else { return Ok(None); };
    if !matches!(entry.mode.as_str(), "100644" | "100755") || entry.stage != 0 { return Err("git_special_entry".into()); }
    let lease = CancellationToken::new();
    let size = text(output(path, &["cat-file", "-s", &entry.oid], &lease, stop, false).await?)?.trim().parse::<usize>().map_err(|_| "git_invalid_output")?;
    if size > files::FILE_LIMIT { return Err("file_too_large".into()); }
    let bytes = output(path, &["cat-file", "blob", &entry.oid], &lease, stop, false).await?;
    if bytes.len() != size { return Err("git_invalid_output".into()); }
    if bytes.contains(&0) { return Err("file_binary".into()); }
    let text = String::from_utf8(bytes).map_err(|_| "file_binary")?;
    Ok(Some(crate::executions::sanitize_text(&text)))
}

async fn head_entry(path: &Path, head: Option<&str>, selected: &str, stop: &CancellationToken) -> Result<Option<GitIndexEntry>, String> {
    let Some(head) = head else { return Ok(None); };
    if !valid_oid(head) { return Err("git_invalid_output".into()); }
    let bytes = output(path, &["ls-tree", "-z", head, "--", selected], &CancellationToken::new(), stop, false).await?;
    let mut entry = None;
    for part in bytes.split(|b| *b == 0).filter(|part| !part.is_empty()) {
        let value = std::str::from_utf8(part).map_err(|_| "git_non_utf8_path")?;
        let (header, name) = value.split_once('\t').ok_or("git_invalid_output")?;
        let fields: Vec<_> = header.split(' ').collect();
        if name != selected || fields.len() != 3 || fields[1] != "blob" || !valid_oid(fields[2]) || entry.is_some() { return Err("git_special_entry".into()); }
        entry = Some(GitIndexEntry { path: name.into(), mode: fields[0].into(), oid: fields[2].into(), stage: 0 });
    }
    Ok(entry)
}

impl AgentManager {
    pub(crate) async fn read_git_file_diff(&self, owner: &str, input: DiffInput) -> Result<FileDiff, String> {
        files::relative(&input.path, false)?;
        if files::sensitive(&input.path) { return Err("file_sensitive".into()); }
        let project = self.task_project(owner, &input.task_id).await?;
        let path = Path::new(&project.path);
        let lease = CancellationToken::new();
        let stop = &self.inner.stop;
        let root = text(output(path, &["rev-parse", "--show-toplevel"], &lease, stop, false).await?)?;
        if tokio::fs::canonicalize(root.trim_end_matches('\n')).await.map_err(|_| "workspace_changed")? != path { return Err("git_workspace_not_root".into()); }
        let head = optional_ref(path, &["rev-parse", "--verify", "HEAD"], &lease, stop).await?;
        let index_bytes = output(path, &["ls-files", "--stage", "-z", "--", &input.path], &lease, stop, false).await?;
        let index = parse_index(&index_bytes)?;
        if index.iter().any(|entry| entry.path != input.path || entry.stage != 0) { return Err("git_unmerged".into()); }
        let head_entry = head_entry(path, head.as_deref(), &input.path, stop).await?;
        let mut authorized = index.clone();
        if let Some(entry) = &head_entry { authorized.push(entry.clone()); }
        validate_paths(&project, std::slice::from_ref(&input.path), &authorized)?;
        let current = match files::read_at(&files::open_root(&project)?, &input.path) {
            Ok(view) => { if let Some(reason) = &view.reason { return Err(reason.clone()); } Some(view) },
            Err(error) if error == "file_not_found" => None,
            Err(error) => return Err(error),
        };
        let (before, after) = match input.source {
            DiffSource::Index => (blob(path, head_entry.as_ref(), stop).await?, blob(path, index.first(), stop).await?),
            DiffSource::Worktree => (blob(path, index.first(), stop).await?, current.as_ref().and_then(|view| view.text.as_deref()).map(crate::executions::sanitize_text)),
            DiffSource::Task => return Err("invalid_request".into()),
        };
        if before.is_none() && after.is_none() { return Err("file_not_found".into()); }
        let diff = if input.source == DiffSource::Worktree && index.is_empty() {
            // Git diff omits untracked files; explicitly construct their complete
            // addition from the capability-validated, bounded current file.
            similar::TextDiff::from_lines("", after.as_deref().unwrap_or("")).unified_diff().context_radius(3).header("/dev/null", &format!("worktree/{}", input.path)).to_string()
        } else {
            let mut args = vec!["diff", "--no-ext-diff", "--no-textconv", "--no-renames", "--no-color", "--no-relative", "--no-indent-heuristic", "--diff-algorithm=myers"];
            if input.source == DiffSource::Index { args.push("--cached"); }
            args.extend(["--", input.path.as_str()]);
            let bytes = output(path, &args, &lease, stop, false).await?;
            if bytes.len() > files::FILE_LIMIT { return Err("file_too_large".into()); }
            let diff = String::from_utf8(bytes).map_err(|_| "file_binary")?;
            crate::executions::sanitize_text(&diff)
        };
        if diff.len() > files::FILE_LIMIT { return Err("file_too_large".into()); }
        // External Git/editors do not honor admission; never combine a patch with
        // previews from another observed HEAD/index/worktree version.
        if head != optional_ref(path, &["rev-parse", "--verify", "HEAD"], &lease, stop).await? || index_bytes != output(path, &["ls-files", "--stage", "-z", "--", &input.path], &lease, stop, false).await? { return Err("file_changed".into()); }
        let latest = match files::read_at(&files::open_root(&project)?, &input.path) {
            Ok(view) => Some(view), Err(error) if error == "file_not_found" => None, Err(error) => return Err(error),
        };
        if current.as_ref().map(|view| (&view.hash, &view.reason)) != latest.as_ref().map(|view| (&view.hash, &view.reason)) { return Err("file_changed".into()); }
        crate::projects::validate_directory(&project).await?;
        Ok(FileDiff { path: input.path, before, after, diff, reason: None })
    }
}
