use gtk4::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::file_backend::FileBackend;

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
    backend: Rc<dyn FileBackend>,
}

#[derive(Debug, Clone)]
struct FileEntry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    depth: u32,
    expanded: bool,
}

impl FileTree {
    pub fn new(root_dir: &Path, on_file_open: OnFileOpen, backend: Rc<dyn FileBackend>) -> Self {
        Self::new_with_context(root_dir, on_file_open, None, backend)
    }

    pub fn new_with_context(root_dir: &Path, on_file_open: OnFileOpen, on_context_action: Option<OnContextAction>, backend: Rc<dyn FileBackend>) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Action buttons bar at bottom
        let actions_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        actions_bar.set_margin_start(4);
        actions_bar.set_margin_end(4);
        actions_bar.set_margin_bottom(2);

        let new_file_btn = gtk4::Button::from_icon_name("document-new-symbolic");
        new_file_btn.add_css_class("flat");
        new_file_btn.set_tooltip_text(Some("New File"));

        let new_dir_btn = gtk4::Button::from_icon_name("folder-new-symbolic");
        new_dir_btn.add_css_class("flat");
        new_dir_btn.set_tooltip_text(Some("New Folder"));

        let collapse_btn = gtk4::Button::from_icon_name("view-list-symbolic");
        collapse_btn.add_css_class("flat");
        collapse_btn.set_tooltip_text(Some("Collapse All"));

        actions_bar.append(&collapse_btn);
        actions_bar.append(&new_file_btn);
        actions_bar.append(&new_dir_btn);

        // Build initial file list
        let file_index = Rc::new(RefCell::new(Vec::new()));
        let entries = Rc::new(RefCell::new(Vec::new()));
        let is_remote = backend.is_remote();
        if !is_remote {
            build_file_entries(root_dir, root_dir, &mut entries.borrow_mut(), &mut file_index.borrow_mut(), 0, &*backend);
        }

        let list_box = gtk4::ListBox::new();
        list_box.set_selection_mode(gtk4::SelectionMode::Single);
        list_box.add_css_class("navigation-sidebar");

        if !is_remote {
            populate_list_box(&list_box, &entries.borrow(), root_dir);
        }

        let scroll = gtk4::ScrolledWindow::new();
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
                    toggle_dir(&entries_c, &fi, &root, idx, depth, expanded, &path, &*be);
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
            let root = root_dir.to_path_buf();
            let ctx_cb = on_context_action.clone();
            let backend = backend.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(3); // right-click
            gesture.connect_pressed(move |g, _n, x, y| {
                let Some(widget) = g.widget() else { return };
                let Some(lb) = widget.downcast_ref::<gtk4::ListBox>() else { return };
                let Some(row) = lb.row_at_y(y as i32) else { return };
                let idx = row.index() as usize;
                let ents = entries_c.borrow();
                let Some(entry) = ents.get(idx) else { return };
                if entry.is_dir { return; }

                let path = entry.path.clone();
                let rel = path.strip_prefix(&root).unwrap_or(&path).to_path_buf();

                let menu_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                menu_box.set_margin_top(4);
                menu_box.set_margin_bottom(4);

                let make_item = |icon: &str, label: &str| -> gtk4::Button {
                    let btn = gtk4::Button::new();
                    btn.add_css_class("flat");
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

                // ── Clipboard ──
                let copy_rel = make_item("edit-copy-symbolic", "Copy Relative Path");
                {
                    let rel_str = rel.to_string_lossy().to_string();
                    copy_rel.connect_clicked(move |btn| {
                        if let Some(d) = gtk4::gdk::Display::default() { d.clipboard().set_text(&rel_str); }
                        if let Some(p) = btn.ancestor(gtk4::Popover::static_type()) {
                            p.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                        }
                    });
                }
                menu_box.append(&copy_rel);

                let copy_abs = make_item("edit-copy-symbolic", "Copy Absolute Path");
                {
                    let abs_str = path.to_string_lossy().to_string();
                    copy_abs.connect_clicked(move |btn| {
                        if let Some(d) = gtk4::gdk::Display::default() { d.clipboard().set_text(&abs_str); }
                        if let Some(p) = btn.ancestor(gtk4::Popover::static_type()) {
                            p.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                        }
                    });
                }
                menu_box.append(&copy_abs);

                menu_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

                // ── File operations ──
                let rename_btn = make_item("document-edit-symbolic", "Rename");
                {
                    let p = path.clone();
                    let be = backend.clone();
                    rename_btn.connect_clicked(move |btn| {
                        // Close popover first
                        if let Some(pop) = btn.ancestor(gtk4::Popover::static_type()) {
                            pop.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                        }
                        // Show a small rename dialog
                        let dialog = gtk4::Window::builder()
                            .title("Rename")
                            .modal(true)
                            .default_width(350)
                            .default_height(80)
                            .build();
                        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
                        vbox.set_margin_top(12);
                        vbox.set_margin_bottom(12);
                        vbox.set_margin_start(12);
                        vbox.set_margin_end(12);
                        let entry = gtk4::Entry::new();
                        let current_name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                        entry.set_text(&current_name);
                        vbox.append(&entry);
                        let ok_btn = gtk4::Button::with_label("Rename");
                        ok_btn.add_css_class("suggested-action");
                        let pp = p.clone();
                        let be2 = be.clone();
                        let d = dialog.clone();
                        ok_btn.connect_clicked(move |_| {
                            let new_name = entry.text().to_string();
                            if !new_name.is_empty() && new_name != current_name {
                                let dest = pp.with_file_name(&new_name);
                                let _ = be2.rename_file(&pp, &dest);
                            }
                            d.close();
                        });
                        vbox.append(&ok_btn);
                        dialog.set_child(Some(&vbox));
                        dialog.present();
                    });
                }
                menu_box.append(&rename_btn);

                let dup_btn = make_item("document-save-as-symbolic", "Duplicate File");
                {
                    let p = path.clone();
                    let be = backend.clone();
                    dup_btn.connect_clicked(move |btn| {
                        if let Some(ext) = p.extension() {
                            let stem = p.file_stem().unwrap_or_default().to_string_lossy();
                            let new_name = format!("{}_copy.{}", stem, ext.to_string_lossy());
                            let dest = p.with_file_name(new_name);
                            let _ = be.copy_file(&p, &dest);
                        } else {
                            let name = p.file_name().unwrap_or_default().to_string_lossy();
                            let dest = p.with_file_name(format!("{}_copy", name));
                            let _ = be.copy_file(&p, &dest);
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
                    del_btn.connect_clicked(move |btn| {
                        let _ = be.delete_file(&p);
                        if let Some(pop) = btn.ancestor(gtk4::Popover::static_type()) {
                            pop.downcast_ref::<gtk4::Popover>().unwrap().popdown();
                        }
                    });
                }
                menu_box.append(&del_btn);

                menu_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

                // ── Git ──
                if let Some(ref ctx) = ctx_cb {
                    let hist_btn = make_item("document-open-recent-symbolic", "Git History");
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

                let popover = gtk4::Popover::new();
                popover.set_child(Some(&menu_box));
                popover.set_parent(&row);
                popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, 0, 1, 1)));
                popover.popup();
            });
            list_box.add_controller(gesture);
        }

        // Collapse all button
        {
            let entries_c = entries.clone();
            let fi = file_index.clone();
            let root = root_dir.to_path_buf();
            let lb = list_box.clone();
            let be_ref = backend.clone();
            collapse_btn.connect_clicked(move |_| {
                // Rebuild with only depth 0 entries (all collapsed)
                let mut new_entries = Vec::new();
                let mut new_index = Vec::new();
                build_file_entries(&root, &root, &mut new_entries, &mut new_index, 0, &*be_ref);
                // All dirs at depth 0 are collapsed (auto_expand only depth < 1,
                // but we want everything collapsed, so mark depth 0 dirs as collapsed too)
                for e in &mut new_entries {
                    if e.is_dir {
                        e.expanded = false;
                    }
                }
                // Remove any children that were auto-expanded
                new_entries.retain(|e| e.depth == 0);
                // Rebuild file index from only visible files
                new_index.clear();
                for e in &new_entries {
                    if !e.is_dir {
                        new_index.push(e.path.clone());
                    }
                }
                *fi.borrow_mut() = new_index;
                *entries_c.borrow_mut() = new_entries;
                populate_list_box(&lb, &entries_c.borrow(), &root);
            });
        }

        // New file button
        {
            let root = root_dir.to_path_buf();
            let be = backend.clone();
            new_file_btn.connect_clicked(move |_| {
                let _ = be.write_file(&root.join("untitled"), "");
            });
        }

        // New folder button
        {
            let root = root_dir.to_path_buf();
            let be = backend.clone();
            new_dir_btn.connect_clicked(move |_| {
                let _ = be.create_dir(&root.join("new_folder"));
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
        };

        // Remote backends: show placeholder, retry periodically until connected
        if is_remote {
            let placeholder = gtk4::Label::new(Some("Connecting to remote host..."));
            placeholder.add_css_class("dim-label");
            placeholder.set_margin_top(16);
            tree.list_box.append(&placeholder);

            let entries_ref = tree.entries.clone();
            let index_ref = tree.file_index.clone();
            let lb_ref = tree.list_box.clone();
            let root = tree.root_dir.clone();
            let be = tree.backend.clone();
            gtk4::glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
                // Stop retrying once we have entries
                if !entries_ref.borrow().is_empty() {
                    return gtk4::glib::ControlFlow::Break;
                }
                // Try to load — ssh_exec returns Err instantly if not connected
                let mut ents = Vec::new();
                let mut idx = Vec::new();
                build_file_entries(&root, &root, &mut ents, &mut idx, 0, &*be);
                if !ents.is_empty() {
                    // Connected! Populate the tree
                    *entries_ref.borrow_mut() = ents;
                    *index_ref.borrow_mut() = idx;
                    populate_list_box(&lb_ref, &entries_ref.borrow(), &root);
                    return gtk4::glib::ControlFlow::Break;
                }
                // Still not connected — update placeholder
                while let Some(child) = lb_ref.first_child() { lb_ref.remove(&child); }
                let msg = if be.is_remote() {
                    "SSH not connected — retrying..."
                } else {
                    "Loading..."
                };
                let lbl = gtk4::Label::new(Some(msg));
                lbl.add_css_class("dim-label");
                lbl.set_margin_top(16);
                lb_ref.append(&lbl);
                gtk4::glib::ControlFlow::Continue
            });
        }

        tree
    }

    /// Rebuild the tree. Call when file system changes are detected.
    pub fn refresh(&self) {
        // Collect expanded dirs to preserve state
        let expanded_dirs: Vec<PathBuf> = self.entries.borrow().iter()
            .filter(|e| e.is_dir && e.expanded)
            .map(|e| e.path.clone())
            .collect();

        let mut entries = Vec::new();
        let mut index = Vec::new();
        build_file_entries(&self.root_dir, &self.root_dir, &mut entries, &mut index, 0, &*self.backend);

        // Restore expanded state
        restore_expanded(&mut entries, &mut index, &self.root_dir, &expanded_dirs, &*self.backend);

        *self.file_index.borrow_mut() = index;
        *self.entries.borrow_mut() = entries;

        let vadj = self.scroll.vadjustment();
        let scroll_pos = vadj.value();
        populate_list_box(&self.list_box, &self.entries.borrow(), &self.root_dir);
        vadj.set_value(scroll_pos);
    }

    /// Expand all parent directories of the given file and scroll to it.
    pub fn reveal_file(&self, file_path: &Path) {
        // Build list of ancestor directories that need expanding
        let mut ancestors: Vec<PathBuf> = Vec::new();
        let mut parent = file_path.parent();
        while let Some(p) = parent {
            if p == self.root_dir { break; }
            ancestors.push(p.to_path_buf());
            parent = p.parent();
        }
        ancestors.reverse(); // root-first order

        // Expand each ancestor if not already expanded
        let mut changed = false;
        for ancestor in &ancestors {
            let needs_expand = {
                let ents = self.entries.borrow();
                ents.iter().any(|e| e.path == *ancestor && e.is_dir && !e.expanded)
            };
            if needs_expand {
                let idx_and_depth = {
                    let ents = self.entries.borrow();
                    ents.iter().enumerate()
                        .find(|(_, e)| e.path == *ancestor && e.is_dir)
                        .map(|(i, e)| (i, e.depth))
                };
                if let Some((idx, depth)) = idx_and_depth {
                    toggle_dir(&self.entries, &self.file_index, &self.root_dir,
                        idx, depth, false, ancestor, &*self.backend);
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
                if !has_sibling { continue; }
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
                cr.line_to(x + 0.5, mid_y);   // └
            } else {
                cr.line_to(x + 0.5, h);       // ├
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
        "rs" | "py" | "js" | "ts" | "c" | "cpp" | "h" | "go" | "java"
        | "rb" | "sh" | "bash" | "zsh" | "lua" | "zig" => "text-x-script-symbolic",
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
        if e.depth == depth { return true; }
        if e.depth < depth { return false; }
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

/// Toggle a directory open/closed and rebuild entries list accordingly.
fn toggle_dir(
    entries: &Rc<RefCell<Vec<FileEntry>>>,
    file_index: &Rc<RefCell<Vec<PathBuf>>>,
    root: &Path,
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
        build_file_entries(root, dir_path, &mut new_entries, &mut new_index, depth + 1, backend);
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
    root: &Path,
    expanded_dirs: &[PathBuf],
    backend: &dyn FileBackend,
) {
    let mut i = 0;
    while i < entries.len() {
        if entries[i].is_dir && !entries[i].expanded && expanded_dirs.contains(&entries[i].path) {
            let depth = entries[i].depth;
            let dir_path = entries[i].path.clone();
            entries[i].expanded = true;
            let mut new_entries = Vec::new();
            let mut new_index = Vec::new();
            build_file_entries(root, &dir_path, &mut new_entries, &mut new_index, depth + 1, backend);
            file_index.extend(new_index);
            let insert_pos = i + 1;
            for (j, entry) in new_entries.into_iter().enumerate() {
                entries.insert(insert_pos + j, entry);
            }
        }
        i += 1;
    }
}

/// Recursively build file entries using the `ignore` crate for .gitignore support.
fn build_file_entries(
    root: &Path,
    dir: &Path,
    entries: &mut Vec<FileEntry>,
    file_index: &mut Vec<PathBuf>,
    depth: u32,
    backend: &dyn FileBackend,
) {
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    if backend.is_remote() {
        // Remote: use backend.list_dir() — one SSH call per directory
        if let Ok(listing) = backend.list_dir(dir) {
            for de in listing {
                let path = dir.join(&de.name);
                if de.is_dir {
                    dirs.push((path, de.name));
                } else {
                    files.push((path, de.name));
                }
            }
        }
    } else {
        // Local: use ignore::WalkBuilder for .gitignore support
        let walker = ignore::WalkBuilder::new(dir)
            .max_depth(Some(1))
            .sort_by_file_name(|a, b| a.cmp(b))
            .build();

        for entry in walker.flatten() {
            let path = entry.path().to_path_buf();
            if path == dir { continue; }

            let name = path.file_name()
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
            build_file_entries(root, &path, entries, file_index, depth + 1, backend);
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
}
