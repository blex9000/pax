use gtk4::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pax_core::workspace::{new_tab_id, LayoutNode, PanelConfig, PanelType, Workspace};

use crate::backend_factory::{
    create_backend_from_registry, insert_ssh_extra, panel_type_to_create_config, panel_type_to_id,
};
use crate::focus::FocusManager;
use crate::layout_ops::{remove_from_layout, replace_in_layout};
use crate::panel_host::{PanelActionCallback, PanelHost, SiblingInfoProvider};
use crate::panels::chooser::{ChooserPanel, OnTypeChosen};
use crate::panels::registry::{self, PanelCreateConfig, PanelRegistry};
use crate::widget_builder::*;

/// Direction for `WorkspaceView::move_focused_panel`. The dispatcher
/// picks the appropriate axis given the focused panel's parent kind;
/// directions that don't match the parent silently no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone)]
struct ActiveTabEdit {
    tab_id: String,
    tab_path: Vec<usize>,
    panel_id: String,
    draft_name: String,
    is_layout: bool,
    original_name: String,
    original_workspace: Workspace,
    original_dirty: bool,
    pending_offset: i32,
    suppress_commit_once: bool,
}

/// Builds the GTK widget tree from a workspace layout.
pub struct WorkspaceView {
    root_widget: gtk4::Widget,
    root_box: gtk4::Box,
    scrolled: gtk4::ScrolledWindow,
    hosts: HashMap<String, PanelHost>,
    focus: FocusManager,
    workspace: Workspace,
    config_path: Option<PathBuf>,
    next_panel_id: usize,
    action_cb: Option<PanelActionCallback>,
    sibling_info_provider: Option<SiblingInfoProvider>,
    registry: PanelRegistry,
    on_type_chosen: Option<OnTypeChosen>,
    layout_change_cb: Option<Rc<dyn Fn()>>,
    dirty: bool,
    /// When a panel is zoomed (fullscreen), store which panel and hidden siblings.
    zoomed_panel: Option<String>,
    /// Panel IDs that are in sync-input mode.
    sync_panels: std::collections::HashSet<String>,
    /// Callback for terminal input sync propagation.
    sync_input_cb: Option<std::rc::Rc<dyn Fn(&str, &[u8])>>,
    tab_edit: Option<ActiveTabEdit>,
}

impl WorkspaceView {
    /// Build the workspace view from a workspace config.
    /// Call `set_action_callback` after wrapping in Rc<RefCell<>> to enable menu actions.
    pub fn build(workspace: &Workspace, config_path: Option<&Path>) -> Self {
        let mut workspace = workspace.clone();
        workspace.ensure_layout_tab_ids();
        let registry = registry::build_default_registry();
        let ws_dir = config_path
            .and_then(|p| p.parent())
            .map(|p| p.to_string_lossy().to_string());
        let config_path_str = config_path.map(|p| p.to_string_lossy().to_string());
        let record_key = workspace.record_key(config_path_str.as_deref());
        let mut hosts = HashMap::new();

        for panel_cfg in &workspace.panels {
            let host = PanelHost::new(&panel_cfg.id, &panel_cfg.name, None);
            if panel_cfg.effective_type() == PanelType::Empty {
                let chooser = ChooserPanel::new(&panel_cfg.id, &registry, None);
                host.set_backend(Box::new(chooser));
            } else {
                let backend = create_backend_from_registry(
                    panel_cfg,
                    &workspace.settings.default_shell,
                    &registry,
                    ws_dir.as_deref(),
                    Some(record_key.as_str()),
                );
                host.set_backend(backend);
            }
            apply_min_size(&host, panel_cfg);
            hosts.insert(panel_cfg.id.clone(), host);
        }

        // Populate the terminal-registry breadcrumbs once the layout is
        // settled — used by the notebook run-target picker.
        publish_terminal_breadcrumbs(&workspace);

        // Build layout widget tree
        let root_widget = build_layout_widget(&workspace.layout, &hosts, &workspace.panels, None);
        root_widget.set_vexpand(true);
        root_widget.set_hexpand(true);

        // Wrap in Box (for reparenting) inside ScrolledWindow (for overflow)
        let root_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root_box.append(&root_widget);

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_child(Some(&root_box));
        scrolled.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic);

        let focus_ids: Vec<String> = workspace
            .layout
            .panel_ids()
            .iter()
            .map(|s| s.to_string())
            .collect();

        // Find highest existing panel ID number for counter
        let next_panel_id = workspace
            .panels
            .iter()
            .filter_map(|p| p.id.strip_prefix('p').and_then(|n| n.parse::<usize>().ok()))
            .max()
            .unwrap_or(0)
            + 1;

        let mut view = Self {
            root_widget,
            root_box,
            scrolled,
            hosts,
            focus: FocusManager::from_ids(focus_ids),
            workspace,
            config_path: config_path.map(|p| p.to_path_buf()),
            next_panel_id,
            action_cb: None,
            sibling_info_provider: None,
            registry,
            on_type_chosen: None,
            layout_change_cb: None,
            dirty: false,
            zoomed_panel: None,
            sync_panels: std::collections::HashSet::new(),
            sync_input_cb: None,
            tab_edit: None,
        };

        // Focus first panel
        view.focus.focus_first(&view.hosts);

        // Record in recent workspaces DB
        view.record_in_db();

        view
    }

    /// Load a workspace struct directly (for New workspace).
    pub fn load_workspace(
        &mut self,
        ws: Workspace,
        config_path: Option<&Path>,
    ) -> Result<(), String> {
        self.config_path = config_path.map(|p| p.to_path_buf());
        self.rebuild_from_workspace(ws)
    }

    /// Reload from a workspace file, rebuilding the entire view.
    pub fn load_from_file(&mut self, path: &Path) -> Result<(), String> {
        tracing::info!("Loading workspace from {}", path.display());
        let ws =
            pax_core::config::load_workspace(path).map_err(|e| format!("Failed to load: {}", e))?;
        tracing::info!(
            "Loaded workspace '{}' with {} panels",
            ws.name,
            ws.panels.len()
        );
        self.config_path = Some(path.to_path_buf());
        self.rebuild_from_workspace(ws)
    }

    fn rebuild_from_workspace(&mut self, mut ws: Workspace) -> Result<(), String> {
        // Remove old root widget
        self.root_box.remove(&self.root_widget);
        ws.ensure_layout_tab_ids();

        let registry = registry::build_default_registry();
        let ws_dir = self
            .config_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_string_lossy().to_string());
        let config_path_str = self
            .config_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());
        let record_key = ws.record_key(config_path_str.as_deref());
        let mut hosts = HashMap::new();

        for panel_cfg in &ws.panels {
            let host = PanelHost::new(&panel_cfg.id, &panel_cfg.name, self.action_cb.clone());
            self.wire_sibling_info_provider_on(&host);
            if panel_cfg.effective_type() == PanelType::Empty {
                let chooser =
                    ChooserPanel::new(&panel_cfg.id, &registry, self.on_type_chosen.clone());
                host.set_backend(Box::new(chooser));
            } else {
                let backend = create_backend_from_registry(
                    panel_cfg,
                    &ws.settings.default_shell,
                    &registry,
                    ws_dir.as_deref(),
                    Some(record_key.as_str()),
                );
                host.set_backend(backend);
            }
            apply_min_size(&host, panel_cfg);
            hosts.insert(panel_cfg.id.clone(), host);
        }

        let root_widget = build_layout_widget(&ws.layout, &hosts, &ws.panels, None);
        root_widget.set_vexpand(true);
        root_widget.set_hexpand(true);
        self.root_box.append(&root_widget);

        if let Some(ref cb) = self.action_cb {
            update_notebook_labels_recursive(&root_widget, cb, &hosts, &ws, None);
            add_plus_buttons_recursive(&root_widget, cb);
        }

        self.root_widget = root_widget;
        self.hosts = hosts;
        self.workspace = ws;
        self.registry = registry;
        self.dirty = false;
        self.tab_edit = None;
        self.connect_layout_change_watchers();

        self.next_panel_id = self
            .workspace
            .panels
            .iter()
            .filter_map(|p| p.id.strip_prefix('p').and_then(|n| n.parse::<usize>().ok()))
            .max()
            .unwrap_or(0)
            + 1;

        self.rebuild_focus_order();
        self.dirty = false;

        self.focus.focus_first(&self.hosts);

        self.record_in_db();
        Ok(())
    }

    /// Get the current panel type for a panel.
    /// Update panel config after Configure dialog.
    /// Recreates the backend with the new type/settings and runs startup commands.
    pub fn apply_panel_config(
        &mut self,
        panel_id: &str,
        new_name: String,
        new_type: PanelType,
        cwd: Option<String>,
        ssh: Option<pax_core::workspace::SshConfig>,
        startup_commands: Vec<String>,
        before_close: Option<String>,
        min_width: u32,
        min_height: u32,
    ) {
        tracing::info!(
            "Configuring panel {}: name={}, type={:?}, cwd={:?}, ssh={}, cmds={}, before_close={}",
            panel_id,
            new_name,
            new_type,
            cwd,
            ssh.is_some(),
            startup_commands.len(),
            before_close.is_some()
        );

        // Run before_close script on the old backend before replacing it
        // (only fires if a script is configured for this panel).
        self.run_before_close(panel_id);

        // Update model
        if let Some(panel_cfg) = self.workspace.panels.iter_mut().find(|p| p.id == panel_id) {
            panel_cfg.name = new_name.clone();
            panel_cfg.panel_type = new_type.clone();
            panel_cfg.cwd = cwd.clone();
            panel_cfg.ssh = ssh;
            panel_cfg.startup_commands = startup_commands.clone();
            panel_cfg.before_close = before_close;
            panel_cfg.min_width = min_width;
            panel_cfg.min_height = min_height;
        }

        // Update host title only — tab labels are user-controlled (renamed
        // via the tab UI) and update_tab_label_in_layout walks ancestors via
        // is_panel_with_id (recursive), so a rename here would clobber the
        // outer tab when the panel is nested inside Hsplit/Vsplit. Panel
        // name vs tab label are separate concerns.
        if let Some(host) = self.hosts.get(panel_id) {
            host.set_title(&new_name);
        }

        // Recreate backend with startup commands queued
        let ws_dir = self
            .config_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_string_lossy().to_string());
        let config_path_str = self
            .config_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());
        let record_key = self.workspace.record_key(config_path_str.as_deref());
        let mut config = panel_type_to_create_config(
            &new_type,
            &self.workspace.settings.default_shell,
            ws_dir.as_deref(),
            Some(record_key.as_str()),
        );
        // Pass cwd and env from panel config
        config.cwd = cwd;
        if let Some(panel_cfg) = self.workspace.panels.iter().find(|p| p.id == panel_id) {
            config.env = panel_cfg
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
        }
        // Pass SSH config if present
        if let Some(panel_cfg) = self.workspace.panels.iter().find(|p| p.id == panel_id) {
            if let Some(ref ssh) = panel_cfg.effective_ssh() {
                insert_ssh_extra(&mut config.extra, ssh);
            }
        }
        // Pass startup commands via extra so the registry factory can queue them
        if !startup_commands.is_empty() {
            config.extra.insert(
                "__startup_commands__".to_string(),
                startup_commands.join("\n"),
            );
        }
        if let Some(backend) = self.registry.create(panel_type_to_id(&new_type), &config) {
            if let Some(host) = self.hosts.get(panel_id) {
                host.set_backend(backend);
            }
        }

        // Apply min size to widget
        if let Some(host) = self.hosts.get(panel_id) {
            let w = if min_width > 0 { min_width as i32 } else { -1 };
            let h = if min_height > 0 {
                min_height as i32
            } else {
                -1
            };
            host.widget().set_size_request(w, h);
        }

        self.dirty = true;

        // Refresh tab labels (name + type icon) in place on the existing
        // Notebook widgets. No layout structure changed here: the backend
        // widget was already swapped in-place by host.set_backend above, and
        // host title + size request were already updated. A full
        // rebuild_layout here would just cause a visible flicker and reset
        // every Notebook's current page to 0 (dropping the user's tab
        // selection and keyboard focus).
        self.refresh_tab_labels();
    }

    /// Execute before_close script for a panel.
    fn run_before_close(&self, panel_id: &str) {
        if let Some(script) = self
            .workspace
            .panel(panel_id)
            .and_then(|p| p.before_close.clone())
        {
            self.execute_close_script(panel_id, &script);
        }
    }

    /// Execute before_close scripts for ALL panels (called on app/window close).
    pub fn run_all_before_close(&self) {
        for panel_cfg in &self.workspace.panels {
            if let Some(ref script) = panel_cfg.before_close {
                self.execute_close_script(&panel_cfg.id, script);
            }
        }
    }

    /// Shut down all panel backends (terminate child processes).
    /// Called on app/window close alongside `run_all_before_close`.
    pub fn shutdown_all_backends(&self) {
        for host in self.hosts.values() {
            host.shutdown_backend();
        }
    }

    fn execute_close_script(&self, panel_id: &str, script: &str) {
        if script.trim().is_empty() {
            return;
        }
        let host = match self.hosts.get(panel_id) {
            Some(h) => h,
            None => return,
        };

        // "file:<path>" → resolve and execute the file
        if script.starts_with("file:") {
            let path = script.trim_start_matches("file:");
            let ws_dir = self.config_path.as_ref().and_then(|p| p.parent());
            let resolved = if std::path::Path::new(path).is_absolute() {
                path.to_string()
            } else if let Some(dir) = ws_dir {
                dir.join(path).to_string_lossy().to_string()
            } else {
                path.to_string()
            };
            let cmd = format!("bash {}\n", resolved);
            host.write_input(cmd.as_bytes());
        } else {
            // Inline script
            let cmd = format!("{}\n", script);
            host.write_input(cmd.as_bytes());
        }
    }

    /// Rename the workspace.
    pub fn rename_workspace(&mut self, new_name: &str) {
        self.workspace.name = new_name.to_string();
        self.dirty = true;
    }

    pub fn rename_panel(&mut self, panel_id: &str, new_name: &str) -> bool {
        let changed = rename_panel_model(&mut self.workspace, panel_id, new_name);
        if changed {
            if let Some(host) = self.hosts.get(panel_id) {
                host.set_title(new_name);
            }
            self.dirty = true;
        }
        changed
    }

    pub fn rename_tab_label(&mut self, panel_id: &str, new_name: &str) -> bool {
        let changed = rename_tab_label_model(&mut self.workspace.layout, panel_id, new_name);
        if changed {
            self.dirty = true;
        }
        changed
    }

    pub fn begin_tab_edit(
        &mut self,
        panel_id: &str,
        tab_id: &str,
        tab_path: Vec<usize>,
        draft_name: String,
        is_layout: bool,
    ) -> bool {
        self.tab_edit = Some(ActiveTabEdit {
            tab_id: tab_id.to_string(),
            tab_path,
            panel_id: panel_id.to_string(),
            draft_name: draft_name.clone(),
            is_layout,
            original_name: draft_name,
            original_workspace: self.workspace.clone(),
            original_dirty: self.dirty,
            pending_offset: 0,
            suppress_commit_once: false,
        });
        true
    }

    pub fn update_tab_edit_draft(&mut self, tab_id: &str, draft_name: String) -> bool {
        let Some(state) = self.tab_edit.as_mut() else {
            return false;
        };
        if state.tab_id != tab_id {
            return false;
        }
        state.draft_name = draft_name;
        true
    }

    pub fn preview_tab_edit_move(&mut self, tab_id: &str, step: i32) -> bool {
        let Some(state) = self.tab_edit.as_mut() else {
            return false;
        };
        if state.tab_id != tab_id {
            return false;
        }

        let Some(new_path) = crate::layout_ops::move_tab_in_layout_by_path(
            &mut self.workspace.layout,
            &state.tab_path,
            step,
        ) else {
            return false;
        };

        state.tab_path = new_path;
        state.pending_offset += step;
        state.suppress_commit_once = true;
        true
    }

    pub fn clear_tab_edit_commit_suppression(&mut self, tab_id: &str) {
        if let Some(state) = self.tab_edit.as_mut() {
            if state.tab_id == tab_id {
                state.suppress_commit_once = false;
            }
        }
    }

    pub fn commit_tab_edit(&mut self, tab_id: &str) -> bool {
        let Some(state) = self.tab_edit.clone() else {
            return false;
        };
        if state.tab_id != tab_id {
            return false;
        }
        if state.suppress_commit_once {
            return false;
        }

        self.tab_edit = None;

        let trimmed_name = state.draft_name.trim();
        let mut changed = state.pending_offset != 0;
        if !trimmed_name.is_empty() && state.draft_name != state.original_name {
            // Only rename the tab label in the layout model — never the panel
            // name. Tab labels and panel names are independent: a tab called
            // "servers" might contain a terminal whose panel name is
            // "web-server-freeflow". The old code called rename_panel_model
            // for single-panel tabs, which silently overwrote the panel name
            // with the tab label on every edit+save cycle.
            changed |= rename_tab_label_model_by_id(
                &mut self.workspace.layout,
                &state.tab_id,
                &state.draft_name,
            );
        }

        self.rebuild_layout();
        self.select_workspace_tab_for_panel(&state.panel_id);
        self.dirty = state.original_dirty || changed;
        changed
    }

    pub fn cancel_tab_edit(&mut self, tab_id: &str) -> bool {
        let Some(state) = self.tab_edit.clone() else {
            return false;
        };
        if state.tab_id != tab_id {
            return false;
        }

        self.workspace = state.original_workspace;
        self.tab_edit = None;
        self.rebuild_layout();
        self.rebuild_focus_order();
        if let Some(index) = self.focus.order.iter().position(|id| id == &state.panel_id) {
            self.focus.index = index;
            self.focus.focus_current_pub(&self.hosts);
        }
        self.select_workspace_tab_for_panel(&state.panel_id);
        self.dirty = state.original_dirty;
        true
    }

    /// Compute the sibling info for a panel — used by the panel menu to
    /// decide which Move items to show.
    pub fn panel_sibling_info(
        &self,
        panel_id: &str,
    ) -> Option<crate::layout_ops::SiblingInfo> {
        crate::layout_ops::panel_sibling_info(&self.workspace.layout, panel_id)
    }

    /// Move the focused panel by one position in its parent split or
    /// tabs node. Returns true if the move happened. Picks the correct
    /// container kind by inspecting the parent — directions that don't
    /// match the parent (e.g. `Up` on an Hsplit) silently no-op so the
    /// caller doesn't need to dispatch by kind.
    pub fn move_focused_panel(&mut self, direction: MoveDirection) -> bool {
        let Some(focused_id) = self.focused_panel_id().map(|s| s.to_string()) else {
            return false;
        };
        let Some(info) = self.panel_sibling_info(&focused_id) else {
            return false;
        };

        use crate::layout_ops::SiblingKind;
        let delta = match (info.kind, direction) {
            (SiblingKind::Hsplit, MoveDirection::Left) => -1,
            (SiblingKind::Hsplit, MoveDirection::Right) => 1,
            (SiblingKind::Tabs, MoveDirection::Left) => -1,
            (SiblingKind::Tabs, MoveDirection::Right) => 1,
            (SiblingKind::Vsplit, MoveDirection::Up) => -1,
            (SiblingKind::Vsplit, MoveDirection::Down) => 1,
            _ => return false,
        };

        let moved = crate::layout_ops::move_panel_in_split(
            &mut self.workspace.layout,
            &focused_id,
            delta,
        );
        if !moved {
            return false;
        }

        self.rebuild_layout();
        self.rebuild_focus_order();
        if let Some(index) = self.focus.order.iter().position(|id| id == &focused_id) {
            self.focus.index = index;
            self.focus.focus_current_pub(&self.hosts);
        }
        self.select_workspace_tab_for_panel(&focused_id);
        self.dirty = true;
        true
    }

    pub fn move_tab_by_panel_id(&mut self, panel_id: &str, direction: i32) -> bool {
        let moved = crate::layout_ops::move_tab_in_layout_steps(
            &mut self.workspace.layout,
            panel_id,
            direction,
        );
        if !moved {
            return false;
        }

        self.rebuild_layout();
        self.rebuild_focus_order();
        if let Some(index) = self.focus.order.iter().position(|id| id == panel_id) {
            self.focus.index = index;
            self.focus.focus_current_pub(&self.hosts);
        }
        self.select_workspace_tab_for_panel(panel_id);
        self.dirty = true;
        true
    }

    fn focus_panel_after_rebuild(&mut self, panel_id: &str) -> bool {
        let Some(index) = self.focus.order.iter().position(|id| id == panel_id) else {
            return false;
        };
        self.select_workspace_tab_for_panel(panel_id);
        self.focus.set_focus_index(index, &self.hosts);
        let root_widget = self.root_widget.clone();
        let layout = self.workspace.layout.clone();
        let panel_id = panel_id.to_string();
        gtk4::glib::idle_add_local_once(move || {
            let _ = select_workspace_tabs_for_panel(&root_widget, &layout, &panel_id);
        });
        true
    }

    pub fn workspace_name(&self) -> &str {
        &self.workspace.name
    }

    /// Record key of the currently loaded workspace — same value stored
    /// on notes / commands so external callers (scheduled-alert click
    /// handler) can compare "is the note in the workspace I'm looking
    /// at?".
    pub fn record_key(&self) -> String {
        let config_path_str = self.config_path_str();
        self.workspace.record_key(config_path_str.as_deref())
    }

    /// Reveal and focus the panel with the given id. If the panel lives
    /// inside a non-active Notebook tab, the containing tab is switched
    /// first. Returns false when the panel isn't in the current
    /// workspace (caller should handle that — e.g. prompt to load the
    /// owning workspace from file).
    pub fn focus_panel(&mut self, panel_id: &str) -> bool {
        self.focus_panel_after_rebuild(panel_id)
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn has_config_path(&self) -> bool {
        self.config_path.is_some()
    }

    pub fn config_path_str(&self) -> Option<String> {
        self.config_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
    }

    fn current_tab_label_edit_state(&self) -> Option<TabLabelEditState> {
        self.tab_edit.as_ref().map(|state| TabLabelEditState {
            tab_id: state.tab_id.clone(),
            draft_name: state.draft_name.clone(),
        })
    }

    pub fn active_tab_edit_tab_id(&self) -> Option<String> {
        self.tab_edit.as_ref().map(|state| state.tab_id.clone())
    }

    pub fn refresh_tab_labels(&self) {
        let Some(ref cb) = self.action_cb else {
            return;
        };
        let edit_state = self.current_tab_label_edit_state();
        update_notebook_labels_recursive(
            &self.root_widget,
            cb,
            &self.hosts,
            &self.workspace,
            edit_state.as_ref(),
        );
        add_plus_buttons_recursive(&self.root_widget, cb);
    }

    /// Set callback for when a panel type is chosen from the chooser.
    /// Rebuilds chooser panels so they get the callback.
    pub fn set_type_chosen_callback(&mut self, cb: OnTypeChosen) {
        self.on_type_chosen = Some(cb.clone());

        // Rebuild any existing chooser panels so they get the callback
        let chooser_ids: Vec<String> = self
            .workspace
            .panels
            .iter()
            .filter(|p| p.effective_type() == PanelType::Empty)
            .map(|p| p.id.clone())
            .collect();
        for id in chooser_ids {
            if let Some(host) = self.hosts.get(&id) {
                let chooser = ChooserPanel::new(&id, &self.registry, Some(cb.clone()));
                host.set_backend(Box::new(chooser));
            }
        }
    }

    /// Reset a panel back to the type chooser (empty state).
    pub fn reset_panel(&mut self, panel_id: &str) {
        let backend = self.create_chooser_backend(panel_id);
        if let Some(host) = self.hosts.get(panel_id) {
            host.set_backend(backend);
            host.set_title("New Panel");
            host.set_type_icon("chooser");
        }
        if let Some(panel_cfg) = self.workspace.panels.iter_mut().find(|p| p.id == panel_id) {
            panel_cfg.panel_type = PanelType::Empty;
            panel_cfg.name = "New Panel".to_string();
        }
        // Chooser doesn't support sync — drop any leftover membership so the
        // group keeps mirroring only real input panels.
        self.drop_sync_if_unsupported(panel_id);
        self.dirty = true;
    }

    /// Change a panel's type. Swaps the backend in the existing PanelHost.
    /// Returns true if the panel needs immediate configuration (markdown, code_editor).
    pub fn set_panel_type(&mut self, panel_id: &str, type_id: &str) -> bool {
        tracing::info!("Setting panel {} type to {}", panel_id, type_id);
        // Pass __workspace_dir__ the same way apply_panel_config does (via
        // panel_type_to_create_config), so factories that anchor their cwd /
        // paths to the workspace directory actually receive it. Without this,
        // a terminal created from the chooser would always open in the
        // process cwd, even on a saved workspace.
        let ws_dir = self
            .config_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_string_lossy().to_string());
        let config_path_str = self
            .config_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());
        let record_key = self.workspace.record_key(config_path_str.as_deref());
        let mut extra = HashMap::new();
        if let Some(dir) = ws_dir {
            extra.insert("__workspace_dir__".to_string(), dir);
        }
        extra.insert("__workspace_record_key__".to_string(), record_key);
        extra.insert("__panel_id__".to_string(), panel_id.to_string());
        let config = PanelCreateConfig {
            shell: self.workspace.settings.default_shell.clone(),
            cwd: None,
            env: vec![],
            extra,
        };

        if let Some(backend) = self.registry.create(type_id, &config) {
            if let Some(host) = self.hosts.get(panel_id) {
                host.set_backend(backend);
            }
        }
        // If the new backend doesn't opt into sync, evict the panel from
        // the synced group rather than leaving a dead member.
        self.drop_sync_if_unsupported(panel_id);

        // Update the model so it saves correctly
        self.dirty = true;
        if let Some(panel_cfg) = self.workspace.panels.iter_mut().find(|p| p.id == panel_id) {
            panel_cfg.panel_type = match type_id {
                "terminal" => PanelType::Terminal,
                "markdown" => PanelType::Markdown {
                    file: "README.md".to_string(),
                },
                "code_editor" => {
                    // Use home directory as default instead of "." which causes permission issues on macOS
                    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                    PanelType::CodeEditor {
                        root_dir: home,
                        ssh: None,
                        remote_path: None,
                        poll_interval: None,
                    }
                }
                "note" => PanelType::Note,
                _ => PanelType::Terminal,
            };
            panel_cfg.name = format!("{}", type_id);
            // Note: we deliberately do NOT touch tab labels here. Tab names
            // are user-controlled (renamed via the tab UI), and
            // update_tab_label_in_layout walks ancestors via is_panel_with_id
            // — picking a chooser type for a panel inside a Hsplit inside a
            // Tabs would clobber the outer tab's label. Panel type vs tab
            // label are separate concerns.
        }

        // Update host title and icon (no rebuild — would destroy the new backend)
        if let Some(host) = self.hosts.get(panel_id) {
            host.set_title(type_id);
            host.set_type_icon(type_id);
        }
        // markdown and code_editor need configuration first
        matches!(type_id, "markdown" | "code_editor")
    }

    /// Get a reference to the panel registry.
    pub fn registry(&self) -> &PanelRegistry {
        &self.registry
    }

    /// Install the SiblingInfoProvider on all current and future hosts so
    /// the panel menu reflects the live layout.
    pub fn set_sibling_info_provider(&mut self, provider: SiblingInfoProvider) {
        self.sibling_info_provider = Some(provider.clone());
        for host in self.hosts.values() {
            host.set_sibling_info_provider(provider.clone());
        }
    }

    /// Install the currently-stored sibling info provider on a freshly
    /// created `PanelHost`. Callers (split, add_tab, etc.) invoke this
    /// after `PanelHost::new` so the new host's ⋮ menu shows Move items
    /// with up-to-date layout context.
    fn wire_sibling_info_provider_on(&self, host: &PanelHost) {
        if let Some(ref provider) = self.sibling_info_provider {
            host.set_sibling_info_provider(provider.clone());
        }
    }

    /// Set the action callback for panel menus. Must be called after wrapping in Rc<RefCell<>>.
    /// Propagates to all existing hosts and updates notebook widgets.
    pub fn set_action_callback(&mut self, cb: PanelActionCallback) {
        self.action_cb = Some(cb.clone());
        // Update all hosts
        for host in self.hosts.values() {
            host.set_action_callback(cb.clone());
        }
        // Update notebook tab labels with close buttons and + buttons
        let edit_state = self.current_tab_label_edit_state();
        update_notebook_labels_recursive(
            &self.root_widget,
            &cb,
            &self.hosts,
            &self.workspace,
            edit_state.as_ref(),
        );
        add_plus_buttons_recursive(&self.root_widget, &cb);
        // Reconnect chooser callbacks
        if let Some(ref tc) = self.on_type_chosen {
            let chooser_ids: Vec<String> = self
                .workspace
                .panels
                .iter()
                .filter(|p| p.effective_type() == PanelType::Empty)
                .map(|p| p.id.clone())
                .collect();
            for id in chooser_ids {
                if let Some(host) = self.hosts.get(&id) {
                    let chooser = ChooserPanel::new(&id, &self.registry, Some(tc.clone()));
                    host.set_backend(Box::new(chooser));
                }
            }
        }
    }

    pub fn set_layout_change_callback(&mut self, cb: Rc<dyn Fn()>) {
        self.layout_change_cb = Some(cb);
        self.connect_layout_change_watchers();
    }

    pub fn widget(&self) -> &gtk4::ScrolledWindow {
        &self.scrolled
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn workspace_mut(&mut self) -> &mut Workspace {
        self.dirty = true;
        &mut self.workspace
    }

    pub fn set_workspace_theme_id_clean(&mut self, theme_id: &str) {
        self.workspace.settings.theme = theme_id.to_string();
    }

    // ── Focus management (delegated to FocusManager) ──────────────────────

    pub fn focus_next(&mut self) {
        self.focus.focus_next(&self.hosts);
    }

    pub fn focus_prev(&mut self) {
        self.focus.focus_prev(&self.hosts);
    }

    pub fn focused_panel_id(&self) -> Option<&str> {
        self.focus.focused_panel_id()
    }

    pub fn focus_order_index(&self, panel_id: &str) -> Option<usize> {
        self.focus.focus_order_index(panel_id)
    }

    pub fn set_focus_index(&mut self, idx: usize) {
        self.focus.set_focus_index(idx, &self.hosts);
    }

    pub fn host(&self, panel_id: &str) -> Option<&PanelHost> {
        self.hosts.get(panel_id)
    }

    pub fn hosts(&self) -> &HashMap<String, PanelHost> {
        &self.hosts
    }

    // ── Zoom (fullscreen single panel) ──────────────────────────────────

    pub fn is_zoomed(&self) -> bool {
        self.zoomed_panel.is_some()
    }

    /// Toggle zoom: focused panel takes the entire workspace area, or restore.
    /// Uses layout rebuild for reliability — no fragile reparenting.
    pub fn toggle_zoom(&mut self) {
        if let Some(zoomed_id) = self.zoomed_panel.take() {
            // Unzoom: reset button state and rebuild
            if let Some(host) = self.hosts.get(&zoomed_id) {
                host.set_zoom_active(false);
            }
            self.rebuild_layout();
            // Restore the active tab to the one that was zoomed
            if let Some(host) = self.hosts.get(&zoomed_id) {
                if let Some(notebook) = find_notebook_ancestor(host.widget()) {
                    let page = notebook.page_num(host.widget());
                    notebook.set_current_page(page);
                }
            }
        } else {
            // Zoom: show only the focused panel
            let focused_id = match self.focus.focused_panel_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            // Remove current layout tree
            self.root_box.remove(&self.root_widget);
            // Detach all panel hosts from their parents
            for host in self.hosts.values() {
                detach_widget(host.widget());
            }
            // Put focused panel directly in root_box
            if let Some(host) = self.hosts.get(&focused_id) {
                host.set_zoom_active(true);
                let w = host.widget().clone();
                w.set_vexpand(true);
                w.set_hexpand(true);
                self.root_box.prepend(&w);
            }
            self.zoomed_panel = Some(focused_id);
        }
    }

    /// Rebuild the GTK widget tree from the workspace layout model.
    /// Reuses existing PanelHost widgets (backends stay alive).
    fn rebuild_layout(&mut self) {
        tracing::debug!(
            "rebuild_layout: {} hosts, action_cb={}, type_chosen={}",
            self.hosts.len(),
            self.action_cb.is_some(),
            self.on_type_chosen.is_some()
        );
        // Remove everything from root_box
        while let Some(child) = self.root_box.first_child() {
            self.root_box.remove(&child);
        }
        // Detach all hosts from any parents (must succeed or layout breaks)
        for host in self.hosts.values() {
            detach_widget(host.widget());
            // Safety: if detach_widget didn't fully remove it, force unparent
            if host.widget().parent().is_some() {
                tracing::warn!("rebuild_layout: force unparent for {}", host.panel_id());
                host.widget().unparent();
            }
        }
        // Rebuild from model (passing action_cb so tab labels get close buttons)
        let edit_state = self.current_tab_label_edit_state();
        let root_widget = build_layout_widget_inner(
            &self.workspace.layout,
            &self.hosts,
            &self.workspace.panels,
            &self.action_cb,
            edit_state.as_ref(),
            &[],
        );
        root_widget.set_vexpand(true);
        root_widget.set_hexpand(true);
        self.root_box.prepend(&root_widget);
        self.root_widget = root_widget;
        self.connect_layout_change_watchers();

        // Reconnect all callbacks on all hosts + notebooks
        if let Some(ref cb) = self.action_cb {
            for host in self.hosts.values() {
                host.set_action_callback(cb.clone());
            }
            add_plus_buttons_recursive(&self.root_widget, cb);
        }

        // Reconnect terminal input sync callbacks
        if let Some(ref cb) = self.sync_input_cb {
            for host in self.hosts.values() {
                host.set_sync_input_callback(cb.clone());
            }
        }

        // Reconnect type chooser callbacks on chooser panels
        if let Some(ref tc) = self.on_type_chosen {
            let chooser_ids: Vec<String> = self
                .workspace
                .panels
                .iter()
                .filter(|p| p.effective_type() == PanelType::Empty)
                .map(|p| p.id.clone())
                .collect();
            for id in chooser_ids {
                if let Some(host) = self.hosts.get(&id) {
                    let chooser = ChooserPanel::new(&id, &self.registry, Some(tc.clone()));
                    host.set_backend(Box::new(chooser));
                }
            }
        }
    }

    // ── Sync input ───────────────────────────────────────────────────────

    /// Toggle sync-input on the focused panel. Returns (panel_id, is_now_synced).
    /// Returns `None` when the focused panel's backend does not opt into
    /// the sync feature (`PanelBackend::supports_sync` = false), so that
    /// Ctrl+Shift+S becomes a no-op on notes/chooser panels.
    pub fn toggle_sync_focused(&mut self) -> Option<(String, bool)> {
        let focused_id = self.focus.focused_panel_id()?.to_string();
        let is_synced = if self.sync_panels.contains(&focused_id) {
            self.sync_panels.remove(&focused_id);
            if let Some(host) = self.hosts.get(&focused_id) {
                host.clear_alert_border();
                host.set_sync_active(false);
            }
            false
        } else {
            let host = self.hosts.get(&focused_id)?;
            if !host.backend_supports_sync() {
                return None;
            }
            self.sync_panels.insert(focused_id.clone());
            host.set_alert_border("yellow");
            host.set_sync_active(true);
            true
        };
        Some((focused_id, is_synced))
    }

    /// Write input to all synced panels except the sender.
    pub fn write_to_synced(&self, data: &[u8], except: &str) {
        for panel_id in &self.sync_panels {
            if panel_id != except {
                if let Some(host) = self.hosts.get(panel_id) {
                    host.write_input(data);
                }
            }
        }
    }

    /// Check if a panel is in sync mode.
    pub fn is_panel_synced(&self, panel_id: &str) -> bool {
        self.sync_panels.contains(panel_id)
    }

    /// Connect terminal input handlers on all panel hosts for sync propagation.
    /// The callback is called with (source_panel_id, bytes) whenever a terminal
    /// receives local user input.
    pub fn setup_sync_callbacks(&mut self, cb: std::rc::Rc<dyn Fn(&str, &[u8])>) {
        self.sync_input_cb = Some(cb.clone());
        for host in self.hosts.values() {
            host.set_sync_input_callback(cb.clone());
        }
    }

    /// Number of panels currently in sync.
    pub fn sync_count(&self) -> usize {
        self.sync_panels.len()
    }

    /// Check if any panels are in sync mode.
    pub fn has_sync(&self) -> bool {
        !self.sync_panels.is_empty()
    }

    /// If the panel is currently in the synced group but its backend no
    /// longer supports sync (because the type was changed to chooser /
    /// notes / etc.), remove it and clear the per-panel visual cues.
    fn drop_sync_if_unsupported(&mut self, panel_id: &str) {
        let Some(host) = self.hosts.get(panel_id) else {
            return;
        };
        if host.backend_supports_sync() {
            return;
        }
        if self.sync_panels.remove(panel_id) {
            host.clear_alert_border();
            host.set_sync_active(false);
        }
    }

    /// Clear all sync panels.
    pub fn clear_sync(&mut self) {
        for panel_id in self.sync_panels.drain() {
            if let Some(host) = self.hosts.get(&panel_id) {
                host.clear_alert_border();
                host.set_sync_active(false);
            }
        }
    }

    // ── Split / Tab / Close ──────────────────────────────────────────────

    fn alloc_panel_id(&mut self) -> String {
        let id = format!("p{}", self.next_panel_id);
        self.next_panel_id += 1;
        id
    }

    /// Split horizontal = horizontal divider = new terminal below (Tilix convention).
    /// Split horizontal = horizontal divider = new terminal BELOW.
    /// In our model this is Vsplit. GTK Paned Vertical = stacked top/bottom.
    pub fn split_focused_h(&mut self) -> Option<String> {
        self.split_focused(gtk4::Orientation::Vertical)
    }

    /// Split vertical = vertical divider = new terminal to the RIGHT.
    /// In our model this is Hsplit. GTK Paned Horizontal = side by side.
    pub fn split_focused_v(&mut self) -> Option<String> {
        self.split_focused(gtk4::Orientation::Horizontal)
    }

    fn split_focused(&mut self, orientation: gtk4::Orientation) -> Option<String> {
        let focused_id = self.focused_panel_id()?.to_string();
        tracing::info!("Split panel {} orientation={:?}", focused_id, orientation);
        let new_id = self.alloc_panel_id();
        let new_name = format!("New Panel {}", &new_id[1..]);

        // 1. Create new panel config + host
        let new_cfg = PanelConfig {
            id: new_id.clone(),
            name: new_name.clone(),
            panel_type: PanelType::Empty,
            target: Default::default(),
            startup_commands: vec![],
            groups: vec![],
            record_output: false,
            cwd: None,
            env: Default::default(),
            pre_script: None,
            post_script: None,
            before_close: None,
            min_width: 0,
            min_height: 0,
            ssh: None,
        };
        let host = PanelHost::new(&new_id, &new_name, self.action_cb.clone());
        self.wire_sibling_info_provider_on(&host);
        let backend = self.create_chooser_backend(&new_id);
        host.set_backend(backend);

        // 2. Update model
        self.update_layout_split(&focused_id, &new_id, orientation);
        self.workspace.panels.push(new_cfg);
        self.hosts.insert(new_id.clone(), host);
        crate::layout_ops::debug_layout_tree(&self.workspace.layout, "BEFORE_REBUILD_SPLIT");

        // 3. Rebuild widget tree from model
        self.rebuild_layout();
        self.rebuild_focus_order();

        // 4. Focus the newly created split pane and reveal all ancestor tabs.
        self.focus_panel_after_rebuild(&new_id);

        Some(new_id)
    }

    /// Insert a new panel as a sibling in the nearest ancestor split.
    pub fn insert_sibling_focused(
        &mut self,
        position: crate::layout_ops::InsertPosition,
    ) -> Option<String> {
        let focused_id = self.focused_panel_id()?.to_string();
        let new_id = self.alloc_panel_id();
        let new_name = format!("New Panel {}", &new_id[1..]);

        let new_cfg = self.make_empty_config(&new_id, &new_name);
        let host = PanelHost::new(&new_id, &new_name, self.action_cb.clone());
        self.wire_sibling_info_provider_on(&host);
        let backend = self.create_chooser_backend(&new_id);
        host.set_backend(backend);

        if !crate::layout_ops::insert_sibling_in_layout(
            &mut self.workspace.layout,
            &focused_id,
            &new_id,
            position,
        ) {
            tracing::warn!(
                "insert_sibling: could not find a parent split for {}",
                focused_id
            );
            return None;
        }

        self.workspace.panels.push(new_cfg);
        self.hosts.insert(new_id.clone(), host);

        self.rebuild_layout();
        self.rebuild_focus_order();
        self.focus_panel_after_rebuild(&new_id);

        Some(new_id)
    }

    /// Wrap the focused panel in a new TabSplit (Notebook) with a second tab.
    pub fn add_tab_focused(&mut self) -> Option<String> {
        let focused_id = self.focused_panel_id()?.to_string();
        let new_id = self.alloc_panel_id();
        let new_name = format!("New Panel {}", &new_id[1..]);

        let new_cfg = self.make_empty_config(&new_id, &new_name);
        let host = PanelHost::new(&new_id, &new_name, self.action_cb.clone());
        self.wire_sibling_info_provider_on(&host);
        let backend = self.create_chooser_backend(&new_id);
        host.set_backend(backend);

        // Update model: wrap focused panel in Tabs node
        let existing_label = self
            .workspace
            .panel(&focused_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| focused_id.clone());
        self.workspace.layout =
            replace_in_layout(&self.workspace.layout, &focused_id, &|_| LayoutNode::Tabs {
                children: vec![
                    LayoutNode::Panel {
                        id: focused_id.clone(),
                    },
                    LayoutNode::Panel { id: new_id.clone() },
                ],
                labels: vec![existing_label.clone(), new_name.clone()],
                tab_ids: vec![new_tab_id(), new_tab_id()],
            });
        self.workspace.panels.push(new_cfg);
        self.hosts.insert(new_id.clone(), host);

        // Rebuild widget tree
        self.rebuild_layout();
        self.rebuild_focus_order();
        self.focus_panel_after_rebuild(&new_id);

        Some(new_id)
    }

    /// Add a new tab to the exact Tabs node identified by its layout path.
    pub fn add_tab_to_tabs_path(&mut self, tabs_path: &[usize]) -> Option<String> {
        let new_id = self.alloc_panel_id();
        let new_name = format!("New Panel {}", &new_id[1..]);

        let new_cfg = self.make_empty_config(&new_id, &new_name);
        let host = PanelHost::new(&new_id, &new_name, self.action_cb.clone());
        self.wire_sibling_info_provider_on(&host);
        let backend = self.create_chooser_backend(&new_id);
        host.set_backend(backend);

        // Update model
        if !crate::layout_ops::add_to_tabs_at_path(
            &mut self.workspace.layout,
            tabs_path,
            &new_id,
            &new_name,
            &new_tab_id(),
        ) {
            return None;
        }
        self.workspace.panels.push(new_cfg);
        self.hosts.insert(new_id.clone(), host);

        // Rebuild widget tree
        self.rebuild_layout();
        self.rebuild_focus_order();
        self.focus_panel_after_rebuild(&new_id);

        Some(new_id)
    }

    fn select_workspace_tab_for_panel(&self, panel_id: &str) -> bool {
        select_workspace_tabs_for_panel(&self.root_widget, &self.workspace.layout, panel_id)
    }

    fn make_empty_config(&self, id: &str, name: &str) -> PanelConfig {
        PanelConfig {
            id: id.to_string(),
            name: name.to_string(),
            panel_type: PanelType::Empty,
            target: Default::default(),
            startup_commands: vec![],
            groups: vec![],
            record_output: false,
            cwd: None,
            env: Default::default(),
            pre_script: None,
            post_script: None,
            before_close: None,
            min_width: 0,
            min_height: 0,
            ssh: None,
        }
    }

    fn create_chooser_backend(&self, panel_id: &str) -> Box<dyn crate::panels::PanelBackend> {
        Box::new(ChooserPanel::new(
            panel_id,
            &self.registry,
            self.on_type_chosen.clone(),
        ))
    }

    /// Close the focused panel. Uses model update + layout rebuild for reliability.
    /// Peek: if the focused panel needs a user confirmation before closing,
    /// return its prompt text. Callers use this BEFORE `close_focused` to
    /// decide whether to show a confirmation dialog.
    ///
    /// `None` can mean either "no confirmation needed" OR "nothing to close
    /// (last panel)" — callers should still check `close_focused`'s return.
    pub fn close_focused_prompt(&self) -> Option<String> {
        if self.focus.order.len() <= 1 {
            return None;
        }
        let focused_id = self.focused_panel_id()?;
        self.hosts.get(focused_id).and_then(|h| h.close_confirmation())
    }

    pub fn close_focused(&mut self) -> bool {
        if self.focus.order.len() <= 1 {
            return false; // Don't close the last panel
        }

        let focused_id = match self.focused_panel_id() {
            Some(id) => id.to_string(),
            None => return false,
        };

        // Determine the best panel to focus after closing: the previous tab
        // sibling in the same Tabs node (or the next one if we're at index 0).
        // Falls back to the generic focus-order approach when the panel isn't
        // inside a Tabs or is the only child.
        let focus_after_close = crate::layout_ops::adjacent_tab_sibling_panel(
            &self.workspace.layout,
            &focused_id,
        );

        // Run before_close script
        self.run_before_close(&focused_id);

        // 1. Update model: remove panel from layout tree and panels list
        self.update_layout_remove(&focused_id);

        // If the panel ID is still in the layout (empty Tabs fallback), replace
        // it with a fresh empty panel so the user sees the type chooser.
        if self
            .workspace
            .layout
            .panel_ids()
            .iter()
            .any(|id| *id == focused_id)
        {
            let new_id = self.alloc_panel_id();
            let new_name = format!("New Panel {}", &new_id[1..]);
            let new_config = self.make_empty_config(&new_id, &new_name);
            self.workspace.layout = replace_in_layout(&self.workspace.layout, &focused_id, &|_| {
                LayoutNode::Panel { id: new_id.clone() }
            });
            let backend = self.create_chooser_backend(&new_id);
            let host = PanelHost::new(&new_id, &new_config.name, self.action_cb.clone());
            self.wire_sibling_info_provider_on(&host);
            host.set_backend(backend);
            self.hosts.insert(new_id.clone(), host);
            self.workspace.panels.push(new_config);
        }

        self.workspace.panels.retain(|p| p.id != focused_id);

        // 2. Shut down the backend (terminate child process), fire the
        //    permanent-close signal (so backends can delete per-instance
        //    persisted state), detach widget, drop the host.
        if let Some(host) = self.hosts.remove(&focused_id) {
            host.permanent_close_backend();
            detach_widget(host.widget());
        }

        // 3. Rebuild the widget tree from the updated model
        self.rebuild_layout();
        self.rebuild_focus_order();

        // 4. Focus the previous tab sibling if we found one; otherwise fall
        //    back to the nearest panel in the flat focus order.
        if let Some(ref target_id) = focus_after_close {
            self.focus_panel_after_rebuild(target_id);
        } else {
            if self.focus.index >= self.focus.order.len() {
                self.focus.index = self.focus.order.len().saturating_sub(1);
            }
            if let Some(target_id) = self.focus.focused_panel_id().map(|id| id.to_string()) {
                self.focus_panel_after_rebuild(&target_id);
            }
        }

        true
    }

    // ── Save ─────────────────────────────────────────────────────────────

    /// Sync ratios from GTK widget positions back into the layout model.
    fn sync_ratios_from_widgets(&mut self) {
        sync_ratios_recursive(&self.root_widget, &mut self.workspace.layout);
    }

    pub fn sync_ratios_from_widgets_if_changed(&mut self) -> bool {
        let mut synced_layout = self.workspace.layout.clone();
        sync_ratios_recursive(&self.root_widget, &mut synced_layout);
        self.apply_synced_layout_if_changed(synced_layout)
    }

    /// Save the current workspace to the original config file.
    pub fn save(&mut self) -> Result<PathBuf, String> {
        self.sync_ratios_from_widgets();
        let path = self
            .config_path
            .as_ref()
            .ok_or("No config path set")?
            .clone();
        pax_core::config::save_workspace(&self.workspace, &path)
            .map_err(|e| format!("Save failed: {}", e))?;
        tracing::info!(
            "Saved {} panels to {}",
            self.workspace.panels.len(),
            path.display()
        );
        for p in &self.workspace.panels {
            if !p.startup_commands.is_empty() {
                tracing::debug!(
                    "  {} startup: {:?}",
                    p.id,
                    &p.startup_commands[0][..p.startup_commands[0].len().min(80)]
                );
            }
            if let Some(ref bc) = p.before_close {
                tracing::debug!("  {} before_close: {:?}", p.id, &bc[..bc.len().min(80)]);
            }
        }
        self.sync_saved_workspace_record(&path);
        self.dirty = false;
        Ok(path)
    }

    /// Save to a specific path.
    pub fn save_as(&mut self, path: &Path) -> Result<(), String> {
        self.sync_ratios_from_widgets();
        pax_core::config::save_workspace(&self.workspace, path)
            .map_err(|e| format!("Save failed: {}", e))?;
        self.config_path = Some(path.to_path_buf());
        self.sync_saved_workspace_record(path);
        self.dirty = false;
        Ok(())
    }

    fn record_in_db(&self) {
        let db_path = pax_db::Database::default_path();
        if let Ok(db) = pax_db::Database::open(&db_path) {
            let config_str = self
                .config_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string());
            db.record_workspace_open(&self.workspace.name, config_str.as_deref())
                .ok();
        }
    }

    fn sync_saved_workspace_record(&self, path: &Path) {
        let db_path = pax_db::Database::default_path();
        if let Ok(db) = pax_db::Database::open(&db_path) {
            let path_str = path.to_string_lossy().to_string();
            db.sync_workspace_path(&self.workspace.name, &path_str).ok();
        }
    }

    // ── Layout model updates ─────────────────────────────────────────────

    fn rebuild_focus_order(&mut self) {
        self.dirty = true;
        let ids: Vec<String> = self
            .workspace
            .layout
            .panel_ids()
            .iter()
            .map(|s| s.to_string())
            .collect();
        self.focus.rebuild(ids);
    }

    fn connect_layout_change_watchers(&self) {
        let Some(callback) = self.layout_change_cb.as_ref() else {
            return;
        };
        let root_widget = self.root_widget.clone();
        let callback = callback.clone();
        gtk4::glib::idle_add_local_once(move || {
            connect_paned_position_watchers(&root_widget, &callback);
        });
    }

    fn apply_synced_layout_if_changed(&mut self, synced_layout: LayoutNode) -> bool {
        if self.workspace.layout == synced_layout {
            return false;
        }
        // f64 → i32 (paned position) → f64 round-trip introduces sub-pixel
        // ratio drift on every layout settle, which would otherwise mark
        // the workspace dirty the moment it opens. Only flip the dirty
        // flag for user-meaningful changes; always update the model.
        let meaningful = layout_differs_meaningfully(&self.workspace.layout, &synced_layout);
        self.workspace.layout = synced_layout;
        if meaningful {
            self.dirty = true;
        }
        meaningful
    }

    fn update_layout_split(
        &mut self,
        existing_id: &str,
        new_id: &str,
        orientation: gtk4::Orientation,
    ) {
        self.workspace.layout = replace_in_layout(&self.workspace.layout, existing_id, &|_| {
            let children = vec![
                LayoutNode::Panel {
                    id: existing_id.to_string(),
                },
                LayoutNode::Panel {
                    id: new_id.to_string(),
                },
            ];
            match orientation {
                gtk4::Orientation::Horizontal => LayoutNode::Hsplit {
                    children,
                    ratios: vec![0.5, 0.5],
                },
                _ => LayoutNode::Vsplit {
                    children,
                    ratios: vec![0.5, 0.5],
                },
            }
        });
    }

    fn update_layout_remove(&mut self, panel_id: &str) {
        self.workspace.layout = remove_from_layout(&self.workspace.layout, panel_id);
    }

    /// Broadcast input to all panels in a group.
    pub fn broadcast_to_group(&self, group_name: &str, data: &[u8]) {
        let panel_ids: Vec<String> = self
            .workspace
            .panels
            .iter()
            .filter(|p| p.groups.iter().any(|g| g == group_name))
            .map(|p| p.id.clone())
            .collect();

        for pid in &panel_ids {
            if let Some(host) = self.hosts.get(pid) {
                host.write_input(data);
            }
        }
    }
}

fn rename_panel_model(workspace: &mut Workspace, panel_id: &str, new_name: &str) -> bool {
    // Only touches `panel_cfg.name` (the panel's own display name shown in
    // the host title bar). Tab labels are user-controlled via the tab
    // rename UI and intentionally not synced here — `update_tab_label_in_layout`
    // walks ancestors recursively, so syncing would clobber an outer Tabs
    // label whenever the panel is nested inside Hsplit/Vsplit.
    if let Some(panel_cfg) = workspace
        .panels
        .iter_mut()
        .find(|panel| panel.id == panel_id)
    {
        if panel_cfg.name != new_name {
            panel_cfg.name = new_name.to_string();
            return true;
        }
    }
    false
}

fn rename_tab_label_model(layout: &mut LayoutNode, panel_id: &str, new_name: &str) -> bool {
    crate::layout_ops::update_tab_label_in_layout(layout, panel_id, new_name)
}

fn rename_tab_label_model_by_id(layout: &mut LayoutNode, tab_id: &str, new_name: &str) -> bool {
    crate::layout_ops::update_tab_label_in_layout_by_id(layout, tab_id, new_name)
}

fn find_layout_path_to_panel(node: &LayoutNode, panel_id: &str) -> Option<Vec<usize>> {
    match node {
        LayoutNode::Panel { id } => (id == panel_id).then(Vec::new),
        LayoutNode::Tabs { children, .. }
        | LayoutNode::Hsplit { children, .. }
        | LayoutNode::Vsplit { children, .. } => {
            for (index, child) in children.iter().enumerate() {
                if let Some(mut path) = find_layout_path_to_panel(child, panel_id) {
                    path.insert(0, index);
                    return Some(path);
                }
            }
            None
        }
    }
}

fn collect_tabs_along_path(
    node: &LayoutNode,
    panel_path: &[usize],
    current_node_path: &mut Vec<usize>,
    tabs_to_select: &mut Vec<(Vec<usize>, u32)>,
) {
    let Some((&index, rest)) = panel_path.split_first() else {
        return;
    };

    match node {
        LayoutNode::Tabs { children, .. } => {
            tabs_to_select.push((current_node_path.clone(), index as u32));
            if let Some(child) = children.get(index) {
                current_node_path.push(index);
                collect_tabs_along_path(child, rest, current_node_path, tabs_to_select);
                current_node_path.pop();
            }
        }
        LayoutNode::Hsplit { children, .. } | LayoutNode::Vsplit { children, .. } => {
            if let Some(child) = children.get(index) {
                current_node_path.push(index);
                collect_tabs_along_path(child, rest, current_node_path, tabs_to_select);
                current_node_path.pop();
            }
        }
        LayoutNode::Panel { .. } => {}
    }
}

fn find_workspace_notebook_by_path(
    widget: &gtk4::Widget,
    tabs_path: &[usize],
) -> Option<gtk4::Notebook> {
    if let Ok(notebook) = widget.clone().downcast::<gtk4::Notebook>() {
        if notebook.has_css_class("workspace-tabs")
            && crate::widget_builder::decode_tabs_widget_name(&notebook.widget_name()).as_deref()
                == Some(tabs_path)
        {
            return Some(notebook);
        }
    }

    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(notebook) = find_workspace_notebook_by_path(&current, tabs_path) {
            return Some(notebook);
        }
        child = current.next_sibling();
    }
    None
}

fn select_workspace_tabs_for_panel(
    root_widget: &gtk4::Widget,
    layout: &LayoutNode,
    panel_id: &str,
) -> bool {
    let Some(panel_path) = find_layout_path_to_panel(layout, panel_id) else {
        return false;
    };

    let mut tabs_to_select = Vec::new();
    collect_tabs_along_path(layout, &panel_path, &mut Vec::new(), &mut tabs_to_select);
    if tabs_to_select.is_empty() {
        return true;
    }

    let mut selected_any = false;
    for (tabs_path, page_index) in tabs_to_select {
        if let Some(notebook) = find_workspace_notebook_by_path(root_widget, &tabs_path) {
            notebook.set_current_page(Some(page_index));
            selected_any = true;
        }
    }
    selected_any
}

/// Whether two layout trees differ in any way a user would notice. Same
/// shape as `PartialEq` except split ratios compare with a small epsilon
/// so f64 → pixel → f64 round-trip drift doesn't count as a real change.
/// Anything else (children identity, tab labels/ids, nesting) compares
/// exactly.
fn layout_differs_meaningfully(a: &LayoutNode, b: &LayoutNode) -> bool {
    const RATIO_EPS: f64 = 1e-4;
    match (a, b) {
        (LayoutNode::Panel { id: ai }, LayoutNode::Panel { id: bi }) => ai != bi,
        (
            LayoutNode::Hsplit {
                children: ac,
                ratios: ar,
            },
            LayoutNode::Hsplit {
                children: bc,
                ratios: br,
            },
        )
        | (
            LayoutNode::Vsplit {
                children: ac,
                ratios: ar,
            },
            LayoutNode::Vsplit {
                children: bc,
                ratios: br,
            },
        ) => {
            if ac.len() != bc.len() || ar.len() != br.len() {
                return true;
            }
            if ar.iter().zip(br.iter()).any(|(x, y)| (x - y).abs() > RATIO_EPS) {
                return true;
            }
            ac.iter()
                .zip(bc.iter())
                .any(|(x, y)| layout_differs_meaningfully(x, y))
        }
        (
            LayoutNode::Tabs {
                children: ac,
                labels: al,
                tab_ids: at,
            },
            LayoutNode::Tabs {
                children: bc,
                labels: bl,
                tab_ids: bt,
            },
        ) => {
            if al != bl || at != bt || ac.len() != bc.len() {
                return true;
            }
            ac.iter()
                .zip(bc.iter())
                .any(|(x, y)| layout_differs_meaningfully(x, y))
        }
        _ => true,
    }
}

fn connect_paned_position_watchers(widget: &gtk4::Widget, callback: &Rc<dyn Fn()>) {
    if let Ok(paned) = widget.clone().downcast::<gtk4::Paned>() {
        let cb = callback.clone();
        let last_position = std::rc::Rc::new(std::cell::Cell::new(paned.position()));
        let last_position_ref = last_position.clone();
        let watched_paned = paned.clone();
        paned.connect_notify_local(Some("position"), move |_, _| {
            let current_position = watched_paned.position();
            if current_position == last_position_ref.get() {
                return;
            }
            last_position_ref.set(current_position);
            cb();
        });
    }

    let mut child = widget.first_child();
    while let Some(current) = child {
        connect_paned_position_watchers(&current, callback);
        child = current.next_sibling();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(children: Vec<LayoutNode>, labels: &[&str]) -> LayoutNode {
        LayoutNode::Tabs {
            children,
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            tab_ids: (0..labels.len()).map(|_| new_tab_id()).collect(),
        }
    }

    fn panel(id: &str) -> LayoutNode {
        LayoutNode::Panel { id: id.to_string() }
    }

    fn panel_config(id: &str, name: &str) -> PanelConfig {
        PanelConfig {
            id: id.to_string(),
            name: name.to_string(),
            panel_type: PanelType::Terminal,
            target: Default::default(),
            startup_commands: Vec::new(),
            groups: Vec::new(),
            record_output: false,
            cwd: None,
            env: Default::default(),
            pre_script: None,
            post_script: None,
            before_close: None,
            min_width: 0,
            min_height: 0,
            ssh: None,
        }
    }

    fn sample_workspace() -> Workspace {
        Workspace {
            name: "demo".to_string(),
            id: uuid::Uuid::new_v4(),
            layout: tabs(vec![panel("a"), panel("b")], &["tab-a", "tab-b"]),
            panels: vec![panel_config("a", "Panel A"), panel_config("b", "Panel B")],
            groups: Vec::new(),
            alerts: Vec::new(),
            startup_script: None,
            notes_file: None,
            settings: Default::default(),
            ssh_configs: Vec::new(),
        }
    }

    #[test]
    fn rename_panel_model_updates_panel_name_only() {
        let mut workspace = sample_workspace();

        let changed = rename_panel_model(&mut workspace, "a", "Renamed A");

        assert!(changed);
        assert_eq!(
            workspace
                .panels
                .iter()
                .find(|panel| panel.id == "a")
                .unwrap()
                .name,
            "Renamed A"
        );
        // Tab label must not be touched by panel rename — it's user-owned.
        match &workspace.layout {
            LayoutNode::Tabs { labels, .. } => assert_eq!(labels[0], "tab-a"),
            _ => panic!("expected tabs layout"),
        }
    }

    #[test]
    fn rename_tab_label_model_only_updates_layout_labels() {
        let mut workspace = sample_workspace();

        let changed = rename_tab_label_model(&mut workspace.layout, "a", "Custom Tab");

        assert!(changed);
        assert_eq!(
            workspace
                .panels
                .iter()
                .find(|panel| panel.id == "a")
                .unwrap()
                .name,
            "Panel A"
        );
        match &workspace.layout {
            LayoutNode::Tabs { labels, .. } => assert_eq!(labels[0], "Custom Tab"),
            _ => panic!("expected tabs layout"),
        }
    }

    #[test]
    fn rename_tab_label_model_by_id_updates_exact_nested_tab() {
        let mut workspace = Workspace {
            name: "demo".to_string(),
            id: uuid::Uuid::new_v4(),
            layout: tabs(
                vec![
                    LayoutNode::Vsplit {
                        children: vec![
                            panel("a"),
                            tabs(vec![panel("b"), panel("d")], &["freeflow", "freeflow-web"]),
                        ],
                        ratios: vec![1.0, 1.0],
                    },
                    panel("c"),
                ],
                &["outer", "other"],
            ),
            panels: vec![
                panel_config("a", "Panel A"),
                panel_config("b", "Panel B"),
                panel_config("c", "Panel C"),
                panel_config("d", "Panel D"),
            ],
            groups: Vec::new(),
            alerts: Vec::new(),
            startup_script: None,
            notes_file: None,
            settings: Default::default(),
            ssh_configs: Vec::new(),
        };

        let target_tab_id = if let LayoutNode::Tabs { children, .. } = &workspace.layout {
            if let LayoutNode::Vsplit { children, .. } = &children[0] {
                if let LayoutNode::Tabs { tab_ids, .. } = &children[1] {
                    tab_ids[1].clone()
                } else {
                    panic!("expected nested tabs");
                }
            } else {
                panic!("expected vsplit");
            }
        } else {
            panic!("expected root tabs");
        };

        let changed =
            rename_tab_label_model_by_id(&mut workspace.layout, &target_tab_id, "freeflow");

        assert!(changed);
        if let LayoutNode::Tabs {
            children, labels, ..
        } = &workspace.layout
        {
            assert_eq!(labels[0], "outer");
            assert_eq!(labels[1], "other");
            if let LayoutNode::Vsplit { children, .. } = &children[0] {
                if let LayoutNode::Tabs { labels, .. } = &children[1] {
                    assert_eq!(labels[0], "freeflow");
                    assert_eq!(labels[1], "freeflow");
                } else {
                    panic!("expected nested tabs");
                }
            } else {
                panic!("expected vsplit");
            }
        } else {
            panic!("expected root tabs");
        }
    }

    #[test]
    fn move_tab_by_panel_id_reorders_layout_labels() {
        let mut workspace = sample_workspace();

        let moved = crate::layout_ops::move_tab_in_layout(&mut workspace.layout, "b", -1);

        assert!(moved);
        match &workspace.layout {
            LayoutNode::Tabs {
                labels, children, ..
            } => {
                assert_eq!(labels, &["tab-b", "tab-a"]);
                assert!(matches!(&children[0], LayoutNode::Panel { id } if id == "b"));
                assert!(matches!(&children[1], LayoutNode::Panel { id } if id == "a"));
            }
            _ => panic!("expected tabs layout"),
        }
    }

    #[test]
    fn move_tab_by_panel_id_supports_multi_step_offsets() {
        let mut layout = LayoutNode::Tabs {
            children: vec![
                LayoutNode::Panel {
                    id: "a".to_string(),
                },
                LayoutNode::Panel {
                    id: "b".to_string(),
                },
                LayoutNode::Panel {
                    id: "c".to_string(),
                },
            ],
            labels: vec!["tab-a".into(), "tab-b".into(), "tab-c".into()],
            tab_ids: vec![new_tab_id(), new_tab_id(), new_tab_id()],
        };

        let moved = crate::layout_ops::move_tab_in_layout_steps(&mut layout, "a", 2);

        assert!(moved);
        match &layout {
            LayoutNode::Tabs {
                labels, children, ..
            } => {
                assert_eq!(labels, &["tab-b", "tab-c", "tab-a"]);
                assert!(matches!(&children[0], LayoutNode::Panel { id } if id == "b"));
                assert!(matches!(&children[1], LayoutNode::Panel { id } if id == "c"));
                assert!(matches!(&children[2], LayoutNode::Panel { id } if id == "a"));
            }
            _ => panic!("expected tabs layout"),
        }
    }

    #[test]
    fn find_layout_path_to_panel_tracks_nested_tabs() {
        let layout = tabs(
            vec![
                LayoutNode::Vsplit {
                    children: vec![
                        panel("a"),
                        tabs(vec![panel("b"), panel("c")], &["inner-1", "inner-2"]),
                    ],
                    ratios: vec![1.0, 1.0],
                },
                panel("d"),
            ],
            &["outer-1", "outer-2"],
        );

        assert_eq!(find_layout_path_to_panel(&layout, "c"), Some(vec![0, 1, 1]));
        assert_eq!(find_layout_path_to_panel(&layout, "d"), Some(vec![1]));
        assert_eq!(find_layout_path_to_panel(&layout, "missing"), None);
    }

    #[test]
    fn select_workspace_tabs_for_panel_reveals_ancestor_tabs() {
        if gtk4::init().is_err() {
            return;
        }

        let root = gtk4::Notebook::new();
        root.add_css_class("workspace-tabs");
        root.set_widget_name(&crate::widget_builder::encode_tabs_widget_name(&[]));
        let root_page_0 = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let root_page_1 = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.append_page(&root_page_0, Some(&gtk4::Label::new(Some("Root 0"))));
        root.append_page(&root_page_1, Some(&gtk4::Label::new(Some("Root 1"))));

        let nested = gtk4::Notebook::new();
        nested.add_css_class("workspace-tabs");
        nested.set_widget_name(&crate::widget_builder::encode_tabs_widget_name(&[1]));
        root_page_1.append(&nested);

        let nested_page_0 = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let nested_page_1 = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        let panel_1 = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        panel_1.add_css_class("panel-frame");
        panel_1.set_widget_name("p1");
        nested_page_0.append(&panel_1);

        let panel_2 = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        panel_2.add_css_class("panel-frame");
        panel_2.set_widget_name("p2");
        nested_page_1.append(&panel_2);

        nested.append_page(&nested_page_0, Some(&gtk4::Label::new(Some("Inner 0"))));
        nested.append_page(&nested_page_1, Some(&gtk4::Label::new(Some("Inner 1"))));

        root.set_current_page(Some(0));
        nested.set_current_page(Some(0));

        let layout = tabs(
            vec![
                panel("root-placeholder"),
                tabs(vec![panel("p1"), panel("p2")], &["Inner 0", "Inner 1"]),
            ],
            &["Root 0", "Root 1"],
        );

        let selected = select_workspace_tabs_for_panel(&root.clone().upcast(), &layout, "p2");

        assert!(selected);
        assert_eq!(root.current_page(), Some(1));
        assert_eq!(nested.current_page(), Some(1));
    }

    #[test]
    fn split_focused_in_nested_tabs_keeps_nested_selection() {
        if gtk4::init().is_err() {
            return;
        }

        let workspace = Workspace {
            name: "demo".to_string(),
            id: uuid::Uuid::new_v4(),
            layout: tabs(
                vec![
                    LayoutNode::Vsplit {
                        children: vec![
                            tabs(vec![panel("a"), panel("b")], &["inner-a", "inner-b"]),
                            panel("c"),
                        ],
                        ratios: vec![1.0, 1.0],
                    },
                    panel("d"),
                ],
                &["outer-left", "outer-right"],
            ),
            panels: vec![
                PanelConfig {
                    panel_type: PanelType::Empty,
                    ..panel_config("a", "Panel A")
                },
                PanelConfig {
                    panel_type: PanelType::Empty,
                    ..panel_config("b", "Panel B")
                },
                PanelConfig {
                    panel_type: PanelType::Empty,
                    ..panel_config("c", "Panel C")
                },
                PanelConfig {
                    panel_type: PanelType::Empty,
                    ..panel_config("d", "Panel D")
                },
            ],
            groups: Vec::new(),
            alerts: Vec::new(),
            startup_script: None,
            notes_file: None,
            settings: Default::default(),
            ssh_configs: Vec::new(),
        };

        let mut view = WorkspaceView::build(&workspace, None);
        let focused = view.focus_order_index("a").expect("panel a in focus order");
        view.set_focus_index(focused);

        let new_id = view
            .split_focused_v()
            .expect("split should create a new panel");

        let context = gtk4::glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }

        let root = find_workspace_notebook_by_path(&view.root_widget, &[])
            .expect("root workspace notebook");
        let nested = find_workspace_notebook_by_path(&view.root_widget, &[0, 0])
            .expect("nested workspace notebook");

        assert_eq!(view.focused_panel_id(), Some(new_id.as_str()));
        assert_eq!(root.current_page(), Some(0));
        assert_eq!(nested.current_page(), Some(0));
    }

    #[test]
    fn apply_synced_layout_if_changed_marks_workspace_dirty() {
        if gtk4::init().is_err() {
            return;
        }
        let mut workspace = sample_workspace();
        workspace.layout = LayoutNode::Hsplit {
            children: vec![panel("a"), panel("b")],
            ratios: vec![0.5, 0.5],
        };
        workspace.panels = vec![panel_config("a", "Panel A"), panel_config("b", "Panel B")];

        let mut view = WorkspaceView::build(&workspace, None);
        assert!(!view.is_dirty());

        let changed = view.apply_synced_layout_if_changed(LayoutNode::Hsplit {
            children: vec![panel("a"), panel("b")],
            ratios: vec![0.7, 0.3],
        });

        assert!(changed);
        assert!(view.is_dirty());
        match view.workspace().layout {
            LayoutNode::Hsplit { ref ratios, .. } => assert_eq!(ratios, &vec![0.7, 0.3]),
            _ => panic!("expected hsplit layout"),
        }
    }

    #[test]
    fn apply_synced_layout_if_changed_ignores_identical_layout() {
        if gtk4::init().is_err() {
            return;
        }
        let workspace = sample_workspace();
        let mut view = WorkspaceView::build(&workspace, None);
        let changed = view.apply_synced_layout_if_changed(view.workspace().layout.clone());
        assert!(!changed);
        assert!(!view.is_dirty());
    }
}

// Free functions moved to widget_builder.rs and backend_factory.rs

/// Walk the workspace layout and write a breadcrumb (e.g. "tab1 ›
/// shells › left-shell") into `terminal_registry` for each terminal
/// panel, so the notebook run-target picker can disambiguate them.
/// Re-run after any layout mutation to keep entries fresh.
pub(crate) fn publish_terminal_breadcrumbs(workspace: &Workspace) {
    use pax_core::workspace::{LayoutNode, PanelType};
    fn walk(node: &LayoutNode, trail: &mut Vec<String>, out: &mut HashMap<String, Vec<String>>) {
        match node {
            LayoutNode::Panel { id } => {
                out.insert(id.clone(), trail.clone());
            }
            LayoutNode::Tabs { children, labels, .. } => {
                for (i, child) in children.iter().enumerate() {
                    let label = labels
                        .get(i)
                        .cloned()
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| format!("tab{}", i + 1));
                    trail.push(label);
                    walk(child, trail, out);
                    trail.pop();
                }
            }
            LayoutNode::Hsplit { children, .. } | LayoutNode::Vsplit { children, .. } => {
                for child in children {
                    walk(child, trail, out);
                }
            }
        }
    }
    let mut trails: HashMap<String, Vec<String>> = HashMap::new();
    walk(&workspace.layout, &mut Vec::new(), &mut trails);
    for cfg in &workspace.panels {
        if cfg.effective_type() != PanelType::Terminal {
            continue;
        }
        let trail = trails.get(&cfg.id).cloned().unwrap_or_default();
        let mut path = trail;
        if !cfg.name.is_empty() {
            path.push(cfg.name.clone());
        }
        let breadcrumb = path.join(" › ");
        crate::panels::terminal_registry::set_breadcrumb(&cfg.id, &breadcrumb);
    }
}
