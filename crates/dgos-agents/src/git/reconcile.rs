use super::*;

impl AgentManager {
    /// Reads evidence only. Unknown operations are never retried, including push.
    pub async fn reconcile_development_git(&self, owner: &str, input: ReconcileGitInput, lease: CancellationToken) -> Result<Value, String> {
        let _admission = self.inner.admission.lock().await;
        self.ready().await?; check_lease(&lease)?;
        uuid::Uuid::parse_str(&input.id).map_err(|_| "invalid_request_id")?;
        let receipt = self.inner.repository.git_operation(owner, &input.id).await?.ok_or("operation_not_found")?;
        if !matches!(receipt["status"].as_str(), Some("unknown" | "pending")) { return Ok(receipt); }
        if receipt["kind"].as_str().is_some_and(|kind| kind.starts_with("workspace_")) {
            return self.reconcile_workspace_operation(owner, &input.id, &lease).await;
        }
        let mut details = receipt["details"].clone();
        let request: ExecuteGitInput = serde_json::from_value(details["request"].clone()).map_err(|_| "invalid_git_operation")?;
        let project = self.task_project(owner, &request.task_id).await?;
        let before: GitReview = serde_json::from_value(details["before"].clone()).map_err(|_| "invalid_git_operation")?;
        self.workspace_available(&project.path, None).await?;
        self.host_workspace_available(&project.path).await?;
        if let Some(common) = &before.common_dir { self.repository_git_available(&project.path, common).await?; }
        let after = self.git_snapshot(owner, &request.task_id, &lease).await?;
        if before.path != after.path || before.common_dir != after.common_dir || before.common_identity != after.common_identity { return Err("workspace_changed".into()); }
        let mut proven = false;
        let mut evidence = json!({"review":after});
        match &request.action {
            GitAction::Stage { .. } | GitAction::Unstage { .. } => {
                let expected: Vec<GitIndexEntry> = serde_json::from_value(details["expected_index"].clone()).map_err(|_| "invalid_git_operation")?;
                proven = after.head == before.head && sorted(after.index.clone()) == expected && after.file_versions == before.file_versions && after.merge_heads == before.merge_heads;
            },
            GitAction::Untrack { paths } => {
                let expected: Vec<GitIndexEntry> = serde_json::from_value(details["expected_index"].clone()).map_err(|_| "invalid_git_operation")?;
                proven = after.head == before.head && after.branch == before.branch && after.merge_heads == before.merge_heads && sorted(after.index.clone()) == expected && restore::untrack_matches(&project, paths, &before, &after, &details).unwrap_or(false);
            },
            GitAction::Restore { .. } | GitAction::RestoreHunk { .. } => {
                proven = restore::matches_plan(&project, &before, &after, &details).unwrap_or(false);
            },
            GitAction::Commit { message, .. } => { proven = self.commit_matches(&project, &after, &details, message, &lease).await?; },
            GitAction::Push { .. } => {
                if !input.trust_directory || !input.trust_network { return Err("git_push_trust_required".into()); }
                let url = details["remote_url"].as_str().ok_or("invalid_git_operation")?; valid_url(url)?;
                let reference = details["expected_remote_ref"].as_str().ok_or("invalid_git_operation")?;
                let branch = reference.strip_prefix("refs/heads/").ok_or("invalid_git_operation")?; valid_branch(branch)?;
                let bytes = output(Path::new(&project.path), &["ls-remote", "--refs", "--", url, reference], &lease, &self.inner.stop, false).await?;
                let listing = text(bytes)?;
                let lines: Vec<_> = listing.lines().collect();
                if lines.len() == 1 {
                    if let Some((oid, actual_ref)) = lines[0].split_once('\t') { proven = valid_oid(oid) && actual_ref == reference && details["expected_remote_oid"].as_str() == Some(oid); }
                }
                // Persist only the observed ref, never credentials or transport URLs.
                evidence["remote_ref"] = json!(reference);
                evidence["remote_matches"] = json!(proven);
            },
            GitAction::Pull { .. } => {
                let target = details["target_oid"].as_str();
                let owned = details["owned_ref_oid"].as_str();
                let fetched = matches!(details["phase"].as_str(), Some("ff_started" | "completed"));
                let reference = format!("refs/dgos-pull/{}", request.id);
                let local = optional_ref(Path::new(&project.path), &["rev-parse", "--verify", &reference], &lease, &self.inner.stop).await?;
                let expected_head = if details["result"] == "already_up_to_date" { before.head.as_deref() } else { target };
                if fetched && target.is_some_and(valid_oid) && owned.is_some_and(valid_oid) && local.as_deref() == owned && after.branch == before.branch && after.head.as_deref() == expected_head && after.files.is_empty() && after.conflicts.is_empty() && after.merge_heads.is_empty() {
                    let tree = head_tree(Path::new(&project.path), after.head.as_deref().ok_or("invalid_git_operation")?, &lease, &self.inner.stop).await?;
                    proven = sorted(after.index.clone()) == sorted(tree);
                }
                evidence["phase"] = details["phase"].clone();
                evidence["target_oid"] = json!(target);
            },
            GitAction::Switch { branch } => {
                proven = after.branch.as_deref() == Some(branch.as_str()) && after.head.as_deref() == details["target_oid"].as_str() && after.files.is_empty() && after.merge_heads.is_empty() && after.conflicts.is_empty();
            },
            GitAction::Merge { .. } => {
                // --no-commit leaves an identifiable merge state. No rewrite/abort.
                let target = details["target_oid"].as_str().ok_or("invalid_git_operation")?;
                if after.head == before.head && after.merge_heads.iter().any(|head| head == target) {
                    evidence["merge_in_progress"] = json!(true);
                    evidence["conflicts_preserved"] = json!(!after.conflicts.is_empty());
                    // Conflicted merge is a known failed action with retained files.
                    let status = if after.conflicts.is_empty() { "succeeded" } else { "failed" };
                    details["reconciliation"] = evidence;
                    details["after"] = json!(after);
                    self.inner.repository.finish_git_operation(owner, &input.id, status, &details).await?;
                    return self.inner.repository.git_operation(owner, &input.id).await?.ok_or_else(|| "storage_failed".into());
                }
                // Already-up-to-date is provable by local commit reachability.
                if after.head == before.head && after.files.is_empty() && after.merge_heads.is_empty() {
                    if let Some(head) = &after.head {
                        match output(Path::new(&project.path), &["merge-base", "--is-ancestor", target, head], &lease, &self.inner.stop, false).await { Ok(_) => proven = true, Err(error) if error == "git_failed" => {}, Err(error) => return Err(error) }
                    }
                }
            },
            GitAction::Discard { files: selected, baseline_version, .. } => {
                let baseline = self.inner.repository.review_baseline(owner, &request.task_id).await?.ok_or("baseline_missing")?;
                if baseline["fingerprint"].as_str() != Some(baseline_version.as_str()) { return Err("baseline_changed".into()); }
                let root = files::open_root(&project)?;
                let mut outcomes = Vec::new();
                for file in selected {
                    let expected = baseline["files"][&file.path]["hash"].as_str();
                    let restored = files::read_at(&root, &file.path).ok().is_some_and(|v| v.reason.is_none() && v.hash.as_deref() == expected && expected.is_some());
                    outcomes.push(json!({"path":file.path,"status":if restored { "succeeded" } else { "unknown" }}));
                }
                proven = outcomes.iter().all(|v| v["status"] == "succeeded") && after.index == before.index;
                evidence["files"] = json!(outcomes);
            },
        }
        details["reconciliation"] = evidence;
        details["after"] = json!(after);
        self.inner.repository.finish_git_operation(owner, &input.id, if proven { "succeeded" } else { "unknown" }, &details).await?;
        if proven && matches!(request.action, GitAction::Pull { .. }) {
            operations::cleanup_pull_ref(Path::new(&project.path), &request.id, &details, &self.inner.stop).await;
        }
        self.inner.repository.git_operation(owner, &input.id).await?.ok_or_else(|| "storage_failed".into())
    }
}
