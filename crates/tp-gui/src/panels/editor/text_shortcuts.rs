use gtk4::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextClipboardAction {
    Copy,
    Cut,
    Paste,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextHistoryAction {
    Undo,
    Redo,
}

fn text_clipboard_action(
    key: gtk4::gdk::Key,
    modifiers: gtk4::gdk::ModifierType,
) -> Option<TextClipboardAction> {
    if !crate::shortcuts::has_primary(modifiers) {
        return None;
    }

    match key {
        gtk4::gdk::Key::c | gtk4::gdk::Key::C => Some(TextClipboardAction::Copy),
        gtk4::gdk::Key::x | gtk4::gdk::Key::X => Some(TextClipboardAction::Cut),
        gtk4::gdk::Key::v | gtk4::gdk::Key::V => Some(TextClipboardAction::Paste),
        _ => None,
    }
}

fn text_history_action(
    key: gtk4::gdk::Key,
    modifiers: gtk4::gdk::ModifierType,
) -> Option<TextHistoryAction> {
    let primary = crate::shortcuts::has_primary(modifiers);
    let shift = modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK);

    if !primary {
        return None;
    }

    match key {
        gtk4::gdk::Key::z if !shift => Some(TextHistoryAction::Undo),
        gtk4::gdk::Key::y if !shift => Some(TextHistoryAction::Redo),
        gtk4::gdk::Key::Z if shift => Some(TextHistoryAction::Redo),
        _ => None,
    }
}

pub(super) fn install_text_clipboard_shortcuts<W: IsA<gtk4::Widget>>(widget: &W) {
    let widget = widget.as_ref().clone();
    let widget_for_action = widget.clone();
    let key_ctrl = gtk4::EventControllerKey::new();
    key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
    key_ctrl.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(action) = text_clipboard_action(key, modifiers) else {
            return gtk4::glib::Propagation::Proceed;
        };

        let action_name = match action {
            TextClipboardAction::Copy => "clipboard.copy",
            TextClipboardAction::Cut => "clipboard.cut",
            TextClipboardAction::Paste => "clipboard.paste",
        };

        if widget_for_action
            .activate_action(action_name, None::<&gtk4::glib::Variant>)
            .is_ok()
        {
            gtk4::glib::Propagation::Stop
        } else {
            gtk4::glib::Propagation::Proceed
        }
    });
    widget.add_controller(key_ctrl);
}

pub(super) fn install_text_history_shortcuts(view: &sourceview5::View) {
    // Look up the buffer at event time: the main editor SourceView swaps its
    // buffer every tab switch, so we must not capture the initial one.
    let view_c = view.clone();
    let key_ctrl = gtk4::EventControllerKey::new();
    key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
    key_ctrl.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(action) = text_history_action(key, modifiers) else {
            return gtk4::glib::Propagation::Proceed;
        };

        let Some(buffer) = view_c.buffer().downcast::<sourceview5::Buffer>().ok() else {
            return gtk4::glib::Propagation::Proceed;
        };

        match action {
            TextHistoryAction::Undo => {
                if buffer.can_undo() {
                    buffer.undo();
                }
            }
            TextHistoryAction::Redo => {
                if buffer.can_redo() {
                    buffer.redo();
                }
            }
        }

        gtk4::glib::Propagation::Stop
    });
    view.add_controller(key_ctrl);
}

/// Install bracket and quote auto-pairing on the SourceView.
///
/// - Typing `(`, `[`, `{`, `"`, `'`, `` ` `` inserts the matching closer
///   and places the cursor between them.
/// - With a selection active, the selection is wrapped instead.
/// - For the symmetrical pairs (quotes, backticks), if the cursor is
///   already sitting on the same character, just step past it instead of
///   inserting a doubled-up closer (matches what users expect from
///   VS Code / IntelliJ).
pub(super) fn install_bracket_auto_pair(view: &sourceview5::View) {
    use gtk4::gdk;

    let key_ctrl = gtk4::EventControllerKey::new();
    key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);

    let view_clone = view.clone();
    key_ctrl.connect_key_pressed(move |_, key, _, state| {
        // Ignore when modifiers are held (Ctrl+(, Alt+", etc. shouldn't pair).
        let allowed = gtk4::gdk::ModifierType::SHIFT_MASK;
        if state.intersects(!allowed) {
            return gtk4::glib::Propagation::Proceed;
        }

        let pair: Option<(&str, &str, bool)> = match key {
            gdk::Key::parenleft => Some(("(", ")", false)),
            gdk::Key::bracketleft => Some(("[", "]", false)),
            gdk::Key::braceleft => Some(("{", "}", false)),
            gdk::Key::quotedbl => Some(("\"", "\"", true)),
            gdk::Key::apostrophe => Some(("'", "'", true)),
            gdk::Key::grave => Some(("`", "`", true)),
            _ => return gtk4::glib::Propagation::Proceed,
        };
        let (open, close, symmetrical) = pair.unwrap();

        let Ok(buffer) = view_clone.buffer().downcast::<sourceview5::Buffer>() else {
            return gtk4::glib::Propagation::Proceed;
        };

        // Wrap selection.
        if buffer.has_selection() {
            if let Some((mut s, mut e)) = buffer.selection_bounds() {
                let text = buffer.text(&s, &e, false).to_string();
                buffer.begin_user_action();
                buffer.delete(&mut s, &mut e);
                buffer.insert(&mut s, &format!("{}{}{}", open, text, close));
                buffer.end_user_action();
                return gtk4::glib::Propagation::Stop;
            }
        }

        // For symmetrical chars (quotes, backticks): step over an existing
        // matching closer rather than inserting a duplicate.
        if symmetrical {
            let cursor = buffer.iter_at_mark(&buffer.get_insert());
            let mut next = cursor;
            if next.forward_char() {
                let next_char = buffer.text(&cursor, &next, false).to_string();
                if next_char == open {
                    buffer.place_cursor(&next);
                    return gtk4::glib::Propagation::Stop;
                }
            }
        }

        // Insert pair, leave cursor between.
        let mut iter = buffer.iter_at_mark(&buffer.get_insert());
        buffer.begin_user_action();
        buffer.insert(&mut iter, &format!("{}{}", open, close));
        let mut cursor = buffer.iter_at_mark(&buffer.get_insert());
        cursor.backward_char();
        buffer.place_cursor(&cursor);
        buffer.end_user_action();
        gtk4::glib::Propagation::Stop
    });
    view.add_controller(key_ctrl);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_text_clipboard_shortcuts() {
        let primary = crate::shortcuts::PRIMARY_MODIFIER;
        assert_eq!(
            text_clipboard_action(gtk4::gdk::Key::c, primary),
            Some(TextClipboardAction::Copy)
        );
        assert_eq!(
            text_clipboard_action(gtk4::gdk::Key::X, primary),
            Some(TextClipboardAction::Cut)
        );
        assert_eq!(
            text_clipboard_action(gtk4::gdk::Key::v, primary),
            Some(TextClipboardAction::Paste)
        );
        assert_eq!(
            text_clipboard_action(gtk4::gdk::Key::c, gtk4::gdk::ModifierType::SHIFT_MASK),
            None
        );
    }

    #[test]
    fn recognizes_text_history_shortcuts() {
        let primary = crate::shortcuts::PRIMARY_MODIFIER;
        assert_eq!(
            text_history_action(gtk4::gdk::Key::z, primary),
            Some(TextHistoryAction::Undo)
        );
        assert_eq!(
            text_history_action(gtk4::gdk::Key::y, primary),
            Some(TextHistoryAction::Redo)
        );
        assert_eq!(
            text_history_action(
                gtk4::gdk::Key::Z,
                primary | gtk4::gdk::ModifierType::SHIFT_MASK,
            ),
            Some(TextHistoryAction::Redo)
        );
        assert_eq!(
            text_history_action(gtk4::gdk::Key::z, gtk4::gdk::ModifierType::SHIFT_MASK),
            None
        );
    }
}
