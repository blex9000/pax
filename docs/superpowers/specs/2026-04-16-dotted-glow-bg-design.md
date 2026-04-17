# Dotted Glow Background — Design

**Date:** 2026-04-16
**Status:** Approved, ready for implementation plan
**Target crate:** `tp-gui`

## Goal

Add an animated "dotted glow" background effect to the Pax application window. The effect is orthogonal to themes (toggle on/off), driven by theme accent color by default, and customizable per-theme via the existing Color Customizer dialog.

## Scope & decisions

- **Orthogonal toggle**: independent from theme selection, but per-theme settings (reuses existing `load_custom_colors(theme)` storage pattern).
- **Rendering**: Cairo via `GtkDrawingArea` — cross-platform (Linux VTE build and macOS `--no-default-features` build both use Cairo through GTK4).
- **Pattern**: staggered grid, 24px spacing, 1.5px dot radius, ~6px halo.
- **Animation**: desynced shimmer per-dot + mouse spotlight (combo). 30fps cap. Pauses when window is not active.
- **Color**: derived from theme `@define-color accent` (lightened in HSL by default), overridable with user-picked color.
- **User controls**: enable switch, color picker, glow intensity slider (0–100%). Lives inside the existing Color Customizer dialog as a new group "Background Effect".
- **Visible surfaces**: window background, header bar, tab bar. Panels stay opaque for terminal readability; the effect shows through the gaps between panels and through semi-transparent header/tab bar.

## Architecture

**Approach: `GtkOverlay` + `GtkDrawingArea`**

Wrap the existing `adw::ToolbarView` in a `gtk4::Overlay`:

```
adw::ApplicationWindow
  └─ gtk4::Overlay
       ├─ base child:  GtkDrawingArea   (animated Cairo-painted bg)
       └─ overlay:     adw::ToolbarView (header + workspace content)
```

Upper layers (headerbar, tab bar) become semi-transparent via CSS so the dots show through. Panels stay opaque.

Mouse tracking: `EventControllerMotion` attached to the Overlay, coordinates forwarded to the DrawingArea state.

Animation loop: `add_tick_callback` on the DrawingArea, skipping frames when elapsed < 33ms or when window is not `is_active`.

Rejected alternative: custom widget with `snapshot()` override via `gtk4::subclass`. Same visual result, but GObject subclassing in Rust is significantly more boilerplate with no practical benefit here. `GtkDrawingArea` remains first-class for custom drawing in GTK4.

## Module layout

**New file:** `crates/tp-gui/src/widgets/dotted_bg.rs`

```rust
pub struct DottedBgConfig {
    pub enabled: bool,
    pub color: String,   // hex "#rrggbb"
    pub glow: f32,       // 0.0 .. 1.0
}

pub struct DottedBackground {
    drawing_area: gtk4::DrawingArea,
    state: Rc<RefCell<State>>,
}

struct State {
    config: DottedBgConfig,
    tick: u64,
    mouse: Option<(f64, f64)>,
    mouse_fade: f32,             // 0..1, fades out 400ms when mouse leaves
    sprite: Option<cairo::ImageSurface>,
    last_frame_ns: i64,
    width: i32,
    height: i32,
}

impl DottedBackground {
    pub fn new(cfg: DottedBgConfig) -> Self;
    pub fn widget(&self) -> &gtk4::DrawingArea;
    pub fn set_config(&self, cfg: DottedBgConfig);   // live update, rebuilds sprite if color/glow changed
    pub fn attach_mouse_tracking(&self, overlay: &gtk4::Overlay);
}
```

**Touch points** (existing files):
- `crates/tp-gui/src/widgets/mod.rs` — export the new module.
- `crates/tp-gui/src/app.rs::setup_workspace_ui` and `setup_welcome_ui` — wrap `ToolbarView` in `Overlay`, instantiate `DottedBackground`, wire mouse tracking, load config via `load_dotted_bg(theme)`.
- `crates/tp-gui/src/theme.rs` — extend storage helpers (see Settings storage below); optional: add utility `lighten_accent(hex) -> String`.
- `crates/tp-gui/src/dialogs/color_customizer.rs` — add new group "Background Effect" with switch, color button, glow slider; extend save/close paths.
- `resources/style.css` — header-bar / tab-bar alpha rules gated on `.has-dotted-bg` class; window-level toggle.

## Settings storage

Per-theme. Extends the JSON file already used by the color customizer (path already owned by `load_custom_colors(theme)` / `save_custom_colors`). The file grows a second top-level key:

```json
{
  "colors": { "accent": "#...", "bg_window": "#..." },
  "dotted_bg": {
    "enabled": true,
    "color": "#a8c3f0",
    "glow": 0.5
  }
}
```

**Defaults when `dotted_bg` is absent:**
- `enabled`: `false`
- `color`: lightened version of the theme's `@define-color accent`
- `glow`: `0.5`

**New helpers** in `theme.rs`:
- `load_dotted_bg(theme: Theme) -> DottedBgConfig`
- `save_dotted_bg(theme: Theme, cfg: &DottedBgConfig)`
- `lighten_accent(hex: &str) -> String` — HSL conversion, `L = min(0.85, L + 0.25)`, `S *= 0.8`.

Serde compatibility: missing `dotted_bg` block → defaults applied; malformed block → defaults + warning log. Same resilience pattern used by existing color loader.

## Rendering & animation

### Dot layout

- `DOT_SPACING = 24.0` (px)
- `DOT_RADIUS = 1.5` (px, base)
- `HALO_RADIUS = 6.0` (px, falloff extent around each dot center)
- Offset grid: every other row is shifted horizontally by `DOT_SPACING / 2.0`. Vertical spacing equals horizontal (`24px`), giving a diamond visual pattern (not a true hexagonal grid with equal edge distances — the staggering is purely aesthetic, to avoid an obvious rectilinear appearance).
- Dot indices are stable across frames; animation state indexed by `(row, col) -> i = row * cols + col`.

### Shimmer (desynced)

Per dot:
- `phase_i = (hash_u64(i) as f32 / u64::MAX as f32) * TAU`
- `base_alpha(t, i) = 0.30 + 0.15 * sin(t * 0.5 + phase_i)` → range `[0.15, 0.45]`

`t` is elapsed seconds since animation start. Frequency `0.5 rad/s` → period ≈ 12.5s.

### Mouse spotlight

- `d² = (dot.x - mouse.x)² + (dot.y - mouse.y)²`
- `falloff = exp(-d² / (160.0 * 160.0))`
- `spotlight_multiplier = 1.0 + SPOTLIGHT_STRENGTH * falloff * mouse_fade` with `SPOTLIGHT_STRENGTH = 1.5`
- `mouse_fade ∈ [0, 1]`: linearly ramps down over 400ms when mouse leaves the window, ramps up instantly when mouse enters.

### Glow intensity slider

- `glow ∈ [0, 1]` (from UI slider 0–100).
- Multiplies both shimmer amplitude and spotlight strength: `final_alpha = clamp(base_alpha * glow + spotlight_boost * glow, 0, 1)`.
- `glow = 0` effectively disables visibility even when `enabled = true` (gives user a "paused" state).

### Rendering path

1. **Sprite pre-render** (once, and on color/glow change, and on first resize):
   - Create `cairo::ImageSurface::create(Format::ARgb32, 16, 16)`.
   - Draw a radial gradient at center: inner stop = dot color (alpha 1.0), outer stop = same color (alpha 0.0), radius `HALO_RADIUS`.
   - Draw solid inner disk of `DOT_RADIUS` at alpha 1.0 on top.
   - Cache in `state.sprite`.

2. **Per-frame draw** (`DrawingArea::set_draw_func`):
   - If `!enabled` → return (clear output with zero alpha fill).
   - For each dot `i` at `(x, y)`:
     - compute `alpha_i = final_alpha(t, i, mouse)`.
     - `cr.set_source_surface(&sprite, x - 8.0, y - 8.0)`.
     - `cr.paint_with_alpha(alpha_i as f64)`.

Expected dot count at 1200×800: `floor(1200/24) * floor(800/24) ≈ 50 * 33 = 1650`. At 30fps: ~50k blits/s. Cairo handles this comfortably; we'll verify during manual testing.

### Frame scheduling

- `add_tick_callback` on the `DrawingArea`:
  - If `!config.enabled` → skip (no state update, no redraw).
  - If `!window.is_active()` → skip (no redraw; preserves last `mouse_fade` value so re-focus resumes smoothly).
  - If `frame_time_ns - last_frame_ns < 33_000_000` → skip.
  - Else → update `tick`, decay `mouse_fade`, `queue_draw()`.

## CSS changes

Added to `resources/style.css` (BASE_CSS):

```css
/* Transparency applied only when dotted bg is enabled */
window.has-dotted-bg .app-headerbar {
    background-color: alpha(@headerbar_bg_color, 0.75);
}
window.has-dotted-bg .app-toolbar-view {
    background-color: transparent;
}
/* Panels already carry .panel-frame with their own bg; leave opaque. */
```

The `.has-dotted-bg` class is added/removed on the `ApplicationWindow` in `app.rs` whenever `DottedBgConfig::enabled` changes. When disabled, header/tab bar are fully opaque (normal appearance).

## UI integration — Color Customizer dialog

New group appended after the existing 4 groups (Backgrounds / Text / Accents / Borders):

```
Background Effect
 ├─ [●──] Enable dotted glow background
 ├─ [🎨] Dot Color                              [Reset]
 └─ [────●────] Glow Intensity           (0 – 100)
```

- **Switch**: bound to `dotted_bg.enabled`. Toggling live-updates the window (adds/removes `.has-dotted-bg` class, calls `DottedBackground::set_config`).
- **ColorButton**: bound to `dotted_bg.color`. "Reset" restores `lighten_accent(current_theme.accent)`.
- **Scale (0–100)**: bound to `dotted_bg.glow` (divided by 100 on save). Live updates.
- **Save** path extends existing save to also write the `dotted_bg` block.
- **Close without save** reverts both color overrides and dotted_bg config (existing `connect_close_request` handler extended).

Enabled/disabled states: color button and glow slider remain interactive even when `enabled = false` (user can tune before turning on).

## Testing strategy

### Unit tests (no GTK runtime)

- `lighten_accent`:
  - Known inputs produce expected lightened output within tolerance.
  - Already-light colors are clamped at `L = 0.85`.
  - Malformed hex returns the documented fallback `#a8c3f0` rather than panicking.
- `dot_phase(i)`:
  - Deterministic: same `i` returns same phase.
  - Distribution over 1000 indices roughly uniform across `[0, 2π)` (coarse histogram check).
- `shimmer_alpha(t, phase, glow)`:
  - At `glow = 1.0`: output range `[0.15, 0.45]`.
  - Scales linearly with `glow`: `shimmer(t, p, 0.5) == shimmer(t, p, 1.0) * 0.5` within `±1e-6`.
- `spotlight_boost(dx, dy, strength, fade)`:
  - Maximum at `(0, 0)`.
  - `~37%` at `d = 160px`, `< 2%` at `d = 400px`.
  - `fade = 0` zeroes the boost.
- `DottedBgConfig` serde roundtrip:
  - Serialize → deserialize returns identical struct.
  - Deserialization of JSON without `dotted_bg` block returns defaults.
  - Deserialization of malformed `dotted_bg` block returns defaults (no panic).

### Integration tests

- `save_dotted_bg` + `load_dotted_bg` against a `tempfile::tempdir`: roundtrip preserves values; load on missing file returns defaults; load on corrupt file returns defaults and logs warning.
- Verify `.has-dotted-bg` CSS class is added/removed when enabling/disabling (may require a lightweight GTK init in a `#[test]` guarded by feature flag; if GTK init is too heavy, cover via a state-change unit test on the config-to-class decision logic).

### Manual verification (Test Plan in PR description)

- Toggle on/off live via dialog → background appears/disappears without restart.
- Change theme while enabled → dots update to the new theme's lightened accent.
- Change custom dot color → dots update live.
- Drag glow slider → intensity changes visibly in real time.
- Move mouse across window → dots near cursor brighten; fade out smoothly when mouse leaves.
- Window unfocus (click another window) → CPU usage drops (check with `top`/`htop`).
- Cross-theme screenshots: Graphite, Dracula, Aurora, Quantum — all with dotted bg enabled; verify panel readability and visual quality.
- Complex workspace: hsplit + vsplit + tabs layout — verify bg is visible in gaps and does not interfere with panel focus border.
- macOS build (`cargo build --no-default-features`): same behavior (Cairo is cross-platform).

## Performance budget & risks

- Expected CPU: a few % on typical workstation at 30fps for 1600 dots. If initial profiling shows worse, fallback options (in order):
  1. Reduce tick rate to 20fps.
  2. Render dots into a single cached surface per frame-group (update every N frames).
  3. Spatial culling around mouse: only recompute dots within spotlight radius, keep others at cached shimmer value.
- Risk: libadwaita widgets may ignore transparent background-color on some themes/versions. Mitigation: the CSS rules target our own classes (`.app-headerbar`, `.app-toolbar-view`) rather than libadwaita internals, and fall back cleanly if ignored (bg just stays opaque in header; dots still visible in gaps).

## Out of scope

- Animation styles other than shimmer+spotlight (pulse, drift, parallax) — can be added later as additional `DottedBgStyle` variants.
- Alternative dot patterns (scatter, variable radius) — future enhancement if user wants more visual variety.
- Global (non-per-theme) settings — rejected in favor of per-theme for consistency with color customization.
- Pre-rendered PNG/SVG static bg option — explicitly replaced by Cairo animated per user decision.
