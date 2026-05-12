use std::borrow::Cow;
use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TextContentError {
    nul_offset: usize,
}

impl fmt::Display for TextContentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "content contains NUL byte at byte offset {}; GTK text buffers cannot display it",
            self.nul_offset
        )
    }
}

pub(super) fn validate_gtk_text(text: &str) -> Result<(), TextContentError> {
    match text.as_bytes().iter().position(|b| *b == 0) {
        Some(nul_offset) => Err(TextContentError { nul_offset }),
        None => Ok(()),
    }
}

pub(super) fn validate_file_text(path: &Path, text: &str) -> Result<(), String> {
    validate_gtk_text(text).map_err(|err| file_text_error(path, err))
}

pub(super) fn displayable_gtk_text(text: &str) -> Cow<'_, str> {
    if validate_gtk_text(text).is_ok() {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(text.replace('\0', "\u{FFFD}"))
    }
}

pub(super) fn file_text_error(path: &Path, err: TextContentError) -> String {
    format!("{}: {}", path.display(), err)
}

#[cfg(feature = "sourceview")]
pub(super) fn set_source_buffer_text(
    buffer: &sourceview5::Buffer,
    text: &str,
) -> Result<(), TextContentError> {
    use sourceview5::prelude::*;

    validate_gtk_text(text)?;
    buffer.set_text(text);
    Ok(())
}

#[cfg(feature = "sourceview")]
pub(super) fn replace_source_buffer_text_preserving_cursor(
    buffer: &sourceview5::Buffer,
    text: &str,
) -> Result<(), TextContentError> {
    use sourceview5::prelude::*;

    validate_gtk_text(text)?;
    let cursor_offset = buffer.cursor_position();
    buffer.set_text(text);
    let restored = buffer.iter_at_offset(cursor_offset.min(buffer.char_count()));
    buffer.place_cursor(&restored);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_gtk_text_accepts_regular_text() {
        assert!(validate_gtk_text("hello\nworld").is_ok());
    }

    #[test]
    fn validate_gtk_text_rejects_nul_bytes() {
        let err = validate_gtk_text("\0abc").unwrap_err();
        assert_eq!(err.nul_offset, 0);
    }
}
