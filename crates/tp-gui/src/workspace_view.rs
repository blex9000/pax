use gtk4::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tp_core::workspace::{LayoutNode, PanelConfig, PanelType, Workspace};

use crate::focus::FocusManager;
use crate::layout_ops::{replace_in_layout, remove_from_layout, add_to_existing_tabs};
use crate::panel_host::{PanelAction, PanelActionCallback, PanelHost};
use crate::panels::chooser::{ChooserPanel, OnTypeChosen};
use crate::panels::registry::{self, PanelCreateConfig, PanelRegistry};
use crate::panels::markdown::MarkdownPanel;

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
    registry: PanelRegistry,
    on_type_chosen: Option<OnTypeChosen>,
    dirty: bool,
    /// When a panel is zoomed (fullscreen), store which panel and hidden siblings.
    zoomed_panel: Option<String>,
    /// Panel IDs that are in sync-input mode.
    sync_panels: std::collections::HashSet<String>,
    /// Callback for VTE commit sync propagation.
    #[cfg(feature = "vte")]
    sync_commit_cb: Option<std::rc::Rc<dyn Fn(&str, &str)>>,
}

impl WorkspaceView {
    /// Build the workspace view from a workspace config.
    /// Call `set_action_callback` after wrapping in Rc<RefCell<>> to enable menu actions.
    pub fn build(workspace: &Workspace, config_path: Option<&Path>) -> Self {
        let registry = registry::build_default_registry();
        let ws_dir = config_path.and_then(|p| p.parent()).map(|p| p.to_string_lossy().to_string());
        let mut hosts = HashMap::new();

        for panel_cfg in &workspace.panels {
            let host = PanelHost::new(&panel_cfg.id, &panel_cfg.name, None);
            if panel_cfg.effective_type() == PanelType::Empty {
                let chooser = ChooserPanel::new(&panel_cfg.id, &registry, None);
                host.set_backend(Box::new(chooser));
            } else {
                let backend = create_backend_from_registry(panel_cfg, &workspace.settings.default_shell, &registry, ws_dir.as_deref());
                host.set_backend(backend);
            }
            apply_min_size(&host, panel_cfg);
            hosts.insert(panel_cfg.id.clone(), host);
        }

        // Build layout widget tree
        let root_widget = build_layout_widget(&workspace.layout, &hosts, &workspace.panels);
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
            .filter_map(|p| {
                p.id.strip_prefix('p')
                    .and_then(|n| n.parse::<usize>().ok())
            })
            .max()
            .unwrap_or(0)
            + 1;

        let mut view = Self {
            root_widget,
            root_box,
            scrolled,
            hosts,
            focus: FocusManager::from_ids(focus_ids),
            workspace: workspace.clone(),
            config_path: config_path.map(|p| p.to_path_buf()),
            next_panel_id,
            action_cb: None,
            registry,
            on_type_chosen: None,
            dirty: false,
            zoomed_panel: None,
            sync_panels: std::collections::HashSet::new(),
            #[cfg(feature = "vte")]
            sync_commit_cb: None,
        };

        // Focus first panel
        view.focus.focus_first(&view.hosts);

        // Record in recent workspaces DB
        view.record_in_db();

        view
    }

    /// Load a workspace struct directly (for New workspace).
    pub fn load_workspace(&mut self, ws: Workspace, config_path: Option<&Path>) -> Result<(), String> {
        self.config_path = config_path.map(|p| p.to_path_buf());
        self.rebuild_from_workspace(ws)
    }

    /// Reload from a workspace file, rebuilding the entire view.
    pub fn load_from_file(&mut self, path: &Path) -> Result<(), String> {
        tracing::info!("Loading workspace from {}", path.display());
        let ws = tp_core::config::load_workspace(path)
            .map_err(|e| format!("Failed to load: {}", e))?;
        tracing::info!("Loaded workspace '{}' with {} panels", ws.name, ws.panels.len());
        self.config_path = Some(path.to_path_buf());
        self.rebuild_from_workspace(ws)
    }

    fn rebuild_from_workspace(&mut self, ws: Workspace) -> Result<(), String> {
        // Remove old root widget
        self.root_box.remove(&self.root_widget);

        let registry = registry::build_default_registry();
        let ws_dir = self.config_path.as_ref().and_then(|p| p.parent()).map(|p| p.to_string_lossy().to_string());
        let mut hosts = HashMap::new();

        for panel_cfg in &ws.panels {
            let host = PanelHost::new(&panel_cfg.id, &panel_cfg.name, self.action_cb.clone());
            if panel_cfg.effective_type() == PanelType::Empty {
                let chooser = ChooserPanel::new(&panel_cfg.id, &registry, self.on_type_chosen.clone());
                host.set_backend(Box::new(chooser));
            } else {
                let backend = create_backend_from_registry(panel_cfg, &ws.settings.default_shell, &registry, ws_dir.as_deref());
                host.set_backend(backend);
            }
            apply_min_size(&host, panel_cfg);
            hosts.insert(panel_cfg.id.clone(), host);
        }

        let root_widget = build_layout_widget(&ws.layout, &hosts, &ws.panels);
        root_widget.set_vexpand(true);
        root_widget.set_hexpand(true);
        self.root_box.append(&root_widget);

        if let Some(ref cb) = self.action_cb {
            add_plus_buttons_recursive(&root_widget, cb);
        }

        self.root_widget = root_widget;
        self.hosts = hosts;
        self.workspace = ws;
        self.registry = registry;
        self.dirty = false;

        self.next_panel_id = self.workspace.panels.iter()
            .filter_map(|p| p.id.strip_prefix('p').and_then(|n| n.parse::<usize>().ok()))
            .max()
            .unwrap_or(0) + 1;

        self.rebuild_focus_order();
        self.dirty = false;

        self.focus.focus_first(&self.hosts);

        self.record_in_db();
        Ok(())
    }

    /// Get the current panel type for a panel.
    pub fn panel_type(&self, panel_id: &str) -> Option<PanelType> {
        self.workspace.panels.iter()
            .find(|p| p.id == panel_id)
            .map(|p| p.effective_type())
    }

    /// Get the panel name.
    pub fn panel_name(&self, panel_id: &str) -> Option<String> {
        self.workspace.panels.iter()
            .find(|p| p.id == panel_id)
            .map(|p| p.name.clone())
    }

    /// Get min_width for a panel.
    pub fn panel_min_width(&self, panel_id: &str) -> u32 {
        self.workspace.panels.iter()
            .find(|p| p.id == panel_id)
            .map(|p| p.min_width)
            .unwrap_or(0)
    }

    /// Get min_height for a panel.
    pub fn panel_min_height(&self, panel_id: &str) -> u32 {
        self.workspace.panels.iter()
            .find(|p| p.id == panel_id)
            .map(|p| p.min_height)
            .unwrap_or(0)
    }

    /// Update panel config after Configure dialog.
    /// Recreates the backend with the new type/settings and runs startup commands.
    pub fn apply_panel_config(&mut self, panel_id: &str, new_name: String, new_type: PanelType, cwd: Option<String>, startup_commands: Vec<String>, before_close: Option<String>, min_width: u32, min_height: u32) {
        tracing::info!("Configuring panel {}: name={}, type={:?}, cwd={:?}, cmds={}, before_close={}",
            panel_id, new_name, new_type, cwd, startup_commands.len(), before_close.is_some());
        // Update model
        if let Some(panel_cfg) = self.workspace.panels.iter_mut().find(|p| p.id == panel_id) {
            panel_cfg.name = new_name.clone();
            panel_cfg.panel_type = new_type.clone();
            panel_cfg.cwd = cwd.clone();
            panel_cfg.startup_commands = startup_commands.clone();
            panel_cfg.before_close = before_close;
            panel_cfg.min_width = min_width;
            panel_cfg.min_height = min_height;
        }

        // Update title + tab label
        crate::layout_ops::update_tab_label_in_layout(&mut self.workspace.layout, panel_id, &new_name);
        if let Some(host) = self.hosts.get(panel_id) {
            host.set_title(&new_name);
        }

        // Recreate backend with startup commands queued
        let ws_dir = self.config_path.as_ref().and_then(|p| p.parent()).map(|p| p.to_string_lossy().to_string());
        let mut config = panel_type_to_create_config(&new_type, &self.workspace.settings.default_shell, ws_dir.as_deref());
        // Pass startup commands via extra so the registry factory can queue them
        if !startup_commands.is_empty() {
            config.extra.insert("__startup_commands__".to_string(), startup_commands.join("\n"));
        }
        if let Some(backend) = self.registry.create(panel_type_to_id(&new_type), &config) {
            if let Some(host) = self.hosts.get(panel_id) {
                host.set_backend(backend);
            }
        }

        // Apply min size to widget
        if let Some(host) = self.hosts.get(panel_id) {
            let w = if min_width > 0 { min_width as i32 } else { -1 };
            let h = if min_height > 0 { min_height as i32 } else { -1 };
            host.widget().set_size_request(w, h);
        }

        self.dirty = true;

        // Rebuild layout so tab labels reflect the new name
        self.rebuild_layout();
    }

    /// Get cwd for a panel.
    pub fn panel_cwd(&self, panel_id: &str) -> Option<String> {
        self.workspace.panels.iter()
            .find(|p| p.id == panel_id)
            .and_then(|p| p.cwd.clone())
    }

    /// Get startup commands for a panel.
    pub fn panel_startup_commands(&self, panel_id: &str) -> Vec<String> {
        self.workspace.panels.iter()
            .find(|p| p.id == panel_id)
            .map(|p| p.startup_commands.clone())
            .unwrap_or_default()
    }

    /// Get before_close script for a panel.
    pub fn panel_before_close(&self, panel_id: &str) -> Option<String> {
        self.workspace.panels.iter()
            .find(|p| p.id == panel_id)
            .and_then(|p| p.before_close.clone())
    }

    /// Execute before_close script for a panel.
    fn run_before_close(&self, panel_id: &str) {
        if let Some(script) = self.panel_before_close(panel_id) {
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

    pub fn workspace_name(&self) -> &str {
        &self.workspace.name
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn has_config_path(&self) -> bool {
        self.config_path.is_some()
    }

    pub fn config_path_str(&self) -> Option<String> {
        self.config_path.as_ref().map(|p| p.to_string_lossy().to_string())
    }

    /// Set callback for when a panel type is chosen from the chooser.
    /// Rebuilds chooser panels so they get the callback.
    pub fn set_type_chosen_callback(&mut self, cb: OnTypeChosen) {
        self.on_type_chosen = Some(cb.clone());

        // Rebuild any existing chooser panels so they get the callback
        let chooser_ids: Vec<String> = self.workspace.panels.iter()
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

    /// Change a panel's type. Swaps the backend in the existing PanelHost.
    pub fn set_panel_type(&mut self, panel_id: &str, type_id: &str) {
        tracing::info!("Setting panel {} type to {}", panel_id, type_id);
        let config = PanelCreateConfig {
            shell: self.workspace.settings.default_shell.clone(),
            cwd: None,
            env: vec![],
            extra: HashMap::new(),
        };

        if let Some(backend) = self.registry.create(type_id, &config) {
            if let Some(host) = self.hosts.get(panel_id) {
                host.set_backend(backend);
            }
        }

        // Update the model so it saves correctly
        self.dirty = true;
        if let Some(panel_cfg) = self.workspace.panels.iter_mut().find(|p| p.id == panel_id) {
            panel_cfg.panel_type = match type_id {
                "terminal" => PanelType::Terminal,
                "markdown" => PanelType::Markdown { file: "README.md".to_string() },
                "browser" => PanelType::Browser { url: "about:blank".to_string() },
                "ssh" => PanelType::Ssh {
                    host: "localhost".to_string(),
                    port: 22,
                    user: None,
                    identity_file: None,
                },
                "remote_tmux" => PanelType::RemoteTmux {
                    host: "localhost".to_string(),
                    session: "main".to_string(),
                    user: None,
                },
                _ => PanelType::Terminal,
            };
            panel_cfg.name = format!("{}", type_id);
            if let Some(host) = self.hosts.get(panel_id) {
                host.set_title(type_id);
            }
        }
    }

    /// Get a reference to the panel registry.
    pub fn registry(&self) -> &PanelRegistry {
        &self.registry
    }

    /// Set the action callback for panel menus. Must be called after wrapping in Rc<RefCell<>>.
    /// Rebuilds the layout so tab labels get close buttons with the callback.
    pub fn set_action_callback(&mut self, cb: PanelActionCallback) {
        self.action_cb = Some(cb);
        // Rebuild layout to propagate callback to all hosts, tab labels, and + buttons
        self.rebuild_layout();
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
        tracing::debug!("rebuild_layout: {} hosts, action_cb={}, type_chosen={}",
            self.hosts.len(), self.action_cb.is_some(), self.on_type_chosen.is_some());
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
        let root_widget = build_layout_widget_inner(
            &self.workspace.layout, &self.hosts, &self.workspace.panels, &self.action_cb,
        );
        root_widget.set_vexpand(true);
        root_widget.set_hexpand(true);
        self.root_box.prepend(&root_widget);
        self.root_widget = root_widget;

        // Reconnect all callbacks on all hosts + notebooks
        if let Some(ref cb) = self.action_cb {
            for host in self.hosts.values() {
                host.set_action_callback(cb.clone());
            }
            add_plus_buttons_recursive(&self.root_widget, cb);
        }

        // Reconnect VTE sync commit callbacks
        #[cfg(feature = "vte")]
        if let Some(ref cb) = self.sync_commit_cb {
            let propagating = std::rc::Rc::new(std::cell::Cell::new(false));
            for host in self.hosts.values() {
                host.set_sync_commit_callback(cb.clone(), propagating.clone());
            }
        }

        // Reconnect type chooser callbacks on chooser panels
        if let Some(ref tc) = self.on_type_chosen {
            let chooser_ids: Vec<String> = self.workspace.panels.iter()
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
            self.sync_panels.insert(focused_id.clone());
            if let Some(host) = self.hosts.get(&focused_id) {
                host.set_alert_border("yellow");
                host.set_sync_active(true);
            }
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

    /// Connect VTE commit handlers on all panel hosts for sync propagation.
    /// The callback is called with (source_panel_id, text) whenever a synced
    /// terminal receives input — the caller should forward to other synced panels.
    #[cfg(feature = "vte")]
    pub fn setup_sync_callbacks(&mut self, cb: std::rc::Rc<dyn Fn(&str, &str)>) {
        self.sync_commit_cb = Some(cb.clone());
        let propagating = std::rc::Rc::new(std::cell::Cell::new(false));
        for host in self.hosts.values() {
            host.set_sync_commit_callback(cb.clone(), propagating.clone());
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
        };
        let host = PanelHost::new(&new_id, &new_name, self.action_cb.clone());
        let backend = self.create_chooser_backend(&new_id);
        host.set_backend(backend);

        // 2. Update model
        self.update_layout_split(&focused_id, &new_id, orientation);
        self.workspace.panels.push(new_cfg);
        self.hosts.insert(new_id.clone(), host);

        // 3. Rebuild widget tree from model
        self.rebuild_layout();
        self.rebuild_focus_order();

        Some(new_id)
    }

    /// Wrap the focused panel in a new TabSplit (Notebook) with a second tab.
    pub fn add_tab_focused(&mut self) -> Option<String> {
        let focused_id = self.focused_panel_id()?.to_string();
        let new_id = self.alloc_panel_id();
        let new_name = format!("New Panel {}", &new_id[1..]);

        let new_cfg = self.make_empty_config(&new_id, &new_name);
        let host = PanelHost::new(&new_id, &new_name, self.action_cb.clone());
        let backend = self.create_chooser_backend(&new_id);
        host.set_backend(backend);

        // Update model: wrap focused panel in Tabs node
        let existing_label = self.workspace
            .panel(&focused_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| focused_id.clone());
        self.workspace.layout = replace_in_layout(
            &self.workspace.layout,
            &focused_id,
            &|_| LayoutNode::Tabs {
                children: vec![
                    LayoutNode::Panel { id: focused_id.clone() },
                    LayoutNode::Panel { id: new_id.clone() },
                ],
                labels: vec![existing_label.clone(), new_name.clone()],
            },
        );
        self.workspace.panels.push(new_cfg);
        self.hosts.insert(new_id.clone(), host);

        // Rebuild widget tree
        self.rebuild_layout();
        self.rebuild_focus_order();

        Some(new_id)
    }

    /// Add a new tab to an existing Notebook. Called by the "+" button.
    pub fn add_tab_to_notebook(&mut self, notebook: &gtk4::Notebook) -> Option<String> {
        // Find which panel is currently active in this notebook to locate in model
        let current_page = notebook.current_page()?;
        let current_widget = notebook.nth_page(Some(current_page))?;
        let sibling_id = self.find_panel_id_in_widget(&current_widget)?;

        let new_id = self.alloc_panel_id();
        let new_name = format!("New Panel {}", &new_id[1..]);

        let new_cfg = self.make_empty_config(&new_id, &new_name);
        let host = PanelHost::new(&new_id, &new_name, self.action_cb.clone());
        let backend = self.create_chooser_backend(&new_id);
        host.set_backend(backend);

        // Update model
        add_to_existing_tabs(&mut self.workspace.layout, &sibling_id, &new_id, &new_name);
        self.workspace.panels.push(new_cfg);
        self.hosts.insert(new_id.clone(), host);

        // Rebuild widget tree
        self.rebuild_layout();
        self.rebuild_focus_order();

        Some(new_id)
    }

    fn find_panel_id_in_widget(&self, widget: &gtk4::Widget) -> Option<String> {
        for (id, host) in &self.hosts {
            if host.widget() == widget {
                return Some(id.clone());
            }
        }
        // Try inside the widget (might be a container)
        None
    }

    fn make_empty_config(&self, id: &str, name: &str) -> PanelConfig {
        PanelConfig {
            id: id.to_string(),
            name: name.to_string(),
            panel_type: PanelType::Empty, // Chooser — user picks the type
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
        }
    }

    fn create_chooser_backend(&self, panel_id: &str) -> Box<dyn crate::panels::PanelBackend> {
        Box::new(ChooserPanel::new(panel_id, &self.registry, self.on_type_chosen.clone()))
    }

    /// Close the focused panel. Uses model update + layout rebuild for reliability.
    pub fn close_focused(&mut self) -> bool {
        if self.focus.order.len() <= 1 {
            return false; // Don't close the last panel
        }

        let focused_id = match self.focused_panel_id() {
            Some(id) => id.to_string(),
            None => return false,
        };

        // Run before_close script
        self.run_before_close(&focused_id);

        // 1. Update model: remove panel from layout tree and panels list
        self.update_layout_remove(&focused_id);
        self.workspace.panels.retain(|p| p.id != focused_id);

        // 2. Detach the closing panel's widget and drop the host
        if let Some(host) = self.hosts.remove(&focused_id) {
            detach_widget(host.widget());
        }

        // 3. Rebuild the widget tree from the updated model
        self.rebuild_layout();
        self.rebuild_focus_order();

        // 4. Focus next available
        if self.focus.index >= self.focus.order.len() {
            self.focus.index = 0;
        }
        self.focus.focus_current_pub(&self.hosts);

        true
    }

    // ── Save ─────────────────────────────────────────────────────────────

    /// Sync ratios from GTK widget positions back into the layout model.
    fn sync_ratios_from_widgets(&mut self) {
        sync_ratios_recursive(&self.root_widget, &mut self.workspace.layout);
    }

    /// Save the current workspace to the original config file.
    pub fn save(&mut self) -> Result<PathBuf, String> {
        self.sync_ratios_from_widgets();
        let path = self
            .config_path
            .as_ref()
            .ok_or("No config path set")?
            .clone();
        tp_core::config::save_workspace(&self.workspace, &path)
            .map_err(|e| format!("Save failed: {}", e))?;
        tracing::info!("Saved {} panels to {}", self.workspace.panels.len(), path.display());
        for p in &self.workspace.panels {
            if !p.startup_commands.is_empty() {
                tracing::debug!("  {} startup: {:?}", p.id, &p.startup_commands[0][..p.startup_commands[0].len().min(80)]);
            }
            if let Some(ref bc) = p.before_close {
                tracing::debug!("  {} before_close: {:?}", p.id, &bc[..bc.len().min(80)]);
            }
        }
        self.dirty = false;
        self.record_in_db();
        Ok(path)
    }

    /// Save to a specific path.
    pub fn save_as(&mut self, path: &Path) -> Result<(), String> {
        self.sync_ratios_from_widgets();
        tp_core::config::save_workspace(&self.workspace, path)
            .map_err(|e| format!("Save failed: {}", e))?;
        self.config_path = Some(path.to_path_buf());
        self.dirty = false;
        self.record_in_db();
        Ok(())
    }

    fn record_in_db(&self) {
        let db_path = tp_db::Database::default_path();
        if let Ok(db) = tp_db::Database::open(&db_path) {
            let config_str = self.config_path.as_ref().map(|p| p.to_string_lossy().to_string());
            db.record_workspace_open(&self.workspace.name, config_str.as_deref()).ok();
        }
    }

    // ── Layout model updates ─────────────────────────────────────────────

    fn rebuild_focus_order(&mut self) {
        self.dirty = true;
        let ids: Vec<String> = self.workspace.layout.panel_ids()
            .iter().map(|s| s.to_string()).collect();
        self.focus.rebuild(ids);
    }

    fn update_layout_split(
        &mut self,
        existing_id: &str,
        new_id: &str,
        orientation: gtk4::Orientation,
    ) {
        self.workspace.layout = replace_in_layout(
            &self.workspace.layout,
            existing_id,
            &|_| {
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
            },
        );
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

// ── Widget helpers ───────────────────────────────────────────────────────────

/// Walk a widget tree and add ⋮ menus to any GtkNotebook found.
fn add_plus_buttons_recursive(widget: &gtk4::Widget, action_cb: &PanelActionCallback) {
    if let Ok(notebook) = widget.clone().downcast::<gtk4::Notebook>() {
        setup_notebook_menu_widget(&notebook, Some(action_cb.clone()));
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        add_plus_buttons_recursive(&c, action_cb);
        child = c.next_sibling();
    }
}

/// Build a tab label widget: "name [x]" — the X button closes the tab.
fn build_tab_label(name: &str, action_cb: &Option<PanelActionCallback>, child_widget: &gtk4::Widget) -> gtk4::Widget {
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

    // Stack: label (view) / entry (edit) — double-click to rename
    let stack = gtk4::Stack::new();
    let label = gtk4::Label::new(Some(name));
    stack.add_named(&label, Some("label"));
    let entry = gtk4::Entry::new();
    entry.set_text(name);
    entry.set_width_chars(12);
    stack.add_named(&entry, Some("entry"));
    stack.set_visible_child_name("label");

    // Double-click on label → edit
    {
        let s = stack.clone();
        let e = entry.clone();
        let l = label.clone();
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(1);
        gesture.connect_released(move |g, n_press, _, _| {
            if n_press == 2 {
                e.set_text(&l.text());
                s.set_visible_child_name("entry");
                e.grab_focus();
                g.set_state(gtk4::EventSequenceState::Claimed);
            }
        });
        stack.add_controller(gesture);
    }

    // Enter → confirm rename
    {
        let s = stack.clone();
        let l = label.clone();
        let cb = action_cb.clone();
        let w = child_widget.clone();
        entry.connect_activate(move |entry| {
            let new_name = entry.text().to_string();
            if !new_name.trim().is_empty() {
                l.set_text(&new_name);
                if let Some(ref cb) = cb {
                    find_panel_id_recursive(&w, &|panel_id| {
                        cb(panel_id, PanelAction::Rename(new_name.clone()));
                    });
                }
            }
            s.set_visible_child_name("label");
        });
    }

    // Escape → cancel
    {
        let s = stack.clone();
        let key_ctrl = gtk4::EventControllerKey::new();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                s.set_visible_child_name("label");
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
        entry.add_controller(key_ctrl);
    }

    hbox.append(&stack);

    // Close button
    let close_btn = gtk4::Button::new();
    close_btn.set_icon_name("window-close-symbolic");
    close_btn.add_css_class("flat");
    close_btn.add_css_class("circular");
    close_btn.add_css_class("tab-close-btn");
    close_btn.set_tooltip_text(Some("Close tab"));

    let cb = action_cb.clone();
    let widget = child_widget.clone();
    close_btn.connect_clicked(move |_| {
        if let Some(ref cb) = cb {
            find_panel_id_recursive(&widget, &|panel_id| {
                cb(&format!("nb:{}", panel_id), PanelAction::RemoveTab);
            });
        }
    });

    hbox.append(&close_btn);
    hbox.upcast::<gtk4::Widget>()
}

/// Add a "+" button to a Notebook's tab bar to add new tabs.
fn setup_notebook_menu_widget(notebook: &gtk4::Notebook, action_cb: Option<PanelActionCallback>) {
    let btn = gtk4::Button::new();
    btn.set_icon_name("tab-new-symbolic");
    btn.add_css_class("flat");
    btn.set_margin_end(14);
    btn.set_tooltip_text(Some("Add tab"));

    let nb = notebook.clone();
    let cb = action_cb;
    btn.connect_clicked(move |_| {
        if let Some(ref cb) = cb {
            if let Some(page) = nb.nth_page(nb.current_page()) {
                find_panel_id_recursive(&page, &|panel_id| {
                    cb(&format!("nb:{}", panel_id), PanelAction::AddTabToNotebook);
                });
            }
        }
    });

    notebook.set_action_widget(&btn, gtk4::PackType::End);
}

/// Find the first PanelHost panel_id inside a widget tree.
/// PanelHost frames have widget_name set to panel_id.
fn find_panel_id_recursive(widget: &gtk4::Widget, callback: &dyn Fn(&str)) {
    if widget.has_css_class("panel-frame") {
        let name = widget.widget_name();
        let name_str = name.as_str();
        if !name_str.is_empty() {
            callback(name_str);
            return;
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        find_panel_id_recursive(&c, callback);
        child = c.next_sibling();
    }
}

/// Walk up the widget tree to find a GtkNotebook ancestor (within 3 levels).
pub fn find_notebook_ancestor(widget: &gtk4::Widget) -> Option<gtk4::Notebook> {
    let mut current = widget.parent();
    for _ in 0..3 {
        let w = current?;
        if let Ok(nb) = w.clone().downcast::<gtk4::Notebook>() {
            return Some(nb);
        }
        current = w.parent();
    }
    None
}

/// Detach a widget from its current parent (Paned, Notebook, Box, etc.).
fn detach_widget(widget: &gtk4::Widget) {
    if let Some(parent) = widget.parent() {
        if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
            let is_start = paned.start_child().map(|w| w == *widget).unwrap_or(false);
            if is_start {
                paned.set_start_child(gtk4::Widget::NONE);
            } else {
                paned.set_end_child(gtk4::Widget::NONE);
            }
        } else if let Some(notebook) = parent.downcast_ref::<gtk4::Notebook>() {
            let page = notebook.page_num(widget);
            notebook.remove_page(page);
        } else if let Some(bx) = parent.downcast_ref::<gtk4::Box>() {
            bx.remove(widget);
        } else if let Some(notebook) = find_notebook_ancestor(widget) {
            // GTK4 notebook wraps children in internal containers
            let page = notebook.page_num(widget);
            notebook.remove_page(page);
        } else {
            // Generic: try widget.unparent() as last resort
            widget.unparent();
        }
    }
}

// ── Backend creation ─────────────────────────────────────────────────────────

fn panel_type_to_id(pt: &PanelType) -> &str {
    match pt {
        PanelType::Empty => "__empty__",
        PanelType::Terminal => "terminal",
        PanelType::Ssh { .. } => "ssh",
        PanelType::RemoteTmux { .. } => "remote_tmux",
        PanelType::Markdown { .. } => "markdown",
        PanelType::Browser { .. } => "browser",
    }
}

fn panel_type_to_create_config(pt: &PanelType, default_shell: &str, workspace_dir: Option<&str>) -> PanelCreateConfig {
    let mut extra = HashMap::new();
    match pt {
        PanelType::Ssh { host, user, .. } => {
            extra.insert("host".to_string(), host.clone());
            if let Some(u) = user { extra.insert("user".to_string(), u.clone()); }
        }
        PanelType::RemoteTmux { host, session, user } => {
            extra.insert("host".to_string(), host.clone());
            extra.insert("session".to_string(), session.clone());
            if let Some(u) = user { extra.insert("user".to_string(), u.clone()); }
        }
        PanelType::Markdown { file } => {
            extra.insert("file".to_string(), file.clone());
        }
        PanelType::Browser { url } => {
            extra.insert("url".to_string(), url.clone());
        }
        _ => {}
    }
    if let Some(dir) = workspace_dir {
        extra.insert("__workspace_dir__".to_string(), dir.to_string());
    }
    PanelCreateConfig {
        shell: default_shell.to_string(),
        cwd: None,
        env: vec![],
        extra,
    }
}

/// Create a PanelBackend from a PanelConfig using the registry.
fn create_backend_from_registry(
    panel_cfg: &PanelConfig,
    default_shell: &str,
    registry: &PanelRegistry,
    workspace_dir: Option<&str>,
) -> Box<dyn crate::panels::PanelBackend> {
    let effective = panel_cfg.effective_type();
    let (type_id, mut extra) = match &effective {
        PanelType::Empty => ("__empty__", HashMap::new()),
        PanelType::Terminal => ("terminal", HashMap::new()),
        PanelType::Ssh { host, user, .. } => {
            let mut extra = HashMap::new();
            extra.insert("host".to_string(), host.clone());
            if let Some(u) = user {
                extra.insert("user".to_string(), u.clone());
            }
            ("ssh", extra)
        }
        PanelType::RemoteTmux { host, session, user } => {
            let mut extra = HashMap::new();
            extra.insert("host".to_string(), host.clone());
            extra.insert("session".to_string(), session.clone());
            if let Some(u) = user {
                extra.insert("user".to_string(), u.clone());
            }
            ("remote_tmux", extra)
        }
        PanelType::Markdown { file } => {
            let mut extra = HashMap::new();
            extra.insert("file".to_string(), file.clone());
            ("markdown", extra)
        }
        PanelType::Browser { url } => {
            let mut extra = HashMap::new();
            extra.insert("url".to_string(), url.clone());
            ("browser", extra)
        }
    };

    if !panel_cfg.startup_commands.is_empty() {
        extra.insert("__startup_commands__".to_string(), panel_cfg.startup_commands.join("\n"));
    }
    if let Some(dir) = workspace_dir {
        extra.insert("__workspace_dir__".to_string(), dir.to_string());
    }

    let config = PanelCreateConfig {
        shell: default_shell.to_string(),
        cwd: panel_cfg.cwd.clone(),
        env: panel_cfg.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        extra,
    };

    let backend = registry.create(type_id, &config)
        .unwrap_or_else(|| Box::new(MarkdownPanel::new("/dev/null")));

    backend
}

// ── Layout widget building ───────────────────────────────────────────────────

/// Recursively build GTK widgets from a LayoutNode tree.
fn build_layout_widget(
    node: &LayoutNode,
    hosts: &HashMap<String, PanelHost>,
    panels: &[PanelConfig],
) -> gtk4::Widget {
    build_layout_widget_inner(node, hosts, panels, &None)
}

fn build_layout_widget_inner(
    node: &LayoutNode,
    hosts: &HashMap<String, PanelHost>,
    panels: &[PanelConfig],
    action_cb: &Option<PanelActionCallback>,
) -> gtk4::Widget {
    match node {
        LayoutNode::Panel { id } => {
            if let Some(host) = hosts.get(id) {
                host.widget().clone()
            } else {
                let label = gtk4::Label::new(Some(&format!("Missing panel: {}", id)));
                label.upcast::<gtk4::Widget>()
            }
        }
        LayoutNode::Hsplit { children, ratios } => {
            build_paned(children, ratios, hosts, panels, action_cb, gtk4::Orientation::Horizontal)
        }
        LayoutNode::Vsplit { children, ratios } => {
            build_paned(children, ratios, hosts, panels, action_cb, gtk4::Orientation::Vertical)
        }
        LayoutNode::Tabs { children, labels } => {
            let notebook = gtk4::Notebook::new();
            notebook.set_show_tabs(true);
            notebook.set_scrollable(true);

            for (i, child) in children.iter().enumerate() {
                let child_widget = build_layout_widget_inner(child, hosts, panels, action_cb);
                let label_text = labels
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("Tab {}", i + 1));
                let label = build_tab_label(&label_text, action_cb, &child_widget);
                notebook.append_page(&child_widget, Some(&label));
            }

            notebook.upcast::<gtk4::Widget>()
        }
    }
}

/// Set paned position based on ratio, deferred until widget has real size.
fn setup_paned_ratio(paned: &gtk4::Paned, ratio: f64, orientation: gtk4::Orientation) {
    use gtk4::glib;

    // Set an initial guess
    paned.set_position((ratio * 800.0) as i32);

    // After the widget is mapped and has real allocation, set correct position
    let r = ratio;
    let p = paned.clone();
    glib::idle_add_local_once(move || {
        let alloc = p.allocation();
        let total = match orientation {
            gtk4::Orientation::Horizontal => alloc.width(),
            _ => alloc.height(),
        };
        if total > 0 {
            p.set_position((r * total as f64) as i32);
        }
    });
}

fn build_paned(
    children: &[LayoutNode],
    ratios: &[f64],
    hosts: &HashMap<String, PanelHost>,
    panels: &[PanelConfig],
    action_cb: &Option<PanelActionCallback>,
    orientation: gtk4::Orientation,
) -> gtk4::Widget {
    if children.is_empty() {
        return gtk4::Box::new(orientation, 0).upcast::<gtk4::Widget>();
    }
    if children.len() == 1 {
        return build_layout_widget_inner(&children[0], hosts, panels, action_cb);
    }

    let sum: f64 = ratios.iter().take(children.len()).sum();
    let normalized: Vec<f64> = if sum > 0.0 {
        ratios
            .iter()
            .take(children.len())
            .map(|r| r / sum)
            .collect()
    } else {
        vec![1.0 / children.len() as f64; children.len()]
    };

    if children.len() == 2 {
        let paned = gtk4::Paned::new(orientation);
        let w1 = build_layout_widget_inner(&children[0], hosts, panels, action_cb);
        let w2 = build_layout_widget_inner(&children[1], hosts, panels, action_cb);
        let c1_fixed = subtree_has_min_size(&children[0], panels);
        let c2_fixed = subtree_has_min_size(&children[1], panels);
        paned.set_start_child(Some(&w1));
        paned.set_end_child(Some(&w2));
        paned.set_shrink_start_child(!c1_fixed);
        paned.set_shrink_end_child(!c2_fixed);
        paned.set_resize_start_child(!c1_fixed || !c2_fixed);
        paned.set_resize_end_child(!c2_fixed || !c1_fixed);

        setup_paned_ratio(&paned, normalized[0], orientation);
        return paned.upcast::<gtk4::Widget>();
    }

    let paned = gtk4::Paned::new(orientation);
    let w1 = build_layout_widget_inner(&children[0], hosts, panels, action_cb);
    let rest_nodes = &children[1..];
    let rest = build_paned(rest_nodes, &ratios[1..], hosts, panels, action_cb, orientation);
    let c1_fixed = subtree_has_min_size(&children[0], panels);
    let rest_fixed = rest_nodes.iter().any(|n| subtree_has_min_size(n, panels));
    paned.set_start_child(Some(&w1));
    paned.set_end_child(Some(&rest));
    paned.set_shrink_start_child(!c1_fixed);
    paned.set_shrink_end_child(!rest_fixed);
    paned.set_resize_start_child(true);
    paned.set_resize_end_child(true);

    setup_paned_ratio(&paned, normalized[0], orientation);
    paned.upcast::<gtk4::Widget>()
}

/// Apply min_width/min_height from PanelConfig to the PanelHost widget.
fn apply_min_size(host: &PanelHost, cfg: &PanelConfig) {
    let w = if cfg.min_width > 0 { cfg.min_width as i32 } else { -1 };
    let h = if cfg.min_height > 0 { cfg.min_height as i32 } else { -1 };
    if w > 0 || h > 0 {
        host.widget().set_size_request(w, h);
    }
}

/// Check if any panel in a layout subtree has a min size set.
fn subtree_has_min_size(node: &LayoutNode, panels: &[PanelConfig]) -> bool {
    match node {
        LayoutNode::Panel { id } => {
            panels.iter().any(|p| p.id == *id && (p.min_width > 0 || p.min_height > 0))
        }
        LayoutNode::Hsplit { children, .. }
        | LayoutNode::Vsplit { children, .. }
        | LayoutNode::Tabs { children, .. } => {
            children.iter().any(|c| subtree_has_min_size(c, panels))
        }
    }
}

/// Recursively sync GTK Paned positions back into LayoutNode ratios.
fn sync_ratios_recursive(widget: &gtk4::Widget, node: &mut LayoutNode) {
    let is_hsplit = matches!(node, LayoutNode::Hsplit { .. });
    match node {
        LayoutNode::Panel { .. } => {}
        LayoutNode::Hsplit { children, ratios } | LayoutNode::Vsplit { children, ratios } => {
            if children.len() < 2 {
                return;
            }
            // The widget should be a Paned
            if let Ok(paned) = widget.clone().downcast::<gtk4::Paned>() {
                let alloc = paned.allocation();
                let total = if paned.orientation() == gtk4::Orientation::Horizontal {
                    alloc.width()
                } else {
                    alloc.height()
                };
                if total > 0 {
                    let pos = paned.position();
                    let r1 = pos as f64 / total as f64;
                    let r2 = 1.0 - r1;

                    if children.len() == 2 {
                        // Simple 2-child split
                        if ratios.len() >= 2 {
                            ratios[0] = r1;
                            ratios[1] = r2;
                        }
                        // Recurse into children
                        if let Some(w1) = paned.start_child() {
                            sync_ratios_recursive(&w1, &mut children[0]);
                        }
                        if let Some(w2) = paned.end_child() {
                            sync_ratios_recursive(&w2, &mut children[1]);
                        }
                    } else {
                        // N>2: first child is start, rest are nested in end
                        if !ratios.is_empty() {
                            ratios[0] = r1;
                        }
                        if let Some(w1) = paned.start_child() {
                            sync_ratios_recursive(&w1, &mut children[0]);
                        }
                        if let Some(w2) = paned.end_child() {
                            let rest_children = children[1..].to_vec();
                            let rest_ratios = if ratios.len() > 1 { ratios[1..].to_vec() } else { vec![1.0; rest_children.len()] };
                            let mut rest_node = if is_hsplit {
                                LayoutNode::Hsplit { children: rest_children, ratios: rest_ratios }
                            } else {
                                LayoutNode::Vsplit { children: rest_children, ratios: rest_ratios }
                            };
                            sync_ratios_recursive(&w2, &mut rest_node);
                            // Copy back
                            match rest_node {
                                LayoutNode::Hsplit { children: rc, ratios: rr }
                                | LayoutNode::Vsplit { children: rc, ratios: rr } => {
                                    for (i, c) in rc.into_iter().enumerate() {
                                        children[i + 1] = c;
                                    }
                                    for (i, r) in rr.into_iter().enumerate() {
                                        if i + 1 < ratios.len() {
                                            ratios[i + 1] = r;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        LayoutNode::Tabs { children, .. } => {
            // For notebooks, recurse into each page
            if let Ok(notebook) = widget.clone().downcast::<gtk4::Notebook>() {
                for (i, child) in children.iter_mut().enumerate() {
                    if let Some(page_widget) = notebook.nth_page(Some(i as u32)) {
                        sync_ratios_recursive(&page_widget, child);
                    }
                }
            }
        }
    }
}
