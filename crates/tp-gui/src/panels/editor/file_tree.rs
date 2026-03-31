use gtk4::prelude::*;
use gtk4::gio;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Callback when a file is double-clicked in the tree.
pub type OnFileOpen = Rc<dyn Fn(&Path)>;

/// File tree widget with gitignore-aware traversal and lazy loading.
pub struct FileTree {
    pub widget: gtk4::Box,
    list_view: gtk4::ListView,
    root_dir: PathBuf,
    #[allow(dead_code)]
    on_file_open: Option<OnFileOpen>,
    /// Flat list of all file paths for fuzzy finder indexing.
    pub file_index: Rc<RefCell<Vec<PathBuf>>>,
    entries: Rc<RefCell<Vec<FileEntry>>>,
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
    pub fn new(root_dir: &Path, on_file_open: OnFileOpen) -> Self {
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

        actions_bar.append(&new_file_btn);
        actions_bar.append(&new_dir_btn);

        // Build initial file list
        let file_index = Rc::new(RefCell::new(Vec::new()));
        let entries = Rc::new(RefCell::new(Vec::new()));
        build_file_entries(root_dir, root_dir, &mut entries.borrow_mut(), &mut file_index.borrow_mut(), 0);

        let model = build_string_model(&entries.borrow());

        let selection = gtk4::SingleSelection::new(Some(model.clone()));
        let factory = gtk4::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk4::ListItem>().unwrap();
            let label = gtk4::Label::new(None);
            label.set_halign(gtk4::Align::Start);
            label.set_margin_start(4);
            label.set_xalign(0.0);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            item.set_child(Some(&label));
        });
        let root_for_tooltip = root_dir.to_path_buf();
        let entries_for_tooltip = entries.clone();
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk4::ListItem>().unwrap();
            let label = item.child().and_downcast::<gtk4::Label>().unwrap();
            let str_obj = item.item().and_downcast::<gtk4::StringObject>().unwrap();
            label.set_text(&str_obj.string());
            // Set tooltip to relative path
            let pos = item.position() as usize;
            let entries = entries_for_tooltip.borrow();
            if let Some(entry) = entries.get(pos) {
                let rel = entry.path.strip_prefix(&root_for_tooltip).unwrap_or(&entry.path);
                label.set_tooltip_text(Some(&rel.to_string_lossy()));
            }
        });

        let list_view = gtk4::ListView::new(Some(selection.clone()), Some(factory));
        list_view.add_css_class("navigation-sidebar");

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_child(Some(&list_view));
        scroll.set_vexpand(true);

        container.append(&scroll);
        container.append(&actions_bar);

        // Click to expand/collapse dirs, double-click to open files
        {
            let entries_c = entries.clone();
            let on_open = on_file_open.clone();
            let fi = file_index.clone();
            let root = root_dir.to_path_buf();
            list_view.connect_activate(move |lv, pos| {
                let idx = pos as usize;
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
                    toggle_dir(&entries_c, &fi, &root, idx, depth, expanded, &path);
                    rebuild_model(lv, &entries_c.borrow());
                } else {
                    on_open(&path);
                }
            });
        }

        // Right-click context menu
        let menu = gio::Menu::new();
        menu.append(Some("New File"), Some("editor.new-file"));
        menu.append(Some("New Folder"), Some("editor.new-folder"));
        menu.append(Some("Rename"), Some("editor.rename"));
        menu.append(Some("Delete"), Some("editor.delete"));
        menu.append(Some("Copy Path"), Some("editor.copy-path"));
        let popover = gtk4::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&container);

        // New file button
        {
            let root = root_dir.to_path_buf();
            new_file_btn.connect_clicked(move |_| {
                let _ = std::fs::write(root.join("untitled"), "");
            });
        }

        // New folder button
        {
            let root = root_dir.to_path_buf();
            new_dir_btn.connect_clicked(move |_| {
                let _ = std::fs::create_dir(root.join("new_folder"));
            });
        }

        Self {
            widget: container,
            list_view,
            root_dir: root_dir.to_path_buf(),
            on_file_open: Some(on_file_open),
            file_index,
            entries,
        }
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
        build_file_entries(&self.root_dir, &self.root_dir, &mut entries, &mut index, 0);

        // Restore expanded state
        restore_expanded(&mut entries, &mut index, &self.root_dir, &expanded_dirs);

        *self.file_index.borrow_mut() = index;
        *self.entries.borrow_mut() = entries;

        rebuild_model(&self.list_view, &self.entries.borrow());
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
) {
    let mut ents = entries.borrow_mut();

    if was_expanded {
        // Collapse: remove all children (entries with depth > this one's depth,
        // contiguous after this entry)
        ents[idx].expanded = false;
        let remove_start = idx + 1;
        let mut remove_end = remove_start;
        while remove_end < ents.len() && ents[remove_end].depth > depth {
            remove_end += 1;
        }
        // Remove collapsed file paths from file_index
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
        build_file_entries(root, dir_path, &mut new_entries, &mut new_index, depth + 1);
        file_index.borrow_mut().extend(new_index);
        // Insert after current entry
        let insert_pos = idx + 1;
        for (i, entry) in new_entries.into_iter().enumerate() {
            ents.insert(insert_pos + i, entry);
        }
    }
}

/// Build a StringList model from entries.
fn build_string_model(entries: &[FileEntry]) -> gtk4::StringList {
    let model = gtk4::StringList::new(&[]);
    for entry in entries {
        model.append(&format_entry(entry));
    }
    model
}

/// Format a single entry for display.
fn format_entry(entry: &FileEntry) -> String {
    let prefix = "  ".repeat(entry.depth as usize);
    if entry.is_dir {
        let arrow = if entry.expanded { "▼" } else { "▶" };
        format!("{}{} \u{1F4C2} {}", prefix, arrow, entry.name)
    } else {
        format!("{}   {}", prefix, entry.name)
    }
}

/// Rebuild the ListView model from entries.
fn rebuild_model(list_view: &gtk4::ListView, entries: &[FileEntry]) {
    if let Some(sel) = list_view.model().and_then(|m| m.downcast::<gtk4::SingleSelection>().ok()) {
        let model = build_string_model(entries);
        sel.set_model(Some(&model));
    }
}

/// Restore expanded directories after a refresh.
fn restore_expanded(
    entries: &mut Vec<FileEntry>,
    file_index: &mut Vec<PathBuf>,
    root: &Path,
    expanded_dirs: &[PathBuf],
) {
    let mut i = 0;
    while i < entries.len() {
        if entries[i].is_dir && !entries[i].expanded && expanded_dirs.contains(&entries[i].path) {
            let depth = entries[i].depth;
            let dir_path = entries[i].path.clone();
            entries[i].expanded = true;
            let mut new_entries = Vec::new();
            let mut new_index = Vec::new();
            build_file_entries(root, &dir_path, &mut new_entries, &mut new_index, depth + 1);
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
) {
    let walker = ignore::WalkBuilder::new(dir)
        .max_depth(Some(1))
        .sort_by_file_name(|a, b| {
            a.cmp(b)
        })
        .build();

    let mut dirs = Vec::new();
    let mut files = Vec::new();

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

    dirs.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    files.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

    for (path, name) in dirs {
        let auto_expand = depth < 1;
        entries.push(FileEntry {
            path: path.clone(),
            name,
            is_dir: true,
            depth,
            expanded: auto_expand,
        });
        if auto_expand {
            build_file_entries(root, &path, entries, file_index, depth + 1);
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
