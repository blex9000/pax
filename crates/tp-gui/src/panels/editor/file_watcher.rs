use gtk4::prelude::*;
use gtk4::glib;
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

use super::EditorState;
use super::file_backend::FileBackend;

/// Start all file watchers. Call once during CodeEditorPanel construction.
pub fn start_watchers(
    state: Rc<RefCell<EditorState>>,
    info_bar_container: gtk4::Box,
    on_tree_changed: Rc<dyn Fn()>,
    on_git_changed: Rc<dyn Fn(String)>,
) {
    let backend = state.borrow().backend.clone();
    let is_remote = backend.is_remote();

    start_open_file_watcher(state.clone(), info_bar_container, backend.clone(), is_remote);
    start_tree_watcher(state.clone(), on_tree_changed, backend.clone(), is_remote);
    start_git_watcher(state, on_git_changed, backend, is_remote);
}

/// Watch open files for external changes (1s local, 5s remote).
fn start_open_file_watcher(
    state: Rc<RefCell<EditorState>>,
    info_bar_container: gtk4::Box,
    backend: Rc<dyn FileBackend>,
    is_remote: bool,
) {
    let interval = if is_remote { 5 } else { 1 };
    glib::timeout_add_local(std::time::Duration::from_secs(interval), move || {
        let mut st = state.borrow_mut();
        for open_file in &mut st.open_files {
            let current_mtime = get_mtime(&open_file.path);
            if current_mtime != open_file.last_disk_mtime && current_mtime != 0 {
                open_file.last_disk_mtime = current_mtime;
                if !open_file.modified {
                    // Silent reload
                    if let Ok(content) = backend.read_file(&open_file.path) {
                        open_file.buffer.set_text(&content);
                        open_file.buffer.set_enable_undo(false);
                        open_file.buffer.set_enable_undo(true);
                    }
                } else {
                    // Show info bar for conflict
                    show_conflict_bar(&info_bar_container, &open_file.path, &open_file.buffer, backend.clone());
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

/// Watch file tree for structural changes (2s local, 30s remote).
fn start_tree_watcher(
    state: Rc<RefCell<EditorState>>,
    on_changed: Rc<dyn Fn()>,
    _backend: Rc<dyn FileBackend>,
    is_remote: bool,
) {
    let last_hash = Rc::new(Cell::new(0u64));
    let interval = if is_remote { 30 } else { 2 };
    glib::timeout_add_local(std::time::Duration::from_secs(interval), move || {
        let root = state.borrow().root_dir.clone();
        let hash = dir_hash(&root, is_remote);
        if hash != last_hash.get() {
            last_hash.set(hash);
            on_changed();
        }
        glib::ControlFlow::Continue
    });
}

/// Watch git status (3s local, 15s remote).
fn start_git_watcher(
    _state: Rc<RefCell<EditorState>>,
    on_changed: Rc<dyn Fn(String)>,
    backend: Rc<dyn FileBackend>,
    is_remote: bool,
) {
    let last_output = Rc::new(RefCell::new(String::new()));
    let interval = if is_remote { 15 } else { 3 };
    glib::timeout_add_local(std::time::Duration::from_secs(interval), move || {
        if let Ok(stdout) = backend.git_command(&["status", "--porcelain"]) {
            if stdout != *last_output.borrow() {
                *last_output.borrow_mut() = stdout.clone();
                on_changed(stdout);
            }
        }
        glib::ControlFlow::Continue
    });
}

#[allow(deprecated)]
fn show_conflict_bar(container: &gtk4::Box, path: &Path, buffer: &sourceview5::Buffer, backend: Rc<dyn FileBackend>) {
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
            if let Ok(content) = backend.read_file(&path_c) {
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
/// For remote backends, skip the expensive dir traversal — return 0 (rely on git status only).
fn dir_hash(dir: &Path, is_remote: bool) -> u64 {
    if is_remote {
        return 0;
    }

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
