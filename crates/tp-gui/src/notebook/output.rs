//! Output items produced by a notebook cell run, plus the stdout marker
//! protocol used to distinguish text from rich content (images, future
//! HTML/table/...).
//!
//! Marker syntax (one per stdout line):
//!   <<pax:image:/abs/path/to/file.png>>
//!   <<pax:image:data:image/png;base64,iVBORw0KGgo...>>

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    Path(PathBuf),
    DataUri(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputItem {
    Text(String),
    Image(ImageSource),
    Error(String),
}

/// Parse a single line of stdout. If it matches a known `<<pax:...>>`
/// marker, returns the corresponding `OutputItem`; otherwise returns
/// `OutputItem::Text(line.to_string())`.
pub fn parse_line(line: &str) -> OutputItem {
    if let Some(rest) = line.strip_prefix("<<pax:image:") {
        if let Some(payload) = rest.strip_suffix(">>") {
            if payload.starts_with("data:image/") {
                return OutputItem::Image(ImageSource::DataUri(payload.to_string()));
            }
            return OutputItem::Image(ImageSource::Path(PathBuf::from(payload)));
        }
    }
    OutputItem::Text(line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_returned_as_text() {
        let item = parse_line("hello world");
        assert_eq!(item, OutputItem::Text("hello world".into()));
    }

    #[test]
    fn marker_image_path() {
        let item = parse_line("<<pax:image:/tmp/foo.png>>");
        assert_eq!(item, OutputItem::Image(ImageSource::Path("/tmp/foo.png".into())));
    }

    #[test]
    fn marker_image_data_uri() {
        let item = parse_line("<<pax:image:data:image/png;base64,AAAA>>");
        assert!(matches!(item, OutputItem::Image(ImageSource::DataUri(_))));
    }

    #[test]
    fn malformed_marker_falls_back_to_text() {
        let item = parse_line("<<pax:image:/tmp/foo.png");
        assert!(matches!(item, OutputItem::Text(_)));
    }

    #[test]
    fn empty_line_returns_empty_text() {
        let item = parse_line("");
        assert_eq!(item, OutputItem::Text(String::new()));
    }
}
