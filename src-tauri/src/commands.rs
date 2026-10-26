// Tauri commands module
// All IPC commands are defined here

#[tauri::command]
pub fn greet(name: &str) -> String {
    format!("Hello, {}! Welcome to DG-OS!", name)
}
