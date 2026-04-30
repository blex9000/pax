# Terminal Command History Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Aggiungere ad ogni terminal panel un pulsante nell'header che apre un popup scorrevole con i comandi eseguiti (DISTINCT, ordinati per ultima esecuzione), e cliccando una riga il comando viene incollato nel terminale.

**Architecture:** Il preexec hook della shell scrive il comando in un file sidechannel per-pannello (`$PAX_CMD_FILE`) ed emette OSC 133;C come trigger. I backend VTE/PTY, su 133;C, leggono il file e chiamano `Database::insert_command` con l'**UUID generico del pannello** (nuovo campo `uuid: Uuid` su `PanelConfig`, persistito nel JSON, generato di default da serde — robusto a rinomine workspace e a riuso `id` umani in caso di edit JSON manuale). Il popup mostra la cronologia DISTINCT via una nuova query `Database::latest_distinct_commands`.

**Tech Stack:** Rust, GTK4 + libadwaita, VTE4 (Linux) / portable-pty + alacritty_terminal (macOS), rusqlite + FTS5, serde, uuid v4.

**Spec:** `docs/superpowers/specs/2026-04-30-terminal-command-history-design.md`

**Convenzioni di progetto da rispettare** (vincolanti per chi esegue):
- Niente unit-test nei commit a meno di richiesta esplicita — verifichiamo via `cargo build` e smoke test manuale.
- Ogni Task termina con un commit con messaggio descrittivo (`feat:`/`refactor:`/`fix:` allineato allo stile del repo).
- Nessun co-author Anthropic/Claude nei messaggi.
- Popover stilato via `crate::theme::configure_popover` (regola `app-popover`), niente `gtk::Popover` nudo.
- L'icona usata (`document-open-recent-symbolic`) è già presente in `resources/share/icons/Pax/symbolic/actions/`: nessun nuovo SVG da bundlare.

---

## Task 1: Aggiungere `uuid: Uuid` a `PanelConfig`

**Files:**
- Modify: `crates/tp-core/src/workspace.rs:67-107`
- Modify: `crates/tp-core/src/template.rs:10-26, 39-55, 82-98`
- Modify: `crates/tp-gui/src/workspace_view.rs:1201-1217, 1354-1370, 1866-1882, 2164-2179`

- [ ] **Step 1: Aggiungere il campo nel modello**

In `crates/tp-core/src/workspace.rs` la struct `PanelConfig` ha attualmente come primo campo `pub id: String`. Aggiungi subito dopo `id` il campo UUID:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelConfig {
    pub id: String,
    /// Stable globally-unique identifier, persisted in the JSON.
    /// Used as the database key for any per-panel persistence (e.g.
    /// command history). Survives panel renames, workspace renames, and
    /// re-allocation of the human-friendly `id` (e.g. `p1`, `p2`).
    #[serde(default = "Uuid::new_v4")]
    pub uuid: Uuid,
    #[serde(default)]
    pub name: String,
    // … resto invariato …
```

`Uuid` è già importato in cima al file (`use uuid::Uuid;`).

- [ ] **Step 2: Inizializzare `uuid` in `crates/tp-core/src/template.rs`**

I tre literal `PanelConfig { … }` in `template.rs` devono inizializzare il nuovo campo. Subito dopo `id: …` aggiungi `uuid: Uuid::new_v4(),` in ciascuno. `Uuid` è già importato.

Esempio per il primo (`empty_workspace`, riga 10):

```rust
panels: vec![PanelConfig {
    id: "p1".to_string(),
    uuid: Uuid::new_v4(),
    name: "New Panel".to_string(),
    panel_type: PanelType::Empty,
    // … resto invariato …
```

Ripeti la stessa modifica nei costruttori `simple_hsplit` (riga ~39) e `grid_2x2` (riga ~82): aggiungi `uuid: Uuid::new_v4(),` subito dopo `id: format!(...)`.

- [ ] **Step 3: Inizializzare `uuid` nei costruttori in `workspace_view.rs`**

In `crates/tp-gui/src/workspace_view.rs` ci sono **5** literal `PanelConfig { … }` da aggiornare. In cima al file aggiungi (se non già presente) `use uuid::Uuid;`. Poi aggiungi `uuid: Uuid::new_v4(),` subito dopo il campo `id` in ciascuno dei seguenti blocchi:

- riga 1201, `split_focused`: `let new_cfg = PanelConfig { id: new_id.clone(), uuid: Uuid::new_v4(), name: new_name.clone(), … }`
- riga 1354, `make_empty_config`: aggiungi `uuid: Uuid::new_v4(),` dopo `id: id.to_string(),`
- riga 1866, `panel_config` (helper di test): aggiungi `uuid: Uuid::new_v4(),` dopo `id: id.to_string(),`
- riga 2164, 2168, 2172, 2176 (tutti sono dentro `vec![ PanelConfig { id: "...", … } ]`): aggiungi `uuid: Uuid::new_v4(),` dopo `id` in ciascuno.

(Verifica con: ricerca `PanelConfig {` nel file e controlla che ogni literal abbia il nuovo campo.)

- [ ] **Step 4: Verificare compile**

Run:
```bash
cargo build -p pax-core -p pax-gui
```
Expected: compile pulito. Se il compilatore riporta `missing field 'uuid' in initializer of PanelConfig`, hai mancato un literal — segui l'errore e aggiungi il campo.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-core/src/workspace.rs crates/tp-core/src/template.rs crates/tp-gui/src/workspace_view.rs
git commit -m "core: add stable per-panel uuid to PanelConfig"
```

---

## Task 2: Query DB `latest_distinct_commands`

**Files:**
- Modify: `crates/tp-db/src/commands.rs:13-72`

- [ ] **Step 1: Aggiungere il metodo**

In `crates/tp-db/src/commands.rs` aggiungi un nuovo metodo dentro `impl Database`. La query usa `MIN(id)` come rappresentante del gruppo (per avere un id stabile nel `CommandRecord`), `MAX(executed_at)` come timestamp ordinatore, raggruppa per `command` e ordina per ultimo run desc.

Dopo `pub fn recent_commands(...)` (riga ~71) aggiungi:

```rust
    /// Last distinct commands for a given panel UUID, deduplicated by
    /// command text and ordered by the most recent execution. Used by
    /// the terminal panel "command history" popup.
    pub fn latest_distinct_commands(
        &self,
        panel_uuid: &str,
        limit: usize,
    ) -> Result<Vec<CommandRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT MIN(id), workspace_name, panel_id, command, \
                    MAX(executed_at) AS last_run, exit_code \
             FROM command_history \
             WHERE panel_id = ?1 \
             GROUP BY command \
             ORDER BY last_run DESC \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![panel_uuid, limit as i64], |row| {
            Ok(CommandRecord {
                id: row.get(0)?,
                workspace_name: row.get(1)?,
                panel_id: row.get(2)?,
                command: row.get(3)?,
                executed_at: row.get(4)?,
                exit_code: row.get(5)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
```

- [ ] **Step 2: Verificare compile**

```bash
cargo build -p pax-db
```
Expected: compile pulito.

- [ ] **Step 3: Commit**

```bash
git add crates/tp-db/src/commands.rs
git commit -m "db: add latest_distinct_commands query for panel history popup"
```

---

## Task 3: Shell-aware bootstrap (bash + zsh) con `$PAX_CMD_FILE`

**Files:**
- Modify: `crates/tp-gui/src/panels/terminal/shell_bootstrap.rs`

- [ ] **Step 1: Aggiungere helper per il path del file sidechannel**

In cima a `shell_bootstrap.rs` (dopo il doc-comment del modulo) aggiungi:

```rust
use std::path::PathBuf;
use uuid::Uuid;

/// Path of the per-panel sidechannel file used to ferry the just-executed
/// command from the shell preexec hook to the GUI. Lives under
/// `XDG_RUNTIME_DIR` if set (Linux), otherwise `/tmp`. Filename is
/// `pax-cmd-<simple-uuid>.txt`.
pub fn cmd_file_path(panel_uuid: &Uuid) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    dir.join(format!("pax-cmd-{}.txt", panel_uuid.simple()))
}
```

(Aggiungi `uuid` come dipendenza di `tp-gui` se non già presente. Controlla `crates/tp-gui/Cargo.toml`. Se manca: `uuid = { workspace = true }` in `[dependencies]`.)

- [ ] **Step 2: Estendere `BootstrapConfig` con shell kind e cmd_file path**

Sostituisci la struct `BootstrapConfig` (righe 28-38) con:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    Zsh,
    /// Any unrecognised shell — we do not inject hooks. Command history,
    /// OSC 133 indicators, OSC 7 footer all stay inert (silent fallback).
    Other,
}

impl ShellKind {
    pub fn detect_from_env() -> Self {
        let shell = std::env::var("SHELL").unwrap_or_default();
        Self::detect_from_path(&shell)
    }

    pub fn detect_from_path(shell_path: &str) -> Self {
        let basename = shell_path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(shell_path)
            .trim();
        match basename {
            "bash" => Self::Bash,
            "zsh" => Self::Zsh,
            _ => Self::Other,
        }
    }
}

pub struct BootstrapConfig<'a> {
    pub shell: ShellKind,
    /// Override PS1 (bash) / PROMPT (zsh) with pax's minimal green prompt.
    pub override_ps1: bool,
    /// Emit OSC 7 (directory URI) from the prompt callback.
    pub emit_osc7: bool,
    /// Path the preexec hook will write the command into. Borrowed so we
    /// do not allocate per call. Use `cmd_file_path(panel_uuid)`.
    pub cmd_file: &'a std::path::Path,
}
```

- [ ] **Step 3: Riscrivere `bootstrap_lines` come dispatcher**

Sostituisci la fn `bootstrap_lines` (righe 42-72) con:

```rust
/// Returns the ordered list of shell lines to feed after the shell has
/// loaded the user's rc file. Both backends feed each line followed by
/// `\n`. For unknown shells returns an empty vec (silent no-op).
pub fn bootstrap_lines(cfg: &BootstrapConfig) -> Vec<String> {
    match cfg.shell {
        ShellKind::Bash => bash_lines(cfg),
        ShellKind::Zsh => zsh_lines(cfg),
        ShellKind::Other => Vec::new(),
    }
}

fn bash_lines(cfg: &BootstrapConfig) -> Vec<String> {
    let cmd_file = cfg.cmd_file.display().to_string();
    let mut lines: Vec<String> = Vec::with_capacity(16);
    lines.push("set +o history".to_string());
    lines.push(format!("export PAX_CMD_FILE='{}'", shell_escape_single(&cmd_file)));
    if cfg.override_ps1 {
        lines.push(r"export PS1='\[\033[32m\]$:\[\033[0m\] '".to_string());
    }
    lines.push(
        "export LS_COLORS='di=38;2;85;136;255:ln=36:so=35:pi=33:ex=32:bd=34;46:cd=34;43:su=30;41:sg=30;46:tw=30;42:ow=34;42'"
            .to_string(),
    );
    lines.push("export CLICOLOR=1".to_string());
    lines.push("export LSCOLORS='ExFxCxDxBxegedabagacad'".to_string());
    lines.push(
        r#"if command -v dircolors >/dev/null 2>&1; then eval "$(dircolors -b 2>/dev/null)"; fi"#
            .to_string(),
    );
    lines.push(
        r#"if command ls --color=auto . >/dev/null 2>&1; then alias ls='ls --color=auto'; else alias ls='ls -G'; fi"#
            .to_string(),
    );
    lines.push(bash_prompt_function(cfg.emit_osc7));
    lines.push(bash_preexec_function());
    lines.push(r#"PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }__pax_prompt""#.to_string());
    lines.push("trap '__pax_preexec' DEBUG".to_string());
    lines.push("set -o history".to_string());
    lines.push("clear".to_string());
    lines
}

fn zsh_lines(cfg: &BootstrapConfig) -> Vec<String> {
    let cmd_file = cfg.cmd_file.display().to_string();
    let mut lines: Vec<String> = Vec::with_capacity(14);
    // zsh: setopt nohistsave isn't a thing; we use HISTFILE='' for the
    // current session via `unset HISTFILE` to avoid leaking the bootstrap.
    // But that nukes user history — instead, just don't `clear` after and
    // count on the lines being short and ephemeral.
    lines.push(format!("export PAX_CMD_FILE='{}'", shell_escape_single(&cmd_file)));
    if cfg.override_ps1 {
        // zsh prompt syntax (single-quoted to keep %F/%f literal until
        // the prompt is evaluated each time).
        lines.push(r"export PROMPT='%F{green}$:%f '".to_string());
    }
    lines.push(
        "export LS_COLORS='di=38;2;85;136;255:ln=36:so=35:pi=33:ex=32:bd=34;46:cd=34;43:su=30;41:sg=30;46:tw=30;42:ow=34;42'"
            .to_string(),
    );
    lines.push("export CLICOLOR=1".to_string());
    lines.push("export LSCOLORS='ExFxCxDxBxegedabagacad'".to_string());
    lines.push(
        r#"if command ls --color=auto . >/dev/null 2>&1; then alias ls='ls --color=auto'; else alias ls='ls -G'; fi"#
            .to_string(),
    );
    lines.push(zsh_prompt_function(cfg.emit_osc7));
    lines.push(zsh_preexec_function());
    lines.push("typeset -ga precmd_functions preexec_functions".to_string());
    lines.push("precmd_functions+=(__pax_prompt_zsh)".to_string());
    lines.push("preexec_functions+=(__pax_preexec_zsh)".to_string());
    lines.push("clear".to_string());
    lines
}

fn shell_escape_single(s: &str) -> String {
    // POSIX single-quote escaping: ' → '\''
    s.replace('\'', r"'\''")
}
```

- [ ] **Step 4: Sostituire le funzioni `prompt_function` / `preexec_function` con varianti per-shell**

Sostituisci le righe 74-88 (le due funzioni esistenti) con:

```rust
fn bash_prompt_function(emit_osc7: bool) -> String {
    let osc7 = if emit_osc7 {
        r#"printf '\033]7;file://%s%s\033\\' "$HOSTNAME" "$PWD"; "#
    } else {
        ""
    };
    format!(
        r##"__pax_prompt() {{ local d="${{PWD/#$HOME/~}}"; printf '\033]0;%s@%s: %s\007' "$USER" "$HOSTNAME" "$d"; {}printf '\033]133;A\007'; __pax_preexec_fired=; }}"##,
        osc7
    )
}

fn bash_preexec_function() -> String {
    // Write the just-recognised command to $PAX_CMD_FILE before signalling
    // 133;C, so the GUI side can read it when 133;C arrives.
    r##"__pax_preexec() { [[ -n "$__pax_preexec_fired" ]] && return; __pax_preexec_fired=1; if [[ -n "$PAX_CMD_FILE" ]]; then printf '%s' "$BASH_COMMAND" > "$PAX_CMD_FILE" 2>/dev/null; fi; printf '\033]133;C\007'; }"##.to_string()
}

fn zsh_prompt_function(emit_osc7: bool) -> String {
    let osc7 = if emit_osc7 {
        r#"print -n -- $'\e]7;file://'"$HOST$PWD"$'\e\\'; "#
    } else {
        ""
    };
    format!(
        r##"__pax_prompt_zsh() {{ local d="${{PWD/#$HOME/~}}"; print -n -- $'\e]0;'"$USER@$HOST: $d"$'\a'; {}print -n -- $'\e]133;A\a'; }}"##,
        osc7
    )
}

fn zsh_preexec_function() -> String {
    // zsh preexec receives the command as $1 (raw) / $2 (after history
    // expansion) — we use $1 to preserve what the user actually typed.
    r##"__pax_preexec_zsh() { if [[ -n "$PAX_CMD_FILE" ]]; then print -nr -- "$1" > "$PAX_CMD_FILE" 2>/dev/null; fi; print -n -- $'\e]133;C\a'; }"##.to_string()
}
```

- [ ] **Step 5: Verificare compile**

```bash
cargo build -p pax-gui
```

Expected: errori di tipo nei due call-site (`vte_backend.rs`, `pty_backend.rs`) perché `BootstrapConfig` ora ha campi nuovi. Sono attesi e vengono risolti in Task 4.

- [ ] **Step 6: Commit**

Non committare ancora — Task 4 sistema i call-site. Lasciamo il working tree dirty.

---

## Task 4: Plumbing UUID + cmd_file + shell kind nei backend

**Files:**
- Modify: `crates/tp-gui/src/panels/terminal/mod.rs`
- Modify: `crates/tp-gui/src/panels/terminal/vte_backend.rs:33, 175-200`
- Modify: `crates/tp-gui/src/panels/terminal/pty_backend.rs:20, 1090-1110`
- Modify: `crates/tp-gui/src/panels/registry.rs:236-241`
- Modify: `crates/tp-gui/src/backend_factory.rs:88-148`

- [ ] **Step 1: Estendere `TerminalPanel::new` con UUID e cmd_file**

In `crates/tp-gui/src/panels/terminal/mod.rs`, sostituisci `TerminalPanel::new` (righe 130-140) con:

```rust
impl TerminalPanel {
    pub fn new(
        shell: &str,
        cwd: Option<&str>,
        env: &[(String, String)],
        workspace_dir: Option<&str>,
        panel_uuid: Option<uuid::Uuid>,
    ) -> Self {
        Self {
            inner: TerminalInner::new(shell, cwd, env, workspace_dir, panel_uuid),
            ssh_info: None,
        }
    }
```

`uuid::Uuid` è un nuovo import in cima al file.

- [ ] **Step 2: Estendere `TerminalInner::new` (VTE backend)**

In `crates/tp-gui/src/panels/terminal/vte_backend.rs`, modifica la firma di `TerminalInner::new` (righe ~140-145) per accettare il nuovo parametro:

```rust
    pub fn new(
        shell: &str,
        cwd: Option<&str>,
        env: &[(String, String)],
        workspace_dir: Option<&str>,
        panel_uuid: Option<uuid::Uuid>,
    ) -> Self {
```

E nel sito di chiamata di `bootstrap_lines` (riga ~192) usa il nuovo `BootstrapConfig` shell-aware:

```rust
                    let shell_kind = super::shell_bootstrap::ShellKind::detect_from_path(&shell_for_cb);
                    let cmd_file = match panel_uuid {
                        Some(u) => super::shell_bootstrap::cmd_file_path(&u),
                        None => std::path::PathBuf::new(),
                    };
                    for line in bootstrap_lines(&BootstrapConfig {
                        shell: shell_kind,
                        override_ps1: true,
                        emit_osc7: true,
                        cmd_file: &cmd_file,
                    }) {
                        let mut bytes = line.into_bytes();
                        bytes.push(b'\n');
                        vte_for_cb.feed_child(&bytes);
                    }
```

`shell_for_cb` è una stringa già clonata dalla shell per l'uso interno; se non esiste, prima dello spawn aggiungi `let shell_for_cb = shell.to_string();`. `panel_uuid` è ora un parametro: clonalo nel context della closure se serve via `let panel_uuid_for_cb = panel_uuid;`.

Salva `panel_uuid` e `cmd_file` come campi di `TerminalInner` (servono al Task 5/9): aggiungi alla struct:

```rust
pub struct TerminalInner {
    // … campi esistenti …
    pub(super) panel_uuid: Option<uuid::Uuid>,
    pub(super) cmd_file: std::path::PathBuf,
}
```

E inizializzali in `Self { … }` alla fine di `new` (subito dopo `cwd_cb`):

```rust
            panel_uuid,
            cmd_file: panel_uuid
                .map(|u| super::shell_bootstrap::cmd_file_path(&u))
                .unwrap_or_default(),
```

- [ ] **Step 3: Estendere `TerminalInner::new` (PTY backend)**

In `crates/tp-gui/src/panels/terminal/pty_backend.rs` (righe ~134-140) cambia firma identica:

```rust
    pub fn new(
        shell: &str,
        cwd: Option<&str>,
        env: &[(String, String)],
        _workspace_dir: Option<&str>,
        panel_uuid: Option<uuid::Uuid>,
    ) -> Self {
```

Trova la chiamata a `bootstrap_lines` (righe ~1097-1101). Sostituiscila con:

```rust
    let shell_kind = super::shell_bootstrap::ShellKind::detect_from_path(shell);
    let cmd_file = match panel_uuid {
        Some(u) => super::shell_bootstrap::cmd_file_path(&u),
        None => std::path::PathBuf::new(),
    };
    let cfg = BootstrapConfig {
        shell: shell_kind,
        override_ps1: false,
        emit_osc7: false,
        cmd_file: &cmd_file,
    };
    for command in bootstrap_lines(&cfg) {
        // … resto invariato …
    }
```

Nota: `shell` è già un `&str` nel context, e `panel_uuid` arriva come parametro. Se `clear` è hard-coded nella PTY mainline (rivedi le righe attorno), tienilo allineato ai bash_lines.

Aggiungi i campi a `TerminalInner` (PTY) e inizializzali in fondo a `new` analogo al VTE:

```rust
pub struct TerminalInner {
    // … campi esistenti …
    pub(super) panel_uuid: Option<uuid::Uuid>,
    pub(super) cmd_file: std::path::PathBuf,
}

// in fondo a Self { … }:
            panel_uuid,
            cmd_file,
```

(In PTY abbiamo già `cmd_file` come variabile locale: clona prima di passarla a `bootstrap_lines`, oppure costruiscila due volte — la fn è economica.)

- [ ] **Step 4: Plumbing factory → registry**

In `crates/tp-gui/src/backend_factory.rs`, dentro `create_backend_from_registry` alla fine (riga ~148, subito prima del `let config = PanelCreateConfig { … }`), inserisci:

```rust
    extra.insert(
        "__panel_uuid__".to_string(),
        panel_cfg.uuid.simple().to_string(),
    );
```

In `crates/tp-gui/src/panels/registry.rs`, dentro la closure factory di `terminal` (righe 205-241), prima di costruire `TerminalPanel::new` estrai il UUID:

```rust
            let panel_uuid = config
                .extra
                .get("__panel_uuid__")
                .and_then(|s| uuid::Uuid::parse_str(s).ok());
            let mut panel = super::terminal::TerminalPanel::new(
                shell,
                effective_cwd,
                &env,
                ws_dir,
                panel_uuid,
            );
```

Aggiungi `use uuid::Uuid;` in cima al file se manca (oppure usa il path qualificato `uuid::Uuid::parse_str`).

- [ ] **Step 5: Cargo.toml di tp-gui — verifica `uuid`**

Controlla `crates/tp-gui/Cargo.toml`: se `uuid = { workspace = true }` non è già nelle `[dependencies]`, aggiungilo:

```toml
[dependencies]
# … esistenti …
uuid = { workspace = true }
```

- [ ] **Step 6: Verificare compile completo**

```bash
cargo build
```

Expected: compile pulito. Errori residui = chiamanti di `TerminalPanel::new` con la vecchia aria — cerca con `Grep "TerminalPanel::new"` e aggiorna ogni call-site con `, None` come ultimo argomento se non si conosce ancora l'UUID.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "terminal: shell-aware bootstrap with PAX_CMD_FILE sidechannel"
```

---

## Task 5: VTE backend → `insert_command` su preexec

**Files:**
- Modify: `crates/tp-gui/src/panels/terminal/vte_backend.rs:261-291`

- [ ] **Step 1: Estendere il preexec handler per leggere $PAX_CMD_FILE**

Trova `vte.connect_shell_preexec` (riga ~276). Esiste una closure che fa `cb(true)`. Sostituiscila con una versione che PRIMA legge il file e inserisce nel DB, POI fa `cb(true)`. Aggiungi i clone necessari:

```rust
        if vte_has_shell_integration_signals() {
            let status_cb_ref = status_cb.clone();
            vte.connect_shell_precmd(move |_| {
                if let Ok(borrowed) = status_cb_ref.try_borrow() {
                    if let Some(ref cb) = *borrowed {
                        cb(false);
                    }
                }
            });
            let status_cb_ref = status_cb.clone();
            let cmd_file_for_cb = if let Some(u) = panel_uuid {
                super::shell_bootstrap::cmd_file_path(&u)
            } else {
                std::path::PathBuf::new()
            };
            let panel_uuid_str: Option<String> =
                panel_uuid.map(|u| u.simple().to_string());
            let workspace_name_for_cb: Option<String> = workspace_dir
                .map(|s| s.to_string());
            vte.connect_shell_preexec(move |_| {
                if !cmd_file_for_cb.as_os_str().is_empty() {
                    if let Ok(raw) = std::fs::read_to_string(&cmd_file_for_cb) {
                        let cmd = raw.trim_end_matches(['\n', '\r']);
                        if !cmd.is_empty() {
                            if let Ok(db) = pax_db::Database::open(&pax_db::Database::default_path()) {
                                let _ = db.insert_command(
                                    workspace_name_for_cb.as_deref(),
                                    panel_uuid_str.as_deref(),
                                    cmd,
                                    None,
                                );
                            }
                        }
                    }
                }
                if let Ok(borrowed) = status_cb_ref.try_borrow() {
                    if let Some(ref cb) = *borrowed {
                        cb(true);
                    }
                }
            });
        } else {
            // (Fallback poller invariato)
            spawn_tcgetpgrp_poller(&vte, shell_pid.clone(), status_cb.clone());
        }
```

`workspace_dir` è già un parametro di `new` ed è `Option<&str>`; clonalo in una `Option<String>` per spostarla nella closure (`'static`).

- [ ] **Step 2: Verificare compile**

```bash
cargo build -p pax-gui
```

Expected: compile pulito. Se rusqlite errors → `cargo build` userà `pax-db` riusando la default_path; se manca import, aggiungi `use pax_db;` in cima al file (già usato da altre parti — verifica).

- [ ] **Step 3: Commit**

```bash
git add crates/tp-gui/src/panels/terminal/vte_backend.rs
git commit -m "terminal/vte: persist commands via OSC 133;C preexec hook"
```

---

## Task 6: PTY backend → `insert_command` su 133;C

**Files:**
- Modify: `crates/tp-gui/src/panels/terminal/pty_backend.rs:60-114, 211-300, 652-663`

- [ ] **Step 1: Aggiungere variant `CommandStarted` all'enum**

In fondo all'enum `TerminalUiEvent` (~riga 660) aggiungi:

```rust
enum TerminalUiEvent {
    Render,
    ClipboardStore(String),
    ClipboardLoad(Arc<dyn Fn(&str) -> String + Sync + Send + 'static>),
    /// OSC 0/2 title update; empty string = reset/clear (from Event::ResetTitle).
    TitleChanged(String),
    /// OSC 133 shell integration: `true` = waiting (prompt up, no command),
    /// `false` = running (command started after prompt).
    StatusChanged(bool),
    /// OSC 7 current-directory-uri update (`file://<host>/<path>`).
    CwdChanged(String),
    /// OSC 133;C arrived — read $PAX_CMD_FILE on the GUI thread and persist.
    CommandStarted,
}
```

- [ ] **Step 2: Emettere `CommandStarted` quando lo scanner trova 133;C**

In `scan_osc_markers` (riga ~80), nel branch `b'C'`, oltre a inviare `StatusChanged(true)` invia anche `CommandStarted`:

```rust
                b'C' => {
                    let _ = ui_tx.send(TerminalUiEvent::StatusChanged(true));
                    let _ = ui_tx.send(TerminalUiEvent::CommandStarted);
                }
```

- [ ] **Step 3: Gestire `CommandStarted` nel main loop**

Trova il match dove gli eventi vengono dispatchati alla GLib main loop (`Ok(TerminalUiEvent::StatusChanged(...)) => { … }`, riga ~266). Aggiungi un nuovo arm subito dopo quello di `CwdChanged`:

```rust
                        Ok(TerminalUiEvent::CommandStarted) => {
                            if !cmd_file_for_main.as_os_str().is_empty() {
                                if let Ok(raw) = std::fs::read_to_string(&cmd_file_for_main) {
                                    let cmd = raw.trim_end_matches(['\n', '\r']);
                                    if !cmd.is_empty() {
                                        if let Ok(db) = pax_db::Database::open(&pax_db::Database::default_path()) {
                                            let _ = db.insert_command(
                                                workspace_name_for_main.as_deref(),
                                                panel_uuid_str_for_main.as_deref(),
                                                cmd,
                                                None,
                                            );
                                        }
                                    }
                                }
                            }
                        }
```

`cmd_file_for_main`, `workspace_name_for_main`, `panel_uuid_str_for_main` sono cloni catturati dalla closure del main-loop attach. Aggiungili prima dello `glib::source::idle_add_local`/`timeout_add_local` o equivalente che dispatcha gli eventi UI — cerca dove `ui_rx` viene consumato (intorno alla riga 250). Subito prima:

```rust
        let cmd_file_for_main = cmd_file.clone();
        let workspace_name_for_main: Option<String> = None; // PTY backend non riceve workspace_name oggi — opzionale
        let panel_uuid_str_for_main: Option<String> = panel_uuid.map(|u| u.simple().to_string());
```

- [ ] **Step 4: Verificare compile**

```bash
cargo build --no-default-features
```

(Build flag macOS — vogliamo verificare il PTY backend specifically.)
Expected: compile pulito.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/panels/terminal/pty_backend.rs
git commit -m "terminal/pty: persist commands on OSC 133;C via PAX_CMD_FILE"
```

---

## Task 7: Pulsante header "cronologia" (terminal-only)

**Files:**
- Modify: `crates/tp-gui/src/panel_host.rs:139-543`

- [ ] **Step 1: Aggiungere il bottone alla struct e crearlo**

In `PanelHost` (riga 139), subito sotto `zoom_button: gtk4::Button,` aggiungi:

```rust
    history_button: gtk4::Button,
```

In `PanelHost::new` (riga ~318, dopo la creazione di `zoom_button`) aggiungi:

```rust
        // Command history button — visible only when backend is a terminal.
        let history_button = gtk4::Button::new();
        history_button.set_icon_name("document-open-recent-symbolic");
        history_button.add_css_class("flat");
        history_button.add_css_class("panel-action-btn");
        history_button.set_tooltip_text(Some("Cronologia comandi"));
        history_button.set_visible(false);
```

- [ ] **Step 2: Inserirlo nell'`end_box`**

In `PanelHost::new` cerca il blocco di `end_box.append(...)` (intorno a riga 381). Inserisci il bottone tra `zoom_button` e `menu_button`:

```rust
        let end_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        end_box.append(&sync_button);
        end_box.append(&zoom_button);
        end_box.append(&history_button);
        end_box.append(&menu_button);
```

- [ ] **Step 3: Salvarlo nello `Self { … }`**

Nel costruttore (riga ~517), aggiungi `history_button` al literal `Self`:

```rust
        Self {
            // … campi esistenti …
            zoom_button,
            history_button,
            menu_button,
            // …
        }
```

- [ ] **Step 4: Mostrarlo solo per terminali in `set_backend`**

Cerca `set_backend` in `panel_host.rs` (`Grep "fn set_backend"`). All'interno, dopo aver salvato il backend, aggiungi:

```rust
        let is_terminal = backend.panel_type() == "terminal";
        self.history_button.set_visible(is_terminal);
```

- [ ] **Step 5: Verificare compile**

```bash
cargo build -p pax-gui
```
Expected: compile pulito.

- [ ] **Step 6: Commit**

```bash
git add crates/tp-gui/src/panel_host.rs
git commit -m "panel-host: add command history button (terminal-only)"
```

---

## Task 8: Popover cronologia comandi (lista + click-to-paste)

**Files:**
- Create: `crates/tp-gui/src/dialogs/command_history.rs`
- Modify: `crates/tp-gui/src/dialogs/mod.rs`
- Modify: `crates/tp-gui/src/panel_host.rs` (agganciare click sul bottone)
- Modify: `crates/tp-gui/src/panels/terminal/mod.rs` (esporre helper per UUID)

- [ ] **Step 1: Esporre l'UUID del terminal panel**

In `crates/tp-gui/src/panels/terminal/mod.rs`, aggiungi alla `impl TerminalPanel`:

```rust
    /// UUID of this panel (if known). Used by the header to query the
    /// per-panel command history.
    pub fn panel_uuid(&self) -> Option<uuid::Uuid> {
        self.inner.panel_uuid
    }
```

- [ ] **Step 2: Trait `PanelBackend` — opzionalmente esporre l'UUID**

In `crates/tp-gui/src/panels/mod.rs` aggiungi alla trait `PanelBackend` (dopo `ssh_label`):

```rust
    /// Stable per-panel UUID. Returned by panels that participate in
    /// per-panel persistence (terminal command history, …). Default
    /// `None`: panels without persistence.
    fn panel_uuid(&self) -> Option<uuid::Uuid> {
        None
    }
```

In `TerminalPanel`'s impl di `PanelBackend` (in `terminal/mod.rs`) aggiungi:

```rust
    fn panel_uuid(&self) -> Option<uuid::Uuid> {
        self.inner.panel_uuid
    }
```

- [ ] **Step 3: Creare il modulo del popover**

Crea `crates/tp-gui/src/dialogs/command_history.rs`:

```rust
//! Command history popover for terminal panels.
//!
//! Shows the per-panel command history from `command_history` table,
//! deduplicated by command text and ordered by last execution. Clicking
//! a row writes the command text into the terminal (no Enter), letting
//! the user edit before executing.

use gtk4::prelude::*;

use crate::panels::PanelInputCallback;

/// Maximum number of distinct commands shown in the popover. Older
/// distinct commands beyond this cap are not loaded — keeps popover
/// snappy on long-lived terminals.
const HISTORY_LIMIT: usize = 500;

/// Build (or rebuild) the contents of the command-history popover for
/// `panel_uuid`. Each row, when clicked, writes its command into the
/// terminal via `input_cb` (no `\r` appended) and pops the popover down.
pub fn build_command_history_popover(
    panel_uuid: &str,
    input_cb: PanelInputCallback,
) -> gtk4::Popover {
    let popover = gtk4::Popover::new();
    crate::theme::configure_popover(&popover);
    popover.add_css_class("command-history-popover");

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    outer.set_margin_top(6);
    outer.set_margin_bottom(6);
    outer.set_margin_start(6);
    outer.set_margin_end(6);

    let heading = gtk4::Label::new(Some("Cronologia comandi"));
    heading.add_css_class("heading");
    heading.set_halign(gtk4::Align::Start);
    outer.append(&heading);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_min_content_width(420);
    scroll.set_min_content_height(280);
    scroll.set_max_content_height(420);
    scroll.set_propagate_natural_height(true);

    let list = gtk4::ListBox::new();
    list.add_css_class("command-history-list");
    list.set_selection_mode(gtk4::SelectionMode::None);

    let db_result = pax_db::Database::open(&pax_db::Database::default_path())
        .and_then(|db| db.latest_distinct_commands(panel_uuid, HISTORY_LIMIT));

    match db_result {
        Ok(records) if !records.is_empty() => {
            for rec in records {
                list.append(&build_history_row(&rec, input_cb.clone(), &popover));
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
    outer.append(&scroll);
    popover.set_child(Some(&outer));
    popover
}

fn build_history_row(
    rec: &pax_db::CommandRecord,
    input_cb: PanelInputCallback,
    popover: &gtk4::Popover,
) -> gtk4::Widget {
    let row_btn = gtk4::Button::new();
    row_btn.add_css_class("flat");
    row_btn.add_css_class("command-history-row");

    let h = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    h.set_margin_start(6);
    h.set_margin_end(6);
    h.set_margin_top(2);
    h.set_margin_bottom(2);

    let time = extract_hh_mm(&rec.executed_at);
    let time_lbl = gtk4::Label::new(Some(&format!("[{}]", time)));
    time_lbl.add_css_class("dim-label");
    time_lbl.add_css_class("command-history-time");
    time_lbl.set_halign(gtk4::Align::Start);
    h.append(&time_lbl);

    let cmd_lbl = gtk4::Label::new(Some(&rec.command));
    cmd_lbl.add_css_class("monospace");
    cmd_lbl.add_css_class("command-history-cmd");
    cmd_lbl.set_halign(gtk4::Align::Start);
    cmd_lbl.set_hexpand(true);
    cmd_lbl.set_xalign(0.0);
    cmd_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    h.append(&cmd_lbl);

    row_btn.set_child(Some(&h));
    row_btn.set_tooltip_text(Some(&rec.executed_at));

    let cmd = rec.command.clone();
    let popover = popover.clone();
    row_btn.connect_clicked(move |_| {
        input_cb(cmd.as_bytes());
        popover.popdown();
    });

    row_btn.upcast::<gtk4::Widget>()
}

fn extract_hh_mm(executed_at: &str) -> String {
    // `executed_at` is SQLite `datetime('now')` format: "YYYY-MM-DD HH:MM:SS".
    // Best-effort: take the substring at positions 11..16. Otherwise fallback
    // to the raw string truncated.
    if executed_at.len() >= 16 {
        executed_at[11..16].to_string()
    } else {
        executed_at.to_string()
    }
}
```

- [ ] **Step 4: Registrare il modulo**

In `crates/tp-gui/src/dialogs/mod.rs` aggiungi:

```rust
pub mod command_history;
```

(Verifica con `Read` lo stile di altri `pub mod` già presenti.)

- [ ] **Step 5: Riesportare `CommandRecord` da pax-db**

In `crates/tp-db/src/lib.rs` verifica che `pub use commands::CommandRecord;` esista. Se manca, aggiungi:

```rust
pub use commands::CommandRecord;
```

- [ ] **Step 6: Agganciare il bottone al popover**

In `panel_host.rs`, nel `set_backend` (lo stesso punto del Task 7 step 4), aggiungi sotto `set_visible`:

```rust
        if is_terminal {
            // Wire the popover lazily on each click — keeps the data fresh
            // without listening to insert events.
            let history_button = self.history_button.clone();
            let backend_ref = self.backend.clone();
            history_button.connect_clicked(move |btn| {
                let (panel_uuid, input_cb): (Option<uuid::Uuid>, Option<crate::panels::PanelInputCallback>) =
                    if let Ok(borrowed) = backend_ref.try_borrow() {
                        if let Some(ref backend) = *borrowed {
                            let uuid = backend.panel_uuid();
                            // Build a thin closure that writes to the backend.
                            let backend_clone_ref = backend_ref.clone();
                            let cb: crate::panels::PanelInputCallback = std::rc::Rc::new(move |bytes: &[u8]| {
                                if let Ok(b) = backend_clone_ref.try_borrow() {
                                    if let Some(ref be) = *b {
                                        be.write_input(bytes);
                                    }
                                }
                            });
                            (uuid, Some(cb))
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    };

                let Some(uuid) = panel_uuid else { return; };
                let Some(input_cb) = input_cb else { return; };
                let popover = crate::dialogs::command_history::build_command_history_popover(
                    &uuid.simple().to_string(),
                    input_cb,
                );
                popover.set_parent(btn);
                popover.popup();
            });
        }
```

Nota: ogni click distrugge il vecchio popover (se c'era) e ne costruisce uno nuovo. È intenzionale per riflettere comandi appena inseriti.

- [ ] **Step 7: CSS per le righe del popover**

In `crates/tp-gui/src/theme.rs`, dentro `BASE_CSS` (cerca `command-` o aggiungi in fondo prima del backtick di chiusura), aggiungi:

```css
.command-history-popover .heading { font-size: 0.95em; padding: 2px 4px; }
.command-history-popover .command-history-list row,
.command-history-popover .command-history-row { padding: 0; min-height: 0; }
.command-history-popover .command-history-time {
    min-width: 4.5em;
    opacity: 0.6;
    font-variant-numeric: tabular-nums;
}
.command-history-popover .command-history-cmd {
    font-family: "JetBrains Mono", "SF Mono", "Cascadia Code", monospace;
    font-size: 0.9em;
}
```

(Trova la regola esistente più vicina e mantieni il formato/indentazione del file.)

- [ ] **Step 8: Verificare compile**

```bash
cargo build
```
Expected: compile pulito.

- [ ] **Step 9: Commit**

```bash
git add crates/tp-gui/src/dialogs/command_history.rs \
        crates/tp-gui/src/dialogs/mod.rs \
        crates/tp-gui/src/panel_host.rs \
        crates/tp-gui/src/panels/mod.rs \
        crates/tp-gui/src/panels/terminal/mod.rs \
        crates/tp-gui/src/theme.rs \
        crates/tp-db/src/lib.rs
git commit -m "panels/terminal: command history popover with click-to-paste"
```

---

## Task 9: Cleanup file sidechannel + smoke test manuale

**Files:**
- Modify: `crates/tp-gui/src/panels/terminal/mod.rs`
- Modify: `crates/tp-gui/src/panels/terminal/vte_backend.rs` (in `shutdown`)
- Modify: `crates/tp-gui/src/panels/terminal/pty_backend.rs` (in `shutdown`)
- Modify: `crates/tp-db/src/commands.rs` (delete-by-panel)

- [ ] **Step 1: Cancellare il file sidechannel su `shutdown`**

In `vte_backend.rs` cerca `pub fn shutdown(&self)` in `impl TerminalInner`. Aggiungi all'inizio:

```rust
    pub fn shutdown(&self) {
        if !self.cmd_file.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.cmd_file);
        }
        // … logica esistente …
    }
```

Stessa modifica in `pty_backend.rs`.

- [ ] **Step 2: Aggiungere `delete_command_history_for_panel`**

In `crates/tp-db/src/commands.rs`, dopo `latest_distinct_commands` aggiungi:

```rust
    /// Remove all command history rows for a given panel UUID. Called
    /// when the panel is permanently closed to avoid leaving orphan rows.
    pub fn delete_command_history_for_panel(&self, panel_uuid: &str) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM command_history WHERE panel_id = ?1",
            rusqlite::params![panel_uuid],
        )?;
        Ok(n)
    }
```

Nota: la trigger FTS5 esistente (`command_history_ai`) gestisce solo INSERT, non DELETE. Per non lasciare orfani in `command_history_fts`, aggiungi anche un AFTER DELETE trigger nello schema. In `crates/tp-db/src/schema.rs` (o equivalente di migrations), trova la creazione del trigger `command_history_ai` e aggiungi subito dopo:

```sql
CREATE TRIGGER IF NOT EXISTS command_history_ad AFTER DELETE ON command_history BEGIN
    INSERT INTO command_history_fts(command_history_fts, rowid, command)
    VALUES ('delete', old.id, old.command);
END;
```

(Verifica la presenza del trigger AI prima di aggiungere AD; se i due esistono già, salta questo step.)

- [ ] **Step 3: Chiamare il delete su `on_permanent_close` per terminali**

In `crates/tp-gui/src/panels/terminal/mod.rs`, nell'`impl PanelBackend for TerminalPanel`, aggiungi un override:

```rust
    fn on_permanent_close(&self) {
        if let Some(uuid) = self.inner.panel_uuid {
            if let Ok(db) = pax_db::Database::open(&pax_db::Database::default_path()) {
                let _ = db.delete_command_history_for_panel(&uuid.simple().to_string());
            }
        }
    }
```

- [ ] **Step 4: Verificare compile**

```bash
cargo build
```
Expected: compile pulito.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/panels/terminal/mod.rs \
        crates/tp-gui/src/panels/terminal/vte_backend.rs \
        crates/tp-gui/src/panels/terminal/pty_backend.rs \
        crates/tp-db/src/commands.rs \
        crates/tp-db/src/schema.rs
git commit -m "terminal: cleanup PAX_CMD_FILE and DB rows on close"
```

- [ ] **Step 6: Smoke test manuale (Linux/VTE)**

Avvia un workspace di test:

```bash
RUST_LOG=pax_gui=info cargo run -- new "history-test"
```

Esegui in un terminale del workspace:
1. `pwd`
2. `ls /tmp`
3. `ls /tmp` (ripetuto)
4. `echo hello world`

Apri il popup col nuovo bottone (icona "recent" nell'header del pannello). Verifica:
- Vedi 3 righe (ls compare 1× nonostante 2 esecuzioni — DISTINCT funziona).
- Le righe sono ordinate per ultima esecuzione decrescente.
- Cliccando una riga, il comando appare nel terminale (NO Enter automatico).
- Tooltip mostra il timestamp completo.

Salva il workspace, chiudi pax, riapri. Apri di nuovo il popup → la cronologia è ancora lì.

Con un secondo pannello terminale aperto, verifica che la sua cronologia sia indipendente (vuota o diversa).

- [ ] **Step 7: Smoke test manuale (macOS/PTY)** *(se sviluppi su mac)*

```bash
cargo run --no-default-features -- new "history-test-mac"
```

Stesso scenario. La shell di default su macOS è zsh: verifica che i comandi vengano comunque registrati (zsh hook). Se non vedi nulla, controlla che `$SHELL` sia effettivamente `/bin/zsh`.

- [ ] **Step 8: Final cleanup**

Niente file da committare se gli smoke test sono andati. Se hai aggiustamenti, committa con un messaggio descrittivo dedicato (`fix: …`).

---

## Riepilogo file modificati

| File | Task |
|---|---|
| `crates/tp-core/src/workspace.rs` | T1 |
| `crates/tp-core/src/template.rs` | T1 |
| `crates/tp-gui/src/workspace_view.rs` | T1 |
| `crates/tp-db/src/commands.rs` | T2, T9 |
| `crates/tp-db/src/lib.rs` | T8 |
| `crates/tp-db/src/schema.rs` | T9 |
| `crates/tp-gui/src/panels/terminal/shell_bootstrap.rs` | T3 |
| `crates/tp-gui/Cargo.toml` | T4 |
| `crates/tp-gui/src/backend_factory.rs` | T4 |
| `crates/tp-gui/src/panels/registry.rs` | T4 |
| `crates/tp-gui/src/panels/terminal/mod.rs` | T4, T8, T9 |
| `crates/tp-gui/src/panels/terminal/vte_backend.rs` | T4, T5, T9 |
| `crates/tp-gui/src/panels/terminal/pty_backend.rs` | T4, T6, T9 |
| `crates/tp-gui/src/panels/mod.rs` | T8 |
| `crates/tp-gui/src/panel_host.rs` | T7, T8 |
| `crates/tp-gui/src/dialogs/command_history.rs` | T8 (new) |
| `crates/tp-gui/src/dialogs/mod.rs` | T8 |
| `crates/tp-gui/src/theme.rs` | T8 |
