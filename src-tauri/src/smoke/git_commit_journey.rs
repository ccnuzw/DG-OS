// Real Git history through the shipped renderer and host IPC; no external services.
async fn git_commit_journey(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    let path = fx.root.join("commit-history-project");
    make_repo(&path);
    fs::write(path.join("README.md"), "committed documentation\n").map_err(|e|e.to_string())?;
    require(fx,ck,ev,"commit history fixture staged",git(&path,&["add","README.md"]),"isolated repository")?;
    require(fx,ck,ev,"commit history fixture committed",git(&path,&["-c","user.name=History Author","-c","user.email=history@example.invalid","commit","-q","-m","Polish history details","-m","Full commit body retained."]),"second local commit")?;
    fs::write(path.join("README.md"), "UNCOMMITTED WORKTREE MUST STAY\n").map_err(|e|e.to_string())?;
    let index = fs::read(path.join(".git/index")).map_err(|e|e.to_string())?;
    let project=host(app,"create_development_project",json!({"input":{"kind":"import","name":"Commit history smoke","path":path,"source":null}})).await?;
    let pid=project["id"].as_str().ok_or("missing project")?;
    require(fx,ck,ev,"commit history shipped route",goto(app,"main","/tasks/agent").await,"production App")?;
    step(app,fx,ck,ev,"history opens immutable commit detail",json!({"project":pid}),r#"
const p=await need('dev-tool-projects');if(p.getAttribute('aria-expanded')!=='true')p.click();await click('dev-project-item-'+A.project);
const f=await need('dev-tool-files');if(f.getAttribute('aria-expanded')!=='true')f.click();
const gitButton=await until(()=>[...document.querySelectorAll('.dev-explorer-tabs button')].find(e=>e.textContent.trim()==='Git'));gitButton.click();
const history=await until(()=>document.querySelector('.git-history'));if(!history.open)history.querySelector('summary').click();
const row=await until(()=>[...document.querySelectorAll('[data-commit-oid]')].find(e=>e.textContent.includes('Polish history details')));row.click();
const panel=await need('git-commit-detail');return {ok:!!await until(()=>panel.innerText.includes('Full commit body retained.')&&panel.innerText.includes('history@example.invalid'))};
"#).await?;
    step(app,fx,ck,ev,"historical file diff excludes dirty worktree",json!({}),r#"
const panel=await need('git-commit-detail');const file=await until(()=>panel.querySelector('[data-commit-path="README.md"]'));file.click();
return {ok:!!await until(()=>panel.innerText.includes('committed documentation')&&panel.innerText.includes('acceptance')&&!panel.innerText.includes('UNCOMMITTED WORKTREE MUST STAY'))};
"#).await?;
    step(app,fx,ck,ev,"close commit detail preserves conversation",json!({}),r#"
await click('git-commit-close');return {ok:!!await until(()=>!tid('git-commit-detail')&&tid('dev-project-conversation'))};
"#).await?;
    require(fx,ck,ev,"history leaves worktree and index unchanged",fs::read_to_string(path.join("README.md")).ok().as_deref()==Some("UNCOMMITTED WORKTREE MUST STAY\n")&&fs::read(path.join(".git/index")).ok().as_deref()==Some(index.as_slice()),"immutable object reads only")?;
    let workspace=host(app,"read_development_workspace",json!({})).await?;
    require(fx,ck,ev,"history creates no task or agent",workspace["tasks"].as_array().is_some_and(Vec::is_empty)&&workspace["sessions"].as_array().is_some_and(Vec::is_empty),"taskless historical inspection")?;
    let _=super::super::capture_native(app,&fx.root,"commit-history-native").await;
    Ok(())
}
