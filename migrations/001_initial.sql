-- Initial schema for myterms database
-- This file is kept for reference; actual migrations run from Rust code.

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

-- FTS5 for full-text search
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
