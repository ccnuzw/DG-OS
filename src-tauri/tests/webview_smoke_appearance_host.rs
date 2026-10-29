// FR-005 Phase 3: appearance preferences through the real host commands and a real SQLite file in a
// real WebView. `harness = false` keeps the event loop on the main thread, which macOS requires. Run with:
//   npm run build && cargo test -p dgos --features webview-smoke --test webview_smoke_appearance_host -- --nocapture
fn main() {
    dgos_lib::smoke::run_appearance_host();
}
