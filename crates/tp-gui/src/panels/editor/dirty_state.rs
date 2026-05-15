use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use super::EditorState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IndicatorState {
    Clean,
    Dirty,
    External,
}

pub(super) fn set_tab_indicator(label: &gtk4::Label, state: IndicatorState) {
    label.remove_css_class("dirty-indicator");
    label.remove_css_class("external-change-indicator");
    match state {
        IndicatorState::Clean => label.set_text(""),
        IndicatorState::Dirty => {
            label.add_css_class("dirty-indicator");
            label.set_text("\u{25CF} ");
        }
        IndicatorState::External => {
            label.add_css_class("external-change-indicator");
            label.set_text("\u{25CF} ");
        }
    }
}

pub(super) fn set_status_indicator(label: &gtk4::Label, state: IndicatorState) {
    label.remove_css_class("dirty-indicator");
    label.remove_css_class("external-change-indicator");
    match state {
        IndicatorState::Clean => label.set_text(""),
        IndicatorState::Dirty => {
            label.add_css_class("dirty-indicator");
            label.set_text("\u{25CF} Modified");
        }
        IndicatorState::External => {
            label.add_css_class("external-change-indicator");
            label.set_text("\u{25CF} Updated externally");
        }
    }
}

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

        if let Ok(mut st) = state_c.try_borrow_mut() {
            if let Some(file_idx) = st.open_files.iter().position(|f| f.tab_id == tab_id) {
                if is_dirty {
                    st.open_files[file_idx].set_external_modified(false);
                }
                st.open_files[file_idx].set_modified(is_dirty);
                let state = if is_dirty {
                    IndicatorState::Dirty
                } else if st.open_files[file_idx].external_modified() {
                    IndicatorState::External
                } else {
                    IndicatorState::Clean
                };
                set_tab_indicator(&dot_c, state);
                let active = st.active_tab == Some(file_idx);
                if active {
                    set_status_indicator(&status_c, state);
                }
                return;
            }
        } else {
            tracing::debug!("editor dirty tracking: skipped state update during active borrow");
        }

        let state = if is_dirty {
            IndicatorState::Dirty
        } else {
            IndicatorState::Clean
        };
        set_tab_indicator(&dot_c, state);
        set_status_indicator(&status_c, state);
    });
}
