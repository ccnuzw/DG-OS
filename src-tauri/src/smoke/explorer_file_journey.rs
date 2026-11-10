async fn explorer_file_journey(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    let path = fx.root.join("explorer-project");
    fs::create_dir(&path).map_err(|e|e.to_string())?;
    fs::create_dir(path.join("src")).map_err(|e|e.to_string())?;
    fs::write(path.join("src/hello.txt"), "original bytes\n").map_err(|e|e.to_string())?;
    let project = host(app,"create_development_project",json!({"input":{"kind":"import","name":"Explorer smoke","path":path,"source":null}})).await?;
    let pid = project["id"].as_str().ok_or("missing project")?;
    require(fx,ck,ev,"native explorer import",!pid.is_empty(),"temporary project")?;
    require(fx,ck,ev,"native explorer shipped route",goto(app,"main","/tasks/agent").await,"production App")?;
    step(app,fx,ck,ev,"native explorer selected project",json!({"project":pid}),r#"
const p=await need('dev-tool-projects');if(p.getAttribute('aria-expanded')!=='true')p.click();await click('dev-project-item-'+A.project);const f=await need('dev-tool-files');if(f.getAttribute('aria-expanded')!=='true')f.click();await need('dev-file-search');return {ok:!!await need('dev-file-item-src')};
"#).await?;
    let snapshot = host(app,"inspect_development_entry",json!({"input":{"project_id":pid,"task_id":null,"path":"src"}})).await?;
    let copy = host(app,"mutate_development_entry",json!({"input":{"project_id":pid,"task_id":null,"path":"src","expected_version":snapshot["version"],"action":"copy","destination":"copied","trust_directory":true}})).await?;
    require(fx,ck,ev,"native IPC recursive copy retains source",copy["path"]=="copied"&&fs::read(path.join("copied/hello.txt")).ok()==fs::read(path.join("src/hello.txt")).ok(),copy.to_string())?;
    let snapshot = host(app,"inspect_development_entry",json!({"input":{"project_id":pid,"task_id":null,"path":"copied"}})).await?;
    host(app,"mutate_development_entry",json!({"input":{"project_id":pid,"task_id":null,"path":"copied","expected_version":snapshot["version"],"action":"rename","destination":"renamed","trust_directory":true}})).await?;
    require(fx,ck,ev,"native IPC rename moves full subtree",!path.join("copied").exists()&&path.join("renamed/hello.txt").is_file(),"renamed")?;
    host(app,"open_development_entry_directory",json!({"input":{"project_id":pid,"task_id":null,"path":"renamed/hello.txt"}})).await?;
    host(app,"open_development_entry_directory",json!({"input":{"project_id":pid,"task_id":null,"path":"src"}})).await?;
    require(fx,ck,ev,"native directory opener accepted",true,"system API accepted containing directory; visual Finder focus not asserted")?;
    let snapshot = host(app,"inspect_development_entry",json!({"input":{"project_id":pid,"task_id":null,"path":"renamed"}})).await?;
    host(app,"mutate_development_entry",json!({"input":{"project_id":pid,"task_id":null,"path":"renamed","expected_version":snapshot["version"],"action":"delete","destination":null,"trust_directory":true}})).await?;
    require(fx,ck,ev,"native IPC recursive delete preserves other entries",!path.join("renamed").exists()&&path.join("src/hello.txt").exists(),"original intact")?;
    step(app,fx,ck,ev,"native tree refresh and file open",json!({}),r#"
await click('dev-review-refresh');await click('dev-file-item-src');await click('dev-file-item-src/hello.txt');return {ok:!!await until(()=>document.body.innerText.includes('original bytes')||tid('dev-file-editor')?.value==='original bytes\n')};
"#).await?;
    step(app,fx,ck,ev,"native file panel is independent",json!({}),r#"
const file=await need('dev-file-workspace'),column=file.closest('.dev-document-column'),main=document.querySelector('.dev-workspace-main'),body=document.querySelector('.dev-workspace-body');const bar=file.querySelector('.workbench-tabbar'),tabs=main.querySelector('.dev-session-tabs');return {ok:column.parentElement===body&&main.parentElement===body&&!main.contains(file)&&Math.abs(bar.getBoundingClientRect().top-tabs.getBoundingClientRect().top)<2&&column.getBoundingClientRect().height>300};
"#).await?;
    let workspace = host(app,"read_development_workspace",json!({})).await?;
    require(fx,ck,ev,"native explorer creates no task or agent",workspace["tasks"].as_array().is_some_and(Vec::is_empty)&&workspace["sessions"].as_array().is_some_and(Vec::is_empty),"taskless file operations")?;
    let _ = super::super::capture_native(app,&fx.root,"explorer-native").await;
    Ok(())
}
