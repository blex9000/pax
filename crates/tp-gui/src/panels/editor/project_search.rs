use gtk4::prelude::*;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Callback when a search result is clicked: (file_path, line_number)
pub type OnResultClick = Rc<dyn Fn(&Path, u32)>;

/// Project-wide search sidebar panel.
pub struct ProjectSearch {
    pub widget: gtk4::Box,
    search_entry: gtk4::SearchEntry,
    #[allow(dead_code)]
    results_list: gtk4::ListBox,
    #[allow(dead_code)]
    root_dir: PathBuf,
    #[allow(dead_code)]
    on_click: OnResultClick,
}

#[derive(Clone)]
struct SearchResult {
    path: PathBuf,
    line_num: u32,
    line_text: String,
}

impl ProjectSearch {
    pub fn new(root_dir: &Path, on_click: OnResultClick) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        let header = gtk4::Label::new(Some("Search in Files"));
        header.add_css_class("heading");
        header.set_halign(gtk4::Align::Start);
        header.set_margin_start(8);
        header.set_margin_top(4);
        header.set_margin_bottom(4);
        container.append(&header);

        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Search in project..."));
        search_entry.set_margin_start(4);
        search_entry.set_margin_end(4);
        container.append(&search_entry);

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_vexpand(true);

        let results_list = gtk4::ListBox::new();
        results_list.add_css_class("navigation-sidebar");
        scroll.set_child(Some(&results_list));
        container.append(&scroll);

        let status_label = gtk4::Label::new(None);
        status_label.add_css_class("dim-label");
        status_label.add_css_class("caption");
        status_label.set_margin_start(8);
        status_label.set_margin_bottom(4);
        container.append(&status_label);

        // Search on Enter (not on every keystroke — project search can be slow)
        let results_store: Rc<std::cell::RefCell<Vec<SearchResult>>> = Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let root = root_dir.to_path_buf();
            let results_list_c = results_list.clone();
            let results_s = results_store.clone();
            let status_l = status_label.clone();
            search_entry.connect_activate(move |entry| {
                let query = entry.text().to_string();
                if query.is_empty() { return; }

                // Clear previous
                while let Some(child) = results_list_c.first_child() {
                    results_list_c.remove(&child);
                }

                let results = search_in_files(&root, &query);
                let count = results.len();
                status_l.set_text(&format!("{} matches", count));

                for result in &results {
                    let rel = result.path.strip_prefix(&root).unwrap_or(&result.path);
                    let row = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                    row.set_margin_start(4);
                    row.set_margin_end(4);
                    row.set_margin_top(2);
                    row.set_margin_bottom(2);

                    let file_label = gtk4::Label::new(Some(
                        &format!("{}:{}", rel.to_string_lossy(), result.line_num)
                    ));
                    file_label.set_halign(gtk4::Align::Start);
                    file_label.add_css_class("caption");
                    file_label.set_opacity(0.7);
                    row.append(&file_label);

                    let text_label = gtk4::Label::new(Some(result.line_text.trim()));
                    text_label.set_halign(gtk4::Align::Start);
                    text_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                    text_label.set_max_width_chars(40);
                    row.append(&text_label);

                    results_list_c.append(&row);
                }

                *results_s.borrow_mut() = results;
            });
        }

        // Click result → open file at line
        {
            let on_click_c = on_click.clone();
            let results_s = results_store.clone();
            results_list.connect_row_activated(move |_, row| {
                let idx = row.index() as usize;
                let results = results_s.borrow();
                if let Some(result) = results.get(idx) {
                    on_click_c(&result.path, result.line_num);
                }
            });
        }

        Self {
            widget: container,
            search_entry,
            results_list,
            root_dir: root_dir.to_path_buf(),
            on_click,
        }
    }

    pub fn focus_entry(&self) {
        self.search_entry.grab_focus();
    }
}

/// Search for a query string across project files using the `ignore` crate.
fn search_in_files(root: &Path, query: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let query_lower = query.to_lowercase();

    let walker = ignore::WalkBuilder::new(root)
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();

        // Skip binary files (simple heuristic: check extension)
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            match ext {
                "png" | "jpg" | "jpeg" | "gif" | "ico" | "woff" | "woff2" | "ttf" |
                "otf" | "eot" | "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" |
                "exe" | "dll" | "so" | "dylib" | "o" | "a" | "class" | "pyc" |
                "db" | "sqlite" | "lock" => continue,
                _ => {}
            }
        }

        if let Ok(content) = std::fs::read_to_string(path) {
            for (line_idx, line) in content.lines().enumerate() {
                if line.to_lowercase().contains(&query_lower) {
                    results.push(SearchResult {
                        path: path.to_path_buf(),
                        line_num: (line_idx + 1) as u32,
                        line_text: line.to_string(),
                    });
                    // Limit results per file
                    if results.len() > 500 { return results; }
                }
            }
        }
    }

    results
}
