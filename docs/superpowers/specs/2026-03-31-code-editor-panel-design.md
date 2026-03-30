# Code Editor Panel — Design Spec

## Overview

A lightweight embedded code editor panel for MyTerms, providing a mini-IDE experience inside a panel. Built on GtkSourceView 5 for syntax highlighting and editing, with a file tree sidebar, tabbed file editing, and Git integration.

## Panel Type

New variant in `PanelType` enum:

```rust
CodeEditor { root_dir: String }
```

JSON config:

```json
{
  "id": "editor1",
  "name": "Code",
  "panel_type": { "type": "code_editor", "root_dir": "/home/user/project" }
}
```

## Architecture

Single `CodeEditorPanel` implementing `PanelBackend`. Internal state centralized in `EditorState` shared via `Rc<RefCell<...>>`.

```rust
struct EditorState {
    root_dir: PathBuf,
    open_files: Vec<OpenFile>,
    active_tab: Option<usize>,
    sidebar_visible: bool,
    sidebar_mode: SidebarMode,
}

struct OpenFile {
    path: PathBuf,
    buffer: sourceview5::Buffer,
    modified: bool,
    last_disk_mtime: u64,
}

enum SidebarMode { Files, Git }
```

### File Structure

```
crates/tp-gui/src/panels/editor/
  mod.rs          -- CodeEditorPanel, EditorState, PanelBackend impl
  file_tree.rs    -- FileTree widget, lazy loading, context menu
  editor_tabs.rs  -- Tab management, buffer switching, dirty tracking
  git_status.rs   -- Status list, diff view, stage/unstage/commit
  file_watcher.rs -- Polling for open files, tree, git status
  fuzzy_finder.rs -- Ctrl+P search overlay
```

## Layout

Top-level widget: `gtk4::Paned` horizontal.

```
┌──────────────────────────────────────────────────────────┐
│ [Files] [Git]           ← Activity bar toggle            │
│─────────────┬────────────────────────────────────────────│
│ [Search]    │ [tab1.rs ●] [tab2.rs] [tab3.rs  x]        │
│─────────────│────────────────────────────────────────────│
│ src/        │ 1  use gtk4::prelude::*;                   │
│  panels/    │ 2  use sourceview5::prelude::*;            │
│   editor.rs │ 3                                          │
│   mod.rs    │ 4  pub struct CodeEditor {                 │
│  app.rs     │ 5      widget: gtk4::Widget,               │
│ Cargo.toml  │ 6  }                                       │
│─────────────│────────────────────────────────────────────│
│ [+File][+Dir]│ Rust | UTF-8 | LF       ● Modified Ln 4:8│
└──────────────┴───────────────────────────────────────────┘
```

- Sidebar visible by default, toggleable with Ctrl+B
- Paned is resizable; sidebar min width ~140px
- When hidden, a hamburger button in the tab bar reopens it

## Sidebar: File Tree

- `gtk4::TreeListModel` + `gtk4::ListView` for performance
- Lazy loading: expand directories on click
- Reads `.gitignore` via the `ignore` crate (same as ripgrep — handles recursive gitignore, `.git/`, etc.)
- Icons: folder open/closed, generic file icon
- Right-click context menu: New File, New Folder, Rename, Delete, Copy Path
- Double-click opens file in a tab

### Fuzzy Finder (Ctrl+P)

- `gtk4::SearchEntry` at the top of the file tree
- Uses the `fuzzy-matcher` crate (already a project dependency)
- Filters in real-time, shows results as flat list with relative paths
- Enter opens selected file, Escape closes finder

## Sidebar: Git Status View

- List of modified/staged/untracked files grouped by state
- Each row: status icon (M/A/D/?), relative path, [+] stage / [-] unstage buttons
- Click on a file opens diff view in editor area
- Bottom: text entry for commit message + Commit button
- Git operations via `std::process::Command` calling `git` directly (no libgit2)

## Editor Area: Tabs + SourceView

### Tab Bar

- `gtk4::Notebook` with custom tabs (label + modified dot + close button)
- `●` dot visible when file has unsaved changes
- Middle-click to close tab
- Tabs reorderable via native Notebook drag
- Closing a modified file: dialog "Save / Discard / Cancel"

### SourceView 5 Setup

- One `sourceview5::Buffer` per open file
- Single `sourceview5::View` that switches buffer on tab change
- Automatic language detection from file extension via `LanguageManager`
- Style scheme: follows workspace theme. Mapping: System/Catppuccin Latte/Solarized Light → "Adwaita" (light); Catppuccin Mocha/Solarized Dark/Nord/Dracula/Gruvbox/Tokyo Night → "Adwaita-dark" (dark). Falls back to "classic"/"classic-dark" if Adwaita schemes unavailable.
- Enabled features: line numbers, highlight current line, auto-indent, bracket matching, tab width 4, show right margin at 80/120
- Native undo/redo from Buffer

### Status Bar

- Detected language, encoding (UTF-8 default), line ending, cursor position
- `● Modified` indicator when file has unsaved changes

## Save Behavior

- Explicit save only: Ctrl+S
- `●` dot on tab indicates unsaved changes
- Buffer `changed` signal tracks dirty state by comparing with last saved content

## Git Diff + Revert per Hunk

### Inline Diff (while editing)

- Gutter indicators in line number column: green = added, red = removed, blue = modified
- Obtained by parsing `git diff` output and mapping line ranges
- Clicking a gutter indicator shows popup with original content + "Revert this change" button
- Gutter refreshes on each file save

### Dedicated Diff View (from Git Status)

- Click on a changed file opens a temporary split view:

```
┌─────────────────────┬─────────────────────┐
│  file.rs (HEAD)     │  file.rs (working)  │
│                     │                     │
│  fn old_code() {    │  fn new_code() {    │
│ -  let x = 1;      │ +  let x = 42;      │
│    ...              │    ...              │
│                     │  [Revert hunk]      │
└─────────────────────┴─────────────────────┘
```

- Two `sourceview5::View` side by side, read-only, synchronized line scrolling
- Hunks highlighted with colored background (green added, red removed)
- Each hunk has a "Revert" button that applies the inverse hunk to the working file
- "Revert All" button to discard all changes in the file
- Uses the `similar` crate for local diffing (works for untracked files too)

## File Watcher

Polling-based, same pattern as `MarkdownPanel` using `glib::timeout_add_local`.

### Open Files (1s interval)

- Compare file `mtime` with `last_disk_mtime` stored in `OpenFile`
- File not modified (clean): silent reload into buffer, update mtime
- File modified (dirty): show `gtk4::InfoBar` at top of editor: "File changed on disk. [Reload] [Keep yours]"

### File Tree (2s interval)

- Compare hash of directory structure (path list + mtime) with previous snapshot
- If changed: rebuild tree model (expanded directories stay expanded)
- Fuzzy finder index updates with the tree

### Git Status (3s interval)

- Poll with `git status --porcelain`
- Update changed file list only if output differs from previous
- Gutter indicators refresh on save, not on poll

## Keybindings

| Key | Action |
|-----|--------|
| Ctrl+S | Save active file |
| Ctrl+W | Close active tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Navigate between tabs |
| Ctrl+P | Fuzzy finder |
| Ctrl+B | Toggle sidebar |
| Ctrl+Shift+G | Switch sidebar to Git view |
| Ctrl+Z / Ctrl+Shift+Z | Undo / Redo (native SourceView) |
| Ctrl+F | Search in file (native SourceView) |

## Integration with Existing Code

### Modified Files

- `tp-core/workspace.rs`: Add `CodeEditor { root_dir: String }` to `PanelType`
- `tp-gui/panels/mod.rs`: Add `pub mod editor;`
- `tp-gui/backend_factory.rs`: Add mapping for `PanelType::CodeEditor` in `panel_type_to_id` and `panel_type_to_create_config`
- `tp-gui/panels/registry.rs`: Register `"code_editor"` in `build_default_registry`

### New Dependencies (tp-gui Cargo.toml)

- `ignore = "0.4"` — `.gitignore`-aware directory traversal
- `similar = "2"` — Rust-native diffing

### Feature Flag

- Code editor requires the `sourceview` feature flag — no fallback without syntax highlighting
- `CodeEditorPanel` exists only under `#[cfg(feature = "sourceview")]`
- Without the flag, registry still registers the type but factory shows a placeholder: "Code Editor requires sourceview feature"
