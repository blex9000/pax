# Panel sibling reordering

## Contesto

Pax permette di costruire layout complessi (Hsplit / Vsplit / Tabs annidati)
ma una volta posizionato un panel non c'è modo di riordinarlo rispetto ai
suoi fratelli senza distruggerlo e ricrearlo. L'unica forma di reorder
esistente è per i tab: `move_tab_in_layout`
(`crates/tp-gui/src/layout_ops.rs:504`) e il wrapper
`WorkspaceView::move_tab_by_panel_id`. Niente di analogo per panel dentro
Hsplit/Vsplit.

L'obiettivo è aggiungere al menu per-panel (⋮) le voci **Move Left /
Move Right / Move Up / Move Down**, attive solo quando l'azione è
applicabile in base al parent del panel focalizzato.

## Comportamento atteso

- Il move sposta il panel di **una posizione** tra i fratelli del parent
  diretto. Non attraversa il parent.
- Il menu mostra **solo le direzioni applicabili**:
  - parent Hsplit → Move Left e/o Move Right
  - parent Vsplit → Move Up e/o Move Down
  - parent Tabs → Move Left e/o Move Right (reorder del tab)
  - panel al primo posto → solo "in avanti" (Right per Hsplit/Tabs, Down per Vsplit)
  - panel all'ultimo posto → solo "indietro"
  - panel root o figlio unico → nessuna voce Move
- Dopo il move il focus rimane sul panel spostato.

## Architettura

L'implementazione si divide in tre layer, ciascuno con responsabilità
chiara.

### 1. Layout-tree (`crates/tp-gui/src/layout_ops.rs`)

Due aggiunte:

```rust
pub enum SiblingKind { Hsplit, Vsplit, Tabs }

pub struct SiblingInfo {
    pub kind: SiblingKind,
    pub index: usize,
    pub len: usize,
}

/// Find the innermost Hsplit/Vsplit/Tabs that directly contains the panel,
/// returning kind + position + sibling count. None if root or only child.
pub fn panel_sibling_info(node: &LayoutNode, panel_id: &str) -> Option<SiblingInfo>;

/// Swap the panel's position with its previous (delta=-1) or next (delta=+1)
/// sibling in the innermost containing split/tabs. Reorders ratios for splits
/// and labels/tab_ids for tabs in lockstep. Returns true if the swap happened.
pub fn move_panel_in_split(node: &mut LayoutNode, panel_id: &str, delta: i32) -> bool;
```

Per i tab `move_panel_in_split` delega a `move_tab_in_layout` (già
esistente) per evitare duplicazione.

### 2. Action enum + menu builder (`crates/tp-gui/src/panel_host.rs`)

Estendere `PanelAction`:

```rust
pub enum PanelAction {
    // ... esistenti ...
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
}
```

Il `build_panel_menu` oggi è chiamato una volta in
`set_action_callback`. Per riflettere lo stato corrente del layout
(che cambia con split/move/close), il menu va **ricostruito al momento
dell'apertura** del popover ⋮. La firma diventa:

```rust
fn build_panel_menu(
    panel_id: &str,
    action_cb: &PanelActionCallback,
    sibling_info: Option<SiblingInfo>,
) -> gtk4::Popover;
```

Le voci Move vengono incluse solo se applicabili:

- `Some(SiblingInfo { kind: Hsplit, index, len })`:
  - `index > 0` → Move Left
  - `index < len - 1` → Move Right
- `Some(SiblingInfo { kind: Vsplit, index, len })`: Move Up / Move Down con stessa logica
- `Some(SiblingInfo { kind: Tabs, index, len })`: Move Left / Move Right
- `None`: nessuna voce Move

L'host expone un nuovo callback "give me sibling info for panel X" che
chiede al `WorkspaceView` di calcolare l'info al momento del click. Il
popover viene rigenerato a ogni apertura.

Etichette inglesi (in linea col resto: "Configure", "Split Horizontal",
"Reset Panel", ecc.). Icone:

- Move Left → `go-previous-symbolic`
- Move Right → `go-next-symbolic`
- Move Up → `go-up-symbolic`
- Move Down → `go-down-symbolic`

### 3. Glue (`workspace_view.rs` + `app.rs`)

`WorkspaceView` esporta:

```rust
pub fn panel_sibling_info(&self, panel_id: &str) -> Option<SiblingInfo>;

pub fn move_focused_panel(&mut self, direction: MoveDirection) -> bool;
```

Dove `MoveDirection { Left, Right, Up, Down }` mappa ai delta
appropriati per il parent kind. La funzione: chiama
`layout_ops::move_panel_in_split`, se ha avuto effetto chiama
`rebuild_layout()`, ri-focalizza il panel, marca `dirty = true`.

`app.rs` aggiunge 4 arm al match dispatcher di `PanelAction`:

```rust
PanelAction::MoveLeft  => { ws.borrow_mut().move_focused_panel(MoveDirection::Left); ... }
PanelAction::MoveRight => { ws.borrow_mut().move_focused_panel(MoveDirection::Right); ... }
PanelAction::MoveUp    => { ws.borrow_mut().move_focused_panel(MoveDirection::Up); ... }
PanelAction::MoveDown  => { ws.borrow_mut().move_focused_panel(MoveDirection::Down); ... }
```

## File coinvolti

- `crates/tp-gui/src/layout_ops.rs` — `SiblingKind`, `SiblingInfo`,
  `panel_sibling_info`, `move_panel_in_split` (+ test unitari sui casi
  Hsplit/Vsplit/Tabs e edge: primo, ultimo, figlio unico, panel non
  trovato, panel annidato in split-of-split).
- `crates/tp-gui/src/panel_host.rs` — variants `PanelAction::Move*`,
  refactor `build_panel_menu` per accettare `Option<SiblingInfo>` e
  rigenerarsi al click di ⋮.
- `crates/tp-gui/src/workspace_view.rs` — `MoveDirection`,
  `move_focused_panel`, `panel_sibling_info`.
- `crates/tp-gui/src/app.rs` — dispatch dei 4 nuovi `PanelAction`.

## Out of scope (YAGNI v1)

- Scorciatoie da tastiera (Alt+frecce / Ctrl+Shift+frecce) — aggiungibili dopo se servono.
- Move cross-container (uscire dal Tabs verso il parent Hsplit, ecc.) — la spec è "fratelli only".
- Animazione del movimento — swap istantaneo, rebuild widget tree.
- Mostrare voci disabilitate invece di nasconderle — la scelta è hide-when-N/A per ridurre rumore visivo.

## Verifica end-to-end

1. **Hsplit a 3 panel**: layout `Hsplit[A, B, C]`, focus su B. Apri ⋮:
   menu mostra Move Left + Move Right (no Up/Down). Clic Move Right →
   layout `Hsplit[A, C, B]`, focus resta su B.
2. **Bordo Hsplit**: layout `Hsplit[A, B]`, focus A. Menu mostra solo
   Move Right. Clic → `Hsplit[B, A]`. Riapri menu, ora A è ultimo →
   solo Move Left.
3. **Vsplit**: `Vsplit[X, Y]`, focus X. Menu mostra Move Down (e basta).
4. **Tabs**: `Tabs[t1, t2, t3]` con label "A","B","C", focus t2. Menu
   mostra Move Left + Move Right. Clic Move Left → tabs riordinati
   `[t2, t1, t3]`, label conservate, focus su t2 (ora index 0).
5. **Single panel**: layout `Panel(A)`. Menu non mostra alcuna voce
   Move.
6. **Panel annidato**: `Tabs[ Hsplit[A, B], C ]`, focus A. Sibling info
   trova Hsplit (parent diretto), non Tabs. Menu mostra Move Right
   solamente. Clic → `Tabs[ Hsplit[B, A], C ]`. Le tab labels (esterne)
   restano invariate.
7. **Build & test**: `cargo build` clean, `cargo test --lib layout_ops`
   verde.
