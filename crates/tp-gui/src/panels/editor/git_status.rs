use gtk4::prelude::*;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Callback when a changed file is clicked (to show diff).
pub type OnDiffOpen = Rc<dyn Fn(&Path, &str)>; // (path, git_status_char)

/// Git status sidebar widget.
pub struct GitStatusView {
    pub widget: gtk4::Box,
    list_box: gtk4::ListBox,
    commit_entry: gtk4::Entry,
    commit_btn: gtk4::Button,
    root_dir: PathBuf,
    on_diff_open: OnDiffOpen,
}

#[derive(Debug, Clone)]
struct GitFileEntry {
    path: PathBuf,
    status: String,      // "M", "A", "D", "??"
    staged: bool,
}

impl GitStatusView {
    pub fn new(root_dir: &Path, on_diff_open: OnDiffOpen) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        let header = gtk4::Label::new(Some("Changes"));
        header.add_css_class("heading");
        header.set_halign(gtk4::Align::Start);
        header.set_margin_start(8);
        header.set_margin_top(4);
        header.set_margin_bottom(4);
        container.append(&header);

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_vexpand(true);

        let list_box = gtk4::ListBox::new();
        list_box.add_css_class("navigation-sidebar");
        scroll.set_child(Some(&list_box));
        container.append(&scroll);

        // Commit section
        container.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        let commit_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        commit_box.set_margin_start(4);
        commit_box.set_margin_end(4);
        commit_box.set_margin_top(4);
        commit_box.set_margin_bottom(4);

        let commit_entry = gtk4::Entry::new();
        commit_entry.set_placeholder_text(Some("Commit message..."));
        commit_box.append(&commit_entry);

        let commit_btn = gtk4::Button::with_label("Commit");
        commit_btn.add_css_class("suggested-action");
        commit_btn.set_sensitive(false);
        commit_box.append(&commit_btn);

        container.append(&commit_box);

        // Enable commit button when message is non-empty
        {
            let btn = commit_btn.clone();
            commit_entry.connect_changed(move |entry| {
                btn.set_sensitive(!entry.text().is_empty());
            });
        }

        // Commit action
        {
            let root = root_dir.to_path_buf();
            let entry = commit_entry.clone();
            commit_btn.connect_clicked(move |_btn| {
                let msg = entry.text().to_string();
                if msg.is_empty() { return; }
                let output = std::process::Command::new("git")
                    .args(["commit", "-m", &msg])
                    .current_dir(&root)
                    .output();
                match output {
                    Ok(o) if o.status.success() => {
                        entry.set_text("");
                        tracing::info!("Committed: {}", msg);
                    }
                    Ok(o) => {
                        tracing::warn!("git commit failed: {}", String::from_utf8_lossy(&o.stderr));
                    }
                    Err(e) => {
                        tracing::error!("git commit error: {}", e);
                    }
                }
            });
        }

        Self {
            widget: container,
            list_box,
            commit_entry,
            commit_btn,
            root_dir: root_dir.to_path_buf(),
            on_diff_open,
        }
    }

    /// Update the git status list from `git status --porcelain` output.
    pub fn update(&self, porcelain_output: &str) {
        // Clear existing
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }

        let entries = parse_porcelain(porcelain_output, &self.root_dir);

        if entries.is_empty() {
            let label = gtk4::Label::new(Some("No changes"));
            label.add_css_class("dim-label");
            label.set_margin_top(16);
            self.list_box.append(&label);
            return;
        }

        for entry in &entries {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            row.set_margin_start(4);
            row.set_margin_end(4);
            row.set_margin_top(2);
            row.set_margin_bottom(2);

            // Status badge
            let status_label = gtk4::Label::new(Some(&entry.status));
            status_label.set_width_chars(2);
            let color_class = match entry.status.as_str() {
                "M" | "MM" => "warning",
                "A" => "success",
                "D" => "error",
                "??" => "dim-label",
                _ => "dim-label",
            };
            status_label.add_css_class(color_class);
            row.append(&status_label);

            // File name
            let rel = entry.path.strip_prefix(&self.root_dir).unwrap_or(&entry.path);
            let name_label = gtk4::Label::new(Some(&rel.to_string_lossy()));
            name_label.set_halign(gtk4::Align::Start);
            name_label.set_hexpand(true);
            name_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
            row.append(&name_label);

            // Stage/unstage button
            let stage_btn = gtk4::Button::new();
            stage_btn.add_css_class("flat");
            if entry.staged {
                stage_btn.set_icon_name("list-remove-symbolic");
                stage_btn.set_tooltip_text(Some("Unstage"));
                let path = entry.path.clone();
                let root = self.root_dir.clone();
                stage_btn.connect_clicked(move |_| {
                    let _ = std::process::Command::new("git")
                        .args(["restore", "--staged", &path.to_string_lossy()])
                        .current_dir(&root)
                        .output();
                });
            } else {
                stage_btn.set_icon_name("list-add-symbolic");
                stage_btn.set_tooltip_text(Some("Stage"));
                let path = entry.path.clone();
                let root = self.root_dir.clone();
                stage_btn.connect_clicked(move |_| {
                    let _ = std::process::Command::new("git")
                        .args(["add", &path.to_string_lossy()])
                        .current_dir(&root)
                        .output();
                });
            }
            row.append(&stage_btn);

            self.list_box.append(&row);
        }

        // Make rows clickable to open diff
        {
            let entries_c = entries.clone();
            let on_diff = self.on_diff_open.clone();
            self.list_box.connect_row_activated(move |_, row| {
                let idx = row.index() as usize;
                if let Some(entry) = entries_c.get(idx) {
                    on_diff(&entry.path, &entry.status);
                }
            });
        }
    }
}

fn parse_porcelain(output: &str, root: &Path) -> Vec<GitFileEntry> {
    output.lines().filter_map(|line| {
        if line.len() < 4 { return None; }
        let index_status = line.chars().nth(0).unwrap_or(' ');
        let work_status = line.chars().nth(1).unwrap_or(' ');
        let file_path = line[3..].trim();

        let staged = index_status != ' ' && index_status != '?';
        let status = if index_status == '?' && work_status == '?' {
            "??".to_string()
        } else if staged {
            index_status.to_string()
        } else {
            work_status.to_string()
        };

        Some(GitFileEntry {
            path: root.join(file_path),
            status,
            staged,
        })
    }).collect()
}
