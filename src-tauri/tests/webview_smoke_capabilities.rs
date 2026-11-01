// Real module JS calling the D093 storage, network, MCP and Skills capabilities, plus the module
// management page, in a real WebView (V1-FR-009). `harness = false` keeps the event loop on the main
// thread, which macOS requires. Run with:
//   npm run smoke:tauri
fn main() {
    dgos_lib::smoke::run_capabilities();
}
