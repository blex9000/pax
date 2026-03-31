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
    #[allow(dead_code)]
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

        // Apply theme scheme
        if let Some(buf) = source_view.buffer().downcast_ref::<sourceview5::Buffer>() {
            let theme = crate::theme::current_theme();
            let scheme_id = theme.sourceview_scheme();
            let fallback_id = theme.sourceview_scheme_fallback();
            let scheme_manager = sourceview5::StyleSchemeManager::default();
            if let Some(scheme) = scheme_manager.scheme(scheme_id)
                .or_else(|| scheme_manager.scheme(fallback_id))
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

        // Switch page: update SourceView buffer and status bar when tab changes
        {
            let state_c = state.clone();
            let sv = source_view.clone();
            let lang_l = status_lang.clone();
            let mod_l = status_modified.clone();
            notebook.connect_switch_page(move |_nb, _page, page_num| {
                if page_num == 0 { return; } // welcome page
                let idx = (page_num - 1) as usize;
                let mut st = state_c.borrow_mut();
                if let Some(open_file) = st.open_files.get(idx) {
                    sv.set_buffer(Some(&open_file.buffer));
                    if let Some(l) = open_file.buffer.language() {
                        lang_l.set_text(&l.name());
                    } else {
                        lang_l.set_text("Plain Text");
                    }
                    mod_l.set_text(if open_file.modified { "\u{25CF} Modified" } else { "" });
                }
                st.active_tab = Some(idx);
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
        let theme = crate::theme::current_theme();
        let scheme_id = theme.sourceview_scheme();
        let fallback_id = theme.sourceview_scheme_fallback();
        let scheme_manager = sourceview5::StyleSchemeManager::default();
        if let Some(scheme) = scheme_manager.scheme(scheme_id)
            .or_else(|| scheme_manager.scheme(fallback_id))
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
            let path_for_dirty = path.to_path_buf();
            buf.connect_changed(move |buf| {
                let mut st = state_c.borrow_mut();
                if let Some(file_idx) = st.open_files.iter().position(|f| f.path == path_for_dirty) {
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
            let path_for_close = path.to_path_buf();
            close_btn.connect_clicked(move |_| {
                let mut st = state_c.borrow_mut();
                if let Some(idx) = st.open_files.iter().position(|f| f.path == path_for_close) {
                    st.open_files.remove(idx);
                    nb.remove_page(Some((idx + 1) as u32)); // +1 for welcome
                    if st.open_files.is_empty() {
                        st.active_tab = None;
                        nb.set_show_tabs(false);
                        nb.set_current_page(Some(0)); // show welcome
                    } else {
                        let new_idx = idx.min(st.open_files.len() - 1);
                        st.active_tab = Some(new_idx);
                    }
                }
            });
        }

        // Middle-click to close tab
        {
            let state_c = state.clone();
            let nb = self.notebook.clone();
            let path_for_middle = path.to_path_buf();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(2); // middle button
            gesture.connect_released(move |_, _, _, _| {
                let mut st = state_c.borrow_mut();
                if let Some(idx) = st.open_files.iter().position(|f| f.path == path_for_middle) {
                    st.open_files.remove(idx);
                    nb.remove_page(Some((idx + 1) as u32));
                    if st.open_files.is_empty() {
                        st.active_tab = None;
                        nb.set_show_tabs(false);
                        nb.set_current_page(Some(0));
                    } else {
                        let new_idx = idx.min(st.open_files.len() - 1);
                        st.active_tab = Some(new_idx);
                        nb.set_current_page(Some((new_idx + 1) as u32));
                    }
                }
            });
            tab_box.add_controller(gesture);
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

    /// Show a side-by-side diff view for the given file.
    pub fn show_diff(&self, root: &Path, file_path: &Path) {
        use super::git_status::compute_diff;

        let _hunks = compute_diff(root, file_path);

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
        if let Some(lang) = lang_manager.guess_language(Some(file_path), None::<&str>) {
            old_buf.set_language(Some(&lang));
            new_buf.set_language(Some(&lang));
        }
        let theme = crate::theme::current_theme();
        let scheme_id = theme.sourceview_scheme();
        let fallback_id = theme.sourceview_scheme_fallback();
        let scheme_manager = sourceview5::StyleSchemeManager::default();
        if let Some(scheme) = scheme_manager.scheme(scheme_id)
            .or_else(|| scheme_manager.scheme(fallback_id))
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

        // Sync scrolling with guard to prevent infinite feedback loop
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
            let diff_widget = diff_box.clone();
            close_btn.connect_clicked(move |_| {
                if let Some(page) = nb.page_num(&diff_widget) {
                    nb.remove_page(Some(page));
                }
            });
        }
    }

    /// Save the currently active file.
    pub fn save_active(&self, state: &Rc<RefCell<EditorState>>, root: &Path) {
        {
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
        // Update gutter marks after save
        self.update_gutter_marks(root, state);
    }

    /// Close the active tab. If modified, save first then close.
    pub fn close_active_tab(&self, state: &Rc<RefCell<EditorState>>, root: &Path) {
        let idx = match state.borrow().active_tab {
            Some(i) => i,
            None => return,
        };

        let is_modified = state.borrow().open_files.get(idx)
            .map(|f| f.modified)
            .unwrap_or(false);

        if is_modified {
            self.save_active(state, root);
        }
        self.remove_tab(idx, state);
    }

    /// Remove the tab at the given index from the notebook and state.
    pub fn remove_tab(&self, idx: usize, state: &Rc<RefCell<EditorState>>) {
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

        let file_path = open_file.path.clone();
        drop(st);

        let hunks = compute_diff(root, &file_path);
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

            // Apply tag to new lines in the working copy
            let mut line_num = hunk.new_start.saturating_sub(1);
            for line in &hunk.new_lines {
                if line.starts_with('+') {
                    if line_num < buf.line_count() as usize {
                        let start = buf.iter_at_line(line_num as i32).unwrap_or(buf.start_iter());
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
}

fn get_mtime(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
        .unwrap_or(0)
}
