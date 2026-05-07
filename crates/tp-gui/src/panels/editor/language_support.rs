use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

/// Resolve the keyword list for a GtkSourceView language by parsing the
/// shipped {id}.lang XML file. Result is cached after first lookup. Returns
/// an empty list when no .lang file is found or it has no <keyword> tags.
///
/// We pull every `<keyword>` token, not only the ones in style-ref="keyword"
/// contexts, because builtin/type/exception identifiers (e.g. `print`,
/// `Vec`, `Exception`) are exactly what users want to autocomplete too.
pub(super) fn keywords_for(lang_id: &str) -> Arc<Vec<String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Vec<String>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache.lock().unwrap().get(lang_id).cloned() {
        return cached;
    }

    let mut words = load_keywords_from_lang(lang_id);
    // Some IDs are thin extensions of a base language (python3 inherits
    // python via <context ref="python:...">). Walking those refs would
    // require a real parser; an alias merge covers the common cases.
    for parent in parent_language_aliases(lang_id) {
        let extra = load_keywords_from_lang(parent);
        words.extend(extra);
    }
    words.sort();
    words.dedup();

    let arc = Arc::new(words);
    cache
        .lock()
        .unwrap()
        .insert(lang_id.to_string(), arc.clone());
    arc
}

/// Pick a GtkSourceView language for files that the upstream mime/glob
/// heuristics fail to recognise. Returns `None` when no override applies,
/// leaving the buffer unstyled (the editor still works as plain text).
pub(super) fn fallback_language_for(
    manager: &sourceview5::LanguageManager,
    path: &Path,
) -> Option<sourceview5::Language> {
    let name = path.file_name().and_then(|s| s.to_str())?;
    // Dotenv-style files: KEY=value with `#` comments, shell-compatible.
    if name == ".env" || name == ".envrc" || name.starts_with(".env.") || name.ends_with(".env") {
        return manager.language("sh");
    }
    None
}

/// Map a language ID to base language(s) whose keywords should also be
/// loaded. Returns an empty slice for self-contained languages.
fn parent_language_aliases(lang_id: &str) -> &'static [&'static str] {
    match lang_id {
        "python3" => &["python"],
        "bash" | "zsh" => &["sh"],
        _ => &[],
    }
}

fn load_keywords_from_lang(lang_id: &str) -> Vec<String> {
    let manager = sourceview5::LanguageManager::default();
    for path in manager.search_path() {
        let candidate = Path::new(path.as_str()).join(format!("{}.lang", lang_id));
        if let Ok(xml) = std::fs::read_to_string(&candidate) {
            return parse_keyword_tags(&xml);
        }
    }
    Vec::new()
}

/// Extract every `<keyword>TOKEN</keyword>` payload from a .lang XML.
/// Whitespace inside the tag is trimmed; duplicates are *not* removed here
/// (the caller dedups after merging multiple files).
fn parse_keyword_tags(xml: &str) -> Vec<String> {
    static KEYWORD_RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = KEYWORD_RE.get_or_init(|| regex::Regex::new(r"<keyword>([^<]+)</keyword>").unwrap());

    re.captures_iter(xml)
        .filter_map(|c| {
            let raw = c.get(1)?.as_str().trim();
            if raw.is_empty() {
                None
            } else {
                Some(raw.to_string())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keyword_tags_extracts_payloads_and_skips_empty() {
        let xml = r#"
            <context id="keywords" style-ref="keyword">
              <keyword>def</keyword>
              <keyword>class</keyword>
              <keyword>  </keyword>
              <keyword>if</keyword>
            </context>
            <context id="builtins" style-ref="builtin-function">
              <keyword>print</keyword>
              <keyword>len</keyword>
            </context>
        "#;
        let mut got = parse_keyword_tags(xml);
        got.sort();
        assert_eq!(got, vec!["class", "def", "if", "len", "print"]);
    }

    #[test]
    fn parent_language_aliases_covers_python3_and_shell_dialects() {
        assert_eq!(parent_language_aliases("python3"), &["python"]);
        assert_eq!(parent_language_aliases("bash"), &["sh"]);
        assert_eq!(parent_language_aliases("zsh"), &["sh"]);
        assert!(parent_language_aliases("rust").is_empty());
        assert!(parent_language_aliases("totally-unknown").is_empty());
    }
}
