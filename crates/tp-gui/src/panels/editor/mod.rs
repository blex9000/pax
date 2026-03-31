#[cfg(feature = "sourceview")]
mod editor_tabs;
// Submodules for future tasks (stubs for now)
#[cfg(feature = "sourceview")]
pub mod file_tree;
#[cfg(feature = "sourceview")]
pub mod git_status;
#[cfg(feature = "sourceview")]
pub mod file_watcher;
#[cfg(feature = "sourceview")]
pub mod fuzzy_finder;
#[cfg(feature = "sourceview")]
pub mod project_search;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use super::PanelBackend;

/// State shared across all editor sub-components.
#[derive(Debug)]
pub struct EditorState {
    pub root_dir: PathBuf,
    #[cfg(feature = "sourceview")]
    pub open_files: Vec<OpenFile>,
    pub active_tab: Option<usize>,
    pub sidebar_visible: bool,
    pub sidebar_mode: SidebarMode,
}

#[cfg(feature = "sourceview")]
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
        let tabs_rc = Rc::new(tabs);

        // Right side: info bar + notebook (tab bar) + search bar + content stack + status bar
        let editor_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        editor_area.append(&tabs_rc.info_bar_container);
        editor_area.append(&tabs_rc.notebook);
        editor_area.append(&tabs_rc.search_bar);
        editor_area.append(&tabs_rc.content_stack);
        editor_area.append(&tabs_rc.status_bar);

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

        let search_btn = gtk4::ToggleButton::new();
        search_btn.set_icon_name("edit-find-symbolic");
        search_btn.add_css_class("flat");
        search_btn.set_tooltip_text(Some("Search in files (Ctrl+Shift+F)"));
        search_btn.set_group(Some(&files_btn));

        activity_bar.append(&files_btn);
        activity_bar.append(&git_btn);
        activity_bar.append(&search_btn);
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

        // Git status view
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

        // Project-wide search view
        let project_search = project_search::ProjectSearch::new(
            &PathBuf::from(root_dir),
            Rc::new({
                let state_c = state.clone();
                let tabs_c = tabs_rc.clone();
                move |path, line_num| {
                    // Open file and scroll to line
                    tabs_c.open_file(path, &state_c);
                    let st = state_c.borrow();
                    if let Some(idx) = st.active_tab {
                        if let Some(open_file) = st.open_files.get(idx) {
                            if let Some(iter) = open_file.buffer.iter_at_line((line_num as i32) - 1) {
                                open_file.buffer.place_cursor(&iter);
                                drop(st);
                                tabs_c.source_view.scroll_to_iter(&mut iter.clone(), 0.1, false, 0.0, 0.0);
                            }
                        }
                    }
                }
            }),
        );

        // Sidebar stack to switch between file tree, git view, and search
        let sidebar_stack = gtk4::Stack::new();
        sidebar_stack.add_named(&file_tree.widget, Some("files"));
        sidebar_stack.add_named(&git_status_view.widget, Some("git"));
        sidebar_stack.add_named(&project_search.widget, Some("search"));
        sidebar.append(&sidebar_stack);

        // Connect activity bar toggle buttons
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
        {
            let stack = sidebar_stack.clone();
            let ps = project_search;
            search_btn.connect_toggled(move |btn| {
                if btn.is_active() {
                    stack.set_visible_child_name("search");
                    ps.focus_entry();
                }
            });
        }

        // Fuzzy finder overlay
        let fuzzy_finder = fuzzy_finder::FuzzyFinder::new(
            &PathBuf::from(root_dir),
            file_tree.file_index.clone(),
            Rc::new({
                let state_c = state.clone();
                let tabs_c = tabs_rc.clone();
                move |path| { tabs_c.open_file(path, &state_c); }
            }),
        );

        // Paned: sidebar | editor
        let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
        paned.set_start_child(Some(&sidebar));
        paned.set_end_child(Some(&editor_area));
        paned.set_position(200);
        paned.set_shrink_start_child(false);
        paned.set_resize_start_child(false);

        // Overlay: paned + fuzzy finder on top
        let main_overlay = gtk4::Overlay::new();
        main_overlay.set_child(Some(&paned));
        main_overlay.add_overlay(&fuzzy_finder.overlay);

        let widget = main_overlay.upcast::<gtk4::Widget>();

        // Keybindings: Ctrl+S save, Ctrl+W close, Ctrl+Tab next tab, Ctrl+B sidebar, Ctrl+P fuzzy finder, Ctrl+Shift+G git view
        {
            let state_c = state.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            let tabs_ref = tabs_rc.clone();
            let sidebar_ref = sidebar.clone();
            let fuzzy_finder_ref = Rc::new(fuzzy_finder);
            let git_btn_ref = git_btn.clone();
            let search_btn_ref = search_btn.clone();
            key_ctrl.connect_key_pressed(move |_, key, _, modifier| {
                if modifier.contains(gtk4::gdk::ModifierType::CONTROL_MASK) {
                    match key {
                        gtk4::gdk::Key::s => {
                            let root = state_c.borrow().root_dir.clone();
                            tabs_ref.save_active(&state_c, &root);
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::w => {
                            let root = state_c.borrow().root_dir.clone();
                            tabs_ref.close_active_tab(&state_c, &root);
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::Tab => {
                            let st = state_c.borrow();
                            if let Some(idx) = st.active_tab {
                                let count = st.open_files.len();
                                if count > 0 {
                                    let next = (idx + 1) % count;
                                    drop(st);
                                    tabs_ref.notebook.set_current_page(Some(next as u32));
                                }
                            }
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::b => {
                            let mut st = state_c.borrow_mut();
                            st.sidebar_visible = !st.sidebar_visible;
                            sidebar_ref.set_visible(st.sidebar_visible);
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::f if modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK) => {
                            // Ctrl+Shift+F → search in project files
                            search_btn_ref.set_active(true);
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::f => {
                            tabs_ref.show_search();
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::h => {
                            tabs_ref.show_replace();
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::p => {
                            fuzzy_finder_ref.show();
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::g if modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK) => {
                            git_btn_ref.set_active(true);
                            return gtk4::glib::Propagation::Stop;
                        }
                        _ => {}
                    }
                }
                gtk4::glib::Propagation::Proceed
            });
            widget.add_controller(key_ctrl);
        }

        // Start file watchers
        {
            let file_tree_ref = file_tree;
            file_watcher::start_watchers(
                state.clone(),
                tabs_rc.info_bar_container.clone(),
                Rc::new(move || {
                    file_tree_ref.refresh();
                }),
                Rc::new(move |git_output: String| {
                    git_status_view.update(&git_output);
                }),
            );
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
