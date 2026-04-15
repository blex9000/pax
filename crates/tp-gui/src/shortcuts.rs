//! Keyboard shortcut helpers.
//!
//! Centralises the "primary modifier" concept so Pax's app-level shortcuts
//! follow platform conventions: `Cmd` on macOS (`META_MASK`) and `Ctrl`
//! elsewhere (`CONTROL_MASK`). Terminal input intentionally stays on raw
//! `CONTROL_MASK` regardless of OS because that's what the shell expects
//! (`Ctrl+C` sends `^C`, unchanged on macOS Terminal.app).

use gtk4::gdk::ModifierType;

/// Modifier users expect for app-level shortcuts (save, copy, paste, undo,
/// close-tab, …). Cmd on macOS, Ctrl elsewhere.
#[cfg(target_os = "macos")]
pub const PRIMARY_MODIFIER: ModifierType = ModifierType::META_MASK;

#[cfg(not(target_os = "macos"))]
pub const PRIMARY_MODIFIER: ModifierType = ModifierType::CONTROL_MASK;

/// Returns true if the event state contains the platform's primary modifier.
/// Convenience wrapper around `mods.contains(PRIMARY_MODIFIER)` to keep call
/// sites readable.
pub fn has_primary(mods: ModifierType) -> bool {
    mods.contains(PRIMARY_MODIFIER)
}
