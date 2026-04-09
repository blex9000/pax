use adw::prelude::*;
use anyhow::Result;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use pax_core::workspace::Workspace;

use crate::actions::{self, DIRTY_INDICATOR, HEADER_WS_LABEL};
use crate::panel_host::PanelAction;
use crate::theme::Theme;
use crate::widgets::status_bar::StatusBar;
use crate::workspace_view::WorkspaceView;

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
            crate::icons::configure_icon_theme(&icon_theme);
        }

        // Register custom GtkSourceView style schemes
        #[cfg(feature = "sourceview")]
        {
            let manager = sourceview5::StyleSchemeManager::default();
            for path in sourceview_style_search_paths() {
                manager.append_search_path(&path.to_string_lossy());
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
    header.add_css_class("app-headerbar");
    header.set_show_end_title_buttons(true);
    header.set_show_start_title_buttons(true);

    // Pax icon in center of welcome header
    let title_icon = gtk4::Image::from_icon_name("pax");
    title_icon.set_pixel_size(20);
    header.set_title_widget(Some(&title_icon));

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_css_class("app-toolbar-view");
    toolbar_view.add_top_bar(&header);

    let win = window.clone();
    let welcome = crate::widgets::welcome::build_welcome(Rc::new(move |choice| {
        use crate::widgets::welcome::WelcomeChoice;
        match choice {
            WelcomeChoice::NewWorkspace => {
                let ws = new_workspace_with_preferred_theme("untitled");
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
                dialog.open(
                    Some(win2.as_ref()),
                    gtk4::gio::Cancellable::NONE,
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                if let Ok(ws) = pax_core::config::load_workspace(&path) {
                                    setup_workspace_ui(&win3, ws, Some(&path));
                                }
                            }
                        }
                    },
                );
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
    let workspace_theme = Theme::from_id(&workspace.settings.theme);
    window.set_title(Some(&format!("Pax — {}", ws_name)));

    // Apply saved theme
    apply_theme(workspace_theme);

    // Header bar with hamburger menu
    let header = adw::HeaderBar::new();
    header.add_css_class("app-headerbar");
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
    menu_btn.add_css_class("flat");
    menu_btn.add_css_class("app-menu-btn");

    let menu = gtk4::gio::Menu::new();
    let file_section = gtk4::gio::Menu::new();
    file_section.append(Some("New Workspace"), Some("app.new"));
    file_section.append(Some("Open Workspace…"), Some("app.open"));
    file_section.append(Some("Open Recent…"), Some("app.recent"));
    file_section.append(Some("Save"), Some("app.save"));
    file_section.append(Some("Save As…"), Some("app.save-as"));
    file_section.append(Some("Auto-save"), Some("app.autosave"));
    menu.append_section(None, &file_section);
    let settings_section = gtk4::gio::Menu::new();
    settings_section.append(Some("Settings…"), Some("app.settings"));
    settings_section.append(Some("Keyboard Shortcuts"), Some("app.shortcuts"));
    settings_section.append(Some("About Pax"), Some("app.about"));
    menu.append_section(None, &settings_section);
    menu.append(Some("Quit"), Some("app.quit"));
    let menu_popover = gtk4::PopoverMenu::from_model(Some(&menu));
    crate::theme::configure_popover(&menu_popover);
    menu_btn.set_popover(Some(&menu_popover));
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
            let needs_config =
                type_id == "markdown" || type_id == "browser" || type_id == "code_editor";

            if needs_config {
                // For panels that need a file/directory, show config FIRST
                let default_type = match type_id {
                    "markdown" => pax_core::workspace::PanelType::Markdown {
                        file: String::new(),
                    },
                    "browser" => pax_core::workspace::PanelType::Browser {
                        url: "https://example.com".to_string(),
                    },
                    "code_editor" => pax_core::workspace::PanelType::CodeEditor {
                        root_dir: String::new(),
                        ssh: None,
                        remote_path: None,
                        poll_interval: None,
                    },
                    _ => pax_core::workspace::PanelType::Terminal,
                };
                let pid = panel_id.to_string();
                let tid = type_id.to_string();
                let ws2 = ws.clone();
                let win2 = win.clone();
                let sa2 = sa.clone();
                let saved_ssh = {
                    let view = ws.borrow();
                    std::rc::Rc::new(std::cell::RefCell::new(
                        view.workspace().ssh_configs.clone(),
                    ))
                };
                crate::dialogs::panel_config::show_panel_config_dialog(
                    &*win,
                    &tid,
                    &default_type,
                    None,
                    None,
                    &[],
                    None,
                    0,
                    0,
                    saved_ssh,
                    move |new_name,
                          new_type,
                          new_cwd,
                          new_ssh,
                          new_cmds,
                          new_close,
                          new_mw,
                          new_mh| {
                        // apply_panel_config handles everything: sets type, creates backend, rebuilds layout
                        ws2.borrow_mut().apply_panel_config(
                            &pid, new_name, new_type, new_cwd, new_ssh, new_cmds, new_close,
                            new_mw, new_mh,
                        );
                        actions::update_dirty_ui(&ws2, &win2, &sa2);
                    },
                );
            } else {
                ws.borrow_mut().set_panel_type(panel_id, type_id);
                sb.borrow()
                    .set_message(&format!("{} → {}", panel_id, type_id));
                actions::update_dirty_ui(&ws, &win, &sa);
            }
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
                                if let Some(new_id) =
                                    ws_for_cb.borrow_mut().add_tab_to_notebook(&nb)
                                {
                                    sb_for_cb
                                        .borrow()
                                        .set_message(&format!("Tab + → {}", new_id));
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
                            PanelAction::MoveTabLeft => {
                                if ws_for_cb.borrow_mut().move_tab_by_panel_id(real_id, -1) {
                                    sb_for_cb.borrow().set_message("Tab moved left");
                                }
                            }
                            PanelAction::MoveTabRight => {
                                if ws_for_cb.borrow_mut().move_tab_by_panel_id(real_id, 1) {
                                    sb_for_cb.borrow().set_message("Tab moved right");
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
                        sb_for_cb
                            .borrow()
                            .set_message(&format!("Split H → {}", new_id));
                    }
                }
                PanelAction::SplitV => {
                    if let Some(new_id) = ws_for_cb.borrow_mut().split_focused_v() {
                        sb_for_cb
                            .borrow()
                            .set_message(&format!("Split V → {}", new_id));
                    }
                }
                PanelAction::AddTab => {
                    if let Some(new_id) = ws_for_cb.borrow_mut().add_tab_focused() {
                        sb_for_cb
                            .borrow()
                            .set_message(&format!("TabSplit → {}", new_id));
                    }
                }
                PanelAction::Reset => {
                    ws_for_cb.borrow_mut().reset_panel(panel_id);
                    sb_for_cb.borrow().set_message("Panel reset");
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
                            pcfg.map(|p| p.effective_type())
                                .unwrap_or(pax_core::workspace::PanelType::Terminal),
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
                    let win2 = win_for_cb.clone();
                    let sa2 = sa_for_cb.clone();
                    // Shared SSH configs — backed by workspace data, changes persist immediately
                    let saved_ssh: std::rc::Rc<
                        std::cell::RefCell<Vec<pax_core::workspace::NamedSshConfig>>,
                    > = {
                        let view = ws_for_cb.borrow();
                        std::rc::Rc::new(std::cell::RefCell::new(
                            view.workspace().ssh_configs.clone(),
                        ))
                    };
                    // Sync changes back to workspace whenever the Rc is modified
                    let ws_sync = ws_for_cb.clone();
                    let saved_ssh_sync = saved_ssh.clone();
                    // Poll for changes every 500ms while dialog is open
                    let last_len = std::rc::Rc::new(std::cell::Cell::new(saved_ssh.borrow().len()));
                    let sync_active = std::rc::Rc::new(std::cell::Cell::new(true));
                    let sync_flag = sync_active.clone();
                    gtk4::glib::timeout_add_local(
                        std::time::Duration::from_millis(500),
                        move || {
                            if !sync_flag.get() {
                                return gtk4::glib::ControlFlow::Break;
                            }
                            let current = saved_ssh_sync.borrow().len();
                            if current != last_len.get() {
                                last_len.set(current);
                                ws_sync.borrow_mut().workspace_mut().ssh_configs =
                                    saved_ssh_sync.borrow().clone();
                            }
                            gtk4::glib::ControlFlow::Continue
                        },
                    );
                    let sync_stop = sync_active.clone();
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
                        move |new_name,
                              new_type,
                              new_cwd,
                              new_ssh,
                              new_cmds,
                              new_close,
                              new_mw,
                              new_mh| {
                            sync_stop.set(false); // Stop polling
                            ws2.borrow_mut().apply_panel_config(
                                &pid, new_name, new_type, new_cwd, new_ssh, new_cmds, new_close,
                                new_mw, new_mh,
                            );
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
                    sb_for_cb
                        .borrow()
                        .set_message(if zoomed { "Zoom ON" } else { "Zoom OFF" });
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
                            sb_for_cb
                                .borrow()
                                .set_message(&format!("Sync ON: {} ({} panels)", pid, count));
                        } else {
                            sb_for_cb
                                .borrow()
                                .set_message(&format!("Sync OFF: {} ({} panels)", pid, count));
                        }
                    }
                }
                PanelAction::Rename(new_name) => {
                    let mut view = ws_for_cb.borrow_mut();
                    view.rename_panel(panel_id, &new_name);
                    drop(view);
                    sb_for_cb
                        .borrow()
                        .set_message(&format!("Renamed: {}", panel_id));
                }
                PanelAction::RenameTab(new_name) => {
                    // Only update the tab label in the layout tree, not the panel name.
                    // panel_id here is the first child panel — used to locate the tab.
                    let mut view = ws_for_cb.borrow_mut();
                    tracing::debug!(
                        "RenameTab: panel_id='{}', new_name='{}'",
                        panel_id,
                        new_name
                    );
                    view.rename_tab_label(panel_id, &new_name);
                    crate::layout_ops::debug_layout_tree(&view.workspace().layout, "AFTER_RENAME");
                    drop(view);
                    actions::update_dirty_ui(&ws_for_cb, &win_for_cb, &sa_for_cb);
                    sb_for_cb
                        .borrow()
                        .set_message(&format!("Tab renamed: {}", new_name));
                }
                PanelAction::Collapse => {
                    let view = ws_for_cb.borrow();
                    if let Some(host) = view.hosts().get(panel_id) {
                        host.expand_collapsed();
                    }
                }
                PanelAction::Focus => {
                    let idx = ws_for_cb.borrow().focus_order_index(panel_id);
                    if let Some(idx) = idx {
                        ws_for_cb.borrow_mut().set_focus_index(idx);
                    }
                }
                PanelAction::AddTabToNotebook
                | PanelAction::RemoveTab
                | PanelAction::MoveTabLeft
                | PanelAction::MoveTabRight => {}
            }
            actions::update_dirty_ui(&ws_for_cb, &win_for_cb, &sa_for_cb);
        });
        ws_view.borrow_mut().set_action_callback(cb);
    }

    // Setup sync input propagation: when a synced terminal gets local input,
    // forward it to all other synced terminals.
    {
        let ws = ws_view.clone();
        let sync_cb: Rc<dyn Fn(&str, &[u8])> = Rc::new(move |source_panel_id, data| {
            if let Ok(view) = ws.try_borrow() {
                if view.is_panel_synced(source_panel_id) {
                    view.write_to_synced(data, source_panel_id);
                }
            }
        });
        ws_view.borrow_mut().setup_sync_callbacks(sync_cb);
    }

    // Auto-save flag (disabled by default, toggled via menu)
    let autosave_enabled = std::rc::Rc::new(std::cell::Cell::new(false));
    let bypass_close_prompt = std::rc::Rc::new(std::cell::Cell::new(false));

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
            let ws2 = ws.clone();
            let sb2 = sb.clone();
            let win2 = win.clone();
            let sa2 = sa.clone();
            let on_continue: Rc<dyn Fn()> = Rc::new(move || {
                let empty = new_workspace_with_preferred_theme("untitled");
                let empty_theme = Theme::from_id(&empty.settings.theme);
                if let Err(e) = ws2.borrow_mut().load_workspace(empty, None) {
                    sb2.borrow().set_message(&format!("Error: {}", e));
                }
                apply_theme(empty_theme);
                actions::update_dirty_ui(&ws2, &win2, &sa2);
                actions::update_status_bar_path(&ws2, &sb2);
            });
            actions::confirm_discard_workspace_changes(&ws, &sb, &win, &sa, on_continue);
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
                save_preferred_theme(new_settings.theme);
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
        let sb = status_bar.clone();
        let sa = save_action.clone();
        let win = window_rc.clone();
        let bypass = bypass_close_prompt.clone();
        action.connect_activate(move |_, _| {
            let win2 = win.clone();
            let bypass2 = bypass.clone();
            let on_continue: Rc<dyn Fn()> = Rc::new(move || {
                bypass2.set(true);
                win2.close();
            });
            actions::confirm_discard_workspace_changes(&ws, &sb, &win, &sa, on_continue);
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
    {
        let action = gtk4::gio::SimpleAction::new("about", None);
        let win = window_rc.clone();
        action.connect_activate(move |_, _| {
            show_about_dialog(&win);
        });
        action_group.add_action(&action);
    }
    {
        let action = gtk4::gio::SimpleAction::new_stateful("autosave", None, &false.to_variant());
        let enabled = autosave_enabled.clone();
        let sb = status_bar.clone();
        action.connect_activate(move |action, _| {
            let current = enabled.get();
            let new_val = !current;
            enabled.set(new_val);
            action.set_state(&new_val.to_variant());
            sb.borrow().set_message(if new_val {
                "Auto-save enabled"
            } else {
                "Auto-save disabled"
            });
        });
        action_group.add_action(&action);
    }

    window.insert_action_group("app", Some(&action_group));

    // Window close request
    {
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let sa = save_action.clone();
        let win = window_rc.clone();
        let bypass = bypass_close_prompt.clone();
        window.connect_close_request(move |_| {
            if bypass.get() {
                ws.borrow().run_all_before_close();
                return glib::Propagation::Proceed;
            }
            if !ws.borrow().is_dirty() {
                ws.borrow().run_all_before_close();
                return glib::Propagation::Proceed;
            }
            let win2 = win.clone();
            let bypass2 = bypass.clone();
            let on_continue: Rc<dyn Fn()> = Rc::new(move || {
                bypass2.set(true);
                win2.close();
            });
            actions::confirm_discard_workspace_changes(&ws, &sb, &win, &sa, on_continue);
            glib::Propagation::Stop
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
    toolbar_view.add_css_class("app-toolbar-view");
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content_box));
    window.set_content(Some(&toolbar_view));
    apply_theme(workspace_theme);

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
                                sb.borrow().set_message(&format!(
                                    "Sync ON: {} ({} panels)",
                                    panel_id, count
                                ));
                            } else {
                                sb.borrow().set_message(&format!(
                                    "Sync OFF: {} ({} panels)",
                                    panel_id, count
                                ));
                            }
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::Z => {
                        let view = ws.borrow();
                        if let Some(id) = view.focused_panel_id() {
                            if let Some(host) = view.hosts().get(id) {
                                host.expand_collapsed();
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
                        // If focused panel is a code editor, let Ctrl+P propagate
                        // to the editor's fuzzy finder
                        let is_code_editor = {
                            let view = ws.borrow();
                            view.focused_panel_id()
                                .and_then(|id| view.workspace().panel(id))
                                .map(|p| {
                                    matches!(
                                        p.effective_type(),
                                        pax_core::workspace::PanelType::CodeEditor { .. }
                                    )
                                })
                                .unwrap_or(false)
                        };
                        if is_code_editor {
                            return glib::Propagation::Proceed;
                        }
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
                                .map(|p| {
                                    matches!(
                                        p.effective_type(),
                                        pax_core::workspace::PanelType::CodeEditor { .. }
                                    )
                                })
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

    // Auto-save workspace every 30s if enabled, dirty, and has a config path
    {
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        let enabled = autosave_enabled.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(30), move || {
            if !enabled.get() {
                return glib::ControlFlow::Continue;
            }
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
            gtk4::EventControllerScrollFlags::VERTICAL
                | gtk4::EventControllerScrollFlags::HORIZONTAL,
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
    // Startup chrome should be deterministic. The welcome page always starts
    // from the preferred app theme if present, otherwise Nord.
    apply_theme(load_preferred_theme());
}

fn new_workspace_with_preferred_theme(name: &str) -> Workspace {
    workspace_with_theme(name, load_preferred_theme())
}

fn workspace_with_theme(name: &str, theme: Theme) -> Workspace {
    let mut workspace = pax_core::template::empty_workspace(name);
    workspace.settings.theme = theme.to_id().to_string();
    workspace
}

fn load_preferred_theme() -> Theme {
    let db_path = pax_db::Database::default_path();
    let Ok(db) = pax_db::Database::open(&db_path) else {
        return Theme::default();
    };
    load_preferred_theme_from_db(&db).unwrap_or_default()
}

fn load_preferred_theme_from_db(db: &pax_db::Database) -> Option<Theme> {
    db.get_app_preference("theme")
        .ok()
        .flatten()
        .map(|value| Theme::from_id(&value))
}

fn save_preferred_theme(theme: Theme) {
    let db_path = pax_db::Database::default_path();
    let Ok(db) = pax_db::Database::open(&db_path) else {
        return;
    };
    let _ = db.set_app_preference("theme", theme.to_id());
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

    for widget in gtk4::Window::list_toplevels() {
        widget.queue_draw();
    }
}

#[cfg(test)]
mod tests {
    use super::{load_preferred_theme_from_db, workspace_with_theme, Theme};
    use pax_db::Database;

    #[test]
    fn startup_theme_uses_default_theme() {
        assert_eq!(Theme::default(), Theme::Nord);
    }

    #[test]
    fn startup_theme_uses_saved_preference_from_db() {
        let db = Database::open_memory().unwrap();
        db.set_app_preference("theme", "dracula").unwrap();

        assert_eq!(load_preferred_theme_from_db(&db), Some(Theme::Dracula));
    }

    #[test]
    fn new_workspace_inherits_selected_theme() {
        let workspace = workspace_with_theme("untitled", Theme::Dracula);

        assert_eq!(workspace.settings.theme, "dracula");
    }
}

#[cfg(feature = "sourceview")]
fn sourceview_style_search_paths() -> Vec<std::path::PathBuf> {
    let exe = std::env::current_exe().ok();
    let candidates = [
        std::path::PathBuf::from("resources/sourceview-styles"),
        exe.as_ref()
            .and_then(|p| p.parent().map(|d| d.join("../Resources/sourceview-styles")))
            .unwrap_or_default(),
        exe.as_ref()
            .and_then(|p| p.parent().map(|d| d.join("../resources/sourceview-styles")))
            .unwrap_or_default(),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../resources/sourceview-styles"),
    ];

    let mut seen = std::collections::HashSet::new();
    let mut paths = Vec::new();
    for path in candidates {
        if path.exists() && seen.insert(path.clone()) {
            paths.push(path);
        }
    }
    paths
}

fn show_about_dialog(window: &Rc<adw::ApplicationWindow>) {
    let about = adw::AboutWindow::builder()
        .application_name("Pax")
        .application_icon("pax")
        .version(pax_core::build_info::VERSION_STRING)
        .developer_name("Pax Contributors")
        .comments("Terminal workspace manager")
        .transient_for(window.as_ref())
        .modal(true)
        .build();
    about.present();
}

fn show_shortcuts_dialog(window: &Rc<adw::ApplicationWindow>) {
    let dialog = gtk4::Window::builder()
        .title("Keyboard Shortcuts")
        .transient_for(window.as_ref())
        .modal(true)
        .default_width(450)
        .default_height(500)
        .build();
    crate::theme::configure_dialog_window(&dialog);

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
        (
            "General",
            vec![
                ("Ctrl+S", "Save workspace"),
                ("Ctrl+O", "Open workspace"),
                ("Ctrl+Q", "Quit"),
            ],
        ),
        (
            "Panels",
            vec![
                ("Ctrl+N", "Focus next panel"),
                ("Ctrl+P", "Focus previous panel"),
                ("Ctrl+Z", "Zoom/unzoom focused panel"),
                ("Ctrl+Shift+Z", "Collapse/expand focused panel"),
                ("Ctrl+R", "Reverse search (terminal)"),
                ("Ctrl+Arrow", "Scroll workspace"),
                ("Ctrl+Scroll", "Scroll workspace (mouse)"),
            ],
        ),
        (
            "Code Editor",
            vec![
                ("Ctrl+S", "Save current file"),
                ("Ctrl+W", "Close current tab"),
                ("Ctrl+Tab", "Next tab"),
                ("Ctrl+P", "Fuzzy file finder"),
                ("Ctrl+E", "Recent files"),
                ("Ctrl+F", "Search in file"),
                ("Ctrl+H", "Search & replace"),
                ("Ctrl+Shift+F", "Search in project"),
                ("Ctrl+Shift+G", "Git changes"),
                ("Ctrl+B", "Toggle sidebar"),
                ("Alt+Left", "Go back (navigation)"),
                ("Alt+Right", "Go forward (navigation)"),
            ],
        ),
        (
            "Layout",
            vec![
                ("Ctrl+Shift+H", "Split horizontal (below)"),
                ("Ctrl+Shift+J", "Split vertical (right)"),
                ("Ctrl+Shift+T", "New tab"),
                ("Ctrl+Shift+W", "Close panel"),
                ("Ctrl+Shift+S", "Toggle sync (alt)"),
            ],
        ),
        (
            "Panel Header",
            vec![
                ("Double-click title", "Rename panel"),
                ("Double-click tab", "Rename tab"),
            ],
        ),
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
