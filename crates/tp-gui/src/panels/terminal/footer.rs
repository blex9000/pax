//! # Terminal footer formatter
//!
//! Turns an OSC 7 URI (`file://host/path`) into the `user@host:path`
//! pair shown in the panel footer bar — once as Pango markup (green
//! user@host, blue path) and once as plain text for the tooltip.
//!
//! Extracted from `panel_host.rs` so both backends (VTE via
//! `current-directory-uri-changed`, PTY via the OSC 7 byte scanner)
//! hit the same parsing + rendering path.

use gtk4::glib;

/// Formatted pair `(markup, plain)` for the footer bar.
pub struct FooterFormat {
    pub markup: String,
    pub plain: String,
}

/// Parse an OSC 7 URI (`file://<host>/<path>`) and return the footer
/// strings. `user`, `host` and `home` are injected so the caller can
/// capture them once at panel creation without re-reading env vars on
/// every cwd change. Returns `None` for empty / non-`file://` inputs.
pub fn format_cwd_footer(uri: &str, user: &str, host: &str, home: &str) -> Option<FooterFormat> {
    if uri.is_empty() {
        return None;
    }
    let after_scheme = uri.strip_prefix("file://").unwrap_or(uri);
    let path = if let Some(slash_pos) = after_scheme.find('/') {
        &after_scheme[slash_pos..]
    } else {
        after_scheme
    };
    let path = percent_decode(path);
    let display_path = if !home.is_empty() && path.starts_with(home) {
        format!("~{}", &path[home.len()..])
    } else {
        path
    };
    let plain = format!("{}@{}:{}", user, host, display_path);
    let markup = format!(
        "<span color='#33cc33'>{}@{}</span>:<span color='#5588ff'>{}</span>",
        glib::markup_escape_text(user),
        glib::markup_escape_text(host),
        glib::markup_escape_text(&display_path),
    );
    Some(FooterFormat { markup, plain })
}

fn percent_decode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                result.push(byte);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(result).unwrap_or_else(|_| s.to_string())
}
