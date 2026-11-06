//! Capability-relative project files. Symlinks, sensitive and special files are
//! never previewed or edited; saves compare the exact version the user read.
use crate::{projects::validate_directory, AgentManager};
use cap_std::{ambient_authority, fs::Dir};
use dgos_db::development_repository::Project;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::{Component, Path}, time::{Duration, Instant}};

pub const FILE_LIMIT: usize = 256 * 1024;
pub const ENTRY_LIMIT: usize = 10_000;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListFilesInput { pub project_id: String, #[serde(default)] pub task_id: Option<String>, pub path: String, pub query: String, pub include_ignored: bool }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadFileInput { pub project_id: String, #[serde(default)] pub task_id: Option<String>, pub path: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileReferenceInput {
    pub project_id: String, pub task_id: String, pub session_id: String,
    pub path: String, pub mode: String, #[serde(default)] pub line: Option<u32>,
}
#[derive(Debug, Serialize)]
pub struct FileReference { pub path: String, pub line: Option<u32> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveFileInput {
    pub project_id: String,
    pub task_id: String,
    pub path: String,
    pub expected_hash: String,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileEntry { pub path: String, pub name: String, pub kind: String, pub size: u64, pub ignored: bool }
#[derive(Serialize)]
pub struct FileList { pub entries: Vec<FileEntry>, pub truncated: bool }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileView { pub path: String, pub text: Option<String>, pub reason: Option<String>, pub hash: Option<String>, pub size: u64 }

pub fn relative(path: &str, empty: bool) -> Result<(), String> {
    if (!empty && path.is_empty()) || path.len() > 4096 || path.contains(['\0', '\\']) || Path::new(path).is_absolute() { return Err("file_access_denied".into()); }
    for c in Path::new(path).components() {
        match c { Component::Normal(s) if s != ".git" => {}, _ => return Err("file_access_denied".into()) }
    }
    Ok(())
}
fn io_error(error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound { "file_not_found" } else { "file_access_denied" }.into()
}
pub(crate) fn open_root(project: &Project) -> Result<Dir, String> {
    let dir = Dir::open_ambient_dir(&project.path, ambient_authority()).map_err(io_error)?;
    #[cfg(unix)] {
        use cap_std::fs::MetadataExt;
        let m = dir.dir_metadata().map_err(io_error)?;
        if format!("{}:{}", m.dev(), m.ino()) != project.directory_identity { return Err("workspace_changed".into()); }
    }
    Ok(dir)
}
fn no_links(root: &Dir, path: &str) -> Result<(), String> {
    let mut current = std::path::PathBuf::new();
    for part in Path::new(path).components() {
        current.push(part);
        if root.symlink_metadata(&current).map_err(io_error)?.file_type().is_symlink() { return Err("file_access_denied".into()); }
    }
    Ok(())
}
pub(crate) fn sensitive(path: &str) -> bool {
    Path::new(path).components().any(|c| {
        let name = c.as_os_str().to_string_lossy().to_lowercase();
        name == ".env" || name.starts_with(".env.") || matches!(name.as_str(), ".ssh" | ".aws" | ".npmrc" | ".netrc" | "credentials" | "credentials.json" | "auth.json" | "secrets.json" | "secrets.yaml" | "id_rsa" | "id_ed25519") || name.ends_with(".pem") || name.ends_with(".key")
    })
}
pub(crate) fn read_at(root: &Dir, path: &str) -> Result<FileView, String> {
    relative(path, false)?;
    no_links(root, path)?;
    let mut view = FileView { path: path.into(), text: None, reason: None, hash: None, size: 0 };
    if sensitive(path) { view.reason = Some("file_sensitive".into()); return Ok(view); }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW); }
    let file = root.open_with(path, &options).map_err(io_error)?;
    let before = file.metadata().map_err(io_error)?;
    view.size = before.len();
    if !before.is_file() { view.reason = Some("file_access_denied".into()); return Ok(view); }
    if before.len() > FILE_LIMIT as u64 { view.reason = Some("file_too_large".into()); return Ok(view); }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    (&file).take(FILE_LIMIT as u64 + 1).read_to_end(&mut bytes).map_err(io_error)?;
    let after = file.metadata().map_err(io_error)?;
    if bytes.len() > FILE_LIMIT || before.len() != after.len() || before.modified().ok() != after.modified().ok() { return Err("file_changed".into()); }
    view.hash = Some(format!("{:x}", Sha256::digest(&bytes)));
    if bytes.contains(&0) { view.reason = Some("file_binary".into()); return Ok(view); }
    match String::from_utf8(bytes) { Ok(text) => view.text = Some(text), Err(_) => view.reason = Some("file_binary".into()) }
    Ok(view)
}

struct Scanner<'a> { root: &'a Dir, absolute: &'a Path, rules: Vec<Gitignore>, entries: Vec<FileEntry>, include: bool, start: Instant, visited: usize, truncated: bool }
impl Scanner<'_> {
    fn rules_for(&mut self, path: &str) -> usize {
        let n = self.rules.len();
        for name in [".gitignore", ".ignore"] {
            let rel = if path.is_empty() { name.into() } else { format!("{path}/{name}") };
            if let Ok(view) = read_at(self.root, &rel) {
                if let Some(text) = view.text {
                    let mut builder = GitignoreBuilder::new(self.absolute.join(path));
                    let source = self.absolute.join(&rel);
                    for line in text.lines() { if builder.add_line(Some(source.clone()), line).is_err() { self.truncated = true; } }
                    if let Ok(rule) = builder.build() { self.rules.push(rule); }
                }
            }
        }
        n
    }
    fn ignored(&self, path: &str, directory: bool) -> bool {
        let mut ignored = false;
        for rules in &self.rules {
            let result = rules.matched_path_or_any_parents(self.absolute.join(path), directory);
            if result.is_ignore() { ignored = true; } else if result.is_whitelist() { ignored = false; }
        }
        ignored
    }
    fn walk(&mut self, path: &str, recursive: bool, inherited: bool) -> Result<(), String> {
        if Path::new(path).components().count() > 64 { self.truncated = true; return Ok(()); }
        no_links(self.root, path)?;
        let n = self.rules_for(path);
        let iterator = self.root.read_dir(if path.is_empty() { "." } else { path }).map_err(io_error)?;
        // Bound allocation before sorting, even for a single huge directory.
        let mut names = Vec::new();
        for item in iterator {
            if self.visited + names.len() >= ENTRY_LIMIT || self.start.elapsed() > Duration::from_secs(10) { self.truncated = true; break; }
            let entry = item.map_err(io_error)?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else { self.truncated = true; continue; };
            if name != ".git" { names.push(name); }
        }
        names.sort();
        for name in names {
            if self.visited >= ENTRY_LIMIT || self.start.elapsed() > Duration::from_secs(10) { self.truncated = true; break; }
            self.visited += 1;
            let rel = if path.is_empty() { name.clone() } else { format!("{path}/{name}") };
            let metadata = match self.root.symlink_metadata(&rel) { Ok(m) => m, Err(_) => { self.truncated = true; continue; } };
            let ignored = inherited || self.ignored(&rel, metadata.is_dir());
            if ignored && !self.include { continue; }
            let kind = if metadata.file_type().is_symlink() { "symlink" } else if metadata.is_dir() { "directory" } else if metadata.is_file() { "file" } else { "blocked" };
            self.entries.push(FileEntry { path: rel.clone(), name, kind: kind.into(), size: metadata.len(), ignored });
            if recursive && kind == "directory" {
                // All opens remain capability-relative, even if an external editor races a rename.
                if self.walk(&rel, true, ignored).is_err() { self.truncated = true; }
            }
        }
        self.rules.truncate(n);
        Ok(())
    }
}
pub(crate) fn scan(project: &Project, path: &str, recursive: bool, include: bool) -> Result<FileList, String> {
    relative(path, true)?;
    let root = open_root(project)?;
    no_links(&root, path)?;
    let mut scanner = Scanner { root: &root, absolute: Path::new(&project.path), rules: vec![], entries: vec![], include, start: Instant::now(), visited: 0, truncated: false };
    // Load ancestor rules for a direct nested-directory view.
    let mut ancestor = String::new();
    for part in Path::new(path).components() {
        scanner.rules_for(&ancestor);
        if !ancestor.is_empty() { ancestor.push('/'); }
        ancestor.push_str(&part.as_os_str().to_string_lossy());
    }
    scanner.walk(path, recursive, false)?;
    Ok(FileList { entries: scanner.entries, truncated: scanner.truncated })
}

/// Atomic replacement prevents partial saves. External editors do not participate in
/// admission: an OS-independent cross-process compare-and-swap is not promised.
pub(crate) fn save_at(project: &Project, input: SaveFileInput, lease: &tokio_util::sync::CancellationToken) -> Result<FileView, String> {
    use std::io::Write;
    relative(&input.path, false)?;
    if sensitive(&input.path) { return Err("file_sensitive".into()); }
    if input.text.len() > FILE_LIMIT { return Err("file_too_large".into()); }
    if input.text.contains('\0') { return Err("file_binary".into()); }
    let root = open_root(project)?;
    no_links(&root, &input.path)?;
    let path = Path::new(&input.path);
    let parent_path = path.parent().filter(|p| !p.as_os_str().is_empty());
    let parent = root.open_dir(parent_path.unwrap_or(Path::new("."))).map_err(io_error)?;
    let name = path.file_name().ok_or("file_access_denied")?;
    let before = read_at(&root, &input.path)?;
    if let Some(reason) = before.reason { return Err(reason); }
    if before.hash.as_deref() != Some(input.expected_hash.as_str()) { return Err("file_changed".into()); }
    let metadata = parent.symlink_metadata(name).map_err(io_error)?;
    #[cfg(unix)] {
        use cap_std::fs::MetadataExt;
        if metadata.nlink() != 1 { return Err("file_access_denied".into()); }
    }
    let temp = format!(".dgos-save-{}", uuid::Uuid::new_v4());
    let mut options = cap_std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.mode(0o600).custom_flags(libc::O_NOFOLLOW); }
    let mut file = parent.open_with(&temp, &options).map_err(io_error)?;
    let result = (|| {
        file.write_all(input.text.as_bytes()).map_err(io_error)?;
        file.set_permissions(metadata.permissions()).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        // Recheck the current path and directory immediately before replacement.
        let current_root = open_root(project)?;
        no_links(&current_root, &input.path)?;
        let current = read_at(&current_root, &input.path)?;
        if current.reason.is_some() || current.hash.as_deref() != Some(input.expected_hash.as_str()) { return Err("file_changed".into()); }
        let current_metadata = current_root.symlink_metadata(&input.path).map_err(io_error)?;
        #[cfg(unix)] {
            use cap_std::fs::MetadataExt;
            if metadata.dev() != current_metadata.dev() || metadata.ino() != current_metadata.ino() { return Err("file_changed".into()); }
            let live_parent = current_root.open_dir(parent_path.unwrap_or(Path::new("."))).map_err(io_error)?.dir_metadata().map_err(io_error)?;
            let held_parent = parent.dir_metadata().map_err(io_error)?;
            if live_parent.dev() != held_parent.dev() || live_parent.ino() != held_parent.ino() { return Err("file_changed".into()); }
        }
        if lease.is_cancelled() { return Err("session_invalid".into()); }
        parent.rename(&temp, &parent, name).map_err(io_error)?;
        let saved = read_at(&open_root(project).map_err(|_| "file_save_unknown")?, &input.path).map_err(|_| "file_save_unknown")?;
        let expected = format!("{:x}", Sha256::digest(input.text.as_bytes()));
        if saved.hash.as_deref() != Some(expected.as_str()) { return Err("file_save_unknown".into()); }
        Ok(saved)
    })();
    if result.is_err() { let _ = parent.remove_file(&temp); }
    result
}
impl AgentManager {
    pub(crate) async fn file_project(&self, owner: &str, id: &str) -> Result<Project, String> {
        self.ready().await?;
        let project = self.inner.repository.project(owner, id).await?;
        validate_directory(&project).await?;
        Ok(project)
    }
    async fn file_context(&self, owner: &str, project_id: &str, task_id: Option<&str>) -> Result<Project, String> {
        if let Some(task_id) = task_id {
            let project = self.task_project(owner, task_id).await?;
            if project.id != project_id { return Err("task_not_found".into()); }
            Ok(project)
        } else { self.file_project(owner, project_id).await }
    }
    pub async fn list_development_files(&self, owner: &str, input: ListFilesInput) -> Result<FileList, String> {
        if input.query.len() > 512 { return Err("invalid_request".into()); }
        let project = self.file_context(owner, &input.project_id, input.task_id.as_deref()).await?;
        tokio::task::spawn_blocking(move || {
            let mut list = scan(&project, &input.path, !input.query.is_empty(), input.include_ignored)?;
            if !input.query.is_empty() { let q = input.query.to_lowercase(); list.entries.retain(|e| e.path.to_lowercase().contains(&q)); }
            Ok(list)
        }).await.map_err(|_| "file_access_denied".to_string())?
    }
    pub async fn read_development_file(&self, owner: &str, input: ReadFileInput) -> Result<FileView, String> {
        let project = self.file_context(owner, &input.project_id, input.task_id.as_deref()).await?;
        tokio::task::spawn_blocking(move || read_at(&open_root(&project)?, &input.path)).await.map_err(|_| "file_access_denied".to_string())?
    }
    /// Structured native event paths are hints, never editor paths or authority.
    /// Resolve against the task's current directory and the originating session.
    pub async fn resolve_development_file_reference(&self, owner: &str, input: FileReferenceInput) -> Result<FileReference, String> {
        let _admission = self.inner.admission.lock().await;
        let project = self.file_context(owner, &input.project_id, Some(&input.task_id)).await?;
        let session = self.list(owner).await.into_iter().find(|s| s.id == input.session_id).ok_or("session_not_found")?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) || session.task_id.as_deref() != Some(input.task_id.as_str()) || session.workspace != project.path { return Err("file_access_denied".into()); }
        if input.path.len() > 4096 || input.path.contains(['\0', '\\']) { return Err("file_access_denied".into()); }
        let candidate = Path::new(&input.path);
        // Reject traversal before stripping an absolute prefix: normalization must
        // not conceal a path that leaves and then re-enters the task directory.
        if candidate.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) { return Err("file_access_denied".into()); }
        let path = if candidate.is_absolute() { candidate.strip_prefix(&project.path).map_err(|_| "file_access_denied")? } else { candidate };
        let path = path.to_str().ok_or("file_access_denied")?.to_owned();
        relative(&path, false)?;
        let source = match input.mode.as_str() {
            "file" => {
                let view = self.read_development_file(owner, ReadFileInput { project_id: input.project_id, task_id: Some(input.task_id), path: path.clone() }).await?;
                if let Some(reason) = view.reason { return Err(reason); }
                view.text.ok_or("file_access_denied")?
            },
            "diff" => {
                let diff = self.read_development_diff(owner, crate::review::DiffInput { task_id: input.task_id, path: path.clone() }).await?;
                if let Some(reason) = diff.reason { return Err(reason); }
                diff.after.or(diff.before).ok_or("file_not_found")?
            },
            _ => return Err("invalid_request".into()),
        };
        if input.line.is_some_and(|line| line == 0 || line as usize > source.lines().count().max(1)) { return Err("file_access_denied".into()); }
        Ok(FileReference { path, line: input.line })
    }
    pub async fn save_development_file(&self, owner: &str, input: SaveFileInput, lease: tokio_util::sync::CancellationToken) -> Result<FileView, String> {
        let _admission = self.inner.admission.lock().await;
        self.ready().await?;
        if lease.is_cancelled() { return Err("session_invalid".into()); }
        let task = self.inner.repository.task(owner, &input.task_id).await?;
        if task.project_id != input.project_id { return Err("task_not_found".into()); }
        let project = self.task_project(owner, &input.task_id).await?;
        if project.archived { return Err("project_archived".into()); }
        self.workspace_available(&project.path, None).await?;
        self.capture_task_baseline(owner, &input.task_id).await?;
        let workspace = project.path.clone();
        let task_id = input.task_id.clone();
        let path = input.path.clone();
        let before_hash = input.expected_hash.clone();
        let result = tokio::task::spawn_blocking(move || save_at(&project, input, &lease)).await.map_err(|_| "file_save_unknown".to_string());
        self.mark_development_mutation(&workspace).await;
        let saved = result??;
        if let Some(after_hash) = &saved.hash { self.inner.repository.save_host_file_ownership(owner, &task_id, &path, &before_hash, after_hash).await?; }
        if let Some(mut observation) = self.inner.repository.review_observation(owner, &task_id).await? {
            let mut recorded = saved.clone();
            recorded.text = None;
            observation["files"][&path] = serde_json::to_value(recorded).map_err(|_| "storage_failed")?;
            self.inner.repository.save_review_observation(owner, &task_id, &observation).await?;
        }
        Ok(saved)
    }
}
#[cfg(test)]
mod save_tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    #[test]
    fn capability_save_replaces_only_the_expected_regular_file() {
        let root = std::env::temp_dir().join(format!("dgos-save-unit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("project/nested")).unwrap();
        let path = root.join("project");
        std::fs::write(path.join("nested/code.txt"), "before\n").unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        let project = Project { id: String::new(), owner: "alice".into(), name: "x".into(), path: path.to_string_lossy().into(), directory_identity: format!("{}:{}", metadata.dev(), metadata.ino()), archived: false, created_at: 0, last_opened_at: 0 };
        let hash = format!("{:x}", Sha256::digest(b"before\n"));
        let lease = tokio_util::sync::CancellationToken::new();
        let saved = save_at(&project, SaveFileInput { project_id: String::new(), task_id: String::new(), path: "nested/code.txt".into(), expected_hash: hash, text: "after\n".into() }, &lease).unwrap();
        assert_eq!(saved.text.as_deref(), Some("after\n"));
        assert_eq!(std::fs::read_to_string(path.join("nested/code.txt")).unwrap(), "after\n");
        let stale = save_at(&project, SaveFileInput { project_id: String::new(), task_id: String::new(), path: "nested/code.txt".into(), expected_hash: format!("{:x}", Sha256::digest(b"before\n")), text: "overwrite\n".into() }, &lease).unwrap_err();
        assert_eq!(stale, "file_changed");
        assert_eq!(std::fs::read_to_string(path.join("nested/code.txt")).unwrap(), "after\n");
        assert_eq!(std::fs::read_dir(path.join("nested")).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
