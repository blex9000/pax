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

        // Right side: info bar + notebook + status bar
        let editor_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        editor_area.append(&tabs_rc.info_bar_container);

        // The SourceView goes in a scrolled window below the notebook
        let source_scroll = gtk4::ScrolledWindow::new();
        source_scroll.set_child(Some(&tabs_rc.source_view));
        source_scroll.set_vexpand(true);
        source_scroll.set_hexpand(true);

        editor_area.append(&tabs_rc.notebook);
        editor_area.append(&source_scroll);
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

        // Keybindings: Ctrl+S to save, Ctrl+B to toggle sidebar, Ctrl+P fuzzy finder
        {
            let state_c = state.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            let tabs_save = tabs_rc.clone();
            let sidebar_ref = sidebar.clone();
            let fuzzy_finder_ref = Rc::new(fuzzy_finder);
            key_ctrl.connect_key_pressed(move |_, key, _, modifier| {
                if modifier.contains(gtk4::gdk::ModifierType::CONTROL_MASK) {
                    match key {
                        gtk4::gdk::Key::s => {
                            tabs_save.save_active(&state_c);
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::b => {
                            let mut st = state_c.borrow_mut();
                            st.sidebar_visible = !st.sidebar_visible;
                            sidebar_ref.set_visible(st.sidebar_visible);
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::p => {
                            fuzzy_finder_ref.show();
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
