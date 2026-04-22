# Code Editor Media Viewers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add image (`.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.bmp`, `.ico`, `.svg`) and Markdown (`.md`, `.markdown`) viewers to the Code Editor panel. Markdown opens rendered by default with a Rendered/Source toggle. Images display with a metadata header (dimensions · size · format) and zoom controls.

**Architecture:**
1. Extract the existing Markdown renderer from the standalone `MarkdownPanel` into a shared module `crate::markdown_render`, so both the panel type and the new editor viewer call one implementation.
2. Refactor the editor's tab model: today a single shared `sourceview5::View` swaps its buffer on every tab switch; afterwards each tab owns its own content widget inside the editor's existing `content_stack`, keyed by `tab_id`.
3. Dispatch on file extension in `open_file()` to create the right `TabContent` variant: `Source`, `Markdown`, or `Image`.

**Tech Stack:** Rust 2021 · gtk4-rs 0.9 (with `v4_10`) · libadwaita 0.7 · sourceview5 0.9 (behind `sourceview` feature) · `gtk::Picture` for image display (uses librsvg transitively for SVG).

**Project conventions:**
- Follow `CLAUDE.md`: package names are `pax-*`, directory names are `tp-*`.
- Commit after each task with a descriptive message. Do **not** add `Co-Authored-By` trailers.
- Do **not** add unit tests unless the user asks. Verification is manual (build + UI test).
- No magic numbers — every numeric constant gets a named `const`.
- Pax documentation is in Italian; code and commit messages are in English.

**Risk:** Task 3 is the largest change — it rewires search, match ruler, cursor-position label, and history shortcuts from the single shared `source_view` onto per-tab views. Expect heavy testing of source-file editing after that task.

---

## File Structure

New files:
- `crates/tp-gui/src/markdown_render.rs` — reusable `render_markdown_to_view` + `render_inline` + tag setup (moved out of `panels/markdown.rs`).
- `crates/tp-gui/src/panels/editor/tab_content.rs` — `TabContent` enum; `SourceTab`, `MarkdownTab`, `ImageTab` structs; helper accessors.
- `crates/tp-gui/src/panels/editor/markdown_view.rs` — `MarkdownTab` construction: Stack (rendered | source) + Rendered/Source toggle button.
- `crates/tp-gui/src/panels/editor/image_view.rs` — `ImageTab` construction: metadata header + `gtk::Picture` + zoom state and controls; `IMAGE_EXTS` constant list.

Modified files:
- `crates/tp-gui/src/lib.rs` — `pub mod markdown_render;`
- `crates/tp-gui/src/panels/markdown.rs` — delete inline `render_markdown_to_view` / `render_inline`; call shared module.
- `crates/tp-gui/src/panels/editor/mod.rs` — `mod tab_content;`, `mod markdown_view;`, `mod image_view;`; update `OpenFile` definition.
- `crates/tp-gui/src/panels/editor/editor_tabs.rs` — extension dispatch in `open_file`, drop single-shared `source_view`, per-tab widgets in `content_stack`, active-view accessor for search/history/ruler.

---

### Task 1: Extract shared markdown renderer

**Goal:** Move `render_markdown_to_view` and `render_inline` from `panels/markdown.rs` into `crate::markdown_render`. The standalone Markdown panel keeps rendering identically.

**Files:**
- Create: `crates/tp-gui/src/markdown_render.rs`
- Modify: `crates/tp-gui/src/lib.rs` (register module)
- Modify: `crates/tp-gui/src/panels/markdown.rs` (delete the two functions, call the shared versions)

- [ ] **Step 1: Create `crates/tp-gui/src/markdown_render.rs`**

Move the two functions verbatim. Do not change their bodies. Make both `pub(crate)` and add a short module comment.

```rust
//! Shared Markdown-to-TextBuffer renderer.
//!
//! Used by both the standalone Markdown panel (`panels::markdown`) and the
//! Code Editor's Markdown tab (`panels::editor::markdown_view`). A hand-rolled
//! parser — deliberately minimal — with GTK `TextTag`s doing the visual work.

use gtk4::prelude::*;

pub(crate) fn render_markdown_to_view(tv: &gtk4::TextView, content: &str) {
    let buf = tv.buffer();
    buf.set_text("");
    let tt = buf.tag_table();

    let ensure = |name: &str, f: &dyn Fn(&gtk4::TextTag)| {
        if tt.lookup(name).is_none() {
            let t = gtk4::TextTag::new(Some(name));
            f(&t);
            tt.add(&t);
        }
    };
    ensure("h1", &|t| {
        t.set_size_points(20.0);
        t.set_weight(700);
    });
    ensure("h2", &|t| {
        t.set_size_points(16.0);
        t.set_weight(700);
    });
    ensure("h3", &|t| {
        t.set_size_points(14.0);
        t.set_weight(700);
    });
    ensure("bold", &|t| {
        t.set_weight(700);
    });
    ensure("italic", &|t| {
        t.set_style(gtk4::pango::Style::Italic);
    });
    ensure("strike", &|t| {
        t.set_strikethrough(true);
    });
    ensure("code", &|t| {
        t.set_family(Some("monospace"));
    });
    ensure("code_block", &|t| {
        t.set_family(Some("monospace"));
        t.set_paragraph_background(Some("#2a2a2a"));
        t.set_left_margin(20);
    });
    ensure("link", &|t| {
        t.set_foreground(Some("#5588ff"));
        t.set_underline(gtk4::pango::Underline::Single);
    });
    ensure("bullet", &|t| {
        t.set_left_margin(20);
    });
    ensure("bq", &|t| {
        t.set_left_margin(20);
        t.set_style(gtk4::pango::Style::Italic);
        t.set_foreground(Some("#888888"));
    });
    ensure("sep", &|t| {
        t.set_foreground(Some("#666666"));
        t.set_size_points(6.0);
    });

    let mut it = buf.end_iter();
    let mut in_code = false;
    for line in content.lines() {
        if line.starts_with("```") {
            in_code = !in_code;
            let hint = line.trim_start_matches('`').trim();
            if in_code && !hint.is_empty() {
                buf.insert_with_tags_by_name(&mut it, &format!("─── {} ───\n", hint), &["sep"]);
            } else if !in_code {
                buf.insert_with_tags_by_name(&mut it, "───────\n", &["sep"]);
            }
            continue;
        }
        if in_code {
            buf.insert_with_tags_by_name(&mut it, &format!("{}\n", line), &["code_block"]);
            continue;
        }
        if line.starts_with("### ") {
            buf.insert_with_tags_by_name(&mut it, &format!("{}\n", &line[4..]), &["h3"]);
        } else if line.starts_with("## ") {
            buf.insert_with_tags_by_name(&mut it, &format!("{}\n", &line[3..]), &["h2"]);
        } else if line.starts_with("# ") {
            buf.insert_with_tags_by_name(&mut it, &format!("{}\n", &line[2..]), &["h1"]);
        } else if line.starts_with("---") || line.starts_with("***") {
            buf.insert_with_tags_by_name(&mut it, "────────────────────\n", &["sep"]);
        } else if line.starts_with("- ") || line.starts_with("* ") {
            buf.insert_with_tags_by_name(&mut it, &format!("  • {}\n", &line[2..]), &["bullet"]);
        } else if line.starts_with("> ") {
            buf.insert_with_tags_by_name(&mut it, &format!("│ {}\n", &line[2..]), &["bq"]);
        } else {
            render_inline(&buf, &mut it, line);
            buf.insert(&mut it, "\n");
        }
    }
}

fn render_inline(buf: &gtk4::TextBuffer, it: &mut gtk4::TextIter, text: &str) {
    let c: Vec<char> = text.chars().collect();
    let n = c.len();
    let mut i = 0;
    let mut p = String::new();
    while i < n {
        if i + 1 < n && c[i] == '*' && c[i + 1] == '*' {
            if !p.is_empty() {
                buf.insert(it, &p);
                p.clear();
            }
            i += 2;
            let s = i;
            while i + 1 < n && !(c[i] == '*' && c[i + 1] == '*') {
                i += 1;
            }
            buf.insert_with_tags_by_name(it, &c[s..i].iter().collect::<String>(), &["bold"]);
            if i + 1 < n {
                i += 2;
            }
        } else if c[i] == '*' {
            if !p.is_empty() {
                buf.insert(it, &p);
                p.clear();
            }
            i += 1;
            let s = i;
            while i < n && c[i] != '*' {
                i += 1;
            }
            buf.insert_with_tags_by_name(it, &c[s..i].iter().collect::<String>(), &["italic"]);
            if i < n {
                i += 1;
            }
        } else if c[i] == '`' {
            if !p.is_empty() {
                buf.insert(it, &p);
                p.clear();
            }
            i += 1;
            let s = i;
            while i < n && c[i] != '`' {
                i += 1;
            }
            buf.insert_with_tags_by_name(it, &c[s..i].iter().collect::<String>(), &["code"]);
            if i < n {
                i += 1;
            }
        } else if c[i] == '[' {
            if !p.is_empty() {
                buf.insert(it, &p);
                p.clear();
            }
            i += 1;
            let s = i;
            while i < n && c[i] != ']' {
                i += 1;
            }
            let lt: String = c[s..i].iter().collect();
            if i + 1 < n && c[i] == ']' && c[i + 1] == '(' {
                i += 2;
                while i < n && c[i] != ')' {
                    i += 1;
                }
                if i < n {
                    i += 1;
                }
            } else if i < n {
                i += 1;
            }
            buf.insert_with_tags_by_name(it, &lt, &["link"]);
        } else {
            p.push(c[i]);
            i += 1;
        }
    }
    if !p.is_empty() {
        buf.insert(it, &p);
    }
}
```

- [ ] **Step 2: Register the module in `crates/tp-gui/src/lib.rs`**

Add `pub mod markdown_render;` in alphabetical position between `layout_ops` and `panel_host`.

```rust
pub mod actions;
pub mod app;
pub mod backend_factory;
pub mod dialogs;
pub mod focus;
mod fonts;
mod icons;
pub mod layout_ops;
pub mod markdown_render;
pub mod panel_host;
pub mod panels;
pub mod shortcuts;
pub mod theme;
pub mod widget_builder;
pub mod widgets;
pub mod workspace_view;
```

- [ ] **Step 3: Delete the two functions from `crates/tp-gui/src/panels/markdown.rs`**

Delete the `fn render_markdown_to_view(...)` block starting at the `// ── Markdown rendering (render mode) ─────` header and the following `fn render_inline(...)` (roughly lines 554–729 in the current file). The `// ── Markdown rendering` header comment goes too.

- [ ] **Step 4: Replace call sites in `panels/markdown.rs`**

Every call to `render_markdown_to_view(&tv, &content)` in that file becomes `crate::markdown_render::render_markdown_to_view(&tv, &content)`. Use `Grep` to list them:

```
rg -n "render_markdown_to_view" crates/tp-gui/src/panels/markdown.rs
```

Replace each occurrence. There should be 1–3 call sites.

- [ ] **Step 5: Verify build**

Run: `cargo build`
Expected: compiles cleanly, no warnings introduced by the refactor.

- [ ] **Step 6: Manual verification — regression on Markdown panel**

Launch the app with a Markdown panel on a test file that exercises the renderer:

```bash
cargo run -- new md-test
```

Open any Markdown file in the Markdown panel (not the editor) — pick one that has headers, bold, italics, a code block, a bullet list, and a link. Compare against the previous behavior. It must render identically.

- [ ] **Step 7: Commit**

```bash
git add crates/tp-gui/src/markdown_render.rs crates/tp-gui/src/lib.rs crates/tp-gui/src/panels/markdown.rs
git commit -m "$(cat <<'EOF'
Extract markdown renderer into shared crate::markdown_render module

Prepares for reuse in the Code Editor markdown viewer; the standalone
Markdown panel delegates to the same function to avoid divergence.
EOF
)"
```

---

### Task 2: Introduce `TabContent` enum and `SourceTab` (state only — no widget refactor yet)

**Goal:** Introduce the type that will hold per-tab state, and migrate the editor's existing `OpenFile` fields into `TabContent::Source(SourceTab)`. Behavior stays identical — the single shared `source_view` still swaps buffers as today; the enum is just a new wrapper.

**Files:**
- Create: `crates/tp-gui/src/panels/editor/tab_content.rs`
- Modify: `crates/tp-gui/src/panels/editor/mod.rs` (add module declaration, replace `OpenFile` fields)
- Modify: `crates/tp-gui/src/panels/editor/editor_tabs.rs` (populate `TabContent::Source` on open; add accessor helpers)

- [ ] **Step 1: Create `crates/tp-gui/src/panels/editor/tab_content.rs`**

Put placeholder variants for Markdown and Image so the enum is exhaustive from the start. `MarkdownTab` and `ImageTab` bodies are empty structs for now — they get filled in later tasks.

```rust
//! Per-tab content in the Code Editor.
//!
//! Each open tab owns one `TabContent`. Source tabs hold a SourceView buffer,
//! Markdown tabs a rendered/source toggle, Image tabs a picture + zoom state.

use std::cell::RefCell;
use std::rc::Rc;

/// Data that's specific to a source-code tab.
#[derive(Debug, Clone)]
pub struct SourceTab {
    pub buffer: sourceview5::Buffer,
    pub modified: bool,
    /// Content on disk at last open/save — drives dirty detection.
    pub saved_content: Rc<RefCell<String>>,
}

/// Data that's specific to a Markdown tab. Populated in Task 4.
#[derive(Debug, Default)]
pub struct MarkdownTab {}

/// Data that's specific to an Image tab. Populated in Task 5.
#[derive(Debug, Default)]
pub struct ImageTab {}

#[derive(Debug)]
pub enum TabContent {
    Source(SourceTab),
    Markdown(MarkdownTab),
    Image(ImageTab),
}

impl TabContent {
    /// Accessor for the code-path that still assumes a source buffer exists
    /// (tab-switch handler, search context, cursor position label). Returns
    /// `None` for Markdown and Image tabs.
    pub fn source_buffer(&self) -> Option<&sourceview5::Buffer> {
        match self {
            TabContent::Source(s) => Some(&s.buffer),
            _ => None,
        }
    }

    pub fn is_modified(&self) -> bool {
        match self {
            TabContent::Source(s) => s.modified,
            _ => false,
        }
    }

    pub fn set_modified(&mut self, v: bool) {
        if let TabContent::Source(s) = self {
            s.modified = v;
        }
    }
}
```

- [ ] **Step 2: Update `OpenFile` in `crates/tp-gui/src/panels/editor/mod.rs`**

Replace the `buffer`, `modified`, `saved_content` fields with `content: TabContent`. Re-expose `buffer`, `modified`, `saved_content` via accessor methods that internally read `TabContent::Source`, so downstream call sites keep compiling with no further changes.

Add at the top of `mod.rs` (near the other `pub mod` declarations, under the `#[cfg(feature = "sourceview")]` group):

```rust
#[cfg(feature = "sourceview")]
pub mod tab_content;
```

Replace the `OpenFile` struct:

```rust
#[cfg(feature = "sourceview")]
#[derive(Debug)]
pub struct OpenFile {
    pub tab_id: u64,
    pub path: PathBuf,
    pub last_disk_mtime: u64,
    pub name_label: gtk4::Label,
    pub content: tab_content::TabContent,
}

#[cfg(feature = "sourceview")]
impl OpenFile {
    /// Convenience accessor for code that still assumes a source buffer.
    /// Returns `None` on non-source tabs — callers must handle gracefully.
    pub fn source_buffer(&self) -> Option<&sourceview5::Buffer> {
        self.content.source_buffer()
    }

    pub fn modified(&self) -> bool {
        self.content.is_modified()
    }

    pub fn set_modified(&mut self, v: bool) {
        self.content.set_modified(v);
    }

    pub fn saved_content(&self) -> Option<Rc<RefCell<String>>> {
        match &self.content {
            tab_content::TabContent::Source(s) => Some(s.saved_content.clone()),
            _ => None,
        }
    }
}
```

- [ ] **Step 3: Update call sites that access `open_file.buffer` / `.modified` / `.saved_content`**

Every direct field access becomes a method call. List them:

```
rg -n "\.buffer" crates/tp-gui/src/panels/editor/
rg -n "\.modified" crates/tp-gui/src/panels/editor/
rg -n "\.saved_content" crates/tp-gui/src/panels/editor/
```

Then for each hit:
- `f.buffer` / `open_file.buffer` (when `f` is an `&OpenFile`) → `f.source_buffer().expect("source tab")` **only where the caller has already established the tab is a source tab**. In places where the tab kind is unknown (e.g., `get_text_content`, `navigate_history`, `push_nav_position`, cursor-position label), replace with `if let Some(buf) = f.source_buffer() { ... }` and fall through on None.
- `f.modified` → `f.modified()`.
- `f.modified = x` → `f.set_modified(x)`.
- `f.saved_content` → `f.saved_content().expect(...)` or the Option variant as appropriate.

At this point only `TabContent::Source` tabs exist in practice, so `.expect(...)` is safe — but use the Option-aware form wherever the code is called from paths that will also handle Markdown/Image tabs later.

- [ ] **Step 4: Populate `TabContent::Source` on file open**

In `editor_tabs.rs`, inside `open_file()`, replace the block that pushes a new `OpenFile` with the new shape. Find around line 799:

```rust
st.open_files.push(super::OpenFile {
    tab_id,
    path: path.to_path_buf(),
    buffer: buf.clone(),
    modified: false,
    last_disk_mtime: mtime,
    saved_content: saved_content.clone(),
    name_label: label.clone(),
});
```

Change to:

```rust
use super::tab_content::{SourceTab, TabContent};
st.open_files.push(super::OpenFile {
    tab_id,
    path: path.to_path_buf(),
    last_disk_mtime: mtime,
    name_label: label.clone(),
    content: TabContent::Source(SourceTab {
        buffer: buf.clone(),
        modified: false,
        saved_content: saved_content.clone(),
    }),
});
```

Place the `use` at the top of the file with the other `use super::...` lines.

- [ ] **Step 5: Verify build**

Run: `cargo build`
Expected: compiles cleanly. Any remaining errors point at a call site missed in Step 3 — fix them.

- [ ] **Step 6: Manual verification — editor functionality unchanged**

```bash
cargo run -- new tab-content-refactor-test
```

- Open 2–3 source files (e.g. `src/**/*.rs` from the workspace). Switch between tabs. Status bar language + Ln/Col + dirty dot must still update correctly.
- Edit one file. Ctrl+S. Confirm saved (dirty dot disappears, mtime changes).
- Ctrl+F → search works.
- Ctrl+Z / Ctrl+Shift+Z → undo/redo work.
- Ctrl+W → close tab works.

- [ ] **Step 7: Commit**

```bash
git add crates/tp-gui/src/panels/editor/tab_content.rs crates/tp-gui/src/panels/editor/mod.rs crates/tp-gui/src/panels/editor/editor_tabs.rs
git commit -m "$(cat <<'EOF'
Introduce TabContent enum wrapping per-tab editor state

Preparatory refactor. OpenFile.buffer/modified/saved_content move into
TabContent::Source(SourceTab); Markdown and Image variants are stubbed
for now. Behavior unchanged — the shared SourceView still swaps buffers
on tab switch. Later tasks wire per-tab widgets into content_stack.
EOF
)"
```

---

### Task 3: Per-tab widgets in `content_stack`

**Goal:** Drop the single shared `source_view`. Each SourceTab owns its own `sourceview5::View` wrapped in a `ScrolledWindow`, added to `content_stack` under a name equal to `tab_id.to_string()`. Tab switch calls `content_stack.set_visible_child_name(...)` instead of `source_view.set_buffer(...)`.

**Files:**
- Modify: `crates/tp-gui/src/panels/editor/tab_content.rs` (SourceTab gains a `view` field)
- Modify: `crates/tp-gui/src/panels/editor/editor_tabs.rs` (major rewrite of `EditorTabs::new`, `open_file`, tab switching, search context, history shortcuts, cursor position label)
- Modify: `crates/tp-gui/src/panels/editor/mod.rs` (update `tabs_rc.source_view` readers — these become `tabs_rc.active_source_view()`)

This is the biggest task. Break into sub-commits only if a clean intermediate state is possible; otherwise commit at the end once everything builds and the editor works end-to-end.

- [ ] **Step 1: Extend `SourceTab` with its widget**

Edit `crates/tp-gui/src/panels/editor/tab_content.rs`:

```rust
#[derive(Debug, Clone)]
pub struct SourceTab {
    pub buffer: sourceview5::Buffer,
    pub view: sourceview5::View,
    /// Scrolled container that holds the view; what goes into the editor's
    /// content_stack.
    pub scroll: gtk4::ScrolledWindow,
    pub modified: bool,
    pub saved_content: Rc<RefCell<String>>,
}
```

Update the `source_buffer` accessor — unchanged. Add one more accessor used by the history-navigation code:

```rust
impl TabContent {
    pub fn source_view(&self) -> Option<&sourceview5::View> {
        match self {
            TabContent::Source(s) => Some(&s.view),
            _ => None,
        }
    }
}
```

- [ ] **Step 2: Drop the shared `source_view` from `EditorTabs`**

Delete `pub source_view: sourceview5::View` from the `EditorTabs` struct definition (around line 135 in the current `editor_tabs.rs`). Delete its construction in `EditorTabs::new` (lines 183–198 plus the scheme/theme registration). The `completion_words` and `keyword_shadow_buffer` fields stay — each new source view registers with them on creation.

Add a method to resolve the currently visible source view:

```rust
impl EditorTabs {
    /// The source view of the currently active tab, or None when the active
    /// tab is Markdown / Image / welcome.
    pub fn active_source_view(&self, state: &Rc<RefCell<EditorState>>) -> Option<sourceview5::View> {
        let st = state.borrow();
        let idx = st.active_tab?;
        st.open_files.get(idx)?.content.source_view().cloned()
    }
}
```

- [ ] **Step 3: Rewrite `open_file` to build a per-tab SourceView and register it with `content_stack`**

In `open_file`, replace the existing block that creates the `buf` and relies on the shared `source_view`. The new flow for a source tab:

1. Build `buf` as today.
2. Build a fresh `sourceview5::View`:
   ```rust
   let view = sourceview5::View::with_buffer(&buf);
   view.add_css_class("editor-code-view");
   view.set_show_line_numbers(true);
   view.set_highlight_current_line(true);
   view.set_auto_indent(true);
   view.set_tab_width(TAB_WIDTH);
   view.set_wrap_mode(gtk4::WrapMode::None);
   view.set_left_margin(EDITOR_LEFT_MARGIN);
   view.set_top_margin(EDITOR_TOP_MARGIN);
   view.set_monospace(true);
   view.set_show_right_margin(true);
   view.set_right_margin_position(RIGHT_MARGIN_POSITION);
   install_text_clipboard_shortcuts(&view);
   install_text_history_shortcuts(&view);
   ```
   Add these constants at the top of `editor_tabs.rs` (replace the inline magic numbers that were previously in `EditorTabs::new`):
   ```rust
   const TAB_WIDTH: u32 = 4;
   const EDITOR_LEFT_MARGIN: i32 = 6;
   const EDITOR_TOP_MARGIN: i32 = 3;
   const RIGHT_MARGIN_POSITION: u32 = 120;
   ```
3. Wrap in a ScrolledWindow:
   ```rust
   let scroll = gtk4::ScrolledWindow::new();
   scroll.set_child(Some(&view));
   scroll.set_vexpand(true);
   scroll.set_hexpand(true);
   ```
4. Attach to the content stack with a per-tab name:
   ```rust
   let child_name = format!("tab-{}", tab_id);
   self.content_stack.add_named(&scroll, Some(&child_name));
   self.content_stack.set_visible_child_name(&child_name);
   ```
5. Register the view's buffer with the completion provider (as today, but now per-tab):
   ```rust
   self.completion_words.register(&buf);
   ```
6. Wire the cursor-position listener to this view's buffer (instead of the shared one). Listener updates the shared status label:
   ```rust
   let pos_label = self.status_pos.clone();
   buf.connect_notify_local(Some("cursor-position"), move |buf, _| {
       let iter = buf.iter_at_offset(buf.cursor_position());
       let line = iter.line() + 1;
       let col = iter.line_offset() + 1;
       pos_label.set_text(&format!("Ln {}, Col {}", line, col));
   });
   ```
7. Store `view`, `scroll` alongside `buffer` / `modified` / `saved_content` in the `SourceTab`.

- [ ] **Step 4: Rewrite tab-switch handler to use the stack, not buffer swap**

In `EditorTabs::new` (around line 320–358), replace the body of `notebook.connect_switch_page`:

```rust
let state_c = state.clone();
let stack = content_stack.clone();
let lang_l = status_lang.clone();
let mod_l = status_modified.clone();
let pos_l = status_pos.clone();
let ml = match_lines.clone();
let mr = match_ruler.clone();
let lsq = last_search_query.clone();
notebook.connect_switch_page(move |_nb, _page, page_num| {
    let idx = page_num as usize;
    let Ok(mut st) = state_c.try_borrow_mut() else { return };
    let Some(open_file) = st.open_files.get(idx) else { return };
    let tab_id = open_file.tab_id;
    stack.set_visible_child_name(&format!("tab-{}", tab_id));

    // Status bar
    match &open_file.content {
        super::tab_content::TabContent::Source(s) => {
            if let Some(l) = s.buffer.language() {
                lang_l.set_text(&l.name());
            } else {
                lang_l.set_text("Plain Text");
            }
            mod_l.set_text(if s.modified { "\u{25CF} Modified" } else { "" });
            let iter = s.buffer.iter_at_offset(s.buffer.cursor_position());
            pos_l.set_text(&format!("Ln {}, Col {}", iter.line() + 1, iter.line_offset() + 1));
        }
        super::tab_content::TabContent::Markdown(_) => {
            lang_l.set_text("Markdown");
            mod_l.set_text("");
            pos_l.set_text("");
        }
        super::tab_content::TabContent::Image(_) => {
            lang_l.set_text("Image");
            mod_l.set_text("");
            pos_l.set_text("");
        }
    }

    // Overview ruler is only meaningful for source tabs.
    if let Some(buf) = open_file.content.source_buffer() {
        let query = lsq.borrow().clone();
        let lines = collect_match_lines(buf, &query);
        let has = !lines.is_empty();
        *ml.borrow_mut() = lines;
        mr.set_visible(has);
        mr.queue_draw();
    } else {
        ml.borrow_mut().clear();
        mr.set_visible(false);
    }

    st.active_tab = Some(idx);
    super::fire_nav_state_changed(&state_c);
});
```

- [ ] **Step 5: Rewire search: per-active-view SearchContext**

The `ensure_ctx` closure (line ~432) today reads `sv.buffer()` from the shared view. Change it to read from the currently active tab's buffer:

```rust
let state_for_search = state.clone();
let ss = search_settings.clone();
let ctx_cell = active_ctx.clone();
let ensure_ctx = move || -> Option<sourceview5::SearchContext> {
    let st = state_for_search.borrow();
    let idx = st.active_tab?;
    let buf = st.open_files.get(idx)?.content.source_buffer()?.clone();
    let mut cell = ctx_cell.borrow_mut();
    let needs_new = cell.as_ref().map(|c| c.buffer() != buf).unwrap_or(true);
    if needs_new {
        let ctx = sourceview5::SearchContext::new(&buf, Some(&ss));
        ctx.set_highlight(true);
        *cell = Some(ctx);
    }
    Some(cell.as_ref().unwrap().clone())
};
```

Every `get_ctx()` call site must now handle the `Option`. On `None` (active tab is Markdown/Image/welcome), the search bar does nothing. Replace `let ctx = get_ctx();` with `let Some(ctx) = get_ctx() else { return };` or equivalent.

- [ ] **Step 6: Rewire `show_search` / focus / scroll helpers**

Any method on `EditorTabs` that does `self.source_view.<something>()` must now resolve the active view:

```
rg -n "self\.source_view" crates/tp-gui/src/panels/editor/editor_tabs.rs
```

For each hit, replace with the `active_source_view(state)` accessor and handle `None`:

```rust
let Some(view) = self.active_source_view(state) else { return };
view.grab_focus(); // or whatever the original call was
```

For `scroll_to_iter`, the same pattern.

- [ ] **Step 7: Rewire the external callers in `mod.rs`**

`tabs_rc.source_view` is also read in `panels/editor/mod.rs` (line 444, 700–736, 965). Replace each with `tabs_rc.active_source_view(&state)` and handle the `None` gracefully (for image/md tabs the shortcuts that seed the search bar from the current selection have no meaning — just use an empty seed).

```
rg -n "\.source_view" crates/tp-gui/src/panels/editor/mod.rs
```

- [ ] **Step 8: Close-tab must remove the stack child**

In `close_tab_for_path` / `close_active_tab` / `close_tabs_under_dir` in `editor_tabs.rs`, after removing the notebook page and the `OpenFile` entry, remove the stack child:

```rust
let child_name = format!("tab-{}", tab_id);
if let Some(child) = self.content_stack.child_by_name(&child_name) {
    self.content_stack.remove(&child);
}
```

If that's the last tab, set the stack's visible child to `"welcome"` (the name used for the initial welcome screen).

- [ ] **Step 9: Update stale comment in `install_text_history_shortcuts`**

The function body (`editor_tabs.rs:97-128`) still works — looking up the buffer via `view.buffer()` at event time is correct for the new per-tab views too, since each view's buffer is stable. But the comment block at lines 98–99 that explains "the main editor SourceView swaps its buffer every tab switch, so we must not capture the initial one" is now stale. Delete those two comment lines. No other change.

- [ ] **Step 10: Verify build**

Run: `cargo build`
Expected: compiles. Any remaining errors point at `source_view` call sites missed earlier — fix them.

- [ ] **Step 11: Manual verification — full editor smoke test**

```bash
cargo run -- new per-tab-view-test
```

Must all work:
- Open 3+ source files, switch tabs. Each tab shows the right content; status bar updates (language, Ln/Col, dirty).
- Edit one file; dirty dot appears. Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z undo/redo (attached per-view).
- Ctrl+F search in current tab; next/prev; gold overview ruler shows matches for the current tab's buffer only. Switch tab — ruler updates.
- Ctrl+S save; dirty dot clears.
- Ctrl+W close tab; correct tab closes; remaining tabs still work.
- Close all tabs; welcome screen shows.
- Reopen files from Ctrl+E recent list; Ctrl+P fuzzy finder; Ctrl+Shift+F project search + click a result — cursor lands on the right line.
- Alt+← / Alt+→ navigate history — scrolls to the saved line in the right tab.
- Buffer-word completion popup still surfaces words from *all* open buffers (verify by opening two files that share a unique token, typing its prefix in one, and confirming the completion is offered).

Anything broken here is regression, not a new feature — fix before moving on.

- [ ] **Step 12: Commit**

```bash
git add crates/tp-gui/src/panels/editor/tab_content.rs crates/tp-gui/src/panels/editor/mod.rs crates/tp-gui/src/panels/editor/editor_tabs.rs
git commit -m "$(cat <<'EOF'
Editor: per-tab SourceView in content_stack (drop shared buffer swap)

Each source tab now owns its own sourceview5::View and ScrolledWindow,
registered in content_stack under tab-{id}. Tab switch changes
visible_child_name instead of set_buffer on a shared view — prerequisite
for heterogeneous Markdown/Image tabs. Search context, history
shortcuts, cursor position label, and match ruler now resolve the
active view dynamically.
EOF
)"
```

---

### Task 4: Markdown tab + `.md` dispatch

**Goal:** Build `MarkdownTab` and enable the extension dispatch so `.md`/`.markdown` files open in rendered view with a Rendered/Source toggle.

**Files:**
- Modify: `crates/tp-gui/src/panels/editor/tab_content.rs` (flesh out `MarkdownTab`)
- Create: `crates/tp-gui/src/panels/editor/markdown_view.rs`
- Modify: `crates/tp-gui/src/panels/editor/mod.rs` (`mod markdown_view;`)
- Modify: `crates/tp-gui/src/panels/editor/editor_tabs.rs` (extension dispatch in `open_file`)

- [ ] **Step 1: Flesh out `MarkdownTab`**

```rust
// tab_content.rs

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarkdownMode {
    Rendered,
    Source,
}

#[derive(Debug, Clone)]
pub struct MarkdownTab {
    pub buffer: sourceview5::Buffer,
    pub source_view: sourceview5::View,
    pub rendered_view: gtk4::TextView,
    /// The stack that switches between rendered and source children.
    pub inner_stack: gtk4::Stack,
    pub mode: Rc<std::cell::Cell<MarkdownMode>>,
    pub modified: bool,
    pub saved_content: Rc<RefCell<String>>,
    /// Outer widget that lives in the editor's content_stack (contains the
    /// Rendered/Source toggle bar above inner_stack).
    pub outer: gtk4::Widget,
}
```

Update `TabContent::is_modified` / `set_modified` / `saved_content`-adjacent accessors to handle `Markdown`:

```rust
pub fn is_modified(&self) -> bool {
    match self {
        TabContent::Source(s) => s.modified,
        TabContent::Markdown(m) => m.modified,
        TabContent::Image(_) => false,
    }
}
pub fn set_modified(&mut self, v: bool) {
    match self {
        TabContent::Source(s) => s.modified = v,
        TabContent::Markdown(m) => m.modified = v,
        TabContent::Image(_) => {}
    }
}
```

Add a helper returning the Markdown buffer (used by save / dirty tracking):

```rust
impl TabContent {
    pub fn writable_buffer(&self) -> Option<&sourceview5::Buffer> {
        match self {
            TabContent::Source(s) => Some(&s.buffer),
            TabContent::Markdown(m) => Some(&m.buffer),
            TabContent::Image(_) => None,
        }
    }
}
```

Audit `editor_tabs.rs` for `source_buffer()` calls on save / dirty paths and switch them to `writable_buffer()` — save works for Markdown too.

- [ ] **Step 2: Create `crates/tp-gui/src/panels/editor/markdown_view.rs`**

```rust
//! Markdown tab: Rendered / Source toggle in one tab.

use gtk4::prelude::*;
use sourceview5::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::tab_content::{MarkdownMode, MarkdownTab};

pub fn build_markdown_tab(content: &str) -> MarkdownTab {
    let mode = Rc::new(Cell::new(MarkdownMode::Rendered));
    let saved_content = Rc::new(RefCell::new(content.to_string()));

    // Source view (markdown language)
    let buffer = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
    buffer.set_text(content);
    if let Some(lang) = sourceview5::LanguageManager::default().language("markdown") {
        buffer.set_language(Some(&lang));
    }
    buffer.set_highlight_syntax(true);
    crate::theme::register_sourceview_buffer(&buffer);
    buffer.set_enable_undo(false);
    buffer.set_enable_undo(true);

    let source_view = sourceview5::View::with_buffer(&buffer);
    source_view.add_css_class("editor-code-view");
    source_view.set_show_line_numbers(true);
    source_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    source_view.set_monospace(true);
    let source_scroll = gtk4::ScrolledWindow::new();
    source_scroll.set_child(Some(&source_view));
    source_scroll.set_vexpand(true);
    source_scroll.set_hexpand(true);

    // Rendered view (read-only TextView)
    let rendered_view = gtk4::TextView::new();
    rendered_view.set_editable(false);
    rendered_view.set_cursor_visible(false);
    rendered_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    rendered_view.set_left_margin(RENDERED_MARGIN);
    rendered_view.set_right_margin(RENDERED_MARGIN);
    rendered_view.set_top_margin(RENDERED_MARGIN);
    rendered_view.set_bottom_margin(RENDERED_MARGIN);
    rendered_view.add_css_class("editor-markdown-rendered");
    crate::markdown_render::render_markdown_to_view(&rendered_view, content);
    let rendered_scroll = gtk4::ScrolledWindow::new();
    rendered_scroll.set_child(Some(&rendered_view));
    rendered_scroll.set_vexpand(true);
    rendered_scroll.set_hexpand(true);

    let inner_stack = gtk4::Stack::new();
    inner_stack.add_named(&rendered_scroll, Some("rendered"));
    inner_stack.add_named(&source_scroll, Some("source"));
    inner_stack.set_visible_child_name("rendered");

    // Toolbar with Rendered/Source toggle buttons
    let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    bar.add_css_class("editor-markdown-toolbar");
    bar.add_css_class("linked");
    bar.set_margin_start(TOOLBAR_MARGIN);
    bar.set_margin_end(TOOLBAR_MARGIN);
    bar.set_margin_top(TOOLBAR_MARGIN);
    bar.set_margin_bottom(TOOLBAR_MARGIN);

    let rendered_btn = gtk4::ToggleButton::with_label("Rendered");
    rendered_btn.set_active(true);
    let source_btn = gtk4::ToggleButton::with_label("Source");
    source_btn.set_group(Some(&rendered_btn));
    bar.append(&rendered_btn);
    bar.append(&source_btn);

    {
        let stack = inner_stack.clone();
        let rv = rendered_view.clone();
        let buf = buffer.clone();
        let mode_c = mode.clone();
        rendered_btn.connect_toggled(move |btn| {
            if !btn.is_active() { return; }
            // Re-render from current buffer content (dirty OK).
            let text = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
            crate::markdown_render::render_markdown_to_view(&rv, &text);
            stack.set_visible_child_name("rendered");
            mode_c.set(MarkdownMode::Rendered);
        });
    }
    {
        let stack = inner_stack.clone();
        let mode_c = mode.clone();
        source_btn.connect_toggled(move |btn| {
            if !btn.is_active() { return; }
            stack.set_visible_child_name("source");
            mode_c.set(MarkdownMode::Source);
        });
    }

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.append(&bar);
    outer.append(&inner_stack);

    MarkdownTab {
        buffer,
        source_view,
        rendered_view,
        inner_stack,
        mode,
        modified: false,
        saved_content,
        outer: outer.upcast::<gtk4::Widget>(),
    }
}

const RENDERED_MARGIN: i32 = 12;
const TOOLBAR_MARGIN: i32 = 4;
```

- [ ] **Step 3: Register module in `mod.rs`**

```rust
#[cfg(feature = "sourceview")]
mod markdown_view;
```

- [ ] **Step 4: Dispatch on extension in `open_file`**

At the start of `open_file()` (after the already-open check, before the shared file-read block), branch:

```rust
fn extension_kind(path: &Path) -> ExtensionKind {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return ExtensionKind::Source;
    };
    let lower = ext.to_lowercase();
    if MARKDOWN_EXTS.contains(&lower.as_str()) {
        ExtensionKind::Markdown
    } else if super::image_view::IMAGE_EXTS.contains(&lower.as_str()) {
        ExtensionKind::Image
    } else {
        ExtensionKind::Source
    }
}

enum ExtensionKind {
    Source,
    Markdown,
    Image,
}

const MARKDOWN_EXTS: &[&str] = &["md", "markdown"];
```

(`IMAGE_EXTS` lives in `image_view` — Task 5. For now, stub `super::image_view::IMAGE_EXTS` as `pub const IMAGE_EXTS: &[&str] = &[]` in a placeholder file — or defer the image branch until Task 5. Simpler: in Task 4, branch only on Markdown, fall through to Source for anything else. Add the Image branch in Task 5.)

For this task, branch on Markdown only:

```rust
match path.extension().and_then(|s| s.to_str()).map(str::to_ascii_lowercase).as_deref() {
    Some("md") | Some("markdown") => {
        self.open_markdown_file(path, state);
        return Some(/* idx computed below */);
    }
    _ => {}
}
```

- [ ] **Step 5: Implement `open_markdown_file`**

Add a private method on `EditorTabs`:

```rust
fn open_markdown_file(&self, path: &Path, state: &Rc<RefCell<EditorState>>) -> Option<usize> {
    let backend = state.borrow().backend.clone();
    let content = match backend.read_file(path) {
        Ok(c) => c,
        Err(e) => { tracing::warn!("Cannot open md {}: {}", path.display(), e); return None; }
    };
    let md = super::markdown_view::build_markdown_tab(&content);

    let tab_id = alloc_tab_id();
    let child_name = format!("tab-{}", tab_id);
    self.content_stack.add_named(&md.outer, Some(&child_name));
    self.content_stack.set_visible_child_name(&child_name);

    // Tab label (re-uses the same dirty-dot/close-button helper that source tabs use).
    let (tab_box, label, dot, close_btn) = build_tab_label(path);
    let page_widget = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page_widget.set_height_request(0);
    let _page = self.notebook.append_page(&page_widget, Some(&tab_box));
    self.notebook.set_show_tabs(true);

    let mtime = get_mtime(path);
    let idx = {
        let mut st = state.borrow_mut();
        st.open_files.push(super::OpenFile {
            tab_id,
            path: path.to_path_buf(),
            last_disk_mtime: mtime,
            name_label: label.clone(),
            content: super::tab_content::TabContent::Markdown(md.clone()),
        });
        st.active_tab = Some(st.open_files.len() - 1);
        st.open_files.len() - 1
    };

    // Dirty tracking: same pattern as source tabs — compare buffer to saved_content.
    {
        let state_c = state.clone();
        let dot_c = dot.clone();
        let mod_label = self.status_modified.clone();
        let saved = md.saved_content.clone();
        md.buffer.connect_changed(move |buf| {
            let current = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
            let is_dirty = current != *saved.borrow();
            dot_c.set_text(if is_dirty { "\u{25CF} " } else { "" });
            mod_label.set_text(if is_dirty { "\u{25CF} Modified" } else { "" });
            if let Ok(mut st) = state_c.try_borrow_mut() {
                if let Some(idx) = st.active_tab {
                    if let Some(f) = st.open_files.get_mut(idx) {
                        f.set_modified(is_dirty);
                    }
                }
            }
        });
    }

    // Close button wiring matches source tabs (find by tab_id, remove page + stack child).
    wire_close_button(&close_btn, tab_id, self, state);

    self.notebook.set_current_page(Some(idx as u32));
    Some(idx)
}
```

**Note:** `build_tab_label` and `wire_close_button` are helpers that almost certainly already exist inline in `open_file` today as duplicated code. If they don't, extract them in this task so both `open_markdown_file` and `open_file` can reuse them — otherwise copy-paste the tab-label and close-button logic from `open_file`. Prefer extracting.

- [ ] **Step 6: Save path handles Markdown tabs**

Find `save_active` in `editor_tabs.rs`:

```
rg -n "fn save_active" crates/tp-gui/src/panels/editor/editor_tabs.rs
```

Replace the call that gets the buffer to save with `content.writable_buffer()` so both Source and Markdown tabs save. On success, update `saved_content` on the appropriate variant.

- [ ] **Step 7: Verify build**

Run: `cargo build`
Expected: compiles.

- [ ] **Step 8: Manual verification**

```bash
cargo run -- new markdown-tab-test
```

- Open `README.md` in the editor → rendered view shows the file with headers/bold/code/lists/links styled.
- Click the **Source** toggle → SourceView with markdown highlighting appears.
- Edit: add `## new section` → status bar shows dirty dot.
- Click **Rendered** → the new section renders.
- Click **Source** again, `Ctrl+S` to save → dirty clears.
- Close the tab (Ctrl+W) → reopens cleanly next time.
- Open a `.markdown`-extension file (any one you can create with `echo '# hi' > /tmp/test.markdown`) — also opens in rendered mode.
- Open a `.rs` file → still opens as source (no regression).

- [ ] **Step 9: Commit**

```bash
git add crates/tp-gui/src/panels/editor/tab_content.rs crates/tp-gui/src/panels/editor/markdown_view.rs crates/tp-gui/src/panels/editor/mod.rs crates/tp-gui/src/panels/editor/editor_tabs.rs
git commit -m "$(cat <<'EOF'
Editor: open .md files in rendered view with Source toggle

Markdown files dispatched via extension match get a MarkdownTab containing
a Rendered TextView (using the shared markdown renderer) and a Source
sourceview5::View. Toggle button switches between them; save path writes
the source buffer to disk. Non-markdown files are unaffected.
EOF
)"
```

---

### Task 5: Image tab + image dispatch

**Goal:** Build `ImageTab` and dispatch image extensions so image files open in a viewer with a metadata header and zoom controls.

**Files:**
- Modify: `crates/tp-gui/src/panels/editor/tab_content.rs` (flesh out `ImageTab`)
- Create: `crates/tp-gui/src/panels/editor/image_view.rs`
- Modify: `crates/tp-gui/src/panels/editor/mod.rs` (`mod image_view;`)
- Modify: `crates/tp-gui/src/panels/editor/editor_tabs.rs` (add Image branch in `open_file` dispatch)

- [ ] **Step 1: Flesh out `ImageTab`**

```rust
// tab_content.rs

#[derive(Debug, Clone)]
pub struct ImageTab {
    pub picture: gtk4::Picture,
    /// Natural width in pixels (from the image's intrinsic size). 0 when unknown.
    pub natural_width: i32,
    /// Natural height in pixels. 0 when unknown.
    pub natural_height: i32,
    pub zoom: Rc<std::cell::Cell<f64>>,
    /// Reset-zoom button label — handle is kept so keyboard shortcuts (Task 6)
    /// can update "100%" to reflect the current zoom.
    pub reset_button: gtk4::Button,
    pub outer: gtk4::Widget,
}
```

- [ ] **Step 2: Create `crates/tp-gui/src/panels/editor/image_view.rs`**

```rust
//! Image tab: metadata header + Picture + zoom controls.

use gtk4::prelude::*;
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use super::tab_content::ImageTab;

pub const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg",
];

const ZOOM_MIN: f64 = 0.1;
const ZOOM_MAX: f64 = 10.0;
const ZOOM_STEP: f64 = 1.25;
const HEADER_MARGIN: i32 = 6;

pub fn build_image_tab(path: &Path) -> ImageTab {
    let picture = gtk4::Picture::for_filename(path);
    picture.set_content_fit(gtk4::ContentFit::Contain);

    let paintable = picture.paintable();
    let natural_width = paintable.as_ref().map(|p| p.intrinsic_width()).unwrap_or(0);
    let natural_height = paintable.as_ref().map(|p| p.intrinsic_height()).unwrap_or(0);

    let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_uppercase())
        .unwrap_or_else(|| "?".into());
    let meta_text = format!(
        "{}×{} · {} · {}",
        natural_width,
        natural_height,
        human_size(size_bytes),
        format
    );

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    header.add_css_class("image-header");
    header.set_margin_start(HEADER_MARGIN);
    header.set_margin_end(HEADER_MARGIN);
    header.set_margin_top(HEADER_MARGIN);
    header.set_margin_bottom(HEADER_MARGIN);

    let meta_label = gtk4::Label::new(Some(&meta_text));
    meta_label.set_halign(gtk4::Align::Start);
    meta_label.set_hexpand(true);
    header.append(&meta_label);

    let zoom = Rc::new(Cell::new(1.0_f64));
    let minus_btn = gtk4::Button::from_icon_name("zoom-out-symbolic");
    minus_btn.add_css_class("flat");
    let reset_btn = gtk4::Button::with_label("100%");
    reset_btn.add_css_class("flat");
    let plus_btn = gtk4::Button::from_icon_name("zoom-in-symbolic");
    plus_btn.add_css_class("flat");
    header.append(&minus_btn);
    header.append(&reset_btn);
    header.append(&plus_btn);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&picture));
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.append(&header);
    outer.append(&scroll);

    let apply_zoom = {
        let pic = picture.clone();
        let reset_lbl = reset_btn.clone();
        let zoom_c = zoom.clone();
        let w = natural_width;
        let h = natural_height;
        move || {
            let z = zoom_c.get();
            pic.set_size_request((w as f64 * z) as i32, (h as f64 * z) as i32);
            reset_lbl.set_label(&format!("{}%", (z * 100.0).round() as i32));
        }
    };

    {
        let zoom_c = zoom.clone();
        let apply = apply_zoom.clone();
        minus_btn.connect_clicked(move |_| {
            let z = (zoom_c.get() / ZOOM_STEP).max(ZOOM_MIN);
            zoom_c.set(z);
            apply();
        });
    }
    {
        let zoom_c = zoom.clone();
        let apply = apply_zoom.clone();
        plus_btn.connect_clicked(move |_| {
            let z = (zoom_c.get() * ZOOM_STEP).min(ZOOM_MAX);
            zoom_c.set(z);
            apply();
        });
    }
    {
        let zoom_c = zoom.clone();
        let apply = apply_zoom.clone();
        reset_btn.connect_clicked(move |_| {
            zoom_c.set(1.0);
            apply();
        });
    }

    // Initial apply so the label reads 100% and size is set.
    apply_zoom();

    ImageTab {
        picture,
        natural_width,
        natural_height,
        zoom,
        reset_button: reset_btn,
        outer: outer.upcast::<gtk4::Widget>(),
    }
}

fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
```

- [ ] **Step 3: Register module in `mod.rs`**

```rust
#[cfg(feature = "sourceview")]
mod image_view;
```

- [ ] **Step 4: Add Image dispatch branch in `open_file`**

Extend the dispatch added in Task 4:

```rust
match path.extension().and_then(|s| s.to_str()).map(str::to_ascii_lowercase).as_deref() {
    Some("md") | Some("markdown") => {
        return self.open_markdown_file(path, state);
    }
    Some(ext) if super::image_view::IMAGE_EXTS.contains(&ext) => {
        return self.open_image_file(path, state);
    }
    _ => {}
}
```

- [ ] **Step 5: Implement `open_image_file`**

```rust
fn open_image_file(&self, path: &Path, state: &Rc<RefCell<EditorState>>) -> Option<usize> {
    // Local-only in first pass — remote images would need backend bytes support.
    if state.borrow().backend.is_remote() {
        tracing::warn!("Image preview not supported over SSH for {}", path.display());
        return None;
    }

    let img = super::image_view::build_image_tab(path);

    let tab_id = alloc_tab_id();
    let child_name = format!("tab-{}", tab_id);
    self.content_stack.add_named(&img.outer, Some(&child_name));
    self.content_stack.set_visible_child_name(&child_name);

    let (tab_box, label, _dot, close_btn) = build_tab_label(path);
    let page_widget = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page_widget.set_height_request(0);
    let _page = self.notebook.append_page(&page_widget, Some(&tab_box));
    self.notebook.set_show_tabs(true);

    let mtime = get_mtime(path);
    let idx = {
        let mut st = state.borrow_mut();
        st.open_files.push(super::OpenFile {
            tab_id,
            path: path.to_path_buf(),
            last_disk_mtime: mtime,
            name_label: label.clone(),
            content: super::tab_content::TabContent::Image(img),
        });
        st.active_tab = Some(st.open_files.len() - 1);
        st.open_files.len() - 1
    };

    wire_close_button(&close_btn, tab_id, self, state);
    self.notebook.set_current_page(Some(idx as u32));
    Some(idx)
}
```

- [ ] **Step 6: Verify build**

Run: `cargo build`
Expected: compiles.

- [ ] **Step 7: Manual verification**

```bash
cargo run -- new image-tab-test
```

- Open any `.png` asset in the project (e.g. a file in `resources/` or an icon).
- Metadata header shows dimensions · size · `PNG`.
- `−` / `+` / `100%` buttons zoom; label updates.
- Zoom in/out clamps at min/max.
- Open an `.svg` → renders (librsvg).
- Open a `.jpg` / `.gif` → renders.
- Open a broken file (create one with `touch /tmp/broken.png`) — picture falls back to empty; no crash. Metadata reads `0×0 · 0 B · PNG`.
- Open any `.rs` file → still opens as source (no regression).
- Open a `.md` file → still opens as Markdown (Task 4 regression).

- [ ] **Step 8: Commit**

```bash
git add crates/tp-gui/src/panels/editor/tab_content.rs crates/tp-gui/src/panels/editor/image_view.rs crates/tp-gui/src/panels/editor/mod.rs crates/tp-gui/src/panels/editor/editor_tabs.rs
git commit -m "$(cat <<'EOF'
Editor: open image files in a Picture-based viewer with zoom controls

PNG/JPG/JPEG/GIF/WEBP/BMP/ICO/SVG dispatched via extension open in an
ImageTab containing a metadata header (dimensions · size · format) and
zoom controls (-, 100%, +). Zoom clamps to [0.1, 10.0] with geometric
step. Remote (SSH) backends log a warning and decline to open images;
first pass is local-only.
EOF
)"
```

---

### Task 6: Keyboard shortcuts for viewers

**Goal:** Wire `Ctrl+Shift+V` to toggle Rendered/Source on Markdown tabs, and `Ctrl+=` / `Ctrl++` / `Ctrl+-` / `Ctrl+0` for image zoom.

**Files:**
- Modify: `crates/tp-gui/src/panels/editor/mod.rs` (extend the top-level key handler)
- Modify: `crates/tp-gui/src/panels/editor/markdown_view.rs` (public helper to toggle mode)
- Modify: `crates/tp-gui/src/panels/editor/image_view.rs` (public helpers to zoom in / out / reset)

- [ ] **Step 1: Verify `Ctrl+Shift+V` is free**

```
rg -n "shift.*[vV]|V.*shift|ControlMask.*ShiftMask" crates/tp-gui/src/
```

Also check existing paste handling — GTK text widgets auto-bind Ctrl+Shift+V to paste-without-formatting. If that's a concern, pick `Ctrl+Alt+M` instead (toggle **M**arkdown mode). Document in the plan the chosen chord before proceeding.

*Decision point for the implementer:* if `Ctrl+Shift+V` pastes without formatting is desired, use `Ctrl+Alt+M` for markdown toggle.

- [ ] **Step 2: Expose helpers on `MarkdownTab` / `ImageTab`**

In `markdown_view.rs`, add to `build_markdown_tab` or as a separate free function operating on the `MarkdownTab`:

```rust
pub fn toggle_mode(tab: &MarkdownTab) {
    let new_mode = match tab.mode.get() {
        MarkdownMode::Rendered => MarkdownMode::Source,
        MarkdownMode::Source => MarkdownMode::Rendered,
    };
    match new_mode {
        MarkdownMode::Rendered => {
            let text = tab.buffer.text(&tab.buffer.start_iter(), &tab.buffer.end_iter(), false).to_string();
            crate::markdown_render::render_markdown_to_view(&tab.rendered_view, &text);
            tab.inner_stack.set_visible_child_name("rendered");
        }
        MarkdownMode::Source => {
            tab.inner_stack.set_visible_child_name("source");
        }
    }
    tab.mode.set(new_mode);
}
```

In `image_view.rs`:

```rust
pub fn zoom_in(tab: &ImageTab) { set_zoom(tab, (tab.zoom.get() * ZOOM_STEP).min(ZOOM_MAX)); }
pub fn zoom_out(tab: &ImageTab) { set_zoom(tab, (tab.zoom.get() / ZOOM_STEP).max(ZOOM_MIN)); }
pub fn zoom_reset(tab: &ImageTab) { set_zoom(tab, 1.0); }

fn set_zoom(tab: &ImageTab, z: f64) {
    tab.zoom.set(z);
    let w = tab.natural_width;
    let h = tab.natural_height;
    tab.picture.set_size_request((w as f64 * z) as i32, (h as f64 * z) as i32);
    tab.reset_button.set_label(&format!("{}%", (z * 100.0).round() as i32));
}
```

The `reset_button` handle was added to `ImageTab` in Task 5 specifically so keyboard shortcuts can update the displayed "100%" label here. The original in-tab button callbacks (in Task 5 `build_image_tab`) must be updated in this task to delegate to these free functions so both paths stay in sync — change the closures so `minus_btn.connect_clicked(...)` calls `zoom_out(&tab)`, etc. (Passing `&ImageTab` requires cloning the struct into the closure; `ImageTab` already derives `Clone`.)

- [ ] **Step 3: Add shortcut handling in `mod.rs` key controller**

In the big `key_ctrl.connect_key_pressed` block (line ~641 in `mod.rs`):

```rust
// Ctrl+Shift+V → markdown render/source toggle (or Ctrl+Alt+M per Step 1 decision)
if crate::shortcuts::has_primary(modifier)
    && modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK)
    && matches!(key, gtk4::gdk::Key::v | gtk4::gdk::Key::V)
{
    let st = state_c.borrow();
    if let Some(idx) = st.active_tab {
        if let Some(f) = st.open_files.get(idx) {
            if let super::tab_content::TabContent::Markdown(m) = &f.content {
                drop(st);
                super::markdown_view::toggle_mode(m);
                return gtk4::glib::Propagation::Stop;
            }
        }
    }
}

// Ctrl+= / Ctrl++ / Ctrl+- / Ctrl+0 → image zoom (only when image tab active)
if crate::shortcuts::has_primary(modifier) {
    let on_image = |f: &mut dyn FnMut(&super::tab_content::ImageTab)| -> bool {
        let st = state_c.borrow();
        let Some(idx) = st.active_tab else { return false };
        let Some(of) = st.open_files.get(idx) else { return false };
        let super::tab_content::TabContent::Image(img) = &of.content else { return false };
        f(img);
        true
    };
    match key {
        gtk4::gdk::Key::equal | gtk4::gdk::Key::plus => {
            let mut done = false;
            let _ = on_image(&mut |img| { super::image_view::zoom_in(img); done = true; });
            if done { return gtk4::glib::Propagation::Stop; }
        }
        gtk4::gdk::Key::minus => {
            let mut done = false;
            let _ = on_image(&mut |img| { super::image_view::zoom_out(img); done = true; });
            if done { return gtk4::glib::Propagation::Stop; }
        }
        gtk4::gdk::Key::_0 => {
            let mut done = false;
            let _ = on_image(&mut |img| { super::image_view::zoom_reset(img); done = true; });
            if done { return gtk4::glib::Propagation::Stop; }
        }
        _ => {}
    }
}
```

Place this block **before** the existing primary-modifier match so it takes precedence. The existing Ctrl+F / Ctrl+S / Ctrl+W etc. still need to fire for source tabs — the new block early-returns only when the active tab is the matching viewer type.

- [ ] **Step 4: Verify build**

Run: `cargo build`
Expected: compiles.

- [ ] **Step 5: Manual verification**

```bash
cargo run -- new viewer-shortcuts-test
```

- Open a `.md` file. `Ctrl+Shift+V` (or `Ctrl+Alt+M`) toggles Rendered ↔ Source. Button state matches the current mode.
- Open an image. `Ctrl+=` / `Ctrl++` zoom in; `Ctrl+-` zoom out; `Ctrl+0` reset to 100%. Header label updates.
- Switch to a source tab and press Ctrl+Shift+V — nothing (must not crash; must not affect other tabs).
- Switch to a source tab and press `Ctrl+0` — nothing (no accidental handling).
- Ctrl+S still saves source / markdown-source; Ctrl+F still searches source; Ctrl+W still closes any tab.

- [ ] **Step 6: Commit**

```bash
git add crates/tp-gui/src/panels/editor/mod.rs crates/tp-gui/src/panels/editor/markdown_view.rs crates/tp-gui/src/panels/editor/image_view.rs crates/tp-gui/src/panels/editor/tab_content.rs
git commit -m "$(cat <<'EOF'
Editor: keyboard shortcuts for markdown toggle and image zoom

Ctrl+Shift+V toggles Rendered/Source on a Markdown tab; Ctrl+=, Ctrl++,
Ctrl+- and Ctrl+0 zoom the active Image tab. Shortcuts no-op on other
tab kinds so existing Source shortcuts are not affected.
EOF
)"
```

---

## Coverage check

| Spec requirement | Task |
|---|---|
| `.md` / `.markdown` open rendered by default | 4 |
| Rendered ↔ Source toggle in markdown tab | 4, 6 |
| Shared markdown renderer (no duplication) | 1 |
| Image extensions dispatched via `IMAGE_EXTS` | 5 |
| Metadata header (dimensions · size · format) | 5 |
| Zoom controls (buttons + shortcuts) | 5, 6 |
| Zoom clamps to `[ZOOM_MIN, ZOOM_MAX]`, step `ZOOM_STEP` | 5 |
| SVG via librsvg | 5 |
| Local-only image loading (SSH declines gracefully) | 5 |
| Per-tab content_stack child | 3, 4, 5 |
| Source tabs continue to work unchanged | 3 (regression tests) |
| Existing Markdown panel unaffected | 1 (regression test) |
| Dirty tracking for Markdown source | 4 |
| No magic numbers | all (const TAB_WIDTH, ZOOM_MIN, etc.) |
| Commit after each task | all |
| No unit tests in commits | all (manual verification only) |
| No `Co-Authored-By` trailers | all |
