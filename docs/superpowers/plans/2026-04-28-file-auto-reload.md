# File auto-reload — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make all editor tabs (source, markdown, image) and the standalone Markdown panel auto-reload when the file changes on disk: silent reload when there are no local edits, InfoBar "Reload / Keep Mine" on conflict.

**Architecture:** Extend the existing polling watchers (`crates/tp-gui/src/panels/editor/file_watcher.rs` and the 500 ms timer in `crates/tp-gui/src/panels/markdown.rs`). No migration to `gio::FileMonitor`. Reuse the existing `gtk4::InfoBar` conflict prompt; add an analogous one to the standalone Markdown panel. Image tabs always reload silently (no local-edits concept).

**Tech Stack:** GTK4, sourceview5, glib timers, `gtk4::InfoBar`, existing `crate::markdown_render::render_markdown_to_view`.

**Spec:** `docs/superpowers/specs/2026-04-28-file-auto-reload-design.md`

**Note on tests:** Per project convention (no unit tests in commits unless explicitly asked), this plan does not add unit tests. Verification is manual.

---

## Task 1 — Refactor `show_conflict_bar` to accept a generic apply callback

The existing helper hard-codes "apply reload = update saved + buffer.set_text". To reuse it for markdown tabs (which also need to re-render the rendered view), make the apply step a closure supplied by the caller.

**Files:**
- Modify: `crates/tp-gui/src/panels/editor/file_watcher.rs`

- [ ] **Step 1: Change the signature and body of `show_conflict_bar`**

In `crates/tp-gui/src/panels/editor/file_watcher.rs:319-382`, replace the function with:

```rust
#[allow(deprecated)]
fn show_conflict_bar(
    container: &gtk4::Box,
    path: &Path,
    backend: Arc<dyn FileBackend>,
    apply_reload: Rc<dyn Fn(&str)>,
) {
    // Remove any existing info bar so we never stack two for the same tab.
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let bar = gtk4::InfoBar::new();
    bar.set_message_type(gtk4::MessageType::Warning);
    bar.set_show_close_button(true);

    let label = gtk4::Label::new(Some(&format!(
        "\"{}\" changed on disk.",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    )));
    bar.add_child(&label);

    bar.add_button("Reload", gtk4::ResponseType::Accept);
    bar.add_button("Keep Mine", gtk4::ResponseType::Reject);

    let path_c = path.to_path_buf();
    let container_c = container.clone();
    let backend_c = backend.clone();
    let apply_reload_c = apply_reload.clone();
    bar.connect_response(move |bar, response| {
        if response == gtk4::ResponseType::Accept {
            let path_reload = path_c.clone();
            let backend_reload = backend_c.clone();
            let apply = apply_reload_c.clone();
            run_blocking(
                move || backend_reload.read_file(&path_reload),
                move |result| {
                    if let Ok(content) = result {
                        apply(&content);
                    }
                },
            );
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
```

- [ ] **Step 2: Update the two existing call sites to build an `apply_reload` closure**

In the same file, lines ~136-143 and ~162-169 currently call `show_conflict_bar(&info_bar_container_c, &open_file.path, &buffer, saved_cell.clone(), backend_for_apply.clone())`. Replace each call with:

```rust
let apply_reload: Rc<dyn Fn(&str)> = {
    let buf = buffer.clone();
    let saved = saved_cell.clone();
    Rc::new(move |content: &str| {
        *saved.borrow_mut() = content.to_string();
        buf.set_text(content);
        buf.set_enable_undo(false);
        buf.set_enable_undo(true);
    })
};
show_conflict_bar(
    &info_bar_container_c,
    &open_file.path,
    backend_for_apply.clone(),
    apply_reload,
);
```

(The closure captures `buffer` and `saved_cell` by clone, matching the previous behavior.)

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: clean build (no new warnings beyond the pre-existing ones).

- [ ] **Step 4: Commit**

```bash
git add crates/tp-gui/src/panels/editor/file_watcher.rs
git commit -m "editor watcher: make conflict bar reload action a caller-supplied closure"
```

---

## Task 2 — Add `reload_from_disk` helpers for markdown and image tabs

Encapsulate the per-kind reload application in the modules that own the widgets.

**Files:**
- Modify: `crates/tp-gui/src/panels/editor/markdown_view.rs`
- Modify: `crates/tp-gui/src/panels/editor/image_view.rs`

- [ ] **Step 1: Add `reload_from_disk` to `markdown_view.rs`**

Append to the bottom of `crates/tp-gui/src/panels/editor/markdown_view.rs`, after `insert_at_cursor`:

```rust
/// Apply external file content to a markdown tab: update the saved snapshot,
/// replace the source buffer, clear undo, and re-render the rendered view if
/// currently in Rendered mode. The connect_changed handler wired in
/// editor_tabs.rs will see `current == saved` and clear the dirty flag.
pub fn reload_from_disk(tab: &MarkdownTab, content: &str) {
    *tab.saved_content.borrow_mut() = content.to_string();
    tab.buffer.set_text(content);
    tab.buffer.set_enable_undo(false);
    tab.buffer.set_enable_undo(true);
    if tab.mode.get() == MarkdownMode::Rendered {
        crate::markdown_render::render_markdown_to_view(&tab.rendered_view, content);
    }
}
```

- [ ] **Step 2: Add `reload_from_disk` to `image_view.rs`**

Append to the bottom of `crates/tp-gui/src/panels/editor/image_view.rs`, after `human_size`:

```rust
/// Re-point the tab's `Picture` at the (possibly changed) file on disk.
/// Image tabs have no local-edits concept, so this is always a silent reload.
/// `natural_width`/`natural_height` and zoom are intentionally left alone:
/// re-querying the paintable here would race the new pixbuf load. If this
/// becomes a problem we can re-fetch them on a follow-up tick.
pub fn reload_from_disk(tab: &ImageTab, path: &std::path::Path) {
    tab.picture.set_filename(Some(path));
}
```

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add crates/tp-gui/src/panels/editor/markdown_view.rs crates/tp-gui/src/panels/editor/image_view.rs
git commit -m "editor: add reload_from_disk for markdown and image tabs"
```

---

## Task 3 — Extend the editor watcher to cover markdown and image tabs

Today `start_open_file_watcher` (`crates/tp-gui/src/panels/editor/file_watcher.rs:61-176`) iterates only over tabs that have a `source_buffer()` and silently drops markdown and image tabs (lines 79-89, 114-120). Replace that with a per-`TabContent`-variant routing.

**Files:**
- Modify: `crates/tp-gui/src/panels/editor/file_watcher.rs`

- [ ] **Step 1: Extend `WatchedFileSnapshot` with the tab kind**

Replace the existing `WatchedFileSnapshot` struct (lines 12-17) with:

```rust
#[derive(Clone)]
enum WatchedKind {
    /// Source / Markdown tab — both have a writable buffer and saved content.
    Text { saved_content: String },
    /// Image tab — no content compare, mtime-only.
    Image,
}

#[derive(Clone)]
struct WatchedFileSnapshot {
    path: PathBuf,
    last_disk_mtime: u64,
    kind: WatchedKind,
}
```

- [ ] **Step 2: Replace the snapshot collection block**

Replace the `let snapshots: Vec<WatchedFileSnapshot> = { ... }` block (lines 74-89) with:

```rust
let snapshots: Vec<WatchedFileSnapshot> = {
    let st = state.borrow();
    st.open_files
        .iter()
        .map(|open_file| {
            use super::tab_content::TabContent;
            let kind = match &open_file.content {
                TabContent::Source(s) => WatchedKind::Text {
                    saved_content: s.saved_content.borrow().clone(),
                },
                TabContent::Markdown(m) => WatchedKind::Text {
                    saved_content: m.saved_content.borrow().clone(),
                },
                TabContent::Image(_) => WatchedKind::Image,
            };
            WatchedFileSnapshot {
                path: open_file.path.clone(),
                last_disk_mtime: open_file.last_disk_mtime,
                kind,
            }
        })
        .collect()
};
```

- [ ] **Step 3: Update `collect_open_file_changes` to skip content read for images**

Replace the function body (lines 282-317) with:

```rust
fn collect_open_file_changes(
    snapshots: &[WatchedFileSnapshot],
    backend: &dyn FileBackend,
    is_remote: bool,
) -> Vec<FileChange> {
    let mut changes = Vec::new();

    for snapshot in snapshots {
        match &snapshot.kind {
            WatchedKind::Text { saved_content } => {
                if is_remote {
                    if let Ok(content) = backend.read_file(&snapshot.path) {
                        if &content != saved_content {
                            changes.push(FileChange {
                                path: snapshot.path.clone(),
                                content,
                                last_disk_mtime: 0,
                            });
                        }
                    }
                    continue;
                }

                let current_mtime = get_mtime(&snapshot.path);
                if current_mtime == 0 || current_mtime == snapshot.last_disk_mtime {
                    continue;
                }
                if let Ok(content) = backend.read_file(&snapshot.path) {
                    changes.push(FileChange {
                        path: snapshot.path.clone(),
                        content,
                        last_disk_mtime: current_mtime,
                    });
                }
            }
            WatchedKind::Image => {
                // Image tabs are local-only (open_image_file refuses remote).
                if is_remote {
                    continue;
                }
                let current_mtime = get_mtime(&snapshot.path);
                if current_mtime == 0 || current_mtime == snapshot.last_disk_mtime {
                    continue;
                }
                changes.push(FileChange {
                    path: snapshot.path.clone(),
                    content: String::new(),
                    last_disk_mtime: current_mtime,
                });
            }
        }
    }

    changes
}
```

- [ ] **Step 4: Replace the apply loop to branch by tab kind**

Replace the body of the `move |changes|` callback (lines 103-171) with the per-variant routing below.

The trick: `open_file.content` is an immutable borrow during the variant match, but later steps mutate `open_file` (e.g. `last_disk_mtime`, `set_modified`). To avoid a borrow conflict, first clone the per-kind handles into an owned local enum, ending the borrow on `open_file.content`, then proceed.

The local + remote branches stay separate inside each text variant to preserve the "remote uses content compare, local uses mtime" distinction; image is local-only.

```rust
enum ApplyKind {
    Source {
        buffer: sourceview5::Buffer,
        saved: Rc<RefCell<String>>,
    },
    Markdown {
        tab: super::tab_content::MarkdownTab,
    },
    Image,
}

move |changes| {
    in_flight_c.set(false);

    let mut st = state_c.borrow_mut();
    let sync_suppress = st.sync_suppress.clone();
    for change in changes {
        let Some(open_file) = st.open_files.iter_mut().find(|f| f.path == change.path)
        else {
            continue;
        };

        use super::tab_content::TabContent;
        // Clone per-kind handles out so the borrow on open_file.content
        // ends before we mutate open_file fields below.
        let apply_kind = match &open_file.content {
            TabContent::Source(s) => ApplyKind::Source {
                buffer: s.buffer.clone(),
                saved: s.saved_content.clone(),
            },
            TabContent::Markdown(m) => ApplyKind::Markdown { tab: m.clone() },
            TabContent::Image(_) => ApplyKind::Image,
        };

        match apply_kind {
            ApplyKind::Source { buffer, saved } => {
                let disk_changed = if is_remote {
                    change.content != *saved.borrow()
                } else if change.last_disk_mtime == 0
                    || change.last_disk_mtime == open_file.last_disk_mtime
                {
                    false
                } else {
                    open_file.last_disk_mtime = change.last_disk_mtime;
                    true
                };
                if !disk_changed {
                    continue;
                }
                if !open_file.modified() {
                    *saved.borrow_mut() = change.content.clone();
                    sync_suppress.set(true);
                    buffer.set_text(&change.content);
                    sync_suppress.set(false);
                    buffer.set_enable_undo(false);
                    buffer.set_enable_undo(true);
                    open_file.set_modified(false);
                } else {
                    let apply_reload: Rc<dyn Fn(&str)> = {
                        let buf = buffer.clone();
                        let saved_c = saved.clone();
                        Rc::new(move |content: &str| {
                            *saved_c.borrow_mut() = content.to_string();
                            buf.set_text(content);
                            buf.set_enable_undo(false);
                            buf.set_enable_undo(true);
                        })
                    };
                    show_conflict_bar(
                        &info_bar_container_c,
                        &open_file.path,
                        backend_for_apply.clone(),
                        apply_reload,
                    );
                }
            }
            ApplyKind::Markdown { tab } => {
                let disk_changed = if is_remote {
                    change.content != *tab.saved_content.borrow()
                } else if change.last_disk_mtime == 0
                    || change.last_disk_mtime == open_file.last_disk_mtime
                {
                    false
                } else {
                    open_file.last_disk_mtime = change.last_disk_mtime;
                    true
                };
                if !disk_changed {
                    continue;
                }
                if !open_file.modified() {
                    super::markdown_view::reload_from_disk(&tab, &change.content);
                    open_file.set_modified(false);
                } else {
                    let md = tab.clone();
                    let apply_reload: Rc<dyn Fn(&str)> = Rc::new(move |content: &str| {
                        super::markdown_view::reload_from_disk(&md, content);
                    });
                    show_conflict_bar(
                        &info_bar_container_c,
                        &open_file.path,
                        backend_for_apply.clone(),
                        apply_reload,
                    );
                }
            }
            ApplyKind::Image => {
                if is_remote {
                    continue;
                }
                if change.last_disk_mtime == 0
                    || change.last_disk_mtime == open_file.last_disk_mtime
                {
                    continue;
                }
                open_file.last_disk_mtime = change.last_disk_mtime;
                // Re-borrow content to access the picture handle.
                if let TabContent::Image(img) = &open_file.content {
                    super::image_view::reload_from_disk(img, &open_file.path);
                }
            }
        }
    }
}
```

- [ ] **Step 5: Drop the now-unused `WatchedFileSnapshot::saved_content` field reference**

The compiler will flag any leftover references to the old `saved_content: String` field. Confirm by building.

Run: `cargo build`
Expected: clean build. The two unit tests at the bottom of `file_watcher.rs` need their snapshot construction updated since `WatchedFileSnapshot` shape changed.

In `collect_open_file_changes_detects_local_disk_updates`, replace the snapshot literal with:

```rust
WatchedFileSnapshot {
    path: path.clone(),
    last_disk_mtime: 0,
    kind: WatchedKind::Text {
        saved_content: "before\n".to_string(),
    },
}
```

In `collect_open_file_changes_detects_remote_content_differences`, replace it with:

```rust
WatchedFileSnapshot {
    path: path.clone(),
    last_disk_mtime: 0,
    kind: WatchedKind::Text {
        saved_content: "local\n".to_string(),
    },
}
```

Run: `cargo test --package pax-gui file_watcher 2>&1 | tail -20`
Expected: both tests pass.

- [ ] **Step 6: Manual verification (regression + new behaviors)**

Build and launch:

```bash
cargo build && cargo run -- new "watcher-test"
```

Then in the running app, open the Code Editor panel pointed at `/tmp` and:

1. Create `/tmp/x.txt` with content `before`. Open it as a source tab. From a shell: `echo after > /tmp/x.txt`. Within ~1 s the tab should show `after`. **No** info bar.
2. Type a local change in the same source tab without saving. From shell: `echo external > /tmp/x.txt`. The InfoBar "Reload / Keep Mine" should appear. Click "Reload" → buffer becomes `external`, dirty cleared. Click "Keep Mine" on a fresh change → bar disappears, buffer untouched.
3. Create `/tmp/x.md` with content `# title`. Open it as a markdown tab. From shell: `echo "# new" > /tmp/x.md`. Tab updates within ~1 s in both Rendered and Source modes. **No** info bar.
4. Markdown tab + dirty: type into the source view, then `echo external > /tmp/x.md`. InfoBar appears. "Reload" applies the disk content (rendered view re-renders if visible).
5. Open a PNG (e.g. any image you have); regenerate it from outside (`cp other.png /tmp/x.png`). The image tab should refresh within ~1 s. No prompt.

- [ ] **Step 7: Commit**

```bash
git add crates/tp-gui/src/panels/editor/file_watcher.rs
git commit -m "editor watcher: include markdown and image tabs in auto-reload

Markdown tabs now silently reload when clean and surface the same
Reload/Keep Mine InfoBar as source tabs when there are local edits.
Image tabs reload silently (no local-edits concept)."
```

---

## Task 4 — Add conflict InfoBar to the standalone Markdown panel

Today's 500 ms timer in `crates/tp-gui/src/panels/markdown.rs:649-677` reloads only in render mode and silently ignores external changes when in edit mode with dirty buffer. Add an InfoBar in the panel's header area and route the timer through it.

**Files:**
- Modify: `crates/tp-gui/src/panels/markdown.rs`

- [ ] **Step 1: Add the InfoBar widget to the panel container**

Find the `Self { ... }` construction near `crates/tp-gui/src/panels/markdown.rs:679-689` and the `let container = gtk4::Box::new(...)` near line 110. After the toolbar/fmt_bar are appended (around line 180-195) but before the stack is appended, insert an InfoBar container (mounted now so it's already in the layout when the timer fires):

After the line `container.append(&toolbar);` (line ~180), insert:

```rust
// Conflict bar slot: shown only when an external change collides with
// a dirty edit-mode buffer. Lives between the toolbar and the content
// stack so it stays out of the way until needed.
let conflict_bar_slot = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
container.append(&conflict_bar_slot);
```

- [ ] **Step 2: Replace the file-watch timer block with a conflict-aware version**

Replace the entire block at lines 649-677 (`// ── File watch (500ms, render mode only) ─` through the closing brace) with:

```rust
// ── File watch (500ms): silent reload when clean, conflict bar when dirty ─
{
    let fp = file_path.to_string();
    let ct = content.clone();
    let rv = render_view.clone();
    let m = mode.clone();
    let mod_flag = modified.clone();
    let sbuf = source_buffer.clone();
    let sb = save_btn.clone();
    let suppress = suppress_emit.clone();
    let nb_engine = notebook_engine.clone();
    let bar_slot = conflict_bar_slot.clone();
    let last_mtime = Rc::new(Cell::new(get_mtime(file_path)));
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        let mtime = get_mtime(&fp);
        if mtime == 0 || mtime == last_mtime.get() {
            return glib::ControlFlow::Continue;
        }
        last_mtime.set(mtime);
        let Ok(text) = std::fs::read_to_string(&fp) else {
            return glib::ControlFlow::Continue;
        };
        if text == *ct.borrow() {
            return glib::ControlFlow::Continue;
        }

        let dirty_in_edit = m.get() == Mode::Edit && mod_flag.get();
        if !dirty_in_edit {
            // Silent reload: update content + the appropriate view.
            *ct.borrow_mut() = text.clone();
            if m.get() == Mode::Render {
                *nb_engine.borrow_mut() = None;
                render_with_engine(&rv, &nb_engine, &text);
            } else {
                // Edit mode but clean: replace buffer text. The
                // connect_changed handler clears dirty when text == content.
                suppress.set(true);
                sbuf.set_text(&text);
                suppress.set(false);
                mod_flag.set(false);
                sb.set_sensitive(false);
            }
            return glib::ControlFlow::Continue;
        }

        // Conflict: surface the InfoBar.
        show_markdown_conflict_bar(
            &bar_slot,
            &fp,
            text.clone(),
            ct.clone(),
            sbuf.clone(),
            mod_flag.clone(),
            sb.clone(),
            suppress.clone(),
            rv.clone(),
            nb_engine.clone(),
            m.clone(),
        );
        glib::ControlFlow::Continue
    });
}
```

- [ ] **Step 3: Add the `show_markdown_conflict_bar` helper**

At the bottom of `crates/tp-gui/src/panels/markdown.rs`, after the existing helpers, add:

```rust
#[allow(clippy::too_many_arguments)]
#[allow(deprecated)]
fn show_markdown_conflict_bar(
    slot: &gtk4::Box,
    file_path: &str,
    new_content: String,
    content: Rc<RefCell<String>>,
    source_buffer: gtk4::TextBuffer,
    mod_flag: Rc<Cell<bool>>,
    save_btn: gtk4::Button,
    suppress: Rc<Cell<bool>>,
    render_view: gtk4::TextView,
    notebook_engine: Rc<RefCell<Option<Rc<NotebookEngine>>>>,
    mode: Rc<Cell<Mode>>,
) {
    // Replace any previous bar so we don't stack one per tick.
    while let Some(child) = slot.first_child() {
        slot.remove(&child);
    }

    let bar = gtk4::InfoBar::new();
    bar.set_message_type(gtk4::MessageType::Warning);
    bar.set_show_close_button(true);

    let name = std::path::Path::new(file_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let label = gtk4::Label::new(Some(&format!("\"{}\" changed on disk.", name)));
    bar.add_child(&label);
    bar.add_button("Reload", gtk4::ResponseType::Accept);
    bar.add_button("Keep Mine", gtk4::ResponseType::Reject);

    let slot_c = slot.clone();
    bar.connect_response(move |bar, response| {
        if response == gtk4::ResponseType::Accept {
            *content.borrow_mut() = new_content.clone();
            mod_flag.set(false);
            save_btn.set_sensitive(false);
            if mode.get() == Mode::Render {
                *notebook_engine.borrow_mut() = None;
                render_with_engine(&render_view, &notebook_engine, &new_content);
            } else {
                suppress.set(true);
                source_buffer.set_text(&new_content);
                suppress.set(false);
            }
        }
        // "Keep Mine" path falls through: just dismiss the bar.
        // Either way, last_mtime was already updated by the caller, so the
        // bar won't reappear until the file changes again.
        slot_c.remove(bar);
    });

    bar.connect_close(move |bar| {
        if let Some(parent) = bar.parent() {
            if let Some(bx) = parent.downcast_ref::<gtk4::Box>() {
                bx.remove(bar);
            }
        }
    });

    slot.append(&bar);
}
```

- [ ] **Step 4: Build**

Run: `cargo build`
Expected: clean build.

- [ ] **Step 5: Manual verification**

Run `cargo run -- new "md-conflict"`. In the workspace, replace one panel with a Markdown panel pointed at `/tmp/x.md` (use the file picker / recent list). Steps:

1. **Render mode regression**: Render mode, no edits. From shell `echo "# new" > /tmp/x.md`. The view re-renders within ~500 ms. No InfoBar. *(This is the existing behavior — must not regress.)*
2. **Edit mode clean**: Switch to Edit mode but make no local changes. From shell change the file. The buffer reloads silently within ~500 ms. No InfoBar.
3. **Edit mode dirty (conflict)**: Switch to Edit mode and type something. From shell change the file. InfoBar appears with "Reload / Keep Mine".
   - Click **Reload**: buffer is replaced with disk content; modified indicator clears.
   - Repeat the dirty scenario, click **Keep Mine**: bar dismisses; buffer unchanged. Change the file again from shell → bar reappears (last_mtime advanced to the dismissed value, only the *next* external change triggers a new bar).
4. **Internal save no-prompt**: With local edits, click the Save button in the toolbar. No InfoBar should appear (the Save path updates `content` and `last_mtime` is already handled by the timer's mtime check on the next tick — verify the bar does not pop up).

- [ ] **Step 6: Commit**

```bash
git add crates/tp-gui/src/panels/markdown.rs
git commit -m "markdown panel: show Reload/Keep Mine InfoBar on edit-mode conflicts

Until now the standalone Markdown panel silently ignored external
changes when the user was in Edit mode with unsaved edits, hiding the
fact that the file had moved underneath them. The 500ms watch timer now
reloads silently when clean (in either mode) and surfaces an InfoBar
with Reload/Keep Mine when there's a real conflict."
```

---

## Task 5 — End-to-end verification

- [ ] **Step 1: Full regression pass**

With a single `cargo run` instance, exercise all watch paths in one session. Use a workspace with: a Code Editor panel rooted at `/tmp` and a standalone Markdown panel pointed at `/tmp/note.md`.

| # | Setup                                              | Action                              | Expected                              |
|---|----------------------------------------------------|-------------------------------------|---------------------------------------|
| 1 | Source tab on `/tmp/a.rs`, no edits                | `echo new > /tmp/a.rs` from shell   | Tab updates within 1 s, no InfoBar    |
| 2 | Source tab on `/tmp/a.rs`, local edits unsaved     | Modify `/tmp/a.rs` from shell       | Editor InfoBar; both buttons work     |
| 3 | Markdown tab on `/tmp/b.md`, no edits, Rendered    | Modify `/tmp/b.md` from shell       | Render updates, no InfoBar            |
| 4 | Markdown tab on `/tmp/b.md`, no edits, Source      | Modify `/tmp/b.md` from shell       | Source buffer updates, no InfoBar     |
| 5 | Markdown tab on `/tmp/b.md`, dirty                 | Modify `/tmp/b.md` from shell       | Editor InfoBar; both buttons work     |
| 6 | Image tab on `/tmp/c.png`                          | `cp other.png /tmp/c.png`           | Picture refreshes, no InfoBar         |
| 7 | Standalone Markdown panel `/tmp/note.md`, Render   | Modify `/tmp/note.md` from shell    | Re-render, no InfoBar (regression)    |
| 8 | Standalone Markdown panel `/tmp/note.md`, Edit clean| Modify `/tmp/note.md` from shell   | Buffer updates, no InfoBar            |
| 9 | Standalone Markdown panel `/tmp/note.md`, Edit dirty| Modify `/tmp/note.md` from shell   | Panel InfoBar; both buttons work      |
| 10| Save from inside Pax (any panel)                   | Click Save button                   | No InfoBar appears                    |

- [ ] **Step 2: Final cleanup commit (only if needed)**

If any minor fixes surface during verification, commit them:

```bash
git add -p
git commit -m "fixup: ..."
```

---

## File map

| File                                                 | Touched in task |
|------------------------------------------------------|-----------------|
| `crates/tp-gui/src/panels/editor/file_watcher.rs`    | 1, 3            |
| `crates/tp-gui/src/panels/editor/markdown_view.rs`   | 2               |
| `crates/tp-gui/src/panels/editor/image_view.rs`      | 2               |
| `crates/tp-gui/src/panels/markdown.rs`               | 4               |
| `docs/superpowers/specs/2026-04-28-file-auto-reload-design.md` | (already committed) |
