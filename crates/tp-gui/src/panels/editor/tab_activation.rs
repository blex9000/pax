use gtk4::glib::clone::Downgrade;
use gtk4::prelude::*;
use sourceview5::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use super::language_support::keywords_for;
use super::overview_ruler::collect_match_lines;
use super::EditorState;

struct TabViewSnapshot {
    child_name: String,
    source_buffer: Option<sourceview5::Buffer>,
    source_scroll: Option<(f64, f64)>,
    status_lang: String,
    status_pos: String,
    modified: bool,
    match_lines: Vec<i32>,
    note_lines: Option<(Vec<i32>, i32)>,
    keyword_lang_id: Option<String>,
}

pub(super) fn apply_tab_view_state(
    idx: usize,
    state: &Rc<RefCell<EditorState>>,
    notebook: &gtk4::Notebook,
    source_view: &sourceview5::View,
    source_scroll: &gtk4::ScrolledWindow,
    content_stack: &gtk4::Stack,
    status_lang: &gtk4::Label,
    status_pos: &gtk4::Label,
    status_modified: &gtk4::Label,
    match_lines: &Rc<RefCell<Vec<i32>>>,
    last_search_query: &Rc<RefCell<String>>,
    match_ruler: &gtk4::DrawingArea,
    notes_ruler: &Rc<super::notes_ruler::NotesRuler>,
    keyword_shadow_buffer: &sourceview5::Buffer,
) -> bool {
    set_active_tab_label(notebook, idx);
    remember_active_source_scroll(idx, state, source_scroll);

    let query = last_search_query.borrow().clone();
    let snapshot = {
        let st = state.borrow();
        let Some(open_file) = st.open_files.get(idx) else {
            return false;
        };
        tab_view_snapshot(open_file, &query)
    };

    super::completion_lifecycle::suspend_until_idle(source_view);
    content_stack.set_visible_child_name(&snapshot.child_name);

    if let Some(buf) = snapshot.source_buffer.as_ref() {
        source_view.set_buffer(Some(buf));
        source_view.set_visible(true);
        if let Some((scroll_x, scroll_y)) = snapshot.source_scroll {
            restore_source_scroll(source_scroll, scroll_x, scroll_y);
        }
        source_view.queue_resize();
        source_view.queue_draw();
        schedule_source_view_refresh(source_view, source_scroll, buf, snapshot.source_scroll);
    }

    status_lang.set_text(&snapshot.status_lang);
    status_pos.set_text(&snapshot.status_pos);
    status_modified.set_text(if snapshot.modified {
        "\u{25CF} Modified"
    } else {
        ""
    });

    let has_matches = !snapshot.match_lines.is_empty();
    *match_lines.borrow_mut() = snapshot.match_lines;
    match_ruler.set_visible(has_matches);
    match_ruler.queue_draw();

    if let Some((lines, total)) = snapshot.note_lines {
        notes_ruler.update(lines, total);
    } else {
        notes_ruler.clear();
    }

    set_keyword_shadow_buffer(keyword_shadow_buffer, snapshot.keyword_lang_id.as_deref());

    if let Ok(mut st) = state.try_borrow_mut() {
        st.active_tab = Some(idx);
    } else {
        tracing::debug!("editor tabs: active_tab update skipped during tab activation");
    }

    true
}

fn tab_view_snapshot(open_file: &super::OpenFile, query: &str) -> TabViewSnapshot {
    let child_name = open_file.content.content_stack_child_name(open_file.tab_id);
    match &open_file.content {
        super::tab_content::TabContent::Source(source) => {
            let language = source.buffer.language();
            let status_lang = language
                .as_ref()
                .map(|l| l.name().to_string())
                .unwrap_or_else(|| "Plain Text".to_string());
            let keyword_lang_id = language.as_ref().map(|l| l.id().to_string());
            let note_lines = source.notes.current_lines(&source.buffer);
            TabViewSnapshot {
                child_name,
                source_buffer: Some(source.buffer.clone()),
                source_scroll: Some((source.scroll_x.get(), source.scroll_y.get())),
                status_lang,
                status_pos: buffer_position_label(&source.buffer),
                modified: open_file.modified(),
                match_lines: collect_match_lines(&source.buffer, query),
                note_lines: Some((note_lines, source.buffer.line_count())),
                keyword_lang_id,
            }
        }
        super::tab_content::TabContent::Markdown(md) => TabViewSnapshot {
            child_name,
            source_buffer: None,
            source_scroll: None,
            status_lang: "Markdown".to_string(),
            status_pos: buffer_position_label(&md.buffer),
            modified: open_file.modified(),
            match_lines: collect_match_lines(&md.buffer, query),
            note_lines: None,
            keyword_lang_id: None,
        },
        super::tab_content::TabContent::Image(_) => TabViewSnapshot {
            child_name,
            source_buffer: None,
            source_scroll: None,
            status_lang: "Image".to_string(),
            status_pos: String::new(),
            modified: open_file.modified(),
            match_lines: Vec::new(),
            note_lines: None,
            keyword_lang_id: None,
        },
    }
}

fn set_active_tab_label(notebook: &gtk4::Notebook, idx: usize) {
    let n = notebook.n_pages();
    for i in 0..n {
        if let Some(page) = notebook.nth_page(Some(i)) {
            if let Some(tab_label) = notebook.tab_label(&page) {
                if i == idx as u32 {
                    tab_label.add_css_class("editor-tab-active");
                } else {
                    tab_label.remove_css_class("editor-tab-active");
                }
            }
        }
    }
}

fn buffer_position_label(buf: &sourceview5::Buffer) -> String {
    let iter = buf.iter_at_offset(buf.cursor_position());
    format!("Ln {}, Col {}", iter.line() + 1, iter.line_offset() + 1)
}

fn set_keyword_shadow_buffer(buffer: &sourceview5::Buffer, lang_id: Option<&str>) {
    let text = match lang_id {
        Some(id) => keywords_for(id).join(" "),
        None => String::new(),
    };
    buffer.set_text(&text);
}

fn remember_active_source_scroll(
    next_idx: usize,
    state: &Rc<RefCell<EditorState>>,
    source_scroll: &gtk4::ScrolledWindow,
) {
    let Ok(st) = state.try_borrow() else {
        return;
    };
    let Some(active_idx) = st.active_tab else {
        return;
    };
    if active_idx == next_idx {
        return;
    }
    let Some(open_file) = st.open_files.get(active_idx) else {
        return;
    };
    let super::tab_content::TabContent::Source(source) = &open_file.content else {
        return;
    };
    source.scroll_x.set(source_scroll.hadjustment().value());
    source.scroll_y.set(source_scroll.vadjustment().value());
}

fn restore_source_scroll(source_scroll: &gtk4::ScrolledWindow, scroll_x: f64, scroll_y: f64) {
    set_adjustment_value(&source_scroll.hadjustment(), scroll_x);
    set_adjustment_value(&source_scroll.vadjustment(), scroll_y);
}

fn set_adjustment_value(adjustment: &gtk4::Adjustment, value: f64) {
    let max = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    adjustment.set_value(value.clamp(adjustment.lower(), max));
}

fn schedule_source_view_refresh(
    source_view: &sourceview5::View,
    source_scroll: &gtk4::ScrolledWindow,
    buffer: &sourceview5::Buffer,
    scroll: Option<(f64, f64)>,
) {
    let view_weak = Downgrade::downgrade(source_view);
    let scroll_weak = Downgrade::downgrade(source_scroll);
    let buffer_weak = Downgrade::downgrade(buffer);
    gtk4::glib::idle_add_local_once(move || {
        let (Some(view), Some(scroll_widget), Some(buffer)) = (
            view_weak.upgrade(),
            scroll_weak.upgrade(),
            buffer_weak.upgrade(),
        ) else {
            return;
        };
        if !refresh_current_source_view(&view, &scroll_widget, &buffer, scroll) {
            return;
        }
    });
}

fn refresh_current_source_view(
    view: &sourceview5::View,
    source_scroll: &gtk4::ScrolledWindow,
    buffer: &sourceview5::Buffer,
    scroll: Option<(f64, f64)>,
) -> bool {
    let current_matches = view
        .buffer()
        .downcast::<sourceview5::Buffer>()
        .map(|current| current == buffer.clone())
        .unwrap_or(false);
    if !current_matches {
        return false;
    }

    if let Some((scroll_x, scroll_y)) = scroll {
        restore_source_scroll(source_scroll, scroll_x, scroll_y);
    }
    view.queue_resize();
    view.queue_draw();
    true
}
