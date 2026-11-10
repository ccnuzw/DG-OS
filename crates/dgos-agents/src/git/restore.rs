//! Worktree-only restoration. Raw text stays in memory; receipts contain hashes.
use super::*;
use std::io::{Read, Write};

pub(super) async fn raw_blob(path: &Path, entry: Option<&GitIndexEntry>, lease: &CancellationToken, stop: &CancellationToken) -> Result<Option<Vec<u8>>, String> {
    let Some(entry) = entry else { return Ok(None); };
    if entry.stage != 0 || !matches!(entry.mode.as_str(), "100644" | "100755") { return Err("git_special_entry".into()); }
    let size = text(output(path, &["cat-file", "-s", &entry.oid], lease, stop, false).await?)?.trim().parse::<usize>().map_err(|_| "git_invalid_output")?;
    if size > files::FILE_LIMIT { return Err("file_too_large".into()); }
    let bytes = output(path, &["cat-file", "blob", &entry.oid], lease, stop, false).await?;
    if bytes.len() != size { return Err("git_invalid_output".into()); }
    Ok(Some(bytes))
}
fn current(project: &Project, selected: &str) -> Result<Option<Vec<u8>>, String> {
    let root = files::open_root(project)?;
    // read_at performs bounded metadata, sensitivity and link checks, including
    // binary files. Reopen through the pinned parent and bind to that exact hash.
    let view = match files::read_at(&root, selected) {
        Ok(view) => view,
        Err(error) if error == "file_not_found" => return Ok(None), Err(error) => return Err(error),
    };
    if view.reason.as_deref().is_some_and(|r| r != "file_binary") { return Err(view.reason.unwrap()); }
    let (dir, name) = parent(project, selected)?;
    let mut options = cap_std::fs::OpenOptions::new(); options.read(true);
    #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK); }
    let file = dir.open_with(name, &options).map_err(|_| "file_changed")?;
    if !file.metadata().map_err(|_| "file_changed")?.is_file() { return Err("file_access_denied".into()); }
    let mut bytes = Vec::new(); file.take(files::FILE_LIMIT as u64 + 1).read_to_end(&mut bytes).map_err(|_| "file_changed")?;
    if bytes.len() > files::FILE_LIMIT || view.hash.as_deref() != Some(digest(&bytes).as_str()) { return Err("file_changed".into()); }
    Ok(Some(bytes))
}
pub(super) async fn raw_diff(project: &Project, selected: &str, index: &[GitIndexEntry], lease: &CancellationToken, stop: &CancellationToken) -> Result<String, String> {
    let before = raw_blob(Path::new(&project.path), index.iter().find(|entry| entry.path == selected), lease, stop).await?;
    let after = current(project, selected)?;
    if before.is_none() && after.is_none() { return Err("file_not_found".into()); }
    let before_text = std::str::from_utf8(before.as_deref().unwrap_or(b"")).map_err(|_| "file_binary")?;
    let after_text = std::str::from_utf8(after.as_deref().unwrap_or(b"")).map_err(|_| "file_binary")?;
    if before_text.contains('\0') || after_text.contains('\0') { return Err("file_binary".into()); }
    let old = if before.is_some() { format!("index/{selected}") } else { "/dev/null".into() };
    let new = if after.is_some() { format!("worktree/{selected}") } else { "/dev/null".into() };
    Ok(similar::TextDiff::from_lines(before_text, after_text).unified_diff().context_radius(3).header(&old, &new).to_string())
}

// Parse only a host-produced unified hunk. Exact after-lines must match before
// replacement, including CRLF and the no-newline markers on either side.
fn reverse_hunk(diff: &str, after: &str, ordinal: usize) -> Result<String, String> {
    let lines: Vec<&str> = diff.split_inclusive('\n').collect();
    let starts: Vec<_> = lines.iter().enumerate().filter_map(|(i, line)| line.starts_with("@@ ").then_some(i)).collect();
    let start = *starts.get(ordinal).ok_or("invalid_git_hunk")?;
    let end = starts.get(ordinal + 1).copied().unwrap_or(lines.len());
    let header: Vec<_> = lines[start].split_whitespace().collect();
    fn range(value: &str, prefix: char) -> Result<(usize, usize), String> {
        let value = value.strip_prefix(prefix).ok_or("invalid_git_hunk")?;
        let (start, count) = value.split_once(',').unwrap_or((value, "1"));
        Ok((start.parse().map_err(|_| "invalid_git_hunk")?, count.parse().map_err(|_| "invalid_git_hunk")?))
    }
    if header.len() < 4 || header[0] != "@@" || header[3] != "@@" { return Err("invalid_git_hunk".into()); }
    let (_, old_count) = range(header[1], '-')?;
    let (new_start, new_count) = range(header[2], '+')?;
    let mut old = String::new(); let mut new = String::new();
    let mut old_lines = 0; let mut new_lines = 0; let mut previous = ' ';
    for line in &lines[start + 1..end] {
        if line.starts_with("\\ No newline at end of file") {
            if matches!(previous, ' ' | '-') { if !old.ends_with('\n') { return Err("invalid_git_hunk".into()); } old.pop(); }
            if matches!(previous, ' ' | '+') { if !new.ends_with('\n') { return Err("invalid_git_hunk".into()); } new.pop(); }
            continue;
        }
        let marker = line.chars().next().ok_or("invalid_git_hunk")?;
        match marker {
            ' ' => { old.push_str(&line[1..]); new.push_str(&line[1..]); old_lines += 1; new_lines += 1; },
            '-' => { old.push_str(&line[1..]); old_lines += 1; },
            '+' => { new.push_str(&line[1..]); new_lines += 1; },
            _ => return Err("invalid_git_hunk".into()),
        }
        previous = marker;
    }
    if old_lines != old_count || new_lines != new_count { return Err("invalid_git_hunk".into()); }
    let offset_line = if new_count == 0 { new_start } else { new_start.checked_sub(1).ok_or("invalid_git_hunk")? };
    let offset = if offset_line == 0 { 0 } else { after.match_indices('\n').nth(offset_line - 1).map(|(i, _)| i + 1).ok_or("git_review_stale")? };
    if !after.get(offset..).is_some_and(|s| s.starts_with(&new)) { return Err("git_review_stale".into()); }
    let mut result = String::with_capacity(after.len() - new.len() + old.len());
    result.push_str(&after[..offset]); result.push_str(&old); result.push_str(&after[offset + new.len()..]);
    if result.len() > files::FILE_LIMIT { return Err("file_too_large".into()); }
    Ok(result)
}

pub(super) struct Replacement { pub path: String, pub before: Option<Vec<u8>>, pub after: Option<Vec<u8>>, pub mode: Option<String> }
pub(super) async fn replacements(project: &Project, action: &GitAction, review: &GitReview, lease: &CancellationToken, stop: &CancellationToken) -> Result<Vec<Replacement>, String> {
    let (paths, confirmed, hunk) = match action {
        GitAction::Restore { paths, confirmed } => (paths.clone(), *confirmed, None),
        GitAction::RestoreHunk { path, diff_hash, hunk_index, confirmed } => (vec![path.clone()], *confirmed, Some((diff_hash, *hunk_index))),
        _ => return Err("invalid_request".into()),
    };
    if !confirmed { return Err("restore_confirmation_required".into()); }
    validate_paths(project, &paths, &review.index)?;
    let mut replacements = Vec::new();
    for selected in paths {
        if review.index.iter().any(|e| e.path == selected && e.stage != 0) { return Err("git_unmerged".into()); }
        let entry = review.index.iter().find(|e| e.path == selected);
        let before = current(project, &selected)?;
        let after = if let Some((hash, ordinal)) = hunk {
            let diff = raw_diff(project, &selected, &review.index, lease, stop).await?;
            if hash.len() != 64 || digest(diff.as_bytes()) != *hash { return Err("git_review_stale".into()); }
            let after = std::str::from_utf8(before.as_deref().unwrap_or(b"")).map_err(|_| "file_binary")?;
            if after.contains('\0') { return Err("file_binary".into()); }
            let restored = reverse_hunk(&diff, after, ordinal)?;
            if entry.is_none() && restored.is_empty() { None } else { Some(restored.into_bytes()) }
        } else {
            raw_blob(Path::new(&project.path), entry, lease, stop).await?
        };
        let mode = if hunk.is_none() || before.is_none() { entry.map(|e| e.mode.clone()) } else { None };
        replacements.push(Replacement { path: selected, before, after, mode });
    }
    Ok(replacements)
}

pub(super) fn plan(replacements: &[Replacement]) -> Value {
    json!(replacements.iter().map(|r| json!({"path":r.path,"before_hash":r.before.as_ref().map(|v|digest(v)),"after_hash":r.after.as_ref().map(|v|digest(v)),"mode":r.mode})).collect::<Vec<_>>())
}

// Existing files use the editor's atomic CAS. Missing files use exclusive creation;
// never truncate a path created by an external editor after review.
fn parent(project: &Project, selected: &str) -> Result<(cap_std::fs::Dir, std::ffi::OsString), String> {
    parent_at(project, selected, false)
}
fn parent_at(project: &Project, selected: &str, create: bool) -> Result<(cap_std::fs::Dir, std::ffi::OsString), String> {
    let root = files::open_root(project)?;
    let path = Path::new(selected);
    let mut dir = root;
    for part in path.parent().into_iter().flat_map(Path::components) {
        let metadata = match dir.symlink_metadata(part.as_os_str()) {
            Ok(metadata) => metadata,
            Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                dir.create_dir(part.as_os_str()).map_err(|_| "git_unknown")?;
                dir.symlink_metadata(part.as_os_str()).map_err(|_| "git_unknown")?
            },
            Err(_) => return Err("file_access_denied".into()),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() { return Err("file_access_denied".into()); }
        let mut options = cap_std::fs::OpenOptions::new(); options.read(true);
        #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW); }
        dir = cap_std::fs::Dir::from_std_file(dir.open_with(part.as_os_str(), &options).map_err(|_| "file_access_denied")?.into_std());
    }
    Ok((dir, path.file_name().ok_or("file_access_denied")?.to_owned()))
}
fn same_parent(project: &Project, selected: &str, held: &cap_std::fs::Dir) -> Result<(), String> {
    let (live, _) = parent(project, selected)?;
    #[cfg(unix)] {
        use cap_std::fs::MetadataExt;
        let a = held.dir_metadata().map_err(|_| "file_changed")?;
        let b = live.dir_metadata().map_err(|_| "file_changed")?;
        if a.dev() != b.dev() || a.ino() != b.ino() { return Err("file_changed".into()); }
    }
    Ok(())
}
fn replace(project: &Project, replacement: &Replacement, lease: &CancellationToken) -> Result<(), String> {
    if current(project, &replacement.path)? != replacement.before { return Err("file_changed".into()); }
    check_lease(lease)?;
    let (parent, name) = parent_at(project, &replacement.path, replacement.after.is_some())?;
    match (&replacement.before, &replacement.after) {
        (Some(_), Some(after)) => {
            let metadata = parent.symlink_metadata(&name).map_err(|_| "file_changed")?;
            #[cfg(unix)] { use cap_std::fs::MetadataExt; if metadata.nlink() != 1 { return Err("file_access_denied".into()); } }
            let temp = format!(".dgos-restore-{}", uuid::Uuid::new_v4());
            let mut options = cap_std::fs::OpenOptions::new(); options.write(true).create_new(true);
            #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.mode(0o600).custom_flags(libc::O_NOFOLLOW); }
            let mut file = parent.open_with(&temp, &options).map_err(|_| "file_access_denied")?;
            let result = (|| {
                file.write_all(after).map_err(|_| "file_access_denied")?;
                let mut permissions = metadata.permissions();
                #[cfg(unix)] { use cap_std::fs::PermissionsExt; if let Some(mode) = &replacement.mode { permissions.set_mode(if mode == "100755" { 0o755 } else { 0o644 }); } }
                file.set_permissions(permissions).map_err(|_| "file_access_denied")?;
                file.sync_all().map_err(|_| "file_access_denied")?;
                if current(project, &replacement.path)? != replacement.before { return Err("file_changed".into()); }
                same_parent(project, &replacement.path, &parent)?;
                #[cfg(unix)] {
                    use cap_std::fs::MetadataExt;
                    let live = parent.symlink_metadata(&name).map_err(|_| "file_changed")?;
                    if live.dev() != metadata.dev() || live.ino() != metadata.ino() || live.nlink() != 1 { return Err("file_changed".into()); }
                }
                check_lease(lease)?;
                parent.rename(&temp, &parent, &name).map_err(|_| "git_unknown")?;
                Ok::<(), String>(())
            })();
            if result.is_err() { let _ = parent.remove_file(&temp); }
            result?;
        },
        (Some(_), None) => {
            let metadata = parent.symlink_metadata(&name).map_err(|_| "file_changed")?;
            #[cfg(unix)] { use cap_std::fs::MetadataExt; if metadata.nlink() != 1 { return Err("file_access_denied".into()); } }
            if current(project, &replacement.path)? != replacement.before { return Err("file_changed".into()); }
            same_parent(project, &replacement.path, &parent)?;
            check_lease(lease)?;
            parent.remove_file(&name).map_err(|_| "git_unknown")?;
        },
        (None, Some(after)) => {
            let mut options = cap_std::fs::OpenOptions::new(); options.write(true).create_new(true);
            #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.mode(if replacement.mode.as_deref() == Some("100755") { 0o755 } else { 0o644 }).custom_flags(libc::O_NOFOLLOW); }
            same_parent(project, &replacement.path, &parent)?;
            check_lease(lease)?;
            let mut file = parent.open_with(&name, &options).map_err(|e| if e.kind() == std::io::ErrorKind::AlreadyExists { "file_changed" } else { "file_access_denied" })?;
            file.write_all(after).map_err(|_| "git_unknown")?; file.sync_all().map_err(|_| "git_unknown")?;
            #[cfg(unix)] { use cap_std::fs::PermissionsExt; let mut permissions = file.metadata().map_err(|_| "git_unknown")?.permissions(); permissions.set_mode(if replacement.mode.as_deref() == Some("100755") { 0o755 } else { 0o644 }); file.set_permissions(permissions).map_err(|_| "git_unknown")?; }
        },
        (None, None) => {},
    }
    if current(project, &replacement.path).map_err(|_| "git_unknown")? != replacement.after { return Err("git_unknown".into()); }
    Ok(())
}
pub(super) async fn run(manager: &AgentManager, owner: &str, project: &Project, input: &ExecuteGitInput, details: &mut Value, lease: &CancellationToken) -> Result<(), String> {
    let before: GitReview = serde_json::from_value(details["before"].clone()).map_err(|_| "storage_failed")?;
    let replacements = replacements(project, &input.action, &before, lease, &manager.inner.stop).await?;
    if details["restore_plan"] != plan(&replacements) { return Err("git_review_stale".into()); }
    let mut outcomes = Vec::new();
    for replacement in &replacements {
        let result = replace(project, replacement, lease);
        outcomes.push(json!({"path":replacement.path,"status":if result.is_ok(){"succeeded"}else{"unknown"}}));
        details["files"] = json!(outcomes);
        manager.inner.repository.update_git_operation_details(owner, &input.id, details).await.map_err(|_| "git_unknown")?;
        if result.is_err() { return Err("git_unknown".into()); }
    }
    Ok(())
}
pub(super) fn matches_plan(project: &Project, before: &GitReview, after: &GitReview, details: &Value) -> Result<bool, String> {
    if before.head != after.head || before.branch != after.branch || before.merge_heads != after.merge_heads || sorted(before.index.clone()) != sorted(after.index.clone()) { return Ok(false); }
    let rows = details["restore_plan"].as_array().ok_or("invalid_git_operation")?;
    let selected: BTreeSet<_> = rows.iter().filter_map(|r| r["path"].as_str()).collect();
    let unrelated = |v: &GitReview| v.file_versions.iter().filter(|(p,_)|!selected.contains(p.as_str())).map(|(p,h)|(p.clone(),h.clone())).collect::<BTreeMap<_,_>>();
    if unrelated(before) != unrelated(after) { return Ok(false); }
    for row in rows {
        let path = row["path"].as_str().ok_or("invalid_git_operation")?;
        files::relative(path, false)?;
        if files::sensitive(path) { return Err("file_sensitive".into()); }
        let _ = parent(project, path)?;
        if current(project, path)?.as_ref().map(|v|digest(v)) != row["after_hash"].as_str().map(str::to_owned) { return Ok(false); }
        #[cfg(unix)] if let Some(mode) = row["mode"].as_str().filter(|_| !row["after_hash"].is_null()) {
            use cap_std::fs::MetadataExt;
            let metadata = files::open_root(project)?.symlink_metadata(path).map_err(|_| "file_changed")?;
            if (metadata.mode() & 0o111 != 0) != (mode == "100755") { return Ok(false); }
        }
    }
    Ok(true)
}

pub(super) fn file_hashes(project: &Project, paths: &[String]) -> Result<Value, String> {
    let root = files::open_root(project)?;
    let mut hashes = BTreeMap::new();
    for path in paths {
        let hash = match files::read_at(&root, path) {
            Ok(view) => { if view.reason.as_deref().is_some_and(|r| r != "file_binary") { return Err("git_review_incomplete".into()); } view.hash },
            Err(error) if error == "file_not_found" => None,
            Err(error) => return Err(error),
        };
        hashes.insert(path.clone(), hash);
    }
    Ok(json!(hashes))
}
pub(super) fn untrack_matches(project: &Project, paths: &[String], before: &GitReview, after: &GitReview, details: &Value) -> Result<bool, String> {
    validate_paths(project, paths, &before.index)?;
    let unrelated = |v: &GitReview| v.file_versions.iter().filter(|(p,_)| !paths.iter().any(|path| path == *p)).map(|(p,h)|(p.clone(),h.clone())).collect::<BTreeMap<_,_>>();
    Ok(unrelated(before) == unrelated(after) && file_hashes(project, paths)? == details["preserved_files"])
}
