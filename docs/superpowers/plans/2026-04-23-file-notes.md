# File Notes + Generic Workspace Metadata — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add workspace-scoped file notes to the Code Editor, persisted in SQLite via a generic `workspace_file_metadata_entries` table that's shape-stable for future metadata types. Covers per-file ruler + right-click add/edit/delete, per-workspace Notes dialog from the sidebar, and a cross-workspace Metadata Manager from the app menu.

**Architecture:**
1. New migration `005_file_metadata_entries` registered alongside the existing inline migrations in `crates/tp-db/src/schema.rs` (migrations are SQL string constants, not files on disk).
2. Two-layer storage: a generic `metadata_entries` module with CRUD over the raw rows, plus a thin `notes` wrapper that serializes `{"text": ...}` payloads and speaks the `FileNote` struct.
3. Workspace identity flows from `workspace_view` → `CodeEditorPanel` via a new `__workspace_record_key__` extra field, then lives on `EditorState`.
4. Notes state lives per source tab on `SourceTab.notes`, indexed by GTK `TextMark`s that GTK updates in place during edits; save path flushes mark positions + line content back to the DB.
5. Notes ruler is a `gtk::DrawingArea` mirroring `build_match_overview_ruler`, painted to the left of the source scroll.
6. Two dialogs: `WorkspaceNotesDialog` (current-workspace, has Jump) from a new sidebar button; `MetadataManagerDialog` (all-workspaces inspector) from a new app-menu entry.

**Tech Stack:** Rust 2021 · rusqlite (bundled) · gtk4-rs 0.9 · sourceview5 · pax-core / pax-db / pax-gui workspace · existing `task::run_blocking` helper for async DB reads.

**Project conventions:**
- Directory names are `tp-*`, package names are `pax-*`. Use package names with cargo, directory names when editing files.
- Commit after each task with a descriptive message. **Do not** add `Co-Authored-By` trailers.
- **Do not** add unit tests unless explicitly requested. Verification is `cargo build` + manual UI test.
- Named constants for any numeric literal that's not obviously structural.
- Italian docs, English code and commit messages.

**Risk notes:**
- `CodeEditorPanel` today has no notion of which workspace it belongs to. Task 3 threads that through and touches `registry.rs` + `panels/editor/mod.rs` + the factory. Expect regressions in editor opening if this is done wrong.
- `TextMark` lifetime is tied to its buffer. `NotesState` must be owned by `SourceTab` and released with it, otherwise cross-buffer mark references panic.

---

## File structure

New files:
- `crates/tp-db/src/metadata_entries.rs` — generic CRUD over `workspace_file_metadata_entries`.
- `crates/tp-db/src/notes.rs` — `FileNote` struct + note-typed helpers.
- `crates/tp-gui/src/panels/editor/notes_state.rs` — per-source-tab `NotesState` and mark resolution.
- `crates/tp-gui/src/panels/editor/notes_ruler.rs` — note-indicator DrawingArea.
- `crates/tp-gui/src/dialogs/notes_dialog.rs` — `WorkspaceNotesDialog` (per-workspace).
- `crates/tp-gui/src/dialogs/metadata_manager.rs` — `MetadataManagerDialog` (cross-workspace).

Modified files:
- `crates/tp-db/src/lib.rs` — register new modules.
- `crates/tp-db/src/schema.rs` — new `MIGRATION_005_FILE_METADATA` constant + `apply_sql_migration` call.
- `crates/tp-db/src/workspaces.rs` — public helper `compute_record_key(name, config_path)` so GUI code can derive the key without side effects.
- `crates/tp-core/src/workspace.rs` — `Workspace::record_key(config_path)` helper delegating to pax-db.
- `crates/tp-gui/src/panels/registry.rs` — thread `__workspace_record_key__` into CodeEditor panel extras.
- `crates/tp-gui/src/workspace_view.rs` — populate the new extra when building CodeEditor panels.
- `crates/tp-gui/src/panels/editor/mod.rs` — add `record_key: String` to `EditorState`, accept it in `CodeEditorPanel::new`/`new_remote`/`new_with_backend`; register the notes_state module.
- `crates/tp-gui/src/panels/editor/tab_content.rs` — `SourceTab.notes: NotesState`.
- `crates/tp-gui/src/panels/editor/editor_tabs.rs` — context-menu Add/Edit/Delete, async note load after buffer creation, save-path flush, notes ruler in the editor_row.
- `crates/tp-gui/src/panels/editor/file_tree.rs` — Notes button in `actions_bar`.
- `crates/tp-gui/src/dialogs/mod.rs` — register the two new dialogs.
- `crates/tp-gui/src/app.rs` — new `app.workspace_metadata` action + menu entry.

---

## Task 1: DB migration + generic metadata_entries module

**Files:**
- Create: `crates/tp-db/src/metadata_entries.rs`
- Modify: `crates/tp-db/src/lib.rs`
- Modify: `crates/tp-db/src/schema.rs`

- [ ] **Step 1: Register the new module**

Edit `crates/tp-db/src/lib.rs`:

```rust
pub mod commands;
pub mod metadata_entries;
pub mod notes;          // will be filled in Task 2; declare it now so lib.rs stays consistent
pub mod output;
pub mod preferences;
pub mod schema;
pub mod workspaces;
```

Create `crates/tp-db/src/notes.rs` as an empty stub for now:

```rust
//! Note-typed accessors on top of `metadata_entries`. Populated in Task 2.
```

- [ ] **Step 2: Add migration 005 to `schema.rs`**

Open `crates/tp-db/src/schema.rs`. Append to the migration list in `run_migrations`:

```rust
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
    apply_sql_migration(db, &applied, "005_file_metadata", MIGRATION_005_FILE_METADATA)?;

    Ok(())
}
```

Add the constant at the bottom of the file, after `MIGRATION_004`:

```rust
const MIGRATION_005_FILE_METADATA: &str = "
CREATE TABLE IF NOT EXISTS workspace_file_metadata_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    record_key TEXT NOT NULL,
    entry_type TEXT NOT NULL,
    file_path TEXT NOT NULL,
    line_number INTEGER NOT NULL,
    line_anchor TEXT,
    payload TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_wfme_lookup
    ON workspace_file_metadata_entries(record_key, entry_type, file_path);

CREATE INDEX IF NOT EXISTS idx_wfme_by_workspace
    ON workspace_file_metadata_entries(record_key);
";
```

- [ ] **Step 3: Create `metadata_entries.rs`**

Full contents for `crates/tp-db/src/metadata_entries.rs`:

```rust
//! Generic storage for `workspace_file_metadata_entries` — the file-scoped
//! per-workspace metadata table. Entries carry a `entry_type` discriminator
//! (e.g. "note") and an opaque JSON payload; type-specific wrappers (see
//! `crate::notes`) translate between strongly-typed structs and the payload.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use crate::Database;

#[derive(Debug, Clone)]
pub struct MetadataEntry {
    pub id: i64,
    pub record_key: String,
    pub entry_type: String,
    pub file_path: String,
    pub line_number: i32,
    pub line_anchor: Option<String>,
    pub payload: String,
    pub created_at: i64,
    pub updated_at: i64,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Database {
    /// Insert a new metadata entry. Returns the assigned row id.
    pub fn insert_metadata_entry(
        &self,
        record_key: &str,
        entry_type: &str,
        file_path: &str,
        line_number: i32,
        line_anchor: Option<&str>,
        payload: &str,
    ) -> Result<i64> {
        let now = now_secs();
        self.conn.execute(
            "INSERT INTO workspace_file_metadata_entries
                (record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![record_key, entry_type, file_path, line_number, line_anchor, payload, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Update an entry's anchor position (line + anchor text).
    pub fn update_metadata_position(
        &self,
        id: i64,
        line_number: i32,
        line_anchor: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE workspace_file_metadata_entries
             SET line_number = ?2, line_anchor = ?3, updated_at = ?4
             WHERE id = ?1",
            params![id, line_number, line_anchor, now_secs()],
        )?;
        Ok(())
    }

    /// Replace the JSON payload of an entry.
    pub fn update_metadata_payload(&self, id: i64, payload: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE workspace_file_metadata_entries
             SET payload = ?2, updated_at = ?3
             WHERE id = ?1",
            params![id, payload, now_secs()],
        )?;
        Ok(())
    }

    /// Delete a single entry by id. Returns how many rows were removed (0 or 1).
    pub fn delete_metadata_entry(&self, id: i64) -> Result<usize> {
        let n = self
            .conn
            .execute("DELETE FROM workspace_file_metadata_entries WHERE id = ?1", [id])?;
        Ok(n)
    }

    /// Delete every entry belonging to a workspace. Returns the row count.
    pub fn delete_metadata_for_workspace(&self, record_key: &str) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM workspace_file_metadata_entries WHERE record_key = ?1",
            [record_key],
        )?;
        Ok(n)
    }

    /// List entries for a single file in a workspace, filtered by type.
    /// Ordered by `line_number` ascending, then `id` for stable order within
    /// the same line.
    pub fn list_metadata_by_file(
        &self,
        record_key: &str,
        entry_type: &str,
        file_path: &str,
    ) -> Result<Vec<MetadataEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at
             FROM workspace_file_metadata_entries
             WHERE record_key = ?1 AND entry_type = ?2 AND file_path = ?3
             ORDER BY line_number ASC, id ASC",
        )?;
        let rows = stmt
            .query_map(params![record_key, entry_type, file_path], row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// List entries for a whole workspace. `entry_type` filters when Some.
    pub fn list_metadata_for_workspace(
        &self,
        record_key: &str,
        entry_type: Option<&str>,
    ) -> Result<Vec<MetadataEntry>> {
        let (sql, params): (&str, Vec<&dyn rusqlite::ToSql>) = match entry_type {
            Some(t) => (
                "SELECT id, record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at
                 FROM workspace_file_metadata_entries
                 WHERE record_key = ?1 AND entry_type = ?2
                 ORDER BY file_path ASC, line_number ASC, id ASC",
                vec![&record_key, &t],
            ),
            None => (
                "SELECT id, record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at
                 FROM workspace_file_metadata_entries
                 WHERE record_key = ?1
                 ORDER BY entry_type ASC, file_path ASC, line_number ASC, id ASC",
                vec![&record_key],
            ),
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params), row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// List entries across every workspace. Both `search` (substring match
    /// over file_path and payload) and `entry_type` filters are optional.
    pub fn list_metadata_across_workspaces(
        &self,
        search: Option<&str>,
        entry_type: Option<&str>,
    ) -> Result<Vec<MetadataEntry>> {
        let mut sql = String::from(
            "SELECT id, record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at
             FROM workspace_file_metadata_entries
             WHERE 1=1",
        );
        let like_pattern: Option<String> =
            search.filter(|s| !s.is_empty()).map(|s| format!("%{}%", s));
        if like_pattern.is_some() {
            sql.push_str(" AND (file_path LIKE ?1 OR payload LIKE ?1)");
        }
        if entry_type.is_some() {
            let next = if like_pattern.is_some() { "?2" } else { "?1" };
            sql.push_str(&format!(" AND entry_type = {}", next));
        }
        sql.push_str(" ORDER BY record_key ASC, entry_type ASC, file_path ASC, line_number ASC, id ASC");

        let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();
        if let Some(p) = &like_pattern {
            params_vec.push(p);
        }
        if let Some(t) = &entry_type {
            params_vec.push(t);
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params_vec), row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Return every distinct `entry_type` seen in the table, alphabetical.
    pub fn list_metadata_entry_types(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT entry_type FROM workspace_file_metadata_entries ORDER BY entry_type ASC",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Fetch an entry by id.
    pub fn get_metadata_entry(&self, id: i64) -> Result<Option<MetadataEntry>> {
        self.conn
            .query_row(
                "SELECT id, record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at
                 FROM workspace_file_metadata_entries WHERE id = ?1",
                [id],
                row_to_entry,
            )
            .optional()
            .map_err(Into::into)
    }
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<MetadataEntry> {
    Ok(MetadataEntry {
        id: row.get(0)?,
        record_key: row.get(1)?,
        entry_type: row.get(2)?,
        file_path: row.get(3)?,
        line_number: row.get(4)?,
        line_anchor: row.get(5)?,
        payload: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}
```

- [ ] **Step 4: Add `compute_record_key` helper**

Append to `crates/tp-db/src/workspaces.rs`:

```rust
/// Derive the DB `record_key` for a workspace without touching the DB. This
/// is the public, side-effect-free counterpart to the logic embedded in
/// `record_workspace_open`. GUI code uses it to find a workspace's metadata.
pub fn compute_record_key(name: &str, config_path: Option<&str>) -> String {
    config_path
        .filter(|path| !path.trim().is_empty())
        .map(|path| format!("path:{}", path))
        .unwrap_or_else(|| format!("name:{}", name))
}
```

- [ ] **Step 5: Build verification**

Run: `cargo build --package pax-db`
Expected: succeeds with no new warnings.

Run: `cargo build --package pax-gui`
Expected: succeeds (nothing in pax-gui references the new symbols yet).

- [ ] **Step 6: Commit**

```bash
git add crates/tp-db/src/lib.rs crates/tp-db/src/schema.rs crates/tp-db/src/metadata_entries.rs crates/tp-db/src/notes.rs crates/tp-db/src/workspaces.rs
git commit -m "$(cat <<'EOF'
Add workspace_file_metadata_entries table + generic CRUD layer

Migration 005 creates a per-workspace, per-file, type-discriminated
metadata table with an opaque JSON payload column. The crate::
metadata_entries module exposes insert/update/delete/list operations
plus cross-workspace queries (for the forthcoming Metadata Manager).
compute_record_key() is factored out of record_workspace_open so UI
code can derive a workspace key without an INSERT side effect.
EOF
)"
```

---

## Task 2: `FileNote` typed wrapper

**Files:**
- Modify: `crates/tp-db/src/notes.rs`

- [ ] **Step 1: Fill in `notes.rs`**

Replace the stub content with:

```rust
//! Note-typed accessors on top of `metadata_entries`.
//!
//! The database treats every entry as `(record_key, entry_type, file_path,
//! line_number, line_anchor, payload JSON)`. This module is the only place
//! that knows `entry_type = "note"` and how to serialize/deserialize the
//! `{"text": "..."}` payload shape.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::metadata_entries::MetadataEntry;
use crate::Database;

pub const NOTE_ENTRY_TYPE: &str = "note";

#[derive(Debug, Clone)]
pub struct FileNote {
    pub id: i64,
    pub record_key: String,
    pub file_path: String,
    pub line_number: i32,
    pub line_anchor: Option<String>,
    pub text: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct NotePayload {
    text: String,
}

fn payload_from_text(text: &str) -> String {
    serde_json::to_string(&NotePayload {
        text: text.to_string(),
    })
    .expect("note payload serialization cannot fail")
}

fn text_from_payload(payload: &str) -> String {
    serde_json::from_str::<NotePayload>(payload)
        .map(|p| p.text)
        .unwrap_or_default()
}

fn entry_to_note(e: MetadataEntry) -> FileNote {
    FileNote {
        id: e.id,
        record_key: e.record_key,
        file_path: e.file_path,
        line_number: e.line_number,
        line_anchor: e.line_anchor,
        text: text_from_payload(&e.payload),
        created_at: e.created_at,
        updated_at: e.updated_at,
    }
}

impl Database {
    /// Insert a new note. Returns the persisted FileNote (id populated).
    pub fn add_note(
        &self,
        record_key: &str,
        file_path: &str,
        line_number: i32,
        line_anchor: Option<&str>,
        text: &str,
    ) -> Result<FileNote> {
        let payload = payload_from_text(text);
        let id = self.insert_metadata_entry(
            record_key,
            NOTE_ENTRY_TYPE,
            file_path,
            line_number,
            line_anchor,
            &payload,
        )?;
        let entry = self
            .get_metadata_entry(id)?
            .expect("row we just inserted must exist");
        Ok(entry_to_note(entry))
    }

    /// Replace a note's text.
    pub fn update_note_text(&self, id: i64, text: &str) -> Result<()> {
        self.update_metadata_payload(id, &payload_from_text(text))
    }

    /// List all notes attached to a file in a workspace.
    pub fn list_notes_for_file(
        &self,
        record_key: &str,
        file_path: &str,
    ) -> Result<Vec<FileNote>> {
        Ok(self
            .list_metadata_by_file(record_key, NOTE_ENTRY_TYPE, file_path)?
            .into_iter()
            .map(entry_to_note)
            .collect())
    }

    /// List every note in a workspace.
    pub fn list_notes_for_workspace(&self, record_key: &str) -> Result<Vec<FileNote>> {
        Ok(self
            .list_metadata_for_workspace(record_key, Some(NOTE_ENTRY_TYPE))?
            .into_iter()
            .map(entry_to_note)
            .collect())
    }
}
```

- [ ] **Step 2: Confirm `serde_json` is already in `tp-db`**

Check `crates/tp-db/Cargo.toml` and add `serde_json` as a workspace-inherited dep if not present. Most likely already transitively available via `serde`, but it needs to be direct.

Command to verify:

```bash
grep -E 'serde|rusqlite' crates/tp-db/Cargo.toml
```

If `serde_json` isn't there, add under `[dependencies]` in `crates/tp-db/Cargo.toml`:

```toml
serde = { workspace = true }
serde_json = { workspace = true }
```

- [ ] **Step 3: Build**

Run: `cargo build --package pax-db`
Expected: succeeds.

- [ ] **Step 4: Commit**

```bash
git add crates/tp-db/src/notes.rs crates/tp-db/Cargo.toml
git commit -m "$(cat <<'EOF'
Add FileNote typed wrapper over metadata_entries

Notes are the first concrete entry_type. Payload shape is
{"text": "..."} with serde_json for serialization; all other fields
(record_key, file_path, line_number, line_anchor, timestamps) stay as
strongly-typed columns so queries stay sargable.
EOF
)"
```

---

## Task 3: Workspace record_key plumbing into the editor

**Files:**
- Modify: `crates/tp-core/src/workspace.rs`
- Modify: `crates/tp-gui/src/workspace_view.rs`
- Modify: `crates/tp-gui/src/panels/registry.rs`
- Modify: `crates/tp-gui/src/panels/editor/mod.rs`

- [ ] **Step 1: Add `Workspace::record_key` convenience on pax-core**

Edit `crates/tp-core/src/workspace.rs`. Add to `impl Workspace`:

```rust
impl Workspace {
    /// Find a panel config by ID.
    pub fn panel(&self, id: &str) -> Option<&PanelConfig> {
        self.panels.iter().find(|p| p.id == id)
    }
    // ... existing methods ...

    /// Derive the pax-db `record_key` for this workspace. The workspace's
    /// config path (when saved) takes precedence over the name, matching the
    /// logic in `pax_db::record_workspace_open`.
    pub fn record_key(&self, config_path: Option<&str>) -> String {
        match config_path {
            Some(path) if !path.trim().is_empty() => format!("path:{}", path),
            _ => format!("name:{}", self.name),
        }
    }
}
```

(We duplicate the formula locally instead of pulling in a `pax-db` dep from `pax-core`, which the current crate graph doesn't have.)

- [ ] **Step 2: Thread record_key through `registry.rs` — Code Editor factory**

Edit `crates/tp-gui/src/panels/registry.rs`. Find the `"code_editor"` registration (around line 346). The factory reads `config.extra` for various `ssh_*` / `root_dir` / `__workspace_dir__` keys. Add a read for `__workspace_record_key__`:

```rust
|config| {
    let ws_dir = config.extra.get("__workspace_dir__").map(|s| s.as_str());
    let record_key = config
        .extra
        .get("__workspace_record_key__")
        .cloned()
        .unwrap_or_else(String::new);
    // ... existing raw_root / root_dir derivation ...
    let ssh_host = config.extra.get("ssh_host").cloned();
    // ... existing ssh reads ...

    if let Some(host) = ssh_host {
        let user = ssh_user.as_deref().unwrap_or("root");
        let rpath = remote_path.as_deref().unwrap_or(root_dir.as_str());
        Box::new(super::editor::CodeEditorPanel::new_remote(
            &host,
            ssh_port,
            user,
            ssh_password.as_deref(),
            ssh_identity.as_deref(),
            rpath,
            record_key,
        ))
    } else {
        Box::new(super::editor::CodeEditorPanel::new(&root_dir, record_key))
    }
}
```

- [ ] **Step 3: Extend `CodeEditorPanel::new` / `new_remote` to accept record_key**

Edit `crates/tp-gui/src/panels/editor/mod.rs`. The current `impl CodeEditorPanel` has three constructors (`new`, `new_remote`, `new_with_backend`). Add a `record_key: String` parameter to all three and store it in `EditorState`.

First, extend `EditorState`:

```rust
pub struct EditorState {
    pub root_dir: PathBuf,
    #[cfg(feature = "sourceview")]
    pub open_files: Vec<OpenFile>,
    pub active_tab: Option<usize>,
    pub sidebar_visible: bool,
    pub sidebar_mode: SidebarMode,
    pub backend: Arc<dyn file_backend::FileBackend>,
    pub poll_interval: u64,
    #[cfg(feature = "sourceview")]
    pub nav_back: Vec<FilePosition>,
    #[cfg(feature = "sourceview")]
    pub nav_forward: Vec<FilePosition>,
    #[cfg(feature = "sourceview")]
    pub recent_files: Vec<PathBuf>,
    #[cfg(feature = "sourceview")]
    pub on_nav_state_changed: Option<Rc<dyn Fn()>>,
    /// DB key used for scoping notes and other metadata. Empty string when
    /// the panel is instantiated outside a workspace (e.g. tests).
    pub record_key: String,
}
```

Update `CodeEditorPanel::new`:

```rust
pub fn new(root_dir: &str, record_key: String) -> Self {
    let backend = Arc::new(file_backend::LocalFileBackend::new(&PathBuf::from(
        root_dir,
    )));
    let mut panel = Self::new_with_backend(root_dir, backend, record_key);
    panel.ssh_info = None;
    panel
}
```

Update `CodeEditorPanel::new_remote` — add `record_key: String` as the last parameter and pass through to `new_with_backend`.

Update `CodeEditorPanel::new_with_backend`:

```rust
fn new_with_backend(
    root_dir: &str,
    backend: Arc<dyn file_backend::FileBackend>,
    record_key: String,
) -> Self {
    let poll_secs = if backend.is_remote() { 5 } else { 2 };
    let is_git_repo = std::path::Path::new(root_dir).join(".git").exists();
    let state = Rc::new(RefCell::new(EditorState {
        root_dir: PathBuf::from(root_dir),
        open_files: Vec::new(),
        active_tab: None,
        sidebar_visible: true,
        sidebar_mode: SidebarMode::Files,
        backend: backend.clone(),
        poll_interval: poll_secs,
        nav_back: Vec::new(),
        nav_forward: Vec::new(),
        recent_files: Vec::new(),
        on_nav_state_changed: None,
        record_key,
    }));
    // ... rest of the function unchanged ...
}
```

Make sure the non-sourceview fallback `CodeEditorPanel` (at the bottom of `mod.rs`) also accepts and ignores the new parameter — match signatures:

```rust
#[cfg(not(feature = "sourceview"))]
impl CodeEditorPanel {
    pub fn new(_root_dir: &str, _record_key: String) -> Self { ... }
    pub fn new_remote(
        _host: &str,
        _port: u16,
        _user: &str,
        _password: Option<&str>,
        _identity_file: Option<&str>,
        _remote_path: &str,
        _record_key: String,
    ) -> Self { ... }
}
```

- [ ] **Step 4: Populate `__workspace_record_key__` in `workspace_view.rs`**

Find where panels are constructed from a `Workspace`. The pattern is: `panel_cfg.extra.insert("__workspace_dir__".to_string(), ...)` etc. Grep for it:

```bash
rg -n '__workspace_dir__' crates/tp-gui/src/workspace_view.rs
```

Right next to the `__workspace_dir__` insertion, add:

```rust
extra.insert(
    "__workspace_record_key__".to_string(),
    workspace.record_key(config_path.as_deref()),
);
```

`config_path` here is whatever variable represents the workspace's config file path in that scope — the same value passed to `record_workspace_open`. Grep for existing calls to `record_workspace_open` in `workspace_view.rs` to find the right local name; pass that same value.

- [ ] **Step 5: Build**

Run: `cargo build --package pax-gui`
Expected: succeeds. Any compile error points at a constructor call site you need to update — the CodeEditor constructors are called from panel registry, panel chooser, and possibly tests. Update each to pass a `record_key` (use `String::new()` for test fixtures).

- [ ] **Step 6: Manual smoke**

```bash
cargo run -- new record-key-smoke
```

Open the Code Editor, open any file. Editor must still work — this task changes no behavior, only threads the key through.

- [ ] **Step 7: Commit**

```bash
git add crates/tp-core/src/workspace.rs crates/tp-gui/src/workspace_view.rs crates/tp-gui/src/panels/registry.rs crates/tp-gui/src/panels/editor/mod.rs
git commit -m "$(cat <<'EOF'
Editor: thread workspace record_key into EditorState

CodeEditorPanel constructors now accept the pax-db record_key of the
owning workspace, plumbed via __workspace_record_key__ in the panel's
extras map. EditorState stores it for later use by the notes and
metadata features. Behavior unchanged for existing flows.
EOF
)"
```

---

## Task 4: `NotesState` per source tab

**Files:**
- Create: `crates/tp-gui/src/panels/editor/notes_state.rs`
- Modify: `crates/tp-gui/src/panels/editor/mod.rs` (register module)
- Modify: `crates/tp-gui/src/panels/editor/tab_content.rs` (`SourceTab.notes`)

- [ ] **Step 1: Create `notes_state.rs`**

Full contents:

```rust
//! Per-source-tab note state: the live set of `FileNote` records attached
//! to a buffer, each anchored by a `gtk::TextMark` that GTK moves along
//! with edits. Loading resolves DB rows to marks; saving flushes the
//! current mark positions + line contents back to the DB so the next open
//! is robust to edits the user made during the session.

use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use pax_db::notes::FileNote;

/// Fuzzy-match window: on reload, if the saved line_number no longer
/// matches the anchor text, scan this many lines above and below looking
/// for the anchor's exact content.
pub const ANCHOR_FUZZY_RADIUS: i32 = 20;

/// A single note live on a buffer. `mark` is None when the note couldn't
/// be resolved to a line (orphan).
#[derive(Debug, Clone)]
pub struct LiveNote {
    pub db_id: i64,
    pub text: String,
    pub saved_line: i32,
    pub saved_anchor: Option<String>,
    pub mark: Option<gtk4::TextMark>,
}

/// Holds every note currently loaded for a source tab.
#[derive(Debug, Default, Clone)]
pub struct NotesState {
    pub entries: Rc<RefCell<Vec<LiveNote>>>,
}

impl NotesState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the current set of note lines (for ruler painting).
    pub fn current_lines(&self, buffer: &sourceview5::Buffer) -> Vec<i32> {
        let entries = self.entries.borrow();
        entries
            .iter()
            .filter_map(|e| e.mark.as_ref().map(|m| line_of_mark(buffer, m)))
            .collect()
    }

    pub fn push(&self, note: LiveNote) {
        self.entries.borrow_mut().push(note);
    }

    /// Remove a note by id. Also deletes the mark from the buffer if present.
    pub fn remove(&self, db_id: i64, buffer: &sourceview5::Buffer) {
        let mut entries = self.entries.borrow_mut();
        if let Some(pos) = entries.iter().position(|e| e.db_id == db_id) {
            let removed = entries.remove(pos);
            if let Some(mark) = removed.mark {
                buffer.delete_mark(&mark);
            }
        }
    }

    /// Update a note's text in place.
    pub fn set_text(&self, db_id: i64, new_text: &str) {
        for entry in self.entries.borrow_mut().iter_mut() {
            if entry.db_id == db_id {
                entry.text = new_text.to_string();
            }
        }
    }

    /// Find notes whose mark currently sits on the given line.
    pub fn notes_on_line(
        &self,
        buffer: &sourceview5::Buffer,
        line: i32,
    ) -> Vec<LiveNote> {
        let entries = self.entries.borrow();
        entries
            .iter()
            .filter(|e| {
                e.mark
                    .as_ref()
                    .map(|m| line_of_mark(buffer, m) == line)
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }
}

/// Attach DB-loaded notes to a buffer: for each FileNote, try to resolve
/// its line (exact line+anchor match first, then fuzzy ±ANCHOR_FUZZY_RADIUS),
/// create a mark if resolved, and push the LiveNote onto `state`.
pub fn apply_loaded_notes(
    state: &NotesState,
    buffer: &sourceview5::Buffer,
    notes: Vec<FileNote>,
) {
    for note in notes {
        let resolved = resolve_anchor(buffer, note.line_number, note.line_anchor.as_deref());
        let mark = resolved.map(|line| create_mark_at_line(buffer, line));
        state.push(LiveNote {
            db_id: note.id,
            text: note.text,
            saved_line: note.line_number,
            saved_anchor: note.line_anchor,
            mark,
        });
    }
}

/// Create a `TextMark` at the start of `line`. The mark has `left_gravity =
/// true` so typed text after the mark's position stays to the right, which
/// matches the intuition "the note is at the start of this line".
pub fn create_mark_at_line(buffer: &sourceview5::Buffer, line: i32) -> gtk4::TextMark {
    let iter = buffer.iter_at_line(line).unwrap_or_else(|| buffer.start_iter());
    buffer.create_mark(None, &iter, true)
}

/// 0-based line number of a mark.
pub fn line_of_mark(buffer: &sourceview5::Buffer, mark: &gtk4::TextMark) -> i32 {
    buffer.iter_at_mark(mark).line()
}

/// The content of `line` in `buffer`, without the trailing newline.
pub fn line_content(buffer: &sourceview5::Buffer, line: i32) -> String {
    let Some(start) = buffer.iter_at_line(line) else {
        return String::new();
    };
    let mut end = start.clone();
    if !end.ends_line() {
        end.forward_to_line_end();
    }
    buffer.text(&start, &end, false).to_string()
}

fn resolve_anchor(
    buffer: &sourceview5::Buffer,
    saved_line: i32,
    anchor: Option<&str>,
) -> Option<i32> {
    let total = buffer.line_count();
    if saved_line < 0 || saved_line >= total {
        return fuzzy_find(buffer, anchor, saved_line);
    }
    let at_saved = line_content(buffer, saved_line);
    if anchor.is_none() || anchor == Some(at_saved.as_str()) {
        return Some(saved_line);
    }
    fuzzy_find(buffer, anchor, saved_line)
}

fn fuzzy_find(
    buffer: &sourceview5::Buffer,
    anchor: Option<&str>,
    center: i32,
) -> Option<i32> {
    let anchor = anchor?;
    let total = buffer.line_count();
    for offset in 1..=ANCHOR_FUZZY_RADIUS {
        for candidate in [center - offset, center + offset] {
            if candidate < 0 || candidate >= total {
                continue;
            }
            if line_content(buffer, candidate) == anchor {
                return Some(candidate);
            }
        }
    }
    None
}
```

- [ ] **Step 2: Register module**

Edit `crates/tp-gui/src/panels/editor/mod.rs`:

```rust
#[cfg(feature = "sourceview")]
pub mod markdown_view;
#[cfg(feature = "sourceview")]
pub mod notes_state;
#[cfg(feature = "sourceview")]
pub mod tab_content;
```

- [ ] **Step 3: Add `notes` field to `SourceTab`**

Edit `crates/tp-gui/src/panels/editor/tab_content.rs`. In the `SourceTab` struct add:

```rust
use crate::panels::editor::notes_state::NotesState;

#[derive(Debug, Clone)]
pub struct SourceTab {
    pub buffer: sourceview5::Buffer,
    pub modified: bool,
    pub saved_content: Rc<RefCell<String>>,
    pub notes: NotesState,
}
```

Update every construction site of `SourceTab` — there's exactly one, in `editor_tabs.rs::open_file`:

```
rg -n 'SourceTab \{' crates/tp-gui/src/panels/editor/
```

At that call site add `notes: NotesState::new(),` — require `use super::notes_state::NotesState;` at the top of that file if not already imported.

- [ ] **Step 4: Build**

Run: `cargo build --package pax-gui`
Expected: succeeds.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/panels/editor/notes_state.rs crates/tp-gui/src/panels/editor/mod.rs crates/tp-gui/src/panels/editor/tab_content.rs crates/tp-gui/src/panels/editor/editor_tabs.rs
git commit -m "$(cat <<'EOF'
Editor: add NotesState with TextMark-anchored live notes per source tab

NotesState holds a Vec<LiveNote> keyed by DB id; each entry owns an
optional GTK TextMark so the note's line tracks buffer edits without
extra code. apply_loaded_notes resolves a saved (line_number,
line_anchor) pair against the current buffer via exact match then
±ANCHOR_FUZZY_RADIUS line scan; unresolvable notes become orphans with
mark=None so they still appear in lists. Field is on SourceTab; other
tab kinds are unaffected.
EOF
)"
```

---

## Task 5: Notes ruler widget

**Files:**
- Create: `crates/tp-gui/src/panels/editor/notes_ruler.rs`
- Modify: `crates/tp-gui/src/panels/editor/mod.rs` (register)
- Modify: `crates/tp-gui/src/panels/editor/editor_tabs.rs` (layout + per-tab ruler state)

- [ ] **Step 1: Create `notes_ruler.rs`**

Full contents:

```rust
//! Note-indicator drawing area. Mirrors the structure of
//! `build_match_overview_ruler` in editor_tabs.rs but paints amber dots at
//! every note line and exposes a click-to-jump gesture targeting the
//! active tab's notes.

use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub const NOTES_RULER_WIDTH: i32 = 14;
const NOTES_BG_ALPHA: f64 = 0.04;
const NOTES_DOT_RADIUS: f64 = 3.0;
const NOTES_DOT_R: f64 = 0.96;
const NOTES_DOT_G: f64 = 0.78;
const NOTES_DOT_B: f64 = 0.25;

pub struct NotesRuler {
    pub widget: gtk4::DrawingArea,
    lines: Rc<RefCell<Vec<i32>>>,
    total_lines: Rc<RefCell<i32>>,
}

impl NotesRuler {
    pub fn new() -> Self {
        let widget = gtk4::DrawingArea::new();
        widget.set_width_request(NOTES_RULER_WIDTH);
        widget.set_vexpand(true);
        widget.add_css_class("editor-notes-ruler");
        widget.set_tooltip_text(Some("Click a note marker to jump to it"));
        widget.set_cursor_from_name(Some("pointer"));

        let lines: Rc<RefCell<Vec<i32>>> = Rc::new(RefCell::new(Vec::new()));
        let total_lines: Rc<RefCell<i32>> = Rc::new(RefCell::new(1));

        {
            let lines = lines.clone();
            let total = total_lines.clone();
            widget.set_draw_func(move |_, cr, w, h| {
                let w_f = w as f64;
                let h_f = h as f64;
                cr.set_source_rgba(0.5, 0.5, 0.5, NOTES_BG_ALPHA);
                let _ = cr.paint();
                let total = (*total.borrow()).max(1) as f64;
                cr.set_source_rgba(NOTES_DOT_R, NOTES_DOT_G, NOTES_DOT_B, 0.95);
                for &line in lines.borrow().iter() {
                    let y = (line as f64 / total) * h_f;
                    let cx = w_f / 2.0;
                    cr.arc(cx, y + NOTES_DOT_RADIUS, NOTES_DOT_RADIUS, 0.0, std::f64::consts::TAU);
                    let _ = cr.fill();
                }
            });
        }

        Self {
            widget,
            lines,
            total_lines,
        }
    }

    /// Refresh with the current set of note lines for a buffer.
    pub fn update(&self, new_lines: Vec<i32>, total_buffer_lines: i32) {
        *self.lines.borrow_mut() = new_lines;
        *self.total_lines.borrow_mut() = total_buffer_lines.max(1);
        let has_any = !self.lines.borrow().is_empty();
        self.widget.set_visible(has_any);
        self.widget.queue_draw();
    }

    pub fn clear(&self) {
        self.lines.borrow_mut().clear();
        self.widget.set_visible(false);
        self.widget.queue_draw();
    }

    /// Given a pixel y coordinate inside the widget, return the buffer line
    /// closest to a painted dot, or None when no dots exist.
    pub fn nearest_line(&self, y: f64, height_px: f64) -> Option<i32> {
        let lines = self.lines.borrow();
        if lines.is_empty() {
            return None;
        }
        let total = (*self.total_lines.borrow()).max(1) as f64;
        let clicked = ((y / height_px).clamp(0.0, 1.0) * total) as i32;
        lines
            .iter()
            .copied()
            .min_by_key(|l| (*l - clicked).abs())
    }
}
```

- [ ] **Step 2: Register module**

Edit `crates/tp-gui/src/panels/editor/mod.rs`:

```rust
#[cfg(feature = "sourceview")]
pub mod notes_ruler;
```

- [ ] **Step 3: Place the ruler in the editor layout**

Edit `crates/tp-gui/src/panels/editor/editor_tabs.rs`. Find the block around line 244 that builds `editor_row`:

```rust
let editor_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
editor_row.set_vexpand(true);
editor_row.set_hexpand(true);
editor_row.append(&source_scroll);
editor_row.append(&match_ruler);
```

Replace with:

```rust
let notes_ruler = super::notes_ruler::NotesRuler::new();
notes_ruler.widget.set_visible(false);

let editor_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
editor_row.set_vexpand(true);
editor_row.set_hexpand(true);
editor_row.append(&notes_ruler.widget);     // left of source
editor_row.append(&source_scroll);
editor_row.append(&match_ruler);            // right of source
```

Store the `NotesRuler` in the `EditorTabs` struct:

```rust
pub struct EditorTabs {
    // ...existing fields...
    notes_ruler: Rc<super::notes_ruler::NotesRuler>,
}
```

Wrap the created ruler in `Rc`:

```rust
let notes_ruler = Rc::new(super::notes_ruler::NotesRuler::new());
// ...
editor_row.append(&notes_ruler.widget);
// ...
Self {
    // ...existing fields...
    notes_ruler,
}
```

And add a public refresh method on `EditorTabs`:

```rust
impl EditorTabs {
    /// Refresh the notes ruler for the active source tab. Callers use this
    /// after mutating the NotesState (add/delete/load).
    pub fn refresh_notes_ruler(&self, state: &Rc<RefCell<EditorState>>) {
        let st = state.borrow();
        let Some(idx) = st.active_tab else {
            self.notes_ruler.clear();
            return;
        };
        let Some(open_file) = st.open_files.get(idx) else {
            self.notes_ruler.clear();
            return;
        };
        let tab_content::TabContent::Source(source) = &open_file.content else {
            self.notes_ruler.clear();
            return;
        };
        let lines = source.notes.current_lines(&source.buffer);
        let total = source.buffer.line_count();
        self.notes_ruler.update(lines, total);
    }
}
```

Add the import at the top of the file if missing:

```rust
use super::tab_content;
```

- [ ] **Step 4: Refresh the ruler on tab switch**

In `connect_switch_page` (currently ~line 344), after `cs.set_visible_child_name(&child)` and the status bar updates, invoke the refresh. Easier: call `self.refresh_notes_ruler(state)` from `switch_to_buffer`, and have the switch-page handler call that too (or dispatch via a shared closure). Match the existing pattern — `switch_to_buffer` already exists, update the ruler at the bottom of its body:

```rust
pub fn switch_to_buffer(&self, idx: usize, state: &Rc<RefCell<EditorState>>) {
    // ...existing body...
    self.refresh_notes_ruler(state);
}
```

For the `connect_switch_page` handler that doesn't go through `switch_to_buffer`, add a minimal refresh at the end (inside the handler's outer block, after `fire_nav_state_changed`). Because the handler captures refs, you'll need to clone an `Rc<NotesRuler>` and the state into the closure.

Simplest: restructure so the notebook's switch_page calls `tabs_c.switch_to_buffer(idx, &state_c)` uniformly, dropping the inline set_visible_child / set_buffer logic (that logic is already also in switch_to_buffer, so the call is idempotent). Concretely, replace the `notebook.connect_switch_page(move |...| {...})` body that was introduced in the media-viewers refactor with:

```rust
let state_c = state.clone();
let tabs_handle: Rc<RefCell<Option<Rc<EditorTabs>>>> = /* already-held self handle */;
notebook.connect_switch_page(move |_nb, _page, page_num| {
    // ... existing try_borrow_mut-based status bar updates ...
    // at the very end:
    //   notes_ruler_c.update(lines, total);
});
```

Rather than restructure notebooks across tabs, keep the existing handler but add a `notes_ruler.clone()` into its captures and invoke:

```rust
let nr = self.notes_ruler.clone();
// ... later inside the closure:
let lines = source.notes.current_lines(&source.buffer);
let total = source.buffer.line_count();
nr.update(lines, total);
```

Do this inside the `if let Some(buf) = open_file.source_buffer()` branch — the same branch that updates the match ruler — so a non-source tab clears the ruler via the else branch (`nr.clear()`).

- [ ] **Step 5: Build**

Run: `cargo build --package pax-gui`
Expected: succeeds.

- [ ] **Step 6: Manual smoke**

```bash
cargo run -- new notes-ruler-smoke
```

Open any source file. The notes ruler should **not** be visible (no notes yet). Editor behavior otherwise unchanged.

- [ ] **Step 7: Commit**

```bash
git add crates/tp-gui/src/panels/editor/notes_ruler.rs crates/tp-gui/src/panels/editor/mod.rs crates/tp-gui/src/panels/editor/editor_tabs.rs
git commit -m "$(cat <<'EOF'
Editor: add notes ruler drawing area + wire into editor_row

NotesRuler paints small amber dots at every line carrying a note in
the active source tab. Placed to the left of the source ScrolledWindow
so it visually balances the existing match ruler on the right. Hidden
when the current tab has no notes or isn't a source tab.

Ruler refresh is triggered from switch_to_buffer and from the notebook
switch_page handler.
EOF
)"
```

---

## Task 6: Right-click Add / Edit / Delete Note on source lines

**Files:**
- Modify: `crates/tp-gui/src/panels/editor/editor_tabs.rs`
- Modify: `crates/tp-gui/src/panels/editor/text_context_menu.rs` (confirm extras callback signature)

- [ ] **Step 1: Read existing context-menu wiring**

Check how the editor's source view installs its right-click menu:

```bash
rg -n 'text_context_menu::install' crates/tp-gui/src/panels/editor/editor_tabs.rs
```

It passes an `extras` callback that returns `Vec<TextContextMenuItem>` given the current click location. That callback is the hook for Add/Edit/Delete Note.

- [ ] **Step 2: Implement the Add Note dialog**

Inside `editor_tabs.rs`, add a helper:

```rust
fn show_note_editor(
    parent: Option<&gtk4::Window>,
    title: &str,
    initial_text: &str,
    on_save: impl Fn(String) + 'static,
) {
    let dialog = gtk4::Window::builder()
        .title(title)
        .modal(true)
        .default_width(NOTE_EDITOR_WIDTH_PX)
        .default_height(NOTE_EDITOR_HEIGHT_PX)
        .build();
    if let Some(win) = parent {
        dialog.set_transient_for(Some(win));
    }

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let text_view = gtk4::TextView::new();
    text_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    text_view.set_vexpand(true);
    text_view.set_hexpand(true);
    text_view.buffer().set_text(initial_text);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&text_view));
    scroll.set_vexpand(true);
    vbox.append(&scroll);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    let save_btn = gtk4::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");

    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    vbox.append(&btn_row);

    {
        let d = dialog.clone();
        cancel_btn.connect_clicked(move |_| d.close());
    }
    {
        let d = dialog.clone();
        let tv = text_view.clone();
        save_btn.connect_clicked(move |_| {
            let buf = tv.buffer();
            let text = buf
                .text(&buf.start_iter(), &buf.end_iter(), false)
                .to_string();
            if !text.trim().is_empty() {
                on_save(text);
            }
            d.close();
        });
    }

    dialog.set_child(Some(&vbox));
    dialog.present();
    text_view.grab_focus();
}
```

Add constants at the top of the file:

```rust
const NOTE_EDITOR_WIDTH_PX: i32 = 440;
const NOTE_EDITOR_HEIGHT_PX: i32 = 240;
```

- [ ] **Step 3: Extend the extras callback to offer Add/Edit/Delete Note**

Find the existing `extras` closure passed to `text_context_menu::install`. Modify its body to insert note-related items. Template (adapt names to match what the current file uses for the click-line extraction):

```rust
let state_for_notes = state.clone();
let tabs_handle_for_notes = tabs_rc.clone();
let extras_cb = move |click_line: i32| -> Vec<TextContextMenuItem> {
    let mut items = Vec::new();

    let (record_key, file_path_str, notes_on_click, buffer_clone) = {
        let st = state_for_notes.borrow();
        let Some(idx) = st.active_tab else {
            return items;
        };
        let Some(open_file) = st.open_files.get(idx) else {
            return items;
        };
        let super::tab_content::TabContent::Source(source) = &open_file.content else {
            return items;
        };
        let file_path_str = relative_file_path(&st.root_dir, &open_file.path);
        let notes_here = source.notes.notes_on_line(&source.buffer, click_line);
        (
            st.record_key.clone(),
            file_path_str,
            notes_here,
            source.buffer.clone(),
        )
    };

    if record_key.is_empty() {
        // No DB scope — notes are disabled.
        return items;
    }

    // Add Note
    {
        let state_c = state_for_notes.clone();
        let tabs_c = tabs_handle_for_notes.clone();
        let rk = record_key.clone();
        let fp = file_path_str.clone();
        let buf = buffer_clone.clone();
        items.push(TextContextMenuItem::Action {
            label: "Add Note".into(),
            icon: Some("document-new-symbolic".into()),
            on_activate: Rc::new(move || {
                let parent = buf
                    .iter_at_mark(&buf.get_insert())
                    .buffer()
                    .root()
                    .and_then(|r| r.downcast::<gtk4::Window>().ok());
                let state_c = state_c.clone();
                let tabs_c = tabs_c.clone();
                let rk = rk.clone();
                let fp = fp.clone();
                let buf = buf.clone();
                show_note_editor(parent.as_ref(), "Add note", "", move |text| {
                    let anchor = notes_state::line_content(&buf, click_line);
                    let db_path = pax_db::Database::default_path();
                    let Ok(db) = pax_db::Database::open(&db_path) else {
                        return;
                    };
                    let Ok(note) = db.add_note(&rk, &fp, click_line, Some(&anchor), &text)
                    else {
                        return;
                    };
                    let live = notes_state::LiveNote {
                        db_id: note.id,
                        text: note.text,
                        saved_line: note.line_number,
                        saved_anchor: note.line_anchor,
                        mark: Some(notes_state::create_mark_at_line(&buf, click_line)),
                    };
                    // Push into the active tab's NotesState.
                    let st = state_c.borrow();
                    if let Some(i) = st.active_tab {
                        if let Some(open_file) = st.open_files.get(i) {
                            if let super::tab_content::TabContent::Source(source) =
                                &open_file.content
                            {
                                source.notes.push(live);
                            }
                        }
                    }
                    drop(st);
                    tabs_c.refresh_notes_ruler(&state_c);
                });
            }),
        });
    }

    // Edit / Delete entries for each note already on the clicked line.
    for note in notes_on_click {
        let preview = note.text.lines().next().unwrap_or("").to_string();
        let label_short = if preview.len() > NOTE_LABEL_PREVIEW_LEN {
            format!("{}…", &preview[..NOTE_LABEL_PREVIEW_LEN])
        } else {
            preview.clone()
        };

        // Edit
        {
            let state_c = state_for_notes.clone();
            let tabs_c = tabs_handle_for_notes.clone();
            let buf = buffer_clone.clone();
            let id = note.db_id;
            let existing = note.text.clone();
            items.push(TextContextMenuItem::Action {
                label: format!("Edit Note: {}", label_short),
                icon: Some("document-edit-symbolic".into()),
                on_activate: Rc::new(move || {
                    let parent = buf
                        .iter_at_mark(&buf.get_insert())
                        .buffer()
                        .root()
                        .and_then(|r| r.downcast::<gtk4::Window>().ok());
                    let state_c = state_c.clone();
                    let tabs_c = tabs_c.clone();
                    let existing = existing.clone();
                    show_note_editor(parent.as_ref(), "Edit note", &existing, move |text| {
                        let db_path = pax_db::Database::default_path();
                        let Ok(db) = pax_db::Database::open(&db_path) else {
                            return;
                        };
                        let _ = db.update_note_text(id, &text);
                        // Update in-memory state.
                        let st = state_c.borrow();
                        if let Some(i) = st.active_tab {
                            if let Some(open_file) = st.open_files.get(i) {
                                if let super::tab_content::TabContent::Source(source) =
                                    &open_file.content
                                {
                                    source.notes.set_text(id, &text);
                                }
                            }
                        }
                        drop(st);
                        tabs_c.refresh_notes_ruler(&state_c);
                    });
                }),
            });
        }

        // Delete
        {
            let state_c = state_for_notes.clone();
            let tabs_c = tabs_handle_for_notes.clone();
            let buf = buffer_clone.clone();
            let id = note.db_id;
            items.push(TextContextMenuItem::Action {
                label: format!("Delete Note: {}", label_short),
                icon: Some("user-trash-symbolic".into()),
                on_activate: Rc::new(move || {
                    let db_path = pax_db::Database::default_path();
                    let Ok(db) = pax_db::Database::open(&db_path) else {
                        return;
                    };
                    let _ = db.delete_metadata_entry(id);
                    let st = state_c.borrow();
                    if let Some(i) = st.active_tab {
                        if let Some(open_file) = st.open_files.get(i) {
                            if let super::tab_content::TabContent::Source(source) =
                                &open_file.content
                            {
                                source.notes.remove(id, &source.buffer);
                            }
                        }
                    }
                    drop(st);
                    tabs_c.refresh_notes_ruler(&state_c);
                }),
            });
        }
    }

    items
};
```

Add the constant:

```rust
const NOTE_LABEL_PREVIEW_LEN: usize = 32;
```

And the helper:

```rust
fn relative_file_path(root: &Path, absolute: &Path) -> String {
    absolute
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| absolute.to_string_lossy().into_owned())
}
```

The existing `text_context_menu::install` signature may need the click-line (integer) as a parameter of the extras callback. Confirm:

```bash
rg -n 'fn install|extras' crates/tp-gui/src/panels/editor/text_context_menu.rs | head -20
```

If the current extras callback takes `()` or `&Path`, extend it to take the clicked line number. Find the gesture handler that computes the clicked line (buffer position from pointer x/y) and pass that line into the callback. Pattern reference: the existing "right-click in file tree" in `file_tree.rs` translates y-pixel to row index via `row_at_y`.

- [ ] **Step 4: Import the items the closure uses**

At the top of `editor_tabs.rs` make sure these are imported:

```rust
use super::notes_state;
use super::text_context_menu::TextContextMenuItem;
```

- [ ] **Step 5: Build**

Run: `cargo build --package pax-gui`
Expected: succeeds. Any error about mismatched extras-callback signature → update `text_context_menu::install` to take the click-line parameter as designed, and update any other callers to pass a dummy value or ignore.

- [ ] **Step 6: Manual verification**

```bash
cargo run -- new notes-context-menu-smoke
```

1. Open a source file.
2. Right-click on a line → menu shows **Add Note**.
3. Click it → dialog opens → type text → Save.
4. Amber dot appears on the notes ruler at the clicked line.
5. Right-click on the same line → menu now also shows **Edit Note: …** and **Delete Note: …** for that note.
6. Edit it → text updates. Delete it → dot disappears.

- [ ] **Step 7: Commit**

```bash
git add crates/tp-gui/src/panels/editor/editor_tabs.rs crates/tp-gui/src/panels/editor/text_context_menu.rs
git commit -m "$(cat <<'EOF'
Editor: right-click Add/Edit/Delete Note on source lines

text_context_menu extras callback receives the clicked buffer line; the
editor extends the returned items with Add Note (always, when the
workspace has a record_key) and Edit/Delete entries per existing note
on the line. Add Note opens a modal dialog with a TextView; Save
persists via Database::add_note and creates a live TextMark in the
NotesState. Edit/Delete go through update_note_text /
delete_metadata_entry. The notes ruler is refreshed after each
mutation.
EOF
)"
```

---

## Task 7: Async load on file open + save-path flush

**Files:**
- Modify: `crates/tp-gui/src/panels/editor/editor_tabs.rs`

- [ ] **Step 1: Kick off an async note load when opening a source file**

Locate `open_file` (around line 724) — just after the block that creates the `SourceTab`, pushes the `OpenFile` into state, and calls `switch_to_buffer`. Add:

```rust
// Async load of any notes attached to this file in the DB. Doesn't block
// the open; when the query returns we resolve each note's line via the
// anchor and paint the ruler.
{
    let record_key = state.borrow().record_key.clone();
    if !record_key.is_empty() {
        let fp = relative_file_path(&state.borrow().root_dir, path);
        let tabs_c = self_rc_for_notes.clone();
        let state_c = state.clone();
        let tab_id = open_file_tab_id; // u64 captured from the freshly-pushed OpenFile
        super::task::run_blocking(
            move || {
                let db = pax_db::Database::open(&pax_db::Database::default_path()).ok()?;
                db.list_notes_for_file(&record_key, &fp).ok()
            },
            move |maybe_notes| {
                let Some(notes) = maybe_notes else { return };
                let st = state_c.borrow();
                let Some(open_file) = st.open_files.iter().find(|f| f.tab_id == tab_id) else {
                    return;
                };
                let super::tab_content::TabContent::Source(source) = &open_file.content else {
                    return;
                };
                notes_state::apply_loaded_notes(&source.notes, &source.buffer, notes);
                drop(st);
                tabs_c.refresh_notes_ruler(&state_c);
            },
        );
    }
}
```

This requires `self` to be reachable inside the closure — `open_file` is a method on `EditorTabs`. Plumb a `Rc<EditorTabs>` handle through the panel setup so the callback can reach `refresh_notes_ruler`. In practice, `mod.rs` already builds `tabs_rc: Rc<EditorTabs>` — expose a `set_self_handle(&self, rc: Rc<EditorTabs>)` on `EditorTabs` called once after construction, storing a `Rc<EditorTabs>` inside `EditorTabs` via `OnceCell<Rc<EditorTabs>>`, and read it in `open_file`. Or, simpler for this plan, pass `self` into `open_file` as `Rc<Self>` via a wrapper method `open_file_rc(self: &Rc<Self>, ...)` and fix call sites.

Simplest concrete pattern that matches the codebase's existing `tabs_rc` use (in `mod.rs` the tabs handle is already `Rc<EditorTabs>`): change `open_file` to take `&self` and do nothing special — then have an `Rc<EditorTabs>` parameter threaded through callers. Grep who calls `open_file`:

```bash
rg -n 'tabs.*\.open_file\(|tabs_rc\.open_file\(' crates/tp-gui/src/panels/editor/
```

All call sites already have an `Rc<EditorTabs>`. Change the signature to:

```rust
pub fn open_file(
    self: &Rc<Self>,
    path: &Path,
    state: &Rc<RefCell<EditorState>>,
) -> Option<usize>
```

And rename `self_rc_for_notes` in the closure above to `self.clone()`.

- [ ] **Step 2: Flush note positions on save**

Find `save_active` (around line 1420):

```bash
rg -n 'fn save_active' crates/tp-gui/src/panels/editor/editor_tabs.rs
```

After the successful `backend.write_file(...)` call, before releasing the mutable borrow, iterate the tab's notes and update their stored positions:

```rust
// Flush note positions: for each note on this tab, read its current line
// from its mark and persist (line, anchor) so the next reload is robust
// to edits the user made during the session.
let record_key = st.record_key.clone();
let root_dir = st.root_dir.clone();
if !record_key.is_empty() {
    let fp = relative_file_path(&root_dir, &open_file.path);
    if let super::tab_content::TabContent::Source(source) = &open_file.content {
        let entries_snapshot: Vec<(i64, i32, String)> = source
            .notes
            .entries
            .borrow()
            .iter()
            .filter_map(|e| {
                let mark = e.mark.as_ref()?;
                let line = notes_state::line_of_mark(&source.buffer, mark);
                let anchor = notes_state::line_content(&source.buffer, line);
                Some((e.db_id, line, anchor))
            })
            .collect();
        let db_path = pax_db::Database::default_path();
        if let Ok(db) = pax_db::Database::open(&db_path) {
            for (id, line, anchor) in entries_snapshot {
                let _ = db.update_metadata_position(id, line, Some(&anchor));
            }
        }
        let _ = fp; // fp is used implicitly via the bound db handle on the same thread
    }
}
```

Run the loop synchronously on the main thread — each update is a sub-millisecond `UPDATE` and the normal count of notes per file is small.

- [ ] **Step 3: Build**

Run: `cargo build --package pax-gui`
Expected: succeeds. Any error about `self: &Rc<Self>` signatures → propagate the change through all `open_file` call sites.

- [ ] **Step 4: Manual verification**

1. Open a file, add a note on line 10.
2. Close the file, reopen from scratch (close the tab with Ctrl+W, then Ctrl+P the file again). The note should reappear via the async load — amber dot on line 10.
3. With the note in place, add 5 blank lines above line 10 and save (Ctrl+S). The in-memory mark follows, so the dot is now on line 15.
4. Close the file, reopen. Dot still on line 15 (the DB was updated on save).
5. Delete the noted line, save, reopen. DB row still exists; but the anchor no longer matches → the note is an orphan (no dot on the ruler). It will still appear in the Notes dialog (Task 8).

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/panels/editor/editor_tabs.rs
git commit -m "$(cat <<'EOF'
Editor: async note load on open + save-time position flush

open_file now dispatches a background DB read (via run_blocking) that
loads notes for (record_key, file_path), resolves each to the current
buffer via exact line+anchor match or a ±20-line fuzzy scan, creates
marks for resolved notes and leaves orphans mark-less. Refresh
rolls into the notes ruler automatically.

save_active flushes each note's current mark line and its current line
content back to the DB so the next open finds the note where the user
left it, even if they edited many lines above it during the session.
EOF
)"
```

---

## Task 8: Per-workspace Notes dialog + sidebar button

**Files:**
- Create: `crates/tp-gui/src/dialogs/notes_dialog.rs`
- Modify: `crates/tp-gui/src/dialogs/mod.rs`
- Modify: `crates/tp-gui/src/panels/editor/file_tree.rs`

- [ ] **Step 1: Register the module**

Edit `crates/tp-gui/src/dialogs/mod.rs`:

```rust
pub mod notes_dialog;
```

- [ ] **Step 2: Create `notes_dialog.rs`**

Full contents:

```rust
//! Per-workspace Notes dialog accessed from the file-tree sidebar.
//! Lists every note in the current workspace with Jump / Edit / Delete.

use gtk4::prelude::*;
use pax_db::Database;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const DIALOG_WIDTH_PX: i32 = 600;
const DIALOG_HEIGHT_PX: i32 = 420;
const PREVIEW_MAX_CHARS: usize = 72;

pub type OnJump = Rc<dyn Fn(&std::path::Path, i32)>;
pub type OnNotesChanged = Rc<dyn Fn()>;

pub fn show_workspace_notes_dialog(
    parent: Option<&gtk4::Window>,
    workspace_label: &str,
    record_key: String,
    workspace_root: PathBuf,
    on_jump: OnJump,
    on_notes_changed: OnNotesChanged,
) {
    let dialog = gtk4::Window::builder()
        .title(&format!("Notes — {}", workspace_label))
        .modal(true)
        .default_width(DIALOG_WIDTH_PX)
        .default_height(DIALOG_HEIGHT_PX)
        .build();
    if let Some(win) = parent {
        dialog.set_transient_for(Some(win));
    }

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(8);
    vbox.set_margin_bottom(8);
    vbox.set_margin_start(8);
    vbox.set_margin_end(8);

    // Top strip: search.
    let search = gtk4::SearchEntry::new();
    search.set_placeholder_text(Some("Search notes…"));
    vbox.append(&search);

    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);
    list_box.add_css_class("boxed-list");

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&list_box));
    scroll.set_vexpand(true);
    vbox.append(&scroll);

    // Action row at the bottom.
    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    let jump_btn = gtk4::Button::with_label("Jump");
    let edit_btn = gtk4::Button::with_label("Edit");
    let delete_btn = gtk4::Button::with_label("Delete");
    delete_btn.add_css_class("destructive-action");
    let close_btn = gtk4::Button::with_label("Close");
    btn_row.append(&jump_btn);
    btn_row.append(&edit_btn);
    btn_row.append(&delete_btn);
    btn_row.append(&close_btn);
    vbox.append(&btn_row);

    let all_notes: Rc<RefCell<Vec<pax_db::notes::FileNote>>> =
        Rc::new(RefCell::new(Vec::new()));

    let reload = {
        let list_box = list_box.clone();
        let all_notes = all_notes.clone();
        let record_key = record_key.clone();
        let search = search.clone();
        Rc::new(move || {
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }
            let db_path = Database::default_path();
            let Ok(db) = Database::open(&db_path) else {
                return;
            };
            let notes = db
                .list_notes_for_workspace(&record_key)
                .unwrap_or_default();
            *all_notes.borrow_mut() = notes.clone();

            let query = search.text().to_string().to_lowercase();
            for (idx, note) in notes.iter().enumerate() {
                if !query.is_empty()
                    && !note.text.to_lowercase().contains(&query)
                    && !note.file_path.to_lowercase().contains(&query)
                {
                    continue;
                }
                let row = build_row(note, idx);
                list_box.append(&row);
            }
        })
    };
    reload();

    {
        let reload = reload.clone();
        search.connect_search_changed(move |_| reload());
    }

    // Jump
    {
        let list_box_c = list_box.clone();
        let all_notes = all_notes.clone();
        let root = workspace_root.clone();
        let d = dialog.clone();
        jump_btn.connect_clicked(move |_| {
            let Some(row) = list_box_c.selected_row() else {
                return;
            };
            let idx: usize = row
                .widget_name()
                .parse()
                .unwrap_or(usize::MAX);
            let Some(note) = all_notes.borrow().get(idx).cloned() else {
                return;
            };
            let full = root.join(&note.file_path);
            on_jump(&full, note.line_number);
            d.close();
        });
    }

    // Edit
    {
        let list_box_c = list_box.clone();
        let all_notes = all_notes.clone();
        let reload = reload.clone();
        let on_notes_changed = on_notes_changed.clone();
        let d = dialog.clone();
        edit_btn.connect_clicked(move |_| {
            let Some(row) = list_box_c.selected_row() else {
                return;
            };
            let idx: usize = row
                .widget_name()
                .parse()
                .unwrap_or(usize::MAX);
            let Some(note) = all_notes.borrow().get(idx).cloned() else {
                return;
            };
            let reload = reload.clone();
            let on_notes_changed = on_notes_changed.clone();
            let parent_for_editor = d.clone().upcast::<gtk4::Window>();
            super::super::panels::editor::editor_tabs::show_note_editor(
                Some(&parent_for_editor),
                "Edit note",
                &note.text,
                move |new_text| {
                    let db_path = Database::default_path();
                    if let Ok(db) = Database::open(&db_path) {
                        let _ = db.update_note_text(note.id, &new_text);
                    }
                    reload();
                    on_notes_changed();
                },
            );
        });
    }

    // Delete
    {
        let list_box_c = list_box.clone();
        let all_notes = all_notes.clone();
        let reload = reload.clone();
        let on_notes_changed = on_notes_changed.clone();
        delete_btn.connect_clicked(move |_| {
            let Some(row) = list_box_c.selected_row() else {
                return;
            };
            let idx: usize = row
                .widget_name()
                .parse()
                .unwrap_or(usize::MAX);
            let Some(note) = all_notes.borrow().get(idx).cloned() else {
                return;
            };
            let db_path = Database::default_path();
            if let Ok(db) = Database::open(&db_path) {
                let _ = db.delete_metadata_entry(note.id);
            }
            reload();
            on_notes_changed();
        });
    }

    {
        let d = dialog.clone();
        close_btn.connect_clicked(move |_| d.close());
    }

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn build_row(note: &pax_db::notes::FileNote, idx: usize) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_widget_name(&idx.to_string());

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_margin_start(8);
    vbox.set_margin_end(8);
    vbox.set_margin_top(6);
    vbox.set_margin_bottom(6);

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let loc = gtk4::Label::new(Some(&format!("{} · L{}", note.file_path, note.line_number + 1)));
    loc.add_css_class("caption");
    loc.set_halign(gtk4::Align::Start);
    header.append(&loc);
    if note.line_anchor.is_none() {
        let badge = gtk4::Label::new(Some("orphan"));
        badge.add_css_class("caption");
        badge.add_css_class("dim-label");
        header.append(&badge);
    }
    vbox.append(&header);

    let preview_text = preview_of(&note.text);
    let preview = gtk4::Label::new(Some(&preview_text));
    preview.set_halign(gtk4::Align::Start);
    preview.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    vbox.append(&preview);

    row.set_child(Some(&vbox));
    row
}

fn preview_of(text: &str) -> String {
    let first = text.lines().next().unwrap_or("");
    if first.chars().count() > PREVIEW_MAX_CHARS {
        let truncated: String = first.chars().take(PREVIEW_MAX_CHARS).collect();
        format!("{}…", truncated)
    } else {
        first.to_string()
    }
}
```

This references `editor_tabs::show_note_editor` — make that function `pub(crate)` so dialogs can call it.

- [ ] **Step 3: Add the sidebar button**

Edit `crates/tp-gui/src/panels/editor/file_tree.rs`. In `actions_bar` setup (around line 169), after the Collapse All button append:

```rust
let notes_btn = gtk4::Button::from_icon_name("user-bookmarks-symbolic");
notes_btn.add_css_class("flat");
notes_btn.set_tooltip_text(Some("Workspace notes"));
actions_bar.append(&notes_btn);
```

Wire the click to open `WorkspaceNotesDialog`. The file-tree needs access to record_key, workspace_root, and a "jump to file" callback. Add new parameters to `FileTree::new_with_context`:

```rust
pub fn new_with_context(
    root: &Path,
    on_open: Rc<dyn Fn(&Path)>,
    on_context_action: Option<OnContextAction>,
    on_file_renamed: Option<Rc<dyn Fn(&Path, &Path)>>,
    on_path_deleted: Option<Rc<dyn Fn(&Path)>>,
    backend: Arc<dyn FileBackend>,
    record_key: String,
    on_notes_jump: Rc<dyn Fn(&Path, i32)>,
) -> Self { ... }
```

Inside, wire the click:

```rust
{
    let record_key = record_key.clone();
    let root = root.to_path_buf();
    let on_jump = on_notes_jump.clone();
    let refresh = { let lb = list_box.clone(); /* existing refresh closure */ };
    notes_btn.connect_clicked(move |btn| {
        let parent = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
        let record_key = record_key.clone();
        let root = root.clone();
        let on_jump = on_jump.clone();
        let refresh = refresh.clone();
        crate::dialogs::notes_dialog::show_workspace_notes_dialog(
            parent.as_ref(),
            root
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "workspace".into())
                .as_str(),
            record_key,
            root,
            on_jump,
            Rc::new(move || refresh()),
        );
    });
}
```

Update `FileTree::new_with_context` callers (in `editor::mod.rs`) to pass the new arguments. Grab `record_key` from state, and pass an `on_notes_jump` closure that resolves the editor and calls `open_file` + cursor placement:

```rust
let on_notes_jump: Rc<dyn Fn(&Path, i32)> = {
    let state_c = state.clone();
    let tabs_c = tabs_rc.clone();
    Rc::new(move |path, line| {
        tabs_c.open_file(path, &state_c);
        // Defer scroll so the freshly-opened view has been laid out.
        let state_c2 = state_c.clone();
        let tabs_c2 = tabs_c.clone();
        gtk4::glib::idle_add_local_once(move || {
            let st = state_c2.borrow();
            let Some(idx) = st.active_tab else { return };
            let Some(open_file) = st.open_files.get(idx) else { return };
            let Some(buf) = open_file.source_buffer() else { return };
            let Some(iter) = buf.iter_at_line(line) else { return };
            buf.place_cursor(&iter);
            if let Some(view) = tabs_c2.active_source_view(&state_c2) {
                view.scroll_to_iter(&mut iter.clone(), 0.1, true, 0.5, 0.3);
            }
        });
    })
};
```

(Adjust the "active_source_view" helper call to whatever the editor exposes for retrieving the currently visible sourceview — see `mod.rs` for the existing scroll-to-line pattern around line 462 for reference.)

- [ ] **Step 4: Build**

Run: `cargo build --package pax-gui`
Expected: succeeds.

- [ ] **Step 5: Manual verification**

1. Create a few notes in two different files in the workspace.
2. Click the new **Notes** button in the sidebar. Dialog opens listing both notes with file + line + preview.
3. Type in the search box → rows filter live.
4. Select a row, click **Jump** → dialog closes, the target file opens (or is brought to front), cursor lands on the line.
5. Select, **Edit** → text dialog, change text, save → list reloads.
6. Select, **Delete** → row disappears, amber dot in the editor (if that tab is open) clears via on_notes_changed → refresh.
7. Delete the line carrying a note, save, reopen the file → note appears with "orphan" badge in the dialog; Jump still opens the file at the saved line number.

- [ ] **Step 6: Commit**

```bash
git add crates/tp-gui/src/dialogs/notes_dialog.rs crates/tp-gui/src/dialogs/mod.rs crates/tp-gui/src/panels/editor/file_tree.rs crates/tp-gui/src/panels/editor/mod.rs
git commit -m "$(cat <<'EOF'
Editor: workspace Notes dialog from sidebar button

Notes button lives next to Collapse All in the file-tree actions bar.
Clicking it opens a modal listing every note in the current workspace
(via Database::list_notes_for_workspace), with a live search over note
text and file path, plus Jump / Edit / Delete actions. Jump resolves
the full path through the workspace root and opens the file in the
editor, scrolling to the note's saved line. Edit and Delete update the
DB and notify the editor so the notes ruler stays in sync.
EOF
)"
```

---

## Task 9: Global Metadata Manager dialog + app menu entry

**Files:**
- Create: `crates/tp-gui/src/dialogs/metadata_manager.rs`
- Modify: `crates/tp-gui/src/dialogs/mod.rs`
- Modify: `crates/tp-gui/src/app.rs`

- [ ] **Step 1: Register module**

Edit `crates/tp-gui/src/dialogs/mod.rs`:

```rust
pub mod metadata_manager;
```

- [ ] **Step 2: Create `metadata_manager.rs`**

Full contents:

```rust
//! Cross-workspace metadata inspector. Lets the user browse every
//! workspace_file_metadata_entries row in the DB, filter by workspace and
//! entry type, search by substring, and bulk-delete (selection or whole
//! workspace).

use gtk4::prelude::*;
use pax_db::metadata_entries::MetadataEntry;
use pax_db::Database;
use std::cell::RefCell;
use std::rc::Rc;

const DIALOG_WIDTH_PX: i32 = 820;
const DIALOG_HEIGHT_PX: i32 = 520;
const ALL_WORKSPACES: &str = "(All workspaces)";
const ALL_TYPES: &str = "(All types)";
const PREVIEW_MAX_CHARS: usize = 80;

#[derive(Debug, Clone)]
struct WorkspaceRow {
    label: String,
    record_key: String,
}

pub fn show_metadata_manager(parent: Option<&gtk4::Window>) {
    let dialog = gtk4::Window::builder()
        .title("Workspace Metadata")
        .modal(true)
        .default_width(DIALOG_WIDTH_PX)
        .default_height(DIALOG_HEIGHT_PX)
        .build();
    if let Some(win) = parent {
        dialog.set_transient_for(Some(win));
    }

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(8);
    vbox.set_margin_bottom(8);
    vbox.set_margin_start(8);
    vbox.set_margin_end(8);

    // Filters.
    let filters = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let workspace_dropdown = gtk4::DropDown::from_strings(&[ALL_WORKSPACES]);
    let type_dropdown = gtk4::DropDown::from_strings(&[ALL_TYPES]);
    let search = gtk4::SearchEntry::new();
    search.set_placeholder_text(Some("Search file path or text…"));
    search.set_hexpand(true);
    filters.append(&workspace_dropdown);
    filters.append(&type_dropdown);
    filters.append(&search);
    vbox.append(&filters);

    // Results list (ListBox with multi-select).
    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Multiple);
    list_box.add_css_class("boxed-list");
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&list_box));
    scroll.set_vexpand(true);
    vbox.append(&scroll);

    // Actions.
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    actions.set_halign(gtk4::Align::End);
    let refresh_btn = gtk4::Button::with_label("Refresh");
    let delete_selected_btn = gtk4::Button::with_label("Delete selected");
    delete_selected_btn.add_css_class("destructive-action");
    let delete_workspace_btn = gtk4::Button::with_label("Delete all for workspace");
    delete_workspace_btn.add_css_class("destructive-action");
    delete_workspace_btn.set_sensitive(false);
    let close_btn = gtk4::Button::with_label("Close");
    actions.append(&refresh_btn);
    actions.append(&delete_selected_btn);
    actions.append(&delete_workspace_btn);
    actions.append(&close_btn);
    vbox.append(&actions);

    let workspaces: Rc<RefCell<Vec<WorkspaceRow>>> = Rc::new(RefCell::new(Vec::new()));
    let types: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let rows: Rc<RefCell<Vec<MetadataEntry>>> = Rc::new(RefCell::new(Vec::new()));

    let repopulate_filters = {
        let workspaces = workspaces.clone();
        let types = types.clone();
        let workspace_dropdown = workspace_dropdown.clone();
        let type_dropdown = type_dropdown.clone();
        Rc::new(move || {
            let db_path = Database::default_path();
            let Ok(db) = Database::open(&db_path) else {
                return;
            };
            let ws_rows: Vec<WorkspaceRow> = db
                .list_workspaces_limit(500)
                .unwrap_or_default()
                .into_iter()
                .map(|w| {
                    let rk = pax_db::workspaces::compute_record_key(
                        &w.name,
                        w.config_path.as_deref(),
                    );
                    let label = match &w.config_path {
                        Some(path) if !path.trim().is_empty() => {
                            format!("{} ({})", w.name, path)
                        }
                        _ => w.name.clone(),
                    };
                    WorkspaceRow { label, record_key: rk }
                })
                .collect();
            let mut labels = vec![ALL_WORKSPACES.to_string()];
            labels.extend(ws_rows.iter().map(|r| r.label.clone()));
            let labels_ref: Vec<&str> = labels.iter().map(String::as_str).collect();
            workspace_dropdown.set_model(Some(&gtk4::StringList::new(&labels_ref)));
            workspace_dropdown.set_selected(0);
            *workspaces.borrow_mut() = ws_rows;

            let type_rows: Vec<String> = db.list_metadata_entry_types().unwrap_or_default();
            let mut tlabels = vec![ALL_TYPES.to_string()];
            tlabels.extend(type_rows.iter().cloned());
            let tlabels_ref: Vec<&str> = tlabels.iter().map(String::as_str).collect();
            type_dropdown.set_model(Some(&gtk4::StringList::new(&tlabels_ref)));
            type_dropdown.set_selected(0);
            *types.borrow_mut() = type_rows;
        })
    };

    let reload = {
        let list_box = list_box.clone();
        let rows = rows.clone();
        let workspaces = workspaces.clone();
        let types = types.clone();
        let workspace_dropdown = workspace_dropdown.clone();
        let type_dropdown = type_dropdown.clone();
        let search = search.clone();
        let delete_workspace_btn = delete_workspace_btn.clone();
        Rc::new(move || {
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }
            let db_path = Database::default_path();
            let Ok(db) = Database::open(&db_path) else {
                return;
            };

            let search_text = search.text().to_string();
            let search_opt = (!search_text.is_empty()).then_some(search_text.as_str());

            let type_idx = type_dropdown.selected() as usize;
            let type_opt = if type_idx == 0 {
                None
            } else {
                types.borrow().get(type_idx - 1).cloned()
            };
            let type_param = type_opt.as_deref();

            let ws_idx = workspace_dropdown.selected() as usize;
            let ws_filter_key = if ws_idx == 0 {
                None
            } else {
                workspaces
                    .borrow()
                    .get(ws_idx - 1)
                    .map(|w| w.record_key.clone())
            };
            delete_workspace_btn.set_sensitive(ws_filter_key.is_some());

            let entries: Vec<MetadataEntry> = if let Some(rk) = &ws_filter_key {
                db.list_metadata_for_workspace(rk, type_param)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|e| match search_opt {
                        Some(q) => {
                            e.file_path.to_lowercase().contains(&q.to_lowercase())
                                || e.payload.to_lowercase().contains(&q.to_lowercase())
                        }
                        None => true,
                    })
                    .collect()
            } else {
                db.list_metadata_across_workspaces(search_opt, type_param)
                    .unwrap_or_default()
            };

            for (idx, entry) in entries.iter().enumerate() {
                list_box.append(&build_row(entry, idx));
            }
            *rows.borrow_mut() = entries;
        })
    };

    repopulate_filters();
    reload();

    {
        let reload = reload.clone();
        search.connect_search_changed(move |_| reload());
    }
    {
        let reload = reload.clone();
        workspace_dropdown.connect_selected_notify(move |_| reload());
    }
    {
        let reload = reload.clone();
        type_dropdown.connect_selected_notify(move |_| reload());
    }

    {
        let repopulate_filters = repopulate_filters.clone();
        let reload = reload.clone();
        refresh_btn.connect_clicked(move |_| {
            repopulate_filters();
            reload();
        });
    }

    {
        let list_box_c = list_box.clone();
        let rows = rows.clone();
        let reload = reload.clone();
        let dialog_c = dialog.clone();
        delete_selected_btn.connect_clicked(move |_| {
            let selected: Vec<i64> = list_box_c
                .selected_rows()
                .iter()
                .filter_map(|row| {
                    let idx: usize = row.widget_name().parse().ok()?;
                    rows.borrow().get(idx).map(|e| e.id)
                })
                .collect();
            if selected.is_empty() {
                return;
            }
            let count = selected.len();
            let reload = reload.clone();
            confirm_delete(
                Some(&dialog_c),
                &format!("Delete {} selected entries?", count),
                move || {
                    let db_path = Database::default_path();
                    if let Ok(db) = Database::open(&db_path) {
                        for id in &selected {
                            let _ = db.delete_metadata_entry(*id);
                        }
                    }
                    reload();
                },
            );
        });
    }

    {
        let workspaces = workspaces.clone();
        let workspace_dropdown = workspace_dropdown.clone();
        let reload = reload.clone();
        let dialog_c = dialog.clone();
        delete_workspace_btn.connect_clicked(move |_| {
            let ws_idx = workspace_dropdown.selected() as usize;
            if ws_idx == 0 {
                return;
            }
            let Some(ws) = workspaces.borrow().get(ws_idx - 1).cloned() else {
                return;
            };
            let reload = reload.clone();
            let record_key = ws.record_key.clone();
            confirm_delete(
                Some(&dialog_c),
                &format!("Delete every metadata entry for \"{}\"?", ws.label),
                move || {
                    let db_path = Database::default_path();
                    if let Ok(db) = Database::open(&db_path) {
                        let _ = db.delete_metadata_for_workspace(&record_key);
                    }
                    reload();
                },
            );
        });
    }

    {
        let d = dialog.clone();
        close_btn.connect_clicked(move |_| d.close());
    }

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn build_row(entry: &MetadataEntry, idx: usize) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_widget_name(&idx.to_string());

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);
    hbox.set_margin_top(4);
    hbox.set_margin_bottom(4);

    let type_label = gtk4::Label::new(Some(&entry.entry_type));
    type_label.add_css_class("dim-label");
    type_label.set_width_chars(8);
    type_label.set_xalign(0.0);
    hbox.append(&type_label);

    let ws_label = gtk4::Label::new(Some(&entry.record_key));
    ws_label.add_css_class("caption");
    ws_label.set_width_chars(32);
    ws_label.set_xalign(0.0);
    ws_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    hbox.append(&ws_label);

    let file_label = gtk4::Label::new(Some(&format!(
        "{}:{}",
        entry.file_path,
        entry.line_number + 1
    )));
    file_label.set_hexpand(true);
    file_label.set_xalign(0.0);
    file_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    hbox.append(&file_label);

    let preview = gtk4::Label::new(Some(&preview_of(&entry.payload)));
    preview.add_css_class("dim-label");
    preview.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    preview.set_width_chars(32);
    preview.set_xalign(0.0);
    hbox.append(&preview);

    row.set_child(Some(&hbox));
    row
}

fn preview_of(payload: &str) -> String {
    // Try to parse as {"text": "..."}; fall back to raw payload truncated.
    match serde_json::from_str::<serde_json::Value>(payload) {
        Ok(v) => {
            if let Some(text) = v.get("text").and_then(|v| v.as_str()) {
                return truncate_line(text);
            }
            truncate_line(payload)
        }
        Err(_) => truncate_line(payload),
    }
}

fn truncate_line(s: &str) -> String {
    let first = s.lines().next().unwrap_or("");
    if first.chars().count() > PREVIEW_MAX_CHARS {
        let t: String = first.chars().take(PREVIEW_MAX_CHARS).collect();
        format!("{}…", t)
    } else {
        first.to_string()
    }
}

fn confirm_delete(
    parent: Option<&gtk4::Window>,
    message: &str,
    on_confirm: impl Fn() + 'static,
) {
    let dialog = gtk4::Window::builder()
        .title("Confirm delete")
        .modal(true)
        .default_width(380)
        .default_height(120)
        .build();
    if let Some(win) = parent {
        dialog.set_transient_for(Some(win));
    }
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);
    vbox.append(&gtk4::Label::new(Some(message)));
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.set_halign(gtk4::Align::End);
    let cancel = gtk4::Button::with_label("Cancel");
    let confirm = gtk4::Button::with_label("Delete");
    confirm.add_css_class("destructive-action");
    row.append(&cancel);
    row.append(&confirm);
    vbox.append(&row);
    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| d.close());
    }
    {
        let d = dialog.clone();
        confirm.connect_clicked(move |_| {
            on_confirm();
            d.close();
        });
    }
    dialog.set_child(Some(&vbox));
    dialog.present();
}
```

- [ ] **Step 3: Hook into the app menu**

Edit `crates/tp-gui/src/app.rs`. Extend `APP_MENU_SETTINGS_ITEMS`:

```rust
const APP_MENU_SETTINGS_ITEMS: &[AppMenuItemSpec] = &[
    AppMenuItemSpec {
        label: "Settings…",
        action: "app.settings",
        icon: "preferences-system-symbolic",
        tooltip: "Open application settings",
    },
    AppMenuItemSpec {
        label: "Keyboard Shortcuts",
        action: "app.shortcuts",
        icon: "input-keyboard-symbolic",
        tooltip: "Show keyboard shortcuts",
    },
    AppMenuItemSpec {
        label: "Workspace Metadata…",
        action: "app.workspace_metadata",
        icon: "document-properties-symbolic",
        tooltip: "Browse, search, and delete metadata across all workspaces",
    },
    AppMenuItemSpec {
        label: "About Pax",
        action: "app.about",
        icon: "help-about-symbolic",
        tooltip: "Show application information",
    },
];
```

Register the action handler alongside the other `app.*` actions. Grep to find where `app.settings` / `app.about` are bound:

```bash
rg -n 'app\.settings|app\.about|ActionEntry' crates/tp-gui/src/app.rs
```

In that block add:

```rust
let workspace_metadata_action = gio::ActionEntry::builder("workspace_metadata")
    .activate({
        let window = window.clone();
        move |_, _, _| {
            crate::dialogs::metadata_manager::show_metadata_manager(Some(&window));
        }
    })
    .build();
```

Append it to the `add_action_entries` list.

- [ ] **Step 4: Build**

Run: `cargo build --package pax-gui`
Expected: succeeds.

- [ ] **Step 5: Manual verification**

1. Create notes in two or three separate workspaces (open each, add a few notes, close).
2. From the hamburger menu → Settings section → **Workspace Metadata…**
3. Dialog opens with all notes across workspaces.
4. Type in search → rows filter live.
5. Select Workspace dropdown = specific workspace → list narrows to that workspace; **Delete all for workspace** button enables.
6. Click it → confirm → entries for that workspace disappear.
7. Select 2–3 rows with Ctrl/Shift-click → **Delete selected** → confirm → only those go.
8. Close the manager; open the Notes dialog in a workspace that still has entries → list matches.

- [ ] **Step 6: Commit**

```bash
git add crates/tp-gui/src/dialogs/metadata_manager.rs crates/tp-gui/src/dialogs/mod.rs crates/tp-gui/src/app.rs
git commit -m "$(cat <<'EOF'
App menu: Workspace Metadata global manager

New "Workspace Metadata…" entry under the Settings section opens a
modal dialog that lists every row in workspace_file_metadata_entries
across all workspaces. Filters by workspace (from the workspace
metadata table), entry_type (distinct values from DB), and substring
search over file_path + payload. Supports multi-select delete and
"delete all for workspace". Jump-to-editor is intentionally omitted —
the manager is a cross-workspace inspector; per-workspace navigation
stays with the sidebar Notes dialog.
EOF
)"
```

---

## Coverage check

| Spec requirement | Task |
|---|---|
| DB schema: `workspace_file_metadata_entries`, indexes | 1 |
| Generic metadata CRUD (`metadata_entries` module) | 1 |
| `FileNote` typed wrapper (JSON payload `{"text"}`) | 2 |
| Workspace record_key helper | 1 + 3 |
| Editor knows its workspace (EditorState.record_key) | 3 |
| SourceTab carries `NotesState` | 4 |
| Mark-based session tracking + anchor content fallback | 4 |
| Notes ruler left of editor | 5 |
| Right-click Add / Edit / Delete Note on source lines | 6 |
| Async note load on open | 7 |
| Save-path flush of mark positions | 7 |
| Per-workspace Notes dialog with Jump / Edit / Delete | 8 |
| Sidebar "Notes" button next to Collapse All | 8 |
| Cross-workspace Metadata Manager from app menu | 9 |
| Multi-select delete + delete-per-workspace | 9 |
| Search + filter by entry_type + by workspace | 9 |
| No unit tests in commits | all |
| No `Co-Authored-By` trailers | all |
| Commit after each task | all |
| Named constants for numeric literals | all |

## Self-review

Placeholders: none. Every code block is complete for the step it belongs to. "Adjust … to match existing names" appears twice (Tasks 6 and 8) where the engineer must reconcile with functions/classes from prior work they are reading in context — both are clearly scoped ("grep for X and pick the same value", not a blanket TODO).

Type consistency: `FileNote`, `MetadataEntry`, `LiveNote`, `NotesState`, `NotesRuler` are consistent across tasks 1, 2, 4, 5, 6, 7, 8, 9. `record_key` threaded from Task 3 is read by Tasks 4, 6, 7, 8 without shape change. The `extras` callback signature change in Task 6 assumes a click-line parameter — the engineer is directed to update `text_context_menu::install` and its other callers in that same task.

Scope: one feature, one plan. No decomposition needed.
