use gtk4::prelude::*;
use regex::RegexBuilder;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use super::file_backend::FileBackend;
use super::task::run_blocking;

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
    pub fn new(root_dir: &Path, on_click: OnResultClick, backend: Arc<dyn FileBackend>) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("editor-sidebar-pane");

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
        scroll.add_css_class("editor-sidebar-pane-scroll");

        let results_list = gtk4::ListBox::new();
        results_list.add_css_class("navigation-sidebar");
        results_list.add_css_class("editor-sidebar-pane-list");
        // Without this, a single click only *selects* a row — the
        // `row-activated` signal (which opens the file) would require a
        // double-click or Enter. Users expect one click to jump.
        results_list.set_activate_on_single_click(true);
        scroll.set_child(Some(&results_list));
        container.append(&scroll);

        // Shared state
        let results_store: Rc<RefCell<Vec<SearchResult>>> = Rc::new(RefCell::new(Vec::new()));
        let last_query: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
        let request_seq = Rc::new(Cell::new(0u64));

        // Search on Enter
        {
            let root = root_dir.to_path_buf();
            let be = backend.clone();
            let results_list_c = results_list.clone();
            let results_s = results_store.clone();
            let status_l = status_label.clone();
            let lq = last_query.clone();
            let seq = request_seq.clone();
            search_entry.connect_activate(move |entry| {
                let query = entry.text().to_string();
                if query.is_empty() {
                    return;
                }
                *lq.borrow_mut() = query.clone();
                request_search(
                    &results_list_c,
                    &status_l,
                    &results_s,
                    &root,
                    be.clone(),
                    query,
                    seq.clone(),
                );
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
                let first_line = results
                    .iter()
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
            let seq = request_seq.clone();
            replace_all_btn.connect_clicked(move |_| {
                let query = se.text().to_string();
                let replacement = re.text().to_string();
                if query.is_empty() {
                    return;
                }
                let request_id = seq.get().wrapping_add(1);
                seq.set(request_id);
                status_l.set_text("Replacing...");
                clear_results_list(&results_list_c);

                let root_c = root.clone();
                let be_c = be.clone();
                let status_l_c = status_l.clone();
                let results_list_c2 = results_list_c.clone();
                let results_s_c = results_s.clone();
                let seq_c = seq.clone();
                run_blocking(
                    move || replace_in_files(&root_c, &query, &replacement, &*be_c),
                    move |count| {
                        if seq_c.get() != request_id {
                            return;
                        }
                        status_l_c.set_text(&format!("{} replaced in files", count));
                        clear_results_list(&results_list_c2);
                        results_s_c.borrow_mut().clear();
                    },
                );
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

    /// Pre-populate the search entry with `text` and select all its contents,
    /// so the next keystroke overwrites it. Used when Ctrl+Shift+F is pressed
    /// while the editor has a selection — the selected word becomes the
    /// starting query.
    pub fn set_query(&self, text: &str) {
        self.search_entry.set_text(text);
        self.search_entry.select_region(0, -1);
    }
}

fn request_search(
    results_list: &gtk4::ListBox,
    status_label: &gtk4::Label,
    results_store: &Rc<RefCell<Vec<SearchResult>>>,
    root: &Path,
    backend: Arc<dyn FileBackend>,
    query: String,
    request_seq: Rc<Cell<u64>>,
) {
    let request_id = request_seq.get().wrapping_add(1);
    request_seq.set(request_id);
    status_label.set_text("Searching...");
    clear_results_list(results_list);

    let results_list_c = results_list.clone();
    let status_label_c = status_label.clone();
    let results_store_c = results_store.clone();
    let root_c = root.to_path_buf();
    let search_root = root_c.clone();
    let seq_c = request_seq.clone();
    run_blocking(
        move || search_in_files(&search_root, &query, &*backend),
        move |results| {
            if seq_c.get() != request_id {
                return;
            }
            render_results(&results_list_c, &status_label_c, &root_c, &results);
            *results_store_c.borrow_mut() = results;
        },
    );
}

fn clear_results_list(results_list: &gtk4::ListBox) {
    while let Some(child) = results_list.first_child() {
        results_list.remove(&child);
    }
}

fn render_results(
    results_list: &gtk4::ListBox,
    status_label: &gtk4::Label,
    root: &Path,
    results: &[SearchResult],
) {
    clear_results_list(results_list);

    let total_matches = results.len();
    let mut file_groups: Vec<(PathBuf, usize, u32)> = Vec::new();
    for result in results {
        if let Some(group) = file_groups.iter_mut().find(|(p, _, _)| *p == result.path) {
            group.1 += 1;
        } else {
            file_groups.push((result.path.clone(), 1, result.line_num));
        }
    }

    status_label.set_text(&format!(
        "{} matches in {} files",
        total_matches,
        file_groups.len()
    ));

    for (file_path, match_count, _first_line) in &file_groups {
        let rel = file_path.strip_prefix(root).unwrap_or(file_path);
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
        results_list.append(&list_row);
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
    let regex = match RegexBuilder::new(&regex::escape(query))
        .case_insensitive(true)
        .build()
    {
        Ok(regex) => regex,
        Err(_) => return Vec::new(),
    };

    if backend.is_remote() {
        // Use backend search for remote
        match backend.search_files(query) {
            Ok(hits) => hits
                .into_iter()
                .take(500)
                .map(|(path_str, line_num, line_text)| {
                    let path = root.join(&path_str);
                    let match_start = regex.find(&line_text).map(|m| m.start()).unwrap_or(0);
                    SearchResult {
                        path,
                        line_num: line_num as u32,
                        line_text,
                        match_start,
                        match_len: query.len(),
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    } else {
        let mut results = Vec::new();

        let walker = ignore::WalkBuilder::new(root).build();

        for entry in walker.flatten() {
            if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();

            // Skip binary files
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                match ext {
                    "png" | "jpg" | "jpeg" | "gif" | "ico" | "woff" | "woff2" | "ttf" | "otf"
                    | "eot" | "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "exe" | "dll" | "so"
                    | "dylib" | "o" | "a" | "class" | "pyc" | "db" | "sqlite" | "lock" => continue,
                    _ => {}
                }
            }

            if let Ok(content) = backend.read_file(path) {
                for (line_idx, line) in content.lines().enumerate() {
                    if let Some(mat) = regex.find(line) {
                        results.push(SearchResult {
                            path: path.to_path_buf(),
                            line_num: (line_idx + 1) as u32,
                            line_text: line.to_string(),
                            match_start: mat.start(),
                            match_len: query.len(),
                        });
                        if results.len() > 500 {
                            return results;
                        }
                    }
                }
            }
        }

        results
    }
}

/// Replace all occurrences of query with replacement across project files.
fn replace_in_files(
    root: &Path,
    query: &str,
    replacement: &str,
    backend: &dyn FileBackend,
) -> usize {
    let regex = match RegexBuilder::new(&regex::escape(query))
        .case_insensitive(true)
        .build()
    {
        Ok(regex) => regex,
        Err(_) => return 0,
    };

    let mut total = 0;

    let walker = ignore::WalkBuilder::new(root).build();

    for entry in walker.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();

        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            match ext {
                "png" | "jpg" | "jpeg" | "gif" | "ico" | "woff" | "woff2" | "ttf" | "otf"
                | "eot" | "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "exe" | "dll" | "so"
                | "dylib" | "o" | "a" | "class" | "pyc" | "db" | "sqlite" | "lock" => continue,
                _ => {}
            }
        }

        if let Ok(content) = backend.read_file(path) {
            let replacements = regex.find_iter(&content).count();
            if replacements > 0 {
                let new_content = regex.replace_all(&content, replacement).to_string();
                total += replacements;
                let _ = backend.write_file(path, &new_content);
            }
        }
    }

    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::editor::file_backend::LocalFileBackend;
    use tempfile::tempdir;

    #[test]
    fn replace_in_files_is_case_insensitive_and_updates_content() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("sample.txt");
        std::fs::write(&file, "Alpha\nALPHA\nbeta\n").unwrap();
        let backend = LocalFileBackend::new(dir.path());

        let replaced = replace_in_files(dir.path(), "alpha", "omega", &backend);
        let content = std::fs::read_to_string(&file).unwrap();

        assert_eq!(replaced, 2);
        assert_eq!(content, "omega\nomega\nbeta\n");
    }

    #[test]
    fn search_in_files_returns_match_offsets_from_regex() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("sample.txt");
        std::fs::write(&file, "123 AbC 456\n").unwrap();
        let backend = LocalFileBackend::new(dir.path());

        let results = search_in_files(dir.path(), "abc", &backend);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, file);
        assert_eq!(results[0].line_num, 1);
        assert_eq!(results[0].match_start, 4);
    }
}
