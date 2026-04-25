//! Centred overlay popover for fast workspace switching.
//!
//! Triggered by Ctrl+Shift+O (the original B3 plan called for Ctrl+Tab
//! but that's already bound by the code editor for tab navigation).
//! Lives on the same `gtk::Overlay` layer as the alert tray, so it
//! floats above whichever workspace screen is currently mounted.
//!
//! Behaviour:
//!   · Search entry filters workspace records by name + config path
//!   · ListBox of matching workspaces; each row carries a "new window"
//!     icon button alongside its label
//!   · Enter / row-activated → open in this window (via registered
//!     handler that does dirty-check + load_from_file)
//!   · Side button on row → open in a separate Pax process
//!   · Esc → dismiss, clears search

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::prelude::*;

use pax_db::workspaces::WorkspaceRecord;

const POPOVER_WIDTH_PX: i32 = 480;
const POPOVER_MAX_HEIGHT_PX: i32 = 380;
const RECENT_LIMIT: usize = 20;

pub struct QuickSwitcher {
    /// Outermost widget — added to the window's GtkOverlay. Hidden by
    /// default (`visible=false`); `show()` flips it on, populates the
    /// list and focuses the search entry.
    container: gtk4::Box,
    search_entry: gtk4::SearchEntry,
    list_box: gtk4::ListBox,
    /// Records currently displayed in the listbox (in row order). Used
    /// by row-activated to look up the path without round-tripping the
    /// widget tree.
    rows: Rc<RefCell<Vec<WorkspaceRecord>>>,
}

impl QuickSwitcher {
    pub fn new() -> Rc<Self> {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        container.add_css_class("quick-switcher");
        container.set_halign(gtk4::Align::Center);
        container.set_valign(gtk4::Align::Start);
        container.set_margin_top(80);
        container.set_width_request(POPOVER_WIDTH_PX);
        container.set_visible(false);

        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Switch to workspace…"));
        search_entry.add_css_class("quick-switcher-search");
        container.append(&search_entry);

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_max_content_height(POPOVER_MAX_HEIGHT_PX);
        scroll.set_propagate_natural_height(true);
        scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);

        let list_box = gtk4::ListBox::new();
        list_box.set_selection_mode(gtk4::SelectionMode::Single);
        list_box.add_css_class("quick-switcher-list");
        scroll.set_child(Some(&list_box));
        container.append(&scroll);

        let me = Rc::new(Self {
            container,
            search_entry,
            list_box,
            rows: Rc::new(RefCell::new(Vec::new())),
        });

        // Esc anywhere inside the popover dismisses it.
        {
            let me_for_esc = me.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
            key_ctrl.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    me_for_esc.hide();
                    return gtk4::glib::Propagation::Stop;
                }
                gtk4::glib::Propagation::Proceed
            });
            me.container.add_controller(key_ctrl);
        }

        // Filter on every keystroke.
        {
            let me_for_search = me.clone();
            me.search_entry
                .connect_search_changed(move |_| me_for_search.repopulate());
        }

        // Down-arrow from the search entry steps into the listbox so
        // the user can navigate without leaving the keyboard.
        {
            let lb = me.list_box.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Down {
                    lb.grab_focus();
                    if let Some(first) = lb.row_at_index(0) {
                        lb.select_row(Some(&first));
                        first.grab_focus();
                    }
                    return gtk4::glib::Propagation::Stop;
                }
                gtk4::glib::Propagation::Proceed
            });
            me.search_entry.add_controller(key_ctrl);
        }

        // Pressing Enter in the search field activates the first row.
        {
            let me_for_activate = me.clone();
            me.search_entry.connect_activate(move |_| {
                if let Some(first) = me_for_activate.list_box.row_at_index(0) {
                    me_for_activate.list_box.select_row(Some(&first));
                    me_for_activate.activate_row(0);
                }
            });
        }

        // Click / Enter on a row → open in this window via the
        // registered handler.
        {
            let me_for_row = me.clone();
            me.list_box.connect_row_activated(move |_, row| {
                let idx = row.index() as usize;
                me_for_row.activate_row(idx);
            });
        }

        me
    }

    pub fn widget(&self) -> &gtk4::Widget {
        self.container.upcast_ref()
    }

    pub fn is_visible(&self) -> bool {
        self.container.is_visible()
    }

    /// Open the popover: clear the search entry, populate the list
    /// with the unfiltered top recents, focus the entry.
    pub fn show(self: &Rc<Self>) {
        self.search_entry.set_text("");
        self.repopulate();
        self.container.set_visible(true);
        self.search_entry.grab_focus();
    }

    pub fn hide(&self) {
        self.container.set_visible(false);
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.is_visible() {
            self.hide();
        } else {
            self.show();
        }
    }

    /// Rebuild the listbox from the DB's recent workspaces, applying
    /// the current search-entry text as a case-insensitive substring
    /// filter against name + config path.
    fn repopulate(self: &Rc<Self>) {
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }
        self.rows.borrow_mut().clear();

        let query = self.search_entry.text().to_string().to_lowercase();
        let recents = pax_db::Database::open(&pax_db::Database::default_path())
            .ok()
            .and_then(|db| db.list_workspaces_limit(RECENT_LIMIT).ok())
            .unwrap_or_default();

        for record in recents.into_iter().filter(|r| matches_query(r, &query)) {
            self.append_row(&record);
            self.rows.borrow_mut().push(record);
        }

        if let Some(first) = self.list_box.row_at_index(0) {
            self.list_box.select_row(Some(&first));
        }
    }

    fn append_row(&self, record: &WorkspaceRecord) {
        let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        row_box.set_margin_top(4);
        row_box.set_margin_bottom(4);
        row_box.set_margin_start(8);
        row_box.set_margin_end(4);

        if record.pinned {
            let pin_icon = gtk4::Image::from_icon_name("view-pin-symbolic");
            pin_icon.add_css_class("accent");
            row_box.append(&pin_icon);
        }

        let info = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
        info.set_hexpand(true);

        let name = gtk4::Label::new(Some(&record.name));
        name.add_css_class("heading");
        name.set_halign(gtk4::Align::Start);
        name.set_xalign(0.0);
        info.append(&name);

        let path_text = record.config_path.as_deref().unwrap_or("(no file)");
        let path = gtk4::Label::new(Some(path_text));
        path.add_css_class("dim-label");
        path.add_css_class("caption");
        path.set_halign(gtk4::Align::Start);
        path.set_xalign(0.0);
        path.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
        info.append(&path);

        row_box.append(&info);

        let new_window_btn = gtk4::Button::from_icon_name("window-new-symbolic");
        new_window_btn.add_css_class("flat");
        new_window_btn.set_tooltip_text(Some("Open in a new window"));
        new_window_btn.set_valign(gtk4::Align::Center);
        let can_open = record
            .config_path
            .as_ref()
            .map(|p| std::path::Path::new(p).exists())
            .unwrap_or(false);
        new_window_btn.set_sensitive(can_open);
        if can_open {
            if let Some(ref p) = record.config_path {
                let path = PathBuf::from(p);
                new_window_btn.connect_clicked(move |_| {
                    dispatch_new_window(&path);
                });
            }
        }
        row_box.append(&new_window_btn);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&row_box));
        row.set_activatable(can_open);
        row.set_selectable(can_open);
        if !can_open {
            row.add_css_class("dim-label");
        }
        self.list_box.append(&row);
    }

    fn activate_row(&self, idx: usize) {
        let path_opt = self
            .rows
            .borrow()
            .get(idx)
            .and_then(|r| r.config_path.clone());
        let Some(path_str) = path_opt else { return };
        let path = PathBuf::from(path_str);
        if !path.exists() {
            return;
        }
        self.hide();
        dispatch_this_window(&path);
    }
}

/// Case-insensitive substring match on `name` and (optionally) the
/// config path. Empty query matches everything.
fn matches_query(record: &WorkspaceRecord, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    if record.name.to_lowercase().contains(query) {
        return true;
    }
    record
        .config_path
        .as_deref()
        .map(|p| p.to_lowercase().contains(query))
        .unwrap_or(false)
}

thread_local! {
    static REGISTERED_SWITCHER: RefCell<Option<Rc<QuickSwitcher>>> = RefCell::new(None);
    static OPEN_THIS_HANDLER: RefCell<Option<Rc<dyn Fn(&Path)>>> = RefCell::new(None);
    static OPEN_NEW_HANDLER: RefCell<Option<Rc<dyn Fn(&Path)>>> = RefCell::new(None);
}

/// Register the singleton tray. Called once when the main window
/// builds its overlay (mirrors widgets::alert_tray::register_global).
pub fn register_global(switcher: Rc<QuickSwitcher>) {
    REGISTERED_SWITCHER.with(|c| *c.borrow_mut() = Some(switcher));
}

/// Register the action handlers. The app installs these once a
/// WorkspaceView is available so the same dirty-check / load flow as
/// the rest of the app gets reused.
pub fn register_handlers(open_this_window: Rc<dyn Fn(&Path)>, open_new_window: Rc<dyn Fn(&Path)>) {
    OPEN_THIS_HANDLER.with(|c| *c.borrow_mut() = Some(open_this_window));
    OPEN_NEW_HANDLER.with(|c| *c.borrow_mut() = Some(open_new_window));
}

/// Show / hide / toggle the registered switcher. No-op when nothing
/// is registered (e.g. during welcome screen with no workspace yet).
pub fn toggle() {
    REGISTERED_SWITCHER.with(|c| {
        if let Some(s) = c.borrow().as_ref() {
            s.toggle();
        }
    });
}

pub fn hide() {
    REGISTERED_SWITCHER.with(|c| {
        if let Some(s) = c.borrow().as_ref() {
            s.hide();
        }
    });
}

fn dispatch_this_window(path: &Path) {
    let handler = OPEN_THIS_HANDLER.with(|c| c.borrow().clone());
    if let Some(h) = handler {
        h(path);
    }
}

fn dispatch_new_window(path: &Path) {
    let handler = OPEN_NEW_HANDLER.with(|c| c.borrow().clone());
    if let Some(h) = handler {
        h(path);
    }
}
