Pax — Workspace Manager con Pannelli Eterogenei

Workspace manager GUI cross-platform in Rust (GTK4 + VTE), stile Tilix, con pannelli di tipi diversi: terminale, SSH, tmux remoto, markdown viewer ed editor codice.

Piattaforme: Linux (primario), macOS (supportato).

Stato

In sviluppo — Fase 0, 1, 2 completate + gran parte di Fase 6 (UX polish).

Funzionalità principali:
  • Layout multi-pannello con split, tab e tipi eterogenei
  • PanelRegistry plugin system per registrazione tipi pannello
  • PanelType::Empty con chooser, menu ⋮ per split/tab/close dinamici
  • Save/Open workspace con FileDialog, dirty tracking con indicatore floppy
  • **Sync ratios**: le posizioni dei separatori vengono salvate nel JSON
  • **Terminal migliorato**: prompt minimale (`$:` verde), footer con `user@host:directory` colorato, directory tracking via OSC 7, colori `ls` personalizzati
  • **Panel config**: working directory, startup script (con toggle), before_close script (con toggle), min width/height
  • **Temi**: Graphite, Dracula, Aurora, Quantum — tema persistito tra sessioni
  • **Welcome page**: carica il tema dall'ultimo workspace usato
  • **Recent workspaces**: dialog con lista workspace recenti da DB SQLite

Download

Linux (AppImage)

L'AppImage è il modo più rapido per provare Pax su Linux senza installare dipendenze:

1. Scarica `Pax-x86_64.AppImage` dall'ultima [GitHub Release](https://github.com/blex9000/pax/releases/latest) o dalla pagina [Actions](https://github.com/blex9000/pax/actions) (sezione Artifacts nel Summary della build)
2. Rendi eseguibile e avvia:

─── bash ───
chmod +x Pax-x86_64.AppImage
./Pax-x86_64.AppImage
───────

macOS (DMG)

1. Scarica `Pax-macos-arm64.dmg` dalla pagina [Actions](https://github.com/blex9000/pax/actions) (sezione Artifacts nel Summary della build)
2. Apri il DMG e trascina Pax nella cartella Applications
3. Al primo avvio, se macOS blocca l'app (non firmata):

─── bash ───
xattr -cr /Applications/Pax.app
───────

Il DMG include tutte le dipendenze (GTK4, libadwaita, GtkSourceView5) — non serve installare nulla.

Nota: la build macOS non include VTE (terminale nativo Linux). Il terminale usa un fallback PTY con funzionalità ridotte.

L'AppImage include tutte le dipendenze GTK4/libadwaita/VTE4. Funziona su qualsiasi distro Linux x86_64 recente.

Installazione da sorgente

Linux (Ubuntu/Debian)

─── bash ───
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

# Dipendenze GTK4 + SourceView
sudo apt install libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev libgtksourceview-5-dev

# Debug: GTK Inspector
gsettings set org.gtk.Settings.Debug enable-inspector-keybinding true
# Poi Ctrl+Shift+D nella finestra per aprire l'inspector
───────

macOS (Homebrew)

─── bash ───
brew install gtk4 libadwaita gtksourceview5 pkg-config
───────

Build

─── bash ───
# Linux — build completa (VTE4 + SourceView — default)
cargo build

# Linux — release ottimizzata
cargo build --release

# macOS — senza VTE (usa PTY+vt100 fallback), con SourceView
cargo build --no-default-features --features sourceview

# macOS — minimale senza VTE ne SourceView
cargo build --no-default-features

# Solo per development/debug veloce
cargo run
───────

Le feature flag disponibili:

| Feature | Default | Descrizione |
|---------|---------|-------------|
| `vte` | Si | Terminale VTE4 nativo (Linux). Su macOS usa fallback PTY+vt100 |
| `sourceview` | Si | GtkSourceView 5 per syntax highlighting nel code editor e markdown. Senza, il code editor mostra un placeholder |

`cargo build` include entrambe. Su macOS usa `--no-default-features --features sourceview` per disabilitare solo VTE.

Uso rapido

─── bash ───
# Genera workspace di esempio
pax init workspace.json

# Lancia workspace
pax launch workspace.json

# Lista workspace recenti
pax list

# Cerca in history e output
pax search "ERROR"
───────

Tipi di pannello

| Tipo | Descrizione | Linux | macOS |
|------|-------------|-------|-------|
| terminal | Shell locale (VTE4 nativo) | VTE4 completo | PTY + vt100 fallback |
| ssh | Terminale connesso via SSH | Sì | Sì |
| remote_tmux | Sessione tmux remota | Sì | Sì |
| markdown | Viewer/editor per note .md | Sì | Sì |
| code_editor | Editor codice con file tree, git, search/replace | Sì (richiede sourceview) | Sì (richiede sourceview) |
| code_editor (SSH) | Editor remoto via SSH — stesse feature del code_editor locale | Sì | Sì |

Tipi di layout

| Tipo | Descrizione |
|------|-------------|
| hsplit | Split orizzontale — pannelli affiancati da sinistra a destra |
| vsplit | Split verticale — pannelli impilati dall'alto in basso |
| tabs | Schede — un pannello visibile alla volta con tab bar |

I layout sono annidabili: tabs dentro split, split dentro tabs, ecc.

Esempio workspace JSON

─── json ───
{
    "name": "dev-workspace",
    "layout": {
        "type": "hsplit",
        "children": [
            { "type": "panel", "id": "main" },
            {
                "type": "tabs",
                "children": [
                    { "type": "panel", "id": "build" },
                    { "type": "panel", "id": "notes" }
                ],
                "labels": ["Build", "Notes"]
            }
        ],
        "ratios": [0.6, 0.4]
    },
    "panels": [
        {
            "id": "main",
            "name": "Shell",
            "panel_type": { "type": "terminal" },
            "groups": ["dev"]
        },
        {
            "id": "build",
            "name": "Build",
            "panel_type": { "type": "terminal" },
            "startup_commands": ["cargo watch -x check"],
            "record_output": true
        },
        {
            "id": "notes",
            "name": "Notes",
            "panel_type": { "type": "markdown", "file": "NOTES.md" }
        }
    ],
    "groups": [
        {
            "name": "dev",
            "color": "green",
            "blocked_patterns": ["^rm\\s+-rf\\s+/"]
        }
    ],
    "alerts": [
        {
            "pattern": "(?i)error|panic|fatal",
            "scope": "all",
            "actions": [{ "border_color": "red" }, "desktop_notification"]
        }
    ]
}
───────

Editor remoto (SSH)

Il code editor può lavorare su progetti remoti via SSH — nessun mount filesystem (SSHFS) necessario. Tutte le operazioni (file tree, lettura/scrittura file, git, search) vengono eseguite direttamente sul server tramite comandi SSH.

─── json ───
{
    "id": "remote-editor",
    "name": "Server",
    "panel_type": {
        "type": "code_editor",
        "root_dir": "/home/user/project",
        "ssh": {
            "host": "server.example.com",
            "port": 22,
            "user": "user"
        },
        "remote_path": "/home/user/project"
    }
}
───────

Configurazione SSH disponibile anche dalla UI: menu ⋮ → Configure → sezione "Remote (SSHFS)" con toggle on/off.

Come funziona:
  • Usa SSH ControlMaster per una connessione persistente (un solo handshake)
  • Ogni operazione file è un singolo comando SSH (cat, ls, git, etc.)
  • Supporta autenticazione con password (via sshpass) o chiave SSH
  • Browse remoto delle cartelle direttamente dal dialog di configurazione
  • Intervalli watcher adattivi: 15s git status, 30s file tree (vs 3s/2s in locale)

Prerequisiti: `ssh` (sempre disponibile), `sshpass` (solo se si usa password)

Architettura FileBackend:
  • `LocalFileBackend` — operazioni su filesystem locale (std::fs + git)
  • `SshFileBackend` — comandi SSH diretti con ControlMaster
  • Il trait è progettato per supportare un futuro `AgentFileBackend` (binario remoto per batch operations)

Scorciatoie

| Tasto | Azione |
|-------|--------|
| Ctrl+Q | Esci |
| Ctrl+N/P | Focus pannello successivo/precedente |
| Ctrl+O | Apri workspace da file |
| Ctrl+S | Salva workspace |
| Ctrl+Shift+H | Split orizzontale (nuovo pannello sotto) |
| Ctrl+Shift+J | Split verticale (nuovo pannello a destra) |
| Ctrl+Shift+T | Nuovo tab |
| Ctrl+Shift+W | Chiudi pannello |
| Ctrl+Shift+Z | Zoom/unzoom pannello, oppure Redo nei pannelli editor |
| Ctrl+Shift+S | Toggle sync input tra terminali |
| Ctrl+R | Reverse search (terminale bash) |
| Ctrl+Z | Undo nei pannelli editor |
| Ctrl+Y | Redo nei pannelli editor |
| Ctrl+C / Ctrl+X / Ctrl+V | Copia / Taglia / Incolla nei pannelli editor |
| Ctrl+Shift+C / Ctrl+Shift+V | Copia / Incolla nel terminale |
| Menu ⋮ | Configure, Split H/V, Add Tab, Close |
| Hamburger ☰ | New, Open, Recent, Save, Save As, Settings, Quit |

Architettura

pax/
├── crates/
│   ├── pax-core/    # Modelli, config JSON, alert, safety, SSH parser
│   ├── pax-db/      # SQLite + FTS5 (history, output, workspaces)
│   ├── pax-gui/     # GTK4 + VTE/fallback (UI principale, cross-platform)
│   │   └── panels/
│   │       ├── terminal/
│   │       │   ├── vte_backend.rs    # Linux VTE4
│   │       │   └── pty_backend.rs    # macOS PTY fallback
│   │       └── editor/
│   │           ├── file_backend.rs   # FileBackend trait (Local/SSH/Agent)
│   │           ├── file_tree.rs      # File tree sidebar
│   │           ├── editor_tabs.rs    # Tab bar + code editing
│   │           ├── git_status.rs     # Git changes view
│   │           ├── git_log.rs        # Git history view
│   │           └── project_search.rs # Project-wide search
│   └── pax-cli/     # Entry point CLI (clap)
├── config/           # Workspace JSON di esempio
├── resources/        # Icone, CSS, style scheme sourceview
───────

Vedi ROADMAP.md per architettura dettagliata e piano di implementazione.

Log

I log vengono scritti in ~/.local/share/pax/pax.log. Per monitorarli in tempo reale:

─── bash ───
tail -f ~/.local/share/pax/pax.log
───────

Il livello di log è configurabile via variabile d'ambiente:

─── bash ───
RUST_LOG=pax_gui=debug pax
───────

Dati persistenti

| File | Contenuto |
|------|-----------|
| ~/.local/share/pax/pax.db | Database SQLite — workspace recenti, history comandi, output salvato |
| ~/.local/share/pax/pax.log | Log applicazione |

Release e Packaging

AppImage (build locale)

─── bash ───
# Requisiti: cargo + dipendenze GTK4 (vedi Installazione da sorgente)
./scripts/build-appimage.sh
# Output: Pax-x86_64.AppImage
───────

Lo script scarica automaticamente `linuxdeploy` e il plugin GTK4 nella cartella `build-tools/` (cachati per build successive).

GitHub Actions (CI/CD)

Il workflow `.github/workflows/release.yml` builda automaticamente l'AppImage e lo pubblica come GitHub Release:

- **Trigger automatico**: push di un tag `v*` (es. `v0.1.0`)
- **Trigger manuale**: workflow_dispatch dalla pagina Actions

─── bash ───
# Creare una release
git tag v0.1.0
git push origin v0.1.0
# → GitHub Actions builda l'AppImage e crea la release
───────

Markdown Notebook

I fenced code blocks marcati con tag eseguibile (`run`, `watch=5s`, …) sono
eseguiti inline dal pannello Markdown. Vedi [`docs/notebook.md`](docs/notebook.md)
per la sintassi completa.

Licenza

MIT
