//! # Terminal footer formatter
//!
//! Turns an OSC 7 URI (`file://host/path` or `file://user@host/path`) into the `user@host:path`
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

/// Parse an OSC 7 URI (`file://<host>/<path>` or
/// `file://<user>@<host>/<path>`) and return the footer strings. The fallback
/// `user`, `host` and `home` values are injected so the caller can capture
/// them once at panel creation without re-reading env vars on every cwd
/// change. Returns `None` for empty / non-`file://` inputs.
pub fn format_cwd_footer(uri: &str, user: &str, host: &str, home: &str) -> Option<FooterFormat> {
    if uri.is_empty() {
        return None;
    }
    let after_scheme = uri.strip_prefix("file://").unwrap_or(uri);
    let (authority, path) = if let Some(slash_pos) = after_scheme.find('/') {
        (&after_scheme[..slash_pos], &after_scheme[slash_pos..])
    } else {
        ("", after_scheme)
    };
    let (display_user, display_host) = parse_authority(authority, user, host);
    let path = percent_decode(path);
    let display_path = if !home.is_empty() && path.starts_with(home) {
        format!("~{}", &path[home.len()..])
    } else {
        path
    };
    let plain = format!("{}@{}:{}", display_user, display_host, display_path);
    let markup = format!(
        "<span color='#33cc33'>{}@{}</span>:<span color='#5588ff'>{}</span>",
        glib::markup_escape_text(&display_user),
        glib::markup_escape_text(&display_host),
        glib::markup_escape_text(&display_path),
    );
    Some(FooterFormat { markup, plain })
}

fn parse_authority(authority: &str, fallback_user: &str, fallback_host: &str) -> (String, String) {
    if authority.is_empty() {
        return (fallback_user.to_string(), fallback_host.to_string());
    }
    let authority = percent_decode(authority);
    if let Some((user, host)) = authority.rsplit_once('@') {
        let user = if user.is_empty() { fallback_user } else { user };
        let host = if host.is_empty() { fallback_host } else { host };
        return (user.to_string(), host.to_string());
    }
    let host = if authority.is_empty() {
        fallback_host.to_string()
    } else {
        authority
    };
    (fallback_user.to_string(), host)
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

#[cfg(test)]
mod tests {
    use super::format_cwd_footer;

    #[test]
    fn formats_remote_user_and_host_from_osc7_authority() {
        let formatted = format_cwd_footer(
            "file://guruai@MILFHAI00APD/home/guruai/project",
            "xb",
            "xb-zbook",
            "/home/xb",
        )
        .unwrap();

        assert_eq!(formatted.plain, "guruai@MILFHAI00APD:/home/guruai/project");
        assert!(formatted.markup.contains("guruai@MILFHAI00APD"));
    }

    #[test]
    fn falls_back_to_local_user_when_authority_has_only_host() {
        let formatted =
            format_cwd_footer("file://xb-zbook/home/xb/src", "xb", "fallback", "/home/xb").unwrap();

        assert_eq!(formatted.plain, "xb@xb-zbook:~/src");
    }

    #[test]
    fn decodes_percent_encoded_authority_and_path() {
        let formatted = format_cwd_footer(
            "file://remote%20user@host%201/srv/project%20one",
            "xb",
            "fallback",
            "/home/xb",
        )
        .unwrap();

        assert_eq!(formatted.plain, "remote user@host 1:/srv/project one");
    }
}
