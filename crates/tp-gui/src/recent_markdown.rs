//! Persistent "last N markdown files opened" list.
//!
//! Stored as JSON at `$XDG_CACHE_HOME/pax/recent_markdown.json` (falling
//! back to `$HOME/.cache/pax/...` and finally to a temp directory).
//! Newest first, deduped, capped at `MAX_ENTRIES`. Recorded by
//! `MarkdownPanel::new`; consumed by the markdown config dialog to
//! offer one-click selection of a previously opened file.

use std::path::PathBuf;

const MAX_ENTRIES: usize = 20;

fn store_path() -> PathBuf {
    let mut p = if let Ok(h) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(h)
    } else if let Ok(h) = std::env::var("HOME") {
        PathBuf::from(h).join(".cache")
    } else {
        std::env::temp_dir()
    };
    p.push("pax");
    let _ = std::fs::create_dir_all(&p);
    p.push("recent_markdown.json");
    p
}

pub fn list() -> Vec<String> {
    let path = store_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    if raw.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn record(path: &str) {
    let path = path.trim();
    if path.is_empty() {
        return;
    }
    let mut entries = list();
    entries.retain(|e| e != path);
    entries.insert(0, path.to_string());
    if entries.len() > MAX_ENTRIES {
        entries.truncate(MAX_ENTRIES);
    }
    if let Ok(s) = serde_json::to_string(&entries) {
        let _ = std::fs::write(store_path(), s);
    }
}

pub fn forget(path: &str) {
    let mut entries = list();
    let before = entries.len();
    entries.retain(|e| e != path);
    if entries.len() != before {
        if let Ok(s) = serde_json::to_string(&entries) {
            let _ = std::fs::write(store_path(), s);
        }
    }
}
