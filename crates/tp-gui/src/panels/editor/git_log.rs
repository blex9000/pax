use gtk4::prelude::*;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Callback when a commit is clicked: (commit_hash)
pub type OnCommitClick = Rc<dyn Fn(&str)>;

/// Git log view showing commit history with clickable entries.
pub struct GitLogView {
    pub widget: gtk4::Box,
    list_box: gtk4::ListBox,
    root_dir: PathBuf,
    #[allow(dead_code)]
    on_commit_click: OnCommitClick,
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

        let list_box = gtk4::ListBox::new();
        list_box.set_selection_mode(gtk4::SelectionMode::Single);
        list_box.add_css_class("navigation-sidebar");

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_child(Some(&list_box));
        scroll.set_vexpand(true);
        container.append(&scroll);

        let view = Self {
            widget: container,
            list_box,
            root_dir: root_dir.to_path_buf(),
            on_commit_click,
        };
        view.refresh();
        view
    }

    /// Refresh the commit list from git log.
    pub fn refresh(&self) {
        let commits = load_commits(&self.root_dir);
        populate_list(&self.list_box, &commits, &self.on_commit_click);
    }
}

fn load_commits(root: &Path) -> Vec<CommitEntry> {
    let output = std::process::Command::new("git")
        .args(["log", "--format=%h\x1f%s\x1f%an\x1f%ar", "-200"])
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

    for commit in commits {
        let row_box = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
        row_box.set_margin_start(8);
        row_box.set_margin_end(8);
        row_box.set_margin_top(3);
        row_box.set_margin_bottom(3);

        // First line: hash + subject
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

        // Second line: author + date
        let meta = format!("{} · {}", commit.author, commit.date);
        let meta_label = gtk4::Label::new(Some(&meta));
        meta_label.add_css_class("dim-label");
        meta_label.add_css_class("caption");
        meta_label.set_halign(gtk4::Align::Start);
        row_box.append(&meta_label);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&row_box));
        // Store hash in widget name for retrieval on click
        row.set_widget_name(&commit.hash);
        list_box.append(&row);
    }

    // Connect row activation
    let cb = on_click.clone();
    list_box.connect_row_activated(move |_, row| {
        let hash = row.widget_name();
        let hash_str = hash.as_str();
        if !hash_str.is_empty() {
            cb(hash_str);
        }
    });
}
