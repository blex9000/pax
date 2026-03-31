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

        // Right side: info bar + notebook + status bar
        let editor_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        editor_area.append(&tabs.info_bar_container);

        // The SourceView goes in a scrolled window below the notebook
        let source_scroll = gtk4::ScrolledWindow::new();
        source_scroll.set_child(Some(&tabs.source_view));
        source_scroll.set_vexpand(true);
        source_scroll.set_hexpand(true);

        editor_area.append(&tabs.notebook);
        editor_area.append(&source_scroll);
        editor_area.append(&tabs.status_bar);

        // Sidebar placeholder (file tree comes in Task 3)
        let sidebar = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        sidebar.set_width_request(200);

        let sidebar_label = gtk4::Label::new(Some("Files"));
        sidebar_label.add_css_class("dim-label");
        sidebar_label.set_margin_top(16);
        sidebar.append(&sidebar_label);

        let dir_label = gtk4::Label::new(Some(root_dir));
        dir_label.add_css_class("dim-label");
        dir_label.add_css_class("caption");
        dir_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
        dir_label.set_margin_top(4);
        sidebar.append(&dir_label);

        // Paned: sidebar | editor
        let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
        paned.set_start_child(Some(&sidebar));
        paned.set_end_child(Some(&editor_area));
        paned.set_position(200);
        paned.set_shrink_start_child(false);
        paned.set_resize_start_child(false);

        let widget = paned.upcast::<gtk4::Widget>();

        // Keybindings: Ctrl+S to save
        {
            let state_c = state.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            let tabs_rc = Rc::new(tabs);
            let tabs_save = tabs_rc.clone();
            key_ctrl.connect_key_pressed(move |_, key, _, modifier| {
                if modifier.contains(gtk4::gdk::ModifierType::CONTROL_MASK) {
                    match key {
                        gtk4::gdk::Key::s => {
                            tabs_save.save_active(&state_c);
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
