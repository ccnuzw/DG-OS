use super::*;

impl AgentManager {
    pub(super) async fn prepare_git_action(&self, owner: &str, project: &Project, input: &ExecuteGitInput, before: &GitReview, details: &mut Value, lease: &CancellationToken) -> Result<(), String> {
        let path = Path::new(&project.path);
        match &input.action {
            GitAction::Stage { paths } => {
                validate_paths(project, paths, &before.index)?;
                let mut expected = before.index.clone();
                for selected in paths {
                    expected.retain(|entry| entry.path != *selected);
                    let metadata = match std::fs::symlink_metadata(path.join(selected)) { Ok(m) => m, Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue, Err(_) => return Err("file_access_denied".into()) };
                    let oid = text(output(path, &["hash-object", &format!("--path={selected}"), "--", selected], lease, &self.inner.stop, false).await?)?.trim().to_string();
                    if !valid_oid(&oid) { return Err("git_invalid_output".into()); }
                    let mut mode = "100644";
                    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; if metadata.permissions().mode() & 0o111 != 0 { mode = "100755"; } }
                    let filemode = optional_ref(path, &["config", "--get", "core.filemode"], lease, &self.inner.stop).await?;
                    if filemode.as_deref() == Some("false") { mode = before.index.iter().find(|e| e.path == *selected && e.stage == 0).map(|e| e.mode.as_str()).unwrap_or("100644"); }
                    expected.push(GitIndexEntry { path: selected.clone(), mode: mode.into(), oid, stage: 0 });
                }
                details["expected_index"] = json!(sorted(expected));
            },
            GitAction::Unstage { paths } => {
                validate_paths(project, paths, &before.index)?;
                let tree = if let Some(head) = &before.head { head_tree(path, head, lease, &self.inner.stop).await? } else { vec![] };
                let mut expected: Vec<_> = before.index.iter().filter(|e| !paths.contains(&e.path)).cloned().collect();
                expected.extend(tree.into_iter().filter(|e| paths.contains(&e.path)));
                details["expected_index"] = json!(sorted(expected));
            },
            GitAction::Commit { message, confirm_full_index, confirm_preexisting_staged } => {
                if !confirm_full_index { return Err("git_full_index_confirmation_required".into()); }
                if details["initial_index"]["staged_paths"].as_array().is_some_and(|a| !a.is_empty()) && !confirm_preexisting_staged { return Err("git_preexisting_staged_confirmation_required".into()); }
                if message.trim().is_empty() || message.len() > 16 * 1024 || message.chars().any(|c| c == '\0' || (c.is_control() && c != '\n' && c != '\t')) { return Err("invalid_commit_message".into()); }
                if !before.conflicts.is_empty() { return Err("git_unmerged".into()); }
                if before.staged_diff.is_empty() && before.merge_heads.is_empty() { return Err("git_empty_commit".into()); }
                details["expected_index"] = json!(sorted(before.index.clone()));
            },
            GitAction::Push { remote, trust_directory, trust_network } => {
                if !trust_directory || !trust_network { return Err("git_push_trust_required".into()); }
                valid_remote(remote)?;
                let branch = before.branch.as_deref().ok_or("git_detached_head")?; valid_branch(branch)?;
                let head = before.head.as_deref().ok_or("git_unborn_head")?;
                let urls = text(output(path, &["remote", "get-url", "--push", "--all", remote], lease, &self.inner.stop, false).await?)?;
                let urls: Vec<_> = urls.lines().collect();
                if urls.len() != 1 { return Err("unsafe_git_remote".into()); }
                valid_url(urls[0])?;
                details["remote_url"] = json!(urls[0]);
                details["expected_remote_ref"] = json!(format!("refs/heads/{branch}"));
                details["expected_remote_oid"] = json!(head);
            },
            GitAction::Pull { remote, branch, preview_token, trust_directory, trust_network } => {
                if !trust_directory || !trust_network { return Err("git_pull_trust_required".into()); }
                valid_remote(remote)?; valid_branch(branch)?;
                if before.kind != "git" { return Err("not_git_repository".into()); }
                if before.branch.is_none() { return Err("git_detached_head".into()); }
                if before.head.is_none() { return Err("git_unborn_head".into()); }
                if !before.files.is_empty() { return Err("git_worktree_dirty".into()); }
                if !before.conflicts.is_empty() || !before.merge_heads.is_empty() { return Err("git_unmerged".into()); }
                let urls = text(output(path, &["remote", "get-url", "--all", remote], lease, &self.inner.stop, false).await?)?;
                let urls: Vec<_> = urls.lines().collect();
                if urls.len() != 1 { return Err("unsafe_git_remote".into()); }
                valid_url(urls[0])?;
                let previews = PULL_PREVIEWS.lock().map_err(|_| "storage_failed")?;
                let preview = previews.get(preview_token).ok_or("git_pull_preview_stale")?;
                if preview.owner != owner || preview.task != input.task_id || preview.review != before.review_token || preview.remote != *remote || preview.branch != *branch || preview.url_hash != digest(urls[0].as_bytes()) || preview.expires <= std::time::Instant::now() || preview.lease.is_cancelled() { return Err("git_pull_preview_stale".into()); }
                details["remote_url_hash"] = json!(digest(urls[0].as_bytes()));
                details["remote_ref"] = json!(format!("refs/heads/{branch}"));
                details["fetch_refspec"] = json!(format!("refs/heads/{branch}"));
            },
            GitAction::Switch { branch } | GitAction::Merge { branch } => {
                valid_branch(branch)?;
                if !before.files.is_empty() || !before.conflicts.is_empty() || !before.merge_heads.is_empty() { return Err("git_worktree_dirty".into()); }
                let oid = text(output(path, &["rev-parse", "--verify", &format!("refs/heads/{branch}^{{commit}}")], lease, &self.inner.stop, false).await?)?.trim().to_string();
                if !valid_oid(&oid) { return Err("invalid_git_ref".into()); }
                details["target_oid"] = json!(oid);
            },
            GitAction::Discard { baseline_version, files: selected, confirmed } => {
                if !confirmed { return Err("discard_confirmation_required".into()); }
                let paths: Vec<_> = selected.iter().map(|f| f.path.clone()).collect();
                validate_paths(project, &paths, &before.index)?;
                let baseline = self.inner.repository.review_baseline(owner, &input.task_id).await?.ok_or("baseline_missing")?;
                if baseline["fingerprint"].as_str() != Some(baseline_version.as_str()) { return Err("baseline_changed".into()); }
                if baseline["truncated"].as_bool() != Some(false) { return Err("baseline_incomplete".into()); }
                for file in selected { self.validate_owned_restore(owner, &input.task_id, project, file, &baseline, before).await?; }
            },
        }
        Ok(())
    }

    async fn validate_owned_restore(&self, owner: &str, task_id: &str, project: &Project, file: &DiscardFile, baseline: &Value, before: &GitReview) -> Result<String, String> {
        let original = &baseline["files"][&file.path];
        let original_text = original["text"].as_str().ok_or("baseline_incomplete")?;
        let original_hash = original["hash"].as_str().ok_or("baseline_incomplete")?;
        if digest(original_text.as_bytes()) != original_hash || !original["reason"].is_null() { return Err("baseline_incomplete".into()); }
        if baseline["initial_changes"].as_array().is_some_and(|items| items.iter().any(|v| v.as_str().is_some_and(|s| s.get(3..).is_some_and(|p| p == file.path || (p.ends_with('/') && file.path.starts_with(p)))))) { return Err("discard_preexisting_changes".into()); }
        if before.files.iter().any(|status| status.path == file.path && status.index != " ") || before.index.iter().any(|entry| entry.path == file.path && entry.stage != 0) { return Err("discard_index_changed".into()); }
        if self.initial_git_index(owner, task_id).await?.as_ref().and_then(|v| v["staged_paths"].as_array()).is_some_and(|items| items.iter().any(|p| p.as_str() == Some(file.path.as_str()))) { return Err("discard_preexisting_changes".into()); }
        let ownership = self.inner.repository.host_file_ownership(owner, task_id, &file.path).await?.ok_or("discard_ownership_uncertain")?;
        if ownership["valid"].as_bool() != Some(true) || ownership["after_hash"].as_str() != Some(file.expected_hash.as_str()) { return Err("discard_ownership_uncertain".into()); }
        let observation = self.inner.repository.review_observation(owner, task_id).await?;
        if observation.as_ref().and_then(|v| v["external"].as_array()).is_some_and(|items| items.iter().any(|p| p.as_str() == Some(file.path.as_str()))) { return Err("discard_external_change".into()); }
        let current = files::read_at(&files::open_root(project)?, &file.path)?;
        if current.reason.is_some() || current.hash.as_deref() != Some(file.expected_hash.as_str()) { return Err("file_changed".into()); }
        Ok(original_text.into())
    }

    pub(super) async fn run_git_action(&self, owner: &str, project: &Project, input: &ExecuteGitInput, details: &mut Value, lease: &CancellationToken) -> Result<(), String> {
        check_lease(lease)?;
        let path = Path::new(&project.path);
        let before: GitReview = serde_json::from_value(details["before"].clone()).map_err(|_| "storage_failed")?;
        let mut checked = details.clone();
        self.prepare_git_action(owner, project, input, &before, &mut checked, lease).await?;
        if checked != *details { return Err("git_review_stale".into()); }
        let bytes = match &input.action {
            GitAction::Stage { paths } => {
                validate_paths(project, paths, &serde_json::from_value::<GitReview>(details["before"].clone()).map_err(|_| "storage_failed")?.index)?;
                let mut command = safe_command(path).await?; command.args(["add", "--"]).args(paths);
                complete(git_output(command, Duration::from_secs(120), lease, &self.inner.stop).await?)?
            },
            GitAction::Unstage { paths } => {
                let mut command = safe_command(path).await?;
                if details["before"]["head"].is_string() { command.args(["reset", "--quiet", "HEAD", "--"]); }
                else { command.args(["update-index", "--force-remove", "--"]); }
                command.args(paths);
                complete(git_output(command, Duration::from_secs(120), lease, &self.inner.stop).await?)?
            },
            GitAction::Commit { message, .. } => output(path, &["-c", "commit.cleanup=verbatim", "commit", "--no-verify", "--no-gpg-sign", "--no-status", "-m", message], lease, &self.inner.stop, true).await?,
            GitAction::Push { .. } => {
                let url = details["remote_url"].as_str().ok_or("storage_failed")?;
                valid_url(url)?;
                let branch_ref = details["expected_remote_ref"].as_str().ok_or("storage_failed")?;
                let refspec = format!("{branch_ref}:{branch_ref}");
                output(path, &["push", "--porcelain", "--no-verify", "--recurse-submodules=no", "--", url, &refspec], lease, &self.inner.stop, true).await?
            },
            GitAction::Pull { remote, .. } => {
                let urls = text(output(path, &["remote", "get-url", "--all", remote], lease, &self.inner.stop, false).await?)?;
                let urls: Vec<_> = urls.lines().collect();
                if urls.len() != 1 || details["remote_url_hash"].as_str() != Some(digest(urls[0].as_bytes()).as_str()) { return Err("git_pull_preview_stale".into()); }
                let url = urls[0]; valid_url(url)?;
                let remote_ref = details["fetch_refspec"].as_str().ok_or("storage_failed")?.to_owned();
                let reference = format!("refs/dgos-pull/{}", input.id);
                let reservation = pull_anchor(path, before.head.as_deref().ok_or("git_unborn_head")?, lease, &self.inner.stop).await?;
                let absent = "0".repeat(reservation.len());
                output(path, &["update-ref", "--no-deref", &reference, &reservation, &absent], lease, &self.inner.stop, true).await.map_err(|error| if error == "git_failed" { "git_pull_ref_conflict".into() } else { error })?;
                details["owned_ref_oid"] = json!(reservation);
                details["phase"] = json!("reserved");
                self.inner.repository.update_git_operation_details(owner, &input.id, details).await?;
                #[cfg(test)] pull_reserved_barrier(&input.id).await;
                let listing = text(output(path, &["ls-remote", "--refs", "--", url, &remote_ref], lease, &self.inner.stop, false).await?)?;
                let lines: Vec<_> = listing.lines().collect();
                let (target, advertised_ref) = if lines.len() == 1 { lines[0].split_once('\t').ok_or("git_invalid_output")? } else { return Err("git_invalid_output".into()); };
                if !valid_oid(target) || advertised_ref != remote_ref { return Err("git_invalid_output".into()); }
                let target = target.to_owned();
                details["phase"] = json!("fetch_started");
                self.inner.repository.update_git_operation_details(owner, &input.id, details).await?;
                // Fetch objects only: Git must never force-update a caller-named ref.
                // Pin the advertised OID so a moving remote cannot change our target.
                let mut command = safe_command(path).await?;
                command.args(["-c", "fetch.fsckObjects=true", "fetch", "--refmap=", "--no-tags", "--no-recurse-submodules", "--no-auto-maintenance", "--no-write-fetch-head", "--", url, &target]);
                complete(git_output(command, Duration::from_secs(120), lease, &self.inner.stop).await?)?;
                #[cfg(test)] pull_install_barrier(&input.id).await;
                // A unique commit both proves ownership and roots the fetched commit
                // until a terminal receipt (or successful reconciliation) is durable.
                let anchor = pull_anchor(path, &target, lease, &self.inner.stop).await?;
                output(path, &["update-ref", "--no-deref", &reference, &anchor, &reservation], lease, &self.inner.stop, true).await.map_err(|error| if error == "git_failed" { "git_unknown".into() } else { error })?;
                details["owned_ref_oid"] = json!(anchor);
                details["target_oid"] = json!(target);
                details["phase"] = json!("fetched");
                self.inner.repository.update_git_operation_details(owner, &input.id, details).await?;
                #[cfg(test)] pull_fetch_barrier(&input.id).await;
                check_lease(lease)?;
                let after_fetch = self.git_snapshot(owner, &input.task_id, lease).await.map_err(|_| "git_unknown")?;
                if !same_version(&before, &after_fetch) { return Err("git_unknown".into()); }
                let mut checked = details.clone();
                self.prepare_git_action(owner, project, input, &after_fetch, &mut checked, lease).await?;
                if checked != *details { return Err("git_unknown".into()); }
                let head = before.head.as_deref().ok_or("git_unborn_head")?;
                let already = output(path, &["merge-base", "--is-ancestor", &target, head], lease, &self.inner.stop, false).await;
                match already {
                    Ok(_) => { details["result"] = json!("already_up_to_date"); },
                    Err(error) if error == "git_failed" => {
                        match output(path, &["merge-base", "--is-ancestor", head, &target], lease, &self.inner.stop, false).await {
                            Ok(_) => {},
                            Err(error) if error == "git_failed" => return Err("git_pull_diverged".into()),
                            Err(error) => return Err(error),
                        }
                        let checked = self.git_snapshot(owner, &input.task_id, lease).await?;
                        if !same_version(&before, &checked) { return Err("git_unknown".into()); }
                        details["phase"] = json!("ff_started");
                        self.inner.repository.update_git_operation_details(owner, &input.id, details).await?;
                        output(path, &["-c", "merge.autoStash=false", "merge", "--ff-only", "--no-edit", "--no-stat", "--no-progress", "--", &target], lease, &self.inner.stop, true).await?;
                        details["result"] = json!("fast_forwarded");
                    },
                    Err(error) => return Err(error),
                }
                let after = self.git_snapshot(owner, &input.task_id, &CancellationToken::new()).await.map_err(|_| "git_unknown")?;
                let expected_head = if details["result"] == "already_up_to_date" { before.head.as_deref() } else { Some(target.as_str()) };
                if after.head.as_deref() != expected_head || after.branch != before.branch || !after.files.is_empty() || !after.conflicts.is_empty() || !after.merge_heads.is_empty() || after.common_identity != before.common_identity { return Err("git_unknown".into()); }
                details["phase"] = json!("completed");
                details["after"] = json!(after);
                self.inner.repository.update_git_operation_details(owner, &input.id, details).await?;
                vec![]
            },
            GitAction::Switch { branch } => output(path, &["switch", "--no-guess", "--", branch], lease, &self.inner.stop, true).await?,
            GitAction::Merge { branch } => output(path, &["-c", "merge.autoStash=false", "merge", "--no-commit", "--no-ff", "--no-edit", "--no-stat", "--no-progress", "--", &format!("refs/heads/{branch}")], lease, &self.inner.stop, true).await?,
            GitAction::Discard { files: selected, .. } => {
                let baseline = self.inner.repository.review_baseline(owner, &input.task_id).await?.ok_or("baseline_missing")?;
                let mut outcomes = Vec::new();
                let mut failed = false;
                for file in selected {
                    let before: GitReview = serde_json::from_value(details["before"].clone()).map_err(|_| "storage_failed")?;
                    let result = async {
                        check_lease(lease)?;
                        let text = self.validate_owned_restore(owner, &input.task_id, project, file, &baseline, &before).await?;
                        let project = project.clone(); let lease = lease.clone();
                        let save = files::SaveFileInput { project_id: project.id.clone(), task_id: input.task_id.clone(), path: file.path.clone(), expected_hash: file.expected_hash.clone(), text };
                        tokio::task::spawn_blocking(move || files::save_at(&project, save, &lease)).await.map_err(|_| "file_save_unknown".to_string())??;
                        Ok::<(), String>(())
                    }.await;
                    match result { Ok(()) => outcomes.push(json!({"path":file.path,"status":"succeeded"})), Err(error) => { failed = true; outcomes.push(json!({"path":file.path,"status":if error == "file_save_unknown" { "unknown" } else { "failed" },"error":error})); } }
                    details["files"] = json!(outcomes);
                    // Each file is a separate CAS replacement; record partial completion.
                    self.inner.repository.update_git_operation_details(owner, &input.id, details).await?;
                }
                if failed { return Err(if outcomes.iter().any(|o| o["status"] == "unknown") { "git_unknown" } else { "discard_partial_failure" }.into()); }
                vec![]
            },
        };
        details["output"] = json!(crate::executions::sanitize_text(&String::from_utf8_lossy(&bytes)));
        let after = self.git_snapshot(owner, &input.task_id, &CancellationToken::new()).await.map_err(|_| "git_unknown")?;
        match &input.action {
            GitAction::Stage { .. } | GitAction::Unstage { .. } => {
                let expected: Vec<GitIndexEntry> = serde_json::from_value(details["expected_index"].clone()).map_err(|_| "storage_failed")?;
                if sorted(after.index.clone()) != expected || after.head != details["before"]["head"].as_str().map(str::to_owned) { return Err("git_unknown".into()); }
            },
            GitAction::Commit { message, .. } => {
                if !self.commit_matches(project, &after, details, message, lease).await? { return Err("git_unknown".into()); }
            },
            GitAction::Switch { branch } if after.branch.as_deref() != Some(branch.as_str()) || after.head.as_deref() != details["target_oid"].as_str() => {
                return Err("git_unknown".into());
            },
            _ => {},
        }
        Ok(())
    }

    pub(super) async fn commit_matches(&self, project: &Project, after: &GitReview, details: &Value, message: &str, lease: &CancellationToken) -> Result<bool, String> {
        let Some(head) = &after.head else { return Ok(false); };
        let expected: Vec<GitIndexEntry> = serde_json::from_value(details["expected_index"].clone()).map_err(|_| "storage_failed")?;
        if sorted(head_tree(Path::new(&project.path), head, lease, &self.inner.stop).await?) != expected || sorted(after.index.clone()) != expected { return Ok(false); }
        let commit = text(output(Path::new(&project.path), &["cat-file", "commit", head], lease, &self.inner.stop, false).await?)?;
        let Some((headers, actual_message)) = commit.split_once("\n\n") else { return Ok(false); };
        let parents: Vec<_> = headers.lines().filter_map(|s| s.strip_prefix("parent ")).collect();
        let mut expected_parents = Vec::new();
        if let Some(parent) = details["before"]["head"].as_str() { expected_parents.push(parent); }
        if let Some(parents) = details["before"]["merge_heads"].as_array() { for p in parents { if let Some(p) = p.as_str() { expected_parents.push(p); } } }
        // Verbatim cleanup preserves supplied blank lines and adds a missing LF.
        let expected_message = if message.ends_with('\n') { message.to_owned() } else { format!("{message}\n") };
        Ok(parents == expected_parents && actual_message == expected_message)
    }
}

async fn pull_anchor(path: &Path, target: &str, lease: &CancellationToken, stop: &CancellationToken) -> Result<String, String> {
    let tree = text(output(path, &["rev-parse", "--verify", &format!("{target}^{{tree}}")], lease, stop, false).await?)?.trim().to_owned();
    if !valid_oid(&tree) { return Err("git_invalid_output".into()); }
    let mut command = safe_command(path).await?;
    command.args(["commit-tree", &tree, "-p", target, "-m", &format!("DGOS pull ownership {}", uuid::Uuid::new_v4())])
        .env("GIT_AUTHOR_NAME", "DGOS").env("GIT_AUTHOR_EMAIL", "dgos@localhost")
        .env("GIT_COMMITTER_NAME", "DGOS").env("GIT_COMMITTER_EMAIL", "dgos@localhost");
    let oid = text(complete(git_output(command, Duration::from_secs(10), lease, stop).await?)?)?.trim().to_owned();
    if !valid_oid(&oid) { return Err("git_invalid_output".into()); }
    Ok(oid)
}

pub(super) async fn cleanup_pull_ref(path: &Path, id: &str, details: &Value, stop: &CancellationToken) {
    // Never infer ownership from target_oid: legacy receipts and rejected
    // reservations do not own refs, even when their target happens to match.
    if let Some(owned) = details["owned_ref_oid"].as_str().filter(|oid| valid_oid(oid)) {
        let reference = format!("refs/dgos-pull/{id}");
        let _ = output(path, &["update-ref", "--no-deref", "-d", &reference, owned], &CancellationToken::new(), stop, true).await;
    }
}
