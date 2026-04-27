# Markdown Notebook — Design

**Data**: 2026-04-27
**Stato**: spec, in attesa di plan

## Obiettivo

Trasformare il pannello Markdown di Pax in un notebook leggero stile Jupyter:
i fenced code blocks marcati con tag eseguibile (`run`, `once`, `watch=Ns`)
vengono eseguiti come subprocess isolati e il loro output (testo, immagini,
plot) viene mostrato inline sotto al blocco. Output transitorio (mai scritto
nel file `.md`).

## Non-obiettivi (esplicitamente fuori scope, prima iterazione)

- Kernel persistenti / stato condiviso tra blocchi (no Jupyter B/C, solo A
  "notebook leggero": ogni blocco è un processo isolato).
- Persistenza dell'output nel file `.md` o in file laterali. L'output vive
  solo in memoria e si perde alla chiusura del pannello.
- Output ricco oltre testo + immagini (no HTML embed, no widget interattivi,
  no audio/video — agganciabili in futuro tramite il marker convenzionale).
- Linguaggi oltre `python`, `bash`, `sh`.

## Sintassi tag (info string fenced block)

Il tag vive nella info string del fenced block, dopo il nome del linguaggio,
in modo retro-compatibile con CommonMark (parser standard ignorano i token
extra).

```
```python run
print("hello")
```

```python once timeout=120s
import time; time.sleep(60)
```

```bash watch=5s
ps aux | head
```

```python watch=2s confirm
import random; print(random.random())
```
```

### Grammar

```
info_string := lang WS exec_mode (WS attr)*
lang        := "python" | "bash" | "sh"
exec_mode   := "run" | "once" | "watch=" duration
attr        := "timeout=" duration | "confirm"
duration    := number ("s" | "m")
```

- `run` e `once` sono **alias**: una sola esecuzione, manuale (pulsante ▶).
- `watch=Ns`: ciclico, parte automaticamente quando il pannello è
  **visibile** (vedi sezione Lifecycle).
- `timeout=Ns`: max wall-clock per singolo run. Default `30s` per `run`/`once`,
  nessun timeout per `watch` (limitato di fatto dall'intervallo + skip
  strategy).
- `confirm`: opt-in. Se presente, il primo bootstrap del watch in una
  sessione mostra un dialog di conferma. Default = silent autostart.

Una info string priva di exec_mode (es. solo `python`) **non** è un blocco
notebook — viene renderizzata come code block normale (rendering attuale,
nessuna interferenza).

## Architettura — 3 strati

### 1. `tp-core/src/notebook_tag.rs` (nuovo)

Parser puro Rust senza dipendenze GTK, completamente testabile.

```rust
pub enum Lang { Python, Bash, Sh }

pub enum ExecMode {
    Once,                   // run | once
    Watch { interval: Duration },
}

pub struct NotebookCellSpec {
    pub lang: Lang,
    pub mode: ExecMode,
    pub timeout: Option<Duration>,
    pub confirm: bool,
}

impl NotebookCellSpec {
    pub fn parse(info_string: &str) -> Option<Self> { ... }
}
```

Unit test del parser coprono tag validi, tag malformati (graceful None),
durations, alias `run`/`once`, ordine attr arbitrario.

### 2. `tp-gui/src/notebook/` modulo nuovo (lazy)

Inizializzato sul `MarkdownPanel` come `Option<Rc<NotebookEngine>>` —
rimane `None` finché il primo render non incontra un blocco eseguibile.

#### `engine.rs` — `NotebookEngine`

- Process manager (mappa `cell_id -> Option<Child>`).
- Watch scheduler: timer GTK (`glib::timeout_add_local`) per ogni cell
  watch attiva.
- Output store: `HashMap<cell_id, Vec<OutputItem>>`.
- Limite globale: max 8 processi notebook attivi nel processo Pax.
- Bootstrap helpers Python: estrae `helpers.py` (embedded via
  `include_str!`) in cache dir (`~/.cache/pax/notebook_helpers/pax/`)
  e aggiunge il path a `PYTHONPATH` per ogni run.

API:
```rust
pub fn register_cell(spec: NotebookCellSpec, code: String) -> CellId;
pub fn run_cell(id: CellId);
pub fn stop_cell(id: CellId);
pub fn set_visible(id: CellId, visible: bool);  // gating watch
pub fn output(id: CellId) -> Ref<Vec<OutputItem>>;
pub fn subscribe_output(id: CellId, cb: impl Fn());
```

#### `cell.rs` — widget `NotebookCell`

Custom GTK widget (subclass di `gtk::Box`):
- Header: badge linguaggio, pulsante ▶/⏸/⏹, indicatore stato
  (idle/running/error/blocked), label "watch every Ns" se applicabile.
- Output area: `gtk::Box` verticale ricostruito sul change dell'output
  store (subscribe via callback engine).
- Connessione signals `map`/`unmap` → `engine.set_visible(id, …)` per
  gating watch.

#### `runner.rs` — spawn + capture

- Spawn subprocess via `std::process::Command`:
  - `python` → risolve `python3` poi `python` in PATH (fail con error item
    se nessuno trovato).
  - `bash` → `/bin/bash`. `sh` → `/bin/sh`.
  - Codice passato via stdin (più robusto di `-c` per script multi-line).
  - Env: `PYTHONPATH+=<helpers_dir>`, `PAX_OUTPUT_DIR=<tmp>`,
    `PAX_CELL_ID=<id>`.
- Pre-spawn: applica `tp-core/command_safety.rs` (riuso della stessa
  blocklist usata dagli startup script del terminale). Hit → emit
  `OutputItem::Error("blocked: <reason>")`, niente spawn.
- Cattura stdout/stderr line-by-line in thread separato. Per ogni riga
  prova a parsare marker `<<pax:image:...>>` (vedi `output.rs`),
  altrimenti emit come `OutputItem::Text` (stdout) o `OutputItem::Error`
  (stderr).
- Timeout: thread killer monitora wall-clock, SIGTERM → wait 2s →
  SIGKILL.
- Su `glib::idle_add_local` aggiorna l'output store dell'engine
  (mai mutare GTK widget da thread non-main).

#### `output.rs`

```rust
pub enum OutputItem {
    Text(String),
    Image { source: ImageSource },  // Path(PathBuf) | DataUri(String)
    Error(String),
}

pub fn parse_marker(line: &str) -> Option<OutputItem>;
```

Marker parsati:
- `<<pax:image:<path>>>` → `Image::Path`
- `<<pax:image:data:image/png;base64,<...>>>` → `Image::DataUri`

Estensibile a `<<pax:html:...>>`, `<<pax:table:...>>` in futuro.

#### `helpers.py` (embedded)

```python
"""Pax notebook helpers - injected via PYTHONPATH."""
import os, sys, tempfile

def show(target):
    """Render an image inline. Accepts file path or 'data:image/...' URI."""
    if isinstance(target, str):
        print(f"<<pax:image:{target}>>", flush=True)
    else:
        raise TypeError("show() expects str path or data URI")

def show_plot(plt):
    """Save a matplotlib figure to a temp PNG and emit a marker."""
    f = tempfile.NamedTemporaryFile(suffix='.png', delete=False,
                                     dir=os.environ.get('PAX_OUTPUT_DIR'))
    f.close()
    plt.savefig(f.name)
    show(f.name)
```

### 3. `tp-gui/src/panels/markdown.rs` — integrazione

Nel pass di rendering (`markdown_render.rs`), sull'evento
`Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))`:

1. `NotebookCellSpec::parse(&info)` → se `Some(spec)`:
   - Engine lazy-init (se ancora None nel pannello).
   - Raccogli il body del code block (eventi `Text` fino a `End(CodeBlock)`).
   - `cell_id = engine.register_cell(spec, body)`.
   - `widget = NotebookCell::new(engine, cell_id)`.
   - Inserisci `TextChildAnchor` nel buffer del render `TextView` alla
     posizione corrente.
   - `text_view.add_child_at_anchor(&widget, &anchor)`.
2. Se `None` → rendering normale come oggi (tag `code_block`).

L'engine sopravvive ai re-render (Render↔Edit toggle), ma viene
ricostruito su switch di file (pannello carica nuovo path).

## Lifecycle

### Watch
- Timer GTK ogni `interval` secondi.
- Pre-tick check:
  - Se `cell.visible == false` → skip silenzioso.
  - Se run precedente ancora vivo → skip (no accodamento).
- Pannello in Edit mode → `set_visible(false)` per tutte le watch cells
  (pausa); ritorno in Render → re-enable.
- Tab non attiva / pannello non mappato → analogo via signal `unmap`.
- Conferma `confirm`: dialog (con `theme::configure_dialog_window`)
  mostrato al primo tick di sessione; risposta cached per cell_id finché
  vive il pannello.

### Cleanup
- `Drop` su `NotebookEngine`:
  - SIGTERM a tutti i child vivi.
  - Wait con deadline 2s.
  - SIGKILL ai sopravvissuti.
  - Join dei reader thread.
- Chiusura pannello/workspace cascata via Drop normale.

### Limiti
- Max 8 processi notebook attivi globalmente (configurabile, costante
  modulo). Oltre → `OutputItem::Error("notebook process limit reached")`.

## Sicurezza

- Riuso `tp-core/src/command_safety.rs` (stessa blocklist degli startup
  script del terminale). Pre-spawn: se il codice contiene pattern
  bloccato → niente esecuzione, output mostra `Error("blocked: …")`.
- Tag `confirm` come safety net opt-in per scenari "file scaricato": chi
  scrive il file mette `confirm` se vuole che l'utente conservi un veto
  manuale.
- **Nessuna trust list automatica** in prima iterazione: la responsabilità
  di non aprire `.md` non fidati ricade sull'utente (decisione consapevole
  dell'autore del progetto, coerente con "default = silent autostart").

## UI / Help

### Toolbar Markdown
Aggiungere un pulsante `?` (help) nella toolbar principale del
`MarkdownPanel`. Click apre dialog (configurato con
`theme::configure_dialog_window`) con:
- Sintassi tag con esempi.
- Lista linguaggi supportati.
- Marker `<<pax:image:...>>` ed esempio helper Python.
- Note su sicurezza (blocklist, `confirm`).
- Link al file `docs/notebook.md` (apre in editor o fallback istruzione).

### Indicatori cell
- Idle: punto grigio.
- Running: spinner.
- Last run OK: punto verde.
- Last run error: punto rosso + tooltip stderr tail.
- Blocked: lucchetto.
- Watch paused (non visibile): punto giallo + tooltip "paused".

### Documentazione esterna
- `docs/notebook.md` — guida completa in italiano (coerente con README/ROADMAP):
  intro, sintassi, esempi run/once/watch, output ricco, helper Python,
  sicurezza, troubleshooting.
- README.md aggiornato con sezione "Markdown Notebook" + link a
  `docs/notebook.md`.
- Doc inline (`//!` su `notebook_tag.rs` e `notebook/engine.rs`).

## Testing

### Unit
- `notebook_tag::tests`: parser su input validi/malformati, durations,
  alias, ordine attr.
- `output::tests`: marker parser su input edge case (vuoto, malformato,
  marker valido, base64 lungo).

### Integration (con processi reali)
- `runner` con `python -c 'print("hi")'` → `Text("hi")`.
- `runner` con script che fa `import pax; pax.show("/tmp/x.png")` →
  `Image::Path` ricevuto.
- `runner` con timeout violato → `Error("timeout")`.
- `runner` con comando blockato dalla safety blocklist → `Error("blocked")`.

### Manuale (workspace di esempio)
- Creare `examples/notebook-demo.md` con esempi `run`, `once`, `watch`,
  immagine matplotlib, blocco bloccato dalla blocklist.
- Verificare visivamente: render, esecuzione, watch su pannello visibile,
  pausa watch su Edit mode, pausa watch su tab cambio, kill su chiusura.

### Regression
- File markdown senza tag eseguibili → comportamento attuale invariato.
- Sync input + watch contemporanei → no race condition (text_sync ha già
  `suppress_emit` per i propri loop, l'engine non scrive nel buffer).
- Switch Render↔Edit ripetuto → engine non ricreato, output preservato.

## Componenti che NON cambiano

- `markdown_render.rs` rendering "puro" markdown resta identico per
  blocchi non-notebook.
- `text_sync.rs` (sync input markdown) — engine non interagisce con il
  buffer di testo, quindi nessun conflitto.
- Persistenza file: il `.md` non viene mai modificato dall'engine.
- Schema DB / migrazioni: nulla.

## File toccati / creati

**Nuovi:**
- `crates/tp-core/src/notebook_tag.rs`
- `crates/tp-gui/src/notebook/mod.rs`
- `crates/tp-gui/src/notebook/engine.rs`
- `crates/tp-gui/src/notebook/cell.rs`
- `crates/tp-gui/src/notebook/runner.rs`
- `crates/tp-gui/src/notebook/output.rs`
- `crates/tp-gui/src/notebook/helpers.py`
- `docs/notebook.md`
- `examples/notebook-demo.md`

**Modificati:**
- `crates/tp-core/src/lib.rs` (pub mod notebook_tag)
- `crates/tp-gui/src/lib.rs` (pub mod notebook)
- `crates/tp-gui/src/panels/markdown.rs` (toolbar `?` button + lazy engine
  + integrazione render)
- `crates/tp-gui/src/markdown_render.rs` (intercept fenced code block info
  string e chiama callback per anchor child)
- `README.md` (sezione Markdown Notebook)

## Open issues / aperture future

- **Persistenza opzionale**: aggiungere flag CLI o impostazione utente
  per salvare output in `.pax-outputs/` accanto al file. Fuori prima iter.
- **Stato condiviso (Jupyter B)**: kernel REPL persistente per linguaggio.
  Richiede protocollo bidirezionale (stdin per inviare codice).
- **Output ricco esteso**: HTML inline, tabelle, audio/video, widget.
  Architettura marker-based estendibile senza rotture.
- **Trust list per percorso/workspace**: richiesta una sola volta poi
  silenziato. Oggi sostituito dal tag `confirm`.

