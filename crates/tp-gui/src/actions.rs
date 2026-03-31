use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

use crate::workspace_view::WorkspaceView;
use crate::widgets::status_bar::StatusBar;

thread_local! {
    pub static DIRTY_INDICATOR: RefCell<Option<(gtk4::Image, gtk4::Separator)>> = RefCell::new(None);
}

pub fn update_dirty_ui(
    ws: &Rc<RefCell<WorkspaceView>>,
    window: &Rc<adw::ApplicationWindow>,
    save_action: &gtk4::gio::SimpleAction,
) {
    let view = ws.borrow();
    let dirty = view.is_dirty();
    let name = &view.workspace().name;
    window.set_title(Some(&format!("Pax — {}", name)));
    save_action.set_enabled(dirty || !view.has_config_path());
    DIRTY_INDICATOR.with(|cell| {
        if let Some((ref icon, ref sep)) = *cell.borrow() {
            icon.set_visible(dirty);
            sep.set_visible(dirty);
        }
    });
}

pub fn update_status_bar_path(ws: &Rc<RefCell<WorkspaceView>>, sb: &Rc<RefCell<StatusBar>>) {
    let view = ws.borrow();
    if let Some(path) = view.config_path_str() {
        sb.borrow().set_path(&path);
    } else {
        sb.borrow().set_path("(unsaved)");
    }
}

pub fn do_save(
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
                    update_status_bar_path(ws, sb);
                    return;
                }
                Err(e) => {
                    sb.borrow().set_message(&format!("Save error: {}", e));
                    return;
                }
            }
        }
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
                            update_status_bar_path(&ws, &sb);
                        }
                        Err(e) => sb.borrow().set_message(&format!("Save error: {}", e)),
                    }
                }
            }
        },
    );
}

pub fn do_open(
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
                        }
                        Err(e) => {
                            sb.borrow().set_message(&format!("Open error: {}", e));
                            return;
                        }
                    }
                    update_dirty_ui(&ws, &win, &sa);
                    update_status_bar_path(&ws, &sb);
                }
            }
        },
    );
}

pub fn show_recent_dialog(
    ws: &Rc<RefCell<WorkspaceView>>,
    sb: &Rc<RefCell<StatusBar>>,
    window: &Rc<adw::ApplicationWindow>,
    save_action: &gtk4::gio::SimpleAction,
) {
    let db_path = pax_db::Database::default_path();
    let workspaces = match pax_db::Database::open(&db_path) {
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

        let name_label = gtk4::Label::new(Some(&record.name));
        name_label.add_css_class("heading");
        name_label.set_halign(gtk4::Align::Start);
        row.append(&name_label);

        let path_text = record.config_path.as_deref().unwrap_or("(no file)");
        let path_label = gtk4::Label::new(Some(path_text));
        path_label.add_css_class("dim-label");
        path_label.set_halign(gtk4::Align::Start);
        path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
        path_label.set_tooltip_text(Some(path_text));
        row.append(&path_label);

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

            if path.exists() {
                open_btn.connect_clicked(move |_| {
                    match ws_clone.borrow_mut().load_from_file(&path) {
                        Ok(()) => sb_clone.borrow().set_message(&format!("Opened: {}", path.display())),
                        Err(e) => sb_clone.borrow().set_message(&format!("Error: {}", e)),
                    }
                    update_dirty_ui(&ws_clone, &win_clone, &sa_clone);
                    update_status_bar_path(&ws_clone, &sb_clone);
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
