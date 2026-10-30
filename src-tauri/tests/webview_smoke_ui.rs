// FR-005 Phase 8: shipped pages in a visible 800x600 WKWebView (theme x density x route matrix).
// `harness = false` keeps the event loop on the main thread, which macOS requires. Run with:
//   npm run build && cargo test -p dgos --features webview-smoke --test webview_smoke_ui -- --nocapture
fn main() {
    dgos_lib::smoke::run_ui();
}
