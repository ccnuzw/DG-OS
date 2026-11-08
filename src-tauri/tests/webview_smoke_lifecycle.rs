// Standalone feature-gated host: macOS requires the WebView event loop on main.
// cargo test --offline --manifest-path src-tauri/Cargo.toml --features webview-smoke --test webview_smoke_lifecycle
fn main() {
    if std::env::args().any(|argument| argument == "--lifecycle-child") {
        dgos_lib::smoke::run_lifecycle();
    } else {
        dgos_lib::smoke::drive_lifecycle();
    }
}
