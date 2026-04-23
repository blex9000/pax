use gtk4::prelude::*;
use sourceview5::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::EditorState;

/// Extensions that dispatch to the Markdown viewer instead of the shared
/// source-code view.
const MARKDOWN_EXTS: &[&str] = &["md", "markdown"];

const NOTE_EDITOR_WIDTH_PX: i32 = 440;
const NOTE_EDITOR_HEIGHT_PX: i32 = 240;

/// GTK-standard symbolic icon used in the line-marks gutter for notes.
const NOTE_MARK_ICON: &str = "user-bookmarks-symbolic";
/// Amber background for the note mark in the gutter. Low alpha keeps the
/// line number readable behind it.
const NOTE_MARK_COLOR_R: f32 = 0.96;
const NOTE_MARK_COLOR_G: f32 = 0.78;
const NOTE_MARK_COLOR_B: f32 = 0.25;
const NOTE_MARK_COLOR_A: f32 = 0.25;
/// Priority for the note-mark category in the gutter renderer. Non-zero
/// so notes win over lower-priority marks if we add more categories.
const NOTE_MARK_PRIORITY: i32 = 10;

/// Resolve a path to the workspace-relative form when possible, else keep
/// it absolute. Used as the `file_path` key for metadata entries.
pub(crate) fn relative_file_path(root: &Path, absolute: &Path) -> String {
    absolute
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| absolute.to_string_lossy().into_owned())
}

/// Build the context-menu extras for the main source editor: the
/// existing format-current-file item plus Add/Edit/Delete Note items
/// scoped to the line the user right-clicked on.
fn build_editor_extras(
    view: &sourceview5::View,
    state: &Rc<RefCell<EditorState>>,
    notes_ruler: &Rc<super::notes_ruler::NotesRuler>,
    click_line: i32,
) -> Vec<super::text_context_menu::TextContextMenuItem> {
    let mut items: Vec<super::text_context_menu::TextContextMenuItem> = Vec::new();

    let (record_key, file_path_str, buffer, notes_state, path) = {
        let st = state.borrow();
        let Some(idx) = st.active_tab else {
            return items;
        };
        let Some(open_file) = st.open_files.get(idx) else {
            return items;
        };
        match &open_file.content {
            super::tab_content::TabContent::Source(source) => (
                st.record_key.clone(),
                relative_file_path(&st.root_dir, &open_file.path),
                source.buffer.clone(),
                source.notes.clone(),
                open_file.path.clone(),
            ),
            _ => return items,
        }
    };

    // Format item — only if a formatter is available for this extension.
    if let Ok(buffer_for_format) = view.buffer().downcast::<sourceview5::Buffer>() {
        if let Some(format_item) =
            super::text_context_menu::format_item_for(&path, &buffer_for_format)
        {
            items.push(format_item);
        }
    }

    items.extend(build_note_menu_items(
        view,
        &buffer,
        &notes_state,
        Some(notes_ruler),
        &record_key,
        &file_path_str,
        click_line,
    ));
    items
}

/// Build just the Add/Edit/Delete Note entries for a given buffer +
/// notes state. Shared between source tabs (which also gets the format
/// item above) and markdown tabs (which don't). `notes_ruler` is Some
/// for source tabs (so the side ruler refreshes after mutations) and
/// None for markdown tabs (no side ruler).
pub(crate) fn build_note_menu_items(
    view: &sourceview5::View,
    buffer: &sourceview5::Buffer,
    notes_state: &super::notes_state::NotesState,
    notes_ruler: Option<&Rc<super::notes_ruler::NotesRuler>>,
    record_key: &str,
    file_path_str: &str,
    click_line: i32,
) -> Vec<super::text_context_menu::TextContextMenuItem> {
    use super::text_context_menu::TextContextMenuItem;
    let mut items: Vec<TextContextMenuItem> = Vec::new();

    if record_key.is_empty() {
        return items;
    }

    let refresh_ruler = |buffer: &sourceview5::Buffer,
                        notes_state: &super::notes_state::NotesState,
                        ruler: &Option<Rc<super::notes_ruler::NotesRuler>>| {
        if let Some(r) = ruler {
            let lines = notes_state.current_lines(buffer);
            r.update(lines, buffer.line_count());
        }
    };
    let _ = refresh_ruler; // used via inline closures below

    let notes_here = notes_state.notes_on_line(buffer, click_line);

    if let Some(existing) = notes_here.into_iter().next() {
        let id = existing.db_id;
        let existing_text = existing.text.clone();

        // Edit
        {
            let text_for_edit = existing_text.clone();
            let buffer = buffer.clone();
            let notes_state = notes_state.clone();
            let ruler = notes_ruler.cloned();
            let parent_widget = view.clone();
            items.push(TextContextMenuItem::button(
                "document-edit-symbolic",
                "Edit Note",
                None,
                move || {
                    let parent = parent_widget
                        .root()
                        .and_then(|r| r.downcast::<gtk4::Window>().ok());
                    let existing_text = text_for_edit.clone();
                    let buffer = buffer.clone();
                    let notes_state = notes_state.clone();
                    let ruler = ruler.clone();
                    show_note_editor(parent.as_ref(), "Edit note", &existing_text, move |text| {
                        let db_path = pax_db::Database::default_path();
                        if let Ok(db) = pax_db::Database::open(&db_path) {
                            let _ = db.update_note_text(id, &text);
                        }
                        notes_state.set_text(id, &text);
                        if let Some(r) = &ruler {
                            let lines = notes_state.current_lines(&buffer);
                            r.update(lines, buffer.line_count());
                        }
                    });
                },
            ));
        }

        // Delete
        {
            let buffer = buffer.clone();
            let notes_state = notes_state.clone();
            let ruler = notes_ruler.cloned();
            items.push(TextContextMenuItem::button(
                "user-trash-symbolic",
                "Delete Note",
                None,
                move || {
                    let db_path = pax_db::Database::default_path();
                    if let Ok(db) = pax_db::Database::open(&db_path) {
                        let _ = db.delete_metadata_entry(id);
                    }
                    notes_state.remove(id, &buffer);
                    if let Some(r) = &ruler {
                        let lines = notes_state.current_lines(&buffer);
                        r.update(lines, buffer.line_count());
                    }
                },
            ));
        }
    } else {
        // Add Note.
        let record_key = record_key.to_string();
        let file_path_str = file_path_str.to_string();
        let buffer = buffer.clone();
        let notes_state = notes_state.clone();
        let ruler = notes_ruler.cloned();
        let parent_widget = view.clone();
        items.push(TextContextMenuItem::button(
            "document-new-symbolic",
            "Add Note",
            None,
            move || {
                let anchor = super::notes_state::line_content(&buffer, click_line);
                let parent = parent_widget
                    .root()
                    .and_then(|r| r.downcast::<gtk4::Window>().ok());
                let record_key = record_key.clone();
                let file_path_str = file_path_str.clone();
                let buffer = buffer.clone();
                let notes_state = notes_state.clone();
                let ruler = ruler.clone();
                show_note_editor(parent.as_ref(), "Add note", "", move |text| {
                    let db_path = pax_db::Database::default_path();
                    let Ok(db) = pax_db::Database::open(&db_path) else {
                        return;
                    };
                    let Ok(note) = db.add_note(
                        &record_key,
                        &file_path_str,
                        click_line,
                        Some(&anchor),
                        &text,
                    ) else {
                        return;
                    };
                    let live = super::notes_state::LiveNote {
                        db_id: note.id,
                        text: note.text,
                        saved_line: note.line_number,
                        saved_anchor: note.line_anchor,
                        mark: Some(super::notes_state::create_mark_at_line(&buffer, click_line)),
                    };
                    notes_state.push(live);
                    if let Some(r) = &ruler {
                        let lines = notes_state.current_lines(&buffer);
                        r.update(lines, buffer.line_count());
                    }
                });
            },
        ));
    }

    items
}

/// Wire context-menu Add/Edit/Delete Note, hover tooltip, and async
/// notes load on a markdown tab's internal source view. Mirrors the
/// equivalent source-tab setup but is scoped to this tab's buffer +
/// NotesState rather than going through the shared source view.
fn install_markdown_notes(
    tabs: &EditorTabs,
    state: &Rc<RefCell<EditorState>>,
    md: &super::tab_content::MarkdownTab,
    path: &Path,
    tab_id: u64,
) {
    // Context menu extras on the markdown source scroll + view.
    {
        let state_c = state.clone();
        super::text_context_menu::install(
            &md.source_scroll,
            &md.source_view,
            true,
            move |click_line| {
                let (record_key, file_path_str, buffer, notes_state, ruler) = {
                    let st = state_c.borrow();
                    let Some(idx) = st.active_tab else { return Vec::new() };
                    let Some(open_file) = st.open_files.get(idx) else {
                        return Vec::new();
                    };
                    let super::tab_content::TabContent::Markdown(m) = &open_file.content
                    else {
                        return Vec::new();
                    };
                    (
                        st.record_key.clone(),
                        relative_file_path(&st.root_dir, &open_file.path),
                        m.buffer.clone(),
                        m.notes.clone(),
                        m.notes_ruler.clone(),
                    )
                };
                build_note_menu_items(
                    &md_source_view_for_closure(&state_c),
                    &buffer,
                    &notes_state,
                    Some(&ruler),
                    &record_key,
                    &file_path_str,
                    click_line,
                )
            },
        );
    }

    // Hover tooltip: surface the note text when hovering a line that
    // owns a note, same as the source tab's tooltip.
    {
        let state_c = state.clone();
        md.source_view.set_has_tooltip(true);
        md.source_view
            .connect_query_tooltip(move |view, _x, y, _keyboard, tooltip| {
                let (_, buf_y) =
                    view.window_to_buffer_coords(gtk4::TextWindowType::Widget, 0, y);
                let (iter, _) = view.line_at_y(buf_y);
                let line = iter.line();
                let st = state_c.borrow();
                let Some(idx) = st.active_tab else { return false };
                let Some(open_file) = st.open_files.get(idx) else { return false };
                let super::tab_content::TabContent::Markdown(m) = &open_file.content
                else {
                    return false;
                };
                let Some(note) =
                    m.notes.notes_on_line(&m.buffer, line).into_iter().next()
                else {
                    return false;
                };
                tooltip.set_text(Some(&note.text));
                true
            });
    }

    // Async DB load of notes for this file.
    let record_key = state.borrow().record_key.clone();
    if !record_key.is_empty() {
        let fp = relative_file_path(&state.borrow().root_dir, path);
        let state_c = state.clone();
        super::task::run_blocking(
            move || {
                let db = pax_db::Database::open(&pax_db::Database::default_path()).ok()?;
                db.list_notes_for_file(&record_key, &fp).ok()
            },
            move |maybe_notes| {
                let Some(notes) = maybe_notes else { return };
                let st = state_c.borrow();
                let Some(open_file) =
                    st.open_files.iter().find(|f| f.tab_id == tab_id)
                else {
                    return;
                };
                let super::tab_content::TabContent::Markdown(m) = &open_file.content
                else {
                    return;
                };
                super::notes_state::apply_loaded_notes(&m.notes, &m.buffer, notes);
                let lines = m.notes.current_lines(&m.buffer);
                m.notes_ruler.update(lines, m.buffer.line_count());
            },
        );
    }
    let _ = tabs; // reserved for future integrations (e.g. focus the ruler on tab switch)
}

/// Lookup helper — returns a clone of the source_view inside the
/// currently active markdown tab so closures that need a view parent
/// (for tooltip / dialog parenting) can reach it without hoarding the
/// outer `md` by value.
fn md_source_view_for_closure(
    state: &Rc<RefCell<EditorState>>,
) -> sourceview5::View {
    let st = state.borrow();
    if let Some(idx) = st.active_tab {
        if let Some(open_file) = st.open_files.get(idx) {
            if let super::tab_content::TabContent::Markdown(m) = &open_file.content {
                return m.source_view.clone();
            }
        }
    }
    // Fallback: a fresh detached view. Only used if the lookup races
    // with a tab close — callbacks that route through this view will
    // no-op gracefully since it's not in any window.
    sourceview5::View::new()
}

/// Modal dialog for editing a note's text (Add and Edit share this).
/// Calls `on_save(text)` with the new text when the user clicks Save.
pub(crate) fn show_note_editor(
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

/// Monotonic counter producing a fresh `tab_id` per opened file. Stable IDs
/// let long-lived per-tab closures survive a rename of the underlying path.
static NEXT_TAB_ID: AtomicU64 = AtomicU64::new(1);

fn alloc_tab_id() -> u64 {
    NEXT_TAB_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextClipboardAction {
    Copy,
    Cut,
    Paste,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextHistoryAction {
    Undo,
    Redo,
}

fn text_clipboard_action(
    key: gtk4::gdk::Key,
    modifiers: gtk4::gdk::ModifierType,
) -> Option<TextClipboardAction> {
    if !crate::shortcuts::has_primary(modifiers) {
        return None;
    }

    match key {
        gtk4::gdk::Key::c | gtk4::gdk::Key::C => Some(TextClipboardAction::Copy),
        gtk4::gdk::Key::x | gtk4::gdk::Key::X => Some(TextClipboardAction::Cut),
        gtk4::gdk::Key::v | gtk4::gdk::Key::V => Some(TextClipboardAction::Paste),
        _ => None,
    }
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

use super::text_context_menu;

fn install_text_clipboard_shortcuts<W: IsA<gtk4::Widget>>(widget: &W) {
    let widget = widget.as_ref().clone();
    let widget_for_action = widget.clone();
    let key_ctrl = gtk4::EventControllerKey::new();
    key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
    key_ctrl.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(action) = text_clipboard_action(key, modifiers) else {
            return gtk4::glib::Propagation::Proceed;
        };

        let action_name = match action {
            TextClipboardAction::Copy => "clipboard.copy",
            TextClipboardAction::Cut => "clipboard.cut",
            TextClipboardAction::Paste => "clipboard.paste",
        };

        if widget_for_action
            .activate_action(action_name, None::<&gtk4::glib::Variant>)
            .is_ok()
        {
            gtk4::glib::Propagation::Stop
        } else {
            gtk4::glib::Propagation::Proceed
        }
    });
    widget.add_controller(key_ctrl);
}

fn install_text_history_shortcuts(view: &sourceview5::View) {
    // Look up the buffer at event time: the main editor SourceView swaps its
    // buffer every tab switch, so we must not capture the initial one.
    let view_c = view.clone();
    let key_ctrl = gtk4::EventControllerKey::new();
    key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
    key_ctrl.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(action) = text_history_action(key, modifiers) else {
            return gtk4::glib::Propagation::Proceed;
        };

        let Some(buffer) = view_c.buffer().downcast::<sourceview5::Buffer>().ok() else {
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
    view.add_controller(key_ctrl);
}

/// Manages the Notebook tabs and SourceView buffers.
/// The notebook is used ONLY as a tab bar — its page content is always empty.
/// Actual content (welcome message or source code) lives in `content_stack`.
pub struct EditorTabs {
    pub notebook: gtk4::Notebook,
    pub source_view: sourceview5::View,
    /// Buffer-word completion provider — every opened buffer is registered
    /// here so the popup can suggest words from any file currently open.
    completion_words: sourceview5::CompletionWords,
    /// Hidden buffer pre-loaded with the active language's keywords so they
    /// surface in the completion popup even before the user has typed them
    /// once. Re-populated whenever the visible buffer (and thus its
    /// language) changes.
    keyword_shadow_buffer: sourceview5::Buffer,
    /// Stack switching between "welcome" and "editor" content.
    pub content_stack: gtk4::Stack,
    /// Search/replace bar (hidden by default, toggled with Ctrl+F / Ctrl+H).
    pub search_bar: gtk4::Box,
    pub status_bar: gtk4::Box,
    pub info_bar_container: gtk4::Box,
    status_lang: gtk4::Label,
    status_pos: gtk4::Label,
    status_modified: gtk4::Label,
    pub search_entry: gtk4::SearchEntry,
    #[allow(dead_code)]
    pub replace_entry: gtk4::Entry,
    pub replace_row: gtk4::Box,
    #[allow(dead_code)]
    search_settings: sourceview5::SearchSettings,
    /// Line numbers (0-based) of search matches in the currently-active
    /// buffer. Repopulated on tab switch, in-file search change, or
    /// project-search result click. Drives the gold overview ruler.
    match_lines: Rc<RefCell<Vec<i32>>>,
    /// The last non-empty search query entered in either the in-file search
    /// bar or the project-wide search; used to recompute match_lines when
    /// the user switches tabs so the ruler stays in sync with the new buffer.
    last_search_query: Rc<RefCell<String>>,
    /// Drawing area beside the editor that paints a gold mark at every line
    /// in match_lines and scrolls to the nearest match on click.
    match_ruler: gtk4::DrawingArea,
    /// Amber note markers to the left of the source view. Populated from
    /// the active source tab's NotesState via `refresh_notes_ruler`.
    pub notes_ruler: Rc<super::notes_ruler::NotesRuler>,
}

impl EditorTabs {
    pub fn new(state: Rc<RefCell<EditorState>>) -> Self {
        let notebook = gtk4::Notebook::new();
        notebook.set_show_border(false);
        notebook.set_scrollable(true);
        notebook.add_css_class("editor-tabs");
        notebook.set_show_tabs(false);
        // Hide the notebook page content area — we only want the tab bar
        notebook.set_vexpand(false);

        // Single SourceView that switches buffers
        let source_view = sourceview5::View::new();
        source_view.add_css_class("editor-code-view");
        source_view.set_show_line_numbers(true);
        source_view.set_show_line_marks(true);
        source_view.set_highlight_current_line(true);

        // Register mark attributes for notes so the gutter paints an amber
        // bookmark icon next to any line that owns a note.
        {
            let attrs = sourceview5::MarkAttributes::new();
            attrs.set_icon_name(NOTE_MARK_ICON);
            let color = gtk4::gdk::RGBA::new(
                NOTE_MARK_COLOR_R,
                NOTE_MARK_COLOR_G,
                NOTE_MARK_COLOR_B,
                NOTE_MARK_COLOR_A,
            );
            attrs.set_background(&color);
            source_view.set_mark_attributes(
                super::notes_state::NOTE_MARK_CATEGORY,
                &attrs,
                NOTE_MARK_PRIORITY,
            );
        }
        source_view.set_auto_indent(true);
        source_view.set_tab_width(4);
        source_view.set_wrap_mode(gtk4::WrapMode::None);
        source_view.set_left_margin(6);
        source_view.set_top_margin(3);
        source_view.set_monospace(true);
        source_view.set_show_right_margin(true);
        source_view.set_right_margin_position(120);
        install_text_clipboard_shortcuts(&source_view);
        install_text_history_shortcuts(&source_view);

        // Apply and register for theme updates
        if let Some(buf) = source_view.buffer().downcast_ref::<sourceview5::Buffer>() {
            crate::theme::register_sourceview_buffer(buf);
        }

        // Hover tooltip: when the user hovers over any line in the active
        // source tab that has a note, show the note text as a tooltip.
        // Scoped to the line so both gutter-icon and text-area hovers
        // surface the same preview.
        {
            let state_c = state.clone();
            source_view.set_has_tooltip(true);
            source_view.connect_query_tooltip(move |view, _x, y, _keyboard, tooltip| {
                let (_, buf_y) = view.window_to_buffer_coords(
                    gtk4::TextWindowType::Widget,
                    0,
                    y,
                );
                let (iter, _) = view.line_at_y(buf_y);
                let line = iter.line();
                let st = state_c.borrow();
                let Some(idx) = st.active_tab else { return false };
                let Some(open_file) = st.open_files.get(idx) else { return false };
                let super::tab_content::TabContent::Source(source) = &open_file.content
                else {
                    return false;
                };
                let notes = source.notes.notes_on_line(&source.buffer, line);
                let Some(note) = notes.into_iter().next() else {
                    return false;
                };
                tooltip.set_text(Some(&note.text));
                true
            });
        }

        // NOTE: line-mark-activated wiring lives below, next to the
        // `notes_ruler` creation — it needs the ruler to refresh after
        // Edit/Delete actions in the popover.

        // Buffer-word autocompletion. The provider scans every registered
        // buffer for words and offers them as proposals as the user types.
        // 3 chars minimum to avoid noise from very short prefixes; the popup
        // also shows icons for proposal types.
        const COMPLETION_MIN_WORD_LEN: u32 = 3;
        let completion_words = sourceview5::CompletionWords::builder()
            .title("Words")
            .minimum_word_size(COMPLETION_MIN_WORD_LEN)
            .build();
        let completion = source_view.completion();
        completion.set_show_icons(true);
        completion.set_select_on_show(true);
        completion.add_provider(&completion_words);

        // Shadow buffer fed with language keywords so they appear in the
        // popup even before the user has typed them. Always registered with
        // the same provider; its contents are swapped by switch_to_buffer.
        let keyword_shadow_buffer = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
        completion_words.register(&keyword_shadow_buffer);

        // Auto-pair brackets/quotes (and wrap selections).
        install_bracket_auto_pair(&source_view);

        let source_scroll = gtk4::ScrolledWindow::new();
        source_scroll.set_child(Some(&source_view));
        source_scroll.set_vexpand(true);
        source_scroll.set_hexpand(true);

        // Search-match overview ruler: thin gold strip to the right of the
        // editor showing a marker at every line that matches the current
        // search query. Hidden until a non-empty query is active.
        let match_lines: Rc<RefCell<Vec<i32>>> = Rc::new(RefCell::new(Vec::new()));
        let last_search_query: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
        let match_ruler = build_match_overview_ruler(
            match_lines.clone(),
            OverviewRulerKind::Match,
            source_view.clone(),
        );
        match_ruler.set_visible(false);

        let notes_ruler = Rc::new(super::notes_ruler::NotesRuler::new(source_view.clone()));
        notes_ruler.widget.set_visible(false);
        // Tooltip callback: look up the note text for a line in the active
        // source tab so hovering the ruler dot reveals the note preview.
        {
            let state_c = state.clone();
            notes_ruler.set_tooltip_callback(move |line| {
                let st = state_c.borrow();
                let idx = st.active_tab?;
                let open_file = st.open_files.get(idx)?;
                let super::tab_content::TabContent::Source(source) = &open_file.content
                else {
                    return None;
                };
                source
                    .notes
                    .notes_on_line(&source.buffer, line)
                    .into_iter()
                    .next()
                    .map(|n| n.text)
            });
        }

        let editor_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        editor_row.set_vexpand(true);
        editor_row.set_hexpand(true);
        editor_row.append(&source_scroll);
        editor_row.append(&notes_ruler.widget);
        editor_row.append(&match_ruler);

        // Content stack. Welcome and editor children are added here; per-tab
        // Markdown/Image children are inserted in open_*_file on demand.
        // Created up front so closures (e.g. the switch-page handler) can
        // clone it before the welcome/editor children are wired.
        let content_stack = gtk4::Stack::new();
        content_stack.set_vexpand(true);
        content_stack.set_hexpand(true);

        // Right-click context menu on the main editor — extras factory looks
        // up the active file each time so the format action follows the
        // currently-open file's extension, plus Add/Edit/Delete Note on the
        // clicked line.
        {
            let view_for_menu = source_view.clone();
            let state_for_menu = state.clone();
            let notes_ruler_for_menu = notes_ruler.clone();
            text_context_menu::install(&source_scroll, &source_view, true, move |click_line| {
                build_editor_extras(
                    &view_for_menu,
                    &state_for_menu,
                    &notes_ruler_for_menu,
                    click_line,
                )
            });
        }

        // InfoBar container (for file-changed-on-disk warnings)
        let info_bar_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Status bar
        let status_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        status_bar.add_css_class("panel-footer-bar");
        status_bar.add_css_class("panel-footer");
        status_bar.add_css_class("editor-file-preview-footer");

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

        // Cursor-position tracking happens per-buffer inside open_file,
        // attached to each newly-opened file's buffer. (SourceView swaps
        // its buffer on every tab switch, so a listener on the initial
        // default buffer would never fire once a file is open.)

        // Switch page: update SourceView buffer and status bar when tab changes.
        // Uses try_borrow_mut to avoid panic when triggered by remove_page/set_current_page
        // while another closure already holds a borrow.
        {
            let state_c = state.clone();
            let sv = source_view.clone();
            let lang_l = status_lang.clone();
            let mod_l = status_modified.clone();
            let ml = match_lines.clone();
            let mr = match_ruler.clone();
            let lsq = last_search_query.clone();
            let cs = content_stack.clone();
            let nr = notes_ruler.clone();
            notebook.connect_switch_page(move |_nb, _page, page_num| {
                let idx = page_num as usize;
                // Resolve child + buffer under an immutable borrow so the
                // content_stack visibility is always applied even when
                // try_borrow_mut loses the race (which previously left the
                // stack showing a stale child — e.g. after a Ctrl+F navigation
                // followed by a tab click the new tab would appear blank).
                let (child_opt, buf_opt) = {
                    let st = state_c.borrow();
                    match st.open_files.get(idx) {
                        Some(f) => (
                            Some(f.content.content_stack_child_name(f.tab_id)),
                            f.source_buffer().cloned(),
                        ),
                        None => (None, None),
                    }
                };
                if let Some(ref child) = child_opt {
                    cs.set_visible_child_name(child);
                }
                if let Some(ref buf) = buf_opt {
                    sv.set_buffer(Some(buf));
                    // Scroll the view to the incoming buffer's cursor so the
                    // user returns to where they were on that tab. Without
                    // this, the view keeps its previous pixel scroll position
                    // across the buffer swap — which can leave the user
                    // looking at blank space past the end of the new buffer,
                    // especially after a Ctrl+F next/prev had scrolled the
                    // old buffer deep into the file.
                    let insert = buf.get_insert();
                    sv.scroll_to_mark(&insert, 0.1, true, 0.0, 0.3);
                }

                if let Ok(mut st) = state_c.try_borrow_mut() {
                    if let Some(open_file) = st.open_files.get(idx) {
                        // Only source tabs participate in the shared source
                        // view, match ruler, and language label. Non-source
                        // tabs own their own widget tree inside content_stack.
                        if let Some(buf) = open_file.source_buffer() {
                            let query = lsq.borrow().clone();
                            let lines = collect_match_lines(buf, &query);
                            let has = !lines.is_empty();
                            *ml.borrow_mut() = lines;
                            mr.set_visible(has);
                            mr.queue_draw();
                            if let Some(l) = buf.language() {
                                lang_l.set_text(&l.name());
                            } else {
                                lang_l.set_text("Plain Text");
                            }
                            // Notes ruler for source tabs.
                            if let super::tab_content::TabContent::Source(source) =
                                &open_file.content
                            {
                                let note_lines = source.notes.current_lines(&source.buffer);
                                nr.update(note_lines, source.buffer.line_count());
                            } else {
                                nr.clear();
                            }
                        } else {
                            ml.borrow_mut().clear();
                            mr.set_visible(false);
                            nr.clear();
                            lang_l.set_text(match &open_file.content {
                                super::tab_content::TabContent::Markdown(_) => "Markdown",
                                super::tab_content::TabContent::Image(_) => "Image",
                                _ => "",
                            });
                        }
                        mod_l.set_text(if open_file.modified() {
                            "\u{25CF} Modified"
                        } else {
                            ""
                        });
                    }
                    st.active_tab = Some(idx);
                }
                super::fire_nav_state_changed(&state_c);
            });
        }

        // ── Search/Replace bar (hidden by default) ──────────────────
        let search_bar = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        search_bar.set_margin_start(4);
        search_bar.set_margin_end(4);
        search_bar.set_margin_top(2);
        search_bar.set_margin_bottom(2);
        search_bar.set_visible(false);

        // Search row: [entry] [prev] [next] [count] [close]
        let search_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_hexpand(true);
        search_entry.set_placeholder_text(Some("Search..."));
        search_row.append(&search_entry);

        let prev_btn = gtk4::Button::from_icon_name("go-up-symbolic");
        prev_btn.add_css_class("flat");
        prev_btn.set_tooltip_text(Some("Previous match (Shift+Enter)"));
        search_row.append(&prev_btn);

        let next_btn = gtk4::Button::from_icon_name("go-down-symbolic");
        next_btn.add_css_class("flat");
        next_btn.set_tooltip_text(Some("Next match (Enter)"));
        search_row.append(&next_btn);

        let match_count_label = gtk4::Label::new(None);
        match_count_label.add_css_class("dim-label");
        match_count_label.add_css_class("caption");
        match_count_label.set_width_chars(10);
        search_row.append(&match_count_label);

        let case_btn = gtk4::ToggleButton::new();
        case_btn.set_icon_name("format-text-uppercase-symbolic");
        case_btn.add_css_class("flat");
        case_btn.set_tooltip_text(Some("Case sensitive"));
        search_row.append(&case_btn);

        let close_search_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_search_btn.add_css_class("flat");
        search_row.append(&close_search_btn);

        search_bar.append(&search_row);

        // Replace row (hidden until Ctrl+H)
        let replace_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        replace_row.set_visible(false);

        let replace_entry = gtk4::Entry::new();
        replace_entry.set_hexpand(true);
        replace_entry.set_placeholder_text(Some("Replace..."));
        replace_row.append(&replace_entry);

        let replace_btn = gtk4::Button::from_icon_name("edit-find-replace-symbolic");
        replace_btn.add_css_class("flat");
        replace_btn.set_tooltip_text(Some("Replace"));
        replace_row.append(&replace_btn);

        let replace_all_btn = gtk4::Button::with_label("All");
        replace_all_btn.add_css_class("flat");
        replace_all_btn.set_tooltip_text(Some("Replace all"));
        replace_row.append(&replace_all_btn);

        search_bar.append(&replace_row);

        // Search settings (shared, SearchContext is created per-buffer)
        let search_settings = sourceview5::SearchSettings::new();
        search_settings.set_wrap_around(true);

        // Helper: get or create SearchContext for the current SourceView buffer
        let active_ctx: Rc<RefCell<Option<sourceview5::SearchContext>>> =
            Rc::new(RefCell::new(None));
        let ensure_ctx = {
            let sv = source_view.clone();
            let ss = search_settings.clone();
            let ctx_cell = active_ctx.clone();
            move || -> sourceview5::SearchContext {
                let buf = sv.buffer().downcast::<sourceview5::Buffer>().unwrap();
                let mut cell = ctx_cell.borrow_mut();
                // Recreate if buffer changed
                let needs_new = cell.as_ref().map(|c| c.buffer() != buf).unwrap_or(true);
                if needs_new {
                    let ctx = sourceview5::SearchContext::new(&buf, Some(&ss));
                    ctx.set_highlight(true);
                    *cell = Some(ctx);
                }
                cell.as_ref().unwrap().clone()
            }
        };

        // Wire search entry
        {
            let get_ctx = ensure_ctx.clone();
            let count_l = match_count_label.clone();
            let sv = source_view.clone();
            let ml = match_lines.clone();
            let mr = match_ruler.clone();
            let lsq = last_search_query.clone();
            search_entry.connect_search_changed(move |entry| {
                let text = entry.text().to_string();
                let ctx = get_ctx();
                let settings = ctx.settings();
                settings.set_search_text(if text.is_empty() { None } else { Some(&text) });
                // Connect count update (re-connected each time, but GTK handles duplicates)
                let cl = count_l.clone();
                ctx.connect_notify_local(Some("occurrences-count"), move |ctx, _| {
                    let n = ctx.occurrences_count();
                    if n > 0 {
                        cl.set_text(&format!("{} found", n));
                    } else {
                        cl.set_text("No results");
                    }
                });
                let n = ctx.occurrences_count();
                if text.is_empty() {
                    count_l.set_text("");
                } else if n > 0 {
                    count_l.set_text(&format!("{} found", n));
                } else {
                    count_l.set_text("No results");
                }

                // Update the overview ruler. Any non-empty query becomes the
                // "last query" used to refresh the ruler on tab switches.
                *lsq.borrow_mut() = text.clone();
                if let Some(buf) = sv.buffer().downcast_ref::<sourceview5::Buffer>() {
                    let lines = collect_match_lines(buf, &text);
                    let has = !lines.is_empty();
                    *ml.borrow_mut() = lines;
                    mr.set_visible(has);
                    mr.queue_draw();
                }
            });
        }

        // Case sensitive toggle
        {
            let ss = search_settings.clone();
            case_btn.connect_toggled(move |btn| {
                ss.set_case_sensitive(btn.is_active());
            });
        }

        // Next match
        {
            let get_ctx = ensure_ctx.clone();
            let sv = source_view.clone();
            next_btn.connect_clicked(move |_| {
                let ctx = get_ctx();
                let buf = sv.buffer();
                let (_, end) = buf.selection_bounds().unwrap_or_else(|| {
                    let iter = buf.iter_at_offset(buf.cursor_position());
                    (iter.clone(), iter)
                });
                if let Some((sm, em, _)) = ctx.forward(&end) {
                    buf.select_range(&sm, &em);
                    sv.scroll_to_iter(&mut sm.clone(), 0.1, false, 0.0, 0.0);
                }
            });
        }

        // Previous match
        {
            let get_ctx = ensure_ctx.clone();
            let sv = source_view.clone();
            prev_btn.connect_clicked(move |_| {
                let ctx = get_ctx();
                let buf = sv.buffer();
                let (start, _) = buf.selection_bounds().unwrap_or_else(|| {
                    let iter = buf.iter_at_offset(buf.cursor_position());
                    (iter.clone(), iter)
                });
                if let Some((sm, em, _)) = ctx.backward(&start) {
                    buf.select_range(&sm, &em);
                    sv.scroll_to_iter(&mut sm.clone(), 0.1, false, 0.0, 0.0);
                }
            });
        }

        // Enter → next match (via SearchEntry's native activate signal,
        // which fires on Enter regardless of other key controllers). Shift+
        // Enter → previous, handled via a capture-phase key controller so it
        // runs before the SearchEntry swallows the key.
        {
            let get_ctx = ensure_ctx.clone();
            let sv = source_view.clone();
            search_entry.connect_activate(move |_| {
                let ctx = get_ctx();
                let buf = sv.buffer();
                let (_, end) = buf.selection_bounds().unwrap_or_else(|| {
                    let iter = buf.iter_at_offset(buf.cursor_position());
                    (iter.clone(), iter)
                });
                if let Some((sm, em, _)) = ctx.forward(&end) {
                    buf.select_range(&sm, &em);
                    sv.scroll_to_iter(&mut sm.clone(), 0.1, false, 0.0, 0.0);
                }
            });
        }
        {
            let get_ctx = ensure_ctx.clone();
            let sv = source_view.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
            key_ctrl.connect_key_pressed(move |_, key, _, modifier| {
                let is_enter = key == gtk4::gdk::Key::Return || key == gtk4::gdk::Key::KP_Enter;
                let shift = modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
                if !(is_enter && shift) {
                    return gtk4::glib::Propagation::Proceed;
                }
                let ctx = get_ctx();
                let buf = sv.buffer();
                let (start, _) = buf.selection_bounds().unwrap_or_else(|| {
                    let iter = buf.iter_at_offset(buf.cursor_position());
                    (iter.clone(), iter)
                });
                if let Some((sm, em, _)) = ctx.backward(&start) {
                    buf.select_range(&sm, &em);
                    sv.scroll_to_iter(&mut sm.clone(), 0.1, false, 0.0, 0.0);
                }
                gtk4::glib::Propagation::Stop
            });
            search_entry.add_controller(key_ctrl);
        }

        // Escape → close
        {
            let sb = search_bar.clone();
            let ss = search_settings.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    sb.set_visible(false);
                    ss.set_search_text(None::<&str>);
                    return gtk4::glib::Propagation::Stop;
                }
                gtk4::glib::Propagation::Proceed
            });
            search_entry.add_controller(key_ctrl);
        }
        {
            let sb = search_bar.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    sb.set_visible(false);
                    return gtk4::glib::Propagation::Stop;
                }
                gtk4::glib::Propagation::Proceed
            });
            replace_entry.add_controller(key_ctrl);
        }

        // Close button
        {
            let sb = search_bar.clone();
            let ss = search_settings.clone();
            close_search_btn.connect_clicked(move |_| {
                sb.set_visible(false);
                ss.set_search_text(None::<&str>);
            });
        }

        // Replace current
        {
            let get_ctx = ensure_ctx.clone();
            let sv = source_view.clone();
            let re = replace_entry.clone();
            replace_btn.connect_clicked(move |_| {
                let ctx = get_ctx();
                let replace_text = re.text().to_string();
                let buf = sv.buffer();
                if let Some((start, end)) = buf.selection_bounds() {
                    let _ = ctx.replace(&mut start.clone(), &mut end.clone(), &replace_text);
                    let cursor = buf.iter_at_offset(buf.cursor_position());
                    if let Some((sm, em, _)) = ctx.forward(&cursor) {
                        buf.select_range(&sm, &em);
                        sv.scroll_to_iter(&mut sm.clone(), 0.1, false, 0.0, 0.0);
                    }
                }
            });
        }

        // Replace all
        {
            let get_ctx = ensure_ctx.clone();
            let re = replace_entry.clone();
            let count_l = match_count_label.clone();
            replace_all_btn.connect_clicked(move |_| {
                let ctx = get_ctx();
                let replace_text = re.text().to_string();
                match ctx.replace_all(&replace_text) {
                    Ok(()) => count_l.set_text("All replaced"),
                    Err(e) => count_l.set_text(&format!("Error: {}", e)),
                }
            });
        }

        // ── Content stack: wire welcome + editor children ───────────
        let welcome_wrap = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        welcome_wrap.add_css_class("editor-welcome");
        welcome_wrap.set_vexpand(true);
        welcome_wrap.set_hexpand(true);
        let welcome = gtk4::Label::new(Some(
            "Open a file from the sidebar\nor press Ctrl+P to search",
        ));
        welcome.add_css_class("dim-label");
        welcome.set_vexpand(true);
        welcome.set_valign(gtk4::Align::Center);
        welcome_wrap.append(&welcome);
        content_stack.add_named(&welcome_wrap, Some("welcome"));
        content_stack.add_named(&editor_row, Some("editor"));
        content_stack.set_visible_child_name("welcome");

        Self {
            notebook,
            source_view,
            completion_words,
            keyword_shadow_buffer,
            content_stack,
            search_bar,
            status_bar,
            info_bar_container,
            status_lang,
            status_pos,
            status_modified,
            search_entry,
            replace_entry,
            replace_row,
            search_settings,
            match_lines,
            last_search_query,
            match_ruler,
            notes_ruler,
        }
    }

    /// Refresh the notes ruler from the active source tab's NotesState.
    /// Called after tab switch and after any note add/edit/delete.
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
        let super::tab_content::TabContent::Source(source) = &open_file.content else {
            self.notes_ruler.clear();
            return;
        };
        let lines = source.notes.current_lines(&source.buffer);
        let total = source.buffer.line_count();
        self.notes_ruler.update(lines, total);
    }

    /// Recompute the search-match overview ruler for the currently-active
    /// buffer against `query`. Call this from the project-wide search result
    /// click so the gold ruler shows up as soon as a file is opened from
    /// outside the in-file search bar. An empty `query` hides the ruler.
    pub fn update_match_ruler(&self, query: &str) {
        *self.last_search_query.borrow_mut() = query.to_string();
        if let Some(buf) = self.source_view.buffer().downcast_ref::<sourceview5::Buffer>() {
            let lines = collect_match_lines(buf, query);
            let has = !lines.is_empty();
            *self.match_lines.borrow_mut() = lines;
            self.match_ruler.set_visible(has);
            self.match_ruler.queue_draw();
        }
    }

    /// Open a file in a new tab. Returns the tab index.
    /// If the file is already open, switches to that tab.
    pub fn open_file(&self, path: &Path, state: &Rc<RefCell<EditorState>>) -> Option<usize> {
        // Push current position to navigation history before switching
        super::push_nav_position(state);

        // Update recent files immediately
        {
            let mut st = state.borrow_mut();
            let p = path.to_path_buf();
            st.recent_files.retain(|r| r != &p);
            st.recent_files.insert(0, p);
            if st.recent_files.len() > 10 {
                st.recent_files.truncate(10);
            }
        }
        super::fire_nav_state_changed(state);

        // Check if already open
        {
            let st = state.borrow();
            if let Some(idx) = st.open_files.iter().position(|f| f.path == path) {
                self.notebook.set_current_page(Some(idx as u32));
                self.switch_to_buffer(idx, state);
                return Some(idx);
            }
        }

        // Dispatch on extension: markdown files get a Rendered/Source viewer,
        // image files get a Picture-based viewer, everything else falls
        // through to the shared source-code path below.
        if let Some(ext) = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
        {
            if MARKDOWN_EXTS.contains(&ext.as_str()) {
                return self.open_markdown_file(path, state);
            }
            if super::image_view::IMAGE_EXTS.contains(&ext.as_str()) {
                return self.open_image_file(path, state);
            }
        }

        // Read file via backend
        let backend = state.borrow().backend.clone();
        let content = match backend.read_file(path) {
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

        // Detect language. GtkSourceView's mime/glob heuristics miss some
        // common files (e.g. .env, .envrc) — fall back to a hand-rolled map
        // so syntax highlighting and the language-aware comment toggle work.
        let lang_manager = sourceview5::LanguageManager::default();
        let lang = lang_manager
            .guess_language(Some(path), None::<&str>)
            .or_else(|| fallback_language_for(&lang_manager, path));
        if let Some(lang) = lang {
            buf.set_language(Some(&lang));
        }

        // Apply scheme and register for live theme updates
        crate::theme::register_sourceview_buffer(&buf);

        // Feed this buffer's words into the autocompletion provider.
        self.completion_words.register(&buf);

        // Drive the Ln/Col label from this buffer's cursor. The listener
        // set up on the SourceView's initial buffer in EditorTabs::new
        // doesn't survive the set_buffer swap, so every newly-opened file
        // needs its own notifier.
        {
            let pos_label = self.status_pos.clone();
            buf.connect_notify_local(Some("cursor-position"), move |b, _| {
                let iter = b.iter_at_offset(b.cursor_position());
                pos_label.set_text(&format!(
                    "Ln {}, Col {}",
                    iter.line() + 1,
                    iter.line_offset() + 1
                ));
            });
        }

        // Reset undo after setting initial text
        buf.set_enable_undo(false);
        buf.set_enable_undo(true);

        let mtime = get_mtime(path);
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "untitled".to_string());

        // Build tab label
        let tab_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        tab_box.add_css_class("editor-tab-label");
        let dot = gtk4::Label::new(None);
        dot.add_css_class("dirty-indicator");
        let label = gtk4::Label::new(Some(&file_name));
        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.add_css_class("tab-close-btn");
        tab_box.append(&dot);
        tab_box.append(&label);
        tab_box.append(&close_btn);

        // Empty placeholder widget for the notebook page (content is in content_stack)
        let page_widget = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        page_widget.set_height_request(0);
        let _page_idx = self.notebook.append_page(&page_widget, Some(&tab_box));
        self.notebook.set_show_tabs(true);
        self.content_stack.set_visible_child_name("editor");

        // Stable id for this tab — all long-lived closures key off it so that
        // a path change (rename) doesn't orphan dirty tracking or close button.
        let tab_id = alloc_tab_id();

        // Add to state
        let idx = {
            let mut st = state.borrow_mut();
            let saved_content = Rc::new(RefCell::new(content.clone()));
            st.open_files.push(super::OpenFile {
                tab_id,
                path: path.to_path_buf(),
                last_disk_mtime: mtime,
                name_label: label.clone(),
                content: super::tab_content::TabContent::Source(super::tab_content::SourceTab {
                    buffer: buf.clone(),
                    modified: false,
                    saved_content: saved_content.clone(),
                    notes: super::notes_state::NotesState::new(),
                }),
            });
            st.active_tab = Some(st.open_files.len() - 1);
            st.open_files.len() - 1
        };

        // Track dirty state
        {
            let state_c = state.clone();
            let dot_c = dot.clone();
            let mod_label = self.status_modified.clone();
            // Compare buffer content against saved content for accurate dirty detection
            let saved_for_changed = state.borrow().open_files[idx]
                .saved_content()
                .expect("source tab just pushed has saved_content")
                .clone();
            buf.connect_changed(move |buf| {
                let current = buf
                    .text(&buf.start_iter(), &buf.end_iter(), false)
                    .to_string();
                let is_dirty = current != *saved_for_changed.borrow();
                dot_c.set_text(if is_dirty { "\u{25CF} " } else { "" });
                mod_label.set_text(if is_dirty { "\u{25CF} Modified" } else { "" });
                if let Ok(mut st) = state_c.try_borrow_mut() {
                    if let Some(file_idx) =
                        st.open_files.iter().position(|f| f.tab_id == tab_id)
                    {
                        st.open_files[file_idx].set_modified(is_dirty);
                    }
                }
            });
        }

        // Close button
        {
            let state_c = state.clone();
            let nb = self.notebook.clone();
            let cs = self.content_stack.clone();
            let close_do_it = {
                let state_c = state_c.clone();
                let nb = nb.clone();
                let cs = cs.clone();
                Rc::new(move || {
                    let (empty_after, new_idx);
                    let per_tab_child = format!("tab-{}", tab_id);
                    {
                        let mut st = state_c.borrow_mut();
                        if let Some(idx) =
                            st.open_files.iter().position(|f| f.tab_id == tab_id)
                        {
                            st.open_files.remove(idx);
                            empty_after = st.open_files.is_empty();
                            new_idx = if empty_after {
                                0
                            } else {
                                idx.min(st.open_files.len() - 1)
                            };
                            if empty_after {
                                st.active_tab = None;
                            } else {
                                st.active_tab = Some(new_idx);
                            }
                            drop(st);
                            nb.remove_page(Some(idx as u32));
                            // Drop the per-tab content widget if this tab had one
                            // (Markdown / Image tabs). Source tabs share the
                            // "editor" child so there's nothing to remove.
                            if let Some(w) = cs.child_by_name(&per_tab_child) {
                                cs.remove(&w);
                            }
                            if empty_after {
                                nb.set_show_tabs(false);
                                cs.set_visible_child_name("welcome");
                            } else {
                                nb.set_current_page(Some(new_idx as u32));
                            }
                            super::fire_nav_state_changed(&state_c);
                        }
                    }
                })
            };
            close_btn.connect_clicked(move |btn| {
                let (is_modified, current_name) = {
                    let st = state_c.borrow();
                    let entry = st.open_files.iter().find(|f| f.tab_id == tab_id);
                    let modified = entry.map(|f| f.modified()).unwrap_or(false);
                    let name = entry
                        .and_then(|f| f.path.file_name().map(|n| n.to_string_lossy().to_string()))
                        .unwrap_or_else(|| "file".to_string());
                    (modified, name)
                };
                if is_modified {
                    // Show save/discard dialog
                    let dialog = gtk4::Window::builder()
                        .title("Unsaved Changes")
                        .modal(true)
                        .default_width(350)
                        .default_height(100)
                        .build();
                    if let Some(win) = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok()) {
                        dialog.set_transient_for(Some(&win));
                    }
                    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
                    vbox.set_margin_top(16);
                    vbox.set_margin_bottom(16);
                    vbox.set_margin_start(16);
                    vbox.set_margin_end(16);

                    let msg = gtk4::Label::new(Some(&format!(
                        "\"{}\" has unsaved changes.",
                        current_name
                    )));
                    vbox.append(&msg);

                    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                    btn_row.set_halign(gtk4::Align::End);

                    let save_btn = gtk4::Button::with_label("Save");
                    save_btn.add_css_class("suggested-action");
                    let discard_btn = gtk4::Button::with_label("Discard");
                    discard_btn.add_css_class("destructive-action");
                    let cancel_btn = gtk4::Button::with_label("Cancel");

                    btn_row.append(&cancel_btn);
                    btn_row.append(&discard_btn);
                    btn_row.append(&save_btn);
                    vbox.append(&btn_row);

                    // Cancel
                    {
                        let d = dialog.clone();
                        cancel_btn.connect_clicked(move |_| d.close());
                    }
                    // Discard — reset modified and close
                    {
                        let d = dialog.clone();
                        let sc = state_c.clone();
                        let close = close_do_it.clone();
                        discard_btn.connect_clicked(move |_| {
                            if let Ok(mut st) = sc.try_borrow_mut() {
                                if let Some(f) =
                                    st.open_files.iter_mut().find(|f| f.tab_id == tab_id)
                                {
                                    f.set_modified(false);
                                }
                            }
                            close();
                            d.close();
                        });
                    }
                    // Save then close
                    {
                        let d = dialog.clone();
                        let sc = state_c.clone();
                        let close = close_do_it.clone();
                        save_btn.connect_clicked(move |_| {
                            let save_result = {
                                let st = sc.borrow();
                                let backend = st.backend.clone();
                                if let Some(f) =
                                    st.open_files.iter().find(|f| f.tab_id == tab_id)
                                {
                                    if let Some(buf) = f.writable_buffer() {
                                        let text = buf
                                            .text(&buf.start_iter(), &buf.end_iter(), false)
                                            .to_string();
                                        backend.write_file(&f.path, &text).map(|_| text)
                                    } else {
                                        Err("Tab is read-only".to_string())
                                    }
                                } else {
                                    Err("File not found".to_string())
                                }
                            };
                            match save_result {
                                Ok(text) => {
                                    if let Ok(mut st) = sc.try_borrow_mut() {
                                        if let Some(f) =
                                            st.open_files.iter_mut().find(|f| f.tab_id == tab_id)
                                        {
                                            f.set_modified(false);
                                            f.last_disk_mtime = get_mtime(&f.path);
                                            if let Some(cell) = f.saved_content() {
                                                *cell.borrow_mut() = text;
                                            }
                                        }
                                    }
                                    close();
                                    d.close();
                                }
                                Err(_) => {
                                    d.close();
                                }
                            }
                        });
                    }

                    dialog.set_child(Some(&vbox));
                    dialog.present();
                    return;
                }
                close_do_it();
            });
        }

        // Middle-click to close tab
        {
            let close_btn = close_btn.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(2);
            gesture.connect_released(move |_, _, _, _| {
                close_btn.emit_clicked();
            });
            tab_box.add_controller(gesture);
        }

        // Async load of notes attached to this file in the DB. The open
        // doesn't block on it; when the query returns we resolve each
        // note's line via anchor match and paint the ruler.
        {
            let record_key = state.borrow().record_key.clone();
            let fp = relative_file_path(&state.borrow().root_dir, path);
            tracing::debug!(
                "notes: open_file record_key='{}' file_path='{}'",
                record_key,
                fp
            );
            if !record_key.is_empty() {
                let state_c = state.clone();
                let notes_ruler = self.notes_ruler.clone();
                let rk_for_log = record_key.clone();
                let fp_for_log = fp.clone();
                super::task::run_blocking(
                    move || {
                        let db = pax_db::Database::open(&pax_db::Database::default_path())
                            .ok()?;
                        db.list_notes_for_file(&record_key, &fp).ok()
                    },
                    move |maybe_notes| {
                        let notes = match maybe_notes {
                            Some(n) => n,
                            None => {
                                tracing::warn!(
                                    "notes: DB load failed for rk='{}' fp='{}'",
                                    rk_for_log,
                                    fp_for_log
                                );
                                return;
                            }
                        };
                        tracing::debug!(
                            "notes: loaded {} note(s) for rk='{}' fp='{}'",
                            notes.len(),
                            rk_for_log,
                            fp_for_log
                        );
                        let st = state_c.borrow();
                        let Some((current_idx, open_file)) = st
                            .open_files
                            .iter()
                            .enumerate()
                            .find(|(_, f)| f.tab_id == tab_id)
                        else {
                            return;
                        };
                        let super::tab_content::TabContent::Source(source) =
                            &open_file.content
                        else {
                            return;
                        };
                        super::notes_state::apply_loaded_notes(
                            &source.notes,
                            &source.buffer,
                            notes,
                        );
                        let is_active = st.active_tab == Some(current_idx);
                        let lines = source.notes.current_lines(&source.buffer);
                        let total = source.buffer.line_count();
                        tracing::debug!(
                            "notes: applied; {} resolved lines, is_active={}",
                            lines.len(),
                            is_active
                        );
                        drop(st);
                        if is_active {
                            notes_ruler.update(lines, total);
                        }
                    },
                );
            }
        }

        // Switch to this buffer
        self.switch_to_buffer(idx, state);
        self.notebook.set_current_page(Some(idx as u32));

        Some(idx)
    }

    /// Open a `.md` file in a Rendered/Source Markdown tab.
    fn open_markdown_file(
        &self,
        path: &Path,
        state: &Rc<RefCell<EditorState>>,
    ) -> Option<usize> {
        let backend = state.borrow().backend.clone();
        let content = match backend.read_file(path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Cannot open markdown {}: {}", path.display(), e);
                return None;
            }
        };

        let md = super::markdown_view::build_markdown_tab(&content);
        self.completion_words.register(&md.buffer);

        let tab_id = alloc_tab_id();
        let child_name = format!("tab-{}", tab_id);
        self.content_stack.add_named(&md.outer, Some(&child_name));
        self.content_stack.set_visible_child_name(&child_name);

        let mtime = get_mtime(path);
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "untitled".to_string());

        // Tab label with dirty dot + name + close button. Mirrors open_file's
        // tab-label layout so CSS and existing tab handling keep working.
        let tab_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        tab_box.add_css_class("editor-tab-label");
        let dot = gtk4::Label::new(None);
        dot.add_css_class("dirty-indicator");
        let label = gtk4::Label::new(Some(&file_name));
        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.add_css_class("tab-close-btn");
        tab_box.append(&dot);
        tab_box.append(&label);
        tab_box.append(&close_btn);

        let page_widget = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        page_widget.set_height_request(0);
        let _page_idx = self.notebook.append_page(&page_widget, Some(&tab_box));
        self.notebook.set_show_tabs(true);

        let md_state = md.clone();
        let idx = {
            let mut st = state.borrow_mut();
            st.open_files.push(super::OpenFile {
                tab_id,
                path: path.to_path_buf(),
                last_disk_mtime: mtime,
                name_label: label.clone(),
                content: super::tab_content::TabContent::Markdown(md_state),
            });
            st.active_tab = Some(st.open_files.len() - 1);
            st.open_files.len() - 1
        };

        // Dirty tracking on the markdown source buffer.
        {
            let state_c = state.clone();
            let dot_c = dot.clone();
            let mod_label = self.status_modified.clone();
            let saved = md.saved_content.clone();
            md.buffer.connect_changed(move |buf| {
                let current = buf
                    .text(&buf.start_iter(), &buf.end_iter(), false)
                    .to_string();
                let is_dirty = current != *saved.borrow();
                dot_c.set_text(if is_dirty { "\u{25CF} " } else { "" });
                mod_label.set_text(if is_dirty { "\u{25CF} Modified" } else { "" });
                if let Ok(mut st) = state_c.try_borrow_mut() {
                    if let Some(file_idx) =
                        st.open_files.iter().position(|f| f.tab_id == tab_id)
                    {
                        st.open_files[file_idx].set_modified(is_dirty);
                    }
                }
            });
        }

        // Close button — mirrors open_file's close path including the
        // unsaved-changes dialog. Per-tab stack child is removed by close_do_it.
        {
            let state_c = state.clone();
            let nb = self.notebook.clone();
            let cs = self.content_stack.clone();
            let close_do_it = {
                let state_c = state_c.clone();
                let nb = nb.clone();
                let cs = cs.clone();
                Rc::new(move || {
                    let (empty_after, new_idx);
                    let per_tab_child = format!("tab-{}", tab_id);
                    {
                        let mut st = state_c.borrow_mut();
                        if let Some(idx) =
                            st.open_files.iter().position(|f| f.tab_id == tab_id)
                        {
                            st.open_files.remove(idx);
                            empty_after = st.open_files.is_empty();
                            new_idx = if empty_after {
                                0
                            } else {
                                idx.min(st.open_files.len() - 1)
                            };
                            if empty_after {
                                st.active_tab = None;
                            } else {
                                st.active_tab = Some(new_idx);
                            }
                            drop(st);
                            nb.remove_page(Some(idx as u32));
                            if let Some(w) = cs.child_by_name(&per_tab_child) {
                                cs.remove(&w);
                            }
                            if empty_after {
                                nb.set_show_tabs(false);
                                cs.set_visible_child_name("welcome");
                            } else {
                                nb.set_current_page(Some(new_idx as u32));
                            }
                            super::fire_nav_state_changed(&state_c);
                        }
                    }
                })
            };
            close_btn.connect_clicked(move |btn| {
                let (is_modified, current_name) = {
                    let st = state_c.borrow();
                    let entry = st.open_files.iter().find(|f| f.tab_id == tab_id);
                    let modified = entry.map(|f| f.modified()).unwrap_or(false);
                    let name = entry
                        .and_then(|f| f.path.file_name().map(|n| n.to_string_lossy().to_string()))
                        .unwrap_or_else(|| "file".to_string());
                    (modified, name)
                };
                if is_modified {
                    let dialog = gtk4::Window::builder()
                        .title("Unsaved Changes")
                        .modal(true)
                        .default_width(350)
                        .default_height(100)
                        .build();
                    if let Some(win) = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok()) {
                        dialog.set_transient_for(Some(&win));
                    }
                    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
                    vbox.set_margin_top(16);
                    vbox.set_margin_bottom(16);
                    vbox.set_margin_start(16);
                    vbox.set_margin_end(16);
                    let msg = gtk4::Label::new(Some(&format!(
                        "\"{}\" has unsaved changes.",
                        current_name
                    )));
                    vbox.append(&msg);
                    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                    btn_row.set_halign(gtk4::Align::End);
                    let save_btn = gtk4::Button::with_label("Save");
                    save_btn.add_css_class("suggested-action");
                    let discard_btn = gtk4::Button::with_label("Discard");
                    discard_btn.add_css_class("destructive-action");
                    let cancel_btn = gtk4::Button::with_label("Cancel");
                    btn_row.append(&cancel_btn);
                    btn_row.append(&discard_btn);
                    btn_row.append(&save_btn);
                    vbox.append(&btn_row);
                    {
                        let d = dialog.clone();
                        cancel_btn.connect_clicked(move |_| d.close());
                    }
                    {
                        let d = dialog.clone();
                        let sc = state_c.clone();
                        let close = close_do_it.clone();
                        discard_btn.connect_clicked(move |_| {
                            if let Ok(mut st) = sc.try_borrow_mut() {
                                if let Some(f) =
                                    st.open_files.iter_mut().find(|f| f.tab_id == tab_id)
                                {
                                    f.set_modified(false);
                                }
                            }
                            close();
                            d.close();
                        });
                    }
                    {
                        let d = dialog.clone();
                        let sc = state_c.clone();
                        let close = close_do_it.clone();
                        save_btn.connect_clicked(move |_| {
                            let save_result = {
                                let st = sc.borrow();
                                let backend = st.backend.clone();
                                if let Some(f) =
                                    st.open_files.iter().find(|f| f.tab_id == tab_id)
                                {
                                    if let Some(buf) = f.writable_buffer() {
                                        let text = buf
                                            .text(&buf.start_iter(), &buf.end_iter(), false)
                                            .to_string();
                                        backend.write_file(&f.path, &text).map(|_| text)
                                    } else {
                                        Err("Tab is read-only".to_string())
                                    }
                                } else {
                                    Err("File not found".to_string())
                                }
                            };
                            match save_result {
                                Ok(text) => {
                                    if let Ok(mut st) = sc.try_borrow_mut() {
                                        if let Some(f) =
                                            st.open_files.iter_mut().find(|f| f.tab_id == tab_id)
                                        {
                                            f.set_modified(false);
                                            f.last_disk_mtime = get_mtime(&f.path);
                                            if let Some(cell) = f.saved_content() {
                                                *cell.borrow_mut() = text;
                                            }
                                        }
                                    }
                                    close();
                                    d.close();
                                }
                                Err(_) => {
                                    d.close();
                                }
                            }
                        });
                    }
                    dialog.set_child(Some(&vbox));
                    dialog.present();
                    return;
                }
                close_do_it();
            });
        }

        // Middle-click to close tab.
        {
            let close_btn = close_btn.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(2);
            gesture.connect_released(move |_, _, _, _| {
                close_btn.emit_clicked();
            });
            tab_box.add_controller(gesture);
        }

        // Notes wiring for the markdown tab's internal source view:
        // context-menu Add/Edit/Delete, hover tooltip, and async load.
        install_markdown_notes(self, state, &md, path, tab_id);

        self.switch_to_buffer(idx, state);
        self.notebook.set_current_page(Some(idx as u32));

        Some(idx)
    }

    /// Open an image file in an Image tab (metadata header + Picture + zoom).
    /// Remote (SSH) backends decline gracefully — first pass is local-only.
    fn open_image_file(
        &self,
        path: &Path,
        state: &Rc<RefCell<EditorState>>,
    ) -> Option<usize> {
        if state.borrow().backend.is_remote() {
            tracing::warn!(
                "Image preview is local-only; skipping remote image {}",
                path.display()
            );
            return None;
        }

        let img = super::image_view::build_image_tab(path);

        let tab_id = alloc_tab_id();
        let child_name = format!("tab-{}", tab_id);
        self.content_stack.add_named(&img.outer, Some(&child_name));
        self.content_stack.set_visible_child_name(&child_name);

        let mtime = get_mtime(path);
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "untitled".to_string());

        let tab_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        tab_box.add_css_class("editor-tab-label");
        let dot = gtk4::Label::new(None);
        dot.add_css_class("dirty-indicator");
        let label = gtk4::Label::new(Some(&file_name));
        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.add_css_class("tab-close-btn");
        tab_box.append(&dot);
        tab_box.append(&label);
        tab_box.append(&close_btn);

        let page_widget = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        page_widget.set_height_request(0);
        let _page_idx = self.notebook.append_page(&page_widget, Some(&tab_box));
        self.notebook.set_show_tabs(true);

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

        // Close button — image tabs are read-only so no unsaved-changes path.
        {
            let state_c = state.clone();
            let nb = self.notebook.clone();
            let cs = self.content_stack.clone();
            let close_do_it = Rc::new(move || {
                let per_tab_child = format!("tab-{}", tab_id);
                let (empty_after, new_idx);
                let mut st = state_c.borrow_mut();
                if let Some(idx) =
                    st.open_files.iter().position(|f| f.tab_id == tab_id)
                {
                    st.open_files.remove(idx);
                    empty_after = st.open_files.is_empty();
                    new_idx = if empty_after {
                        0
                    } else {
                        idx.min(st.open_files.len() - 1)
                    };
                    if empty_after {
                        st.active_tab = None;
                    } else {
                        st.active_tab = Some(new_idx);
                    }
                    drop(st);
                    nb.remove_page(Some(idx as u32));
                    if let Some(w) = cs.child_by_name(&per_tab_child) {
                        cs.remove(&w);
                    }
                    if empty_after {
                        nb.set_show_tabs(false);
                        cs.set_visible_child_name("welcome");
                    } else {
                        nb.set_current_page(Some(new_idx as u32));
                    }
                    super::fire_nav_state_changed(&state_c);
                }
            });
            close_btn.connect_clicked(move |_| close_do_it());
        }

        // Middle-click to close tab.
        {
            let close_btn = close_btn.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(2);
            gesture.connect_released(move |_, _, _, _| {
                close_btn.emit_clicked();
            });
            tab_box.add_controller(gesture);
        }

        self.switch_to_buffer(idx, state);
        self.notebook.set_current_page(Some(idx as u32));

        Some(idx)
    }

    /// Switch the SourceView to display the buffer at the given index.
    pub fn switch_to_buffer(&self, idx: usize, state: &Rc<RefCell<EditorState>>) {
        // Toggle active CSS class on editor tab labels
        let n = self.notebook.n_pages();
        for i in 0..n {
            if let Some(page) = self.notebook.nth_page(Some(i)) {
                if let Some(tab_label) = self.notebook.tab_label(&page) {
                    if i == idx as u32 {
                        tab_label.add_css_class("editor-tab-active");
                    } else {
                        tab_label.remove_css_class("editor-tab-active");
                    }
                }
            }
        }

        let st = state.borrow();
        if let Some(open_file) = st.open_files.get(idx) {
            let child = open_file.content.content_stack_child_name(open_file.tab_id);
            self.content_stack.set_visible_child_name(&child);
            if let Some(buf) = open_file.source_buffer() {
                self.source_view.set_buffer(Some(buf));
                let insert = buf.get_insert();
                self.source_view.scroll_to_mark(&insert, 0.1, true, 0.0, 0.3);
                let language = buf.language();
                if let Some(lang) = language.as_ref() {
                    self.status_lang.set_text(&lang.name());
                } else {
                    self.status_lang.set_text("Plain Text");
                }
                self.refresh_keyword_shadow(
                    language.as_ref().map(|l| l.id().to_string()).as_deref(),
                );
            } else {
                self.status_lang.set_text(match &open_file.content {
                    super::tab_content::TabContent::Markdown(_) => "Markdown",
                    super::tab_content::TabContent::Image(_) => "Image",
                    _ => "",
                });
            }
            self.status_modified.set_text(if open_file.modified() {
                "\u{25CF} Modified"
            } else {
                ""
            });
        }
        drop(st);
        self.refresh_notes_ruler(state);
    }

    /// Re-populate the shadow buffer with the keyword list for the active
    /// language so completion proposals reflect the current file's syntax.
    fn refresh_keyword_shadow(&self, lang_id: Option<&str>) {
        let text = match lang_id {
            // CompletionWords scans buffers for word boundaries; whitespace
            // separation is enough.
            Some(id) => keywords_for(id).join(" "),
            None => String::new(),
        };
        self.keyword_shadow_buffer.set_text(&text);
    }

    /// Show a side-by-side diff view for the given file.
    /// The diff replaces the content_stack view. Close button goes back to editor.
    pub fn show_diff(
        &self,
        root: &Path,
        file_path: &Path,
        backend: Arc<dyn super::file_backend::FileBackend>,
    ) {
        let rel = file_path.strip_prefix(root).unwrap_or(file_path);
        let old_content = backend
            .git_show(&format!("HEAD:{}", rel.to_string_lossy()))
            .unwrap_or_default();
        let new_content = backend.read_file(file_path).unwrap_or_default();

        let old_buf = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
        old_buf.set_text(&old_content);
        old_buf.set_highlight_syntax(true);
        let new_buf = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
        new_buf.set_text(&new_content);
        new_buf.set_highlight_syntax(true);

        let lang_manager = sourceview5::LanguageManager::default();
        if let Some(lang) = lang_manager.guess_language(Some(file_path), None::<&str>) {
            old_buf.set_language(Some(&lang));
            new_buf.set_language(Some(&lang));
        }
        crate::theme::register_sourceview_buffer(&old_buf);
        crate::theme::register_sourceview_buffer(&new_buf);

        // Highlight changed lines using similar. We also collect the per-side
        // line numbers of every delete/insert so the overview ruler can draw
        // clickable markers that jump to each change.
        let mut old_change_lines: Vec<i32> = Vec::new();
        let mut new_change_lines: Vec<i32> = Vec::new();
        {
            let diff = similar::TextDiff::from_lines(&old_content, &new_content);

            // Create tags for highlighting
            let ensure_diff_tags = |buf: &sourceview5::Buffer| {
                let tt = buf.tag_table();
                if tt.lookup("diff-del").is_none() {
                    let tag = gtk4::TextTag::new(Some("diff-del"));
                    tag.set_paragraph_background(Some("rgba(220, 50, 47, 0.25)"));
                    tt.add(&tag);
                }
                if tt.lookup("diff-add").is_none() {
                    let tag = gtk4::TextTag::new(Some("diff-add"));
                    tag.set_paragraph_background(Some("rgba(40, 180, 60, 0.25)"));
                    tt.add(&tag);
                }
            };
            ensure_diff_tags(&old_buf);
            ensure_diff_tags(&new_buf);

            let mut old_line = 0i32;
            let mut new_line = 0i32;
            for change in diff.iter_all_changes() {
                match change.tag() {
                    similar::ChangeTag::Equal => {
                        old_line += 1;
                        new_line += 1;
                    }
                    similar::ChangeTag::Delete => {
                        if let Some(start) = old_buf.iter_at_line(old_line) {
                            let mut end = start.clone();
                            end.forward_to_line_end();
                            // Include the newline
                            end.forward_char();
                            old_buf.apply_tag_by_name("diff-del", &start, &end);
                        }
                        old_change_lines.push(old_line);
                        old_line += 1;
                    }
                    similar::ChangeTag::Insert => {
                        if let Some(start) = new_buf.iter_at_line(new_line) {
                            let mut end = start.clone();
                            end.forward_to_line_end();
                            end.forward_char();
                            new_buf.apply_tag_by_name("diff-add", &start, &end);
                        }
                        new_change_lines.push(new_line);
                        new_line += 1;
                    }
                }
            }
        }

        let file_path_owned = file_path.to_path_buf();
        let make_sv =
            |buf: &sourceview5::Buffer,
             editable: bool|
             -> (sourceview5::View, gtk4::ScrolledWindow) {
                let view = sourceview5::View::with_buffer(buf);
                view.add_css_class("editor-code-view");
                view.set_editable(editable);
                view.set_show_line_numbers(true);
                view.set_monospace(true);
                view.set_left_margin(3);
                install_text_clipboard_shortcuts(&view);
                install_text_history_shortcuts(&view);
                if editable {
                    view.set_auto_indent(true);
                    view.set_tab_width(4);
                }
                let scroll = gtk4::ScrolledWindow::new();
                scroll.set_child(Some(&view));
                scroll.set_vexpand(true);
                scroll.set_hexpand(true);
                let file_path_factory = file_path_owned.clone();
                let buf_factory = buf.clone();
                text_context_menu::install(&scroll, &view, editable, move |_click_line| {
                    if !editable {
                        return Vec::new();
                    }
                    text_context_menu::format_item_for(&file_path_factory, &buf_factory)
                        .map(|i| vec![i])
                        .unwrap_or_default()
                });
                (view, scroll)
            };

        // Left: HEAD version (read-only), Right: working version (editable)
        let (old_view, old_scroll) = make_sv(&old_buf, false);
        let (new_view, new_scroll) = make_sv(&new_buf, true);

        // Overview rulers: narrow DrawingAreas next to each editor that mark
        // every changed line proportionally through the file. Clicking a
        // marker scrolls the view (and its counterpart via the adjustment
        // sync below) to the corresponding line.
        let old_bar = build_overview_ruler(
            old_change_lines,
            old_buf.line_count(),
            OverviewRulerKind::Delete,
            &old_view,
        );
        let new_bar = build_overview_ruler(
            new_change_lines,
            new_buf.line_count(),
            OverviewRulerKind::Insert,
            &new_view,
        );
        // Rulers on the outer edges of the diff so the Paned separator
        // between old/new scrollviews stays grabable. A generous margin
        // on the outer-left ruler keeps it clear of the *main* sidebar
        // paned's separator, whose resize grab zone extends further into
        // the editor surface than the visible handle suggests.
        old_bar.set_margin_start(12);
        let old_column = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        old_column.append(&old_bar);
        old_column.append(&old_scroll);
        let new_column = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        new_column.append(&new_scroll);
        new_column.append(&new_bar);

        // Sync scrolling between old and new
        let syncing = Rc::new(std::cell::Cell::new(false));
        {
            let ns = new_scroll.clone();
            let s = syncing.clone();
            old_scroll.vadjustment().connect_value_changed(move |adj| {
                if !s.get() {
                    s.set(true);
                    ns.vadjustment().set_value(adj.value());
                    s.set(false);
                }
            });
        }
        {
            let os = old_scroll.clone();
            let s = syncing.clone();
            new_scroll.vadjustment().connect_value_changed(move |adj| {
                if !s.get() {
                    s.set(true);
                    os.vadjustment().set_value(adj.value());
                    s.set(false);
                }
            });
        }

        // Build diff UI
        let diff_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        diff_box.set_vexpand(true);

        // Header: back button + file name + revert all
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        header.set_margin_start(8);
        header.set_margin_end(8);
        header.set_margin_top(4);
        header.set_margin_bottom(4);

        let back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
        back_btn.add_css_class("flat");
        back_btn.set_tooltip_text(Some("Back to editor"));
        header.append(&back_btn);

        let file_label = gtk4::Label::new(Some(&format!("Diff: {}", rel.to_string_lossy())));
        file_label.add_css_class("heading");
        file_label.set_hexpand(true);
        file_label.set_halign(gtk4::Align::Start);
        header.append(&file_label);

        // Stage button
        let stage_btn = gtk4::Button::from_icon_name("list-add-symbolic");
        stage_btn.add_css_class("flat");
        stage_btn.set_tooltip_text(Some("Stage this file"));
        {
            let fp = file_path.to_path_buf();
            let be = backend.clone();
            stage_btn.connect_clicked(move |_| {
                let _ = be.git_command(&["add", &fp.to_string_lossy()]);
            });
        }
        header.append(&stage_btn);

        // Revert button
        let revert_btn = gtk4::Button::from_icon_name("edit-undo-symbolic");
        revert_btn.add_css_class("flat");
        revert_btn.set_tooltip_text(Some("Revert all changes"));
        {
            let fp = file_path.to_path_buf();
            let root_c = root.to_path_buf();
            let cs = self.content_stack.clone();
            let nb = self.notebook.clone();
            let be = backend.clone();
            revert_btn.connect_clicked(move |_| {
                let rel = fp.strip_prefix(&root_c).unwrap_or(&fp);
                let _ = be.git_command(&["checkout", "--", &rel.to_string_lossy()]);
                if nb.n_pages() > 0 {
                    cs.set_visible_child_name("editor");
                } else {
                    cs.set_visible_child_name("welcome");
                }
            });
        }
        header.append(&revert_btn);
        diff_box.append(&header);

        // Column labels
        let labels = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        let old_label = gtk4::Label::new(Some(&format!(
            "← PREVIOUS  {}  (HEAD)",
            rel.to_string_lossy()
        )));
        old_label.add_css_class("dim-label");
        old_label.set_hexpand(true);
        old_label.set_margin_start(8);
        let new_label = gtk4::Label::new(Some(&format!(
            "CURRENT  {}  (working) →",
            rel.to_string_lossy()
        )));
        new_label.add_css_class("dim-label");
        new_label.set_hexpand(true);
        new_label.set_margin_start(8);
        labels.append(&old_label);
        labels.append(&new_label);
        diff_box.append(&labels);

        // Paned: HEAD (read-only) | working (editable)
        let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
        paned.set_vexpand(true);
        paned.set_start_child(Some(&old_column));
        paned.set_end_child(Some(&new_column));
        diff_box.append(&paned);

        // Save working side on Ctrl+S (via key controller on the diff_box)
        {
            let fp = file_path.to_path_buf();
            let nb = new_buf.clone();
            let be = backend.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.connect_key_pressed(move |_, key, _, modifier| {
                if crate::shortcuts::has_primary(modifier) && key == gtk4::gdk::Key::s {
                    let text = nb.text(&nb.start_iter(), &nb.end_iter(), false).to_string();
                    let _ = be.write_file(&fp, &text);
                    tracing::info!("Diff: saved working copy");
                    return gtk4::glib::Propagation::Stop;
                }
                gtk4::glib::Propagation::Proceed
            });
            diff_box.add_controller(key_ctrl);
        }

        // Remove previous diff child if any, then add new one
        if let Some(old_diff) = self.content_stack.child_by_name("diff") {
            self.content_stack.remove(&old_diff);
        }
        self.content_stack.add_named(&diff_box, Some("diff"));
        self.content_stack.set_visible_child_name("diff");

        // Back button returns to editor or welcome
        {
            let cs = self.content_stack.clone();
            let nb = self.notebook.clone();
            back_btn.connect_clicked(move |_| {
                if nb.n_pages() > 0 {
                    cs.set_visible_child_name("editor");
                } else {
                    cs.set_visible_child_name("welcome");
                }
            });
        }
    }

    /// Show the search bar (Ctrl+F). Hides replace row.
    pub fn show_search(&self) {
        self.replace_row.set_visible(false);
        self.search_bar.set_visible(true);
        self.search_entry.grab_focus();
        // Pre-fill with current selection
        let buf = self.source_view.buffer();
        if let Some((start, end)) = buf.selection_bounds() {
            let text = buf.text(&start, &end, false).to_string();
            if !text.is_empty() && !text.contains('\n') {
                self.search_entry.set_text(&text);
            }
        }
    }

    /// Show the search+replace bar (Ctrl+H).
    pub fn show_replace(&self) {
        self.replace_row.set_visible(true);
        self.search_bar.set_visible(true);
        self.search_entry.grab_focus();
        // Pre-fill with current selection
        let buf = self.source_view.buffer();
        if let Some((start, end)) = buf.selection_bounds() {
            let text = buf.text(&start, &end, false).to_string();
            if !text.is_empty() && !text.contains('\n') {
                self.search_entry.set_text(&text);
            }
        }
    }

    /// Save the currently active file.
    pub fn save_active(&self, state: &Rc<RefCell<EditorState>>, _root: &Path) {
        {
            let mut st = state.borrow_mut();
            let backend = st.backend.clone();
            let record_key = st.record_key.clone();
            if let Some(idx) = st.active_tab {
                if let Some(open_file) = st.open_files.get_mut(idx) {
                    let Some(buf) = open_file.writable_buffer().cloned() else {
                        return;
                    };
                    let text = buf
                        .text(&buf.start_iter(), &buf.end_iter(), false)
                        .to_string();
                    if let Err(e) = backend.write_file(&open_file.path, &text) {
                        tracing::error!("Failed to save {}: {}", open_file.path.display(), e);
                        return;
                    }
                    open_file.set_modified(false);
                    open_file.last_disk_mtime = get_mtime(&open_file.path);
                    // Update saved content so dirty detection compares against new save
                    if let Some(cell) = open_file.saved_content() {
                        *cell.borrow_mut() = text;
                    }
                    // Flush note positions: for each note on this tab, read
                    // its current line from its mark and persist (line,
                    // anchor) so the next reload is robust to edits the user
                    // made during the session. Applies to both source and
                    // markdown tabs (both carry a NotesState over a buffer).
                    if !record_key.is_empty() {
                        let (notes_opt, buffer_opt): (
                            Option<&super::notes_state::NotesState>,
                            Option<&sourceview5::Buffer>,
                        ) = match &open_file.content {
                            super::tab_content::TabContent::Source(source) => {
                                (Some(&source.notes), Some(&source.buffer))
                            }
                            super::tab_content::TabContent::Markdown(m) => {
                                (Some(&m.notes), Some(&m.buffer))
                            }
                            _ => (None, None),
                        };
                        if let (Some(notes), Some(buffer)) = (notes_opt, buffer_opt) {
                            let snapshot: Vec<(i64, i32, String)> = notes
                                .entries
                                .borrow()
                                .iter()
                                .filter_map(|e| {
                                    let mark = e.mark.as_ref()?;
                                    let line =
                                        super::notes_state::line_of_mark(buffer, mark);
                                    let anchor =
                                        super::notes_state::line_content(buffer, line);
                                    Some((e.db_id, line, anchor))
                                })
                                .collect();
                            if !snapshot.is_empty() {
                                let db_path = pax_db::Database::default_path();
                                if let Ok(db) = pax_db::Database::open(&db_path) {
                                    for (id, line, anchor) in snapshot {
                                        let _ = db.update_metadata_position(
                                            id,
                                            line,
                                            Some(&anchor),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    // Clear the modified indicator in tab and status bar
                    if let Some(page) = self.notebook.nth_page(Some(idx as u32)) {
                        if let Some(tab_label) = self.notebook.tab_label(&page) {
                            // Tab label is a Box: [dot, icon, name, close_btn]
                            if let Some(dot) = tab_label.first_child() {
                                if let Some(label) = dot.downcast_ref::<gtk4::Label>() {
                                    label.set_text("");
                                }
                            }
                        }
                    }
                }
            }
        }
        self.status_modified.set_text("");
    }

    /// Close the active tab. If modified, save first then close.
    pub fn close_active_tab(&self, state: &Rc<RefCell<EditorState>>, root: &Path) {
        let idx = match state.borrow().active_tab {
            Some(i) => i,
            None => return,
        };

        let is_modified = state
            .borrow()
            .open_files
            .get(idx)
            .map(|f| f.modified())
            .unwrap_or(false);

        if is_modified {
            self.save_active(state, root);
        }
        self.remove_tab(idx, state);
    }

    /// Remove the tab at the given index from the notebook and state.
    pub fn remove_tab(&self, idx: usize, state: &Rc<RefCell<EditorState>>) {
        let empty_after;
        let new_idx;
        {
            let mut st = state.borrow_mut();
            if idx >= st.open_files.len() {
                return;
            }
            st.open_files.remove(idx);
            empty_after = st.open_files.is_empty();
            new_idx = if empty_after {
                0
            } else {
                idx.min(st.open_files.len() - 1)
            };
            if empty_after {
                st.active_tab = None;
            } else {
                st.active_tab = Some(new_idx);
            }
        }
        // Borrow is dropped — safe to call notebook methods that trigger switch_page
        self.notebook.remove_page(Some(idx as u32));
        if empty_after {
            self.notebook.set_show_tabs(false);
            self.content_stack.set_visible_child_name("welcome");
        } else {
            self.notebook.set_current_page(Some(new_idx as u32));
            self.switch_to_buffer(new_idx, state);
        }
        super::fire_nav_state_changed(state);
    }

    /// Propagate an on-disk rename to any tab currently showing `old_path`.
    /// Updates the stored path, refreshes the tab label widget, and re-guesses
    /// the sourceview language from the new filename so syntax highlighting
    /// tracks the extension change.
    pub fn rename_open_file(
        &self,
        old_path: &Path,
        new_path: &Path,
        state: &Rc<RefCell<EditorState>>,
    ) {
        let mut st = state.borrow_mut();
        let Some(open_file) = st.open_files.iter_mut().find(|f| f.path == old_path) else {
            return;
        };
        open_file.path = new_path.to_path_buf();
        let new_name = new_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        open_file.name_label.set_text(&new_name);

        // Re-detect language against the new filename so the buffer switches
        // schemes when, e.g., `foo` is renamed to `foo.rs`.
        let lang_manager = sourceview5::LanguageManager::default();
        let new_lang = lang_manager
            .guess_language(Some(new_path), None::<&str>)
            .or_else(|| fallback_language_for(&lang_manager, new_path));
        if let Some(buf) = open_file.source_buffer() {
            buf.set_language(new_lang.as_ref());
        }
        tracing::info!(
            "editor.tabs: rename_open_file old={} new={}",
            old_path.display(),
            new_path.display()
        );
    }

    /// Close any tab showing `path`. No-op if no tab is open for that path.
    /// Does NOT prompt on unsaved changes — callers that want a prompt should
    /// surface one before invoking this (used for file-deleted-on-disk flows
    /// where the file is already gone and prompting would be pointless).
    pub fn close_tab_for_path(&self, path: &Path, state: &Rc<RefCell<EditorState>>) {
        let idx = state
            .borrow()
            .open_files
            .iter()
            .position(|f| f.path == path);
        if let Some(idx) = idx {
            tracing::info!(
                "editor.tabs: close_tab_for_path idx={} path={}",
                idx,
                path.display()
            );
            self.remove_tab(idx, state);
        }
    }

    /// Close every tab whose path lives under `dir` (inclusive). Used after a
    /// directory is deleted so orphaned tabs don't linger pointing at paths
    /// that no longer exist on disk. Iterates in reverse index so removals
    /// don't shift unprocessed indices.
    pub fn close_tabs_under_dir(&self, dir: &Path, state: &Rc<RefCell<EditorState>>) {
        let indices: Vec<usize> = {
            let st = state.borrow();
            st.open_files
                .iter()
                .enumerate()
                .filter(|(_, f)| f.path.starts_with(dir))
                .map(|(i, _)| i)
                .collect()
        };
        if indices.is_empty() {
            return;
        }
        tracing::info!(
            "editor.tabs: close_tabs_under_dir dir={} count={}",
            dir.display(),
            indices.len()
        );
        for idx in indices.into_iter().rev() {
            self.remove_tab(idx, state);
        }
    }

    /// Update gutter diff indicators for the active file.
    pub fn update_gutter_marks(&self, _root: &Path, state: &Rc<RefCell<EditorState>>) {
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

        // Diff markers apply only to source tabs.
        let buf = match open_file.source_buffer() {
            Some(b) => b,
            None => return,
        };

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

        let (start, end) = (buf.start_iter(), buf.end_iter());
        buf.remove_tag_by_name("diff-added", &start, &end);
        buf.remove_tag_by_name("diff-removed", &start, &end);
        buf.remove_tag_by_name("diff-modified", &start, &end);

        let file_path = open_file.path.clone();
        let backend = st.backend.clone();
        drop(st);

        let hunks = compute_diff(&*backend, &file_path);
        let st = state.borrow();
        let open_file = match st.open_files.get(idx) {
            Some(f) => f,
            None => return,
        };
        let buf = match open_file.source_buffer() {
            Some(b) => b,
            None => return,
        };

        for hunk in &hunks {
            let has_old = hunk.old_lines.iter().any(|l| l.starts_with('-'));
            let has_new = hunk.new_lines.iter().any(|l| l.starts_with('+'));

            let tag_name = match (has_old, has_new) {
                (true, true) => "diff-modified",
                (false, true) => "diff-added",
                (true, false) => "diff-removed",
                _ => continue,
            };

            let mut line_num = hunk.new_start.saturating_sub(1);
            for line in &hunk.new_lines {
                if line.starts_with('+') {
                    if line_num < buf.line_count() as usize {
                        let start = buf
                            .iter_at_line(line_num as i32)
                            .unwrap_or(buf.start_iter());
                        let mut end = start.clone();
                        end.forward_to_line_end();
                        buf.apply_tag_by_name(tag_name, &start, &end);
                    }
                }
                if line.starts_with('+') || line.starts_with(' ') {
                    line_num += 1;
                }
            }
        }
    }
    /// Show a commit's diff: header with info, file list, click file for side-by-side diff.
    pub fn show_commit_diff(
        &self,
        _root: &Path,
        commit_hash: &str,
        backend: Arc<dyn super::file_backend::FileBackend>,
    ) {
        // Get commit info
        let info = backend
            .git_command(&["log", "-1", "--format=%H%n%s%n%an%n%ar", commit_hash])
            .unwrap_or_default();

        let info_lines: Vec<&str> = info.lines().collect();
        let full_hash = info_lines.first().copied().unwrap_or(commit_hash);
        let subject = info_lines.get(1).copied().unwrap_or("");
        let author = info_lines.get(2).copied().unwrap_or("");
        let date = info_lines.get(3).copied().unwrap_or("");

        // Get list of changed files with status
        let diff_stat = backend
            .git_command(&[
                "diff-tree",
                "--no-commit-id",
                "-r",
                "--name-status",
                commit_hash,
            ])
            .unwrap_or_default();

        // Get numeric stats (additions/deletions per file)
        let numstat = backend
            .git_command(&[
                "diff-tree",
                "--no-commit-id",
                "-r",
                "--numstat",
                commit_hash,
            ])
            .unwrap_or_default();
        let stats: std::collections::HashMap<&str, (String, String)> = numstat
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                if parts.len() == 3 {
                    Some((parts[2], (parts[0].to_string(), parts[1].to_string())))
                } else {
                    None
                }
            })
            .collect();

        // Build UI
        let commit_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        commit_box.set_vexpand(true);

        // Header: back button + commit info
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        header.set_margin_start(8);
        header.set_margin_end(8);
        header.set_margin_top(4);
        header.set_margin_bottom(4);

        let back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
        back_btn.add_css_class("flat");
        back_btn.set_tooltip_text(Some("Back to editor"));
        header.append(&back_btn);

        let info_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        info_box.set_hexpand(true);

        let title_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let hash_label = gtk4::Label::new(Some(&full_hash[..full_hash.len().min(8)]));
        hash_label.add_css_class("dim-label");
        hash_label.add_css_class("monospace");
        title_row.append(&hash_label);
        let subject_label = gtk4::Label::new(Some(subject));
        subject_label.add_css_class("heading");
        subject_label.set_halign(gtk4::Align::Start);
        subject_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        title_row.append(&subject_label);
        info_box.append(&title_row);

        let meta_label = gtk4::Label::new(Some(&format!("{} · {}", author, date)));
        meta_label.add_css_class("dim-label");
        meta_label.add_css_class("caption");
        meta_label.set_halign(gtk4::Align::Start);
        info_box.append(&meta_label);

        header.append(&info_box);

        // Revert commit button
        let revert_commit_btn = gtk4::Button::from_icon_name("edit-undo-symbolic");
        revert_commit_btn.add_css_class("flat");
        revert_commit_btn.set_tooltip_text(Some("Revert this commit (git revert)"));
        {
            let be = backend.clone();
            let hash = commit_hash.to_string();
            revert_commit_btn.connect_clicked(move |_| {
                let _ = be.git_command(&["revert", "--no-edit", &hash]);
            });
        }
        header.append(&revert_commit_btn);

        commit_box.append(&header);
        commit_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        // File list (plain Box, no ListBox background)
        let file_list = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        file_list.set_margin_start(8);
        file_list.set_margin_end(8);
        file_list.set_margin_top(4);

        for line in diff_stat.lines() {
            let parts: Vec<&str> = line.splitn(2, '\t').collect();
            if parts.len() != 2 {
                continue;
            }
            let status_char = parts[0];
            let file_path_str = parts[1];

            let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            row_box.set_margin_start(4);
            row_box.set_margin_end(4);
            row_box.set_margin_top(4);
            row_box.set_margin_bottom(4);

            // Status badge
            let status_label = gtk4::Label::new(Some(status_char));
            status_label.add_css_class("monospace");
            match status_char {
                "A" => status_label.add_css_class("success"),
                "D" => status_label.add_css_class("error"),
                "M" => status_label.add_css_class("warning"),
                _ => status_label.add_css_class("dim-label"),
            }
            row_box.append(&status_label);

            // Clickable filename → opens diff
            let name_btn = gtk4::Button::with_label(file_path_str);
            name_btn.add_css_class("flat");
            name_btn.set_hexpand(true);
            name_btn.set_halign(gtk4::Align::Start);
            name_btn.set_tooltip_text(Some(file_path_str));
            if let Some(child) = name_btn.child() {
                if let Some(label) = child.downcast_ref::<gtk4::Label>() {
                    label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
                    label.set_halign(gtk4::Align::Start);
                }
            }
            {
                let hash = commit_hash.to_string();
                let cs = self.content_stack.clone();
                let nb = self.notebook.clone();
                let fp = file_path_str.to_string();
                let be = backend.clone();
                name_btn.connect_clicked(move |_| {
                    show_commit_file_diff(&cs, &nb, &hash, &fp, be.clone());
                });
            }
            row_box.append(&name_btn);

            // Change stats (+N / -N)
            if let Some((added, removed)) = stats.get(file_path_str) {
                let stat_text = format!("+{}  −{}", added, removed);
                let stat_label = gtk4::Label::new(Some(&stat_text));
                stat_label.add_css_class("dim-label");
                stat_label.add_css_class("caption");
                stat_label.add_css_class("monospace");
                row_box.append(&stat_label);
            }

            file_list.append(&row_box);
            file_list.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
        }

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_child(Some(&file_list));
        scroll.set_vexpand(true);
        commit_box.append(&scroll);

        // Add to content stack
        if let Some(old) = self.content_stack.child_by_name("commit-diff") {
            self.content_stack.remove(&old);
        }
        self.content_stack
            .add_named(&commit_box, Some("commit-diff"));
        self.content_stack.set_visible_child_name("commit-diff");

        // Back button
        {
            let cs = self.content_stack.clone();
            let nb = self.notebook.clone();
            back_btn.connect_clicked(move |_| {
                if nb.n_pages() > 0 {
                    cs.set_visible_child_name("editor");
                } else {
                    cs.set_visible_child_name("welcome");
                }
            });
        }
    }
}

/// Show a side-by-side diff for a single file within a commit.
fn show_commit_file_diff(
    content_stack: &gtk4::Stack,
    _notebook: &gtk4::Notebook,
    commit_hash: &str,
    file_rel: &str,
    backend: Arc<dyn super::file_backend::FileBackend>,
) {
    // Get old version (parent commit) and new version (this commit)
    let parent = format!("{}~1", commit_hash);
    let old_content = backend
        .git_show(&format!("{}:{}", parent, file_rel))
        .unwrap_or_default();
    let new_content = backend
        .git_show(&format!("{}:{}", commit_hash, file_rel))
        .unwrap_or_default();

    let old_buf = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
    old_buf.set_text(&old_content);
    old_buf.set_highlight_syntax(true);
    let new_buf = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
    new_buf.set_text(&new_content);
    new_buf.set_highlight_syntax(true);

    // Syntax highlighting
    let lang_manager = sourceview5::LanguageManager::default();
    let file_path = Path::new(file_rel);
    if let Some(lang) = lang_manager.guess_language(Some(file_path), None::<&str>) {
        old_buf.set_language(Some(&lang));
        new_buf.set_language(Some(&lang));
    }
    crate::theme::register_sourceview_buffer(&old_buf);
    crate::theme::register_sourceview_buffer(&new_buf);

    // Highlight diff
    {
        let diff = similar::TextDiff::from_lines(&old_content, &new_content);
        let ensure_tags = |buf: &sourceview5::Buffer| {
            let tt = buf.tag_table();
            if tt.lookup("diff-del").is_none() {
                let tag = gtk4::TextTag::new(Some("diff-del"));
                tag.set_paragraph_background(Some("rgba(220, 50, 47, 0.25)"));
                tt.add(&tag);
            }
            if tt.lookup("diff-add").is_none() {
                let tag = gtk4::TextTag::new(Some("diff-add"));
                tag.set_paragraph_background(Some("rgba(40, 180, 60, 0.25)"));
                tt.add(&tag);
            }
        };
        ensure_tags(&old_buf);
        ensure_tags(&new_buf);

        let mut old_line = 0i32;
        let mut new_line = 0i32;
        for change in diff.iter_all_changes() {
            match change.tag() {
                similar::ChangeTag::Equal => {
                    old_line += 1;
                    new_line += 1;
                }
                similar::ChangeTag::Delete => {
                    if let Some(start) = old_buf.iter_at_line(old_line) {
                        let mut end = start.clone();
                        end.forward_to_line_end();
                        end.forward_char();
                        old_buf.apply_tag_by_name("diff-del", &start, &end);
                    }
                    old_line += 1;
                }
                similar::ChangeTag::Insert => {
                    if let Some(start) = new_buf.iter_at_line(new_line) {
                        let mut end = start.clone();
                        end.forward_to_line_end();
                        end.forward_char();
                        new_buf.apply_tag_by_name("diff-add", &start, &end);
                    }
                    new_line += 1;
                }
            }
        }
    }

    let make_sv = |buf: &sourceview5::Buffer| -> gtk4::ScrolledWindow {
        let view = sourceview5::View::with_buffer(buf);
        view.add_css_class("editor-code-view");
        view.set_editable(false);
        view.set_show_line_numbers(true);
        view.set_monospace(true);
        view.set_left_margin(3);
        install_text_clipboard_shortcuts(&view);
        install_text_history_shortcuts(&view);
        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_child(Some(&view));
        scroll.set_vexpand(true);
        scroll.set_hexpand(true);
        text_context_menu::install(&scroll, &view, false, |_click_line| Vec::new());
        scroll
    };

    let old_scroll = make_sv(&old_buf);
    let new_scroll = make_sv(&new_buf);

    // Sync scrolling
    let syncing = Rc::new(std::cell::Cell::new(false));
    {
        let ns = new_scroll.clone();
        let s = syncing.clone();
        old_scroll.vadjustment().connect_value_changed(move |adj| {
            if !s.get() {
                s.set(true);
                ns.vadjustment().set_value(adj.value());
                s.set(false);
            }
        });
    }
    {
        let os = old_scroll.clone();
        let s = syncing;
        new_scroll.vadjustment().connect_value_changed(move |adj| {
            if !s.get() {
                s.set(true);
                os.vadjustment().set_value(adj.value());
                s.set(false);
            }
        });
    }

    // Build diff view
    let diff_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    diff_box.set_vexpand(true);

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    header.set_margin_start(8);
    header.set_margin_end(8);
    header.set_margin_top(4);
    header.set_margin_bottom(4);

    let back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
    back_btn.add_css_class("flat");
    back_btn.set_tooltip_text(Some("Back to commit"));
    header.append(&back_btn);

    let file_label = gtk4::Label::new(Some(&format!(
        "{}  {} → {}",
        file_rel,
        &parent[..parent.len().min(8)],
        &commit_hash[..commit_hash.len().min(8)]
    )));
    file_label.add_css_class("heading");
    file_label.set_hexpand(true);
    file_label.set_halign(gtk4::Align::Start);
    header.append(&file_label);

    // Revert this file to before this commit
    let revert_btn = gtk4::Button::from_icon_name("edit-undo-symbolic");
    revert_btn.add_css_class("flat");
    revert_btn.set_tooltip_text(Some("Revert this file to before this commit"));
    {
        let be = backend.clone();
        let parent_c = parent.clone();
        let fp = file_rel.to_string();
        let cs = content_stack.clone();
        revert_btn.connect_clicked(move |_| {
            let _ = be.git_command(&["checkout", &parent_c, "--", &fp]);
            cs.set_visible_child_name("commit-diff");
        });
    }
    header.append(&revert_btn);

    diff_box.append(&header);

    // Column labels
    let labels = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    let old_label = gtk4::Label::new(Some(&format!(
        "← PREVIOUS  {}  ({})",
        file_rel,
        &parent[..parent.len().min(8)]
    )));
    old_label.add_css_class("dim-label");
    old_label.set_hexpand(true);
    old_label.set_margin_start(8);
    let new_label = gtk4::Label::new(Some(&format!(
        "CURRENT  {}  ({}) →",
        file_rel,
        &commit_hash[..commit_hash.len().min(8)]
    )));
    new_label.add_css_class("dim-label");
    new_label.set_hexpand(true);
    new_label.set_margin_start(8);
    labels.append(&old_label);
    labels.append(&new_label);
    diff_box.append(&labels);

    let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
    paned.set_vexpand(true);
    paned.set_start_child(Some(&old_scroll));
    paned.set_end_child(Some(&new_scroll));
    diff_box.append(&paned);

    // Replace content
    if let Some(old) = content_stack.child_by_name("commit-file-diff") {
        content_stack.remove(&old);
    }
    content_stack.add_named(&diff_box, Some("commit-file-diff"));
    content_stack.set_visible_child_name("commit-file-diff");

    // Back goes to commit-diff view
    {
        let cs = content_stack.clone();
        back_btn.connect_clicked(move |_| {
            cs.set_visible_child_name("commit-diff");
        });
    }
}

/// Resolve the keyword list for a GtkSourceView language by parsing the
/// shipped {id}.lang XML file. Result is cached after first lookup. Returns
/// an empty list when no .lang file is found or it has no <keyword> tags.
///
/// We pull every `<keyword>` token, not only the ones in style-ref="keyword"
/// contexts, because builtin/type/exception identifiers (e.g. `print`,
/// `Vec`, `Exception`) are exactly what users want to autocomplete too.
fn keywords_for(lang_id: &str) -> std::sync::Arc<Vec<String>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};

    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Vec<String>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache.lock().unwrap().get(lang_id).cloned() {
        return cached;
    }

    let mut words = load_keywords_from_lang(lang_id);
    // Some IDs are thin extensions of a base language (python3 inherits
    // python via <context ref="python:...">). Walking those refs would
    // require a real parser; an alias merge covers the common cases.
    for parent in parent_language_aliases(lang_id) {
        let extra = load_keywords_from_lang(parent);
        words.extend(extra);
    }
    words.sort();
    words.dedup();

    let arc = Arc::new(words);
    cache
        .lock()
        .unwrap()
        .insert(lang_id.to_string(), arc.clone());
    arc
}

/// Map a language ID to base language(s) whose keywords should also be
/// loaded. Returns an empty slice for self-contained languages.
fn parent_language_aliases(lang_id: &str) -> &'static [&'static str] {
    match lang_id {
        "python3" => &["python"],
        "bash" | "zsh" => &["sh"],
        _ => &[],
    }
}

fn load_keywords_from_lang(lang_id: &str) -> Vec<String> {
    let manager = sourceview5::LanguageManager::default();
    for path in manager.search_path() {
        let candidate = std::path::Path::new(path.as_str()).join(format!("{}.lang", lang_id));
        if let Ok(xml) = std::fs::read_to_string(&candidate) {
            return parse_keyword_tags(&xml);
        }
    }
    Vec::new()
}

/// Extract every `<keyword>TOKEN</keyword>` payload from a .lang XML.
/// Whitespace inside the tag is trimmed; duplicates are *not* removed here
/// (the caller dedups after merging multiple files).
fn parse_keyword_tags(xml: &str) -> Vec<String> {
    use std::sync::OnceLock;

    static KEYWORD_RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = KEYWORD_RE
        .get_or_init(|| regex::Regex::new(r"<keyword>([^<]+)</keyword>").unwrap());

    re.captures_iter(xml)
        .filter_map(|c| {
            let raw = c.get(1)?.as_str().trim();
            if raw.is_empty() {
                None
            } else {
                Some(raw.to_string())
            }
        })
        .collect()
}

/// Install bracket and quote auto-pairing on the SourceView.
///
/// - Typing `(`, `[`, `{`, `"`, `'`, `` ` `` inserts the matching closer
///   and places the cursor between them.
/// - With a selection active, the selection is wrapped instead.
/// - For the symmetrical pairs (quotes, backticks), if the cursor is
///   already sitting on the same character, just step past it instead of
///   inserting a doubled-up closer (matches what users expect from
///   VS Code / IntelliJ).
fn install_bracket_auto_pair(view: &sourceview5::View) {
    use gtk4::gdk;

    let key_ctrl = gtk4::EventControllerKey::new();
    key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);

    let view_clone = view.clone();
    key_ctrl.connect_key_pressed(move |_, key, _, state| {
        // Ignore when modifiers are held (Ctrl+(, Alt+", etc. shouldn't pair).
        let allowed = gtk4::gdk::ModifierType::SHIFT_MASK;
        if state.intersects(!allowed) {
            return gtk4::glib::Propagation::Proceed;
        }

        let pair: Option<(&str, &str, bool)> = match key {
            gdk::Key::parenleft => Some(("(", ")", false)),
            gdk::Key::bracketleft => Some(("[", "]", false)),
            gdk::Key::braceleft => Some(("{", "}", false)),
            gdk::Key::quotedbl => Some(("\"", "\"", true)),
            gdk::Key::apostrophe => Some(("'", "'", true)),
            gdk::Key::grave => Some(("`", "`", true)),
            _ => return gtk4::glib::Propagation::Proceed,
        };
        let (open, close, symmetrical) = pair.unwrap();

        let Ok(buffer) = view_clone.buffer().downcast::<sourceview5::Buffer>() else {
            return gtk4::glib::Propagation::Proceed;
        };

        // Wrap selection.
        if buffer.has_selection() {
            if let Some((mut s, mut e)) = buffer.selection_bounds() {
                let text = buffer.text(&s, &e, false).to_string();
                buffer.begin_user_action();
                buffer.delete(&mut s, &mut e);
                buffer.insert(&mut s, &format!("{}{}{}", open, text, close));
                buffer.end_user_action();
                return gtk4::glib::Propagation::Stop;
            }
        }

        // For symmetrical chars (quotes, backticks): step over an existing
        // matching closer rather than inserting a duplicate.
        if symmetrical {
            let cursor = buffer.iter_at_mark(&buffer.get_insert());
            let mut next = cursor.clone();
            if next.forward_char() {
                let next_char = buffer.text(&cursor, &next, false).to_string();
                if next_char == open {
                    buffer.place_cursor(&next);
                    return gtk4::glib::Propagation::Stop;
                }
            }
        }

        // Insert pair, leave cursor between.
        let mut iter = buffer.iter_at_mark(&buffer.get_insert());
        buffer.begin_user_action();
        buffer.insert(&mut iter, &format!("{}{}", open, close));
        let mut cursor = buffer.iter_at_mark(&buffer.get_insert());
        cursor.backward_char();
        buffer.place_cursor(&cursor);
        buffer.end_user_action();
        gtk4::glib::Propagation::Stop
    });
    view.add_controller(key_ctrl);
}

/// Pick a GtkSourceView language for files that the upstream mime/glob
/// heuristics fail to recognise. Returns `None` when no override applies,
/// leaving the buffer unstyled (the editor still works as plain text).
fn fallback_language_for(
    manager: &sourceview5::LanguageManager,
    path: &Path,
) -> Option<sourceview5::Language> {
    let name = path.file_name().and_then(|s| s.to_str())?;
    // Dotenv-style files: KEY=value with `#` comments, shell-compatible.
    if name == ".env"
        || name == ".envrc"
        || name.starts_with(".env.")
        || name.ends_with(".env")
    {
        return manager.language("sh");
    }
    None
}

#[derive(Debug, Clone, Copy)]
enum OverviewRulerKind {
    /// Red marks for deleted lines (on the PREVIOUS side).
    Delete,
    /// Green marks for inserted lines (on the CURRENT side).
    Insert,
    /// Gold marks for search-match lines (main editor overview).
    Match,
}

/// Pixel width of the overview ruler strip. Wide enough to be tappable
/// without crowding the editor's scrollbar.
const OVERVIEW_RULER_WIDTH: i32 = 10;
/// Minimum pixel height of a single marker so it stays visible and clickable
/// even in very long files where each line would otherwise collapse to
/// sub-pixel size.
const OVERVIEW_RULER_MARK_MIN_HEIGHT: f64 = 2.0;
/// Alpha for the neutral backdrop behind the marks. Low enough to blend
/// with the surrounding chrome but present so the strip has a visual
/// identity even when the file has no changes.
const OVERVIEW_RULER_BG_ALPHA: f64 = 0.05;

fn overview_ruler_color(kind: OverviewRulerKind) -> (f64, f64, f64) {
    // Match the rgba fills already used for diff-del / diff-add paragraph
    // backgrounds so the minimap reads as the same language as the inline
    // highlighting.
    match kind {
        OverviewRulerKind::Delete => (220.0 / 255.0, 50.0 / 255.0, 47.0 / 255.0),
        OverviewRulerKind::Insert => (40.0 / 255.0, 180.0 / 255.0, 60.0 / 255.0),
        // Gold, matches the `#e5a50a` highlight used for search matches.
        OverviewRulerKind::Match => (229.0 / 255.0, 165.0 / 255.0, 10.0 / 255.0),
    }
}

/// Build a narrow clickable strip that shows every changed line at its
/// proportional position in the file. Clicking a marker (or anywhere in the
/// strip) scrolls `view` to the nearest change and places the cursor there.
fn build_overview_ruler(
    change_lines: Vec<i32>,
    total_lines: i32,
    kind: OverviewRulerKind,
    view: &sourceview5::View,
) -> gtk4::DrawingArea {
    let bar = gtk4::DrawingArea::new();
    bar.set_width_request(OVERVIEW_RULER_WIDTH);
    bar.set_vexpand(true);
    bar.add_css_class("diff-overview-ruler");
    bar.set_tooltip_text(Some("Click a marker to jump to that change"));
    bar.set_cursor_from_name(Some("pointer"));

    let lines = Rc::new(change_lines);
    let total = total_lines.max(1);

    {
        let lines = lines.clone();
        bar.set_draw_func(move |_, cr, w, h| {
            let (r, g, b) = overview_ruler_color(kind);
            let h_f = h as f64;
            let w_f = w as f64;
            cr.set_source_rgba(0.5, 0.5, 0.5, OVERVIEW_RULER_BG_ALPHA);
            let _ = cr.paint();
            cr.set_source_rgba(r, g, b, 0.9);
            let mark_h = (h_f / total as f64).max(OVERVIEW_RULER_MARK_MIN_HEIGHT);
            for &line in lines.iter() {
                let y = (line as f64 / total as f64) * h_f;
                cr.rectangle(0.0, y, w_f, mark_h);
            }
            let _ = cr.fill();
        });
    }

    {
        let view = view.clone();
        let lines = lines.clone();
        let bar_for_click = bar.clone();
        let gesture = gtk4::GestureClick::new();
        gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
        gesture.connect_pressed(move |g, _n, _x, y| {
            // Claim the event so the enclosing Paned doesn't start a
            // drag-to-resize on the same press.
            g.set_state(gtk4::EventSequenceState::Claimed);
            let h = bar_for_click.height().max(1) as f64;
            let proportion = (y / h).clamp(0.0, 1.0);
            let clicked = (proportion * total as f64) as i32;
            // Snap to the nearest known change so clicking the backdrop
            // between two markers still lands on a real change.
            let target = lines
                .iter()
                .copied()
                .min_by_key(|l| (*l - clicked).abs())
                .unwrap_or(clicked);
            let buf = view.buffer();
            if let Some(iter) = buf.iter_at_line(target) {
                buf.place_cursor(&iter);
                view.scroll_to_iter(&mut iter.clone(), 0.1, true, 0.5, 0.5);
            }
        });
        bar.add_controller(gesture);
    }

    bar
}

/// Like `build_overview_ruler` but the marked lines and total line count are
/// re-read on every draw, so the ruler can follow the active buffer as the
/// user edits, switches tabs, or changes the search query. `lines` is shared
/// state the caller mutates; after mutating, call `queue_draw` on the returned
/// widget to repaint.
fn build_match_overview_ruler(
    lines: Rc<RefCell<Vec<i32>>>,
    kind: OverviewRulerKind,
    view: sourceview5::View,
) -> gtk4::DrawingArea {
    let bar = gtk4::DrawingArea::new();
    bar.set_width_request(OVERVIEW_RULER_WIDTH);
    bar.set_vexpand(true);
    bar.add_css_class("editor-match-ruler");
    bar.set_tooltip_text(Some("Click a marker to jump to that match"));
    bar.set_cursor_from_name(Some("pointer"));

    {
        let lines = lines.clone();
        let view = view.clone();
        bar.set_draw_func(move |_, cr, w, h| {
            let total = view.buffer().line_count().max(1);
            let (r, g, b) = overview_ruler_color(kind);
            let h_f = h as f64;
            let w_f = w as f64;
            cr.set_source_rgba(0.5, 0.5, 0.5, OVERVIEW_RULER_BG_ALPHA);
            let _ = cr.paint();
            cr.set_source_rgba(r, g, b, 0.9);
            let mark_h = (h_f / total as f64).max(OVERVIEW_RULER_MARK_MIN_HEIGHT);
            let ls = lines.borrow();
            for &line in ls.iter() {
                let y = (line as f64 / total as f64) * h_f;
                cr.rectangle(0.0, y, w_f, mark_h);
            }
            let _ = cr.fill();
        });
    }

    {
        let view = view.clone();
        let lines = lines.clone();
        let bar_for_click = bar.clone();
        let gesture = gtk4::GestureClick::new();
        gesture.connect_pressed(move |_, _n, _x, y| {
            let total = view.buffer().line_count().max(1);
            let h = bar_for_click.height().max(1) as f64;
            let proportion = (y / h).clamp(0.0, 1.0);
            let clicked = (proportion * total as f64) as i32;
            let ls = lines.borrow();
            if ls.is_empty() {
                return;
            }
            let target = ls
                .iter()
                .copied()
                .min_by_key(|l| (*l - clicked).abs())
                .unwrap_or(clicked);
            let buf = view.buffer();
            if let Some(iter) = buf.iter_at_line(target) {
                buf.place_cursor(&iter);
                view.scroll_to_iter(&mut iter.clone(), 0.1, true, 0.5, 0.5);
            }
        });
        bar.add_controller(gesture);
    }

    bar
}

/// Scan `buf` for `query` (case-insensitive, substring) and return the 0-based
/// line numbers of every matching line. Used to populate the match overview
/// ruler without depending on a `SearchContext`.
fn collect_match_lines(buf: &sourceview5::Buffer, query: &str) -> Vec<i32> {
    if query.is_empty() {
        return Vec::new();
    }
    let start = buf.start_iter();
    let end = buf.end_iter();
    let text = buf.text(&start, &end, true).to_string();
    let needle = query.to_lowercase();
    let mut out = Vec::new();
    for (idx, line) in text.split('\n').enumerate() {
        if line.to_lowercase().contains(&needle) {
            out.push(idx as i32);
        }
    }
    out
}

fn get_mtime(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keyword_tags_extracts_payloads_and_skips_empty() {
        let xml = r#"
            <context id="keywords" style-ref="keyword">
              <keyword>def</keyword>
              <keyword>class</keyword>
              <keyword>  </keyword>
              <keyword>if</keyword>
            </context>
            <context id="builtins" style-ref="builtin-function">
              <keyword>print</keyword>
              <keyword>len</keyword>
            </context>
        "#;
        let mut got = parse_keyword_tags(xml);
        got.sort();
        assert_eq!(got, vec!["class", "def", "if", "len", "print"]);
    }

    #[test]
    fn parent_language_aliases_covers_python3_and_shell_dialects() {
        assert_eq!(parent_language_aliases("python3"), &["python"]);
        assert_eq!(parent_language_aliases("bash"), &["sh"]);
        assert_eq!(parent_language_aliases("zsh"), &["sh"]);
        assert!(parent_language_aliases("rust").is_empty());
        assert!(parent_language_aliases("totally-unknown").is_empty());
    }

    #[test]
    fn recognizes_text_clipboard_shortcuts() {
        let primary = crate::shortcuts::PRIMARY_MODIFIER;
        assert_eq!(
            text_clipboard_action(gtk4::gdk::Key::c, primary),
            Some(TextClipboardAction::Copy)
        );
        assert_eq!(
            text_clipboard_action(gtk4::gdk::Key::X, primary),
            Some(TextClipboardAction::Cut)
        );
        assert_eq!(
            text_clipboard_action(gtk4::gdk::Key::v, primary),
            Some(TextClipboardAction::Paste)
        );
        assert_eq!(
            text_clipboard_action(gtk4::gdk::Key::c, gtk4::gdk::ModifierType::SHIFT_MASK),
            None
        );
    }

    #[test]
    fn recognizes_text_history_shortcuts() {
        let primary = crate::shortcuts::PRIMARY_MODIFIER;
        assert_eq!(
            text_history_action(gtk4::gdk::Key::z, primary),
            Some(TextHistoryAction::Undo)
        );
        assert_eq!(
            text_history_action(gtk4::gdk::Key::y, primary),
            Some(TextHistoryAction::Redo)
        );
        assert_eq!(
            text_history_action(
                gtk4::gdk::Key::Z,
                primary | gtk4::gdk::ModifierType::SHIFT_MASK,
            ),
            Some(TextHistoryAction::Redo)
        );
        assert_eq!(
            text_history_action(gtk4::gdk::Key::z, gtk4::gdk::ModifierType::SHIFT_MASK),
            None
        );
    }
}
