use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use super::EditorState;

pub(super) fn install_dirty_tracking(
    buffer: &sourceview5::Buffer,
    saved_content: Rc<RefCell<String>>,
    tab_id: u64,
    state: &Rc<RefCell<EditorState>>,
    tab_dirty_dot: &gtk4::Label,
    status_modified: &gtk4::Label,
) {
    let state_c = state.clone();
    let dot_c = tab_dirty_dot.clone();
    let status_c = status_modified.clone();

    buffer.connect_changed(move |buf| {
        let current = buf
            .text(&buf.start_iter(), &buf.end_iter(), false)
            .to_string();
        let is_dirty = current != *saved_content.borrow();

        dot_c.set_text(if is_dirty { "\u{25CF} " } else { "" });
        status_c.set_text(if is_dirty { "\u{25CF} Modified" } else { "" });

        if let Ok(mut st) = state_c.try_borrow_mut() {
            if let Some(file_idx) = st.open_files.iter().position(|f| f.tab_id == tab_id) {
                st.open_files[file_idx].set_modified(is_dirty);
            }
        } else {
            tracing::debug!("editor dirty tracking: skipped state update during active borrow");
        }
    });
}
