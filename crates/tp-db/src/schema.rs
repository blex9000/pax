use anyhow::Result;
use rusqlite::OptionalExtension;

use crate::Database;

/// Run all database migrations.
pub fn run_migrations(db: &Database) -> Result<()> {
    db.conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT DEFAULT (datetime('now'))
        );",
    )?;

    let applied: Vec<String> = {
        let mut stmt = db
            .conn
            .prepare("SELECT name FROM _migrations ORDER BY id")?;
        let result = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        result
    };

    apply_sql_migration(db, &applied, "001_initial", MIGRATION_001)?;
    apply_sql_migration(db, &applied, "002_fts5", MIGRATION_002)?;
    ensure_workspace_metadata_key_migration(db, &applied)?;
    apply_sql_migration(db, &applied, "004_app_preferences", MIGRATION_004)?;

    Ok(())
}

fn apply_sql_migration(db: &Database, applied: &[String], name: &str, sql: &str) -> Result<()> {
    if !applied.iter().any(|entry| entry == name) {
        db.conn.execute_batch(sql)?;
        record_migration(db, name)?;
    }
    Ok(())
}

fn record_migration(db: &Database, name: &str) -> Result<()> {
    db.conn
        .execute("INSERT INTO _migrations (name) VALUES (?1)", [name])?;
    Ok(())
}

fn ensure_workspace_metadata_key_migration(db: &Database, applied: &[String]) -> Result<()> {
    const NAME: &str = "003_workspace_metadata_key";

    let has_workspace_metadata = table_exists(db, "workspace_metadata")?;
    let has_workspace_metadata_old = table_exists(db, "workspace_metadata_old")?;
    let workspace_has_record_key =
        has_workspace_metadata && table_has_column(db, "workspace_metadata", "record_key")?;

    if has_workspace_metadata_old {
        let legacy_rows = load_legacy_workspace_rows(db)?;

        if !workspace_has_record_key {
            db.conn.execute_batch(
                "DROP TABLE IF EXISTS workspace_metadata;
                 CREATE TABLE workspace_metadata (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    config_path TEXT,
                    record_key TEXT NOT NULL UNIQUE,
                    last_opened TEXT DEFAULT (datetime('now')),
                    open_count INTEGER DEFAULT 1
                 );",
            )?;
        }

        for (name, config_path, record_key, last_opened, open_count) in legacy_rows {
            db.conn.execute(
                "INSERT INTO workspace_metadata (name, config_path, record_key, last_opened, open_count)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(record_key) DO UPDATE SET
                    name = excluded.name,
                    config_path = excluded.config_path,
                    last_opened = excluded.last_opened,
                    open_count = CASE
                        WHEN workspace_metadata.open_count > excluded.open_count THEN workspace_metadata.open_count
                        ELSE excluded.open_count
                    END",
                rusqlite::params![name, config_path, record_key, last_opened, open_count],
            )?;
        }

        db.conn.execute_batch(
            "DROP TABLE IF EXISTS workspace_metadata_old;
             CREATE INDEX IF NOT EXISTS idx_workspace_last_opened ON workspace_metadata(last_opened);",
        )?;

        if !applied.iter().any(|entry| entry == NAME) {
            record_migration(db, NAME)?;
        }
        return Ok(());
    }

    if workspace_has_record_key {
        db.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_workspace_last_opened ON workspace_metadata(last_opened);",
        )?;
        if !applied.iter().any(|entry| entry == NAME) {
            record_migration(db, NAME)?;
        }
        return Ok(());
    }

    if !has_workspace_metadata && !has_workspace_metadata_old {
        db.conn.execute_batch(
            "CREATE TABLE workspace_metadata (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                config_path TEXT,
                record_key TEXT NOT NULL UNIQUE,
                last_opened TEXT DEFAULT (datetime('now')),
                open_count INTEGER DEFAULT 1
            );
            CREATE INDEX IF NOT EXISTS idx_workspace_last_opened ON workspace_metadata(last_opened);",
        )?;
        if !applied.iter().any(|entry| entry == NAME) {
            record_migration(db, NAME)?;
        }
        return Ok(());
    }

    if has_workspace_metadata {
        db.conn
            .execute_batch("ALTER TABLE workspace_metadata RENAME TO workspace_metadata_old;")?;
        return ensure_workspace_metadata_key_migration(db, applied);
    }

    if !applied.iter().any(|entry| entry == NAME) {
        record_migration(db, NAME)?;
    }

    Ok(())
}

fn table_exists(db: &Database, table_name: &str) -> Result<bool> {
    Ok(db
        .conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            [table_name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn table_has_column(db: &Database, table_name: &str, column_name: &str) -> Result<bool> {
    let pragma = format!("PRAGMA table_info({table_name})");
    let mut stmt = db.conn.prepare(&pragma)?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for column in columns {
        if column? == column_name {
            return Ok(true);
        }
    }
    Ok(false)
}

fn load_legacy_workspace_rows(
    db: &Database,
) -> Result<Vec<(String, Option<String>, String, String, i64)>> {
    let mut stmt = db
        .conn
        .prepare("SELECT name, config_path, last_opened, open_count FROM workspace_metadata_old")?;
    let rows = stmt.query_map([], |row| {
        let name: String = row.get(0)?;
        let config_path: Option<String> = row.get(1)?;
        let last_opened: String = row.get(2)?;
        let open_count: i64 = row.get(3)?;
        let record_key = config_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(|path| format!("path:{path}"))
            .unwrap_or_else(|| format!("name:{name}"));
        Ok((name, config_path, record_key, last_opened, open_count))
    })?;

    Ok(rows.filter_map(|row| row.ok()).collect())
}

const MIGRATION_001: &str = "
CREATE TABLE IF NOT EXISTS command_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_name TEXT,
    panel_id TEXT,
    command TEXT NOT NULL,
    executed_at TEXT DEFAULT (datetime('now')),
    exit_code INTEGER
);

CREATE TABLE IF NOT EXISTS saved_output (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_name TEXT,
    panel_id TEXT NOT NULL,
    content TEXT NOT NULL,
    saved_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS workspace_metadata (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    config_path TEXT,
    last_opened TEXT DEFAULT (datetime('now')),
    open_count INTEGER DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_cmd_workspace ON command_history(workspace_name);
CREATE INDEX IF NOT EXISTS idx_cmd_executed ON command_history(executed_at);
CREATE INDEX IF NOT EXISTS idx_output_panel ON saved_output(panel_id);
";

const MIGRATION_002: &str = "
CREATE VIRTUAL TABLE IF NOT EXISTS command_history_fts USING fts5(
    command,
    content='command_history',
    content_rowid='id'
);

CREATE TRIGGER IF NOT EXISTS command_history_ai AFTER INSERT ON command_history BEGIN
    INSERT INTO command_history_fts(rowid, command) VALUES (new.id, new.command);
END;

CREATE VIRTUAL TABLE IF NOT EXISTS saved_output_fts USING fts5(
    content,
    content='saved_output',
    content_rowid='id'
);

CREATE TRIGGER IF NOT EXISTS saved_output_ai AFTER INSERT ON saved_output BEGIN
    INSERT INTO saved_output_fts(rowid, content) VALUES (new.id, new.content);
END;
";

const MIGRATION_004: &str = "
CREATE TABLE IF NOT EXISTS app_preferences (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT DEFAULT (datetime('now'))
);
";

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_db_without_running_migrations() -> Database {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE _migrations (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT DEFAULT (datetime('now'))
            );",
        )
        .unwrap();
        Database { conn }
    }

    #[test]
    fn migration_003_upgrades_old_workspace_metadata_schema() {
        let db = setup_db_without_running_migrations();
        db.conn.execute_batch(MIGRATION_001).unwrap();
        record_migration(&db, "001_initial").unwrap();
        db.conn.execute_batch(MIGRATION_002).unwrap();
        record_migration(&db, "002_fts5").unwrap();
        db.conn
            .execute(
                "INSERT INTO workspace_metadata (name, config_path, last_opened, open_count)
                 VALUES (?1, ?2, datetime('now'), 3)",
                ["/tmp/demo", "/tmp/demo.json"],
            )
            .unwrap();

        run_migrations(&db).unwrap();

        assert!(table_has_column(&db, "workspace_metadata", "record_key").unwrap());
        assert!(!table_exists(&db, "workspace_metadata_old").unwrap());
        let record_key: String = db
            .conn
            .query_row(
                "SELECT record_key FROM workspace_metadata WHERE config_path = '/tmp/demo.json'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(record_key, "path:/tmp/demo.json");
    }

    #[test]
    fn migration_003_recovers_from_leftover_workspace_metadata_old_table() {
        let db = setup_db_without_running_migrations();
        db.conn
            .execute_batch(
                "CREATE TABLE workspace_metadata (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    config_path TEXT,
                    record_key TEXT NOT NULL UNIQUE,
                    last_opened TEXT DEFAULT (datetime('now')),
                    open_count INTEGER DEFAULT 1
                );
                CREATE TABLE workspace_metadata_old (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL UNIQUE,
                    config_path TEXT,
                    last_opened TEXT DEFAULT (datetime('now')),
                    open_count INTEGER DEFAULT 1
                );",
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO workspace_metadata_old (name, config_path, last_opened, open_count)
                 VALUES ('legacy', '/tmp/legacy.json', datetime('now'), 7)",
                [],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO workspace_metadata (name, config_path, record_key, last_opened, open_count)
                 VALUES ('current', '/tmp/current.json', 'path:/tmp/current.json', datetime('now'), 2)",
                [],
            )
            .unwrap();

        run_migrations(&db).unwrap();

        assert!(table_has_column(&db, "workspace_metadata", "record_key").unwrap());
        assert!(!table_exists(&db, "workspace_metadata_old").unwrap());
        let rows: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM workspace_metadata", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 2);
        let legacy_count: i64 = db
            .conn
            .query_row(
                "SELECT open_count FROM workspace_metadata WHERE config_path = '/tmp/legacy.json'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(legacy_count, 7);
        let applied = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM _migrations WHERE name = '003_workspace_metadata_key'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(applied, 1);
    }

    #[test]
    fn migration_004_creates_app_preferences_table() {
        let db = setup_db_without_running_migrations();

        run_migrations(&db).unwrap();

        assert!(table_exists(&db, "app_preferences").unwrap());
        let applied = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM _migrations WHERE name = '004_app_preferences'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(applied, 1);
    }
}
