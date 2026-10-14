//! The SQLite database, and conversions of what it stores.

use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use time::OffsetDateTime;

/// Opens the database at `path`, creating it if needed, and migrates it.
pub async fn open(path: &std::path::Path) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    connect(options, 8).await
}

/// A fresh database in memory, for tests.
pub async fn in_memory() -> anyhow::Result<SqlitePool> {
    // One connection, since each in-memory connection is its own database.
    connect(SqliteConnectOptions::from_str("sqlite::memory:")?, 1).await
}

async fn connect(options: SqliteConnectOptions, max: u32) -> anyhow::Result<SqlitePool> {
    let options = options
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(max)
        .connect_with(options)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Milliseconds since the Unix epoch, as the database keeps times.
pub fn now() -> i64 {
    to_millis(OffsetDateTime::now_utc())
}

pub fn to_millis(t: OffsetDateTime) -> i64 {
    (t.unix_timestamp_nanos() / 1_000_000) as i64
}

pub fn from_millis(ms: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

/// Whether an error is the violation of a unique constraint.
pub fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(e) if e.is_unique_violation())
}
