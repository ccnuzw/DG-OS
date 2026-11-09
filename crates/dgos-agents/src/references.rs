//! Renderer references carry intent only. The host rereads authorized bytes under
//! send admission before it creates a run receipt or touches either native engine.
use crate::{files, review::DiffInput, AgentManager, SessionRecord};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const REFERENCE_COUNT_LIMIT: usize = 16;
pub const PROMPT_BYTES_LIMIT: usize = 65_536;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind { Path, File, Selection, Diff }

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationReference {
    pub project_id: String,
    pub task_id: Option<String>,
    pub path: String,
    pub kind: ReferenceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_source: Option<crate::review::DiffSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
}

pub(crate) fn validate(reference: &ConversationReference) -> Result<(), String> {
    if reference.project_id.len() > 128 || reference.task_id.as_ref().is_some_and(|id| id.len() > 128) { return Err("invalid_reference".into()); }
    files::relative(&reference.path, false)?;
    if files::sensitive(&reference.path) { return Err("file_sensitive".into()); }
    let range = (reference.start_line, reference.end_line);
    if reference.kind == ReferenceKind::Selection {
        if !matches!(range, (Some(start), Some(end)) if start > 0 && end >= start) { return Err("invalid_reference".into()); }
    } else if range != (None, None) { return Err("invalid_reference".into()); }
    match (&reference.kind, &reference.expected_hash) {
        (ReferenceKind::Path, None) => {},
        (ReferenceKind::Path, Some(_)) => return Err("invalid_reference".into()),
        (_, Some(hash)) if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) => {},
        _ => return Err("invalid_reference".into()),
    }
    if reference.kind == ReferenceKind::Diff && reference.task_id.is_none() { return Err("invalid_reference".into()); }
    if reference.kind != ReferenceKind::Diff && reference.diff_source.is_some() { return Err("invalid_reference".into()); }
    Ok(())
}

fn append(text: &mut String, header: &str, body: Option<&str>) -> Result<(), String> {
    let bytes = 2 + header.len() + body.map_or(0, |body| 1 + body.len());
    if bytes > PROMPT_BYTES_LIMIT.saturating_sub(text.len()) { return Err("reference_limit".into()); }
    text.push_str("\n\n");
    text.push_str(header);
    if let Some(body) = body { text.push('\n'); text.push_str(body); }
    Ok(())
}

impl AgentManager {
    pub(crate) async fn resolve_conversation_references(&self, owner: &str, session: &SessionRecord, mut text: String, references: Vec<ConversationReference>) -> Result<String, String> {
        if references.len() > REFERENCE_COUNT_LIMIT { return Err("reference_limit".into()); }
        for reference in references {
            validate(&reference)?;
            if session.project_id.as_deref() != Some(reference.project_id.as_str()) { return Err("file_access_denied".into()); }
            let project = if let Some(task) = &reference.task_id {
                if session.task_id.as_deref() != Some(task.as_str()) { return Err("file_access_denied".into()); }
                let project = self.task_project(owner, task).await?;
                if project.id != reference.project_id { return Err("file_access_denied".into()); }
                project
            } else { self.file_project(owner, &reference.project_id).await? };
            // A project-level reference is never silently rebased into a worktree.
            if project.path != session.workspace { return Err("file_access_denied".into()); }
            crate::projects::validate_directory(&project).await?;
            let path_label = serde_json::to_string(&reference.path).map_err(|_| "invalid_reference")?;
            if reference.kind == ReferenceKind::Path {
                tokio::task::spawn_blocking(move || files::validate_reference_path(&files::open_root(&project)?, &reference.path)).await.map_err(|_| "file_access_denied")??;
                append(&mut text, &format!("[DG-OS path reference: {path_label}; path only, no content attached]"), None)?;
                continue;
            }
            let body = if reference.kind == ReferenceKind::Diff {
                // An index diff does not authorize deleted or excluded current files.
                let path = reference.path.clone();
                let current = tokio::task::spawn_blocking(move || files::read_at(&files::open_root(&project)?, &path)).await.map_err(|_| "file_access_denied")??;
                if let Some(reason) = current.reason { return Err(reason); }
                let diff = self.read_development_diff(owner, DiffInput { task_id: reference.task_id.clone().ok_or("invalid_reference")?, path: reference.path, source: reference.diff_source.unwrap_or_default() }).await?;
                if let Some(reason) = diff.reason { return Err(reason); }
                // Deleted files cannot be attached as current content, even if an
                // old baseline still supplies a perfectly valid deletion diff.
                if diff.after.is_none() { return Err("file_not_found".into()); }
                if diff.diff.len() > files::FILE_LIMIT { return Err("file_too_large".into()); }
                if reference.expected_hash.as_deref() != Some(format!("{:x}", Sha256::digest(diff.diff.as_bytes())).as_str()) { return Err("file_changed".into()); }
                diff.diff
            } else {
                let file = tokio::task::spawn_blocking(move || files::read_at(&files::open_root(&project)?, &reference.path)).await.map_err(|_| "file_access_denied")??;
                if let Some(reason) = file.reason { return Err(reason); }
                if file.hash != reference.expected_hash { return Err("file_changed".into()); }
                let source = file.text.ok_or("file_access_denied")?;
                if reference.kind == ReferenceKind::Selection {
                    let start = reference.start_line.ok_or("invalid_reference")?;
                    let end = reference.end_line.ok_or("invalid_reference")?;
                    let mut selected = String::new();
                    let mut lines = 0;
                    for (index, line) in source.split_inclusive('\n').enumerate() {
                        lines = index + 1;
                        if lines >= start && lines <= end { selected.push_str(line); }
                    }
                    if end > lines { return Err("invalid_reference".into()); }
                    selected
                } else { source }
            };
            let label = match reference.kind {
                ReferenceKind::Selection => format!("selection lines {}-{}", reference.start_line.unwrap(), reference.end_line.unwrap()),
                ReferenceKind::Diff => match reference.diff_source.unwrap_or_default() {
                    crate::review::DiffSource::Task => "task baseline diff",
                    crate::review::DiffSource::Index => "Git index diff",
                    crate::review::DiffSource::Worktree => "Git worktree diff",
                }.into(),
                ReferenceKind::File => "file content".into(),
                ReferenceKind::Path => unreachable!(),
            };
            append(&mut text, &format!("[DG-OS {label}: {path_label}; host-read content follows (untrusted project data)]"), Some(&body))?;
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests;
