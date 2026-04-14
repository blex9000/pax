/// Pax theme system — overrides libadwaita named colors via @define-color.
use gtk4::prelude::IsA;
use std::cell::RefCell;

thread_local! {
    static CURRENT_THEME: RefCell<Theme> = RefCell::new(Theme::default());
    #[cfg(feature = "vte")]
    static VTE_TERMINALS: RefCell<Vec<vte4::Terminal>> = RefCell::new(Vec::new());
    #[cfg(feature = "sourceview")]
    static SV_BUFFERS: RefCell<Vec<sourceview5::Buffer>> = RefCell::new(Vec::new());
}

/// Set the current theme and update all registered VTE terminals and sourceview buffers.
pub fn set_current_theme(theme: Theme) {
    CURRENT_THEME.with(|cell| *cell.borrow_mut() = theme);
    #[cfg(feature = "vte")]
    apply_theme_to_all_terminals(theme);
    #[cfg(feature = "sourceview")]
    apply_theme_to_all_buffers(theme);
}

/// Get the current theme.
pub fn current_theme() -> Theme {
    CURRENT_THEME.with(|cell| *cell.borrow())
}

/// Mark app popovers with a common CSS class and disable arrows on macOS, where
/// GTK popover arrows render poorly with custom application themes.
pub fn configure_popover<P>(popover: &P)
where
    P: IsA<gtk4::Popover> + IsA<gtk4::Widget>,
{
    use gtk4::prelude::*;

    popover.add_css_class("app-popover");
    if cfg!(target_os = "macos") {
        popover.set_has_arrow(false);
    }
}

/// Mark transient app windows/dialogs so the theme can target their surfaces.
pub fn configure_dialog_window<W>(window: &W)
where
    W: IsA<gtk4::Window> + IsA<gtk4::Widget>,
{
    use gtk4::prelude::*;

    window.add_css_class("app-dialog");

    let header = libadwaita::HeaderBar::new();
    header.add_css_class("app-headerbar");
    header.set_show_end_title_buttons(true);
    header.set_show_start_title_buttons(true);

    if let Some(title) = window.title() {
        let title_label = gtk4::Label::new(Some(&title));
        title_label.add_css_class("heading");
        header.set_title_widget(Some(&title_label));
    }

    window.set_titlebar(Some(&header));
}

/// Register a VTE terminal for theme updates.
#[cfg(feature = "vte")]
pub fn register_vte_terminal(vte: &vte4::Terminal) {
    use vte4::prelude::*;
    // Apply current theme colors
    let theme = current_theme();
    if let Some((bg, fg)) = theme.terminal_colors() {
        vte.set_color_background(&bg);
        vte.set_color_foreground(&fg);
    }
    VTE_TERMINALS.with(|cell| {
        cell.borrow_mut().push(vte.clone());
    });
}

/// Apply theme colors to all registered VTE terminals, pruning dead ones.
#[cfg(feature = "vte")]
fn apply_theme_to_all_terminals(theme: Theme) {
    use gtk4::prelude::*;
    use vte4::prelude::*;
    VTE_TERMINALS.with(|cell| {
        let mut terminals = cell.borrow_mut();
        // Prune terminals whose widget has been destroyed
        terminals.retain(|vte| vte.parent().is_some());
        let colors = theme.terminal_colors();
        for vte in terminals.iter() {
            if let Some((ref bg, ref fg)) = colors {
                vte.set_color_background(bg);
                vte.set_color_foreground(fg);
            } else {
                // Reset to VTE defaults
                vte.set_color_background(&gtk4::gdk::RGBA::new(0.0, 0.0, 0.0, 1.0));
                vte.set_color_foreground(&gtk4::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0));
            }
        }
    });
}

/// Apply custom bg/fg overrides to all registered VTE terminals.
/// Called by the color customizer when the user changes bg_surface or fg_content.
#[cfg(feature = "vte")]
pub fn apply_custom_vte_colors(bg: Option<&gtk4::gdk::RGBA>, fg: Option<&gtk4::gdk::RGBA>) {
    use gtk4::prelude::*;
    use vte4::prelude::*;
    VTE_TERMINALS.with(|cell| {
        let mut terminals = cell.borrow_mut();
        terminals.retain(|vte| vte.parent().is_some());
        for vte in terminals.iter() {
            if let Some(bg) = bg {
                vte.set_color_background(bg);
            }
            if let Some(fg) = fg {
                vte.set_color_foreground(fg);
            }
        }
    });
}

/// Register a sourceview5 Buffer for theme updates.
#[cfg(feature = "sourceview")]
pub fn register_sourceview_buffer(buf: &sourceview5::Buffer) {
    use sourceview5::prelude::*;
    // Apply current scheme
    let theme = current_theme();
    let scheme_id = theme.sourceview_scheme();
    let fallback_id = theme.sourceview_scheme_fallback();
    let manager = sourceview5::StyleSchemeManager::default();
    if let Some(scheme) = manager
        .scheme(scheme_id)
        .or_else(|| manager.scheme(fallback_id))
    {
        buf.set_style_scheme(Some(&scheme));
    }
    SV_BUFFERS.with(|cell| {
        cell.borrow_mut().push(buf.clone());
    });
}

/// Apply theme to all registered sourceview buffers, pruning stale ones.
#[cfg(feature = "sourceview")]
fn apply_theme_to_all_buffers(theme: Theme) {
    use gtk4::prelude::*;
    use sourceview5::prelude::*;
    SV_BUFFERS.with(|cell| {
        let mut buffers = cell.borrow_mut();
        // Prune buffers with zero ref count (no longer in use)
        buffers.retain(|buf| buf.tag_table().size() >= 0); // always true, but keeps ref alive
        let scheme_id = theme.sourceview_scheme();
        let fallback_id = theme.sourceview_scheme_fallback();
        let manager = sourceview5::StyleSchemeManager::default();
        let scheme = manager
            .scheme(scheme_id)
            .or_else(|| manager.scheme(fallback_id));
        for buf in buffers.iter() {
            buf.set_style_scheme(scheme.as_ref());
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Theme {
    System,
    Graphite,
    Dracula,
    Aurora,
    Quantum,
}

impl Default for Theme {
    fn default() -> Self {
        Self::Graphite
    }
}

impl Theme {
    fn resolved(self) -> Self {
        match self {
            Theme::System => Theme::Graphite,
            other => other,
        }
    }

    pub fn label(&self) -> &str {
        match self.resolved() {
            Theme::Graphite => "Graphite",
            Theme::Dracula => "Dracula",
            Theme::Aurora => "Aurora",
            Theme::Quantum => "Quantum",
            Theme::System => unreachable!(),
        }
    }

    pub fn all() -> &'static [Theme] {
        &[
            Theme::Graphite,
            Theme::Dracula,
            Theme::Aurora,
            Theme::Quantum,
        ]
    }

    pub fn color_scheme(&self) -> libadwaita::ColorScheme {
        match self.resolved() {
            Theme::Aurora | Theme::Quantum => libadwaita::ColorScheme::ForceLight,
            Theme::Graphite | Theme::Dracula => libadwaita::ColorScheme::ForceDark,
            Theme::System => unreachable!(),
        }
    }

    pub fn to_id(&self) -> &str {
        match self.resolved() {
            Theme::Graphite => "graphite",
            Theme::Dracula => "dracula",
            Theme::Aurora => "aurora",
            Theme::Quantum => "quantum",
            Theme::System => unreachable!(),
        }
    }

    pub fn from_id(id: &str) -> Theme {
        match id {
            "graphite" => Theme::Graphite,
            "dracula" => Theme::Dracula,
            "aurora" => Theme::Aurora,
            "quantum" => Theme::Quantum,
            // "system", "", and any legacy/unknown id (e.g. "nord",
            // "catppuccin-*", "hologram") fall back to the default theme.
            _ => Theme::default(),
        }
    }

    /// Returns (background, foreground) RGBA for VTE terminal.
    pub fn terminal_colors(&self) -> Option<(gtk4::gdk::RGBA, gtk4::gdk::RGBA)> {
        match self.resolved() {
            Theme::Graphite => Some((
                gtk4::gdk::RGBA::new(0.059, 0.078, 0.106, 1.0), // #0f141b
                gtk4::gdk::RGBA::new(0.898, 0.925, 0.953, 1.0), // #e5ecf3
            )),
            Theme::Dracula => Some((
                gtk4::gdk::RGBA::new(0.157, 0.165, 0.212, 1.0), // #282a36
                gtk4::gdk::RGBA::new(0.973, 0.973, 0.949, 1.0), // #f8f8f2
            )),
            Theme::Aurora => Some((
                gtk4::gdk::RGBA::new(0.965, 0.976, 0.992, 1.0), // #f6f9fd
                gtk4::gdk::RGBA::new(0.102, 0.153, 0.251, 1.0), // #1a2740
            )),
            Theme::Quantum => Some((
                gtk4::gdk::RGBA::new(0.980, 0.988, 0.996, 1.0), // #fafcfe
                gtk4::gdk::RGBA::new(0.039, 0.102, 0.200, 1.0), // #0a1a33
            )),
            Theme::System => unreachable!(),
        }
    }

    /// Returns the GtkSourceView 5 style scheme ID for this theme.
    #[cfg(feature = "sourceview")]
    pub fn sourceview_scheme(&self) -> &str {
        match self.resolved() {
            Theme::Graphite => "pax-graphite",
            Theme::Dracula => "pax-dracula",
            Theme::Aurora => "pax-aurora",
            Theme::Quantum => "pax-quantum",
            Theme::System => unreachable!(),
        }
    }

    /// Fallback scheme if the primary is not available.
    #[cfg(feature = "sourceview")]
    pub fn sourceview_scheme_fallback(&self) -> &str {
        match self.resolved() {
            Theme::Graphite | Theme::Dracula => "Adwaita-dark",
            Theme::Aurora | Theme::Quantum => "Adwaita",
            Theme::System => unreachable!(),
        }
    }

    /// Returns CSS @define-color overrides for libadwaita named colors.
    pub fn css_overrides(&self) -> &str {
        match self.resolved() {
            Theme::Graphite => GRAPHITE_CSS,
            Theme::Dracula => DRACULA_CSS,
            Theme::Aurora => AURORA_CSS,
            Theme::Quantum => QUANTUM_CSS,
            Theme::System => unreachable!(),
        }
    }

    /// Optional theme-specific CSS rules (selectors + declarations) appended
    /// AFTER BASE_CSS so they override base styling. Currently no theme
    /// opts in; the hook is preserved for future structural themes.
    pub fn css_extra(&self) -> &str {
        ""
    }
}

/// Extract the raw value of `@define-color <token> <value>;` from a CSS
/// string. Returns the value portion (e.g. `"#141a22"` or `"alpha(#fff, 0.1)"`).
pub fn parse_define_color(css: &str, token: &str) -> Option<String> {
    for line in css.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("@define-color ") {
            let mut parts = rest.splitn(2, char::is_whitespace);
            if let (Some(name), Some(value)) = (parts.next(), parts.next()) {
                if name == token {
                    return Some(value.trim_end_matches(';').trim().to_string());
                }
            }
        }
    }
    None
}

/// Replace `@define-color` values in a CSS string using the given overrides map.
/// Lines whose token name appears in the map get their value swapped; other
/// lines pass through unchanged.
pub fn apply_color_overrides(base_css: &str, overrides: &std::collections::HashMap<String, String>) -> String {
    let mut result = String::with_capacity(base_css.len());
    for line in base_css.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("@define-color ") {
            if let Some(token_name) = rest.split_whitespace().next() {
                if let Some(custom_val) = overrides.get(token_name) {
                    result.push_str(&format!("@define-color {} {};", token_name, custom_val));
                    result.push('\n');
                    continue;
                }
            }
        }
        result.push_str(line);
        result.push('\n');
    }
    result
}

/// Minimal CSS — only layout, no colors.
pub const BASE_CSS: &str = "
window, .background {
  background-color: @window_bg_color;
  color: @window_fg_color;
  font-family: \"Inter\", \"SF Pro Text\", \"Segoe UI Variable\", \"Segoe UI\", \"Noto Sans\", \"Cantarell\", sans-serif;
  font-size: 11px;
}
window.app-dialog,
window.app-dialog > * {
  background-color: @headerbar_bg_color;
  color: @headerbar_fg_color;
}
toolbarview.app-toolbar-view { background-color: @window_bg_color; color: @window_fg_color; }
toolbarview.app-toolbar-view .top-bar { background-color: @headerbar_bg_color; color: @headerbar_fg_color; border-bottom: 1px solid @headerbar_border_color; min-height: 28px; padding-top: 0; padding-bottom: 0; }
toolbarview.app-toolbar-view .top-bar > * { background-color: @headerbar_bg_color; color: @headerbar_fg_color; }
headerbar.app-headerbar { background-color: @headerbar_bg_color; color: @headerbar_fg_color; border: none; min-height: 28px; padding-top: 0; padding-bottom: 0; }
headerbar.app-headerbar windowhandle { min-height: 28px; }
headerbar.app-headerbar box, headerbar.app-headerbar label, headerbar.app-headerbar image, headerbar.app-headerbar button, headerbar.app-headerbar menubutton > button { color: @headerbar_fg_color; }
box.panel-title-bar, box.panel-footer-bar, .status-bar, .markdown-toolbar { background-color: @headerbar_bg_color; color: @headerbar_fg_color; }
button,
menubutton > button,
togglebutton {
  min-height: 18px;
  min-width: 18px;
  padding: 0 2px;
}
button image,
menubutton > button image,
togglebutton image {
  -gtk-icon-size: 14px;
}
button.flat,
togglebutton.flat,
menubutton.flat > button {
  min-height: 16px;
  min-width: 16px;
  padding: 0 1px;
}
button:hover,
togglebutton:hover,
menubutton > button:hover,
button.flat:hover,
togglebutton.flat:hover,
menubutton.flat > button:hover {
  background-color: transparent;
  background-image: none;
  border-color: transparent;
  box-shadow: none;
}
button:hover image,
togglebutton:hover image,
menubutton > button:hover image {
  color: @accent_color;
}
button.flat:checked,
togglebutton.flat:checked,
menubutton.flat > button:checked {
  background-color: transparent;
  background-image: none;
  border-color: transparent;
  box-shadow: none;
  color: @accent_color;
}
button.flat:checked image,
button.flat:checked label,
togglebutton.flat:checked image,
togglebutton.flat:checked label,
menubutton.flat > button:checked image,
menubutton.flat > button:checked label {
  color: @accent_color;
}
button.flat image,
togglebutton.flat image,
menubutton.flat > button image {
  -gtk-icon-size: 13px;
}
.editor-sidebar-toolbar-surface,
.editor-sidebar-toolbar {
  background-color: @headerbar_bg_color;
  color: @headerbar_fg_color;
}
.editor-sidebar-toolbar {
  min-height: 18px;
  padding: 0 2px;
}
.editor-sidebar-toolbar button,
.editor-sidebar-toolbar togglebutton {
  min-height: 15px;
  min-width: 15px;
  padding: 0;
  margin-left: 1px;
  margin-right: 1px;
}
.editor-sidebar-toolbar button image,
.editor-sidebar-toolbar togglebutton image {
  -gtk-icon-size: 12px;
}
.editor-sidebar-toolbar togglebutton:checked {
  background-color: transparent;
  background-image: none;
  border-color: transparent;
  box-shadow: none;
  color: @accent_color;
}
.editor-sidebar-toolbar togglebutton:checked image,
.editor-sidebar-toolbar togglebutton:checked label {
  color: @accent_color;
}
.editor-file-tree-actions {
  background-color: @view_bg_color;
  color: @view_fg_color;
  border-top: 1px solid alpha(@borders, 0.4);
  border-right: none;
  border-bottom: none;
  border-left: none;
}
.editor-file-tree-header-wrap {
  background-color: @view_bg_color;
  color: @view_fg_color;
  border-top: none;
  border-right: none;
  border-bottom: 1px solid alpha(@borders, 0.4);
  border-left: none;
}
.editor-file-tree-header {
  background-color: @view_bg_color;
  color: @view_fg_color;
  border-top: none;
  border-right: none;
  border-bottom: none;
  border-left: none;
}
notebook.workspace-tabs > header > tabs > tab {
  background-color: @workspace_tabs_bar_bg_color;
  color: @headerbar_fg_color;
  border: 1px solid @headerbar_border_color;
  background-image: none;
  box-shadow: none;
}
notebook.workspace-tabs {
  background-color: transparent;
  color: @headerbar_fg_color;
  border-color: transparent;
  background-image: none;
  box-shadow: none;
}
notebook.workspace-tabs > header {
  background-color: transparent;
  border-bottom: none;
  box-shadow: none;
  min-height: 16px;
}
notebook.workspace-tabs > header > tabs {
  background-color: transparent;
  box-shadow: none;
  min-height: 14px;
  /* Match the panel-frame margin (6px) on both sides so the tab strip
     aligns with the panels below it. */
  padding-left: 6px;
  padding-right: 6px;
  /* Force tabs flush against each other so the per-tab bottom borders
     join into a single continuous line (no inherited GTK tab spacing). */
  border-spacing: 0;
}
notebook.workspace-tabs > header > tabs > tab {
  /* Inactive tabs: only a bottom border. Margin/border-spacing zeroed so
     the borders of adjacent tabs touch and form one continuous baseline. */
  border-top: none;
  border-left: none;
  border-right: none;
  border-bottom: 1px solid @headerbar_border_color;
  border-radius: 0;
  margin: 0;
  border-spacing: 0;
  min-height: 14px;
  background-color: transparent;
  padding-top: 0;
  padding-bottom: 0;
  padding-left: 8px;
  padding-right: 8px;
  box-shadow: none;
}
notebook.workspace-tabs > header > tabs > tab label {
  font-size: 10px;
  font-weight: 600;
  color: alpha(@headerbar_fg_color, 0.46);
}
notebook.workspace-tabs > header > tabs > tab image {
  -gtk-icon-size: 10px;
  color: alpha(@headerbar_fg_color, 0.44);
}
image.workspace-tab-type-icon {
  -gtk-icon-size: 13px;
  min-height: 13px;
  min-width: 13px;
  margin-top: 1px;
  color: alpha(@headerbar_fg_color, 0.56);
}
notebook.workspace-tabs > header > tabs > tab:checked label {
  color: alpha(@headerbar_fg_color, 0.96);
}
notebook.workspace-tabs > header > tabs > tab:checked image,
notebook.workspace-tabs > header > tabs > tab:checked image.workspace-tab-type-icon {
  color: @accent_color;
}
notebook.workspace-tabs > header > tabs > tab:hover {
  background-color: alpha(@accent_color, 0.08);
  border-top-left-radius: 12px;
  border-top-right-radius: 12px;
  border-bottom-left-radius: 0;
  border-bottom-right-radius: 0;
}
notebook.workspace-tabs > header > tabs > tab:checked {
  /* Active tab: arch chrome (top-left + top-right rounded), three-sided
     outline in the same gray used by unfocused panel borders. No bg fill
     so the baseline reads as the bottom edge. */
  border-top: 1px solid @headerbar_border_color;
  border-left: 1px solid @headerbar_border_color;
  border-right: 1px solid @headerbar_border_color;
  border-bottom: none;
  border-top-left-radius: 12px;
  border-top-right-radius: 12px;
  border-bottom-left-radius: 0;
  border-bottom-right-radius: 0;
  background-color: transparent;
  box-shadow: none;
}
notebook.workspace-tabs-root {
  margin-top: 6px;
  background-color: transparent;
}
notebook.workspace-tabs-nested {
  margin-top: 6px;
  background-color: transparent;
}
notebook.workspace-tabs stack,
notebook.workspace-tabs stackpage {
  background-color: transparent;
}
box.workspace-tab-page-shell {
  border-radius: 0 0 14px 14px;
  background-color: transparent;
  box-shadow: none;
}
box.workspace-tab-page-shell > box.panel-frame {
  margin: 6px;
  box-shadow: none;
}
box.workspace-tab-page-shell > box.panel-frame > box.panel-title-bar {
  border-radius: 14px 14px 0 0;
}
box.workspace-tab-page-shell > box.panel-frame > box.panel-footer-bar {
  border-radius: 0 0 14px 14px;
}
notebook.workspace-tabs > header > tabs > tab button.workspace-tab-close-btn {
  min-height: 15px;
  min-width: 15px;
  margin-right: 2px;
  opacity: 0;
}
notebook.workspace-tabs > header > tabs > tab button.workspace-tab-close-btn image {
  -gtk-icon-size: 14px;
  min-height: 14px;
  min-width: 14px;
}
notebook.workspace-tabs > header > tabs > tab:hover button.workspace-tab-close-btn {
  opacity: 0.9;
}
notebook.workspace-tabs > header > tabs > tab button.workspace-tab-close-btn:hover {
  opacity: 1.0;
}
box.workspace-tab-add-wrap {
  background-color: transparent;
  border: none;
  box-shadow: none;
  min-width: 16px;
  min-height: 16px;
}
box.workspace-tab-add-wrap > label.workspace-tab-add-label {
  min-height: 16px;
  min-width: 16px;
  font-size: 15px;
  font-weight: 800;
  opacity: 0.65;
  color: alpha(@headerbar_fg_color, 0.74);
}
notebook.workspace-tabs > header > tabs > tab:hover box.workspace-tab-add-wrap > label.workspace-tab-add-label {
  color: @accent_color;
}
checkbutton.app-popover-check {
  min-height: 22px;
  padding: 2px 4px;
}
checkbutton.app-popover-check:hover {
  color: @accent_color;
}
checkbutton.app-popover-check check {
  -gtk-icon-size: 13px;
}
button.panel-action-btn, menubutton.panel-menu-btn > button, menubutton.app-menu-btn > button, headerbar.app-headerbar button, headerbar.app-headerbar menubutton > button {
  background-image: none;
  background-color: transparent;
  border-color: transparent;
  box-shadow: none;
  margin-left: 1px;
  margin-right: 1px;
}
button.panel-action-btn:hover, menubutton.panel-menu-btn > button:hover, menubutton.app-menu-btn > button:hover, headerbar.app-headerbar button:hover, headerbar.app-headerbar menubutton > button:hover {
  background-color: transparent;
  background-image: none;
  border-color: transparent;
  box-shadow: none;
}
button.panel-action-btn:hover image,
menubutton.panel-menu-btn > button:hover image,
menubutton.app-menu-btn > button:hover image,
headerbar.app-headerbar button:hover image,
headerbar.app-headerbar menubutton > button:hover image {
  color: @accent_color;
}
button.panel-action-btn:checked, menubutton.panel-menu-btn > button:checked, menubutton.app-menu-btn > button:checked, headerbar.app-headerbar button:checked, headerbar.app-headerbar menubutton > button:checked {
  background-color: transparent;
  background-image: none;
  border-color: transparent;
  box-shadow: none;
  color: @accent_color;
}
button.panel-action-btn:checked image,
menubutton.panel-menu-btn > button:checked image,
menubutton.app-menu-btn > button:checked image,
headerbar.app-headerbar button:checked image,
headerbar.app-headerbar menubutton > button:checked image {
  color: @accent_color;
}
popover.app-popover {
  background-color: transparent;
  background-image: none;
  box-shadow: none;
}
popover.app-popover > contents {
  background-color: @popover_bg_color;
  color: @popover_fg_color;
  border: 1px solid alpha(@borders, 0.9);
  border-radius: 12px;
  box-shadow: none;
  padding: 3px;
}
popover.app-popover > arrow {
  background-color: @popover_bg_color;
  color: @popover_bg_color;
}
popover.app-popover menu,
popover.app-popover box,
popover.app-popover modelbutton,
popover.app-popover button,
popover.app-popover label,
popover.app-popover image {
  color: @popover_fg_color;
}
box.panel-frame { border: 1px solid @headerbar_border_color; border-radius: 14px; margin: 6px; padding: 0; }
box.panel-frame.panel-collapsed-placeholder {
  border: none;
  background-color: transparent;
  box-shadow: none;
  margin: 0;
  padding: 0;
}
box.panel-frame > box { margin: 0; padding: 0; }
box.panel-frame > box.panel-title-bar { border-radius: 14px 14px 0 0; }
box.panel-frame > box.panel-footer-bar { border-radius: 0 0 14px 14px; }
box.panel-title-bar { padding: 0 4px; margin: 0; min-height: 13px; border-bottom: none; }
.panel-title { font-size: 10px; font-weight: bold; }
box.panel-frame box.panel-title-bar {
  background-color: transparent;
  border-bottom: none;
}
/* Focused panels keep the same gray borders as unfocused ones — only the
   type icon switches to the accent color so focus is communicated via a
   single accent cue. */
box.panel-frame.panel-focused box.panel-title-bar,
box.panel-frame.panel-unfocused box.panel-title-bar {
  background-color: transparent;
}
box.panel-frame.panel-unfocused .panel-title,
box.panel-frame.panel-unfocused .panel-footer {
  color: alpha(@headerbar_fg_color, 0.74);
  opacity: 0.68;
}
box.panel-frame.panel-unfocused .panel-title-type-icon {
  color: alpha(@headerbar_fg_color, 0.74);
  opacity: 0.68;
}
box.panel-frame.panel-focused .panel-title,
box.panel-frame.panel-focused .panel-footer {
  color: @headerbar_fg_color;
  opacity: 1.0;
}
box.panel-frame.panel-focused .panel-title-type-icon {
  color: @accent_color;
  opacity: 1.0;
}
.panel-type-icon { min-height: 9px; min-width: 9px; opacity: 0.6; margin-right: 1px; }
.panel-title-type-icon {
  -gtk-icon-size: 13px;
  min-height: 13px;
  min-width: 13px;
  margin-left: 14px;
  margin-right: 3px;
  margin-top: 1px;
}
.panel-menu-btn { min-height: 10px; min-width: 10px; padding: 0; }
.panel-action-btn { min-height: 10px; min-width: 10px; padding: 0; opacity: 0.5; }
box.panel-frame.panel-unfocused .panel-action-btn,
box.panel-frame.panel-unfocused menubutton.panel-menu-btn > button {
  opacity: 0.28;
}
box.panel-frame.panel-focused .panel-action-btn,
box.panel-frame.panel-focused menubutton.panel-menu-btn > button {
  opacity: 0.74;
}
.panel-action-btn image,
menubutton.panel-menu-btn > button image {
  -gtk-icon-size: 10px;
}
headerbar.app-headerbar button image,
headerbar.app-headerbar menubutton > button image {
  -gtk-icon-size: 12px;
}
button.panel-action-btn,
menubutton.panel-menu-btn > button {
  min-height: 10px;
  min-width: 10px;
  padding: 0;
}
.panel-action-btn:hover { opacity: 1.0; }
.sync-active { opacity: 1.0; color: #e5a50a; }
.zoom-active { opacity: 1.0; color: #5588ff; }
.panel-focused { border: none; }
.panel-unfocused { border: none; }
.panel-type-btn { min-width: 120px; }
.panel-footer-bar { padding: 0 5px 0 8px; min-height: 13px; border-top: 1px solid alpha(@borders, 0.4); }
.panel-footer { font-size: 10px; }
box.panel-footer-bar.terminal-panel-footer {
  background-color: @terminal_bg_color;
  color: @terminal_fg_color;
  border-top: 1px solid alpha(@terminal_fg_color, 0.14);
  border-right: none;
  border-bottom: none;
  border-left: none;
}
.terminal-fallback {
  background-color: @terminal_bg_color;
  color: @terminal_fg_color;
}
box.panel-footer-bar.editor-file-preview-footer,
box.editor-file-preview-footer.panel-footer {
  background-color: @view_bg_color;
  color: @view_fg_color;
  border-top: 1px solid alpha(@borders, 0.4);
  border-right: none;
  border-bottom: none;
  border-left: none;
}
.status-bar { padding: 0 5px; min-height: 16px; }
.status-mode { font-weight: bold; padding: 0 6px; }
.markdown-panel {
  font-family: \"Inter\", \"SF Pro Text\", \"Segoe UI Variable\", \"Segoe UI\", \"Noto Sans\", \"Cantarell\", sans-serif;
  font-size: 11px;
}
.editor-code-view,
.editor-code-view text {
  font-family: \"JetBrains Mono\", \"SF Mono\", \"Cascadia Code\", \"IBM Plex Mono\", \"Fira Code\", monospace;
  font-size: 11px;
  background-color: @view_bg_color;
  color: @view_fg_color;
}
.markdown-toolbar { border-bottom: 1px solid alpha(@borders, 0.3); padding: 0 2px; }
.markdown-toolbar button,
.markdown-toolbar togglebutton {
  margin-left: 1px;
  margin-right: 1px;
}
.tab-close-btn { min-height: 15px; min-width: 15px; padding: 0; }
.tab-close-btn image {
  -gtk-icon-size: 14px;
}
.panel-collapsed-overlay {
  background-color: @panel_header_bg_color;
  color: @headerbar_fg_color;
  border: 1px solid @headerbar_border_color;
  border-radius: 14px;
  box-shadow: none;
  padding: 0;
  min-width: 0;
  min-height: 0;
}
.panel-collapsed-chip {
  background-color: transparent;
  color: @headerbar_fg_color;
  border: none;
  border-radius: 13px;
  padding: 0;
  min-width: 22px;
  min-height: 22px;
}
.panel-collapsed-chip image {
  -gtk-icon-size: 12px;
}
.panel-collapsed-drag-strip {
  background-color: transparent;
  border: none;
  padding: 0;
}
paned > separator {
  min-width: 1px;
  min-height: 1px;
  background-image: none;
  background-color: transparent;
  border: none;
  box-shadow: none;
}
.dirty-indicator { color: #ff8c00; }
.editor-tabs { border-bottom: 1px solid alpha(@borders, 0.3); background-color: @view_bg_color; color: @view_fg_color; }
.editor-welcome { background-color: @view_bg_color; color: @view_fg_color; }
.editor-sidebar { border-right: 1px solid alpha(@borders, 0.3); }
.navigation-sidebar, .boxed-list { background-color: @sidebar_bg_color; color: @sidebar_fg_color; }
.editor-file-tree,
.editor-file-tree-scroll,
.editor-file-tree-scroll viewport,
.editor-file-tree-list,
.editor-file-tree-list > row {
  background-color: @view_bg_color;
  color: @view_fg_color;
}
.editor-file-tree-list > row,
.editor-file-tree-row {
  min-height: 18px;
  padding-top: 0;
  padding-bottom: 0;
  margin-top: 0;
  margin-bottom: 0;
  border-radius: 3px;
}
.editor-file-tree-entry {
  min-height: 18px;
  padding-top: 0;
  padding-bottom: 0;
}
.editor-file-tree-list > row > box,
.editor-file-tree-list > row > box > * {
  background-color: transparent;
}
.editor-file-tree-list > row:hover {
  background-color: alpha(@view_fg_color, 0.06);
}
.editor-file-tree-entry.editor-file-tree-ignored,
.editor-file-tree-entry.editor-file-tree-ignored > label,
.editor-file-tree-entry.editor-file-tree-ignored > image,
.editor-file-tree-entry.editor-file-tree-ignored > drawingarea {
  color: alpha(@view_fg_color, 0.58);
  opacity: 0.72;
}
.editor-file-tree-list > row:selected,
.editor-file-tree-list > row:selected:hover,
.editor-file-tree-list > row:selected:focus {
  background-color: @accent_color;
  color: @accent_fg_color;
}
.editor-file-tree-list > row:selected > box,
.editor-file-tree-list > row:selected > box > label,
.editor-file-tree-list > row:selected > box > image {
  color: @accent_fg_color;
}
.editor-file-tree-list > row:selected > box.editor-file-tree-ignored,
.editor-file-tree-list > row:selected > box.editor-file-tree-ignored > label,
.editor-file-tree-list > row:selected > box.editor-file-tree-ignored > image {
  opacity: 1.0;
}
.editor-sidebar-pane,
.editor-sidebar-pane-scroll,
.editor-sidebar-pane-scroll viewport,
.editor-sidebar-pane-content,
.editor-sidebar-pane-list,
.editor-sidebar-pane-list > row {
  background-color: @view_bg_color;
  color: @view_fg_color;
}
.editor-sidebar-pane-content,
.editor-sidebar-pane-list > row > box,
.editor-sidebar-pane-list > row > box > * {
  background-color: transparent;
}
.editor-sidebar-pane-footer {
  background-color: @view_bg_color;
  color: @view_fg_color;
  border-top: 1px solid alpha(@borders, 0.4);
  border-right: none;
  border-bottom: none;
  border-left: none;
}
.card, button.card, .welcome-action-btn { background-color: @card_bg_color; color: @card_fg_color; }
entry,
spinbutton,
textview,
dropdown > button,
combobox > box > button {
  background-color: @view_bg_color;
  color: @view_fg_color;
  border-color: alpha(@borders, 0.8);
  background-image: none;
  box-shadow: none;
  min-height: 28px;
  padding: 2px 6px;
}
entry text,
spinbutton text,
textview text {
  background-color: transparent;
  color: @view_fg_color;
  border: none;
  box-shadow: none;
}
entry image,
spinbutton image,
dropdown > button label,
dropdown > button image,
combobox > box > button label,
combobox > box > button image {
  color: @view_fg_color;
}
entry:focus,
spinbutton:focus-within,
textview:focus,
dropdown > button:focus,
combobox > box > button:focus {
  border-color: @accent_color;
  box-shadow: inset 0 0 0 1px alpha(@accent_color, 0.35);
}
entry selection,
text selection,
textview text selection {
  background-color: @accent_color;
  color: @accent_fg_color;
}
button.git-has-changes, togglebutton.git-has-changes { color: #ff8c00; }
button.git-has-changes:hover, togglebutton.git-has-changes:hover { color: #ffaa33; }
popover.app-popover separator { background-color: @borders; }
popover.app-popover modelbutton,
popover.app-popover button.app-popover-button {
  background-image: none;
  background-color: transparent;
  border: none;
  border-radius: 8px;
  box-shadow: none;
  min-height: 20px;
  padding: 0 4px;
}
popover.app-popover modelbutton image,
popover.app-popover button.app-popover-button image {
  -gtk-icon-size: 13px;
}
popover.app-popover modelbutton:hover,
popover.app-popover button.app-popover-button:hover {
  background-color: transparent;
  background-image: none;
  border-color: transparent;
  box-shadow: none;
  color: @accent_color;
}
popover.app-popover modelbutton:hover image,
popover.app-popover button.app-popover-button:hover image {
  color: @accent_color;
}
popover.app-popover modelbutton:hover label,
popover.app-popover button.app-popover-button:hover label {
  color: @accent_color;
}
window.app-dialog dropdown.settings-theme-dropdown > popover.menu,
.editor-sidebar-pane dropdown > popover.menu {
  background-color: transparent;
  background-image: none;
  box-shadow: none;
}
window.app-dialog dropdown.settings-theme-dropdown > popover.menu > contents,
.editor-sidebar-pane dropdown > popover.menu > contents {
  background-color: @popover_bg_color;
  color: @popover_fg_color;
  border: 1px solid alpha(@borders, 0.9);
  border-radius: 12px;
  box-shadow: none;
  padding: 4px;
}
window.app-dialog dropdown.settings-theme-dropdown > popover.menu scrolledwindow,
window.app-dialog dropdown.settings-theme-dropdown > popover.menu viewport,
.editor-sidebar-pane dropdown > popover.menu scrolledwindow,
.editor-sidebar-pane dropdown > popover.menu viewport {
  background-color: transparent;
  background-image: none;
  border: none;
  box-shadow: none;
}
window.app-dialog dropdown.settings-theme-dropdown > popover.menu > arrow,
.editor-sidebar-pane dropdown > popover.menu > arrow {
  background-color: @popover_bg_color;
  color: @popover_bg_color;
}
window.app-dialog dropdown.settings-theme-dropdown > popover.menu listview,
window.app-dialog dropdown.settings-theme-dropdown > popover.menu row,
.editor-sidebar-pane dropdown > popover.menu listview,
.editor-sidebar-pane dropdown > popover.menu row {
  background-color: transparent;
  color: @popover_fg_color;
}
window.app-dialog dropdown.settings-theme-dropdown > popover.menu row:hover,
window.app-dialog dropdown.settings-theme-dropdown > popover.menu row:selected,
.editor-sidebar-pane dropdown > popover.menu row:hover,
.editor-sidebar-pane dropdown > popover.menu row:selected {
  background-color: alpha(@accent_color, 0.18);
  color: @popover_fg_color;
}
";

const GRAPHITE_CSS: &str = "\
@define-color bg_window #141a22;\n\
@define-color bg_chrome @bg_window;\n\
@define-color bg_surface #0f141b;\n\
@define-color bg_elevated #202938;\n\
@define-color fg_ui #e5ecf3;\n\
@define-color fg_content #e5ecf3;\n\
@define-color accent #6ea7ff;\n\
@define-color accent_bg @accent;\n\
@define-color accent_fg #0f141b;\n\
@define-color border_soft alpha(white, 0.10);\n\
@define-color border_hard alpha(white, 0.10);\n\
@define-color window_bg_color @bg_window;\n\
@define-color window_fg_color @fg_ui;\n\
@define-color headerbar_bg_color @bg_chrome;\n\
@define-color workspace_tabs_bar_bg_color @bg_chrome;\n\
@define-color headerbar_fg_color @fg_ui;\n\
@define-color view_bg_color @bg_surface;\n\
@define-color view_fg_color @fg_content;\n\
@define-color terminal_bg_color @bg_surface;\n\
@define-color terminal_fg_color @fg_content;\n\
@define-color panel_header_bg_color @bg_chrome;\n\
@define-color card_bg_color @bg_elevated;\n\
@define-color card_fg_color @fg_ui;\n\
@define-color dialog_bg_color @bg_elevated;\n\
@define-color dialog_fg_color @fg_ui;\n\
@define-color popover_bg_color @bg_elevated;\n\
@define-color popover_fg_color @fg_ui;\n\
@define-color popover_shade_color alpha(black, 0.28);\n\
@define-color sidebar_bg_color @bg_elevated;\n\
@define-color sidebar_fg_color @fg_content;\n\
@define-color secondary_sidebar_bg_color @bg_chrome;\n\
@define-color secondary_sidebar_fg_color @fg_content;\n\
@define-color thumbnail_bg_color @bg_elevated;\n\
@define-color accent_color @accent;\n\
@define-color accent_bg_color @accent_bg;\n\
@define-color accent_fg_color @accent_fg;\n\
@define-color borders @border_soft;\n\
@define-color headerbar_border_color @border_hard;\n\
@define-color headerbar_backdrop_color @bg_chrome;\n\
";

const DRACULA_CSS: &str = "\
@define-color bg_window #282a36;\n\
@define-color bg_chrome @bg_window;\n\
@define-color bg_surface #282a36;\n\
@define-color bg_elevated #44475a;\n\
@define-color fg_ui #f8f8f2;\n\
@define-color fg_content #f8f8f2;\n\
@define-color accent #bd93f9;\n\
@define-color accent_bg @accent;\n\
@define-color accent_fg #282a36;\n\
@define-color border_soft alpha(white, 0.15);\n\
@define-color border_hard alpha(white, 0.15);\n\
@define-color window_bg_color @bg_window;\n\
@define-color window_fg_color @fg_ui;\n\
@define-color headerbar_bg_color @bg_chrome;\n\
@define-color workspace_tabs_bar_bg_color @bg_chrome;\n\
@define-color headerbar_fg_color @fg_ui;\n\
@define-color view_bg_color @bg_surface;\n\
@define-color view_fg_color @fg_content;\n\
@define-color terminal_bg_color @bg_surface;\n\
@define-color terminal_fg_color @fg_content;\n\
@define-color panel_header_bg_color @bg_chrome;\n\
@define-color card_bg_color @bg_elevated;\n\
@define-color card_fg_color @fg_ui;\n\
@define-color dialog_bg_color @bg_elevated;\n\
@define-color dialog_fg_color @fg_ui;\n\
@define-color popover_bg_color @bg_elevated;\n\
@define-color popover_fg_color @fg_ui;\n\
@define-color popover_shade_color alpha(black, 0.25);\n\
@define-color sidebar_bg_color @bg_elevated;\n\
@define-color sidebar_fg_color @fg_content;\n\
@define-color secondary_sidebar_bg_color @bg_chrome;\n\
@define-color secondary_sidebar_fg_color @fg_content;\n\
@define-color thumbnail_bg_color @bg_elevated;\n\
@define-color accent_color @accent;\n\
@define-color accent_bg_color @accent_bg;\n\
@define-color accent_fg_color @accent_fg;\n\
@define-color borders @border_soft;\n\
@define-color headerbar_border_color @border_hard;\n\
@define-color headerbar_backdrop_color @bg_chrome;\n\
";

const AURORA_CSS: &str = "\
@define-color bg_window #f4f7fb;\n\
@define-color bg_chrome @bg_window;\n\
@define-color bg_surface #fafcfe;\n\
@define-color bg_elevated #e0e8f3;\n\
@define-color fg_ui #1a2740;\n\
@define-color fg_content #1a2740;\n\
@define-color accent #00b4d8;\n\
@define-color accent_bg @accent;\n\
@define-color accent_fg #ffffff;\n\
@define-color border_soft alpha(#5b8def, 0.35);\n\
@define-color border_hard alpha(#7e6aff, 0.30);\n\
@define-color window_bg_color @bg_window;\n\
@define-color window_fg_color @fg_ui;\n\
@define-color headerbar_bg_color @bg_chrome;\n\
@define-color workspace_tabs_bar_bg_color @bg_chrome;\n\
@define-color headerbar_fg_color @fg_ui;\n\
@define-color view_bg_color @bg_surface;\n\
@define-color view_fg_color @fg_content;\n\
@define-color terminal_bg_color @bg_surface;\n\
@define-color terminal_fg_color @fg_content;\n\
@define-color panel_header_bg_color @bg_chrome;\n\
@define-color card_bg_color @bg_elevated;\n\
@define-color card_fg_color @fg_ui;\n\
@define-color dialog_bg_color @bg_elevated;\n\
@define-color dialog_fg_color @fg_ui;\n\
@define-color popover_bg_color @bg_chrome;\n\
@define-color popover_fg_color @fg_ui;\n\
@define-color popover_shade_color alpha(#1a2740, 0.10);\n\
@define-color sidebar_bg_color @bg_elevated;\n\
@define-color sidebar_fg_color @fg_content;\n\
@define-color secondary_sidebar_bg_color @bg_chrome;\n\
@define-color secondary_sidebar_fg_color @fg_content;\n\
@define-color thumbnail_bg_color @bg_elevated;\n\
@define-color accent_color @accent;\n\
@define-color accent_bg_color @accent_bg;\n\
@define-color accent_fg_color @accent_fg;\n\
@define-color borders @border_soft;\n\
@define-color headerbar_border_color @border_hard;\n\
@define-color headerbar_backdrop_color @bg_chrome;\n\
";

const QUANTUM_CSS: &str = "\
@define-color bg_window #e8eff8;\n\
@define-color bg_chrome @bg_window;\n\
@define-color bg_surface #ffffff;\n\
@define-color bg_elevated #c5d3e6;\n\
@define-color fg_ui #0a1a33;\n\
@define-color fg_content #0a1a33;\n\
@define-color accent #0044cc;\n\
@define-color accent_bg @accent;\n\
@define-color accent_fg #ffffff;\n\
@define-color border_soft alpha(#0044cc, 0.45);\n\
@define-color border_hard alpha(#003a99, 0.55);\n\
@define-color window_bg_color @bg_window;\n\
@define-color window_fg_color @fg_ui;\n\
@define-color headerbar_bg_color @bg_chrome;\n\
@define-color workspace_tabs_bar_bg_color @bg_chrome;\n\
@define-color headerbar_fg_color @fg_ui;\n\
@define-color view_bg_color @bg_surface;\n\
@define-color view_fg_color @fg_content;\n\
@define-color terminal_bg_color @bg_surface;\n\
@define-color terminal_fg_color @fg_content;\n\
@define-color panel_header_bg_color @bg_chrome;\n\
@define-color card_bg_color @bg_elevated;\n\
@define-color card_fg_color @fg_ui;\n\
@define-color dialog_bg_color @bg_elevated;\n\
@define-color dialog_fg_color @fg_ui;\n\
@define-color popover_bg_color @bg_chrome;\n\
@define-color popover_fg_color @fg_ui;\n\
@define-color popover_shade_color alpha(#0a1a33, 0.18);\n\
@define-color sidebar_bg_color @bg_elevated;\n\
@define-color sidebar_fg_color @fg_content;\n\
@define-color secondary_sidebar_bg_color @bg_chrome;\n\
@define-color secondary_sidebar_fg_color @fg_content;\n\
@define-color thumbnail_bg_color @bg_elevated;\n\
@define-color accent_color @accent;\n\
@define-color accent_bg_color @accent_bg;\n\
@define-color accent_fg_color @accent_fg;\n\
@define-color borders @border_soft;\n\
@define-color headerbar_border_color @border_hard;\n\
@define-color headerbar_backdrop_color @bg_chrome;\n\
";

#[cfg(test)]
mod tests {
    use super::{Theme, AURORA_CSS, BASE_CSS, DRACULA_CSS, GRAPHITE_CSS, QUANTUM_CSS};

    #[test]
    fn base_css_uses_opaque_app_surfaces() {
        assert!(BASE_CSS.contains("toolbarview.app-toolbar-view .top-bar"));
        assert!(BASE_CSS.contains("window.app-dialog"));
        assert!(BASE_CSS.contains("popover.app-popover > contents"));
        assert!(BASE_CSS.contains("notebook.workspace-tabs > header > tabs > tab:checked"));
        assert!(BASE_CSS.contains("box.workspace-tab-add-wrap"));
        assert!(BASE_CSS.contains("entry,\nspinbutton"));
        assert!(!BASE_CSS.contains("alpha(@headerbar_bg_color, 0.95)"));
    }

    #[test]
    fn system_theme_is_disabled_and_aliases_to_default() {
        assert_eq!(Theme::default(), Theme::Graphite);
        assert_eq!(Theme::from_id("system"), Theme::Graphite);
        assert_eq!(Theme::System.to_id(), "graphite");
        assert_eq!(Theme::System.css_overrides(), GRAPHITE_CSS);
        assert!(Theme::all().iter().all(|theme| *theme != Theme::System));
    }

    #[test]
    fn legacy_theme_ids_fall_back_to_default() {
        // IDs of removed themes (Nord, Catppuccin variants, Hologram) should
        // gracefully resolve to the current default rather than error.
        for legacy in ["nord", "catppuccin-mocha", "catppuccin-latte", "hologram"] {
            assert_eq!(Theme::from_id(legacy), Theme::default());
        }
    }

    #[test]
    fn custom_themes_define_base_palette_and_semantic_aliases() {
        for css in [GRAPHITE_CSS, DRACULA_CSS, AURORA_CSS, QUANTUM_CSS] {
            for token in [
                "@define-color bg_window",
                "@define-color bg_chrome",
                "@define-color bg_surface",
                "@define-color bg_elevated",
                "@define-color fg_ui",
                "@define-color fg_content",
                "@define-color accent ",
                "@define-color accent_bg ",
                "@define-color accent_fg ",
                "@define-color border_soft",
                "@define-color border_hard",
                "@define-color window_bg_color",
                "@define-color headerbar_bg_color",
                "@define-color view_bg_color",
                "@define-color accent_color",
                "@define-color borders",
                "@define-color headerbar_border_color",
            ] {
                assert!(css.contains(token), "missing token {token} in theme css");
            }
        }
    }

    #[test]
    fn aurora_theme_is_available_as_light_theme() {
        assert!(Theme::all().contains(&Theme::Aurora));
        assert_eq!(Theme::from_id("aurora"), Theme::Aurora);
        assert_eq!(
            Theme::Aurora.color_scheme(),
            libadwaita::ColorScheme::ForceLight
        );
        assert!(AURORA_CSS.contains("@define-color bg_window #f4f7fb;"));
        assert!(AURORA_CSS.contains("@define-color accent #00b4d8;"));
    }

    #[test]
    fn quantum_theme_is_available_as_light_theme() {
        assert!(Theme::all().contains(&Theme::Quantum));
        assert_eq!(Theme::from_id("quantum"), Theme::Quantum);
        assert_eq!(Theme::Quantum.to_id(), "quantum");
        assert_eq!(
            Theme::Quantum.color_scheme(),
            libadwaita::ColorScheme::ForceLight
        );
        assert!(QUANTUM_CSS.contains("@define-color accent #0044cc;"));
        assert!(QUANTUM_CSS.contains("@define-color bg_window #e8eff8;"));
        assert!(QUANTUM_CSS.contains("@define-color bg_surface #ffffff;"));
    }

    #[test]
    fn no_theme_defines_extra_css_today() {
        // The css_extra hook exists for future structural themes; currently
        // every theme returns an empty string.
        for theme in Theme::all() {
            assert!(
                theme.css_extra().is_empty(),
                "theme {:?} unexpectedly defines css_extra",
                theme
            );
        }
    }

    #[test]
    fn graphite_theme_is_available_as_dark_theme() {
        assert!(Theme::all().contains(&Theme::Graphite));
        assert_eq!(Theme::from_id("graphite"), Theme::Graphite);
        assert_eq!(Theme::Graphite.to_id(), "graphite");
        assert_eq!(
            Theme::Graphite.color_scheme(),
            libadwaita::ColorScheme::ForceDark
        );
        assert!(GRAPHITE_CSS.contains("@define-color bg_surface #0f141b;"));
    }

    #[test]
    fn editor_file_tree_uses_view_surface_colors() {
        assert!(BASE_CSS.contains(".editor-file-tree-list"));
        assert!(BASE_CSS.contains(".editor-file-tree-scroll viewport"));
        assert!(BASE_CSS.contains(".editor-file-tree-actions"));
        assert!(BASE_CSS.contains(".editor-file-tree-header"));
        assert!(BASE_CSS.contains(".editor-file-tree-header-wrap"));
        assert!(BASE_CSS.contains("box.panel-footer-bar.editor-file-preview-footer"));
        assert!(BASE_CSS.contains(".editor-sidebar-pane-list"));
        assert!(BASE_CSS.contains(".editor-sidebar-pane-footer"));
        assert!(BASE_CSS.contains("background-color: @view_bg_color;"));
        assert!(BASE_CSS.contains("color: @view_fg_color;"));
        assert!(BASE_CSS.contains("box.panel-footer-bar.terminal-panel-footer"));
        assert!(BASE_CSS.contains("background-color: @terminal_bg_color;"));
        assert!(BASE_CSS.contains(".editor-file-tree-entry.editor-file-tree-ignored"));
        assert!(BASE_CSS.contains(".editor-file-tree-list > row:selected"));
        assert!(BASE_CSS.contains(".editor-file-tree-entry {\n  min-height: 18px;"));
    }

    #[test]
    fn app_dialog_windows_use_headerbar_surface() {
        assert!(BASE_CSS.contains("window.app-dialog,"));
        assert!(BASE_CSS.contains("background-color: @headerbar_bg_color;"));
        assert!(BASE_CSS.contains("color: @headerbar_fg_color;"));
    }

    #[test]
    fn text_selection_uses_full_accent_surface_for_contrast() {
        assert!(BASE_CSS.contains("entry selection,"));
        assert!(BASE_CSS.contains("background-color: @accent_color;"));
        assert!(!BASE_CSS.contains("alpha(@accent_color, 0.32)"));
    }

    #[test]
    fn form_controls_only_draw_borders_on_outer_widget() {
        assert!(BASE_CSS.contains("entry text,"));
        assert!(BASE_CSS.contains("background-color: transparent;"));
        assert!(BASE_CSS.contains("border: none;"));
        assert!(!BASE_CSS.contains("text,\ndropdown > button"));
    }

    #[test]
    fn base_css_sets_compact_professional_font_stacks() {
        assert!(BASE_CSS.contains("font-family: \"Inter\""));
        assert!(BASE_CSS.contains(".editor-code-view"));
        assert!(BASE_CSS.contains("\"JetBrains Mono\""));
        assert!(BASE_CSS.contains("min-height: 28px;"));
        assert!(BASE_CSS.contains("headerbar.app-headerbar"));
        assert!(BASE_CSS.contains("headerbar.app-headerbar windowhandle { min-height: 28px; }"));
        assert!(BASE_CSS.contains("button.flat,\ntogglebutton.flat"));
        assert!(BASE_CSS.contains("min-height: 18px;\n  min-width: 18px;"));
        assert!(BASE_CSS.contains("min-height: 16px;\n  min-width: 16px;"));
        assert!(BASE_CSS.contains("-gtk-icon-size: 14px;"));
        assert!(BASE_CSS.contains("-gtk-icon-size: 12px;"));
        assert!(BASE_CSS.contains("box.panel-title-bar { padding: 0 4px;"));
        assert!(BASE_CSS.contains(".panel-title { font-size: 9px;"));
        assert!(BASE_CSS.contains(".panel-title-type-icon"));
        assert!(BASE_CSS.contains("margin-left: 14px;"));
        assert!(BASE_CSS.contains(".panel-action-btn { min-height: 10px;"));
        assert!(BASE_CSS.contains(".tab-close-btn { min-height: 15px;"));
        assert!(BASE_CSS.contains(".panel-collapsed-overlay {\n  background-color: @panel_header_bg_color;"));
        assert!(BASE_CSS.contains(".panel-collapsed-chip"));
        assert!(BASE_CSS.contains(".panel-collapsed-drag-strip"));
        assert!(BASE_CSS.contains("border-radius: 14px;"));
        assert!(BASE_CSS.contains("min-height: 22px;"));
    }

    #[test]
    fn collapsed_panels_use_panel_toolbar_surface_and_rounding() {
        assert!(BASE_CSS.contains(".panel-collapsed-overlay {\n  background-color: @panel_header_bg_color;"));
        assert!(BASE_CSS.contains("border: 1px solid @headerbar_border_color;"));
        assert!(BASE_CSS.contains("border-radius: 14px;"));
        assert!(BASE_CSS.contains(".panel-collapsed-chip {\n  background-color: transparent;"));
    }

    #[test]
    fn button_hover_changes_icon_color_without_filling_background() {
        assert!(BASE_CSS.contains("button:hover,\ntogglebutton:hover,\nmenubutton > button:hover"));
        assert!(BASE_CSS.contains("button:hover image,\ntogglebutton:hover image"));
        assert!(BASE_CSS.contains("color: @accent_color;"));
        assert!(!BASE_CSS.contains("button.panel-action-btn:hover, menubutton.panel-menu-btn > button:hover, menubutton.app-menu-btn > button:hover, headerbar.app-headerbar button:hover, headerbar.app-headerbar menubutton > button:hover {\n  background-color: alpha"));
        assert!(!BASE_CSS.contains("box.workspace-tab-add-wrap > button.workspace-tab-add-btn:hover {\n  background-color: alpha"));
        assert!(!BASE_CSS.contains(
            "popover.app-popover button.app-popover-button:hover {\n  background-color: alpha"
        ));
    }

    #[test]
    fn popover_menu_hover_changes_label_color_with_icon() {
        assert!(BASE_CSS.contains(
            "popover.app-popover modelbutton:hover,\npopover.app-popover button.app-popover-button:hover {\n  background-color: transparent;"
        ));
        assert!(BASE_CSS.contains(
            "popover.app-popover modelbutton:hover image,\npopover.app-popover button.app-popover-button:hover image {\n  color: @accent_color;"
        ));
        assert!(BASE_CSS.contains(
            "popover.app-popover modelbutton:hover label,\npopover.app-popover button.app-popover-button:hover label {\n  color: @accent_color;"
        ));
    }

    #[test]
    fn checked_icon_buttons_use_accent_without_filling_background() {
        assert!(BASE_CSS.contains("button.flat:checked,\ntogglebutton.flat:checked"));
        assert!(BASE_CSS.contains("togglebutton.flat:checked image"));
        assert!(BASE_CSS.contains(".editor-sidebar-toolbar togglebutton:checked"));
        assert!(BASE_CSS.contains(".editor-sidebar-toolbar togglebutton:checked image"));
        assert!(BASE_CSS.contains("button.panel-action-btn:checked image"));
        assert!(!BASE_CSS.contains("button.panel-action-btn:checked, menubutton.panel-menu-btn > button:checked, menubutton.app-menu-btn > button:checked, headerbar.app-headerbar button:checked, headerbar.app-headerbar menubutton > button:checked {\n  background-color: alpha"));
    }

    #[test]
    fn split_tab_bar_uses_compact_height() {
        assert!(BASE_CSS.contains("notebook.workspace-tabs > header"));
        assert!(BASE_CSS.contains("min-height: 14px;"));
        assert!(BASE_CSS.contains("padding-top: 0;"));
        assert!(BASE_CSS.contains("notebook.workspace-tabs > header > tabs > tab label"));
        assert!(BASE_CSS.contains("font-size: 9px;"));
        assert!(BASE_CSS.contains("notebook.workspace-tabs > header > tabs > tab image"));
        assert!(BASE_CSS.contains("-gtk-icon-size: 9px;"));
        assert!(BASE_CSS.contains("box.workspace-tab-page-shell {"));
    }

    #[test]
    fn panel_frames_keep_visible_card_margins_and_rounding() {
        assert!(BASE_CSS.contains(
            "box.panel-frame { border: 1px solid @headerbar_border_color; border-radius: 14px; margin: 6px; padding: 0; }"
        ));
        assert!(BASE_CSS.contains(
            "box.panel-frame > box.panel-title-bar { border-radius: 14px 14px 0 0; }"
        ));
        assert!(BASE_CSS.contains(
            "box.panel-frame > box.panel-footer-bar { border-radius: 0 0 14px 14px; }"
        ));
    }

    #[test]
    fn collapsed_placeholder_frame_is_hidden_when_overlay_control_is_used() {
        assert!(BASE_CSS.contains("box.panel-frame.panel-collapsed-placeholder {"));
        assert!(BASE_CSS.contains("border: none;"));
        assert!(BASE_CSS.contains("background-color: transparent;"));
        assert!(BASE_CSS.contains("margin: 0;"));
    }

    #[test]
    fn paned_separator_is_transparent() {
        assert!(BASE_CSS.contains("paned > separator {\n  min-width: 1px;"));
        assert!(BASE_CSS.contains("background-color: transparent;"));
        assert!(BASE_CSS.contains("box-shadow: none;"));
    }

    #[test]
    fn progressive_chrome_css_distinguishes_root_nested_and_focus_path() {
        assert!(BASE_CSS.contains("notebook.workspace-tabs > header {\n  border-bottom: none;\n  box-shadow: none;\n  min-height: 16px;"));
        assert!(BASE_CSS.contains("notebook.workspace-tabs-root {\n  margin-top: 6px;"));
        assert!(BASE_CSS.contains("notebook.workspace-tabs-nested {\n  margin-top: 6px;"));
        assert!(BASE_CSS.contains("background-color: @panel_header_bg_color;"));
        assert!(BASE_CSS.contains("notebook.workspace-tabs > header > tabs > tab:checked label {\n  color: alpha(@headerbar_fg_color, 0.96);"));
        assert!(BASE_CSS.contains("button.workspace-tab-close-btn"));
        assert!(BASE_CSS.contains("box.panel-frame.panel-focused box.panel-title-bar"));
        assert!(BASE_CSS.contains("box.panel-frame.panel-unfocused .panel-title"));
    }

    #[test]
    fn dropdown_popups_use_transparent_internal_wrappers() {
        assert!(
            BASE_CSS.contains("window.app-dialog dropdown.settings-theme-dropdown > popover.menu")
        );
        assert!(BASE_CSS.contains(".editor-sidebar-pane dropdown > popover.menu > contents"));
        assert!(BASE_CSS.contains("> popover.menu > arrow"));
        assert!(BASE_CSS.contains("popover.menu scrolledwindow,"));
        assert!(BASE_CSS.contains("background-color: transparent;"));
    }
}
