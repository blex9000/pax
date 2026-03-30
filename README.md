MyTerms — Workspace Manager con Pannelli Eterogenei

Workspace manager GUI cross-platform in Rust (GTK4 + VTE), stile Tilix, con pannelli di tipi diversi: terminale, SSH, tmux remoto, markdown viewer, browser embed.

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
  • **Temi**: System, Catppuccin Mocha/Latte, Solarized Dark/Light, Nord, Dracula, Gruvbox, Tokyo Night — tema persistito tra sessioni
  • **Welcome page**: carica il tema dall'ultimo workspace usato
  • **Recent workspaces**: dialog con lista workspace recenti da DB SQLite

Installazione

Linux (Ubuntu/Debian)

─── bash ───
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

# Dipendenze GTK4
sudo apt install libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev

# Opzionale (pannello browser)
sudo apt install libwebkitgtk-6.0-dev

# Debug: GTK Inspector
gsettings set org.gtk.Settings.Debug enable-inspector-keybinding true
# Poi Ctrl+Shift+D nella finestra per aprire l'inspector

# Build
cargo build --release
───────

macOS (Homebrew)

─── bash ───
# Dipendenze
brew install gtk4 libadwaita pkg-config

# Build (senza VTE — usa backend fallback PTY+vt100)
cargo build --release --no-default-features
───────

Uso rapido

─── bash ───
# Genera workspace di esempio
myterms init workspace.json

# Lancia workspace
myterms launch workspace.json

# Lista workspace recenti
myterms list

# Cerca in history e output
myterms search "ERROR"
───────

Tipi di pannello

| Tipo | Descrizione | Linux | macOS |
|------|-------------|-------|-------|
| terminal | Shell locale (VTE4 nativo) | VTE4 completo | PTY + vt100 fallback |
| ssh | Terminale connesso via SSH | Sì | Sì |
| remote_tmux | Sessione tmux remota | Sì | Sì |
| markdown | Viewer per note .md | Sì | Sì |
| browser | WebView per dashboard, docs | WebKitGTK | Non disponibile |

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
| Menu ⋮ | Configure, Split H/V, Add Tab, Close |
| Hamburger ☰ | New, Open, Recent, Save, Save As, Settings, Quit |

Architettura

myterms/
├── crates/
│   ├── tp-core/    # Modelli, config JSON, alert, safety, SSH parser
│   ├── tp-pty/     # PTY locale + SSH sessions + broadcast
│   ├── tp-db/      # SQLite + FTS5 (history, output, workspaces)
│   ├── tp-gui/     # GTK4 + VTE/fallback (UI principale, cross-platform)
│   └── tp-cli/     # Entry point CLI (clap)
├── config/
│   ├── default_workspace.json   # 3 terminali in split
│   ├── mixed_workspace.json     # Terminal + markdown + browser
│   └── tabs_workspace.json      # Split + tabs annidati
───────

Vedi ROADMAP.md per architettura dettagliata e piano di implementazione.

Log

I log vengono scritti in ~/.local/share/myterms/myterms.log. Per monitorarli in tempo reale:

─── bash ───
tail -f ~/.local/share/myterms/myterms.log
───────

Il livello di log è configurabile via variabile d'ambiente:

─── bash ───
RUST_LOG=tp_gui=debug myterms
───────

Dati persistenti

| File | Contenuto |
|------|-----------|
| ~/.local/share/myterms/myterms.db | Database SQLite — workspace recenti, history comandi, output salvato |
| ~/.local/share/myterms/myterms.log | Log applicazione |

Licenza

MIT
