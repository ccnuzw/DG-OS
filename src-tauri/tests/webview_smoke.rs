// Real-WebView smoke entry point (D035). `harness = false` keeps the event
// loop on the main thread, which macOS requires. Run with:
//   npm run smoke:tauri
fn main() {
    dgos_lib::smoke::run();
}
