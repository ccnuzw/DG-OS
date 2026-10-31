// DG-OS Database Layer
// Database entities, migrations, and repository patterns

pub mod entities;
pub mod migrations;
pub mod repositories;

#[cfg(test)]
mod appearance_tests;
#[cfg(test)]
mod chat_tests;
#[cfg(test)]
mod mcp_tests;
#[cfg(test)]
mod module_permission_tests;
#[cfg(test)]
mod provider_scope_tests;
mod workbench_tool_tests;

/// Log targets that must never emit below `warn`. sea-orm records every executed statement, bound
/// values included, in `trace`-level spans, so `RUST_LOG=trace` would write chat text, titles and
/// search queries to the log (D072, AC24). Subscribers apply this as a separate `Targets` filter
/// AND-ed with the user's filter, not as extra `RUST_LOG` directives: `EnvFilter` lets a more
/// specific target (`sea_orm::driver=trace`) win over `sea_orm=warn`, so a directive cap can be
/// lifted by a debugging session.
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
