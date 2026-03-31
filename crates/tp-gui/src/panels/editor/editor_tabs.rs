use gtk4::prelude::*;
use sourceview5::prelude::*;
use std::path::Path;
use std::cell::RefCell;
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
    pub fn new(_state: Rc<RefCell<EditorState>>) -> Self {
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
        if let Some(lang) = lang_manager.guess_language(Some(path), None::<&str>) {
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
        let _page_idx = self.notebook.append_page(&page_widget, Some(&tab_box));
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
