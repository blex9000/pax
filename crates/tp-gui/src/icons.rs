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

    for path in existing_paths(bundled_theme_search_paths()) {
        icon_theme.add_search_path(path);
    }

    if missing_required_icons(icon_theme) {
        for path in existing_paths(external_theme_search_paths(
            std::env::var("XDG_DATA_DIRS").ok().as_deref(),
            std::env::var_os("HOME").as_deref().map(Path::new),
        )) {
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
    let exe = std::env::current_exe().ok();

    dedupe_paths(vec![
        PathBuf::from("resources/icons"),
        bundle_resources_dir(exe.as_deref()).join("icons"),
        bundle_resources_dir_legacy_case(exe.as_deref()).join("icons"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/icons"),
    ])
}

fn bundled_theme_search_paths() -> Vec<PathBuf> {
    let exe = std::env::current_exe().ok();

    dedupe_paths(vec![
        bundle_resources_dir(exe.as_deref()).join("share/icons"),
        bundle_resources_dir_legacy_case(exe.as_deref()).join("share/icons"),
        PathBuf::from("resources/share/icons"),
    ])
}

fn external_theme_search_paths(xdg_data_dirs: Option<&str>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(xdg_data_dirs) = xdg_data_dirs {
        for base in xdg_data_dirs.split(':').filter(|part| !part.is_empty()) {
            paths.push(PathBuf::from(base).join("icons"));
        }
    }

    if let Some(home) = home {
        paths.push(home.join(".local/share/icons"));
        paths.push(home.join(".icons"));
        paths.push(home.join("Library/Icons"));
    }

    paths.extend([
        PathBuf::from("/opt/homebrew/share/icons"),
        PathBuf::from("/usr/local/share/icons"),
        PathBuf::from("/opt/local/share/icons"),
        PathBuf::from("/usr/share/icons"),
    ]);

    dedupe_paths(paths)
}

fn bundle_resources_dir(exe: Option<&Path>) -> PathBuf {
    exe.and_then(|path| path.parent()?.parent().map(|dir| dir.join("Resources")))
        .unwrap_or_default()
}

fn bundle_resources_dir_legacy_case(exe: Option<&Path>) -> PathBuf {
    exe.and_then(|path| path.parent()?.parent().map(|dir| dir.join("resources")))
        .unwrap_or_default()
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
        let paths = external_theme_search_paths(
            Some("/one/share:/two/share"),
            Some(Path::new("/home/tester")),
        );

        assert_eq!(paths[0], PathBuf::from("/one/share/icons"));
        assert_eq!(paths[1], PathBuf::from("/two/share/icons"));
        assert!(paths.contains(&PathBuf::from("/home/tester/.local/share/icons")));
        assert!(paths.contains(&PathBuf::from("/home/tester/.icons")));
        assert!(paths.contains(&PathBuf::from("/home/tester/Library/Icons")));
        assert!(paths.contains(&PathBuf::from("/opt/homebrew/share/icons")));
    }

    #[test]
    fn fallback_icon_search_paths_deduplicate_entries() {
        let paths = external_theme_search_paths(
            Some("/dup/share:/dup/share"),
            Some(Path::new("/home/tester")),
        );

        let dup = PathBuf::from("/dup/share/icons");
        assert_eq!(paths.iter().filter(|path| **path == dup).count(), 1);
    }

    #[test]
    fn custom_icon_paths_include_macos_bundle_resources() {
        let exe = Path::new("/Applications/Pax.app/Contents/MacOS/pax");
        let paths = dedupe_paths(vec![
            PathBuf::from("resources/icons"),
            bundle_resources_dir(Some(exe)).join("icons"),
            bundle_resources_dir_legacy_case(Some(exe)).join("icons"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/icons"),
        ]);

        assert!(paths.contains(&PathBuf::from(
            "/Applications/Pax.app/Contents/Resources/icons"
        )));
    }

    #[test]
    fn bundled_theme_paths_include_macos_bundle_share_icons() {
        let exe = Path::new("/Applications/Pax.app/Contents/MacOS/pax");
        let paths = dedupe_paths(vec![
            bundle_resources_dir(Some(exe)).join("share/icons"),
            bundle_resources_dir_legacy_case(Some(exe)).join("share/icons"),
            PathBuf::from("resources/share/icons"),
        ]);

        assert!(paths.contains(&PathBuf::from(
            "/Applications/Pax.app/Contents/Resources/share/icons"
        )));
    }
}
