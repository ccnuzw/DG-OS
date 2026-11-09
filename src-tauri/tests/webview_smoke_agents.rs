//! Safe targeted invocation (macOS desktop, production dist built first):
//! DGOS_PROJECT_ACCEPTANCE_PHASE=phase6-local
//! DGOS_PROJECT_ACCEPTANCE_ROOT=/private/tmp/dgos-phase6-local-<unique>
//! cargo test --offline -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture
//! The root must not exist; other phases may intentionally execute real engines.
fn main() { dgos_lib::smoke::run_agents(); }
