// File-backed schema2..5 -> schema6 migration and actual macOS NSEvent layout-scale shortcuts in the
// shipped WKWebView: draft/cancel, auto-CAS, fresh-process persistence, Slider, repeat/bounds,
// unchanged focused text/fonts/pageZoom, and actual AppKit menu accelerators. No synthetic DOM keys.
// `harness = false` keeps the macOS event loop on the main thread. Run from the repository root:
//   npm run build && cargo test -p dgos --features webview-smoke --test webview_smoke_appearance_host -- --nocapture
fn main() {
    dgos_lib::smoke::run_appearance_host();
}
