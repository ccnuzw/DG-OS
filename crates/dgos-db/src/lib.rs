// DG-OS Database Layer
// Database entities, migrations, and repository patterns

pub mod entities;
pub mod migrations;
pub mod repositories;

use sea_orm::{Database, DatabaseConnection, DbErr};

pub async fn establish_connection(database_url: &str) -> Result<DatabaseConnection, DbErr> {
    Database::connect(database_url).await
}

pub use sea_orm_migration;

// Re-export commonly used types
pub use entities::prelude::*;
pub use repositories::*;
pub use sea_orm;
