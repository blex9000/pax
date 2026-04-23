//! Per-workspace Notes dialog accessed from the file-tree sidebar.
//! Lists every note in the current workspace with Jump / Edit / Delete.

use gtk4::prelude::*;
use pax_db::notes::FileNote;
use pax_db::Database;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const DIALOG_WIDTH_PX: i32 = 600;
const DIALOG_HEIGHT_PX: i32 = 420;
const PREVIEW_MAX_CHARS: usize = 72;

pub type OnJump = Rc<dyn Fn(&std::path::Path, i32)>;
pub type OnNotesChanged = Rc<dyn Fn()>;

pub fn show_workspace_notes_dialog(
    parent: Option<&gtk4::Window>,
    workspace_label: &str,
    record_key: String,
    workspace_root: PathBuf,
    on_jump: OnJump,
    on_notes_changed: OnNotesChanged,
) {
    let dialog = gtk4::Window::builder()
        .title(&format!("Notes — {}", workspace_label))
        .modal(true)
        .default_width(DIALOG_WIDTH_PX)
        .default_height(DIALOG_HEIGHT_PX)
        .build();
    if let Some(win) = parent {
        dialog.set_transient_for(Some(win));
    }

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(8);
    vbox.set_margin_bottom(8);
    vbox.set_margin_start(8);
    vbox.set_margin_end(8);

    let search = gtk4::SearchEntry::new();
    search.set_placeholder_text(Some("Search notes…"));
    vbox.append(&search);

    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);
    list_box.add_css_class("boxed-list");

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&list_box));
    scroll.set_vexpand(true);
    vbox.append(&scroll);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    let jump_btn = gtk4::Button::with_label("Jump");
    let edit_btn = gtk4::Button::with_label("Edit");
    let delete_btn = gtk4::Button::with_label("Delete");
    delete_btn.add_css_class("destructive-action");
    let close_btn = gtk4::Button::with_label("Close");
    btn_row.append(&jump_btn);
    btn_row.append(&edit_btn);
    btn_row.append(&delete_btn);
    btn_row.append(&close_btn);
    vbox.append(&btn_row);

    let all_notes: Rc<RefCell<Vec<FileNote>>> = Rc::new(RefCell::new(Vec::new()));

    let reload = {
        let list_box = list_box.clone();
        let all_notes = all_notes.clone();
        let record_key = record_key.clone();
        let search = search.clone();
        Rc::new(move || {
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }
            let db_path = Database::default_path();
            let Ok(db) = Database::open(&db_path) else {
                return;
            };
            let notes = db
                .list_notes_for_workspace(&record_key)
                .unwrap_or_default();
            *all_notes.borrow_mut() = notes.clone();

            let query = search.text().to_string().to_lowercase();
            for (idx, note) in notes.iter().enumerate() {
                if !query.is_empty()
                    && !note.text.to_lowercase().contains(&query)
                    && !note.file_path.to_lowercase().contains(&query)
                {
                    continue;
                }
                let row = build_row(note, idx);
                list_box.append(&row);
            }
        })
    };
    reload();

    {
        let reload = reload.clone();
        search.connect_search_changed(move |_| reload());
    }

    // Jump helper used by both the Jump button and double-click.
    let do_jump = {
        let all_notes = all_notes.clone();
        let root = workspace_root.clone();
        let d = dialog.clone();
        let on_jump = on_jump.clone();
        Rc::new(move |idx: usize| {
            let Some(note) = all_notes.borrow().get(idx).cloned() else {
                return;
            };
            let full = root.join(&note.file_path);
            on_jump(&full, note.line_number);
            d.close();
        })
    };

    // Jump button
    {
        let list_box_c = list_box.clone();
        let do_jump = do_jump.clone();
        jump_btn.connect_clicked(move |_| {
            let Some(row) = list_box_c.selected_row() else {
                return;
            };
            let idx: usize = row.widget_name().parse().unwrap_or(usize::MAX);
            do_jump(idx);
        });
    }

    // Double-click (or Enter) on a row → same as Jump.
    {
        let do_jump = do_jump.clone();
        list_box.connect_row_activated(move |_, row| {
            let idx: usize = row.widget_name().parse().unwrap_or(usize::MAX);
            do_jump(idx);
        });
    }

    // Edit
    {
        let list_box_c = list_box.clone();
        let all_notes = all_notes.clone();
        let reload = reload.clone();
        let on_notes_changed = on_notes_changed.clone();
        let d = dialog.clone();
        edit_btn.connect_clicked(move |_| {
            let Some(row) = list_box_c.selected_row() else {
                return;
            };
            let idx: usize = row.widget_name().parse().unwrap_or(usize::MAX);
            let Some(note) = all_notes.borrow().get(idx).cloned() else {
                return;
            };
            let reload = reload.clone();
            let on_notes_changed = on_notes_changed.clone();
            let parent_window = d.clone().upcast::<gtk4::Window>();
            crate::panels::editor::editor_tabs::show_note_editor(
                Some(&parent_window),
                "Edit note",
                &note.text,
                move |new_text| {
                    let db_path = Database::default_path();
                    if let Ok(db) = Database::open(&db_path) {
                        let _ = db.update_note_text(note.id, &new_text);
                    }
                    reload();
                    on_notes_changed();
                },
            );
        });
    }

    // Delete
    {
        let list_box_c = list_box.clone();
        let all_notes = all_notes.clone();
        let reload = reload.clone();
        let on_notes_changed = on_notes_changed.clone();
        delete_btn.connect_clicked(move |_| {
            let Some(row) = list_box_c.selected_row() else {
                return;
            };
            let idx: usize = row.widget_name().parse().unwrap_or(usize::MAX);
            let Some(note) = all_notes.borrow().get(idx).cloned() else {
                return;
            };
            let db_path = Database::default_path();
            if let Ok(db) = Database::open(&db_path) {
                let _ = db.delete_metadata_entry(note.id);
            }
            reload();
            on_notes_changed();
        });
    }

    {
        let d = dialog.clone();
        close_btn.connect_clicked(move |_| d.close());
    }

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn build_row(note: &FileNote, idx: usize) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_widget_name(&idx.to_string());

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_margin_start(8);
    vbox.set_margin_end(8);
    vbox.set_margin_top(6);
    vbox.set_margin_bottom(6);

    // Header row: file name bold on the left, line-number badge on the right.
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let file_label = gtk4::Label::new(None);
    file_label.set_markup(&format!(
        "<b>{}</b>",
        gtk4::glib::markup_escape_text(&note.file_path)
    ));
    file_label.set_halign(gtk4::Align::Start);
    file_label.set_hexpand(true);
    file_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    header.append(&file_label);
    if note.line_anchor.is_none() {
        let orphan = gtk4::Label::new(Some("orphan"));
        orphan.add_css_class("caption");
        orphan.add_css_class("dim-label");
        header.append(&orphan);
    }
    let line_badge = gtk4::Label::new(Some(&format!("L{}", note.line_number + 1)));
    line_badge.add_css_class("editor-note-line-badge");
    header.append(&line_badge);
    vbox.append(&header);

    // Preview row: note text in dimmed grey.
    let preview_text = preview_of(&note.text);
    let preview = gtk4::Label::new(Some(&preview_text));
    preview.add_css_class("dim-label");
    preview.set_halign(gtk4::Align::Start);
    preview.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    vbox.append(&preview);

    row.set_child(Some(&vbox));
    row
}

fn preview_of(text: &str) -> String {
    let first = text.lines().next().unwrap_or("");
    if first.chars().count() > PREVIEW_MAX_CHARS {
        let truncated: String = first.chars().take(PREVIEW_MAX_CHARS).collect();
        format!("{}…", truncated)
    } else {
        first.to_string()
    }
}
