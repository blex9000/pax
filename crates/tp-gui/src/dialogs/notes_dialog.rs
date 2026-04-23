//! Per-workspace Notes dialog accessed from the file-tree sidebar.
//! Single-click on a row jumps to the note; hovering a row reveals
//! inline Edit / Delete buttons. The dialog's footer only carries a
//! Close button — no global action buttons.

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
    crate::theme::configure_dialog_window(&dialog);
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
    // Single click on a row activates it — we wire row_activated to jump.
    list_box.set_activate_on_single_click(true);
    list_box.add_css_class("boxed-list");

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&list_box));
    scroll.set_vexpand(true);
    vbox.append(&scroll);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    let close_btn = gtk4::Button::with_label("Close");
    btn_row.append(&close_btn);
    vbox.append(&btn_row);

    let all_notes: Rc<RefCell<Vec<FileNote>>> = Rc::new(RefCell::new(Vec::new()));

    // `reload` and the per-row Edit/Delete actions have a circular
    // dependency: the actions need to refresh the list after mutating,
    // and the list rows embed those actions as button callbacks. Break
    // the cycle with a cell the actions read through; fill it once the
    // reload closure exists.
    let reload_cell: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let on_edit: Rc<dyn Fn(FileNote)> = {
        let dialog_c = dialog.clone();
        let on_notes_changed = on_notes_changed.clone();
        let reload_cell = reload_cell.clone();
        Rc::new(move |note: FileNote| {
            let reload_opt = reload_cell.borrow().clone();
            let on_notes_changed = on_notes_changed.clone();
            let parent_window = dialog_c.clone().upcast::<gtk4::Window>();
            crate::panels::editor::editor_tabs::show_note_editor(
                Some(&parent_window),
                "Edit note",
                &note.text,
                move |new_text| {
                    let db_path = Database::default_path();
                    if let Ok(db) = Database::open(&db_path) {
                        let _ = db.update_note_text(note.id, &new_text);
                    }
                    if let Some(r) = reload_opt.clone() {
                        r();
                    }
                    on_notes_changed();
                },
            );
        })
    };

    let on_delete: Rc<dyn Fn(FileNote)> = {
        let on_notes_changed = on_notes_changed.clone();
        let reload_cell = reload_cell.clone();
        Rc::new(move |note: FileNote| {
            let db_path = Database::default_path();
            if let Ok(db) = Database::open(&db_path) {
                let _ = db.delete_metadata_entry(note.id);
            }
            if let Some(r) = reload_cell.borrow().clone() {
                r();
            }
            on_notes_changed();
        })
    };

    let reload: Rc<dyn Fn()> = {
        let list_box = list_box.clone();
        let all_notes = all_notes.clone();
        let record_key = record_key.clone();
        let search = search.clone();
        let on_edit = on_edit.clone();
        let on_delete = on_delete.clone();
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
                let row = build_row(note, idx, on_edit.clone(), on_delete.clone());
                list_box.append(&row);
            }
        })
    };
    *reload_cell.borrow_mut() = Some(reload.clone());
    reload();

    {
        let reload = reload.clone();
        search.connect_search_changed(move |_| reload());
    }

    // Single-click on a row (or Enter) → jump.
    {
        let all_notes = all_notes.clone();
        let root = workspace_root.clone();
        let d = dialog.clone();
        list_box.connect_row_activated(move |_, row| {
            let idx: usize = row.widget_name().parse().unwrap_or(usize::MAX);
            let Some(note) = all_notes.borrow().get(idx).cloned() else {
                return;
            };
            let full = root.join(&note.file_path);
            on_jump(&full, note.line_number);
            d.close();
        });
    }

    {
        let d = dialog.clone();
        close_btn.connect_clicked(move |_| d.close());
    }

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn build_row(
    note: &FileNote,
    idx: usize,
    on_edit: Rc<dyn Fn(FileNote)>,
    on_delete: Rc<dyn Fn(FileNote)>,
) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_widget_name(&idx.to_string());

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);
    hbox.set_margin_top(6);
    hbox.set_margin_bottom(6);

    // Text side: file (bold) + preview (dim).
    let texts = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    texts.set_hexpand(true);

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
    texts.append(&header);

    let preview_text = preview_of(&note.text);
    let preview = gtk4::Label::new(Some(&preview_text));
    preview.add_css_class("dim-label");
    preview.set_halign(gtk4::Align::Start);
    preview.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    texts.append(&preview);

    hbox.append(&texts);

    // Inline action buttons — revealed on row hover / selection via CSS.
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    actions.add_css_class("editor-note-row-actions");
    actions.set_valign(gtk4::Align::Center);

    let edit_btn = gtk4::Button::from_icon_name("document-edit-symbolic");
    edit_btn.add_css_class("flat");
    edit_btn.set_tooltip_text(Some("Edit note"));
    {
        let note_c = note.clone();
        let on_edit = on_edit.clone();
        edit_btn.connect_clicked(move |_| on_edit(note_c.clone()));
    }
    actions.append(&edit_btn);

    let delete_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    delete_btn.add_css_class("flat");
    delete_btn.set_tooltip_text(Some("Delete note"));
    {
        let note_c = note.clone();
        let on_delete = on_delete.clone();
        delete_btn.connect_clicked(move |_| on_delete(note_c.clone()));
    }
    actions.append(&delete_btn);

    hbox.append(&actions);

    row.set_child(Some(&hbox));
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
