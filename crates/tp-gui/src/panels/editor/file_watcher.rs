use gtk4::glib;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use super::file_backend::FileBackend;
use super::task::run_blocking;
use super::EditorState;

#[derive(Clone)]
struct WatchedFileSnapshot {
    path: PathBuf,
    saved_content: String,
    last_disk_mtime: u64,
}

#[derive(Debug, Clone)]
struct FileChange {
    path: PathBuf,
    content: String,
    last_disk_mtime: u64,
}

/// Start all file watchers. Call once during CodeEditorPanel construction.
pub fn start_watchers(
    state: Rc<RefCell<EditorState>>,
    info_bar_container: gtk4::Box,
    on_tree_changed: Rc<dyn Fn()>,
    on_git_changed: Rc<dyn Fn(String)>,
) {
    let backend = state.borrow().backend.clone();
    let is_remote = backend.is_remote();
    let poll = state.borrow().poll_interval;

    start_open_file_watcher(
        state.clone(),
        info_bar_container,
        backend.clone(),
        is_remote,
    );
    if !is_remote {
        start_tree_watcher(state.clone(), on_tree_changed, is_remote, poll);
    }
    start_git_watcher(state, on_git_changed, backend, is_remote, poll);
}

pub fn request_git_status_refresh(on_changed: Rc<dyn Fn(String)>, backend: Arc<dyn FileBackend>) {
    run_blocking(
        move || backend.git_command(&["status", "--porcelain"]),
        move |result| {
            if let Ok(stdout) = result {
                on_changed(stdout);
            }
        },
    );
}

/// Watch open files for external changes (1s local, 5s remote).
fn start_open_file_watcher(
    state: Rc<RefCell<EditorState>>,
    info_bar_container: gtk4::Box,
    backend: Arc<dyn FileBackend>,
    is_remote: bool,
) {
    let interval = if is_remote { 5 } else { 1 };
    let in_flight = Rc::new(Cell::new(false));
    glib::timeout_add_local(std::time::Duration::from_secs(interval), move || {
        if in_flight.get() {
            return glib::ControlFlow::Continue;
        }

        let snapshots: Vec<WatchedFileSnapshot> = {
            let st = state.borrow();
            st.open_files
                .iter()
                .map(|open_file| WatchedFileSnapshot {
                    path: open_file.path.clone(),
                    saved_content: open_file.saved_content.borrow().clone(),
                    last_disk_mtime: open_file.last_disk_mtime,
                })
                .collect()
        };

        if snapshots.is_empty() {
            return glib::ControlFlow::Continue;
        }

        in_flight.set(true);
        let state_c = state.clone();
        let info_bar_container_c = info_bar_container.clone();
        let backend_for_task = backend.clone();
        let backend_for_apply = backend.clone();
        let in_flight_c = in_flight.clone();
        run_blocking(
            move || collect_open_file_changes(&snapshots, &*backend_for_task, is_remote),
            move |changes| {
                in_flight_c.set(false);

                let mut st = state_c.borrow_mut();
                for change in changes {
                    let Some(open_file) = st.open_files.iter_mut().find(|f| f.path == change.path)
                    else {
                        continue;
                    };

                    if is_remote {
                        let disk_changed = change.content != *open_file.saved_content.borrow();
                        if !disk_changed {
                            continue;
                        }
                        if !open_file.modified {
                            *open_file.saved_content.borrow_mut() = change.content.clone();
                            open_file.buffer.set_text(&change.content);
                            open_file.buffer.set_enable_undo(false);
                            open_file.buffer.set_enable_undo(true);
                            open_file.modified = false;
                        } else {
                            show_conflict_bar(
                                &info_bar_container_c,
                                &open_file.path,
                                &open_file.buffer,
                                open_file.saved_content.clone(),
                                backend_for_apply.clone(),
                            );
                        }
                        continue;
                    }

                    if change.last_disk_mtime == 0
                        || change.last_disk_mtime == open_file.last_disk_mtime
                    {
                        continue;
                    }
                    open_file.last_disk_mtime = change.last_disk_mtime;
                    if !open_file.modified {
                        *open_file.saved_content.borrow_mut() = change.content.clone();
                        open_file.buffer.set_text(&change.content);
                        open_file.buffer.set_enable_undo(false);
                        open_file.buffer.set_enable_undo(true);
                        open_file.modified = false;
                    } else {
                        show_conflict_bar(
                            &info_bar_container_c,
                            &open_file.path,
                            &open_file.buffer,
                            open_file.saved_content.clone(),
                            backend_for_apply.clone(),
                        );
                    }
                }
            },
        );

        glib::ControlFlow::Continue
    });
}

/// Watch file tree for structural changes (2s local, 30s remote).
fn start_tree_watcher(
    state: Rc<RefCell<EditorState>>,
    on_changed: Rc<dyn Fn()>,
    is_remote: bool,
    poll: u64,
) {
    let last_hash: Rc<RefCell<Option<u64>>> = Rc::new(RefCell::new(None));
    let in_flight = Rc::new(Cell::new(false));
    let interval = poll;
    glib::timeout_add_local(std::time::Duration::from_secs(interval), move || {
        if in_flight.get() {
            return glib::ControlFlow::Continue;
        }

        in_flight.set(true);
        let root = state.borrow().root_dir.clone();
        let last_hash_c = last_hash.clone();
        let on_changed_c = on_changed.clone();
        let in_flight_c = in_flight.clone();
        let root_for_trace = root.clone();
        run_blocking(
            move || {
                tracing::debug!(
                    "editor.watcher: tree dir_hash begin root={}",
                    root_for_trace.display()
                );
                let h = dir_hash(&root, is_remote);
                tracing::debug!(
                    "editor.watcher: tree dir_hash end root={} hash={:x}",
                    root_for_trace.display(),
                    h
                );
                h
            },
            move |hash| {
                in_flight_c.set(false);
                let previous = *last_hash_c.borrow();
                *last_hash_c.borrow_mut() = Some(hash);
                if previous.is_some() && previous != Some(hash) {
                    tracing::info!("editor.watcher: tree changed → refresh");
                    on_changed_c();
                }
            },
        );
        glib::ControlFlow::Continue
    });
}

/// Watch git status (3s local, 15s remote).
fn start_git_watcher(
    _state: Rc<RefCell<EditorState>>,
    on_changed: Rc<dyn Fn(String)>,
    backend: Arc<dyn FileBackend>,
    _is_remote: bool,
    poll: u64,
) {
    let last_output: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let in_flight = Rc::new(Cell::new(false));
    let interval = poll;
    request_polled_git_status(
        last_output.clone(),
        on_changed.clone(),
        backend.clone(),
        in_flight.clone(),
    );
    glib::timeout_add_local(std::time::Duration::from_secs(interval), move || {
        request_polled_git_status(
            last_output.clone(),
            on_changed.clone(),
            backend.clone(),
            in_flight.clone(),
        );
        glib::ControlFlow::Continue
    });
}

fn request_polled_git_status(
    last_output: Rc<RefCell<Option<String>>>,
    on_changed: Rc<dyn Fn(String)>,
    backend: Arc<dyn FileBackend>,
    in_flight: Rc<Cell<bool>>,
) {
    if in_flight.get() {
        return;
    }

    in_flight.set(true);
    run_blocking(
        move || backend.git_command(&["status", "--porcelain"]),
        move |result| {
            in_flight.set(false);
            if let Ok(stdout) = result {
                let mut last_output = last_output.borrow_mut();
                let changed = last_output.as_ref() != Some(&stdout);
                if changed {
                    *last_output = Some(stdout.clone());
                    on_changed(stdout);
                }
            }
        },
    );
}

fn collect_open_file_changes(
    snapshots: &[WatchedFileSnapshot],
    backend: &dyn FileBackend,
    is_remote: bool,
) -> Vec<FileChange> {
    let mut changes = Vec::new();

    for snapshot in snapshots {
        if is_remote {
            if let Ok(content) = backend.read_file(&snapshot.path) {
                if content != snapshot.saved_content {
                    changes.push(FileChange {
                        path: snapshot.path.clone(),
                        content,
                        last_disk_mtime: 0,
                    });
                }
            }
            continue;
        }

        let current_mtime = get_mtime(&snapshot.path);
        if current_mtime == 0 || current_mtime == snapshot.last_disk_mtime {
            continue;
        }
        if let Ok(content) = backend.read_file(&snapshot.path) {
            changes.push(FileChange {
                path: snapshot.path.clone(),
                content,
                last_disk_mtime: current_mtime,
            });
        }
    }

    changes
}

#[allow(deprecated)]
fn show_conflict_bar(
    container: &gtk4::Box,
    path: &Path,
    buffer: &sourceview5::Buffer,
    saved_content: Rc<RefCell<String>>,
    backend: Arc<dyn FileBackend>,
) {
    // Remove any existing info bar
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let bar = gtk4::InfoBar::new();
    bar.set_message_type(gtk4::MessageType::Warning);
    bar.set_show_close_button(true);

    let label = gtk4::Label::new(Some(&format!(
        "\"{}\" changed on disk.",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    )));
    bar.add_child(&label);

    bar.add_button("Reload", gtk4::ResponseType::Accept);
    bar.add_button("Keep Mine", gtk4::ResponseType::Reject);

    let path_c = path.to_path_buf();
    let buf_c = buffer.clone();
    let container_c = container.clone();
    let saved_content_c = saved_content.clone();
    let backend_c = backend.clone();
    bar.connect_response(move |bar, response| {
        if response == gtk4::ResponseType::Accept {
            let path_reload = path_c.clone();
            let buf_reload = buf_c.clone();
            let saved_reload = saved_content_c.clone();
            let backend_reload = backend_c.clone();
            run_blocking(
                move || backend_reload.read_file(&path_reload),
                move |result| {
                    if let Ok(content) = result {
                        *saved_reload.borrow_mut() = content.clone();
                        buf_reload.set_text(&content);
                        buf_reload.set_enable_undo(false);
                        buf_reload.set_enable_undo(true);
                    }
                },
            );
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
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        })
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
    let walker = ignore::WalkBuilder::new(dir).max_depth(Some(5)).build();

    for entry in walker.flatten() {
        entry.path().hash(&mut hasher);
        get_mtime(entry.path()).hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::editor::file_backend::{DirEntry, LocalFileBackend};
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[derive(Debug)]
    struct MockRemoteBackend {
        content: Mutex<String>,
        root: PathBuf,
    }

    impl MockRemoteBackend {
        fn new(content: &str) -> Self {
            Self {
                content: Mutex::new(content.to_string()),
                root: PathBuf::from("/remote"),
            }
        }
    }

    impl FileBackend for MockRemoteBackend {
        fn list_dir(&self, _path: &Path) -> Result<Vec<DirEntry>, String> {
            unreachable!()
        }

        fn read_file(&self, _path: &Path) -> Result<String, String> {
            Ok(self.content.lock().unwrap().clone())
        }

        fn write_file(&self, _path: &Path, _content: &str) -> Result<(), String> {
            unreachable!()
        }

        fn file_exists(&self, _path: &Path) -> bool {
            true
        }

        fn delete_file(&self, _path: &Path) -> Result<(), String> {
            unreachable!()
        }

        fn delete_dir(&self, _path: &Path) -> Result<(), String> {
            unreachable!()
        }

        fn rename_file(&self, _from: &Path, _to: &Path) -> Result<(), String> {
            unreachable!()
        }

        fn copy_file(&self, _from: &Path, _to: &Path) -> Result<(), String> {
            unreachable!()
        }

        fn create_dir(&self, _path: &Path) -> Result<(), String> {
            unreachable!()
        }

        fn git_command(&self, _args: &[&str]) -> Result<String, String> {
            unreachable!()
        }

        fn root(&self) -> &Path {
            &self.root
        }

        fn is_remote(&self) -> bool {
            true
        }
    }

    #[test]
    fn collect_open_file_changes_detects_local_disk_updates() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sample.txt");
        std::fs::write(&path, "after\n").unwrap();
        let backend = LocalFileBackend::new(dir.path());

        let changes = collect_open_file_changes(
            &[WatchedFileSnapshot {
                path: path.clone(),
                saved_content: "before\n".to_string(),
                last_disk_mtime: 0,
            }],
            &backend,
            false,
        );

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, path);
        assert_eq!(changes[0].content, "after\n");
        assert_ne!(changes[0].last_disk_mtime, 0);
    }

    #[test]
    fn collect_open_file_changes_detects_remote_content_differences() {
        let backend = MockRemoteBackend::new("remote\n");
        let path = PathBuf::from("/remote/app.rs");

        let changes = collect_open_file_changes(
            &[WatchedFileSnapshot {
                path: path.clone(),
                saved_content: "local\n".to_string(),
                last_disk_mtime: 0,
            }],
            &backend,
            true,
        );

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, path);
        assert_eq!(changes[0].content, "remote\n");
        assert_eq!(changes[0].last_disk_mtime, 0);
    }
}
