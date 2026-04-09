use gtk4::prelude::*;
use std::cell::Cell;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use webkit6::prelude::*;

use super::PanelBackend;

#[derive(Debug)]
pub struct BrowserPanel {
    widget: gtk4::Widget,
    web_view: webkit6::WebView,
}

impl BrowserPanel {
    pub fn new(url: &str, workspace_dir: Option<&str>) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        toolbar.add_css_class("markdown-toolbar");
        toolbar.set_margin_start(4);
        toolbar.set_margin_end(4);
        toolbar.set_margin_top(2);
        toolbar.set_margin_bottom(2);

        let back_btn = gtk4::Button::new();
        back_btn.set_icon_name("go-previous-symbolic");
        back_btn.add_css_class("flat");
        back_btn.set_tooltip_text(Some("Back"));
        back_btn.set_sensitive(false);

        let forward_btn = gtk4::Button::new();
        forward_btn.set_icon_name("go-next-symbolic");
        forward_btn.add_css_class("flat");
        forward_btn.set_tooltip_text(Some("Forward"));
        forward_btn.set_sensitive(false);

        let reload_btn = gtk4::Button::new();
        reload_btn.set_icon_name("view-refresh-symbolic");
        reload_btn.add_css_class("flat");
        reload_btn.set_tooltip_text(Some("Reload"));

        let address_entry = gtk4::Entry::new();
        address_entry.set_hexpand(true);
        address_entry.set_placeholder_text(Some("https://example.com"));

        let progress = gtk4::ProgressBar::new();
        progress.set_show_text(false);
        progress.set_hexpand(true);
        progress.set_visible(false);
        let is_loading = Rc::new(Cell::new(false));

        let status_label = gtk4::Label::new(None);
        status_label.add_css_class("caption");
        status_label.add_css_class("dim-label");
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_margin_start(8);
        status_label.set_margin_end(8);
        status_label.set_margin_bottom(4);

        toolbar.append(&back_btn);
        toolbar.append(&forward_btn);
        toolbar.append(&reload_btn);
        toolbar.append(&address_entry);
        container.append(&toolbar);
        container.append(&progress);

        let web_view = webkit6::WebView::new();
        web_view.set_hexpand(true);
        web_view.set_vexpand(true);

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_child(Some(&web_view));
        scrolled.set_hexpand(true);
        scrolled.set_vexpand(true);
        container.append(&scrolled);
        container.append(&status_label);

        let initial_uri =
            normalized_browser_uri(url, workspace_dir).unwrap_or_else(|| "about:blank".to_string());
        address_entry.set_text(&initial_uri);
        web_view.load_uri(&initial_uri);

        {
            let web_view = web_view.clone();
            let status_label = status_label.clone();
            let workspace_dir = workspace_dir.map(|dir| dir.to_string());
            address_entry.connect_activate(move |entry| {
                let normalized =
                    normalized_browser_uri(entry.text().as_str(), workspace_dir.as_deref());
                match normalized {
                    Some(uri) => {
                        status_label.set_text("");
                        entry.set_text(&uri);
                        web_view.load_uri(&uri);
                    }
                    None => {
                        status_label.set_text("Invalid URL or file path");
                    }
                }
            });
        }

        {
            let web_view = web_view.clone();
            back_btn.connect_clicked(move |_| {
                web_view.go_back();
            });
        }

        {
            let web_view = web_view.clone();
            forward_btn.connect_clicked(move |_| {
                web_view.go_forward();
            });
        }

        {
            let web_view = web_view.clone();
            let is_loading = is_loading.clone();
            reload_btn.connect_clicked(move |_| {
                if is_loading.get() {
                    web_view.stop_loading();
                } else {
                    web_view.reload();
                }
            });
        }

        {
            let address_entry = address_entry.clone();
            web_view.connect_uri_notify(move |view| {
                if let Some(uri) = view.uri() {
                    address_entry.set_text(uri.as_str());
                }
            });
        }

        {
            let status_label = status_label.clone();
            web_view.connect_title_notify(move |view| {
                status_label.set_tooltip_text(view.title().as_deref());
            });
        }

        {
            let progress = progress.clone();
            let back_btn = back_btn.clone();
            let forward_btn = forward_btn.clone();
            let reload_btn = reload_btn.clone();
            let status_label = status_label.clone();
            let is_loading = is_loading.clone();
            web_view.connect_load_changed(move |view, event| {
                back_btn.set_sensitive(view.can_go_back());
                forward_btn.set_sensitive(view.can_go_forward());
                match event {
                    webkit6::LoadEvent::Started => {
                        is_loading.set(true);
                        progress.set_visible(true);
                        progress.set_fraction(0.0);
                        reload_btn.set_icon_name("process-stop-symbolic");
                        status_label.set_text("Loading...");
                    }
                    webkit6::LoadEvent::Committed => {
                        status_label.set_text("");
                    }
                    webkit6::LoadEvent::Finished => {
                        is_loading.set(false);
                        progress.set_fraction(1.0);
                        progress.set_visible(false);
                        reload_btn.set_icon_name("view-refresh-symbolic");
                        status_label.set_text("");
                    }
                    _ => {}
                }
            });
        }

        {
            let progress = progress.clone();
            web_view.connect_estimated_load_progress_notify(move |view| {
                progress.set_fraction(view.estimated_load_progress().clamp(0.0, 1.0));
            });
        }

        {
            let status_label = status_label.clone();
            let progress = progress.clone();
            let reload_btn = reload_btn.clone();
            let is_loading = is_loading.clone();
            web_view.connect_load_failed(move |_view, _event, _uri, error| {
                is_loading.set(false);
                status_label.set_text(&format!("Load failed: {}", error));
                progress.set_visible(false);
                reload_btn.set_icon_name("view-refresh-symbolic");
                false
            });
        }

        let widget = container.upcast::<gtk4::Widget>();
        Self { widget, web_view }
    }
}

impl PanelBackend for BrowserPanel {
    fn panel_type(&self) -> &str {
        "browser"
    }

    fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }

    fn on_focus(&self) {
        self.web_view.grab_focus();
    }

    fn get_text_content(&self) -> Option<String> {
        self.web_view.uri().map(|uri| uri.to_string())
    }
}

fn normalized_browser_uri(input: &str, workspace_dir: Option<&str>) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    if has_uri_scheme(trimmed) {
        return Some(trimmed.to_string());
    }

    if let Some(path) = resolve_browser_path(trimmed, workspace_dir) {
        return Some(gtk4::gio::File::for_path(path).uri().to_string());
    }

    if trimmed.chars().any(char::is_whitespace) {
        return None;
    }

    if looks_like_local_http_target(trimmed) {
        return Some(format!("http://{}", trimmed));
    }

    Some(format!("https://{}", trimmed))
}

fn has_uri_scheme(value: &str) -> bool {
    value.starts_with("about:")
        || value.starts_with("data:")
        || value.starts_with("file:")
        || value.contains("://")
}

fn looks_like_local_http_target(value: &str) -> bool {
    value.starts_with("localhost")
        || value.starts_with("127.")
        || value.starts_with("[::1]")
        || value
            .split('/')
            .next()
            .is_some_and(|host| host.parse::<std::net::IpAddr>().is_ok() || host.contains(':'))
}

fn resolve_browser_path(value: &str, workspace_dir: Option<&str>) -> Option<PathBuf> {
    let candidate = expand_home_path(value)?;
    if candidate.exists() {
        return Some(candidate);
    }

    let relative = Path::new(value);
    if !relative.is_relative() || relative.components().next().is_none() {
        return None;
    }

    let base = workspace_dir.map(PathBuf::from)?;
    let resolved = base.join(relative);
    resolved.exists().then_some(resolved)
}

fn expand_home_path(value: &str) -> Option<PathBuf> {
    if value == "~" {
        return std::env::var_os("HOME").map(PathBuf::from);
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return std::env::var_os("HOME").map(|home| PathBuf::from(home).join(rest));
    }

    let path = PathBuf::from(value);
    match path.components().next() {
        Some(Component::Normal(_)) | Some(Component::RootDir) | Some(Component::CurDir) => {
            Some(path)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::normalized_browser_uri;
    use tempfile::tempdir;

    #[test]
    fn normalized_browser_uri_preserves_existing_scheme() {
        assert_eq!(
            normalized_browser_uri("https://example.com/app", None).as_deref(),
            Some("https://example.com/app")
        );
        assert_eq!(
            normalized_browser_uri("about:blank", None).as_deref(),
            Some("about:blank")
        );
    }

    #[test]
    fn normalized_browser_uri_adds_http_for_local_targets() {
        assert_eq!(
            normalized_browser_uri("localhost:3000/dashboard", None).as_deref(),
            Some("http://localhost:3000/dashboard")
        );
        assert_eq!(
            normalized_browser_uri("127.0.0.1:8080", None).as_deref(),
            Some("http://127.0.0.1:8080")
        );
    }

    #[test]
    fn normalized_browser_uri_adds_https_for_domains() {
        assert_eq!(
            normalized_browser_uri("example.com/docs", None).as_deref(),
            Some("https://example.com/docs")
        );
    }

    #[test]
    fn normalized_browser_uri_resolves_workspace_relative_files() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("preview.html");
        std::fs::write(&file, "<html></html>").unwrap();

        let uri = normalized_browser_uri("preview.html", dir.path().to_str()).unwrap();

        assert!(uri.starts_with("file://"));
        assert!(uri.contains("preview.html"));
    }
}
