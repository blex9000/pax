use gtk4::prelude::*;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Callback when a changed file is clicked (to show diff).
pub type OnDiffOpen = Rc<dyn Fn(&Path, &str)>; // (path, git_status_char)

/// Git status sidebar widget.
pub struct GitStatusView {
    pub widget: gtk4::Box,
    list_container: gtk4::Box,
    #[allow(dead_code)]
    commit_entry: gtk4::Entry,
    #[allow(dead_code)]
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

        let list_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        scroll.set_child(Some(&list_container));
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

        let view = Self {
            widget: container,
            list_container,
            commit_entry,
            commit_btn,
            root_dir: root_dir.to_path_buf(),
            on_diff_open,
        };

        // Initial population
        view.refresh();

        view
    }

    /// Refresh by running git status.
    pub fn refresh(&self) {
        tracing::info!("GitStatusView::refresh() root_dir={}", self.root_dir.display());
        let output = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&self.root_dir)
            .output();
        match &output {
            Ok(o) => {
                tracing::info!("git status exit={} stdout_len={}", o.status, o.stdout.len());
                if o.status.success() {
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    self.update(&stdout);
                }
            }
            Err(e) => {
                tracing::error!("git status failed: {}", e);
            }
        }
    }

    /// Update the git status list from `git status --porcelain` output.
    pub fn update(&self, porcelain_output: &str) {
        // Clear existing
        while let Some(child) = self.list_container.first_child() {
            self.list_container.remove(&child);
        }

        let entries = parse_porcelain(porcelain_output, &self.root_dir);

        if entries.is_empty() {
            let label = gtk4::Label::new(Some("No changes"));
            label.add_css_class("dim-label");
            label.set_margin_top(16);
            self.list_container.append(&label);
            return;
        }

        for entry in &entries {
            let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            outer.set_margin_start(6);
            outer.set_margin_end(6);
            outer.set_margin_top(4);
            outer.set_margin_bottom(4);

            // Top row: badge + filename (clickable for diff)
            let top_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

            let (badge_text, badge_class) = match entry.status.as_str() {
                "M" | "MM" => ("M", "warning"),
                "A" => ("A", "success"),
                "D" => ("D", "error"),
                "R" => ("R", "accent"),
                "??" => ("U", "dim-label"),
                other => (other, "dim-label"),
            };
            let badge = gtk4::Label::new(Some(badge_text));
            badge.add_css_class("monospace");
            badge.add_css_class(badge_class);
            top_row.append(&badge);

            let rel = entry.path.strip_prefix(&self.root_dir).unwrap_or(&entry.path);
            let rel_str = rel.to_string_lossy().to_string();

            // Make filename a link button for opening diff
            let name_btn = gtk4::Button::with_label(&rel_str);
            name_btn.add_css_class("flat");
            name_btn.set_halign(gtk4::Align::Start);
            name_btn.set_hexpand(true);
            name_btn.set_tooltip_text(Some("Click to view diff"));
            // Ellipsize the label inside the button
            if let Some(child) = name_btn.child() {
                if let Some(label) = child.downcast_ref::<gtk4::Label>() {
                    label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
                    label.set_halign(gtk4::Align::Start);
                }
            }
            {
                let path = entry.path.clone();
                let status = entry.status.clone();
                let on_diff = self.on_diff_open.clone();
                name_btn.connect_clicked(move |_| {
                    on_diff(&path, &status);
                });
            }
            top_row.append(&name_btn);
            outer.append(&top_row);

            // Bottom row: action buttons
            let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            btn_row.set_halign(gtk4::Align::End);

            let stage_btn = gtk4::Button::new();
            stage_btn.add_css_class("flat");
            if entry.staged {
                stage_btn.set_icon_name("list-remove-symbolic");
                stage_btn.set_label("Unstage");
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
                stage_btn.set_label("Stage");
                let path = entry.path.clone();
                let root = self.root_dir.clone();
                stage_btn.connect_clicked(move |_| {
                    let _ = std::process::Command::new("git")
                        .args(["add", &path.to_string_lossy()])
                        .current_dir(&root)
                        .output();
                });
            }
            btn_row.append(&stage_btn);

            if entry.status != "??" {
                let revert_btn = gtk4::Button::new();
                revert_btn.set_icon_name("edit-undo-symbolic");
                revert_btn.set_label("Revert");
                revert_btn.add_css_class("flat");
                revert_btn.add_css_class("destructive-action");
                let path = entry.path.clone();
                let root = self.root_dir.clone();
                revert_btn.connect_clicked(move |_| {
                    let rel = path.strip_prefix(&root).unwrap_or(&path);
                    let _ = std::process::Command::new("git")
                        .args(["checkout", "--", &rel.to_string_lossy()])
                        .current_dir(&root)
                        .output();
                });
                btn_row.append(&revert_btn);
            }

            outer.append(&btn_row);
            self.list_container.append(&outer);
            self.list_container.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
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

/// Represents a diff hunk.
#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
}

/// Get diff hunks for a file using the `similar` crate.
pub fn compute_diff(root: &Path, file_path: &Path) -> Vec<DiffHunk> {
    // Get HEAD version
    let rel = file_path.strip_prefix(root).unwrap_or(file_path);
    let old_content = std::process::Command::new("git")
        .args(["show", &format!("HEAD:{}", rel.to_string_lossy())])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    // Get working version
    let new_content = std::fs::read_to_string(file_path).unwrap_or_default();

    let diff = similar::TextDiff::from_lines(&old_content, &new_content);
    let mut hunks = Vec::new();

    for group in diff.grouped_ops(3) {
        let mut old_lines = Vec::new();
        let mut new_lines = Vec::new();
        let mut old_start = 0;
        let mut new_start = 0;

        for op in &group {
            match op {
                similar::DiffOp::Equal { old_index, new_index, len } => {
                    if old_start == 0 { old_start = *old_index + 1; }
                    if new_start == 0 { new_start = *new_index + 1; }
                    for i in 0..*len {
                        let line = diff.old_slices()[old_index + i].to_string();
                        old_lines.push(format!(" {}", line));
                        new_lines.push(format!(" {}", line));
                    }
                }
                similar::DiffOp::Delete { old_index, old_len, .. } => {
                    if old_start == 0 { old_start = *old_index + 1; }
                    for i in 0..*old_len {
                        old_lines.push(format!("-{}", diff.old_slices()[old_index + i]));
                    }
                }
                similar::DiffOp::Insert { new_index, new_len, .. } => {
                    if new_start == 0 { new_start = *new_index + 1; }
                    for i in 0..*new_len {
                        new_lines.push(format!("+{}", diff.new_slices()[new_index + i]));
                    }
                }
                similar::DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                    if old_start == 0 { old_start = *old_index + 1; }
                    if new_start == 0 { new_start = *new_index + 1; }
                    for i in 0..*old_len {
                        old_lines.push(format!("-{}", diff.old_slices()[old_index + i]));
                    }
                    for i in 0..*new_len {
                        new_lines.push(format!("+{}", diff.new_slices()[new_index + i]));
                    }
                }
            }
        }

        hunks.push(DiffHunk {
            old_start,
            old_count: old_lines.len(),
            new_start,
            new_count: new_lines.len(),
            old_lines,
            new_lines,
        });
    }

    hunks
}

/// Revert a single hunk by restoring old lines at the hunk position.
pub fn revert_hunk(file_path: &Path, hunk: &DiffHunk) -> Result<(), String> {
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| format!("Cannot read file: {}", e))?;
    let lines: Vec<&str> = content.lines().collect();

    let mut result = Vec::new();
    let mut i = 0;
    let hunk_start = hunk.new_start.saturating_sub(1);

    // Lines before the hunk
    while i < hunk_start && i < lines.len() {
        result.push(lines[i].to_string());
        i += 1;
    }

    // Replace with old lines (skip context and removed markers)
    for line in &hunk.old_lines {
        if line.starts_with(' ') || line.starts_with('-') {
            result.push(line[1..].to_string());
        }
    }

    // Skip new lines in the hunk
    let new_actual_count = hunk.new_lines.iter()
        .filter(|l| l.starts_with('+') || l.starts_with(' '))
        .count();
    i += new_actual_count;

    // Lines after the hunk
    while i < lines.len() {
        result.push(lines[i].to_string());
        i += 1;
    }

    let output = result.join("\n");
    std::fs::write(file_path, &output)
        .map_err(|e| format!("Cannot write file: {}", e))
}
