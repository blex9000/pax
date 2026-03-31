use gtk4::prelude::*;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Overlay fuzzy finder for quick file open.
pub struct FuzzyFinder {
    pub overlay: gtk4::Box,
    search_entry: gtk4::SearchEntry,
    results_list: gtk4::ListBox,
    file_index: Rc<RefCell<Vec<PathBuf>>>,
    root_dir: PathBuf,
    on_select: Rc<dyn Fn(&Path)>,
}

impl FuzzyFinder {
    pub fn new(
        root_dir: &Path,
        file_index: Rc<RefCell<Vec<PathBuf>>>,
        on_select: Rc<dyn Fn(&Path)>,
    ) -> Self {
        let overlay = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        overlay.set_halign(gtk4::Align::Center);
        overlay.set_valign(gtk4::Align::Start);
        overlay.set_margin_top(40);
        overlay.set_width_request(400);
        overlay.add_css_class("card");
        overlay.set_visible(false);

        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Search files..."));
        search_entry.set_margin_start(8);
        search_entry.set_margin_end(8);
        search_entry.set_margin_top(8);
        overlay.append(&search_entry);

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_max_content_height(300);
        scroll.set_propagate_natural_height(true);

        let results_list = gtk4::ListBox::new();
        results_list.add_css_class("navigation-sidebar");
        scroll.set_child(Some(&results_list));
        overlay.append(&scroll);

        let root = root_dir.to_path_buf();

        // Filter on text change
        {
            let results = results_list.clone();
            let index = file_index.clone();
            let root_c = root.clone();
            search_entry.connect_search_changed(move |entry| {
                let query = entry.text().to_string();
                // Clear previous results
                while let Some(child) = results.first_child() {
                    results.remove(&child);
                }
                if query.is_empty() { return; }

                let matcher = SkimMatcherV2::default();
                let files = index.borrow();
                let mut scored: Vec<(i64, &PathBuf)> = files.iter()
                    .filter_map(|p| {
                        let rel = p.strip_prefix(&root_c).unwrap_or(p);
                        let name = rel.to_string_lossy();
                        matcher.fuzzy_match(&name, &query).map(|score| (score, p))
                    })
                    .collect();
                scored.sort_by(|a, b| b.0.cmp(&a.0));

                for (_, path) in scored.iter().take(20) {
                    let rel = path.strip_prefix(&root_c).unwrap_or(path);
                    let label = gtk4::Label::new(Some(&rel.to_string_lossy()));
                    label.set_halign(gtk4::Align::Start);
                    label.set_margin_start(8);
                    label.set_margin_top(2);
                    label.set_margin_bottom(2);
                    results.append(&label);
                }
            });
        }

        // Enter to open selected, Escape to close
        {
            let overlay_c = overlay.clone();
            let index = file_index.clone();
            let root_c = root.clone();
            let on_sel = on_select.clone();
            search_entry.connect_activate(move |entry| {
                let query = entry.text().to_string();
                if query.is_empty() { return; }

                // Open the first result
                let matcher = SkimMatcherV2::default();
                let files = index.borrow();
                let mut scored: Vec<(i64, &PathBuf)> = files.iter()
                    .filter_map(|p| {
                        let rel = p.strip_prefix(&root_c).unwrap_or(p);
                        matcher.fuzzy_match(&rel.to_string_lossy(), &query).map(|s| (s, p))
                    })
                    .collect();
                scored.sort_by(|a, b| b.0.cmp(&a.0));

                if let Some((_, path)) = scored.first() {
                    on_sel(path);
                    overlay_c.set_visible(false);
                    entry.set_text("");
                }
            });
        }

        // Row activation
        {
            let overlay_c = overlay.clone();
            let entry_c = search_entry.clone();
            let index = file_index.clone();
            let root_c = root.clone();
            let on_sel = on_select.clone();
            results_list.connect_row_activated(move |_, row| {
                let idx = row.index() as usize;
                let query = entry_c.text().to_string();
                let matcher = SkimMatcherV2::default();
                let files = index.borrow();
                let mut scored: Vec<(i64, &PathBuf)> = files.iter()
                    .filter_map(|p| {
                        let rel = p.strip_prefix(&root_c).unwrap_or(p);
                        matcher.fuzzy_match(&rel.to_string_lossy(), &query).map(|s| (s, p))
                    })
                    .collect();
                scored.sort_by(|a, b| b.0.cmp(&a.0));

                if let Some((_, path)) = scored.get(idx) {
                    on_sel(path);
                    overlay_c.set_visible(false);
                    entry_c.set_text("");
                }
            });
        }

        // Escape to close
        {
            let overlay_c = overlay.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    overlay_c.set_visible(false);
                    return gtk4::glib::Propagation::Stop;
                }
                gtk4::glib::Propagation::Proceed
            });
            search_entry.add_controller(key_ctrl);
        }

        Self {
            overlay,
            search_entry,
            results_list,
            file_index,
            root_dir: root,
            on_select,
        }
    }

    pub fn show(&self) {
        self.search_entry.set_text("");
        self.overlay.set_visible(true);
        self.search_entry.grab_focus();
    }

    pub fn hide(&self) {
        self.overlay.set_visible(false);
    }
}
