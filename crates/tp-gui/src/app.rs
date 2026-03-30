use anyhow::Result;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::gdk;
use libadwaita as adw;
use adw::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use tp_core::workspace::Workspace;

use crate::actions::{self, DIRTY_INDICATOR};
use crate::layout_ops::update_tab_label_in_layout;
use crate::panel_host::PanelAction;
use crate::theme::Theme;
use crate::workspace_view::WorkspaceView;
use crate::widgets::status_bar::StatusBar;

/// Single entry point — shows welcome if no workspace, or workspace directly.
pub fn run_app(workspace: Option<Workspace>, config_path: Option<&Path>) -> Result<()> {
    let app = adw::Application::builder()
        .application_id("com.sinelec.myterms")
        .build();

    let ws = workspace;
    let cp = config_path.map(|p| p.to_path_buf());

    app.connect_activate(move |app| {
        load_css();

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("MyTerms")
            .default_width(1200)
            .default_height(800)
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

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);

    let win = window.clone();
    let welcome = crate::widgets::welcome::build_welcome(Rc::new(move |choice| {
        use crate::widgets::welcome::WelcomeChoice;
        match choice {
            WelcomeChoice::NewWorkspace => {
                let ws = tp_core::template::empty_workspace("untitled");
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
                            if let Ok(ws) = tp_core::config::load_workspace(&path) {
                                setup_workspace_ui(&win3, ws, Some(&path));
                            }
                        }
                    }
                });
            }
            WelcomeChoice::OpenRecent(path) => {
                let p = std::path::PathBuf::from(&path);
                if let Ok(ws) = tp_core::config::load_workspace(&p) {
                    setup_workspace_ui(&win, ws, Some(&p));
                }
            }
        }
    }));

    toolbar_view.set_content(Some(&welcome));
    window.set_content(Some(&toolbar_view));
    window.set_default_size(700, 550);
}

/// Setup the full workspace UI in the window (replaces any existing content).
fn setup_workspace_ui(
    window: &Rc<adw::ApplicationWindow>,
    workspace: Workspace,
    config_path: Option<&Path>,
) {
    let ws_name = workspace.name.clone();
    window.set_title(Some(&format!("MyTerms — {}", ws_name)));
    window.set_default_size(1200, 800);

    // Apply saved theme
    apply_theme(Theme::from_id(&workspace.settings.theme));

    // Header bar with hamburger menu
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(true);
    header.set_show_start_title_buttons(true);

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
                        (
                            view.panel_name(panel_id).unwrap_or_default(),
                            view.panel_type(panel_id).unwrap_or(tp_core::workspace::PanelType::Terminal),
                            view.panel_cwd(panel_id),
                            view.panel_ssh(panel_id),
                            view.panel_startup_commands(panel_id),
                            view.panel_before_close(panel_id),
                            view.panel_min_width(panel_id),
                            view.panel_min_height(panel_id),
                        )
                    };
                    let pid = panel_id.to_string();
                    let ws2 = ws_for_cb.clone();
                    let win2 = win_for_cb.clone();
                    let sa2 = sa_for_cb.clone();
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
                        move |new_name, new_type, new_cwd, new_ssh, new_cmds, new_close, new_mw, new_mh| {
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
                PanelAction::AddTabToNotebook | PanelAction::RemoveTab => {}
            }
            actions::update_dirty_ui(&ws_for_cb, &win_for_cb, &sa_for_cb);
        });
        ws_view.borrow_mut().set_action_callback(cb);
    }

    // Setup sync input propagation: when a synced terminal gets input,
    // forward it to all other synced terminals
    {
        let ws = ws_view.clone();
        let sync_cb: Rc<dyn Fn(&str, &str)> = Rc::new(move |source_panel_id, text| {
            let view = ws.borrow();
            if view.is_panel_synced(source_panel_id) {
                view.write_to_synced(text.as_bytes(), source_panel_id);
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
            let empty = tp_core::template::empty_workspace("untitled");
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
                        if let Some((panel_id, is_synced)) = ws.borrow_mut().toggle_sync_focused() {
                            let count = ws.borrow().sync_count();
                            if is_synced {
                                sb.borrow().set_message(&format!("Sync ON: {} ({} panels)", panel_id, count));
                            } else {
                                sb.borrow().set_message(&format!("Sync OFF: {} ({} panels)", panel_id, count));
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
                        actions::do_save(&ws, &sb, &win, &sa, false);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::o => {
                        actions::do_open(&ws, &sb, &win, &sa);
                        return glib::Propagation::Stop;
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
                    _ => {}
                }
            }

            glib::Propagation::Proceed
        });
    }

    window.add_controller(controller);

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
    let db_path = tp_db::Database::default_path();
    let db = tp_db::Database::open(&db_path).ok()?;
    let workspaces = db.list_workspaces_limit(1).ok()?;
    let record = workspaces.first()?;
    let path = record.config_path.as_ref()?;
    let ws = tp_core::config::load_workspace(std::path::Path::new(path)).ok()?;
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
