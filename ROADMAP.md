# Pax — Roadmap

## Visione

**Pax** è un workspace manager GUI in Rust, stile Tilix/Terminator, con pannelli di tipi eterogenei. Non è un terminale dentro un terminale: è una finestra nativa con pannelli separati che possono essere:

- **Terminale locale** — shell con emulatore VTE completo
- **Terminale SSH** — connessione remota via russh
- **Tmux remoto** — aggancia/crea sessioni tmux su host remoti
- **Markdown viewer** — note .md renderizzate
- **Browser embed** — WebView per dashboard, Grafana, docs
- *(futuro)* Log viewer, monitor sistema, editor, ...

I pannelli sono organizzati in layout configurabili (hsplit, vsplit, tabs) e raggruppabili per broadcast simultaneo.

---

## Piattaforme target

| Piattaforma | Supporto | Terminale | Note |
|-------------|----------|-----------|------|
| **Linux** | Primario | VTE4 (nativo, completo) | GTK4 + libadwaita + VTE4 |
| **macOS** | Supportato | PTY + vt100 + TextView (fallback) | GTK4 + libadwaita via Homebrew, no VTE |

### Compilazione condizionale

Il crate `pax-gui` usa feature flags per gestire le differenze tra piattaforme:

| Feature | Default | Descrizione |
|---------|---------|-------------|
| `vte` | Sì (Linux) | Abilita VTE4 per terminale nativo completo |

- **Linux**: `cargo build` — usa VTE4, supporto completo colori, mouse, resize, hyperlink
- **macOS**: `cargo build --no-default-features` — fallback PTY + vt100 parser + GtkTextView

Il backend fallback spawna un PTY reale e renderizza via vt100 crate in un TextView monospace. Funzionale per shell e comandi, senza supporto colori ANSI nella UI (il parsing avviene, la resa grafica è semplificata).

### Dipendenze sistema

**Linux (Ubuntu/Debian)**:
```bash
sudo apt install libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev
# Opzionale per pannello browser:
sudo apt install libwebkitgtk-6.0-dev
```

**macOS (Homebrew)**:
```bash
brew install gtk4 libadwaita pkg-config
```

---

## Stack tecnologico

| Componente | Tecnologia | Linux | macOS |
|------------|-----------|-------|-------|
| GUI framework | **GTK4 + libadwaita** (gtk4-rs) | Nativo | Via Homebrew |
| Terminale | **VTE4** / **PTY+vt100 fallback** | VTE4 completo | Fallback TextView |
| Browser embed | **WebKitGTK** (opzionale) | Sì | No |
| Markdown | **GTK4 TextView** + pulldown-cmark | Sì | Sì |
| SSH | **russh** | Sì | Sì |
| Config | **serde + JSON** | Sì | Sì |
| DB | **rusqlite** (bundled, FTS5) | Sì | Sì |
| Async | **tokio** | Sì | Sì |
| CLI | **clap** | Sì | Sì |

### Alternative considerate

| Opzione | Pro | Contro | Decisione |
|---------|-----|--------|-----------|
| Tauri + xterm.js | Cross-platform nativo, UI web | Terminale web meno performante, overhead Chromium | Scartato |
| VS Code fork | Già ha terminali, estensioni, UI | 1.5M righe, Electron = 300+ MB RAM, merge hell | Scartato |
| ratatui (TUI) | Leggero, funziona via SSH | Non può avere pannelli eterogenei | Scartato — era l'approccio v0 |
| Iced | Pure Rust, no binding C | Nessun widget terminale maturo | Scartato |

---

## Architettura

```
┌──────────────────────────────────────────────────────────────┐
│                       pax (GUI)                           │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │             pax-gui (GTK4 + libadwaita)                   │ │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────────┐ │ │
│  │  │ Terminal  │ │ Terminal │ │ Markdown │ │  Browser   │ │ │
│  │  │ (VTE4 o  │ │ SSH      │ │ Viewer   │ │ (WebKit)   │ │ │
│  │  │ fallback) │ │          │ │          │ │            │ │ │
│  │  └──────────┘ └──────────┘ └──────────┘ └────────────┘ │ │
│  │          ↕ PanelBackend trait (polimorfismo)             │ │
│  └──────────────────────────────────────────────────────────┘ │
│           ▼                    ▼              ▼               │
│   ┌────────────┐    ┌──────────────┐  ┌──────────┐          │
│   │  pax-core   │    │   pax-pty     │  │  pax-db   │          │
│   │ modelli    │    │ PTY locale   │  │ rusqlite │          │
│   │ config     │    │ SSH session  │  │ FTS5     │          │
│   │ alert      │    │ broadcast    │  │ history  │          │
│   │ safety     │    │ output buf   │  │          │          │
│   └────────────┘    └──────────────┘  └──────────┘          │
│                                                               │
│   pax-cli: pax launch / list / search / init / edit       │
└──────────────────────────────────────────────────────────────┘
```

### Panel backend trait

Ogni tipo di pannello implementa un trait comune:

```rust
pub trait PanelBackend {
    fn panel_type(&self) -> &str;              // "terminal", "ssh", "markdown", "browser"
    fn widget(&self) -> &gtk4::Widget;         // il widget GTK da inserire nel layout
    fn on_focus(&self);
    fn on_blur(&self) {}
    fn write_input(&self, data: &[u8]) -> bool { false }
    fn get_text_content(&self) -> Option<String> { None }
    fn accepts_input(&self) -> bool { false }
}
```

Aggiungere un nuovo tipo di pannello = implementare il trait + registrarlo in `workspace_view.rs`.

### Terminal backend condizionale

```rust
// Linux (feature = "vte"):   VTE4 nativo — completo
// macOS (no feature "vte"):  PTY + vt100 + GtkTextView — funzionale
#[cfg(feature = "vte")]     mod backend { /* VTE4 */ }
#[cfg(not(feature = "vte"))] mod backend { /* PTY + vt100 + TextView */ }
```

Entrambi i backend espongono la stessa API pubblica (`TerminalPanel::new()`, `send_commands()`, `write_input()`). Il codice applicativo non sa quale backend è in uso.

---

## Struttura progetto

```
pax/
├── Cargo.toml                      # workspace root
├── crates/
│   ├── pax-core/src/                # Modelli, config, alert, safety
│   │   ├── workspace.rs            # Workspace, PanelConfig, PanelType, LayoutNode
│   │   ├── config.rs               # Load/save/validate JSON
│   │   ├── ssh.rs                  # Parser ~/.ssh/config
│   │   ├── safety.rs               # Blocklist regex per gruppo
│   │   ├── alert.rs                # Regex pattern matching su output
│   │   └── template.rs             # Generatori workspace template
│   ├── pax-pty/src/                 # PTY + SSH
│   │   ├── manager.rs              # Spawn, resize, kill PTY
│   │   ├── multiplexer.rs          # Broadcast con safety check
│   │   ├── output.rs               # Ring buffer + alert scan
│   │   └── ssh.rs                  # (futuro) Sessioni SSH via russh
│   ├── pax-db/src/                  # SQLite embedded
│   │   ├── schema.rs               # Migrazioni SQL + FTS5
│   │   ├── commands.rs             # History comandi
│   │   ├── output.rs               # Output salvato
│   │   └── workspaces.rs           # Metadata workspace
│   ├── pax-gui/src/                 # GUI GTK4 (cross-platform)
│   │   ├── app.rs                  # AdwApplication, window, keybindings, theme loading
│   │   ├── workspace_view.rs       # LayoutNode → GtkPaned/Notebook, crea backend, sync ratios
│   │   ├── panel_host.rs           # Container con title bar + footer (user@host:dir) + focus/alert
│   │   ├── theme.rs                # CSS temi (9 schemi colore) + VTE color management
│   │   ├── panels/
│   │   │   ├── mod.rs              # PanelBackend trait
│   │   │   ├── terminal.rs         # VTE4 backend (Linux) + PTY fallback (macOS)
│   │   │   ├── markdown.rs         # TextView + parsing markdown
│   │   │   ├── chooser.rs          # Empty panel type selector
│   │   │   └── registry.rs         # Panel factory/registry system
│   │   ├── widgets/
│   │   │   ├── status_bar.rs       # Barra di stato applicazione
│   │   │   └── welcome.rs          # Welcome screen con recent workspaces
│   │   └── dialogs/
│   │       ├── panel_config.rs     # Dialog config pannello (CWD, script, min size)
│   │       └── settings.rs         # Dialog impostazioni workspace
│   └── pax-cli/src/main.rs          # Entry point CLI
├── config/
│   ├── default_workspace.json      # 3 terminali in split
│   ├── mixed_workspace.json        # Terminal + markdown + browser
│   └── tabs_workspace.json         # Split + tabs annidati
├── migrations/001_initial.sql
└── resources/
    └── style.css                   # GTK CSS theming
```

---

## Modelli dati

### PanelType

```rust
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelType {
    Terminal,                           // shell locale (VTE o fallback)
    Ssh { host, port, user, ... },      // terminale SSH
    RemoteTmux { host, session, ... },  // tmux remoto
    Markdown { file: String },          // viewer markdown
    Browser { url: String },            // WebView embed
}
```

### LayoutNode

Albero ricorsivo che definisce la disposizione dei pannelli:

- `Panel { id }` — foglia, referenzia un pannello
- `Hsplit { children, ratios }` — split orizzontale → `GtkPaned` horizontal
- `Vsplit { children, ratios }` — split verticale → `GtkPaned` vertical
- `Tabs { children, labels }` — schede → `GtkNotebook`

I layout sono annidabili arbitrariamente: tabs dentro split, split dentro tabs, ecc.

---

## Fasi di implementazione

### Fase 0: Setup GTK4 + primo terminale — COMPLETATA

| Task | Stato |
|------|-------|
| Setup gtk4-rs + vte4-rs + libadwaita | Done |
| AdwApplicationWindow + HeaderBar con controlli finestra | Done |
| Terminale VTE4 funzionante | Done |
| Backend fallback PTY+vt100 per macOS | Done |
| Feature flag `vte` per compilazione condizionale | Done |
| ScrolledWindow root per overflow | Done |
| Pannelli shrinkable con dimensione minima | Done |
| FileDialog per Open/Save (GTK 4.10+) | Done |
| Hamburger menu con Open/Save/SaveAs/Quit | Done |
| Dirty tracking con indicatore ● nel titolo | Done |

### Fase 1: Layout engine + multi-pannello — COMPLETATA

| Task | Stato |
|------|-------|
| PanelBackend trait | Done |
| PanelHost widget con title bar + focus styling | Done |
| LayoutNode → GtkPaned (hsplit/vsplit) + GtkNotebook (tabs) | Done |
| Focus management (Ctrl+N/P) | Done |
| Status bar | Done |
| Caricamento workspace JSON → layout GUI | Done |
| Workspace config con pannelli eterogenei | Done |
| ScrolledWindow root per pannelli shrinkable | Done |
| Dimensione minima pannelli | Done |

### Fase 2: Tipi pannello diversi — COMPLETATA

| Task | Stato |
|------|-------|
| MarkdownPanel con pulldown-cmark | Done |
| BrowserPanel placeholder (solo Linux) | Done |
| PanelType::Empty con chooser per selezionare tipo | Done |
| PanelRegistry plugin system per registrazione tipi pannello | Done |
| Menu pannello ⋮ (split, tab, chiudi, cambia tipo) | Done |
| Gestione pannelli mancanti con placeholder | Done |

### Fase 3: Broadcast + Safety (sett. 2-3)

**Obiettivo**: scrittura simultanea su gruppi di terminali con safety filter.

| Task | Dettagli |
|------|----------|
| Gruppi broadcast | Seleziona gruppo, input va a tutti i terminali del gruppo |
| Safety filter | Regex blocklist, conferma interattiva (GTK dialog) |
| Indicatore broadcast | Bordo colorato sui pannelli in broadcast |
| Barra input broadcast dedicata | Input in basso, visibile a tutti i pannelli |

**Verifica**: broadcast "echo test" → appare in tutti i terminali del gruppo.

### Fase 4: Alert + Recording (sett. 3-5)

**Obiettivo**: alert su output terminale, output salvato e ricercabile.

| Task | Dettagli |
|------|----------|
| Cattura output VTE | Callback su contenuto terminale per alert scan |
| Alert → bordo colorato + notifica desktop | notify-rust (Linux), osascript (macOS) |
| Toggle recording per pannello | Output → SQLite in batch |
| CLI `pax search` | FTS5 ricerca su comandi e output salvato |

**Verifica**: `echo ERROR` → bordo rosso, `pax search ERROR` lo trova.

### Fase 5: SSH + Tmux remoto — COMPLETATA

| Task | Stato |
|------|-------|
| SSH integrato in Terminal con SshConfig | Done |
| Autenticazione password (sshpass) + chiave | Done |
| Tmux remoto (`ssh -t host 'tmux new-session -A -s session'`) | Done |
| Startup script su remoto via heredoc | Done |
| SSH config nel dialog Terminal (host, port, user, password, identity, tmux) | Done |
| SSH host picker da ~/.ssh/config | Da fare (futuro) |
| Reconnection automatica | Da fare (futuro) |

### Fase 6: UX polish — COMPLETATA

| Task | Stato |
|------|-------|
| Split/tab/close dinamici dal menu ⋮ pannello | Done |
| Save/Open workspace con FileDialog | Done |
| Dirty indicator floppy nel header | Done |
| Sync ratios separatori → JSON al save | Done |
| Terminal: prompt minimale `$:` verde + footer `user@host:dir` colorato | Done |
| Terminal: directory tracking via OSC 7 + PROMPT_COMMAND | Done |
| Terminal: colori `ls` personalizzati | Done |
| Terminal: working directory, startup/close script con toggle | Done |
| SSH unificato in Terminal con SshConfig (host, port, user, password, tmux_session) | Done |
| SSH auto-login con sshpass, startup script via heredoc su remoto | Done |
| Panel config dialog: CWD, SSH, startup, close, min size | Done |
| Temi: 9 schemi colore, persistenza tra sessioni | Done |
| Welcome page: carica tema dall'ultimo workspace | Done |
| Recent workspaces dialog, Settings dialog | Done |
| Zoom pannello (Ctrl+Z) con rebuild layout | Done |
| Sync input tra pannelli (Ctrl+Shift+S) via VTE commit | Done |
| Bottoni sync/zoom nell'header pannello | Done |
| Double-click per rinominare pannelli e tab | Done |
| Icone tipo pannello nell'header e nei tab | Done |
| Title nascosta nei tab (ridondanza evitata) | Done |
| Model-first + rebuild per tutte le operazioni layout | Done |

### Fase 7: Layout builder + Packaging (sett. 8-10)

**Obiettivo**: editor visuale workspace + distribuzione.

| Task | Dettagli |
|------|----------|
| `pax edit` | GUI per creare/modificare workspace visivamente |
| Drag & drop pannelli nel builder | Crea layout trascinando |
| Form configurazione pannello | Tipo, nome, target, comandi, gruppi |
| Export/import JSON | Da builder a JSON e viceversa |
| **Linux**: Flatpak / .deb packaging | Distribuzione con tutte le deps |
| **macOS**: .app bundle / Homebrew formula | Distribuzione nativa macOS |
| .desktop file (Linux) + Info.plist (macOS) | Integrazione desktop |

**Verifica**: `pax edit` apre builder, salva JSON valido.

---

## Rischi e mitigazioni

| Rischio | Mitigazione |
|---------|-------------|
| GTK4 su macOS meno stabile che su Linux | Test CI su entrambe le piattaforme, fallback graceful |
| VTE4 non disponibile su macOS | Backend fallback PTY+vt100 già implementato |
| WebKitGTK solo Linux | Pannello browser opzionale, placeholder su macOS |
| SSH auth complessa (2FA, jump hosts) | Fallback a `ssh` binario di sistema |
| Packaging dipendenze GTK | Flatpak (Linux), Homebrew formula (macOS) |
| Performance con molti pannelli VTE | Lazy render per pannelli non visibili (tab) |

---

## Assessment architetturale (Marzo 2026)

### Architettura attuale

```
pax-gui/src/
├── app.rs              (936 LOC) — GTK app lifecycle, azioni, keybindings
├── workspace_view.rs   (980 LOC) — WorkspaceView: split/tab/close/zoom/sync/save
├── widget_builder.rs   (436 LOC) — Costruzione widget GTK (layout, tab labels, paned)
├── backend_factory.rs   (89 LOC) — Creazione backend da PanelConfig
├── panel_host.rs       (575 LOC) — Container pannello con title/footer/buttons
├── focus.rs             (92 LOC) — FocusManager
├── layout_ops.rs       (184 LOC) — Manipolazione albero LayoutNode
├── theme.rs            (251 LOC) — CSS temi + VTE colori
├── panels/
│   ├── terminal.rs     (488 LOC) — VTE4 + PTY fallback
│   ├── registry.rs     (212 LOC) — Panel factory system
│   ├── markdown.rs      — Viewer markdown
│   └── chooser.rs       — Type selector
├── dialogs/
│   ├── panel_config.rs (685 LOC) — Config terminal/markdown/browser + SSH
│   └── settings.rs     (211 LOC) — Workspace settings
└── widgets/
    ├── welcome.rs       — Welcome screen
    └── status_bar.rs    — Status bar
```

### Cosa funziona bene

- **Model-first layout** — tutte le operazioni (split/tab/close/zoom) aggiornano il modello poi rebuild il widget tree
- **Panel plugin system** — Registry + PanelBackend trait estensibile
- **Terminal UX** — prompt minimale, footer directory, colori personalizzati, SSH integrato
- **Sync input** — propagazione trasparente via VTE commit signal con anti-ricorsione
- **Temi** — 9 schemi colore per GTK + VTE, persistenza

### Debito tecnico rimanente

| Priorità | Problema | Azione |
|----------|---------|--------|
| Media | `app.rs` (936 LOC) — azioni GIO + callback nesting | Estrarre `actions.rs` |
| Media | Thread-local state (DIRTY_INDICATOR, THEME_PROVIDER) | Eventuale DI |
| Bassa | `panel_config.rs` (685 LOC) — terminal dialog è grande | Estrarre sezione SSH |

### Refactoring completati

1. ~~pax-tui rimosso~~ Done
2. ~~pax-pty rimosso~~ Done
3. ~~FocusManager estratto in focus.rs~~ Done
4. ~~Layout ops estratti in layout_ops.rs~~ Done
5. ~~Widget builder estratto in widget_builder.rs~~ Done
6. ~~Backend factory estratto in backend_factory.rs~~ Done
7. ~~Zero compiler warnings~~ Done
8. ~~SSH/RemoteTmux unificati in Terminal + SshConfig~~ Done

### Prossime feature (in ordine di valore utente)

1. **Estrarre actions da app.rs** — refactoring in corso
2. **Command palette (Ctrl+K)** — fuzzy search per azioni, pannelli, comandi
3. **Browser panel** — WebKitGTK (Linux), alternativa futura: wry per cross-platform
4. **Alert su output** — collegare pax-core/alert.rs a VTE output, bordo + notifica
5. **Drag & drop** — riordinare pannelli/tab trascinando
6. **Scorciatoie configurabili** — keybinding personalizzabili

---

## Funzionalità future (post-v1)

| Feature | Descrizione |
|---------|-------------|
| **RAM usage macOS** | Status bar mostra RSS via `/proc/self/status` (solo Linux). Su macOS serve `mach_task_basic_info` / `task_info()` con `#[cfg(target_os)]` |
| **Plugin system** | Nuovi tipi pannello via plugin (WASM o .so/dylib) |
| **Pannello log viewer** | Viewer specializzato per log strutturati |
| **Pannello monitor** | Grafici CPU/RAM/disco integrati |
| **Session restore** | Salva/ripristina stato completo workspace |
| **Snippets** | Libreria comandi frequenti |
| **Sync workspace** | Sync config via git |
| **Multi-window** | Più finestre per un workspace |
| **Multiplayer** | Sessione condivisa (read-only o full) |
| **Supporto Windows** | Via MSYS2/GTK4 o backend alternativo |
