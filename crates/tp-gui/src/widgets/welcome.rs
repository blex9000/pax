use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Callback when user makes a choice on the welcome screen.
#[derive(Clone)]
pub enum WelcomeChoice {
    NewWorkspace,
    OpenFile,
    OpenRecent(String), // config path
}

pub type WelcomeCallback = Rc<dyn Fn(WelcomeChoice)>;

fn welcome_version_text() -> String {
    format!("Version {}", pax_core::build_info::VERSION_STRING)
}

/// Welcome screen shown on startup — like VS Code / IntelliJ start page.
pub fn build_welcome(on_choice: WelcomeCallback) -> gtk4::Widget {
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 24);
    container.set_valign(gtk4::Align::Center);
    container.set_halign(gtk4::Align::Center);
    container.set_margin_top(40);
    container.set_margin_bottom(40);
    container.set_margin_start(40);
    container.set_margin_end(40);
    container.set_width_request(600);

    // App icon
    let icon = gtk4::Image::from_icon_name("pax");
    icon.set_pixel_size(128);
    icon.set_margin_bottom(8);
    container.append(&icon);

    // Title
    let title = gtk4::Label::new(Some("Pax"));
    title.add_css_class("title-1");
    container.append(&title);

    let subtitle = gtk4::Label::new(Some("Workspace Manager"));
    subtitle.add_css_class("dim-label");
    container.append(&subtitle);

    let version_text = welcome_version_text();
    let version = gtk4::Label::new(Some(&version_text));
    version.add_css_class("dim-label");
    version.add_css_class("caption");
    version.set_tooltip_text(Some(&version_text));
    container.append(&version);

    // Action buttons row
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
    actions.set_halign(gtk4::Align::Center);
    actions.set_margin_top(8);

    // New Workspace button
    {
        let btn = gtk4::Button::new();
        btn.add_css_class("card");
        btn.add_css_class("welcome-action-btn");

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        vbox.set_margin_top(20);
        vbox.set_margin_bottom(20);
        vbox.set_margin_start(32);
        vbox.set_margin_end(32);

        let icon = gtk4::Image::from_icon_name("document-new-symbolic");
        icon.set_pixel_size(32);
        vbox.append(&icon);

        let label = gtk4::Label::new(Some("New Workspace"));
        label.add_css_class("heading");
        vbox.append(&label);

        let desc = gtk4::Label::new(Some("Start from scratch"));
        desc.add_css_class("dim-label");
        desc.add_css_class("caption");
        vbox.append(&desc);

        btn.set_child(Some(&vbox));

        let cb = on_choice.clone();
        btn.connect_clicked(move |_| cb(WelcomeChoice::NewWorkspace));
        actions.append(&btn);
    }

    // Open File button
    {
        let btn = gtk4::Button::new();
        btn.add_css_class("card");
        btn.add_css_class("welcome-action-btn");

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        vbox.set_margin_top(20);
        vbox.set_margin_bottom(20);
        vbox.set_margin_start(32);
        vbox.set_margin_end(32);

        let icon = gtk4::Image::from_icon_name("document-open-symbolic");
        icon.set_pixel_size(32);
        vbox.append(&icon);

        let label = gtk4::Label::new(Some("Open File"));
        label.add_css_class("heading");
        vbox.append(&label);

        let desc = gtk4::Label::new(Some("Load workspace JSON"));
        desc.add_css_class("dim-label");
        desc.add_css_class("caption");
        vbox.append(&desc);

        btn.set_child(Some(&vbox));

        let cb = on_choice.clone();
        btn.connect_clicked(move |_| cb(WelcomeChoice::OpenFile));
        actions.append(&btn);
    }

    container.append(&actions);

    // Recent workspaces section — same limit as the Recent Workspaces
    // dialog (actions::show_recent_dialog) so both surfaces show the
    // same cardinality. Entries whose file no longer exists are shown
    // dimmed and disabled, matching the dialog's behaviour (not filtered
    // out — so users still see their history even after moves/deletes).
    let db_path = pax_db::Database::default_path();
    let visible_recents = pax_db::Database::open(&db_path)
        .ok()
        .and_then(|db| db.list_workspaces_limit(20).ok())
        .unwrap_or_default();

    if !visible_recents.is_empty() {
        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        sep.set_margin_top(8);
        container.append(&sep);

        let recent_label = gtk4::Label::new(Some("Recent Workspaces"));
        recent_label.add_css_class("title-4");
        recent_label.set_halign(gtk4::Align::Start);
        container.append(&recent_label);

        let list_box = gtk4::ListBox::new();
        list_box.set_selection_mode(gtk4::SelectionMode::None);
        list_box.add_css_class("boxed-list");

        // The recent-list rows are rebuilt on every pin toggle so the
        // ordering reflects the new state immediately. `repopulate` is
        // a self-referential Rc<RefCell<...>> so the pin button's
        // click handler can call back into it without cyclic Rc
        // ownership.
        let repopulate: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));

        let build_rows: Box<dyn Fn()> = {
            let list_box = list_box.clone();
            let on_choice = on_choice.clone();
            let repopulate_for_pin = repopulate.clone();
            Box::new(move || {
                while let Some(child) = list_box.first_child() {
                    list_box.remove(&child);
                }
                let records = pax_db::Database::open(&pax_db::Database::default_path())
                    .ok()
                    .and_then(|db| db.list_workspaces_limit(20).ok())
                    .unwrap_or_default();
                for record in records {
                    populate_recent_row(
                        &list_box,
                        &record,
                        &on_choice,
                        &repopulate_for_pin,
                    );
                }
            })
        };
        build_rows();
        *repopulate.borrow_mut() = Some(build_rows);

        // Connect row activation
        let cb = on_choice.clone();
        list_box.connect_row_activated(move |_, row| {
            if let Some(child) = row.child() {
                let name = child.widget_name();
                let name_str = name.as_str();
                if !name_str.is_empty() {
                    cb(WelcomeChoice::OpenRecent(name_str.to_string()));
                }
            }
        });

        // Scrollable recent list — sized like the Recent Workspaces
        // dialog (fixed height, vertical scroll when there are many
        // entries). Previously the list used propagate_natural_height
        // with a 250px cap, so with 3-4 recents it looked shrunken.
        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_child(Some(&list_box));
        scroll.set_min_content_height(300);
        scroll.set_max_content_height(400);
        scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
        scroll.set_vscrollbar_policy(gtk4::PolicyType::Automatic);
        container.append(&scroll);
    }

    container.upcast::<gtk4::Widget>()
}

/// Append a single recent-workspace row to the listbox, including the
/// pin / new-window action buttons. Factored out of build_welcome so
/// the pin button can repopulate the whole list on toggle.
fn populate_recent_row(
    list_box: &gtk4::ListBox,
    record: &pax_db::workspaces::WorkspaceRecord,
    _on_choice: &WelcomeCallback,
    repopulate: &Rc<RefCell<Option<Box<dyn Fn()>>>>,
) {
    let config_path_opt = record.config_path.clone();
    let file_exists = config_path_opt
        .as_deref()
        .map(std::path::Path::new)
        .map(|p| p.exists())
        .unwrap_or(false);

    let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    row_box.set_margin_top(6);
    row_box.set_margin_bottom(6);
    row_box.set_margin_start(8);
    row_box.set_margin_end(8);

    let icon = gtk4::Image::from_icon_name("document-open-recent-symbolic");
    row_box.append(&icon);

    let info = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    info.set_hexpand(true);

    let name = gtk4::Label::new(Some(&record.name));
    name.add_css_class("heading");
    name.set_halign(gtk4::Align::Start);
    info.append(&name);

    let path_text = config_path_opt.as_deref().unwrap_or("(no file)");
    let path_label = gtk4::Label::new(Some(path_text));
    path_label.add_css_class("dim-label");
    path_label.add_css_class("caption");
    path_label.set_halign(gtk4::Align::Start);
    path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
    path_label.set_tooltip_text(Some(if file_exists {
        path_text
    } else if config_path_opt.is_some() {
        "File not found"
    } else {
        "No config file"
    }));
    info.append(&path_label);

    row_box.append(&info);

    // Pin toggle — pinned entries float to the top of list_workspaces.
    // Click rewrites DB and asks the parent to repopulate the rows so
    // the new ordering shows up without restarting the welcome.
    let pin_btn = gtk4::Button::new();
    pin_btn.set_icon_name("view-pin-symbolic");
    pin_btn.add_css_class("flat");
    pin_btn.set_valign(gtk4::Align::Center);
    if record.pinned {
        pin_btn.add_css_class("accent");
        pin_btn.set_tooltip_text(Some("Unpin"));
    } else {
        pin_btn.set_tooltip_text(Some("Pin"));
    }
    {
        let record = record.clone();
        let repopulate = repopulate.clone();
        pin_btn.connect_clicked(move |_| {
            let key = pax_db::Database::record_key_for(&record);
            let new_state = !record.pinned;
            if let Ok(db) = pax_db::Database::open(&pax_db::Database::default_path()) {
                if let Err(e) = db.set_workspace_pinned(&key, new_state) {
                    tracing::warn!("welcome: failed to update pin state: {e}");
                    return;
                }
            }
            if let Some(repop) = repopulate.borrow().as_ref() {
                repop();
            }
        });
    }
    row_box.append(&pin_btn);

    let new_window_btn = gtk4::Button::new();
    new_window_btn.set_icon_name("window-new-symbolic");
    new_window_btn.add_css_class("flat");
    new_window_btn.set_tooltip_text(Some("Open in a new window"));
    new_window_btn.set_valign(gtk4::Align::Center);
    new_window_btn.set_sensitive(file_exists);
    if file_exists {
        if let Some(path) = config_path_opt.clone() {
            new_window_btn.connect_clicked(move |_| {
                if let Err(e) = crate::workspace_launcher::open_in_new_window(
                    std::path::Path::new(&path),
                ) {
                    tracing::warn!("welcome: could not spawn new window: {e}");
                }
            });
        }
    }
    row_box.append(&new_window_btn);

    let row = gtk4::ListBoxRow::new();
    row.set_child(Some(&row_box));
    row.set_activatable(file_exists);
    row.set_selectable(file_exists);
    if !file_exists {
        row.add_css_class("dim-label");
    }

    row_box.set_widget_name(if file_exists {
        config_path_opt.as_deref().unwrap_or("")
    } else {
        ""
    });

    list_box.append(&row);
}

#[cfg(test)]
mod tests {
    use super::welcome_version_text;

    #[test]
    fn welcome_version_text_uses_build_metadata() {
        let text = welcome_version_text();

        assert!(text.starts_with("Version "));
        assert!(text.contains(pax_core::build_info::PACKAGE_VERSION));
        assert!(text.contains(pax_core::build_info::GIT_COMMIT));
        assert!(text.contains(pax_core::build_info::GIT_DATE));
    }
}
