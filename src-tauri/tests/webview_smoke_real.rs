// Real module execution in a real WebView (D042-D046). `harness = false` keeps
// the event loop on the main thread, which macOS requires. Run with:
//   npm run smoke:tauri
fn main() {
    dgos_lib::smoke::run_real();
}
