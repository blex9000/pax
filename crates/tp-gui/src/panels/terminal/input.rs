use gtk4::gdk;

pub(crate) fn encode_key_input(key: gdk::Key, modifiers: gdk::ModifierType) -> Option<Vec<u8>> {
    if modifiers.contains(gdk::ModifierType::ALT_MASK) {
        return None;
    }

    let bytes = match key {
        gdk::Key::Return => vec![b'\r'],
        gdk::Key::BackSpace => vec![0x7f],
        gdk::Key::Tab => vec![b'\t'],
        gdk::Key::Escape => vec![0x1b],
        gdk::Key::Up => b"\x1b[A".to_vec(),
        gdk::Key::Down => b"\x1b[B".to_vec(),
        gdk::Key::Right => b"\x1b[C".to_vec(),
        gdk::Key::Left => b"\x1b[D".to_vec(),
        other => {
            if let Some(c) = other.to_unicode() {
                if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
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

    Some(bytes)
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
    fn ignores_alt_modified_keys() {
        assert_eq!(
            encode_key_input(gdk::Key::A, gdk::ModifierType::ALT_MASK),
            None
        );
    }
}
