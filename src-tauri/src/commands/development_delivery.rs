//! Stage5 host boundaries. Identity and cancellable account lease never come from the renderer.
use crate::{host::{self,Capability},session::SessionResolver};
use dgos_agents::{AgentManager,git::{GitReview,ExecuteGitInput,ReconcileGitInput},workspaces::{ReadWorkspacesInput,SaveWorkspaceInput,WorkspaceView},handoffs::{PrepareHandoffInput,AcceptHandoffInput,AcceptedHandoff}};
use dgos_db::development_repository::StoredHandoff;
use dgos_types::BuiltinCaller;
use serde_json::Value;
use std::sync::Arc;
use tauri::{Manager,WebviewWindow};

#[tauri::command]
pub async fn read_development_workspaces(app:tauri::AppHandle,window:WebviewWindow,input:ReadWorkspacesInput)->Result<WorkspaceView,String>{
 let context=host::context(&window,BuiltinCaller::Workbench,Capability::TasksRun).map_err(|e|e.to_string())?;
 let(session,lease)=app.state::<Arc<SessionResolver>>().agent_session().map_err(|e|e.to_string())?;
 if session.user_id()!=context.user_id()||lease.is_cancelled(){return Err("session_invalid".into());}
 let result=app.state::<AgentManager>().read_development_workspaces(context.user_id(),input).await;
 if lease.is_cancelled(){return Err("session_invalid".into());}result
}
#[tauri::command]
pub async fn save_development_workspaces(app:tauri::AppHandle,window:WebviewWindow,input:SaveWorkspaceInput)->Result<WorkspaceView,String>{
 let context=host::context(&window,BuiltinCaller::Workbench,Capability::TasksRun).map_err(|e|e.to_string())?;
 let(session,lease)=app.state::<Arc<SessionResolver>>().agent_session().map_err(|e|e.to_string())?;
 if session.user_id()!=context.user_id()||lease.is_cancelled(){return Err("session_invalid".into());}
 let result=app.state::<AgentManager>().save_development_workspaces(context.user_id(),input,lease.clone()).await;
 if lease.is_cancelled(){return Err("session_invalid".into());}result
}
#[tauri::command]
pub async fn read_development_git(app:tauri::AppHandle,window:WebviewWindow,task_id:String)->Result<GitReview,String>{
 let context=host::context(&window,BuiltinCaller::Workbench,Capability::TasksRun).map_err(|e|e.to_string())?;
 let(session,lease)=app.state::<Arc<SessionResolver>>().agent_session().map_err(|e|e.to_string())?;
 if session.user_id()!=context.user_id()||lease.is_cancelled(){return Err("session_invalid".into());}
 let result=app.state::<AgentManager>().read_development_git(context.user_id(),&task_id).await;
 if lease.is_cancelled(){return Err("session_invalid".into());}result
}
#[tauri::command]
pub async fn execute_development_git(app:tauri::AppHandle,window:WebviewWindow,input:ExecuteGitInput)->Result<Value,String>{
 let context=host::context(&window,BuiltinCaller::Workbench,Capability::TasksRun).map_err(|e|e.to_string())?;
 let(session,lease)=app.state::<Arc<SessionResolver>>().agent_session().map_err(|e|e.to_string())?;
 if session.user_id()!=context.user_id()||lease.is_cancelled(){return Err("session_invalid".into());}
 let result=app.state::<AgentManager>().execute_development_git(context.user_id(),input,lease.clone()).await;
 if lease.is_cancelled(){return Err("session_invalid".into());}result
}
#[tauri::command]
pub async fn reconcile_development_git(app:tauri::AppHandle,window:WebviewWindow,input:ReconcileGitInput)->Result<Value,String>{
 let context=host::context(&window,BuiltinCaller::Workbench,Capability::TasksRun).map_err(|e|e.to_string())?;
 let(session,lease)=app.state::<Arc<SessionResolver>>().agent_session().map_err(|e|e.to_string())?;
 if session.user_id()!=context.user_id()||lease.is_cancelled(){return Err("session_invalid".into());}
 let result=app.state::<AgentManager>().reconcile_development_git(context.user_id(),input,lease.clone()).await;
 if lease.is_cancelled(){return Err("session_invalid".into());}result
}
#[tauri::command]
pub async fn prepare_development_handoff(app:tauri::AppHandle,window:WebviewWindow,input:PrepareHandoffInput)->Result<StoredHandoff,String>{
 let context=host::context(&window,BuiltinCaller::Workbench,Capability::TasksRun).map_err(|e|e.to_string())?;
 let(session,lease)=app.state::<Arc<SessionResolver>>().agent_session().map_err(|e|e.to_string())?;
 if session.user_id()!=context.user_id()||lease.is_cancelled(){return Err("session_invalid".into());}
 let result=app.state::<AgentManager>().prepare_development_handoff(context.user_id(),input,lease.clone()).await;
 if lease.is_cancelled(){return Err("session_invalid".into());}result
}
#[tauri::command]
pub async fn list_development_handoffs(app:tauri::AppHandle,window:WebviewWindow,task_id:String)->Result<Vec<StoredHandoff>,String>{
 let context=host::context(&window,BuiltinCaller::Workbench,Capability::TasksRun).map_err(|e|e.to_string())?;
 let(session,lease)=app.state::<Arc<SessionResolver>>().agent_session().map_err(|e|e.to_string())?;
 if session.user_id()!=context.user_id()||lease.is_cancelled(){return Err("session_invalid".into());}
 let result=app.state::<AgentManager>().list_development_handoffs(context.user_id(),&task_id).await;
 if lease.is_cancelled(){return Err("session_invalid".into());}result
}
#[tauri::command]
pub async fn accept_development_handoff(app:tauri::AppHandle,window:WebviewWindow,mut input:AcceptHandoffInput)->Result<AcceptedHandoff,String>{
 let context=host::context(&window,BuiltinCaller::Workbench,Capability::TasksRun).map_err(|e|e.to_string())?;
 let(session,lease)=app.state::<Arc<SessionResolver>>().agent_session().map_err(|e|e.to_string())?;
 if session.user_id()!=context.user_id()||lease.is_cancelled(){return Err("session_invalid".into());}
 let manager=app.state::<AgentManager>();
 // Executable preferences are host-owned, just like open_agent_session.
 let handoff=manager.development_handoff(context.user_id(),&input.handoff_id).await?;
 let preferences=app.state::<dgos_db::CliRepository>().get(context.user_id()).await?;
 input.options.executable=if handoff.target_engine=="codex"{preferences.paths.codex}else{preferences.paths.claude};
 let result=manager.accept_development_handoff(context.user_id(),input,lease.clone()).await;
 if lease.is_cancelled(){return Err("session_invalid".into());}result
}
