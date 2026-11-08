//! Capture only the smoke's actual main NSWindow, never the user's whole desktop.
use std::path::Path;
use tauri::AppHandle;

#[cfg(target_os = "macos")]
pub(super) async fn capture_native(app: &AppHandle, root: &Path, name: &str) -> Result<String, String> {
    use objc2::{msg_send, runtime::AnyObject};
    use tauri::Manager;
    let window = app.get_webview_window("main").ok_or("main window absent")?;
    window.show().map_err(|e| e.to_string())?;
    let (send, receive) = tokio::sync::oneshot::channel();
    let target = window.clone();
    window.run_on_main_thread(move || {
        let number = target.ns_window().map_err(|e| e.to_string()).and_then(|pointer| {
            let native = pointer as *mut AnyObject;
            if native.is_null() { return Err("native target window absent".into()); }
            let number: isize = unsafe { msg_send![native, windowNumber] };
            if number > 0 { Ok(number) } else { Err("native target window number absent".into()) }
        });
        let _ = send.send(number);
    }).map_err(|e| e.to_string())?;
    let number = tokio::time::timeout(std::time::Duration::from_secs(10), receive).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())??;
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    let path = root.join(format!("{name}.png"));
    let status = std::process::Command::new("/usr/sbin/screencapture").args(["-x", "-o", "-l"]).arg(number.to_string()).arg(&path).status().map_err(|e| e.to_string())?;
    if !status.success() { return Err(format!("native target-window screencapture failed: {status}")); }
    let evidence = serde_json::json!({"png":path,"window_label":"main","window_number":number,"window_number_source":"actual target NSWindow.windowNumber via run_on_main_thread","process_id":std::process::id(),"capture":"/usr/sbin/screencapture -x -o -l <window_number>"});
    std::fs::write(path.with_extension("capture.json"), serde_json::to_vec_pretty(&evidence).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn capture_native(_: &AppHandle, _: &Path, _: &str) -> Result<String, String> {
    Err("native window capture requires macOS WKWebView".into())
}
