// Default: original W4 compatibility evidence. Set DGOS_MCP_SMOKE_MODE=page for
// independent production MCP page acceptance (mcp_page.rs documents prerequisites).
// Requires --features mcp-smoke; page evidence is retained in a fresh /tmp root.
#[cfg(all(not(target_os = "windows"), not(target_os = "linux")))]
fn main() {
    dgos_lib::smoke::run_mcp();
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn main() {
    panic!("WebView smoke tests only run on macOS in this build");
}
