use gtk4::glib;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[cfg(feature = "sourceview")]
use sourceview5::prelude::*;

use super::{text_sync, PanelBackend, PanelInputCallback};
use crate::notebook::cell::NotebookCell;
use crate::notebook::engine::NotebookEngine;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Render,
    Edit,
}

#[derive(Debug, Clone)]
enum MarkdownDocument {
    File {
        path: String,
    },
    Database {
        record_key: String,
        panel_id: String,
    },
}

impl MarkdownDocument {
    fn file(path: &str) -> Self {
        Self::File {
            path: path.to_string(),
        }
    }

    fn database(record_key: String, panel_id: String) -> Self {
        Self::Database {
            record_key: if record_key.trim().is_empty() {
                "__unknown_workspace__".to_string()
            } else {
                record_key
            },
            panel_id: if panel_id.trim().is_empty() {
                "__markdown_panel__".to_string()
            } else {
                panel_id
            },
        }
    }

    fn label(&self) -> String {
        match self {
            Self::File { path } => path.clone(),
            Self::Database { .. } => "In-memory markdown document".to_string(),
        }
    }

    fn file_path(&self) -> Option<&str> {
        match self {
            Self::File { path } => Some(path.as_str()),
            Self::Database { .. } => None,
        }
    }

    fn load(&self) -> Result<String, String> {
        match self {
            Self::File { path } => std::fs::read_to_string(path).map_err(|e| e.to_string()),
            Self::Database {
                record_key,
                panel_id,
            } => Self::open_db()?
                .get_or_create_markdown_document(record_key, panel_id)
                .map_err(|e| e.to_string()),
        }
    }

    fn save(&self, content: &str) -> Result<(), String> {
        match self {
            Self::File { path } => std::fs::write(path, content).map_err(|e| e.to_string()),
            Self::Database {
                record_key,
                panel_id,
            } => Self::open_db()?
                .save_markdown_document(record_key, panel_id, content)
                .map_err(|e| e.to_string()),
        }
    }

    fn content_len(&self) -> i64 {
        let Self::Database {
            record_key,
            panel_id,
        } = self
        else {
            return 0;
        };
        Self::open_db()
            .and_then(|db| {
                db.markdown_document_len(record_key, panel_id)
                    .map_err(|e| e.to_string())
            })
            .unwrap_or(0)
    }

    fn delete_persisted_state(&self) {
        let Self::Database {
            record_key,
            panel_id,
        } = self
        else {
            return;
        };
        match Self::open_db().and_then(|db| {
            db.delete_markdown_document(record_key, panel_id)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }) {
            Ok(()) => {}
            Err(e) => tracing::warn!("markdown panel: failed to delete DB document: {e}"),
        }
    }

    fn is_database(&self) -> bool {
        matches!(self, Self::Database { .. })
    }

    fn open_db() -> Result<pax_db::Database, String> {
        pax_db::Database::open(&pax_db::Database::default_path()).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextHistoryAction {
    Undo,
    Redo,
}

fn text_history_action(
    key: gtk4::gdk::Key,
    modifiers: gtk4::gdk::ModifierType,
) -> Option<TextHistoryAction> {
    let primary = crate::shortcuts::has_primary(modifiers);
    let shift = modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
    if !primary {
        return None;
    }

    match key {
        gtk4::gdk::Key::z if !shift => Some(TextHistoryAction::Undo),
        gtk4::gdk::Key::y if !shift => Some(TextHistoryAction::Redo),
        gtk4::gdk::Key::Z if shift => Some(TextHistoryAction::Redo),
        _ => None,
    }
}

fn install_text_history_shortcuts<W: IsA<gtk4::Widget>>(widget: &W, buffer: &gtk4::TextBuffer) {
    let buffer = buffer.clone();
    let key_ctrl = gtk4::EventControllerKey::new();
    key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
    key_ctrl.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(action) = text_history_action(key, modifiers) else {
            return gtk4::glib::Propagation::Proceed;
        };

        match action {
            TextHistoryAction::Undo => {
                if buffer.can_undo() {
                    buffer.undo();
                }
            }
            TextHistoryAction::Redo => {
                if buffer.can_redo() {
                    buffer.redo();
                }
            }
        }

        gtk4::glib::Propagation::Stop
    });
    widget.add_controller(key_ctrl);
}

/// Markdown viewer/editor panel.
/// Uses GtkSourceView 5 for edit mode when available (Linux),
/// falls back to plain TextView on macOS/no-sourceview.
pub struct MarkdownPanel {
    widget: gtk4::Widget,
    render_view: gtk4::TextView,
    #[cfg(feature = "sourceview")]
    source_view: sourceview5::View,
    #[cfg(not(feature = "sourceview"))]
    source_view: gtk4::TextView,
    /// Edit buffer — exposed so sync-input can mutate it directly.
    source_buffer: gtk4::TextBuffer,
    /// Toggled to Edit mode when remote sync input arrives so the user can
    /// see the mirrored text take effect.
    edit_btn: gtk4::ToggleButton,
    /// Set while applying sync-input from a peer to break the
    /// `insert-text → input_cb → write_input → insert-text` feedback loop.
    suppress_emit: Rc<Cell<bool>>,
    /// Active sync-input observer registered by the host.
    input_cb: Rc<RefCell<Option<PanelInputCallback>>>,
    #[allow(dead_code)]
    file_path: String,
    document: MarkdownDocument,
    content: Rc<RefCell<String>>,
    modified: Rc<Cell<bool>>,
    /// Lazily-created notebook engine. Created on the first render that
    /// encounters a notebook cell, dropped (and recreated) when render mode
    /// is re-entered or the file is reloaded so watch timers don't pile up.
    #[allow(dead_code)]
    notebook_engine: Rc<RefCell<Option<Rc<NotebookEngine>>>>,
    theme_observer: crate::theme::ThemeObserverId,
    watch_active: Rc<Cell<bool>>,
}

impl std::fmt::Debug for MarkdownPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarkdownPanel")
            .field("file_path", &self.file_path)
            .field("document", &self.document)
            .finish()
    }
}

impl MarkdownPanel {
    pub fn new(file_path: &str) -> Self {
        Self::new_file(file_path)
    }

    pub fn new_file(file_path: &str) -> Self {
        crate::recent_markdown::record(file_path);
        Self::new_with_document(MarkdownDocument::file(file_path))
    }

    pub fn new_database(record_key: String, panel_id: String) -> Self {
        Self::new_with_document(MarkdownDocument::database(record_key, panel_id))
    }

    fn new_with_document(document: MarkdownDocument) -> Self {
        let file_path = document.label();
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let mode = Rc::new(Cell::new(Mode::Render));
        let content = Rc::new(RefCell::new(String::new()));
        let modified = Rc::new(Cell::new(false));
        let suppress_emit: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let input_cb: Rc<RefCell<Option<PanelInputCallback>>> = Rc::new(RefCell::new(None));
        let watch_active = Rc::new(Cell::new(true));

        // ── Main toolbar ─────────────────────────────────────────────────
        let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
        toolbar.add_css_class("markdown-toolbar");
        toolbar.set_margin_start(2);
        toolbar.set_margin_end(2);
        toolbar.set_margin_top(1);
        toolbar.set_margin_bottom(1);

        let render_btn = gtk4::ToggleButton::new();
        render_btn.set_icon_name("emblem-documents-symbolic");
        render_btn.set_active(true);
        render_btn.add_css_class("flat");
        render_btn.set_tooltip_text(Some("Render view"));

        let edit_btn = gtk4::ToggleButton::new();
        edit_btn.set_icon_name("document-edit-symbolic");
        edit_btn.add_css_class("flat");
        edit_btn.set_tooltip_text(Some("Edit mode"));
        edit_btn.set_group(Some(&render_btn));

        let undo_btn = gtk4::Button::new();
        undo_btn.set_icon_name("edit-undo-symbolic");
        undo_btn.add_css_class("flat");
        undo_btn.set_sensitive(false);
        undo_btn.set_tooltip_text(Some("Undo (Ctrl+Z)"));

        let redo_btn = gtk4::Button::new();
        redo_btn.set_icon_name("edit-redo-symbolic");
        redo_btn.add_css_class("flat");
        redo_btn.set_sensitive(false);
        redo_btn.set_tooltip_text(Some("Redo (Ctrl+Y / Ctrl+Shift+Z)"));

        let save_btn = gtk4::Button::new();
        save_btn.set_icon_name("media-floppy-symbolic");
        save_btn.set_sensitive(false);
        save_btn.add_css_class("flat");

        let reload_btn = gtk4::Button::new();
        reload_btn.set_icon_name("view-refresh-symbolic");
        reload_btn.add_css_class("flat");

        let export_pdf_btn = gtk4::Button::new();
        export_pdf_btn.set_icon_name("document-save-as-symbolic");
        export_pdf_btn.add_css_class("flat");
        export_pdf_btn.set_tooltip_text(Some("Export to PDF"));
        export_pdf_btn.set_margin_end(8);

        let help_btn = gtk4::Button::new();
        help_btn.set_icon_name("help-about-symbolic");
        help_btn.add_css_class("flat");
        help_btn.set_tooltip_text(Some("Markdown notebook help"));

        let file_label = gtk4::Label::new(Some(&file_path));
        file_label.add_css_class("dim-label");
        file_label.add_css_class("caption");
        file_label.set_hexpand(true);
        file_label.set_halign(gtk4::Align::End);
        file_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);

        toolbar.append(&render_btn);
        toolbar.append(&edit_btn);
        toolbar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
        toolbar.append(&undo_btn);
        toolbar.append(&redo_btn);
        toolbar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
        toolbar.append(&save_btn);
        toolbar.append(&reload_btn);
        toolbar.append(&export_pdf_btn);
        toolbar.append(&help_btn);
        toolbar.append(&file_label);
        container.append(&toolbar);

        // Conflict bar slot: shown only when an external change collides with
        // a dirty edit-mode buffer. Lives between the toolbar and the content
        // stack so it stays out of the way until needed.
        let conflict_bar_slot = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.append(&conflict_bar_slot);

        // Help button: opens the notebook help dialog.
        {
            let parent = container.clone();
            help_btn.connect_clicked(move |_| {
                crate::dialogs::notebook_help::show(parent.upcast_ref::<gtk4::Widget>());
            });
        }

        // ── Formatting toolbar (edit mode only) ──────────────────────────
        let fmt_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        fmt_bar.add_css_class("markdown-toolbar");
        fmt_bar.set_margin_start(2);
        fmt_bar.set_margin_end(2);
        fmt_bar.set_visible(false);

        let edit_buf_ref: Rc<RefCell<Option<gtk4::TextBuffer>>> = Rc::new(RefCell::new(None));

        let fmt_items: Vec<(&str, &str, &str)> = vec![
            ("format-text-bold-symbolic", "Bold", "**"),
            ("format-text-italic-symbolic", "Italic", "*"),
            ("format-text-strikethrough-symbolic", "Strikethrough", "~~"),
            ("accessories-text-editor-symbolic", "Code", "`"),
        ];
        for (icon, tooltip, marker) in &fmt_items {
            let btn = gtk4::Button::new();
            btn.set_icon_name(icon);
            btn.add_css_class("flat");
            btn.set_tooltip_text(Some(tooltip));
            let m = marker.to_string();
            let br = edit_buf_ref.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref buf) = *br.borrow() {
                    wrap_selection_buf(buf, &m);
                }
            });
            fmt_bar.append(&btn);
        }
        fmt_bar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
        for (level, label) in &[(1, "H1"), (2, "H2"), (3, "H3")] {
            let btn = gtk4::Button::with_label(label);
            btn.add_css_class("flat");
            let prefix = "#".repeat(*level);
            let br = edit_buf_ref.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref buf) = *br.borrow() {
                    prepend_line_buf(buf, &format!("{} ", prefix));
                }
            });
            fmt_bar.append(&btn);
        }
        fmt_bar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
        for (icon, tooltip, text) in &[
            ("view-list-symbolic", "List", "- "),
            ("mail-attachment-symbolic", "Link", "[text](url)"),
            ("utilities-terminal-symbolic", "Code block", "```\n\n```"),
            (
                "view-grid-symbolic",
                "Table",
                "| Column 1 | Column 2 | Column 3 |\n|----------|----------|----------|\n| cell     | cell     | cell     |\n",
            ),
        ] {
            let btn = gtk4::Button::new();
            btn.set_icon_name(icon);
            btn.add_css_class("flat");
            btn.set_tooltip_text(Some(tooltip));
            let t = text.to_string();
            let br = edit_buf_ref.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref buf) = *br.borrow() {
                    insert_at_cursor_buf(buf, &t);
                }
            });
            fmt_bar.append(&btn);
        }

        // ── Media / extras group ─────────────────────────────────────
        // Image button opens a file dialog and inserts a relative path,
        // so the user gets a working `![](path)` immediately. The other
        // entries are template-insert helpers like the existing group.
        fmt_bar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
        {
            let img_btn = gtk4::Button::new();
            img_btn.set_icon_name("insert-image-symbolic");
            img_btn.add_css_class("flat");
            img_btn.set_tooltip_text(Some("Insert image (file picker)"));
            let br = edit_buf_ref.clone();
            let parent_for_dialog = container.clone();
            let host_path = document.file_path().map(str::to_string);
            img_btn.connect_clicked(move |_| {
                let dialog = gtk4::FileDialog::builder()
                    .title("Select image")
                    .modal(true)
                    .build();
                let filter = gtk4::FileFilter::new();
                filter.set_name(Some("Images"));
                for pat in ["*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.svg"] {
                    filter.add_pattern(pat);
                }
                let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
                filters.append(&filter);
                dialog.set_filters(Some(&filters));
                let host_dir = host_path
                    .as_deref()
                    .and_then(|path| std::path::Path::new(path).parent())
                    .map(|p| p.to_path_buf());
                let br = br.clone();
                let parent_window = parent_for_dialog
                    .root()
                    .and_then(|r| r.downcast::<gtk4::Window>().ok());
                dialog.open(
                    parent_window.as_ref(),
                    gtk4::gio::Cancellable::NONE,
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                // Relative path if the image lives under the
                                // markdown file's directory; absolute otherwise.
                                let rel = host_dir
                                    .as_ref()
                                    .and_then(|hd| path.strip_prefix(hd).ok())
                                    .map(|p| p.to_path_buf())
                                    .unwrap_or(path);
                                let alt =
                                    rel.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
                                let snippet = format!("![{}]({})", alt, rel.to_string_lossy());
                                if let Some(ref buf) = *br.borrow() {
                                    insert_at_cursor_buf(buf, &snippet);
                                }
                            }
                        }
                    },
                );
            });
            fmt_bar.append(&img_btn);
        }
        for (icon, tooltip, text) in &[
            ("format-indent-more-symbolic", "Quote", "> "),
            ("list-remove-symbolic", "Horizontal rule", "\n---\n"),
            ("emblem-ok-symbolic", "Task item", "- [ ] "),
            (
                "system-run-symbolic",
                "Notebook cell (python run)",
                "```python run\n\n```\n",
            ),
            (
                "applications-graphics-symbolic",
                "Mermaid flowchart",
                "```mermaid\nflowchart TD\n    A[Start] --> B{Condition?}\n    B -- Yes --> C[Action 1]\n    B -- No --> D[Action 2]\n    C --> E[End]\n    D --> E\n```\n",
            ),
        ] {
            let btn = gtk4::Button::new();
            btn.set_icon_name(icon);
            btn.add_css_class("flat");
            btn.set_tooltip_text(Some(tooltip));
            let t = text.to_string();
            let br = edit_buf_ref.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref buf) = *br.borrow() {
                    insert_at_cursor_buf(buf, &t);
                }
            });
            fmt_bar.append(&btn);
        }
        container.append(&fmt_bar);

        // ── Stack: render + edit ─────────────────────────────────────────
        let stack = gtk4::Stack::new();

        let render_view = gtk4::TextView::new();
        render_view.set_editable(false);
        render_view.set_cursor_visible(false);
        // Read-only render — no focus needed. Without this, clicking the
        // view focuses it and ScrolledWindow's Viewport (which wraps the
        // Overlay used for the bq bar) calls scroll-to-focus, snapping
        // the scroll back to the top of the document on every click.
        render_view.set_can_focus(false);
        render_view.set_wrap_mode(gtk4::WrapMode::Word);
        render_view.set_left_margin(10);
        render_view.set_right_margin(10);
        render_view.set_top_margin(6);
        render_view.set_bottom_margin(6);
        render_view.add_css_class("markdown-panel");

        let render_scroll = gtk4::ScrolledWindow::new();
        render_scroll.set_child(Some(&render_view));
        render_scroll.set_vexpand(true);
        render_scroll.set_hexpand(true);
        crate::markdown_render::attach_blockquote_bar_overlay(&render_scroll, &render_view);
        stack.add_named(&render_scroll, Some("render"));

        // Edit view — sourceview5 or plain TextView
        #[cfg(feature = "sourceview")]
        let (source_view, source_buffer) = {
            let buf = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
            if let Some(lang) = sourceview5::LanguageManager::default().language("markdown") {
                buf.set_language(Some(&lang));
            }
            buf.set_highlight_syntax(true);
            crate::theme::register_sourceview_buffer(&buf);
            let sv = sourceview5::View::with_buffer(&buf);
            sv.add_css_class("editor-code-view");
            sv.set_show_line_numbers(true);
            sv.set_highlight_current_line(true);
            sv.set_auto_indent(true);
            sv.set_tab_width(4);
            sv.set_wrap_mode(gtk4::WrapMode::Word);
            sv.set_left_margin(6);
            sv.set_top_margin(3);
            sv.set_monospace(true);
            (sv, buf.upcast::<gtk4::TextBuffer>())
        };

        #[cfg(not(feature = "sourceview"))]
        let (source_view, source_buffer) = {
            let tv = gtk4::TextView::new();
            tv.add_css_class("editor-code-view");
            tv.set_wrap_mode(gtk4::WrapMode::Word);
            tv.set_left_margin(6);
            tv.set_top_margin(3);
            tv.set_monospace(true);
            let buf = tv.buffer();
            buf.set_enable_undo(true);
            (tv, buf)
        };

        *edit_buf_ref.borrow_mut() = Some(source_buffer.clone());
        install_text_history_shortcuts(&source_view, &source_buffer);

        // Sync-input outgoing wiring: forward user edits to peer panels via
        // the input callback. Gated on Edit mode so the buffer's mutations
        // during render→edit / edit→render transitions don't leak out, and
        // suppressed during inbound apply to break feedback loops.
        {
            let mode_for_gate = mode.clone();
            let gate: Rc<dyn Fn() -> bool> = Rc::new(move || mode_for_gate.get() == Mode::Edit);
            text_sync::connect_buffer_emit_input(
                &source_buffer,
                input_cb.clone(),
                suppress_emit.clone(),
                gate,
            );
        }

        let source_scroll = gtk4::ScrolledWindow::new();
        source_scroll.set_child(Some(&source_view));
        source_scroll.set_vexpand(true);
        source_scroll.set_hexpand(true);
        stack.add_named(&source_scroll, Some("edit"));
        stack.set_visible_child_name("render");
        container.append(&stack);

        let widget = container.upcast::<gtk4::Widget>();

        // Lazy notebook engine — created on the first render that encounters
        // a notebook cell, dropped (and recreated) on render-mode re-entry
        // and reload so watch timers don't pile up across re-renders.
        let notebook_engine: Rc<RefCell<Option<Rc<NotebookEngine>>>> = Rc::new(RefCell::new(None));

        // Export PDF: pull current markdown source from buffer (edit
        // mode) or saved content cell (render mode) and run gtk's
        // PrintOperation in Export mode through markdown_export.
        {
            let parent_box: gtk4::Widget = export_pdf_btn.clone().upcast();
            let ct = content.clone();
            let m = mode.clone();
            let sbuf = source_buffer.clone();
            let suggested_source = document
                .file_path()
                .map(str::to_string)
                .unwrap_or_else(|| "document.md".to_string());
            export_pdf_btn.connect_clicked(move |_| {
                let source = if m.get() == Mode::Edit {
                    sbuf.text(&sbuf.start_iter(), &sbuf.end_iter(), false)
                        .to_string()
                } else {
                    ct.borrow().clone()
                };
                let parent = parent_box
                    .root()
                    .and_then(|r| r.downcast::<gtk4::Window>().ok());
                let suggested = crate::markdown_export::suggested_pdf_name(std::path::Path::new(
                    &suggested_source,
                ));
                if let Some(win) = parent.as_ref() {
                    crate::markdown_export::export_markdown_to_pdf(win, &source, &suggested);
                }
            });
        }

        // Render closure that wires the notebook hook via the shared
        // `render_with_engine` free function (also reused by the file-watch
        // timer and the public `reload` method, so cells survive every
        // re-render path consistently).
        let render_with_notebook: Rc<dyn Fn(&str)> = {
            let rv = render_view.clone();
            let nb_engine_holder = notebook_engine.clone();
            Rc::new(move |content: &str| {
                render_with_engine(&rv, &nb_engine_holder, content);
            })
        };

        // Load initial content
        let initial = document
            .load()
            .unwrap_or_else(|e| format!("Error loading {}: {}", file_path, e));
        *content.borrow_mut() = initial.clone();
        render_with_notebook(&initial);

        // Re-render on theme change so code blocks pick up the new palette.
        let theme_observer = {
            let rv = render_view.downgrade();
            let ct = Rc::downgrade(&content);
            let m = Rc::downgrade(&mode);
            let nb_engine = Rc::downgrade(&notebook_engine);
            crate::theme::register_theme_observer(Rc::new(move || {
                let (Some(rv), Some(ct), Some(m), Some(nb_engine)) =
                    (rv.upgrade(), ct.upgrade(), m.upgrade(), nb_engine.upgrade())
                else {
                    return;
                };
                if m.get() == Mode::Render {
                    let text = ct.borrow().clone();
                    render_with_engine(&rv, &nb_engine, &text);
                }
            }))
        };

        // ── Render button ────────────────────────────────────────────────
        {
            let ct = content.clone();
            let m = mode.clone();
            let fb = fmt_bar.clone();
            let mod_flag = modified.clone();
            let doc = document.clone();
            let sb = save_btn.clone();
            let st = stack.clone();
            let sbuf = source_buffer.clone();
            let ub = undo_btn.clone();
            let rb = redo_btn.clone();
            let r = render_with_notebook.clone();
            let nb_engine = notebook_engine.clone();
            render_btn.connect_toggled(move |btn| {
                if !btn.is_active() {
                    return;
                }
                if m.get() == Mode::Edit {
                    let text = sbuf
                        .text(&sbuf.start_iter(), &sbuf.end_iter(), false)
                        .to_string();
                    *ct.borrow_mut() = text.clone();
                    if mod_flag.get() {
                        if let Err(e) = doc.save(&text) {
                            tracing::warn!("markdown panel: save failed: {e}");
                        }
                        mod_flag.set(false);
                        sb.set_sensitive(false);
                    }
                }
                m.set(Mode::Render);
                fb.set_visible(false);
                ub.set_sensitive(false);
                rb.set_sensitive(false);
                st.set_visible_child_name("render");
                // Drop the previous engine so watch timers cancel and a
                // fresh set of cells is constructed against the new buffer.
                *nb_engine.borrow_mut() = None;
                r(&ct.borrow());
            });
        }

        // ── Edit button ──────────────────────────────────────────────────
        {
            let ct = content.clone();
            let m = mode.clone();
            let fb = fmt_bar.clone();
            let st = stack.clone();
            let sbuf = source_buffer.clone();
            let sv = source_view.clone();
            let ub = undo_btn.clone();
            let rb = redo_btn.clone();
            let suppress = suppress_emit.clone();
            edit_btn.connect_toggled(move |btn| {
                if !btn.is_active() {
                    return;
                }
                m.set(Mode::Edit);
                fb.set_visible(true);
                suppress.set(true);
                sbuf.set_text(&ct.borrow());
                suppress.set(false);
                ub.set_sensitive(sbuf.can_undo());
                rb.set_sensitive(sbuf.can_redo());
                st.set_visible_child_name("edit");
                sv.grab_focus();
            });
        }

        // ── Undo/Redo ────────────────────────────────────────────────────
        {
            let sbuf = source_buffer.clone();
            let ub2 = undo_btn.clone();
            let rb2 = redo_btn.clone();
            undo_btn.connect_clicked(move |_| {
                sbuf.undo();
                ub2.set_sensitive(sbuf.can_undo());
                rb2.set_sensitive(sbuf.can_redo());
            });
        }
        {
            let sbuf = source_buffer.clone();
            let ub2 = undo_btn.clone();
            let rb2 = redo_btn.clone();
            redo_btn.connect_clicked(move |_| {
                sbuf.redo();
                ub2.set_sensitive(sbuf.can_undo());
                rb2.set_sensitive(sbuf.can_redo());
            });
        }

        // ── Track undo/redo state ────────────────────────────────────────
        {
            let ub = undo_btn.clone();
            let m = mode.clone();
            source_buffer.connect_notify_local(Some("can-undo"), move |buf, _| {
                ub.set_sensitive(m.get() == Mode::Edit && buf.can_undo());
            });
        }
        {
            let rb = redo_btn.clone();
            let m = mode.clone();
            source_buffer.connect_notify_local(Some("can-redo"), move |buf, _| {
                rb.set_sensitive(m.get() == Mode::Edit && buf.can_redo());
            });
        }

        // ── Track save state (compare with saved content) ────────────────
        {
            let sb = save_btn.clone();
            let ct = content.clone();
            let m = mode.clone();
            let mod_flag = modified.clone();
            source_buffer.connect_changed(move |buf| {
                if m.get() != Mode::Edit {
                    return;
                }
                let current = buf
                    .text(&buf.start_iter(), &buf.end_iter(), false)
                    .to_string();
                let dirty = current != *ct.borrow();
                mod_flag.set(dirty);
                sb.set_sensitive(dirty);
            });
        }

        // ── Save button ─────────────────────────────────────────────────
        {
            let doc = document.clone();
            let ct = content.clone();
            let sbuf = source_buffer.clone();
            let mod_flag = modified.clone();
            let sb2 = save_btn.clone();
            let save_current: Rc<dyn Fn()> = Rc::new(move || {
                let text = sbuf
                    .text(&sbuf.start_iter(), &sbuf.end_iter(), false)
                    .to_string();
                *ct.borrow_mut() = text.clone();
                if let Err(e) = doc.save(&text) {
                    tracing::warn!("markdown panel: save failed: {e}");
                }
                mod_flag.set(false);
                sb2.set_sensitive(false);
            });
            {
                let save_current = save_current.clone();
                save_btn.connect_clicked(move |_| save_current());
            }
            {
                let save_current = save_current.clone();
                let key_ctrl = gtk4::EventControllerKey::new();
                key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
                key_ctrl.connect_key_pressed(move |_, key, _, modifiers| {
                    if crate::shortcuts::has_primary(modifiers) && key == gtk4::gdk::Key::s {
                        save_current();
                        return gtk4::glib::Propagation::Stop;
                    }
                    gtk4::glib::Propagation::Proceed
                });
                source_view.add_controller(key_ctrl);
            }
        }

        // ── Reload button ────────────────────────────────────────────────
        {
            let doc = document.clone();
            let ct = content.clone();
            let sbuf = source_buffer.clone();
            let m = mode.clone();
            let mod_flag = modified.clone();
            let sb = save_btn.clone();
            let suppress = suppress_emit.clone();
            let r = render_with_notebook.clone();
            let nb_engine = notebook_engine.clone();
            reload_btn.connect_clicked(move |_| {
                if let Ok(text) = doc.load() {
                    *ct.borrow_mut() = text.clone();
                    mod_flag.set(false);
                    sb.set_sensitive(false);
                    if m.get() == Mode::Render {
                        // Drop the previous engine so watch timers cancel
                        // and a fresh set of cells is built from the
                        // reloaded content.
                        *nb_engine.borrow_mut() = None;
                        r(&text);
                    } else {
                        suppress.set(true);
                        sbuf.set_text(&text);
                        suppress.set(false);
                    }
                }
            });
        }

        // ── File watch (500ms): silent reload when clean, conflict bar when dirty ─
        if let Some(fp) = document.file_path().map(str::to_string) {
            let ct = content.clone();
            let rv = render_view.clone();
            let m = mode.clone();
            let mod_flag = modified.clone();
            let sbuf = source_buffer.clone();
            let sb = save_btn.clone();
            let suppress = suppress_emit.clone();
            let nb_engine = notebook_engine.clone();
            let bar_slot = conflict_bar_slot.clone();
            let watch_active = watch_active.clone();
            let last_mtime = Rc::new(Cell::new(get_mtime(&fp)));
            glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
                if !watch_active.get() {
                    return glib::ControlFlow::Break;
                }
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

        Self {
            widget,
            render_view,
            source_view,
            source_buffer,
            edit_btn,
            suppress_emit,
            input_cb,
            file_path: file_path.to_string(),
            document,
            content,
            modified,
            notebook_engine,
            theme_observer,
            watch_active,
        }
    }

    fn current_document_text(&self) -> String {
        if self.edit_btn.is_active() {
            self.source_buffer
                .text(
                    &self.source_buffer.start_iter(),
                    &self.source_buffer.end_iter(),
                    false,
                )
                .to_string()
        } else {
            self.content.borrow().clone()
        }
    }

    fn save_database_document(&self) {
        if !self.document.is_database() {
            return;
        }
        let text = self.current_document_text();
        *self.content.borrow_mut() = text.clone();
        if let Err(e) = self.document.save(&text) {
            tracing::warn!("markdown panel: failed to save DB document: {e}");
        } else {
            self.modified.set(false);
        }
    }

    fn database_document_has_content(&self) -> bool {
        if !self.document.is_database() {
            return false;
        }
        !self.current_document_text().is_empty() || self.document.content_len() > 0
    }

    pub fn reload(&mut self) {
        if let Ok(text) = self.document.load() {
            *self.content.borrow_mut() = text.clone();
            self.modified.set(false);
            *self.notebook_engine.borrow_mut() = None;
            render_with_engine(&self.render_view, &self.notebook_engine, &text);
        }
    }
}

/// Render `content` into `rv` while routing notebook cells through the
/// engine: each fenced block whose info string parses as a notebook spec
/// gets registered in (the lazily-created) engine, materialised as a
/// `NotebookCell` widget, and anchored at the renderer-provided
/// `TextChildAnchor`. Used by every render path (initial, theme observer,
/// mode toggle, reload, file watch) so cells survive consistently.
fn render_with_engine(
    rv: &gtk4::TextView,
    engine_holder: &Rc<RefCell<Option<Rc<NotebookEngine>>>>,
    content: &str,
) {
    let engine_holder = engine_holder.clone();
    let rv_for_hook = rv.clone();
    let mut hook = move |spec: &pax_core::notebook_tag::NotebookCellSpec,
                         body: &str,
                         anchor: &gtk4::TextChildAnchor| {
        let mut holder = engine_holder.borrow_mut();
        let engine = holder.get_or_insert_with(NotebookEngine::new).clone();
        drop(holder);
        let id = engine.register_cell(spec.clone(), body.to_string());
        let cell = NotebookCell::new(engine, id, &rv_for_hook);
        rv_for_hook.add_child_at_anchor(&cell.root, anchor);
    };
    crate::markdown_render::render_markdown_to_view_with_hook(rv, content, Some(&mut hook));
}

impl PanelBackend for MarkdownPanel {
    fn panel_type(&self) -> &str {
        "markdown"
    }
    fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }
    fn on_focus(&self) {
        self.source_view.grab_focus();
    }
    fn footer_text(&self) -> Option<String> {
        Some(self.file_path.clone())
    }

    fn shutdown(&self) {
        self.watch_active.set(false);
        crate::theme::unregister_theme_observer(self.theme_observer);
        *self.notebook_engine.borrow_mut() = None;
        self.save_database_document();
    }

    fn get_text_content(&self) -> Option<String> {
        Some(self.current_document_text())
    }

    fn close_confirmation(&self) -> Option<String> {
        if !self.database_document_has_content() {
            return None;
        }
        Some(
            "This Markdown panel contains an in-memory document stored in the workspace database. Closing it will delete that document permanently. Continue?"
                .to_string(),
        )
    }

    fn on_permanent_close(&self) {
        self.document.delete_persisted_state();
    }

    fn supports_sync(&self) -> bool {
        true
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn set_input_callback(&self, callback: Option<PanelInputCallback>) {
        *self.input_cb.borrow_mut() = callback;
    }

    fn write_input(&self, data: &[u8]) -> bool {
        if !self.edit_btn.is_active() {
            self.edit_btn.set_active(true);
        }
        text_sync::apply_input_to_buffer(&self.source_buffer, data, &self.suppress_emit);
        true
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn wrap_selection_buf(buf: &gtk4::TextBuffer, marker: &str) {
    if let Some((start, end)) = buf.selection_bounds() {
        let text = buf.text(&start, &end, false).to_string();
        buf.delete(&mut start.clone(), &mut end.clone());
        buf.insert(
            &mut buf.iter_at_offset(start.offset()),
            &format!("{}{}{}", marker, text, marker),
        );
    } else {
        buf.insert(
            &mut buf.iter_at_mark(&buf.get_insert()),
            &format!("{}text{}", marker, marker),
        );
    }
}

fn prepend_line_buf(buf: &gtk4::TextBuffer, prefix: &str) {
    let mut iter = buf.iter_at_mark(&buf.get_insert());
    iter.set_line_offset(0);
    buf.insert(&mut iter, prefix);
}

fn insert_at_cursor_buf(buf: &gtk4::TextBuffer, text: &str) {
    buf.insert(&mut buf.iter_at_mark(&buf.get_insert()), text);
}

fn get_mtime(path: &str) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        })
        .unwrap_or(0)
}

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
        // last_mtime was already advanced by the caller, so the bar won't
        // reappear until the file changes again.
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
