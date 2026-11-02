#[cfg(all(not(target_os = "windows"), not(target_os = "linux")))]
fn main() {
    dgos_lib::smoke::run_mcp();
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn main() {
    panic!("WebView smoke tests only run on macOS in this build");
}
