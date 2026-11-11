// DG-OS Database Layer
// Database entities, migrations, and repository patterns

pub mod entities;
pub mod migrations;
pub mod repositories;
pub mod startup;

#[cfg(test)]
mod ai_content_tests;
#[cfg(test)]
mod ai_run_tests;
#[cfg(test)]
mod cli_cutover_tests;
#[cfg(test)]
mod appearance_tests;
#[cfg(test)]
mod desktop_tests;
#[cfg(test)]
mod chat_tests;
#[cfg(test)]
mod chat_usage_tests;
#[cfg(test)]
mod lan_audit_tests;
#[cfg(test)]
mod mcp_tests;
#[cfg(test)]
mod module_permission_tests;
#[cfg(test)]
mod provider_scope_tests;
#[cfg(test)]
mod user_scope_tests;
mod workbench_tool_tests;

/// Log targets that must never emit below `warn`. sea-orm records every executed statement, bound
/// values included, in `trace`-level spans, so `RUST_LOG=trace` would write chat text, titles and
/// search queries to the log (D072, AC24). Subscribers enforce this independently of user
/// directives and check it before applying verbosity. The host requests dynamic callsite
/// interests so another subscriber cannot cache an unconditional enable for its privacy filter.
/// Disabling SQLx statement logging alone would leave sea-orm's statement spans enabled.
pub const QUIET_LOG_TARGETS: [&str; 2] = ["sea_orm", "sqlx"];

use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr};

/// Every connection the application opens goes through here. `secure_delete = ON` zeroes freed
/// database pages so deleted chat text is not left in the main file (D072, D077). It is a
/// per-connection setting, so it must be applied to each pool; it is not a promise of physical
/// erasure (journal files and backups are outside it).
pub async fn establish_connection(database_url: &str) -> Result<DatabaseConnection, DbErr> {
    let mut options = ConnectOptions::new(database_url);
    options.map_sqlx_sqlite_opts(|sqlite| sqlite.pragma("secure_delete", "ON"));
    Database::connect(options).await
}

pub use sea_orm_migration;

// Re-export commonly used types
pub use entities::prelude::*;
pub use repositories::*;
pub use sea_orm;
