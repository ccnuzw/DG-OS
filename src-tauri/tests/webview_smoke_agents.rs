//! Safe targeted invocations (macOS desktop, production dist built first).
//! Local fixture lifecycle (never a real model):
//! DGOS_PROJECT_ACCEPTANCE_PHASE=phase6-local
//! DGOS_PROJECT_ACCEPTANCE_ROOT=/private/tmp/dgos-phase6-local-<unique>
//! cargo test --offline -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture
//! The root must not exist; other phases may intentionally execute real engines.
//! `DGOS_CODEX_LIFECYCLE_SMOKE=1` adds configure/prepare, high reasoning/read-only
//! settings through production popovers, explicit Plan opt-in, fixture approval
//! denial, steering, interrupt, resend, close and resume. It uses a codex-cli
//! 0.167.0 protocol fixture with model/collaboration metadata, not a real model.
//! Real authorized Codex lifecycle (no protocol fixture; only Codex is selected):
//! DGOS_CODEX_REAL_LIFECYCLE_SMOKE=1 DGOS_PROJECT_ACCEPTANCE_PHASE=phase6-local
//! DGOS_PROJECT_ACCEPTANCE_ROOT=/private/tmp/dgos-real-codex-<unique>
//! DGOS_STAGE6_CODEX_URL=<authorized endpoint> DGOS_STAGE6_CODEX_KEY=<authorized key>
//! DGOS_STAGE6_CODEX_MODEL=<authorized model>
//! cargo test --offline -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture
//! The real root must not exist and the installed CLI must report exactly
//! `codex-cli 0.167.0`. HOME/CODEX_HOME/config/cache/data and the DB are isolated;
//! credentials are memory-vault-only. The journey does not run Claude, does not
//! read production account/config/vault state, and never logs raw keys or model
//! transcript/protocol. Evidence is phase6-real-codex-*.json plus content-masked
//! actual target-window captures before first send and after resume.
fn main() { dgos_lib::smoke::run_agents(); }
