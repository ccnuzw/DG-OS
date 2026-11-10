//! Immutable commit inspection. No index, working-tree contents, or checkout is read.
use super::*;
use crate::review::FileDiff;

const COMMIT_LIMIT: usize = 128 * 1024;
const COMMIT_FILES: usize = 1000;
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitCommitInput { pub project_id: String, pub task_id: Option<String>, pub oid: String }
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitCommitDiffInput { pub project_id: String, pub task_id: Option<String>, pub oid: String, pub path: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitCommitFile { pub path: String, pub original_path: Option<String>, pub status: String, pub added: Option<u64>, pub deleted: Option<u64> }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitCommitDetails { pub oid: String, pub author: String, pub author_email: String, pub timestamp: i64, pub message: String, pub parents: Vec<String>, pub files: Vec<GitCommitFile>, pub truncated: bool }
struct Change { file: GitCommitFile, before_mode: String, before_oid: String, after_mode: String, after_oid: String }

async fn commit_output(path: &Path, args: &[&str], attributes: Option<&str>, lease: &CancellationToken, stop: &CancellationToken) -> Result<Vec<u8>, String> {
    let mut command = safe_command(path).await?;
    // Missing promisor objects must never cause an implicit network fetch.
    command.env("GIT_NO_LAZY_FETCH", "1").args(["-c", "protocol.file.allow=never", "-c", "protocol.http.allow=never", "-c", "protocol.https.allow=never", "-c", "protocol.ssh.allow=never"]);
    if let Some(oid) = attributes { command.env("GIT_ATTR_SOURCE", oid); }
    command.args(args);
    complete(git_output(command, Duration::from_secs(10), lease, stop).await?)
}
async fn output(path: &Path, args: &[&str], lease: &CancellationToken, stop: &CancellationToken, _write: bool) -> Result<Vec<u8>, String> {
    commit_output(path, args, None, lease, stop).await
}

fn display(value: &str, multiline: bool) -> String {
    crate::executions::sanitize_text(&value.chars().filter(|c| !c.is_control() || (multiline && matches!(c, '\n' | '\t'))).collect::<String>())
}
fn path_field(value: &[u8]) -> Result<String, String> {
    let value = std::str::from_utf8(value).map_err(|_| "git_non_utf8_path")?;
    files::relative(value, false)?;
    if value.chars().any(char::is_control) { return Err("invalid_git_paths".into()); }
    Ok(value.into())
}
fn nul_fields(bytes: &[u8]) -> Result<Vec<&[u8]>, String> {
    if bytes.is_empty() { return Ok(vec![]); }
    if bytes.last() != Some(&0) { return Err("git_invalid_output".into()); }
    Ok(bytes[..bytes.len() - 1].split(|b| *b == 0).collect())
}
fn raw_changes(bytes: &[u8]) -> Result<Vec<Change>, String> {
    let fields = nul_fields(bytes)?; let mut fields = fields.into_iter(); let mut result = vec![];
    while let Some(header) = fields.next() {
        let header = std::str::from_utf8(header).map_err(|_| "git_invalid_output")?;
        let parts: Vec<_> = header.strip_prefix(':').ok_or("git_invalid_output")?.split(' ').collect();
        if parts.len() != 5 || !valid_oid(parts[2]) || !valid_oid(parts[3]) || !parts[..2].iter().all(|v| v.len() == 6 && v.bytes().all(|b| b.is_ascii_digit())) { return Err("git_invalid_output".into()); }
        let status = parts[4];
        let rename = status.strip_prefix('R').is_some_and(|score| !score.is_empty() && score.parse::<u8>().is_ok_and(|score| score <= 100));
        if !matches!(status, "A" | "M" | "D" | "T") && !rename { return Err("git_invalid_output".into()); }
        let first = path_field(fields.next().ok_or("git_invalid_output")?)?;
        let (path, original_path) = if rename { (path_field(fields.next().ok_or("git_invalid_output")?)?, Some(first)) } else { (first, None) };
        result.push(Change { file: GitCommitFile { path, original_path, status: status[..1].into(), added: None, deleted: None }, before_mode: parts[0].into(), before_oid: parts[2].into(), after_mode: parts[1].into(), after_oid: parts[3].into() });
    }
    Ok(result)
}
fn apply_stats(changes: &mut [Change], bytes: &[u8]) -> Result<(), String> {
    let fields = nul_fields(bytes)?; let mut fields = fields.into_iter(); let mut seen = BTreeSet::new();
    let indices: BTreeMap<_, _> = changes.iter().enumerate().map(|(i, c)| (c.file.path.clone(), i)).collect();
    while let Some(record) = fields.next() {
        let mut record = record.splitn(3, |b| *b == b'\t');
        let number = |v: Option<&[u8]>| -> Result<Option<u64>, String> { let v = v.ok_or("git_invalid_output")?; if v == b"-" { Ok(None) } else { Ok(Some(std::str::from_utf8(v).map_err(|_| "git_invalid_output")?.parse().map_err(|_| "git_invalid_output")?)) } };
        let added = number(record.next())?; let deleted = number(record.next())?;
        let name = record.next().ok_or("git_invalid_output")?;
        let (path, original) = if name.is_empty() { let old = path_field(fields.next().ok_or("git_invalid_output")?)?; (path_field(fields.next().ok_or("git_invalid_output")?)?, Some(old)) } else { (path_field(name)?, None) };
        let index = *indices.get(&path).ok_or("git_invalid_output")?;
        if !seen.insert(index) || changes[index].file.original_path != original { return Err("git_invalid_output".into()); }
        changes[index].file.added = added; changes[index].file.deleted = deleted;
    }
    if seen.len() != changes.len() { return Err("git_invalid_output".into()); }
    Ok(())
}
async fn changes(path: &Path, oid: &str, parent: Option<&str>, lease: &CancellationToken, stop: &CancellationToken) -> Result<Vec<Change>, String> {
    // Historical attributes and explicit rename options keep this independent of
    // the checkout. No textconv, external diff, or submodule programs may run.
    let base = ["diff-tree", "--root", "--no-commit-id", "-r", "-z", "--no-ext-diff", "--no-textconv", "--no-relative", "--no-color", "-O/dev/null", "--ignore-submodules=none", "--find-renames=50%", "-l1000", "--no-abbrev", "--diff-algorithm=myers", "--no-indent-heuristic"];
    let mut raw = base.to_vec(); raw.push("--raw");
    if let Some(parent) = parent { raw.push(parent); } raw.extend([oid, "--"]);
    let mut result = raw_changes(&commit_output(path, &raw, Some(oid), lease, stop).await?)?;
    let mut stats = base.to_vec(); stats.push("--numstat");
    if let Some(parent) = parent { stats.push(parent); } stats.extend([oid, "--"]);
    apply_stats(&mut result, &commit_output(path, &stats, Some(oid), lease, stop).await?)?;
    result.sort_by(|a, b| a.file.path.cmp(&b.file.path));
    Ok(result)
}
async fn metadata(path: &Path, oid: &str, lease: &CancellationToken, stop: &CancellationToken) -> Result<GitCommitDetails, String> {
    if !valid_oid(oid) { return Err("invalid_git_ref".into()); }
    let kind = text(output(path, &["cat-file", "-t", oid], lease, stop, false).await?)?;
    if kind.trim() != "commit" { return Err("invalid_git_ref".into()); }
    let size: usize = text(output(path, &["cat-file", "-s", oid], lease, stop, false).await?)?.trim().parse().map_err(|_| "git_invalid_output")?;
    if size > COMMIT_LIMIT { return Err("file_too_large".into()); }
    let bytes = output(path, &["show", "-s", "--no-show-signature", "--no-notes", "--no-decorate", "--format=%H%x00%an%x00%ae%x00%at%x00%P%x00%B%x00", oid, "--"], lease, stop, false).await?;
    if bytes.len() > COMMIT_LIMIT + 4096 { return Err("file_too_large".into()); }
    // Git appends one record newline, beyond the explicit final NUL.
    let bytes = bytes.strip_suffix(b"\n").ok_or("git_invalid_output")?;
    let fields = nul_fields(bytes)?;
    if fields.len() != 6 { return Err("git_invalid_output".into()); }
    let fields: Vec<_> = fields.iter().map(|v| std::str::from_utf8(v).map_err(|_| "git_invalid_output")).collect::<Result<_, _>>()?;
    let parents: Vec<String> = fields[4].split_whitespace().map(str::to_owned).collect();
    if !fields[0].eq_ignore_ascii_case(oid) || parents.iter().any(|v| !valid_oid(v)) { return Err("git_invalid_output".into()); }
    Ok(GitCommitDetails { oid: fields[0].into(), author: display(fields[1], false), author_email: display(fields[2], false), timestamp: fields[3].parse().map_err(|_| "git_invalid_output")?, message: display(fields[5], true), parents, files: vec![], truncated: false })
}
async fn historical_blob(path: &Path, mode: &str, oid: &str, lease: &CancellationToken, stop: &CancellationToken) -> Result<Option<String>, String> {
    if mode == "000000" { return Ok(None); }
    if !matches!(mode, "100644" | "100755") { return Err("git_special_entry".into()); }
    let size: usize = text(output(path, &["cat-file", "-s", oid], lease, stop, false).await?)?.trim().parse().map_err(|_| "git_invalid_output")?;
    if size > files::FILE_LIMIT { return Err("file_too_large".into()); }
    let bytes = output(path, &["cat-file", "blob", oid], lease, stop, false).await?;
    if bytes.len() != size { return Err("git_invalid_output".into()); }
    if bytes.contains(&0) { return Err("file_binary".into()); }
    Ok(Some(crate::executions::sanitize_text(&String::from_utf8(bytes).map_err(|_| "file_binary")?)))
}
impl AgentManager {
    async fn commit_project(&self, owner: &str, input: &GitCommitInput, lease: &CancellationToken) -> Result<Project, String> {
        self.ready().await?; check_lease(lease)?;
        let project = if let Some(task) = &input.task_id { let project = self.task_project(owner, task).await?; if project.id != input.project_id { return Err("task_not_found".into()); } project } else { self.file_project(owner, &input.project_id).await? };
        let root = text(output(Path::new(&project.path), &["rev-parse", "--show-toplevel"], lease, &self.inner.stop, false).await?)?;
        if tokio::fs::canonicalize(root.trim_end_matches('\n')).await.map_err(|_| "workspace_changed")? != Path::new(&project.path) { return Err("git_workspace_not_root".into()); }
        Ok(project)
    }
    pub async fn read_development_git_commit(&self, owner: &str, input: GitCommitInput, lease: CancellationToken) -> Result<GitCommitDetails, String> {
        let _admission = self.inner.admission.lock().await;
        let project = self.commit_project(owner, &input, &lease).await?;
        let path = Path::new(&project.path);
        let mut result = metadata(path, &input.oid, &lease, &self.inner.stop).await?;
        let changed = changes(path, &result.oid, result.parents.first().map(String::as_str), &lease, &self.inner.stop).await?;
        result.truncated = changed.len() > COMMIT_FILES;
        result.files = changed.into_iter().take(COMMIT_FILES).map(|v| v.file).collect();
        crate::projects::validate_directory(&project).await?; check_lease(&lease)?;
        Ok(result)
    }
    pub async fn read_development_git_commit_diff(&self, owner: &str, input: GitCommitDiffInput, lease: CancellationToken) -> Result<FileDiff, String> {
        let _admission = self.inner.admission.lock().await;
        let project = self.commit_project(owner, &GitCommitInput { project_id: input.project_id, task_id: input.task_id, oid: input.oid.clone() }, &lease).await?;
        files::relative(&input.path, false)?;
        if files::sensitive(&input.path) { return Err("file_sensitive".into()); }
        let path = Path::new(&project.path); let stop = &self.inner.stop;
        let commit = metadata(path, &input.oid, &lease, stop).await?;
        let changed = changes(path, &commit.oid, commit.parents.first().map(String::as_str), &lease, stop).await?;
        let change = changed.into_iter().find(|v| v.file.path == input.path).ok_or("file_not_found")?;
        let original = change.file.original_path.as_deref().unwrap_or(&input.path);
        if files::sensitive(original) { return Err("file_sensitive".into()); }
        let content = async {
            let before = historical_blob(path, &change.before_mode, &change.before_oid, &lease, stop).await?;
            let after = historical_blob(path, &change.after_mode, &change.after_oid, &lease, stop).await?;
            let old = if before.is_some() { format!("a/{original}") } else { "/dev/null".into() };
            let new = if after.is_some() { format!("b/{}", input.path) } else { "/dev/null".into() };
            let diff = similar::TextDiff::from_lines(before.as_deref().unwrap_or(""), after.as_deref().unwrap_or("")).unified_diff().context_radius(3).header(&old, &new).to_string();
            if diff.len() > files::FILE_LIMIT { return Err("file_too_large".into()); }
            Ok::<_, String>((before, after, diff))
        }.await;
        crate::projects::validate_directory(&project).await?; check_lease(&lease)?;
        match content {
            Ok((before, after, diff)) => Ok(FileDiff { path: input.path, before, after, diff, diff_hash: None, reason: None }),
            Err(reason) if matches!(reason.as_str(), "file_binary" | "file_too_large" | "git_special_entry") => Ok(FileDiff { path: input.path, before: None, after: None, diff: String::new(), diff_hash: None, reason: Some(reason) }),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;
    #[test]
    fn nul_rename_parser_rejects_malformed_and_keeps_literal_paths() {
        let a = "1".repeat(40); let b = "2".repeat(40);
        let raw = format!(":100644 100644 {a} {b} R075\0old name.txt\0new name.txt\0");
        let mut parsed = raw_changes(raw.as_bytes()).unwrap();
        apply_stats(&mut parsed, b"1\t2\t\0old name.txt\0new name.txt\0").unwrap();
        assert_eq!(parsed[0].file.path, "new name.txt"); assert_eq!(parsed[0].file.deleted, Some(2));
        assert!(raw_changes(raw.trim_end_matches('\0').as_bytes()).is_err());
        assert!(apply_stats(&mut parsed, b"1\t2\twrong.txt\0").is_err());
    }
}
