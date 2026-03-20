use anyhow::Result;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::gdk;
use libadwaita as adw;
use adw::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use tp_core::workspace::Workspace;

use crate::panel_host::PanelAction;
use crate::workspace_view::WorkspaceView;
use crate::widgets::status_bar::StatusBar;

/// Main application entry point.
pub fn run_app(workspace: Workspace, config_path: Option<&Path>) -> Result<()> {
    let app = adw::Application::builder()
        .application_id("com.sinelec.myterms")
        .build();

    let ws_name = workspace.name.clone();
    let config_path_owned = config_path.map(|p| p.to_path_buf());

    app.connect_activate(move |app| {
        load_css();

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title(&format!("MyTerms — {}", ws_name))
            .default_width(1200)
            .default_height(800)
            .build();

        // Header bar with hamburger menu
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(true);
        header.set_show_start_title_buttons(true);

        // Hamburger menu
        let menu_btn = gtk4::MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_tooltip_text(Some("Menu"));

        let menu = gtk4::gio::Menu::new();
        let file_section = gtk4::gio::Menu::new();
        file_section.append(Some("New Workspace"), Some("app.new"));
        file_section.append(Some("Open Workspace…"), Some("app.open"));
        file_section.append(Some("Open Recent…"), Some("app.recent"));
        file_section.append(Some("Save Workspace"), Some("app.save"));
        file_section.append(Some("Save Workspace As…"), Some("app.save-as"));
        menu.append_section(None, &file_section);
        let ws_section = gtk4::gio::Menu::new();
        ws_section.append(Some("Rename Workspace…"), Some("app.rename"));
        menu.append_section(None, &ws_section);
        menu.append(Some("Quit"), Some("app.quit"));
        menu_btn.set_menu_model(Some(&menu));

        header.pack_start(&menu_btn);

        // Shared state
        let ws_view = Rc::new(RefCell::new(WorkspaceView::build(
            &workspace,
            config_path_owned.as_deref(),
        )));
        let status_bar = Rc::new(RefCell::new(StatusBar::new(&workspace.name)));
        let window_rc = Rc::new(window.clone());

        // Create save action early so callbacks can reference it
        let save_action = gtk4::gio::SimpleAction::new("save", None);

        // Wire up type chooser callback
        {
            let ws_for_chooser = ws_view.clone();
            let sb_for_chooser = status_bar.clone();
            let win_for_chooser = window_rc.clone();
            let sa_for_chooser = save_action.clone();
            let cb: crate::panels::chooser::OnTypeChosen = Rc::new(move |panel_id, type_id| {
                ws_for_chooser.borrow_mut().set_panel_type(panel_id, type_id);
                sb_for_chooser.borrow().set_message(&format!("{} → {}", panel_id, type_id));
                update_dirty_ui(&ws_for_chooser, &win_for_chooser, &sa_for_chooser);
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
                // "nb:<panel_id>" means action on notebook containing panel_id
                if let Some(real_id) = panel_id.strip_prefix("nb:") {
                    let view = ws_for_cb.borrow();
                    if let Some(host) = view.host(real_id) {
                        let widget = host.widget().clone();
                        if let Some(nb) = crate::workspace_view::find_notebook_ancestor(&widget) {
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
                        let (pname, ptype) = {
                            let view = ws_for_cb.borrow();
                            (
                                view.panel_name(panel_id).unwrap_or_default(),
                                view.panel_type(panel_id).unwrap_or(tp_core::workspace::PanelType::Terminal),
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
                            move |new_name, new_type| {
                                ws2.borrow_mut().apply_panel_config(&pid, new_name, new_type);
                                update_dirty_ui(&ws2, &win2, &sa2);
                            },
                        );
                    }
                    PanelAction::AddTabToNotebook | PanelAction::RemoveTab => {}
                }
                update_dirty_ui(&ws_for_cb, &win_for_cb, &sa_for_cb);
            });
            ws_view.borrow_mut().set_action_callback(cb);
        }

        // Register GIO actions for menu items
        let action_group = gtk4::gio::SimpleActionGroup::new();

        let save_as_action = gtk4::gio::SimpleAction::new("save-as", None);

        // Open action
        {
            let action = gtk4::gio::SimpleAction::new("open", None);
            let ws = ws_view.clone();
            let win = window_rc.clone();
            let sb = status_bar.clone();
            let sa = save_action.clone();
            action.connect_activate(move |_, _| {
                do_open(&ws, &sb, &win, &sa);
            });
            action_group.add_action(&action);
        }

        // Save action
        {
            let ws = ws_view.clone();
            let sb = status_bar.clone();
            let win = window_rc.clone();
            let sa = save_action.clone();
            save_action.connect_activate(move |_, _| {
                do_save(&ws, &sb, &win, &sa, false);
            });
            action_group.add_action(&save_action);
        }

        // Save As action
        {
            let ws = ws_view.clone();
            let sb = status_bar.clone();
            let win = window_rc.clone();
            let sa = save_action.clone();
            save_as_action.connect_activate(move |_, _| {
                do_save(&ws, &sb, &win, &sa, true);
            });
            action_group.add_action(&save_as_action);
        }

        // New workspace action
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
                update_dirty_ui(&ws, &win, &sa);
            });
            action_group.add_action(&action);
        }

        // Open recent action
        {
            let action = gtk4::gio::SimpleAction::new("recent", None);
            let ws = ws_view.clone();
            let win = window_rc.clone();
            let sa = save_action.clone();
            let sb = status_bar.clone();
            action.connect_activate(move |_, _| {
                show_recent_dialog(&ws, &sb, &win, &sa);
            });
            action_group.add_action(&action);
        }

        // Rename workspace action
        {
            let action = gtk4::gio::SimpleAction::new("rename", None);
            let ws = ws_view.clone();
            let win = window_rc.clone();
            let sa = save_action.clone();
            action.connect_activate(move |_, _| {
                let current_name = ws.borrow().workspace_name().to_string();
                show_rename_dialog(&win, &current_name, {
                    let ws = ws.clone();
                    let win = win.clone();
                    let sa = sa.clone();
                    move |new_name| {
                        ws.borrow_mut().rename_workspace(&new_name);
                        update_dirty_ui(&ws, &win, &sa);
                    }
                });
            });
            action_group.add_action(&action);
        }

        // Quit action
        {
            let action = gtk4::gio::SimpleAction::new("quit", None);
            action.connect_activate(move |_, _| {
                std::process::exit(0);
            });
            action_group.add_action(&action);
        }

        window.insert_action_group("app", Some(&action_group));

        // Initial dirty state
        update_dirty_ui(&ws_view, &window_rc, &save_action);

        // Content area
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let ws_widget = ws_view.borrow().widget().clone();
        ws_widget.set_vexpand(true);
        ws_widget.set_hexpand(true);
        content_box.append(&ws_widget);
        content_box.append(status_bar.borrow().widget());

        // ToolbarView
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content_box));
        window.set_content(Some(&toolbar_view));

        // Keyboard shortcuts — CAPTURE phase to intercept before VTE
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
                            return glib::Propagation::Stop;
                        }
                        gdk::Key::J => {
                            if let Some(new_id) = ws.borrow_mut().split_focused_v() {
                                sb.borrow().set_message(&format!("Split V → {}", new_id));
                            }
                            return glib::Propagation::Stop;
                        }
                        gdk::Key::T => {
                            if let Some(new_id) = ws.borrow_mut().add_tab_focused() {
                                sb.borrow().set_message(&format!("Tab → {}", new_id));
                            }
                            return glib::Propagation::Stop;
                        }
                        gdk::Key::W => {
                            if ws.borrow_mut().close_focused() {
                                if let Some(id) = ws.borrow().focused_panel_id() {
                                    sb.borrow().set_panel(id);
                                }
                                sb.borrow().set_message("Panel closed");
                            }
                            return glib::Propagation::Stop;
                        }
                        _ => {}
                    }
                }

                if ctrl && !shift {
                    match key {
                        gdk::Key::q => std::process::exit(0),
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
                            do_save(&ws, &sb, &win, &sa, false);
                            return glib::Propagation::Stop;
                        }
                        gdk::Key::o => {
                            do_open(&ws, &sb, &win, &sa);
                            return glib::Propagation::Stop;
                        }
                        _ => {}
                    }
                }

                glib::Propagation::Proceed
            });
        }

        window.add_controller(controller);
        window.present();
    });

    app.run_with_args::<String>(&[]);
    Ok(())
}

/// Update window title and save action sensitivity based on dirty state.
fn show_recent_dialog(
    ws: &Rc<RefCell<WorkspaceView>>,
    sb: &Rc<RefCell<StatusBar>>,
    window: &Rc<adw::ApplicationWindow>,
    save_action: &gtk4::gio::SimpleAction,
) {
    let db_path = tp_db::Database::default_path();
    let workspaces = match tp_db::Database::open(&db_path) {
        Ok(db) => db.list_workspaces_limit(20).unwrap_or_default(),
        Err(_) => vec![],
    };

    if workspaces.is_empty() {
        sb.borrow().set_message("No recent workspaces");
        return;
    }

    let dialog = gtk4::Window::builder()
        .title("Recent Workspaces")
        .transient_for(window.as_ref())
        .modal(true)
        .default_width(550)
        .default_height(400)
        .build();

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    // Header
    let header_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    header_box.set_margin_top(12);
    header_box.set_margin_bottom(8);
    header_box.set_margin_start(16);
    header_box.set_margin_end(16);
    let title = gtk4::Label::new(Some("Recent Workspaces"));
    title.add_css_class("title-3");
    title.set_hexpand(true);
    title.set_halign(gtk4::Align::Start);
    header_box.append(&title);
    vbox.append(&header_box);

    // List in scrolled window
    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::None);
    list_box.add_css_class("boxed-list");
    list_box.set_margin_start(16);
    list_box.set_margin_end(16);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_child(Some(&list_box));
    scrolled.set_vexpand(true);
    vbox.append(&scrolled);

    for record in &workspaces {
        let row = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(12);
        row.set_margin_end(12);

        // Row 1: name
        let name_label = gtk4::Label::new(Some(&record.name));
        name_label.add_css_class("heading");
        name_label.set_halign(gtk4::Align::Start);
        row.append(&name_label);

        // Row 2: filepath (truncated, full on hover)
        let path_text = record.config_path.as_deref().unwrap_or("(no file)");
        let path_label = gtk4::Label::new(Some(path_text));
        path_label.add_css_class("dim-label");
        path_label.set_halign(gtk4::Align::Start);
        path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
        path_label.set_tooltip_text(Some(path_text));
        row.append(&path_label);

        // Row 3: stats + open button
        let bottom_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let stats = format!("Opened {} times • {}", record.open_count, record.last_opened);
        let stats_label = gtk4::Label::new(Some(&stats));
        stats_label.add_css_class("dim-label");
        stats_label.add_css_class("caption");
        stats_label.set_halign(gtk4::Align::Start);
        stats_label.set_hexpand(true);
        bottom_row.append(&stats_label);

        let open_btn = gtk4::Button::new();
        open_btn.set_icon_name("document-open-symbolic");
        open_btn.add_css_class("flat");
        open_btn.set_tooltip_text(Some("Open this workspace"));

        if let Some(ref path) = record.config_path {
            let path = std::path::PathBuf::from(path);
            let ws_clone = ws.clone();
            let sb_clone = sb.clone();
            let win_clone = window.clone();
            let sa_clone = save_action.clone();
            let d = dialog.clone();
            let path_exists = path.exists();

            if path_exists {
                open_btn.connect_clicked(move |_| {
                    match ws_clone.borrow_mut().load_from_file(&path) {
                        Ok(()) => {
                            sb_clone.borrow().set_message(&format!("Opened: {}", path.display()));
                        }
                        Err(e) => {
                            sb_clone.borrow().set_message(&format!("Error: {}", e));
                        }
                    }
                    update_dirty_ui(&ws_clone, &win_clone, &sa_clone);
                    d.close();
                });
            } else {
                open_btn.set_sensitive(false);
                open_btn.set_tooltip_text(Some("File not found"));
            }
        } else {
            open_btn.set_sensitive(false);
        }

        bottom_row.append(&open_btn);
        row.append(&bottom_row);

        let list_row = gtk4::ListBoxRow::new();
        list_row.set_child(Some(&row));
        list_box.append(&list_row);
    }

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_rename_dialog(
    window: &Rc<adw::ApplicationWindow>,
    current_name: &str,
    on_done: impl Fn(String) + 'static,
) {
    let dialog = gtk4::Window::builder()
        .title("Rename Workspace")
        .transient_for(window.as_ref())
        .modal(true)
        .default_width(350)
        .build();

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let label = gtk4::Label::new(Some("Workspace name:"));
    label.set_halign(gtk4::Align::Start);
    vbox.append(&label);

    let entry = gtk4::Entry::new();
    entry.set_text(current_name);
    entry.set_activates_default(true);
    vbox.append(&entry);

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(12);

    let cancel = gtk4::Button::with_label("Cancel");
    cancel.add_css_class("flat");
    let apply = gtk4::Button::with_label("Rename");
    apply.add_css_class("suggested-action");

    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());

    let d = dialog.clone();
    let e = entry.clone();
    apply.connect_clicked(move |_| {
        let name = e.text().to_string();
        if !name.is_empty() {
            on_done(name);
        }
        d.close();
    });

    btn_box.append(&cancel);
    btn_box.append(&apply);
    vbox.append(&btn_box);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn update_dirty_ui(
    ws: &Rc<RefCell<WorkspaceView>>,
    window: &Rc<adw::ApplicationWindow>,
    save_action: &gtk4::gio::SimpleAction,
) {
    let view = ws.borrow();
    let dirty = view.is_dirty();
    let name = &view.workspace().name;
    let title = if dirty {
        format!("● MyTerms — {}", name)
    } else {
        format!("MyTerms — {}", name)
    };
    window.set_title(Some(&title));

    // Disable save if not dirty and already has a file
    save_action.set_enabled(dirty || !view.has_config_path());
}

/// Save workspace. If force_dialog or no path set, open "Save As" dialog.
fn do_save(
    ws: &Rc<RefCell<WorkspaceView>>,
    sb: &Rc<RefCell<StatusBar>>,
    window: &Rc<adw::ApplicationWindow>,
    save_action: &gtk4::gio::SimpleAction,
    force_dialog: bool,
) {
    if !force_dialog {
        let has_path = ws.borrow().has_config_path();
        if has_path {
            let save_result = ws.borrow_mut().save();
            match save_result {
                Ok(path) => {
                    sb.borrow().set_message(&format!("Saved: {}", path.display()));
                    update_dirty_ui(ws, window, save_action);
                    return;
                }
                Err(e) => {
                    sb.borrow().set_message(&format!("Save error: {}", e));
                    return;
                }
            }
        }
        // No path — fall through to Save As
    }

    let dialog = gtk4::FileDialog::builder()
        .title("Save Workspace")
        .modal(true)
        .initial_name("workspace.json")
        .build();

    let filter = gtk4::FileFilter::new();
    filter.set_name(Some("JSON files"));
    filter.add_pattern("*.json");
    let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));

    let ws = ws.clone();
    let sb = sb.clone();
    let win = window.clone();
    let sa = save_action.clone();
    dialog.save(
        Some(window.as_ref()),
        gtk4::gio::Cancellable::NONE,
        move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    let save_result = ws.borrow_mut().save_as(&path);
                    match save_result {
                        Ok(()) => {
                            sb.borrow().set_message(&format!("Saved: {}", path.display()));
                            update_dirty_ui(&ws, &win, &sa);
                        }
                        Err(e) => sb.borrow().set_message(&format!("Save error: {}", e)),
                    }
                }
            }
        },
    );
}

/// Open workspace — file dialog, then load in current window.
fn do_open(
    ws: &Rc<RefCell<WorkspaceView>>,
    sb: &Rc<RefCell<StatusBar>>,
    window: &Rc<adw::ApplicationWindow>,
    save_action: &gtk4::gio::SimpleAction,
) {
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

    let ws = ws.clone();
    let sb = sb.clone();
    let win = window.clone();
    let sa = save_action.clone();
    dialog.open(
        Some(window.as_ref()),
        gtk4::gio::Cancellable::NONE,
        move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    match ws.borrow_mut().load_from_file(&path) {
                        Ok(()) => {
                            sb.borrow().set_message(&format!("Opened: {}", path.display()));
                            // Need to drop borrow before update_dirty_ui
                        }
                        Err(e) => {
                            sb.borrow().set_message(&format!("Open error: {}", e));
                            return;
                        }
                    }
                    update_dirty_ui(&ws, &win, &sa);
                }
            }
        },
    );
}

fn load_css() {
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(CSS);
    gtk4::style_context_add_provider_for_display(
        &gdk::Display::default().expect("Could not connect to display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

const CSS: &str = r#"
.panel-frame {
    border: none;
    border-radius: 0;
    margin: 0;
}
.panel-focused {
    border: none;
}
.panel-unfocused {
    border: none;
}
.panel-title-bar {
    padding: 2px 6px;
    min-height: 20px;
    border-bottom: 1px solid @borders;
}
.panel-title {
    font-size: 11px;
    font-weight: bold;
}
.panel-menu-btn {
    min-height: 16px;
    min-width: 16px;
    padding: 2px;
}
.panel-type-btn {
    min-width: 120px;
}
.alert-red { border-color: red; border-width: 2px; }
.alert-yellow { border-color: #e5a50a; border-width: 2px; }
.alert-green { border-color: green; border-width: 2px; }
.status-bar {
    background-color: alpha(@headerbar_bg_color, 0.9);
    padding: 2px 8px;
    min-height: 22px;
}
.status-mode { font-weight: bold; padding: 0 6px; }
.markdown-panel { font-family: sans-serif; font-size: 12px; }
.tab-close-btn { min-height: 14px; min-width: 14px; padding: 1px; }
"#;
