# Panel sibling reordering — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add Move Left/Right/Up/Down to the panel ⋮ menu so the focused panel can swap position with its previous/next sibling within the parent split or tabs node.

**Architecture:** Reorder is implemented at the layout-tree level (`layout_ops`) reusing the existing `move_tab_in_layout` for Tabs and adding a sibling-swap for Hsplit/Vsplit. The panel menu is rebuilt on each ⋮ click via `MenuButton::set_create_popup_func`, querying a `SiblingInfoProvider` callback installed by `WorkspaceView` so the items reflect the current layout. Only applicable directions are shown.

**Tech Stack:** Rust 2021, gtk4 0.9 (`MenuButton::set_create_popup_func`), existing `LayoutNode` enum from `pax_core::workspace`.

**Spec:** `docs/superpowers/specs/2026-04-28-panel-move-siblings-design.md`

**Note on tests:** Per project convention (no unit tests in Pax commits unless explicitly asked), this plan does not add unit tests. Verification is manual.

---

## Task 1 — Layout-tree primitives

Add the data shape that describes a panel's position relative to its
siblings, plus a sibling-swap operation that handles all three container
kinds (Hsplit, Vsplit, Tabs).

**Files:**
- Modify: `crates/tp-gui/src/layout_ops.rs`

- [ ] **Step 1: Add `SiblingKind` and `SiblingInfo` types**

At the top of `crates/tp-gui/src/layout_ops.rs`, just below the
`use pax_core::workspace::{new_tab_id, LayoutNode};` line, insert:

```rust
/// Kind of immediate parent that contains a panel's siblings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiblingKind {
    Hsplit,
    Vsplit,
    Tabs,
}

/// A panel's position within the innermost split/tabs that directly
/// contains it. `None` from `panel_sibling_info` means root or only-child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SiblingInfo {
    pub kind: SiblingKind,
    pub index: usize,
    pub len: usize,
}
```

- [ ] **Step 2: Add `panel_sibling_info`**

At the bottom of `crates/tp-gui/src/layout_ops.rs`, before the
`#[cfg(test)] mod tests { ... }` block, add:

```rust
/// Walk the tree and return the sibling info for the innermost
/// split/tabs node that DIRECTLY contains the given panel as one of its
/// children (a Panel node, not nested deeper). Returns `None` if the
/// panel is the root, isn't found, or is the only child of its parent.
pub fn panel_sibling_info(node: &LayoutNode, panel_id: &str) -> Option<SiblingInfo> {
    fn walk(node: &LayoutNode, panel_id: &str) -> Option<SiblingInfo> {
        match node {
            LayoutNode::Panel { .. } => None,
            LayoutNode::Hsplit { children, .. } => walk_container(children, panel_id, SiblingKind::Hsplit),
            LayoutNode::Vsplit { children, .. } => walk_container(children, panel_id, SiblingKind::Vsplit),
            LayoutNode::Tabs { children, .. }   => walk_container(children, panel_id, SiblingKind::Tabs),
        }
    }
    fn walk_container(
        children: &[LayoutNode],
        panel_id: &str,
        kind: SiblingKind,
    ) -> Option<SiblingInfo> {
        // Recurse first so we always return the innermost match.
        for c in children {
            if let Some(info) = walk(c, panel_id) {
                return Some(info);
            }
        }
        // No deeper match — does this container directly hold the panel?
        if children.len() < 2 {
            return None;
        }
        for (i, c) in children.iter().enumerate() {
            if matches!(c, LayoutNode::Panel { id } if id == panel_id) {
                return Some(SiblingInfo {
                    kind,
                    index: i,
                    len: children.len(),
                });
            }
        }
        None
    }
    walk(node, panel_id)
}
```

- [ ] **Step 3: Add `move_panel_in_split`**

Append, just below `panel_sibling_info`:

```rust
/// Swap the panel's position in its innermost containing
/// Hsplit/Vsplit/Tabs with its previous (delta=-1) or next (delta=+1)
/// sibling. For splits, ratios are reordered in lockstep; for tabs,
/// labels and tab_ids too. Returns true on success.
///
/// Only direct Panel children are moved — if the panel is nested inside
/// a deeper split, that deeper split is the parent considered.
pub fn move_panel_in_split(node: &mut LayoutNode, panel_id: &str, delta: i32) -> bool {
    if delta == 0 {
        return false;
    }
    match node {
        LayoutNode::Panel { .. } => false,
        LayoutNode::Hsplit { children, ratios } => {
            for child in children.iter_mut() {
                if move_panel_in_split(child, panel_id, delta) {
                    return true;
                }
            }
            swap_direct_panel_in_split(children, Some(ratios), panel_id, delta)
        }
        LayoutNode::Vsplit { children, ratios } => {
            for child in children.iter_mut() {
                if move_panel_in_split(child, panel_id, delta) {
                    return true;
                }
            }
            swap_direct_panel_in_split(children, Some(ratios), panel_id, delta)
        }
        LayoutNode::Tabs { children, labels, tab_ids } => {
            for child in children.iter_mut() {
                if move_panel_in_split(child, panel_id, delta) {
                    return true;
                }
            }
            // Direct panel-child of a Tabs: reuse the existing helper so
            // labels and tab_ids stay in lockstep (move_tab_in_layout
            // already handles both vectors).
            for (i, c) in children.iter().enumerate() {
                if matches!(c, LayoutNode::Panel { id } if id == panel_id) {
                    let target = i as i32 + delta;
                    if !(0..children.len() as i32).contains(&target) {
                        return false;
                    }
                    let target = target as usize;
                    children.swap(i, target);
                    if i < labels.len() && target < labels.len() {
                        labels.swap(i, target);
                    }
                    if i < tab_ids.len() && target < tab_ids.len() {
                        tab_ids.swap(i, target);
                    }
                    return true;
                }
            }
            false
        }
    }
}

fn swap_direct_panel_in_split(
    children: &mut Vec<LayoutNode>,
    ratios: Option<&mut Vec<f64>>,
    panel_id: &str,
    delta: i32,
) -> bool {
    for (i, c) in children.iter().enumerate() {
        if matches!(c, LayoutNode::Panel { id } if id == panel_id) {
            let target = i as i32 + delta;
            if !(0..children.len() as i32).contains(&target) {
                return false;
            }
            let target = target as usize;
            children.swap(i, target);
            if let Some(r) = ratios {
                if i < r.len() && target < r.len() {
                    r.swap(i, target);
                }
            }
            return true;
        }
    }
    false
}
```

- [ ] **Step 4: Build**

Run: `cargo build`
Expected: clean build, no new warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/layout_ops.rs
git commit -m "layout_ops: add panel_sibling_info and move_panel_in_split helpers"
```

---

## Task 2 — PanelAction variants + smart menu builder

Extend the action enum with the four new directions and rebuild
`build_panel_menu` so it can take an optional `SiblingInfo` and
conditionally include only the applicable Move items.

**Files:**
- Modify: `crates/tp-gui/src/panel_host.rs`

- [ ] **Step 1: Add four variants to `PanelAction`**

In `crates/tp-gui/src/panel_host.rs`, locate the `pub enum PanelAction { ... }` (line ~60). After the `Focus,` variant and before the closing brace, insert:

```rust
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
```

- [ ] **Step 2: Update icon and hint match arms in `build_panel_menu`**

Find the `let icon_name = match action { ... }` block (line ~965). Add four arms before the closing brace:

```rust
            PanelAction::MoveLeft  => "go-previous-symbolic",
            PanelAction::MoveRight => "go-next-symbolic",
            PanelAction::MoveUp    => "go-up-symbolic",
            PanelAction::MoveDown  => "go-down-symbolic",
```

Then find the `let hint_text = match action { ... }` block (line ~995). Add four arms with empty hints (no keyboard shortcuts in v1):

```rust
            PanelAction::MoveLeft  => "",
            PanelAction::MoveRight => "",
            PanelAction::MoveUp    => "",
            PanelAction::MoveDown  => "",
```

- [ ] **Step 3: Refactor `build_panel_menu` signature to accept `Option<SiblingInfo>`**

Change the function declaration at `crates/tp-gui/src/panel_host.rs:925` from:

```rust
fn build_panel_menu(panel_id: &str, action_cb: Option<PanelActionCallback>) -> gtk4::Popover {
```

to:

```rust
fn build_panel_menu(
    panel_id: &str,
    action_cb: Option<PanelActionCallback>,
    sibling_info: Option<crate::layout_ops::SiblingInfo>,
) -> gtk4::Popover {
```

- [ ] **Step 4: Inject Move items into the items vector based on sibling_info**

In `build_panel_menu`, immediately after the existing `let items: Vec<(&str, &str, PanelAction)> = vec![ ... ];` (ends at line ~953), insert the conditional Move-items injection:

```rust
    // Append Move items based on the current parent kind + position.
    // Only directions with a valid target are shown — no disabled rows.
    let mut items = items;
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
                    items.push((
                        "Move Up",
                        "Swap with previous sibling",
                        PanelAction::MoveUp,
                    ));
                }
                if info.index + 1 < info.len {
                    items.push((
                        "Move Down",
                        "Swap with next sibling",
                        PanelAction::MoveDown,
                    ));
                }
            }
        }
    }
```

- [ ] **Step 5: Update the existing call in `set_action_callback`**

Locate `set_action_callback` (~line 513). The current call is:

```rust
let popover = build_panel_menu(&self.panel_id, Some(cb));
self.menu_button.set_popover(Some(&popover));
```

Replace with:

```rust
let popover = build_panel_menu(&self.panel_id, Some(cb), None);
self.menu_button.set_popover(Some(&popover));
```

(The `None` is a placeholder for the static install; Task 3 makes the menu rebuild on each click using the real provider.)

- [ ] **Step 6: Find the other call to `build_panel_menu`**

There is a call at PanelHost construction time (around line 320 in `PanelHost::new`). Search for `let popover = build_panel_menu(` and update that call too:

```rust
let popover = build_panel_menu(panel_id, action_cb.clone(), None);
```

- [ ] **Step 7: Build**

Run: `cargo build`
Expected: clean build.

- [ ] **Step 8: Commit**

```bash
git add crates/tp-gui/src/panel_host.rs
git commit -m "panel_host: add Move Left/Right/Up/Down PanelAction variants and conditional menu items"
```

---

## Task 3 — Per-click popover rebuild + sibling info provider

Wire `MenuButton::set_create_popup_func` so the popover is rebuilt
fresh on every ⋮ click, using a provider callback that the
`WorkspaceView` installs to compute current sibling info.

**Files:**
- Modify: `crates/tp-gui/src/panel_host.rs`

- [ ] **Step 1: Add the provider type and the host field**

In `crates/tp-gui/src/panel_host.rs`, just below `pub type PanelActionCallback = Rc<dyn Fn(&str, PanelAction)>;` (~line 118), add:

```rust
/// Callback that returns the current `SiblingInfo` for a panel — used by
/// the panel menu to decide which Move items to show. Returning `None`
/// hides all Move items (e.g. root or only-child).
pub type SiblingInfoProvider =
    Rc<dyn Fn(&str) -> Option<crate::layout_ops::SiblingInfo>>;
```

In the `PanelHost` struct (~line 121), add a new field next to `action_cb_ref`:

```rust
    sibling_info_provider_ref: Rc<RefCell<Option<SiblingInfoProvider>>>,
```

- [ ] **Step 2: Initialize the new field**

In `PanelHost::new` (~line 171), add the initializer near `let action_cb_ref: Rc<RefCell<Option<PanelActionCallback>>> = ...`:

```rust
let sibling_info_provider_ref: Rc<RefCell<Option<SiblingInfoProvider>>> =
    Rc::new(RefCell::new(None));
```

In the `Self { ... }` literal (~line 487), add the field:

```rust
            sibling_info_provider_ref: sibling_info_provider_ref.clone(),
```

- [ ] **Step 3: Install `set_create_popup_func` on the menu button**

In `PanelHost::new`, after the line `menu_button.set_popover(Some(&popover));` (~line 320), add:

```rust
{
    let panel_id_c = panel_id.to_string();
    let action_ref = action_cb_ref.clone();
    let sib_ref = sibling_info_provider_ref.clone();
    menu_button.set_create_popup_func(move |btn| {
        let action_cb = action_ref.borrow().clone();
        let sibling_info = sib_ref
            .borrow()
            .as_ref()
            .and_then(|f| f(&panel_id_c));
        let popover = build_panel_menu(&panel_id_c, action_cb, sibling_info);
        btn.set_popover(Some(&popover));
    });
}
```

- [ ] **Step 4: Add `set_sibling_info_provider` setter**

After `set_action_callback` (~line 513), add:

```rust
    /// Install a closure that the menu uses (on each ⋮ open) to compute
    /// the current panel's `SiblingInfo`. Pass through the `WorkspaceView`
    /// so the rebuilt menu reflects the live layout.
    pub fn set_sibling_info_provider(&self, provider: SiblingInfoProvider) {
        if let Ok(mut r) = self.sibling_info_provider_ref.try_borrow_mut() {
            *r = Some(provider);
        }
    }
```

- [ ] **Step 5: Build**

Run: `cargo build`
Expected: clean build. If gtk4-rs's `set_create_popup_func` isn't found, search the gtk4 `MenuButton` API in the Cargo.lock'ed version (0.9) — it should be present.

- [ ] **Step 6: Commit**

```bash
git add crates/tp-gui/src/panel_host.rs
git commit -m "panel_host: rebuild panel menu on each ⋮ click using SiblingInfoProvider"
```

---

## Task 4 — WorkspaceView: query + move methods

Add the workspace-side helpers that the host menu and the action
dispatcher will call.

**Files:**
- Modify: `crates/tp-gui/src/workspace_view.rs`

- [ ] **Step 1: Add `MoveDirection` enum**

At a sensible top-of-impl location in `crates/tp-gui/src/workspace_view.rs` — for example just above the `pub fn move_tab_by_panel_id` method (~line 571), but at module scope, add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDirection {
    Left,
    Right,
    Up,
    Down,
}
```

(Place it next to other module-scope items, e.g. right above the `impl WorkspaceView { ... }` block where `move_tab_by_panel_id` lives, or in the existing module-scope items area near the top of the file.)

- [ ] **Step 2: Add `panel_sibling_info` on `WorkspaceView`**

Inside the `impl WorkspaceView { ... }` block (find any public method like `pub fn move_tab_by_panel_id`), add as a sibling method:

```rust
    /// Compute the sibling info for a panel — used by the panel menu to
    /// decide which Move items to show.
    pub fn panel_sibling_info(
        &self,
        panel_id: &str,
    ) -> Option<crate::layout_ops::SiblingInfo> {
        crate::layout_ops::panel_sibling_info(&self.workspace.layout, panel_id)
    }
```

- [ ] **Step 3: Add `move_focused_panel`**

Right after `move_tab_by_panel_id` (~line 590), add:

```rust
    /// Move the focused panel by one position in its parent split or
    /// tabs node. Returns true if the move happened. Picks the correct
    /// container kind by inspecting the parent — directions that don't
    /// match the parent (e.g. `Up` on an Hsplit) silently no-op so the
    /// caller doesn't need to dispatch by kind.
    pub fn move_focused_panel(&mut self, direction: MoveDirection) -> bool {
        let Some(focused_id) = self.focused_panel_id().map(|s| s.to_string()) else {
            return false;
        };
        let Some(info) = self.panel_sibling_info(&focused_id) else {
            return false;
        };

        use crate::layout_ops::SiblingKind;
        let delta = match (info.kind, direction) {
            (SiblingKind::Hsplit, MoveDirection::Left)  => -1,
            (SiblingKind::Hsplit, MoveDirection::Right) =>  1,
            (SiblingKind::Tabs,   MoveDirection::Left)  => -1,
            (SiblingKind::Tabs,   MoveDirection::Right) =>  1,
            (SiblingKind::Vsplit, MoveDirection::Up)    => -1,
            (SiblingKind::Vsplit, MoveDirection::Down)  =>  1,
            _ => return false,
        };

        let moved = crate::layout_ops::move_panel_in_split(
            &mut self.workspace.layout,
            &focused_id,
            delta,
        );
        if !moved {
            return false;
        }

        self.rebuild_layout();
        self.rebuild_focus_order();
        if let Some(index) = self.focus.order.iter().position(|id| id == &focused_id) {
            self.focus.index = index;
            self.focus.focus_current_pub(&self.hosts);
        }
        self.select_workspace_tab_for_panel(&focused_id);
        self.dirty = true;
        true
    }
```

- [ ] **Step 4: Build**

Run: `cargo build`
Expected: clean build.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/workspace_view.rs
git commit -m "workspace_view: add panel_sibling_info and move_focused_panel"
```

---

## Task 5 — Wire the provider and dispatch actions in app.rs

Install the sibling-info provider on every panel host so the menu
rebuilds with current data, and dispatch the four new actions.

**Files:**
- Modify: `crates/tp-gui/src/app.rs`

- [ ] **Step 1: Find where `PanelHost`s receive their action callback**

Search for `set_action_callback(` in `crates/tp-gui/src/app.rs`. There is at least one call site that wires the action callback for each host (typically inside a loop or a per-host setup). The same place is the right spot to wire the sibling-info provider.

```bash
grep -n "set_action_callback" crates/tp-gui/src/app.rs
```

- [ ] **Step 2: Install the sibling-info provider next to the action callback**

At every call site of `host.set_action_callback(cb.clone())` in app.rs, insert immediately after it:

```rust
let ws_for_provider = ws.clone();
host.set_sibling_info_provider(Rc::new(move |panel_id| {
    ws_for_provider.borrow().panel_sibling_info(panel_id)
}));
```

(`ws` here is the `Rc<RefCell<WorkspaceView>>` already in scope at the call site. Use whatever local name app.rs already has for that reference.)

- [ ] **Step 3: Dispatch the four new `PanelAction`s**

Locate the `match action { ... }` dispatcher (~line 1262, between `PanelAction::SplitH` and `PanelAction::Configure`). Just before the closing brace of the match (after the last existing arm), add:

```rust
                PanelAction::MoveLeft => {
                    if ws_for_cb.borrow_mut().move_focused_panel(crate::workspace_view::MoveDirection::Left) {
                        sb_for_cb.borrow().set_message("Move Left");
                    }
                }
                PanelAction::MoveRight => {
                    if ws_for_cb.borrow_mut().move_focused_panel(crate::workspace_view::MoveDirection::Right) {
                        sb_for_cb.borrow().set_message("Move Right");
                    }
                }
                PanelAction::MoveUp => {
                    if ws_for_cb.borrow_mut().move_focused_panel(crate::workspace_view::MoveDirection::Up) {
                        sb_for_cb.borrow().set_message("Move Up");
                    }
                }
                PanelAction::MoveDown => {
                    if ws_for_cb.borrow_mut().move_focused_panel(crate::workspace_view::MoveDirection::Down) {
                        sb_for_cb.borrow().set_message("Move Down");
                    }
                }
```

If the dispatch is a `match action { ... }` with `_ =>` catch-all somewhere, place these new arms before the catch-all.

- [ ] **Step 4: Confirm the focus pre-step covers the new actions**

The dispatcher already focuses the panel that triggered the action at lines ~1254-1261 (`Focus the panel that triggered the action` block) before entering `match action`. Since `move_focused_panel` reads `focused_panel_id`, the new arms automatically operate on the panel whose ⋮ menu was used. No change needed unless that block has been altered to skip a subset of actions.

- [ ] **Step 5: Build**

Run: `cargo build`
Expected: clean build.

- [ ] **Step 6: Commit**

```bash
git add crates/tp-gui/src/app.rs
git commit -m "app: dispatch Move Left/Right/Up/Down + install panel sibling-info provider"
```

---

## Task 6 — End-to-end manual verification

- [ ] **Step 1: Run a complex workspace and exercise each scenario**

```bash
cargo run -- new "move-test"
```

In the running app, build the layouts below using existing actions
(Split H/V from the panel menu, Add Tab, etc.) and verify each row.

| # | Layout                                 | Focus | Expected menu items                  | Action          | Expected after                        |
|---|----------------------------------------|-------|--------------------------------------|-----------------|---------------------------------------|
| 1 | `Hsplit[A, B, C]`                      | B     | Move Left + Move Right (no Up/Down)  | Move Right      | `Hsplit[A, C, B]`, B still focused    |
| 2 | `Hsplit[A, B]`                         | A     | Only Move Right                      | Move Right      | `Hsplit[B, A]`, focus on A            |
| 3 | `Hsplit[A, B]` (post-#2 state)         | A     | Only Move Left                       | Move Left       | `Hsplit[A, B]`                        |
| 4 | `Vsplit[X, Y]`                         | X     | Only Move Down                       | Move Down       | `Vsplit[Y, X]`                        |
| 5 | `Tabs[t1, t2, t3]` labels A,B,C        | t2    | Move Left + Move Right               | Move Left       | tabs reorder to [t2, t1, t3], labels follow, focus stays on t2 |
| 6 | Single panel (no parent)               | —     | No Move items at all                 | —               | —                                     |
| 7 | `Tabs[ Hsplit[A, B], C ]` labels X,Y   | A     | Move Right only (Hsplit, not Tabs)   | Move Right      | `Tabs[ Hsplit[B, A], C ]`, outer labels X,Y unchanged |

- [ ] **Step 2: Sanity-check ratios**

After moving panels in an Hsplit/Vsplit where you previously dragged the
divider to a non-50/50 ratio, confirm the dragged column **follows the
panel** (i.e. ratios swap in lockstep with children). Otherwise the
panel would jump to the neighbour's old size.

Build a 3-panel `Hsplit[A, B, C]`, drag the A/B divider so A is wide and
B narrow. Move B Right. Now the layout should be `Hsplit[A, C, B]`;
A's width unchanged, C taking what was B's narrow slot, B in C's old
slot.

- [ ] **Step 3: Save → reload roundtrip**

Save the workspace (Ctrl+S) after a Move action, close the app, and
reopen the same workspace JSON. The new order should persist.

- [ ] **Step 4: Final fixup commit (only if anything surfaces)**

```bash
git add -p
git commit -m "fixup: ..."
```

---

## File map

| File                                      | Touched in task |
|-------------------------------------------|-----------------|
| `crates/tp-gui/src/layout_ops.rs`         | 1               |
| `crates/tp-gui/src/panel_host.rs`         | 2, 3            |
| `crates/tp-gui/src/workspace_view.rs`     | 4               |
| `crates/tp-gui/src/app.rs`                | 5               |
| `docs/superpowers/specs/2026-04-28-panel-move-siblings-design.md` | (already committed) |
