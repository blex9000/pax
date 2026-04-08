use std::collections::HashSet;
use std::path::{Path, PathBuf};

const REQUIRED_THEME_ICONS: &[&str] = &[
    "document-open-symbolic",
    "window-close-symbolic",
    "utilities-terminal-symbolic",
];

pub fn configure_icon_theme(icon_theme: &gtk4::IconTheme) {
    for path in existing_paths(custom_icon_search_paths()) {
        icon_theme.add_search_path(path);
    }

    if missing_required_icons(icon_theme) {
        for path in existing_paths(system_icon_search_paths()) {
            icon_theme.add_search_path(path);
        }
    }

    if missing_required_icons(icon_theme) {
        icon_theme.set_theme_name(Some("Adwaita"));
        if let Some(settings) = gtk4::Settings::default() {
            settings.set_gtk_icon_theme_name(Some("Adwaita"));
        }
    }
}

fn missing_required_icons(icon_theme: &gtk4::IconTheme) -> bool {
    REQUIRED_THEME_ICONS
        .iter()
        .any(|icon_name| !icon_theme.has_icon(icon_name))
}

fn custom_icon_search_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("resources/icons"),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("../resources/icons")))
            .unwrap_or_default(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/icons"),
    ]
}

fn system_icon_search_paths() -> Vec<PathBuf> {
    fallback_icon_search_paths(
        std::env::var("XDG_DATA_DIRS").ok().as_deref(),
        std::env::var_os("HOME").as_deref().map(Path::new),
    )
}

fn fallback_icon_search_paths(xdg_data_dirs: Option<&str>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(xdg_data_dirs) = xdg_data_dirs {
        for base in xdg_data_dirs.split(':').filter(|part| !part.is_empty()) {
            paths.push(PathBuf::from(base).join("icons"));
        }
    }

    if let Some(home) = home {
        paths.push(home.join(".local/share/icons"));
    }

    paths.extend([
        PathBuf::from("/opt/homebrew/share/icons"),
        PathBuf::from("/usr/local/share/icons"),
        PathBuf::from("/opt/local/share/icons"),
        PathBuf::from("/usr/share/icons"),
    ]);

    dedupe_paths(paths)
}

fn existing_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.into_iter().filter(|path| path.exists()).collect()
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for path in paths {
        if seen.insert(path.clone()) {
            deduped.push(path);
        }
    }
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_icon_search_paths_expand_xdg_and_home() {
        let paths = fallback_icon_search_paths(
            Some("/one/share:/two/share"),
            Some(Path::new("/home/tester")),
        );

        assert_eq!(paths[0], PathBuf::from("/one/share/icons"));
        assert_eq!(paths[1], PathBuf::from("/two/share/icons"));
        assert!(paths.contains(&PathBuf::from("/home/tester/.local/share/icons")));
        assert!(paths.contains(&PathBuf::from("/opt/homebrew/share/icons")));
    }

    #[test]
    fn fallback_icon_search_paths_deduplicate_entries() {
        let paths = fallback_icon_search_paths(
            Some("/dup/share:/dup/share"),
            Some(Path::new("/home/tester")),
        );

        let dup = PathBuf::from("/dup/share/icons");
        assert_eq!(paths.iter().filter(|path| **path == dup).count(), 1);
    }
}
