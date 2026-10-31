// Real /workbench chat UI over a real Tauri Channel in a real WebView against a loopback scripted SSE
// upstream (V1-E2E-06, fixture level). `harness = false` keeps the event loop on the main thread, which
// macOS requires. Run with:
//   npm run smoke:tauri:chat
fn main() {
    dgos_lib::smoke::run_chat();
}
