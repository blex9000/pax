//! List view for Note panels.
//!
//! Owns the panel body: a header (search + tag filter + "New note") over a
//! ListBox of NoteCard rows. Reload-on-change keeps things simple: any
//! mutation (save, delete, create, cycle severity) re-runs the query and
//! rebuilds the list. Notes scale to tens per panel, so the cost is
//! negligible and state management stays trivial.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;

use pax_db::workspace_notes::{
    NOTE_SEVERITIES, SEVERITY_IMPORTANT, SEVERITY_INFO, SEVERITY_WARNING, WorkspaceNote,
};

use super::card::{build_note_card, NoteCardActions};
use super::editor_dialog::{
    draft_default, draft_from_note, open_note_dialog, NoteDraft,
};

/// Delay before the undo-toast auto-dismisses.
const UNDO_TOAST_TIMEOUT_SECS: u32 = 5;
/// Sentinel string in the tag dropdown meaning "no filter".
const ALL_TAGS_LABEL: &str = "All tags";

#[derive(Default)]
struct ListState {
    query: String,
    tag_filter: Option<String>,
    /// Last deleted note kept in memory so the undo-toast can reinsert it.
    /// Stored without id; reinsert creates a fresh row.
    last_deleted: Option<WorkspaceNote>,
    /// Source id of the pending toast auto-dismiss timer, if any. We kill
    /// any previous timer on a new deletion so a stale timeout can't wipe
    /// the freshly set `last_deleted` snapshot.
    pending_toast_timer: Option<gtk4::glib::SourceId>,
}

pub struct NoteListView {
    root: gtk4::Box,
    flow: gtk4::FlowBox,
    search_entry: gtk4::SearchEntry,
    tag_dropdown: gtk4::DropDown,
    toast_revealer: gtk4::Revealer,
    toast_label: gtk4::Label,
    record_key: Rc<String>,
    panel_id: Rc<String>,
    state: Rc<RefCell<ListState>>,
    /// Suppresses `selected-notify` re-entry: `refresh_tag_dropdown` mutates
    /// the model + selection during a reload, which emits `notify::selected`
    /// synchronously. Without this flag the handler would call `reload`
    /// again and recurse until the stack blows.
    updating: Rc<Cell<bool>>,
}

impl NoteListView {
    pub fn new(record_key: Rc<String>, panel_id: Rc<String>) -> Rc<Self> {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);

        // ── Header ──────────────────────────────────────────────────
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        header.add_css_class("notes-header");
        header.set_margin_top(6);
        header.set_margin_bottom(6);
        header.set_margin_start(8);
        header.set_margin_end(8);

        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Search notes…"));
        search_entry.set_hexpand(true);
        header.append(&search_entry);

        let tag_dropdown = gtk4::DropDown::from_strings(&[ALL_TAGS_LABEL]);
        tag_dropdown.set_tooltip_text(Some("Filter by tag"));
        header.append(&tag_dropdown);

        let new_btn = gtk4::Button::from_icon_name("list-add-symbolic");
        new_btn.set_tooltip_text(Some("New note"));
        new_btn.add_css_class("suggested-action");
        header.append(&new_btn);

        root.append(&header);

        // ── Scrollable fluid grid ───────────────────────────────────
        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
        scroll.set_vscrollbar_policy(gtk4::PolicyType::Automatic);

        // FlowBox wraps cards to columns based on available width; cards
        // carry a min_width via CSS so the wrap happens at a sensible
        // breakpoint.
        let flow = gtk4::FlowBox::new();
        flow.add_css_class("notes-list");
        flow.set_selection_mode(gtk4::SelectionMode::None);
        // Without this, FlowBox swallows single clicks before they reach
        // buttons nested inside each card.
        flow.set_activate_on_single_click(false);
        flow.set_homogeneous(false);
        flow.set_column_spacing(4);
        flow.set_row_spacing(4);
        flow.set_min_children_per_line(1);
        flow.set_max_children_per_line(4);
        flow.set_margin_start(6);
        flow.set_margin_end(6);
        flow.set_margin_top(4);
        flow.set_margin_bottom(4);

        scroll.set_child(Some(&flow));
        root.append(&scroll);

        // ── Undo toast (revealer over the bottom edge) ──────────────
        let toast_revealer = gtk4::Revealer::new();
        toast_revealer.set_transition_type(gtk4::RevealerTransitionType::SlideUp);
        toast_revealer.set_transition_duration(150);
        toast_revealer.set_reveal_child(false);

        let toast_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        toast_box.add_css_class("note-toast");
        toast_box.set_margin_start(8);
        toast_box.set_margin_end(8);
        toast_box.set_margin_top(4);
        toast_box.set_margin_bottom(8);

        let toast_label = gtk4::Label::new(Some(""));
        toast_label.set_hexpand(true);
        toast_label.set_halign(gtk4::Align::Start);
        toast_box.append(&toast_label);

        let undo_btn = gtk4::Button::with_label("Undo");
        undo_btn.add_css_class("flat");
        toast_box.append(&undo_btn);

        toast_revealer.set_child(Some(&toast_box));
        root.append(&toast_revealer);

        let view = Rc::new(Self {
            root,
            flow,
            search_entry,
            tag_dropdown,
            toast_revealer,
            toast_label,
            record_key,
            panel_id,
            state: Rc::new(RefCell::new(ListState::default())),
            updating: Rc::new(Cell::new(false)),
        });

        // Wire handlers.
        {
            let v = view.clone();
            view.search_entry.connect_search_changed(move |entry| {
                v.state.borrow_mut().query = entry.text().to_string();
                v.reload();
            });
        }
        {
            let v = view.clone();
            view.tag_dropdown.connect_selected_notify(move |dd| {
                if v.updating.get() {
                    return;
                }
                let selected = dd
                    .selected_item()
                    .and_then(|o| o.downcast::<gtk4::StringObject>().ok())
                    .map(|s| s.string().to_string());
                v.state.borrow_mut().tag_filter = selected
                    .filter(|s| s != ALL_TAGS_LABEL)
                    .filter(|s| !s.is_empty());
                v.reload();
            });
        }
        {
            let v = view.clone();
            new_btn.connect_clicked(move |_| v.on_new_note());
        }
        {
            let v = view.clone();
            undo_btn.connect_clicked(move |_| v.undo_delete());
        }

        view.reload();
        view
    }

    pub fn widget(&self) -> &gtk4::Widget {
        self.root.upcast_ref()
    }

    /// Re-query the database and rebuild the list.
    pub fn reload(self: &Rc<Self>) {
        let Some(db) = open_db() else {
            return;
        };

        // Suppress `notify::selected` re-entry while we rewrite the
        // dropdown model below; see the `updating` field's doc.
        self.updating.set(true);

        // Refresh the tag dropdown options (keeping the current selection if
        // it still exists, otherwise fall back to "All tags").
        let tags = db
            .list_tags_for_panel(&self.record_key, &self.panel_id)
            .unwrap_or_default();
        self.refresh_tag_dropdown(&tags);

        self.updating.set(false);

        // Query notes, then apply the tag filter client-side (FTS doesn't
        // know about tag equality — only substring matches).
        let query = self.state.borrow().query.clone();
        let tag_filter = self.state.borrow().tag_filter.clone();
        let notes = db
            .search_notes_for_panel(&self.record_key, &self.panel_id, &query)
            .unwrap_or_default();
        let notes: Vec<WorkspaceNote> = match tag_filter {
            Some(tag) => notes
                .into_iter()
                .filter(|n| n.tags.iter().any(|t| t == &tag))
                .collect(),
            None => notes,
        };

        // Rebuild grid.
        while let Some(child) = self.flow.first_child() {
            self.flow.remove(&child);
        }

        if notes.is_empty() {
            let placeholder = gtk4::Label::new(Some(
                "No notes yet — click + to create one.",
            ));
            placeholder.add_css_class("dim-label");
            placeholder.set_margin_top(24);
            placeholder.set_margin_bottom(24);
            self.flow.insert(&placeholder, -1);
            return;
        }

        for note in notes {
            let id = note.id;
            let severity = note.severity.clone();
            let card = build_note_card(
                &note,
                NoteCardActions {
                    on_delete: {
                        let this = self.clone();
                        let snapshot = note.clone();
                        Box::new(move || this.on_delete(id, &snapshot))
                    },
                    on_cycle_severity: {
                        let this = self.clone();
                        let sev = severity.clone();
                        Box::new(move || this.on_cycle_severity(id, &sev))
                    },
                    on_open_editor: {
                        let this = self.clone();
                        Box::new(move || this.on_open_editor(id))
                    },
                },
            );
            // FlowBox auto-wraps the card in a FlowBoxChild. Don't pre-wrap
            // manually: the auto-wrap is what makes clicks reach our
            // nested buttons, whereas a self-made FlowBoxChild tended to
            // intercept them.
            self.flow.insert(&card, -1);
        }
    }

    fn refresh_tag_dropdown(&self, tags: &[String]) {
        let previous = self.state.borrow().tag_filter.clone();
        let mut options = vec![ALL_TAGS_LABEL.to_string()];
        options.extend(tags.iter().cloned());
        let refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
        let model = gtk4::StringList::new(&refs);
        self.tag_dropdown.set_model(Some(&model));
        // Restore previous selection if still present.
        if let Some(prev) = previous {
            if let Some(idx) = options.iter().position(|o| o == &prev) {
                self.tag_dropdown.set_selected(idx as u32);
            } else {
                // Previous tag no longer exists — clear filter silently.
                self.tag_dropdown.set_selected(0);
                self.state.borrow_mut().tag_filter = None;
            }
        } else {
            self.tag_dropdown.set_selected(0);
        }
    }

    fn on_new_note(self: &Rc<Self>) {
        let Some(parent) = self.parent_window() else {
            return;
        };
        let this = self.clone();
        open_note_dialog(
            &parent,
            "New note",
            draft_default(),
            Rc::new(move |draft| this.persist_new(draft)),
        );
    }

    fn persist_new(self: &Rc<Self>, draft: NoteDraft) {
        let Some(db) = open_db() else {
            return;
        };
        let result = db.add_workspace_note(
            &self.record_key,
            &self.panel_id,
            &draft.title,
            &draft.text,
            &draft.tags,
            &draft.severity,
            draft.alert_at,
        );
        if let Err(e) = result {
            tracing::warn!("notes: could not create note: {e}");
            return;
        }
        self.reload();
    }

    fn on_open_editor(self: &Rc<Self>, id: i64) {
        let Some(db) = open_db() else {
            return;
        };
        let Some(note) = db.get_workspace_note(id).ok().flatten() else {
            return;
        };
        let Some(parent) = self.parent_window() else {
            return;
        };
        let this = self.clone();
        open_note_dialog(
            &parent,
            "Edit note",
            draft_from_note(&note),
            Rc::new(move |draft| this.persist_update(id, draft)),
        );
    }

    fn persist_update(self: &Rc<Self>, id: i64, draft: NoteDraft) {
        let Some(db) = open_db() else {
            return;
        };
        if let Err(e) = db.update_workspace_note(
            id,
            &draft.title,
            &draft.text,
            &draft.tags,
            &draft.severity,
            draft.alert_at,
        ) {
            tracing::warn!("notes: could not update note {id}: {e}");
            return;
        }
        self.reload();
    }

    fn parent_window(&self) -> Option<gtk4::Window> {
        self.root
            .ancestor(gtk4::Window::static_type())
            .and_then(|w| w.downcast::<gtk4::Window>().ok())
    }

    fn on_delete(self: &Rc<Self>, id: i64, snapshot: &WorkspaceNote) {
        tracing::info!("note list: on_delete id={id}");
        let Some(db) = open_db() else {
            return;
        };
        if let Err(e) = db.delete_workspace_note(id) {
            tracing::warn!("notes: could not delete note {id}: {e}");
            return;
        }
        self.state.borrow_mut().last_deleted = Some(snapshot.clone());
        self.show_undo_toast("Note deleted");
        self.reload();
    }

    fn on_cycle_severity(self: &Rc<Self>, id: i64, current: &str) {
        tracing::info!("note list: cycle severity id={id} from={current}");
        let next = match current {
            SEVERITY_INFO => SEVERITY_WARNING,
            SEVERITY_WARNING => SEVERITY_IMPORTANT,
            _ => SEVERITY_INFO,
        };
        debug_assert!(NOTE_SEVERITIES.contains(&next));
        let Some(db) = open_db() else {
            return;
        };
        let Some(note) = db.get_workspace_note(id).ok().flatten() else {
            return;
        };
        if let Err(e) = db.update_workspace_note(
            id,
            &note.title,
            &note.text,
            &note.tags,
            next,
            note.alert_at,
        ) {
            tracing::warn!("notes: could not set severity {next} on {id}: {e}");
            return;
        }
        self.reload();
    }

    fn show_undo_toast(self: &Rc<Self>, message: &str) {
        self.toast_label.set_text(message);
        self.toast_revealer.set_reveal_child(true);
        // Cancel any in-flight dismiss timer so a newer deletion doesn't
        // get its snapshot wiped by a stale timeout from a prior toast.
        if let Some(prev) = self.state.borrow_mut().pending_toast_timer.take() {
            prev.remove();
        }
        let revealer = self.toast_revealer.clone();
        let state = self.state.clone();
        let id = gtk4::glib::timeout_add_seconds_local(UNDO_TOAST_TIMEOUT_SECS, move || {
            revealer.set_reveal_child(false);
            let mut s = state.borrow_mut();
            s.last_deleted = None;
            s.pending_toast_timer = None;
            gtk4::glib::ControlFlow::Break
        });
        self.state.borrow_mut().pending_toast_timer = Some(id);
    }

    fn undo_delete(self: &Rc<Self>) {
        let Some(snapshot) = self.state.borrow_mut().last_deleted.take() else {
            return;
        };
        let Some(db) = open_db() else {
            return;
        };
        let result = db.add_workspace_note(
            &self.record_key,
            &self.panel_id,
            &snapshot.title,
            &snapshot.text,
            &snapshot.tags,
            &snapshot.severity,
            snapshot.alert_at,
        );
        if let Err(e) = result {
            tracing::warn!("notes: undo failed: {e}");
            return;
        }
        self.toast_revealer.set_reveal_child(false);
        self.reload();
    }
}

fn open_db() -> Option<pax_db::Database> {
    pax_db::Database::open(&pax_db::Database::default_path())
        .map_err(|e| {
            tracing::warn!("notes list: could not open database: {e}");
            e
        })
        .ok()
}
