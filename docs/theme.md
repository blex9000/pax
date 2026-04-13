# Theme System

Il sistema temi di Pax vive in [`crates/tp-gui/src/theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs).

## Struttura

Ci sono 3 livelli:

1. `BASE_CSS`
   Regole CSS comuni a tutti i temi. Qui si decide quale componente usa quale token colore.

2. Palette semantica `bg_*`
   Ogni tema definisce una palette di superfici semanticamente nominate. Questa e` la zona da modificare per cambiare il look di un tema senza rincorrere i componenti.

3. Alias compatibili
   I token storici (`window_bg_color`, `headerbar_bg_color`, `view_bg_color`, ecc.) vengono assegnati dai `bg_*`. Questo evita di dover rifattorizzare tutto il CSS in una volta sola.

## Dove cambiare un tema

I temi si trovano in fondo a [`theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs):

- `CATPPUCCIN_MOCHA_CSS`
- `GRAPHITE_CSS`
- `CATPPUCCIN_LATTE_CSS`
- `DRACULA_CSS`
- `NORD_CSS`

Per cambiare un tema, in genere basta toccare solo i token `bg_*` nel blocco di quel tema.

## Palette Semantica Background

Questi sono i token background da usare come riferimento:

- `bg_primary`
  Superficie principale dell'app. Base di finestra e contenuti primari.

- `bg_secondary`
  Chrome principale. Toolbar, headerbar, superfici top-level dell'app.

- `bg_tertiary`
  Chrome secondario. Oggi usato soprattutto per la barra dei tab split quando serve differenziarla da `bg_secondary`.

- `bg_card`
  Card e superfici promozionali/boxed.

- `bg_dialog`
  Finestre dialog/settings/config.

- `bg_popover`
  Popup, menu e popover.

- `bg_sidebar`
  Sidebar principali.

- `bg_sidebar_secondary`
  Sidebar secondarie o superfici laterali piu' profonde.

- `bg_thumbnail`
  Surface per thumbnail/preview container.

- `bg_view`
  Area contenuto editor/file preview/form controls.

- `bg_panel_header`
  Header dei panel focused e selected workspace tab.

- `bg_terminal`
  Surface del terminale.

## Alias Legacy

Questi token vengono ancora usati dal CSS base ma sono derivati dai `bg_*`:

- `window_bg_color <- bg_primary`
- `headerbar_bg_color <- bg_secondary`
- `workspace_tabs_bar_bg_color <- bg_tertiary`
- `card_bg_color <- bg_card`
- `dialog_bg_color <- bg_dialog`
- `popover_bg_color <- bg_popover`
- `sidebar_bg_color <- bg_sidebar`
- `secondary_sidebar_bg_color <- bg_sidebar_secondary`
- `thumbnail_bg_color <- bg_thumbnail`
- `view_bg_color <- bg_view`
- `panel_header_bg_color <- bg_panel_header`
- `terminal_bg_color <- bg_terminal`

## Mapping Componenti -> Token

I mapping principali oggi sono questi:

- Finestra app / root background
  - `window_bg_color`
  - Rif: [`theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs#L298)

- Top bar / headerbar app
  - `headerbar_bg_color`
  - Rif: [`theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs#L299)

- Toolbar panel / footer panel / markdown toolbar / status bar
  - `headerbar_bg_color`
  - Rif: [`theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs#L318)

- Selected workspace tab / focused panel header
  - `panel_header_bg_color`
  - Rif: [`theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs#L496)
  - Rif: [`theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs#L628)

- Tab split bar
  - `workspace_tabs_bar_bg_color`
  - Rif: [`theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs#L433)

- File tree / preview / editor side panes / form controls
  - `view_bg_color`
  - Rif: [`theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs#L760)
  - Rif: [`theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs#L836)

- Terminale e footer terminale
  - `terminal_bg_color`
  - Rif: [`theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs#L693)

- Sidebar principali
  - `sidebar_bg_color`
  - Rif: [`theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs#L760)

- Popup / popover
  - `popover_bg_color`
  - Rif: [`theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs#L599)

- Card / welcome actions
  - `card_bg_color`
  - Rif: [`theme.rs`](/home/xb/dev/me/pax/crates/tp-gui/src/theme.rs#L835)

## Come chiedere modifiche

Esempi di richieste precise:

- "Nel tema Graphite schiarisci solo `bg_view`"
  Cambia editor, file preview e form controls senza toccare toolbar e popup.

- "Rendi popup uguali alle toolbar"
  Allinea `bg_popover` a `bg_secondary`.

- "Scurisci il chrome dei tab split"
  Tocca `bg_tertiary`.

- "Rendi header panel e tab selezionato piu' evidenti"
  Tocca `bg_panel_header`.

- "Rendi sidebars piu' distaccate dal content"
  Differenzia `bg_sidebar` e `bg_view`.

## Colori ancora hardcoded

Questi non passano ancora dalla palette tema:

- `dirty-indicator` arancione
- icona sync attiva
- icona zoom attiva
- stato git changes arancione

Se vuoi un sistema completamente theme-driven, il passo successivo e` tokenizzare anche questi.
