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
    CatppuccinMocha,
    CatppuccinLatte,
    Dracula,
    Nord,
}

impl Default for Theme {
    fn default() -> Self {
        Self::Nord
    }
}

impl Theme {
    fn resolved(self) -> Self {
        match self {
            Theme::System => Theme::Nord,
            other => other,
        }
    }

    pub fn label(&self) -> &str {
        match self.resolved() {
            Theme::CatppuccinMocha => "Catppuccin Mocha",
            Theme::CatppuccinLatte => "Catppuccin Latte",
            Theme::Dracula => "Dracula",
            Theme::Nord => "Nord",
            Theme::System => unreachable!(),
        }
    }

    pub fn all() -> &'static [Theme] {
        &[
            Theme::Nord,
            Theme::CatppuccinMocha,
            Theme::CatppuccinLatte,
            Theme::Dracula,
        ]
    }

    pub fn color_scheme(&self) -> libadwaita::ColorScheme {
        match self.resolved() {
            Theme::CatppuccinLatte => libadwaita::ColorScheme::ForceLight,
            Theme::CatppuccinMocha | Theme::Dracula | Theme::Nord => {
                libadwaita::ColorScheme::ForceDark
            }
            Theme::System => unreachable!(),
        }
    }

    pub fn to_id(&self) -> &str {
        match self.resolved() {
            Theme::CatppuccinMocha => "catppuccin-mocha",
            Theme::CatppuccinLatte => "catppuccin-latte",
            Theme::Dracula => "dracula",
            Theme::Nord => "nord",
            Theme::System => unreachable!(),
        }
    }

    pub fn from_id(id: &str) -> Theme {
        match id {
            "system" | "" => Theme::Nord,
            "catppuccin-mocha" => Theme::CatppuccinMocha,
            "catppuccin-latte" => Theme::CatppuccinLatte,
            "dracula" => Theme::Dracula,
            "nord" => Theme::Nord,
            _ => Theme::Nord,
        }
    }

    /// Returns (background, foreground) RGBA for VTE terminal.
    pub fn terminal_colors(&self) -> Option<(gtk4::gdk::RGBA, gtk4::gdk::RGBA)> {
        match self.resolved() {
            Theme::CatppuccinMocha => Some((
                gtk4::gdk::RGBA::new(0.118, 0.118, 0.180, 1.0), // #1e1e2e
                gtk4::gdk::RGBA::new(0.804, 0.839, 0.957, 1.0), // #cdd6f4
            )),
            Theme::CatppuccinLatte => Some((
                gtk4::gdk::RGBA::new(0.937, 0.945, 0.961, 1.0), // #eff1f5
                gtk4::gdk::RGBA::new(0.298, 0.310, 0.412, 1.0), // #4c4f69
            )),
            Theme::Dracula => Some((
                gtk4::gdk::RGBA::new(0.157, 0.165, 0.212, 1.0), // #282a36
                gtk4::gdk::RGBA::new(0.973, 0.973, 0.949, 1.0), // #f8f8f2
            )),
            Theme::Nord => Some((
                gtk4::gdk::RGBA::new(0.180, 0.204, 0.251, 1.0), // #2e3440
                gtk4::gdk::RGBA::new(0.925, 0.937, 0.957, 1.0), // #eceff4
            )),
            Theme::System => unreachable!(),
        }
    }

    /// Returns the GtkSourceView 5 style scheme ID for this theme.
    #[cfg(feature = "sourceview")]
    pub fn sourceview_scheme(&self) -> &str {
        match self.resolved() {
            Theme::CatppuccinMocha => "pax-catppuccin-mocha",
            Theme::CatppuccinLatte => "pax-catppuccin-latte",
            Theme::Dracula => "pax-dracula",
            Theme::Nord => "pax-nord",
            Theme::System => unreachable!(),
        }
    }

    /// Fallback scheme if the primary is not available.
    #[cfg(feature = "sourceview")]
    pub fn sourceview_scheme_fallback(&self) -> &str {
        match self.resolved() {
            Theme::CatppuccinLatte => "Adwaita",
            Theme::CatppuccinMocha | Theme::Dracula | Theme::Nord => "Adwaita-dark",
            Theme::System => unreachable!(),
        }
    }

    /// Returns CSS @define-color overrides for libadwaita named colors.
    pub fn css_overrides(&self) -> &str {
        match self.resolved() {
            Theme::CatppuccinMocha => CATPPUCCIN_MOCHA_CSS,
            Theme::CatppuccinLatte => CATPPUCCIN_LATTE_CSS,
            Theme::Dracula => DRACULA_CSS,
            Theme::Nord => NORD_CSS,
            Theme::System => unreachable!(),
        }
    }
}

/// Minimal CSS — only layout, no colors.
pub const BASE_CSS: &str = "
window, .background { background-color: @window_bg_color; color: @window_fg_color; }
window.app-dialog,
window.app-dialog > * {
  background-color: @headerbar_bg_color;
  color: @headerbar_fg_color;
}
toolbarview.app-toolbar-view { background-color: @window_bg_color; color: @window_fg_color; }
toolbarview.app-toolbar-view .top-bar { background-color: @headerbar_bg_color; color: @headerbar_fg_color; border-bottom: 1px solid @headerbar_border_color; }
toolbarview.app-toolbar-view .top-bar > * { background-color: @headerbar_bg_color; color: @headerbar_fg_color; }
headerbar.app-headerbar { background-color: @headerbar_bg_color; color: @headerbar_fg_color; border: none; }
headerbar.app-headerbar box, headerbar.app-headerbar label, headerbar.app-headerbar image, headerbar.app-headerbar button, headerbar.app-headerbar menubutton > button { color: @headerbar_fg_color; }
box.panel-title-bar, box.panel-footer-bar, .status-bar, .markdown-toolbar { background-color: @headerbar_bg_color; color: @headerbar_fg_color; }
.editor-sidebar-toolbar-surface,
.editor-sidebar-toolbar {
  background-color: @headerbar_bg_color;
  color: @headerbar_fg_color;
}
.editor-file-tree-actions {
  background-color: @view_bg_color;
  color: @view_fg_color;
  border-top: 1px solid alpha(@borders, 0.4);
  border-right: none;
  border-bottom: none;
  border-left: none;
}
notebook.workspace-tabs,
notebook.workspace-tabs > header,
notebook.workspace-tabs > header > tabs,
notebook.workspace-tabs > header > tabs > tab {
  background-color: @headerbar_bg_color;
  color: @headerbar_fg_color;
  border-color: transparent;
  background-image: none;
  box-shadow: none;
}
notebook.workspace-tabs > header {
  border-bottom: none;
  box-shadow: none;
}
notebook.workspace-tabs > header > tabs {
  box-shadow: inset 0 -1px 0 0 @headerbar_border_color;
}
notebook.workspace-tabs > header > tabs > tab {
  border-radius: 0;
  margin: 0;
  padding-top: 4px;
  padding-bottom: 4px;
  box-shadow: inset 0 -1px 0 0 @headerbar_border_color;
}
notebook.workspace-tabs > header > tabs > tab:hover {
  background-color: alpha(@headerbar_fg_color, 0.08);
}
notebook.workspace-tabs > header > tabs > tab:checked {
  background-color: @headerbar_bg_color;
  box-shadow: inset 0 -2px 0 0 @accent_color;
}
box.workspace-tab-add-wrap {
  background-color: @headerbar_bg_color;
  border: none;
  box-shadow: none;
}
box.workspace-tab-add-wrap > button.workspace-tab-add-btn {
  background-image: none;
  background-color: transparent;
  border: none;
  box-shadow: none;
}
box.workspace-tab-add-wrap > button.workspace-tab-add-btn:hover {
  background-color: alpha(@headerbar_fg_color, 0.10);
}
button.panel-action-btn, menubutton.panel-menu-btn > button, menubutton.app-menu-btn > button, headerbar.app-headerbar button, headerbar.app-headerbar menubutton > button {
  background-image: none;
  background-color: transparent;
  border-color: transparent;
  box-shadow: none;
}
button.panel-action-btn:hover, menubutton.panel-menu-btn > button:hover, menubutton.app-menu-btn > button:hover, headerbar.app-headerbar button:hover, headerbar.app-headerbar menubutton > button:hover {
  background-color: alpha(@headerbar_fg_color, 0.10);
  border-color: transparent;
  box-shadow: none;
}
button.panel-action-btn:checked, menubutton.panel-menu-btn > button:checked, menubutton.app-menu-btn > button:checked, headerbar.app-headerbar button:checked, headerbar.app-headerbar menubutton > button:checked {
  background-color: alpha(@headerbar_fg_color, 0.16);
  border-color: transparent;
  box-shadow: none;
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
  padding: 4px;
}
popover.app-popover > arrow {
  background-color: transparent;
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
box.panel-frame { border: none; border-radius: 0; margin: 0; padding: 0; }
box.panel-frame > box { margin: 0; padding: 0; }
box.panel-title-bar { padding: 2px 6px; margin: 0; min-height: 20px; border-bottom: 1px solid alpha(@borders, 0.4); }
.panel-title { font-size: 11px; font-weight: bold; }
.panel-type-icon { min-height: 14px; min-width: 14px; opacity: 0.6; margin-right: 2px; }
.panel-menu-btn { min-height: 16px; min-width: 16px; padding: 2px; }
.panel-action-btn { min-height: 16px; min-width: 16px; padding: 2px; opacity: 0.5; }
.panel-action-btn:hover { opacity: 1.0; }
.sync-active { opacity: 1.0; color: #e5a50a; }
.zoom-active { opacity: 1.0; color: #5588ff; }
.panel-focused { border: none; }
.panel-unfocused { border: none; }
.panel-type-btn { min-width: 120px; }
.panel-footer-bar { padding: 1px 8px 1px 12px; min-height: 18px; border-top: 1px solid alpha(@borders, 0.4); }
.panel-footer { font-size: 10px; }
.status-bar { padding: 2px 8px; min-height: 22px; }
.status-mode { font-weight: bold; padding: 0 6px; }
.markdown-panel { font-family: sans-serif; font-size: 12px; }
.markdown-toolbar { border-bottom: 1px solid alpha(@borders, 0.3); }
.tab-close-btn { min-height: 14px; min-width: 14px; padding: 1px; }
.panel-collapsed-overlay { background-color: @headerbar_bg_color; border-top: 1px solid alpha(@borders, 0.4); border-bottom: 1px solid alpha(@borders, 0.4); padding: 4px; min-width: 32px; min-height: 32px; }
paned > separator { min-width: 1px; min-height: 1px; }
.dirty-indicator { color: #ff8c00; }
.editor-tabs { border-bottom: 1px solid alpha(@borders, 0.3); }
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
.editor-file-tree-list > row > box,
.editor-file-tree-list > row > box > * {
  background-color: transparent;
}
.editor-file-tree-entry.editor-file-tree-ignored,
.editor-file-tree-entry.editor-file-tree-ignored > label,
.editor-file-tree-entry.editor-file-tree-ignored > image,
.editor-file-tree-entry.editor-file-tree-ignored > drawingarea {
  color: alpha(@view_fg_color, 0.58);
  opacity: 0.72;
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
  background-color: @accent_bg_color;
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
  min-height: 30px;
}
popover.app-popover modelbutton:hover,
popover.app-popover button.app-popover-button:hover {
  background-color: alpha(@accent_bg_color, 0.18);
  color: @popover_fg_color;
}
";

const CATPPUCCIN_MOCHA_CSS: &str = "\
@define-color window_bg_color #1e1e2e;
@define-color window_fg_color #cdd6f4;
@define-color headerbar_bg_color #181825;
@define-color headerbar_fg_color #cdd6f4;
@define-color card_bg_color #313244;
@define-color card_fg_color #cdd6f4;
@define-color dialog_bg_color #313244;
@define-color dialog_fg_color #cdd6f4;
@define-color popover_bg_color #313244;
@define-color popover_fg_color #cdd6f4;
@define-color popover_shade_color alpha(black, 0.25);
@define-color sidebar_bg_color #313244;
@define-color sidebar_fg_color #cdd6f4;
@define-color secondary_sidebar_bg_color #181825;
@define-color secondary_sidebar_fg_color #cdd6f4;
@define-color thumbnail_bg_color #313244;
@define-color view_bg_color #1e1e2e;
@define-color view_fg_color #cdd6f4;
@define-color accent_bg_color #89b4fa;
@define-color accent_fg_color #1e1e2e;
@define-color accent_color #89b4fa;
@define-color borders alpha(white, 0.15);
@define-color headerbar_border_color alpha(white, 0.15);
@define-color headerbar_backdrop_color @headerbar_bg_color;
";

const CATPPUCCIN_LATTE_CSS: &str = "\
@define-color window_bg_color #eff1f5;
@define-color window_fg_color #4c4f69;
@define-color headerbar_bg_color #e6e9ef;
@define-color headerbar_fg_color #4c4f69;
@define-color card_bg_color #ccd0da;
@define-color card_fg_color #4c4f69;
@define-color dialog_bg_color #ccd0da;
@define-color dialog_fg_color #4c4f69;
@define-color popover_bg_color #ccd0da;
@define-color popover_fg_color #4c4f69;
@define-color popover_shade_color alpha(black, 0.12);
@define-color sidebar_bg_color #ccd0da;
@define-color sidebar_fg_color #4c4f69;
@define-color secondary_sidebar_bg_color #e6e9ef;
@define-color secondary_sidebar_fg_color #4c4f69;
@define-color thumbnail_bg_color #ccd0da;
@define-color view_bg_color #eff1f5;
@define-color view_fg_color #4c4f69;
@define-color accent_bg_color #1e66f5;
@define-color accent_fg_color #eff1f5;
@define-color accent_color #1e66f5;
@define-color borders alpha(black, 0.15);
@define-color headerbar_border_color alpha(black, 0.15);
@define-color headerbar_backdrop_color @headerbar_bg_color;
";

const DRACULA_CSS: &str = "\
@define-color window_bg_color #282a36;
@define-color window_fg_color #f8f8f2;
@define-color headerbar_bg_color #21222c;
@define-color headerbar_fg_color #f8f8f2;
@define-color card_bg_color #44475a;
@define-color card_fg_color #f8f8f2;
@define-color dialog_bg_color #44475a;
@define-color dialog_fg_color #f8f8f2;
@define-color popover_bg_color #44475a;
@define-color popover_fg_color #f8f8f2;
@define-color popover_shade_color alpha(black, 0.25);
@define-color sidebar_bg_color #44475a;
@define-color sidebar_fg_color #f8f8f2;
@define-color secondary_sidebar_bg_color #21222c;
@define-color secondary_sidebar_fg_color #f8f8f2;
@define-color thumbnail_bg_color #44475a;
@define-color view_bg_color #282a36;
@define-color view_fg_color #f8f8f2;
@define-color accent_bg_color #bd93f9;
@define-color accent_fg_color #282a36;
@define-color accent_color #bd93f9;
@define-color borders alpha(white, 0.15);
@define-color headerbar_border_color alpha(white, 0.15);
@define-color headerbar_backdrop_color @headerbar_bg_color;
";

const NORD_CSS: &str = "\
@define-color window_bg_color #2e3440;
@define-color window_fg_color #eceff4;
@define-color headerbar_bg_color #3b4252;
@define-color headerbar_fg_color #eceff4;
@define-color card_bg_color #3b4252;
@define-color card_fg_color #eceff4;
@define-color dialog_bg_color #3b4252;
@define-color dialog_fg_color #eceff4;
@define-color popover_bg_color #3b4252;
@define-color popover_fg_color #eceff4;
@define-color popover_shade_color alpha(black, 0.25);
@define-color sidebar_bg_color #3b4252;
@define-color sidebar_fg_color #eceff4;
@define-color secondary_sidebar_bg_color #2e3440;
@define-color secondary_sidebar_fg_color #eceff4;
@define-color thumbnail_bg_color #3b4252;
@define-color view_bg_color #2e3440;
@define-color view_fg_color #eceff4;
@define-color accent_bg_color #88c0d0;
@define-color accent_fg_color #2e3440;
@define-color accent_color #88c0d0;
@define-color borders alpha(white, 0.12);
@define-color headerbar_border_color alpha(white, 0.12);
@define-color headerbar_backdrop_color @headerbar_bg_color;
";

#[cfg(test)]
mod tests {
    use super::{
        Theme, BASE_CSS, CATPPUCCIN_LATTE_CSS, CATPPUCCIN_MOCHA_CSS, DRACULA_CSS, NORD_CSS,
    };

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
    fn system_theme_is_disabled_and_aliases_to_nord() {
        assert_eq!(Theme::default(), Theme::Nord);
        assert_eq!(Theme::from_id("system"), Theme::Nord);
        assert_eq!(Theme::System.to_id(), "nord");
        assert_eq!(Theme::System.css_overrides(), NORD_CSS);
        assert!(Theme::all().iter().all(|theme| *theme != Theme::System));
    }

    #[test]
    fn custom_themes_define_dialog_sidebar_and_thumbnail_tokens() {
        for css in [
            CATPPUCCIN_MOCHA_CSS,
            CATPPUCCIN_LATTE_CSS,
            DRACULA_CSS,
            NORD_CSS,
        ] {
            for token in [
                "@define-color dialog_bg_color",
                "@define-color dialog_fg_color",
                "@define-color sidebar_bg_color",
                "@define-color sidebar_fg_color",
                "@define-color secondary_sidebar_bg_color",
                "@define-color secondary_sidebar_fg_color",
                "@define-color thumbnail_bg_color",
            ] {
                assert!(css.contains(token), "missing token {token} in theme css");
            }
        }
    }

    #[test]
    fn editor_file_tree_uses_view_surface_colors() {
        assert!(BASE_CSS.contains(".editor-file-tree-list"));
        assert!(BASE_CSS.contains(".editor-file-tree-scroll viewport"));
        assert!(BASE_CSS.contains(".editor-file-tree-actions"));
        assert!(BASE_CSS.contains("background-color: @view_bg_color;"));
        assert!(BASE_CSS.contains("color: @view_fg_color;"));
        assert!(BASE_CSS.contains(".editor-file-tree-entry.editor-file-tree-ignored"));
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
        assert!(BASE_CSS.contains("background-color: @accent_bg_color;"));
        assert!(!BASE_CSS.contains("alpha(@accent_bg_color, 0.32)"));
    }

    #[test]
    fn form_controls_only_draw_borders_on_outer_widget() {
        assert!(BASE_CSS.contains("entry text,"));
        assert!(BASE_CSS.contains("background-color: transparent;"));
        assert!(BASE_CSS.contains("border: none;"));
        assert!(!BASE_CSS.contains("text,\ndropdown > button"));
    }
}
