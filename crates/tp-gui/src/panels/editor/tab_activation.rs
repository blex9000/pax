use gtk4::prelude::*;
use sourceview5::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use super::language_support::keywords_for;
use super::overview_ruler::collect_match_lines;
use super::EditorState;

struct TabViewSnapshot {
    child_name: String,
    visible_view: Option<sourceview5::View>,
    status_lang: String,
    status_pos: String,
    modified: bool,
    external_modified: bool,
    match_lines: Vec<i32>,
    note_lines: Option<(Vec<i32>, i32)>,
    keyword_lang_id: Option<String>,
}

pub(super) fn apply_tab_view_state(
    idx: usize,
    state: &Rc<RefCell<EditorState>>,
    notebook: &gtk4::Notebook,
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
    remember_active_source_scroll(idx, state);

    let query = last_search_query.borrow().clone();
    let snapshot = {
        let st = state.borrow();
        let Some(open_file) = st.open_files.get(idx) else {
            return false;
        };
        tab_view_snapshot(open_file, &query)
    };

    if let Some(view) = snapshot.visible_view.as_ref() {
        super::completion_lifecycle::suspend_until_idle(view);
    }
    content_stack.set_visible_child_name(&snapshot.child_name);
    if let Some(view) = snapshot.visible_view.as_ref() {
        view.queue_resize();
        view.queue_draw();
    }

    status_lang.set_text(&snapshot.status_lang);
    status_pos.set_text(&snapshot.status_pos);
    let indicator = if snapshot.modified {
        super::dirty_state::IndicatorState::Dirty
    } else if snapshot.external_modified {
        super::dirty_state::IndicatorState::External
    } else {
        super::dirty_state::IndicatorState::Clean
    };
    super::dirty_state::set_status_indicator(status_modified, indicator);

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
                visible_view: Some(source.source_view.clone()),
                status_lang,
                status_pos: buffer_position_label(&source.buffer),
                modified: open_file.modified(),
                external_modified: open_file.external_modified(),
                match_lines: collect_match_lines(&source.buffer, query),
                note_lines: Some((note_lines, source.buffer.line_count())),
                keyword_lang_id,
            }
        }
        super::tab_content::TabContent::Markdown(md) => TabViewSnapshot {
            child_name,
            visible_view: Some(md.source_view.clone()),
            status_lang: "Markdown".to_string(),
            status_pos: buffer_position_label(&md.buffer),
            modified: open_file.modified(),
            external_modified: open_file.external_modified(),
            match_lines: collect_match_lines(&md.buffer, query),
            note_lines: None,
            keyword_lang_id: None,
        },
        super::tab_content::TabContent::Image(_) => TabViewSnapshot {
            child_name,
            visible_view: None,
            status_lang: "Image".to_string(),
            status_pos: String::new(),
            modified: open_file.modified(),
            external_modified: open_file.external_modified(),
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

fn remember_active_source_scroll(next_idx: usize, state: &Rc<RefCell<EditorState>>) {
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
    source
        .scroll_x
        .set(source.source_scroll.hadjustment().value());
    source
        .scroll_y
        .set(source.source_scroll.vadjustment().value());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::editor::file_backend::{FileBackend, LocalFileBackend};
    use crate::panels::editor::tab_content::{SourceTab, TabContent};
    use crate::panels::editor::{OpenFile, SidebarMode};
    use serial_test::serial;
    use std::cell::Cell;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn state_with_source_tab(
        root: &Path,
        buffer: sourceview5::Buffer,
        scroll_x: Rc<Cell<f64>>,
        scroll_y: Rc<Cell<f64>>,
    ) -> Rc<RefCell<EditorState>> {
        let backend: Arc<dyn FileBackend> = Arc::new(LocalFileBackend::new(root));
        let source_view = sourceview5::View::with_buffer(&buffer);
        let source_scroll = gtk4::ScrolledWindow::new();
        source_scroll.set_child(Some(&source_view));
        Rc::new(RefCell::new(EditorState {
            root_dir: root.to_path_buf(),
            open_files: vec![OpenFile {
                tab_id: 1,
                path: root.join("active.rs"),
                last_disk_mtime: 0,
                name_label: gtk4::Label::new(Some("active.rs")),
                content: TabContent::Source(SourceTab {
                    buffer,
                    source_view,
                    source_scroll,
                    modified: false,
                    scroll_x,
                    scroll_y,
                    saved_content: Rc::new(RefCell::new(String::new())),
                    external_modified: false,
                    notes: crate::panels::editor::notes_state::NotesState::new(),
                }),
            }],
            active_tab: Some(0),
            sidebar_visible: true,
            sidebar_mode: SidebarMode::Files,
            backend,
            poll_interval: 2,
            nav_back: Vec::new(),
            nav_forward: Vec::new(),
            recent_files: Vec::new(),
            on_nav_state_changed: None,
            record_key: String::new(),
            sync_suppress: Rc::new(Cell::new(false)),
            sync_input_cb: Rc::new(RefCell::new(None)),
        }))
    }

    fn set_scroll_offsets(scroll: &gtk4::ScrolledWindow, x: f64, y: f64) {
        let h = scroll.hadjustment();
        h.configure(x, 0.0, 1000.0, 1.0, 10.0, 100.0);
        let v = scroll.vadjustment();
        v.configure(y, 0.0, 1000.0, 1.0, 10.0, 100.0);
    }

    #[test]
    #[serial]
    fn apply_source_tab_uses_its_per_tab_stack_child() {
        crate::test_support::run_on_gtk_thread(|| {
            let dir = tempdir().unwrap();
            let buffer = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
            buffer.set_text("one\ntwo\nthree\n");
            let scroll_x = Rc::new(Cell::new(0.0));
            let scroll_y = Rc::new(Cell::new(0.0));
            let state = state_with_source_tab(dir.path(), buffer.clone(), scroll_x, scroll_y);
            let (source_view, source_scroll) = {
                let st = state.borrow();
                let TabContent::Source(source) = &st.open_files[0].content else {
                    unreachable!();
                };
                (source.source_view.clone(), source.source_scroll.clone())
            };

            let notebook = gtk4::Notebook::new();
            let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            notebook.append_page(&page, Some(&gtk4::Label::new(Some("active.rs"))));

            let content_stack = gtk4::Stack::new();
            let welcome = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            content_stack.add_named(&welcome, Some("welcome"));
            content_stack.add_named(&source_scroll, Some("tab-1"));
            content_stack.set_visible_child_name("welcome");

            let status_lang = gtk4::Label::new(None);
            let status_pos = gtk4::Label::new(None);
            let status_modified = gtk4::Label::new(None);
            let match_lines = Rc::new(RefCell::new(Vec::new()));
            let last_query = Rc::new(RefCell::new(String::new()));
            let match_ruler = gtk4::DrawingArea::new();
            let notes_ruler = Rc::new(super::super::notes_ruler::NotesRuler::new(
                source_view.clone(),
            ));
            let keyword_shadow_buffer = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);

            assert!(apply_tab_view_state(
                0,
                &state,
                &notebook,
                &content_stack,
                &status_lang,
                &status_pos,
                &status_modified,
                &match_lines,
                &last_query,
                &match_ruler,
                &notes_ruler,
                &keyword_shadow_buffer,
            ));

            assert_eq!(content_stack.visible_child_name().as_deref(), Some("tab-1"));
            assert_eq!(
                source_view
                    .buffer()
                    .downcast::<sourceview5::Buffer>()
                    .unwrap(),
                buffer
            );
        });
    }

    #[test]
    #[serial]
    fn remember_active_source_scroll_saves_per_tab_adjustment() {
        crate::test_support::run_on_gtk_thread(|| {
            let dir = tempdir().unwrap();
            let active_buffer = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
            let scroll_x = Rc::new(Cell::new(0.0));
            let scroll_y = Rc::new(Cell::new(0.0));
            let state = state_with_source_tab(
                dir.path(),
                active_buffer,
                scroll_x.clone(),
                scroll_y.clone(),
            );
            let source_scroll = {
                let st = state.borrow();
                let TabContent::Source(source) = &st.open_files[0].content else {
                    unreachable!();
                };
                source.source_scroll.clone()
            };
            source_scroll.set_child(None::<&gtk4::Widget>);
            set_scroll_offsets(&source_scroll, 0.0, 500.0);

            remember_active_source_scroll(1, &state);

            assert_eq!(scroll_x.get(), 0.0);
            assert_eq!(scroll_y.get(), 500.0);
        });
    }
}
