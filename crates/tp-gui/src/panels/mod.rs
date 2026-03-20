pub mod terminal;
pub mod markdown;
pub mod chooser;
pub mod registry;

/// Trait implemented by all panel backends.
/// Each panel type creates a GTK widget and handles its lifecycle.
/// This is the contract that plugins must implement.
pub trait PanelBackend: std::fmt::Debug {
    /// Panel type identifier (e.g., "terminal", "markdown", "browser", "chooser").
    fn panel_type(&self) -> &str;

    /// The GTK widget for this panel.
    fn widget(&self) -> &gtk4::Widget;

    /// Called when the panel receives focus.
    fn on_focus(&self);

    /// Called when the panel loses focus.
    fn on_blur(&self) {}

    /// Write input to the panel (for terminal-like panels).
    fn write_input(&self, _data: &[u8]) -> bool {
        false
    }

    /// Get current text content for recording/alert scanning.
    fn get_text_content(&self) -> Option<String> {
        None
    }

    /// Whether this panel supports text input.
    fn accepts_input(&self) -> bool {
        false
    }
}
