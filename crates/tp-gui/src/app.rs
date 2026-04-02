use anyhow::Result;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::gdk;
use libadwaita as adw;
use adw::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use pax_core::workspace::Workspace;

use crate::actions::{self, DIRTY_INDICATOR, HEADER_WS_LABEL};
use crate::layout_ops::update_tab_label_in_layout;
use crate::panel_host::PanelAction;
use crate::theme::Theme;
use crate::workspace_view::WorkspaceView;
use crate::widgets::status_bar::StatusBar;

/// Single entry point — shows welcome if no workspace, or workspace directly.
pub fn run_app(workspace: Option<Workspace>, config_path: Option<&Path>) -> Result<()> {
    let app = adw::Application::builder()
        .application_id("com.sinelec.pax")
        .build();

    let ws = workspace;
    let cp = config_path.map(|p| p.to_path_buf());

    app.connect_activate(move |app| {
        load_css();

        // Register custom icons from resources/icons/
        if let Some(display) = gtk4::gdk::Display::default() {
            let icon_theme = gtk4::IconTheme::for_display(&display);
            // Try to find icons relative to the executable or manifest dir
            let icon_paths = [
                std::path::PathBuf::from("resources/icons"),
                std::env::current_exe().ok()
                    .and_then(|p| p.parent().map(|d| d.join("../resources/icons")))
                    .unwrap_or_default(),
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/icons"),
            ];
            for path in &icon_paths {
                if path.exists() {
                    icon_theme.add_search_path(path);
                    break;
                }
            }
        }

        // Register custom GtkSourceView style schemes
        #[cfg(feature = "sourceview")]
        {
            let style_paths = [
                std::path::PathBuf::from("resources/sourceview-styles"),
                std::env::current_exe().ok()
                    .and_then(|p| p.parent().map(|d| d.join("../resources/sourceview-styles")))
                    .unwrap_or_default(),
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/sourceview-styles"),
            ];
            let manager = sourceview5::StyleSchemeManager::default();
            for path in &style_paths {
                if path.exists() {
                    manager.append_search_path(&path.to_string_lossy());
                    break;
                }
            }
        }

        // Set default window icon (used by window manager / taskbar)
        gtk4::Window::set_default_icon_name("pax");

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Pax")
            .default_width(1200)
            .default_height(800)
            .icon_name("pax")
            .build();

        let window_rc = Rc::new(window.clone());

        if let Some(ref workspace) = ws {
            // Direct workspace launch
            setup_workspace_ui(&window_rc, workspace.clone(), cp.as_deref());
        } else {
            // Show welcome, then transition to workspace
            setup_welcome_ui(&window_rc);
        }

        window.present();
    });

    app.run_with_args::<String>(&[]);
    Ok(())
}

/// Convenience: launch with welcome screen.
pub fn run_app_welcome() -> Result<()> {
    run_app(None, None)
}

/// Setup the welcome screen in the window.
fn setup_welcome_ui(window: &Rc<adw::ApplicationWindow>) {
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(true);
    header.set_show_start_title_buttons(true);

    // Pax icon in center of welcome header
    let title_icon = gtk4::Image::from_icon_name("pax");
    title_icon.set_pixel_size(20);
    header.set_title_widget(Some(&title_icon));

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);

    let win = window.clone();
    let welcome = crate::widgets::welcome::build_welcome(Rc::new(move |choice| {
        use crate::widgets::welcome::WelcomeChoice;
        match choice {
            WelcomeChoice::NewWorkspace => {
                let ws = pax_core::template::empty_workspace("untitled");
                setup_workspace_ui(&win, ws, None);
            }
            WelcomeChoice::OpenFile => {
                let win2 = win.clone();
                let dialog = gtk4::FileDialog::builder()
                    .title("Open Workspace")
                    .modal(true)
                    .build();
                let filter = gtk4::FileFilter::new();
                filter.set_name(Some("JSON files"));
                filter.add_pattern("*.json");
                let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
                filters.append(&filter);
                dialog.set_filters(Some(&filters));

                let win3 = win2.clone();
                dialog.open(Some(win2.as_ref()), gtk4::gio::Cancellable::NONE, move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            if let Ok(ws) = pax_core::config::load_workspace(&path) {
                                setup_workspace_ui(&win3, ws, Some(&path));
                            }
                        }
                    }
                });
            }
            WelcomeChoice::OpenRecent(path) => {
                let p = std::path::PathBuf::from(&path);
                if let Ok(ws) = pax_core::config::load_workspace(&p) {
                    setup_workspace_ui(&win, ws, Some(&p));
                }
            }
        }
    }));

    toolbar_view.set_content(Some(&welcome));
    window.set_content(Some(&toolbar_view));
}

/// Setup the full workspace UI in the window (replaces any existing content).
fn setup_workspace_ui(
    window: &Rc<adw::ApplicationWindow>,
    workspace: Workspace,
    config_path: Option<&Path>,
) {
    let ws_name = workspace.name.clone();
    window.set_title(Some(&format!("Pax — {}", ws_name)));

    // Apply saved theme
    apply_theme(Theme::from_id(&workspace.settings.theme));

    // Header bar with hamburger menu
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(true);
    header.set_show_start_title_buttons(true);

    // Pax icon + workspace name centered in header
    let title_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    title_box.set_halign(gtk4::Align::Center);
    let title_icon = gtk4::Image::from_icon_name("pax");
    title_icon.set_pixel_size(22);
    title_box.append(&title_icon);
    let app_label = gtk4::Label::new(Some("Pax"));
    app_label.add_css_class("heading");
    title_box.append(&app_label);
    let sep_label = gtk4::Label::new(Some("—"));
    sep_label.add_css_class("dim-label");
    title_box.append(&sep_label);
    let ws_label = gtk4::Label::new(Some(&ws_name));
    ws_label.add_css_class("dim-label");
    title_box.append(&ws_label);
    header.set_title_widget(Some(&title_box));
    HEADER_WS_LABEL.with(|cell| {
        cell.borrow_mut().replace(ws_label.clone());
    });

    let menu_btn = gtk4::MenuButton::new();
    menu_btn.set_icon_name("open-menu-symbolic");
    menu_btn.set_tooltip_text(Some("Menu"));

    let menu = gtk4::gio::Menu::new();
    let file_section = gtk4::gio::Menu::new();
    file_section.append(Some("New Workspace"), Some("app.new"));
    file_section.append(Some("Open Workspace…"), Some("app.open"));
    file_section.append(Some("Open Recent…"), Some("app.recent"));
    file_section.append(Some("Save"), Some("app.save"));
    file_section.append(Some("Save As…"), Some("app.save-as"));
    menu.append_section(None, &file_section);
    let settings_section = gtk4::gio::Menu::new();
    settings_section.append(Some("Settings…"), Some("app.settings"));
    settings_section.append(Some("Keyboard Shortcuts"), Some("app.shortcuts"));
    menu.append_section(None, &settings_section);
    menu.append(Some("Quit"), Some("app.quit"));
    menu_btn.set_menu_model(Some(&menu));
    header.pack_start(&menu_btn);

    // Dirty indicator (orange floppy) — packed at end (right side, near window buttons)
    let dirty_sep = gtk4::Separator::new(gtk4::Orientation::Vertical);
    dirty_sep.set_margin_start(4);
    dirty_sep.set_margin_end(4);
    dirty_sep.set_visible(false);
    header.pack_end(&dirty_sep);

    let dirty_icon = gtk4::Image::from_icon_name("media-floppy-symbolic");
    dirty_icon.add_css_class("dirty-indicator");
    dirty_icon.set_tooltip_text(Some("Unsaved changes"));
    dirty_icon.set_visible(false);
    header.pack_end(&dirty_icon);
    DIRTY_INDICATOR.with(|cell| {
        cell.borrow_mut().replace((dirty_icon, dirty_sep));
    });

    // Shared state
    let ws_view = Rc::new(RefCell::new(WorkspaceView::build(&workspace, config_path)));
    let status_bar = Rc::new(RefCell::new(StatusBar::new()));
    let save_action = gtk4::gio::SimpleAction::new("save", None);
    let window_rc = window.clone();

    // Wire up type chooser callback
    {
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        let cb: crate::panels::chooser::OnTypeChosen = Rc::new(move |panel_id, type_id| {
            ws.borrow_mut().set_panel_type(panel_id, type_id);
            sb.borrow().set_message(&format!("{} → {}", panel_id, type_id));
            actions::update_dirty_ui(&ws, &win, &sa);
        });
        ws_view.borrow_mut().set_type_chosen_callback(cb);
    }

    // Wire up panel ⋮ menu action callback
    {
        let ws_for_cb = ws_view.clone();
        let sb_for_cb = status_bar.clone();
        let win_for_cb = window_rc.clone();
        let sa_for_cb = save_action.clone();
        let cb: crate::panel_host::PanelActionCallback = Rc::new(move |panel_id, action| {
            // "nb:<panel_id>" means action on notebook
            if let Some(real_id) = panel_id.strip_prefix("nb:") {
                let view = ws_for_cb.borrow();
                if let Some(host) = view.host(real_id) {
                    let widget = host.widget().clone();
                    if let Some(nb) = crate::widget_builder::find_notebook_ancestor(&widget) {
                        drop(view);
                        match action {
                            PanelAction::AddTabToNotebook => {
                                if let Some(new_id) = ws_for_cb.borrow_mut().add_tab_to_notebook(&nb) {
                                    sb_for_cb.borrow().set_message(&format!("Tab + → {}", new_id));
                                }
                            }
                            PanelAction::RemoveTab => {
                                {
                                    let v = ws_for_cb.borrow();
                                    if let Some(idx) = v.focus_order_index(real_id) {
                                        drop(v);
                                        ws_for_cb.borrow_mut().set_focus_index(idx);
                                    }
                                }
                                if ws_for_cb.borrow_mut().close_focused() {
                                    if let Some(id) = ws_for_cb.borrow().focused_panel_id() {
                                        sb_for_cb.borrow().set_panel(id);
                                    }
                                    sb_for_cb.borrow().set_message("Tab removed");
                                }
                            }
                            _ => {}
                        }
                    }
                }
                actions::update_dirty_ui(&ws_for_cb, &win_for_cb, &sa_for_cb);
                return;
            }

            // Focus the panel that triggered the action
            {
                let view = ws_for_cb.borrow();
                if let Some(idx) = view.focus_order_index(panel_id) {
                    drop(view);
                    ws_for_cb.borrow_mut().set_focus_index(idx);
                }
            }
            match action {
                PanelAction::SplitH => {
                    if let Some(new_id) = ws_for_cb.borrow_mut().split_focused_h() {
                        sb_for_cb.borrow().set_message(&format!("Split H → {}", new_id));
                    }
                }
                PanelAction::SplitV => {
                    if let Some(new_id) = ws_for_cb.borrow_mut().split_focused_v() {
                        sb_for_cb.borrow().set_message(&format!("Split V → {}", new_id));
                    }
                }
                PanelAction::AddTab => {
                    if let Some(new_id) = ws_for_cb.borrow_mut().add_tab_focused() {
                        sb_for_cb.borrow().set_message(&format!("TabSplit → {}", new_id));
                    }
                }
                PanelAction::Close => {
                    if ws_for_cb.borrow_mut().close_focused() {
                        if let Some(id) = ws_for_cb.borrow().focused_panel_id() {
                            sb_for_cb.borrow().set_panel(id);
                        }
                        sb_for_cb.borrow().set_message("Panel closed");
                    } else {
                        sb_for_cb.borrow().set_message("Cannot close last panel");
                    }
                }
                PanelAction::Configure => {
                    let (pname, ptype, pcwd, pssh, pcmds, pclose, pmw, pmh) = {
                        let view = ws_for_cb.borrow();
                        let ws = view.workspace();
                        let pcfg = ws.panel(panel_id);
                        (
                            pcfg.map(|p| p.name.clone()).unwrap_or_default(),
                            pcfg.map(|p| p.effective_type()).unwrap_or(pax_core::workspace::PanelType::Terminal),
                            pcfg.and_then(|p| p.cwd.clone()),
                            pcfg.and_then(|p| p.effective_ssh()),
                            pcfg.map(|p| p.startup_commands.clone()).unwrap_or_default(),
                            pcfg.and_then(|p| p.before_close.clone()),
                            pcfg.map(|p| p.min_width).unwrap_or(0),
                            pcfg.map(|p| p.min_height).unwrap_or(0),
                        )
                    };
                    let pid = panel_id.to_string();
                    let ws2 = ws_for_cb.clone();
                    let ws3 = ws_for_cb.clone();
                    let win2 = win_for_cb.clone();
                    let sa2 = sa_for_cb.clone();
                    // Share saved SSH configs with the dialog
                    let saved_ssh = {
                        let view = ws_for_cb.borrow();
                        std::rc::Rc::new(std::cell::RefCell::new(view.workspace().ssh_configs.clone()))
                    };
                    let saved_ssh_for_save = saved_ssh.clone();
                    crate::dialogs::panel_config::show_panel_config_dialog(
                        &*win_for_cb,
                        &pname,
                        &ptype,
                        pcwd.as_deref(),
                        pssh.as_ref(),
                        &pcmds,
                        pclose.as_deref(),
                        pmw,
                        pmh,
                        saved_ssh,
                        move |new_name, new_type, new_cwd, new_ssh, new_cmds, new_close, new_mw, new_mh| {
                            // Save updated SSH configs back to workspace
                            ws3.borrow_mut().workspace_mut().ssh_configs = saved_ssh_for_save.borrow().clone();
                            ws2.borrow_mut().apply_panel_config(&pid, new_name, new_type, new_cwd, new_ssh, new_cmds, new_close, new_mw, new_mh);
                            actions::update_dirty_ui(&ws2, &win2, &sa2);
                        },
                    );
                }
                PanelAction::Zoom => {
                    let idx = ws_for_cb.borrow().focus_order_index(panel_id);
                    if let Some(idx) = idx {
                        ws_for_cb.borrow_mut().set_focus_index(idx);
                    }
                    ws_for_cb.borrow_mut().toggle_zoom();
                    let zoomed = ws_for_cb.borrow().is_zoomed();
                    sb_for_cb.borrow().set_message(if zoomed { "Zoom ON" } else { "Zoom OFF" });
                }
                PanelAction::Sync => {
                    let idx = ws_for_cb.borrow().focus_order_index(panel_id);
                    if let Some(idx) = idx {
                        ws_for_cb.borrow_mut().set_focus_index(idx);
                    }
                    let result = ws_for_cb.borrow_mut().toggle_sync_focused();
                    if let Some((pid, is_synced)) = result {
                        let count = ws_for_cb.borrow().sync_count();
                        if is_synced {
                            sb_for_cb.borrow().set_message(&format!("Sync ON: {} ({} panels)", pid, count));
                        } else {
                            sb_for_cb.borrow().set_message(&format!("Sync OFF: {} ({} panels)", pid, count));
                        }
                    }
                }
                PanelAction::Rename(new_name) => {
                    let mut view = ws_for_cb.borrow_mut();
                    // Update panel config name
                    if let Some(panel_cfg) = view.workspace_mut()
                        .panels.iter_mut().find(|p| p.id == panel_id)
                    {
                        panel_cfg.name = new_name.clone();
                    }
                    // Update tab label in layout tree
                    update_tab_label_in_layout(&mut view.workspace_mut().layout, panel_id, &new_name);
                    // Update host title bar
                    if let Some(host) = view.host(panel_id) {
                        host.set_title(&new_name);
                    }
                    drop(view);
                    sb_for_cb.borrow().set_message(&format!("Renamed: {}", panel_id));
                }
                PanelAction::RenameTab(new_name) => {
                    // Only update the tab label in the layout tree, not the panel name.
                    // panel_id here is the first child panel — used to locate the tab.
                    let mut view = ws_for_cb.borrow_mut();
                    update_tab_label_in_layout(&mut view.workspace_mut().layout, panel_id, &new_name);
                    drop(view);
                    actions::update_dirty_ui(&ws_for_cb, &win_for_cb, &sa_for_cb);
                    sb_for_cb.borrow().set_message(&format!("Tab renamed: {}", new_name));
                }
                PanelAction::Collapse => {
                    let view = ws_for_cb.borrow();
                    if let Some(host) = view.hosts().get(panel_id) {
                        host.toggle_collapsed();
                    }
                }
                PanelAction::Focus => {
                    let idx = ws_for_cb.borrow().focus_order_index(panel_id);
                    if let Some(idx) = idx {
                        ws_for_cb.borrow_mut().set_focus_index(idx);
                    }
                }
                PanelAction::AddTabToNotebook | PanelAction::RemoveTab => {}
            }
            actions::update_dirty_ui(&ws_for_cb, &win_for_cb, &sa_for_cb);
        });
        ws_view.borrow_mut().set_action_callback(cb);
    }

    // Setup sync input propagation: when a synced terminal gets input,
    // forward it to all other synced terminals (VTE-only feature)
    #[cfg(feature = "vte")]
    {
        let ws = ws_view.clone();
        let sync_cb: Rc<dyn Fn(&str, &str)> = Rc::new(move |source_panel_id, text| {
            // try_borrow: the RefCell may already be mutably borrowed (e.g.
            // during focus changes that trigger VTE commit signals).
            if let Ok(view) = ws.try_borrow() {
                if view.is_panel_synced(source_panel_id) {
                    view.write_to_synced(text.as_bytes(), source_panel_id);
                }
            }
        });
        ws_view.borrow_mut().setup_sync_callbacks(sync_cb);
    }

    // Register GIO actions
    let action_group = gtk4::gio::SimpleActionGroup::new();

    // New workspace
    {
        let action = gtk4::gio::SimpleAction::new("new", None);
        let ws = ws_view.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        let sb = status_bar.clone();
        action.connect_activate(move |_, _| {
            let empty = pax_core::template::empty_workspace("untitled");
            if let Err(e) = ws.borrow_mut().load_workspace(empty, None) {
                sb.borrow().set_message(&format!("Error: {}", e));
            }
            actions::update_dirty_ui(&ws, &win, &sa);
            actions::update_status_bar_path(&ws, &sb);
        });
        action_group.add_action(&action);
    }

    // Open recent
    {
        let action = gtk4::gio::SimpleAction::new("recent", None);
        let ws = ws_view.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        let sb = status_bar.clone();
        action.connect_activate(move |_, _| {
            actions::show_recent_dialog(&ws, &sb, &win, &sa);
        });
        action_group.add_action(&action);
    }

    // Open file
    {
        let action = gtk4::gio::SimpleAction::new("open", None);
        let ws = ws_view.clone();
        let win = window_rc.clone();
        let sb = status_bar.clone();
        let sa = save_action.clone();
        action.connect_activate(move |_, _| {
            actions::do_open(&ws, &sb, &win, &sa);
        });
        action_group.add_action(&action);
    }

    // Save
    {
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        save_action.connect_activate(move |_, _| {
            actions::do_save(&ws, &sb, &win, &sa, false);
        });
        action_group.add_action(&save_action);
    }

    // Save As
    {
        let action = gtk4::gio::SimpleAction::new("save-as", None);
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        action.connect_activate(move |_, _| {
            actions::do_save(&ws, &sb, &win, &sa, true);
        });
        action_group.add_action(&action);
    }

    // Settings
    {
        let action = gtk4::gio::SimpleAction::new("settings", None);
        let ws = ws_view.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        action.connect_activate(move |_, _| {
            let current = {
                let view = ws.borrow();
                crate::dialogs::settings::AppSettings {
                    workspace_name: view.workspace().name.clone(),
                    theme: Theme::from_id(&view.workspace().settings.theme),
                    default_shell: view.workspace().settings.default_shell.clone(),
                    scrollback_lines: view.workspace().settings.scrollback_lines,
                    output_retention_days: view.workspace().settings.output_retention_days,
                }
            };
            let ws2 = ws.clone();
            let win2 = win.clone();
            let sa2 = sa.clone();
            crate::dialogs::settings::show_settings_dialog(&*win, &current, move |new_settings| {
                apply_theme(new_settings.theme);
                {
                    let mut view = ws2.borrow_mut();
                    view.rename_workspace(&new_settings.workspace_name);
                    let ws = view.workspace_mut();
                    ws.settings.theme = new_settings.theme.to_id().to_string();
                    ws.settings.default_shell = new_settings.default_shell;
                    ws.settings.scrollback_lines = new_settings.scrollback_lines;
                    ws.settings.output_retention_days = new_settings.output_retention_days;
                }
                actions::update_dirty_ui(&ws2, &win2, &sa2);
            });
        });
        action_group.add_action(&action);
    }

    // Quit
    {
        let action = gtk4::gio::SimpleAction::new("quit", None);
        let ws = ws_view.clone();
        action.connect_activate(move |_, _| {
            ws.borrow().run_all_before_close();
            std::process::exit(0);
        });
        action_group.add_action(&action);
    }

    // Keyboard shortcuts dialog
    {
        let action = gtk4::gio::SimpleAction::new("shortcuts", None);
        let win = window_rc.clone();
        action.connect_activate(move |_, _| {
            show_shortcuts_dialog(&win);
        });
        action_group.add_action(&action);
    }

    window.insert_action_group("app", Some(&action_group));

    // Window close request
    {
        let ws = ws_view.clone();
        window.connect_close_request(move |_| {
            ws.borrow().run_all_before_close();
            glib::Propagation::Proceed
        });
    }

    // Content
    let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let ws_widget = ws_view.borrow().widget().clone();
    ws_widget.set_vexpand(true);
    ws_widget.set_hexpand(true);
    content_box.append(&ws_widget);
    content_box.append(status_bar.borrow().widget());

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content_box));
    window.set_content(Some(&toolbar_view));

    // Keyboard shortcuts
    let controller = gtk4::EventControllerKey::new();
    controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

    {
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        controller.connect_key_pressed(move |_ctrl, key, _code, modifiers| {
            let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);

            if ctrl && shift {
                match key {
                    gdk::Key::H => {
                        if let Some(new_id) = ws.borrow_mut().split_focused_h() {
                            sb.borrow().set_message(&format!("Split H → {}", new_id));
                        }
                        actions::update_dirty_ui(&ws, &win, &sa);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::J => {
                        if let Some(new_id) = ws.borrow_mut().split_focused_v() {
                            sb.borrow().set_message(&format!("Split V → {}", new_id));
                        }
                        actions::update_dirty_ui(&ws, &win, &sa);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::T => {
                        if let Some(new_id) = ws.borrow_mut().add_tab_focused() {
                            sb.borrow().set_message(&format!("Tab → {}", new_id));
                        }
                        actions::update_dirty_ui(&ws, &win, &sa);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::W => {
                        if ws.borrow_mut().close_focused() {
                            if let Some(id) = ws.borrow().focused_panel_id() {
                                sb.borrow().set_panel(id);
                            }
                            sb.borrow().set_message("Panel closed");
                        }
                        actions::update_dirty_ui(&ws, &win, &sa);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::S => {
                        let result = ws.borrow_mut().toggle_sync_focused();
                        if let Some((panel_id, is_synced)) = result {
                            let count = ws.borrow().sync_count();
                            if is_synced {
                                sb.borrow().set_message(&format!("Sync ON: {} ({} panels)", panel_id, count));
                            } else {
                                sb.borrow().set_message(&format!("Sync OFF: {} ({} panels)", panel_id, count));
                            }
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::Z => {
                        let view = ws.borrow();
                        if let Some(id) = view.focused_panel_id() {
                            if let Some(host) = view.hosts().get(id) {
                                host.toggle_collapsed();
                            }
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::C | gdk::Key::V => {
                        return glib::Propagation::Proceed;
                    }
                    _ => {}
                }
            }

            if ctrl && !shift {
                match key {
                    gdk::Key::q => {
                        ws.borrow().run_all_before_close();
                        std::process::exit(0);
                    }
                    gdk::Key::n => {
                        ws.borrow_mut().focus_next();
                        if let Some(id) = ws.borrow().focused_panel_id() {
                            sb.borrow().set_panel(id);
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::p => {
                        ws.borrow_mut().focus_prev();
                        if let Some(id) = ws.borrow().focused_panel_id() {
                            sb.borrow().set_panel(id);
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::s => {
                        // If focused panel is a code editor, let Ctrl+S propagate
                        // to the editor's own save handler (saves the file, not workspace)
                        let is_code_editor = {
                            let view = ws.borrow();
                            view.focused_panel_id()
                                .and_then(|id| view.workspace().panel(id))
                                .map(|p| matches!(p.effective_type(), pax_core::workspace::PanelType::CodeEditor { .. }))
                                .unwrap_or(false)
                        };
                        if is_code_editor {
                            return glib::Propagation::Proceed;
                        }
                        actions::do_save(&ws, &sb, &win, &sa, false);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::o => {
                        actions::do_open(&ws, &sb, &win, &sa);
                        return glib::Propagation::Stop;
                    }
                    // Ctrl+R is reserved for terminal reverse-search (bash)
                    gdk::Key::r => {
                        return glib::Propagation::Proceed;
                    }
                    gdk::Key::z => {
                        ws.borrow_mut().toggle_zoom();
                        let zoomed = ws.borrow().is_zoomed();
                        if zoomed {
                            if let Some(id) = ws.borrow().focused_panel_id() {
                                sb.borrow().set_message(&format!("Zoom: {}", id));
                            }
                        } else {
                            sb.borrow().set_message("Zoom off");
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::Up | gdk::Key::Down | gdk::Key::Left | gdk::Key::Right => {
                        let ws_widget = ws.borrow().widget().clone();
                        let step = 80.0;
                        match key {
                            gdk::Key::Up => {
                                let adj = ws_widget.vadjustment();
                                adj.set_value(adj.value() - step);
                            }
                            gdk::Key::Down => {
                                let adj = ws_widget.vadjustment();
                                adj.set_value(adj.value() + step);
                            }
                            gdk::Key::Left => {
                                let adj = ws_widget.hadjustment();
                                adj.set_value(adj.value() - step);
                            }
                            gdk::Key::Right => {
                                let adj = ws_widget.hadjustment();
                                adj.set_value(adj.value() + step);
                            }
                            _ => {}
                        }
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }

            glib::Propagation::Proceed
        });
    }

    window.add_controller(controller);

    // Auto-save workspace every 30s if dirty and has a config path
    {
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(30), move || {
            let has_path = ws.borrow().has_config_path();
            let is_dirty = ws.borrow().is_dirty();
            if has_path && is_dirty {
                match ws.borrow_mut().save() {
                    Ok(path) => {
                        tracing::info!("Auto-saved workspace: {}", path.display());
                        sb.borrow().set_message("Auto-saved");
                    }
                    Err(e) => {
                        tracing::warn!("Auto-save failed: {}", e);
                    }
                }
                actions::update_dirty_ui(&ws, &win, &sa);
            }
            glib::ControlFlow::Continue
        });
    }

    // Ctrl+Scroll: scroll the entire workspace (bypasses VTE scroll capture)
    {
        let scroll_ctrl = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::VERTICAL | gtk4::EventControllerScrollFlags::HORIZONTAL,
        );
        scroll_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let ws_widget = ws_view.borrow().widget().clone();
        scroll_ctrl.connect_scroll(move |ctrl, dx, dy| {
            let mods = ctrl.current_event_state();
            if mods.contains(gdk::ModifierType::CONTROL_MASK) {
                let vadj = ws_widget.vadjustment();
                vadj.set_value(vadj.value() + dy * 50.0);
                let hadj = ws_widget.hadjustment();
                hadj.set_value(hadj.value() + dx * 50.0);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(scroll_ctrl);
    }

    // Initial UI state
    actions::update_dirty_ui(&ws_view, &window_rc, &save_action);
    actions::update_status_bar_path(&ws_view, &status_bar);

}

thread_local! {
    static THEME_PROVIDER: RefCell<Option<gtk4::CssProvider>> = RefCell::new(None);
}

fn load_css() {
    // Try to restore theme from last used workspace
    let theme = load_last_theme().unwrap_or(Theme::System);
    apply_theme(theme);
}

/// Load the theme from the most recently opened workspace.
fn load_last_theme() -> Option<Theme> {
    let db_path = pax_db::Database::default_path();
    let db = pax_db::Database::open(&db_path).ok()?;
    let workspaces = db.list_workspaces_limit(1).ok()?;
    let record = workspaces.first()?;
    let path = record.config_path.as_ref()?;
    let ws = pax_core::config::load_workspace(std::path::Path::new(path)).ok()?;
    Some(Theme::from_id(&ws.settings.theme))
}

fn apply_theme(theme: Theme) {
    let display = gdk::Display::default().expect("Could not connect to display");

    // Remove old provider
    THEME_PROVIDER.with(|cell| {
        if let Some(old) = cell.borrow_mut().take() {
            gtk4::style_context_remove_provider_for_display(&display, &old);
        }
    });

    // Set color scheme
    let style_manager = adw::StyleManager::default();
    style_manager.set_color_scheme(theme.color_scheme());

    // Update VTE terminal colors
    crate::theme::set_current_theme(theme);

    // Build CSS: theme overrides + base layout
    let css = format!("{}\n{}", theme.css_overrides(), crate::theme::BASE_CSS);

    let provider = gtk4::CssProvider::new();
    provider.load_from_data(&css);
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    THEME_PROVIDER.with(|cell| {
        cell.borrow_mut().replace(provider);
    });
}

fn show_shortcuts_dialog(window: &Rc<adw::ApplicationWindow>) {
    let dialog = gtk4::Window::builder()
        .title("Keyboard Shortcuts")
        .transient_for(window.as_ref())
        .modal(true)
        .default_width(450)
        .default_height(500)
        .build();

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(20);
    vbox.set_margin_end(20);

    let title = gtk4::Label::new(Some("Keyboard Shortcuts"));
    title.add_css_class("title-3");
    title.set_halign(gtk4::Align::Start);
    title.set_margin_bottom(12);
    vbox.append(&title);

    let shortcuts = [
        ("General", vec![
            ("Ctrl+S", "Save workspace"),
            ("Ctrl+O", "Open workspace"),
            ("Ctrl+Q", "Quit"),
        ]),
        ("Panels", vec![
            ("Ctrl+N", "Focus next panel"),
            ("Ctrl+P", "Focus previous panel"),
            ("Ctrl+Z", "Zoom/unzoom focused panel"),
            ("Ctrl+Shift+Z", "Collapse/expand focused panel"),
            ("Ctrl+R", "Reverse search (terminal)"),
            ("Ctrl+Arrow", "Scroll workspace"),
            ("Ctrl+Scroll", "Scroll workspace (mouse)"),
        ]),
        ("Layout", vec![
            ("Ctrl+Shift+H", "Split horizontal (below)"),
            ("Ctrl+Shift+J", "Split vertical (right)"),
            ("Ctrl+Shift+T", "New tab"),
            ("Ctrl+Shift+W", "Close panel"),
            ("Ctrl+Shift+S", "Toggle sync (alt)"),
        ]),
        ("Panel Header", vec![
            ("Double-click title", "Rename panel"),
            ("Double-click tab", "Rename tab"),
        ]),
    ];

    for (section, items) in &shortcuts {
        let section_label = gtk4::Label::new(Some(section));
        section_label.add_css_class("heading");
        section_label.set_halign(gtk4::Align::Start);
        section_label.set_margin_top(12);
        section_label.set_margin_bottom(4);
        vbox.append(&section_label);

        for (key, desc) in items {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            row.set_margin_top(2);
            row.set_margin_bottom(2);

            let key_label = gtk4::Label::new(Some(key));
            key_label.set_width_chars(22);
            key_label.set_halign(gtk4::Align::Start);
            key_label.add_css_class("monospace");
            key_label.set_opacity(0.7);

            let desc_label = gtk4::Label::new(Some(desc));
            desc_label.set_halign(gtk4::Align::Start);
            desc_label.set_hexpand(true);

            row.append(&key_label);
            row.append(&desc_label);
            vbox.append(&row);
        }
    }

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_child(Some(&vbox));

    dialog.set_child(Some(&scrolled));
    dialog.present();
}
