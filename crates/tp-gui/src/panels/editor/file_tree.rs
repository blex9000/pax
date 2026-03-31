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
    on_file_open: Option<OnFileOpen>,
    /// Flat list of all file paths for fuzzy finder indexing.
    pub file_index: Rc<RefCell<Vec<PathBuf>>>,
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

        // ListView with a GtkStringList model for simplicity
        let model = gtk4::StringList::new(&[]);
        for entry in entries.borrow().iter() {
            let prefix = "  ".repeat(entry.depth as usize);
            let icon = if entry.is_dir { "\u{1F4C2} " } else { "  " };
            model.append(&format!("{}{}{}", prefix, icon, entry.name));
        }

        let selection = gtk4::SingleSelection::new(Some(model.clone()));
        let factory = gtk4::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk4::ListItem>().unwrap();
            let label = gtk4::Label::new(None);
            label.set_halign(gtk4::Align::Start);
            label.set_margin_start(4);
            label.set_xalign(0.0);
            item.set_child(Some(&label));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk4::ListItem>().unwrap();
            let label = item.child().and_downcast::<gtk4::Label>().unwrap();
            let str_obj = item.item().and_downcast::<gtk4::StringObject>().unwrap();
            label.set_text(&str_obj.string());
        });

        let list_view = gtk4::ListView::new(Some(selection.clone()), Some(factory));
        list_view.add_css_class("navigation-sidebar");

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_child(Some(&list_view));
        scroll.set_vexpand(true);

        container.append(&scroll);
        container.append(&actions_bar);

        // Double-click to open file
        {
            let entries_c = entries.clone();
            let on_open = on_file_open.clone();
            list_view.connect_activate(move |_, pos| {
                let entries = entries_c.borrow();
                if let Some(entry) = entries.get(pos as usize) {
                    if !entry.is_dir {
                        on_open(&entry.path);
                    }
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
                // Create empty file in root (basic implementation)
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
        }
    }

    /// Rebuild the tree. Call when file system changes are detected.
    pub fn refresh(&self) {
        // Re-scan and rebuild the model
        let mut entries = Vec::new();
        let mut index = Vec::new();
        build_file_entries(&self.root_dir, &self.root_dir, &mut entries, &mut index, 0);
        *self.file_index.borrow_mut() = index;

        // Rebuild model
        if let Some(sel) = self.list_view.model().and_then(|m| m.downcast::<gtk4::SingleSelection>().ok()) {
            let model = gtk4::StringList::new(&[]);
            for entry in &entries {
                let prefix = "  ".repeat(entry.depth as usize);
                let icon = if entry.is_dir { "\u{1F4C2} " } else { "  " };
                model.append(&format!("{}{}{}", prefix, icon, entry.name));
            }
            sel.set_model(Some(&model));
        }
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
            // Directories first, then alphabetical
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
        entries.push(FileEntry {
            path: path.clone(),
            name,
            is_dir: true,
            depth,
            expanded: depth < 1, // auto-expand first level
        });
        if depth < 1 {
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
