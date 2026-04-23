#[cfg(feature = "sourceview")]
mod editor_tabs;
#[cfg(feature = "sourceview")]
mod text_context_menu;
// Submodules for future tasks (stubs for now)
pub mod file_backend;
#[cfg(feature = "sourceview")]
pub mod file_tree;
#[cfg(feature = "sourceview")]
pub mod file_watcher;
#[cfg(feature = "sourceview")]
pub mod fuzzy_finder;
#[cfg(feature = "sourceview")]
pub mod git_log;
#[cfg(feature = "sourceview")]
pub mod image_view;
#[cfg(feature = "sourceview")]
pub mod markdown_view;
#[cfg(feature = "sourceview")]
pub mod notes_ruler;
#[cfg(feature = "sourceview")]
pub mod notes_state;
#[cfg(feature = "sourceview")]
pub mod tab_content;
#[cfg(feature = "sourceview")]
pub mod git_status;
#[cfg(feature = "sourceview")]
pub mod project_search;
#[cfg(feature = "sourceview")]
pub mod task;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use super::PanelBackend;
use gtk4::prelude::*;

/// A position in a file (for navigation history).
#[cfg(feature = "sourceview")]
#[derive(Debug, Clone)]
pub struct FilePosition {
    pub path: PathBuf,
    pub line: i32,
}

/// State shared across all editor sub-components.
impl std::fmt::Debug for EditorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorState")
            .field("root_dir", &self.root_dir)
            .finish()
    }
}
pub struct EditorState {
    pub root_dir: PathBuf,
    #[cfg(feature = "sourceview")]
    pub open_files: Vec<OpenFile>,
    pub active_tab: Option<usize>,
    pub sidebar_visible: bool,
    pub sidebar_mode: SidebarMode,
    /// File backend — Local for local projects, SSH for remote.
    pub backend: Arc<dyn file_backend::FileBackend>,
    /// Watcher poll interval in seconds (configurable per panel).
    pub poll_interval: u64,
    /// Back stack: positions you can go back to.
    #[cfg(feature = "sourceview")]
    pub nav_back: Vec<FilePosition>,
    /// Forward stack: positions you can go forward to (after going back).
    #[cfg(feature = "sourceview")]
    pub nav_forward: Vec<FilePosition>,
    /// Recent files history (last 10 focused files).
    #[cfg(feature = "sourceview")]
    pub recent_files: Vec<PathBuf>,
    /// Fired after any mutation that affects the activity-bar button
    /// sensitivities (active_tab, nav stacks, recent_files). Set once during
    /// panel setup.
    #[cfg(feature = "sourceview")]
    pub on_nav_state_changed: Option<Rc<dyn Fn()>>,
    /// pax-db `record_key` of the workspace owning this editor. Used to
    /// scope workspace metadata (notes, future types). Empty when the
    /// editor is spawned outside a workspace (tests, standalone runs).
    pub record_key: String,
}

/// Invoke the nav-state callback if one is installed, swallowing borrow
/// conflicts the caller can't reasonably handle.
#[cfg(feature = "sourceview")]
pub(crate) fn fire_nav_state_changed(state: &Rc<RefCell<EditorState>>) {
    let cb = state.borrow().on_nav_state_changed.clone();
    if let Some(cb) = cb {
        cb();
    }
}

#[cfg(feature = "sourceview")]
#[derive(Debug)]
pub struct OpenFile {
    /// Stable identifier for this tab, independent of `path` so that
    /// long-lived closures (dirty-tracking, close button, external rename
    /// propagation) keep matching the right tab after a rename.
    pub tab_id: u64,
    pub path: PathBuf,
    pub last_disk_mtime: u64,
    /// The label widget inside the tab bar that shows the file name. Kept as
    /// a direct reference so rename propagation can update it in O(1) without
    /// traversing the notebook's tab widget tree.
    pub name_label: gtk4::Label,
    /// Per-tab content (source / markdown / image).
    pub content: tab_content::TabContent,
}

#[cfg(feature = "sourceview")]
impl OpenFile {
    /// Source-code buffer, or `None` for non-source tabs. Markdown tabs have
    /// a writable buffer too but not a *source-code* one — use
    /// `writable_buffer()` for save/dirty tracking that applies to both.
    pub fn source_buffer(&self) -> Option<&sourceview5::Buffer> {
        self.content.source_buffer()
    }

    /// Writable buffer (source or markdown-source mode). `None` for image tabs.
    pub fn writable_buffer(&self) -> Option<&sourceview5::Buffer> {
        self.content.writable_buffer()
    }

    pub fn modified(&self) -> bool {
        self.content.is_modified()
    }

    pub fn set_modified(&mut self, v: bool) {
        self.content.set_modified(v);
    }

    /// Dirty-tracking cell. `None` for tabs without a writable buffer (image).
    pub fn saved_content(&self) -> Option<&Rc<RefCell<String>>> {
        self.content.saved_content()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SidebarMode {
    Files,
    Git,
}

/// Embedded code editor panel with file tree, tabs, and git integration.
/// Supports both local directories and remote projects via SSHFS.
#[cfg(feature = "sourceview")]
#[derive(Debug)]
pub struct CodeEditorPanel {
    widget: gtk4::Widget,
    state: Rc<RefCell<EditorState>>,
    /// SSH connection label for remote panels (e.g. "user@host").
    ssh_info: Option<String>,
}

#[cfg(feature = "sourceview")]
impl CodeEditorPanel {
    /// Create a code editor for a remote project via SSH.
    /// Uses SshFileBackend (direct SSH commands with ControlMaster) — no SSHFS.
    pub fn new_remote(
        host: &str,
        port: u16,
        user: &str,
        password: Option<&str>,
        identity_file: Option<&str>,
        remote_path: &str,
        record_key: String,
    ) -> Self {
        let ssh_label = format!("{}@{}", user, host);

        // Create SSH backend — ControlMaster connection established in constructor
        let backend: Arc<dyn file_backend::FileBackend> =
            Arc::new(file_backend::SshFileBackend::new(
                remote_path,
                host,
                port,
                user,
                password,
                identity_file,
            ));

        let mut panel = Self::new_with_backend(remote_path, backend, record_key);
        panel.ssh_info = Some(ssh_label);
        panel
    }

    pub fn new(root_dir: &str, record_key: String) -> Self {
        let backend = Arc::new(file_backend::LocalFileBackend::new(&PathBuf::from(
            root_dir,
        )));
        let mut panel = Self::new_with_backend(root_dir, backend, record_key);
        panel.ssh_info = None;
        panel
    }

    fn new_with_backend(
        root_dir: &str,
        backend: Arc<dyn file_backend::FileBackend>,
        record_key: String,
    ) -> Self {
        let poll_secs = if backend.is_remote() { 5 } else { 2 };

        // Detect whether the project root is a git repository. We check both
        // .git/ (regular repo) and .git as a file (worktree / submodule
        // pointer). For SSHFS-mounted remote roots this still works because
        // the mount surfaces .git through the local filesystem path. When the
        // root is not a git project we hide the Git activity-bar buttons and
        // the file-tree's "Git History" context entry — there's nothing
        // useful behind them and showing them is misleading.
        let is_git_repo = std::path::Path::new(root_dir).join(".git").exists();
        let state = Rc::new(RefCell::new(EditorState {
            root_dir: PathBuf::from(root_dir),
            open_files: Vec::new(),
            active_tab: None,
            sidebar_visible: true,
            sidebar_mode: SidebarMode::Files,
            backend: backend.clone(),
            poll_interval: poll_secs,
            nav_back: Vec::new(),
            nav_forward: Vec::new(),
            recent_files: Vec::new(),
            on_nav_state_changed: None,
            record_key,
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
        sidebar.add_css_class("editor-sidebar-toolbar-surface");
        sidebar.set_width_request(150);

        // Activity bar: Files / Git toggle
        let activity_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
        activity_bar.add_css_class("editor-sidebar-toolbar");
        activity_bar.add_css_class("editor-file-tree-header");
        activity_bar.set_margin_start(2);
        activity_bar.set_margin_end(2);
        activity_bar.set_margin_top(0);
        activity_bar.set_margin_bottom(0);

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
        search_btn.set_tooltip_text(Some(
            "Search: content (Ctrl+Shift+F) or file name (Ctrl+Shift+P)",
        ));
        search_btn.set_group(Some(&files_btn));

        let history_btn = gtk4::ToggleButton::new();
        history_btn.set_icon_name("document-open-recent-symbolic");
        history_btn.add_css_class("flat");
        history_btn.set_tooltip_text(Some("Git History"));
        history_btn.set_group(Some(&files_btn));

        if !is_git_repo {
            git_btn.set_visible(false);
            history_btn.set_visible(false);
        }

        // Spacer to push nav buttons to the right
        let bar_spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        bar_spacer.set_hexpand(true);

        let nav_back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
        nav_back_btn.add_css_class("flat");
        nav_back_btn.set_tooltip_text(Some("Go Back (Alt+←)"));

        let nav_fwd_btn = gtk4::Button::from_icon_name("go-next-symbolic");
        nav_fwd_btn.add_css_class("flat");
        nav_fwd_btn.set_tooltip_text(Some("Go Forward (Alt+→)"));

        let recent_btn = gtk4::Button::from_icon_name("view-list-symbolic");
        recent_btn.add_css_class("flat");
        recent_btn.set_tooltip_text(Some("Recent Files (Ctrl+E)"));

        activity_bar.append(&files_btn);
        activity_bar.append(&git_btn);
        activity_bar.append(&history_btn);
        activity_bar.append(&search_btn);
        let nav_sep = gtk4::Separator::new(gtk4::Orientation::Vertical);
        nav_sep.set_margin_start(3);
        nav_sep.set_margin_end(2);
        nav_sep.set_margin_top(3);
        nav_sep.set_margin_bottom(3);
        activity_bar.append(&nav_sep);
        activity_bar.append(&nav_back_btn);
        activity_bar.append(&nav_fwd_btn);

        let reveal_btn = gtk4::Button::from_icon_name("find-location-symbolic");
        reveal_btn.add_css_class("flat");
        reveal_btn.set_tooltip_text(Some("Reveal active file in tree"));

        activity_bar.append(&reveal_btn);
        activity_bar.append(&bar_spacer);
        activity_bar.append(&recent_btn);

        let sidebar_hide_btn = gtk4::Button::from_icon_name("sidebar-show-symbolic");
        sidebar_hide_btn.add_css_class("flat");
        sidebar_hide_btn.set_tooltip_text(Some("Hide sidebar (Ctrl+B)"));
        activity_bar.append(&sidebar_hide_btn);

        // Disable activity-bar buttons when they would no-op. Updated
        // through `state.on_nav_state_changed` after any mutation to
        // active_tab / nav stacks / recent_files.
        {
            let nav_back = nav_back_btn.clone();
            let nav_fwd = nav_fwd_btn.clone();
            let reveal = reveal_btn.clone();
            let recent = recent_btn.clone();
            let state_c = state.clone();
            let refresh: Rc<dyn Fn()> = Rc::new(move || {
                let st = state_c.borrow();
                nav_back.set_sensitive(!st.nav_back.is_empty());
                nav_fwd.set_sensitive(!st.nav_forward.is_empty());
                reveal.set_sensitive(st.active_tab.is_some());
                recent.set_sensitive(!st.recent_files.is_empty());
            });
            state.borrow_mut().on_nav_state_changed = Some(refresh.clone());
            refresh();
        }

        let header_wrap = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        header_wrap.add_css_class("editor-file-tree-header-wrap");
        header_wrap.append(&activity_bar);
        sidebar.append(&header_wrap);

        // Git log (history) view — created early so file tree can reference it
        let git_log_view = Rc::new(git_log::GitLogView::new(
            &PathBuf::from(root_dir),
            Rc::new({
                let root_c = PathBuf::from(root_dir);
                let tabs_c = tabs_rc.clone();
                let be = backend.clone();
                move |hash| {
                    tabs_c.show_commit_diff(&root_c, hash, be.clone());
                }
            }),
            backend.clone(),
        ));

        // File tree with context menu
        let state_for_open = state.clone();
        let tabs_for_open = tabs_rc.clone();
        let state_for_rename = state.clone();
        let tabs_for_rename = tabs_rc.clone();
        let state_for_delete = state.clone();
        let tabs_for_delete = tabs_rc.clone();
        // Only wire the "git-history" context action when the root is a git
        // repo. Passing None makes file_tree skip the "Git History" menu
        // entry entirely (see file_tree.rs around the `if let Some(ref ctx)`
        // check) — which is what we want for non-git projects.
        let on_ctx_action: Option<file_tree::OnContextAction> = if is_git_repo {
            let root_for_ctx = PathBuf::from(root_dir);
            let glv_for_ctx = git_log_view.clone();
            let history_btn_for_ctx = history_btn.clone();
            Some(Rc::new(move |action, path| {
                if action == "git-history" {
                    let rel = path.strip_prefix(&root_for_ctx).unwrap_or(path);
                    glv_for_ctx.filter_by_file(&rel.to_string_lossy());
                    history_btn_for_ctx.set_active(true); // switches sidebar to history
                }
            }))
        } else {
            None
        };
        let file_tree = Rc::new(file_tree::FileTree::new_with_context(
            &PathBuf::from(root_dir),
            Rc::new(move |path| {
                tabs_for_open.open_file(path, &state_for_open);
            }),
            on_ctx_action,
            Some(Rc::new(move |old_path, new_path| {
                tabs_for_rename.rename_open_file(old_path, new_path, &state_for_rename);
            })),
            Some(Rc::new(move |deleted_path| {
                // A deleted path might be either a file (exact tab match) or a
                // directory (prefix match for any tab under it). Close both to
                // keep stale tabs from lingering.
                tabs_for_delete.close_tab_for_path(deleted_path, &state_for_delete);
                tabs_for_delete.close_tabs_under_dir(deleted_path, &state_for_delete);
            })),
            backend.clone(),
        ));

        // Git status view
        let git_status_view_slot: Rc<RefCell<Option<Rc<git_status::GitStatusView>>>> =
            Rc::new(RefCell::new(None));
        let on_git_changed: Rc<dyn Fn(String)> = Rc::new({
            let git_btn = git_btn.clone();
            let git_status_view_slot = git_status_view_slot.clone();
            move |git_output: String| {
                let has_changes = !git_output.trim().is_empty();
                if has_changes {
                    git_btn.add_css_class("git-has-changes");
                } else {
                    git_btn.remove_css_class("git-has-changes");
                }
                if let Some(view) = git_status_view_slot.borrow().as_ref() {
                    view.update(&git_output);
                }
            }
        });

        // Callback for immediate git status refresh after any git action
        let git_action_cb: Rc<dyn Fn()> = Rc::new({
            let on_git_changed = on_git_changed.clone();
            let be = backend.clone();
            move || {
                file_watcher::request_git_status_refresh(on_git_changed.clone(), be.clone());
            }
        });

        let git_status_view = Rc::new(git_status::GitStatusView::new(
            &PathBuf::from(root_dir),
            Rc::new({
                let root_c = PathBuf::from(root_dir);
                let tabs_c = tabs_rc.clone();
                let be = backend.clone();
                move |path, _status| {
                    tabs_c.show_diff(&root_c, path, be.clone());
                }
            }),
            backend.clone(),
            git_action_cb,
        ));
        *git_status_view_slot.borrow_mut() = Some(git_status_view.clone());

        // Project-wide search view
        let project_search_file_index = file_tree.file_index.clone();
        let project_search = Rc::new(project_search::ProjectSearch::new(
            &PathBuf::from(root_dir),
            Rc::new({
                let state_c = state.clone();
                let tabs_c = tabs_rc.clone();
                move |path, line_num, query| {
                    // Open the file synchronously so the tab exists.
                    tabs_c.open_file(path, &state_c);

                    // Prime the in-file search bar immediately so the user
                    // sees the active query and the overview ruler.
                    if !query.is_empty() {
                        tabs_c.search_entry.set_text(query);
                        tabs_c.search_bar.set_visible(true);
                        tabs_c.replace_row.set_visible(false);
                    }
                    tabs_c.update_match_ruler(query);

                    // Defer the scroll: a freshly-opened SourceView hasn't
                    // been laid out yet, so an immediate scroll_to_iter is a
                    // no-op. idle_add_local_once runs after GTK finishes the
                    // layout pass, mirroring what the nav-history code does.
                    let state_c2 = state_c.clone();
                    let tabs_c2 = tabs_c.clone();
                    let line_zero_based = (line_num as i32).saturating_sub(1);
                    gtk4::glib::idle_add_local_once(move || {
                        let st = state_c2.borrow();
                        let Some(idx) = st.active_tab else { return };
                        let Some(open_file) = st.open_files.get(idx) else {
                            return;
                        };
                        let Some(buf) = open_file.source_buffer() else { return };
                        let Some(iter) = buf.iter_at_line(line_zero_based) else {
                            return;
                        };
                        buf.place_cursor(&iter);
                        tabs_c2.source_view.scroll_to_iter(
                            &mut iter.clone(),
                            0.1,
                            true,
                            0.5,
                            0.3,
                        );
                    });
                }
            }),
            backend.clone(),
            project_search_file_index,
        ));

        // Sidebar stack to switch between file tree, git view, history, and search
        let sidebar_stack = gtk4::Stack::new();
        sidebar_stack.add_named(&file_tree.widget, Some("files"));
        sidebar_stack.add_named(&git_status_view.widget, Some("git"));
        sidebar_stack.add_named(&git_log_view.widget, Some("history"));
        sidebar_stack.add_named(&project_search.widget, Some("search"));
        sidebar.append(&sidebar_stack);

        // Hide sidebar button removed — use Ctrl+B or sidebar_open_btn instead

        // Connect activity bar toggle buttons
        {
            let stack = sidebar_stack.clone();
            files_btn.connect_toggled(move |btn| {
                if btn.is_active() {
                    stack.set_visible_child_name("files");
                }
            });
        }
        {
            let stack = sidebar_stack.clone();
            git_btn.connect_toggled(move |btn| {
                if btn.is_active() {
                    stack.set_visible_child_name("git");
                }
            });
        }
        {
            let stack = sidebar_stack.clone();
            let glv = git_log_view.clone();
            history_btn.connect_toggled(move |btn| {
                if btn.is_active() {
                    stack.set_visible_child_name("history");
                    glv.refresh();
                }
            });
        }
        {
            let stack = sidebar_stack.clone();
            let ps = project_search.clone();
            search_btn.connect_toggled(move |btn| {
                if btn.is_active() {
                    stack.set_visible_child_name("search");
                    ps.focus_entry();
                }
            });
        }

        // Fuzzy finder overlay
        let fuzzy_finder = Rc::new(fuzzy_finder::FuzzyFinder::new(
            &PathBuf::from(root_dir),
            file_tree.file_index.clone(),
            Rc::new({
                let state_c = state.clone();
                let tabs_c = tabs_rc.clone();
                move |path| {
                    tabs_c.open_file(path, &state_c);
                }
            }),
        ));

        // Sidebar toggle button — visible only when sidebar is hidden
        let sidebar_open_btn = gtk4::Button::from_icon_name("sidebar-show-symbolic");
        sidebar_open_btn.add_css_class("flat");
        sidebar_open_btn.set_tooltip_text(Some("Show sidebar (Ctrl+B)"));
        sidebar_open_btn.set_visible(false);
        sidebar_open_btn.set_halign(gtk4::Align::Start);
        sidebar_open_btn.set_valign(gtk4::Align::Start);
        sidebar_open_btn.set_margin_top(2);
        sidebar_open_btn.set_margin_start(2);
        {
            let sc = state.clone();
            let sb = sidebar.clone();
            let btn = sidebar_open_btn.clone();
            sidebar_open_btn.connect_clicked(move |_| {
                let mut st = sc.borrow_mut();
                st.sidebar_visible = true;
                sb.set_visible(true);
                btn.set_visible(false);
            });
        }
        editor_area.prepend(&sidebar_open_btn);

        // Wire the in-sidebar hide button — mirror Ctrl+B / sidebar_open_btn.
        {
            let sc = state.clone();
            let sb = sidebar.clone();
            let open_btn = sidebar_open_btn.clone();
            sidebar_hide_btn.connect_clicked(move |_| {
                let mut st = sc.borrow_mut();
                st.sidebar_visible = false;
                sb.set_visible(false);
                open_btn.set_visible(true);
            });
        }

        // Ctrl+B hides sidebar (handled in key event below)

        // Paned: sidebar | editor
        editor_area.set_width_request(-1);
        let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
        paned.set_start_child(Some(&sidebar));
        paned.set_end_child(Some(&editor_area));
        paned.set_position(200);
        paned.set_shrink_start_child(false);
        paned.set_shrink_end_child(false);
        paned.set_resize_start_child(false);

        // Wrap in a ScrolledWindow for horizontal scroll when space is tight
        let scroll_wrap = gtk4::ScrolledWindow::new();
        scroll_wrap.set_child(Some(&paned));
        scroll_wrap.set_hscrollbar_policy(gtk4::PolicyType::Automatic);
        scroll_wrap.set_vscrollbar_policy(gtk4::PolicyType::Never);
        scroll_wrap.set_propagate_natural_width(true);

        // Overlay: scroll + fuzzy finder on top
        let main_overlay = gtk4::Overlay::new();
        main_overlay.set_child(Some(&scroll_wrap));
        main_overlay.add_overlay(&fuzzy_finder.overlay);

        let widget = main_overlay.upcast::<gtk4::Widget>();
        widget.set_focusable(true);
        // Click anywhere to grab focus (needed for shortcuts without open file)
        {
            let w = widget.clone();
            let fuzzy_finder = fuzzy_finder.clone();
            let finder_overlay = fuzzy_finder.overlay.clone().upcast::<gtk4::Widget>();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(1);
            gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
            gesture.connect_pressed(move |_, _, x, y| {
                if fuzzy_finder.is_visible() {
                    let picked = w.pick(x, y, gtk4::PickFlags::DEFAULT);
                    let clicked_inside_finder = picked
                        .as_ref()
                        .map(|widget| {
                            let mut current = Some(widget.clone());
                            while let Some(w) = current {
                                if w == finder_overlay {
                                    return true;
                                }
                                current = w.parent();
                            }
                            false
                        })
                        .unwrap_or(false);
                    if !clicked_inside_finder {
                        fuzzy_finder.hide();
                    }
                }
                w.grab_focus();
            });
            widget.add_controller(gesture);
        }

        {
            let fuzzy_finder = fuzzy_finder.clone();
            let shortcut_ctrl = gtk4::ShortcutController::new();
            shortcut_ctrl.set_scope(gtk4::ShortcutScope::Managed);
            if let Some(trigger) = gtk4::ShortcutTrigger::parse_string("<Control>p") {
                let action = gtk4::CallbackAction::new(move |_, _| {
                    fuzzy_finder.show();
                    gtk4::glib::Propagation::Stop
                });
                shortcut_ctrl.add_shortcut(gtk4::Shortcut::new(Some(trigger), Some(action)));
            }
            widget.add_controller(shortcut_ctrl);
        }

        // Keybindings: Ctrl+S save, Ctrl+W close, Ctrl+Tab next tab, Ctrl+B sidebar, Ctrl+P fuzzy finder, Ctrl+Shift+G git view
        {
            let state_c = state.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
            let tabs_ref = tabs_rc.clone();
            let sidebar_ref = sidebar.clone();
            let git_btn_ref = git_btn.clone();
            let search_btn_ref = search_btn.clone();
            let save_backend = backend.clone();
            let save_git_changed = on_git_changed.clone();
            let sidebar_open_btn_ref = sidebar_open_btn.clone();
            let sidebar_stack_ref = sidebar_stack.clone();
            let project_search_ref = project_search.clone();
            key_ctrl.connect_key_pressed(move |_, key, _, modifier| {
                if crate::shortcuts::has_primary(modifier) {
                    let shift = modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
                    // Ctrl+Shift+V → toggle Rendered/Source on Markdown tab.
                    if shift && matches!(key, gtk4::gdk::Key::v | gtk4::gdk::Key::V) {
                        let md = {
                            let st = state_c.borrow();
                            st.active_tab
                                .and_then(|i| st.open_files.get(i))
                                .and_then(|f| match &f.content {
                                    tab_content::TabContent::Markdown(m) => Some(m.clone()),
                                    _ => None,
                                })
                        };
                        if let Some(md) = md {
                            markdown_view::toggle_mode(&md);
                            return gtk4::glib::Propagation::Stop;
                        }
                    }
                    // Ctrl+= / Ctrl++ / Ctrl+- / Ctrl+0 → image zoom.
                    if !shift {
                        let img_op = match key {
                            gtk4::gdk::Key::equal | gtk4::gdk::Key::plus => Some(0),
                            gtk4::gdk::Key::minus => Some(1),
                            gtk4::gdk::Key::_0 => Some(2),
                            _ => None,
                        };
                        if let Some(op) = img_op {
                            let img = {
                                let st = state_c.borrow();
                                st.active_tab
                                    .and_then(|i| st.open_files.get(i))
                                    .and_then(|f| match &f.content {
                                        tab_content::TabContent::Image(img) => {
                                            Some(img.clone())
                                        }
                                        _ => None,
                                    })
                            };
                            if let Some(img) = img {
                                match op {
                                    0 => image_view::zoom_in(&img),
                                    1 => image_view::zoom_out(&img),
                                    _ => image_view::zoom_reset(&img),
                                }
                                return gtk4::glib::Propagation::Stop;
                            }
                        }
                    }
                    match key {
                        gtk4::gdk::Key::s => {
                            let root = state_c.borrow().root_dir.clone();
                            tabs_ref.save_active(&state_c, &root);
                            file_watcher::request_git_status_refresh(
                                save_git_changed.clone(),
                                save_backend.clone(),
                            );
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
                            sidebar_open_btn_ref.set_visible(!st.sidebar_visible);
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::f | gtk4::gdk::Key::F
                            if modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK) =>
                        {
                            // Ctrl+Shift+F → search in project files.
                            // Drive the sidebar directly instead of leaning
                            // on the toggle button's `toggled` signal: the
                            // signal doesn't fire when the button is already
                            // active.
                            {
                                let mut st = state_c.borrow_mut();
                                if !st.sidebar_visible {
                                    st.sidebar_visible = true;
                                    sidebar_ref.set_visible(true);
                                    sidebar_open_btn_ref.set_visible(false);
                                }
                            }
                            sidebar_stack_ref.set_visible_child_name("search");
                            search_btn_ref.set_active(true);
                            // If the editor has a selection, seed the search
                            // entry with it so the user can hit Enter
                            // immediately. Nothing is selected → leave the
                            // existing entry text untouched.
                            let selected = {
                                let buf = tabs_ref.source_view.buffer();
                                buf.selection_bounds().and_then(|(s, e)| {
                                    let text = buf.text(&s, &e, false).to_string();
                                    if text.is_empty() || text.contains('\n') {
                                        None
                                    } else {
                                        Some(text)
                                    }
                                })
                            };
                            if let Some(text) = selected {
                                project_search_ref.set_query(&text);
                            }
                            project_search_ref.set_mode(project_search::SearchMode::Content);
                            project_search_ref.focus_entry();
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::p | gtk4::gdk::Key::P
                            if modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK) =>
                        {
                            // Ctrl+Shift+P → search files by name in the same
                            // sidebar tab as Ctrl+Shift+F, but in Files mode.
                            {
                                let mut st = state_c.borrow_mut();
                                if !st.sidebar_visible {
                                    st.sidebar_visible = true;
                                    sidebar_ref.set_visible(true);
                                    sidebar_open_btn_ref.set_visible(false);
                                }
                            }
                            sidebar_stack_ref.set_visible_child_name("search");
                            search_btn_ref.set_active(true);
                            project_search_ref.set_mode(project_search::SearchMode::Files);
                            let selected = {
                                let buf = tabs_ref.source_view.buffer();
                                buf.selection_bounds().and_then(|(s, e)| {
                                    let text = buf.text(&s, &e, false).to_string();
                                    if text.is_empty() || text.contains('\n') {
                                        None
                                    } else {
                                        Some(text)
                                    }
                                })
                            };
                            if let Some(text) = selected {
                                project_search_ref.set_query(&text);
                            }
                            project_search_ref.focus_entry();
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
                        gtk4::gdk::Key::g
                            if modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK) =>
                        {
                            git_btn_ref.set_active(true);
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::e => {
                            // Ctrl+E → show recent files popup
                            show_recent_files_popup(&state_c, &tabs_ref);
                            return gtk4::glib::Propagation::Stop;
                        }
                        _ => {}
                    }
                }
                // Alt+Left/Right → navigate back/forward in file history
                let alt = modifier.contains(gtk4::gdk::ModifierType::ALT_MASK);
                if alt {
                    match key {
                        gtk4::gdk::Key::Left => {
                            navigate_history(&state_c, &tabs_ref, false);
                            return gtk4::glib::Propagation::Stop;
                        }
                        gtk4::gdk::Key::Right => {
                            navigate_history(&state_c, &tabs_ref, true);
                            return gtk4::glib::Propagation::Stop;
                        }
                        _ => {}
                    }
                }
                gtk4::glib::Propagation::Proceed
            });
            widget.add_controller(key_ctrl);
        }

        // Wire navigation buttons
        {
            let sc = state.clone();
            let tc = tabs_rc.clone();
            nav_back_btn.connect_clicked(move |_| {
                navigate_history(&sc, &tc, false);
            });
        }
        {
            let sc = state.clone();
            let tc = tabs_rc.clone();
            nav_fwd_btn.connect_clicked(move |_| {
                navigate_history(&sc, &tc, true);
            });
        }
        {
            let sc = state.clone();
            let tc = tabs_rc.clone();
            recent_btn.connect_clicked(move |_| {
                show_recent_files_popup(&sc, &tc);
            });
        }

        // Reveal active file in tree button
        {
            let sc = state.clone();
            let ft = file_tree.clone();
            let files_btn_c = files_btn.clone();
            reveal_btn.connect_clicked(move |_| {
                let st = sc.borrow();
                if let Some(idx) = st.active_tab {
                    if let Some(f) = st.open_files.get(idx) {
                        let path = f.path.clone();
                        drop(st);
                        files_btn_c.set_active(true); // switch sidebar to files
                        ft.reveal_file(&path);
                    }
                }
            });
        }

        // Start file watchers
        {
            let file_tree_ref = file_tree.clone();
            file_watcher::start_watchers(
                state.clone(),
                tabs_rc.info_bar_container.clone(),
                Rc::new(move || {
                    file_tree_ref.refresh();
                }),
                on_git_changed.clone(),
            );
        }

        Self {
            widget,
            state,
            ssh_info: None,
        }
    }
}

/// Push current cursor position to navigation history.
#[cfg(feature = "sourceview")]
fn push_nav_position(state: &Rc<RefCell<EditorState>>) {
    let pos = {
        let st = state.borrow();
        st.active_tab
            .and_then(|idx| st.open_files.get(idx))
            .and_then(|f| {
                let buf = f.source_buffer()?;
                let iter = buf.iter_at_mark(&buf.get_insert());
                Some(FilePosition {
                    path: f.path.clone(),
                    line: iter.line(),
                })
            })
    };
    if let Some(pos) = pos {
        {
            let mut st = state.borrow_mut();
            st.nav_back.push(pos);
            st.nav_forward.clear(); // new action clears forward stack
            if st.nav_back.len() > 50 {
                st.nav_back.remove(0);
            }
        }
        fire_nav_state_changed(state);
    }
}

/// Navigate back or forward in file history.
/// Two-stack approach: back stack and forward stack, like a browser.
#[cfg(feature = "sourceview")]
fn navigate_history(
    state: &Rc<RefCell<EditorState>>,
    tabs: &Rc<editor_tabs::EditorTabs>,
    forward: bool,
) {
    // Get current position to save on the opposite stack
    let current_pos = {
        let st = state.borrow();
        st.active_tab
            .and_then(|idx| st.open_files.get(idx))
            .and_then(|f| {
                let buf = f.source_buffer()?;
                let iter = buf.iter_at_mark(&buf.get_insert());
                Some(FilePosition {
                    path: f.path.clone(),
                    line: iter.line(),
                })
            })
    };

    let target = {
        let mut st = state.borrow_mut();
        if forward {
            if st.nav_forward.is_empty() {
                return;
            }
            // Push current to back stack
            if let Some(cur) = current_pos {
                st.nav_back.push(cur);
            }
            st.nav_forward.pop()
        } else {
            if st.nav_back.is_empty() {
                return;
            }
            // Push current to forward stack
            if let Some(cur) = current_pos {
                st.nav_forward.push(cur);
            }
            st.nav_back.pop()
        }
    };

    if let Some(pos) = target {
        // Check if file exists
        let file_exists = state.borrow().backend.file_exists(&pos.path);
        if !file_exists {
            // Remove all entries for this file from both stacks
            let mut st = state.borrow_mut();
            st.nav_back.retain(|p| p.path != pos.path);
            st.nav_forward.retain(|p| p.path != pos.path);
            // Try next entry
            drop(st);
            navigate_history(state, tabs, forward);
            return;
        }

        // Switch to file (reopen if closed)
        let already_open = {
            let st = state.borrow();
            st.open_files.iter().position(|f| f.path == pos.path)
        };
        if let Some(idx) = already_open {
            tabs.notebook.set_current_page(Some(idx as u32));
            tabs.switch_to_buffer(idx, state);
        } else {
            // File is closed — reopen it, undo the nav push it causes
            tabs.open_file(&pos.path, state);
            let mut st = state.borrow_mut();
            st.nav_back.pop(); // undo the push from open_file
        }
        fire_nav_state_changed(state);

        // Scroll to saved line (deferred so layout has time to complete)
        // If line doesn't exist, go to last line
        let line = pos.line;
        let sv = tabs.source_view.clone();
        let state_c = state.clone();
        gtk4::glib::idle_add_local_once(move || {
            let st = state_c.borrow();
            if let Some(idx) = st.active_tab {
                if let Some(f) = st.open_files.get(idx) {
                    if let Some(buf) = f.source_buffer() {
                        let target_line = if line < buf.line_count() {
                            line
                        } else {
                            buf.line_count() - 1
                        };
                        if let Some(iter) = buf.iter_at_line(target_line) {
                            buf.place_cursor(&iter);
                            sv.scroll_to_iter(&mut iter.clone(), 0.1, false, 0.0, 0.0);
                        }
                    }
                }
            }
        });
    }
}

/// Show a popup with recent files for quick switching (Ctrl+E).
#[cfg(feature = "sourceview")]
fn show_recent_files_popup(state: &Rc<RefCell<EditorState>>, tabs: &Rc<editor_tabs::EditorTabs>) {
    let recent = state.borrow().recent_files.clone();
    let root = state.borrow().root_dir.clone();
    if recent.is_empty() {
        return;
    }

    let dialog = gtk4::Window::builder()
        .title("Recent Files")
        .modal(true)
        .default_width(400)
        .default_height(300)
        .build();

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);

    for path in &recent {
        let rel = path.strip_prefix(&root).unwrap_or(path);
        let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row_box.set_margin_start(8);
        row_box.set_margin_end(8);
        row_box.set_margin_top(4);
        row_box.set_margin_bottom(4);
        let icon = gtk4::Image::from_icon_name("text-x-generic-symbolic");
        icon.set_pixel_size(16);
        row_box.append(&icon);
        let label = gtk4::Label::new(Some(&rel.to_string_lossy()));
        label.set_halign(gtk4::Align::Start);
        label.set_hexpand(true);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
        row_box.append(&label);
        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&row_box));
        row.set_widget_name(&path.to_string_lossy());
        list_box.append(&row);
    }

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&list_box));
    scroll.set_vexpand(true);
    vbox.append(&scroll);

    // Click to open
    {
        let d = dialog.clone();
        let state_c = state.clone();
        let tabs_c = tabs.clone();
        list_box.connect_row_activated(move |_, row| {
            let path_str = row.widget_name();
            let path = PathBuf::from(path_str.as_str());
            tabs_c.open_file(&path, &state_c);
            d.close();
        });
    }

    dialog.set_child(Some(&vbox));
    dialog.present();
}

/// SSH ControlMaster connection is cleaned up by SshFileBackend::Drop.
#[cfg(feature = "sourceview")]
impl Drop for CodeEditorPanel {
    fn drop(&mut self) {}
}

#[cfg(feature = "sourceview")]
impl PanelBackend for CodeEditorPanel {
    fn panel_type(&self) -> &str {
        "code_editor"
    }
    fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }
    fn on_focus(&self) {}

    fn ssh_label(&self) -> Option<String> {
        self.ssh_info.clone()
    }

    fn get_text_content(&self) -> Option<String> {
        let st = self.state.borrow();
        st.active_tab.and_then(|idx| {
            st.open_files.get(idx).and_then(|f| {
                let buf = f.content.source_buffer()?;
                Some(
                    buf.text(&buf.start_iter(), &buf.end_iter(), false)
                        .to_string(),
                )
            })
        })
    }

    fn footer_text(&self) -> Option<String> {
        let st = self.state.borrow();
        let p = st.root_dir.to_string_lossy();
        if p.is_empty() {
            None
        } else {
            Some(p.into_owned())
        }
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
    pub fn new(_root_dir: &str, _record_key: String) -> Self {
        let label = gtk4::Label::new(Some("Code Editor requires the 'sourceview' feature.\nRecompile with: cargo build --features sourceview"));
        label.set_margin_top(32);
        label.set_margin_bottom(32);
        label.add_css_class("dim-label");
        Self {
            widget: label.upcast::<gtk4::Widget>(),
        }
    }

    pub fn new_remote(
        _host: &str,
        _port: u16,
        _user: &str,
        _password: Option<&str>,
        _identity_file: Option<&str>,
        _remote_path: &str,
        _record_key: String,
    ) -> Self {
        Self::new("", String::new())
    }
}

#[cfg(not(feature = "sourceview"))]
impl PanelBackend for CodeEditorPanel {
    fn panel_type(&self) -> &str {
        "code_editor"
    }
    fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }
    fn on_focus(&self) {}
}
