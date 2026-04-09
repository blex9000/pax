use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use super::file_backend::FileBackend;
use super::task::run_blocking;

/// Callback when a commit is clicked: (commit_hash)
pub type OnCommitClick = Rc<dyn Fn(&str)>;

/// Git log view showing commit history with search filter and clickable entries.
pub struct GitLogView {
    pub widget: gtk4::Box,
    list_box: gtk4::ListBox,
    search_entry: gtk4::SearchEntry,
    #[allow(dead_code)]
    filter_label: gtk4::Label,
    backend: Arc<dyn FileBackend>,
    on_commit_click: OnCommitClick,
    /// Current file filter (None = show all commits)
    file_filter: Rc<RefCell<Option<String>>>,
    request_seq: Rc<Cell<u64>>,
}

struct CommitEntry {
    hash: String,
    subject: String,
    author: String,
    date: String,
}

impl GitLogView {
    pub fn new(
        _root_dir: &Path,
        on_commit_click: OnCommitClick,
        backend: Arc<dyn FileBackend>,
    ) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("editor-sidebar-pane");

        let header = gtk4::Label::new(Some("History"));
        header.add_css_class("heading");
        header.set_halign(gtk4::Align::Start);
        header.set_margin_start(8);
        header.set_margin_top(4);
        header.set_margin_bottom(4);
        container.append(&header);

        // Search entry for filtering by file path
        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Filter by file path..."));
        search_entry.set_margin_start(4);
        search_entry.set_margin_end(4);
        search_entry.set_margin_bottom(4);
        container.append(&search_entry);

        // Active filter indicator (hidden when no filter)
        let filter_label = gtk4::Label::new(None);
        filter_label.add_css_class("dim-label");
        filter_label.add_css_class("caption");
        filter_label.set_halign(gtk4::Align::Start);
        filter_label.set_margin_start(8);
        filter_label.set_margin_bottom(2);
        filter_label.set_visible(false);
        container.append(&filter_label);

        let list_box = gtk4::ListBox::new();
        list_box.set_selection_mode(gtk4::SelectionMode::Single);
        list_box.add_css_class("editor-sidebar-pane-list");

        let scroll = gtk4::ScrolledWindow::new();
        scroll.add_css_class("editor-sidebar-pane-scroll");
        scroll.set_child(Some(&list_box));
        scroll.set_vexpand(true);
        container.append(&scroll);

        let file_filter: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let request_seq = Rc::new(Cell::new(0));

        let view = Self {
            widget: container,
            list_box,
            search_entry: search_entry.clone(),
            filter_label: filter_label.clone(),
            backend: backend.clone(),
            on_commit_click,
            file_filter: file_filter.clone(),
            request_seq: request_seq.clone(),
        };

        {
            let cb = view.on_commit_click.clone();
            view.list_box.connect_row_activated(move |_, row| {
                let hash = row.widget_name();
                let hash_str = hash.as_str();
                if !hash_str.is_empty() {
                    cb(hash_str);
                }
            });
        }

        // Search on Enter or search-changed (debounced by GTK)
        {
            let be = backend.clone();
            let lb = view.list_box.clone();
            let ff = file_filter.clone();
            let fl = filter_label.clone();
            let seq = request_seq.clone();
            search_entry.connect_search_changed(move |entry| {
                let query = entry.text().to_string();
                let filter = if query.trim().is_empty() {
                    None
                } else {
                    Some(query.trim().to_string())
                };
                *ff.borrow_mut() = filter.clone();
                if let Some(ref f) = filter {
                    fl.set_text(&format!("Showing commits for: {}", f));
                    fl.set_visible(true);
                } else {
                    fl.set_visible(false);
                }
                request_commits(&lb, be.clone(), filter, seq.clone());
            });
        }

        // Deferred for remote backends to avoid blocking UI
        if !view.backend.is_remote() {
            view.refresh();
        }
        view
    }

    /// Refresh the commit list.
    pub fn refresh(&self) {
        let filter = self.file_filter.borrow().clone();
        request_commits(
            &self.list_box,
            self.backend.clone(),
            filter,
            self.request_seq.clone(),
        );
    }

    /// Filter commits by a specific file path (relative to root).
    pub fn filter_by_file(&self, rel_path: &str) {
        self.search_entry.set_text(rel_path);
        // search_changed signal will trigger the actual filtering
    }

    /// Clear the file filter and show all commits.
    pub fn clear_filter(&self) {
        self.search_entry.set_text("");
    }
}

fn load_commits(backend: &dyn FileBackend, file_filter: Option<&str>) -> Vec<CommitEntry> {
    let mut args: Vec<&str> = vec!["log", "--format=%h\x1f%s\x1f%an\x1f%ar", "-200"];
    let follow_str;
    let dashdash_str;
    let file_owned: String;
    if let Some(file) = file_filter {
        follow_str = "--follow";
        dashdash_str = "--";
        file_owned = file.to_string();
        args.push(follow_str);
        args.push(dashdash_str);
        args.push(&file_owned);
    }

    let stdout = match backend.git_command(&args) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '\x1f').collect();
            if parts.len() == 4 {
                Some(CommitEntry {
                    hash: parts[0].to_string(),
                    subject: parts[1].to_string(),
                    author: parts[2].to_string(),
                    date: parts[3].to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}

fn request_commits(
    list_box: &gtk4::ListBox,
    backend: Arc<dyn FileBackend>,
    filter: Option<String>,
    request_seq: Rc<Cell<u64>>,
) {
    let request_id = request_seq.get().wrapping_add(1);
    request_seq.set(request_id);
    populate_message(list_box, "Loading history...");

    let list_box = list_box.clone();
    run_blocking(
        move || load_commits(&*backend, filter.as_deref()),
        move |commits| {
            if request_seq.get() != request_id {
                return;
            }
            populate_list(&list_box, &commits);
        },
    );
}

fn populate_message(list_box: &gtk4::ListBox, message: &str) {
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }
    let label = gtk4::Label::new(Some(message));
    label.add_css_class("dim-label");
    label.set_margin_top(16);
    list_box.append(&label);
}

fn populate_list(list_box: &gtk4::ListBox, commits: &[CommitEntry]) {
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }

    if commits.is_empty() {
        populate_message(list_box, "No commits found");
        return;
    }

    for commit in commits {
        let row_box = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
        row_box.set_margin_start(8);
        row_box.set_margin_end(8);
        row_box.set_margin_top(3);
        row_box.set_margin_bottom(3);

        let top_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

        let hash_label = gtk4::Label::new(Some(&commit.hash));
        hash_label.add_css_class("dim-label");
        hash_label.add_css_class("monospace");
        hash_label.set_halign(gtk4::Align::Start);
        top_row.append(&hash_label);

        let subject_label = gtk4::Label::new(Some(&commit.subject));
        subject_label.set_halign(gtk4::Align::Start);
        subject_label.set_hexpand(true);
        subject_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        top_row.append(&subject_label);

        row_box.append(&top_row);

        let meta = format!("{} · {}", commit.author, commit.date);
        let meta_label = gtk4::Label::new(Some(&meta));
        meta_label.add_css_class("dim-label");
        meta_label.add_css_class("caption");
        meta_label.set_halign(gtk4::Align::Start);
        row_box.append(&meta_label);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&row_box));
        row.set_widget_name(&commit.hash);
        row.set_tooltip_text(Some(&format!(
            "{}\n\n{}\n\n{}  ·  {}",
            commit.hash, commit.subject, commit.author, commit.date
        )));
        list_box.append(&row);
    }
}
