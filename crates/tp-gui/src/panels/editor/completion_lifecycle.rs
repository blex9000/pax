use gtk4::prelude::*;
use sourceview5::prelude::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

const COMPLETION_SURFACE_HIDE_DELAY: Duration = Duration::from_millis(120);

struct GuardState {
    focused: Cell<bool>,
    mapped: Cell<bool>,
    window_active: Cell<bool>,
    blocked: Cell<bool>,
}

impl GuardState {
    fn new(view: &sourceview5::View) -> Self {
        Self {
            focused: Cell::new(view.has_focus()),
            mapped: Cell::new(view.is_mapped()),
            window_active: Cell::new(false),
            blocked: Cell::new(false),
        }
    }
}

/// Keep GtkSourceView completion transient surfaces tied to the editor view.
///
/// GtkSourceCompletion owns native popup/info surfaces. Calling only `hide()`
/// while the view is losing focus or swapping buffers can leave a pending
/// interactive request alive long enough for GTK to orphan a small top-level
/// popup. Blocking until the next main-loop tick drains that transient state.
pub(super) fn suspend_until_idle(view: &sourceview5::View) {
    let completion = view.completion();
    let view_weak = view.downgrade();
    completion.block_interactive();
    hide_now(view);

    let idle_completion = completion.clone();
    let idle_view = view_weak.clone();
    gtk4::glib::idle_add_local_once(move || {
        if let Some(view) = idle_view.upgrade() {
            hide_now(&view);
        } else {
            idle_completion.hide();
        }
    });

    gtk4::glib::timeout_add_local_once(COMPLETION_SURFACE_HIDE_DELAY, move || {
        if let Some(view) = view_weak.upgrade() {
            hide_now(&view);
        } else {
            completion.hide();
        }
        completion.hide();
        completion.unblock_interactive();
    });
}

pub(super) fn configure(completion: &sourceview5::Completion) {
    // GtkSourceCompletion uses native popover surfaces that can remain
    // orphaned by GTK after rapid editor focus/buffer changes. Keep the
    // built-in interactive completion disabled until it is replaced by a
    // popup owned entirely by the editor widget tree.
    completion.block_interactive();
    completion.hide();
    completion.set_remember_info_visibility(false);
}

pub(super) fn install_view_guards(view: &sourceview5::View) {
    let state = Rc::new(GuardState::new(view));

    let focus = gtk4::EventControllerFocus::new();
    {
        let view = view.clone();
        let state = state.clone();
        focus.connect_enter(move |_| {
            state.focused.set(true);
            update_guard(&view, &state);
        });
    }
    {
        let view = view.clone();
        let state = state.clone();
        focus.connect_leave(move |_| {
            state.focused.set(false);
            update_guard(&view, &state);
        });
    }
    view.add_controller(focus);

    let key_controller = gtk4::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let view = view.clone();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                suspend_until_idle(&view);
            }
            gtk4::glib::Propagation::Proceed
        });
    }
    view.add_controller(key_controller);

    let click_controller = gtk4::GestureClick::new();
    click_controller.set_button(0);
    click_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let view = view.clone();
        click_controller.connect_pressed(move |_, _, _, _| {
            hide_now(&view);
        });
    }
    view.add_controller(click_controller);

    {
        let view = view.clone();
        let state = state.clone();
        view.clone().connect_hide(move |_| {
            state.mapped.set(false);
            update_guard(&view, &state);
        });
    }
    {
        let view = view.clone();
        let state = state.clone();
        view.clone().connect_map(move |_| {
            state.mapped.set(true);
            update_guard(&view, &state);
        });
    }
    {
        let view = view.clone();
        let state = state.clone();
        view.clone().connect_unmap(move |_| {
            state.mapped.set(false);
            update_guard(&view, &state);
        });
    }

    attach_window_guard(view, &state);
    {
        let guarded_view = view.clone();
        let state = state.clone();
        view.connect_root_notify(move |view| {
            attach_window_guard(view, &state);
            update_guard(&guarded_view, &state);
        });
    }

    update_guard(view, &state);
}

fn attach_window_guard(view: &sourceview5::View, state: &Rc<GuardState>) {
    let Some(window) = view
        .root()
        .and_then(|root| root.downcast::<gtk4::Window>().ok())
    else {
        state.window_active.set(false);
        return;
    };
    state.window_active.set(window.is_active());
    let view = view.clone();
    let state = state.clone();
    window.connect_is_active_notify(move |window| {
        state.window_active.set(window.is_active());
        update_guard(&view, &state);
    });
}

fn update_guard(view: &sourceview5::View, state: &GuardState) {
    let should_block = !state.focused.get() || !state.mapped.get() || !state.window_active.get();
    let completion = view.completion();
    hide_now(view);

    if should_block {
        if !state.blocked.replace(true) {
            completion.block_interactive();
        }
    } else if state.blocked.replace(false) {
        completion.unblock_interactive();
    }
}

fn hide_now(view: &sourceview5::View) {
    view.completion().hide();
    close_completion_surfaces(view);
}

fn close_completion_surfaces(view: &sourceview5::View) {
    let root_window = view
        .root()
        .and_then(|root| root.downcast::<gtk4::Window>().ok());

    if let Some(window) = root_window {
        close_completion_widget_tree(window.upcast_ref());
    }

    for widget in gtk4::Window::list_toplevels() {
        close_completion_widget_tree(&widget);
    }
}

fn close_completion_widget_tree(widget: &gtk4::Widget) -> bool {
    let self_match = is_completion_surface(widget);
    let mut child_match = false;
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        child_match |= close_completion_widget_tree(&current);
    }

    let subtree_match = self_match || child_match;
    if self_match || should_close_completion_container(widget, child_match) {
        close_widget_surface(widget);
    }
    subtree_match
}

fn is_completion_surface(widget: &gtk4::Widget) -> bool {
    is_completion_surface_type(widget.type_().name())
}

fn is_completion_surface_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "GtkSourceCompletionList" | "GtkSourceCompletionInfo" | "GtkSourceAssistant"
    )
}

fn should_close_completion_container(widget: &gtk4::Widget, child_match: bool) -> bool {
    child_match && widget.is::<gtk4::Popover>()
}

fn close_widget_surface(widget: &gtk4::Widget) {
    if let Some(popover) = widget.downcast_ref::<gtk4::Popover>() {
        popover.popdown();
        popover.set_visible(false);
        return;
    }

    if widget.is::<gtk4::Window>() {
        return;
    }

    widget.set_visible(false);
}

#[cfg(test)]
mod tests {
    use super::is_completion_surface_type;

    #[test]
    fn recognizes_gtksource_completion_popover_types() {
        assert!(is_completion_surface_type("GtkSourceCompletionList"));
        assert!(is_completion_surface_type("GtkSourceCompletionInfo"));
        assert!(is_completion_surface_type("GtkSourceAssistant"));
        assert!(!is_completion_surface_type("GtkSourceCompletionListBox"));
        assert!(!is_completion_surface_type("GtkPopover"));
        assert!(!is_completion_surface_type("GtkApplicationWindow"));
    }
}
