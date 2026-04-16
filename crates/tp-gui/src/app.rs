use adw::prelude::*;
use anyhow::Result;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use pax_core::workspace::Workspace;

use crate::actions::{self, DIRTY_INDICATOR, HEADER_WS_LABEL};
use crate::panel_host::PanelAction;
use crate::theme::Theme;
use crate::widgets::status_bar::StatusBar;
use crate::workspace_view::WorkspaceView;

fn panel_type_uses_text_editing_shortcuts(panel_type: &pax_core::workspace::PanelType) -> bool {
    matches!(
        panel_type,
        pax_core::workspace::PanelType::CodeEditor { .. }
            | pax_core::workspace::PanelType::Markdown { .. }
    )
}

fn focused_panel_uses_text_editing_shortcuts(view: &WorkspaceView) -> bool {
    view.focused_panel_id()
        .and_then(|id| view.workspace().panel(id))
        .map(|panel| panel_type_uses_text_editing_shortcuts(&panel.effective_type()))
        .unwrap_or(false)
}

fn focused_panel_is_code_editor(view: &WorkspaceView) -> bool {
    view.focused_panel_id()
        .and_then(|id| view.workspace().panel(id))
        .map(|panel| {
            matches!(
                panel.effective_type(),
                pax_core::workspace::PanelType::CodeEditor { .. }
            )
        })
        .unwrap_or(false)
}

#[derive(Clone, Copy)]
struct AppMenuItemSpec {
    label: &'static str,
    action: &'static str,
    icon: &'static str,
    tooltip: &'static str,
}

const APP_MENU_FILE_ITEMS: &[AppMenuItemSpec] = &[
    AppMenuItemSpec {
        label: "New Workspace",
        action: "app.new",
        icon: "document-new-symbolic",
        tooltip: "Create a new workspace",
    },
    AppMenuItemSpec {
        label: "Open Workspace…",
        action: "app.open",
        icon: "document-open-symbolic",
        tooltip: "Open a workspace file",
    },
    AppMenuItemSpec {
        label: "Open Recent…",
        action: "app.recent",
        icon: "document-open-recent-symbolic",
        tooltip: "Open a recent workspace",
    },
];

const APP_MENU_SAVE_ITEM: AppMenuItemSpec = AppMenuItemSpec {
    label: "Save",
    action: "app.save",
    icon: "media-floppy-symbolic",
    tooltip: "Save the current workspace",
};

const APP_MENU_SAVE_SECONDARY_ITEMS: &[AppMenuItemSpec] = &[AppMenuItemSpec {
    label: "Save As…",
    action: "app.save-as",
    icon: "document-save-as-symbolic",
    tooltip: "Save the workspace under a new name",
}];

const APP_MENU_AUTOSAVE_ITEM: AppMenuItemSpec = AppMenuItemSpec {
    label: "Auto-save",
    action: "app.autosave",
    icon: "document-save-symbolic",
    tooltip: "Toggle workspace auto-save",
};

const APP_MENU_SETTINGS_ITEMS: &[AppMenuItemSpec] = &[
    AppMenuItemSpec {
        label: "Settings…",
        action: "app.settings",
        icon: "preferences-system-symbolic",
        tooltip: "Open application settings",
    },
    AppMenuItemSpec {
        label: "Keyboard Shortcuts",
        action: "app.shortcuts",
        icon: "input-keyboard-symbolic",
        tooltip: "Show keyboard shortcuts",
    },
    AppMenuItemSpec {
        label: "About Pax",
        action: "app.about",
        icon: "help-about-symbolic",
        tooltip: "Show application information",
    },
];

const APP_MENU_QUIT_ITEM: AppMenuItemSpec = AppMenuItemSpec {
    label: "Quit",
    action: "app.quit",
    icon: "application-exit-symbolic",
    tooltip: "Close Pax",
};

fn build_app_menu_button(item: AppMenuItemSpec, suffix: Option<gtk4::Widget>) -> gtk4::Button {
    let btn = gtk4::Button::new();
    btn.add_css_class("flat");
    btn.add_css_class("app-popover-button");
    btn.set_tooltip_text(Some(item.tooltip));

    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    content.set_margin_start(4);
    content.set_margin_end(8);

    let icon = gtk4::Image::from_icon_name(item.icon);
    icon.set_pixel_size(16);
    content.append(&icon);

    let label = gtk4::Label::new(Some(item.label));
    label.set_hexpand(true);
    label.set_halign(gtk4::Align::Start);
    content.append(&label);

    if let Some(suffix) = suffix {
        content.append(&suffix);
    }

    btn.set_child(Some(&content));
    btn
}

fn append_app_menu_item(
    container: &gtk4::Box,
    popover: &gtk4::Popover,
    window: &Rc<adw::ApplicationWindow>,
    item: AppMenuItemSpec,
    suffix: Option<gtk4::Widget>,
) -> gtk4::Button {
    let btn = build_app_menu_button(item, suffix);
    let pop = popover.clone();
    let win = window.clone();
    btn.connect_clicked(move |_| {
        pop.popdown();
        let _ = gtk4::prelude::WidgetExt::activate_action(
            win.as_ref(),
            item.action,
            None::<&gtk4::glib::Variant>,
        );
    });
    container.append(&btn);
    btn
}

fn append_app_menu_section(
    container: &gtk4::Box,
    popover: &gtk4::Popover,
    window: &Rc<adw::ApplicationWindow>,
    items: &[AppMenuItemSpec],
) {
    for item in items {
        append_app_menu_item(container, popover, window, *item, None);
    }
}

fn build_app_menu_popover(
    window: &Rc<adw::ApplicationWindow>,
    save_action: &gtk4::gio::SimpleAction,
    autosave_enabled: &Rc<std::cell::Cell<bool>>,
) -> gtk4::Popover {
    let popover = gtk4::Popover::new();
    crate::theme::configure_popover(&popover);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_margin_top(4);
    vbox.set_margin_bottom(4);
    vbox.set_margin_start(4);
    vbox.set_margin_end(4);

    append_app_menu_section(&vbox, &popover, window, APP_MENU_FILE_ITEMS);

    let save_btn = build_app_menu_button(APP_MENU_SAVE_ITEM, None);
    save_btn.set_sensitive(save_action.is_enabled());
    {
        let pop = popover.clone();
        let win = window.clone();
        save_btn.connect_clicked(move |_| {
            pop.popdown();
            let _ = gtk4::prelude::WidgetExt::activate_action(
                win.as_ref(),
                "app.save",
                None::<&gtk4::glib::Variant>,
            );
        });
    }
    {
        let save_btn = save_btn.clone();
        save_action.connect_notify_local(Some("enabled"), move |action, _| {
            save_btn.set_sensitive(action.is_enabled());
        });
    }
    vbox.append(&save_btn);

    append_app_menu_section(&vbox, &popover, window, APP_MENU_SAVE_SECONDARY_ITEMS);

    let autosave_indicator = gtk4::Image::from_icon_name("object-select-symbolic");
    autosave_indicator.add_css_class("dim-label");
    autosave_indicator.set_pixel_size(14);
    autosave_indicator.set_visible(autosave_enabled.get());
    let autosave_btn = append_app_menu_item(
        &vbox,
        &popover,
        window,
        APP_MENU_AUTOSAVE_ITEM,
        Some(autosave_indicator.clone().upcast()),
    );

    {
        let indicator = autosave_indicator.clone();
        let enabled = autosave_enabled.clone();
        autosave_btn.connect_clicked(move |_| {
            indicator.set_visible(enabled.get());
        });
    }

    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    sep.set_margin_top(4);
    sep.set_margin_bottom(4);
    vbox.append(&sep);

    append_app_menu_section(&vbox, &popover, window, APP_MENU_SETTINGS_ITEMS);

    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    sep.set_margin_top(4);
    sep.set_margin_bottom(4);
    vbox.append(&sep);

    append_app_menu_item(&vbox, &popover, window, APP_MENU_QUIT_ITEM, None);

    popover.set_child(Some(&vbox));

    popover
}

/// Single entry point — shows welcome if no workspace, or workspace directly.
pub fn run_app(workspace: Option<Workspace>, config_path: Option<&Path>) -> Result<()> {
    // Register bundled fonts BEFORE the Adwaita/GTK application is created.
    // On macOS, Pango builds its CoreText-backed font map eagerly during GTK
    // initialization; fonts registered after that point are ignored by the
    // already-built font map, which means Pango would fall back to a system
    // font for "JetBrains Mono" / "Inter" even though CoreText has them. The
    // fontconfig path (Linux) is less sensitive to ordering but putting the
    // call here doesn't hurt — it still runs before any widget is created.
    crate::fonts::register_bundled_fonts();

    let app = adw::Application::builder()
        .application_id("com.sinelec.pax")
        .build();

    let ws = workspace;
    let cp = config_path.map(|p| p.to_path_buf());

    app.connect_activate(move |app| {
        load_css();

        // Register custom icons from resources/icons/
        if let Some(display) = gtk4::gdk::Display::default() {
            let icon_theme = gtk4::IconTheme::for_display(&display);
            crate::icons::configure_icon_theme(&icon_theme);
        }

        // Register custom GtkSourceView style schemes
        #[cfg(feature = "sourceview")]
        {
            let manager = sourceview5::StyleSchemeManager::default();
            for path in sourceview_style_search_paths() {
                manager.append_search_path(&path.to_string_lossy());
            }
        }

        // Set default window icon (used by window manager / taskbar)
        gtk4::Window::set_default_icon_name("pax");

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Pax")
            .default_width(1200)
            .default_height(800)
            .icon_name("pax")
            .build();

        let window_rc = Rc::new(window.clone());

        if let Some(ref workspace) = ws {
            // Direct workspace launch
            setup_workspace_ui(&window_rc, workspace.clone(), cp.as_deref());
        } else {
            // Show welcome, then transition to workspace
            setup_welcome_ui(&window_rc);
        }

        window.present();
    });

    app.run_with_args::<String>(&[]);
    Ok(())
}

/// Convenience: launch with welcome screen.
pub fn run_app_welcome() -> Result<()> {
    run_app(None, None)
}

/// Setup the welcome screen in the window.
fn setup_welcome_ui(window: &Rc<adw::ApplicationWindow>) {
    let header = adw::HeaderBar::new();
    header.add_css_class("app-headerbar");
    header.set_show_end_title_buttons(true);
    header.set_show_start_title_buttons(true);

    // Pax icon in center of welcome header
    let title_icon = gtk4::Image::from_icon_name("pax");
    title_icon.set_pixel_size(20);
    header.set_title_widget(Some(&title_icon));

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_css_class("app-toolbar-view");
    toolbar_view.add_top_bar(&header);

    let win = window.clone();
    let welcome = crate::widgets::welcome::build_welcome(Rc::new(move |choice| {
        use crate::widgets::welcome::WelcomeChoice;
        match choice {
            WelcomeChoice::NewWorkspace => {
                let ws = new_workspace_with_preferred_theme("untitled");
                setup_workspace_ui(&win, ws, None);
            }
            WelcomeChoice::OpenFile => {
                let win2 = win.clone();
                let dialog = gtk4::FileDialog::builder()
                    .title("Open Workspace")
                    .modal(true)
                    .build();
                let filter = gtk4::FileFilter::new();
                filter.set_name(Some("JSON files"));
                filter.add_pattern("*.json");
                let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
                filters.append(&filter);
                dialog.set_filters(Some(&filters));

                let win3 = win2.clone();
                dialog.open(
                    Some(win2.as_ref()),
                    gtk4::gio::Cancellable::NONE,
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                if let Ok(ws) = pax_core::config::load_workspace(&path) {
                                    setup_workspace_ui(&win3, ws, Some(&path));
                                }
                            }
                        }
                    },
                );
            }
            WelcomeChoice::OpenRecent(path) => {
                let p = std::path::PathBuf::from(&path);
                if let Ok(ws) = pax_core::config::load_workspace(&p) {
                    setup_workspace_ui(&win, ws, Some(&p));
                }
            }
        }
    }));

    toolbar_view.set_content(Some(&welcome));
    window.set_content(Some(&toolbar_view));
}

/// Build the theme-picker popover attached to the header bar button.
/// Radio check buttons for each theme + a Customize button at the bottom.
fn build_theme_popover(parent_window: &Rc<adw::ApplicationWindow>) -> gtk4::Popover {
    use gtk4::prelude::*;

    let popover = gtk4::Popover::new();
    crate::theme::configure_popover(&popover);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_margin_top(6);
    vbox.set_margin_bottom(6);
    vbox.set_margin_start(6);
    vbox.set_margin_end(6);

    let group_leader = gtk4::CheckButton::new();
    let current = crate::theme::current_theme();

    let checks: Vec<gtk4::CheckButton> = Theme::all()
        .iter()
        .map(|theme| {
            let check = gtk4::CheckButton::with_label(theme.label());
            check.set_group(Some(&group_leader));
            check.add_css_class("app-popover-check");
            if *theme == current {
                check.set_active(true);
            }
            let t = *theme;
            let pop = popover.clone();
            check.connect_toggled(move |c| {
                if c.is_active() {
                    apply_theme(t);
                    save_preferred_theme(t);
                    pop.popdown();
                }
            });
            vbox.append(&check);
            check
        })
        .collect();

    // Refresh radio state every time the popover opens (theme may have been
    // changed via Settings dialog).
    let checks_for_show = checks.clone();
    popover.connect_show(move |_| {
        let active = crate::theme::current_theme();
        for (i, theme) in Theme::all().iter().enumerate() {
            if *theme == active {
                if let Some(c) = checks_for_show.get(i) {
                    c.set_active(true);
                }
            }
        }
    });

    vbox.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

    let customize_btn = gtk4::Button::new();
    customize_btn.add_css_class("flat");
    customize_btn.add_css_class("app-popover-check");
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    hbox.append(&gtk4::Image::from_icon_name("preferences-color-symbolic"));
    hbox.append(&gtk4::Label::new(Some("Customize...")));
    customize_btn.set_child(Some(&hbox));
    let pop2 = popover.clone();
    let win = parent_window.clone();
    customize_btn.connect_clicked(move |_| {
        pop2.popdown();
        crate::dialogs::color_customizer::show_color_customizer_dialog(&*win);
    });
    vbox.append(&customize_btn);

    popover.set_child(Some(&vbox));
    popover
}

/// Setup the full workspace UI in the window (replaces any existing content).
fn setup_workspace_ui(
    window: &Rc<adw::ApplicationWindow>,
    workspace: Workspace,
    config_path: Option<&Path>,
) {
    let mut workspace = workspace;
    let workspace_theme = normalize_workspace_theme(&mut workspace, load_preferred_theme());
    let ws_name = workspace.name.clone();
    window.set_title(Some(&format!("Pax — {}", ws_name)));

    // Apply the app-wide preferred theme before building the workspace chrome.
    apply_theme(workspace_theme);

    // Header bar with hamburger menu
    let header = adw::HeaderBar::new();
    header.add_css_class("app-headerbar");
    header.set_show_end_title_buttons(true);
    header.set_show_start_title_buttons(true);

    // Pax icon + workspace name centered in header
    let title_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    title_box.set_halign(gtk4::Align::Center);
    let title_icon = gtk4::Image::from_icon_name("pax");
    title_icon.set_pixel_size(22);
    title_box.append(&title_icon);
    let app_label = gtk4::Label::new(Some("Pax"));
    app_label.add_css_class("heading");
    title_box.append(&app_label);
    let sep_label = gtk4::Label::new(Some("—"));
    sep_label.add_css_class("dim-label");
    title_box.append(&sep_label);
    let ws_label = gtk4::Label::new(Some(&ws_name));
    ws_label.add_css_class("dim-label");
    title_box.append(&ws_label);
    header.set_title_widget(Some(&title_box));
    HEADER_WS_LABEL.with(|cell| {
        cell.borrow_mut().replace(ws_label.clone());
    });

    let menu_btn = gtk4::MenuButton::new();
    menu_btn.set_icon_name("open-menu-symbolic");
    menu_btn.set_tooltip_text(Some("Menu"));
    menu_btn.add_css_class("flat");
    menu_btn.add_css_class("app-menu-btn");
    header.pack_start(&menu_btn);

    // Dirty indicator (orange floppy) — packed at end (right side, near window buttons)
    let dirty_sep = gtk4::Separator::new(gtk4::Orientation::Vertical);
    dirty_sep.set_margin_start(4);
    dirty_sep.set_margin_end(4);
    dirty_sep.set_visible(false);
    header.pack_end(&dirty_sep);

    let dirty_icon = gtk4::Image::from_icon_name("media-floppy-symbolic");
    dirty_icon.add_css_class("dirty-indicator");
    dirty_icon.set_tooltip_text(Some("Unsaved changes"));
    dirty_icon.set_visible(false);
    header.pack_end(&dirty_icon);
    DIRTY_INDICATOR.with(|cell| {
        cell.borrow_mut().replace((dirty_icon, dirty_sep));
    });

    // Theme picker button (right side of header)
    let theme_btn = gtk4::MenuButton::new();
    theme_btn.set_icon_name("applications-graphics-symbolic");
    theme_btn.set_tooltip_text(Some("Theme"));
    theme_btn.add_css_class("flat");
    let theme_popover = build_theme_popover(window);
    theme_btn.set_popover(Some(&theme_popover));
    header.pack_end(&theme_btn);

    // Shared state
    let ws_view = Rc::new(RefCell::new(WorkspaceView::build(&workspace, config_path)));
    let status_bar = Rc::new(RefCell::new(StatusBar::new()));
    let save_action = gtk4::gio::SimpleAction::new("save", None);
    let window_rc = window.clone();

    // Wire up type chooser callback
    {
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        let cb: crate::panels::chooser::OnTypeChosen = Rc::new(move |panel_id, type_id| {
            let needs_config = type_id == "markdown" || type_id == "code_editor";

            if needs_config {
                // For panels that need a file/directory, show config FIRST
                let default_type = match type_id {
                    "markdown" => pax_core::workspace::PanelType::Markdown {
                        // Prefill with the same default the registry factory
                        // would otherwise use, so the dialog shows the user
                        // exactly what file will be opened/created.
                        file: "README.md".to_string(),
                    },
                    "code_editor" => pax_core::workspace::PanelType::CodeEditor {
                        root_dir: String::new(),
                        ssh: None,
                        remote_path: None,
                        poll_interval: None,
                    },
                    _ => pax_core::workspace::PanelType::Terminal,
                };
                let pid = panel_id.to_string();
                // Human-readable default name in the config dialog, instead of
                // the raw type id ("markdown", "code_editor") — users saw it
                // prefilled in the Name field and had to retype. For markdown
                // we match the default file stem so Name and File agree.
                let default_name: &str = match type_id {
                    "terminal" => "Terminal",
                    "markdown" => "README",
                    "code_editor" => "Code Editor",
                    _ => type_id,
                };
                let ws2 = ws.clone();
                let win2 = win.clone();
                let sa2 = sa.clone();
                let saved_ssh = {
                    let view = ws.borrow();
                    std::rc::Rc::new(std::cell::RefCell::new(
                        view.workspace().ssh_configs.clone(),
                    ))
                };
                crate::dialogs::panel_config::show_panel_config_dialog(
                    &*win,
                    default_name,
                    &default_type,
                    None,
                    None,
                    &[],
                    None,
                    0,
                    0,
                    saved_ssh,
                    move |new_name,
                          new_type,
                          new_cwd,
                          new_ssh,
                          new_cmds,
                          new_close,
                          new_mw,
                          new_mh| {
                        ws2.borrow_mut().apply_panel_config(
                            &pid, new_name, new_type, new_cwd, new_ssh, new_cmds, new_close,
                            new_mw, new_mh,
                        );
                        actions::update_dirty_ui(&ws2, &win2, &sa2);
                    },
                );
            } else {
                ws.borrow_mut().set_panel_type(panel_id, type_id);
                sb.borrow()
                    .set_message(&format!("{} → {}", panel_id, type_id));
                actions::update_dirty_ui(&ws, &win, &sa);
            }
        });
        ws_view.borrow_mut().set_type_chosen_callback(cb);
    }

    // Wire up panel ⋮ menu action callback
    {
        let ws_for_cb = ws_view.clone();
        let sb_for_cb = status_bar.clone();
        let win_for_cb = window_rc.clone();
        let sa_for_cb = save_action.clone();
        let cb: crate::panel_host::PanelActionCallback = Rc::new(move |panel_id, action| {
            if panel_id.strip_prefix("nb-tab:").is_some() {
                match action {
                    PanelAction::UpdateTabDraft { tab_id, name } => {
                        ws_for_cb.borrow_mut().update_tab_edit_draft(&tab_id, name);
                    }
                    PanelAction::PreviewTabMove { tab_id, offset } => {
                        if ws_for_cb
                            .borrow_mut()
                            .preview_tab_edit_move(&tab_id, offset)
                        {
                            let ws_for_idle = ws_for_cb.clone();
                            let tab_id = tab_id.clone();
                            glib::idle_add_local_once(move || {
                                ws_for_idle
                                    .borrow_mut()
                                    .clear_tab_edit_commit_suppression(&tab_id);
                            });
                        }
                    }
                    PanelAction::CommitTabEdit { tab_id } => {
                        ws_for_cb.borrow_mut().commit_tab_edit(&tab_id);
                    }
                    PanelAction::CancelTabEdit { tab_id } => {
                        ws_for_cb.borrow_mut().cancel_tab_edit(&tab_id);
                    }
                    _ => {}
                }
                actions::update_dirty_ui(&ws_for_cb, &win_for_cb, &sa_for_cb);
                return;
            }

            if let Some(tabs_path) = panel_id.strip_prefix("nb-tabs:") {
                match action {
                    PanelAction::AddTabToNotebook => {
                        if let Some(tabs_path) = crate::widget_builder::decode_tab_path(tabs_path) {
                            if let Some(new_id) =
                                ws_for_cb.borrow_mut().add_tab_to_tabs_path(&tabs_path)
                            {
                                sb_for_cb
                                    .borrow()
                                    .set_message(&format!("Tab + → {}", new_id));
                            }
                        }
                    }
                    _ => {}
                }
                actions::update_dirty_ui(&ws_for_cb, &win_for_cb, &sa_for_cb);
                return;
            }

            // "nb:<panel_id>" means action on notebook
            if let Some(real_id) = panel_id.strip_prefix("nb:") {
                let view = ws_for_cb.borrow();
                if let Some(host) = view.host(real_id) {
                    let widget = host.widget().clone();
                    if let Some(nb) = crate::widget_builder::find_notebook_ancestor(&widget) {
                        drop(view);
                        match action {
                            PanelAction::AddTabToNotebook => {
                                let _ = nb;
                            }
                            PanelAction::RemoveTab => {
                                {
                                    let v = ws_for_cb.borrow();
                                    if let Some(idx) = v.focus_order_index(real_id) {
                                        drop(v);
                                        ws_for_cb.borrow_mut().set_focus_index(idx);
                                    }
                                }
                                if ws_for_cb.borrow_mut().close_focused() {
                                    if let Some(id) = ws_for_cb.borrow().focused_panel_id() {
                                        sb_for_cb.borrow().set_panel(id);
                                    }
                                    sb_for_cb.borrow().set_message("Tab removed");
                                }
                            }
                            PanelAction::BeginTabEdit {
                                tab_id,
                                tab_path,
                                panel_id,
                                name,
                                is_layout,
                            } => {
                                if ws_for_cb
                                    .borrow_mut()
                                    .begin_tab_edit(&panel_id, &tab_id, tab_path, name, is_layout)
                                {
                                }
                            }
                            PanelAction::UpdateTabDraft { tab_id, name } => {
                                ws_for_cb.borrow_mut().update_tab_edit_draft(&tab_id, name);
                            }
                            PanelAction::PreviewTabMove { tab_id, offset } => {
                                if ws_for_cb
                                    .borrow_mut()
                                    .preview_tab_edit_move(&tab_id, offset)
                                {
                                    let ws_for_idle = ws_for_cb.clone();
                                    let tab_id = tab_id.clone();
                                    glib::idle_add_local_once(move || {
                                        ws_for_idle
                                            .borrow_mut()
                                            .clear_tab_edit_commit_suppression(&tab_id);
                                    });
                                }
                            }
                            PanelAction::CommitTabEdit { tab_id } => {
                                ws_for_cb.borrow_mut().commit_tab_edit(&tab_id);
                            }
                            PanelAction::CancelTabEdit { tab_id } => {
                                ws_for_cb.borrow_mut().cancel_tab_edit(&tab_id);
                            }
                            _ => {}
                        }
                    }
                }
                actions::update_dirty_ui(&ws_for_cb, &win_for_cb, &sa_for_cb);
                return;
            }

            // Focus the panel that triggered the action
            {
                let view = ws_for_cb.borrow();
                if let Some(idx) = view.focus_order_index(panel_id) {
                    drop(view);
                    ws_for_cb.borrow_mut().set_focus_index(idx);
                }
            }
            match action {
                PanelAction::SplitH => {
                    if let Some(new_id) = ws_for_cb.borrow_mut().split_focused_h() {
                        sb_for_cb
                            .borrow()
                            .set_message(&format!("Split H → {}", new_id));
                    }
                }
                PanelAction::SplitV => {
                    if let Some(new_id) = ws_for_cb.borrow_mut().split_focused_v() {
                        sb_for_cb
                            .borrow()
                            .set_message(&format!("Split V → {}", new_id));
                    }
                }
                PanelAction::AddTab => {
                    if let Some(new_id) = ws_for_cb.borrow_mut().add_tab_focused() {
                        sb_for_cb
                            .borrow()
                            .set_message(&format!("TabSplit → {}", new_id));
                    }
                }
                PanelAction::InsertBefore => {
                    if let Some(new_id) = ws_for_cb
                        .borrow_mut()
                        .insert_sibling_focused(crate::layout_ops::InsertPosition::Before)
                    {
                        sb_for_cb
                            .borrow()
                            .set_message(&format!("Insert Before → {}", new_id));
                    }
                }
                PanelAction::InsertAfter => {
                    if let Some(new_id) = ws_for_cb
                        .borrow_mut()
                        .insert_sibling_focused(crate::layout_ops::InsertPosition::After)
                    {
                        sb_for_cb
                            .borrow()
                            .set_message(&format!("Insert After → {}", new_id));
                    }
                }
                PanelAction::Reset => {
                    ws_for_cb.borrow_mut().reset_panel(panel_id);
                    sb_for_cb.borrow().set_message("Panel reset");
                }
                PanelAction::Close => {
                    if ws_for_cb.borrow_mut().close_focused() {
                        if let Some(id) = ws_for_cb.borrow().focused_panel_id() {
                            sb_for_cb.borrow().set_panel(id);
                        }
                        sb_for_cb.borrow().set_message("Panel closed");
                    } else {
                        sb_for_cb.borrow().set_message("Cannot close last panel");
                    }
                }
                PanelAction::Configure => {
                    let (pname, ptype, pcwd, pssh, pcmds, pclose, pmw, pmh) = {
                        let view = ws_for_cb.borrow();
                        let ws = view.workspace();
                        let pcfg = ws.panel(panel_id);
                        (
                            pcfg.map(|p| p.name.clone()).unwrap_or_default(),
                            pcfg.map(|p| p.effective_type())
                                .unwrap_or(pax_core::workspace::PanelType::Terminal),
                            pcfg.and_then(|p| p.cwd.clone()),
                            pcfg.and_then(|p| p.effective_ssh()),
                            pcfg.map(|p| p.startup_commands.clone()).unwrap_or_default(),
                            pcfg.and_then(|p| p.before_close.clone()),
                            pcfg.map(|p| p.min_width).unwrap_or(0),
                            pcfg.map(|p| p.min_height).unwrap_or(0),
                        )
                    };
                    let pid = panel_id.to_string();
                    let ws2 = ws_for_cb.clone();
                    let win2 = win_for_cb.clone();
                    let sa2 = sa_for_cb.clone();
                    // Shared SSH configs — backed by workspace data, changes persist immediately
                    let saved_ssh: std::rc::Rc<
                        std::cell::RefCell<Vec<pax_core::workspace::NamedSshConfig>>,
                    > = {
                        let view = ws_for_cb.borrow();
                        std::rc::Rc::new(std::cell::RefCell::new(
                            view.workspace().ssh_configs.clone(),
                        ))
                    };
                    // Sync changes back to workspace whenever the Rc is modified
                    let ws_sync = ws_for_cb.clone();
                    let saved_ssh_sync = saved_ssh.clone();
                    // Poll for changes every 500ms while dialog is open
                    let last_len = std::rc::Rc::new(std::cell::Cell::new(saved_ssh.borrow().len()));
                    let sync_active = std::rc::Rc::new(std::cell::Cell::new(true));
                    let sync_flag = sync_active.clone();
                    gtk4::glib::timeout_add_local(
                        std::time::Duration::from_millis(500),
                        move || {
                            if !sync_flag.get() {
                                return gtk4::glib::ControlFlow::Break;
                            }
                            let current = saved_ssh_sync.borrow().len();
                            if current != last_len.get() {
                                last_len.set(current);
                                ws_sync.borrow_mut().workspace_mut().ssh_configs =
                                    saved_ssh_sync.borrow().clone();
                            }
                            gtk4::glib::ControlFlow::Continue
                        },
                    );
                    let sync_stop = sync_active.clone();
                    crate::dialogs::panel_config::show_panel_config_dialog(
                        &*win_for_cb,
                        &pname,
                        &ptype,
                        pcwd.as_deref(),
                        pssh.as_ref(),
                        &pcmds,
                        pclose.as_deref(),
                        pmw,
                        pmh,
                        saved_ssh,
                        move |new_name,
                              new_type,
                              new_cwd,
                              new_ssh,
                              new_cmds,
                              new_close,
                              new_mw,
                              new_mh| {
                            sync_stop.set(false);
                            ws2.borrow_mut().apply_panel_config(
                                &pid, new_name, new_type, new_cwd, new_ssh, new_cmds, new_close,
                                new_mw, new_mh,
                            );
                            actions::update_dirty_ui(&ws2, &win2, &sa2);
                        },
                    );
                }
                PanelAction::Zoom => {
                    let idx = ws_for_cb.borrow().focus_order_index(panel_id);
                    if let Some(idx) = idx {
                        ws_for_cb.borrow_mut().set_focus_index(idx);
                    }
                    ws_for_cb.borrow_mut().toggle_zoom();
                    let zoomed = ws_for_cb.borrow().is_zoomed();
                    sb_for_cb
                        .borrow()
                        .set_message(if zoomed { "Zoom ON" } else { "Zoom OFF" });
                }
                PanelAction::Sync => {
                    let idx = ws_for_cb.borrow().focus_order_index(panel_id);
                    if let Some(idx) = idx {
                        ws_for_cb.borrow_mut().set_focus_index(idx);
                    }
                    let result = ws_for_cb.borrow_mut().toggle_sync_focused();
                    if let Some((pid, is_synced)) = result {
                        let count = ws_for_cb.borrow().sync_count();
                        if is_synced {
                            sb_for_cb
                                .borrow()
                                .set_message(&format!("Sync ON: {} ({} panels)", pid, count));
                        } else {
                            sb_for_cb
                                .borrow()
                                .set_message(&format!("Sync OFF: {} ({} panels)", pid, count));
                        }
                    }
                }
                PanelAction::Rename(new_name) => {
                    let mut view = ws_for_cb.borrow_mut();
                    view.rename_panel(panel_id, &new_name);
                    drop(view);
                    sb_for_cb
                        .borrow()
                        .set_message(&format!("Renamed: {}", panel_id));
                }
                PanelAction::RenameTab(new_name) => {
                    // Only update the tab label in the layout tree, not the panel name.
                    // panel_id here is the first child panel — used to locate the tab.
                    let mut view = ws_for_cb.borrow_mut();
                    tracing::debug!(
                        "RenameTab: panel_id='{}', new_name='{}'",
                        panel_id,
                        new_name
                    );
                    view.rename_tab_label(panel_id, &new_name);
                    crate::layout_ops::debug_layout_tree(&view.workspace().layout, "AFTER_RENAME");
                    drop(view);
                    actions::update_dirty_ui(&ws_for_cb, &win_for_cb, &sa_for_cb);
                    sb_for_cb
                        .borrow()
                        .set_message(&format!("Tab renamed: {}", new_name));
                }
                PanelAction::Collapse => {
                    let view = ws_for_cb.borrow();
                    if let Some(host) = view.hosts().get(panel_id) {
                        host.expand_collapsed();
                    }
                }
                PanelAction::Focus => {
                    let idx = ws_for_cb.borrow().focus_order_index(panel_id);
                    if let Some(idx) = idx {
                        ws_for_cb.borrow_mut().set_focus_index(idx);
                    }
                }
                PanelAction::AddTabToNotebook
                | PanelAction::RemoveTab
                | PanelAction::BeginTabEdit { .. }
                | PanelAction::UpdateTabDraft { .. }
                | PanelAction::PreviewTabMove { .. }
                | PanelAction::CommitTabEdit { .. }
                | PanelAction::CancelTabEdit { .. } => {}
            }
            actions::update_dirty_ui(&ws_for_cb, &win_for_cb, &sa_for_cb);
        });
        ws_view.borrow_mut().set_action_callback(cb);
    }

    {
        let ws_for_click = ws_view.clone();
        let win_for_click = window_rc.clone();
        let sa_for_click = save_action.clone();
        let outside_click = gtk4::GestureClick::new();
        outside_click.set_button(1);
        outside_click.set_propagation_phase(gtk4::PropagationPhase::Capture);
        outside_click.connect_pressed(move |_, _, x, y| {
            let picked = win_for_click.pick(x, y, gtk4::PickFlags::DEFAULT);
            let (active_editor, tab_id) = {
                let view = ws_for_click.borrow();
                let active_editor = crate::widget_builder::find_active_tab_editor_recursive(
                    view.widget().upcast_ref(),
                );
                let tab_id = view.active_tab_edit_tab_id();
                (active_editor, tab_id)
            };
            let (Some(active_editor), Some(tab_id)) = (active_editor, tab_id) else {
                return;
            };
            let clicked_inside_editor = picked
                .as_ref()
                .map(|widget| {
                    let mut current = Some(widget.clone());
                    while let Some(w) = current {
                        if w == active_editor {
                            return true;
                        }
                        current = w.parent();
                    }
                    false
                })
                .unwrap_or(false);
            if clicked_inside_editor {
                return;
            }
            if ws_for_click.borrow_mut().commit_tab_edit(&tab_id) {
                actions::update_dirty_ui(&ws_for_click, &win_for_click, &sa_for_click);
            }
        });
        window.add_controller(outside_click);
    }

    {
        let ws_for_layout = ws_view.clone();
        let win_for_layout = window_rc.clone();
        let sa_for_layout = save_action.clone();
        let cb: Rc<dyn Fn()> = Rc::new(move || {
            if ws_for_layout
                .borrow_mut()
                .sync_ratios_from_widgets_if_changed()
            {
                actions::update_dirty_ui(&ws_for_layout, &win_for_layout, &sa_for_layout);
            }
        });
        ws_view.borrow_mut().set_layout_change_callback(cb);
    }

    // Setup sync input propagation: when a synced terminal gets local input,
    // forward it to all other synced terminals.
    {
        let ws = ws_view.clone();
        let sync_cb: Rc<dyn Fn(&str, &[u8])> = Rc::new(move |source_panel_id, data| {
            if let Ok(view) = ws.try_borrow() {
                if view.is_panel_synced(source_panel_id) {
                    view.write_to_synced(data, source_panel_id);
                }
            }
        });
        ws_view.borrow_mut().setup_sync_callbacks(sync_cb);
    }

    // Auto-save flag (disabled by default, toggled via menu)
    let autosave_enabled = std::rc::Rc::new(std::cell::Cell::new(false));
    let bypass_close_prompt = std::rc::Rc::new(std::cell::Cell::new(false));

    // Register GIO actions
    let action_group = gtk4::gio::SimpleActionGroup::new();

    // New workspace
    {
        let action = gtk4::gio::SimpleAction::new("new", None);
        let ws = ws_view.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        let sb = status_bar.clone();
        action.connect_activate(move |_, _| {
            let ws2 = ws.clone();
            let sb2 = sb.clone();
            let win2 = win.clone();
            let sa2 = sa.clone();
            let on_continue: Rc<dyn Fn()> = Rc::new(move || {
                let empty = new_workspace_with_preferred_theme("untitled");
                let empty_theme = load_preferred_theme();
                if let Err(e) = ws2.borrow_mut().load_workspace(empty, None) {
                    sb2.borrow().set_message(&format!("Error: {}", e));
                }
                apply_theme(empty_theme);
                actions::update_dirty_ui(&ws2, &win2, &sa2);
                actions::update_status_bar_path(&ws2, &sb2);
            });
            actions::confirm_discard_workspace_changes(&ws, &sb, &win, &sa, on_continue);
        });
        action_group.add_action(&action);
    }

    // Open recent
    {
        let action = gtk4::gio::SimpleAction::new("recent", None);
        let ws = ws_view.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        let sb = status_bar.clone();
        action.connect_activate(move |_, _| {
            actions::show_recent_dialog(&ws, &sb, &win, &sa);
        });
        action_group.add_action(&action);
    }

    // Open file
    {
        let action = gtk4::gio::SimpleAction::new("open", None);
        let ws = ws_view.clone();
        let win = window_rc.clone();
        let sb = status_bar.clone();
        let sa = save_action.clone();
        action.connect_activate(move |_, _| {
            actions::do_open(&ws, &sb, &win, &sa);
        });
        action_group.add_action(&action);
    }

    // Save
    {
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        save_action.connect_activate(move |_, _| {
            actions::do_save(&ws, &sb, &win, &sa, false);
        });
        action_group.add_action(&save_action);
    }

    // Save As
    {
        let action = gtk4::gio::SimpleAction::new("save-as", None);
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        action.connect_activate(move |_, _| {
            actions::do_save(&ws, &sb, &win, &sa, true);
        });
        action_group.add_action(&action);
    }

    // Settings
    {
        let action = gtk4::gio::SimpleAction::new("settings", None);
        let ws = ws_view.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        action.connect_activate(move |_, _| {
            let current = {
                let view = ws.borrow();
                crate::dialogs::settings::AppSettings {
                    workspace_name: view.workspace().name.clone(),
                    theme: load_preferred_theme(),
                    default_shell: view.workspace().settings.default_shell.clone(),
                    scrollback_lines: view.workspace().settings.scrollback_lines,
                    output_retention_days: view.workspace().settings.output_retention_days,
                }
            };
            let ws2 = ws.clone();
            let win2 = win.clone();
            let sa2 = sa.clone();
            crate::dialogs::settings::show_settings_dialog(&*win, &current, move |new_settings| {
                apply_theme(new_settings.theme);
                save_preferred_theme(new_settings.theme);
                {
                    let mut view = ws2.borrow_mut();
                    view.rename_workspace(&new_settings.workspace_name);
                    let ws = view.workspace_mut();
                    ws.settings.theme = new_settings.theme.to_id().to_string();
                    ws.settings.default_shell = new_settings.default_shell;
                    ws.settings.scrollback_lines = new_settings.scrollback_lines;
                    ws.settings.output_retention_days = new_settings.output_retention_days;
                }
                actions::update_dirty_ui(&ws2, &win2, &sa2);
            });
        });
        action_group.add_action(&action);
    }

    // Quit
    {
        let action = gtk4::gio::SimpleAction::new("quit", None);
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let sa = save_action.clone();
        let win = window_rc.clone();
        let bypass = bypass_close_prompt.clone();
        action.connect_activate(move |_, _| {
            let win2 = win.clone();
            let bypass2 = bypass.clone();
            let on_continue: Rc<dyn Fn()> = Rc::new(move || {
                bypass2.set(true);
                win2.close();
            });
            actions::confirm_discard_workspace_changes(&ws, &sb, &win, &sa, on_continue);
        });
        action_group.add_action(&action);
    }

    // Keyboard shortcuts dialog
    {
        let action = gtk4::gio::SimpleAction::new("shortcuts", None);
        let win = window_rc.clone();
        action.connect_activate(move |_, _| {
            show_shortcuts_dialog(&win);
        });
        action_group.add_action(&action);
    }
    {
        let action = gtk4::gio::SimpleAction::new("about", None);
        let win = window_rc.clone();
        action.connect_activate(move |_, _| {
            show_about_dialog(&win);
        });
        action_group.add_action(&action);
    }
    {
        let action = gtk4::gio::SimpleAction::new_stateful("autosave", None, &false.to_variant());
        let enabled = autosave_enabled.clone();
        let sb = status_bar.clone();
        action.connect_activate(move |action, _| {
            let current = enabled.get();
            let new_val = !current;
            enabled.set(new_val);
            action.set_state(&new_val.to_variant());
            sb.borrow().set_message(if new_val {
                "Auto-save enabled"
            } else {
                "Auto-save disabled"
            });
        });
        action_group.add_action(&action);
    }

    window.insert_action_group("app", Some(&action_group));

    let menu_popover = build_app_menu_popover(&window_rc, &save_action, &autosave_enabled);
    menu_btn.set_popover(Some(&menu_popover));

    // Window close request
    {
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let sa = save_action.clone();
        let win = window_rc.clone();
        let bypass = bypass_close_prompt.clone();
        window.connect_close_request(move |_| {
            if bypass.get() {
                ws.borrow().run_all_before_close();
                ws.borrow().shutdown_all_backends();
                return glib::Propagation::Proceed;
            }
            if !ws.borrow().is_dirty() {
                ws.borrow().run_all_before_close();
                ws.borrow().shutdown_all_backends();
                return glib::Propagation::Proceed;
            }
            let win2 = win.clone();
            let bypass2 = bypass.clone();
            let on_continue: Rc<dyn Fn()> = Rc::new(move || {
                bypass2.set(true);
                win2.close();
            });
            actions::confirm_discard_workspace_changes(&ws, &sb, &win, &sa, on_continue);
            glib::Propagation::Stop
        });
    }

    // Content
    let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let ws_widget = ws_view.borrow().widget().clone();
    ws_widget.set_vexpand(true);
    ws_widget.set_hexpand(true);
    content_box.append(&ws_widget);
    content_box.append(status_bar.borrow().widget());

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_css_class("app-toolbar-view");
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content_box));
    window.set_content(Some(&toolbar_view));
    apply_theme(workspace_theme);

    // Keyboard shortcuts
    let controller = gtk4::EventControllerKey::new();
    controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

    {
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        controller.connect_key_pressed(move |_ctrl, key, _code, modifiers| {
            let ctrl = crate::shortcuts::has_primary(modifiers);
            let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);

            if ctrl && shift {
                match key {
                    gdk::Key::H => {
                        if let Some(new_id) = ws.borrow_mut().split_focused_h() {
                            sb.borrow().set_message(&format!("Split H → {}", new_id));
                        }
                        actions::update_dirty_ui(&ws, &win, &sa);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::J => {
                        if let Some(new_id) = ws.borrow_mut().split_focused_v() {
                            sb.borrow().set_message(&format!("Split V → {}", new_id));
                        }
                        actions::update_dirty_ui(&ws, &win, &sa);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::T => {
                        if let Some(new_id) = ws.borrow_mut().add_tab_focused() {
                            sb.borrow().set_message(&format!("Tab → {}", new_id));
                        }
                        actions::update_dirty_ui(&ws, &win, &sa);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::W => {
                        if ws.borrow_mut().close_focused() {
                            if let Some(id) = ws.borrow().focused_panel_id() {
                                sb.borrow().set_panel(id);
                            }
                            sb.borrow().set_message("Panel closed");
                        }
                        actions::update_dirty_ui(&ws, &win, &sa);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::S => {
                        let result = ws.borrow_mut().toggle_sync_focused();
                        if let Some((panel_id, is_synced)) = result {
                            let count = ws.borrow().sync_count();
                            if is_synced {
                                sb.borrow().set_message(&format!(
                                    "Sync ON: {} ({} panels)",
                                    panel_id, count
                                ));
                            } else {
                                sb.borrow().set_message(&format!(
                                    "Sync OFF: {} ({} panels)",
                                    panel_id, count
                                ));
                            }
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::Z => {
                        if focused_panel_uses_text_editing_shortcuts(&ws.borrow()) {
                            return glib::Propagation::Proceed;
                        }
                        ws.borrow_mut().toggle_zoom();
                        let zoomed = ws.borrow().is_zoomed();
                        if zoomed {
                            if let Some(id) = ws.borrow().focused_panel_id() {
                                sb.borrow().set_message(&format!("Zoom: {}", id));
                            }
                        } else {
                            sb.borrow().set_message("Zoom off");
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::C | gdk::Key::V => {
                        return glib::Propagation::Proceed;
                    }
                    _ => {}
                }
            }

            if ctrl && !shift {
                match key {
                    gdk::Key::q => {
                        ws.borrow().run_all_before_close();
                        ws.borrow().shutdown_all_backends();
                        std::process::exit(0);
                    }
                    gdk::Key::n => {
                        ws.borrow_mut().focus_next();
                        if let Some(id) = ws.borrow().focused_panel_id() {
                            sb.borrow().set_panel(id);
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::p => {
                        // If focused panel is a code editor, let Ctrl+P propagate
                        // to the editor's fuzzy finder
                        let is_code_editor = focused_panel_is_code_editor(&ws.borrow());
                        if is_code_editor {
                            return glib::Propagation::Proceed;
                        }
                        ws.borrow_mut().focus_prev();
                        if let Some(id) = ws.borrow().focused_panel_id() {
                            sb.borrow().set_panel(id);
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::s => {
                        // Let editor/markdown panels handle file save themselves.
                        if focused_panel_uses_text_editing_shortcuts(&ws.borrow()) {
                            return glib::Propagation::Proceed;
                        }
                        actions::do_save(&ws, &sb, &win, &sa, false);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::o => {
                        actions::do_open(&ws, &sb, &win, &sa);
                        return glib::Propagation::Stop;
                    }
                    // Ctrl+R is reserved for terminal reverse-search (bash)
                    gdk::Key::r => {
                        return glib::Propagation::Proceed;
                    }
                    gdk::Key::z | gdk::Key::y => return glib::Propagation::Proceed,
                    gdk::Key::Up | gdk::Key::Down | gdk::Key::Left | gdk::Key::Right => {
                        let ws_widget = ws.borrow().widget().clone();
                        let step = 80.0;
                        match key {
                            gdk::Key::Up => {
                                let adj = ws_widget.vadjustment();
                                adj.set_value(adj.value() - step);
                            }
                            gdk::Key::Down => {
                                let adj = ws_widget.vadjustment();
                                adj.set_value(adj.value() + step);
                            }
                            gdk::Key::Left => {
                                let adj = ws_widget.hadjustment();
                                adj.set_value(adj.value() - step);
                            }
                            gdk::Key::Right => {
                                let adj = ws_widget.hadjustment();
                                adj.set_value(adj.value() + step);
                            }
                            _ => {}
                        }
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }

            glib::Propagation::Proceed
        });
    }

    window.add_controller(controller);

    // Auto-save workspace every 30s if enabled, dirty, and has a config path
    {
        let ws = ws_view.clone();
        let sb = status_bar.clone();
        let win = window_rc.clone();
        let sa = save_action.clone();
        let enabled = autosave_enabled.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(30), move || {
            if !enabled.get() {
                return glib::ControlFlow::Continue;
            }
            let has_path = ws.borrow().has_config_path();
            let is_dirty = ws.borrow().is_dirty();
            if has_path && is_dirty {
                match ws.borrow_mut().save() {
                    Ok(path) => {
                        tracing::info!("Auto-saved workspace: {}", path.display());
                        sb.borrow().set_message("Auto-saved");
                    }
                    Err(e) => {
                        tracing::warn!("Auto-save failed: {}", e);
                    }
                }
                actions::update_dirty_ui(&ws, &win, &sa);
            }
            glib::ControlFlow::Continue
        });
    }

    // Ctrl+Scroll: scroll the entire workspace (bypasses VTE scroll capture)
    {
        let scroll_ctrl = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::VERTICAL
                | gtk4::EventControllerScrollFlags::HORIZONTAL,
        );
        scroll_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let ws_widget = ws_view.borrow().widget().clone();
        scroll_ctrl.connect_scroll(move |ctrl, dx, dy| {
            let mods = ctrl.current_event_state();
            if mods.contains(gdk::ModifierType::CONTROL_MASK) {
                let vadj = ws_widget.vadjustment();
                vadj.set_value(vadj.value() + dy * 50.0);
                let hadj = ws_widget.hadjustment();
                hadj.set_value(hadj.value() + dx * 50.0);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(scroll_ctrl);
    }

    // Initial UI state
    actions::update_dirty_ui(&ws_view, &window_rc, &save_action);
    actions::update_status_bar_path(&ws_view, &status_bar);

    // macOS workaround: libadwaita HeaderBars and view-bg surfaces sometimes
    // capture the libadwaita default colors before our CSS provider has fully
    // cascaded, which leaves the main window header (and editor tabs) the
    // wrong color until something — like opening a transient dialog — forces
    // a style re-evaluation. Re-applying the theme on idle, after the entire
    // workspace UI has been built and presented, mimics that re-evaluation
    // explicitly. No-op on other platforms.
    #[cfg(target_os = "macos")]
    {
        let theme_for_idle = workspace_theme;
        glib::idle_add_local_once(move || {
            apply_theme(theme_for_idle);
        });
    }
}

thread_local! {
    static THEME_PROVIDER: RefCell<Option<gtk4::CssProvider>> = RefCell::new(None);
}

fn load_css() {
    // Startup chrome should be deterministic. The welcome page always starts
    // from the preferred app theme if present, otherwise the default theme.
    apply_theme(load_preferred_theme());
}

fn new_workspace_with_preferred_theme(name: &str) -> Workspace {
    workspace_with_theme(name, load_preferred_theme())
}

fn normalize_workspace_theme(workspace: &mut Workspace, preferred_theme: Theme) -> Theme {
    workspace.settings.theme = preferred_theme.to_id().to_string();
    preferred_theme
}

fn workspace_with_theme(name: &str, theme: Theme) -> Workspace {
    let mut workspace = pax_core::template::empty_workspace(name);
    workspace.settings.theme = theme.to_id().to_string();
    workspace
}

fn load_preferred_theme() -> Theme {
    let db_path = pax_db::Database::default_path();
    let Ok(db) = pax_db::Database::open(&db_path) else {
        return Theme::default();
    };
    load_preferred_theme_from_db(&db).unwrap_or_default()
}

fn load_preferred_theme_from_db(db: &pax_db::Database) -> Option<Theme> {
    db.get_app_preference("theme")
        .ok()
        .flatten()
        .map(|value| Theme::from_id(&value))
}

pub(crate) fn save_preferred_theme(theme: Theme) {
    let db_path = pax_db::Database::default_path();
    let Ok(db) = pax_db::Database::open(&db_path) else {
        return;
    };
    let _ = db.set_app_preference("theme", theme.to_id());
}

pub(crate) fn apply_preferred_theme() -> Theme {
    let theme = load_preferred_theme();
    apply_theme(theme);
    theme
}

pub(crate) fn apply_theme(theme: Theme) {
    let display = gdk::Display::default().expect("Could not connect to display");

    // Remove old provider
    THEME_PROVIDER.with(|cell| {
        if let Some(old) = cell.borrow_mut().take() {
            gtk4::style_context_remove_provider_for_display(&display, &old);
        }
    });

    // Set color scheme
    let style_manager = adw::StyleManager::default();
    style_manager.set_color_scheme(theme.color_scheme());

    // Update VTE terminal colors
    crate::theme::set_current_theme(theme);

    // Build CSS: color overrides → base layout → theme-specific structural rules.
    // If the user has saved custom color tweaks for this theme, apply them
    // on top of the base palette.
    let base_overrides = theme.css_overrides();
    let custom = crate::dialogs::color_customizer::load_custom_colors(theme);
    let effective_overrides = match custom.as_ref() {
        Some(c) => crate::theme::apply_color_overrides(base_overrides, c),
        None => base_overrides.to_string(),
    };
    let css = format!(
        "{}\n{}\n{}",
        effective_overrides,
        crate::theme::BASE_CSS,
        theme.css_extra(),
    );

    let provider = gtk4::CssProvider::new();
    provider.load_from_data(&css);
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    THEME_PROVIDER.with(|cell| {
        cell.borrow_mut().replace(provider);
    });

    // Push custom bg/fg to VTE terminals (which use programmatic colors,
    // not CSS). Without this, Save + close would revert the terminal to
    // the base theme palette while the rest of the UI keeps the overrides.
    #[cfg(feature = "vte")]
    if let Some(ref c) = custom {
        let bg = c.get("bg_surface").and_then(|h| gtk4::gdk::RGBA::parse(h).ok());
        let fg = c.get("fg_content").and_then(|h| gtk4::gdk::RGBA::parse(h).ok());
        if bg.is_some() || fg.is_some() {
            crate::theme::apply_custom_vte_colors(bg.as_ref(), fg.as_ref());
        }
    }

    for widget in gtk4::Window::list_toplevels() {
        widget.queue_draw();
    }
}

/// Like `apply_theme` but patches the base CSS overrides with per-token
/// color values before loading. Used by the color customizer for live
/// preview without saving.
pub(crate) fn apply_theme_with_overrides(
    theme: Theme,
    overrides: &std::collections::HashMap<String, String>,
) {
    let display = gdk::Display::default().expect("Could not connect to display");

    THEME_PROVIDER.with(|cell| {
        if let Some(old) = cell.borrow_mut().take() {
            gtk4::style_context_remove_provider_for_display(&display, &old);
        }
    });

    let style_manager = adw::StyleManager::default();
    style_manager.set_color_scheme(theme.color_scheme());
    crate::theme::set_current_theme(theme);

    let patched = crate::theme::apply_color_overrides(theme.css_overrides(), overrides);
    let css = format!("{}\n{}\n{}", patched, crate::theme::BASE_CSS, theme.css_extra());

    let provider = gtk4::CssProvider::new();
    provider.load_from_data(&css);
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    THEME_PROVIDER.with(|cell| {
        cell.borrow_mut().replace(provider);
    });

    // If bg_surface or fg_content were overridden, push those colors
    // to VTE terminals which use programmatic coloring, not CSS.
    #[cfg(feature = "vte")]
    {
        let bg = overrides
            .get("bg_surface")
            .and_then(|hex| gtk4::gdk::RGBA::parse(hex).ok());
        let fg = overrides
            .get("fg_content")
            .and_then(|hex| gtk4::gdk::RGBA::parse(hex).ok());
        if bg.is_some() || fg.is_some() {
            crate::theme::apply_custom_vte_colors(bg.as_ref(), fg.as_ref());
        }
    }

    for widget in gtk4::Window::list_toplevels() {
        widget.queue_draw();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        load_preferred_theme_from_db, normalize_workspace_theme,
        panel_type_uses_text_editing_shortcuts, workspace_with_theme, Theme,
        APP_MENU_AUTOSAVE_ITEM, APP_MENU_FILE_ITEMS, APP_MENU_QUIT_ITEM, APP_MENU_SAVE_ITEM,
        APP_MENU_SAVE_SECONDARY_ITEMS, APP_MENU_SETTINGS_ITEMS,
    };
    use pax_core::template::empty_workspace;
    use pax_core::workspace::PanelType;
    use pax_db::Database;

    #[test]
    fn startup_theme_uses_default_theme() {
        assert_eq!(Theme::default(), Theme::Graphite);
    }

    #[test]
    fn startup_theme_uses_saved_preference_from_db() {
        let db = Database::open_memory().unwrap();
        db.set_app_preference("theme", "dracula").unwrap();

        assert_eq!(load_preferred_theme_from_db(&db), Some(Theme::Dracula));
    }

    #[test]
    fn new_workspace_inherits_selected_theme() {
        let workspace = workspace_with_theme("untitled", Theme::Dracula);

        assert_eq!(workspace.settings.theme, "dracula");
    }

    #[test]
    fn workspace_theme_is_normalized_to_saved_preference() {
        let mut workspace = empty_workspace("test");
        workspace.settings.theme = Theme::Aurora.to_id().to_string();

        let theme = normalize_workspace_theme(&mut workspace, Theme::Graphite);

        assert_eq!(theme, Theme::Graphite);
        assert_eq!(workspace.settings.theme, Theme::Graphite.to_id());
    }

    #[test]
    fn text_editing_shortcuts_apply_to_code_and_markdown_panels() {
        assert!(panel_type_uses_text_editing_shortcuts(
            &PanelType::CodeEditor {
                root_dir: ".".into(),
                ssh: None,
                remote_path: None,
                poll_interval: None,
            }
        ));
        assert!(panel_type_uses_text_editing_shortcuts(
            &PanelType::Markdown {
                file: "notes.md".into(),
            }
        ));
        assert!(!panel_type_uses_text_editing_shortcuts(
            &PanelType::Terminal
        ));
    }

    #[test]
    fn main_menu_specs_define_icons_for_every_entry() {
        let sections = [
            APP_MENU_FILE_ITEMS,
            APP_MENU_SAVE_SECONDARY_ITEMS,
            APP_MENU_SETTINGS_ITEMS,
        ];

        for section in sections {
            for item in section {
                assert!(
                    !item.icon.is_empty(),
                    "menu item '{}' is missing an icon",
                    item.label
                );
                assert!(
                    item.action.starts_with("app."),
                    "menu item '{}' has invalid action '{}'",
                    item.label,
                    item.action
                );
            }
        }

        for item in [
            APP_MENU_SAVE_ITEM,
            APP_MENU_AUTOSAVE_ITEM,
            APP_MENU_QUIT_ITEM,
        ] {
            assert!(
                !item.icon.is_empty(),
                "menu item '{}' is missing an icon",
                item.label
            );
            assert!(
                item.action.starts_with("app."),
                "menu item '{}' has invalid action '{}'",
                item.label,
                item.action
            );
        }
    }
}

#[cfg(feature = "sourceview")]
pub(crate) fn sourceview_style_search_paths() -> Vec<std::path::PathBuf> {
    let exe = std::env::current_exe().ok();
    let candidates = [
        std::path::PathBuf::from("resources/sourceview-styles"),
        exe.as_ref()
            .and_then(|p| p.parent().map(|d| d.join("../Resources/sourceview-styles")))
            .unwrap_or_default(),
        exe.as_ref()
            .and_then(|p| p.parent().map(|d| d.join("../resources/sourceview-styles")))
            .unwrap_or_default(),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../resources/sourceview-styles"),
    ];

    let mut seen = std::collections::HashSet::new();
    let mut paths = Vec::new();
    for path in candidates {
        if path.exists() && seen.insert(path.clone()) {
            paths.push(path);
        }
    }
    paths
}

fn show_about_dialog(window: &Rc<adw::ApplicationWindow>) {
    let about = adw::AboutWindow::builder()
        .application_name("Pax")
        .application_icon("pax")
        .version(pax_core::build_info::VERSION_STRING)
        .developer_name("Pax Contributors")
        .comments("Terminal workspace manager")
        .transient_for(window.as_ref())
        .modal(true)
        .build();
    about.present();
}

fn show_shortcuts_dialog(window: &Rc<adw::ApplicationWindow>) {
    let dialog = gtk4::Window::builder()
        .title("Keyboard Shortcuts")
        .transient_for(window.as_ref())
        .modal(true)
        .default_width(450)
        .default_height(560)
        .build();
    crate::theme::configure_dialog_window(&dialog);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(20);
    vbox.set_margin_end(20);

    let title = gtk4::Label::new(Some("Keyboard Shortcuts"));
    title.add_css_class("title-3");
    title.set_halign(gtk4::Align::Start);
    title.set_margin_bottom(12);
    vbox.append(&title);

    let shortcuts = [
        (
            "General",
            vec![
                ("Ctrl+S", "Save workspace"),
                ("Ctrl+O", "Open workspace"),
                ("Ctrl+Q", "Quit"),
            ],
        ),
        (
            "Panels",
            vec![
                ("Ctrl+N", "Focus next panel"),
                ("Ctrl+P", "Focus previous panel"),
                ("Ctrl+Shift+Z", "Zoom/unzoom focused panel"),
                ("Ctrl+R", "Reverse search (terminal)"),
                ("Ctrl+Arrow", "Scroll workspace"),
                ("Ctrl+Scroll", "Scroll workspace (mouse)"),
            ],
        ),
        (
            "Text Editing",
            vec![
                ("Ctrl+Z", "Undo"),
                ("Ctrl+Y", "Redo"),
                ("Ctrl+Shift+Z", "Redo"),
                ("Ctrl+C", "Copy"),
                ("Ctrl+X", "Cut"),
                ("Ctrl+V", "Paste"),
            ],
        ),
        (
            "Code Editor",
            vec![
                ("Ctrl+S", "Save current file"),
                ("Ctrl+W", "Close current tab"),
                ("Ctrl+Tab", "Next tab"),
                ("Ctrl+P", "Fuzzy file finder"),
                ("Ctrl+E", "Recent files"),
                ("Ctrl+F", "Search in file"),
                ("Ctrl+H", "Search & replace"),
                ("Ctrl+Shift+F", "Search in project"),
                ("Ctrl+Shift+G", "Git changes"),
                ("Ctrl+B", "Toggle sidebar"),
                ("Alt+Left", "Go back (navigation)"),
                ("Alt+Right", "Go forward (navigation)"),
            ],
        ),
        (
            "Terminal",
            vec![
                ("Ctrl+Shift+C", "Copy"),
                ("Ctrl+Shift+V", "Paste"),
                ("Ctrl+R", "Reverse search"),
            ],
        ),
        (
            "Layout",
            vec![
                ("Ctrl+Shift+H", "Split horizontal (below)"),
                ("Ctrl+Shift+J", "Split vertical (right)"),
                ("Ctrl+Shift+T", "New tab"),
                ("Ctrl+Shift+W", "Close panel"),
                ("Ctrl+Shift+S", "Toggle sync (alt)"),
            ],
        ),
        (
            "Panel Header",
            vec![
                ("Double-click title", "Rename panel"),
                ("Double-click tab", "Rename tab"),
            ],
        ),
    ];

    for (section, items) in &shortcuts {
        let section_label = gtk4::Label::new(Some(section));
        section_label.add_css_class("heading");
        section_label.set_halign(gtk4::Align::Start);
        section_label.set_margin_top(12);
        section_label.set_margin_bottom(4);
        vbox.append(&section_label);

        for (key, desc) in items {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            row.set_margin_top(2);
            row.set_margin_bottom(2);

            let key_label = gtk4::Label::new(Some(key));
            key_label.set_width_chars(22);
            key_label.set_halign(gtk4::Align::Start);
            key_label.add_css_class("monospace");
            key_label.set_opacity(0.7);

            let desc_label = gtk4::Label::new(Some(desc));
            desc_label.set_halign(gtk4::Align::Start);
            desc_label.set_hexpand(true);

            row.append(&key_label);
            row.append(&desc_label);
            vbox.append(&row);
        }
    }

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_child(Some(&vbox));

    dialog.set_child(Some(&scrolled));
    dialog.present();
}
