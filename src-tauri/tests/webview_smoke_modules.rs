// Module management, hot update, crash handling and startup load in a real
// WebView (D055). `harness = false` keeps the event loop on the main thread,
// which macOS requires. Run with:
//   npm run smoke:tauri
fn main() {
    dgos_lib::smoke::run_modules();
}
