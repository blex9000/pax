# MyTerms — Roadmap

## Visione

**MyTerms** è un workspace manager GUI in Rust, stile Tilix/Terminator, con pannelli di tipi eterogenei. Non è un terminale dentro un terminale: è una finestra nativa con pannelli separati che possono essere:

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

Il crate `tp-gui` usa feature flags per gestire le differenze tra piattaforme:

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
│                       myterms (GUI)                           │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │             tp-gui (GTK4 + libadwaita)                   │ │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────────┐ │ │
│  │  │ Terminal  │ │ Terminal │ │ Markdown │ │  Browser   │ │ │
│  │  │ (VTE4 o  │ │ SSH      │ │ Viewer   │ │ (WebKit)   │ │ │
│  │  │ fallback) │ │          │ │          │ │            │ │ │
│  │  └──────────┘ └──────────┘ └──────────┘ └────────────┘ │ │
│  │          ↕ PanelBackend trait (polimorfismo)             │ │
│  └──────────────────────────────────────────────────────────┘ │
│           ▼                    ▼              ▼               │
│   ┌────────────┐    ┌──────────────┐  ┌──────────┐          │
│   │  tp-core   │    │   tp-pty     │  │  tp-db   │          │
│   │ modelli    │    │ PTY locale   │  │ rusqlite │          │
│   │ config     │    │ SSH session  │  │ FTS5     │          │
│   │ alert      │    │ broadcast    │  │ history  │          │
│   │ safety     │    │ output buf   │  │          │          │
│   └────────────┘    └──────────────┘  └──────────┘          │
│                                                               │
│   tp-cli: myterms launch / list / search / init / edit       │
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
myterms/
├── Cargo.toml                      # workspace root
├── crates/
│   ├── tp-core/src/                # Modelli, config, alert, safety
│   │   ├── workspace.rs            # Workspace, PanelConfig, PanelType, LayoutNode
│   │   ├── config.rs               # Load/save/validate JSON
│   │   ├── ssh.rs                  # Parser ~/.ssh/config
│   │   ├── safety.rs               # Blocklist regex per gruppo
│   │   ├── alert.rs                # Regex pattern matching su output
│   │   └── template.rs             # Generatori workspace template
│   ├── tp-pty/src/                 # PTY + SSH
│   │   ├── manager.rs              # Spawn, resize, kill PTY
│   │   ├── multiplexer.rs          # Broadcast con safety check
│   │   ├── output.rs               # Ring buffer + alert scan
│   │   └── ssh.rs                  # (futuro) Sessioni SSH via russh
│   ├── tp-db/src/                  # SQLite embedded
│   │   ├── schema.rs               # Migrazioni SQL + FTS5
│   │   ├── commands.rs             # History comandi
│   │   ├── output.rs               # Output salvato
│   │   └── workspaces.rs           # Metadata workspace
│   ├── tp-gui/src/                 # GUI GTK4 (cross-platform)
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
│   └── tp-cli/src/main.rs          # Entry point CLI
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
| CLI `myterms search` | FTS5 ricerca su comandi e output salvato |

**Verifica**: `echo ERROR` → bordo rosso, `myterms search ERROR` lo trova.

### Fase 5: SSH nativo + Tmux remoto (sett. 5-7)

**Obiettivo**: pannelli SSH e tmux remoti funzionanti.

| Task | Dettagli |
|------|----------|
| SshTerminalPanel | russh → VTE/fallback (pipe PTY remoto in widget locale) |
| Autenticazione | Password, chiave, ssh-agent |
| RemoteTmuxPanel | Crea/aggancia sessione tmux via SSH |
| SSH host picker | Dialog fuzzy da ~/.ssh/config |
| Reconnection | Retry automatico su disconnect |

**Verifica**: pannello SSH si connette, pannello tmux aggancia sessione.

### Fase 6: Command palette + UX polish (sett. 7-8) — IN CORSO

**Obiettivo**: UX completa e rifinita.

| Task | Stato |
|------|-------|
| Split/tab/close dinamici dal menu ⋮ pannello | Done |
| Save/Open workspace con FileDialog | Done |
| Dirty indicator floppy nel header | Done |
| ScrolledWindow per overflow pannelli | Done |
| Sync ratios separatori → JSON al save | Done |
| Terminal: prompt minimale `$:` verde + footer `user@host:dir` colorato | Done |
| Terminal: directory tracking via OSC 7 + PROMPT_COMMAND | Done |
| Terminal: colori `ls` personalizzati (#5588ff per directory) | Done |
| Terminal: working directory configurabile | Done |
| Terminal: startup/close script con toggle enable/disable | Done |
| Panel config dialog: CWD, startup, close, min size | Done |
| Temi: 9 temi colore (System, Catppuccin, Solarized, Nord, Dracula, Gruvbox, Tokyo Night) | Done |
| Welcome page: carica tema dall'ultimo workspace | Done |
| Recent workspaces dialog | Done |
| Settings dialog (nome, tema, shell, scrollback) | Done |
| Script startup unici per pannello (counter atomico) | Done |
| Command palette (Ctrl+K) | Da fare |
| Zoom pannello (Ctrl+Z) | Da fare |
| Drag & drop split | Da fare |
| Scorciatoie tastiera configurabili | Da fare |

**Verifica**: palette funziona, zoom funziona, drag & drop crea split.

### Fase 7: Layout builder + Packaging (sett. 8-10)

**Obiettivo**: editor visuale workspace + distribuzione.

| Task | Dettagli |
|------|----------|
| `myterms edit` | GUI per creare/modificare workspace visivamente |
| Drag & drop pannelli nel builder | Crea layout trascinando |
| Form configurazione pannello | Tipo, nome, target, comandi, gruppi |
| Export/import JSON | Da builder a JSON e viceversa |
| **Linux**: Flatpak / .deb packaging | Distribuzione con tutte le deps |
| **macOS**: .app bundle / Homebrew formula | Distribuzione nativa macOS |
| .desktop file (Linux) + Info.plist (macOS) | Integrazione desktop |

**Verifica**: `myterms edit` apre builder, salva JSON valido.

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

### Cosa funziona bene

- **Panel plugin system** — Registry + PanelBackend trait è solido e estensibile
- **Layout engine** — Paned/Notebook recursivo con ratios sync funziona bene
- **Terminal UX** — Prompt minimale, footer con directory, colori personalizzati
- **Temi** — 9 temi che funzionano sia per GTK che per VTE, persistenza tra sessioni

### Debito tecnico da risolvere

| Priorità | Problema | Azione |
|----------|---------|--------|
| Alta | `workspace_view.rs` è 1.674 LOC — split/tab/close/focus/model tutto insieme | Estrarre FocusManager e LayoutOps in moduli separati |
| Alta | `tp-pty` è codice morto — GUI usa VTE direttamente | Rimuovere il crate o integrarlo |
| Alta | `tp-tui` è abbandonato | Rimuovere dal workspace |
| Media | Callback hell in `app.rs` — 4+ livelli di Rc<RefCell<>> nested | Refactoring graduale |
| Media | Features dichiarate ma non implementate (Browser, SSH nativo, alerts, broadcast) | Implementare o rimuovere dal registry |
| Media | Dead code: unused imports, unused functions | Cleanup |
| Bassa | Thread-local state (DIRTY_INDICATOR, THEME_PROVIDER) | Eventuale dependency injection |

### Piano refactoring (da eseguire ora)

1. Rimuovere `tp-tui` dal workspace (crate abbandonato)
2. Estrarre `FocusManager` da `workspace_view.rs` in `focus.rs`
3. Estrarre layout operations (split/tab/close/model updates) in `layout_ops.rs`
4. Cleanup dead code (unused imports, unused functions in tutti i crate)
5. Rimuovere `tp-pty` (non usato, GUI usa VTE direttamente) oppure marcarlo come futuro

### Prossime feature (in ordine di valore utente)

1. **Command palette (Ctrl+K)** — fuzzy search per azioni, pannelli, comandi recenti
2. **Zoom pannello (Ctrl+Z)** — fullscreen singolo pannello
3. **Browser panel reale** — WebKitGTK per dashboard/Grafana
4. **Alert su output** — collegare tp-core/alert.rs a VTE output
5. **Broadcast groups** — UI per attivare/disattivare broadcast su gruppi

---

## Funzionalità future (post-v1)

| Feature | Descrizione |
|---------|-------------|
| **Plugin system** | Nuovi tipi pannello via plugin (WASM o .so/dylib) |
| **Pannello log viewer** | Viewer specializzato per log strutturati |
| **Pannello monitor** | Grafici CPU/RAM/disco integrati |
| **Session restore** | Salva/ripristina stato completo workspace |
| **Snippets** | Libreria comandi frequenti |
| **Sync workspace** | Sync config via git |
| **Multi-window** | Più finestre per un workspace |
| **Multiplayer** | Sessione condivisa (read-only o full) |
| **Supporto Windows** | Via MSYS2/GTK4 o backend alternativo |
