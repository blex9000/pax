use gtk4::glib;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Drag threshold baseline for collapsed panels.
pub const COLLAPSE_SIZE: i32 = 44;
/// Actual allocated size for a collapsed panel.
pub(crate) const COLLAPSED_PANEL_SIZE: i32 = 22;
/// Visual collapsed chrome size. Does not affect drag-collapse threshold.
pub(crate) const COLLAPSED_CHROME_SIZE: i32 = 22;
pub(crate) const COLLAPSED_ICON_SIZE: i32 = 12;
/// Max characters retained from an OSC title payload before truncation.
pub(crate) const MAX_OSC_TITLE_LEN: usize = 256;
/// Max characters to lay out in the centered OSC title label.
pub(crate) const OSC_TITLE_MAX_WIDTH_CHARS: i32 = 60;

// ── KNOWN LIMITATIONS: Drag Collapse/Expand ──────────────────────────────
//
// The drag-collapse system (setup_paned_drag_collapse in widget_builder.rs)
// monitors Paned position changes and toggles panel visibility at a threshold.
//
// CONSTRAINT: No set_position() inside the position notify handler.
//   Calling set_position() triggers another notify, which fights with GTK's
//   drag gesture causing an infinite loop that freezes the UI.
//   Attempted and reverted: clamp logic, snap_target, saved_pos restore.
//
// CONSTRAINT: No set_shrink toggling between collapse/expand.
//   Setting shrink=false after collapse prevents resize below the collapsed size
//   (good), but if expand happens via click instead of drag, the shrink=false
//   persists and blocks ALL future resize on that side.
//   Attempted and reverted: shrink=false on collapse, shrink=true on expand.
//
// CURRENT BEHAVIOR:
//   - Panels collapse when dragged to threshold (COLLAPSE_SIZE + 8 = 52px)
//   - Panels expand when dragged above threshold from collapsed state
//   - Collapsed panels snap to COLLAPSED_PANEL_SIZE on idle to avoid notify loops
//   - Click on collapsed overlay expands to 50% via expand_collapsed()
//
// POTENTIAL IMPROVEMENT:
//   To prevent resize below COLLAPSE_SIZE without the above issues, would need
//   a GTK-native approach like a custom Paned widget or a drag gesture that
//   intercepts motion events before GTK's built-in handler.
// ─────────────────────────────────────────────────────────────────────────

use crate::panels::{PanelBackend, PanelCwdCallback, PanelStatusCallback, PanelTitleCallback};

/// Strip control characters (C0 except tab, and DEL) and truncate to
/// `MAX_OSC_TITLE_LEN` characters. Used to render OSC 0/2 title payloads
/// safely in the centered panel header label.
pub(crate) fn sanitize_osc_title(raw: &str) -> String {
    raw.chars()
        .filter(|&c| c == '\t' || (c >= ' ' && c != '\u{7f}'))
        .take(MAX_OSC_TITLE_LEN)
        .collect()
}

/// Actions that can be triggered from panel/tab menus.
#[derive(Debug, Clone, PartialEq)]
pub enum PanelAction {
    SplitH,
    SplitV,
    AddTab,
    /// Insert a new panel before this one in the parent split.
    InsertBefore,
    /// Insert a new panel after this one in the parent split.
    InsertAfter,
    Close,
    /// Configure panel settings
    Configure,
    /// Add a new tab to existing Notebook (from tab bar menu)
    AddTabToNotebook,
    /// Remove current tab from Notebook (from tab bar menu)
    RemoveTab,
    /// Begin editing a workspace tab label
    BeginTabEdit {
        tab_id: String,
        tab_path: Vec<usize>,
        panel_id: String,
        name: String,
        is_layout: bool,
    },
    /// Update the in-progress tab label draft text
    UpdateTabDraft {
        tab_id: String,
        name: String,
    },
    /// Preview move of the currently edited workspace tab
    PreviewTabMove {
        tab_id: String,
        offset: i32,
    },
    /// Commit the in-progress workspace tab edit
    CommitTabEdit {
        tab_id: String,
    },
    /// Cancel the in-progress workspace tab edit
    CancelTabEdit {
        tab_id: String,
    },
    /// Toggle zoom/fullscreen
    Zoom,
    /// Toggle sync input
    Sync,
    /// Rename panel (carries new name)
    Rename(String),
    /// Rename only the tab label (for layout tabs), not the child panels
    RenameTab(String),
    /// Reset panel to type chooser
    Reset,
    /// Collapse/expand panel to icon
    Collapse,
    /// Focus this panel
    Focus,
    /// Move the panel one step toward the previous sibling in its parent
    /// Hsplit (left) or Tabs (left).
    MoveLeft,
    /// Move the panel one step toward the next sibling in its parent
    /// Hsplit (right) or Tabs (right).
    MoveRight,
    /// Move the panel one step toward the previous sibling in its
    /// parent Vsplit (up).
    MoveUp,
    /// Move the panel one step toward the next sibling in its parent
    /// Vsplit (down).
    MoveDown,
}

/// Callback type for panel menu actions.
pub type PanelActionCallback = Rc<dyn Fn(&str, PanelAction)>;

/// Callback that returns the current `SiblingInfo` for a panel — used by
/// the panel menu to decide which Move items to show. Returning `None`
/// hides all Move items (e.g. root or only-child).
pub type SiblingInfoProvider = Rc<dyn Fn(&str) -> Option<crate::layout_ops::SiblingInfo>>;

/// Container widget that hosts a PanelBackend with title bar.
pub struct PanelHost {
    pub(crate) outer: gtk4::Box,
    pub(crate) container: gtk4::Box,
    _title_bar: gtk4::CenterBox,
    type_icon: gtk4::Image,
    title_label: gtk4::Label,
    osc_title_label: gtk4::Label,
    status_icon: gtk4::Image,
    sync_button: gtk4::Button,
    zoom_button: gtk4::Button,
    history_button: gtk4::Button,
    menu_button: gtk4::MenuButton,
    pub(crate) collapsed_view: gtk4::Widget,
    collapsed_icon: gtk4::Image,
    ssh_indicator: gtk4::Box,
    pub(crate) footer_bar: gtk4::Box,
    pub(crate) footer_label: gtk4::Label,
    widget: gtk4::Widget,
    panel_id: String,
    backend: Rc<RefCell<Option<Box<dyn PanelBackend>>>>,
    focused: RefCell<bool>,
    /// Shared callback ref — updated by set_action_callback, read by button handlers.
    action_cb_ref: Rc<RefCell<Option<PanelActionCallback>>>,
    /// Shared provider ref — `WorkspaceView` installs a closure that
    /// computes the panel's current `SiblingInfo` so the menu (rebuilt
    /// on every ⋮ open) reflects the live layout.
    sibling_info_provider_ref: Rc<RefCell<Option<SiblingInfoProvider>>>,
    /// Shared input callback used by terminal-like backends for sync propagation.
    sync_input_cb_ref: Rc<RefCell<Option<crate::panels::PanelInputCallback>>>,
    /// External observers of OSC 133 waiting-state transitions. Used so
    /// parent tab labels can mirror the header indicator.
    status_listeners: Rc<RefCell<Vec<Box<dyn Fn(bool)>>>>,
    /// Last known waiting state. Replayed to new listeners on registration
    /// so mirrors added after a layout rebuild match the current shell state.
    last_waiting: Rc<std::cell::Cell<bool>>,
}

impl Drop for PanelHost {
    fn drop(&mut self) {
        // Best-effort cleanup of the terminal registry — if this host was
        // currently exposing a terminal, the entry would otherwise outlive
        // the panel and feed stale closures.
        crate::panels::terminal_registry::unregister(&self.panel_id);
    }
}

impl std::fmt::Debug for PanelHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PanelHost")
            .field("panel_id", &self.panel_id)
            .finish()
    }
}

impl PanelHost {
    pub fn new(panel_id: &str, name: &str, action_cb: Option<PanelActionCallback>) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let action_cb_ref: Rc<RefCell<Option<PanelActionCallback>>> =
            Rc::new(RefCell::new(action_cb.clone()));
        let sibling_info_provider_ref: Rc<RefCell<Option<SiblingInfoProvider>>> =
            Rc::new(RefCell::new(None));
        // Create the backend Rc early so we can close over it in button handlers
        // wired below (before the struct is assembled).
        let backend: Rc<RefCell<Option<Box<dyn PanelBackend>>>> = Rc::new(RefCell::new(None));

        // Title bar: CenterBox so the OSC title sits in the geometric center
        // of the bar regardless of the widths of start/end content.
        let title_bar = gtk4::CenterBox::new();
        title_bar.add_css_class("panel-title-bar");

        // Panel type icon
        let type_icon = gtk4::Image::from_icon_name("radio-symbolic"); // default: empty/chooser dot
        type_icon.add_css_class("panel-type-icon");
        type_icon.add_css_class("panel-title-type-icon");

        // Title: stack with label (view) and entry (edit), double-click to rename.
        // No hexpand — CenterBox decides placement via start/center/end slots.
        let title_stack = gtk4::Stack::new();
        title_stack.set_halign(gtk4::Align::Start);

        let title_label = gtk4::Label::new(Some(name));
        title_label.add_css_class("panel-title");
        title_label.set_halign(gtk4::Align::Start);
        title_stack.add_named(&title_label, Some("label"));

        // Centered OSC title label (updated by terminal backends via callback).
        // Placement handled by CenterBox — just style + ellipsize here.
        let osc_title_label = gtk4::Label::new(None);
        osc_title_label.add_css_class("panel-osc-title");
        osc_title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        osc_title_label.set_max_width_chars(OSC_TITLE_MAX_WIDTH_CHARS);
        osc_title_label.set_visible(false);

        // "Command running" activity indicator driven by OSC 133;A/C shell
        // integration. Hidden at the prompt; shown while a foreground command
        // is executing in the panel's shell.
        let status_icon = gtk4::Image::from_icon_name("media-record-symbolic");
        status_icon.add_css_class("panel-status-icon");
        status_icon.set_pixel_size(10);
        status_icon.set_tooltip_text(Some("Command running"));
        status_icon.set_visible(false);

        let title_entry = gtk4::Entry::new();
        title_entry.set_text(name);
        title_entry.add_css_class("panel-title-edit");
        title_stack.add_named(&title_entry, Some("entry"));

        title_stack.set_visible_child_name("label");

        // Double-click on title label → switch to edit mode
        {
            let stack = title_stack.clone();
            let entry = title_entry.clone();
            let label = title_label.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(1);
            gesture.set_propagation_phase(gtk4::PropagationPhase::Bubble);
            gesture.connect_released(move |g, n_press, _, _| {
                if n_press == 2 {
                    entry.set_text(&label.text());
                    stack.set_visible_child_name("entry");
                    entry.grab_focus();
                    g.set_state(gtk4::EventSequenceState::Claimed);
                }
                // Single clicks: don't claim — let them propagate to window (for fullscreen etc.)
            });
            title_stack.add_controller(gesture);
        }

        // Enter on entry → confirm rename
        {
            let stack = title_stack.clone();
            let label = title_label.clone();
            let cb_ref = action_cb_ref.clone();
            let pid = panel_id.to_string();
            title_entry.connect_activate(move |entry| {
                let new_name = entry.text().to_string();
                if !new_name.trim().is_empty() {
                    label.set_text(&new_name);
                    // Notify via action callback — use Configure with the new name
                    if let Ok(borrowed) = cb_ref.try_borrow() {
                        if let Some(ref cb) = *borrowed {
                            cb(&pid, PanelAction::Rename(new_name));
                        }
                    }
                }
                stack.set_visible_child_name("label");
            });
        }

        // Escape on entry → cancel
        {
            let stack = title_stack.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    stack.set_visible_child_name("label");
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            title_entry.add_controller(key_ctrl);
        }

        // Sync button
        let sync_button = gtk4::Button::new();
        sync_button.set_icon_name("media-playlist-consecutive-symbolic");
        sync_button.add_css_class("flat");
        sync_button.add_css_class("panel-action-btn");
        sync_button.set_tooltip_text(Some("Toggle sync input (Ctrl+Shift+S)"));
        {
            let cb_ref = action_cb_ref.clone();
            let pid = panel_id.to_string();
            sync_button.connect_clicked(move |_| {
                if let Ok(borrowed) = cb_ref.try_borrow() {
                    if let Some(ref cb) = *borrowed {
                        cb(&pid, PanelAction::Sync);
                    }
                }
            });
        }

        // Zoom button
        let zoom_button = gtk4::Button::new();
        zoom_button.set_icon_name("view-fullscreen-symbolic");
        zoom_button.add_css_class("flat");
        zoom_button.add_css_class("panel-action-btn");
        zoom_button.set_tooltip_text(Some("Toggle zoom (Ctrl+Shift+Z)"));
        {
            let cb_ref = action_cb_ref.clone();
            let pid = panel_id.to_string();
            zoom_button.connect_clicked(move |_| {
                if let Ok(borrowed) = cb_ref.try_borrow() {
                    if let Some(ref cb) = *borrowed {
                        cb(&pid, PanelAction::Zoom);
                    }
                }
            });
        }

        // Command history button — visible only when backend is a terminal.
        let history_button = gtk4::Button::new();
        history_button.set_icon_name("utilities-terminal-symbolic");
        history_button.add_css_class("flat");
        history_button.add_css_class("panel-action-btn");
        history_button.set_tooltip_text(Some("Commands"));
        history_button.set_visible(false);

        // Wire the click handler ONCE here. The closure checks at click time
        // whether the current backend is a terminal — set_backend only toggles
        // set_visible, never (re-)connects this handler.
        {
            let backend_ref = backend.clone();
            history_button.connect_clicked(move |btn| {
                let (panel_uuid, input_cb) = match backend_ref.try_borrow() {
                    Ok(borrowed) => match &*borrowed {
                        Some(b) if b.panel_type() == "terminal" => {
                            let uuid = b.panel_uuid();
                            let inner_ref = backend_ref.clone();
                            // History paste targets THIS panel only by design:
                            // we go directly through `write_input` instead of
                            // the synced-input path, so a synced sibling does
                            // not also receive the recalled command.
                            let cb: crate::panels::PanelInputCallback =
                                std::rc::Rc::new(move |bytes: &[u8]| {
                                    if let Ok(bb) = inner_ref.try_borrow() {
                                        if let Some(ref be) = *bb {
                                            be.write_input(bytes);
                                        }
                                    }
                                });
                            (uuid, Some(cb))
                        }
                        _ => (None, None),
                    },
                    Err(_) => (None, None),
                };
                let Some(uuid) = panel_uuid else {
                    return;
                };
                let Some(input_cb) = input_cb else {
                    return;
                };
                let popover = crate::dialogs::command_history::build_command_history_popover(
                    &uuid.simple().to_string(),
                    input_cb,
                );
                popover.set_parent(btn);
                popover.connect_closed(|popover| {
                    if popover.parent().is_some() {
                        popover.unparent();
                    }
                });
                popover.popup();
            });
        }

        // ⋮ menu button
        let menu_button = gtk4::MenuButton::new();
        menu_button.set_icon_name("view-more-symbolic");
        menu_button.add_css_class("flat");
        menu_button.add_css_class("panel-menu-btn");
        menu_button.set_tooltip_text(Some("Panel actions"));

        // Build popover menu
        let popover = build_panel_menu(panel_id, action_cb, None);
        menu_button.set_popover(Some(&popover));

        // Rebuild the menu on every ⋮ click so Move items reflect the
        // current layout (which the static popover above can't see).
        {
            let panel_id_c = panel_id.to_string();
            let action_ref = action_cb_ref.clone();
            let sib_ref = sibling_info_provider_ref.clone();
            menu_button.set_create_popup_func(move |btn| {
                let action_cb = action_ref.borrow().clone();
                let sibling_info = sib_ref.borrow().as_ref().and_then(|f| f(&panel_id_c));
                let popover = build_panel_menu(&panel_id_c, action_cb, sibling_info);
                btn.set_popover(Some(&popover));
            });
        }

        // SSH indicator (hidden by default, shown for remote panels)
        let ssh_indicator = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
        ssh_indicator.set_visible(false);
        ssh_indicator.set_margin_end(4);
        {
            let ssh_icon = gtk4::Image::from_icon_name("network-server-symbolic");
            ssh_icon.set_pixel_size(12);
            ssh_indicator.append(&ssh_icon);
            let ssh_lbl = gtk4::Label::new(None);
            ssh_lbl.add_css_class("caption");
            ssh_lbl.add_css_class("dim-label");
            ssh_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            ssh_indicator.append(&ssh_lbl);
        }

        // Layout: start=[icon][ssh][title], center=osc_title, end=[sync][zoom][menu]
        let start_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        start_box.append(&type_icon);
        start_box.append(&ssh_indicator);
        start_box.append(&title_stack);

        let end_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        end_box.append(&sync_button);
        end_box.append(&zoom_button);
        end_box.append(&history_button);
        end_box.append(&menu_button);

        // Center slot holds the OSC 0/2 title pushed by the shell. The
        // OSC 133 "command running" status_icon used to live here too but
        // it duplicates the per-tab indicator and is just visual noise on
        // top of the title text — keep the field for set_status_callback
        // so listeners (tab labels) still get notified, but don't show it
        // in the panel header itself.
        let center_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        center_box.append(&osc_title_label);

        title_bar.set_start_widget(Some(&start_box));
        title_bar.set_center_widget(Some(&center_box));
        title_bar.set_end_widget(Some(&end_box));

        // Click on title bar → focus this panel
        {
            let cb_ref = action_cb_ref.clone();
            let pid = panel_id.to_string();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(1);
            gesture.set_propagation_phase(gtk4::PropagationPhase::Bubble);
            gesture.connect_released(move |_, n_press, _, _| {
                if n_press == 1 {
                    if let Ok(borrowed) = cb_ref.try_borrow() {
                        if let Some(ref cb) = *borrowed {
                            cb(&pid, PanelAction::Focus);
                        }
                    }
                }
            });
            title_bar.add_controller(gesture);
        }

        container.append(&title_bar);

        // Footer bar
        let footer_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        footer_bar.add_css_class("panel-footer-bar");
        let footer_label = gtk4::Label::new(None);
        footer_label.set_use_markup(true);
        footer_label.add_css_class("panel-footer");
        footer_label.set_halign(gtk4::Align::Start);
        footer_label.set_hexpand(true);
        footer_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        footer_label.set_margin_start(6);
        footer_bar.append(&footer_label);
        footer_bar.set_visible(false); // Hidden until content is set

        // Collapsed view: shown when panel is minimized — expand arrow, name in tooltip
        let collapsed_view = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        collapsed_view.set_halign(gtk4::Align::Fill);
        collapsed_view.set_valign(gtk4::Align::Fill);
        collapsed_view.set_vexpand(true);
        collapsed_view.set_hexpand(true);
        collapsed_view.set_visible(false);
        collapsed_view.add_css_class("panel-collapsed-overlay");
        // Default arrow — updated by drag-collapse based on orientation.
        let collapsed_icon = gtk4::Image::from_icon_name("go-next-symbolic");
        collapsed_icon.set_pixel_size(COLLAPSED_ICON_SIZE);
        collapsed_icon.set_halign(gtk4::Align::Center);
        collapsed_icon.set_valign(gtk4::Align::Center);
        collapsed_icon.set_can_target(false);
        let collapsed_chip = gtk4::CenterBox::new();
        collapsed_chip.add_css_class("panel-collapsed-chip");
        collapsed_chip.set_size_request(COLLAPSED_CHROME_SIZE, COLLAPSED_CHROME_SIZE);
        collapsed_chip.set_halign(gtk4::Align::Fill);
        collapsed_chip.set_valign(gtk4::Align::Fill);
        collapsed_chip.set_hexpand(true);
        collapsed_chip.set_vexpand(true);
        collapsed_chip.set_center_widget(Some(&collapsed_icon));
        collapsed_view.append(&collapsed_chip);
        collapsed_view.set_tooltip_text(Some(&format!("Click to expand: {}", name)));

        let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        outer.append(&container);
        outer.append(&collapsed_view);
        outer.append(&footer_bar);
        // Clip backend content to the rounded panel frame so inner widgets
        // (editor/terminal/footer) do not visually square off the corners.
        outer.set_overflow(gtk4::Overflow::Hidden);
        {
            let cb_ref = action_cb_ref.clone();
            let pid = panel_id.to_string();
            let container_ref = container.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(1);
            gesture.connect_released(move |g, _, _, _| {
                if container_ref.is_visible() {
                    return;
                }
                if let Ok(borrowed) = cb_ref.try_borrow() {
                    if let Some(ref cb) = *borrowed {
                        cb(&pid, PanelAction::Collapse);
                    }
                }
                g.set_state(gtk4::EventSequenceState::Claimed);
            });
            outer.add_controller(gesture);
        }
        outer.add_css_class("panel-frame");
        outer.add_css_class("panel-unfocused");
        outer.set_widget_name(panel_id);
        outer.set_size_request(COLLAPSE_SIZE, COLLAPSE_SIZE);

        // Click anywhere in the panel → focus it
        {
            let cb_ref = action_cb_ref.clone();
            let pid = panel_id.to_string();
            let container_ref = container.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(1);
            gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
            gesture.connect_pressed(move |g, _, _, _| {
                if container_ref.is_visible() {
                    if let Ok(borrowed) = cb_ref.try_borrow() {
                        if let Some(ref cb) = *borrowed {
                            cb(&pid, PanelAction::Focus);
                        }
                    }
                }
                // Explicitly deny the sequence so child gestures (Button::clicked
                // inside note cards, VTE, TextView) are not held pending waiting
                // for this observational gesture to decide. Leaving it in the
                // "None" state caused clicks on card buttons to be recognized
                // only occasionally — a classic gesture-claiming race.
                g.set_state(gtk4::EventSequenceState::Denied);
            });
            outer.add_controller(gesture);
        }

        let widget = outer.clone().upcast::<gtk4::Widget>();

        Self {
            outer,
            container,
            _title_bar: title_bar,
            type_icon,
            title_label,
            osc_title_label,
            status_icon,
            sync_button,
            zoom_button,
            history_button,
            menu_button,
            collapsed_view: collapsed_view.upcast(),
            collapsed_icon,
            ssh_indicator,
            footer_bar,
            footer_label,
            widget,
            panel_id: panel_id.to_string(),
            backend,
            focused: RefCell::new(false),
            action_cb_ref,
            sibling_info_provider_ref,
            sync_input_cb_ref: Rc::new(RefCell::new(None)),
            status_listeners: Rc::new(RefCell::new(Vec::new())),
            last_waiting: Rc::new(std::cell::Cell::new(false)),
        }
    }

    /// Register an observer invoked when the panel's waiting state changes.
    /// The observer is also invoked immediately with the current state so
    /// late-registered mirrors (e.g. from a layout rebuild) stay in sync.
    pub fn add_status_listener(&self, cb: Box<dyn Fn(bool)>) {
        cb(self.last_waiting.get());
        self.status_listeners.borrow_mut().push(cb);
    }

    /// Update the action callback (rebuilds the popover menu; buttons use shared ref automatically).
    pub fn set_action_callback(&self, cb: PanelActionCallback) {
        // Use try_borrow_mut to avoid panic if called during a button click handler
        if let Ok(mut r) = self.action_cb_ref.try_borrow_mut() {
            *r = Some(cb.clone());
        }
        let popover = build_panel_menu(&self.panel_id, Some(cb), None);
        self.menu_button.set_popover(Some(&popover));
    }

    /// Install a closure that the menu uses (on each ⋮ open) to compute
    /// the current panel's `SiblingInfo`. Pass through the `WorkspaceView`
    /// so the rebuilt menu reflects the live layout.
    pub fn set_sibling_info_provider(&self, provider: SiblingInfoProvider) {
        if let Ok(mut r) = self.sibling_info_provider_ref.try_borrow_mut() {
            *r = Some(provider);
        }
    }

    /// Shut down the current backend (terminate child processes).
    /// Call before dropping the host or when explicit cleanup is needed.
    pub fn shutdown_backend(&self) {
        if let Ok(current) = self.backend.try_borrow() {
            if let Some(ref backend) = *current {
                backend.shutdown();
            }
        }
    }

    /// If the current backend asks for a confirmation dialog before close,
    /// return its prompt text. Callers show the dialog and, on confirm, call
    /// `close_focused` on the owning `WorkspaceView`.
    pub fn close_confirmation(&self) -> Option<String> {
        if let Ok(current) = self.backend.try_borrow() {
            if let Some(ref backend) = *current {
                return backend.close_confirmation();
            }
        }
        None
    }

    /// Shutdown + permanent-close signal. Used only on user-initiated close
    /// (not on backend swaps) so backends can delete per-instance persisted
    /// state.
    pub fn permanent_close_backend(&self) {
        if let Ok(current) = self.backend.try_borrow() {
            if let Some(ref backend) = *current {
                backend.shutdown();
                backend.on_permanent_close();
            }
        }
    }

    /// Set the panel backend, placing its widget inside this host.
    /// If a backend is already set, shuts it down and removes the old widget first.
    pub fn set_backend(&self, backend: Box<dyn PanelBackend>) {
        // Shut down and remove old backend widget if present
        {
            if let Ok(current) = self.backend.try_borrow() {
                if let Some(ref old) = *current {
                    old.shutdown();
                    let old_widget = old.widget().clone();
                    drop(current); // Release borrow before remove
                    self.container.remove(&old_widget);
                }
            }
        }
        self.footer_bar.set_visible(false);

        let panel_widget = backend.widget().clone();
        panel_widget.set_vexpand(true);
        panel_widget.set_hexpand(true);
        self.container.append(&panel_widget);

        // OSC 7 footer is now driven by `set_cwd_callback` (registered
        // below) — backend-agnostic. Old VTE-only downcast helper removed.

        // Show SSH indicator if backend is remote
        if let Some(ssh_label) = backend.ssh_label() {
            self.set_ssh_indicator(Some(&ssh_label));
        } else {
            self.set_ssh_indicator(None);
        }

        // Static footer (file path for document panels, project dir for code
        // editor). Terminals drive their footer dynamically via OSC 7 and
        // return None here, so we skip them.
        if let Some(footer) = backend.footer_text() {
            if !footer.is_empty() {
                self.set_footer(&footer);
            }
        }

        // Show the sync toggle only for backends that opt in. Notes,
        // chooser, and other passive panels keep the title bar clean.
        // Resetting the visual state on each backend swap also means a
        // panel that was synced and then converted to a non-sync type
        // doesn't carry its old `sync-active` styling.
        let supports_sync = backend.supports_sync();
        self.sync_button.set_visible(supports_sync);
        if !supports_sync {
            self.sync_button.remove_css_class("sync-active");
        }
        if supports_sync {
            if let Ok(borrowed) = self.sync_input_cb_ref.try_borrow() {
                backend.set_input_callback(borrowed.clone());
            }
        }

        // Reset any leftover OSC title from a previous backend and wire the
        // new backend to push title updates into the centered label. The Label
        // widget is a GObject — cloning bumps a refcount, no cycle with Self.
        self.set_osc_title("");
        let osc_label = self.osc_title_label.clone();
        let title_cb: PanelTitleCallback = Rc::new(move |t: &str| {
            let sanitized = sanitize_osc_title(t);
            if sanitized.is_empty() {
                osc_label.set_text("");
                osc_label.set_tooltip_text(None);
                osc_label.set_visible(false);
            } else {
                osc_label.set_text(&sanitized);
                osc_label.set_tooltip_text(Some(&sanitized));
                osc_label.set_visible(true);
            }
        });
        backend.set_title_callback(Some(title_cb));

        // Reset waiting state on backend swap; wire OSC 133 updates.
        self.set_waiting(false);
        self.last_waiting.set(false);
        let status_icon = self.status_icon.clone();
        let listeners = self.status_listeners.clone();
        let last_waiting = self.last_waiting.clone();
        let status_cb: PanelStatusCallback = Rc::new(move |waiting: bool| {
            status_icon.set_visible(waiting);
            last_waiting.set(waiting);
            for l in listeners.borrow().iter() {
                l(waiting);
            }
        });
        backend.set_status_callback(Some(status_cb));

        // OSC 7 current-directory-uri → footer bar. Same formatter for VTE
        // (signal-driven) and PTY (byte-scanner driven) backends.
        let footer_label = self.footer_label.clone();
        let footer_bar = self.footer_bar.clone();
        let user = std::env::var("USER").unwrap_or_default();
        let hostname = std::env::var("HOSTNAME")
            .or_else(|_| std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()))
            .unwrap_or_else(|_| "localhost".to_string());
        let home = std::env::var("HOME").unwrap_or_default();
        let cwd_cb: PanelCwdCallback = Rc::new(move |uri: &str| {
            if uri.is_empty() {
                footer_bar.set_visible(false);
                return;
            }
            if let Some(fmt) =
                crate::panels::terminal::format_cwd_footer(uri, &user, &hostname, &home)
            {
                footer_label.set_markup(&fmt.markup);
                footer_label.set_tooltip_text(Some(&fmt.plain));
                footer_bar.set_visible(true);
            }
        });
        backend.set_cwd_callback(Some(cwd_cb));

        *self.backend.borrow_mut() = Some(backend);

        // Terminal-registry hookup: expose this panel to the markdown
        // notebook "Send to terminal" picker. Re-registered on every
        // backend swap so the entry reflects the *current* backend; the
        // `send` closure routes through the same Rc<RefCell> the host
        // owns, so a future swap is reflected without re-register.
        let panel_type = self
            .backend
            .borrow()
            .as_ref()
            .map(|b| b.panel_type().to_string())
            .unwrap_or_default();
        self.history_button.set_visible(panel_type == "terminal");
        if panel_type == "terminal" {
            let backend_for_send = self.backend.clone();
            let send: Rc<dyn Fn(&[u8]) -> bool> = Rc::new(move |data| {
                backend_for_send
                    .borrow()
                    .as_ref()
                    .map(|b| b.write_input(data))
                    .unwrap_or(false)
            });
            let label = self.title_label.text().to_string();
            crate::panels::terminal_registry::register(&self.panel_id, &label, send);
        } else {
            crate::panels::terminal_registry::unregister(&self.panel_id);
        }
    }

    pub fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }

    pub fn panel_id(&self) -> &str {
        &self.panel_id
    }

    pub fn set_focused(&self, focused: bool) {
        *self.focused.borrow_mut() = focused;
        update_ancestor_workspace_tabs_focus_path(&self.outer.clone().upcast(), focused);
        if focused {
            self.outer.add_css_class("panel-focused");
            self.outer.remove_css_class("panel-unfocused");
            // Visual focus indicator removed — keeping it clean
            if let Some(ref backend) = *self.backend.borrow() {
                backend.on_focus();
            }
        } else {
            self.outer.remove_css_class("panel-focused");
            self.outer.add_css_class("panel-unfocused");
            if let Some(ref backend) = *self.backend.borrow() {
                backend.on_blur();
            }
        }
    }

    pub fn set_alert_border(&self, color: &str) {
        self.outer.remove_css_class("alert-red");
        self.outer.remove_css_class("alert-yellow");
        self.outer.remove_css_class("alert-green");
        self.outer.add_css_class(&format!("alert-{}", color));
    }

    pub fn clear_alert_border(&self) {
        self.outer.remove_css_class("alert-red");
        self.outer.remove_css_class("alert-yellow");
        self.outer.remove_css_class("alert-green");
    }

    pub fn set_title(&self, title: &str) {
        self.title_label.set_text(title);
    }

    /// Show or hide the "waiting for input" indicator. Driven by OSC 133
    /// shell integration: true on prompt start (A), false on command start (C).
    pub fn set_waiting(&self, waiting: bool) {
        self.status_icon.set_visible(waiting);
    }

    /// Update the centered OSC title label.
    ///
    /// Control characters (C0 except tab, and DEL) are stripped and the
    /// payload is truncated to `MAX_OSC_TITLE_LEN` chars. Empty input hides
    /// the label so non-terminal panels keep a clean header.
    pub fn set_osc_title(&self, raw: &str) {
        let sanitized = sanitize_osc_title(raw);
        if sanitized.is_empty() {
            self.osc_title_label.set_text("");
            self.osc_title_label.set_tooltip_text(None);
            self.osc_title_label.set_visible(false);
        } else {
            self.osc_title_label.set_text(&sanitized);
            self.osc_title_label.set_tooltip_text(Some(&sanitized));
            self.osc_title_label.set_visible(true);
        }
    }

    /// Hide/show the title name and icon (when panel is inside a tab — already shown there).
    /// Keeps the action buttons (sync, zoom, menu) visible.
    pub fn set_title_visible(&self, visible: bool) {
        self.type_icon.set_visible(visible);
        // Hide the title stack (contains label + entry)
        if let Some(parent) = self.title_label.parent() {
            parent.set_visible(visible);
        }
    }

    /// Update the panel type icon.
    pub fn set_type_icon(&self, panel_type: &str) {
        let icon_name = match panel_type {
            "terminal" => "utilities-terminal-symbolic",
            "markdown" => "text-x-generic-symbolic",
            "code_editor" => "accessories-text-editor-symbolic",
            "docker_help" => "applications-system-symbolic",
            _ => "radio-symbolic", // Empty/chooser — dot
        };
        self.type_icon.set_icon_name(Some(icon_name));
        self.collapsed_icon.set_icon_name(Some(icon_name));
        if panel_type == "terminal" {
            self.footer_bar.add_css_class("terminal-panel-footer");
        } else {
            self.footer_bar.remove_css_class("terminal-panel-footer");
        }
    }

    /// Show or hide the SSH connection indicator in the title bar.
    pub fn set_ssh_indicator(&self, label: Option<&str>) {
        if let Some(text) = label {
            self.ssh_indicator.set_visible(true);
            // Update the label (second child of ssh_indicator)
            if let Some(icon) = self.ssh_indicator.first_child() {
                if let Some(lbl_widget) = icon.next_sibling() {
                    if let Some(lbl) = lbl_widget.downcast_ref::<gtk4::Label>() {
                        lbl.set_text(text);
                    }
                }
            }
        } else {
            self.ssh_indicator.set_visible(false);
        }
    }

    /// Update zoom button visual state and icon.
    pub fn set_zoom_active(&self, active: bool) {
        if active {
            self.zoom_button.set_icon_name("view-restore-symbolic");
            self.zoom_button.add_css_class("zoom-active");
        } else {
            self.zoom_button.set_icon_name("view-fullscreen-symbolic");
            self.zoom_button.remove_css_class("zoom-active");
        }
    }

    /// Expand a collapsed panel. Called from click on collapsed_view overlay.
    /// Finds the parent Paned and restores to 50%.
    pub fn expand_collapsed(&self) {
        if !self.is_collapsed() {
            return;
        }

        // Find first parent Paned
        let mut widget = self.outer.parent();
        while let Some(w) = widget {
            if let Some(paned) = w.downcast_ref::<gtk4::Paned>() {
                let total = if paned.orientation() == gtk4::Orientation::Horizontal {
                    paned.allocation().width()
                } else {
                    paned.allocation().height()
                };
                if total > 0 {
                    paned.set_position(total / 2);
                }
                paned.set_shrink_start_child(true);
                paned.set_shrink_end_child(true);
                break;
            }
            widget = w.parent();
        }

        self.collapsed_view.set_visible(false);
        self.container.set_visible(true);
        self.footer_bar
            .set_visible(!self.footer_label.text().is_empty());
        self.outer.set_size_request(-1, -1);
    }

    /// Whether the panel is collapsed (container hidden).
    pub fn is_collapsed(&self) -> bool {
        !self.container.is_visible()
    }

    /// Update sync button visual state.
    pub fn set_sync_active(&self, active: bool) {
        if active {
            self.sync_button.add_css_class("sync-active");
        } else {
            self.sync_button.remove_css_class("sync-active");
        }
    }

    /// Set the input callback used for sync propagation.
    pub fn set_sync_input_callback(&self, cb: Rc<dyn Fn(&str, &[u8])>) {
        let wrapped = wrap_panel_input_callback(&self.panel_id, cb);
        *self.sync_input_cb_ref.borrow_mut() = Some(wrapped.clone());
        if let Some(ref backend) = *self.backend.borrow() {
            backend.set_input_callback(Some(wrapped));
        }
    }

    pub fn clear_sync_input_callback(&self) {
        *self.sync_input_cb_ref.borrow_mut() = None;
        if let Some(ref backend) = *self.backend.borrow() {
            backend.set_input_callback(None);
        }
    }

    /// Set footer text (e.g. user@host:directory). Empty string hides the footer.
    pub fn set_footer(&self, text: &str) {
        if text.is_empty() {
            self.footer_bar.set_visible(false);
        } else {
            self.footer_label.set_text(text);
            self.footer_label.set_tooltip_text(Some(text));
            self.footer_bar.set_visible(true);
        }
    }

    pub fn write_input(&self, data: &[u8]) -> bool {
        if let Some(ref backend) = *self.backend.borrow() {
            backend.write_input(data)
        } else {
            false
        }
    }

    pub fn accepts_input(&self) -> bool {
        if let Some(ref backend) = *self.backend.borrow() {
            backend.accepts_input()
        } else {
            false
        }
    }

    /// Whether the currently-installed backend opts into the sync-input
    /// feature. Used by `WorkspaceView` to avoid adding panels (notes,
    /// chooser, etc.) to the synced group via Ctrl+Shift+S.
    pub fn backend_supports_sync(&self) -> bool {
        if let Some(ref backend) = *self.backend.borrow() {
            backend.supports_sync()
        } else {
            false
        }
    }
}

/// Build the ⋮ popover menu with panel actions.
fn build_panel_menu(
    panel_id: &str,
    action_cb: Option<PanelActionCallback>,
    sibling_info: Option<crate::layout_ops::SiblingInfo>,
) -> gtk4::Popover {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_margin_top(4);
    vbox.set_margin_bottom(4);
    vbox.set_margin_start(4);
    vbox.set_margin_end(4);

    let mut items: Vec<(&str, &str, PanelAction)> = vec![
        ("Configure…", "Panel settings", PanelAction::Configure),
        ("Split Horizontal", "New panel below", PanelAction::SplitH),
        (
            "Split Vertical",
            "New panel to the right",
            PanelAction::SplitV,
        ),
        ("Add Tab", "New panel as tab", PanelAction::AddTab),
        (
            "Add Panel Before",
            "Insert panel before this one in parent split",
            PanelAction::InsertBefore,
        ),
        (
            "Add Panel After",
            "Insert panel after this one in parent split",
            PanelAction::InsertAfter,
        ),
        ("Reset Panel", "Reset to type chooser", PanelAction::Reset),
        ("Close Panel", "Close this panel", PanelAction::Close),
    ];

    // Append Move items based on the current parent kind + position.
    // Only directions with a valid target are shown — no disabled rows.
    if let Some(info) = sibling_info {
        use crate::layout_ops::SiblingKind;
        match info.kind {
            SiblingKind::Hsplit | SiblingKind::Tabs => {
                if info.index > 0 {
                    items.push((
                        "Move Left",
                        "Swap with previous sibling",
                        PanelAction::MoveLeft,
                    ));
                }
                if info.index + 1 < info.len {
                    items.push((
                        "Move Right",
                        "Swap with next sibling",
                        PanelAction::MoveRight,
                    ));
                }
            }
            SiblingKind::Vsplit => {
                if info.index > 0 {
                    items.push(("Move Up", "Swap with previous sibling", PanelAction::MoveUp));
                }
                if info.index + 1 < info.len {
                    items.push(("Move Down", "Swap with next sibling", PanelAction::MoveDown));
                }
            }
        }
    }

    let popover = gtk4::Popover::new();
    let pid = panel_id.to_string();

    for (label, tooltip, action) in items {
        let btn = gtk4::Button::new();
        btn.add_css_class("flat");
        btn.add_css_class("app-popover-button");
        btn.set_tooltip_text(Some(tooltip));

        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let icon_name = match action {
            PanelAction::Configure => "preferences-system-symbolic",
            PanelAction::SplitH => "view-dual-symbolic",
            PanelAction::SplitV => "view-dual-symbolic",
            PanelAction::AddTab => "tab-new-symbolic",
            PanelAction::InsertBefore => "list-add-symbolic",
            PanelAction::InsertAfter => "list-add-symbolic",
            PanelAction::Reset => "edit-clear-symbolic",
            PanelAction::Close => "window-close-symbolic",
            PanelAction::AddTabToNotebook => "tab-new-symbolic",
            PanelAction::RemoveTab => "window-close-symbolic",
            PanelAction::BeginTabEdit { .. } => "document-edit-symbolic",
            PanelAction::UpdateTabDraft { .. } => "document-edit-symbolic",
            PanelAction::PreviewTabMove { offset, .. } if offset < 0 => "go-previous-symbolic",
            PanelAction::PreviewTabMove { .. } => "go-next-symbolic",
            PanelAction::CommitTabEdit { .. } => "object-select-symbolic",
            PanelAction::CancelTabEdit { .. } => "process-stop-symbolic",
            PanelAction::Zoom => "view-fullscreen-symbolic",
            PanelAction::Sync => "media-playlist-consecutive-symbolic",
            PanelAction::Rename(_) => "document-edit-symbolic",
            PanelAction::RenameTab(_) => "document-edit-symbolic",
            PanelAction::Collapse => "go-previous-symbolic",
            PanelAction::Focus => "radio-symbolic",
            PanelAction::MoveLeft => "go-previous-symbolic",
            PanelAction::MoveRight => "go-next-symbolic",
            PanelAction::MoveUp => "go-up-symbolic",
            PanelAction::MoveDown => "go-down-symbolic",
        };
        let icon = gtk4::Image::from_icon_name(icon_name);
        let lbl = gtk4::Label::new(Some(label));
        lbl.set_halign(gtk4::Align::Start);
        lbl.set_hexpand(true);

        // Add shortcut hint
        let hint_text = match action {
            PanelAction::Configure => "",
            PanelAction::SplitH => "Ctrl+Shift+H",
            PanelAction::SplitV => "Ctrl+Shift+J",
            PanelAction::AddTab => "Ctrl+Shift+T",
            PanelAction::InsertBefore => "",
            PanelAction::InsertAfter => "",
            PanelAction::Reset => "",
            PanelAction::Close => "Ctrl+Shift+W",
            PanelAction::AddTabToNotebook => "",
            PanelAction::RemoveTab => "",
            PanelAction::BeginTabEdit { .. } => "Dbl-click",
            PanelAction::UpdateTabDraft { .. } => "",
            PanelAction::PreviewTabMove { .. } => "",
            PanelAction::CommitTabEdit { .. } => "Enter",
            PanelAction::CancelTabEdit { .. } => "Esc",
            PanelAction::Zoom => "Ctrl+Shift+Z",
            PanelAction::Sync => "Ctrl+Shift+S",
            PanelAction::Rename(_) => "Dbl-click",
            PanelAction::RenameTab(_) => "Dbl-click",
            PanelAction::Collapse => "",
            PanelAction::Focus => "",
            PanelAction::MoveLeft => "",
            PanelAction::MoveRight => "",
            PanelAction::MoveUp => "",
            PanelAction::MoveDown => "",
        };
        let hint = gtk4::Label::new(Some(hint_text));
        hint.add_css_class("dim-label");
        hint.set_halign(gtk4::Align::End);

        btn_box.append(&icon);
        btn_box.append(&lbl);
        btn_box.append(&hint);
        btn.set_child(Some(&btn_box));

        let cb = action_cb.clone();
        let id = pid.clone();
        let pop = popover.clone();
        let act = action.clone();
        btn.connect_clicked(move |_| {
            pop.popdown(); // Close menu first
            if let Some(ref callback) = cb {
                callback(&id, act.clone());
            }
        });

        vbox.append(&btn);

        // Add separator after Configure
        if action == PanelAction::Configure {
            let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            sep.set_margin_top(4);
            sep.set_margin_bottom(4);
            vbox.append(&sep);
        }
    }

    crate::theme::configure_popover(&popover);
    popover.set_child(Some(&vbox));
    popover
}

/// Decode percent-encoded URI path (e.g. %20 → space).
fn wrap_panel_input_callback(
    panel_id: &str,
    cb: Rc<dyn Fn(&str, &[u8])>,
) -> crate::panels::PanelInputCallback {
    let panel_id = panel_id.to_string();
    Rc::new(move |data| cb(&panel_id, data))
}

fn update_ancestor_workspace_tabs_focus_path(widget: &gtk4::Widget, focused: bool) {
    let mut current = widget.parent();
    while let Some(parent) = current {
        if let Some(notebook) = parent.downcast_ref::<gtk4::Notebook>() {
            if notebook.has_css_class("workspace-tabs") {
                if focused {
                    notebook.add_css_class("workspace-tabs-focus-path");
                } else {
                    notebook.remove_css_class("workspace-tabs-focus-path");
                }
            }
        }
        current = parent.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn wrapped_input_callback_forwards_panel_id_and_bytes() {
        let seen = Rc::new(RefCell::new(None::<(String, Vec<u8>)>));
        let seen_clone = seen.clone();
        let cb: Rc<dyn Fn(&str, &[u8])> = Rc::new(move |panel_id, data| {
            *seen_clone.borrow_mut() = Some((panel_id.to_string(), data.to_vec()));
        });

        let wrapped = wrap_panel_input_callback("p42", cb);
        wrapped(b"ls -la");

        let payload = seen.borrow();
        let (panel_id, data) = payload.as_ref().expect("callback payload");
        assert_eq!(panel_id, "p42");
        assert_eq!(data, b"ls -la");
    }

    #[test]
    fn collapsed_visual_chrome_is_smaller_than_drag_threshold() {
        assert_eq!(COLLAPSE_SIZE, 44);
        assert_eq!(COLLAPSED_PANEL_SIZE, 22);
        assert_eq!(COLLAPSED_CHROME_SIZE, 22);
        assert_eq!(COLLAPSED_ICON_SIZE, 12);
        assert_eq!(COLLAPSED_CHROME_SIZE, COLLAPSED_PANEL_SIZE);
        assert!(COLLAPSED_PANEL_SIZE < COLLAPSE_SIZE);
        assert!(COLLAPSED_ICON_SIZE < COLLAPSED_CHROME_SIZE);
    }

    #[test]
    #[serial]
    fn panel_host_clips_contents_to_rounded_frame() {
        crate::test_support::run_on_gtk_thread(|| {
            let host = PanelHost::new("panel-1", "Panel", None);
            assert_eq!(host.outer.overflow(), gtk4::Overflow::Hidden);
        });
    }
}
