use gtk4::prelude::*;
use gtk4::glib;
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

use super::EditorState;

/// Start all file watchers. Call once during CodeEditorPanel construction.
pub fn start_watchers(
    state: Rc<RefCell<EditorState>>,
    info_bar_container: gtk4::Box,
    on_tree_changed: Rc<dyn Fn()>,
    on_git_changed: Rc<dyn Fn(String)>,
) {
    start_open_file_watcher(state.clone(), info_bar_container);
    start_tree_watcher(state.clone(), on_tree_changed);
    start_git_watcher(state, on_git_changed);
}

/// Watch open files for external changes (1s interval).
fn start_open_file_watcher(
    state: Rc<RefCell<EditorState>>,
    info_bar_container: gtk4::Box,
) {
    glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
        let mut st = state.borrow_mut();
        for open_file in &mut st.open_files {
            let current_mtime = get_mtime(&open_file.path);
            if current_mtime != open_file.last_disk_mtime && current_mtime != 0 {
                open_file.last_disk_mtime = current_mtime;
                if !open_file.modified {
                    // Silent reload
                    if let Ok(content) = std::fs::read_to_string(&open_file.path) {
                        open_file.buffer.set_text(&content);
                        open_file.buffer.set_enable_undo(false);
                        open_file.buffer.set_enable_undo(true);
                    }
                } else {
                    // Show info bar for conflict
                    show_conflict_bar(&info_bar_container, &open_file.path, &open_file.buffer);
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

/// Watch file tree for structural changes (2s interval).
fn start_tree_watcher(
    state: Rc<RefCell<EditorState>>,
    on_changed: Rc<dyn Fn()>,
) {
    let last_hash = Rc::new(Cell::new(0u64));
    glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
        let root = state.borrow().root_dir.clone();
        let hash = dir_hash(&root);
        if hash != last_hash.get() {
            last_hash.set(hash);
            on_changed();
        }
        glib::ControlFlow::Continue
    });
}

/// Watch git status (3s interval).
fn start_git_watcher(
    state: Rc<RefCell<EditorState>>,
    on_changed: Rc<dyn Fn(String)>,
) {
    let last_output = Rc::new(RefCell::new(String::new()));
    glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
        let root = state.borrow().root_dir.clone();
        if let Ok(output) = std::process::Command::new("git")
            .arg("status")
            .arg("--porcelain")
            .current_dir(&root)
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if stdout != *last_output.borrow() {
                *last_output.borrow_mut() = stdout.clone();
                on_changed(stdout);
            }
        }
        glib::ControlFlow::Continue
    });
}

#[allow(deprecated)]
fn show_conflict_bar(container: &gtk4::Box, path: &Path, buffer: &sourceview5::Buffer) {
    // Remove any existing info bar
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let bar = gtk4::InfoBar::new();
    bar.set_message_type(gtk4::MessageType::Warning);
    bar.set_show_close_button(true);

    let label = gtk4::Label::new(Some(&format!(
        "\"{}\" changed on disk.",
        path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    )));
    bar.add_child(&label);

    bar.add_button("Reload", gtk4::ResponseType::Accept);
    bar.add_button("Keep Mine", gtk4::ResponseType::Reject);

    let path_c = path.to_path_buf();
    let buf_c = buffer.clone();
    let container_c = container.clone();
    bar.connect_response(move |bar, response| {
        if response == gtk4::ResponseType::Accept {
            if let Ok(content) = std::fs::read_to_string(&path_c) {
                buf_c.set_text(&content);
                buf_c.set_enable_undo(false);
                buf_c.set_enable_undo(true);
            }
        }
        container_c.remove(bar);
    });

    bar.connect_close(move |bar| {
        if let Some(parent) = bar.parent() {
            if let Some(bx) = parent.downcast_ref::<gtk4::Box>() {
                bx.remove(bar);
            }
        }
    });

    container.append(&bar);
}

fn get_mtime(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
        .unwrap_or(0)
}

/// Quick hash of directory structure (paths + mtimes) for change detection.
fn dir_hash(dir: &Path) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    let walker = ignore::WalkBuilder::new(dir)
        .max_depth(Some(5))
        .build();

    for entry in walker.flatten() {
        entry.path().hash(&mut hasher);
        get_mtime(entry.path()).hash(&mut hasher);
    }
    hasher.finish()
}
