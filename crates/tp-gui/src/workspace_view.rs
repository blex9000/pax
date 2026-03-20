use gtk4::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use tp_core::workspace::{LayoutNode, PanelConfig, PanelType, Workspace};

use crate::panel_host::{PanelAction, PanelActionCallback, PanelHost};
use crate::panels::chooser::{ChooserPanel, OnTypeChosen};
use crate::panels::registry::{self, PanelCreateConfig, PanelRegistry};
use crate::panels::markdown::MarkdownPanel;
use crate::panels::terminal::TerminalPanel;

/// Builds the GTK widget tree from a workspace layout.
pub struct WorkspaceView {
    root_widget: gtk4::Widget,
    root_box: gtk4::Box,
    scrolled: gtk4::ScrolledWindow,
    hosts: HashMap<String, PanelHost>,
    focus_order: Vec<String>,
    focus_index: usize,
    workspace: Workspace,
    config_path: Option<PathBuf>,
    next_panel_id: usize,
    action_cb: Option<PanelActionCallback>,
    registry: PanelRegistry,
    on_type_chosen: Option<OnTypeChosen>,
    dirty: bool,
}

impl WorkspaceView {
    /// Build the workspace view from a workspace config.
    /// Call `set_action_callback` after wrapping in Rc<RefCell<>> to enable menu actions.
    pub fn build(workspace: &Workspace, config_path: Option<&Path>) -> Self {
        let registry = registry::build_default_registry();
        let mut hosts = HashMap::new();

        // Create panel hosts — Empty panels get chooser, others get their type
        for panel_cfg in &workspace.panels {
            let host = PanelHost::new(&panel_cfg.id, &panel_cfg.name, None);
            if panel_cfg.effective_type() == PanelType::Empty {
                // Chooser will be set later when on_type_chosen callback is wired
                let chooser = ChooserPanel::new(&panel_cfg.id, &registry, None);
                host.set_backend(Box::new(chooser));
            } else {
                let backend = create_backend_from_registry(panel_cfg, &workspace.settings.default_shell, &registry);
                host.set_backend(backend);
            }
            hosts.insert(panel_cfg.id.clone(), host);
        }

        // Build layout widget tree
        let root_widget = build_layout_widget(&workspace.layout, &hosts);
        root_widget.set_vexpand(true);
        root_widget.set_hexpand(true);

        // Wrap in Box (for reparenting) inside ScrolledWindow (for overflow)
        let root_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root_box.append(&root_widget);

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_child(Some(&root_box));
        scrolled.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic);

        let focus_order: Vec<String> = workspace
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

        let view = Self {
            root_widget,
            root_box,
            scrolled,
            hosts,
            focus_order,
            focus_index: 0,
            workspace: workspace.clone(),
            config_path: config_path.map(|p| p.to_path_buf()),
            next_panel_id,
            action_cb: None,
            registry,
            on_type_chosen: None,
            dirty: false,
        };

        // Focus first panel
        if let Some(first) = view.focus_order.first() {
            if let Some(host) = view.hosts.get(first) {
                host.set_focused(true);
            }
        }

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
        let ws = tp_core::config::load_workspace(path)
            .map_err(|e| format!("Failed to load: {}", e))?;
        self.config_path = Some(path.to_path_buf());
        self.rebuild_from_workspace(ws)
    }

    fn rebuild_from_workspace(&mut self, ws: Workspace) -> Result<(), String> {
        // Remove old root widget
        self.root_box.remove(&self.root_widget);

        let registry = registry::build_default_registry();
        let mut hosts = HashMap::new();

        for panel_cfg in &ws.panels {
            let host = PanelHost::new(&panel_cfg.id, &panel_cfg.name, self.action_cb.clone());
            if panel_cfg.effective_type() == PanelType::Empty {
                let chooser = ChooserPanel::new(&panel_cfg.id, &registry, self.on_type_chosen.clone());
                host.set_backend(Box::new(chooser));
            } else {
                let backend = create_backend_from_registry(panel_cfg, &ws.settings.default_shell, &registry);
                host.set_backend(backend);
            }
            hosts.insert(panel_cfg.id.clone(), host);
        }

        let root_widget = build_layout_widget(&ws.layout, &hosts);
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

        if let Some(first) = self.focus_order.first() {
            if let Some(host) = self.hosts.get(first) {
                host.set_focused(true);
            }
        }

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

    /// Update panel config after Configure dialog.
    /// Recreates the backend with the new type/settings and runs startup commands.
    pub fn apply_panel_config(&mut self, panel_id: &str, new_name: String, new_type: PanelType, startup_commands: Vec<String>) {
        // Update model
        if let Some(panel_cfg) = self.workspace.panels.iter_mut().find(|p| p.id == panel_id) {
            panel_cfg.name = new_name.clone();
            panel_cfg.panel_type = new_type.clone();
            panel_cfg.startup_commands = startup_commands.clone();
        }

        // Update title
        if let Some(host) = self.hosts.get(panel_id) {
            host.set_title(&new_name);
        }

        // Recreate backend with startup commands
        let config = panel_type_to_create_config(&new_type, &self.workspace.settings.default_shell);
        if let Some(backend) = self.registry.create(panel_type_to_id(&new_type), &config) {
            if let Some(host) = self.hosts.get(panel_id) {
                host.set_backend(backend);
                // Send startup commands
                for cmd in &startup_commands {
                    let line = format!("{}\n", cmd);
                    host.write_input(line.as_bytes());
                }
            }
        }

        self.dirty = true;
    }

    /// Get startup commands for a panel.
    pub fn panel_startup_commands(&self, panel_id: &str) -> Vec<String> {
        self.workspace.panels.iter()
            .find(|p| p.id == panel_id)
            .map(|p| p.startup_commands.clone())
            .unwrap_or_default()
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
    /// Propagates to all existing panel hosts and adds "+" buttons to existing notebooks.
    pub fn set_action_callback(&mut self, cb: PanelActionCallback) {
        for host in self.hosts.values() {
            host.set_action_callback(cb.clone());
        }
        // Add "+" buttons to any existing notebooks in the widget tree
        add_plus_buttons_recursive(&self.root_widget, &cb);
        self.action_cb = Some(cb);
    }

    pub fn widget(&self) -> &gtk4::ScrolledWindow {
        &self.scrolled
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    // ── Focus management ─────────────────────────────────────────────────

    pub fn focus_next(&mut self) {
        if self.focus_order.is_empty() {
            return;
        }
        if let Some(current) = self.focus_order.get(self.focus_index) {
            if let Some(host) = self.hosts.get(current) {
                host.set_focused(false);
            }
        }
        self.focus_index = (self.focus_index + 1) % self.focus_order.len();
        if let Some(next) = self.focus_order.get(self.focus_index) {
            if let Some(host) = self.hosts.get(next) {
                host.set_focused(true);
            }
        }
    }

    pub fn focus_prev(&mut self) {
        if self.focus_order.is_empty() {
            return;
        }
        if let Some(current) = self.focus_order.get(self.focus_index) {
            if let Some(host) = self.hosts.get(current) {
                host.set_focused(false);
            }
        }
        self.focus_index = if self.focus_index == 0 {
            self.focus_order.len() - 1
        } else {
            self.focus_index - 1
        };
        if let Some(next) = self.focus_order.get(self.focus_index) {
            if let Some(host) = self.hosts.get(next) {
                host.set_focused(true);
            }
        }
    }

    pub fn focused_panel_id(&self) -> Option<&str> {
        self.focus_order
            .get(self.focus_index)
            .map(|s| s.as_str())
    }

    pub fn focus_order_index(&self, panel_id: &str) -> Option<usize> {
        self.focus_order.iter().position(|s| s == panel_id)
    }

    pub fn set_focus_index(&mut self, idx: usize) {
        if let Some(current) = self.focus_order.get(self.focus_index) {
            if let Some(host) = self.hosts.get(current) {
                host.set_focused(false);
            }
        }
        self.focus_index = idx.min(self.focus_order.len().saturating_sub(1));
        if let Some(next) = self.focus_order.get(self.focus_index) {
            if let Some(host) = self.hosts.get(next) {
                host.set_focused(true);
            }
        }
    }

    pub fn host(&self, panel_id: &str) -> Option<&PanelHost> {
        self.hosts.get(panel_id)
    }

    pub fn hosts(&self) -> &HashMap<String, PanelHost> {
        &self.hosts
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
        let new_id = self.alloc_panel_id();
        let new_name = format!("New Panel {}", &new_id[1..]);

        // 1. Create new panel
        let new_cfg = PanelConfig {
            id: new_id.clone(),
            name: new_name.clone(),
            panel_type: PanelType::Terminal,
            target: Default::default(),
            startup_commands: vec![],
            groups: vec![],
            record_output: false,
            cwd: None,
            env: Default::default(),
            pre_script: None,
            post_script: None,
        };
        let host = PanelHost::new(&new_id, &new_name, self.action_cb.clone());
        let backend = self.create_chooser_backend(&new_id);
        host.set_backend(backend);

        // 2. Reparent in widget tree
        let focused_widget = self.hosts.get(&focused_id)?.widget().clone();
        let parent = focused_widget.parent()?;

        let new_paned = gtk4::Paned::new(orientation);
        new_paned.set_shrink_start_child(true);
        new_paned.set_shrink_end_child(true);

        let new_host_widget = host.widget().clone();
        new_host_widget.set_vexpand(true);
        new_host_widget.set_hexpand(true);

        // Remove focused from parent, insert paned, put both in paned
        if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
            let is_start = paned
                .start_child()
                .map(|w| w == focused_widget)
                .unwrap_or(false);

            if is_start {
                paned.set_start_child(gtk4::Widget::NONE);
                new_paned.set_start_child(Some(&focused_widget));
                new_paned.set_end_child(Some(&new_host_widget));
                paned.set_start_child(Some(new_paned.upcast_ref::<gtk4::Widget>()));
            } else {
                paned.set_end_child(gtk4::Widget::NONE);
                new_paned.set_start_child(Some(&focused_widget));
                new_paned.set_end_child(Some(&new_host_widget));
                paned.set_end_child(Some(new_paned.upcast_ref::<gtk4::Widget>()));
            }
        } else if let Some(notebook) = parent.downcast_ref::<gtk4::Notebook>() {
            let page_num = notebook.page_num(&focused_widget);
            let tab_label_widget = notebook.tab_label(&focused_widget);
            notebook.remove_page(page_num);
            new_paned.set_start_child(Some(&focused_widget));
            new_paned.set_end_child(Some(&new_host_widget));
            let paned_widget = new_paned.clone().upcast::<gtk4::Widget>();
            notebook.insert_page(&paned_widget, tab_label_widget.as_ref(), page_num);
            notebook.set_current_page(page_num);
        } else if let Some(notebook) = find_notebook_ancestor(&focused_widget) {
            // Panel is inside a Notebook but parent is a GTK internal wrapper
            let page_num = notebook.page_num(&focused_widget);
            let tab_label_widget = notebook.tab_label(&focused_widget);
            notebook.remove_page(page_num);
            new_paned.set_start_child(Some(&focused_widget));
            new_paned.set_end_child(Some(&new_host_widget));
            let paned_widget = new_paned.clone().upcast::<gtk4::Widget>();
            notebook.insert_page(&paned_widget, tab_label_widget.as_ref(), page_num);
            notebook.set_current_page(page_num);
        } else if let Some(bx) = parent.downcast_ref::<gtk4::Box>() {
            // Root box
            bx.remove(&focused_widget);
            new_paned.set_start_child(Some(&focused_widget));
            new_paned.set_end_child(Some(&new_host_widget));
            let paned_widget = new_paned.clone().upcast::<gtk4::Widget>();
            paned_widget.set_vexpand(true);
            paned_widget.set_hexpand(true);
            bx.prepend(&paned_widget);
            self.root_widget = paned_widget;
        } else {
            return None;
        }

        // Set 50/50 split after realize
        new_paned.connect_realize(move |paned| {
            let alloc = paned.allocation();
            let total = match orientation {
                gtk4::Orientation::Horizontal => alloc.width(),
                _ => alloc.height(),
            };
            paned.set_position(total / 2);
        });

        // 3. Update model
        self.update_layout_split(&focused_id, &new_id, orientation);
        self.workspace.panels.push(new_cfg);

        // 4. Update focus order and hosts
        self.hosts.insert(new_id.clone(), host);
        self.rebuild_focus_order();

        Some(new_id)
    }

    /// Wrap the focused panel in a new TabSplit (Notebook) with a second tab.
    /// The Notebook gets a "+" button in the tab bar to add more tabs.
    pub fn add_tab_focused(&mut self) -> Option<String> {
        let focused_id = self.focused_panel_id()?.to_string();
        let new_id = self.alloc_panel_id();
        let new_name = format!("New Panel {}", &new_id[1..]);

        let new_cfg = self.make_empty_config(&new_id, &new_name);
        let host = PanelHost::new(&new_id, &new_name, self.action_cb.clone());
        let backend = self.create_chooser_backend(&new_id);
        host.set_backend(backend);

        let focused_widget = self.hosts.get(&focused_id)?.widget().clone();
        let parent = focused_widget.parent()?;
        let new_host_widget = host.widget().clone();
        new_host_widget.set_vexpand(true);
        new_host_widget.set_hexpand(true);

        // Always create a new Notebook (TabSplit)
        let notebook = gtk4::Notebook::new();
        notebook.set_show_tabs(true);
        notebook.set_scrollable(true);

        let focused_name = self.workspace
            .panel(&focused_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| focused_id.clone());

        // Detach focused widget from parent, wrap in notebook
        if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
            let is_start = paned.start_child().map(|w| w == focused_widget).unwrap_or(false);
            if is_start {
                paned.set_start_child(gtk4::Widget::NONE);
            } else {
                paned.set_end_child(gtk4::Widget::NONE);
            }

            let label1 = build_tab_label(&focused_name, &self.action_cb, &focused_widget);
            notebook.append_page(&focused_widget, Some(&label1));
            let label2 = build_tab_label(&new_name, &self.action_cb, &new_host_widget);
            notebook.append_page(&new_host_widget, Some(&label2));

            let nb_widget = notebook.clone().upcast::<gtk4::Widget>();
            if is_start {
                paned.set_start_child(Some(&nb_widget));
            } else {
                paned.set_end_child(Some(&nb_widget));
            }
        } else if let Some(bx) = parent.downcast_ref::<gtk4::Box>() {
            bx.remove(&focused_widget);

            let label1 = build_tab_label(&focused_name, &self.action_cb, &focused_widget);
            notebook.append_page(&focused_widget, Some(&label1));
            let label2 = build_tab_label(&new_name, &self.action_cb, &new_host_widget);
            notebook.append_page(&new_host_widget, Some(&label2));

            let nb_widget = notebook.clone().upcast::<gtk4::Widget>();
            nb_widget.set_vexpand(true);
            nb_widget.set_hexpand(true);
            bx.prepend(&nb_widget);
            self.root_widget = nb_widget.clone();
        } else if let Some(parent_nb) = find_notebook_ancestor(&focused_widget) {
            // Panel is inside another Notebook tab — replace the tab content
            let page_num = parent_nb.page_num(&focused_widget);
            let old_tab_label = parent_nb.tab_label(&focused_widget);
            parent_nb.remove_page(page_num);

            let label1 = build_tab_label(&focused_name, &self.action_cb, &focused_widget);
            notebook.append_page(&focused_widget, Some(&label1));
            let label2 = build_tab_label(&new_name, &self.action_cb, &new_host_widget);
            notebook.append_page(&new_host_widget, Some(&label2));

            let nb_widget = notebook.clone().upcast::<gtk4::Widget>();
            parent_nb.insert_page(&nb_widget, old_tab_label.as_ref(), page_num);
            parent_nb.set_current_page(page_num);
        } else {
            return None;
        }

        // Add ⋮ menu to notebook tab bar
        self.setup_notebook_menu(&notebook);

        notebook.set_current_page(Some(1));

        // Update model
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

        let new_widget = host.widget().clone();
        new_widget.set_vexpand(true);
        new_widget.set_hexpand(true);

        let label = build_tab_label(&new_name, &self.action_cb, &new_widget);
        notebook.append_page(&new_widget, Some(&label));
        let new_page = notebook.n_pages() - 1;
        notebook.set_current_page(Some(new_page));

        // Update model — append to the Tabs node containing sibling_id
        add_to_existing_tabs(&mut self.workspace.layout, &sibling_id, &new_id, &new_name);
        self.workspace.panels.push(new_cfg);
        self.hosts.insert(new_id.clone(), host);
        self.rebuild_focus_order();

        Some(new_id)
    }

    /// Set up the ⋮ menu button in a Notebook's tab bar area.
    fn setup_notebook_menu(&self, notebook: &gtk4::Notebook) {
        setup_notebook_menu_widget(notebook, self.action_cb.clone());
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
            panel_type: PanelType::Terminal, // Will be set when user chooses
            target: Default::default(),
            startup_commands: vec![],
            groups: vec![],
            record_output: false,
            cwd: None,
            env: Default::default(),
            pre_script: None,
            post_script: None,
        }
    }

    fn create_chooser_backend(&self, panel_id: &str) -> Box<dyn crate::panels::PanelBackend> {
        Box::new(ChooserPanel::new(panel_id, &self.registry, self.on_type_chosen.clone()))
    }

    /// Close the focused panel.
    pub fn close_focused(&mut self) -> bool {
        if self.focus_order.len() <= 1 {
            return false; // Don't close the last panel
        }

        let focused_id = match self.focused_panel_id() {
            Some(id) => id.to_string(),
            None => return false,
        };

        let focused_widget = match self.hosts.get(&focused_id) {
            Some(h) => h.widget().clone(),
            None => return false,
        };

        let parent = match focused_widget.parent() {
            Some(p) => p,
            None => return false,
        };

        // Remove from widget tree
        if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
            let is_start = paned
                .start_child()
                .map(|w| w == focused_widget)
                .unwrap_or(false);

            // Get the sibling (the one that stays)
            let sibling = if is_start {
                paned.end_child()
            } else {
                paned.start_child()
            };

            if let Some(sibling) = sibling {
                // Remove both children from paned
                paned.set_start_child(gtk4::Widget::NONE);
                paned.set_end_child(gtk4::Widget::NONE);

                // Replace paned with sibling in paned's parent
                let grandparent = paned.parent();
                if let Some(gp) = grandparent {
                    let paned_widget = paned.clone().upcast::<gtk4::Widget>();
                    if let Some(gp_paned) = gp.downcast_ref::<gtk4::Paned>() {
                        let is_gp_start = gp_paned
                            .start_child()
                            .map(|w| w == paned_widget)
                            .unwrap_or(false);
                        if is_gp_start {
                            gp_paned.set_start_child(gtk4::Widget::NONE);
                            gp_paned.set_start_child(Some(&sibling));
                        } else {
                            gp_paned.set_end_child(gtk4::Widget::NONE);
                            gp_paned.set_end_child(Some(&sibling));
                        }
                    } else if let Some(gp_nb) = gp.downcast_ref::<gtk4::Notebook>() {
                        let page_num = gp_nb.page_num(&paned_widget);
                        let tab_label = gp_nb.tab_label(&paned_widget);
                        gp_nb.remove_page(page_num);
                        gp_nb.insert_page(&sibling, tab_label.as_ref(), page_num);
                    } else if let Some(gp_box) = gp.downcast_ref::<gtk4::Box>() {
                        gp_box.remove(&paned_widget);
                        sibling.set_vexpand(true);
                        sibling.set_hexpand(true);
                        gp_box.prepend(&sibling);
                        self.root_widget = sibling;
                    }
                }
            }
        } else if let Some(notebook) = parent.downcast_ref::<gtk4::Notebook>()
            .cloned()
            .or_else(|| find_notebook_ancestor(&focused_widget))
        {
            let page_num = notebook.page_num(&focused_widget);
            notebook.remove_page(page_num);

            // If only 1 tab left, unwrap the notebook
            if notebook.n_pages() == 1 {
                if let Some(remaining) = notebook.nth_page(Some(0)) {
                    notebook.remove_page(Some(0));
                    let nb_widget = notebook.clone().upcast::<gtk4::Widget>();
                    let nb_parent = nb_widget.parent();
                    if let Some(nbp) = nb_parent {
                        if let Some(p) = nbp.downcast_ref::<gtk4::Paned>() {
                            let is_start = p
                                .start_child()
                                .map(|w| w == nb_widget)
                                .unwrap_or(false);
                            if is_start {
                                p.set_start_child(gtk4::Widget::NONE);
                                p.set_start_child(Some(&remaining));
                            } else {
                                p.set_end_child(gtk4::Widget::NONE);
                                p.set_end_child(Some(&remaining));
                            }
                        } else if let Some(bx) = nbp.downcast_ref::<gtk4::Box>() {
                            bx.remove(&nb_widget);
                            remaining.set_vexpand(true);
                            remaining.set_hexpand(true);
                            bx.prepend(&remaining);
                            self.root_widget = remaining;
                        }
                    }
                }
            }
        }

        // Update model
        self.update_layout_remove(&focused_id);
        self.workspace.panels.retain(|p| p.id != focused_id);
        self.hosts.remove(&focused_id);
        self.rebuild_focus_order();

        // Focus next available
        if self.focus_index >= self.focus_order.len() {
            self.focus_index = 0;
        }
        if let Some(next) = self.focus_order.get(self.focus_index) {
            if let Some(host) = self.hosts.get(next) {
                host.set_focused(true);
            }
        }

        true
    }

    // ── Save ─────────────────────────────────────────────────────────────

    /// Save the current workspace to the original config file.
    pub fn save(&mut self) -> Result<PathBuf, String> {
        let path = self
            .config_path
            .as_ref()
            .ok_or("No config path set")?
            .clone();
        tp_core::config::save_workspace(&self.workspace, &path)
            .map_err(|e| format!("Save failed: {}", e))?;
        self.dirty = false;
        self.record_in_db();
        Ok(path)
    }

    /// Save to a specific path.
    pub fn save_as(&mut self, path: &Path) -> Result<(), String> {
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
        self.focus_order = self
            .workspace
            .layout
            .panel_ids()
            .iter()
            .map(|s| s.to_string())
            .collect();
        if self.focus_index >= self.focus_order.len() && !self.focus_order.is_empty() {
            self.focus_index = self.focus_order.len() - 1;
        }
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

    fn update_layout_add_tab(&mut self, existing_id: &str, new_id: &str, new_label: &str) {
        // Check if the panel is already inside a Tabs node — if so, append to it
        if add_to_existing_tabs(&mut self.workspace.layout, existing_id, new_id, new_label) {
            return;
        }
        // Otherwise wrap the panel in a new Tabs node
        let existing_label = self
            .workspace
            .panel(existing_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| existing_id.to_string());
        let new_label = new_label.to_string();
        let new_id = new_id.to_string();
        let existing_id = existing_id.to_string();
        self.workspace.layout = replace_in_layout(
            &self.workspace.layout,
            &existing_id,
            &|_| LayoutNode::Tabs {
                children: vec![
                    LayoutNode::Panel { id: existing_id.clone() },
                    LayoutNode::Panel { id: new_id.clone() },
                ],
                labels: vec![existing_label.clone(), new_label.clone()],
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

// ── Layout model helpers ─────────────────────────────────────────────────────

/// Replace a Panel node in the layout tree, returning a new tree.
fn replace_in_layout(
    node: &LayoutNode,
    panel_id: &str,
    replacer: &dyn Fn(&LayoutNode) -> LayoutNode,
) -> LayoutNode {
    match node {
        LayoutNode::Panel { id } if id == panel_id => replacer(node),
        LayoutNode::Panel { .. } => node.clone(),
        LayoutNode::Hsplit { children, ratios } => LayoutNode::Hsplit {
            children: children
                .iter()
                .map(|c| replace_in_layout(c, panel_id, replacer))
                .collect(),
            ratios: ratios.clone(),
        },
        LayoutNode::Vsplit { children, ratios } => LayoutNode::Vsplit {
            children: children
                .iter()
                .map(|c| replace_in_layout(c, panel_id, replacer))
                .collect(),
            ratios: ratios.clone(),
        },
        LayoutNode::Tabs { children, labels } => LayoutNode::Tabs {
            children: children
                .iter()
                .map(|c| replace_in_layout(c, panel_id, replacer))
                .collect(),
            labels: labels.clone(),
        },
    }
}

/// Remove a panel from the layout tree, collapsing empty containers.
fn remove_from_layout(node: &LayoutNode, panel_id: &str) -> LayoutNode {
    match node {
        LayoutNode::Panel { id } if id == panel_id => {
            // This shouldn't be called on the root directly,
            // but return a dummy that parent will clean up
            node.clone()
        }
        LayoutNode::Panel { .. } => node.clone(),
        LayoutNode::Hsplit { children, ratios } => {
            let filtered: Vec<LayoutNode> = children
                .iter()
                .filter(|c| !is_panel_with_id(c, panel_id))
                .map(|c| remove_from_layout(c, panel_id))
                .collect();
            let new_ratios: Vec<f64> = children
                .iter()
                .zip(ratios.iter())
                .filter(|(c, _)| !is_panel_with_id(c, panel_id))
                .map(|(_, r)| *r)
                .collect();
            if filtered.len() == 1 {
                filtered.into_iter().next().unwrap()
            } else {
                LayoutNode::Hsplit {
                    children: filtered,
                    ratios: new_ratios,
                }
            }
        }
        LayoutNode::Vsplit { children, ratios } => {
            let filtered: Vec<LayoutNode> = children
                .iter()
                .filter(|c| !is_panel_with_id(c, panel_id))
                .map(|c| remove_from_layout(c, panel_id))
                .collect();
            let new_ratios: Vec<f64> = children
                .iter()
                .zip(ratios.iter())
                .filter(|(c, _)| !is_panel_with_id(c, panel_id))
                .map(|(_, r)| *r)
                .collect();
            if filtered.len() == 1 {
                filtered.into_iter().next().unwrap()
            } else {
                LayoutNode::Vsplit {
                    children: filtered,
                    ratios: new_ratios,
                }
            }
        }
        LayoutNode::Tabs { children, labels } => {
            let mut new_children = Vec::new();
            let mut new_labels = Vec::new();
            for (i, child) in children.iter().enumerate() {
                if !is_panel_with_id(child, panel_id) {
                    new_children.push(remove_from_layout(child, panel_id));
                    if let Some(l) = labels.get(i) {
                        new_labels.push(l.clone());
                    }
                }
            }
            if new_children.len() == 1 {
                new_children.into_iter().next().unwrap()
            } else {
                LayoutNode::Tabs {
                    children: new_children,
                    labels: new_labels,
                }
            }
        }
    }
}

/// Try to add a new panel to an existing Tabs node that contains the given panel.
/// Returns true if successful.
fn add_to_existing_tabs(node: &mut LayoutNode, panel_id: &str, new_id: &str, new_label: &str) -> bool {
    match node {
        LayoutNode::Tabs { children, labels } => {
            // Check if any direct child is this panel
            let contains = children.iter().any(|c| is_panel_with_id(c, panel_id));
            if contains {
                children.push(LayoutNode::Panel { id: new_id.to_string() });
                labels.push(new_label.to_string());
                return true;
            }
            // Recurse into children
            for child in children.iter_mut() {
                if add_to_existing_tabs(child, panel_id, new_id, new_label) {
                    return true;
                }
            }
            false
        }
        LayoutNode::Hsplit { children, .. } | LayoutNode::Vsplit { children, .. } => {
            for child in children.iter_mut() {
                if add_to_existing_tabs(child, panel_id, new_id, new_label) {
                    return true;
                }
            }
            false
        }
        LayoutNode::Panel { .. } => false,
    }
}

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
    let label = gtk4::Label::new(Some(name));
    hbox.append(&label);

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

fn is_panel_with_id(node: &LayoutNode, panel_id: &str) -> bool {
    matches!(node, LayoutNode::Panel { id } if id == panel_id)
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

fn panel_type_to_create_config(pt: &PanelType, default_shell: &str) -> PanelCreateConfig {
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
) -> Box<dyn crate::panels::PanelBackend> {
    let effective = panel_cfg.effective_type();
    let (type_id, extra) = match &effective {
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

    let config = PanelCreateConfig {
        shell: default_shell.to_string(),
        cwd: panel_cfg.cwd.clone(),
        env: panel_cfg.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        extra,
    };

    let mut backend = registry.create(type_id, &config)
        .unwrap_or_else(|| Box::new(MarkdownPanel::new("/dev/null")));

    // Send startup commands for terminal-like panels
    if !panel_cfg.startup_commands.is_empty() && backend.accepts_input() {
        // The backend is already created with commands via the factory
        // but startup_commands from config need to be sent separately
        // We handle this by writing to the backend
        for cmd in &panel_cfg.startup_commands {
            let line = format!("{}\n", cmd);
            backend.write_input(line.as_bytes());
        }
    }

    backend
}

// ── Layout widget building ───────────────────────────────────────────────────

/// Recursively build GTK widgets from a LayoutNode tree.
fn build_layout_widget(
    node: &LayoutNode,
    hosts: &HashMap<String, PanelHost>,
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
            build_paned(children, ratios, hosts, gtk4::Orientation::Horizontal)
        }
        LayoutNode::Vsplit { children, ratios } => {
            build_paned(children, ratios, hosts, gtk4::Orientation::Vertical)
        }
        LayoutNode::Tabs { children, labels } => {
            let notebook = gtk4::Notebook::new();
            notebook.set_show_tabs(true);
            notebook.set_scrollable(true);

            for (i, child) in children.iter().enumerate() {
                let child_widget = build_layout_widget(child, hosts);
                let label_text = labels
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("Tab {}", i + 1));
                let label = gtk4::Label::new(Some(&label_text));
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
    orientation: gtk4::Orientation,
) -> gtk4::Widget {
    if children.is_empty() {
        return gtk4::Box::new(orientation, 0).upcast::<gtk4::Widget>();
    }
    if children.len() == 1 {
        return build_layout_widget(&children[0], hosts);
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
        let w1 = build_layout_widget(&children[0], hosts);
        let w2 = build_layout_widget(&children[1], hosts);
        paned.set_start_child(Some(&w1));
        paned.set_end_child(Some(&w2));
        paned.set_shrink_start_child(true);
        paned.set_shrink_end_child(true);
        paned.set_resize_start_child(true);
        paned.set_resize_end_child(true);

        setup_paned_ratio(&paned, normalized[0], orientation);
        return paned.upcast::<gtk4::Widget>();
    }

    let paned = gtk4::Paned::new(orientation);
    let w1 = build_layout_widget(&children[0], hosts);
    let rest = build_paned(&children[1..], &ratios[1..], hosts, orientation);
    paned.set_start_child(Some(&w1));
    paned.set_end_child(Some(&rest));
    paned.set_shrink_start_child(true);
    paned.set_shrink_end_child(true);
    paned.set_resize_start_child(true);
    paned.set_resize_end_child(true);

    setup_paned_ratio(&paned, normalized[0], orientation);
    paned.upcast::<gtk4::Widget>()
}
