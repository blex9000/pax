use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

use crate::workspace_view::WorkspaceView;
use crate::widgets::status_bar::StatusBar;

thread_local! {
    pub static DIRTY_INDICATOR: RefCell<Option<(gtk4::Image, gtk4::Separator)>> = RefCell::new(None);
    pub static HEADER_WS_LABEL: RefCell<Option<gtk4::Label>> = RefCell::new(None);
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
    HEADER_WS_LABEL.with(|cell| {
        if let Some(ref label) = *cell.borrow() {
            label.set_text(name);
        }
    });
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
    do_save_internal(ws, sb, window, save_action, force_dialog, None);
}

fn do_save_internal(
    ws: &Rc<RefCell<WorkspaceView>>,
    sb: &Rc<RefCell<StatusBar>>,
    window: &Rc<adw::ApplicationWindow>,
    save_action: &gtk4::gio::SimpleAction,
    force_dialog: bool,
    after_save: Option<Rc<dyn Fn()>>,
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
                    if let Some(ref cb) = after_save {
                        cb();
                    }
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
    let after_save_cb = after_save.clone();
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
                            if let Some(ref cb) = after_save_cb {
                                cb();
                            }
                        }
                        Err(e) => sb.borrow().set_message(&format!("Save error: {}", e)),
                    }
                }
            }
        },
    );
}

pub fn confirm_discard_workspace_changes(
    ws: &Rc<RefCell<WorkspaceView>>,
    sb: &Rc<RefCell<StatusBar>>,
    window: &Rc<adw::ApplicationWindow>,
    save_action: &gtk4::gio::SimpleAction,
    on_continue: Rc<dyn Fn()>,
) {
    if !ws.borrow().is_dirty() {
        on_continue();
        return;
    }

    let dialog = gtk4::Window::builder()
        .title("Unsaved Workspace Changes")
        .transient_for(window.as_ref())
        .modal(true)
        .default_width(420)
        .default_height(120)
        .build();

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let ws_name = ws.borrow().workspace_name().to_string();
    let title = gtk4::Label::new(Some(&format!("Save changes to \"{}\" before continuing?", ws_name)));
    title.set_wrap(true);
    title.set_halign(gtk4::Align::Start);
    vbox.append(&title);

    let subtitle = gtk4::Label::new(Some("If you continue without saving, the current workspace changes will be lost."));
    subtitle.add_css_class("dim-label");
    subtitle.set_wrap(true);
    subtitle.set_halign(gtk4::Align::Start);
    vbox.append(&subtitle);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    let discard_btn = gtk4::Button::with_label("Discard");
    discard_btn.add_css_class("destructive-action");
    let save_btn = gtk4::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");

    btn_row.append(&cancel_btn);
    btn_row.append(&discard_btn);
    btn_row.append(&save_btn);
    vbox.append(&btn_row);

    {
        let d = dialog.clone();
        cancel_btn.connect_clicked(move |_| d.close());
    }
    {
        let d = dialog.clone();
        let cont = on_continue.clone();
        discard_btn.connect_clicked(move |_| {
            d.close();
            cont();
        });
    }
    {
        let d = dialog.clone();
        let ws = ws.clone();
        let sb = sb.clone();
        let win = window.clone();
        let sa = save_action.clone();
        let cont = on_continue.clone();
        save_btn.connect_clicked(move |_| {
            d.close();
            do_save_internal(&ws, &sb, &win, &sa, false, Some(cont.clone()));
        });
    }

    dialog.set_child(Some(&vbox));
    dialog.present();
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
                    let ws2 = ws.clone();
                    let sb2 = sb.clone();
                    let win2 = win.clone();
                    let sa2 = sa.clone();
                    let on_continue: Rc<dyn Fn()> = Rc::new(move || {
                        // Drop the borrow_mut() guard before entering the arm,
                        // otherwise the second borrow_mut() inside Ok(()) would
                        // panic with "RefCell already borrowed".
                        let load_result = ws2.borrow_mut().load_from_file(&path);
                        match load_result {
                            Ok(()) => {
                                let theme = crate::app::apply_preferred_theme();
                                ws2.borrow_mut()
                                    .set_workspace_theme_id_clean(theme.to_id());
                                sb2.borrow().set_message(&format!("Opened: {}", path.display()));
                            }
                            Err(e) => {
                                sb2.borrow().set_message(&format!("Open error: {}", e));
                                return;
                            }
                        }
                        update_dirty_ui(&ws2, &win2, &sa2);
                        update_status_bar_path(&ws2, &sb2);
                    });
                    confirm_discard_workspace_changes(&ws, &sb, &win, &sa, on_continue);
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
                    let ws2 = ws_clone.clone();
                    let sb2 = sb_clone.clone();
                    let win2 = win_clone.clone();
                    let sa2 = sa_clone.clone();
                    let d2 = d.clone();
                    let path2 = path.clone();
                    let on_continue: Rc<dyn Fn()> = Rc::new(move || {
                        // Drop the borrow_mut() guard before entering the arm,
                        // otherwise the second borrow_mut() inside Ok(()) would
                        // panic with "RefCell already borrowed".
                        let load_result = ws2.borrow_mut().load_from_file(&path2);
                        match load_result {
                            Ok(()) => {
                                let theme = crate::app::apply_preferred_theme();
                                ws2.borrow_mut()
                                    .set_workspace_theme_id_clean(theme.to_id());
                                sb2.borrow().set_message(&format!("Opened: {}", path2.display()));
                            }
                            Err(e) => {
                                sb2.borrow().set_message(&format!("Error: {}", e));
                                return;
                            }
                        }
                        update_dirty_ui(&ws2, &win2, &sa2);
                        update_status_bar_path(&ws2, &sb2);
                        d2.close();
                    });
                    confirm_discard_workspace_changes(&ws_clone, &sb_clone, &win_clone, &sa_clone, on_continue);
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
