// FR-005 Phase 1 token calibration in a real WebView. `harness = false` keeps the event loop on the
// main thread, which macOS requires. Run with:
//   npm run build && cargo test -p dgos --features webview-smoke --test webview_smoke_appearance -- --nocapture
fn main() {
    dgos_lib::smoke::run_appearance();
}
