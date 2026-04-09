use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

#[cfg(target_os = "macos")]
use gtk4::glib;
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "linux")]
use webkit6::prelude::*;
#[cfg(target_os = "macos")]
use {
    gdk4_macos::MacosSurface,
    objc2::{rc::Retained, sel, MainThreadMarker},
    objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindow},
    objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSURLRequest, NSURL},
    objc2_web_kit::WKWebView,
};

use super::PanelBackend;

pub struct BrowserPanel {
    widget: gtk4::Widget,
    focus_widget: gtk4::Widget,
    current_uri: Rc<RefCell<Option<String>>>,
    #[cfg(target_os = "macos")]
    _native_bridge: MacEmbeddedBrowser,
}

impl std::fmt::Debug for BrowserPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserPanel").finish()
    }
}

impl BrowserPanel {
    pub fn new(url: &str, workspace_dir: Option<&str>) -> Self {
        let current_uri = Rc::new(RefCell::new(None));
        let workspace_dir = workspace_dir.map(str::to_string);
        let initial_uri = normalized_browser_uri(url, workspace_dir.as_deref())
            .unwrap_or_else(|| "about:blank".to_string());

        #[cfg(target_os = "linux")]
        let (widget, focus_widget) =
            build_embedded_browser_panel(&initial_uri, workspace_dir.as_deref(), &current_uri);

        #[cfg(target_os = "macos")]
        let (widget, focus_widget, native_bridge) = build_macos_embedded_browser_panel(
            &initial_uri,
            workspace_dir.as_deref(),
            &current_uri,
        );

        #[cfg(all(not(target_os = "linux"), not(target_os = "macos")))]
        let (widget, focus_widget) = build_native_browser_launcher_panel(
            &initial_uri,
            workspace_dir.as_deref(),
            &current_uri,
        );

        Self {
            widget,
            focus_widget,
            current_uri,
            #[cfg(target_os = "macos")]
            _native_bridge: native_bridge,
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
    let devtools_btn = toolbar.devtools_btn.clone();
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
    container.append(&web_view);
    container.append(&status_label);

    setup_linux_browser_context_menu(&web_view, &status_label);

    let is_loading = Rc::new(Cell::new(false));

    address_entry.set_text(initial_uri);
    load_linux_browser_uri(&web_view, initial_uri, &status_label);
    *current_uri.borrow_mut() = Some(initial_uri.to_string());

    {
        let web_view = web_view.clone();
        let status_label = status_label.clone();
        let workspace_dir = workspace_dir.map(str::to_string);
        address_entry.connect_activate(move |entry| {
            let Some(uri) = normalized_browser_uri(entry.text().as_str(), workspace_dir.as_deref())
            else {
                status_label.set_text("Invalid URL or file path");
                return;
            };
            status_label.set_text("");
            entry.set_text(&uri);
            load_linux_browser_uri(&web_view, &uri, &status_label);
        });
    }

    {
        let web_view = web_view.clone();
        let status_label = status_label.clone();
        devtools_btn.connect_clicked(move |_| {
            open_linux_browser_devtools(&web_view, &status_label);
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

#[cfg(target_os = "linux")]
fn load_linux_browser_uri(web_view: &webkit6::WebView, uri: &str, status_label: &gtk4::Label) {
    if uri == "about:blank" {
        web_view.load_html(
            &browser_placeholder_html(),
            Some("https://pax.local/browser"),
        );
        status_label.set_text("Browser panel ready");
        return;
    }

    web_view.load_uri(uri);
}

#[cfg(target_os = "linux")]
fn setup_linux_browser_context_menu(web_view: &webkit6::WebView, status_label: &gtk4::Label) {
    let status_label = status_label.clone();
    web_view.connect_context_menu(move |view, context_menu, hit_test| {
        let popover = gtk4::Popover::new();
        crate::theme::configure_popover(&popover);
        popover.set_has_arrow(false);
        popover.set_autohide(true);

        let menu_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        menu_box.set_margin_top(4);
        menu_box.set_margin_bottom(4);
        menu_box.set_margin_start(4);
        menu_box.set_margin_end(4);

        append_browser_context_action(
            &menu_box,
            "go-previous-symbolic",
            "Back",
            view.can_go_back(),
            {
                let view = view.clone();
                let popover = popover.clone();
                move || {
                    view.go_back();
                    popover.popdown();
                }
            },
        );
        append_browser_context_action(
            &menu_box,
            "go-next-symbolic",
            "Forward",
            view.can_go_forward(),
            {
                let view = view.clone();
                let popover = popover.clone();
                move || {
                    view.go_forward();
                    popover.popdown();
                }
            },
        );
        append_browser_context_action(&menu_box, "view-refresh-symbolic", "Reload", true, {
            let view = view.clone();
            let popover = popover.clone();
            move || {
                view.reload();
                popover.popdown();
            }
        });
        append_browser_context_action(
            &menu_box,
            "applications-development-symbolic",
            "Developer Tools",
            true,
            {
                let view = view.clone();
                let status_label = status_label.clone();
                let popover = popover.clone();
                move || {
                    open_linux_browser_devtools(&view, &status_label);
                    popover.popdown();
                }
            },
        );

        let mut has_dynamic_items = false;

        if hit_test.context_is_link() {
            has_dynamic_items = true;
            append_browser_context_separator(&menu_box);
            if let Some(link_uri) = hit_test.link_uri().map(|value| value.to_string()) {
                append_browser_context_action(
                    &menu_box,
                    "document-open-symbolic",
                    "Open Link Externally",
                    true,
                    {
                        let status_label = status_label.clone();
                        let popover = popover.clone();
                        let link_uri = link_uri.clone();
                        move || {
                            launch_uri(&link_uri, &status_label);
                            popover.popdown();
                        }
                    },
                );
                append_browser_context_action(
                    &menu_box,
                    "edit-copy-symbolic",
                    "Copy Link",
                    true,
                    {
                        let popover = popover.clone();
                        move || {
                            if let Some(display) = gtk4::gdk::Display::default() {
                                display.clipboard().set_text(&link_uri);
                            }
                            popover.popdown();
                        }
                    },
                );
            }
        }

        if hit_test.context_is_editable() || hit_test.context_is_selection() {
            if !has_dynamic_items {
                append_browser_context_separator(&menu_box);
            }
            append_browser_context_action(&menu_box, "edit-copy-symbolic", "Copy", true, {
                let view = view.clone();
                let popover = popover.clone();
                move || {
                    view.execute_editing_command("Copy");
                    popover.popdown();
                }
            });
            append_browser_context_action(
                &menu_box,
                "edit-cut-symbolic",
                "Cut",
                hit_test.context_is_editable(),
                {
                    let view = view.clone();
                    let popover = popover.clone();
                    move || {
                        view.execute_editing_command("Cut");
                        popover.popdown();
                    }
                },
            );
            append_browser_context_action(
                &menu_box,
                "edit-paste-symbolic",
                "Paste",
                hit_test.context_is_editable(),
                {
                    let view = view.clone();
                    let popover = popover.clone();
                    move || {
                        view.execute_editing_command("Paste");
                        popover.popdown();
                    }
                },
            );
            append_browser_context_action(
                &menu_box,
                "edit-select-all-symbolic",
                "Select All",
                true,
                {
                    let view = view.clone();
                    let popover = popover.clone();
                    move || {
                        view.execute_editing_command("SelectAll");
                        popover.popdown();
                    }
                },
            );
        }

        popover.set_child(Some(&menu_box));
        popover.set_parent(view);
        if let Some((x, y)) = context_menu.event().and_then(|event| event.position()) {
            popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        }
        popover.connect_closed(|popover| {
            popover.unparent();
        });
        popover.popup();
        true
    });
}

#[cfg(target_os = "linux")]
fn append_browser_context_action<F>(
    menu_box: &gtk4::Box,
    icon_name: &str,
    label: &str,
    sensitive: bool,
    on_activate: F,
) where
    F: Fn() + 'static,
{
    let button = gtk4::Button::new();
    button.add_css_class("flat");
    button.add_css_class("app-popover-button");
    button.set_sensitive(sensitive);

    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let icon = gtk4::Image::from_icon_name(icon_name);
    row.append(&icon);

    let text = gtk4::Label::new(Some(label));
    text.set_hexpand(true);
    text.set_halign(gtk4::Align::Start);
    row.append(&text);

    button.set_child(Some(&row));
    button.connect_clicked(move |_| on_activate());
    menu_box.append(&button);
}

#[cfg(target_os = "linux")]
fn append_browser_context_separator(menu_box: &gtk4::Box) {
    menu_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
}

#[cfg(target_os = "linux")]
fn open_linux_browser_devtools(web_view: &webkit6::WebView, status_label: &gtk4::Label) {
    if let Some(settings) = webkit6::prelude::WebViewExt::settings(web_view) {
        settings.set_enable_developer_extras(true);
    }
    if let Some(inspector) = web_view.inspector() {
        inspector.show();
        status_label.set_text("Developer Tools opened");
    } else {
        status_label.set_text("Developer Tools unavailable");
    }
}

#[cfg(target_os = "linux")]
fn launch_uri(uri: &str, status_label: &gtk4::Label) {
    let launcher = gtk4::UriLauncher::new(uri);
    let status_label = status_label.clone();
    let uri = uri.to_string();
    launcher.launch(
        None::<&gtk4::Window>,
        None::<&gtk4::gio::Cancellable>,
        move |result| match result {
            Ok(()) => status_label.set_text(&format!("Opened {}", uri)),
            Err(error) => status_label.set_text(&format!("Open failed: {}", error)),
        },
    );
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct MacEmbeddedBrowser {
    state: Rc<MacWebViewState>,
    sync_source: Option<glib::SourceId>,
}

#[cfg(target_os = "macos")]
impl Drop for MacEmbeddedBrowser {
    fn drop(&mut self) {
        if let Some(source) = self.sync_source.take() {
            source.remove();
        }
        self.state.detach();
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct MacWebViewState {
    web_view: Retained<WKWebView>,
    attached_host_ptr: Cell<usize>,
}

#[cfg(target_os = "macos")]
impl MacWebViewState {
    fn detach(&self) {
        unsafe {
            self.web_view.setHidden(true);
            self.web_view.removeFromSuperview();
        }
        self.attached_host_ptr.set(0);
    }
}

#[cfg(target_os = "macos")]
fn build_macos_embedded_browser_panel(
    initial_uri: &str,
    workspace_dir: Option<&str>,
    current_uri: &Rc<RefCell<Option<String>>>,
) -> (gtk4::Widget, gtk4::Widget, MacEmbeddedBrowser) {
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let toolbar = build_browser_toolbar();
    let back_btn = toolbar.back_btn.clone();
    let devtools_btn = toolbar.devtools_btn.clone();
    let forward_btn = toolbar.forward_btn.clone();
    let reload_btn = toolbar.reload_btn.clone();
    let address_entry = toolbar.address_entry.clone();
    let progress = toolbar.progress.clone();
    let status_label = toolbar.status_label.clone();

    container.append(&toolbar.toolbar);
    container.append(&progress);

    let placeholder = gtk4::Frame::new(None);
    placeholder.set_hexpand(true);
    placeholder.set_vexpand(true);
    placeholder.add_css_class("browser-panel-host");

    let loading_label = gtk4::Label::new(Some("Loading WebKit view..."));
    loading_label.add_css_class("dim-label");
    loading_label.set_halign(gtk4::Align::Center);
    loading_label.set_valign(gtk4::Align::Center);
    placeholder.set_child(Some(&loading_label));

    container.append(&placeholder);
    container.append(&status_label);

    let mtm = MainThreadMarker::new().expect("WKWebView must be created on the main thread");
    let web_view = unsafe { WKWebView::new(mtm) };
    unsafe {
        web_view.setHidden(true);
        web_view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
    }

    let state = Rc::new(MacWebViewState {
        web_view,
        attached_host_ptr: Cell::new(0),
    });

    let inspectable = configure_macos_browser_inspection(&state.web_view);
    devtools_btn.set_sensitive(inspectable);
    devtools_btn.set_tooltip_text(Some(if inspectable {
        "Open Web Inspector via Control-click > Inspect Element or Safari > Develop"
    } else {
        "Web Inspector requires macOS 13.3 or newer"
    }));

    let navigate_to = {
        let state = state.clone();
        let status_label = status_label.clone();
        let current_uri = current_uri.clone();
        move |uri: String| {
            *current_uri.borrow_mut() = Some(uri.clone());
            load_wkwebview_uri(&state.web_view, &uri, &status_label);
        }
    };

    address_entry.set_text(initial_uri);
    navigate_to(initial_uri.to_string());

    {
        let workspace_dir = workspace_dir.map(str::to_string);
        let status_label = status_label.clone();
        let address_entry = address_entry.clone();
        let navigate_to = navigate_to.clone();
        address_entry.connect_activate(move |entry| {
            let Some(uri) = normalized_browser_uri(entry.text().as_str(), workspace_dir.as_deref())
            else {
                status_label.set_text("Invalid URL or file path");
                return;
            };
            entry.set_text(&uri);
            navigate_to(uri);
        });
    }

    {
        let status_label = status_label.clone();
        let inspectable = inspectable;
        devtools_btn.connect_clicked(move |_| {
            if inspectable {
                show_macos_browser_devtools_help(&status_label);
            } else {
                status_label.set_text("Web Inspector requires macOS 13.3 or newer");
            }
        });
    }

    {
        let state = state.clone();
        back_btn.connect_clicked(move |_| unsafe {
            state.web_view.goBack();
        });
    }

    {
        let state = state.clone();
        forward_btn.connect_clicked(move |_| unsafe {
            state.web_view.goForward();
        });
    }

    {
        let state = state.clone();
        reload_btn.connect_clicked(move |_| unsafe {
            if state.web_view.isLoading() {
                state.web_view.stopLoading();
            } else {
                state.web_view.reload();
            }
        });
    }

    let sync_source = {
        let state = state.clone();
        let placeholder = placeholder.clone();
        let address_entry = address_entry.clone();
        let back_btn = back_btn.clone();
        let forward_btn = forward_btn.clone();
        let reload_btn = reload_btn.clone();
        let progress = progress.clone();
        let status_label = status_label.clone();
        let current_uri = current_uri.clone();

        Some(glib::timeout_add_local(
            Duration::from_millis(60),
            move || {
                sync_wkwebview_host(&state, &placeholder);
                poll_wkwebview_state(
                    &state.web_view,
                    &address_entry,
                    &back_btn,
                    &forward_btn,
                    &reload_btn,
                    &progress,
                    &status_label,
                    &current_uri,
                );
                glib::ControlFlow::Continue
            },
        ))
    };

    let native_bridge = MacEmbeddedBrowser { state, sync_source };

    (
        container.upcast::<gtk4::Widget>(),
        address_entry.upcast::<gtk4::Widget>(),
        native_bridge,
    )
}

#[cfg(target_os = "macos")]
fn sync_wkwebview_host(state: &MacWebViewState, placeholder: &gtk4::Frame) {
    if !placeholder.is_drawable() || !placeholder.is_visible() {
        unsafe {
            state.web_view.setHidden(true);
        }
        return;
    }

    let Some(window_widget) = placeholder.ancestor(gtk4::Window::static_type()) else {
        state.detach();
        return;
    };
    let Ok(window) = window_widget.downcast::<gtk4::Window>() else {
        state.detach();
        return;
    };
    let Some(surface) = window.surface() else {
        state.detach();
        return;
    };
    let Ok(macos_surface) = surface.downcast::<MacosSurface>() else {
        state.detach();
        return;
    };
    let Some(ns_window) = (unsafe { Retained::<NSWindow>::retain(macos_surface.native().cast()) })
    else {
        state.detach();
        return;
    };
    let Some(content_view) = (unsafe { ns_window.contentView() }) else {
        state.detach();
        return;
    };

    let window_widget: gtk4::Widget = window.upcast();
    let Some(bounds) = placeholder.compute_bounds(&window_widget) else {
        unsafe {
            state.web_view.setHidden(true);
        }
        return;
    };

    let width = f64::from(bounds.width()).round().max(0.0);
    let height = f64::from(bounds.height()).round().max(0.0);
    if width <= 1.0 || height <= 1.0 {
        unsafe {
            state.web_view.setHidden(true);
        }
        return;
    }

    let host_ptr = (&*content_view as *const NSView) as usize;
    if state.attached_host_ptr.get() != host_ptr {
        state.detach();
        unsafe {
            content_view.addSubview(&state.web_view);
        }
        state.attached_host_ptr.set(host_ptr);
    }

    let origin_x = f64::from(bounds.x()).round();
    let content_frame = unsafe { content_view.frame() };
    let content_height = content_frame.size.height;
    let origin_y = cocoa_view_origin_y(f64::from(bounds.y()), height, content_height, unsafe {
        content_view.isFlipped()
    });
    let frame = NSRect::new(NSPoint::new(origin_x, origin_y), NSSize::new(width, height));

    unsafe {
        state.web_view.setFrame(frame);
        state.web_view.setHidden(false);
    }
}

#[cfg(any(test, target_os = "macos"))]
fn cocoa_view_origin_y(y: f64, height: f64, host_height: f64, is_flipped: bool) -> f64 {
    if is_flipped {
        y.round().max(0.0)
    } else {
        (host_height - y - height).round().max(0.0)
    }
}

#[cfg(target_os = "macos")]
fn poll_wkwebview_state(
    web_view: &WKWebView,
    address_entry: &gtk4::Entry,
    back_btn: &gtk4::Button,
    forward_btn: &gtk4::Button,
    reload_btn: &gtk4::Button,
    progress: &gtk4::ProgressBar,
    status_label: &gtk4::Label,
    current_uri: &Rc<RefCell<Option<String>>>,
) {
    let is_loading = unsafe { web_view.isLoading() };
    back_btn.set_sensitive(unsafe { web_view.canGoBack() });
    forward_btn.set_sensitive(unsafe { web_view.canGoForward() });
    reload_btn.set_icon_name(if is_loading {
        "process-stop-symbolic"
    } else {
        "view-refresh-symbolic"
    });

    let progress_value = unsafe { web_view.estimatedProgress() }.clamp(0.0, 1.0);
    progress.set_fraction(progress_value);
    progress.set_visible(is_loading);

    let uri = unsafe { web_view.URL() }
        .and_then(|url| unsafe { url.absoluteString() })
        .map(|value| value.to_string());
    if let Some(uri) = uri {
        *current_uri.borrow_mut() = Some(uri.clone());
        if !address_entry.has_focus() && address_entry.text().as_str() != uri {
            address_entry.set_text(&uri);
        }
    }

    if is_loading {
        status_label.set_text("Loading...");
        return;
    }

    let title = unsafe { web_view.title() }.map(|value| value.to_string());
    status_label.set_text(title.as_deref().unwrap_or(""));
}

#[cfg(target_os = "macos")]
fn load_wkwebview_uri(web_view: &WKWebView, uri: &str, status_label: &gtk4::Label) {
    if uri == "about:blank" {
        let html =
            NSString::from_str("<html><body style=\"background: transparent\"></body></html>");
        unsafe {
            web_view.loadHTMLString_baseURL(&html, None);
        }
        status_label.set_text("");
        return;
    }

    if let Some(path) = file_path_from_browser_uri(uri) {
        let load_path = path.canonicalize().unwrap_or(path);
        let read_access = if load_path.is_dir() {
            load_path.clone()
        } else {
            load_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| load_path.clone())
        };

        let file_url = ns_file_url(&load_path);
        let read_access_url = ns_file_url(&read_access);
        unsafe {
            web_view.loadFileURL_allowingReadAccessToURL(&file_url, &read_access_url);
        }
        status_label.set_text("Loading...");
        return;
    }

    let uri_string = NSString::from_str(uri);
    let Some(url) = (unsafe { NSURL::URLWithString(&uri_string) }) else {
        status_label.set_text("Invalid URL");
        return;
    };
    let request = NSURLRequest::requestWithURL(&url);
    unsafe {
        web_view.loadRequest(&request);
    }
    status_label.set_text("Loading...");
}

#[cfg(target_os = "macos")]
fn configure_macos_browser_inspection(web_view: &WKWebView) -> bool {
    if web_view.class().responds_to(sel!(setInspectable:)) {
        unsafe {
            web_view.setInspectable(true);
        }
        true
    } else {
        false
    }
}

#[cfg(target_os = "macos")]
fn show_macos_browser_devtools_help(status_label: &gtk4::Label) {
    status_label.set_text(macos_browser_devtools_help_message());
}

#[cfg(target_os = "macos")]
fn macos_browser_devtools_help_message() -> &'static str {
    "Web Inspector enabled. Use Control-click > Inspect Element or Safari > Develop for this Mac."
}

#[cfg(target_os = "macos")]
fn file_path_from_browser_uri(uri: &str) -> Option<PathBuf> {
    uri.starts_with("file://")
        .then(|| gtk4::gio::File::for_uri(uri).path())
        .flatten()
}

#[cfg(target_os = "macos")]
fn ns_file_url(path: &Path) -> Retained<NSURL> {
    let path_string = path.to_string_lossy();
    let ns_path = NSString::from_str(&path_string);
    NSURL::fileURLWithPath(&ns_path)
}

#[cfg(all(not(target_os = "linux"), not(target_os = "macos")))]
fn build_native_browser_launcher_panel(
    initial_uri: &str,
    workspace_dir: Option<&str>,
    current_uri: &Rc<RefCell<Option<String>>>,
) -> (gtk4::Widget, gtk4::Widget) {
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let toolbar = build_browser_toolbar();
    let back_btn = toolbar.back_btn.clone();
    let devtools_btn = toolbar.devtools_btn.clone();
    let forward_btn = toolbar.forward_btn.clone();
    let reload_btn = toolbar.reload_btn.clone();
    let address_entry = toolbar.address_entry.clone();
    let status_label = toolbar.status_label.clone();

    devtools_btn.set_sensitive(false);
    devtools_btn.set_tooltip_text(Some(
        "Developer Tools are only available on the embedded Linux browser backend",
    ));

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
            let Some(uri) = normalized_browser_uri(entry.text().as_str(), workspace_dir.as_deref())
            else {
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

#[cfg(all(not(target_os = "linux"), not(target_os = "macos")))]
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

    let devtools_btn = gtk4::Button::new();
    devtools_btn.set_icon_name("applications-development-symbolic");
    devtools_btn.add_css_class("flat");
    devtools_btn.set_tooltip_text(Some("Open Developer Tools"));

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
    toolbar.append(&devtools_btn);
    toolbar.append(&reload_btn);
    toolbar.append(&address_entry);

    BrowserToolbar {
        toolbar,
        back_btn,
        forward_btn,
        devtools_btn,
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
    devtools_btn: gtk4::Button,
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

fn browser_placeholder_html() -> String {
    r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="color-scheme" content="dark light">
  <style>
    :root {
      color-scheme: dark;
      --bg: #2e3440;
      --card: #3b4252;
      --fg: #eceff4;
      --muted: #d8dee9;
      --accent: #88c0d0;
      font-family: Inter, system-ui, sans-serif;
    }
    body {
      margin: 0;
      min-height: 100vh;
      display: grid;
      place-items: center;
      background: var(--bg);
      color: var(--fg);
    }
    .card {
      max-width: 38rem;
      margin: 2rem;
      padding: 1.5rem 1.75rem;
      border-radius: 14px;
      background: var(--card);
      box-shadow: 0 18px 48px rgba(0, 0, 0, 0.2);
    }
    h1 {
      margin: 0 0 0.5rem;
      font-size: 1.35rem;
    }
    p {
      margin: 0.5rem 0;
      color: var(--muted);
      line-height: 1.5;
    }
    code {
      color: var(--accent);
      font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    }
  </style>
</head>
<body>
  <section class="card">
    <h1>Browser panel ready</h1>
    <p>Insert a URL like <code>https://example.com</code> or a local target such as <code>localhost:3000</code>.</p>
    <p>Relative HTML files inside the workspace are supported too.</p>
  </section>
</body>
</html>"#
        .to_string()
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
    #[cfg(target_os = "macos")]
    use super::macos_browser_devtools_help_message;
    use super::{
        browser_placeholder_html, cocoa_view_origin_y, normalized_browser_uri, BrowserHistory,
    };
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

    #[test]
    fn browser_placeholder_is_visibly_branded() {
        let html = browser_placeholder_html();
        assert!(html.contains("Browser panel ready"));
        assert!(html.contains("localhost:3000"));
    }

    #[test]
    fn cocoa_view_origin_y_handles_flipped_and_unflipped_hosts() {
        assert_eq!(cocoa_view_origin_y(48.0, 120.0, 800.0, false), 632.0);
        assert_eq!(cocoa_view_origin_y(48.0, 120.0, 800.0, true), 48.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_devtools_help_message_mentions_inspect_element() {
        let message = macos_browser_devtools_help_message();
        assert!(message.contains("Inspect Element"));
        assert!(message.contains("Safari > Develop"));
    }
}
