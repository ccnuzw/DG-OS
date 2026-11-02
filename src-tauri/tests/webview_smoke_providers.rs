// Real-WebView smoke for Provider CRUD operations (FR-001 AC01, AC03, AC05, AC10, AC12, AC13, AC14).
// Run with: cargo test --offline -p dgos --features webview-smoke --test webview_smoke_providers
#[cfg(all(not(target_os = "windows"), not(target_os = "linux")))]
fn main() {
    dgos_lib::smoke::run_providers();
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn main() {
    panic!("WebView smoke tests only run on macOS in this build");
}
