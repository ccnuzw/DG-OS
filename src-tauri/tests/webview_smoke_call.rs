// Real module JS calling providers.call through the typed channel in a real
// WebView (D050, D051). `harness = false` keeps the event loop on the main
// thread, which macOS requires. Run with:
//   npm run smoke:tauri
fn main() {
    dgos_lib::smoke::run_call();
}
