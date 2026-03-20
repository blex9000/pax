use anyhow::Result;

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
        let mut stmt = db.conn.prepare("SELECT name FROM _migrations ORDER BY id")?;
        let result = stmt.query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        result
    };

    let migrations: Vec<(&str, &str)> = vec![
        ("001_initial", MIGRATION_001),
        ("002_fts5", MIGRATION_002),
    ];

    for (name, sql) in migrations {
        if !applied.contains(&name.to_string()) {
            db.conn.execute_batch(sql)?;
            db.conn.execute(
                "INSERT INTO _migrations (name) VALUES (?1)",
                [name],
            )?;
        }
    }

    Ok(())
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
