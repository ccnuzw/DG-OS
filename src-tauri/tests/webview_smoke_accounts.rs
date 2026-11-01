// V1-FR-008: local accounts through the real host commands, session resolver and SQLite file in a
// real WebView. `harness = false` keeps the event loop on the main thread, which macOS requires. Run with:
//   npm run build && cargo test -p dgos --features webview-smoke --test webview_smoke_accounts -- --nocapture
fn main() {
    dgos_lib::smoke::run_accounts();
}
