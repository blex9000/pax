use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

#[cfg(target_os = "linux")]
use webkit6::prelude::*;

use super::PanelBackend;

#[derive(Debug)]
pub struct BrowserPanel {
    widget: gtk4::Widget,
    focus_widget: gtk4::Widget,
    current_uri: Rc<RefCell<Option<String>>>,
}

impl BrowserPanel {
    pub fn new(url: &str, workspace_dir: Option<&str>) -> Self {
        let current_uri = Rc::new(RefCell::new(None));
        let workspace_dir = workspace_dir.map(str::to_string);
        let initial_uri =
            normalized_browser_uri(url, workspace_dir.as_deref()).unwrap_or_else(|| "about:blank".to_string());

        #[cfg(target_os = "linux")]
        let (widget, focus_widget) =
            build_embedded_browser_panel(&initial_uri, workspace_dir.as_deref(), &current_uri);

        #[cfg(not(target_os = "linux"))]
        let (widget, focus_widget) = build_native_browser_launcher_panel(
            &initial_uri,
            workspace_dir.as_deref(),
            &current_uri,
        );

        Self {
            widget,
            focus_widget,
            current_uri,
        }
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
        self.focus_widget.grab_focus();
    }

    fn get_text_content(&self) -> Option<String> {
        self.current_uri.borrow().clone()
    }
}

#[cfg(target_os = "linux")]
fn build_embedded_browser_panel(
    initial_uri: &str,
    workspace_dir: Option<&str>,
    current_uri: &Rc<RefCell<Option<String>>>,
) -> (gtk4::Widget, gtk4::Widget) {
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let toolbar = build_browser_toolbar();
    let back_btn = toolbar.back_btn.clone();
    let forward_btn = toolbar.forward_btn.clone();
    let reload_btn = toolbar.reload_btn.clone();
    let address_entry = toolbar.address_entry.clone();
    let progress = toolbar.progress.clone();
    let status_label = toolbar.status_label.clone();

    container.append(&toolbar.toolbar);
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

    let is_loading = Rc::new(Cell::new(false));

    address_entry.set_text(initial_uri);
    web_view.load_uri(initial_uri);
    *current_uri.borrow_mut() = Some(initial_uri.to_string());

    {
        let web_view = web_view.clone();
        let status_label = status_label.clone();
        let workspace_dir = workspace_dir.map(str::to_string);
        address_entry.connect_activate(move |entry| {
            let Some(uri) = normalized_browser_uri(entry.text().as_str(), workspace_dir.as_deref()) else {
                status_label.set_text("Invalid URL or file path");
                return;
            };
            status_label.set_text("");
            entry.set_text(&uri);
            web_view.load_uri(&uri);
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
        let current_uri = current_uri.clone();
        web_view.connect_uri_notify(move |view| {
            let uri = view.uri().map(|value| value.to_string());
            address_entry.set_text(uri.as_deref().unwrap_or(""));
            *current_uri.borrow_mut() = uri;
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

    (
        container.upcast::<gtk4::Widget>(),
        web_view.upcast::<gtk4::Widget>(),
    )
}

#[cfg(not(target_os = "linux"))]
fn build_native_browser_launcher_panel(
    initial_uri: &str,
    workspace_dir: Option<&str>,
    current_uri: &Rc<RefCell<Option<String>>>,
) -> (gtk4::Widget, gtk4::Widget) {
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let toolbar = build_browser_toolbar();
    let back_btn = toolbar.back_btn.clone();
    let forward_btn = toolbar.forward_btn.clone();
    let reload_btn = toolbar.reload_btn.clone();
    let address_entry = toolbar.address_entry.clone();
    let status_label = toolbar.status_label.clone();

    let open_btn = gtk4::Button::new();
    open_btn.set_icon_name("document-open-symbolic");
    open_btn.add_css_class("flat");
    open_btn.set_tooltip_text(Some("Open in default browser"));
    toolbar.toolbar.append(&open_btn);

    container.append(&toolbar.toolbar);
    container.append(&toolbar.progress);

    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    body.set_hexpand(true);
    body.set_vexpand(true);
    body.set_halign(gtk4::Align::Center);
    body.set_valign(gtk4::Align::Center);
    body.set_margin_top(24);
    body.set_margin_bottom(24);
    body.set_margin_start(24);
    body.set_margin_end(24);

    let icon = gtk4::Image::from_icon_name("web-browser-symbolic");
    icon.set_pixel_size(64);
    body.append(&icon);

    let title = gtk4::Label::new(Some("Native Browser on macOS"));
    title.add_css_class("title-3");
    body.append(&title);

    let subtitle = gtk4::Label::new(Some(
        "Pages open in your default browser. This keeps macOS on the native WebKit stack instead of embedding WebKitGTK inside GTK.",
    ));
    subtitle.add_css_class("dim-label");
    subtitle.set_wrap(true);
    subtitle.set_max_width_chars(52);
    subtitle.set_justify(gtk4::Justification::Center);
    body.append(&subtitle);

    let current_label = gtk4::Label::new(None);
    current_label.add_css_class("caption");
    current_label.add_css_class("dim-label");
    current_label.set_wrap(true);
    current_label.set_max_width_chars(56);
    current_label.set_justify(gtk4::Justification::Center);
    body.append(&current_label);

    let launch_btn = gtk4::Button::with_label("Open Current Page");
    launch_btn.add_css_class("suggested-action");
    body.append(&launch_btn);

    container.append(&body);
    container.append(&status_label);

    let history = Rc::new(RefCell::new(BrowserHistory::default()));

    let update_buttons = {
        let history = history.clone();
        let back_btn = back_btn.clone();
        let forward_btn = forward_btn.clone();
        move || {
            let history = history.borrow();
            back_btn.set_sensitive(history.can_go_back());
            forward_btn.set_sensitive(history.can_go_forward());
        }
    };

    let set_current_uri = {
        let current_uri = current_uri.clone();
        let current_label = current_label.clone();
        let address_entry = address_entry.clone();
        move |uri: &str| {
            address_entry.set_text(uri);
            current_label.set_text(uri);
            *current_uri.borrow_mut() = Some(uri.to_string());
        }
    };

    let open_current = {
        let current_uri = current_uri.clone();
        let status_label = status_label.clone();
        move || {
            if let Some(uri) = current_uri.borrow().clone() {
                launch_in_default_browser(&uri, &status_label);
            } else {
                status_label.set_text("Enter a URL or file path first");
            }
        }
    };

    {
        let history = history.clone();
        let status_label = status_label.clone();
        let update_buttons = update_buttons.clone();
        let set_current_uri = set_current_uri.clone();
        let workspace_dir = workspace_dir.map(str::to_string);
        address_entry.connect_activate(move |entry| {
            let Some(uri) = normalized_browser_uri(entry.text().as_str(), workspace_dir.as_deref()) else {
                status_label.set_text("Invalid URL or file path");
                return;
            };
            history.borrow_mut().visit(uri.clone());
            set_current_uri(&uri);
            update_buttons();
            launch_in_default_browser(&uri, &status_label);
        });
    }

    {
        let open_current = open_current.clone();
        open_btn.connect_clicked(move |_| open_current());
    }

    {
        let open_current = open_current.clone();
        launch_btn.connect_clicked(move |_| open_current());
    }

    {
        let open_current = open_current.clone();
        reload_btn.connect_clicked(move |_| open_current());
    }

    {
        let history = history.clone();
        let status_label = status_label.clone();
        let update_buttons = update_buttons.clone();
        let set_current_uri = set_current_uri.clone();
        back_btn.connect_clicked(move |_| {
            if let Some(uri) = history.borrow_mut().go_back() {
                set_current_uri(&uri);
                update_buttons();
                launch_in_default_browser(&uri, &status_label);
            }
        });
    }

    {
        let history = history.clone();
        let status_label = status_label.clone();
        let update_buttons = update_buttons.clone();
        let set_current_uri = set_current_uri.clone();
        forward_btn.connect_clicked(move |_| {
            if let Some(uri) = history.borrow_mut().go_forward() {
                set_current_uri(&uri);
                update_buttons();
                launch_in_default_browser(&uri, &status_label);
            }
        });
    }

    if initial_uri != "about:blank" {
        history.borrow_mut().visit(initial_uri.to_string());
        set_current_uri(initial_uri);
        update_buttons();
        launch_in_default_browser(initial_uri, &status_label);
    } else {
        current_label.set_text("No page loaded");
        status_label.set_text("Use the address bar to open a page in your default browser");
    }

    (
        container.upcast::<gtk4::Widget>(),
        address_entry.upcast::<gtk4::Widget>(),
    )
}

#[cfg(not(target_os = "linux"))]
fn launch_in_default_browser(uri: &str, status_label: &gtk4::Label) {
    status_label.set_text("Opening in default browser...");
    let launcher = gtk4::UriLauncher::new(uri);
    let status_label = status_label.clone();
    let uri_string = uri.to_string();
    launcher.launch(
        None::<&gtk4::Window>,
        None::<&gtk4::gio::Cancellable>,
        move |result| match result {
            Ok(()) => status_label.set_text(&format!("Opened {}", uri_string)),
            Err(error) => status_label.set_text(&format!("Open failed: {}", error)),
        },
    );
}

fn build_browser_toolbar() -> BrowserToolbar {
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

    BrowserToolbar {
        toolbar,
        back_btn,
        forward_btn,
        reload_btn,
        address_entry,
        progress,
        status_label,
    }
}

#[derive(Debug)]
struct BrowserToolbar {
    toolbar: gtk4::Box,
    back_btn: gtk4::Button,
    forward_btn: gtk4::Button,
    reload_btn: gtk4::Button,
    address_entry: gtk4::Entry,
    progress: gtk4::ProgressBar,
    status_label: gtk4::Label,
}

#[cfg(any(test, not(target_os = "linux")))]
#[derive(Debug, Default, Clone)]
struct BrowserHistory {
    entries: Vec<String>,
    current_index: Option<usize>,
}

#[cfg(any(test, not(target_os = "linux")))]
impl BrowserHistory {
    fn can_go_back(&self) -> bool {
        matches!(self.current_index, Some(index) if index > 0)
    }

    fn can_go_forward(&self) -> bool {
        matches!(self.current_index, Some(index) if index + 1 < self.entries.len())
    }

    fn visit(&mut self, uri: String) {
        if self.current().is_some_and(|current| current == uri) {
            return;
        }

        if let Some(index) = self.current_index {
            self.entries.truncate(index + 1);
        }

        self.entries.push(uri);
        self.current_index = self.entries.len().checked_sub(1);
    }

    fn go_back(&mut self) -> Option<String> {
        let index = self.current_index?;
        let next = index.checked_sub(1)?;
        self.current_index = Some(next);
        self.current().map(ToString::to_string)
    }

    fn go_forward(&mut self) -> Option<String> {
        let index = self.current_index?;
        let next = index + 1;
        if next >= self.entries.len() {
            return None;
        }
        self.current_index = Some(next);
        self.current().map(ToString::to_string)
    }

    fn current(&self) -> Option<&str> {
        self.current_index
            .and_then(|index| self.entries.get(index))
            .map(String::as_str)
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
    use super::{normalized_browser_uri, BrowserHistory};
    use tempfile::tempdir;

    #[test]
    fn browser_history_truncates_forward_entries_on_new_visit() {
        let mut history = BrowserHistory::default();
        history.visit("https://one.test".to_string());
        history.visit("https://two.test".to_string());
        assert!(history.can_go_back());
        assert_eq!(history.go_back().as_deref(), Some("https://one.test"));
        assert!(history.can_go_forward());
        assert_eq!(history.go_forward().as_deref(), Some("https://two.test"));
        assert_eq!(history.go_back().as_deref(), Some("https://one.test"));

        history.visit("https://three.test".to_string());

        assert!(!history.can_go_forward());
        assert_eq!(history.current(), Some("https://three.test"));
    }

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
