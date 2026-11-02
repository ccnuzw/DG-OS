//! Opening the application database at startup (V1-FR-008 §10.3, D098 item 9, D116).
//!
//! A database that cannot be opened or migrated is reported, never repaired: the file is left
//! exactly as found. The reason is a bounded single line with the file path removed, so it can be
//! shown next to the path without repeating it; it never carries row data (SQLite and
//! sea-orm-migration errors name statements and schema objects, not bound values).

use crate::migrations::Migrator;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement};
use sea_orm_migration::MigratorTrait;
use std::path::Path;

/// Longest reason shown, in characters; a longer one ends with `…`.
const REASON_MAX_CHARS: usize = 300;

/// The `sqlite://` URL every pool for this file uses (`rwc`: create when missing).
pub fn database_url(path: &Path) -> String {
    format!("sqlite://{}?mode=rwc", path.display())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenStage {
    /// Connecting: the file is missing and cannot be created, unreadable, or not a database.
    Open,
    /// The schema migrations did not complete.
    Migrate,
}

impl OpenStage {
    pub fn as_str(self) -> &'static str {
        match self {
            OpenStage::Open => "open",
            OpenStage::Migrate => "migrate",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenFailure {
    pub stage: OpenStage,
    pub reason: String,
}

/// Opens `path` and runs every pending migration. On failure nothing is retried, deleted,
/// renamed or rewritten; the caller shows the failure and the path.
pub async fn open_database(path: &Path) -> Result<DatabaseConnection, OpenFailure> {
    let fail = |stage, error: DbErr| OpenFailure {
        stage,
        reason: reason(&error, path),
    };
    let db = crate::establish_connection(&database_url(path))
        .await
        .map_err(|error| fail(OpenStage::Open, error))?;
    // SQLite reads the header lazily: a file that is not a database only fails on the first
    // statement. Read the schema here so that failure is reported as opening, not migrating.
    db.query_one(Statement::from_string(
        db.get_database_backend(),
        "SELECT count(*) FROM sqlite_master",
    ))
    .await
    .map_err(|error| fail(OpenStage::Open, error))?;
    Migrator::up(&db, None)
        .await
        .map_err(|error| fail(OpenStage::Migrate, error))?;
    Ok(db)
}

/// One line, at most `REASON_MAX_CHARS` characters, with the URL and the path replaced.
fn reason(error: &DbErr, path: &Path) -> String {
    let path = path.display().to_string();
    let text = error
        .to_string()
        .replace(&database_url(Path::new(&path)), "<database file>")
        .replace(&path, "<database file>");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    if text.chars().count() <= REASON_MAX_CHARS {
        return text;
    }
    let mut short: String = text.chars().take(REASON_MAX_CHARS).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_safe(failure: &OpenFailure, path: &Path) {
        assert!(!failure.reason.trim().is_empty());
        assert!(failure.reason.chars().count() <= REASON_MAX_CHARS + 1);
        assert!(!failure.reason.chars().any(char::is_control));
        assert!(!failure.reason.contains(&path.display().to_string()));
        assert!(!failure.reason.contains("sqlite://"));
    }

    #[tokio::test]
    async fn a_file_that_cannot_be_created_fails_at_the_open_stage() {
        let dir = tempfile::tempdir().unwrap();
        // The parent is a regular file, so SQLite can neither open nor create the database.
        let blocker = dir.path().join("not-a-directory");
        std::fs::write(&blocker, b"x").unwrap();
        let path = blocker.join("dgos.db");
        let failure = open_database(&path).await.unwrap_err();
        assert_eq!(failure.stage, OpenStage::Open);
        assert_safe(&failure, &path);
    }

    #[tokio::test]
    async fn a_file_that_is_not_a_database_fails_at_the_open_stage_and_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dgos.db");
        let garbage = b"this is not an SQLite database file, it only has a .db name ".repeat(40);
        std::fs::write(&path, &garbage).unwrap();
        let failure = open_database(&path).await.unwrap_err();
        assert_eq!(failure.stage, OpenStage::Open);
        assert_safe(&failure, &path);
        assert_eq!(std::fs::read(&path).unwrap(), garbage);
    }

    #[tokio::test]
    async fn a_migration_failure_reports_the_migrate_stage_and_repairs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dgos.db");
        let db = open_database(&path).await.unwrap();
        // A history row whose migration this build does not have: `up` refuses to continue.
        db.execute(Statement::from_string(
            db.get_database_backend(),
            "INSERT INTO seaql_migrations (version, applied_at) VALUES ('m29991231_000001_from_the_future', 0)",
        ))
        .await
        .unwrap();
        db.close().await.unwrap();

        let failure = open_database(&path).await.unwrap_err();
        assert_eq!(failure.stage, OpenStage::Migrate);
        assert_safe(&failure, &path);

        let db = crate::establish_connection(&database_url(&path))
            .await
            .unwrap();
        let kept = db
            .query_one(Statement::from_string(
                db.get_database_backend(),
                "SELECT COUNT(*) AS n FROM seaql_migrations WHERE version = 'm29991231_000001_from_the_future'",
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "n")
            .unwrap();
        assert_eq!(kept, 1, "the failing history row must not be removed");
    }

    #[test]
    fn the_reason_is_one_bounded_line_without_the_path() {
        let path = Path::new("/Users/someone/Library/Application Support/com.dgos.app/dgos.db");
        let error = DbErr::Custom(format!(
            "cannot open sqlite://{}?mode=rwc\nsecond line\t{}",
            path.display(),
            "x".repeat(2000)
        ));
        let text = reason(&error, path);
        assert!(!text.contains("someone"));
        assert!(!text.contains('\n') && !text.contains('\t'));
        assert!(text.chars().count() <= REASON_MAX_CHARS + 1);
        assert!(text.starts_with("Custom Error: cannot open <database file>"));
    }
}
