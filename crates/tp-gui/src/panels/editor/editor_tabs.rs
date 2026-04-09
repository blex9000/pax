use gtk4::prelude::*;
use sourceview5::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use super::EditorState;

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
    if !modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK) {
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
    let ctrl = modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
    let shift = modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK);

    if !ctrl {
        return None;
    }

    match key {
        gtk4::gdk::Key::z if !shift => Some(TextHistoryAction::Undo),
        gtk4::gdk::Key::y if !shift => Some(TextHistoryAction::Redo),
        gtk4::gdk::Key::Z if shift => Some(TextHistoryAction::Redo),
        _ => None,
    }
}

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

fn install_text_history_shortcuts<W: IsA<gtk4::Widget>>(widget: &W, buffer: &sourceview5::Buffer) {
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

/// Manages the Notebook tabs and SourceView buffers.
/// The notebook is used ONLY as a tab bar — its page content is always empty.
/// Actual content (welcome message or source code) lives in `content_stack`.
pub struct EditorTabs {
    pub notebook: gtk4::Notebook,
    pub source_view: sourceview5::View,
    /// Stack switching between "welcome" and "editor" content.
    pub content_stack: gtk4::Stack,
    /// Search/replace bar (hidden by default, toggled with Ctrl+F / Ctrl+H).
    pub search_bar: gtk4::Box,
    pub status_bar: gtk4::Box,
    pub info_bar_container: gtk4::Box,
    status_lang: gtk4::Label,
    #[allow(dead_code)]
    status_pos: gtk4::Label,
    status_modified: gtk4::Label,
    pub search_entry: gtk4::SearchEntry,
    #[allow(dead_code)]
    pub replace_entry: gtk4::Entry,
    pub replace_row: gtk4::Box,
    #[allow(dead_code)]
    search_settings: sourceview5::SearchSettings,
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
        install_text_clipboard_shortcuts(&source_view);
        if let Some(buffer) = source_view.buffer().downcast_ref::<sourceview5::Buffer>() {
            install_text_history_shortcuts(&source_view, buffer);
        }

        // Apply and register for theme updates
        if let Some(buf) = source_view.buffer().downcast_ref::<sourceview5::Buffer>() {
            crate::theme::register_sourceview_buffer(buf);
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

        // Track cursor position
        {
            let pos_label = status_pos.clone();
            source_view
                .buffer()
                .connect_notify_local(Some("cursor-position"), move |buf, _| {
                    let iter = buf.iter_at_offset(buf.cursor_position());
                    let line = iter.line() + 1;
                    let col = iter.line_offset() + 1;
                    pos_label.set_text(&format!("Ln {}, Col {}", line, col));
                });
        }

        // Switch page: update SourceView buffer and status bar when tab changes.
        // Uses try_borrow_mut to avoid panic when triggered by remove_page/set_current_page
        // while another closure already holds a borrow.
        {
            let state_c = state.clone();
            let sv = source_view.clone();
            let lang_l = status_lang.clone();
            let mod_l = status_modified.clone();
            notebook.connect_switch_page(move |_nb, _page, page_num| {
                let idx = page_num as usize;
                if let Ok(mut st) = state_c.try_borrow_mut() {
                    if let Some(open_file) = st.open_files.get(idx) {
                        sv.set_buffer(Some(&open_file.buffer));
                        if let Some(l) = open_file.buffer.language() {
                            lang_l.set_text(&l.name());
                        } else {
                            lang_l.set_text("Plain Text");
                        }
                        mod_l.set_text(if open_file.modified {
                            "\u{25CF} Modified"
                        } else {
                            ""
                        });
                    }
                    st.active_tab = Some(idx);
                }
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

        // Enter → next, Shift+Enter → prev
        {
            let get_ctx = ensure_ctx.clone();
            let sv = source_view.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.connect_key_pressed(move |_, key, _, modifier| {
                if key == gtk4::gdk::Key::Return || key == gtk4::gdk::Key::KP_Enter {
                    let ctx = get_ctx();
                    let buf = sv.buffer();
                    if modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK) {
                        let (start, _) = buf.selection_bounds().unwrap_or_else(|| {
                            let iter = buf.iter_at_offset(buf.cursor_position());
                            (iter.clone(), iter)
                        });
                        if let Some((sm, em, _)) = ctx.backward(&start) {
                            buf.select_range(&sm, &em);
                            sv.scroll_to_iter(&mut sm.clone(), 0.1, false, 0.0, 0.0);
                        }
                    } else {
                        let (_, end) = buf.selection_bounds().unwrap_or_else(|| {
                            let iter = buf.iter_at_offset(buf.cursor_position());
                            (iter.clone(), iter)
                        });
                        if let Some((sm, em, _)) = ctx.forward(&end) {
                            buf.select_range(&sm, &em);
                            sv.scroll_to_iter(&mut sm.clone(), 0.1, false, 0.0, 0.0);
                        }
                    }
                    return gtk4::glib::Propagation::Stop;
                }
                gtk4::glib::Propagation::Proceed
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

        // ── Content stack ────────────────────────────────────────────
        let content_stack = gtk4::Stack::new();
        content_stack.set_vexpand(true);
        content_stack.set_hexpand(true);

        let welcome = gtk4::Label::new(Some(
            "Open a file from the sidebar\nor press Ctrl+P to search",
        ));
        welcome.add_css_class("dim-label");
        welcome.set_vexpand(true);
        welcome.set_valign(gtk4::Align::Center);
        content_stack.add_named(&welcome, Some("welcome"));
        content_stack.add_named(&source_scroll, Some("editor"));
        content_stack.set_visible_child_name("welcome");

        Self {
            notebook,
            source_view,
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

        // Check if already open
        {
            let st = state.borrow();
            if let Some(idx) = st.open_files.iter().position(|f| f.path == path) {
                self.notebook.set_current_page(Some(idx as u32));
                self.switch_to_buffer(idx, state);
                return Some(idx);
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

        // Detect language
        let lang_manager = sourceview5::LanguageManager::default();
        if let Some(lang) = lang_manager.guess_language(Some(path), None::<&str>) {
            buf.set_language(Some(&lang));
        }

        // Apply scheme and register for live theme updates
        crate::theme::register_sourceview_buffer(&buf);

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

        // Add to state
        let idx = {
            let mut st = state.borrow_mut();
            let saved_content = Rc::new(RefCell::new(content.clone()));
            st.open_files.push(super::OpenFile {
                path: path.to_path_buf(),
                buffer: buf.clone(),
                modified: false,
                last_disk_mtime: mtime,
                saved_content: saved_content.clone(),
            });
            st.active_tab = Some(st.open_files.len() - 1);
            st.open_files.len() - 1
        };

        // Track dirty state
        {
            let state_c = state.clone();
            let dot_c = dot.clone();
            let mod_label = self.status_modified.clone();
            let path_for_dirty = path.to_path_buf();
            // Compare buffer content against saved content for accurate dirty detection
            let saved_for_changed = state.borrow().open_files[idx].saved_content.clone();
            buf.connect_changed(move |buf| {
                let current = buf
                    .text(&buf.start_iter(), &buf.end_iter(), false)
                    .to_string();
                let is_dirty = current != *saved_for_changed.borrow();
                dot_c.set_text(if is_dirty { "\u{25CF} " } else { "" });
                mod_label.set_text(if is_dirty { "\u{25CF} Modified" } else { "" });
                if let Ok(mut st) = state_c.try_borrow_mut() {
                    if let Some(file_idx) =
                        st.open_files.iter().position(|f| f.path == path_for_dirty)
                    {
                        st.open_files[file_idx].modified = is_dirty;
                    }
                }
            });
        }

        // Close button
        {
            let state_c = state.clone();
            let nb = self.notebook.clone();
            let cs = self.content_stack.clone();
            let path_for_close = path.to_path_buf();
            let close_do_it = {
                let state_c = state_c.clone();
                let nb = nb.clone();
                let cs = cs.clone();
                let path_for_close = path_for_close.clone();
                Rc::new(move || {
                    let (empty_after, new_idx);
                    {
                        let mut st = state_c.borrow_mut();
                        if let Some(idx) =
                            st.open_files.iter().position(|f| f.path == path_for_close)
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
                            if empty_after {
                                nb.set_show_tabs(false);
                                cs.set_visible_child_name("welcome");
                            } else {
                                nb.set_current_page(Some(new_idx as u32));
                            }
                        }
                    }
                })
            };
            close_btn.connect_clicked(move |btn| {
                let is_modified = {
                    let st = state_c.borrow();
                    st.open_files
                        .iter()
                        .find(|f| f.path == path_for_close)
                        .map(|f| f.modified)
                        .unwrap_or(false)
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

                    let file_name = path_for_close
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "file".to_string());
                    let msg =
                        gtk4::Label::new(Some(&format!("\"{}\" has unsaved changes.", file_name)));
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
                        let pfc = path_for_close.clone();
                        let close = close_do_it.clone();
                        discard_btn.connect_clicked(move |_| {
                            if let Ok(mut st) = sc.try_borrow_mut() {
                                if let Some(f) = st.open_files.iter_mut().find(|f| f.path == pfc) {
                                    f.modified = false;
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
                        let pfc = path_for_close.clone();
                        let close = close_do_it.clone();
                        save_btn.connect_clicked(move |_| {
                            let save_result = {
                                let st = sc.borrow();
                                let backend = st.backend.clone();
                                if let Some(f) = st.open_files.iter().find(|f| f.path == pfc) {
                                    let text = f
                                        .buffer
                                        .text(&f.buffer.start_iter(), &f.buffer.end_iter(), false)
                                        .to_string();
                                    backend
                                        .write_file(&f.path, &text)
                                        .map(|_| (f.path.clone(), text))
                                } else {
                                    Err("File not found".to_string())
                                }
                            };
                            match save_result {
                                Ok((fpath, text)) => {
                                    if let Ok(mut st) = sc.try_borrow_mut() {
                                        if let Some(f) =
                                            st.open_files.iter_mut().find(|f| f.path == fpath)
                                        {
                                            f.modified = false;
                                            f.last_disk_mtime = get_mtime(&f.path);
                                            *f.saved_content.borrow_mut() = text;
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

        // Switch to this buffer
        self.switch_to_buffer(idx, state);
        self.notebook.set_current_page(Some(idx as u32));

        Some(idx)
    }

    /// Switch the SourceView to display the buffer at the given index.
    pub fn switch_to_buffer(&self, idx: usize, state: &Rc<RefCell<EditorState>>) {
        let st = state.borrow();
        if let Some(open_file) = st.open_files.get(idx) {
            self.source_view.set_buffer(Some(&open_file.buffer));
            if let Some(lang) = open_file.buffer.language() {
                self.status_lang.set_text(&lang.name());
            } else {
                self.status_lang.set_text("Plain Text");
            }
            self.status_modified.set_text(if open_file.modified {
                "\u{25CF} Modified"
            } else {
                ""
            });
        }
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

        // Highlight changed lines using similar
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

        let make_sv = |buf: &sourceview5::Buffer, editable: bool| -> gtk4::ScrolledWindow {
            let view = sourceview5::View::with_buffer(buf);
            view.set_editable(editable);
            view.set_show_line_numbers(true);
            view.set_monospace(true);
            view.set_left_margin(4);
            install_text_clipboard_shortcuts(&view);
            install_text_history_shortcuts(&view, buf);
            if editable {
                view.set_auto_indent(true);
                view.set_tab_width(4);
            }
            let scroll = gtk4::ScrolledWindow::new();
            scroll.set_child(Some(&view));
            scroll.set_vexpand(true);
            scroll.set_hexpand(true);
            scroll
        };

        // Left: HEAD version (read-only), Right: working version (editable)
        let old_scroll = make_sv(&old_buf, false);
        let new_scroll = make_sv(&new_buf, true);

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
        let old_label =
            gtk4::Label::new(Some(&format!("← PRIMA  {}  (HEAD)", rel.to_string_lossy())));
        old_label.add_css_class("dim-label");
        old_label.set_hexpand(true);
        old_label.set_margin_start(8);
        let new_label = gtk4::Label::new(Some(&format!(
            "→ DOPO  {}  (working)",
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
        paned.set_start_child(Some(&old_scroll));
        paned.set_end_child(Some(&new_scroll));
        diff_box.append(&paned);

        // Save working side on Ctrl+S (via key controller on the diff_box)
        {
            let fp = file_path.to_path_buf();
            let nb = new_buf.clone();
            let be = backend.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.connect_key_pressed(move |_, key, _, modifier| {
                if modifier.contains(gtk4::gdk::ModifierType::CONTROL_MASK)
                    && key == gtk4::gdk::Key::s
                {
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
            if let Some(idx) = st.active_tab {
                if let Some(open_file) = st.open_files.get_mut(idx) {
                    let buf = &open_file.buffer;
                    let text = buf
                        .text(&buf.start_iter(), &buf.end_iter(), false)
                        .to_string();
                    if let Err(e) = backend.write_file(&open_file.path, &text) {
                        tracing::error!("Failed to save {}: {}", open_file.path.display(), e);
                        return;
                    }
                    open_file.modified = false;
                    open_file.last_disk_mtime = get_mtime(&open_file.path);
                    // Update saved content so dirty detection compares against new save
                    *open_file.saved_content.borrow_mut() = text;
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
            .map(|f| f.modified)
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

        let buf = &open_file.buffer;

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
        let buf = &open_file.buffer;

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
        view.set_editable(false);
        view.set_show_line_numbers(true);
        view.set_monospace(true);
        view.set_left_margin(4);
        install_text_clipboard_shortcuts(&view);
        install_text_history_shortcuts(&view, buf);
        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_child(Some(&view));
        scroll.set_vexpand(true);
        scroll.set_hexpand(true);
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
        "← PRIMA  {}  ({})",
        file_rel,
        &parent[..parent.len().min(8)]
    )));
    old_label.add_css_class("dim-label");
    old_label.set_hexpand(true);
    old_label.set_margin_start(8);
    let new_label = gtk4::Label::new(Some(&format!(
        "→ DOPO  {}  ({})",
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
    fn recognizes_text_clipboard_shortcuts() {
        assert_eq!(
            text_clipboard_action(gtk4::gdk::Key::c, gtk4::gdk::ModifierType::CONTROL_MASK,),
            Some(TextClipboardAction::Copy)
        );
        assert_eq!(
            text_clipboard_action(gtk4::gdk::Key::X, gtk4::gdk::ModifierType::CONTROL_MASK,),
            Some(TextClipboardAction::Cut)
        );
        assert_eq!(
            text_clipboard_action(gtk4::gdk::Key::v, gtk4::gdk::ModifierType::CONTROL_MASK,),
            Some(TextClipboardAction::Paste)
        );
        assert_eq!(
            text_clipboard_action(gtk4::gdk::Key::c, gtk4::gdk::ModifierType::SHIFT_MASK),
            None
        );
    }

    #[test]
    fn recognizes_text_history_shortcuts() {
        assert_eq!(
            text_history_action(gtk4::gdk::Key::z, gtk4::gdk::ModifierType::CONTROL_MASK),
            Some(TextHistoryAction::Undo)
        );
        assert_eq!(
            text_history_action(gtk4::gdk::Key::y, gtk4::gdk::ModifierType::CONTROL_MASK),
            Some(TextHistoryAction::Redo)
        );
        assert_eq!(
            text_history_action(
                gtk4::gdk::Key::Z,
                gtk4::gdk::ModifierType::CONTROL_MASK | gtk4::gdk::ModifierType::SHIFT_MASK,
            ),
            Some(TextHistoryAction::Redo)
        );
        assert_eq!(
            text_history_action(gtk4::gdk::Key::z, gtk4::gdk::ModifierType::SHIFT_MASK),
            None
        );
    }
}
