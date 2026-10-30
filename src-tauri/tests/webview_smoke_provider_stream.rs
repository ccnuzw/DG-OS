// Real call_provider_stream / cancel_provider_stream over a real Tauri Channel in a real WebView
// (FR-001 AC33). `harness = false` keeps the event loop on the main thread, which macOS requires.
// Run with:
//   npm run smoke:tauri
fn main() {
    dgos_lib::smoke::run_provider_stream();
}
