use gtk4::glib;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use super::file_backend::FileBackend;
use super::task::run_blocking;

/// Callback when a file is opened in the tree.
pub type OnFileOpen = Rc<dyn Fn(&Path)>;
/// Callback for context menu actions: (action, file_path)
pub type OnContextAction = Rc<dyn Fn(&str, &Path)>;
/// Callback after a successful file/dir rename on disk: `(old_path, new_path)`.
/// Used by the editor tabs to update open tab labels and stored paths so a
/// renamed file doesn't appear as a duplicate tab on the next click.
pub type OnFileRenamed = Rc<dyn Fn(&Path, &Path)>;
/// Callback after a successful file or directory deletion. The receiver
/// decides whether to close tabs for the exact path (file delete) or all
/// tabs under the prefix (directory delete).
pub type OnPathDeleted = Rc<dyn Fn(&Path)>;

/// File tree widget with gitignore-aware traversal and expand/collapse.
pub struct FileTree {
    pub widget: gtk4::Box,
    list_box: gtk4::ListBox,
    scroll: gtk4::ScrolledWindow,
    root_dir: PathBuf,
    #[allow(dead_code)]
    on_file_open: Option<OnFileOpen>,
    /// Flat list of all file paths for fuzzy finder indexing.
    pub file_index: Rc<RefCell<Vec<PathBuf>>>,
    entries: Rc<RefCell<Vec<FileEntry>>>,
    #[allow(dead_code)]
    on_context_action: Option<OnContextAction>,
    #[allow(dead_code)]
    on_file_renamed: Option<OnFileRenamed>,
    #[allow(dead_code)]
    on_path_deleted: Option<OnPathDeleted>,
    #[allow(dead_code)]
    backend: Arc<dyn FileBackend>,
    request_seq: Rc<Cell<u64>>,
}

#[derive(Debug, Clone)]
struct FileEntry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    is_ignored: bool,
    depth: u32,
    expanded: bool,
}

#[derive(Default)]
struct TreeSnapshot {
    entries: Vec<FileEntry>,
    file_index: Vec<PathBuf>,
}

impl FileTree {
    pub fn new(root_dir: &Path, on_file_open: OnFileOpen, backend: Arc<dyn FileBackend>) -> Self {
        Self::new_with_context(root_dir, on_file_open, None, None, None, backend)
    }

    pub fn new_with_context(
        root_dir: &Path,
        on_file_open: OnFileOpen,
        on_context_action: Option<OnContextAction>,
        on_file_renamed: Option<OnFileRenamed>,
        on_path_deleted: Option<OnPathDeleted>,
        backend: Arc<dyn FileBackend>,
    ) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("editor-file-tree");

        // Action buttons bar at bottom
        let actions_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
        actions_bar.add_css_class("editor-sidebar-toolbar");
        actions_bar.add_css_class("editor-file-tree-actions");
        actions_bar.set_margin_start(2);
        actions_bar.set_margin_end(2);
        actions_bar.set_margin_bottom(0);

        let collapse_btn = gtk4::Button::from_icon_name("go-up-symbolic");
        collapse_btn.add_css_class("flat");
        collapse_btn.set_tooltip_text(Some("Collapse All"));

        actions_bar.append(&collapse_btn);

        let file_index: Rc<RefCell<Vec<PathBuf>>> = Rc::new(RefCell::new(Vec::new()));
        let entries: Rc<RefCell<Vec<FileEntry>>> = Rc::new(RefCell::new(Vec::new()));
        let is_remote = backend.is_remote();
        let request_seq = Rc::new(Cell::new(0u64));

        let list_box = gtk4::ListBox::new();
        list_box.set_selection_mode(gtk4::SelectionMode::Single);
        list_box.add_css_class("navigation-sidebar");
        list_box.add_css_class("editor-file-tree-list");
        populate_message(
            &list_box,
            if is_remote {
                "Connecting to remote host..."
            } else {
                "Loading files..."
            },
        );

        let scroll = gtk4::ScrolledWindow::new();
        scroll.add_css_class("editor-file-tree-scroll");
        scroll.set_child(Some(&list_box));
        scroll.set_vexpand(true);

        container.append(&scroll);
        container.append(&actions_bar);

        // Single click: expand/collapse dirs, open files
        {
            let entries_c = entries.clone();
            let on_open = on_file_open.clone();
            let fi = file_index.clone();
            let root = root_dir.to_path_buf();
            let sw = scroll.clone();
            let be = backend.clone();
            list_box.connect_row_activated(move |lb, row| {
                let idx = row.index() as usize;
                let is_dir;
                let expanded;
                let path;
                let depth;
                {
                    let ents = entries_c.borrow();
                    let Some(entry) = ents.get(idx) else { return };
                    is_dir = entry.is_dir;
                    expanded = entry.expanded;
                    path = entry.path.clone();
                    depth = entry.depth;
                }
                if is_dir {
                    let vadj = sw.vadjustment();
                    let scroll_pos = vadj.value();
                    let count_before = entries_c.borrow().len();
                    toggle_dir(&entries_c, &fi, idx, depth, expanded, &path, &*be);
                    let entries = entries_c.borrow();
                    let count_after = entries.len();
                    if expanded {
                        let removed = count_before - count_after;
                        incremental_collapse(lb, &entries, &root, idx, removed);
                    } else {
                        let added = count_after - count_before;
                        incremental_expand(lb, &entries, &root, idx, added);
                    }
                    drop(entries);
                    vadj.set_value(scroll_pos);
                } else {
                    on_open(&path);
                }
            });
        }

        // Right-click context menu on files
        {
            let entries_c = entries.clone();
            let entries_for_refresh = entries.clone();
            let list_box_for_refresh = list_box.clone();
            let scroll_for_refresh = scroll.clone();
            let file_index_for_refresh = file_index.clone();
            let request_seq_for_refresh = request_seq.clone();
            let root = root_dir.to_path_buf();
            let ctx_cb = on_context_action.clone();
            let rename_cb = on_file_renamed.clone();
            let delete_cb = on_path_deleted.clone();
            let backend = backend.clone();
            let on_open = on_file_open.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(3); // right-click
            gesture.connect_pressed(move |g, _n, x, y| {
                let Some(widget) = g.widget() else { return };
                let Some(lb) = widget.downcast_ref::<gtk4::ListBox>() else {
                    return;
                };
                let clicked_row = lb.row_at_y(y as i32);
                if let Some(ref row) = clicked_row {
                    lb.select_row(Some(row));
                }
                let clicked_idx = clicked_row.as_ref().map(|row| row.index() as usize);
                let selected_idx = lb.selected_row().map(|row| row.index() as usize);
                let ents = entries_c.borrow();
                let selected_entry =
                    resolve_tree_selection(&root, &ents, clicked_idx, selected_idx);
                let selected_path = selected_entry
                    .as_ref()
                    .map(|entry| entry.path.clone())
                    .unwrap_or_else(|| root.clone());
                let rel = selected_path
                    .strip_prefix(&root)
                    .unwrap_or(&selected_path)
                    .to_path_buf();
                let target_dir = creation_target_dir(&root, &ents, clicked_idx, selected_idx);
                let selected_is_dir = selected_entry
                    .as_ref()
                    .map(|entry| entry.is_dir)
                    .unwrap_or(true);
                let parent_window = find_transient_parent_window(lb.upcast_ref::<gtk4::Widget>());
                let refresh_tree: Rc<dyn Fn()> = {
                    let root = root.clone();
                    let backend = backend.clone();
                    let list_box = list_box_for_refresh.clone();
                    let scroll = scroll_for_refresh.clone();
                    let entries = entries_for_refresh.clone();
                    let file_index = file_index_for_refresh.clone();
                    let request_seq = request_seq_for_refresh.clone();
                    Rc::new(move || {
                        request_tree_reload(
                            &list_box,
                            &scroll,
                            &root,
                            &entries,
                            &file_index,
                            backend.clone(),
                            request_seq.clone(),
                            collect_expanded_dirs(&entries.borrow()),
                            false,
                            "Refreshing files...",
                            "No files found",
                        );
                    })
                };

                let menu_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                menu_box.set_margin_top(4);
                menu_box.set_margin_bottom(4);

                let make_item = |icon: &str, label: &str| -> gtk4::Button {
                    let btn = gtk4::Button::new();
                    btn.add_css_class("flat");
                    btn.add_css_class("app-popover-button");
                    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                    content.set_margin_start(4);
                    content.set_margin_end(8);
                    let img = gtk4::Image::from_icon_name(icon);
                    img.set_pixel_size(16);
                    content.append(&img);
                    let lbl = gtk4::Label::new(Some(label));
                    lbl.set_halign(gtk4::Align::Start);
                    content.append(&lbl);
                    btn.set_child(Some(&content));
                    btn
                };

                let create_file_btn = make_item("document-new-symbolic", "New File");
                {
                    let target_dir = target_dir.clone();
                    let be = backend.clone();
                    let refresh_tree = refresh_tree.clone();
                    let on_open = on_open.clone();
                    let parent_window = parent_window.clone();
                    create_file_btn.connect_clicked(move |btn| {
                        if let Some(pop) = btn.ancestor(gtk4::Popover::static_type()) {
                            pop.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                        }
                        let parent_window = parent_window.clone();
                        let target_dir = target_dir.clone();
                        let be = be.clone();
                        let refresh_tree = refresh_tree.clone();
                        let on_open = on_open.clone();
                        glib::idle_add_local_once(move || {
                            show_name_input_dialog(
                                parent_window.as_ref(),
                                "New File",
                                "Create File",
                                "",
                                Rc::new(move |name| {
                                    let Some(dest) =
                                        creation_destination_for_dir(&target_dir, &name)
                                    else {
                                        return;
                                    };
                                    tracing::info!(
                                        "editor.ft: create_file begin path={}",
                                        dest.display()
                                    );
                                    let write_result = be.write_file(&dest, "");
                                    tracing::info!(
                                        "editor.ft: create_file write result ok={}",
                                        write_result.is_ok()
                                    );
                                    if write_result.is_ok() {
                                        refresh_tree();
                                        tracing::info!("editor.ft: create_file refresh_tree scheduled");
                                        on_open(&dest);
                                        tracing::info!(
                                            "editor.ft: create_file on_open done path={}",
                                            dest.display()
                                        );
                                    }
                                }),
                            );
                        });
                    });
                }
                menu_box.append(&create_file_btn);

                let create_folder_btn = make_item("folder-new-symbolic", "New Folder");
                {
                    let target_dir = target_dir.clone();
                    let be = backend.clone();
                    let refresh_tree = refresh_tree.clone();
                    let parent_window = parent_window.clone();
                    create_folder_btn.connect_clicked(move |btn| {
                        if let Some(pop) = btn.ancestor(gtk4::Popover::static_type()) {
                            pop.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                        }
                        let parent_window = parent_window.clone();
                        let target_dir = target_dir.clone();
                        let be = be.clone();
                        let refresh_tree = refresh_tree.clone();
                        glib::idle_add_local_once(move || {
                            show_name_input_dialog(
                                parent_window.as_ref(),
                                "New Folder",
                                "Create Folder",
                                "",
                                Rc::new(move |name| {
                                    let Some(dest) =
                                        creation_destination_for_dir(&target_dir, &name)
                                    else {
                                        return;
                                    };
                                    tracing::info!(
                                        "editor.ft: create_dir begin path={}",
                                        dest.display()
                                    );
                                    let mkdir_result = be.create_dir(&dest);
                                    tracing::info!(
                                        "editor.ft: create_dir result ok={}",
                                        mkdir_result.is_ok()
                                    );
                                    if mkdir_result.is_ok() {
                                        refresh_tree();
                                        tracing::info!("editor.ft: create_dir refresh_tree scheduled");
                                    }
                                }),
                            );
                        });
                    });
                }
                menu_box.append(&create_folder_btn);

                menu_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

                // ── Clipboard ──
                let copy_rel = make_item("edit-copy-symbolic", "Copy Relative Path");
                {
                    let rel_str = rel.to_string_lossy().to_string();
                    copy_rel.connect_clicked(move |btn| {
                        if let Some(d) = gtk4::gdk::Display::default() {
                            d.clipboard().set_text(&rel_str);
                        }
                        if let Some(p) = btn.ancestor(gtk4::Popover::static_type()) {
                            p.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                        }
                    });
                }
                menu_box.append(&copy_rel);

                let copy_abs = make_item("edit-copy-symbolic", "Copy Absolute Path");
                {
                    let abs_str = selected_path.to_string_lossy().to_string();
                    copy_abs.connect_clicked(move |btn| {
                        if let Some(d) = gtk4::gdk::Display::default() {
                            d.clipboard().set_text(&abs_str);
                        }
                        if let Some(p) = btn.ancestor(gtk4::Popover::static_type()) {
                            p.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                        }
                    });
                }
                menu_box.append(&copy_abs);

                if selected_entry.is_some() {
                    menu_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
                }

                // ── Rename ──
                if let Some(entry) = selected_entry {
                    let path = entry.path.clone();
                    let is_dir = selected_is_dir;
                    let rename_btn = make_item(
                        "document-edit-symbolic",
                        if is_dir {
                            "Rename Folder"
                        } else {
                            "Rename File"
                        },
                    );
                    {
                        let p = path.clone();
                        let be = backend.clone();
                        let refresh_tree = refresh_tree.clone();
                        let parent_window = parent_window.clone();
                        let rename_cb_for_btn = rename_cb.clone();
                        rename_btn.connect_clicked(move |btn| {
                            if let Some(pop) = btn.ancestor(gtk4::Popover::static_type()) {
                                pop.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                            }
                            let parent_window = parent_window.clone();
                            let current_name = p
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            let initial_name = current_name.clone();
                            let p = p.clone();
                            let be = be.clone();
                            let refresh_tree = refresh_tree.clone();
                            let rename_cb = rename_cb_for_btn.clone();
                            glib::idle_add_local_once(move || {
                                show_name_input_dialog(
                                    parent_window.as_ref(),
                                    "Rename",
                                    if is_dir {
                                        "Rename Folder"
                                    } else {
                                        "Rename File"
                                    },
                                    &initial_name,
                                    Rc::new(move |new_name| {
                                        if new_name == current_name {
                                            return;
                                        }
                                        if let Some(dest) =
                                            rename_destination_for_path(&p, &new_name)
                                        {
                                            tracing::info!(
                                                "editor.ft: rename begin from={} to={}",
                                                p.display(),
                                                dest.display()
                                            );
                                            let rename_result = be.rename_file(&p, &dest);
                                            tracing::info!(
                                                "editor.ft: rename result ok={}",
                                                rename_result.is_ok()
                                            );
                                            if rename_result.is_ok() {
                                                if let Some(ref cb) = rename_cb {
                                                    cb(&p, &dest);
                                                }
                                                refresh_tree();
                                                tracing::info!(
                                                    "editor.ft: rename refresh_tree scheduled"
                                                );
                                            }
                                        }
                                    }),
                                );
                            });
                        });
                    }
                    menu_box.append(&rename_btn);

                    if is_dir {
                        let del_btn = make_item("user-trash-symbolic", "Delete Folder");
                        {
                            let p = path.clone();
                            let be = backend.clone();
                            let refresh_tree = refresh_tree.clone();
                            let delete_cb = delete_cb.clone();
                            del_btn.connect_clicked(move |btn| {
                                tracing::info!(
                                    "editor.ft: delete_dir begin path={}",
                                    p.display()
                                );
                                let del_result = be.delete_dir(&p);
                                tracing::info!(
                                    "editor.ft: delete_dir result ok={}",
                                    del_result.is_ok()
                                );
                                if del_result.is_ok() {
                                    if let Some(ref cb) = delete_cb {
                                        cb(&p);
                                    }
                                    refresh_tree();
                                    tracing::info!("editor.ft: delete_dir refresh_tree scheduled");
                                }
                                if let Some(pop) = btn.ancestor(gtk4::Popover::static_type()) {
                                    pop.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                                }
                                tracing::info!("editor.ft: delete_dir popdown done");
                            });
                        }
                        menu_box.append(&del_btn);
                    } else {
                        let dup_btn = make_item("document-save-as-symbolic", "Duplicate File");
                        {
                            let p = path.clone();
                            let be = backend.clone();
                            let refresh_tree = refresh_tree.clone();
                            dup_btn.connect_clicked(move |btn| {
                                let result = if let Some(ext) = p.extension() {
                                    let stem = p.file_stem().unwrap_or_default().to_string_lossy();
                                    let new_name =
                                        format!("{}_copy.{}", stem, ext.to_string_lossy());
                                    let dest = p.with_file_name(new_name);
                                    be.copy_file(&p, &dest)
                                } else {
                                    let name = p.file_name().unwrap_or_default().to_string_lossy();
                                    let dest = p.with_file_name(format!("{}_copy", name));
                                    be.copy_file(&p, &dest)
                                };
                                if result.is_ok() {
                                    refresh_tree();
                                }
                                if let Some(pop) = btn.ancestor(gtk4::Popover::static_type()) {
                                    pop.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                                }
                            });
                        }
                        menu_box.append(&dup_btn);

                        let del_btn = make_item("user-trash-symbolic", "Delete File");
                        {
                            let p = path.clone();
                            let be = backend.clone();
                            let refresh_tree = refresh_tree.clone();
                            let delete_cb = delete_cb.clone();
                            del_btn.connect_clicked(move |btn| {
                                tracing::info!(
                                    "editor.ft: delete_file begin path={}",
                                    p.display()
                                );
                                let del_result = be.delete_file(&p);
                                tracing::info!(
                                    "editor.ft: delete_file result ok={}",
                                    del_result.is_ok()
                                );
                                if del_result.is_ok() {
                                    if let Some(ref cb) = delete_cb {
                                        cb(&p);
                                    }
                                    refresh_tree();
                                    tracing::info!("editor.ft: delete_file refresh_tree scheduled");
                                }
                                if let Some(pop) = btn.ancestor(gtk4::Popover::static_type()) {
                                    pop.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                                }
                                tracing::info!("editor.ft: delete_file popdown done");
                            });
                        }
                        menu_box.append(&del_btn);

                        menu_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

                        if let Some(ref ctx) = ctx_cb {
                            let hist_btn =
                                make_item("document-open-recent-symbolic", "Git History");
                            let cb = ctx.clone();
                            let p = path.clone();
                            hist_btn.connect_clicked(move |btn| {
                                cb("git-history", &p);
                                if let Some(pop) = btn.ancestor(gtk4::Popover::static_type()) {
                                    pop.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                                }
                            });
                            menu_box.append(&hist_btn);
                        }
                    }
                }

                let popover = gtk4::Popover::new();
                crate::theme::configure_popover(&popover);
                popover.set_child(Some(&menu_box));
                popover.set_parent(lb);
                popover.connect_closed(|popover| {
                    if popover.parent().is_some() {
                        popover.unparent();
                    }
                });
                popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                popover.popup();
            });
            list_box.add_controller(gesture);
        }

        // Collapse all button
        {
            let root = root_dir.to_path_buf();
            let lb = list_box.clone();
            let sw = scroll.clone();
            let entries_c = entries.clone();
            let fi = file_index.clone();
            let be_ref = backend.clone();
            let seq = request_seq.clone();
            collapse_btn.connect_clicked(move |_| {
                request_tree_reload(
                    &lb,
                    &sw,
                    &root,
                    &entries_c,
                    &fi,
                    be_ref.clone(),
                    seq.clone(),
                    Vec::new(),
                    false,
                    "Collapsing files...",
                    "No files found",
                );
            });
        }

        let tree = Self {
            widget: container,
            list_box,
            scroll,
            root_dir: root_dir.to_path_buf(),
            on_file_open: Some(on_file_open),
            file_index,
            entries,
            on_context_action,
            on_file_renamed,
            on_path_deleted,
            backend,
            request_seq,
        };

        request_tree_reload(
            &tree.list_box,
            &tree.scroll,
            &tree.root_dir,
            &tree.entries,
            &tree.file_index,
            tree.backend.clone(),
            tree.request_seq.clone(),
            Vec::new(),
            !is_remote,
            if is_remote {
                "Connecting to remote host..."
            } else {
                "Loading files..."
            },
            "No files found",
        );

        tree
    }

    /// Rebuild the tree. Call when file system changes are detected.
    pub fn refresh(&self) {
        request_tree_reload(
            &self.list_box,
            &self.scroll,
            &self.root_dir,
            &self.entries,
            &self.file_index,
            self.backend.clone(),
            self.request_seq.clone(),
            collect_expanded_dirs(&self.entries.borrow()),
            false,
            "Refreshing files...",
            "No files found",
        );
    }

    /// Expand all parent directories of the given file and scroll to it.
    pub fn reveal_file(&self, file_path: &Path) {
        let ancestors = reveal_ancestor_dirs(&self.root_dir, file_path);

        // Expand each ancestor if not already expanded
        let mut changed = false;
        for ancestor in &ancestors {
            let needs_expand = {
                let ents = self.entries.borrow();
                ents.iter()
                    .any(|e| e.path == *ancestor && e.is_dir && !e.expanded)
            };
            if needs_expand {
                let idx_and_depth = {
                    let ents = self.entries.borrow();
                    ents.iter()
                        .enumerate()
                        .find(|(_, e)| e.path == *ancestor && e.is_dir)
                        .map(|(i, e)| (i, e.depth))
                };
                if let Some((idx, depth)) = idx_and_depth {
                    toggle_dir(
                        &self.entries,
                        &self.file_index,
                        idx,
                        depth,
                        false,
                        ancestor,
                        &*self.backend,
                    );
                    changed = true;
                }
            }
        }

        if changed {
            populate_list_box(&self.list_box, &self.entries.borrow(), &self.root_dir);
        }

        // Find the file row and scroll to it. Repeat on idle because reveal is
        // often triggered immediately after switching the sidebar stack to Files.
        let file_idx = find_entry_index_by_path(&self.entries.borrow(), file_path);
        if let Some(idx) = file_idx {
            select_and_scroll_to_row(&self.list_box, idx);
            let list_box = self.list_box.clone();
            glib::idle_add_local_once(move || {
                select_and_scroll_to_row(&list_box, idx);
            });
        } else {
            tracing::debug!("reveal_file: row not found for {}", file_path.display());
        }
    }
}

/// Indent step in pixels per depth level.
const INDENT_PX: i32 = 16;
/// Width of each guide column.
const GUIDE_W: f64 = 16.0;
/// Compact file tree row height.
const ROW_HEIGHT_PX: i32 = 18;
/// Compact symbolic icon size.
const ROW_ICON_PX: i32 = 14;
/// Width reserved for the directory expander/spacer.
const EXPANDER_WIDTH_PX: i32 = 12;
/// Height assumed for drawing (actual is allocated at render time).
const ROW_H: f64 = 18.0;

/// Build a single row widget for a file entry.
/// `guides` is a bool per depth level (0..depth): true = draw a vertical continuation line.
/// `is_last` is true if this entry is the last sibling at its depth.
fn build_row_widget(entry: &FileEntry, root: &Path, guides: &[bool], is_last: bool) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 1);
    row.set_size_request(-1, ROW_HEIGHT_PX);
    row.set_margin_start(3);
    row.set_margin_top(0);
    row.set_margin_bottom(0);
    row.add_css_class("editor-file-tree-entry");
    if entry.is_ignored {
        row.add_css_class("editor-file-tree-ignored");
        row.set_tooltip_text(Some("Ignored by Git"));
    }

    // Draw tree guide lines via a DrawingArea
    if entry.depth > 0 {
        let depth = entry.depth as usize;
        let width = depth as i32 * INDENT_PX as i32;
        let guides_owned: Vec<bool> = guides.to_vec();
        let is_last_owned = is_last;

        let drawing = gtk4::DrawingArea::new();
        drawing.set_content_width(width);
        drawing.set_content_height(ROW_H as i32);
        drawing.set_size_request(width, -1);

        drawing.set_draw_func(move |_da, cr, w, h| {
            let _ = (w, h);
            let h = h as f64;
            let mid_y = h / 2.0;

            // Use a subtle color for the lines
            cr.set_source_rgba(0.5, 0.5, 0.5, 0.35);
            cr.set_line_width(1.0);

            // Draw vertical continuation lines for ancestor levels
            for (level, &has_sibling) in guides_owned.iter().enumerate() {
                if !has_sibling {
                    continue;
                }
                let x = level as f64 * GUIDE_W + GUIDE_W / 2.0;
                cr.move_to(x + 0.5, 0.0);
                cr.line_to(x + 0.5, h);
            }

            // Draw the connector for this entry's own level (last column)
            let last_level = depth - 1;
            let x = last_level as f64 * GUIDE_W + GUIDE_W / 2.0;
            // Vertical line: from top to mid (or full height if not last)
            cr.move_to(x + 0.5, 0.0);
            if is_last_owned {
                cr.line_to(x + 0.5, mid_y); // └
            } else {
                cr.line_to(x + 0.5, h); // ├
            }
            // Horizontal line: from vertical to the right edge
            cr.move_to(x + 0.5, mid_y + 0.5);
            cr.line_to(depth as f64 * GUIDE_W, mid_y + 0.5);

            let _ = cr.stroke();
        });

        row.append(&drawing);
    }

    if entry.is_dir {
        // +/- expander
        let expander_label = if entry.expanded { "\u{2212}" } else { "+" };
        let expander = gtk4::Label::new(Some(expander_label));
        expander.set_width_request(EXPANDER_WIDTH_PX);
        expander.set_valign(gtk4::Align::Center);
        expander.add_css_class("dim-label");
        row.append(&expander);

        // Folder icon (symbolic, matches app theme)
        let icon_name = entry_icon_name(entry);
        let icon = gtk4::Image::from_icon_name(icon_name);
        icon.set_pixel_size(ROW_ICON_PX);
        icon.set_valign(gtk4::Align::Center);
        row.append(&icon);
    } else {
        // Spacer to align with dirs (expander width)
        let spacer = gtk4::Label::new(None);
        spacer.set_width_request(EXPANDER_WIDTH_PX);
        row.append(&spacer);

        let icon = gtk4::Image::from_icon_name(entry_icon_name(entry));
        icon.set_pixel_size(ROW_ICON_PX);
        icon.set_valign(gtk4::Align::Center);
        row.append(&icon);
    }

    let label = gtk4::Label::new(Some(&entry.name));
    label.set_halign(gtk4::Align::Start);
    label.set_valign(gtk4::Align::Center);
    label.set_hexpand(true);
    label.set_margin_start(3);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    if !entry.is_ignored {
        let rel = entry.path.strip_prefix(root).unwrap_or(&entry.path);
        label.set_tooltip_text(Some(&rel.to_string_lossy()));
    }
    row.append(&label);

    row
}

/// Pick an appropriate symbolic icon for a file based on its extension.
fn file_icon_name(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "rs" | "py" | "js" | "ts" | "c" | "cpp" | "h" | "go" | "java" | "rb" | "sh" | "bash"
        | "zsh" | "lua" | "zig" => "text-x-script-symbolic",
        "json" | "toml" | "yaml" | "yml" | "xml" | "ini" | "conf" => "text-x-generic-symbolic",
        "md" | "txt" | "rst" | "org" => "text-x-generic-symbolic",
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" => "image-x-generic-symbolic",
        "css" | "scss" | "html" | "htm" => "text-html-symbolic",
        "lock" => "changes-prevent-symbolic",
        _ => "text-x-generic-symbolic",
    }
}

fn entry_icon_name(entry: &FileEntry) -> &'static str {
    if entry.is_ignored {
        return "vcs-ignored-symbolic";
    }

    if entry.is_dir {
        if entry.expanded {
            "folder-open-symbolic"
        } else {
            "folder-symbolic"
        }
    } else {
        file_icon_name(&entry.name)
    }
}

/// Pre-compute `is_last` (no next sibling) for every entry in O(n).
fn precompute_is_last(entries: &[FileEntry]) -> Vec<bool> {
    let n = entries.len();
    let mut is_last = vec![true; n];
    let max_depth = entries.iter().map(|e| e.depth as usize).max().unwrap_or(0);
    let mut seen_at_depth = vec![false; max_depth + 2];

    for i in (0..n).rev() {
        let d = entries[i].depth as usize;
        for level in (d + 1)..seen_at_depth.len() {
            seen_at_depth[level] = false;
        }
        is_last[i] = !seen_at_depth[d];
        seen_at_depth[d] = true;
    }
    is_last
}

/// Pre-computed guide data for a single row.
struct RowGuides {
    guides: Vec<bool>,
    is_last: bool,
}

/// Pre-compute guide arrays and is_last for all entries in O(n).
fn precompute_all_guides(entries: &[FileEntry]) -> Vec<RowGuides> {
    let is_last = precompute_is_last(entries);
    let mut result = Vec::with_capacity(entries.len());
    let mut active_guides: Vec<bool> = Vec::new();

    for (i, entry) in entries.iter().enumerate() {
        let depth = entry.depth as usize;
        active_guides.resize(depth, false);
        active_guides.truncate(depth);

        let guides = if depth > 0 {
            active_guides[..depth - 1].to_vec()
        } else {
            vec![]
        };

        result.push(RowGuides { guides, is_last: is_last[i] });

        if depth > 0 {
            active_guides.resize(depth, false);
            active_guides[depth - 1] = !is_last[i];
        }
    }
    result
}

/// Build a ListBoxRow from an entry and its guide data.
fn make_list_row(entry: &FileEntry, root: &Path, guide: &RowGuides) -> gtk4::ListBoxRow {
    let row_widget = build_row_widget(entry, root, &guide.guides, guide.is_last);
    let list_row = gtk4::ListBoxRow::new();
    list_row.add_css_class("editor-file-tree-row");
    list_row.set_selectable(true);
    list_row.set_activatable(true);
    list_row.set_child(Some(&row_widget));
    list_row
}

/// Populate the ListBox from entries (full rebuild, used for initial load and refresh).
fn populate_list_box(list_box: &gtk4::ListBox, entries: &[FileEntry], root: &Path) {
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }
    let guide_data = precompute_all_guides(entries);
    for (i, entry) in entries.iter().enumerate() {
        list_box.append(&make_list_row(entry, root, &guide_data[i]));
    }
}

/// Incremental expand: update toggled row and insert new child rows.
fn incremental_expand(
    list_box: &gtk4::ListBox,
    entries: &[FileEntry],
    root: &Path,
    toggle_idx: usize,
    added_count: usize,
) {
    let guide_data = precompute_all_guides(entries);

    // Update the toggled row (folder icon → folder-open, + → −)
    if let Some(row) = list_box.row_at_index(toggle_idx as i32) {
        row.set_child(Some(&build_row_widget(
            &entries[toggle_idx], root,
            &guide_data[toggle_idx].guides, guide_data[toggle_idx].is_last,
        )));
    }

    // Insert new child rows
    for i in 0..added_count {
        let entry_idx = toggle_idx + 1 + i;
        let list_row = make_list_row(&entries[entry_idx], root, &guide_data[entry_idx]);
        list_box.insert(&list_row, entry_idx as i32);
    }
}

/// Incremental collapse: remove child rows and update toggled row.
fn incremental_collapse(
    list_box: &gtk4::ListBox,
    entries: &[FileEntry],
    root: &Path,
    toggle_idx: usize,
    removed_count: usize,
) {
    // Remove child rows (always remove at toggle_idx+1, rows shift up)
    for _ in 0..removed_count {
        if let Some(row) = list_box.row_at_index((toggle_idx + 1) as i32) {
            list_box.remove(&row);
        }
    }

    // Update the toggled row (folder-open → folder, − → +)
    let guide_data = precompute_all_guides(entries);
    if let Some(row) = list_box.row_at_index(toggle_idx as i32) {
        row.set_child(Some(&build_row_widget(
            &entries[toggle_idx], root,
            &guide_data[toggle_idx].guides, guide_data[toggle_idx].is_last,
        )));
    }
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

fn reveal_ancestor_dirs(root_dir: &Path, file_path: &Path) -> Vec<PathBuf> {
    let mut ancestors = Vec::new();
    let mut parent = file_path.parent();
    while let Some(path) = parent {
        if path == root_dir {
            break;
        }
        ancestors.push(path.to_path_buf());
        parent = path.parent();
    }
    ancestors.reverse();
    ancestors
}

fn find_entry_index_by_path(entries: &[FileEntry], file_path: &Path) -> Option<usize> {
    entries.iter().position(|entry| entry.path == file_path)
}

fn select_and_scroll_to_row(list_box: &gtk4::ListBox, idx: usize) -> bool {
    let Some(row) = list_box.row_at_index(idx as i32) else {
        return false;
    };
    list_box.select_row(Some(&row));
    row.grab_focus();
    true
}

fn collect_expanded_dirs(entries: &[FileEntry]) -> Vec<PathBuf> {
    entries
        .iter()
        .filter(|entry| entry.is_dir && entry.expanded)
        .map(|entry| entry.path.clone())
        .collect()
}

fn request_tree_reload(
    list_box: &gtk4::ListBox,
    scroll: &gtk4::ScrolledWindow,
    root: &Path,
    entries: &Rc<RefCell<Vec<FileEntry>>>,
    file_index: &Rc<RefCell<Vec<PathBuf>>>,
    backend: Arc<dyn FileBackend>,
    request_seq: Rc<Cell<u64>>,
    expanded_dirs: Vec<PathBuf>,
    expand_root_dirs: bool,
    loading_message: &'static str,
    empty_message: &'static str,
) {
    let request_id = request_seq.get().wrapping_add(1);
    request_seq.set(request_id);
    populate_message(list_box, loading_message);
    tracing::info!(
        "editor.ft: request_tree_reload req={} root={} expanded_dirs={}",
        request_id,
        root.display(),
        expanded_dirs.len()
    );

    let root_c = root.to_path_buf();
    let build_root = root_c.clone();
    let list_box_c = list_box.clone();
    let scroll_c = scroll.clone();
    let entries_c = entries.clone();
    let file_index_c = file_index.clone();
    let backend_for_task = backend.clone();
    let request_seq_c = request_seq.clone();
    let scroll_pos = scroll.vadjustment().value();
    let retry_root = root.to_path_buf();
    let retry_list_box = list_box.clone();
    let retry_scroll = scroll.clone();
    let retry_entries = entries.clone();
    let retry_file_index = file_index.clone();
    let retry_backend = backend.clone();
    let retry_seq = request_seq.clone();
    let retry_expanded_dirs = expanded_dirs.clone();

    run_blocking(
        move || {
            tracing::info!("editor.ft: build_tree_snapshot thread begin req={}", request_id);
            let result = build_tree_snapshot(
                &build_root,
                &expanded_dirs,
                &*backend_for_task,
                expand_root_dirs,
            );
            tracing::info!(
                "editor.ft: build_tree_snapshot thread end req={} ok={}",
                request_id,
                result.is_ok()
            );
            result
        },
        move |result| {
            if request_seq_c.get() != request_id {
                tracing::info!(
                    "editor.ft: request_tree_reload req={} superseded",
                    request_id
                );
                return;
            }

            match result {
                Ok(snapshot) if snapshot.entries.is_empty() => {
                    populate_message(&list_box_c, empty_message);
                    tracing::info!(
                        "editor.ft: request_tree_reload req={} done (empty)",
                        request_id
                    );
                }
                Ok(snapshot) => {
                    let n = snapshot.entries.len();
                    *entries_c.borrow_mut() = snapshot.entries;
                    *file_index_c.borrow_mut() = snapshot.file_index;
                    populate_list_box(&list_box_c, &entries_c.borrow(), &root_c);
                    scroll_c.vadjustment().set_value(scroll_pos);
                    tracing::info!(
                        "editor.ft: request_tree_reload req={} done entries={}",
                        request_id,
                        n
                    );
                }
                Err(_) if backend.is_remote() => {
                    populate_message(&list_box_c, "SSH not connected — retrying...");
                    glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
                        if retry_seq.get() != request_id {
                            return glib::ControlFlow::Break;
                        }
                        request_tree_reload(
                            &retry_list_box,
                            &retry_scroll,
                            &retry_root,
                            &retry_entries,
                            &retry_file_index,
                            retry_backend.clone(),
                            retry_seq.clone(),
                            retry_expanded_dirs.clone(),
                            expand_root_dirs,
                            "Connecting to remote host...",
                            empty_message,
                        );
                        glib::ControlFlow::Break
                    });
                }
                Err(err) => {
                    populate_message(&list_box_c, &format!("Unable to load files: {err}"));
                }
            }
        },
    );
}

/// Toggle a directory open/closed and rebuild entries list accordingly.
fn toggle_dir(
    entries: &Rc<RefCell<Vec<FileEntry>>>,
    file_index: &Rc<RefCell<Vec<PathBuf>>>,
    idx: usize,
    depth: u32,
    was_expanded: bool,
    dir_path: &Path,
    backend: &dyn FileBackend,
) {
    let mut ents = entries.borrow_mut();

    if was_expanded {
        // Collapse: remove all children with depth > this entry's depth
        ents[idx].expanded = false;
        let remove_start = idx + 1;
        let mut remove_end = remove_start;
        while remove_end < ents.len() && ents[remove_end].depth > depth {
            remove_end += 1;
        }
        {
            let removed_paths: Vec<PathBuf> = ents[remove_start..remove_end]
                .iter()
                .filter(|e| !e.is_dir)
                .map(|e| e.path.clone())
                .collect();
            let mut fi = file_index.borrow_mut();
            fi.retain(|p| !removed_paths.contains(p));
        }
        ents.drain(remove_start..remove_end);
    } else {
        // Expand: insert children after this entry
        ents[idx].expanded = true;
        let mut new_entries = Vec::new();
        let mut new_index = Vec::new();
        if build_file_entries(
            dir_path,
            &mut new_entries,
            &mut new_index,
            depth + 1,
            backend,
        )
        .is_err()
        {
            ents[idx].expanded = false;
            return;
        }
        file_index.borrow_mut().extend(new_index);
        let insert_pos = idx + 1;
        for (i, entry) in new_entries.into_iter().enumerate() {
            ents.insert(insert_pos + i, entry);
        }
    }
}

/// Restore expanded directories after a refresh.
fn restore_expanded(
    entries: &mut Vec<FileEntry>,
    file_index: &mut Vec<PathBuf>,
    expanded_dirs: &[PathBuf],
    backend: &dyn FileBackend,
) -> Result<(), String> {
    let expanded_dirs: std::collections::HashSet<PathBuf> = expanded_dirs.iter().cloned().collect();
    let mut i = 0;
    while i < entries.len() {
        if entries[i].is_dir && !entries[i].expanded && expanded_dirs.contains(&entries[i].path) {
            let depth = entries[i].depth;
            let dir_path = entries[i].path.clone();
            entries[i].expanded = true;
            let mut new_entries = Vec::new();
            let mut new_index = Vec::new();
            build_file_entries(
                &dir_path,
                &mut new_entries,
                &mut new_index,
                depth + 1,
                backend,
            )?;
            file_index.extend(new_index);
            let insert_pos = i + 1;
            for (j, entry) in new_entries.into_iter().enumerate() {
                entries.insert(insert_pos + j, entry);
            }
        }
        i += 1;
    }
    Ok(())
}

fn build_tree_snapshot(
    root: &Path,
    expanded_dirs: &[PathBuf],
    backend: &dyn FileBackend,
    expand_root_dirs: bool,
) -> Result<TreeSnapshot, String> {
    let mut snapshot = TreeSnapshot::default();
    build_collapsed_entries(
        root,
        &mut snapshot.entries,
        &mut snapshot.file_index,
        0,
        backend,
    )?;

    let mut dirs_to_expand = expanded_dirs.to_vec();
    if expand_root_dirs && !backend.is_remote() {
        dirs_to_expand.extend(
            snapshot
                .entries
                .iter()
                .filter(|entry| entry.is_dir && entry.depth == 0)
                .map(|entry| entry.path.clone()),
        );
        dirs_to_expand.sort();
        dirs_to_expand.dedup();
    }

    restore_expanded(
        &mut snapshot.entries,
        &mut snapshot.file_index,
        &dirs_to_expand,
        backend,
    )?;

    Ok(snapshot)
}

fn build_collapsed_entries(
    dir: &Path,
    entries: &mut Vec<FileEntry>,
    file_index: &mut Vec<PathBuf>,
    depth: u32,
    backend: &dyn FileBackend,
) -> Result<(), String> {
    let (dirs, files) = list_directory_entries(dir, backend)?;

    for (path, name, is_ignored) in dirs {
        entries.push(FileEntry {
            path,
            name,
            is_dir: true,
            is_ignored,
            depth,
            expanded: false,
        });
    }

    for (path, name, is_ignored) in files {
        file_index.push(path.clone());
        entries.push(FileEntry {
            path,
            name,
            is_dir: false,
            is_ignored,
            depth,
            expanded: false,
        });
    }

    Ok(())
}

/// Recursively build file entries using the `ignore` crate for .gitignore support.
fn build_file_entries(
    dir: &Path,
    entries: &mut Vec<FileEntry>,
    file_index: &mut Vec<PathBuf>,
    depth: u32,
    backend: &dyn FileBackend,
) -> Result<(), String> {
    let (dirs, files) = list_directory_entries(dir, backend)?;

    // Remote: don't auto-expand (each expand is an SSH call)
    let auto_expand_depth = if backend.is_remote() { 0 } else { 1 };

    for (path, name, is_ignored) in dirs {
        let auto_expand = depth < auto_expand_depth;
        entries.push(FileEntry {
            path: path.clone(),
            name,
            is_dir: true,
            is_ignored,
            depth,
            expanded: auto_expand,
        });
        if auto_expand {
            build_file_entries(&path, entries, file_index, depth + 1, backend)?;
        }
    }

    for (path, name, is_ignored) in files {
        file_index.push(path.clone());
        entries.push(FileEntry {
            path,
            name,
            is_dir: false,
            is_ignored,
            depth,
            expanded: false,
        });
    }

    Ok(())
}

fn list_directory_entries(
    dir: &Path,
    backend: &dyn FileBackend,
) -> Result<(Vec<(PathBuf, String, bool)>, Vec<(PathBuf, String, bool)>), String> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for de in backend.list_dir(dir)? {
        let path = dir.join(&de.name);
        if de.is_dir {
            dirs.push((path, de.name, de.is_ignored));
        } else {
            files.push((path, de.name, de.is_ignored));
        }
    }

    dirs.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    files.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    Ok((dirs, files))
}

fn rename_destination_for_path(path: &Path, new_name: &str) -> Option<PathBuf> {
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut components = Path::new(trimmed).components();
    let component = components.next()?;
    if components.next().is_some() {
        return None;
    }

    match component {
        std::path::Component::Normal(name) => Some(path.with_file_name(name)),
        _ => None,
    }
}

fn creation_destination_for_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut components = Path::new(trimmed).components();
    let component = components.next()?;
    if components.next().is_some() {
        return None;
    }

    match component {
        std::path::Component::Normal(name) => Some(dir.join(name)),
        _ => None,
    }
}

fn resolve_tree_selection(
    root: &Path,
    entries: &[FileEntry],
    clicked_index: Option<usize>,
    selected_index: Option<usize>,
) -> Option<FileEntry> {
    clicked_index
        .or(selected_index)
        .and_then(|idx| entries.get(idx).cloned())
        .or_else(|| entries.iter().find(|entry| entry.path == root).cloned())
}

fn creation_target_dir(
    root: &Path,
    entries: &[FileEntry],
    clicked_index: Option<usize>,
    selected_index: Option<usize>,
) -> PathBuf {
    match resolve_tree_selection(root, entries, clicked_index, selected_index) {
        Some(entry) if entry.is_dir => entry.path,
        Some(entry) => entry
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf()),
        None => root.to_path_buf(),
    }
}

fn find_transient_parent_window(anchor: &impl IsA<gtk4::Widget>) -> Option<gtk4::Window> {
    anchor
        .root()
        .and_then(|root| root.downcast::<gtk4::Window>().ok())
}

fn show_name_input_dialog(
    transient_parent: Option<&gtk4::Window>,
    title: &str,
    button_label: &str,
    initial_value: &str,
    on_submit: Rc<dyn Fn(String)>,
) {
    let dialog = gtk4::Window::builder()
        .title(title)
        .modal(true)
        .default_width(360)
        .default_height(110)
        .build();
    if let Some(win) = transient_parent {
        dialog.set_transient_for(Some(win));
    }
    dialog.set_destroy_with_parent(true);
    crate::theme::configure_dialog_window(&dialog);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let entry = gtk4::Entry::new();
    entry.set_text(initial_value);
    vbox.append(&entry);

    let button_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    button_row.set_halign(gtk4::Align::End);
    let cancel_btn = gtk4::Button::with_label("Cancel");
    let ok_btn = gtk4::Button::with_label(button_label);
    ok_btn.add_css_class("suggested-action");
    button_row.append(&cancel_btn);
    button_row.append(&ok_btn);
    vbox.append(&button_row);
    dialog.set_child(Some(&vbox));

    {
        let dialog = dialog.clone();
        cancel_btn.connect_clicked(move |_| dialog.close());
    }

    let submit: Rc<dyn Fn()> = Rc::new({
        let dialog = dialog.clone();
        let entry = entry.clone();
        move || {
            on_submit(entry.text().to_string());
            dialog.close();
        }
    });

    {
        let submit = submit.clone();
        ok_btn.connect_clicked(move |_| submit());
    }

    {
        let submit = submit.clone();
        entry.connect_activate(move |_| submit());
    }

    dialog.present();
    entry.grab_focus();
    entry.set_position(-1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::editor::file_backend::LocalFileBackend;
    use tempfile::tempdir;

    #[test]
    fn build_tree_snapshot_expands_root_dirs_for_local_projects() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# demo\n").unwrap();
        let backend = LocalFileBackend::new(dir.path());

        let snapshot = build_tree_snapshot(dir.path(), &[], &backend, true).unwrap();

        assert!(snapshot.entries.iter().any(|entry| entry.name == "src"
            && entry.is_dir
            && entry.depth == 0
            && entry.expanded));
        assert!(snapshot
            .entries
            .iter()
            .any(|entry| entry.name == "main.rs" && !entry.is_dir && entry.depth == 1));
        assert!(snapshot
            .entries
            .iter()
            .any(|entry| entry.name == "README.md" && !entry.is_dir && entry.depth == 0));
    }

    #[test]
    fn build_tree_snapshot_preserves_only_requested_expanded_dirs() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("tests")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            dir.path().join("tests/app.rs"),
            "#[test]\nfn it_works() {}\n",
        )
        .unwrap();
        let backend = LocalFileBackend::new(dir.path());

        let snapshot =
            build_tree_snapshot(dir.path(), &[dir.path().join("tests")], &backend, false).unwrap();

        assert!(snapshot
            .entries
            .iter()
            .any(|entry| entry.name == "tests" && entry.is_dir && entry.expanded));
        assert!(snapshot
            .entries
            .iter()
            .any(|entry| entry.name == "app.rs" && !entry.is_dir && entry.depth == 1));
        assert!(snapshot
            .entries
            .iter()
            .any(|entry| entry.name == "src" && entry.is_dir && !entry.expanded));
        assert!(!snapshot
            .entries
            .iter()
            .any(|entry| entry.name == "main.rs" && !entry.is_dir));
    }

    #[test]
    fn rename_destination_replaces_only_the_basename() {
        let path = Path::new("/tmp/demo/src/main.rs");

        assert_eq!(
            rename_destination_for_path(path, "lib.rs"),
            Some(PathBuf::from("/tmp/demo/src/lib.rs"))
        );
        assert_eq!(
            rename_destination_for_path(Path::new("/tmp/demo/assets"), "static"),
            Some(PathBuf::from("/tmp/demo/static"))
        );
    }

    #[test]
    fn rename_destination_rejects_empty_or_nested_names() {
        let path = Path::new("/tmp/demo/src/main.rs");

        assert_eq!(rename_destination_for_path(path, ""), None);
        assert_eq!(rename_destination_for_path(path, "  "), None);
        assert_eq!(rename_destination_for_path(path, "../other.rs"), None);
        assert_eq!(rename_destination_for_path(path, "nested/other.rs"), None);
    }

    #[test]
    fn creation_target_prefers_clicked_directory() {
        let root = PathBuf::from("/tmp/demo");
        let entries = vec![
            FileEntry {
                path: root.join("src"),
                name: "src".into(),
                is_dir: true,
                is_ignored: false,
                depth: 0,
                expanded: false,
            },
            FileEntry {
                path: root.join("src/main.rs"),
                name: "main.rs".into(),
                is_dir: false,
                is_ignored: false,
                depth: 1,
                expanded: false,
            },
        ];

        assert_eq!(
            creation_target_dir(&root, &entries, Some(0), Some(1)),
            root.join("src")
        );
    }

    #[test]
    fn creation_target_uses_selected_folder_or_file_parent() {
        let root = PathBuf::from("/tmp/demo");
        let entries = vec![
            FileEntry {
                path: root.join("src"),
                name: "src".into(),
                is_dir: true,
                is_ignored: false,
                depth: 0,
                expanded: false,
            },
            FileEntry {
                path: root.join("src/main.rs"),
                name: "main.rs".into(),
                is_dir: false,
                is_ignored: false,
                depth: 1,
                expanded: false,
            },
        ];

        assert_eq!(
            creation_target_dir(&root, &entries, None, Some(0)),
            root.join("src")
        );
        assert_eq!(
            creation_target_dir(&root, &entries, None, Some(1)),
            root.join("src")
        );
        assert_eq!(creation_target_dir(&root, &entries, None, None), root);
    }

    #[test]
    fn creation_destination_rejects_empty_or_nested_names() {
        let dir = Path::new("/tmp/demo/src");

        assert_eq!(creation_destination_for_dir(dir, ""), None);
        assert_eq!(creation_destination_for_dir(dir, "  "), None);
        assert_eq!(creation_destination_for_dir(dir, "../other.rs"), None);
        assert_eq!(creation_destination_for_dir(dir, "nested/other.rs"), None);
        assert_eq!(
            creation_destination_for_dir(dir, "new.rs"),
            Some(PathBuf::from("/tmp/demo/src/new.rs"))
        );
    }

    #[test]
    fn reveal_ancestor_dirs_are_root_first_and_exclude_root() {
        let root = PathBuf::from("/tmp/demo");
        let file = root.join("src/ui/main.rs");

        assert_eq!(
            reveal_ancestor_dirs(&root, &file),
            vec![root.join("src"), root.join("src/ui")]
        );
    }

    #[test]
    fn file_tree_row_metrics_are_compact() {
        assert_eq!(ROW_HEIGHT_PX, 18);
        assert_eq!(ROW_H, 18.0);
        assert_eq!(ROW_ICON_PX, 14);
        assert_eq!(EXPANDER_WIDTH_PX, 12);
    }

    #[test]
    fn find_transient_parent_window_uses_widget_root_window() {
        if gtk4::init().is_err() {
            return;
        }

        let window = gtk4::Window::new();
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let button = gtk4::Button::with_label("Create");
        container.append(&button);
        window.set_child(Some(&container));

        let resolved = find_transient_parent_window(&button).expect("parent window");

        assert_eq!(resolved, window);
    }

    #[test]
    fn ignored_entries_use_dedicated_gitignore_icon() {
        let ignored_file = FileEntry {
            path: PathBuf::from("/tmp/demo/ignored.log"),
            name: "ignored.log".into(),
            is_dir: false,
            is_ignored: true,
            depth: 0,
            expanded: false,
        };
        let ignored_dir = FileEntry {
            path: PathBuf::from("/tmp/demo/ignored_dir"),
            name: "ignored_dir".into(),
            is_dir: true,
            is_ignored: true,
            depth: 0,
            expanded: false,
        };

        assert_eq!(entry_icon_name(&ignored_file), "vcs-ignored-symbolic");
        assert_eq!(entry_icon_name(&ignored_dir), "vcs-ignored-symbolic");
    }
}
