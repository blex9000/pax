# Code Editor media viewers — design

Status: approved
Date: 2026-04-22
Scope: Add viewers for images and Markdown in the Code Editor panel. JSON/YAML structured viewers are out of scope for this spec (follow-up).

## Goal

When the user opens a file in the Code Editor, dispatch on extension so that:

- Markdown files (`.md`, `.markdown`) open in a **rendered** view by default, with a toggle to switch to source editing.
- Image files (`.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.bmp`, `.ico`, `.svg`) open in an **image viewer** with zoom controls and a metadata header (dimensions · size · format).
- Every other extension continues to open in the existing `sourceview5::View`.

The Markdown panel type (`crates/tp-gui/src/panels/markdown.rs`) already contains a working markdown renderer; the editor viewer reuses that implementation via extraction to a shared module.

## Non-goals

- JSON / YAML structured viewers (outline, folding, breadcrumb). Separate spec.
- Images over SSH (`file_backend` remote path): first pass is local-only; a remote image shows an "image preview not supported over SSH" placeholder.
- Binary-file detection by content sniffing. Dispatch is by file extension only.
- Asynchronous image loading. First pass loads synchronously; revisit if a large image noticeably blocks the UI thread.

## Current state (for context)

- `crates/tp-gui/src/panels/editor/editor_tabs.rs` keeps `EditorState.open_files: Vec<OpenFile>` and a single long-lived `sourceview5::View` whose buffer is swapped on every tab switch (lines 183–191, 331–358). Every open file must therefore expose a `sourceview5::Buffer`.
- `crates/tp-gui/src/panels/markdown.rs` implements a Render/Edit mode panel; `render_markdown_to_view()` (around line 556) parses Markdown line-by-line into a `gtk::TextBuffer` with `TextTag`s. No external Markdown library.
- `gtk4 0.9` is in use with the `v4_10` feature enabled; `gtk::Picture` is available. `gdk-pixbuf` is transitively available through gtk4 but needs to be declared explicitly if we use the `Pixbuf` API directly.

## Design

### 1. File type dispatch

`open_file()` in `editor_tabs.rs` lowercases the extension and branches:

| Extensions | Tab kind |
|---|---|
| `md`, `markdown` | `TabContent::Markdown` (rendered by default) |
| `png`, `jpg`, `jpeg`, `gif`, `webp`, `bmp`, `ico`, `svg` | `TabContent::Image` |
| anything else (including no extension) | `TabContent::Source` (unchanged) |

The image extension list lives as `const IMAGE_EXTS: &[&str] = &[...]` in `image_view.rs`. There is no content sniffing, and no MIME check — a file named `foo.png` renamed to `foo.txt` opens as source. Dispatch is re-evaluated on every `open_file()` call, so rename-then-reopen reflects the new extension.

### 2. Tab-content refactor

Today the editor uses one `sourceview5::View` and swaps its `sourceview5::Buffer` on every tab switch. To support heterogeneous tabs, each `OpenFile` owns its own content widget.

```rust
// crates/tp-gui/src/panels/editor/tab_content.rs  (NEW)
pub enum TabContent {
    Source(SourceTab),
    Markdown(MarkdownTab),
    Image(ImageTab),
}
```

`OpenFile` gains a `content: TabContent` field; the dedicated `buffer: sourceview5::Buffer` and `modified: bool` fields are migrated into `SourceTab`/`MarkdownTab` as appropriate (image tabs cannot be dirty).

The editor's existing `content_stack: gtk::Stack` (welcome / editor today) becomes the per-tab stack: one stack child per `tab_id`, named by `tab_id.to_string()`. Tab switch performs `content_stack.set_visible_child_name(&tab_id)` instead of `source_view.set_buffer(...)`. The welcome child is a fixed special name.

The single shared `source_view` is dropped. Source tabs each own their own `sourceview5::View`; the completion-words provider and keyword-shadow buffer remain shared and each new view registers with them on creation.

Status bar reads from the active tab:

- Source / Markdown-source: `Ln X, Col Y` + language + modified.
- Markdown-rendered: language = `Markdown`, no line/column, no modified.
- Image: `{W}×{H} · {size} · {FORMAT}` in the language slot, line/column empty, no modified.

### 3. Markdown tab

Module: `crates/tp-gui/src/panels/editor/markdown_view.rs` (NEW).

Layout:

```
Box(V)
├── ToolBar  [ Rendered | Source ]   (gtk::ToggleButton pair, linked styling)
└── Stack
    ├── "rendered" → ScrolledWindow → TextView (read-only)
    └── "source"   → ScrolledWindow → sourceview5::View (markdown language)
```

- Default visible child: `"rendered"`.
- Rendering is delegated to `crate::markdown_render::render_markdown_to_view(&text_view, &text)`, a module extracted from `panels/markdown.rs`. The existing `MarkdownPanel` is migrated to call the same shared function; its current inline implementation is deleted.
- Toggle button switches the stack's visible child. Every time the user switches to Rendered, re-render from the current source buffer text (dirty OK, matches existing Markdown panel behavior).
- Source mode uses the editor's normal save path (Ctrl+S). Dirty tracking works as for any source tab. After save, re-rendering on next toggle already reflects the saved content because it reads from the buffer.
- File watcher: if the file changes on disk and the buffer is clean, reload buffer and re-render. If dirty, raise the existing "file changed externally" info bar (same policy as source tabs today).
- Keyboard shortcut: `Ctrl+Shift+V` toggles Rendered/Source on the active tab (verify at impl time that no existing shortcut collides; otherwise pick a free chord).

### 4. Image tab

Module: `crates/tp-gui/src/panels/editor/image_view.rs` (NEW).

Layout:

```
Box(V)
├── Box(H) .image-header  [ "{W}×{H} · {size} · {FORMAT}"   (spacer)   − 100% + ]
└── ScrolledWindow
    └── Picture   (content-fit: Contain; size request = natural × zoom)
```

(The header is a plain `gtk::Box(Horizontal)` with a CSS class, not `gtk::HeaderBar`.)

Loading:

- Raster: `gdk_pixbuf::Pixbuf::from_file(path)`; reads width, height, and (from file metadata) byte size. Format is derived from extension (uppercased).
- SVG: load through `gtk::Picture::for_file()` (librsvg via gtk4). Dimensions taken from the picture's `paintable` intrinsic size.
- Errors (corrupt file, unsupported format): show a centered error label inside the tab instead of the Picture. Do not close the tab — the user can retry or close manually.

Zoom:

- State: `Rc<Cell<f64>>`, start `1.0`, clamp to `[ZOOM_MIN, ZOOM_MAX]` = `[0.1, 10.0]`, step `ZOOM_STEP = 1.25` (geometric).
- Buttons `−` / `100%` / `+` plus shortcuts `Ctrl+-`, `Ctrl+0`, `Ctrl+=` / `Ctrl++`, and `Ctrl+scroll` over the image.
- Applied by `picture.set_size_request(w_natural × zoom, h_natural × zoom)`.
- Constants live at the top of `image_view.rs`: `const ZOOM_MIN: f64 = 0.1;`, `const ZOOM_MAX: f64 = 10.0;`, `const ZOOM_STEP: f64 = 1.25;`, `const IMAGE_EXTS: &[&str] = &[...];`.

Read-only: Ctrl+S is a no-op for image tabs, dirty marker is never set, search bar (Ctrl+F) is inert.

### 5. Shared markdown renderer extraction

New file `crates/tp-gui/src/markdown_render.rs` exports:

- `pub fn render_markdown_to_view(view: &gtk4::TextView, source: &str)` — the current body of `render_markdown_to_view` from `panels/markdown.rs`.
- Helper functions / TextTag setup referenced by it.

`panels/markdown.rs` changes:

- Removes the inline implementation of `render_markdown_to_view`.
- Imports and calls `crate::markdown_render::render_markdown_to_view`.
- Keeps everything else (mode toggle, file watcher, save/reload).

Regression check: after the move, opening a file in the standalone Markdown panel must look pixel-identical to before.

### 6. Save / dirty semantics

| Tab kind | Save action | Dirty tracking |
|---|---|---|
| Source | existing path | existing path |
| Markdown (source mode) | existing path (writes buffer to file) | existing path |
| Markdown (rendered mode) | no-op (rendered is a view of the in-memory buffer) | same dirty flag as source mode — switching to rendered does **not** clear it |
| Image | no-op | never dirty |

### 7. Keybindings

| Shortcut | Context | Action |
|---|---|---|
| `Ctrl+Shift+V` | Markdown tab | Toggle Rendered ↔ Source |
| `Ctrl+=` / `Ctrl++` | Image tab | Zoom in |
| `Ctrl+-` | Image tab | Zoom out |
| `Ctrl+0` | Image tab | Reset zoom to 100% |
| `Ctrl+scroll` | Image tab | Zoom around cursor |

Verify at implementation time that `Ctrl+Shift+V` is not already bound by the editor or the wider shell; fall back to another free chord if it is.

## Files

New:

- `crates/tp-gui/src/markdown_render.rs`
- `crates/tp-gui/src/panels/editor/tab_content.rs`
- `crates/tp-gui/src/panels/editor/markdown_view.rs`
- `crates/tp-gui/src/panels/editor/image_view.rs`

Modified:

- `crates/tp-gui/src/lib.rs` — `pub mod markdown_render;`
- `crates/tp-gui/src/panels/markdown.rs` — delegate to shared renderer
- `crates/tp-gui/src/panels/editor/editor_tabs.rs` — extension dispatch in `open_file()`, drop single shared `source_view`, per-tab stack children
- `crates/tp-gui/src/panels/editor/mod.rs` — re-exports for new modules
- `crates/tp-gui/Cargo.toml` — add `gdk-pixbuf` dependency if not already pulled in via gtk4 re-exports

## Test plan

Manual, executed with the build after each coherent step (per project convention in `CLAUDE.md`):

1. Open `README.md` in the editor → rendered view shows headers, bold, code blocks, lists.
2. Click Source → edit text → `Ctrl+S` → reopen in file manager / `stat` mtime confirms save → click Rendered → edits visible.
3. Open `config/example.json` → still opens in `SourceView` with syntax highlighting (unchanged).
4. Open a `.png` asset → image + metadata header visible.
5. Zoom image with `Ctrl+=`, `Ctrl+-`, `Ctrl+0`, Ctrl+scroll, and the three buttons → size updates; zoom clamps at min/max.
6. Open an SVG → renders via librsvg.
7. Rename `foo.png` → `foo.txt` and open → opens as source (extension-based dispatch).
8. Open a broken/truncated PNG → error label inside tab; tab still closable.
9. Close a Markdown tab, reopen → fresh state (no leaked buffer).
10. Switch between Source / Markdown / Image tabs → correct widget shown; status bar updates.
11. Open a Markdown file in the standalone **Markdown panel** (not the editor) → identical rendering to before the refactor. Regression check on the shared renderer.

## Risks

- **Stack-per-tab refactor touches a lot of editor code.** Search, replace, completion, match-ruler, cursor-position label are all wired against the single shared `source_view`. Each must rewire to read from the active tab's view. This is the largest risk in the spec and the most likely source of regressions — the refactor needs to be done carefully with the test plan exercised each step.
- **Shared renderer extraction** could regress the Markdown panel. Mitigated by the explicit regression step in the test plan.
- **`Ctrl+Shift+V`** may collide with an existing shortcut; verify at implementation time.
- **Large images** load synchronously. Acceptable for first pass; revisit if the UI noticeably stalls.
