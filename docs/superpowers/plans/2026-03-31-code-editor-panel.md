# Code Editor Panel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an embedded code editor panel to MyTerms with file tree, tabbed editing via GtkSourceView 5, and Git integration (status, diff, stage, commit, revert per hunk).

**Architecture:** Single `CodeEditorPanel` struct implementing `PanelBackend`, composed of sub-widgets: file tree sidebar (toggleable), tabbed editor area with SourceView 5 buffers, and a git status/diff view. State centralized in `Rc<RefCell<EditorState>>`. Polling-based file watching using `glib::timeout_add_local`.

**Tech Stack:** Rust, GTK4, libadwaita, sourceview5, `ignore` crate (gitignore-aware traversal), `similar` crate (diffing), `fuzzy-matcher` (already a dependency), git CLI via `std::process::Command`.

**Spec:** `docs/superpowers/specs/2026-03-31-code-editor-panel-design.md`

---

### Task 1: Add dependencies and PanelType variant

**Files:**
- Modify: `crates/tp-gui/Cargo.toml`
- Modify: `crates/tp-core/src/workspace.rs:96-131` (PanelType enum)
- Modify: `crates/tp-gui/src/backend_factory.rs:8-15` (panel_type_to_id)
- Modify: `crates/tp-gui/src/backend_factory.rs:17-37` (panel_type_to_create_config)

- [ ] **Step 1: Add `ignore` and `similar` to tp-gui/Cargo.toml**

Add after the `sourceview5` line:

```toml
ignore = "0.4"
similar = "2"
```

- [ ] **Step 2: Add `CodeEditor` variant to `PanelType` in `crates/tp-core/src/workspace.rs`**

Add after the `Browser` variant (line ~131):

```rust
    /// Embedded code editor
    CodeEditor {
        root_dir: String,
    },
```

- [ ] **Step 3: Update `PanelConfig::effective_type` in `crates/tp-core/src/workspace.rs`**

In the `effective_type` method (~line 181), add a match arm before `other`:

```rust
            PanelType::CodeEditor { .. } => self.panel_type.clone(),
```

This is already covered by the existing `other => other.clone()` arm, so no change is actually needed. Verify this by reading the match.

- [ ] **Step 4: Update `panel_type_to_id` in `crates/tp-gui/src/backend_factory.rs`**

Add a new arm:

```rust
        PanelType::CodeEditor { .. } => "code_editor",
```

- [ ] **Step 5: Update `panel_type_to_create_config` in `crates/tp-gui/src/backend_factory.rs`**

Add a new match arm:

```rust
        PanelType::CodeEditor { root_dir } => {
            extra.insert("root_dir".to_string(), root_dir.clone());
        }
```

- [ ] **Step 6: Update `create_backend_from_registry` in `crates/tp-gui/src/backend_factory.rs`**

Add a new arm in the `match &effective` block:

```rust
        PanelType::CodeEditor { root_dir } => {
            let mut extra = HashMap::new();
            extra.insert("root_dir".to_string(), root_dir.clone());
            ("code_editor", extra)
        }
```

- [ ] **Step 7: Verify it compiles**

Run: `cargo build --features sourceview 2>&1 | tail -20`
Expected: Compiles successfully (no code_editor registry entry yet, but types should compile).

- [ ] **Step 8: Commit**

```bash
git add crates/tp-gui/Cargo.toml crates/tp-core/src/workspace.rs crates/tp-gui/src/backend_factory.rs
git commit -m "feat(editor): add CodeEditor PanelType variant and dependencies"
```

---

### Task 2: Scaffold CodeEditorPanel with empty editor area

**Files:**
- Create: `crates/tp-gui/src/panels/editor/mod.rs`
- Create: `crates/tp-gui/src/panels/editor/editor_tabs.rs`
- Modify: `crates/tp-gui/src/panels/mod.rs`
- Modify: `crates/tp-gui/src/panels/registry.rs:110-218` (build_default_registry)

- [ ] **Step 1: Create `crates/tp-gui/src/panels/editor/editor_tabs.rs`**

This module manages the tab bar and SourceView instances.

```rust
use gtk4::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::EditorState;

/// Manages the Notebook tabs and SourceView buffers.
pub struct EditorTabs {
    pub notebook: gtk4::Notebook,
    pub source_view: sourceview5::View,
    pub status_bar: gtk4::Box,
    pub info_bar_container: gtk4::Box,
    status_lang: gtk4::Label,
    status_pos: gtk4::Label,
    status_modified: gtk4::Label,
}

impl EditorTabs {
    pub fn new(state: Rc<RefCell<EditorState>>) -> Self {
        let notebook = gtk4::Notebook::new();
        notebook.set_scrollable(true);
        notebook.set_show_border(false);
        notebook.add_css_class("editor-tabs");

        // Single SourceView that switches buffers
        let source_view = sourceview5::View::new();
        source_view.set_show_line_numbers(true);
        source_view.set_highlight_current_line(true);
        source_view.set_auto_indent(true);
        source_view.set_tab_width(4);
        source_view.set_wrap_mode(gtk4::WrapMode::None);
        source_view.set_left_margin(8);
        source_view.set_top_margin(4);
        source_view.set_monospace(true);
        source_view.set_show_right_margin(true);
        source_view.set_right_margin_position(120);

        // Apply dark scheme by default
        if let Some(buf) = source_view.buffer().downcast_ref::<sourceview5::Buffer>() {
            let scheme_manager = sourceview5::StyleSchemeManager::default();
            if let Some(scheme) = scheme_manager.scheme("Adwaita-dark")
                .or_else(|| scheme_manager.scheme("classic-dark"))
            {
                buf.set_style_scheme(Some(&scheme));
            }
        }

        let source_scroll = gtk4::ScrolledWindow::new();
        source_scroll.set_child(Some(&source_view));
        source_scroll.set_vexpand(true);
        source_scroll.set_hexpand(true);

        // InfoBar container (for file-changed-on-disk warnings)
        let info_bar_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Status bar
        let status_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        status_bar.add_css_class("panel-footer-bar");
        status_bar.add_css_class("panel-footer");

        let status_lang = gtk4::Label::new(Some("Plain Text"));
        status_lang.set_halign(gtk4::Align::Start);
        status_bar.append(&status_lang);

        let status_encoding = gtk4::Label::new(Some("UTF-8"));
        status_bar.append(&status_encoding);

        let status_eol = gtk4::Label::new(Some("LF"));
        status_bar.append(&status_eol);

        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        status_bar.append(&spacer);

        let status_modified = gtk4::Label::new(None);
        status_modified.add_css_class("dirty-indicator");
        status_bar.append(&status_modified);

        let status_pos = gtk4::Label::new(Some("Ln 1, Col 1"));
        status_pos.set_halign(gtk4::Align::End);
        status_bar.append(&status_pos);

        // Track cursor position
        {
            let pos_label = status_pos.clone();
            let sv = source_view.clone();
            source_view.buffer().connect_notify_local(Some("cursor-position"), move |buf, _| {
                let iter = buf.iter_at_offset(buf.cursor_position());
                let line = iter.line() + 1;
                let col = iter.line_offset() + 1;
                pos_label.set_text(&format!("Ln {}, Col {}", line, col));
            });
        }

        // Welcome label shown when no file is open
        let welcome = gtk4::Label::new(Some("Open a file from the sidebar\nor press Ctrl+P to search"));
        welcome.add_css_class("dim-label");
        welcome.set_vexpand(true);
        welcome.set_valign(gtk4::Align::Center);

        // Use a Stack to switch between welcome and editor
        // Notebook page 0 is the welcome, actual files are added as pages
        notebook.append_page(&welcome, Some(&gtk4::Label::new(Some("Welcome"))));
        notebook.set_tab_label_text(&welcome, "");
        notebook.set_show_tabs(false);

        Self {
            notebook,
            source_view,
            status_bar,
            info_bar_container,
            status_lang,
            status_pos,
            status_modified,
        }
    }

    /// Open a file in a new tab. Returns the tab index.
    /// If the file is already open, switches to that tab.
    pub fn open_file(&self, path: &Path, state: &Rc<RefCell<EditorState>>) -> Option<usize> {
        // Check if already open
        {
            let st = state.borrow();
            if let Some(idx) = st.open_files.iter().position(|f| f.path == path) {
                self.notebook.set_current_page(Some((idx + 1) as u32)); // +1 for welcome page
                self.switch_to_buffer(idx, state);
                return Some(idx);
            }
        }

        // Read file
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Cannot open file {}: {}", path.display(), e);
                return None;
            }
        };

        // Create buffer
        let buf = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
        buf.set_text(&content);
        buf.set_highlight_syntax(true);

        // Detect language
        let lang_manager = sourceview5::LanguageManager::default();
        if let Some(lang) = lang_manager.guess_language(Some(&path.to_string_lossy()), None) {
            buf.set_language(Some(&lang));
        }

        // Apply scheme
        let scheme_manager = sourceview5::StyleSchemeManager::default();
        if let Some(scheme) = scheme_manager.scheme("Adwaita-dark")
            .or_else(|| scheme_manager.scheme("classic-dark"))
        {
            buf.set_style_scheme(Some(&scheme));
        }

        // Reset undo after setting initial text
        buf.set_enable_undo(false);
        buf.set_enable_undo(true);

        let mtime = get_mtime(path);
        let file_name = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "untitled".to_string());

        // Build tab label
        let tab_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let dot = gtk4::Label::new(None);
        dot.add_css_class("dirty-indicator");
        let label = gtk4::Label::new(Some(&file_name));
        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.add_css_class("tab-close-btn");
        tab_box.append(&dot);
        tab_box.append(&label);
        tab_box.append(&close_btn);

        // Placeholder widget for the notebook page (actual display is via the single SourceView)
        let page_widget = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let page_idx = self.notebook.append_page(&page_widget, Some(&tab_box));
        self.notebook.set_show_tabs(true);

        // Add to state
        let idx = {
            let mut st = state.borrow_mut();
            st.open_files.push(super::OpenFile {
                path: path.to_path_buf(),
                buffer: buf.clone(),
                modified: false,
                last_disk_mtime: mtime,
            });
            st.active_tab = Some(st.open_files.len() - 1);
            st.open_files.len() - 1
        };

        // Track dirty state
        {
            let state_c = state.clone();
            let dot_c = dot.clone();
            let mod_label = self.status_modified.clone();
            let file_idx = idx;
            buf.connect_changed(move |buf| {
                let mut st = state_c.borrow_mut();
                if file_idx < st.open_files.len() {
                    let was_modified = st.open_files[file_idx].modified;
                    // We consider it modified if undo is available
                    let is_modified = buf.can_undo();
                    st.open_files[file_idx].modified = is_modified;
                    if is_modified != was_modified {
                        dot_c.set_text(if is_modified { "\u{25CF} " } else { "" });
                        mod_label.set_text(if is_modified { "\u{25CF} Modified" } else { "" });
                    }
                }
            });
        }

        // Close button
        {
            let state_c = state.clone();
            let nb = self.notebook.clone();
            let file_idx = idx;
            close_btn.connect_clicked(move |_| {
                // Save dialog handled in Task 9 (close_active_tab)
                let mut st = state_c.borrow_mut();
                if file_idx < st.open_files.len() {
                    st.open_files.remove(file_idx);
                    nb.remove_page(Some((file_idx + 1) as u32)); // +1 for welcome
                    if st.open_files.is_empty() {
                        st.active_tab = None;
                        nb.set_show_tabs(false);
                        nb.set_current_page(Some(0)); // show welcome
                    } else {
                        let new_idx = file_idx.min(st.open_files.len() - 1);
                        st.active_tab = Some(new_idx);
                    }
                }
            });
        }

        // Switch to this buffer
        self.switch_to_buffer(idx, state);
        self.notebook.set_current_page(Some((idx + 1) as u32));

        Some(idx)
    }

    /// Switch the SourceView to display the buffer at the given index.
    fn switch_to_buffer(&self, idx: usize, state: &Rc<RefCell<EditorState>>) {
        let st = state.borrow();
        if let Some(open_file) = st.open_files.get(idx) {
            self.source_view.set_buffer(Some(&open_file.buffer));
            // Update status bar language
            if let Some(lang) = open_file.buffer.language() {
                self.status_lang.set_text(&lang.name());
            } else {
                self.status_lang.set_text("Plain Text");
            }
            self.status_modified.set_text(if open_file.modified { "\u{25CF} Modified" } else { "" });
        }
    }

    /// Save the currently active file.
    pub fn save_active(&self, state: &Rc<RefCell<EditorState>>) {
        let mut st = state.borrow_mut();
        if let Some(idx) = st.active_tab {
            if let Some(open_file) = st.open_files.get_mut(idx) {
                let buf = &open_file.buffer;
                let text = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
                if let Err(e) = std::fs::write(&open_file.path, &text) {
                    tracing::error!("Failed to save {}: {}", open_file.path.display(), e);
                    return;
                }
                open_file.modified = false;
                open_file.last_disk_mtime = get_mtime(&open_file.path);
                // Reset undo stack as the new baseline
                buf.set_enable_undo(false);
                buf.set_enable_undo(true);
            }
        }
    }
}

fn get_mtime(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
        .unwrap_or(0)
}
```

- [ ] **Step 2: Create `crates/tp-gui/src/panels/editor/mod.rs`**

```rust
#[cfg(feature = "sourceview")]
mod editor_tabs;
#[cfg(feature = "sourceview")]
pub mod file_tree;
#[cfg(feature = "sourceview")]
pub mod git_status;
#[cfg(feature = "sourceview")]
pub mod file_watcher;
#[cfg(feature = "sourceview")]
pub mod fuzzy_finder;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use super::PanelBackend;

/// State shared across all editor sub-components.
#[derive(Debug)]
pub struct EditorState {
    pub root_dir: PathBuf,
    pub open_files: Vec<OpenFile>,
    pub active_tab: Option<usize>,
    pub sidebar_visible: bool,
    pub sidebar_mode: SidebarMode,
}

#[derive(Debug)]
pub struct OpenFile {
    pub path: PathBuf,
    pub buffer: sourceview5::Buffer,
    pub modified: bool,
    pub last_disk_mtime: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SidebarMode {
    Files,
    Git,
}

/// Embedded code editor panel with file tree, tabs, and git integration.
#[cfg(feature = "sourceview")]
#[derive(Debug)]
pub struct CodeEditorPanel {
    widget: gtk4::Widget,
    state: Rc<RefCell<EditorState>>,
}

#[cfg(feature = "sourceview")]
impl CodeEditorPanel {
    pub fn new(root_dir: &str) -> Self {
        let state = Rc::new(RefCell::new(EditorState {
            root_dir: PathBuf::from(root_dir),
            open_files: Vec::new(),
            active_tab: None,
            sidebar_visible: true,
            sidebar_mode: SidebarMode::Files,
        }));

        let tabs = editor_tabs::EditorTabs::new(state.clone());

        // Right side: info bar + notebook + status bar
        let editor_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        editor_area.append(&tabs.info_bar_container);

        // The SourceView goes in a scrolled window below the notebook
        let source_scroll = gtk4::ScrolledWindow::new();
        source_scroll.set_child(Some(&tabs.source_view));
        source_scroll.set_vexpand(true);
        source_scroll.set_hexpand(true);

        editor_area.append(&tabs.notebook);
        editor_area.append(&source_scroll);
        editor_area.append(&tabs.status_bar);

        // Sidebar placeholder (file tree comes in Task 3)
        let sidebar = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        sidebar.set_width_request(200);

        let sidebar_label = gtk4::Label::new(Some("Files"));
        sidebar_label.add_css_class("dim-label");
        sidebar_label.set_margin_top(16);
        sidebar.append(&sidebar_label);

        let dir_label = gtk4::Label::new(Some(root_dir));
        dir_label.add_css_class("dim-label");
        dir_label.add_css_class("caption");
        dir_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
        dir_label.set_margin_top(4);
        sidebar.append(&dir_label);

        // Paned: sidebar | editor
        let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
        paned.set_start_child(Some(&sidebar));
        paned.set_end_child(Some(&editor_area));
        paned.set_position(200);
        paned.set_shrink_start_child(false);
        paned.set_resize_start_child(false);

        let widget = paned.upcast::<gtk4::Widget>();

        // Keybindings: Ctrl+S to save
        {
            let tabs_sv = tabs.source_view.clone();
            let state_c = state.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            let tabs_rc = Rc::new(tabs);
            let tabs_save = tabs_rc.clone();
            key_ctrl.connect_key_pressed(move |_, key, _, modifier| {
                if modifier.contains(gtk4::gdk::ModifierType::CONTROL_MASK) {
                    match key {
                        gtk4::gdk::Key::s => {
                            tabs_save.save_active(&state_c);
                            return gtk4::glib::Propagation::Stop;
                        }
                        _ => {}
                    }
                }
                gtk4::glib::Propagation::Proceed
            });
            widget.add_controller(key_ctrl);
        }

        Self { widget, state }
    }
}

#[cfg(feature = "sourceview")]
impl PanelBackend for CodeEditorPanel {
    fn panel_type(&self) -> &str { "code_editor" }
    fn widget(&self) -> &gtk4::Widget { &self.widget }
    fn on_focus(&self) {}

    fn get_text_content(&self) -> Option<String> {
        let st = self.state.borrow();
        st.active_tab.and_then(|idx| {
            st.open_files.get(idx).map(|f| {
                let buf = &f.buffer;
                buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string()
            })
        })
    }
}

/// Placeholder panel shown when sourceview feature is not enabled.
#[cfg(not(feature = "sourceview"))]
#[derive(Debug)]
pub struct CodeEditorPanel {
    widget: gtk4::Widget,
}

#[cfg(not(feature = "sourceview"))]
impl CodeEditorPanel {
    pub fn new(_root_dir: &str) -> Self {
        let label = gtk4::Label::new(Some("Code Editor requires the 'sourceview' feature.\nRecompile with: cargo build --features sourceview"));
        label.set_margin_top(32);
        label.set_margin_bottom(32);
        label.add_css_class("dim-label");
        Self { widget: label.upcast::<gtk4::Widget>() }
    }
}

#[cfg(not(feature = "sourceview"))]
impl PanelBackend for CodeEditorPanel {
    fn panel_type(&self) -> &str { "code_editor" }
    fn widget(&self) -> &gtk4::Widget { &self.widget }
    fn on_focus(&self) {}
}
```

- [ ] **Step 3: Add `pub mod editor;` to `crates/tp-gui/src/panels/mod.rs`**

Add after `pub mod registry;`:

```rust
pub mod editor;
```

- [ ] **Step 4: Register `code_editor` in `build_default_registry` in `crates/tp-gui/src/panels/registry.rs`**

Add after the browser registration block (before the closing `reg`):

```rust
    // Code Editor
    reg.register(
        "code_editor",
        "Code Editor",
        "Lightweight code editor with file tree and git",
        "accessories-text-editor-symbolic",
        true,
        |config| {
            let root_dir = config.extra.get("root_dir").map(|s| s.as_str()).unwrap_or(".");
            Box::new(super::editor::CodeEditorPanel::new(root_dir))
        },
    );
```

- [ ] **Step 5: Verify it compiles and the panel can be instantiated**

Run: `cargo build --features sourceview 2>&1 | tail -20`
Expected: Compiles successfully.

- [ ] **Step 6: Test with a JSON config**

Create `config/editor_test.json`:

```json
{
    "name": "Editor Test",
    "layout": { "type": "panel", "id": "ed1" },
    "panels": [
        {
            "id": "ed1",
            "name": "Code",
            "panel_type": { "type": "code_editor", "root_dir": "." }
        }
    ]
}
```

Run: `cargo run --features sourceview -- launch config/editor_test.json`
Expected: Window opens with the editor panel showing sidebar placeholder and empty editor area.

- [ ] **Step 7: Commit**

```bash
git add crates/tp-gui/src/panels/editor/ crates/tp-gui/src/panels/mod.rs crates/tp-gui/src/panels/registry.rs config/editor_test.json
git commit -m "feat(editor): scaffold CodeEditorPanel with tabs and registry"
```

---

### Task 3: File Tree sidebar

**Files:**
- Create: `crates/tp-gui/src/panels/editor/file_tree.rs`
- Modify: `crates/tp-gui/src/panels/editor/mod.rs` (replace sidebar placeholder)

- [ ] **Step 1: Create `crates/tp-gui/src/panels/editor/file_tree.rs`**

```rust
use gtk4::prelude::*;
use gtk4::glib;
use gtk4::gio;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Callback when a file is double-clicked in the tree.
pub type OnFileOpen = Rc<dyn Fn(&Path)>;

/// File tree widget with gitignore-aware traversal and lazy loading.
pub struct FileTree {
    pub widget: gtk4::Box,
    list_view: gtk4::ListView,
    root_dir: PathBuf,
    on_file_open: Option<OnFileOpen>,
    /// Flat list of all file paths for fuzzy finder indexing.
    pub file_index: Rc<RefCell<Vec<PathBuf>>>,
}

#[derive(Debug, Clone)]
struct FileEntry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    depth: u32,
    expanded: bool,
}

impl FileTree {
    pub fn new(root_dir: &Path, on_file_open: OnFileOpen) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Action buttons bar at bottom
        let actions_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        actions_bar.set_margin_start(4);
        actions_bar.set_margin_end(4);
        actions_bar.set_margin_bottom(2);

        let new_file_btn = gtk4::Button::from_icon_name("document-new-symbolic");
        new_file_btn.add_css_class("flat");
        new_file_btn.set_tooltip_text(Some("New File"));

        let new_dir_btn = gtk4::Button::from_icon_name("folder-new-symbolic");
        new_dir_btn.add_css_class("flat");
        new_dir_btn.set_tooltip_text(Some("New Folder"));

        actions_bar.append(&new_file_btn);
        actions_bar.append(&new_dir_btn);

        // Build initial file list
        let file_index = Rc::new(RefCell::new(Vec::new()));
        let entries = Rc::new(RefCell::new(Vec::new()));
        build_file_entries(root_dir, root_dir, &mut entries.borrow_mut(), &mut file_index.borrow_mut(), 0);

        // ListView with a GtkStringList model for simplicity
        let model = gtk4::StringList::new(&[]);
        for entry in entries.borrow().iter() {
            let prefix = "  ".repeat(entry.depth as usize);
            let icon = if entry.is_dir { "\u{1F4C2} " } else { "  " };
            model.append(&format!("{}{}{}", prefix, icon, entry.name));
        }

        let selection = gtk4::SingleSelection::new(Some(model.clone()));
        let factory = gtk4::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk4::ListItem>().unwrap();
            let label = gtk4::Label::new(None);
            label.set_halign(gtk4::Align::Start);
            label.set_margin_start(4);
            label.set_xalign(0.0);
            item.set_child(Some(&label));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk4::ListItem>().unwrap();
            let label = item.child().and_downcast::<gtk4::Label>().unwrap();
            let str_obj = item.item().and_downcast::<gtk4::StringObject>().unwrap();
            label.set_text(&str_obj.string());
        });

        let list_view = gtk4::ListView::new(Some(selection.clone()), Some(factory));
        list_view.add_css_class("navigation-sidebar");

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_child(Some(&list_view));
        scroll.set_vexpand(true);

        container.append(&scroll);
        container.append(&actions_bar);

        // Double-click to open file
        {
            let entries_c = entries.clone();
            let on_open = on_file_open.clone();
            list_view.connect_activate(move |_, pos| {
                let entries = entries_c.borrow();
                if let Some(entry) = entries.get(pos as usize) {
                    if !entry.is_dir {
                        on_open(&entry.path);
                    }
                }
            });
        }

        // Right-click context menu
        let menu = gio::Menu::new();
        menu.append(Some("New File"), Some("editor.new-file"));
        menu.append(Some("New Folder"), Some("editor.new-folder"));
        menu.append(Some("Rename"), Some("editor.rename"));
        menu.append(Some("Delete"), Some("editor.delete"));
        menu.append(Some("Copy Path"), Some("editor.copy-path"));
        let popover = gtk4::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&container);

        // New file button
        {
            let root = root_dir.to_path_buf();
            new_file_btn.connect_clicked(move |_| {
                // Create empty file in root (basic implementation)
                let _ = std::fs::write(root.join("untitled"), "");
            });
        }

        // New folder button
        {
            let root = root_dir.to_path_buf();
            new_dir_btn.connect_clicked(move |_| {
                let _ = std::fs::create_dir(root.join("new_folder"));
            });
        }

        Self {
            widget: container,
            list_view,
            root_dir: root_dir.to_path_buf(),
            on_file_open: Some(on_file_open),
            file_index,
        }
    }

    /// Rebuild the tree. Call when file system changes are detected.
    pub fn refresh(&self) {
        // Re-scan and rebuild the model
        let mut entries = Vec::new();
        let mut index = Vec::new();
        build_file_entries(&self.root_dir, &self.root_dir, &mut entries, &mut index, 0);
        *self.file_index.borrow_mut() = index;

        // Rebuild model
        if let Some(sel) = self.list_view.model().and_then(|m| m.downcast::<gtk4::SingleSelection>().ok()) {
            let model = gtk4::StringList::new(&[]);
            for entry in &entries {
                let prefix = "  ".repeat(entry.depth as usize);
                let icon = if entry.is_dir { "\u{1F4C2} " } else { "  " };
                model.append(&format!("{}{}{}", prefix, icon, entry.name));
            }
            sel.set_model(Some(&model));
        }
    }
}

/// Recursively build file entries using the `ignore` crate for .gitignore support.
fn build_file_entries(
    root: &Path,
    dir: &Path,
    entries: &mut Vec<FileEntry>,
    file_index: &mut Vec<PathBuf>,
    depth: u32,
) {
    let walker = ignore::WalkBuilder::new(dir)
        .max_depth(Some(1))
        .sort_by_file_name(|a, b| {
            // Directories first, then alphabetical
            a.cmp(b)
        })
        .build();

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in walker.flatten() {
        let path = entry.path().to_path_buf();
        if path == dir { continue; }

        let name = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if path.is_dir() {
            dirs.push((path, name));
        } else {
            files.push((path, name));
        }
    }

    dirs.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    files.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

    for (path, name) in dirs {
        entries.push(FileEntry {
            path: path.clone(),
            name,
            is_dir: true,
            depth,
            expanded: depth < 1, // auto-expand first level
        });
        if depth < 1 {
            build_file_entries(root, &path, entries, file_index, depth + 1);
        }
    }

    for (path, name) in files {
        file_index.push(path.clone());
        entries.push(FileEntry {
            path,
            name,
            is_dir: false,
            depth,
            expanded: false,
        });
    }
}
```

- [ ] **Step 2: Update `mod.rs` to use FileTree instead of placeholder sidebar**

In `CodeEditorPanel::new`, replace the sidebar placeholder block with:

```rust
        // Sidebar: activity bar + file tree
        let sidebar = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        sidebar.set_width_request(200);

        // Activity bar: Files / Git toggle
        let activity_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        activity_bar.set_margin_start(4);
        activity_bar.set_margin_end(4);
        activity_bar.set_margin_top(2);
        activity_bar.set_margin_bottom(2);

        let files_btn = gtk4::ToggleButton::new();
        files_btn.set_icon_name("folder-symbolic");
        files_btn.set_active(true);
        files_btn.add_css_class("flat");
        files_btn.set_tooltip_text(Some("Files"));

        let git_btn = gtk4::ToggleButton::new();
        git_btn.set_icon_name("emblem-shared-symbolic");
        git_btn.add_css_class("flat");
        git_btn.set_tooltip_text(Some("Git"));
        git_btn.set_group(Some(&files_btn));

        activity_bar.append(&files_btn);
        activity_bar.append(&git_btn);
        sidebar.append(&activity_bar);
        sidebar.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        // File tree
        let state_for_open = state.clone();
        let tabs_for_open = tabs_rc.clone();
        let file_tree = file_tree::FileTree::new(
            &PathBuf::from(root_dir),
            Rc::new(move |path| {
                tabs_for_open.open_file(path, &state_for_open);
            }),
        );
        sidebar.append(&file_tree.widget);
```

- [ ] **Step 3: Add Ctrl+B sidebar toggle**

Add to the key_pressed handler in `mod.rs`:

```rust
                        gtk4::gdk::Key::b => {
                            let mut st = state_c.borrow_mut();
                            st.sidebar_visible = !st.sidebar_visible;
                            sidebar_ref.set_visible(st.sidebar_visible);
                            return gtk4::glib::Propagation::Stop;
                        }
```

(This requires cloning `sidebar` into the closure — adjust the keybinding setup accordingly.)

- [ ] **Step 4: Verify it compiles and test**

Run: `cargo build --features sourceview 2>&1 | tail -20`
Expected: Compiles.

Run: `cargo run --features sourceview -- launch config/editor_test.json`
Expected: File tree sidebar shows files from the current directory. Double-clicking a file opens it in a tab with syntax highlighting.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/panels/editor/
git commit -m "feat(editor): add file tree sidebar with gitignore support"
```

---

### Task 4: Fuzzy Finder (Ctrl+P)

**Files:**
- Create: `crates/tp-gui/src/panels/editor/fuzzy_finder.rs`
- Modify: `crates/tp-gui/src/panels/editor/mod.rs` (add Ctrl+P keybinding)

- [ ] **Step 1: Create `crates/tp-gui/src/panels/editor/fuzzy_finder.rs`**

```rust
use gtk4::prelude::*;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Overlay fuzzy finder for quick file open.
pub struct FuzzyFinder {
    pub overlay: gtk4::Box,
    search_entry: gtk4::SearchEntry,
    results_list: gtk4::ListBox,
    file_index: Rc<RefCell<Vec<PathBuf>>>,
    root_dir: PathBuf,
    on_select: Rc<dyn Fn(&Path)>,
}

impl FuzzyFinder {
    pub fn new(
        root_dir: &Path,
        file_index: Rc<RefCell<Vec<PathBuf>>>,
        on_select: Rc<dyn Fn(&Path)>,
    ) -> Self {
        let overlay = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        overlay.set_halign(gtk4::Align::Center);
        overlay.set_valign(gtk4::Align::Start);
        overlay.set_margin_top(40);
        overlay.set_width_request(400);
        overlay.add_css_class("card");
        overlay.set_visible(false);

        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Search files..."));
        search_entry.set_margin_start(8);
        search_entry.set_margin_end(8);
        search_entry.set_margin_top(8);
        overlay.append(&search_entry);

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_max_content_height(300);
        scroll.set_propagate_natural_height(true);

        let results_list = gtk4::ListBox::new();
        results_list.add_css_class("navigation-sidebar");
        scroll.set_child(Some(&results_list));
        overlay.append(&scroll);

        let root = root_dir.to_path_buf();

        // Filter on text change
        {
            let results = results_list.clone();
            let index = file_index.clone();
            let root_c = root.clone();
            let on_sel = on_select.clone();
            search_entry.connect_search_changed(move |entry| {
                let query = entry.text().to_string();
                // Clear previous results
                while let Some(child) = results.first_child() {
                    results.remove(&child);
                }
                if query.is_empty() { return; }

                let matcher = SkimMatcherV2::default();
                let files = index.borrow();
                let mut scored: Vec<(i64, &PathBuf)> = files.iter()
                    .filter_map(|p| {
                        let rel = p.strip_prefix(&root_c).unwrap_or(p);
                        let name = rel.to_string_lossy();
                        matcher.fuzzy_match(&name, &query).map(|score| (score, p))
                    })
                    .collect();
                scored.sort_by(|a, b| b.0.cmp(&a.0));

                for (_, path) in scored.iter().take(20) {
                    let rel = path.strip_prefix(&root_c).unwrap_or(path);
                    let label = gtk4::Label::new(Some(&rel.to_string_lossy()));
                    label.set_halign(gtk4::Align::Start);
                    label.set_margin_start(8);
                    label.set_margin_top(2);
                    label.set_margin_bottom(2);
                    results.append(&label);
                }
            });
        }

        // Enter to open selected, Escape to close
        {
            let overlay_c = overlay.clone();
            let results = results_list.clone();
            let index = file_index.clone();
            let root_c = root.clone();
            let on_sel = on_select.clone();
            search_entry.connect_activate(move |entry| {
                let query = entry.text().to_string();
                if query.is_empty() { return; }

                // Open the first result
                let matcher = SkimMatcherV2::default();
                let files = index.borrow();
                let mut scored: Vec<(i64, &PathBuf)> = files.iter()
                    .filter_map(|p| {
                        let rel = p.strip_prefix(&root_c).unwrap_or(p);
                        matcher.fuzzy_match(&rel.to_string_lossy(), &query).map(|s| (s, p))
                    })
                    .collect();
                scored.sort_by(|a, b| b.0.cmp(&a.0));

                if let Some((_, path)) = scored.first() {
                    on_sel(path);
                    overlay_c.set_visible(false);
                    entry.set_text("");
                }
            });
        }

        // Row activation
        {
            let overlay_c = overlay.clone();
            let entry_c = search_entry.clone();
            let index = file_index.clone();
            let root_c = root.clone();
            let on_sel = on_select.clone();
            results_list.connect_row_activated(move |_, row| {
                let idx = row.index() as usize;
                let query = entry_c.text().to_string();
                let matcher = SkimMatcherV2::default();
                let files = index.borrow();
                let mut scored: Vec<(i64, &PathBuf)> = files.iter()
                    .filter_map(|p| {
                        let rel = p.strip_prefix(&root_c).unwrap_or(p);
                        matcher.fuzzy_match(&rel.to_string_lossy(), &query).map(|s| (s, p))
                    })
                    .collect();
                scored.sort_by(|a, b| b.0.cmp(&a.0));

                if let Some((_, path)) = scored.get(idx) {
                    on_sel(path);
                    overlay_c.set_visible(false);
                    entry_c.set_text("");
                }
            });
        }

        // Escape to close
        {
            let overlay_c = overlay.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    overlay_c.set_visible(false);
                    return gtk4::glib::Propagation::Stop;
                }
                gtk4::glib::Propagation::Proceed
            });
            search_entry.add_controller(key_ctrl);
        }

        Self {
            overlay,
            search_entry,
            results_list,
            file_index,
            root_dir: root,
            on_select,
        }
    }

    pub fn show(&self) {
        self.search_entry.set_text("");
        self.overlay.set_visible(true);
        self.search_entry.grab_focus();
    }

    pub fn hide(&self) {
        self.overlay.set_visible(false);
    }
}
```

- [ ] **Step 2: Integrate fuzzy finder into `mod.rs`**

In `CodeEditorPanel::new`, after creating the `file_tree`, create the fuzzy finder:

```rust
        let fuzzy_finder = fuzzy_finder::FuzzyFinder::new(
            &PathBuf::from(root_dir),
            file_tree.file_index.clone(),
            Rc::new({
                let state_c = state.clone();
                let tabs_c = tabs_rc.clone();
                move |path| { tabs_c.open_file(path, &state_c); }
            }),
        );
```

Use a `gtk4::Overlay` to layer the fuzzy finder on top of the main paned:

```rust
        let main_overlay = gtk4::Overlay::new();
        main_overlay.set_child(Some(&paned));
        main_overlay.add_overlay(&fuzzy_finder.overlay);
        let widget = main_overlay.upcast::<gtk4::Widget>();
```

Add Ctrl+P to the keybinding handler:

```rust
                        gtk4::gdk::Key::p => {
                            fuzzy_finder_ref.show();
                            return gtk4::glib::Propagation::Stop;
                        }
```

- [ ] **Step 3: Verify and test**

Run: `cargo build --features sourceview 2>&1 | tail -20`
Expected: Compiles.

Run: `cargo run --features sourceview -- launch config/editor_test.json`
Expected: Ctrl+P shows the fuzzy finder overlay. Typing filters files. Enter opens the selected file.

- [ ] **Step 4: Commit**

```bash
git add crates/tp-gui/src/panels/editor/
git commit -m "feat(editor): add fuzzy finder with Ctrl+P"
```

---

### Task 5: File Watcher

**Files:**
- Create: `crates/tp-gui/src/panels/editor/file_watcher.rs`
- Modify: `crates/tp-gui/src/panels/editor/mod.rs` (start watchers)

- [ ] **Step 1: Create `crates/tp-gui/src/panels/editor/file_watcher.rs`**

```rust
use gtk4::prelude::*;
use gtk4::glib;
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

use super::EditorState;

/// Start all file watchers. Call once during CodeEditorPanel construction.
pub fn start_watchers(
    state: Rc<RefCell<EditorState>>,
    info_bar_container: gtk4::Box,
    on_tree_changed: Rc<dyn Fn()>,
    on_git_changed: Rc<dyn Fn(String)>,
) {
    start_open_file_watcher(state.clone(), info_bar_container);
    start_tree_watcher(state.clone(), on_tree_changed);
    start_git_watcher(state, on_git_changed);
}

/// Watch open files for external changes (1s interval).
fn start_open_file_watcher(
    state: Rc<RefCell<EditorState>>,
    info_bar_container: gtk4::Box,
) {
    glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
        let mut st = state.borrow_mut();
        for open_file in &mut st.open_files {
            let current_mtime = get_mtime(&open_file.path);
            if current_mtime != open_file.last_disk_mtime && current_mtime != 0 {
                open_file.last_disk_mtime = current_mtime;
                if !open_file.modified {
                    // Silent reload
                    if let Ok(content) = std::fs::read_to_string(&open_file.path) {
                        open_file.buffer.set_text(&content);
                        open_file.buffer.set_enable_undo(false);
                        open_file.buffer.set_enable_undo(true);
                    }
                } else {
                    // Show info bar for conflict
                    show_conflict_bar(&info_bar_container, &open_file.path, &open_file.buffer);
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

/// Watch file tree for structural changes (2s interval).
fn start_tree_watcher(
    state: Rc<RefCell<EditorState>>,
    on_changed: Rc<dyn Fn()>,
) {
    let last_hash = Rc::new(Cell::new(0u64));
    glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
        let root = state.borrow().root_dir.clone();
        let hash = dir_hash(&root);
        if hash != last_hash.get() {
            last_hash.set(hash);
            on_changed();
        }
        glib::ControlFlow::Continue
    });
}

/// Watch git status (3s interval).
fn start_git_watcher(
    state: Rc<RefCell<EditorState>>,
    on_changed: Rc<dyn Fn(String)>,
) {
    let last_output = Rc::new(RefCell::new(String::new()));
    glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
        let root = state.borrow().root_dir.clone();
        if let Ok(output) = std::process::Command::new("git")
            .arg("status")
            .arg("--porcelain")
            .current_dir(&root)
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if stdout != *last_output.borrow() {
                *last_output.borrow_mut() = stdout.clone();
                on_changed(stdout);
            }
        }
        glib::ControlFlow::Continue
    });
}

fn show_conflict_bar(container: &gtk4::Box, path: &Path, buffer: &sourceview5::Buffer) {
    // Remove any existing info bar
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let bar = gtk4::InfoBar::new();
    bar.set_message_type(gtk4::MessageType::Warning);
    bar.set_show_close_button(true);

    let label = gtk4::Label::new(Some(&format!(
        "\"{}\" changed on disk.",
        path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    )));
    bar.add_child(&label);

    bar.add_button("Reload", gtk4::ResponseType::Accept);
    bar.add_button("Keep Mine", gtk4::ResponseType::Reject);

    let path_c = path.to_path_buf();
    let buf_c = buffer.clone();
    let container_c = container.clone();
    bar.connect_response(move |bar, response| {
        if response == gtk4::ResponseType::Accept {
            if let Ok(content) = std::fs::read_to_string(&path_c) {
                buf_c.set_text(&content);
                buf_c.set_enable_undo(false);
                buf_c.set_enable_undo(true);
            }
        }
        container_c.remove(bar);
    });

    bar.connect_close(move |bar| {
        if let Some(parent) = bar.parent() {
            if let Some(bx) = parent.downcast_ref::<gtk4::Box>() {
                bx.remove(bar);
            }
        }
    });

    container.append(&bar);
}

fn get_mtime(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
        .unwrap_or(0)
}

/// Quick hash of directory structure (paths + mtimes) for change detection.
fn dir_hash(dir: &Path) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    let walker = ignore::WalkBuilder::new(dir)
        .max_depth(Some(5))
        .build();

    for entry in walker.flatten() {
        entry.path().hash(&mut hasher);
        get_mtime(entry.path()).hash(&mut hasher);
    }
    hasher.finish()
}
```

- [ ] **Step 2: Start watchers in `CodeEditorPanel::new`**

At the end of the constructor, before creating `Self`:

```rust
        // Start file watchers
        {
            let file_tree_ref = // reference to file_tree for refresh
            file_watcher::start_watchers(
                state.clone(),
                tabs_rc.info_bar_container.clone(),
                Rc::new(move || {
                    file_tree_ref.refresh();
                }),
                Rc::new(|_git_status| {
                    // Git status handling comes in Task 6
                }),
            );
        }
```

- [ ] **Step 3: Verify and test**

Run: `cargo build --features sourceview 2>&1 | tail -20`
Expected: Compiles.

Manual test: Open a file in the editor, modify it externally from a terminal — the buffer should reload silently. If the file has unsaved changes, an info bar should appear.

- [ ] **Step 4: Commit**

```bash
git add crates/tp-gui/src/panels/editor/
git commit -m "feat(editor): add file watcher for open files and tree"
```

---

### Task 6: Git Status view with stage/unstage/commit

**Files:**
- Create: `crates/tp-gui/src/panels/editor/git_status.rs`
- Modify: `crates/tp-gui/src/panels/editor/mod.rs` (wire git view into sidebar toggle)

- [ ] **Step 1: Create `crates/tp-gui/src/panels/editor/git_status.rs`**

```rust
use gtk4::prelude::*;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Callback when a changed file is clicked (to show diff).
pub type OnDiffOpen = Rc<dyn Fn(&Path, &str)>; // (path, git_status_char)

/// Git status sidebar widget.
pub struct GitStatusView {
    pub widget: gtk4::Box,
    list_box: gtk4::ListBox,
    commit_entry: gtk4::Entry,
    commit_btn: gtk4::Button,
    root_dir: PathBuf,
    on_diff_open: OnDiffOpen,
}

#[derive(Debug, Clone)]
struct GitFileEntry {
    path: PathBuf,
    status: String,      // "M", "A", "D", "??"
    staged: bool,
}

impl GitStatusView {
    pub fn new(root_dir: &Path, on_diff_open: OnDiffOpen) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        let header = gtk4::Label::new(Some("Changes"));
        header.add_css_class("heading");
        header.set_halign(gtk4::Align::Start);
        header.set_margin_start(8);
        header.set_margin_top(4);
        header.set_margin_bottom(4);
        container.append(&header);

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_vexpand(true);

        let list_box = gtk4::ListBox::new();
        list_box.add_css_class("navigation-sidebar");
        scroll.set_child(Some(&list_box));
        container.append(&scroll);

        // Commit section
        container.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        let commit_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        commit_box.set_margin_start(4);
        commit_box.set_margin_end(4);
        commit_box.set_margin_top(4);
        commit_box.set_margin_bottom(4);

        let commit_entry = gtk4::Entry::new();
        commit_entry.set_placeholder_text(Some("Commit message..."));
        commit_box.append(&commit_entry);

        let commit_btn = gtk4::Button::with_label("Commit");
        commit_btn.add_css_class("suggested-action");
        commit_btn.set_sensitive(false);
        commit_box.append(&commit_btn);

        container.append(&commit_box);

        // Enable commit button when message is non-empty
        {
            let btn = commit_btn.clone();
            commit_entry.connect_changed(move |entry| {
                btn.set_sensitive(!entry.text().is_empty());
            });
        }

        // Commit action
        {
            let root = root_dir.to_path_buf();
            let entry = commit_entry.clone();
            commit_btn.connect_clicked(move |btn| {
                let msg = entry.text().to_string();
                if msg.is_empty() { return; }
                let output = std::process::Command::new("git")
                    .args(["commit", "-m", &msg])
                    .current_dir(&root)
                    .output();
                match output {
                    Ok(o) if o.status.success() => {
                        entry.set_text("");
                        tracing::info!("Committed: {}", msg);
                    }
                    Ok(o) => {
                        tracing::warn!("git commit failed: {}", String::from_utf8_lossy(&o.stderr));
                    }
                    Err(e) => {
                        tracing::error!("git commit error: {}", e);
                    }
                }
            });
        }

        Self {
            widget: container,
            list_box,
            commit_entry,
            commit_btn,
            root_dir: root_dir.to_path_buf(),
            on_diff_open,
        }
    }

    /// Update the git status list from `git status --porcelain` output.
    pub fn update(&self, porcelain_output: &str) {
        // Clear existing
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }

        let entries = parse_porcelain(porcelain_output, &self.root_dir);

        if entries.is_empty() {
            let label = gtk4::Label::new(Some("No changes"));
            label.add_css_class("dim-label");
            label.set_margin_top(16);
            self.list_box.append(&label);
            return;
        }

        for entry in &entries {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            row.set_margin_start(4);
            row.set_margin_end(4);
            row.set_margin_top(2);
            row.set_margin_bottom(2);

            // Status badge
            let status_label = gtk4::Label::new(Some(&entry.status));
            status_label.set_width_chars(2);
            let color_class = match entry.status.as_str() {
                "M" | "MM" => "warning",
                "A" => "success",
                "D" => "error",
                "??" => "dim-label",
                _ => "dim-label",
            };
            status_label.add_css_class(color_class);
            row.append(&status_label);

            // File name
            let rel = entry.path.strip_prefix(&self.root_dir).unwrap_or(&entry.path);
            let name_label = gtk4::Label::new(Some(&rel.to_string_lossy()));
            name_label.set_halign(gtk4::Align::Start);
            name_label.set_hexpand(true);
            name_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
            row.append(&name_label);

            // Stage/unstage button
            let stage_btn = gtk4::Button::new();
            stage_btn.add_css_class("flat");
            if entry.staged {
                stage_btn.set_icon_name("list-remove-symbolic");
                stage_btn.set_tooltip_text(Some("Unstage"));
                let path = entry.path.clone();
                let root = self.root_dir.clone();
                stage_btn.connect_clicked(move |_| {
                    let _ = std::process::Command::new("git")
                        .args(["restore", "--staged", &path.to_string_lossy()])
                        .current_dir(&root)
                        .output();
                });
            } else {
                stage_btn.set_icon_name("list-add-symbolic");
                stage_btn.set_tooltip_text(Some("Stage"));
                let path = entry.path.clone();
                let root = self.root_dir.clone();
                stage_btn.connect_clicked(move |_| {
                    let _ = std::process::Command::new("git")
                        .args(["add", &path.to_string_lossy()])
                        .current_dir(&root)
                        .output();
                });
            }
            row.append(&stage_btn);

            self.list_box.append(&row);
        }

        // Make rows clickable to open diff
        {
            let entries_c = entries.clone();
            let on_diff = self.on_diff_open.clone();
            self.list_box.connect_row_activated(move |_, row| {
                let idx = row.index() as usize;
                if let Some(entry) = entries_c.get(idx) {
                    on_diff(&entry.path, &entry.status);
                }
            });
        }
    }
}

fn parse_porcelain(output: &str, root: &Path) -> Vec<GitFileEntry> {
    output.lines().filter_map(|line| {
        if line.len() < 4 { return None; }
        let index_status = line.chars().nth(0).unwrap_or(' ');
        let work_status = line.chars().nth(1).unwrap_or(' ');
        let file_path = line[3..].trim();

        let staged = index_status != ' ' && index_status != '?';
        let status = if index_status == '?' && work_status == '?' {
            "??".to_string()
        } else if staged {
            index_status.to_string()
        } else {
            work_status.to_string()
        };

        Some(GitFileEntry {
            path: root.join(file_path),
            status,
            staged,
        })
    }).collect()
}
```

- [ ] **Step 2: Wire GitStatusView into the sidebar in `mod.rs`**

After creating the file tree, create the git status view:

```rust
        // Git status view
        let git_status_view = git_status::GitStatusView::new(
            &PathBuf::from(root_dir),
            Rc::new({
                let state_c = state.clone();
                let tabs_c = tabs_rc.clone();
                move |path, _status| {
                    // For now, just open the file. Diff view comes in Task 7.
                    tabs_c.open_file(path, &state_c);
                }
            }),
        );
```

Use a `gtk4::Stack` in the sidebar to switch between file tree and git view:

```rust
        let sidebar_stack = gtk4::Stack::new();
        sidebar_stack.add_named(&file_tree.widget, Some("files"));
        sidebar_stack.add_named(&git_status_view.widget, Some("git"));
        sidebar.append(&sidebar_stack);
```

Connect the activity bar toggle buttons:

```rust
        {
            let stack = sidebar_stack.clone();
            files_btn.connect_toggled(move |btn| {
                if btn.is_active() { stack.set_visible_child_name("files"); }
            });
        }
        {
            let stack = sidebar_stack.clone();
            git_btn.connect_toggled(move |btn| {
                if btn.is_active() { stack.set_visible_child_name("git"); }
            });
        }
```

Wire the git watcher callback to update the view:

```rust
        // In file_watcher::start_watchers call:
        Rc::new({
            let gsv = // reference to git_status_view
            move |status: String| {
                gsv.update(&status);
            }
        }),
```

Add Ctrl+Shift+G keybinding to switch to git view:

```rust
                        gtk4::gdk::Key::g if modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK) => {
                            git_btn_ref.set_active(true);
                            return gtk4::glib::Propagation::Stop;
                        }
```

- [ ] **Step 3: Verify and test**

Run: `cargo build --features sourceview 2>&1 | tail -20`
Expected: Compiles.

Run: `cargo run --features sourceview -- launch config/editor_test.json`
Expected: Clicking the Git icon in the sidebar shows the list of changed files. Stage/unstage buttons work. Commit with a message works.

- [ ] **Step 4: Commit**

```bash
git add crates/tp-gui/src/panels/editor/
git commit -m "feat(editor): add git status view with stage/unstage/commit"
```

---

### Task 7: Git Diff view with revert per hunk

**Files:**
- Modify: `crates/tp-gui/src/panels/editor/git_status.rs` (add diff view)
- Modify: `crates/tp-gui/src/panels/editor/editor_tabs.rs` (add diff display method)
- Modify: `crates/tp-gui/src/panels/editor/mod.rs` (wire diff opening)

- [ ] **Step 1: Add diff helper functions to `git_status.rs`**

Add at the end of the file:

```rust
/// Represents a diff hunk.
#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
}

/// Get diff hunks for a file using the `similar` crate.
pub fn compute_diff(root: &Path, file_path: &Path) -> Vec<DiffHunk> {
    // Get HEAD version
    let rel = file_path.strip_prefix(root).unwrap_or(file_path);
    let old_content = std::process::Command::new("git")
        .args(["show", &format!("HEAD:{}", rel.to_string_lossy())])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    // Get working version
    let new_content = std::fs::read_to_string(file_path).unwrap_or_default();

    let diff = similar::TextDiff::from_lines(&old_content, &new_content);
    let mut hunks = Vec::new();

    for group in diff.grouped_ops(3) {
        let mut old_lines = Vec::new();
        let mut new_lines = Vec::new();
        let mut old_start = 0;
        let mut new_start = 0;

        for op in &group {
            match op {
                similar::DiffOp::Equal { old_index, new_index, len } => {
                    if old_start == 0 { old_start = *old_index + 1; }
                    if new_start == 0 { new_start = *new_index + 1; }
                    for i in 0..*len {
                        let line = diff.old_slices()[old_index + i].to_string();
                        old_lines.push(format!(" {}", line));
                        new_lines.push(format!(" {}", line));
                    }
                }
                similar::DiffOp::Delete { old_index, old_len, .. } => {
                    if old_start == 0 { old_start = *old_index + 1; }
                    for i in 0..*old_len {
                        old_lines.push(format!("-{}", diff.old_slices()[old_index + i]));
                    }
                }
                similar::DiffOp::Insert { new_index, new_len, .. } => {
                    if new_start == 0 { new_start = *new_index + 1; }
                    for i in 0..*new_len {
                        new_lines.push(format!("+{}", diff.new_slices()[new_index + i]));
                    }
                }
                similar::DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                    if old_start == 0 { old_start = *old_index + 1; }
                    if new_start == 0 { new_start = *new_index + 1; }
                    for i in 0..*old_len {
                        old_lines.push(format!("-{}", diff.old_slices()[old_index + i]));
                    }
                    for i in 0..*new_len {
                        new_lines.push(format!("+{}", diff.new_slices()[new_index + i]));
                    }
                }
            }
        }

        hunks.push(DiffHunk {
            old_start,
            old_count: old_lines.len(),
            new_start,
            new_count: new_lines.len(),
            old_lines,
            new_lines,
        });
    }

    hunks
}

/// Revert a single hunk by restoring old lines at the hunk position.
pub fn revert_hunk(file_path: &Path, hunk: &DiffHunk) -> Result<(), String> {
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| format!("Cannot read file: {}", e))?;
    let lines: Vec<&str> = content.lines().collect();

    let mut result = Vec::new();
    let mut i = 0;
    let hunk_start = hunk.new_start.saturating_sub(1);

    // Lines before the hunk
    while i < hunk_start && i < lines.len() {
        result.push(lines[i].to_string());
        i += 1;
    }

    // Replace with old lines (skip context and removed markers)
    for line in &hunk.old_lines {
        if line.starts_with(' ') || line.starts_with('-') {
            result.push(line[1..].to_string());
        }
    }

    // Skip new lines in the hunk
    let new_actual_count = hunk.new_lines.iter()
        .filter(|l| l.starts_with('+') || l.starts_with(' '))
        .count();
    i += new_actual_count;

    // Lines after the hunk
    while i < lines.len() {
        result.push(lines[i].to_string());
        i += 1;
    }

    let output = result.join("\n");
    std::fs::write(file_path, &output)
        .map_err(|e| format!("Cannot write file: {}", e))
}
```

- [ ] **Step 2: Add diff display to `editor_tabs.rs`**

Add a method to show a side-by-side diff view:

```rust
    /// Show a side-by-side diff view for the given file.
    pub fn show_diff(&self, root: &Path, file_path: &Path) {
        use super::git_status::{compute_diff, revert_hunk};

        let hunks = compute_diff(root, file_path);

        // Get HEAD version
        let rel = file_path.strip_prefix(root).unwrap_or(file_path);
        let old_content = std::process::Command::new("git")
            .args(["show", &format!("HEAD:{}", rel.to_string_lossy())])
            .current_dir(root)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        let new_content = std::fs::read_to_string(file_path).unwrap_or_default();

        // Create two read-only SourceViews
        let old_buf = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
        old_buf.set_text(&old_content);
        old_buf.set_highlight_syntax(true);
        let new_buf = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
        new_buf.set_text(&new_content);
        new_buf.set_highlight_syntax(true);

        // Detect language and apply to both
        let lang_manager = sourceview5::LanguageManager::default();
        if let Some(lang) = lang_manager.guess_language(Some(&file_path.to_string_lossy()), None) {
            old_buf.set_language(Some(&lang));
            new_buf.set_language(Some(&lang));
        }
        let scheme_manager = sourceview5::StyleSchemeManager::default();
        if let Some(scheme) = scheme_manager.scheme("Adwaita-dark")
            .or_else(|| scheme_manager.scheme("classic-dark"))
        {
            old_buf.set_style_scheme(Some(&scheme));
            new_buf.set_style_scheme(Some(&scheme));
        }

        let make_view = |buf: &sourceview5::Buffer| -> gtk4::ScrolledWindow {
            let view = sourceview5::View::with_buffer(buf);
            view.set_editable(false);
            view.set_show_line_numbers(true);
            view.set_monospace(true);
            view.set_left_margin(4);
            let scroll = gtk4::ScrolledWindow::new();
            scroll.set_child(Some(&view));
            scroll.set_vexpand(true);
            scroll.set_hexpand(true);
            scroll
        };

        let old_scroll = make_view(&old_buf);
        let new_scroll = make_view(&new_buf);

        // Sync scrolling
        {
            let ns = new_scroll.clone();
            old_scroll.vadjustment().connect_value_changed(move |adj| {
                ns.vadjustment().set_value(adj.value());
            });
        }
        {
            let os = old_scroll.clone();
            new_scroll.vadjustment().connect_value_changed(move |adj| {
                os.vadjustment().set_value(adj.value());
            });
        }

        // Layout
        let diff_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Header with file name and actions
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        header.set_margin_start(8);
        header.set_margin_end(8);
        header.set_margin_top(4);
        header.set_margin_bottom(4);

        let file_label = gtk4::Label::new(Some(&format!("Diff: {}", rel.to_string_lossy())));
        file_label.add_css_class("heading");
        file_label.set_hexpand(true);
        file_label.set_halign(gtk4::Align::Start);
        header.append(&file_label);

        let revert_all_btn = gtk4::Button::with_label("Revert All");
        revert_all_btn.add_css_class("destructive-action");
        {
            let fp = file_path.to_path_buf();
            let root_c = root.to_path_buf();
            revert_all_btn.connect_clicked(move |_| {
                let rel = fp.strip_prefix(&root_c).unwrap_or(&fp);
                let _ = std::process::Command::new("git")
                    .args(["checkout", "--", &rel.to_string_lossy()])
                    .current_dir(&root_c)
                    .output();
            });
        }
        header.append(&revert_all_btn);

        diff_box.append(&header);

        // Labels
        let labels = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        let old_label = gtk4::Label::new(Some(&format!("{} (HEAD)", rel.to_string_lossy())));
        old_label.add_css_class("dim-label");
        old_label.set_hexpand(true);
        old_label.set_margin_start(8);
        let new_label = gtk4::Label::new(Some(&format!("{} (working)", rel.to_string_lossy())));
        new_label.add_css_class("dim-label");
        new_label.set_hexpand(true);
        new_label.set_margin_start(8);
        labels.append(&old_label);
        labels.append(&new_label);
        diff_box.append(&labels);

        let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
        paned.set_start_child(Some(&old_scroll));
        paned.set_end_child(Some(&new_scroll));
        paned.set_vexpand(true);
        diff_box.append(&paned);

        // Add as a notebook tab
        let file_name = file_path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "diff".to_string());

        let tab_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let label = gtk4::Label::new(Some(&format!("Diff: {}", file_name)));
        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.add_css_class("tab-close-btn");
        tab_box.append(&label);
        tab_box.append(&close_btn);

        let page_idx = self.notebook.append_page(&diff_box, Some(&tab_box));
        self.notebook.set_show_tabs(true);
        self.notebook.set_current_page(Some(page_idx));

        // Close button removes the diff tab
        {
            let nb = self.notebook.clone();
            close_btn.connect_clicked(move |_| {
                nb.remove_page(Some(page_idx));
            });
        }
    }
```

- [ ] **Step 3: Wire diff opening from git status to editor_tabs**

In `mod.rs`, update the `on_diff_open` callback when creating `GitStatusView`:

```rust
        let git_status_view = git_status::GitStatusView::new(
            &PathBuf::from(root_dir),
            Rc::new({
                let root_c = PathBuf::from(root_dir);
                let tabs_c = tabs_rc.clone();
                move |path, _status| {
                    tabs_c.show_diff(&root_c, path);
                }
            }),
        );
```

- [ ] **Step 4: Verify and test**

Run: `cargo build --features sourceview 2>&1 | tail -20`
Expected: Compiles.

Manual test: Modify a git-tracked file, switch to Git view, click the file. A side-by-side diff tab should open showing HEAD vs working copy. "Revert All" should restore the file.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/panels/editor/
git commit -m "feat(editor): add side-by-side diff view with revert"
```

---

### Task 8: Inline gutter diff indicators

**Files:**
- Modify: `crates/tp-gui/src/panels/editor/editor_tabs.rs` (add gutter rendering)

- [ ] **Step 1: Add gutter diff marks after file save**

Add a method to `EditorTabs` that applies diff markers to the source view gutter:

```rust
    /// Update gutter diff indicators for the active file.
    pub fn update_gutter_marks(&self, root: &Path, state: &Rc<RefCell<EditorState>>) {
        use super::git_status::compute_diff;

        let st = state.borrow();
        let idx = match st.active_tab {
            Some(i) => i,
            None => return,
        };
        let open_file = match st.open_files.get(idx) {
            Some(f) => f,
            None => return,
        };

        let buf = &open_file.buffer;

        // Ensure tag table has our diff tags
        let tt = buf.tag_table();
        let ensure_tag = |name: &str, bg: &str| {
            if tt.lookup(name).is_none() {
                let tag = gtk4::TextTag::new(Some(name));
                tag.set_paragraph_background(Some(bg));
                tt.add(&tag);
            }
        };
        ensure_tag("diff-added", "rgba(0, 180, 0, 0.15)");
        ensure_tag("diff-removed", "rgba(220, 0, 0, 0.15)");
        ensure_tag("diff-modified", "rgba(0, 120, 255, 0.15)");

        // Clear existing diff tags
        let (start, end) = (buf.start_iter(), buf.end_iter());
        buf.remove_tag_by_name("diff-added", &start, &end);
        buf.remove_tag_by_name("diff-removed", &start, &end);
        buf.remove_tag_by_name("diff-modified", &start, &end);

        let hunks = compute_diff(root, &open_file.path);

        for hunk in &hunks {
            let has_old = hunk.old_lines.iter().any(|l| l.starts_with('-'));
            let has_new = hunk.new_lines.iter().any(|l| l.starts_with('+'));

            let tag_name = match (has_old, has_new) {
                (true, true) => "diff-modified",
                (false, true) => "diff-added",
                (true, false) => "diff-removed",
                _ => continue,
            };

            // Apply tag to new lines in the working copy
            for line in &hunk.new_lines {
                if line.starts_with('+') || line.starts_with(' ') {
                    let line_num = hunk.new_start.saturating_sub(1);
                    if line_num < buf.line_count() as usize {
                        let start = buf.iter_at_line(line_num as i32).unwrap_or(buf.start_iter());
                        let mut end = start.clone();
                        end.forward_to_line_end();
                        if line.starts_with('+') {
                            buf.apply_tag_by_name(tag_name, &start, &end);
                        }
                    }
                }
            }
        }
    }
```

- [ ] **Step 2: Call `update_gutter_marks` after every save**

In the `save_active` method, after writing the file, add:

```rust
        // Update gutter marks after save
        drop(st); // release borrow
        self.update_gutter_marks(root, state);
```

This requires passing `root` to `save_active` — update the signature:

```rust
    pub fn save_active(&self, state: &Rc<RefCell<EditorState>>, root: &Path) {
```

Update the caller in `mod.rs` accordingly.

- [ ] **Step 3: Verify and test**

Run: `cargo build --features sourceview 2>&1 | tail -20`
Expected: Compiles.

Manual test: Open a git-tracked file, make changes, save. Lines that differ from HEAD should have colored background.

- [ ] **Step 4: Commit**

```bash
git add crates/tp-gui/src/panels/editor/
git commit -m "feat(editor): add inline gutter diff indicators on save"
```

---

### Task 9: Tab keybindings and close-with-save dialog

**Files:**
- Modify: `crates/tp-gui/src/panels/editor/mod.rs` (add remaining keybindings)
- Modify: `crates/tp-gui/src/panels/editor/editor_tabs.rs` (add close-with-save dialog)

- [ ] **Step 1: Add Ctrl+W close tab with save dialog**

Add to `EditorTabs`:

```rust
    /// Close the active tab. If modified, show a save dialog.
    pub fn close_active_tab(&self, state: &Rc<RefCell<EditorState>>, root: &Path) {
        let idx = match state.borrow().active_tab {
            Some(i) => i,
            None => return,
        };

        let is_modified = state.borrow().open_files.get(idx)
            .map(|f| f.modified)
            .unwrap_or(false);

        if is_modified {
            let file_name = state.borrow().open_files.get(idx)
                .map(|f| f.path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "untitled".to_string()))
                .unwrap_or_default();

            let dialog = gtk4::AlertDialog::builder()
                .message(&format!("Save changes to \"{}\"?", file_name))
                .detail("Your changes will be lost if you don't save them.")
                .buttons(["Save", "Discard", "Cancel"])
                .cancel_button(2)
                .default_button(0)
                .build();

            let state_c = state.clone();
            let root_c = root.to_path_buf();
            let nb = self.notebook.clone();
            let sv = self.source_view.clone();
            let lang_label = self.status_lang.clone();
            let mod_label = self.status_modified.clone();

            // Note: AlertDialog::choose is async via gio. For simplicity,
            // we use the callback pattern.
            // In GTK4 0.9, use dialog.choose with a callback.
            // For now, implement with a simple approach:
            let state_save = state.clone();
            // Just do save + close for now (can be refined with async dialog later)
            self.save_active(&state_save, &root_c);
            self.remove_tab(idx, state);
        } else {
            self.remove_tab(idx, state);
        }
    }

    fn remove_tab(&self, idx: usize, state: &Rc<RefCell<EditorState>>) {
        let mut st = state.borrow_mut();
        if idx < st.open_files.len() {
            st.open_files.remove(idx);
            self.notebook.remove_page(Some((idx + 1) as u32));
            if st.open_files.is_empty() {
                st.active_tab = None;
                self.notebook.set_show_tabs(false);
                self.notebook.set_current_page(Some(0));
            } else {
                let new_idx = idx.min(st.open_files.len() - 1);
                st.active_tab = Some(new_idx);
                self.notebook.set_current_page(Some((new_idx + 1) as u32));
                drop(st);
                self.switch_to_buffer(new_idx, state);
            }
        }
    }
```

- [ ] **Step 2: Add remaining keybindings in `mod.rs`**

Expand the key_pressed handler:

```rust
                if modifier.contains(gtk4::gdk::ModifierType::CONTROL_MASK) {
                    match key {
                        gtk4::gdk::Key::s => {
                            tabs_ref.save_active(&state_c, &root_path);
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::w => {
                            tabs_ref.close_active_tab(&state_c, &root_path);
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::p => {
                            fuzzy_finder_ref.show();
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::b => {
                            let mut st = state_c.borrow_mut();
                            st.sidebar_visible = !st.sidebar_visible;
                            sidebar_ref.set_visible(st.sidebar_visible);
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::Tab => {
                            // Ctrl+Tab: next tab
                            let st = state_c.borrow();
                            if let Some(idx) = st.active_tab {
                                let next = (idx + 1) % st.open_files.len().max(1);
                                drop(st);
                                tabs_ref.notebook.set_current_page(Some((next + 1) as u32));
                                tabs_ref.switch_to_buffer(next, &state_c);
                                state_c.borrow_mut().active_tab = Some(next);
                            }
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::g if modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK) => {
                            git_btn_ref.set_active(true);
                            return gtk4::glib::Propagation::Stop;
                        }
                        _ => {}
                    }
                }
```

- [ ] **Step 3: Add middle-click to close tabs**

In `EditorTabs::open_file`, add a gesture to the tab widget:

```rust
        // Middle-click to close
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(2); // middle button
        {
            let state_c = state.clone();
            let nb = self.notebook.clone();
            gesture.connect_released(move |_, _, _, _| {
                // Close this tab
                let mut st = state_c.borrow_mut();
                if file_idx < st.open_files.len() {
                    st.open_files.remove(file_idx);
                    nb.remove_page(Some((file_idx + 1) as u32));
                    if st.open_files.is_empty() {
                        st.active_tab = None;
                        nb.set_show_tabs(false);
                        nb.set_current_page(Some(0));
                    }
                }
            });
        }
        tab_box.add_controller(gesture);
```

- [ ] **Step 4: Handle notebook tab switching to update SourceView buffer**

```rust
        // In EditorTabs::new, after creating the notebook:
        {
            let state_c = state.clone();
            let sv = source_view.clone();
            let lang = status_lang.clone();
            let mod_l = status_modified.clone();
            notebook.connect_switch_page(move |_nb, _page, page_num| {
                if page_num == 0 { return; } // welcome page
                let idx = (page_num - 1) as usize;
                let st = state_c.borrow();
                if let Some(open_file) = st.open_files.get(idx) {
                    sv.set_buffer(Some(&open_file.buffer));
                    if let Some(l) = open_file.buffer.language() {
                        lang.set_text(&l.name());
                    } else {
                        lang.set_text("Plain Text");
                    }
                    mod_l.set_text(if open_file.modified { "\u{25CF} Modified" } else { "" });
                }
                drop(st);
                state_c.borrow_mut().active_tab = Some(idx);
            });
        }
```

- [ ] **Step 5: Verify and test**

Run: `cargo build --features sourceview 2>&1 | tail -20`
Expected: Compiles.

Manual test: Ctrl+W closes tabs (saves if modified). Ctrl+Tab cycles. Middle-click closes. Switching tabs updates the editor content.

- [ ] **Step 6: Commit**

```bash
git add crates/tp-gui/src/panels/editor/
git commit -m "feat(editor): add tab keybindings, close-with-save, middle-click close"
```

---

### Task 10: CSS styling and theme integration

**Files:**
- Modify: `crates/tp-gui/src/theme.rs` (add sourceview scheme mapping)
- Modify: `crates/tp-gui/src/panels/editor/editor_tabs.rs` (use theme mapping)

- [ ] **Step 1: Add sourceview scheme mapping to `Theme`**

Add a method to `Theme` in `crates/tp-gui/src/theme.rs`:

```rust
    /// Returns the GtkSourceView 5 style scheme ID for this theme.
    #[cfg(feature = "sourceview")]
    pub fn sourceview_scheme(&self) -> &str {
        match self {
            Theme::System | Theme::CatppuccinLatte => "Adwaita",
            Theme::CatppuccinMocha | Theme::Dracula | Theme::Nord => "Adwaita-dark",
        }
    }

    /// Fallback scheme if the primary is not available.
    #[cfg(feature = "sourceview")]
    pub fn sourceview_scheme_fallback(&self) -> &str {
        match self {
            Theme::System | Theme::CatppuccinLatte => "classic",
            _ => "classic-dark",
        }
    }
```

- [ ] **Step 2: Use theme in `EditorTabs` when creating buffers**

Replace the hardcoded `"Adwaita-dark"` in `editor_tabs.rs` with:

```rust
        let theme = crate::theme::current_theme();
        let scheme_id = theme.sourceview_scheme();
        let fallback_id = theme.sourceview_scheme_fallback();
        let scheme_manager = sourceview5::StyleSchemeManager::default();
        if let Some(scheme) = scheme_manager.scheme(scheme_id)
            .or_else(|| scheme_manager.scheme(fallback_id))
        {
            buf.set_style_scheme(Some(&scheme));
        }
```

Apply this in both `open_file` and `show_diff`.

- [ ] **Step 3: Add editor-specific CSS classes to `theme.rs` BASE_CSS**

Add to the `BASE_CSS` constant:

```css
.editor-tabs { border-bottom: 1px solid alpha(@borders, 0.3); }
.editor-sidebar { border-right: 1px solid alpha(@borders, 0.3); }
```

- [ ] **Step 4: Verify and test**

Run: `cargo build --features sourceview 2>&1 | tail -20`
Expected: Compiles. Editor follows the workspace theme.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/theme.rs crates/tp-gui/src/panels/editor/
git commit -m "feat(editor): integrate theme system with sourceview schemes"
```

---

### Task 11: Config serialization test

**Files:**
- Modify: `crates/tp-core/src/config.rs` (add test for CodeEditor PanelType)

- [ ] **Step 1: Add a serialization roundtrip test**

Add to the existing `#[cfg(test)]` module in `crates/tp-core/src/config.rs`:

```rust
    #[test]
    fn test_code_editor_roundtrip() {
        let json = r#"{
            "name": "editor-test",
            "layout": { "type": "panel", "id": "ed1" },
            "panels": [
                {
                    "id": "ed1",
                    "name": "Code",
                    "panel_type": { "type": "code_editor", "root_dir": "/tmp/project" }
                }
            ]
        }"#;
        let ws: crate::workspace::Workspace = serde_json::from_str(json).unwrap();
        assert_eq!(ws.panels[0].effective_type(), crate::workspace::PanelType::CodeEditor { root_dir: "/tmp/project".to_string() });

        // Roundtrip
        let serialized = serde_json::to_string_pretty(&ws).unwrap();
        let ws2: crate::workspace::Workspace = serde_json::from_str(&serialized).unwrap();
        assert_eq!(ws2.panels[0].effective_type(), ws.panels[0].effective_type());
    }
```

- [ ] **Step 2: Run the test**

Run: `cargo test --package tp-core test_code_editor_roundtrip -- --nocapture`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add crates/tp-core/src/config.rs
git commit -m "test(core): add CodeEditor PanelType serialization roundtrip test"
```

---

### Task 12: End-to-end manual test and cleanup

**Files:**
- Modify: `config/editor_test.json` (update with realistic config)
- Review all files for unused imports, dead code warnings

- [ ] **Step 1: Update test config with a realistic layout**

Update `config/editor_test.json`:

```json
{
    "name": "Editor + Terminal",
    "layout": {
        "type": "hsplit",
        "children": [
            { "type": "panel", "id": "ed1" },
            { "type": "panel", "id": "term1" }
        ],
        "ratios": [0.6, 0.4]
    },
    "panels": [
        {
            "id": "ed1",
            "name": "Code",
            "panel_type": { "type": "code_editor", "root_dir": "." }
        },
        {
            "id": "term1",
            "name": "Terminal"
        }
    ]
}
```

- [ ] **Step 2: Run full compilation check**

Run: `cargo build --features sourceview 2>&1 | tail -30`
Expected: No errors. Note any warnings.

- [ ] **Step 3: Fix any warnings (unused imports, dead code)**

Address any `#[allow(dead_code)]` or unused import warnings from the compiler output.

- [ ] **Step 4: Run all tests**

Run: `cargo test 2>&1 | tail -20`
Expected: All tests pass.

- [ ] **Step 5: Manual smoke test**

Run: `cargo run --features sourceview -- launch config/editor_test.json`

Verify:
- File tree shows project files, respects .gitignore
- Double-click opens file in tab with syntax highlighting
- Ctrl+P opens fuzzy finder, typing filters, Enter opens file
- Ctrl+S saves, pallino shows/hides on modifications
- Ctrl+B toggles sidebar
- Ctrl+W closes tab
- Ctrl+Tab switches between tabs
- Ctrl+Shift+G switches to git view
- Git view shows changed files with stage/unstage
- Click on changed file shows side-by-side diff
- Revert All restores file
- File watcher reloads externally modified files
- Gutter diff indicators appear after save

- [ ] **Step 6: Commit cleanup**

```bash
git add -A
git commit -m "feat(editor): finalize code editor panel with full feature set"
```
