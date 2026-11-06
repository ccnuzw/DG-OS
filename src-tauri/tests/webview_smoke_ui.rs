// FR-005 Phase 8: shipped pages in a visible 800x600 WKWebView (theme x density x route matrix).
// `harness = false` keeps the event loop on the main thread, which macOS requires. Run with:
//   npm run build && cargo test -p dgos --features webview-smoke --test webview_smoke_ui -- --nocapture
// Focused Modal viewport regression (visible macOS WKWebView, fresh dist required):
//   DGOS_UI_MODAL_ONLY=1 cargo test -p dgos --features webview-smoke --test webview_smoke_ui -- --nocapture
// Runs 520x600 at 175%, then 800x600 at 100% and 175%; skips the route/theme matrix.
fn main() {
    dgos_lib::smoke::run_ui();
}
