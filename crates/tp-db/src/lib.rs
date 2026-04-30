pub mod commands;

pub use commands::{CommandRecord, PinnedCommand};
pub mod metadata_entries;
pub mod notes;
pub mod output;
pub mod preferences;
pub mod schema;
pub mod workspace_notes;
pub mod workspaces;

use anyhow::Result;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Database handle wrapping SQLite connection.
pub struct Database {
    pub conn: Connection,
}

impl Database {
    /// Open or create the database at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let db = Self { conn };
        schema::run_migrations(&db)?;
        Ok(db)
    }

    /// Open an in-memory database (for testing).
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let db = Self { conn };
        schema::run_migrations(&db)?;
        Ok(db)
    }

    /// Default database path: ~/.local/share/pax/pax.db
    pub fn default_path() -> PathBuf {
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("pax");
        std::fs::create_dir_all(&data_dir).ok();
        data_dir.join("pax.db")
    }
}
