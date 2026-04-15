use gtk4::prelude::*;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use super::file_backend::FileBackend;

/// Callback when a changed file is clicked (to show diff).
pub type OnDiffOpen = Rc<dyn Fn(&Path, &str)>; // (path, git_status_char)

/// Callback to trigger after any git action (stage, unstage, commit, revert).
pub type OnGitAction = Rc<dyn Fn()>;

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
    backend: Arc<dyn FileBackend>,
    on_git_action: OnGitAction,
}

#[derive(Debug, Clone)]
struct GitFileEntry {
    path: PathBuf,
    status: String, // "M", "A", "D", "??"
    staged: bool,
}

impl GitStatusView {
    pub fn new(
        root_dir: &Path,
        on_diff_open: OnDiffOpen,
        backend: Arc<dyn FileBackend>,
        on_git_action: OnGitAction,
    ) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("editor-sidebar-pane");

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.add_css_class("editor-sidebar-pane-scroll");

        let list_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        list_container.add_css_class("editor-sidebar-pane-content");
        let loading_label = gtk4::Label::new(Some("Loading git status..."));
        loading_label.add_css_class("dim-label");
        loading_label.set_margin_top(16);
        list_container.append(&loading_label);
        scroll.set_child(Some(&list_container));
        container.append(&scroll);

        // Commit section
        container.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        let commit_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        commit_box.add_css_class("editor-sidebar-pane-footer");
        commit_box.set_margin_start(4);
        commit_box.set_margin_end(4);
        commit_box.set_margin_top(4);
        commit_box.set_margin_bottom(4);

        let commit_entry = gtk4::Entry::new();
        commit_entry.set_placeholder_text(Some("Commit message..."));
        commit_box.append(&commit_entry);

        // Action row: compact buttons aligned to the right.
        let action_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        action_row.set_halign(gtk4::Align::End);

        let stage_all_btn = gtk4::Button::with_label("Stage All");
        stage_all_btn.set_tooltip_text(Some("Stage all changes — equivalent to `git add -A`"));
        action_row.append(&stage_all_btn);

        let commit_btn = gtk4::Button::with_label("Commit");
        commit_btn.add_css_class("suggested-action");
        commit_btn.set_sensitive(false);
        action_row.append(&commit_btn);

        commit_box.append(&action_row);

        container.append(&commit_box);

        // Enable commit button when message is non-empty
        {
            let btn = commit_btn.clone();
            commit_entry.connect_changed(move |entry| {
                btn.set_sensitive(!entry.text().is_empty());
            });
        }

        // Stage All action: `git add -A` then refresh
        {
            let be = backend.clone();
            let action_cb = on_git_action.clone();
            stage_all_btn.connect_clicked(move |_| {
                match be.git_command(&["add", "-A"]) {
                    Ok(_) => {
                        tracing::info!("Staged all changes");
                        action_cb();
                    }
                    Err(e) => {
                        tracing::warn!("git add -A failed: {}", e);
                    }
                }
            });
        }

        // Commit action
        {
            let be = backend.clone();
            let entry = commit_entry.clone();
            let action_cb = on_git_action.clone();
            commit_btn.connect_clicked(move |_btn| {
                let msg = entry.text().to_string();
                if msg.is_empty() {
                    return;
                }
                match be.git_command(&["commit", "-m", &msg]) {
                    Ok(_) => {
                        entry.set_text("");
                        tracing::info!("Committed: {}", msg);
                        action_cb();
                    }
                    Err(e) => {
                        tracing::warn!("git commit failed: {}", e);
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
            backend,
            on_git_action,
        };

        view
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

            let rel = entry
                .path
                .strip_prefix(&self.root_dir)
                .unwrap_or(&entry.path);
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

            // Stage/unstage icon button on the right of the top row
            if entry.staged {
                let btn = gtk4::Button::from_icon_name("list-remove-symbolic");
                btn.add_css_class("flat");
                btn.set_tooltip_text(Some("Unstage — remove this file from the next commit"));
                let path = entry.path.clone();
                let be = self.backend.clone();
                let cb = self.on_git_action.clone();
                btn.connect_clicked(move |_| {
                    let _ = be.git_command(&["restore", "--staged", &path.to_string_lossy()]);
                    cb();
                });
                top_row.append(&btn);
            } else {
                let btn = gtk4::Button::from_icon_name("list-add-symbolic");
                btn.add_css_class("flat");
                btn.set_tooltip_text(Some("Stage — add this file to the next commit"));
                let path = entry.path.clone();
                let be = self.backend.clone();
                let cb = self.on_git_action.clone();
                btn.connect_clicked(move |_| {
                    let _ = be.git_command(&["add", &path.to_string_lossy()]);
                    cb();
                });
                top_row.append(&btn);
            }
            self.list_container.append(&outer);
            self.list_container
                .append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
        }
    }
}

fn parse_porcelain(output: &str, root: &Path) -> Vec<GitFileEntry> {
    output
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let index_status = line.chars().nth(0).unwrap_or(' ');
            let work_status = line.chars().nth(1).unwrap_or(' ');
            let raw_path = line[3..].trim();

            let staged = index_status != ' ' && index_status != '?';
            let status = if index_status == '?' && work_status == '?' {
                "??".to_string()
            } else if staged {
                index_status.to_string()
            } else {
                work_status.to_string()
            };
            let file_path = if matches!(status.as_str(), "R" | "C") {
                raw_path
                    .rsplit_once(" -> ")
                    .map(|(_, new_path)| new_path)
                    .unwrap_or(raw_path)
            } else {
                raw_path
            };

            Some(GitFileEntry {
                path: root.join(file_path),
                status,
                staged,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_porcelain_uses_destination_path_for_renames() {
        let root = Path::new("/tmp/repo");
        let entries = parse_porcelain("R  old/name.txt -> new/name.txt", root);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "R");
        assert_eq!(entries[0].path, root.join("new/name.txt"));
        assert!(entries[0].staged);
    }

    #[test]
    fn parse_porcelain_keeps_untracked_files() {
        let root = Path::new("/tmp/repo");
        let entries = parse_porcelain("?? scratch.txt", root);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "??");
        assert_eq!(entries[0].path, root.join("scratch.txt"));
        assert!(!entries[0].staged);
    }
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
pub fn compute_diff(backend: &dyn FileBackend, file_path: &Path) -> Vec<DiffHunk> {
    // Get HEAD version
    let root = backend.root();
    let rel = file_path.strip_prefix(root).unwrap_or(file_path);
    let old_content = backend
        .git_show(&format!("HEAD:{}", rel.to_string_lossy()))
        .unwrap_or_default();

    // Get working version
    let new_content = backend.read_file(file_path).unwrap_or_default();

    let diff = similar::TextDiff::from_lines(&old_content, &new_content);
    let mut hunks = Vec::new();

    for group in diff.grouped_ops(3) {
        let mut old_lines = Vec::new();
        let mut new_lines = Vec::new();
        let mut old_start = 0;
        let mut new_start = 0;

        for op in &group {
            match op {
                similar::DiffOp::Equal {
                    old_index,
                    new_index,
                    len,
                } => {
                    if old_start == 0 {
                        old_start = *old_index + 1;
                    }
                    if new_start == 0 {
                        new_start = *new_index + 1;
                    }
                    for i in 0..*len {
                        let line = diff.old_slices()[old_index + i].to_string();
                        old_lines.push(format!(" {}", line));
                        new_lines.push(format!(" {}", line));
                    }
                }
                similar::DiffOp::Delete {
                    old_index, old_len, ..
                } => {
                    if old_start == 0 {
                        old_start = *old_index + 1;
                    }
                    for i in 0..*old_len {
                        old_lines.push(format!("-{}", diff.old_slices()[old_index + i]));
                    }
                }
                similar::DiffOp::Insert {
                    new_index, new_len, ..
                } => {
                    if new_start == 0 {
                        new_start = *new_index + 1;
                    }
                    for i in 0..*new_len {
                        new_lines.push(format!("+{}", diff.new_slices()[new_index + i]));
                    }
                }
                similar::DiffOp::Replace {
                    old_index,
                    old_len,
                    new_index,
                    new_len,
                } => {
                    if old_start == 0 {
                        old_start = *old_index + 1;
                    }
                    if new_start == 0 {
                        new_start = *new_index + 1;
                    }
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
pub fn revert_hunk(
    backend: &dyn FileBackend,
    file_path: &Path,
    hunk: &DiffHunk,
) -> Result<(), String> {
    let content = backend
        .read_file(file_path)
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
    let new_actual_count = hunk
        .new_lines
        .iter()
        .filter(|l| l.starts_with('+') || l.starts_with(' '))
        .count();
    i += new_actual_count;

    // Lines after the hunk
    while i < lines.len() {
        result.push(lines[i].to_string());
        i += 1;
    }

    let output = result.join("\n");
    backend
        .write_file(file_path, &output)
        .map_err(|e| format!("Cannot write file: {}", e))
}
