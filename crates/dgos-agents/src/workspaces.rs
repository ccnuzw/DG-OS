//! Task-bound directories and explicit, journaled Git worktree lifecycle.
use crate::{projects::{directory, git_command, git_output, validate_directory}, AgentManager};
use dgos_db::development_repository::{GitOperationIntent, Project, StoredWorkspace};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadWorkspacesInput { pub project_id: String, pub task_id: Option<String> }
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SaveWorkspaceInput {
    pub request_id: String, pub task_id: String, pub action: String,
    pub workspace_id: Option<String>, pub path: Option<String>, pub branch: Option<String>,
    pub base_commit: Option<String>, pub retain_branch: Option<String>,
}
#[derive(Serialize, Debug)]
pub struct WorkspaceView { pub workspaces: Vec<StoredWorkspace>, pub selected_workspace_id: Option<String>, pub operation: Option<Value>, pub operations: Vec<Value> }
fn live(lease:&CancellationToken)->Result<(),String> {if lease.is_cancelled() {Err("cancelled".into())} else {Ok(())}}
fn valid_branch(branch:&str)->Result<(),String> {
    if branch.is_empty() || branch.len()>200 || branch.starts_with(['-','.','/']) || branch.ends_with(['.','/']) || branch.ends_with(".lock") || branch.contains([ '\0','\n','\r']) || branch.contains("..") || branch.contains("//") || branch.split('/').any(|s|s.starts_with('.') || s.ends_with(".lock")) || !branch.bytes().all(|b|b.is_ascii_alphanumeric() || b"-_/ .".contains(&b) && b!=b' ') {return Err("invalid_branch".into());}Ok(())
}
pub(crate) async fn git_identity(path:&str,lease:&CancellationToken,stop:&CancellationToken)->Result<(String,String),String> {
    let mut command=git_command(Path::new(path));command.args(["rev-parse","--path-format=absolute","--git-common-dir"]);
    let (bytes,_)=git_output(command,Duration::from_secs(10),lease,stop).await?;
    let common=std::str::from_utf8(&bytes).map_err(|_|"git_failed")?.trim_end_matches('\n');
    directory(common).await
}
pub(crate) async fn validate_workspace(w:&StoredWorkspace,stop:&CancellationToken)->Result<(),String> {
    let (path,identity)=directory(&w.path).await?;
    if path!=w.path || identity!=w.directory_identity {return Err("workspace_changed".into());}
    if let Some(common)=&w.common_dir {
        let (actual,identity)=git_identity(&w.path,&CancellationToken::new(),stop).await?;
        if &actual!=common || Some(&identity)!=w.common_identity.as_ref() {return Err("workspace_changed".into());}
    }
    Ok(())
}
impl AgentManager {
    pub async fn task_project(&self,owner:&str,task_id:&str)->Result<Project,String> {
        self.ready().await?;
        let task=self.inner.repository.task(owner,task_id).await?;
        let mut project=self.inner.repository.project(owner,&task.project_id).await?;
        if let Some(w)=self.inner.repository.task_workspace(owner,task_id).await? {
            validate_workspace(&w,&self.inner.stop).await?;
            project.path=w.path;project.directory_identity=w.directory_identity;
        } else {validate_directory(&project).await?;}
        Ok(project)
    }
    /// Caller holds admission; pin legacy/original paths before new evidence is stored.
    pub(crate) async fn ensure_task_workspace(&self,owner:&str,task_id:&str)->Result<Project,String> {
        if self.inner.repository.task_workspace(owner,task_id).await?.is_none() {
            let task=self.inner.repository.task(owner,task_id).await?;
            let project=self.inner.repository.project(owner,&task.project_id).await?;
            let original=self.original_workspace(owner,&project).await?;
            if self.inner.repository.workspace_by_id(owner,&original.id).await?.is_none() {self.inner.repository.insert_workspace(&original).await?;}
            self.inner.repository.bind_task_workspace(owner,task_id,&original.id).await?;
        }
        self.task_project(owner,task_id).await
    }
    async fn original_workspace(&self,owner:&str,project:&Project)->Result<StoredWorkspace,String> {
        if let Some(w)=self.inner.repository.workspaces(owner,&project.id).await?.into_iter().find(|w|w.kind=="original") {
            validate_workspace(&w,&self.inner.stop).await?;return Ok(w);
        }
        validate_directory(project).await?;
        let git=git_identity(&project.path,&CancellationToken::new(),&self.inner.stop).await;
        let (common_dir,common_identity)=match git {
            Ok((p,i))=>(Some(p),Some(i)),
            Err(error)=>{
                let mut cursor=Some(Path::new(&project.path));
                while let Some(path)=cursor {if tokio::fs::symlink_metadata(path.join(".git")).await.is_ok() {return Err(error);}cursor=path.parent();}
                if error!="git_failed" && error!="git_unavailable" {return Err(error);} (None,None)
            }
        };
        Ok(StoredWorkspace{id:format!("original-{}",project.id),owner:owner.into(),project_id:project.id.clone(),path:project.path.clone(),directory_identity:project.directory_identity.clone(),common_dir,common_identity,kind:"original".into(),created_operation:None,base_commit:None})
    }
    pub async fn read_development_workspaces(&self,owner:&str,input:ReadWorkspacesInput)->Result<WorkspaceView,String> {
        self.ready().await?;
        let project=self.inner.repository.project(owner,&input.project_id).await?;
        let selected=if let Some(task_id)=&input.task_id {let task=self.inner.repository.task(owner,task_id).await?;if task.project_id!=project.id {return Err("task_not_found".into());}self.inner.repository.task_workspace(owner,task_id).await?.map(|w|w.id)} else {None};
        let mut workspaces=self.inner.repository.workspaces(owner,&project.id).await?;
        if !workspaces.iter().any(|w|w.kind=="original") {workspaces.insert(0,self.original_workspace(owner,&project).await?);}
        let operations=if let Some(task_id)=&input.task_id {self.inner.repository.list_git_operations(owner,task_id).await?} else {Vec::new()};
        Ok(WorkspaceView{workspaces,selected_workspace_id:selected,operation:None,operations})
    }
    pub async fn save_development_workspaces(&self,owner:&str,input:SaveWorkspaceInput,lease:CancellationToken)->Result<WorkspaceView,String> {
        let _admission=self.inner.admission.lock().await;self.ready().await?;live(&lease)?;
        Uuid::parse_str(&input.request_id).map_err(|_|"invalid_request_id")?;
        let fingerprint=format!("{:x}",Sha256::digest(serde_json::to_vec(&input).map_err(|_|"invalid_request")?));
        let task=self.inner.repository.task(owner,&input.task_id).await?;
        let project=self.inner.repository.project(owner,&task.project_id).await?;
        if let Some(old)=self.inner.repository.git_operation(owner,&input.request_id).await? {
            if old["request_fingerprint"].as_str()!=Some(fingerprint.as_str()) {return Err("request_conflict".into());}
            let mut view=self.read_development_workspaces(owner,ReadWorkspacesInput{project_id:project.id,task_id:Some(task.id)}).await?;view.operation=Some(old);return Ok(view);
        }
        if project.archived {return Err("project_archived".into());}
        if !matches!(input.action.as_str(),"create"|"import"|"select"|"remove") {return Err("invalid_workspace".into());}
        let original=self.original_workspace(owner,&project).await?;
        if self.inner.repository.workspace_by_id(owner,&original.id).await?.is_none() {live(&lease)?;self.inner.repository.insert_workspace(&original).await?;}
        let current=self.inner.repository.task_workspace(owner,&task.id).await?;
        let locked=self.inner.repository.review_baseline(owner,&task.id).await?.is_some() || self.list(owner).await.iter().any(|r|r.task_id.as_deref()==Some(task.id.as_str()));
        if locked && matches!(input.action.as_str(),"create"|"import") {return Err("workspace_immutable".into());}
        let chosen=if matches!(input.action.as_str(),"select"|"remove") {
            let w=self.inner.repository.workspace_by_id(owner,input.workspace_id.as_deref().ok_or("workspace_not_found")?).await?.ok_or("workspace_not_found")?;
            if w.project_id!=project.id {return Err("workspace_not_found".into());}
            validate_workspace(&w,&self.inner.stop).await?;
            if input.action=="select" && locked && current.as_ref().map(|c|c.path.as_str()).unwrap_or(&project.path)!=w.path {return Err("workspace_immutable".into());}
            Some(w)
        } else {None};
        let kind=format!("workspace_{}",input.action);
        let mut details=json!({"path":input.path.as_deref().or_else(||chosen.as_ref().map(|w|w.path.as_str())).unwrap_or(&project.path),"request":input});
        if let Some(w)=&chosen {details["workspace"]=serde_json::to_value(w).map_err(|_|"storage_failed")?;}
        self.inner.repository.start_git_operation(owner,&GitOperationIntent {
            id: &input.request_id,
            project_id: &project.id,
            task_id: &task.id,
            workspace_id: chosen.as_ref().map(|w|w.id.as_str()),
            kind: &kind,
            fingerprint: &fingerprint,
            details: &details,
        }).await?;
        let mut external_started=false;
        let result:Result<(),String>=async {
            live(&lease)?;self.ready().await?;
            match input.action.as_str() {
                "select"=>{let w=chosen.as_ref().ok_or("workspace_not_found")?;self.inner.repository.bind_task_workspace(owner,&task.id,&w.id).await?;details["workspace"]=serde_json::to_value(w).map_err(|_|"storage_failed")?;},
                "import"|"create"=>{
                    let common=original.common_dir.as_deref().ok_or("not_git_repository")?;
                    self.repository_git_available(&project.path,common).await?;
                    let (path,base)=if input.action=="create" {
                        let branch=input.branch.as_deref().ok_or("invalid_branch")?;valid_branch(branch)?;
                        let commit=input.base_commit.as_deref().ok_or("invalid_commit")?;
                        if !(7..=64).contains(&commit.len()) || !commit.bytes().all(|b|b.is_ascii_hexdigit()) {return Err("invalid_commit".into());}
                        let mut resolve=crate::git::safe_command(Path::new(&project.path)).await?;resolve.args(["rev-parse","--verify",&format!("{commit}^{{commit}}")]);
                        let (bytes,_)=git_output(resolve,Duration::from_secs(10),&lease,&self.inner.stop).await?;
                        let commit=String::from_utf8(bytes).map_err(|_|"invalid_commit")?.trim().to_owned();
                        let target=Path::new(input.path.as_deref().ok_or("invalid_workspace")?);
                        if !target.is_absolute() {return Err("invalid_workspace".into());}
                        let parent=tokio::fs::canonicalize(target.parent().ok_or("invalid_workspace")?).await.map_err(|_|"invalid_workspace")?;
                        let target=parent.join(target.file_name().ok_or("invalid_workspace")?);
                        match tokio::fs::symlink_metadata(&target).await {Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},_=>return Err("invalid_workspace".into())}
                        details["expected_commit"]=json!(commit);details["branch"]=json!(branch);details["path"]=json!(target);
                        self.inner.repository.update_git_operation_details(owner,&input.request_id,&details).await?;
                        validate_workspace(&original,&self.inner.stop).await?;
                        let mut command=crate::git::safe_command(Path::new(&project.path)).await?;
                        command.args(["worktree","add","-b",branch,"--"]).arg(&target).arg(&commit);
                        live(&lease)?;
                        tokio::fs::create_dir(&target).await.map_err(|_|"invalid_workspace")?;
                        let (reserved,reserved_identity)=directory(target.to_str().ok_or("invalid_workspace")?).await?;
                        details["reserved_identity"]=json!(reserved_identity);
                        self.inner.repository.update_git_operation_details(owner,&input.request_id,&details).await?;
                        let (again,identity)=directory(&reserved).await?;
                        if again!=reserved || Some(identity.as_str())!=details["reserved_identity"].as_str() {return Err("workspace_changed".into());}
                        external_started=true;
                        git_output(command,Duration::from_secs(120),&lease,&self.inner.stop).await?;
                        (target.to_string_lossy().into_owned(),Some(commit))
                    } else {(input.path.clone().ok_or("invalid_workspace")?,None)};
                    let (path,directory_identity)=directory(&path).await?;
                    let (common_dir,common_identity)=git_identity(&path,&lease,&self.inner.stop).await?;
                    if common_dir!=common || Some(&common_identity)!=original.common_identity.as_ref() {return Err("foreign_workspace".into());}
                    let mut root=git_command(Path::new(&path));root.args(["rev-parse","--show-toplevel"]);
                    let (bytes,_)=git_output(root,Duration::from_secs(10),&lease,&self.inner.stop).await?;
                    let (root,_)=directory(String::from_utf8(bytes).map_err(|_|"invalid_workspace")?.trim()).await?;
                    if root!=path {return Err("invalid_workspace".into());}
                    let mut registered=git_command(Path::new(&project.path));registered.args(["worktree","list","--porcelain","-z"]);
                    let (bytes,_)=git_output(registered,Duration::from_secs(10),&lease,&self.inner.stop).await?;
                    if !bytes.split(|b|*b==0).any(|part|part.strip_prefix(b"worktree ")==Some(path.as_bytes())) {return Err("invalid_workspace".into());}
                    let w=StoredWorkspace{id:input.request_id.clone(),owner:owner.into(),project_id:project.id.clone(),path,directory_identity,common_dir:Some(common_dir),common_identity:Some(common_identity),kind:if input.action=="create" {"managed"} else {"imported"}.into(),created_operation:Some(input.request_id.clone()),base_commit:base};
                    details["workspace"]=serde_json::to_value(&w).map_err(|_|"storage_failed")?;
                    self.inner.repository.update_git_operation_details(owner,&input.request_id,&details).await?;
                    live(&lease)?;self.inner.repository.insert_workspace(&w).await?;self.inner.repository.bind_task_workspace(owner,&task.id,&w.id).await?;
                },
                "remove"=>{
                    let w=chosen.as_ref().ok_or("workspace_not_found")?;
                    if w.kind!="managed" {return Err("workspace_remove_refused".into());}
                    let created=self.inner.repository.git_operation(owner,w.created_operation.as_deref().ok_or("workspace_remove_refused")?).await?.ok_or("workspace_remove_refused")?;
                    if created["kind"]!="workspace_create" || created["status"]!="succeeded" || created["details"]["workspace"]!=serde_json::to_value(w).map_err(|_|"storage_failed")? {return Err("workspace_remove_refused".into());}
                    if self.inner.repository.workspace_needed(owner,&w.id,&task.id).await? {return Err("workspace_in_use".into());}
                    self.repository_git_available(&w.path,w.common_dir.as_deref().ok_or("not_git_repository")?).await?;
                    let retain=input.retain_branch.as_deref().ok_or("invalid_branch")?;valid_branch(retain)?;
                    let mut status=crate::git::safe_command(Path::new(&w.path)).await?;status.args(["status","--porcelain=v1","-z","--untracked-files=all","--ignored=matching"]);
                    if !git_output(status,Duration::from_secs(10),&lease,&self.inner.stop).await?.0.is_empty() {return Err("workspace_dirty".into());}
                    let mut head=git_command(Path::new(&w.path));head.args(["rev-parse","--verify","HEAD"]);
                    let (head,_)=git_output(head,Duration::from_secs(10),&lease,&self.inner.stop).await?;
                    details["expected_head"]=json!(String::from_utf8(head).map_err(|_|"git_failed")?.trim());details["retained_branch"]=json!(retain);
                    let mut merged=crate::git::safe_command(Path::new(&w.path)).await?;merged.args(["merge-base","--is-ancestor","HEAD",&format!("refs/heads/{retain}")]);
                    git_output(merged,Duration::from_secs(10),&lease,&self.inner.stop).await.map_err(|e|if e=="git_failed" {"workspace_unmerged".into()}else{e})?;
                    validate_workspace(w,&self.inner.stop).await?;live(&lease)?;
                    self.inner.repository.update_git_operation_details(owner,&input.request_id,&details).await?;
                    let mut remove=crate::git::safe_command(Path::new(&project.path)).await?;remove.args(["worktree","remove","--",&w.path]);
                    external_started=true;git_output(remove,Duration::from_secs(120),&lease,&self.inner.stop).await?;
                    live(&lease)?;details["removed"]=json!(w.id);
                }, _=>unreachable!()
            }
            live(&lease)?;Ok(())
        }.await;
        let status=match &result {Ok(())=>"succeeded",Err(e) if external_started || matches!(e.as_str(),"cancelled"|"git_timeout"|"git_output_limit"|"storage_failed")=>"unknown",Err(_)=>"failed"};
        if let Err(error)=&result {details["error"]=json!(error);}
        self.inner.repository.finish_git_operation(owner,&input.request_id,status,&details).await?;
        result?;
        let mut view=self.read_development_workspaces(owner,ReadWorkspacesInput{project_id:project.id,task_id:Some(task.id)}).await?;
        view.operation=self.inner.repository.git_operation(owner,&input.request_id).await?;Ok(view)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn branch_names_cannot_be_options_or_revision_expressions() {
        for name in ["-force","main..other","refs@{0}","a.lock/x","a//b","a\nb","/tmp/x","a b"] {assert!(valid_branch(name).is_err(),"{name}");}
        for name in ["feature/task-27","retained_main","release/1.0"] {assert!(valid_branch(name).is_ok(),"{name}");}
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    fn git(path:&Path,args:&[&str])->String {
        let output=std::process::Command::new("git").current_dir(path).args(args).output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));String::from_utf8(output.stdout).unwrap().trim().into()
    }
    fn input(task:&str,action:&str)->SaveWorkspaceInput {SaveWorkspaceInput{request_id:Uuid::new_v4().to_string(),task_id:task.into(),action:action.into(),workspace_id:None,path:None,branch:None,base_commit:None,retain_branch:None}}
    #[tokio::test]
    async fn dirty_original_isolated_binding_freezes_and_managed_remove_preserves_branch() {
        use crate::{files::ReadFileInput,projects::{CreateProjectInput,SaveTaskInput}};
        let root=std::env::temp_dir().join(format!("dgos-workspaces-{}",Uuid::new_v4()));let source=root.join("source");std::fs::create_dir_all(&source).unwrap();
        git(&source,&["init","-b","main"]);std::fs::write(source.join("code.txt"),"base\n").unwrap();git(&source,&["add","code.txt"]);git(&source,&["-c","user.name=Fixture","-c","user.email=fixture@example.invalid","commit","-m","base"]);let commit=git(&source,&["rev-parse","HEAD"]);std::fs::write(source.join("code.txt"),"user dirty\n").unwrap();
        let manager=crate::test_support::manager(&root).await;let project=manager.create_project("alice",CreateProjectInput{kind:"import".into(),name:"Workspaces".into(),path:source.to_string_lossy().into(),source:None},CancellationToken::new()).await.unwrap();
        let task=manager.save_development_task("alice",SaveTaskInput{id:None,project_id:project.id.clone(),goal:"Isolate".into(),requirements:"Keep original".into(),status:"todo".into()},CancellationToken::new()).await.unwrap();let target=root.join("isolated");let mut create=input(&task.id,"create");create.path=Some(target.to_string_lossy().into());create.branch=Some("task/isolated".into());create.base_commit=Some(commit);
        let created=manager.save_development_workspaces("alice",create,CancellationToken::new()).await.unwrap();let selected=created.selected_workspace_id.unwrap();let canonical_target=std::fs::canonicalize(&target).unwrap();assert_eq!(std::fs::read_to_string(source.join("code.txt")).unwrap(),"user dirty\n");assert_eq!(manager.task_project("alice",&task.id).await.unwrap().path,canonical_target.to_string_lossy().as_ref());
        let file=manager.read_development_file("alice",ReadFileInput{project_id:project.id.clone(),task_id:Some(task.id.clone()),path:"code.txt".into()}).await.unwrap();assert_eq!(file.text.as_deref(),Some("base\n"));
        manager.capture_task_baseline("alice",&task.id).await.unwrap();let mut select=input(&task.id,"select");select.workspace_id=Some(format!("original-{}",project.id));assert_eq!(manager.save_development_workspaces("alice",select,CancellationToken::new()).await.unwrap_err().to_string(),"workspace_immutable");
        manager.save_development_task("alice",SaveTaskInput{id:Some(task.id.clone()),project_id:project.id.clone(),goal:task.goal,requirements:task.requirements,status:"completed".into()},CancellationToken::new()).await.unwrap();let mut remove=input(&task.id,"remove");remove.workspace_id=Some(selected);remove.retain_branch=Some("main".into());manager.save_development_workspaces("alice",remove,CancellationToken::new()).await.unwrap();assert!(!target.exists());assert!(!git(&source,&["show-ref","--verify","refs/heads/task/isolated"]).is_empty());assert_eq!(std::fs::read_to_string(source.join("code.txt")).unwrap(),"user dirty\n");std::fs::remove_dir_all(root).unwrap();
    }

    struct Fixture {
        root: std::path::PathBuf,
        source: std::path::PathBuf,
        target: std::path::PathBuf,
        manager: AgentManager,
        task: String,
        workspace: String,
    }
    impl Fixture {
        async fn new(action: &str) -> Self {
            let root = std::env::temp_dir().join(format!("dgos-workspace-cleanup-{}", Uuid::new_v4()));
            std::fs::create_dir_all(root.join("source")).unwrap();
            let root = root.canonicalize().unwrap();
            let source = root.join("source");
            let target = root.join("workspace");
            git(&source, &["init", "-b", "main"]);
            std::fs::write(source.join("code.txt"), "base\n").unwrap();
            std::fs::write(source.join(".gitignore"), "valuable/\n").unwrap();
            git(&source, &["add", "code.txt", ".gitignore"]);
            git(&source, &["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-m", "base"]);
            let base = git(&source, &["rev-parse", "HEAD"]);
            let manager = crate::test_support::manager(&root).await;
            let task = crate::test_support::task(&manager, &source).await;
            if action == "import" {
                git(&source, &["worktree", "add", "-b", "task/cleanup", target.to_str().unwrap(), &base]);
            }
            let mut create = input(&task, action);
            create.path = Some(target.to_string_lossy().into_owned());
            if action == "create" {
                create.branch = Some("task/cleanup".into());
                create.base_commit = Some(base);
            }
            let created = manager.save_development_workspaces("alice", create, CancellationToken::new()).await.unwrap();
            let task_record = manager.inner.repository.task("alice", &task).await.unwrap();
            manager.save_development_task("alice", crate::projects::SaveTaskInput {
                id: Some(task.clone()), project_id: task_record.project_id,
                goal: task_record.goal, requirements: task_record.requirements, status: "completed".into(),
            }, CancellationToken::new()).await.unwrap();
            Self { root, source, target, manager, task, workspace: created.selected_workspace_id.unwrap() }
        }
        fn removal(&self) -> SaveWorkspaceInput {
            let mut remove = input(&self.task, "remove");
            remove.workspace_id = Some(self.workspace.clone());
            remove.retain_branch = Some("main".into());
            remove
        }
        fn assert_preserved(&self, head: &str) {
            assert_eq!(std::fs::read_to_string(self.target.join("code.txt")).unwrap(), "base\n");
            assert_eq!(std::fs::read_to_string(self.source.join("code.txt")).unwrap(), "base\n");
            assert_eq!(git(&self.target, &["rev-parse", "HEAD"]), head);
            assert_eq!(git(&self.source, &["rev-parse", "refs/heads/task/cleanup"]), head);
            assert!(git(&self.source, &["worktree", "list", "--porcelain"]).lines().any(|line| line == format!("worktree {}", self.target.display())));
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
    }

    #[tokio::test]
    async fn workspace_receipts_survive_read_restart_and_reconcile_without_replaying_actions() {
        use crate::git::ReconcileGitInput;
        let fixture = Fixture::new("create").await;
        let project = fixture.manager.inner.repository.task("alice", &fixture.task).await.unwrap().project_id;
        let read = || ReadWorkspacesInput { project_id: project.clone(), task_id: Some(fixture.task.clone()) };
        let initial = fixture.manager.read_development_workspaces("alice", read()).await.unwrap();
        let created = initial.operations.iter().find(|operation| operation["kind"] == "workspace_create").unwrap().clone();
        assert_eq!(created["status"], "succeeded");
        assert_eq!(initial.selected_workspace_id.as_deref(), Some(fixture.workspace.as_str()));
        assert_eq!(serde_json::to_value(&initial).unwrap()["operations"][0], created);
        assert!(fixture.manager.read_development_workspaces("bob", read()).await.is_err());
        // The worktree and binding were saved, but the terminal receipt was lost.
        fixture.manager.inner.repository.finish_git_operation("alice", created["id"].as_str().unwrap(), "unknown", &created["details"]).await.unwrap();
        let restarted = crate::test_support::manager(&fixture.root).await;
        assert_eq!(restarted.read_development_workspaces("alice", read()).await.unwrap().operations[0]["status"], "unknown");
        let reconcile = |id: &str| ReconcileGitInput { id: id.into(), trust_directory: false, trust_network: false };
        let reconciled = restarted.reconcile_development_git("alice", reconcile(created["id"].as_str().unwrap()), CancellationToken::new()).await.unwrap();
        assert_eq!(reconciled["status"], "succeeded");
        std::fs::write(fixture.target.join("later.txt"), "preserve later work\n").unwrap();
        let create: SaveWorkspaceInput = serde_json::from_value(created["details"]["request"].clone()).unwrap();
        assert_eq!(restarted.save_development_workspaces("alice", create, CancellationToken::new()).await.unwrap().operation.unwrap(), reconciled);
        assert_eq!(std::fs::read_to_string(fixture.target.join("later.txt")).unwrap(), "preserve later work\n");
        std::fs::remove_file(fixture.target.join("later.txt")).unwrap();
        // A separate removal crashed after intent persistence and before Git ran.
        let pending = fixture.removal();
        let workspace = restarted.inner.repository.workspace_by_id("alice", &fixture.workspace).await.unwrap().unwrap();
        let head = git(&fixture.target, &["rev-parse", "HEAD"]);
        let details = json!({"path": workspace.path, "workspace": workspace, "request": pending, "expected_head": head, "retained_branch": "main"});
        let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&pending).unwrap()));
        restarted.inner.repository.start_git_operation("alice", &GitOperationIntent {
            id: &pending.request_id, project_id: &project, task_id: &fixture.task, workspace_id: Some(&fixture.workspace),
            kind: "workspace_remove", fingerprint: &fingerprint, details: &details,
        }).await.unwrap();
        let next = crate::test_support::manager(&fixture.root).await;
        let unresolved = next.reconcile_development_git("alice", reconcile(&pending.request_id), CancellationToken::new()).await.unwrap();
        assert_eq!(unresolved["status"], "unknown");
        assert_eq!(next.save_development_workspaces("alice", pending, CancellationToken::new()).await.unwrap().operation.unwrap(), unresolved);
        fixture.assert_preserved(&head);
        let removal = fixture.removal();
        let removal_id = removal.request_id.clone();
        let removed = next.save_development_workspaces("alice", removal, CancellationToken::new()).await.unwrap();
        let removed_receipt = removed.operation.unwrap();
        assert_eq!(removed_receipt["status"], "succeeded"); assert!(!fixture.target.exists());
        let final_manager = crate::test_support::manager(&fixture.root).await;
        let history = final_manager.read_development_workspaces("alice", read()).await.unwrap();
        assert_eq!(history.operations.len(), 3);
        assert!(history.operations.contains(&reconciled)); assert!(history.operations.contains(&unresolved)); assert!(history.operations.contains(&removed_receipt));
        assert_eq!(serde_json::to_value(&history).unwrap()["operations"].as_array().unwrap().len(), 3);
        let create: SaveWorkspaceInput = serde_json::from_value(created["details"]["request"].clone()).unwrap();
        final_manager.save_development_workspaces("alice", create, CancellationToken::new()).await.unwrap();
        let removal: SaveWorkspaceInput = serde_json::from_value(removed_receipt["details"]["request"].clone()).unwrap();
        assert_eq!(final_manager.save_development_workspaces("alice", removal, CancellationToken::new()).await.unwrap().operation.unwrap(), removed_receipt);
        assert_eq!(final_manager.reconcile_development_git("alice", reconcile(&removal_id), CancellationToken::new()).await.unwrap(), removed_receipt);
        assert!(!fixture.target.exists()); assert_eq!(git(&fixture.source, &["rev-parse", "refs/heads/task/cleanup"]), head);
        assert!(fixture.manager.shutdown().await); assert!(restarted.shutdown().await); assert!(next.shutdown().await); assert!(final_manager.shutdown().await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_cleanup_rejects_live_services_in_workspace_and_shared_repository() {
        let fixture = Fixture::new("create").await;
        let head = git(&fixture.target, &["rev-parse", "HEAD"]);
        let task_record = fixture.manager.inner.repository.task("alice", &fixture.task).await.unwrap();
        let source_task = fixture.manager.save_development_task("alice", crate::projects::SaveTaskInput {
            id: None, project_id: task_record.project_id, goal: "Run source service".into(),
            requirements: "Hold the shared repository".into(), status: "todo".into(),
        }, CancellationToken::new()).await.unwrap().id;
        for (task, expected_error) in [(&fixture.task, "workspace_busy"), (&source_task, "repository_busy")] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            let service = fixture.manager.start_development_process("alice", crate::runtime::StartProcessInput {
                id: Uuid::new_v4().to_string(), task_id: task.clone(), kind: "service".into(),
                program: "/bin/sh".into(), args: vec!["-c".into(), "printf 'pid:%s\\n' $$; while :; do sleep 1; done".into()],
                cwd: String::new(), env_refs: Default::default(), trust_workspace: true, verification: false,
                readiness: Some(crate::runtime::ReadinessInput { port, path: "/".into(), timeout_ms: 120000 }), native_run_id: None,
            }, CancellationToken::new()).await.unwrap();
            let running = tokio::time::timeout(Duration::from_secs(12), async {
                loop {
                    let view = fixture.manager.read_development_process("alice", &service.id).await.unwrap();
                    if view.output.lines().any(|line| line.starts_with("pid:")) { break view; }
                    assert!(view.finished_at.is_none(), "service exited before cleanup attempt: {:?}", view);
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
            }).await.expect("service did not start");
            let pid: i32 = running.output.lines().find_map(|line| line.strip_prefix("pid:")).unwrap().parse().unwrap();
            // Starting a run marks its task in progress. Complete it explicitly so
            // removal reaches the live-process guard rather than workspace_in_use.
            let task_record = fixture.manager.inner.repository.task("alice", &fixture.task).await.unwrap();
            fixture.manager.save_development_task("alice", crate::projects::SaveTaskInput {
                id: Some(fixture.task.clone()), project_id: task_record.project_id,
                goal: task_record.goal, requirements: task_record.requirements, status: "completed".into(),
            }, CancellationToken::new()).await.unwrap();
            let alive_before = unsafe { libc::kill(pid, 0) } == 0;
            let removal = fixture.manager.save_development_workspaces("alice", fixture.removal(), CancellationToken::new()).await;
            let alive_after = unsafe { libc::kill(pid, 0) } == 0;
            let after = fixture.manager.read_development_process("alice", &service.id).await.unwrap();
            let stopped = fixture.manager.stop_development_process("alice", &service.id).await.unwrap();
            assert!(alive_before && alive_after, "cleanup stopped an active service");
            assert!(after.finished_at.is_none());
            assert_eq!(removal.unwrap_err(), expected_error);
            assert!(stopped.finished_at.is_some());
            assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "explicit stop did not reap the service");
            fixture.assert_preserved(&head);
        }
        fixture.manager.save_development_workspaces("alice", fixture.removal(), CancellationToken::new()).await.unwrap();
        assert!(!fixture.target.exists());
        assert_eq!(git(&fixture.source, &["rev-parse", "refs/heads/task/cleanup"]), head);
        assert!(fixture.manager.shutdown().await);
    }

    #[tokio::test]
    async fn managed_cleanup_requires_retained_branch_to_contain_workspace_head() {
        let fixture = Fixture::new("create").await;
        let main = git(&fixture.source, &["rev-parse", "main"]);
        std::fs::write(fixture.target.join("valuable.txt"), "unmerged work\n").unwrap();
        git(&fixture.target, &["add", "valuable.txt"]);
        git(&fixture.target, &["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-m", "valuable work"]);
        let head = git(&fixture.target, &["rev-parse", "HEAD"]);
        assert_ne!(head, main);
        assert!(git(&fixture.target, &["status", "--porcelain"]).is_empty());
        assert_eq!(fixture.manager.save_development_workspaces("alice", fixture.removal(), CancellationToken::new()).await.unwrap_err(), "workspace_unmerged");
        fixture.assert_preserved(&head);
        assert_eq!(git(&fixture.source, &["rev-parse", "main"]), main);
        assert_eq!(std::fs::read_to_string(fixture.target.join("valuable.txt")).unwrap(), "unmerged work\n");
        git(&fixture.source, &["merge", "--ff-only", "task/cleanup"]);
        fixture.manager.save_development_workspaces("alice", fixture.removal(), CancellationToken::new()).await.unwrap();
        assert!(!fixture.target.exists());
        assert_eq!(git(&fixture.source, &["rev-parse", "main"]), head);
        assert_eq!(git(&fixture.source, &["rev-parse", "refs/heads/task/cleanup"]), head);
        assert_eq!(std::fs::read_to_string(fixture.source.join("valuable.txt")).unwrap(), "unmerged work\n");
    }

    #[tokio::test]
    async fn imported_workspace_cleanup_is_refused_and_preserves_user_files() {
        let fixture = Fixture::new("import").await;
        let head = git(&fixture.target, &["rev-parse", "HEAD"]);
        std::fs::write(fixture.target.join("user-notes.txt"), "keep these notes\n").unwrap();
        std::fs::create_dir(fixture.target.join("valuable")).unwrap();
        std::fs::write(fixture.target.join("valuable/private.txt"), "keep ignored data\n").unwrap();
        assert_eq!(fixture.manager.save_development_workspaces("alice", fixture.removal(), CancellationToken::new()).await.unwrap_err(), "workspace_remove_refused");
        fixture.assert_preserved(&head);
        assert_eq!(std::fs::read_to_string(fixture.target.join("user-notes.txt")).unwrap(), "keep these notes\n");
        assert_eq!(std::fs::read_to_string(fixture.target.join("valuable/private.txt")).unwrap(), "keep ignored data\n");
    }

    #[tokio::test]
    async fn ignored_valuable_contents_block_managed_cleanup_until_user_removes_them() {
        let fixture = Fixture::new("create").await;
        let head = git(&fixture.target, &["rev-parse", "HEAD"]);
        std::fs::create_dir(fixture.target.join("valuable")).unwrap();
        let valuable = fixture.target.join("valuable/private.txt");
        std::fs::write(&valuable, "irreplaceable ignored data\n").unwrap();
        assert!(git(&fixture.target, &["status", "--porcelain", "--untracked-files=all"]).is_empty());
        assert_eq!(git(&fixture.target, &["check-ignore", "valuable/private.txt"]), "valuable/private.txt");
        assert_eq!(fixture.manager.save_development_workspaces("alice", fixture.removal(), CancellationToken::new()).await.unwrap_err(), "workspace_dirty");
        fixture.assert_preserved(&head);
        assert_eq!(std::fs::read_to_string(&valuable).unwrap(), "irreplaceable ignored data\n");
        std::fs::remove_file(valuable).unwrap();
        std::fs::remove_dir(fixture.target.join("valuable")).unwrap();
        fixture.manager.save_development_workspaces("alice", fixture.removal(), CancellationToken::new()).await.unwrap();
        assert!(!fixture.target.exists());
        assert_eq!(git(&fixture.source, &["rev-parse", "refs/heads/task/cleanup"]), head);
    }
}

impl AgentManager {
    /// Reconciliation observes durable evidence only; it never repeats worktree actions.
    pub(crate) async fn reconcile_workspace_operation(&self,owner:&str,id:&str,lease:&CancellationToken)->Result<Value,String> {
        self.ready().await?;live(lease)?;
        let old=self.inner.repository.git_operation(owner,id).await?.ok_or("operation_not_found")?;
        if !matches!(old["status"].as_str(),Some("unknown"|"pending")) {return Ok(old);}
        let task_id=old["task_id"].as_str().ok_or("task_not_found")?;
        let task=self.inner.repository.task(owner,task_id).await?;
        let project=self.inner.repository.project(owner,&task.project_id).await?;
        let mut details=old["details"].clone();
        let proven=match old["kind"].as_str() {
            Some("workspace_create"|"workspace_import"|"workspace_select")=>{
                if let Ok(w)=serde_json::from_value::<StoredWorkspace>(details["workspace"].clone()) {
                    let bound=self.inner.repository.task_workspace(owner,task_id).await?;
                    let stored=self.inner.repository.workspace_by_id(owner,&w.id).await?;
                    if bound.as_ref()==Some(&w) && stored.as_ref()==Some(&w) && validate_workspace(&w,&self.inner.stop).await.is_ok() {
                        if w.common_dir.is_some() {
                            let mut registry=git_command(Path::new(&w.path));registry.args(["worktree","list","--porcelain","-z"]);
                            git_output(registry,Duration::from_secs(10),lease,&self.inner.stop).await?.0.split(|b|*b==0).any(|p|p.strip_prefix(b"worktree ")==Some(w.path.as_bytes()))
                        } else {true}
                    } else {false}
                } else {false}
            },
            Some("workspace_remove")=>{
                if let Ok(w)=serde_json::from_value::<StoredWorkspace>(details["workspace"].clone()) {
                    if matches!(tokio::fs::symlink_metadata(&w.path).await,Err(e) if e.kind()==std::io::ErrorKind::NotFound) {
                        validate_directory(&project).await?;
                        let (common,identity)=git_identity(&project.path,lease,&self.inner.stop).await?;
                        if Some(&common)==w.common_dir.as_ref() && Some(&identity)==w.common_identity.as_ref() {
                            let mut registry=git_command(Path::new(&project.path));registry.args(["worktree","list","--porcelain","-z"]);
                            let absent=!git_output(registry,Duration::from_secs(10),lease,&self.inner.stop).await?.0.split(|b|*b==0).any(|p|p.strip_prefix(b"worktree ")==Some(w.path.as_bytes()));
                            if absent {
                                if let (Some(head),Some(retain))=(details["expected_head"].as_str(),details["retained_branch"].as_str()) {
                                    valid_branch(retain)?;
                                    let mut retained=git_command(Path::new(&project.path));retained.args(["merge-base","--is-ancestor",head,&format!("refs/heads/{retain}")]);
                                    git_output(retained,Duration::from_secs(10),lease,&self.inner.stop).await.is_ok()
                                } else {false}
                            } else {false}
                        } else {false}
                    } else {false}
                } else {false}
            }, _=>return Err("invalid_operation".into())
        };
        live(lease)?;details["reconciled"]=json!(proven);
        self.inner.repository.finish_git_operation(owner,id,if proven {"succeeded"} else {"unknown"},&details).await?;
        self.inner.repository.git_operation(owner,id).await?.ok_or("operation_not_found".into())
    }
}
