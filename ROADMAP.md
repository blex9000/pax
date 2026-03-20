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
│   │   ├── app.rs                  # AdwApplication, window, keybindings, CSS
│   │   ├── workspace_view.rs       # LayoutNode → GtkPaned/Notebook, crea backend
│   │   ├── panel_host.rs           # Container con title bar + focus/alert styling
│   │   ├── panels/
│   │   │   ├── mod.rs              # PanelBackend trait
│   │   │   ├── terminal.rs         # VTE4 backend (Linux) + PTY fallback (macOS)
│   │   │   └── markdown.rs         # TextView + parsing markdown
│   │   ├── widgets/
│   │   │   └── status_bar.rs       # Barra di stato
│   │   └── dialogs/
│   │       └── mod.rs              # (futuro) Broadcast picker, SSH picker
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

### Fase 6: Command palette + UX polish (sett. 7-8) — PARZIALMENTE COMPLETATA

**Obiettivo**: UX completa e rifinita.

| Task | Stato |
|------|-------|
| Split/tab/close dinamici dal menu ⋮ pannello | Done |
| Save/Open workspace con FileDialog | Done |
| Dirty indicator ● nel titolo finestra | Done |
| ScrolledWindow per overflow pannelli | Done |
| Command palette (Ctrl+K) | Da fare |
| Zoom pannello (Ctrl+Z) | Da fare |
| Drag & drop split | Da fare |
| Scorciatoie tastiera configurabili | Da fare |
| GTK CSS theming avanzato | Da fare |
| Pre/post script | Da fare |

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
