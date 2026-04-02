use gtk4::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Callback when a commit is clicked: (commit_hash)
pub type OnCommitClick = Rc<dyn Fn(&str)>;

/// Git log view showing commit history with search filter and clickable entries.
pub struct GitLogView {
    pub widget: gtk4::Box,
    list_box: gtk4::ListBox,
    search_entry: gtk4::SearchEntry,
    #[allow(dead_code)]
    filter_label: gtk4::Label,
    root_dir: PathBuf,
    on_commit_click: OnCommitClick,
    /// Current file filter (None = show all commits)
    file_filter: Rc<RefCell<Option<String>>>,
}

struct CommitEntry {
    hash: String,
    subject: String,
    author: String,
    date: String,
}

impl GitLogView {
    pub fn new(root_dir: &Path, on_commit_click: OnCommitClick) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

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

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_child(Some(&list_box));
        scroll.set_vexpand(true);
        container.append(&scroll);

        let file_filter: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        let view = Self {
            widget: container,
            list_box,
            search_entry: search_entry.clone(),
            filter_label: filter_label.clone(),
            root_dir: root_dir.to_path_buf(),
            on_commit_click,
            file_filter: file_filter.clone(),
        };

        // Search on Enter or search-changed (debounced by GTK)
        {
            let root = root_dir.to_path_buf();
            let lb = view.list_box.clone();
            let cb = view.on_commit_click.clone();
            let ff = file_filter.clone();
            let fl = filter_label.clone();
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
                let commits = load_commits(&root, filter.as_deref());
                populate_list(&lb, &commits, &cb);
            });
        }

        view.refresh();
        view
    }

    /// Refresh the commit list.
    pub fn refresh(&self) {
        let filter = self.file_filter.borrow().clone();
        let commits = load_commits(&self.root_dir, filter.as_deref());
        populate_list(&self.list_box, &commits, &self.on_commit_click);
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

fn load_commits(root: &Path, file_filter: Option<&str>) -> Vec<CommitEntry> {
    let mut args = vec![
        "log".to_string(),
        format!("--format=%h\x1f%s\x1f%an\x1f%ar"),
        "-200".to_string(),
    ];
    if let Some(file) = file_filter {
        args.push("--follow".to_string());
        args.push("--".to_string());
        args.push(file.to_string());
    }

    let output = std::process::Command::new("git")
        .args(&args)
        .current_dir(root)
        .output();

    let Ok(output) = output else { return Vec::new() };
    if !output.status.success() { return Vec::new(); }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().filter_map(|line| {
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
    }).collect()
}

fn populate_list(list_box: &gtk4::ListBox, commits: &[CommitEntry], on_click: &OnCommitClick) {
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }

    if commits.is_empty() {
        let label = gtk4::Label::new(Some("No commits found"));
        label.add_css_class("dim-label");
        label.set_margin_top(16);
        list_box.append(&label);
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

    let cb = on_click.clone();
    list_box.connect_row_activated(move |_, row| {
        let hash = row.widget_name();
        let hash_str = hash.as_str();
        if !hash_str.is_empty() {
            cb(hash_str);
        }
    });
}
