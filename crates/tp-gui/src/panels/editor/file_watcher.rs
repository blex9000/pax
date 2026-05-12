use gtk4::glib;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use super::file_backend::FileBackend;
use super::task::run_blocking;
use super::EditorState;

/// Opens the side-by-side merge view for a file that changed externally while
/// the user has unsaved edits. Receives:
///   - the file path,
///   - the current on-disk content (left, read-only),
///   - the user's unsaved buffer content (right, editable),
///   - an `apply_merged` callback that pushes the final merged text back into
///     the source buffer + saved-content shadow when the user saves.
/// Wired from `mod.rs` through `start_watchers` so file_watcher stays
/// decoupled from `EditorTabs`.
pub type OnMergeOpen = Rc<dyn Fn(&Path, &str, &str, Rc<dyn Fn(&str)>)>;

#[derive(Clone)]
enum WatchedKind {
    /// Source / Markdown tab — both have a writable buffer and saved content.
    Text { saved_content: String },
    /// Image tab — no content compare, mtime-only.
    Image,
}

#[derive(Clone)]
struct WatchedFileSnapshot {
    path: PathBuf,
    last_disk_mtime: u64,
    kind: WatchedKind,
}

#[derive(Debug, Clone)]
struct FileChange {
    path: PathBuf,
    content: String,
    last_disk_mtime: u64,
}

/// Per-tab apply payload carried out of the immutable borrow on
/// `open_file.content` so we can subsequently mutate `open_file` itself.
enum ApplyKind {
    Source {
        buffer: sourceview5::Buffer,
        saved: Rc<RefCell<String>>,
    },
    Markdown {
        tab: super::tab_content::MarkdownTab,
    },
    Image,
}

/// Start all file watchers. Call once during CodeEditorPanel construction.
pub fn start_watchers(
    state: Rc<RefCell<EditorState>>,
    info_bar_container: gtk4::Box,
    on_merge_open: OnMergeOpen,
    on_tree_changed: Rc<dyn Fn()>,
    on_git_changed: Rc<dyn Fn(String)>,
) {
    let backend = state.borrow().backend.clone();
    let is_remote = backend.is_remote();
    let poll = state.borrow().poll_interval;

    start_open_file_watcher(
        state.clone(),
        info_bar_container,
        on_merge_open,
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
    on_merge_open: OnMergeOpen,
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
                .map(|open_file| {
                    use super::tab_content::TabContent;
                    let kind = match &open_file.content {
                        TabContent::Source(s) => WatchedKind::Text {
                            saved_content: s.saved_content.borrow().clone(),
                        },
                        TabContent::Markdown(m) => WatchedKind::Text {
                            saved_content: m.saved_content.borrow().clone(),
                        },
                        TabContent::Image(_) => WatchedKind::Image,
                    };
                    WatchedFileSnapshot {
                        path: open_file.path.clone(),
                        last_disk_mtime: open_file.last_disk_mtime,
                        kind,
                    }
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
        let on_merge_open_c = on_merge_open.clone();
        run_blocking(
            move || collect_open_file_changes(&snapshots, &*backend_for_task, is_remote),
            move |changes| {
                in_flight_c.set(false);

                let mut st = state_c.borrow_mut();
                let sync_suppress = st.sync_suppress.clone();
                for change in changes {
                    let Some(open_file) = st.open_files.iter_mut().find(|f| f.path == change.path)
                    else {
                        continue;
                    };

                    use super::tab_content::TabContent;
                    // Clone per-kind handles out so the borrow on
                    // open_file.content ends before we mutate open_file
                    // (last_disk_mtime, set_modified) below.
                    let apply_kind = match &open_file.content {
                        TabContent::Source(s) => ApplyKind::Source {
                            buffer: s.buffer.clone(),
                            saved: s.saved_content.clone(),
                        },
                        TabContent::Markdown(m) => ApplyKind::Markdown { tab: m.clone() },
                        TabContent::Image(_) => ApplyKind::Image,
                    };

                    match apply_kind {
                        ApplyKind::Source { buffer, saved } => {
                            let disk_changed = if is_remote {
                                change.content != *saved.borrow()
                            } else if change.last_disk_mtime == 0
                                || change.last_disk_mtime == open_file.last_disk_mtime
                            {
                                false
                            } else {
                                open_file.last_disk_mtime = change.last_disk_mtime;
                                true
                            };
                            if !disk_changed {
                                continue;
                            }
                            if !open_file.modified() {
                                *saved.borrow_mut() = change.content.clone();
                                sync_suppress.set(true);
                                buffer.set_text(&change.content);
                                sync_suppress.set(false);
                                buffer.set_enable_undo(false);
                                buffer.set_enable_undo(true);
                                open_file.set_modified(false);
                            } else {
                                let apply_reload: Rc<dyn Fn(&str)> = {
                                    let buf = buffer.clone();
                                    let saved_c = saved.clone();
                                    Rc::new(move |content: &str| {
                                        *saved_c.borrow_mut() = content.to_string();
                                        buf.set_text(content);
                                        buf.set_enable_undo(false);
                                        buf.set_enable_undo(true);
                                    })
                                };
                                let on_merge: Rc<dyn Fn()> = {
                                    let path = open_file.path.clone();
                                    let disk = change.content.clone();
                                    let buf = buffer.clone();
                                    let apply = apply_reload.clone();
                                    let merge_cb = on_merge_open_c.clone();
                                    Rc::new(move || {
                                        let mine = buf
                                            .text(&buf.start_iter(), &buf.end_iter(), false)
                                            .to_string();
                                        merge_cb(&path, &disk, &mine, apply.clone());
                                    })
                                };
                                show_conflict_bar(
                                    &info_bar_container_c,
                                    &open_file.path,
                                    backend_for_apply.clone(),
                                    apply_reload,
                                    Some(on_merge),
                                );
                            }
                        }
                        ApplyKind::Markdown { tab } => {
                            let disk_changed = if is_remote {
                                change.content != *tab.saved_content.borrow()
                            } else if change.last_disk_mtime == 0
                                || change.last_disk_mtime == open_file.last_disk_mtime
                            {
                                false
                            } else {
                                open_file.last_disk_mtime = change.last_disk_mtime;
                                true
                            };
                            if !disk_changed {
                                continue;
                            }
                            if !open_file.modified() {
                                super::markdown_view::reload_from_disk(&tab, &change.content);
                                open_file.set_modified(false);
                            } else {
                                let md = tab.clone();
                                let apply_reload: Rc<dyn Fn(&str)> =
                                    Rc::new(move |content: &str| {
                                        super::markdown_view::reload_from_disk(&md, content);
                                    });
                                let on_merge: Rc<dyn Fn()> = {
                                    let path = open_file.path.clone();
                                    let disk = change.content.clone();
                                    let buf = tab.buffer.clone();
                                    let apply = apply_reload.clone();
                                    let merge_cb = on_merge_open_c.clone();
                                    Rc::new(move || {
                                        let mine = buf
                                            .text(&buf.start_iter(), &buf.end_iter(), false)
                                            .to_string();
                                        merge_cb(&path, &disk, &mine, apply.clone());
                                    })
                                };
                                show_conflict_bar(
                                    &info_bar_container_c,
                                    &open_file.path,
                                    backend_for_apply.clone(),
                                    apply_reload,
                                    Some(on_merge),
                                );
                            }
                        }
                        ApplyKind::Image => {
                            if is_remote {
                                continue;
                            }
                            if change.last_disk_mtime == 0
                                || change.last_disk_mtime == open_file.last_disk_mtime
                            {
                                continue;
                            }
                            open_file.last_disk_mtime = change.last_disk_mtime;
                            // Re-borrow content to access the picture handle.
                            if let TabContent::Image(img) = &open_file.content {
                                super::image_view::reload_from_disk(img, &open_file.path);
                            }
                        }
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
                tracing::trace!(
                    "editor.watcher: tree dir_hash begin root={}",
                    root_for_trace.display()
                );
                let h = dir_hash(&root, is_remote);
                tracing::trace!(
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
        match &snapshot.kind {
            WatchedKind::Text { saved_content } => {
                if is_remote {
                    if let Ok(content) = backend.read_file(&snapshot.path) {
                        if &content != saved_content {
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
            WatchedKind::Image => {
                // Image tabs are local-only (open_image_file refuses remote).
                if is_remote {
                    continue;
                }
                let current_mtime = get_mtime(&snapshot.path);
                if current_mtime == 0 || current_mtime == snapshot.last_disk_mtime {
                    continue;
                }
                changes.push(FileChange {
                    path: snapshot.path.clone(),
                    content: String::new(),
                    last_disk_mtime: current_mtime,
                });
            }
        }
    }

    changes
}

#[allow(deprecated)]
fn show_conflict_bar(
    container: &gtk4::Box,
    path: &Path,
    backend: Arc<dyn FileBackend>,
    apply_reload: Rc<dyn Fn(&str)>,
    on_merge: Option<Rc<dyn Fn()>>,
) {
    // Remove any existing info bar so we never stack two for the same tab.
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
    // ResponseType::Other is used as the "Merge" sentinel — InfoBar's
    // built-in response variants don't have a "third action" slot.
    const MERGE_RESPONSE: u16 = 1;
    if on_merge.is_some() {
        bar.add_button("Merge", gtk4::ResponseType::Other(MERGE_RESPONSE));
    }

    let path_c = path.to_path_buf();
    let container_c = container.clone();
    let backend_c = backend.clone();
    let apply_reload_c = apply_reload.clone();
    let on_merge_c = on_merge.clone();
    bar.connect_response(move |bar, response| {
        match response {
            gtk4::ResponseType::Accept => {
                let path_reload = path_c.clone();
                let backend_reload = backend_c.clone();
                let apply = apply_reload_c.clone();
                run_blocking(
                    move || backend_reload.read_file(&path_reload),
                    move |result| {
                        if let Ok(content) = result {
                            apply(&content);
                        }
                    },
                );
            }
            gtk4::ResponseType::Other(code) if code == MERGE_RESPONSE => {
                if let Some(cb) = on_merge_c.as_ref() {
                    cb();
                }
            }
            _ => {}
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

        fn copy_dir(&self, _from: &Path, _to: &Path) -> Result<(), String> {
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
                last_disk_mtime: 0,
                kind: WatchedKind::Text {
                    saved_content: "before\n".to_string(),
                },
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
                last_disk_mtime: 0,
                kind: WatchedKind::Text {
                    saved_content: "local\n".to_string(),
                },
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
