//! Modal for customizing keyboard shortcuts.
//!
//! One row per [`crate::keymap::Action`]: label, current binding
//! (rendered as a chip), and a "Change" button. Clicking the button
//! flips the chip into "Press a key…" mode; the next key event on
//! the dialog is captured, validated against existing bindings (the
//! UI flags conflicts inline), and saved on Apply.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::keymap::{self, Action, KeyBinding, KeyMap};

/// Open the keybindings customizer. `on_apply` fires when the user
/// commits their changes; the new map is also saved to disk.
pub fn show(parent: &impl IsA<gtk4::Window>) {
    let dialog = gtk4::Window::builder()
        .title("Keyboard Shortcuts")
        .transient_for(parent)
        .modal(true)
        .default_width(560)
        .default_height(540)
        .build();
    crate::theme::configure_dialog_window(&dialog);

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(16);
    root.set_margin_end(16);

    let intro = gtk4::Label::new(Some(
        "Click a binding to capture a new shortcut. Esc cancels capture.",
    ));
    intro.add_css_class("dim-label");
    intro.set_halign(gtk4::Align::Start);
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    root.append(&intro);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);

    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::None);
    list_box.add_css_class("boxed-list");
    scroll.set_child(Some(&list_box));
    root.append(&scroll);

    // Working copy mutated by row buttons; committed on Apply.
    let working: Rc<RefCell<KeyMap>> = Rc::new(RefCell::new(keymap::current()));

    // Holds the row currently in capture mode (so the Esc handler
    // knows which row to revert). At most one is active.
    let active_capture: Rc<RefCell<Option<CaptureState>>> = Rc::new(RefCell::new(None));

    // Window-level capture: when a row is in capture mode the next
    // key press is consumed here and routed to the active row.
    {
        let working = working.clone();
        let active = active_capture.clone();
        let key_ctrl = gtk4::EventControllerKey::new();
        key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
        key_ctrl.connect_key_pressed(move |_, key, _, modifiers| {
            let mut active_borrow = active.borrow_mut();
            let Some(state) = active_borrow.as_mut() else {
                return gtk4::glib::Propagation::Proceed;
            };
            if key == gtk4::gdk::Key::Escape {
                // Revert label, drop active state.
                state.button.set_label(&state.original_label);
                state.button.remove_css_class("suggested-action");
                *active_borrow = None;
                return gtk4::glib::Propagation::Stop;
            }
            // Ignore lone modifier presses so the user can hold
            // Ctrl/Shift/Alt without committing.
            if is_modifier_key(key) {
                return gtk4::glib::Propagation::Stop;
            }
            let Some(binding) = KeyBinding::from_event(key, modifiers) else {
                return gtk4::glib::Propagation::Stop;
            };
            // Apply to working copy.
            working
                .borrow_mut()
                .set_binding(state.action, binding.clone());
            // Repaint this row + any conflicting siblings.
            state.button.set_label(&binding.display());
            state.button.remove_css_class("suggested-action");
            // Conflict detection — if another action already had this
            // combo, mark it visually so the user can pick a different
            // shortcut for one of them.
            let conflicts = working.borrow().conflicts_with(&binding, state.action);
            if !conflicts.is_empty() {
                state.button.add_css_class("destructive-action");
                state.button.set_tooltip_text(Some(&format!(
                    "Conflicts with: {}",
                    conflicts
                        .iter()
                        .map(|a| a.label())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            } else {
                state.button.remove_css_class("destructive-action");
                state.button.set_tooltip_text(None);
            }
            *active_borrow = None;
            gtk4::glib::Propagation::Stop
        });
        dialog.add_controller(key_ctrl);
    }

    // Build a row per action.
    for &action in Action::all() {
        let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        row_box.set_margin_top(4);
        row_box.set_margin_bottom(4);
        row_box.set_margin_start(8);
        row_box.set_margin_end(8);

        let label = gtk4::Label::new(Some(action.label()));
        label.set_halign(gtk4::Align::Start);
        label.set_hexpand(true);
        label.set_xalign(0.0);
        row_box.append(&label);

        let binding_btn = gtk4::Button::new();
        binding_btn.add_css_class("flat");
        binding_btn.add_css_class("monospace");
        let current_label = working
            .borrow()
            .binding_for(action)
            .map(|b| b.display())
            .unwrap_or_else(|| "—".to_string());
        binding_btn.set_label(&current_label);
        binding_btn.set_tooltip_text(Some("Click to record a new shortcut"));
        {
            let active = active_capture.clone();
            let btn = binding_btn.clone();
            binding_btn.connect_clicked(move |_| {
                // If another row is already capturing, revert it.
                let mut a = active.borrow_mut();
                if let Some(prev) = a.take() {
                    prev.button.set_label(&prev.original_label);
                    prev.button.remove_css_class("suggested-action");
                }
                let original = btn.label().map(|g| g.to_string()).unwrap_or_default();
                btn.set_label("Press a key…");
                btn.add_css_class("suggested-action");
                *a = Some(CaptureState {
                    action,
                    button: btn.clone(),
                    original_label: original,
                });
            });
        }
        row_box.append(&binding_btn);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&row_box));
        row.set_activatable(false);
        list_box.append(&row);
    }

    // Footer buttons.
    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_top(8);

    let reset_btn = gtk4::Button::with_label("Reset to defaults");
    reset_btn.add_css_class("flat");
    let cancel_btn = gtk4::Button::with_label("Cancel");
    cancel_btn.add_css_class("flat");
    let apply_btn = gtk4::Button::with_label("Apply");
    apply_btn.add_css_class("suggested-action");
    btn_row.append(&reset_btn);
    btn_row.append(&cancel_btn);
    btn_row.append(&apply_btn);
    root.append(&btn_row);

    {
        let d = dialog.clone();
        cancel_btn.connect_clicked(move |_| d.close());
    }
    {
        let working = working.clone();
        let parent_window = dialog.clone().upcast::<gtk4::Window>();
        reset_btn.connect_clicked(move |_| {
            *working.borrow_mut() = KeyMap::default();
            // Closing and reopening is cheaper than rebuilding the
            // listbox in place — defaults reset is a rare action.
            parent_window.close();
            // Schedule re-open on next idle so the close completes
            // before we mount a new dialog.
            let parent_again = parent_window.clone();
            gtk4::glib::idle_add_local_once(move || {
                if let Some(transient_for) = parent_again.transient_for() {
                    show(&transient_for);
                }
            });
        });
    }
    {
        let d = dialog.clone();
        let working = working.clone();
        apply_btn.connect_clicked(move |_| {
            let new_map = working.borrow().clone();
            keymap::set_current(new_map.clone());
            if let Err(e) = keymap::save(&new_map) {
                tracing::warn!("keybindings: save failed: {e}");
            }
            d.close();
        });
    }

    dialog.set_child(Some(&root));
    dialog.present();
}

struct CaptureState {
    action: Action,
    button: gtk4::Button,
    original_label: String,
}

fn is_modifier_key(key: gtk4::gdk::Key) -> bool {
    use gtk4::gdk::Key;
    matches!(
        key,
        Key::Control_L
            | Key::Control_R
            | Key::Shift_L
            | Key::Shift_R
            | Key::Alt_L
            | Key::Alt_R
            | Key::Meta_L
            | Key::Meta_R
            | Key::Super_L
            | Key::Super_R
    )
}
