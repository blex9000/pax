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
    backend: Arc<dyn FileBackend>,
    request_seq: Rc<Cell<u64>>,
}

#[derive(Debug, Clone)]
struct FileEntry {
    path: PathBuf,
    name: String,
    is_dir: bool,
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
        Self::new_with_context(root_dir, on_file_open, None, backend)
    }

    pub fn new_with_context(
        root_dir: &Path,
        on_file_open: OnFileOpen,
        on_context_action: Option<OnContextAction>,
        backend: Arc<dyn FileBackend>,
    ) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("editor-file-tree");

        // Action buttons bar at bottom
        let actions_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        actions_bar.add_css_class("editor-sidebar-toolbar");
        actions_bar.set_margin_start(4);
        actions_bar.set_margin_end(4);
        actions_bar.set_margin_bottom(2);

        let collapse_btn = gtk4::Button::from_icon_name("view-list-symbolic");
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
                    // Save scroll position before rebuilding
                    let vadj = sw.vadjustment();
                    let scroll_pos = vadj.value();
                    toggle_dir(&entries_c, &fi, idx, depth, expanded, &path, &*be);
                    populate_list_box(lb, &entries_c.borrow(), &root);
                    // Restore scroll position after rebuild
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
                let selected_entry = resolve_tree_selection(&root, &ents, clicked_idx, selected_idx);
                let selected_path = selected_entry
                    .as_ref()
                    .map(|entry| entry.path.clone())
                    .unwrap_or_else(|| root.clone());
                let rel = selected_path
                    .strip_prefix(&root)
                    .unwrap_or(&selected_path)
                    .to_path_buf();
                let target_dir =
                    creation_target_dir(&root, &ents, clicked_idx, selected_idx);
                let selected_is_dir = selected_entry
                    .as_ref()
                    .map(|entry| entry.is_dir)
                    .unwrap_or(true);
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
                    create_file_btn.connect_clicked(move |btn| {
                        if let Some(pop) = btn.ancestor(gtk4::Popover::static_type()) {
                            pop.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                        }
                        let target_dir = target_dir.clone();
                        let be = be.clone();
                        let refresh_tree = refresh_tree.clone();
                        let on_open = on_open.clone();
                        show_name_input_dialog(
                            btn.upcast_ref::<gtk4::Widget>(),
                            "New File",
                            "Create File",
                            "",
                            Rc::new(move |name| {
                                let Some(dest) = creation_destination_for_dir(&target_dir, &name)
                                else {
                                    return;
                                };
                                if be.write_file(&dest, "").is_ok() {
                                    refresh_tree();
                                    on_open(&dest);
                                }
                            }),
                        );
                    });
                }
                menu_box.append(&create_file_btn);

                let create_folder_btn = make_item("folder-new-symbolic", "New Folder");
                {
                    let target_dir = target_dir.clone();
                    let be = backend.clone();
                    let refresh_tree = refresh_tree.clone();
                    create_folder_btn.connect_clicked(move |btn| {
                        if let Some(pop) = btn.ancestor(gtk4::Popover::static_type()) {
                            pop.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                        }
                        let target_dir = target_dir.clone();
                        let be = be.clone();
                        let refresh_tree = refresh_tree.clone();
                        show_name_input_dialog(
                            btn.upcast_ref::<gtk4::Widget>(),
                            "New Folder",
                            "Create Folder",
                            "",
                            Rc::new(move |name| {
                                let Some(dest) = creation_destination_for_dir(&target_dir, &name)
                                else {
                                    return;
                                };
                                if be.create_dir(&dest).is_ok() {
                                    refresh_tree();
                                }
                            }),
                        );
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
                        rename_btn.connect_clicked(move |btn| {
                            if let Some(pop) = btn.ancestor(gtk4::Popover::static_type()) {
                                pop.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                            }
                            let current_name = p
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            let initial_name = current_name.clone();
                            let p = p.clone();
                            let be = be.clone();
                            let refresh_tree = refresh_tree.clone();
                            show_name_input_dialog(
                                btn.upcast_ref::<gtk4::Widget>(),
                                "Rename",
                                if is_dir { "Rename Folder" } else { "Rename File" },
                                &initial_name,
                                Rc::new(move |new_name| {
                                    if new_name == current_name {
                                        return;
                                    }
                                    if let Some(dest) = rename_destination_for_path(&p, &new_name)
                                    {
                                        if be.rename_file(&p, &dest).is_ok() {
                                            refresh_tree();
                                        }
                                    }
                                }),
                            );
                        });
                    }
                    menu_box.append(&rename_btn);

                    if !is_dir {
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
                            del_btn.connect_clicked(move |btn| {
                                if be.delete_file(&p).is_ok() {
                                    refresh_tree();
                                }
                                if let Some(pop) = btn.ancestor(gtk4::Popover::static_type()) {
                                    pop.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                                }
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
                popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(
                    x as i32, y as i32, 1, 1,
                )));
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
        // Build list of ancestor directories that need expanding
        let mut ancestors: Vec<PathBuf> = Vec::new();
        let mut parent = file_path.parent();
        while let Some(p) = parent {
            if p == self.root_dir {
                break;
            }
            ancestors.push(p.to_path_buf());
            parent = p.parent();
        }
        ancestors.reverse(); // root-first order

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

        // Find the file row and scroll to it
        let file_idx = {
            let ents = self.entries.borrow();
            ents.iter().position(|e| e.path == file_path)
        };
        if let Some(idx) = file_idx {
            if let Some(row) = self.list_box.row_at_index(idx as i32) {
                self.list_box.select_row(Some(&row));
                // Scroll to make the row visible
                row.grab_focus();
            }
        }
    }
}

/// Indent step in pixels per depth level.
const INDENT_PX: i32 = 16;
/// Width of each guide column.
const GUIDE_W: f64 = 16.0;
/// Height assumed for drawing (actual is allocated at render time).
const ROW_H: f64 = 24.0;

/// Build a single row widget for a file entry.
/// `guides` is a bool per depth level (0..depth): true = draw a vertical continuation line.
/// `is_last` is true if this entry is the last sibling at its depth.
fn build_row_widget(entry: &FileEntry, root: &Path, guides: &[bool], is_last: bool) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    row.set_margin_start(4);
    row.set_margin_top(0);
    row.set_margin_bottom(0);

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
        expander.set_width_request(14);
        expander.add_css_class("dim-label");
        row.append(&expander);

        // Folder icon (symbolic, matches app theme)
        let icon_name = if entry.expanded {
            "folder-open-symbolic"
        } else {
            "folder-symbolic"
        };
        let icon = gtk4::Image::from_icon_name(icon_name);
        icon.set_pixel_size(16);
        row.append(&icon);
    } else {
        // Spacer to align with dirs (expander width)
        let spacer = gtk4::Label::new(None);
        spacer.set_width_request(14);
        row.append(&spacer);

        let icon = gtk4::Image::from_icon_name(file_icon_name(&entry.name));
        icon.set_pixel_size(16);
        row.append(&icon);
    }

    let label = gtk4::Label::new(Some(&entry.name));
    label.set_halign(gtk4::Align::Start);
    label.set_hexpand(true);
    label.set_margin_start(4);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    let rel = entry.path.strip_prefix(root).unwrap_or(&entry.path);
    label.set_tooltip_text(Some(&rel.to_string_lossy()));
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

/// Check if entry at `idx` has a following sibling at the same depth
/// (i.e., there's a later entry at the same depth before we go back to a shallower depth).
fn has_next_sibling(entries: &[FileEntry], idx: usize) -> bool {
    let depth = entries[idx].depth;
    for e in &entries[idx + 1..] {
        if e.depth == depth {
            return true;
        }
        if e.depth < depth {
            return false;
        }
    }
    false
}

/// Populate the ListBox from entries.
fn populate_list_box(list_box: &gtk4::ListBox, entries: &[FileEntry], root: &Path) {
    // Remove all existing rows
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }

    // For each entry, compute guide lines and is_last status.
    // `active_guides[d]` = true means there's a continuation line at depth d
    // (i.e., the parent at that depth still has more siblings below).
    let mut active_guides: Vec<bool> = Vec::new();

    for (i, entry) in entries.iter().enumerate() {
        let depth = entry.depth as usize;

        // Ensure active_guides has enough levels
        active_guides.resize(depth, false);
        active_guides.truncate(depth);

        let is_last = !has_next_sibling(entries, i);

        // Guides for this row: for levels 0..depth-1, use the active state.
        // The last level (depth-1) is drawn as the connector (├ or └), not as
        // a continuation guide — so we pass active_guides[0..depth-1] and let
        // build_row_widget draw the connector separately.
        let guides: Vec<bool> = if depth > 0 {
            active_guides[..depth - 1].to_vec()
        } else {
            vec![]
        };

        let row_widget = build_row_widget(entry, root, &guides, is_last);
        list_box.append(&row_widget);

        // Update active_guides: if this entry has a next sibling at its depth,
        // mark its depth level as active (for children to draw │).
        if depth > 0 {
            // Set the parent guide for this depth
            if active_guides.len() < depth {
                active_guides.resize(depth, false);
            }
            active_guides[depth - 1] = !is_last;
        }
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
            build_tree_snapshot(
                &build_root,
                &expanded_dirs,
                &*backend_for_task,
                expand_root_dirs,
            )
        },
        move |result| {
            if request_seq_c.get() != request_id {
                return;
            }

            match result {
                Ok(snapshot) if snapshot.entries.is_empty() => {
                    populate_message(&list_box_c, empty_message);
                }
                Ok(snapshot) => {
                    *entries_c.borrow_mut() = snapshot.entries;
                    *file_index_c.borrow_mut() = snapshot.file_index;
                    populate_list_box(&list_box_c, &entries_c.borrow(), &root_c);
                    scroll_c.vadjustment().set_value(scroll_pos);
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

    for (path, name) in dirs {
        entries.push(FileEntry {
            path,
            name,
            is_dir: true,
            depth,
            expanded: false,
        });
    }

    for (path, name) in files {
        file_index.push(path.clone());
        entries.push(FileEntry {
            path,
            name,
            is_dir: false,
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

    for (path, name) in dirs {
        let auto_expand = depth < auto_expand_depth;
        entries.push(FileEntry {
            path: path.clone(),
            name,
            is_dir: true,
            depth,
            expanded: auto_expand,
        });
        if auto_expand {
            build_file_entries(&path, entries, file_index, depth + 1, backend)?;
        }
    }

    for (path, name) in files {
        file_index.push(path.clone());
        entries.push(FileEntry {
            path,
            name,
            is_dir: false,
            depth,
            expanded: false,
        });
    }

    Ok(())
}

fn list_directory_entries(
    dir: &Path,
    backend: &dyn FileBackend,
) -> Result<(Vec<(PathBuf, String)>, Vec<(PathBuf, String)>), String> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    if backend.is_remote() {
        for de in backend.list_dir(dir)? {
            let path = dir.join(&de.name);
            if de.is_dir {
                dirs.push((path, de.name));
            } else {
                files.push((path, de.name));
            }
        }
    } else {
        let walker = ignore::WalkBuilder::new(dir)
            .max_depth(Some(1))
            .sort_by_file_name(|a, b| a.cmp(b))
            .build();

        for entry in walker.flatten() {
            let path = entry.path().to_path_buf();
            if path == dir {
                continue;
            }

            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            if path.is_dir() {
                dirs.push((path, name));
            } else {
                files.push((path, name));
            }
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
        .or_else(|| {
            entries
                .iter()
                .find(|entry| entry.path == root)
                .cloned()
        })
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

fn show_name_input_dialog(
    anchor: &impl IsA<gtk4::Widget>,
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
    if let Some(win) = anchor.root().and_then(|r| r.downcast::<gtk4::Window>().ok()) {
        dialog.set_transient_for(Some(&win));
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
                depth: 0,
                expanded: false,
            },
            FileEntry {
                path: root.join("src/main.rs"),
                name: "main.rs".into(),
                is_dir: false,
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
                depth: 0,
                expanded: false,
            },
            FileEntry {
                path: root.join("src/main.rs"),
                name: "main.rs".into(),
                is_dir: false,
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
}
