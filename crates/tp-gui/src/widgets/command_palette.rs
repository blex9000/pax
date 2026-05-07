//! Command palette (Ctrl+K) — a centred overlay popover that fuzzy-
//! filters a list of registered actions and runs the selected one.
//!
//! Architecture mirrors widgets::quick_switcher (overlay layer,
//! thread-local singleton, register_global). The action set is
//! populated by the app (setup_workspace_ui) so each PaletteItem can
//! capture whatever closures it needs (ws_view, window, status bar,
//! save action). Adding a new entry is one push to a Vec<PaletteItem>.

use std::cell::RefCell;
use std::rc::Rc;

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use gtk4::prelude::*;

const POPOVER_WIDTH_PX: i32 = 520;
const POPOVER_MAX_HEIGHT_PX: i32 = 380;
/// Cap on visible matches so a palette with hundreds of registered
/// items doesn't grow into a wall of text.
const MAX_VISIBLE_ITEMS: usize = 30;

/// One actionable entry in the palette. Built once per session and
/// fuzzy-matched against the user's query.
#[derive(Clone)]
pub struct PaletteItem {
    pub title: String,
    /// Short context line shown under the title (typically the
    /// shortcut hint or a category like "Workspace" / "Layout").
    pub subtitle: Option<String>,
    /// Optional GTK icon name shown as a leading glyph.
    pub icon: Option<String>,
    /// Invoked when the user activates the item. Runs after the
    /// palette has hidden itself.
    pub action: Rc<dyn Fn()>,
}

pub struct CommandPalette {
    container: gtk4::Box,
    search_entry: gtk4::SearchEntry,
    list_box: gtk4::ListBox,
    /// All items registered by the app. The visible listbox is
    /// rebuilt from this on each keystroke.
    all_items: Rc<RefCell<Vec<PaletteItem>>>,
    /// Items currently displayed (in row order); used by row-activated
    /// to look up the action without storing it on the widget tree.
    visible_items: Rc<RefCell<Vec<PaletteItem>>>,
}

impl CommandPalette {
    pub fn new() -> Rc<Self> {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        container.add_css_class("command-palette");
        // Reuse the quick-switcher's overlay placement so the two
        // popovers feel like the same surface family.
        container.add_css_class("quick-switcher");
        container.set_halign(gtk4::Align::Center);
        container.set_valign(gtk4::Align::Start);
        container.set_margin_top(80);
        container.set_width_request(POPOVER_WIDTH_PX);
        container.set_visible(false);

        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Type a command…"));
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
            all_items: Rc::new(RefCell::new(Vec::new())),
            visible_items: Rc::new(RefCell::new(Vec::new())),
        });

        // Esc dismisses.
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

        // Down-arrow from search → step into list and select the first
        // row, mirroring the quick-switcher's keyboard flow.
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

        // Enter on search → run the first match.
        {
            let me_for_activate = me.clone();
            me.search_entry.connect_activate(move |_| {
                me_for_activate.activate_row(0);
            });
        }

        // Click / Enter on a row → run that item's action.
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

    /// Replace the registered action set. Called from the app on
    /// workspace setup so the items capture the live ws_view / window
    /// / save_action references.
    pub fn set_items(&self, items: Vec<PaletteItem>) {
        *self.all_items.borrow_mut() = items;
    }

    /// Rebuild the visible list from `all_items`, applying the search
    /// query as a fuzzy filter (SkimMatcherV2). Empty query shows all
    /// items in their registered order.
    fn repopulate(self: &Rc<Self>) {
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }
        self.visible_items.borrow_mut().clear();

        let query = self.search_entry.text().to_string();
        let all = self.all_items.borrow().clone();

        let filtered: Vec<PaletteItem> = if query.trim().is_empty() {
            all.into_iter().take(MAX_VISIBLE_ITEMS).collect()
        } else {
            let matcher = SkimMatcherV2::default();
            let mut scored: Vec<(i64, PaletteItem)> = all
                .into_iter()
                .filter_map(|item| {
                    let haystack = match item.subtitle.as_deref() {
                        Some(sub) if !sub.is_empty() => format!("{} {}", item.title, sub),
                        _ => item.title.clone(),
                    };
                    matcher
                        .fuzzy_match(&haystack, &query)
                        .map(|score| (score, item))
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0));
            scored
                .into_iter()
                .take(MAX_VISIBLE_ITEMS)
                .map(|(_, item)| item)
                .collect()
        };

        for item in filtered {
            self.append_row(&item);
            self.visible_items.borrow_mut().push(item);
        }

        if let Some(first) = self.list_box.row_at_index(0) {
            self.list_box.select_row(Some(&first));
        }
    }

    fn append_row(&self, item: &PaletteItem) {
        let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        row_box.set_margin_top(4);
        row_box.set_margin_bottom(4);
        row_box.set_margin_start(8);
        row_box.set_margin_end(8);

        if let Some(ref icon) = item.icon {
            let img = gtk4::Image::from_icon_name(icon);
            row_box.append(&img);
        }

        let info = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
        info.set_hexpand(true);
        let title = gtk4::Label::new(Some(&item.title));
        title.add_css_class("heading");
        title.set_halign(gtk4::Align::Start);
        title.set_xalign(0.0);
        info.append(&title);

        if let Some(ref sub) = item.subtitle {
            let sub_label = gtk4::Label::new(Some(sub));
            sub_label.add_css_class("dim-label");
            sub_label.add_css_class("caption");
            sub_label.set_halign(gtk4::Align::Start);
            sub_label.set_xalign(0.0);
            info.append(&sub_label);
        }

        row_box.append(&info);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&row_box));
        row.set_activatable(true);
        self.list_box.append(&row);
    }

    fn activate_row(&self, idx: usize) {
        let action = self
            .visible_items
            .borrow()
            .get(idx)
            .map(|i| i.action.clone());
        if let Some(action) = action {
            self.hide();
            action();
        }
    }
}

thread_local! {
    static REGISTERED_PALETTE: RefCell<Option<Rc<CommandPalette>>> = RefCell::new(None);
}

pub fn register_global(palette: Rc<CommandPalette>) {
    REGISTERED_PALETTE.with(|c| *c.borrow_mut() = Some(palette));
}

/// Replace the items shown in the registered palette. No-op if the
/// palette hasn't been registered yet (e.g. welcome screen).
pub fn set_items(items: Vec<PaletteItem>) {
    REGISTERED_PALETTE.with(|c| {
        if let Some(p) = c.borrow().as_ref() {
            p.set_items(items);
        }
    });
}

pub fn toggle() {
    REGISTERED_PALETTE.with(|c| {
        if let Some(p) = c.borrow().as_ref() {
            p.toggle();
        }
    });
}

pub fn hide() {
    REGISTERED_PALETTE.with(|c| {
        if let Some(p) = c.borrow().as_ref() {
            p.hide();
        }
    });
}
