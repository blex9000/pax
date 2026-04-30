# Terminal command history popup — design

Status: **draft, awaiting review**
Date: 2026-04-30
Scope: `pax-core`, `pax-db`, `pax-gui`

## 1. Obiettivo

Aggiungere ad ogni **panel terminale** una cronologia persistente dei comandi
eseguiti, accessibile da un pulsante nell'header che apre un popup
scorrevole. Cliccando una riga il comando viene **incollato** nel terminale
corrente (senza Enter) per permettere edit prima dell'esecuzione.

## 2. Vincoli e decisioni di scope

| Decisione | Valore | Motivazione |
|---|---|---|
| Scope cronologia | per-pannello, persistente tra riavvii | richiesta utente |
| Chiave persistenza | UUID generico per `PanelConfig`, persistito nel JSON | gli `id` umani (`p1`, `p2`) sono riusabili dopo edit manuale del JSON; `record_key` cambia su rinomina workspace; un UUID al pannello evita collisioni e orfanizzazioni |
| Click su riga | incolla in terminale, no auto-Enter | richiesta utente |
| Cattura del testo del comando | shell-bootstrap shell-aware (bash + zsh) | macOS è zsh-default, Linux bash-default — coprire entrambi |
| Trigger | OSC 133;C (già scattato dai due backend) | non serve intercettare il PTY o un OSC custom: VTE consuma le sequenze ignote |
| Trasporto del comando | file sidechannel `$PAX_CMD_FILE` per pannello | semplice, atomico (`> file`), backend-agnostico |
| Contenuto riga | `[HH:MM] <comando>`, **DISTINCT** per `command`, ordinato `MAX(executed_at) DESC` | richiesta utente: 10 esecuzioni dello stesso comando = 1 riga |
| Tooltip riga | timestamp completo (`YYYY-MM-DD HH:MM:SS`) | data piena su hover, layout compatto |
| Persistenza JSON | il campo `uuid` viene aggiunto alla `PanelConfig` ed è generico (non `history_id`) | riutilizzabile per future feature panel-scoped |

## 3. Architettura

```
┌──────────────────────────────────────────────────────────────────┐
│                          TerminalPanel                            │
│  ┌────────────────────┐   ┌──────────────────────────────────┐   │
│  │ shell process      │   │ pax (rust)                        │   │
│  │                    │   │                                   │   │
│  │ __pax_preexec:     │──▶│ scan_osc_markers → 133;C event    │   │
│  │  echo $cmd > FILE  │   │   ↓                               │   │
│  │  printf 133;C      │   │ read $PAX_CMD_FILE                │   │
│  └────────────────────┘   │   ↓                               │   │
│         shell hook         │ Database::insert_command(uuid,..) │   │
│         (bash/zsh)         └──────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────┘
                                          │
                                          ▼
                       ┌──────────────────────────────────┐
                       │ Header button (term-only)         │
                       │   click → Popover                 │
                       │     SELECT command, MAX(at)        │
                       │     GROUP BY command DESC          │
                       │     row click → write_input(cmd)   │
                       └──────────────────────────────────┘
```

### 3.1 Crate boundaries

| Crate | Responsabilità nuova |
|---|---|
| `tp-core` | aggiungere `uuid: Uuid` a `PanelConfig` con `serde(default = "Uuid::new_v4")` |
| `tp-db`   | aggiungere query `latest_distinct_commands(panel_uuid, limit)` |
| `tp-gui`  | shell bootstrap shell-aware; cattura del comando da preexec; pulsante header term-only; popup scrollabile; cleanup file su shutdown |

## 4. Componenti

### 4.1 Schema dati

**Nessuna migration richiesta.** La tabella `command_history` esiste già con
la forma giusta:

```sql
CREATE TABLE command_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_name TEXT,
    panel_id TEXT,                    -- ⬅ scriviamo qui l'UUID del pannello
    command TEXT NOT NULL,
    executed_at TEXT DEFAULT (datetime('now')),
    exit_code INTEGER
);
```

La colonna `panel_id` cambia semantica: oggi non viene popolata (`insert_command`
non è mai chiamata), da domani contiene l'UUID stringa del pannello.
`workspace_name` resta opzionale e popolato per debug/grep ma non usato come
chiave.

### 4.2 Modello (`tp-core/src/workspace.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelConfig {
    pub id: String,
    #[serde(default = "Uuid::new_v4")]
    pub uuid: Uuid,                    // 🆕
    #[serde(default)]
    pub name: String,
    // … resto invariato …
}
```

Su workspace già esistenti senza il campo, serde popola un nuovo UUID alla
deserializzazione; al primo salvataggio il campo si materializza nel JSON.
Tutti i call-site che costruiscono `PanelConfig` letteralmente (es. test,
template, `workspace_view.rs` `split_focused`) devono inizializzare
`uuid: Uuid::new_v4()`.

### 4.3 Shell bootstrap (`tp-gui/src/panels/terminal/shell_bootstrap.rs`)

Lo script attuale è bash-only. Refactor:

```rust
pub(super) fn build_bootstrap(panel_uuid: &Uuid, cmd_file: &Path) -> String {
    let kind = detect_shell_kind();              // SHELL env → bash | zsh | other
    match kind {
        ShellKind::Bash => bash_bootstrap(cmd_file),
        ShellKind::Zsh  => zsh_bootstrap(cmd_file),
        ShellKind::Other => String::new(),       // fallback silenzioso
    }
}
```

**bash** (path attuale, esteso):
```bash
set +o history
export PAX_CMD_FILE='/tmp/pax-cmd-<uuid>.txt'
export PS1='\[\033[32m\]$:\[\033[0m\] '
__pax_prompt() { … invariato … }
__pax_preexec() {
    [[ -n "$__pax_preexec_fired" ]] && return
    __pax_preexec_fired=1
    printf '%s' "$BASH_COMMAND" > "$PAX_CMD_FILE"
    printf '\033]133;C\007'
}
PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }__pax_prompt"
trap '__pax_preexec' DEBUG
set -o history
```

**zsh** (nuovo):
```zsh
export PAX_CMD_FILE='/tmp/pax-cmd-<uuid>.txt'
__pax_prompt_zsh() {
    local d="${PWD/#$HOME/~}"
    print -n -- $'\e]0;'"$USER@$HOST: $d"$'\a'
    print -n -- $'\e]7;file://'"$HOST$PWD"$'\e\\'
    print -n -- $'\e]133;A\a'
}
__pax_preexec_zsh() {
    print -nr -- "$1" > "$PAX_CMD_FILE"
    print -n  -- $'\e]133;C\a'
}
typeset -ga precmd_functions preexec_functions
precmd_functions+=(__pax_prompt_zsh)
preexec_functions+=(__pax_preexec_zsh)
```

`PAX_CMD_FILE` è un path stabile per la vita del pannello, derivato dall'UUID
del pannello: `<XDG_RUNTIME_DIR or /tmp>/pax-cmd-<uuid>.txt`.

### 4.4 Cattura del comando

L'esistente trigger `OSC 133;C` continua a guidare l'indicatore di stato.
Aggiungiamo un secondo consumer:

**VTE backend** (`vte_backend.rs`) — già `connect_shell_preexec` esiste.
Estendiamo la closure:
```rust
vte.connect_shell_preexec(clone!(@strong panel_uuid_str, @strong cmd_file => move |_| {
    if let Ok(cmd) = std::fs::read_to_string(&cmd_file) {
        let cmd = cmd.trim_end_matches('\n');
        if !cmd.is_empty() {
            if let Ok(db) = pax_db::Database::open(&pax_db::Database::default_path()) {
                let _ = db.insert_command(
                    workspace_name.as_deref(),
                    Some(&panel_uuid_str),
                    cmd,
                    None,
                );
            }
        }
    }
    if let Some(cb) = status_cb_ref.try_borrow().ok().and_then(|b| b.clone()) {
        cb(true);
    }
}));
```

Pattern allineato a `app.rs` e `actions.rs`: `Database::open(&Database::default_path())`
apre un handle fresco per la singola operazione (rusqlite è cheap da
aprire, niente connection pool).

**PTY backend** (`pty_backend.rs`) — il branch `b'C'` di `scan_osc_markers`
già emette `TerminalUiEvent::StatusChanged(true)`. Aggiungiamo un nuovo
evento `TerminalUiEvent::CommandStarted` ricevuto dalla GLib main loop e
gestito esattamente come la VTE-side: `read_to_string($PAX_CMD_FILE)` →
`insert_command(...)`.

In entrambi i backend la lettura è best-effort: se il file manca o non è
parseable (UTF-8 non valido), la cronologia di quel comando viene saltata
silenziosamente.

### 4.5 Database query (`tp-db/src/commands.rs`)

```rust
impl Database {
    /// Last unique command per text, scoped to a panel UUID.
    pub fn latest_distinct_commands(
        &self,
        panel_uuid: &str,
        limit: usize,
    ) -> Result<Vec<CommandRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT MIN(id), workspace_name, panel_id, command,
                    MAX(executed_at) AS last_run, exit_code
             FROM command_history
             WHERE panel_id = ?1
             GROUP BY command
             ORDER BY last_run DESC
             LIMIT ?2",
        )?;
        // … standard map …
    }
}
```

### 4.6 UI: pulsante header

In `panel_host.rs`, dentro la costruzione del title bar, aggiunge un
`gtk::Button` simile a `sync_button`/`zoom_button`:

```rust
let history_button = gtk4::Button::new();
history_button.set_icon_name("document-open-recent-symbolic");
history_button.add_css_class("flat");
history_button.add_css_class("panel-action-btn");
history_button.set_tooltip_text(Some("Cronologia comandi"));
history_button.set_visible(false);   // mostrato solo quando il backend è terminale
```

`PanelHost::set_backend(...)` mostra il pulsante quando
`backend.panel_type() == "terminal"`, lo nasconde altrimenti.

Il pulsante è inserito in `end_box` prima di `menu_button`, allineato con
il pattern Sync/Zoom esistente.

### 4.7 UI: popup

Click → costruzione lazy del `gtk::Popover` (rebuild ad ogni apertura per
catturare comandi nuovi):

```rust
fn build_history_popover(panel_uuid: &str, terminal_input: PanelInputCallback) -> gtk4::Popover {
    let popover = gtk4::Popover::new();
    crate::theme::configure_popover(&popover);    // ← classe app-popover

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_min_content_height(280);
    scroll.set_max_content_height(420);
    scroll.set_min_content_width(420);

    let list = gtk4::ListBox::new();
    list.add_css_class("command-history-list");
    list.set_selection_mode(gtk4::SelectionMode::None);

    let db_result = pax_db::Database::open(&pax_db::Database::default_path())
        .and_then(|db| db.latest_distinct_commands(panel_uuid, 500));
    match db_result {
        Ok(records) if !records.is_empty() => {
            for rec in records {
                list.append(&build_history_row(&rec, &terminal_input, &popover));
            }
        }
        _ => {
            let empty = gtk4::Label::new(Some("Nessun comando registrato"));
            empty.add_css_class("dim-label");
            empty.set_margin_top(24);
            empty.set_margin_bottom(24);
            list.append(&empty);
        }
    }

    scroll.set_child(Some(&list));
    popover.set_child(Some(&scroll));
    popover
}
```

Ogni riga è un `gtk::Button` flat con layout orizzontale `[HH:MM]<spazio><cmd>`:
- ora estratta da `executed_at` (ISO format `YYYY-MM-DD HH:MM:SS`),
- comando in font monospace, ellipsize-end,
- tooltip = data completa,
- click = `terminal_input(cmd.as_bytes())` (incolla, NO `\r`) + `popover.popdown()`.

### 4.8 Cleanup

`TerminalPanel::shutdown()` cancella `$PAX_CMD_FILE` (se esiste) per non
lasciare residui. Best-effort: errori ignorati.

I record DB **non** vengono cancellati alla chiusura del pannello — la
cronologia è il punto. La cancellazione del pannello dal workspace (azione
permanente) sì: `on_permanent_close()` chiama una nuova
`Database::delete_command_history_for_panel(panel_uuid)`.

## 5. Test

- **Unit (tp-core):** `PanelConfig` deserializza JSON senza il campo `uuid`
  generandone uno nuovo; il `uuid` viene preservato a roundtrip ser/de.
- **Unit (tp-db):** `latest_distinct_commands` rispetta DISTINCT su `command`
  e ordine `MAX(executed_at) DESC`.
- **Integrazione manuale (richiesta dall'utente):**
  - apri due pannelli terminale, esegui `ls`, `pwd`, `ls` di nuovo →
    popup mostra `pwd` + `ls` (un solo `ls`, con timestamp dell'ultima
    esecuzione);
  - chiudi pax, riapri lo stesso workspace, riapri il popup → la
    cronologia è ancora lì;
  - cancella il pannello, ne crea uno nuovo nello stesso slot → il nuovo
    pannello ha cronologia vuota (UUID diverso);
  - rinomina il workspace → la cronologia per-pannello sopravvive (chiave =
    UUID, indipendente da `record_key`).

## 6. Limiti accettati

- **fish e altre shell esotiche**: la cronologia resta vuota (nessun hook
  iniettato). Coerente con quanto già succede oggi per OSC 133.
- **Comandi con NUL byte**: il transport via `>` redirect è binary-clean
  finché la shell non incappa in NUL — bash e zsh tagliano stringhe a NUL,
  ma è raro nei comandi shell normali.
- **Race teorica**: se la shell esegue due comandi in pipeline rapidissima
  e il GUI è lento a leggere `$PAX_CMD_FILE` tra due preexec successivi,
  potremmo leggere il secondo comando due volte. Mitigato dal flag
  `__pax_preexec_fired` che limita il fire a 1× per prompt.
- **Privacy**: i comandi finiscono in chiaro nel DB. Non filtriamo
  password/token — fuori scope (in linea con `command_history` shell-side).

## 7. Out of scope (lasciato a futuri spec)

- exit code (richiederebbe `OSC 133;D`)
- ricerca testuale dentro al popup (FTS5 esiste già, ma per ora l'UX è
  scroll + DISTINCT)
- groupby giornaliero
- impostazione di privacy/redaction
