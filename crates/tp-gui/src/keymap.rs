//! User-configurable keybindings.
//!
//! Defines the catalogue of bindable [`Action`]s, the [`KeyBinding`]
//! data structure, and a [`KeyMap`] mapping key combinations to
//! actions. Defaults match the previously hard-coded shortcuts in
//! `app.rs::connect_key_pressed`. Users can override via the Settings
//! dialog; persistence lives in `~/.local/share/pax/keybindings.json`
//! (alongside the existing pax.db / pax.log) so it travels with the
//! data dir Pax already manages.

use std::path::PathBuf;

use gtk4::gdk;
use serde::{Deserialize, Serialize};

/// Catalogue of every action that can be bound to a keystroke.
///
/// Keep variant names stable — the JSON file uses them as keys via
/// serde's default snake_case rename. Adding a new variant is safe;
/// removing or renaming one breaks existing user configs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    CommandPalette,
    QuickSwitchWorkspace,
    OpenWorkspace,
    SaveWorkspace,
    Quit,
    FocusNextPanel,
    FocusPrevPanel,
    SplitHorizontal,
    SplitVertical,
    NewTab,
    ClosePanel,
    ToggleZoom,
    ToggleSync,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
}

impl Action {
    /// Stable list of all actions, for the UI catalogue.
    pub fn all() -> &'static [Action] {
        &[
            Action::CommandPalette,
            Action::QuickSwitchWorkspace,
            Action::OpenWorkspace,
            Action::SaveWorkspace,
            Action::Quit,
            Action::FocusNextPanel,
            Action::FocusPrevPanel,
            Action::SplitHorizontal,
            Action::SplitVertical,
            Action::NewTab,
            Action::ClosePanel,
            Action::ToggleZoom,
            Action::ToggleSync,
            Action::ScrollUp,
            Action::ScrollDown,
            Action::ScrollLeft,
            Action::ScrollRight,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Action::CommandPalette => "Command palette",
            Action::QuickSwitchWorkspace => "Quick-switch workspace",
            Action::OpenWorkspace => "Open workspace",
            Action::SaveWorkspace => "Save workspace",
            Action::Quit => "Quit",
            Action::FocusNextPanel => "Focus next panel",
            Action::FocusPrevPanel => "Focus previous panel",
            Action::SplitHorizontal => "Split horizontal",
            Action::SplitVertical => "Split vertical",
            Action::NewTab => "New tab",
            Action::ClosePanel => "Close focused panel",
            Action::ToggleZoom => "Toggle zoom on focused panel",
            Action::ToggleSync => "Toggle sync on focused panel",
            Action::ScrollUp => "Scroll workspace up",
            Action::ScrollDown => "Scroll workspace down",
            Action::ScrollLeft => "Scroll workspace left",
            Action::ScrollRight => "Scroll workspace right",
        }
    }
}

/// A single (modifier set + key) tuple. We model the modifiers as
/// individual booleans rather than a bitfield so the JSON form is
/// human-editable. The "primary" bool maps to Ctrl on Linux/Windows
/// and Cmd on macOS, matching `crate::shortcuts::PRIMARY_MODIFIER`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyBinding {
    pub primary: bool,
    pub shift: bool,
    pub alt: bool,
    /// Stored as the key's standard name (e.g. "k", "T", "Up", "F5").
    /// Use gdk::Key::name() to round-trip between gdk and string.
    pub key: String,
}

impl KeyBinding {
    /// Build a binding from a live key event. Normalizes the key name
    /// to its lowercase form for letters so Ctrl+K and Ctrl+k bind to
    /// the same Action.
    pub fn from_event(key: gdk::Key, modifiers: gdk::ModifierType) -> Option<Self> {
        let primary = crate::shortcuts::has_primary(modifiers);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
        let name = key.name()?.to_string();
        Some(Self {
            primary,
            shift,
            alt,
            key: normalize_key_name(&name),
        })
    }

    pub fn matches(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
        let primary = crate::shortcuts::has_primary(modifiers);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
        if primary != self.primary || shift != self.shift || alt != self.alt {
            return false;
        }
        match key.name() {
            Some(n) => normalize_key_name(&n) == self.key,
            None => false,
        }
    }

    /// Human-readable form, e.g. "Ctrl+Shift+H".
    pub fn display(&self) -> String {
        let mut parts: Vec<&str> = Vec::with_capacity(4);
        if self.primary {
            parts.push(if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" });
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        let key_display = display_key(&self.key);
        let mut out = parts.join("+");
        if !out.is_empty() {
            out.push('+');
        }
        out.push_str(&key_display);
        out
    }
}

/// Lowercase letters for predictable matching; preserve named keys
/// (Up, Down, Return…) as gdk reports them.
fn normalize_key_name(name: &str) -> String {
    if name.chars().count() == 1 {
        name.to_ascii_lowercase()
    } else {
        name.to_string()
    }
}

fn display_key(key: &str) -> String {
    if key.chars().count() == 1 {
        key.to_ascii_uppercase()
    } else {
        key.to_string()
    }
}

/// The full mapping. Stored as a Vec for stable iteration order in the
/// settings UI; a HashMap-backed lookup wraps it for O(1) dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMap {
    pub bindings: Vec<(Action, KeyBinding)>,
}

impl KeyMap {
    pub fn lookup(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(_, b)| b.matches(key, modifiers))
            .map(|(a, _)| *a)
    }

    pub fn binding_for(&self, action: Action) -> Option<&KeyBinding> {
        self.bindings
            .iter()
            .find(|(a, _)| *a == action)
            .map(|(_, b)| b)
    }

    pub fn set_binding(&mut self, action: Action, binding: KeyBinding) {
        if let Some(slot) = self.bindings.iter_mut().find(|(a, _)| *a == action) {
            slot.1 = binding;
        } else {
            self.bindings.push((action, binding));
        }
    }

    /// Return action(s) currently bound to the same combo as `binding`,
    /// excluding `action` itself. Used by the UI to flag conflicts.
    pub fn conflicts_with(&self, binding: &KeyBinding, ignore: Action) -> Vec<Action> {
        self.bindings
            .iter()
            .filter(|(a, b)| *a != ignore && b == binding)
            .map(|(a, _)| *a)
            .collect()
    }
}

impl Default for KeyMap {
    fn default() -> Self {
        let mut bindings = Vec::new();
        let kb = |primary, shift, alt, key: &str| KeyBinding {
            primary,
            shift,
            alt,
            key: key.to_string(),
        };
        bindings.push((Action::CommandPalette, kb(true, false, false, "k")));
        bindings.push((Action::QuickSwitchWorkspace, kb(true, true, false, "o")));
        bindings.push((Action::OpenWorkspace, kb(true, false, false, "o")));
        bindings.push((Action::SaveWorkspace, kb(true, false, false, "s")));
        bindings.push((Action::Quit, kb(true, false, false, "q")));
        bindings.push((Action::FocusNextPanel, kb(true, false, false, "n")));
        bindings.push((Action::FocusPrevPanel, kb(true, false, false, "p")));
        bindings.push((Action::SplitHorizontal, kb(true, true, false, "h")));
        bindings.push((Action::SplitVertical, kb(true, true, false, "j")));
        bindings.push((Action::NewTab, kb(true, true, false, "t")));
        bindings.push((Action::ClosePanel, kb(true, true, false, "w")));
        bindings.push((Action::ToggleZoom, kb(true, true, false, "z")));
        bindings.push((Action::ToggleSync, kb(true, true, false, "s")));
        bindings.push((Action::ScrollUp, kb(true, false, false, "Up")));
        bindings.push((Action::ScrollDown, kb(true, false, false, "Down")));
        bindings.push((Action::ScrollLeft, kb(true, false, false, "Left")));
        bindings.push((Action::ScrollRight, kb(true, false, false, "Right")));
        Self { bindings }
    }
}

fn config_path() -> PathBuf {
    // Anchor next to the existing pax.db / pax.log so all per-user
    // app data lives under one directory. Reusing pax_db's
    // default_path saves wiring a `dirs` crate dependency in tp-gui.
    let db = pax_db::Database::default_path();
    let dir = db.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    let _ = std::fs::create_dir_all(&dir);
    dir.join("keybindings.json")
}

/// Load the keymap from disk; falls back to defaults if the file is
/// missing or malformed (with a debug-level log line). User overrides
/// are merged over defaults so adding new actions in code doesn't
/// require users to re-save their config.
pub fn load() -> KeyMap {
    let path = config_path();
    let mut map = KeyMap::default();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return map;
    };
    let parsed: Result<KeyMap, _> = serde_json::from_str(&text);
    match parsed {
        Ok(user) => {
            for (action, binding) in user.bindings {
                map.set_binding(action, binding);
            }
            map
        }
        Err(e) => {
            tracing::warn!("keymap: could not parse {}: {e}", path.display());
            map
        }
    }
}

pub fn save(map: &KeyMap) -> std::io::Result<()> {
    let path = config_path();
    let text = serde_json::to_string_pretty(map)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(&path, text)
}

thread_local! {
    /// Single live keymap shared across the app. Loaded once on
    /// startup; mutated when the settings UI saves a new binding.
    static CURRENT: std::cell::RefCell<KeyMap> = std::cell::RefCell::new(load());
}

pub fn current() -> KeyMap {
    CURRENT.with(|c| c.borrow().clone())
}

pub fn set_current(map: KeyMap) {
    CURRENT.with(|c| *c.borrow_mut() = map);
}

pub fn lookup(key: gdk::Key, modifiers: gdk::ModifierType) -> Option<Action> {
    CURRENT.with(|c| c.borrow().lookup(key, modifiers))
}
