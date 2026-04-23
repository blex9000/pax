# File notes + generic workspace metadata — design

Status: approved
Date: 2026-04-23

## Goal

Add user-visible "notes" attached to file lines inside the Code Editor, persisted in the Pax SQLite database, scoped to the workspace they were taken in. The feature is designed as a concrete first instance of a broader "per-workspace, per-file metadata" facility so future types (bookmarks, review comments, etc.) drop into the same storage and UI without schema churn.

Primary user flows:

- Right-click a line in a source tab of the Code Editor → **Add Note** → text dialog → note saved.
- Re-open the file later (same workspace) → the note's line gets a marker in a dedicated vertical ruler to the left of the code, the right-click menu on that line offers Edit/Delete, and the note is reachable via the workspace Notes dialog.
- Sidebar button next to **Collapse all** → **Notes** dialog: list of every note in the current workspace with Jump / Edit / Delete.
- App menu → **Workspace Metadata…** → global manager across **all** workspaces with search, filter by workspace / entry type, multi-select delete, and "delete all entries for a workspace" bulk action.

## Non-goals

- Rich-text / markdown notes. First pass is plain multi-line text.
- Notes on remote (SSH-backed) files. The editor opens these today via `SshFileBackend`; the notes feature is local-backend-only for now. Remote files render notes as "orphan" when encountered in the lists.
- Sharing / sync of notes across machines. Storage is the per-user SQLite DB.
- Notes on non-source tabs (Markdown rendered, Image). Only `TabContent::Source` participates.

## Schema — single generic table

New migration `migrations/002_file_metadata.sql`:

```sql
CREATE TABLE workspace_file_metadata_entries (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    record_key   TEXT    NOT NULL,  -- matches workspace_metadata.record_key
    entry_type   TEXT    NOT NULL,  -- "note" today; extensible
    file_path    TEXT    NOT NULL,  -- relative to workspace root when possible,
                                    -- else absolute
    line_number  INTEGER NOT NULL,  -- 0-based, last persisted position
    line_anchor  TEXT,              -- content of the anchored line at save
                                    -- time; used to re-locate the note on
                                    -- reload when the line number shifted
    payload      TEXT    NOT NULL,  -- JSON, schema depends on entry_type
    created_at   INTEGER NOT NULL,  -- unix seconds
    updated_at   INTEGER NOT NULL
);

CREATE INDEX idx_wfme_lookup
    ON workspace_file_metadata_entries(record_key, entry_type, file_path);

CREATE INDEX idx_wfme_by_workspace
    ON workspace_file_metadata_entries(record_key);
```

Payload shape for `entry_type = "note"`:

```json
{ "text": "free-form multi-line string" }
```

Future types add their own payload shape without touching columns; the common bits (workspace, file, line, anchor, timestamps) stay in indexed columns.

## Storage layer (`pax-db`)

Two modules, layered:

**`crates/tp-db/src/metadata_entries.rs`** — generic.

```rust
pub struct MetadataEntry {
    pub id: i64,
    pub record_key: String,
    pub entry_type: String,
    pub file_path: String,
    pub line_number: i32,
    pub line_anchor: Option<String>,
    pub payload: String, // opaque JSON
    pub created_at: i64,
    pub updated_at: i64,
}

impl Database {
    pub fn insert_metadata_entry(&self, e: &MetadataEntry) -> Result<i64>;
    pub fn update_metadata_position(&self, id: i64, line: i32, anchor: Option<&str>) -> Result<()>;
    pub fn update_metadata_payload(&self, id: i64, payload: &str) -> Result<()>;
    pub fn delete_metadata_entry(&self, id: i64) -> Result<()>;
    pub fn delete_metadata_for_workspace(&self, record_key: &str) -> Result<usize>;
    pub fn list_metadata_by_file(&self, record_key: &str, entry_type: &str, file_path: &str) -> Result<Vec<MetadataEntry>>;
    pub fn list_metadata_for_workspace(&self, record_key: &str, entry_type: Option<&str>) -> Result<Vec<MetadataEntry>>;
    pub fn list_metadata_across_workspaces(&self, search: Option<&str>, entry_type: Option<&str>) -> Result<Vec<MetadataEntry>>;
}
```

**`crates/tp-db/src/notes.rs`** — thin `note` typing on top.

```rust
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

impl Database {
    pub fn add_note(&self, record_key: &str, file_path: &str, line: i32, anchor: Option<&str>, text: &str) -> Result<FileNote>;
    pub fn update_note_text(&self, id: i64, text: &str) -> Result<()>;
    pub fn list_notes_for_file(&self, record_key: &str, file_path: &str) -> Result<Vec<FileNote>>;
    pub fn list_notes_for_workspace(&self, record_key: &str) -> Result<Vec<FileNote>>;
}
```

The `notes` module serializes/deserializes the `{"text": ...}` payload and is the only place that knows about the `note` entry type. Shared operations (delete, position update) go through the generic API.

## Workspace identity

The DB's `workspace_metadata.record_key` is either `path:<config-file-path>` or `name:<workspace-name>`, resolved by `pax_db::record_workspace_open`. The notes feature uses the same `record_key`. A small helper lives on `pax_core::Workspace`:

```rust
impl Workspace {
    pub fn record_key(&self, config_path: Option<&Path>) -> String { ... }
}
```

Derived from the existing logic in `workspaces.rs::record_workspace_open`, reused rather than duplicated.

## File identity

`file_path` is stored as:

- Relative to the workspace root when `path.strip_prefix(editor_state.root_dir).is_ok()` — i.e. the common case for files inside the workspace.
- Absolute otherwise (files opened from outside the project tree; rare in practice).

This keeps notes portable if the user moves the workspace directory while keeping the same config.

## Line tracking (approach C: marks + content fallback)

Two distinct phases.

**During a session.** Each resolved note owns a `gtk::TextMark` in the source tab's `sourceview5::Buffer`. GTK's marks move along with edits for free, so the ruler + right-click menu track the correct line continuously as the user types.

**On load (fresh file open).** The editor has just built the buffer; now a background task reads all notes for `(record_key, file_path)` from the DB. For each note, resolve the target line:

1. If `buffer.line(note.line_number)` exists and its content equals `note.line_anchor`, use that line.
2. Else scan ±20 lines around `note.line_number` for an exact content match, use the nearest.
3. Else mark the note as **orphan** — no mark, no ruler entry, but it still appears in the lists with an "orphan" badge.

Resolved notes get a mark created at their line. The ruler is populated from the live set of marks.

**On save.** Before writing the file to disk, iterate the current session's notes for the active source tab; for each, read the mark's current line and the content of that line. Update `line_number` and `line_anchor` in the DB. This keeps the anchor fresh even if the user never reopens the file.

## UI — right-click Add/Edit/Delete on editor lines

`text_context_menu::install` on the source view accepts an `extras` factory. Add an **Add Note** item that, when the click position is on a line, opens a small dialog:

```
┌ Add note · <file>:<line> ────────┐
│ [multiline TextView, focus]      │
│                                  │
│        [ Cancel ] [ Save ]       │
└──────────────────────────────────┘
```

When the click lands on a line that **already has a note**, the context menu also shows **Edit Note** and **Delete Note** items (listing each note on that line if multiple). Multiple notes per line are allowed.

## UI — notes ruler

New widget `notes_ruler: gtk::DrawingArea`, 14px wide, placed **to the left** of the source view (opposite side from the match ruler, which stays on the right). Same `draw_func` pattern as `build_match_overview_ruler`:

- Reads `notes_lines: Rc<RefCell<Vec<i32>>>` (populated from marks' current lines).
- For each line, paints a small amber dot at the proportional y-position.
- Click → cursor jumps to nearest note line and a popover shows the note text with quick Edit/Delete buttons.

Hidden when no notes are present for the active tab.

## UI — per-workspace Notes dialog (sidebar button)

`file_tree.rs::actions_bar` appends a new button next to Collapse All:

```rust
let notes_btn = gtk4::Button::from_icon_name("user-bookmarks-symbolic");
notes_btn.add_css_class("flat");
notes_btn.set_tooltip_text(Some("Workspace notes"));
actions_bar.append(&notes_btn);
```

Click opens `WorkspaceNotesDialog` (`crates/tp-gui/src/dialogs/notes_dialog.rs`):

- Modal window, 600×420, title "Notes — <workspace name>".
- Top strip: search entry (filter by text), entry-type dropdown (currently only "Notes", wired generically so future types slot in).
- Middle: `ListView` with rows `[file relative path] · line N · first line of note text`. Orphan rows carry an "orphan" badge.
- Bottom toolbar: **Jump** (opens the file in the editor, cursor at the note), **Edit** (reopens the same Add/Edit dialog), **Delete** (confirm + remove).

## UI — Global Metadata Manager (app menu)

Add a new item under `APP_MENU_SETTINGS_ITEMS` in `app.rs`:

```rust
AppMenuItemSpec {
    label: "Workspace Metadata…",
    action: "app.workspace_metadata",
    icon: "document-properties-symbolic",
    tooltip: "Browse, search, and delete metadata across all workspaces",
},
```

New dialog `MetadataManagerDialog` (`crates/tp-gui/src/dialogs/metadata_manager.rs`):

- Modal window, 820×520, title "Workspace Metadata".
- Top strip:
  - **Workspace** dropdown — "All workspaces" + one entry per row in `workspace_metadata` (name labeled, tooltip = config path).
  - **Type** dropdown — "All types" + unique `entry_type` values from the DB.
  - Search entry — substring match over `file_path` and `payload`.
- Middle: `ColumnView` with columns: `Workspace`, `Type`, `File`, `Line`, `Preview`. Multi-select enabled (`SelectionMode::Multiple`).
- Right-side action column or bottom bar:
  - **Delete selected** — confirm, then delete rows by id.
  - **Delete all for workspace** — enabled when a single workspace is chosen in the dropdown; confirm with workspace name; bulk delete.
  - **Refresh** — re-runs the query.

The query routes through `Database::list_metadata_across_workspaces(search, entry_type)` plus optional record_key filter. Deletions go through `delete_metadata_entry` / `delete_metadata_for_workspace`.

Open actions on entries in the manager do **not** jump into the editor (the manager is a cross-workspace inspector — jumping would have to open the target workspace first, which is out of scope). Jump is reserved to the per-workspace Notes dialog.

## Async load — don't block file open

In `EditorTabs::open_file`, for `TabContent::Source`, after the buffer is built and the tab shown:

```rust
let path_rel = relative_file_path(&state.root_dir, path);
let record_key = state.borrow().record_key.clone();
super::task::run_blocking(
    move || db.list_notes_for_file(&record_key, &path_rel),
    move |notes| {
        apply_notes_to_tab(&notes, &tab_handle);
    },
);
```

`apply_notes_to_tab` runs on the GTK main thread, resolves each note's line (anchor match), creates marks, and populates `notes_lines` for the ruler. If the tab was closed before the task finished, the callback detects the missing tab and drops the result.

## Notes state tracking

New module `crates/tp-gui/src/panels/editor/notes_state.rs`:

- Per-source-tab `NotesState { entries: Vec<NoteEntry> }` stored on `SourceTab` (`tab_content.rs`), where `NoteEntry = { db_id, mark: Option<gtk::TextMark>, text: String, anchor: Option<String> }`. Orphan notes have `mark = None`.
- Functions: `apply_loaded_notes`, `add_note`, `edit_note`, `delete_note`, `resolve_line_from_mark`, `flush_positions_to_db` (called from the save path).
- The ruler widget reads lines from this module.

## Save-path hook

`EditorTabs::save_active`, after a successful `backend.write_file`:

```rust
if let Some(st) = source_tab_notes_state(&open_file) {
    notes_state::flush_positions_to_db(&st, &record_key, &file_path, &db);
}
```

This persists the updated line numbers + anchors before the next reload.

## Concurrency / error handling

- DB access goes through the existing `Database` wrapper (serialized `rusqlite::Connection` behind a `Mutex`), same as `workspaces.rs` and friends.
- Background loads use `task::run_blocking` — same pattern as the file watcher and project search.
- Any DB error is logged via `tracing::warn!` and the UI degrades (no notes shown); never panics.
- Orphan notes remain in DB — we don't auto-delete them. The user can clean them up through the manager.

## Files touched

New:

- `migrations/002_file_metadata.sql`
- `crates/tp-db/src/metadata_entries.rs`
- `crates/tp-db/src/notes.rs`
- `crates/tp-gui/src/dialogs/notes_dialog.rs`
- `crates/tp-gui/src/dialogs/metadata_manager.rs`
- `crates/tp-gui/src/panels/editor/notes_state.rs`
- `crates/tp-gui/src/panels/editor/notes_ruler.rs`

Modified:

- `crates/tp-db/src/lib.rs` — `pub mod metadata_entries;`, `pub mod notes;`, include migration 002 in `run_migrations`.
- `crates/tp-core/src/workspace.rs` — `Workspace::record_key()` helper (migrated from the private logic in `tp-db/src/workspaces.rs`).
- `crates/tp-gui/src/app.rs` — new Settings menu item + `app.workspace_metadata` action handler opening `MetadataManagerDialog`.
- `crates/tp-gui/src/dialogs/mod.rs` — register new dialog modules.
- `crates/tp-gui/src/panels/editor/tab_content.rs` — `SourceTab.notes: NotesState` (default empty).
- `crates/tp-gui/src/panels/editor/editor_tabs.rs` — extras callback extends right-click menu with Add / Edit / Delete Note, `open_file` kicks off async note load, `save_active` flushes mark positions.
- `crates/tp-gui/src/panels/editor/text_context_menu.rs` — extras already passes click position; no change beyond consuming it.
- `crates/tp-gui/src/panels/editor/file_tree.rs` — Notes button in `actions_bar`, wired to `WorkspaceNotesDialog`.

## Test plan

Manual, per project convention (no unit tests in commits):

1. Right-click a line in a `.rs` file → Add Note → type a multi-line message → Save. Note marker appears in the new left ruler.
2. Close + reopen the file → marker still there at the same line.
3. Insert 3 empty lines above the note, save the file, reopen from scratch → marker follows the original content.
4. Modify the line's content subtly, save, reopen → marker still finds it via ±20 line fuzzy match.
5. Delete the noted line entirely, save, reopen → marker gone, note shows as orphan in the workspace Notes dialog.
6. Sidebar **Notes** button → dialog lists every note, search works, Jump focuses the editor on the line, Edit updates text, Delete removes.
7. App menu **Workspace Metadata…** → manager dialog lists notes across multiple workspaces. Workspace + Type dropdowns filter. Search text filters by file/payload. Multi-select + Delete works. Switch workspace dropdown to a specific one → "Delete all for workspace" button enables; confirm deletes only that workspace's entries.
8. Open a file from a different workspace: notes from workspace A are **not** shown in workspace B (scoping works).

## Risks

- **Mark lifetime**: a `gtk::TextMark` lives with its buffer; if the tab is closed the mark disappears. `NotesState` must be per-tab, not shared, and release on tab close — `OpenFile` drop path.
- **Anchor false positives**: a line of common content (`}`, `    }`) might collide in fuzzy match. Accepted risk; first pass prefers exact numeric match, then exact content match, then nothing. No approximate matching.
- **Cross-workspace delete in manager** is irreversible. Both delete paths require a confirm dialog naming the exact count or workspace before proceeding.
- **Remote (SSH) tabs**: we don't attach notes state. An existing note on a file that matches a remote path gets no ruler and no right-click entries — but still appears as orphan in the manager.
