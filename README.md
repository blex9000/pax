# MyTerms — Workspace Manager con Pannelli Eterogenei

Workspace manager GUI cross-platform in Rust (GTK4 + VTE), stile Tilix, con pannelli di tipi diversi: terminale, SSH, tmux remoto, markdown viewer, browser embed.

**Piattaforme**: Linux (primario), macOS (supportato).

## Stato

**In sviluppo** — Fase 0, 1 e 2 completate. Layout multi-pannello con split, tab e tipi eterogenei funzionanti. PanelRegistry plugin system per registrazione tipi pannello, PanelType::Empty con chooser, menu ⋮ per split/tab/close dinamici. Save/Open workspace con FileDialog, dirty tracking con indicatore ●.

## Installazione

### Linux (Ubuntu/Debian)

```bash
# Dipendenze
sudo apt install libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev

# Opzionale (pannello browser)
sudo apt install libwebkitgtk-6.0-dev

# Build
cargo build --release
```

### macOS (Homebrew)

```bash
# Dipendenze
brew install gtk4 libadwaita pkg-config

# Build (senza VTE — usa backend fallback PTY+vt100)
cargo build --release --no-default-features
```

## Uso rapido

```bash
# Genera workspace di esempio
myterms init workspace.json

# Lancia workspace
myterms launch workspace.json

# Lista workspace recenti
myterms list

# Cerca in history e output
myterms search "ERROR"
```

## Tipi di pannello

| Tipo | Descrizione | Linux | macOS |
|------|-------------|-------|-------|
| `terminal` | Shell locale (VTE4 nativo) | VTE4 completo | PTY + vt100 fallback |
| `ssh` | Terminale connesso via SSH | Sì | Sì |
| `remote_tmux` | Sessione tmux remota | Sì | Sì |
| `markdown` | Viewer per note .md | Sì | Sì |
| `browser` | WebView per dashboard, docs | WebKitGTK | Non disponibile |

## Tipi di layout

| Tipo | Descrizione |
|------|-------------|
| `hsplit` | Split orizzontale — pannelli affiancati da sinistra a destra |
| `vsplit` | Split verticale — pannelli impilati dall'alto in basso |
| `tabs` | Schede — un pannello visibile alla volta con tab bar |

I layout sono annidabili: tabs dentro split, split dentro tabs, ecc.

## Esempio workspace JSON

```json
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
```

## Scorciatoie

| Tasto | Azione |
|-------|--------|
| `Ctrl+Q` | Esci |
| `Ctrl+N/P` | Focus pannello successivo/precedente |
| `Ctrl+O` | Apri workspace da file |
| `Ctrl+S` | Salva workspace |
| `Ctrl+Z` | Zoom pannello a schermo intero |
| `Ctrl+B` | Cicla gruppi broadcast |
| `Ctrl+T` | Cicla tab |
| `Ctrl+K` | Command palette |
| Menu ⋮ | Split H/V, nuovo tab, chiudi pannello, cambia tipo |
| Hamburger ☰ | Open, Save, Save As, Quit |

## Architettura

```
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
```

Vedi [ROADMAP.md](ROADMAP.md) per architettura dettagliata e piano di implementazione.

## Licenza

MIT
