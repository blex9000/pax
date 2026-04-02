use gtk4::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::file_backend::FileBackend;

/// Callback when a search result is clicked: (file_path, line_number, search_query)
pub type OnResultClick = Rc<dyn Fn(&Path, u32, &str)>;

/// Callback for replace in files: (root_dir, search_query, replace_text) → number replaced
pub type OnReplaceInFiles = Rc<dyn Fn(&Path, &str, &str) -> usize>;

/// Project-wide search sidebar panel with replace support.
pub struct ProjectSearch {
    pub widget: gtk4::Box,
    search_entry: gtk4::SearchEntry,
    #[allow(dead_code)]
    root_dir: PathBuf,
}

#[derive(Clone)]
struct SearchResult {
    path: PathBuf,
    line_num: u32,
    line_text: String,
    #[allow(dead_code)]
    match_start: usize,
    #[allow(dead_code)]
    match_len: usize,
}

impl ProjectSearch {
    pub fn new(root_dir: &Path, on_click: OnResultClick, backend: Rc<dyn FileBackend>) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        let header = gtk4::Label::new(Some("Search in Files"));
        header.add_css_class("heading");
        header.set_halign(gtk4::Align::Start);
        header.set_margin_start(8);
        header.set_margin_top(4);
        header.set_margin_bottom(4);
        container.append(&header);

        // Search entry
        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Search in project..."));
        search_entry.set_margin_start(4);
        search_entry.set_margin_end(4);
        container.append(&search_entry);

        // Replace row
        let replace_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        replace_row.set_margin_start(4);
        replace_row.set_margin_end(4);
        replace_row.set_margin_top(2);

        let replace_entry = gtk4::Entry::new();
        replace_entry.set_placeholder_text(Some("Replace with..."));
        replace_entry.set_hexpand(true);
        replace_row.append(&replace_entry);

        let replace_all_btn = gtk4::Button::from_icon_name("edit-find-replace-symbolic");
        replace_all_btn.add_css_class("flat");
        replace_all_btn.set_tooltip_text(Some("Replace all occurrences in all files"));
        replace_row.append(&replace_all_btn);

        container.append(&replace_row);

        // Status label
        let status_label = gtk4::Label::new(None);
        status_label.add_css_class("dim-label");
        status_label.add_css_class("caption");
        status_label.set_margin_start(8);
        status_label.set_margin_top(2);
        status_label.set_margin_bottom(2);
        container.append(&status_label);

        // Results list
        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_vexpand(true);

        let results_list = gtk4::ListBox::new();
        results_list.add_css_class("navigation-sidebar");
        scroll.set_child(Some(&results_list));
        container.append(&scroll);

        // Shared state
        let results_store: Rc<RefCell<Vec<SearchResult>>> = Rc::new(RefCell::new(Vec::new()));
        let last_query: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

        // Search on Enter
        {
            let root = root_dir.to_path_buf();
            let be = backend.clone();
            let results_list_c = results_list.clone();
            let results_s = results_store.clone();
            let status_l = status_label.clone();
            let lq = last_query.clone();
            search_entry.connect_activate(move |entry| {
                let query = entry.text().to_string();
                if query.is_empty() { return; }
                *lq.borrow_mut() = query.clone();

                // Clear previous
                while let Some(child) = results_list_c.first_child() {
                    results_list_c.remove(&child);
                }

                let results = search_in_files(&root, &query, &*be);
                let total_matches = results.len();

                // Group by file: count matches per file, keep first result per file for click
                let mut file_groups: Vec<(PathBuf, usize, u32)> = Vec::new(); // (path, count, first_line)
                for result in &results {
                    if let Some(group) = file_groups.iter_mut().find(|(p, _, _)| *p == result.path) {
                        group.1 += 1;
                    } else {
                        file_groups.push((result.path.clone(), 1, result.line_num));
                    }
                }

                let file_count = file_groups.len();
                status_l.set_text(&format!("{} matches in {} files", total_matches, file_count));

                for (file_path, match_count, _first_line) in &file_groups {
                    let rel = file_path.strip_prefix(&root).unwrap_or(file_path);
                    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
                    row.set_margin_start(6);
                    row.set_margin_end(6);
                    row.set_margin_top(3);
                    row.set_margin_bottom(3);

                    let icon = gtk4::Image::from_icon_name("text-x-generic-symbolic");
                    icon.set_pixel_size(14);
                    row.append(&icon);

                    let name_label = gtk4::Label::new(Some(&rel.to_string_lossy()));
                    name_label.set_halign(gtk4::Align::Start);
                    name_label.set_hexpand(true);
                    name_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
                    name_label.set_tooltip_text(Some(&rel.to_string_lossy()));
                    row.append(&name_label);

                    let count_label = gtk4::Label::new(Some(&format!("{}", match_count)));
                    count_label.add_css_class("dim-label");
                    count_label.add_css_class("caption");
                    row.append(&count_label);

                    let list_row = gtk4::ListBoxRow::new();
                    list_row.set_child(Some(&row));
                    list_row.set_widget_name(&file_path.to_string_lossy());
                    results_list_c.append(&list_row);
                }

                *results_s.borrow_mut() = results;
            });
        }

        // Click result → open file at line with search highlight
        {
            let on_click_c = on_click.clone();
            let results_s = results_store.clone();
            let lq = last_query.clone();
            results_list.connect_row_activated(move |_, row| {
                let file_path_str = row.widget_name();
                let file_path = PathBuf::from(file_path_str.as_str());
                let query = lq.borrow().clone();
                // Find first match in this file
                let results = results_s.borrow();
                let first_line = results.iter()
                    .find(|r| r.path == file_path)
                    .map(|r| r.line_num)
                    .unwrap_or(1);
                on_click_c(&file_path, first_line, &query);
            });
        }

        // Replace All in files
        {
            let root = root_dir.to_path_buf();
            let be = backend.clone();
            let se = search_entry.clone();
            let re = replace_entry.clone();
            let status_l = status_label.clone();
            let results_list_c = results_list.clone();
            let results_s = results_store.clone();
            replace_all_btn.connect_clicked(move |_| {
                let query = se.text().to_string();
                let replacement = re.text().to_string();
                if query.is_empty() { return; }

                let count = replace_in_files(&root, &query, &replacement, &*be);
                status_l.set_text(&format!("{} replaced in files", count));

                // Clear results (they're stale now)
                while let Some(child) = results_list_c.first_child() {
                    results_list_c.remove(&child);
                }
                results_s.borrow_mut().clear();
            });
        }

        Self {
            widget: container,
            search_entry,
            root_dir: root_dir.to_path_buf(),
        }
    }

    pub fn focus_entry(&self) {
        self.search_entry.grab_focus();
    }
}

/// Highlight occurrences of query in text using Pango markup.
fn build_highlight_markup(text: &str, query: &str) -> String {
    let text_escaped = gtk4::glib::markup_escape_text(text);
    let query_lower = query.to_lowercase();
    let text_lower = text_escaped.to_lowercase();

    let mut result = String::new();
    let mut last = 0;
    while let Some(pos) = text_lower[last..].find(&query_lower) {
        let abs_pos = last + pos;
        result.push_str(&text_escaped[last..abs_pos]);
        result.push_str("<b><span foreground=\"#e5a50a\">");
        result.push_str(&text_escaped[abs_pos..abs_pos + query.len()]);
        result.push_str("</span></b>");
        last = abs_pos + query.len();
    }
    result.push_str(&text_escaped[last..]);
    result
}

/// Search for a query string across project files.
/// For remote backends, uses backend.search_files(). For local, uses ignore::WalkBuilder.
fn search_in_files(root: &Path, query: &str, backend: &dyn FileBackend) -> Vec<SearchResult> {
    if backend.is_remote() {
        // Use backend search for remote
        let query_lower = query.to_lowercase();
        match backend.search_files(&query_lower) {
            Ok(hits) => {
                hits.into_iter()
                    .take(500)
                    .map(|(path_str, line_num, line_text)| {
                        let path = root.join(&path_str);
                        let match_start = line_text.to_lowercase().find(&query_lower).unwrap_or(0);
                        SearchResult {
                            path,
                            line_num: line_num as u32,
                            line_text,
                            match_start,
                            match_len: query.len(),
                        }
                    })
                    .collect()
            }
            Err(_) => Vec::new(),
        }
    } else {
        let mut results = Vec::new();
        let query_lower = query.to_lowercase();

        let walker = ignore::WalkBuilder::new(root).build();

        for entry in walker.flatten() {
            if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();

            // Skip binary files
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                match ext {
                    "png" | "jpg" | "jpeg" | "gif" | "ico" | "woff" | "woff2" | "ttf" |
                    "otf" | "eot" | "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" |
                    "exe" | "dll" | "so" | "dylib" | "o" | "a" | "class" | "pyc" |
                    "db" | "sqlite" | "lock" => continue,
                    _ => {}
                }
            }

            if let Ok(content) = backend.read_file(path) {
                for (line_idx, line) in content.lines().enumerate() {
                    let line_lower = line.to_lowercase();
                    if let Some(pos) = line_lower.find(&query_lower) {
                        results.push(SearchResult {
                            path: path.to_path_buf(),
                            line_num: (line_idx + 1) as u32,
                            line_text: line.to_string(),
                            match_start: pos,
                            match_len: query.len(),
                        });
                        if results.len() > 500 { return results; }
                    }
                }
            }
        }

        results
    }
}

/// Replace all occurrences of query with replacement across project files.
fn replace_in_files(root: &Path, query: &str, replacement: &str, backend: &dyn FileBackend) -> usize {
    let mut total = 0;
    let query_lower = query.to_lowercase();

    let walker = ignore::WalkBuilder::new(root).build();

    for entry in walker.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();

        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            match ext {
                "png" | "jpg" | "jpeg" | "gif" | "ico" | "woff" | "woff2" | "ttf" |
                "otf" | "eot" | "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" |
                "exe" | "dll" | "so" | "dylib" | "o" | "a" | "class" | "pyc" |
                "db" | "sqlite" | "lock" => continue,
                _ => {}
            }
        }

        if let Ok(content) = backend.read_file(path) {
            if content.to_lowercase().contains(&query_lower) {
                // Case-insensitive replace preserving structure
                let mut new_content = String::new();
                let mut last = 0;
                let content_lower = content.to_lowercase();
                while let Some(pos) = content_lower[last..].find(&query_lower) {
                    let abs_pos = last + pos;
                    new_content.push_str(&content[last..abs_pos]);
                    new_content.push_str(replacement);
                    total += 1;
                    last = abs_pos + query.len();
                }
                new_content.push_str(&content[last..]);
                let _ = backend.write_file(path, &new_content);
            }
        }
    }

    total
}
