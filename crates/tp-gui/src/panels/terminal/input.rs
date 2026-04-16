use gtk4::gdk;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalClipboardAction {
    Copy,
    Paste,
}

pub(crate) fn terminal_clipboard_action(
    key: gdk::Key,
    modifiers: gdk::ModifierType,
) -> Option<TerminalClipboardAction> {
    if !modifiers.contains(gdk::ModifierType::CONTROL_MASK)
        || !modifiers.contains(gdk::ModifierType::SHIFT_MASK)
    {
        return None;
    }

    match key {
        gdk::Key::c | gdk::Key::C => Some(TerminalClipboardAction::Copy),
        gdk::Key::v | gdk::Key::V => Some(TerminalClipboardAction::Paste),
        _ => None,
    }
}

pub(crate) fn encode_key_input(key: gdk::Key, modifiers: gdk::ModifierType) -> Option<Vec<u8>> {
    let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
    let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);

    let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);

    let mut bytes = match key {
        gdk::Key::Return => vec![b'\r'],
        gdk::Key::BackSpace => vec![0x7f],
        gdk::Key::Tab if shift => b"\x1b[Z".to_vec(),
        gdk::Key::ISO_Left_Tab => b"\x1b[Z".to_vec(),
        gdk::Key::Tab => vec![b'\t'],
        gdk::Key::Escape => vec![0x1b],
        gdk::Key::Up => b"\x1b[A".to_vec(),
        gdk::Key::Down => b"\x1b[B".to_vec(),
        gdk::Key::Right => b"\x1b[C".to_vec(),
        gdk::Key::Left => b"\x1b[D".to_vec(),
        gdk::Key::Home => b"\x1b[H".to_vec(),
        gdk::Key::End => b"\x1b[F".to_vec(),
        gdk::Key::Delete => b"\x1b[3~".to_vec(),
        gdk::Key::Insert => b"\x1b[2~".to_vec(),
        gdk::Key::Page_Up => b"\x1b[5~".to_vec(),
        gdk::Key::Page_Down => b"\x1b[6~".to_vec(),
        other => {
            if let Some(c) = other.to_unicode() {
                if control {
                    encode_control_char(c)?
                } else {
                    let mut buf = [0u8; 4];
                    c.encode_utf8(&mut buf).as_bytes().to_vec()
                }
            } else {
                return None;
            }
        }
    };

    if alt {
        let mut escaped = Vec::with_capacity(bytes.len() + 1);
        escaped.push(0x1b);
        escaped.append(&mut bytes);
        Some(escaped)
    } else {
        Some(bytes)
    }
}

fn encode_control_char(c: char) -> Option<Vec<u8>> {
    match c {
        '@' | ' ' => Some(vec![0x00]),
        'a'..='z' | 'A'..='Z' => Some(vec![(c.to_ascii_uppercase() as u8) - b'@']),
        '[' => Some(vec![0x1b]),
        '\\' => Some(vec![0x1c]),
        ']' => Some(vec![0x1d]),
        '^' => Some(vec![0x1e]),
        '_' => Some(vec![0x1f]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_printable_unicode_without_modifiers() {
        assert_eq!(
            encode_key_input(gdk::Key::A, gdk::ModifierType::empty()),
            Some(b"A".to_vec())
        );
        assert_eq!(
            encode_key_input(gdk::Key::ntilde, gdk::ModifierType::empty()),
            Some("ñ".as_bytes().to_vec())
        );
    }

    #[test]
    fn encodes_navigation_keys() {
        assert_eq!(
            encode_key_input(gdk::Key::Up, gdk::ModifierType::empty()),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            encode_key_input(gdk::Key::BackSpace, gdk::ModifierType::empty()),
            Some(vec![0x7f])
        );
        assert_eq!(
            encode_key_input(gdk::Key::Delete, gdk::ModifierType::empty()),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(
            encode_key_input(gdk::Key::Page_Down, gdk::ModifierType::empty()),
            Some(b"\x1b[6~".to_vec())
        );
    }

    #[test]
    fn encodes_control_letters() {
        assert_eq!(
            encode_key_input(gdk::Key::c, gdk::ModifierType::CONTROL_MASK),
            Some(vec![0x03])
        );
        assert_eq!(
            encode_key_input(gdk::Key::bracketleft, gdk::ModifierType::CONTROL_MASK),
            Some(vec![0x1b])
        );
    }

    #[test]
    fn prefixes_alt_modified_keys_with_escape() {
        assert_eq!(
            encode_key_input(gdk::Key::A, gdk::ModifierType::ALT_MASK),
            Some(b"\x1bA".to_vec())
        );
        assert_eq!(
            encode_key_input(gdk::Key::Left, gdk::ModifierType::ALT_MASK),
            Some(b"\x1b\x1b[D".to_vec())
        );
    }

    #[test]
    fn recognizes_terminal_clipboard_shortcuts() {
        let modifiers = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK;

        assert_eq!(
            terminal_clipboard_action(gdk::Key::c, modifiers),
            Some(TerminalClipboardAction::Copy)
        );
        assert_eq!(
            terminal_clipboard_action(gdk::Key::V, modifiers),
            Some(TerminalClipboardAction::Paste)
        );
        assert_eq!(
            terminal_clipboard_action(gdk::Key::c, gdk::ModifierType::CONTROL_MASK),
            None
        );
    }
}
