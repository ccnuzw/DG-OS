// V1-E2E-10 (macOS local): the real /settings/lan page and the real LAN server with the shipped remote SPA in
// separate incognito WebViews, across three real process starts over one data directory. `harness = false`
// keeps the event loop on the main thread, which macOS requires. Run with:
//   npm run smoke:tauri:lan
fn main() {
    dgos_lib::smoke::run_lan();
}
