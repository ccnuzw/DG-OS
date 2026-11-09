//! Task review is a comparison of disk snapshots, never an assertion of authorship.
use crate::{files::{self, FileView}, AgentManager};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, BTreeSet}, time::{SystemTime, UNIX_EPOCH}};

const TEXT_BUDGET: usize = 8 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
struct Baseline { captured_at: i64, baseline_late: bool, truncated: bool, files: BTreeMap<String, FileView>, initial_changes: Vec<String>, fingerprint: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffInput { pub task_id: String, pub path: String, #[serde(default)] pub source: DiffSource }
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiffSource { #[default] Task, Index, Worktree }
#[derive(Serialize)]
pub struct ReviewChange { pub path: String, pub kind: String, pub pre_existing: bool, pub origin: String, pub previewable: bool }
#[derive(Serialize)]
pub struct TaskReview { pub task_id: String, pub captured_at: Option<i64>, pub baseline_late: bool, pub truncated: bool, pub fingerprint: String, pub changes: Vec<ReviewChange>, pub excluded: Vec<ReviewExclusion>, pub runs_stale: bool }
#[derive(Serialize)]
pub struct FileDiff { pub path: String, pub before: Option<String>, pub after: Option<String>, pub diff: String, pub reason: Option<String> }
#[derive(Serialize)]
pub struct ReviewExclusion { pub path: String, pub reason: String }
fn now() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 }
fn same(a: Option<&FileView>, b: Option<&FileView>) -> bool {
    match (a, b) { (Some(a), Some(b)) => a.hash == b.hash && a.size == b.size && a.reason == b.reason, (None, None) => true, _ => false }
}
fn fingerprint(files: &BTreeMap<String, FileView>) -> String {
    let mut hash = Sha256::new();
    for (path, file) in files { hash.update(path.as_bytes()); hash.update([0]); hash.update(file.hash.as_deref().unwrap_or("unavailable").as_bytes()); hash.update(file.size.to_le_bytes()); hash.update([0]); }
    format!("{:x}", hash.finalize())
}
async fn snapshot(project: dgos_db::development_repository::Project) -> Result<Baseline, String> {
    tokio::task::spawn_blocking(move || {
        let list = files::scan(&project, "", true, false)?;
        let root = files::open_root(&project)?;
        let mut files = BTreeMap::new();
        let mut truncated = list.truncated;
        let mut bytes = 0;
        let mut encoded_bytes = 0;
        let mut metadata_bytes = 0;
        for entry in list.entries.into_iter().filter(|e| e.kind != "directory") {
            // Covers duplicated JSON paths, escaping, hashes and an external-path set.
            metadata_bytes += entry.path.len().saturating_mul(12) + 384;
            if metadata_bytes > 384 * 1024 { truncated = true; break; }
            let mut file = if bytes >= TEXT_BUDGET {
                truncated = true;
                FileView { path: entry.path.clone(), text: None, reason: Some("scan_limit".into()), hash: None, size: entry.size }
            } else { match files::read_at(&root, &entry.path) {
                Ok(file) => file,
                Err(error) => FileView { path: entry.path.clone(), text: None, reason: Some(error), hash: None, size: entry.size },
            } };
            if let Some(text) = file.text.take() {
                let text = crate::executions::sanitize_text(&text);
                let encoded = serde_json::to_string(&text).map_err(|_| "storage_failed")?.len();
                if bytes + text.len() <= TEXT_BUDGET && encoded_bytes + encoded <= TEXT_BUDGET {
                    bytes += text.len(); encoded_bytes += encoded; file.text = Some(text);
                } else { file.reason = Some("scan_limit".into()); truncated = true; }
            }
            if file.reason.is_some() { truncated = true; }
            files.insert(entry.path, file);
        }
        let fingerprint = fingerprint(&files);
        Ok(Baseline { captured_at: now(), baseline_late: false, truncated, files, initial_changes: vec![], fingerprint })
    }).await.map_err(|_| "file_access_denied".to_string())?
}
impl AgentManager {
    async fn review_snapshot(&self, owner: &str, task_id: &str) -> Result<Baseline, String> {
        let project = self.task_project(owner, task_id).await?;
        snapshot(project).await
    }
    /// Called under manager admission before the first native send, never recaptures.
    pub(crate) async fn capture_task_baseline(&self, owner: &str, task_id: &str) -> Result<String, String> {
        self.ensure_task_workspace(owner, task_id).await?;
        if let Some(value) = self.inner.repository.review_baseline(owner, task_id).await? {
            let baseline: Baseline = serde_json::from_value(value).map_err(|_| "storage_failed")?;
            return Ok(baseline.fingerprint);
        }
        let mut baseline = self.review_snapshot(owner, task_id).await?;
        for record in self.list(owner).await.into_iter().filter(|r| r.task_id.as_deref() == Some(task_id)) {
            if let Ok(view) = self.snapshot(owner, &record.id).await {
                if view.view.events.iter().any(|e| matches!(e.kind.as_str(), "userMessage" | "agentMessage" | "message")) { baseline.baseline_late = true; }
            }
        }
        let inspection = self.inspect_task_project(owner, task_id).await?;
        if inspection.error.is_some() || inspection.truncated { baseline.truncated = true; }
        baseline.initial_changes = inspection.changes;
        let fingerprint = baseline.fingerprint.clone();
        self.inner.repository.save_review_baseline(owner, task_id, &serde_json::to_value(&baseline).map_err(|_| "storage_failed")?).await?;
        let mut observed = baseline.files;
        for file in observed.values_mut() { file.text = None; }
        self.inner.repository.save_review_observation(owner, task_id, &json!({"files":observed,"idle":false,"external":[]})).await?;
        Ok(fingerprint)
    }
    /// Reuses the safe review scanner; incomplete snapshots cannot establish verification.
    pub(crate) async fn verification_snapshot(&self, owner: &str, task_id: &str) -> Result<(String,bool),String> {
        let snapshot=self.review_snapshot(owner,task_id).await?;
        Ok((snapshot.fingerprint,!snapshot.truncated))
    }
    pub(crate) async fn task_fingerprint(&self, owner: &str, task_id: &str) -> Result<String, String> {
        Ok(self.review_snapshot(owner, task_id).await?.fingerprint)
    }
    pub async fn read_development_review(&self, owner: &str, task_id: &str) -> Result<TaskReview, String> {
        self.ready().await?;
        let mut current = self.review_snapshot(owner, task_id).await?;
        let excluded = current.files.iter().filter_map(|(path, file)| file.reason.as_ref().map(|reason| ReviewExclusion { path: path.clone(), reason: reason.clone() })).collect();
        let baseline = self.inner.repository.review_baseline(owner, task_id).await?.map(serde_json::from_value::<Baseline>).transpose().map_err(|_| "storage_failed")?;
        let Some(baseline) = baseline else {
            return Ok(TaskReview { task_id: task_id.into(), captured_at: None, baseline_late: false, truncated: current.truncated, fingerprint: current.fingerprint, changes: vec![], excluded, runs_stale: false });
        };
        let mut idle = true;
        let runs = self.list_development_runs(owner, task_id).await?;
        if runs.iter().any(|r|r.engine=="host" && matches!(r.status.as_str(),"running"|"waiting")) { idle=false; }
        let run_marker = serde_json::to_string(&runs.iter().map(|r| (&r.id, &r.status)).collect::<Vec<_>>()).map_err(|_| "storage_failed")?;
        for record in self.list(owner).await.into_iter().filter(|r| r.task_id.as_deref() == Some(task_id)) {
            if let Ok(view) = self.snapshot(owner, &record.id).await { if matches!(view.view.status.as_str(), "running" | "waiting" | "starting" | "unknown") { idle = false; } }
        }
        let observation = self.inner.repository.review_observation(owner, task_id).await?;
        let mut external: BTreeSet<String> = observation.as_ref().and_then(|o| o.get("external")).cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        if idle && observation.as_ref().and_then(|o| o.get("idle")).and_then(Value::as_bool) == Some(true)
            && observation.as_ref().and_then(|o| o.get("run_marker")).and_then(Value::as_str) == Some(run_marker.as_str()) {
            if let Some(old) = observation.as_ref().and_then(|o| o.get("files")).cloned().and_then(|v| serde_json::from_value::<BTreeMap<String, FileView>>(v).ok()) {
                for path in old.keys().chain(current.files.keys()) { if !same(old.get(path), current.files.get(path)) { external.insert(path.clone()); } }
            }
        }
        let mut external_bytes = 0;
        external.retain(|path| {
            external_bytes += path.len().saturating_mul(6) + 4;
            let keep = external_bytes <= 256 * 1024;
            if !keep { current.truncated = true; }
            keep
        });
        // Observation stores hashes and reasons only; no extra source-content copy.
        let mut observed = current.files.clone();
        for file in observed.values_mut() { file.text = None; }
        self.inner.repository.save_review_observation(owner, task_id, &json!({"files":observed,"idle":idle,"external":external,"run_marker":run_marker})).await?;
        let mut paths: BTreeSet<String> = baseline.files.keys().chain(current.files.keys()).cloned().collect();
        for status in &baseline.initial_changes { if status.len() > 3 { paths.insert(status[3..].to_string()); } }
        let mut changes = Vec::new();
        for path in paths {
            let before = baseline.files.get(&path); let after = current.files.get(&path);
            let pre_existing = baseline.initial_changes.iter().any(|s| s.get(3..).is_some_and(|p| path == p || (p.ends_with('/') && path.starts_with(p))));
            let changed = !same(before, after);
            if !changed && !pre_existing { continue; }
            let kind = if !changed { "pre_existing" } else if before.is_none() { "added" } else if after.is_none() { "deleted" } else { "modified" };
            changes.push(ReviewChange { origin: if external.contains(&path) { "external" } else { "uncertain" }.into(), path, kind: kind.into(), pre_existing, previewable: before.is_none_or(|f| f.text.is_some()) && after.is_none_or(|f| f.text.is_some()) });
        }
        let runs_stale = runs.iter().any(|r| r.fingerprint != current.fingerprint);
        Ok(TaskReview { task_id: task_id.into(), captured_at: Some(baseline.captured_at), baseline_late: baseline.baseline_late, truncated: baseline.truncated || current.truncated, fingerprint: current.fingerprint, changes, excluded, runs_stale })
    }
    pub async fn read_development_diff(&self, owner: &str, input: DiffInput) -> Result<FileDiff, String> {
        files::relative(&input.path, false)?;
        if input.source != DiffSource::Task { return self.read_git_file_diff(owner, input).await; }
        let project = self.task_project(owner, &input.task_id).await?;
        let path = input.path.clone();
        let after = tokio::task::spawn_blocking(move || {
            match files::read_at(&files::open_root(&project)?, &path) {
                Ok(mut view) => { view.text = view.text.map(|text| crate::executions::sanitize_text(&text)); Ok(Some(view)) },
                Err(error) if error == "file_not_found" => Ok(None),
                Err(error) => Err(error),
            }
        }).await.map_err(|_| "file_access_denied".to_string())??;
        let baseline: Baseline = serde_json::from_value(self.inner.repository.review_baseline(owner, &input.task_id).await?.ok_or("baseline_missing")?).map_err(|_| "storage_failed")?;
        let before = baseline.files.get(&input.path);
        let after = after.as_ref();
        if before.is_none() && after.is_none() { return Err("file_not_found".into()); }
        let reason = before.and_then(|f| f.reason.clone()).or_else(|| after.and_then(|f| f.reason.clone())).or_else(|| (baseline.truncated && before.is_none()).then(|| "scan_limit".into()));
        let old = before.and_then(|f| f.text.clone()); let new = after.and_then(|f| f.text.clone());
        let diff = if reason.is_none() { similar::TextDiff::from_lines(old.as_deref().unwrap_or(""), new.as_deref().unwrap_or("")).unified_diff().context_radius(3).header(&format!("baseline/{}", input.path), &format!("current/{}", input.path)).to_string() } else { String::new() };
        Ok(FileDiff { path: input.path, before: old, after: new, diff, reason })
    }
}
