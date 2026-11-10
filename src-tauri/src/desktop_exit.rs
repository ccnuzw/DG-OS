//! A window-bound save/confirm barrier before user-requested application shutdown.
//! Forced termination cannot run this handshake. Programmatic exits retain the
//! existing lifecycle path, including isolated smoke hosts without a frontend.
use serde::Serialize;
use std::{sync::Mutex, time::{Duration, Instant}};
use tauri::{Emitter, Manager};

pub(crate) const QUIT_MENU_ID: &str = "dgos-save-and-quit";
const REPLY_WINDOW: Duration = Duration::from_secs(60);
#[derive(Clone, Serialize)]
pub(crate) struct PrepareExit { request_id: String }
struct Pending { request_id: String, window: String, expires: Instant }
#[derive(Default)]
pub(crate) struct ExitCoordinator { pending: Mutex<Option<Pending>> }
impl ExitCoordinator {
    fn begin(&self, window: &str, now: Instant) -> Option<PrepareExit> {
        let mut pending = self.pending.lock().unwrap_or_else(|error| error.into_inner());
        if pending.as_ref().is_some_and(|request| request.expires > now) { return None; }
        let request_id = uuid::Uuid::new_v4().to_string();
        *pending = Some(Pending { request_id: request_id.clone(), window: window.into(), expires: now + REPLY_WINDOW });
        Some(PrepareExit { request_id })
    }
    pub(crate) fn complete(&self, window: &str, request_id: &str, proceed: bool) -> Result<bool, String> {
        self.complete_at(window, request_id, proceed, Instant::now())
    }
    fn complete_at(&self, window: &str, request_id: &str, proceed: bool, now: Instant) -> Result<bool, String> {
        let mut pending = self.pending.lock().unwrap_or_else(|error| error.into_inner());
        let request = pending.as_ref().ok_or("exit_not_pending")?;
        if request.expires <= now { *pending = None; return Err("exit_request_expired".into()); }
        if request.window != window || request.request_id != request_id { return Err("exit_request_mismatch".into()); }
        *pending = None;
        Ok(proceed)
    }
}
/// Returns false only when no main WebView exists to save. A missing listener,
/// delivery failure or rejected save never authorizes exit while the view exists.
pub(crate) fn request(app: &tauri::AppHandle) -> bool {
    let Some(window) = app.get_webview_window("main") else { return false; };
    let Some(coordinator) = app.try_state::<ExitCoordinator>() else { return false; };
    if let Some(payload) = coordinator.begin(window.label(), Instant::now()) {
        let _ = window.show();
        let _ = window.set_focus();
        if window.emit("development:prepare-exit", &payload).is_err() {
            let _ = coordinator.complete(window.label(), &payload.request_id, false);
        }
    }
    true
}
pub(crate) fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    app.manage(ExitCoordinator::default());
    #[cfg(target_os = "macos")]
    {
        use tauri::menu::{Menu, MenuItem, MenuItemKind};
        // Tauri's default macOS application submenu ends in native Quit. Keep
        // all its other standard items, but replace that direct terminate action.
        let menu = Menu::default(app)?;
        let Some(MenuItemKind::Submenu(application)) = menu.items()?.into_iter().next() else {
            return Err(tauri::Error::Anyhow(anyhow::anyhow!("default application menu unavailable")));
        };
        let items = application.items()?;
        let Some(MenuItemKind::Predefined(quit)) = items.last() else {
            return Err(tauri::Error::Anyhow(anyhow::anyhow!("default Quit menu unavailable")));
        };
        let replacement = MenuItem::with_id(app, QUIT_MENU_ID, quit.text()?, true, Some("CmdOrCtrl+Q"))?;
        application.remove_at(items.len() - 1)?;
        application.append(&replacement)?;
        app.set_menu(menu)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exit_confirmation_is_one_shot_window_bound_and_rejection_never_exits() {
        let state=ExitCoordinator::default();let now=Instant::now();
        let request=state.begin("main",now).unwrap();
        assert!(state.begin("main",now).is_none());
        assert_eq!(state.complete_at("other",&request.request_id,true,now).unwrap_err(),"exit_request_mismatch");
        assert_eq!(state.complete_at("main","old-request",true,now).unwrap_err(),"exit_request_mismatch");
        assert!(!state.complete_at("main",&request.request_id,false,now).unwrap());
        assert_eq!(state.complete_at("main",&request.request_id,true,now).unwrap_err(),"exit_not_pending");
        let next=state.begin("main",now).unwrap();
        assert!(state.complete_at("main",&next.request_id,true,now).unwrap());
        assert!(state.complete_at("main",&next.request_id,true,now).is_err());
    }
    #[test]
    fn missing_frontend_reply_expires_closed_without_waiting_or_authorizing_exit() {
        let state=ExitCoordinator::default();let now=Instant::now();
        let request=state.begin("main",now).unwrap();
        assert_eq!(state.complete_at("main",&request.request_id,true,now+REPLY_WINDOW).unwrap_err(),"exit_request_expired");
        let next=state.begin("main",now+REPLY_WINDOW).unwrap();
        assert_ne!(request.request_id,next.request_id);
    }
}
