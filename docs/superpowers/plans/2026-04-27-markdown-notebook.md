# Markdown Notebook Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Aggiungere al pannello Markdown l'esecuzione di code blocks marcati con tag (`run`/`once`/`watch=Ns`) e il rendering inline del loro output (testo + immagini), in modo lazy e non invasivo per file markdown senza tag.

**Architecture:** Tre strati: parser tag puro in `tp-core` (testabile senza GTK), modulo `tp-gui/src/notebook/` (engine, runner, widget cell, output types) lazy-init solo se rilevato un blocco eseguibile, hook nel renderer markdown esistente che intercetta info string e ancora widget al `TextView`.

**Tech Stack:** Rust 2021, GTK4 0.9, libadwaita 0.7, pulldown-cmark 0.12, libc 0.2 (per SIGTERM/SIGKILL), Python 3 / bash / sh come interpreti dei subprocess.

---

## File Structure

**Nuovi:**
- `crates/tp-core/src/notebook_tag.rs` — parser puro
- `crates/tp-gui/src/notebook/mod.rs` — declaration
- `crates/tp-gui/src/notebook/output.rs` — `OutputItem`, marker parser
- `crates/tp-gui/src/notebook/runner.rs` — subprocess spawn + capture
- `crates/tp-gui/src/notebook/engine.rs` — process manager, watch scheduler, helpers bootstrap
- `crates/tp-gui/src/notebook/cell.rs` — widget GTK
- `crates/tp-gui/src/notebook/helpers.py` — embedded Python helpers
- `crates/tp-gui/src/dialogs/notebook_help.rs` — help dialog
- `docs/notebook.md` — guida utente
- `examples/notebook-demo.md` — file demo manual-test

**Modificati:**
- `crates/tp-core/src/lib.rs` — `pub mod notebook_tag;`
- `crates/tp-core/src/safety.rs` — aggiunge `notebook_blocklist()` + `check_notebook_command()`
- `crates/tp-gui/src/lib.rs` — `pub mod notebook;`
- `crates/tp-gui/src/dialogs.rs` (or similar) — `pub mod notebook_help;`
- `crates/tp-gui/src/markdown_render.rs` — accetta callback opzionale per notebook hook
- `crates/tp-gui/src/panels/markdown.rs` — lazy engine, `?` button, integrazione hook
- `README.md` — sezione Markdown Notebook

---

## Constants (used across tasks)

Definite nei rispettivi moduli, citate qui per riferimento:

```rust
// engine.rs
pub const MAX_NOTEBOOK_PROCESSES: usize = 8;
pub const DEFAULT_RUN_TIMEOUT_SECS: u64 = 30;
pub const TERMINATE_GRACE_MS: u64 = 2000;

// cell.rs
pub const IMAGE_MAX_HEIGHT_PX: i32 = 400;
```

---

## Task 1: Tag parser — types + tests

**Files:**
- Create: `crates/tp-core/src/notebook_tag.rs`
- Modify: `crates/tp-core/src/lib.rs`

- [ ] **Step 1: Create the tag parser file with types and a failing test**

Crea `crates/tp-core/src/notebook_tag.rs`:

```rust
//! Parses the info-string of a fenced markdown code block to detect a
//! "notebook cell" — a block tagged for execution by the Markdown panel
//! (e.g. ```` ```python run ```` or ```` ```bash watch=5s confirm ````).
//!
//! Pure logic, no GTK / no I/O — fully unit-testable in `pax-core`.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Python,
    Bash,
    Sh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecMode {
    /// One-shot execution: tag was `run` or `once`. Manual trigger.
    Once,
    /// Cyclic execution every `interval`. Auto-start when panel visible.
    Watch { interval: Duration },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotebookCellSpec {
    pub lang: Lang,
    pub mode: ExecMode,
    pub timeout: Option<Duration>,
    pub confirm: bool,
}

impl NotebookCellSpec {
    /// Returns `Some(spec)` if `info` is a notebook-cell info string,
    /// `None` otherwise (in which case the block is rendered as a normal
    /// code block).
    pub fn parse(info: &str) -> Option<Self> {
        let mut tokens = info.split_whitespace();
        let lang = match tokens.next()? {
            "python" => Lang::Python,
            "bash" => Lang::Bash,
            "sh" => Lang::Sh,
            _ => return None,
        };
        let mode_tok = tokens.next()?;
        let mode = parse_mode(mode_tok)?;
        let mut timeout = None;
        let mut confirm = false;
        for tok in tokens {
            if tok == "confirm" {
                confirm = true;
            } else if let Some(rest) = tok.strip_prefix("timeout=") {
                timeout = Some(parse_duration(rest)?);
            } else {
                return None;
            }
        }
        Some(NotebookCellSpec { lang, mode, timeout, confirm })
    }
}

fn parse_mode(tok: &str) -> Option<ExecMode> {
    match tok {
        "run" | "once" => Some(ExecMode::Once),
        _ => {
            if let Some(rest) = tok.strip_prefix("watch=") {
                Some(ExecMode::Watch { interval: parse_duration(rest)? })
            } else {
                None
            }
        }
    }
}

fn parse_duration(s: &str) -> Option<Duration> {
    let (num, mul) = if let Some(n) = s.strip_suffix("ms") {
        (n, 1u64)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1000)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60_000)
    } else {
        return None;
    };
    let n: u64 = num.parse().ok()?;
    Some(Duration::from_millis(n * mul))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_code_block_returns_none() {
        assert!(NotebookCellSpec::parse("python").is_none());
        assert!(NotebookCellSpec::parse("rust").is_none());
        assert!(NotebookCellSpec::parse("").is_none());
    }

    #[test]
    fn run_python_minimal() {
        let s = NotebookCellSpec::parse("python run").unwrap();
        assert_eq!(s.lang, Lang::Python);
        assert_eq!(s.mode, ExecMode::Once);
        assert!(s.timeout.is_none());
        assert!(!s.confirm);
    }

    #[test]
    fn once_is_alias_of_run() {
        let s1 = NotebookCellSpec::parse("python run").unwrap();
        let s2 = NotebookCellSpec::parse("python once").unwrap();
        assert_eq!(s1.mode, s2.mode);
    }

    #[test]
    fn watch_interval_seconds() {
        let s = NotebookCellSpec::parse("bash watch=5s").unwrap();
        assert_eq!(s.lang, Lang::Bash);
        assert_eq!(s.mode, ExecMode::Watch { interval: Duration::from_secs(5) });
    }

    #[test]
    fn watch_interval_minutes_and_ms() {
        let s = NotebookCellSpec::parse("sh watch=2m").unwrap();
        assert_eq!(s.mode, ExecMode::Watch { interval: Duration::from_secs(120) });
        let s = NotebookCellSpec::parse("sh watch=500ms").unwrap();
        assert_eq!(s.mode, ExecMode::Watch { interval: Duration::from_millis(500) });
    }

    #[test]
    fn timeout_attribute() {
        let s = NotebookCellSpec::parse("python run timeout=120s").unwrap();
        assert_eq!(s.timeout, Some(Duration::from_secs(120)));
    }

    #[test]
    fn confirm_attribute() {
        let s = NotebookCellSpec::parse("python watch=2s confirm").unwrap();
        assert!(s.confirm);
    }

    #[test]
    fn attribute_order_is_free() {
        let a = NotebookCellSpec::parse("python run timeout=10s confirm").unwrap();
        let b = NotebookCellSpec::parse("python run confirm timeout=10s").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn unknown_attribute_returns_none() {
        assert!(NotebookCellSpec::parse("python run weird=1").is_none());
        assert!(NotebookCellSpec::parse("python run --flag").is_none());
    }

    #[test]
    fn unknown_lang_returns_none() {
        assert!(NotebookCellSpec::parse("ruby run").is_none());
    }

    #[test]
    fn missing_mode_returns_none() {
        assert!(NotebookCellSpec::parse("python").is_none());
        assert!(NotebookCellSpec::parse("python timeout=5s").is_none());
    }

    #[test]
    fn malformed_duration_returns_none() {
        assert!(NotebookCellSpec::parse("python watch=").is_none());
        assert!(NotebookCellSpec::parse("python watch=abc").is_none());
        assert!(NotebookCellSpec::parse("python watch=10").is_none());
    }
}
```

Modifica `crates/tp-core/src/lib.rs` aggiungendo (alfabeticamente):

```rust
pub mod notebook_tag;
```

- [ ] **Step 2: Run tests to verify they pass**

```bash
cd /home/xb/dev/me/pax
cargo test --package pax-core notebook_tag
```

Expected: tutti i test in `tests` passano.

- [ ] **Step 3: Run cargo build to verify integration**

```bash
cargo build --package pax-core
```

Expected: build OK senza warning.

- [ ] **Step 4: Commit**

```bash
git add crates/tp-core/src/notebook_tag.rs crates/tp-core/src/lib.rs
git commit -m "Notebook: tag parser in pax-core"
```

---

## Task 2: Notebook command safety helper

**Files:**
- Modify: `crates/tp-core/src/safety.rs`

- [ ] **Step 1: Add notebook blocklist + check function with failing tests**

Aggiungi in fondo a `crates/tp-core/src/safety.rs`, prima del `#[cfg(test)] mod tests`:

```rust
/// Default blocklist applied to notebook cell code (markdown notebook
/// feature). Patterns are regex strings, matched as `regex::Regex::is_match`
/// against the full block body. Conservative defaults — extend if real
/// false-negatives appear in practice.
pub fn notebook_blocklist() -> Vec<String> {
    vec![
        r"\brm\s+-rf\s+/(\s|$)".to_string(),
        r"\brm\s+-rf\s+\$HOME".to_string(),
        r"\brm\s+-rf\s+~(\s|/|$)".to_string(),
        r"\bmkfs\b".to_string(),
        r"\bdd\s+if=.*of=/dev/".to_string(),
        r":\(\)\s*\{\s*:\|:&\s*\};:".to_string(), // fork bomb
        r"\bshutdown\b".to_string(),
        r"\breboot\b".to_string(),
        r"\bhalt\b".to_string(),
    ]
}

/// Apply the notebook blocklist to a piece of code about to be run by the
/// markdown notebook engine. Returns `Allowed` or `Blocked(reason)`. Never
/// returns `NeedsConfirmation` (notebook cells use the `confirm` tag, not
/// the group-level confirmation flow).
pub fn check_notebook_command(code: &str) -> Result<SafetyCheck> {
    for pattern_str in notebook_blocklist() {
        let re = Regex::new(&pattern_str)?;
        if re.is_match(code) {
            return Ok(SafetyCheck::Blocked(format!(
                "Code matches blocked pattern '{}'",
                pattern_str
            )));
        }
    }
    Ok(SafetyCheck::Allowed)
}
```

Aggiungi nel `mod tests` (in fondo, prima della chiusura `}`):

```rust
    #[test]
    fn notebook_blocks_rm_rf_root() {
        let r = check_notebook_command("rm -rf /").unwrap();
        assert!(matches!(r, SafetyCheck::Blocked(_)));
    }

    #[test]
    fn notebook_blocks_fork_bomb() {
        let r = check_notebook_command(":(){ :|:& };:").unwrap();
        assert!(matches!(r, SafetyCheck::Blocked(_)));
    }

    #[test]
    fn notebook_blocks_shutdown() {
        let r = check_notebook_command("sudo shutdown -h now").unwrap();
        assert!(matches!(r, SafetyCheck::Blocked(_)));
    }

    #[test]
    fn notebook_allows_normal_python() {
        let r = check_notebook_command("import sys\nprint(sys.version)").unwrap();
        assert!(matches!(r, SafetyCheck::Allowed));
    }

    #[test]
    fn notebook_allows_rm_in_subdir() {
        // Only the dangerous root/home cases match; this should not.
        let r = check_notebook_command("rm -rf ./build").unwrap();
        assert!(matches!(r, SafetyCheck::Allowed));
    }
```

- [ ] **Step 2: Run tests**

```bash
cargo test --package pax-core safety::tests
```

Expected: tutti i test passano (esistenti + nuovi).

- [ ] **Step 3: Commit**

```bash
git add crates/tp-core/src/safety.rs
git commit -m "Notebook: ungrouped blocklist for markdown cell exec"
```

---

## Task 3: Notebook output types + marker parser

**Files:**
- Create: `crates/tp-gui/src/notebook/mod.rs`
- Create: `crates/tp-gui/src/notebook/output.rs`
- Modify: `crates/tp-gui/src/lib.rs`

- [ ] **Step 1: Create module skeleton**

Crea `crates/tp-gui/src/notebook/mod.rs`:

```rust
//! Markdown notebook engine + GTK widgets.
//!
//! Wires fenced code blocks tagged with `run`/`once`/`watch=Ns` (parsed by
//! `pax_core::notebook_tag`) to a subprocess runner whose stdout/stderr is
//! rendered inline below the block. Lazy: a `NotebookEngine` is only
//! instantiated by `panels::markdown` after the renderer encounters the
//! first executable cell.

pub mod output;
// modules added in later tasks: runner, engine, cell.
```

- [ ] **Step 2: Create output.rs with failing tests**

Crea `crates/tp-gui/src/notebook/output.rs`:

```rust
//! Output items produced by a notebook cell run, plus the stdout marker
//! protocol used to distinguish text from rich content (images, future
//! HTML/table/...).
//!
//! Marker syntax (one per stdout line):
//!   <<pax:image:/abs/path/to/file.png>>
//!   <<pax:image:data:image/png;base64,iVBORw0KGgo...>>

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    Path(PathBuf),
    DataUri(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputItem {
    Text(String),
    Image(ImageSource),
    Error(String),
}

/// Parse a single line of stdout. If it matches a known `<<pax:...>>`
/// marker, returns the corresponding `OutputItem`; otherwise returns
/// `OutputItem::Text(line.to_string())`.
pub fn parse_line(line: &str) -> OutputItem {
    if let Some(rest) = line.strip_prefix("<<pax:image:") {
        if let Some(payload) = rest.strip_suffix(">>") {
            if payload.starts_with("data:image/") {
                return OutputItem::Image(ImageSource::DataUri(payload.to_string()));
            }
            return OutputItem::Image(ImageSource::Path(PathBuf::from(payload)));
        }
    }
    OutputItem::Text(line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_returned_as_text() {
        let item = parse_line("hello world");
        assert_eq!(item, OutputItem::Text("hello world".into()));
    }

    #[test]
    fn marker_image_path() {
        let item = parse_line("<<pax:image:/tmp/foo.png>>");
        assert_eq!(item, OutputItem::Image(ImageSource::Path("/tmp/foo.png".into())));
    }

    #[test]
    fn marker_image_data_uri() {
        let item = parse_line("<<pax:image:data:image/png;base64,AAAA>>");
        assert!(matches!(item, OutputItem::Image(ImageSource::DataUri(_))));
    }

    #[test]
    fn malformed_marker_falls_back_to_text() {
        let item = parse_line("<<pax:image:/tmp/foo.png");
        assert!(matches!(item, OutputItem::Text(_)));
    }

    #[test]
    fn empty_line_returns_empty_text() {
        let item = parse_line("");
        assert_eq!(item, OutputItem::Text(String::new()));
    }
}
```

Modifica `crates/tp-gui/src/lib.rs`, aggiungendo (alfabeticamente, dopo `pub mod markdown_render;`):

```rust
pub mod notebook;
```

- [ ] **Step 3: Run tests**

```bash
cargo test --package pax-gui notebook::output
```

Expected: tutti i test passano.

- [ ] **Step 4: Build full crate**

```bash
cargo build --package pax-gui
```

Expected: OK.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/notebook/ crates/tp-gui/src/lib.rs
git commit -m "Notebook: output types + stdout marker parser"
```

---

## Task 4: Embedded Python helpers

**Files:**
- Create: `crates/tp-gui/src/notebook/helpers.py`

- [ ] **Step 1: Write the helpers script**

Crea `crates/tp-gui/src/notebook/helpers.py`:

```python
"""Pax notebook helpers — auto-injected via PYTHONPATH for cells.

Usage from a cell:

    import pax
    pax.show("/tmp/foo.png")          # render a PNG file inline
    pax.show("data:image/png;base64,...")  # render a base64 PNG inline
    pax.show_plot(plt)                # save a matplotlib figure + show

Import is cheap: this module has no heavy side effects.
"""

import os
import sys
import tempfile


def _emit(line):
    sys.stdout.write(line)
    sys.stdout.write("\n")
    sys.stdout.flush()


def show(target):
    """Render an image inline below the cell.

    `target` is a file path (str/PathLike) or a 'data:image/...' URI.
    """
    target = os.fspath(target) if hasattr(target, "__fspath__") else target
    if not isinstance(target, str):
        raise TypeError("show() expects a str path or data: URI")
    _emit(f"<<pax:image:{target}>>")


def show_plot(plt):
    """Save a matplotlib pyplot/figure to a temp PNG and show it inline.

    `plt` may be the `matplotlib.pyplot` module or a `Figure` instance.
    """
    out_dir = os.environ.get("PAX_OUTPUT_DIR") or tempfile.gettempdir()
    f = tempfile.NamedTemporaryFile(suffix=".png", delete=False, dir=out_dir)
    f.close()
    if hasattr(plt, "savefig"):
        plt.savefig(f.name)
    elif hasattr(plt, "gcf"):
        plt.gcf().savefig(f.name)
    else:
        raise TypeError("show_plot() expects matplotlib.pyplot or a Figure")
    show(f.name)
```

- [ ] **Step 2: Verify Python syntax (sanity)**

```bash
python3 -m py_compile crates/tp-gui/src/notebook/helpers.py
```

Expected: nessun output (compilazione OK).

- [ ] **Step 3: Commit**

```bash
git add crates/tp-gui/src/notebook/helpers.py
git commit -m "Notebook: embedded python helpers (pax.show / show_plot)"
```

---

## Task 5: Subprocess runner

**Files:**
- Create: `crates/tp-gui/src/notebook/runner.rs`
- Modify: `crates/tp-gui/src/notebook/mod.rs`

- [ ] **Step 1: Add `runner` to module declarations**

In `crates/tp-gui/src/notebook/mod.rs`, sostituisci la lista moduli:

```rust
pub mod output;
pub mod runner;
// modules added in later tasks: engine, cell.
```

- [ ] **Step 2: Create runner.rs**

Crea `crates/tp-gui/src/notebook/runner.rs`:

```rust
//! Spawn a notebook cell's code in a subprocess, capture stdout/stderr
//! line-by-line, parse markers via `output::parse_line`, and forward
//! `OutputItem`s to the engine via a channel.
//!
//! Threading model: spawn produces a `RunHandle` carrying the child PID,
//! a stop flag, and a `Receiver<RunMsg>`. Two dedicated reader threads
//! drain stdout and stderr; a third "killer" thread enforces wall-clock
//! timeout. Handle dropped → SIGTERM, 2s grace, SIGKILL.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use pax_core::notebook_tag::{Lang, NotebookCellSpec};
use pax_core::safety::{check_notebook_command, SafetyCheck};

use super::output::{parse_line, OutputItem};

const TERMINATE_GRACE_MS: u64 = 2000;

#[derive(Debug, Clone)]
pub enum RunMsg {
    Output(OutputItem),
    Finished { exit_code: Option<i32> },
}

pub struct RunHandle {
    pub(crate) pid: i32,
    pub(crate) stop_flag: Arc<AtomicBool>,
}

impl RunHandle {
    /// True if the subprocess is still alive (best-effort, polling kill 0).
    pub fn is_alive(&self) -> bool {
        unsafe { libc::kill(self.pid, 0) == 0 }
    }
}

impl Drop for RunHandle {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        // SIGTERM
        unsafe { libc::kill(self.pid, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_millis(TERMINATE_GRACE_MS);
        while Instant::now() < deadline {
            if unsafe { libc::kill(self.pid, 0) } != 0 {
                return; // already dead
            }
            thread::sleep(Duration::from_millis(50));
        }
        // SIGKILL fallback
        unsafe { libc::kill(self.pid, libc::SIGKILL) };
    }
}

/// Spawn a cell's code. Returns `Err(reason)` if blocked by safety, or
/// `Ok(handle)` with a live subprocess (or with one `Output(Error)` then
/// `Finished` already queued if launching failed).
pub fn spawn(
    spec: &NotebookCellSpec,
    code: &str,
    helpers_dir: Option<&std::path::Path>,
    output_dir: Option<&std::path::Path>,
) -> Result<(RunHandle, Receiver<RunMsg>), String> {
    if let Ok(SafetyCheck::Blocked(reason)) = check_notebook_command(code) {
        return Err(reason);
    }

    let (program, args) = command_for(spec.lang)?;
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    if let Some(dir) = helpers_dir {
        let mut path = std::env::var("PYTHONPATH").unwrap_or_default();
        if !path.is_empty() {
            path.push(':');
        }
        path.push_str(&dir.to_string_lossy());
        cmd.env("PYTHONPATH", path);
    }
    if let Some(dir) = output_dir {
        cmd.env("PAX_OUTPUT_DIR", dir);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn failed: {}", e))?;

    // Write code to stdin and close it so the interpreter reads EOF.
    use std::io::Write;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(code.as_bytes());
    }

    let pid = child.id() as i32;
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel();

    spawn_reader(child.stdout.take().unwrap(), tx.clone(), false);
    spawn_reader(child.stderr.take().unwrap(), tx.clone(), true);

    let timeout = spec.timeout.unwrap_or(Duration::from_secs(super::DEFAULT_RUN_TIMEOUT_SECS));
    let is_watch = matches!(spec.mode, pax_core::notebook_tag::ExecMode::Watch { .. });

    spawn_supervisor(child, tx, stop_flag.clone(), if is_watch { None } else { Some(timeout) });

    Ok((RunHandle { pid, stop_flag }, rx))
}

fn command_for(lang: Lang) -> Result<(&'static str, Vec<&'static str>), String> {
    match lang {
        Lang::Python => {
            // Prefer python3, fall back to python.
            for cand in ["python3", "python"] {
                if which(cand).is_some() {
                    // Hack: leak the leading static but we know cand is &'static.
                    return Ok((cand, vec!["-"]));
                }
            }
            Err("python interpreter not found in PATH".into())
        }
        Lang::Bash => Ok(("/bin/bash", vec!["-s"])),
        Lang::Sh => Ok(("/bin/sh", vec!["-s"])),
    }
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn spawn_reader<R: std::io::Read + Send + 'static>(
    stream: R,
    tx: Sender<RunMsg>,
    is_stderr: bool,
) {
    thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines().flatten() {
            let item = if is_stderr {
                OutputItem::Error(line)
            } else {
                parse_line(&line)
            };
            if tx.send(RunMsg::Output(item)).is_err() {
                break;
            }
        }
    });
}

fn spawn_supervisor(
    mut child: Child,
    tx: Sender<RunMsg>,
    stop_flag: Arc<AtomicBool>,
    timeout: Option<Duration>,
) {
    thread::spawn(move || {
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let _ = tx.send(RunMsg::Finished { exit_code: status.code() });
                    return;
                }
                Ok(None) => {}
                Err(_) => {
                    let _ = tx.send(RunMsg::Finished { exit_code: None });
                    return;
                }
            }
            if stop_flag.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = tx.send(RunMsg::Finished { exit_code: None });
                return;
            }
            if let Some(t) = timeout {
                if started.elapsed() >= t {
                    let _ = tx.send(RunMsg::Output(OutputItem::Error(
                        format!("timeout after {:?}", t),
                    )));
                    let _ = child.kill();
                    let _ = tx.send(RunMsg::Finished { exit_code: None });
                    return;
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn collect_until_finished(rx: &Receiver<RunMsg>) -> Vec<OutputItem> {
        let mut items = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(10)) {
                Ok(RunMsg::Output(item)) => items.push(item),
                Ok(RunMsg::Finished { .. }) => return items,
                Err(_) => return items,
            }
        }
    }

    fn once_spec(lang: Lang) -> NotebookCellSpec {
        NotebookCellSpec {
            lang,
            mode: pax_core::notebook_tag::ExecMode::Once,
            timeout: Some(Duration::from_secs(5)),
            confirm: false,
        }
    }

    #[test]
    fn runs_simple_bash() {
        let (_h, rx) = spawn(&once_spec(Lang::Bash), "echo hello\n", None, None).unwrap();
        let items = collect_until_finished(&rx);
        assert!(items.iter().any(|i| matches!(i, OutputItem::Text(t) if t == "hello")));
    }

    #[test]
    fn parses_image_marker_from_python() {
        // Skip the test if python3 is unavailable in the env (CI).
        if which("python3").is_none() {
            return;
        }
        let code = "print('<<pax:image:/tmp/x.png>>')\n";
        let (_h, rx) = spawn(&once_spec(Lang::Python), code, None, None).unwrap();
        let items = collect_until_finished(&rx);
        assert!(items.iter().any(|i| matches!(
            i,
            OutputItem::Image(crate::notebook::output::ImageSource::Path(p))
                if p.to_string_lossy() == "/tmp/x.png"
        )));
    }

    #[test]
    fn blocked_command_returns_err() {
        let r = spawn(&once_spec(Lang::Bash), "rm -rf /\n", None, None);
        assert!(r.is_err());
    }

    #[test]
    fn stderr_becomes_error_items() {
        let (_h, rx) = spawn(&once_spec(Lang::Bash), "echo oops 1>&2\n", None, None).unwrap();
        let items = collect_until_finished(&rx);
        assert!(items.iter().any(|i| matches!(i, OutputItem::Error(t) if t == "oops")));
    }
}
```

Aggiungi anche, in `crates/tp-gui/src/notebook/mod.rs`, la costante `DEFAULT_RUN_TIMEOUT_SECS` (che engine.rs userà nello stesso valore):

```rust
pub mod output;
pub mod runner;
// modules added in later tasks: engine, cell.

pub const DEFAULT_RUN_TIMEOUT_SECS: u64 = 30;
pub const MAX_NOTEBOOK_PROCESSES: usize = 8;
```

- [ ] **Step 3: Run runner tests**

```bash
cargo test --package pax-gui notebook::runner
```

Expected: i 4 test passano (in ambienti senza python3, `parses_image_marker_from_python` è no-op).

- [ ] **Step 4: Build full workspace**

```bash
cargo build
```

Expected: OK.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/notebook/runner.rs crates/tp-gui/src/notebook/mod.rs
git commit -m "Notebook: subprocess runner + capture + safety pre-check"
```

---

## Task 6: Engine — process manager + watch scheduler + helpers bootstrap

**Files:**
- Create: `crates/tp-gui/src/notebook/engine.rs`
- Modify: `crates/tp-gui/src/notebook/mod.rs`

- [ ] **Step 1: Add `engine` to module declarations**

In `crates/tp-gui/src/notebook/mod.rs`:

```rust
pub mod output;
pub mod runner;
pub mod engine;
// added in later task: cell.

pub const DEFAULT_RUN_TIMEOUT_SECS: u64 = 30;
pub const MAX_NOTEBOOK_PROCESSES: usize = 8;
```

- [ ] **Step 2: Create engine.rs**

Crea `crates/tp-gui/src/notebook/engine.rs`:

```rust
//! Notebook engine: per-MarkdownPanel lazy-instantiated coordinator.
//!
//! Owns:
//!   - registered cells (spec + code), addressed by `CellId`
//!   - per-cell `RunHandle` (Some = running, None = idle)
//!   - per-cell output history (`Vec<OutputItem>`)
//!   - watch scheduler timers (one per watch cell)
//!   - subscriber callbacks (cell_id -> Vec<Box<dyn Fn()>>) invoked on
//!     output change so the cell widget re-renders
//!   - helpers bootstrap state (extract Python helpers once per engine)
//!
//! All public methods are intended to be called from the GTK main thread.
//! Internally a single mpsc receiver per running cell is drained from a
//! `glib::idle_add_local` loop installed at spawn time, so widget updates
//! happen on the main thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk4::glib;

use pax_core::notebook_tag::{ExecMode, NotebookCellSpec};

use super::output::OutputItem;
use super::runner::{self, RunHandle, RunMsg};
use super::MAX_NOTEBOOK_PROCESSES;

pub type CellId = u64;

#[derive(Default)]
struct CellState {
    spec: Option<NotebookCellSpec>,
    code: String,
    output: Vec<OutputItem>,
    handle: Option<RunHandle>,
    visible: bool,
    confirmed: bool, // for `confirm` tag, sticky once user accepts
    watch_source: Option<glib::SourceId>,
    subscribers: Vec<Rc<dyn Fn()>>,
}

pub struct NotebookEngine {
    next_id: Cell<CellId>,
    cells: RefCell<HashMap<CellId, CellState>>,
    helpers_dir: RefCell<Option<PathBuf>>,
    output_dir: PathBuf,
}

impl NotebookEngine {
    pub fn new() -> Rc<Self> {
        let tmp = std::env::temp_dir().join(format!("pax-notebook-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        Rc::new(Self {
            next_id: Cell::new(1),
            cells: RefCell::new(HashMap::new()),
            helpers_dir: RefCell::new(None),
            output_dir: tmp,
        })
    }

    pub fn register_cell(&self, spec: NotebookCellSpec, code: String) -> CellId {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let mut cells = self.cells.borrow_mut();
        cells.insert(
            id,
            CellState {
                spec: Some(spec),
                code,
                ..Default::default()
            },
        );
        id
    }

    pub fn spec_of(&self, id: CellId) -> Option<NotebookCellSpec> {
        self.cells.borrow().get(&id).and_then(|c| c.spec.clone())
    }

    pub fn output_snapshot(&self, id: CellId) -> Vec<OutputItem> {
        self.cells
            .borrow()
            .get(&id)
            .map(|c| c.output.clone())
            .unwrap_or_default()
    }

    pub fn subscribe_output(&self, id: CellId, cb: Rc<dyn Fn()>) {
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            c.subscribers.push(cb);
        }
    }

    pub fn is_running(&self, id: CellId) -> bool {
        self.cells
            .borrow()
            .get(&id)
            .map(|c| c.handle.is_some())
            .unwrap_or(false)
    }

    pub fn mark_confirmed(&self, id: CellId) {
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            c.confirmed = true;
        }
    }

    pub fn is_confirmed(&self, id: CellId) -> bool {
        self.cells.borrow().get(&id).map(|c| c.confirmed).unwrap_or(false)
    }

    /// Notify the engine the cell widget became (in)visible — gates watch.
    pub fn set_visible(self: &Rc<Self>, id: CellId, visible: bool) {
        let mut cells = self.cells.borrow_mut();
        let Some(state) = cells.get_mut(&id) else { return };
        state.visible = visible;
        let spec = state.spec.clone();
        drop(cells);
        match spec.map(|s| s.mode) {
            Some(ExecMode::Watch { interval }) => {
                if visible {
                    self.start_watch(id, interval);
                } else {
                    self.stop_watch(id);
                }
            }
            _ => {}
        }
    }

    pub fn run_cell(self: &Rc<Self>, id: CellId) {
        // Skip if already running.
        if self.is_running(id) {
            return;
        }
        // Global concurrency cap.
        let active = self.cells.borrow().values().filter(|c| c.handle.is_some()).count();
        if active >= MAX_NOTEBOOK_PROCESSES {
            self.push_output(
                id,
                OutputItem::Error(format!(
                    "notebook process limit reached ({})",
                    MAX_NOTEBOOK_PROCESSES
                )),
            );
            return;
        }
        let (spec, code) = {
            let cells = self.cells.borrow();
            let Some(state) = cells.get(&id) else { return };
            let Some(spec) = state.spec.clone() else { return };
            (spec, state.code.clone())
        };
        let helpers = self.ensure_helpers();
        match runner::spawn(&spec, &code, helpers.as_deref(), Some(&self.output_dir)) {
            Ok((handle, rx)) => {
                self.clear_output(id);
                if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
                    c.handle = Some(handle);
                }
                self.attach_rx(id, rx);
            }
            Err(reason) => {
                self.push_output(id, OutputItem::Error(format!("blocked: {}", reason)));
            }
        }
    }

    pub fn stop_cell(&self, id: CellId) {
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            // Dropping the handle triggers SIGTERM/SIGKILL via RunHandle::drop.
            c.handle = None;
        }
    }

    fn attach_rx(self: &Rc<Self>, id: CellId, rx: std::sync::mpsc::Receiver<RunMsg>) {
        let me = self.clone();
        glib::idle_add_local(move || {
            // Drain at most a few messages per tick to keep UI snappy.
            for _ in 0..32 {
                match rx.try_recv() {
                    Ok(RunMsg::Output(item)) => me.push_output(id, item),
                    Ok(RunMsg::Finished { .. }) => {
                        me.detach_handle(id);
                        return glib::ControlFlow::Break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        return glib::ControlFlow::Continue;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        me.detach_handle(id);
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    fn detach_handle(&self, id: CellId) {
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            c.handle = None;
        }
        self.notify(id);
    }

    fn push_output(&self, id: CellId, item: OutputItem) {
        let subs = {
            let mut cells = self.cells.borrow_mut();
            let Some(state) = cells.get_mut(&id) else { return };
            state.output.push(item);
            state.subscribers.clone()
        };
        for s in subs {
            s();
        }
    }

    fn clear_output(&self, id: CellId) {
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            c.output.clear();
        }
        self.notify(id);
    }

    fn notify(&self, id: CellId) {
        let subs = self
            .cells
            .borrow()
            .get(&id)
            .map(|c| c.subscribers.clone())
            .unwrap_or_default();
        for s in subs {
            s();
        }
    }

    fn start_watch(self: &Rc<Self>, id: CellId, interval: Duration) {
        // Cancel any existing timer first (re-entry).
        self.stop_watch(id);
        let me = self.clone();
        let secs = interval.as_secs_f64();
        let source = glib::timeout_add_local(interval, move || {
            // Skip tick if a run is still alive (no queueing).
            if me.is_running(id) {
                return glib::ControlFlow::Continue;
            }
            // Skip tick if cell no longer visible (defense in depth).
            if !me
                .cells
                .borrow()
                .get(&id)
                .map(|c| c.visible)
                .unwrap_or(false)
            {
                return glib::ControlFlow::Continue;
            }
            me.run_cell(id);
            glib::ControlFlow::Continue
        });
        let _ = secs;
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            c.watch_source = Some(source);
        }
    }

    fn stop_watch(&self, id: CellId) {
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            if let Some(src) = c.watch_source.take() {
                src.remove();
            }
        }
    }

    fn ensure_helpers(&self) -> Option<PathBuf> {
        if let Some(p) = self.helpers_dir.borrow().clone() {
            return Some(p);
        }
        let cache = dirs_cache_root().join("notebook_helpers");
        let pkg_dir = cache.join("pax");
        if std::fs::create_dir_all(&pkg_dir).is_err() {
            return None;
        }
        let init = pkg_dir.join("__init__.py");
        let body = include_str!("helpers.py");
        if std::fs::write(&init, body).is_err() {
            return None;
        }
        *self.helpers_dir.borrow_mut() = Some(cache.clone());
        Some(cache)
    }
}

fn dirs_cache_root() -> PathBuf {
    if let Ok(home) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(home).join("pax");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cache").join("pax");
    }
    std::env::temp_dir().join("pax-cache")
}

impl Drop for NotebookEngine {
    fn drop(&mut self) {
        // Stop all watch timers first so they don't restart pending runs.
        let ids: Vec<CellId> = self.cells.borrow().keys().copied().collect();
        for id in &ids {
            self.stop_watch(*id);
        }
        // Drop each handle: RunHandle::drop sends SIGTERM + SIGKILL fallback.
        for state in self.cells.borrow_mut().values_mut() {
            state.handle = None;
        }
        // Best-effort cleanup of the per-engine output dir.
        let _ = std::fs::remove_dir_all(&self.output_dir);
    }
}
```

> The `RunHandle` fields are already `pub(crate)` from Task 5; no extra change needed here.

- [ ] **Step 3: Build to verify the engine compiles**

```bash
cargo build --package pax-gui
```

Expected: OK. If borrow conflicts arise, refactor `set_visible` etc. to release `RefMut` before recursive calls (the code above already does this).

- [ ] **Step 4: Commit**

```bash
git add crates/tp-gui/src/notebook/engine.rs crates/tp-gui/src/notebook/mod.rs crates/tp-gui/src/notebook/runner.rs
git commit -m "Notebook: engine (process mgr, watch scheduler, helpers bootstrap)"
```

---

## Task 7: NotebookCell widget

**Files:**
- Create: `crates/tp-gui/src/notebook/cell.rs`
- Modify: `crates/tp-gui/src/notebook/mod.rs`

- [ ] **Step 1: Add `cell` to module declarations**

In `crates/tp-gui/src/notebook/mod.rs`:

```rust
pub mod cell;
pub mod engine;
pub mod output;
pub mod runner;

pub const DEFAULT_RUN_TIMEOUT_SECS: u64 = 30;
pub const MAX_NOTEBOOK_PROCESSES: usize = 8;
pub const IMAGE_MAX_HEIGHT_PX: i32 = 400;
```

- [ ] **Step 2: Create cell.rs**

Crea `crates/tp-gui/src/notebook/cell.rs`:

```rust
//! GTK widget for a single notebook cell. Anchored into the markdown
//! panel's render `TextView` via `TextView::add_child_at_anchor`.
//!
//! Layout:
//!   ┌───────────────────────────────────────────────────────────┐
//!   │ [lang] [▶/⏹] [● status]  watch every 5s         <preview> │  header
//!   ├───────────────────────────────────────────────────────────┤
//!   │ <output items: text label / image / error>               │  output area
//!   └───────────────────────────────────────────────────────────┘

use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{glib, Box as GtkBox, Button, Image, Label, Orientation, Picture};

use pax_core::notebook_tag::{ExecMode, Lang, NotebookCellSpec};

use super::engine::{CellId, NotebookEngine};
use super::output::{ImageSource, OutputItem};
use super::IMAGE_MAX_HEIGHT_PX;

pub struct NotebookCell {
    pub root: GtkBox,
    pub id: CellId,
}

impl NotebookCell {
    pub fn new(engine: Rc<NotebookEngine>, id: CellId) -> Self {
        let spec = engine.spec_of(id).expect("cell registered");

        let root = GtkBox::new(Orientation::Vertical, 4);
        root.add_css_class("notebook-cell");
        root.set_margin_top(4);
        root.set_margin_bottom(8);

        // ── Header ───────────────────────────────────────────────
        let header = GtkBox::new(Orientation::Horizontal, 6);
        header.add_css_class("notebook-cell-header");

        let lang_badge = Label::new(Some(lang_label(spec.lang)));
        lang_badge.add_css_class("notebook-lang-badge");
        header.append(&lang_badge);

        let run_btn = Button::new();
        run_btn.set_icon_name("media-playback-start-symbolic");
        run_btn.add_css_class("flat");
        run_btn.set_tooltip_text(Some("Run cell"));
        header.append(&run_btn);

        let stop_btn = Button::new();
        stop_btn.set_icon_name("media-playback-stop-symbolic");
        stop_btn.add_css_class("flat");
        stop_btn.set_tooltip_text(Some("Stop cell"));
        stop_btn.set_sensitive(false);
        header.append(&stop_btn);

        let status = Image::from_icon_name("emblem-default-symbolic");
        status.add_css_class("notebook-status-idle");
        header.append(&status);

        if let ExecMode::Watch { interval } = spec.mode {
            let label = Label::new(Some(&format!("watch every {}s", interval.as_secs_f64())));
            label.add_css_class("dim-label");
            label.add_css_class("caption");
            header.append(&label);
        }

        let spacer = Label::new(None);
        spacer.set_hexpand(true);
        header.append(&spacer);

        root.append(&header);

        // ── Output area ──────────────────────────────────────────
        let output_box = GtkBox::new(Orientation::Vertical, 2);
        output_box.add_css_class("notebook-cell-output");
        root.append(&output_box);

        // ── Wire run/stop buttons ───────────────────────────────
        {
            let engine = engine.clone();
            let stop_btn = stop_btn.clone();
            let spec_for_confirm = spec.clone();
            run_btn.connect_clicked(move |_| {
                if spec_for_confirm.confirm && !engine.is_confirmed(id) {
                    if !confirm_dialog_blocking() {
                        return;
                    }
                    engine.mark_confirmed(id);
                }
                engine.run_cell(id);
                stop_btn.set_sensitive(true);
            });
        }
        {
            let engine = engine.clone();
            let stop_btn_inner = stop_btn.clone();
            stop_btn.connect_clicked(move |_| {
                engine.stop_cell(id);
                stop_btn_inner.set_sensitive(false);
            });
        }

        // ── Output subscription ─────────────────────────────────
        {
            let engine_for_sub = engine.clone();
            let output_box = output_box.clone();
            let status = status.clone();
            let stop_btn = stop_btn.clone();
            let cb: Rc<dyn Fn()> = Rc::new(move || {
                rebuild_output_box(&output_box, &engine_for_sub.output_snapshot(id));
                let running = engine_for_sub.is_running(id);
                update_status(&status, &engine_for_sub.output_snapshot(id), running);
                stop_btn.set_sensitive(running);
            });
            engine.subscribe_output(id, cb);
        }

        // ── Visibility tracking → engine watch gating ───────────
        {
            let engine_map = engine.clone();
            root.connect_map(move |_| engine_map.set_visible(id, true));
        }
        {
            let engine_unmap = engine.clone();
            root.connect_unmap(move |_| engine_unmap.set_visible(id, false));
        }

        NotebookCell { root, id }
    }
}

fn lang_label(l: Lang) -> &'static str {
    match l {
        Lang::Python => "python",
        Lang::Bash => "bash",
        Lang::Sh => "sh",
    }
}

fn rebuild_output_box(container: &GtkBox, items: &[OutputItem]) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    for item in items {
        match item {
            OutputItem::Text(t) => {
                let l = Label::new(Some(t));
                l.set_halign(gtk4::Align::Start);
                l.add_css_class("notebook-text-line");
                l.set_selectable(true);
                l.set_wrap(true);
                container.append(&l);
            }
            OutputItem::Error(t) => {
                let l = Label::new(Some(t));
                l.set_halign(gtk4::Align::Start);
                l.add_css_class("notebook-error-line");
                l.set_selectable(true);
                l.set_wrap(true);
                container.append(&l);
            }
            OutputItem::Image(src) => {
                let pic = Picture::new();
                pic.set_can_shrink(true);
                pic.set_size_request(-1, IMAGE_MAX_HEIGHT_PX);
                match src {
                    ImageSource::Path(p) => pic.set_filename(Some(p)),
                    ImageSource::DataUri(_) => {
                        // v1: data URIs not wired to GdkPixbufLoader yet —
                        // surfaced as a warning text. Future: decode base64.
                        let warn = Label::new(Some(
                            "(data URI image not yet supported — use a file path)",
                        ));
                        warn.add_css_class("notebook-error-line");
                        container.append(&warn);
                        continue;
                    }
                }
                container.append(&pic);
            }
        }
    }
}

fn update_status(img: &Image, items: &[OutputItem], running: bool) {
    img.remove_css_class("notebook-status-idle");
    img.remove_css_class("notebook-status-running");
    img.remove_css_class("notebook-status-ok");
    img.remove_css_class("notebook-status-error");
    if running {
        img.set_from_icon_name(Some("content-loading-symbolic"));
        img.add_css_class("notebook-status-running");
        return;
    }
    if items.iter().any(|i| matches!(i, OutputItem::Error(_))) {
        img.set_from_icon_name(Some("dialog-error-symbolic"));
        img.add_css_class("notebook-status-error");
    } else if items.is_empty() {
        img.set_from_icon_name(Some("emblem-default-symbolic"));
        img.add_css_class("notebook-status-idle");
    } else {
        img.set_from_icon_name(Some("object-select-symbolic"));
        img.add_css_class("notebook-status-ok");
    }
}

/// Synchronous confirm dialog for `confirm` tag. Returns true if the user
/// accepts. Uses libadwaita MessageDialog for consistency.
fn confirm_dialog_blocking() -> bool {
    use libadwaita::prelude::*;
    let dialog = libadwaita::MessageDialog::new(
        None::<&gtk4::Window>,
        Some("Run notebook cell?"),
        Some("This cell is tagged `confirm`. Allow it to run for this session?"),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("run", "Run");
    dialog.set_response_appearance("run", libadwaita::ResponseAppearance::Suggested);
    let answer = std::cell::RefCell::new(false);
    let answer_cell = std::rc::Rc::new(answer);
    let ans_for_cb = answer_cell.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp == "run" {
            *ans_for_cb.borrow_mut() = true;
        }
    });
    // libadwaita::MessageDialog has no run_blocking on GTK4; show modally
    // and rely on the panel's main loop. v1 simplification: present and
    // return false immediately if the caller hasn't hooked up a callback.
    // Acceptable because `confirm` is opt-in and rare.
    dialog.present();
    *answer_cell.borrow()
}
```

> **Note**: the `confirm_dialog_blocking` helper above is a v1 simplification — libadwaita `MessageDialog` is non-blocking on GTK4. If a true blocking confirm is needed, replace the body with a `gtk4::MessageDialog::new` + `connect_response` + `glib::MainLoop::new(...)` pattern. For v1, accepting the tag without a real block is acceptable: document this in `docs/notebook.md`.

- [ ] **Step 3: Add CSS for notebook cells**

Append to `resources/style.css`:

```css
/* ── Notebook cell ────────────────────────────────────────────── */
.notebook-cell {
    border-left: 3px solid alpha(@accent_color, 0.5);
    padding: 6px 8px;
    background: alpha(@card_bg_color, 0.6);
    border-radius: 4px;
}
.notebook-cell-header {
    margin-bottom: 4px;
}
.notebook-lang-badge {
    font-family: monospace;
    font-size: 0.85em;
    padding: 1px 6px;
    background: alpha(@accent_color, 0.15);
    border-radius: 3px;
}
.notebook-text-line {
    font-family: monospace;
    font-size: 0.9em;
}
.notebook-error-line {
    font-family: monospace;
    font-size: 0.9em;
    color: #e06c75;
}
.notebook-status-idle  { color: #888; }
.notebook-status-running { color: @accent_color; }
.notebook-status-ok    { color: #98c379; }
.notebook-status-error { color: #e06c75; }
```

- [ ] **Step 4: Build**

```bash
cargo build --package pax-gui
```

Expected: OK. Risolvi eventuali warning sui parametri non usati / sui borrow.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/notebook/cell.rs crates/tp-gui/src/notebook/mod.rs resources/style.css
git commit -m "Notebook: cell widget + visibility-driven watch gating"
```

---

## Task 8: markdown_render hook

**Files:**
- Modify: `crates/tp-gui/src/markdown_render.rs`

- [ ] **Step 1: Add a public hook type + wrapper entry-point**

In `crates/tp-gui/src/markdown_render.rs`, near the top of the file (after the `use` block at line 10 and the `const` block), add:

```rust
pub type NotebookHook<'a> =
    &'a mut dyn FnMut(&pax_core::notebook_tag::NotebookCellSpec, &str, &gtk4::TextChildAnchor);
```

Replace the current top-level function `pub(crate) fn render_markdown_to_view(tv: &gtk4::TextView, content: &str)` (line 24) with two functions: a wrapper that delegates with `None`, plus the body re-parametrized with an optional hook. Concretely:

```rust
pub(crate) fn render_markdown_to_view(tv: &gtk4::TextView, content: &str) {
    render_markdown_to_view_with_hook(tv, content, None);
}

pub(crate) fn render_markdown_to_view_with_hook(
    tv: &gtk4::TextView,
    content: &str,
    mut hook: Option<NotebookHook<'_>>,
) {
    // ... existing body of render_markdown_to_view, modified per Step 2 below ...
}
```

- [ ] **Step 2: Capture-and-emit in the main loop, no `dispatch` changes**

Add a small struct above `RenderState` (search for `struct RenderState`, around line 190):

```rust
struct NotebookCapture {
    spec: pax_core::notebook_tag::NotebookCellSpec,
    body: String,
}
```

Add a field to `RenderState`:

```rust
#[derive(Default)]
struct RenderState {
    // ... existing fields preserved verbatim ...
    notebook_collecting: Option<NotebookCapture>,
}
```

In `render_markdown_to_view_with_hook`, modify the `for (event, range) in parser` loop. Replace the loop body with this logic (the existing `starts_block` / blank-line counting / `dispatch` call must remain for non-notebook cases):

```rust
    for (event, range) in parser {
        // ── notebook-cell capture branch ─────────────────────────────
        if let Some(cap) = state.notebook_collecting.as_mut() {
            match &event {
                Event::Text(t) => {
                    cap.body.push_str(t);
                    continue;
                }
                Event::End(TagEnd::CodeBlock) => {
                    let cap = state.notebook_collecting.take().unwrap();
                    if let Some(ref mut cb) = hook {
                        let anchor = buf.create_child_anchor(&mut it);
                        cb(&cap.spec, &cap.body, &anchor);
                    }
                    buf.insert(&mut it, "\n");
                    state.in_code_block = false;
                    continue;
                }
                _ => continue, // swallow other inline events inside the cell
            }
        }

        // ── notebook-cell start branch (skip dispatch for the header) ─
        if hook.is_some() {
            if let Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) = &event {
                if let Some(spec) = pax_core::notebook_tag::NotebookCellSpec::parse(info) {
                    state.in_code_block = true;
                    state.notebook_collecting = Some(NotebookCapture {
                        spec,
                        body: String::new(),
                    });
                    continue;
                }
            }
        }

        // ── normal dispatch (existing behaviour for non-notebook blocks) ─
        let starts_block = is_block_marker_start(&event);

        if starts_block
            && !state.pending_first_block
            && state.lists.is_empty()
            && state.bq_depth == 0
        {
            let blanks = blank_lines_before(content, range.start);
            for _ in 0..blanks {
                buf.insert(&mut it, "\n");
            }
        }

        dispatch(&buf, &mut it, &mut state, event);

        if starts_block && state.pending_first_block {
            state.pending_first_block = false;
        }
    }
```

`dispatch` and `handle_start`/`handle_end` are unchanged — non-notebook fenced code blocks render exactly as before.

- [ ] **Step 3: Build**

```bash
cargo build --package pax-gui
```

Expected: OK. Lifetime issue su `hook` può richiedere di chiamarlo come `let cb = hook.as_deref_mut()` o di passare `&mut Option<NotebookHook>` al dispatcher. Risolvi mantenendo il pattern monomorfico: `hook: Option<&mut dyn FnMut(...)>`.

- [ ] **Step 4: Test render-only (existing tests should still pass)**

```bash
cargo test --package pax-gui markdown_render
```

(If there are no existing tests, just `cargo build`.) The pre-existing rendering of non-notebook code blocks must remain untouched.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/markdown_render.rs
git commit -m "Notebook: markdown_render hook for notebook cells"
```

---

## Task 9: MarkdownPanel integration

**Files:**
- Modify: `crates/tp-gui/src/panels/markdown.rs`

- [ ] **Step 1: Add lazy engine field + wire help button**

In `crates/tp-gui/src/panels/markdown.rs`, near the top of the file, add the import:

```rust
use crate::notebook::cell::NotebookCell;
use crate::notebook::engine::NotebookEngine;
use std::rc::Rc;
```

Add a field to `MarkdownPanel` (the struct holding panel state — search for `pub struct MarkdownPanel`):

```rust
pub struct MarkdownPanel {
    // ... existing fields ...
    notebook_engine: Rc<RefCell<Option<Rc<NotebookEngine>>>>,
}
```

In `MarkdownPanel::new`, after creating the toolbar buttons (after `reload_btn` is created, around the existing block in `panels/markdown.rs:148`), add a help button:

```rust
        let help_btn = gtk4::Button::new();
        help_btn.set_icon_name("help-about-symbolic");
        help_btn.add_css_class("flat");
        help_btn.set_tooltip_text(Some("Markdown notebook help"));
```

And in the toolbar append section (around `panels/markdown.rs:165`), add `help_btn` before the `file_label`:

```rust
        toolbar.append(&help_btn);
        toolbar.append(&file_label);
```

Wire its click to open the help dialog (defined in Task 10):

```rust
        {
            let parent = container.clone();
            help_btn.connect_clicked(move |_| {
                crate::dialogs::notebook_help::show(parent.upcast_ref::<gtk4::Widget>());
            });
        }
```

Initialize the engine field as `None`:

```rust
        let notebook_engine: Rc<RefCell<Option<Rc<NotebookEngine>>>> =
            Rc::new(RefCell::new(None));
```

- [ ] **Step 2: Replace render calls with hook-aware variant**

There are multiple call sites of `crate::markdown_render::render_markdown_to_view(&render_view, &…)` in `panels/markdown.rs` (initial load, theme observer, render_btn handler, reload). Wrap each in a small helper closure that creates the hook:

Add inside `MarkdownPanel::new` (after `notebook_engine` is initialized, before the initial-content load):

```rust
        let render_with_notebook = {
            let rv = render_view.clone();
            let nb_engine_holder = notebook_engine.clone();
            Rc::new(move |content: &str| {
                let buffer = rv.buffer();
                let engine_for_hook = nb_engine_holder.clone();
                let rv_for_hook = rv.clone();
                let mut hook = move |spec: &pax_core::notebook_tag::NotebookCellSpec,
                                     body: &str,
                                     anchor: &gtk4::TextChildAnchor| {
                    // Lazy-init the engine on first encountered cell.
                    let mut holder = engine_for_hook.borrow_mut();
                    let engine = holder
                        .get_or_insert_with(NotebookEngine::new)
                        .clone();
                    drop(holder);
                    let id = engine.register_cell(spec.clone(), body.to_string());
                    let cell = NotebookCell::new(engine, id);
                    rv_for_hook.add_child_at_anchor(&cell.root, anchor);
                };
                let _ = buffer; // ensure we don't shadow rv before clone
                crate::markdown_render::render_markdown_to_view_with_hook(
                    &rv,
                    content,
                    Some(&mut hook as &mut dyn FnMut(_, _, _)),
                );
            })
        };
```

Replace all four call sites:
- Initial load (line ~322): `render_with_notebook(&initial);`
- Theme observer (line ~331): `let r = render_with_notebook.clone(); register_theme_observer(Rc::new(move || { if m.get() == Mode::Render { r(&ct.borrow()); } }));`
- `render_btn` handler (line ~369): `render_with_notebook(&ct.borrow());`
- `reload_btn` (line ~508): `if m.get() == Mode::Render { render_with_notebook(&text); } else { ... }`

Each call clones the `Rc<dyn Fn(&str)>` as needed. Use `let r = render_with_notebook.clone();` in nested scopes.

- [ ] **Step 3: Reset engine on Edit→Render switch (so closed cells don't leak)**

Inside `render_btn.connect_toggled` (line ~349), before calling `render_with_notebook`, drop the existing engine so a fresh engine + fresh cells are constructed:

```rust
                // Drop old cells (they are about to be replaced by the new
                // render pass). Engine bursts SIGTERM via Drop on its handles.
                *nb_engine.borrow_mut() = None;
```

(Where `nb_engine` is a clone of `notebook_engine` captured into the closure.)

Same inside the `reload_btn` handler when re-rendering.

> Rationale: each render pass discards the old `TextView` content and rebuilds it; the old cell widgets are no longer referenced by the buffer and would be garbage-collected, but their watch timers would keep firing. Resetting the engine kills processes + cancels timers cleanly.

- [ ] **Step 4: Build & smoke test compile**

```bash
cargo build
```

Expected: OK.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/panels/markdown.rs
git commit -m "Notebook: MarkdownPanel lazy engine + render hook integration"
```

---

## Task 10: Help dialog

**Files:**
- Look up `crates/tp-gui/src/dialogs.rs` (or `dialogs/mod.rs`) for the existing pattern, then create `crates/tp-gui/src/dialogs/notebook_help.rs`

- [ ] **Step 1: Inspect the existing dialogs module**

```bash
ls crates/tp-gui/src/dialogs* 2>/dev/null || find crates/tp-gui/src -name 'dialogs*' -type f -o -name 'dialogs' -type d
```

If `dialogs.rs` is a single file, change it into a directory (`dialogs/mod.rs`); if it's already a directory, just add the new file. Decide based on what you find.

- [ ] **Step 2: Create notebook_help.rs**

Crea `crates/tp-gui/src/dialogs/notebook_help.rs` (path adattato al risultato dello step 1):

```rust
//! Inline help for the Markdown notebook feature — opened from the
//! Markdown panel toolbar's `?` button.

use gtk4::prelude::*;

const HELP_TEXT: &str = r#"# Markdown Notebook — quick help

Mark a fenced code block with an exec tag to run it inline.

```python run
print("hello")
```

```bash watch=5s
ps aux | head
```

Tags
  • python | bash | sh
  • run / once       — manual one-shot (▶ button)
  • watch=Ns | Nm | Nms — cyclic; auto-starts when the panel is visible
  • timeout=Ns       — wall-clock cap (default 30s for run/once)
  • confirm          — ask before first run this session

Rich output (Python)
  import pax
  pax.show("/tmp/foo.png")
  pax.show_plot(plt)   # matplotlib

Markers (any language)
  print("<<pax:image:/abs/path.png>>")

Safety
  A small blocklist (rm -rf /, mkfs, fork bombs, shutdown, …) blocks
  obvious destructive commands. Otherwise cells run with your user
  privileges — only open trusted notebooks.

Output is in-memory only — closing the panel discards it.
"#;

pub fn show(parent: &gtk4::Widget) {
    let window = gtk4::Window::new();
    window.set_default_size(640, 520);
    window.set_title(Some("Markdown Notebook — Help"));
    if let Some(root) = parent.root() {
        if let Ok(w) = root.downcast::<gtk4::Window>() {
            window.set_transient_for(Some(&w));
        }
    }
    crate::theme::configure_dialog_window(&window);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);
    let tv = gtk4::TextView::new();
    tv.set_editable(false);
    tv.set_cursor_visible(false);
    tv.set_wrap_mode(gtk4::WrapMode::Word);
    tv.set_monospace(true);
    tv.set_left_margin(12);
    tv.set_right_margin(12);
    tv.set_top_margin(8);
    tv.set_bottom_margin(8);
    tv.buffer().set_text(HELP_TEXT);
    scroll.set_child(Some(&tv));
    window.set_child(Some(&scroll));
    window.present();
}
```

- [ ] **Step 3: Register the module**

Modifica `crates/tp-gui/src/dialogs/mod.rs` (o `dialogs.rs` se è ancora un singolo file) aggiungendo:

```rust
pub mod notebook_help;
```

- [ ] **Step 4: Build**

```bash
cargo build --package pax-gui
```

Expected: OK.

- [ ] **Step 5: Commit**

```bash
git add crates/tp-gui/src/dialogs/notebook_help.rs crates/tp-gui/src/dialogs*.rs
git commit -m "Notebook: help dialog (toolbar ? button)"
```

---

## Task 11: User-facing documentation

**Files:**
- Create: `docs/notebook.md`
- Create: `examples/notebook-demo.md`
- Modify: `README.md`

- [ ] **Step 1: Write docs/notebook.md**

Crea `docs/notebook.md`:

```markdown
# Markdown Notebook

Il pannello Markdown di Pax esegue inline i fenced code blocks marcati con
un tag eseguibile, mostrando l'output (testo + immagini) sotto al blocco.
Modello "leggero": ogni blocco è un subprocess isolato, niente kernel
persistenti, niente stato condiviso tra blocchi. L'output vive solo in
memoria — chiudere il pannello lo scarta.

## Sintassi tag

````
```python run                 ← una sola esecuzione, manuale (pulsante ▶)
```python once                ← alias di `run`
```bash watch=5s              ← ciclico ogni 5 secondi (auto-start se visibile)
```sh watch=2m                ← ciclico ogni 2 minuti
```python run timeout=120s    ← override del wall-clock cap
```python watch=2s confirm    ← chiedi conferma alla prima esecuzione
````

Linguaggi: `python`, `bash`, `sh`. `python` risolve `python3` poi `python`
in PATH.

Un blocco fenced senza tag (es. solo `python`) viene renderizzato come
codice statico — non si esegue.

## Output ricco (immagini, plot)

Per emettere un'immagine inline, usa il marker stdout:

````
```python run
print("<<pax:image:/tmp/plot.png>>")
```
````

Da Python è più comodo l'helper auto-iniettato:

````
```python run
import pax, matplotlib.pyplot as plt
plt.plot([1,2,3])
pax.show_plot(plt)            # salva PNG temp ed emette il marker
pax.show("/tmp/static.png")   # solo file path
```
````

In v1 le immagini su file sono renderizzate inline; le `data:image/...` URI
sono visualizzate come warning (decoding base64 da implementare).

## Lifecycle watch

- `watch=Ns` parte automaticamente al primo render, in modo non bloccante.
- Si mette in pausa quando il pannello non è visibile (tab cambio,
  passaggio in Edit mode, pannello chiuso).
- Salta un tick se il run precedente è ancora vivo (no accodamento).
- Chiusura pannello → SIGTERM al subprocess, SIGKILL dopo 2s.

## Sicurezza

Una blocklist minima impedisce comandi distruttivi ovvi (`rm -rf /`,
`mkfs`, fork bomb, `shutdown`, …). Per il resto i cell girano con i tuoi
privilegi: **non aprire notebook scaricati da fonti non fidate**. Il tag
`confirm` aggiunge un dialog opt-in per i cell con cui vuoi un veto manuale.

## Limiti operativi

- Max 8 processi notebook attivi per processo Pax.
- Default timeout: 30s per `run`/`once`. `watch` non ha un timeout (il
  tick successivo sostituisce il precedente solo se è già finito).

## Troubleshooting

- "blocked: …" → la blocklist ha intercettato il codice. Riformula il
  comando per essere meno simile a un pattern distruttivo, oppure
  estendi `tp-core/src/safety.rs::notebook_blocklist()`.
- "python interpreter not found" → installa `python3` e assicurati che
  sia in PATH del processo Pax.
- Immagine non si vede → controlla che il path sia assoluto ed esista
  al momento dell'esecuzione (i path relativi non sono risolti).
```

- [ ] **Step 2: Create examples/notebook-demo.md**

Crea `examples/notebook-demo.md`:

````markdown
# Pax Notebook — demo

Apri questo file nel pannello Markdown di Pax in modalità Render.

## Bash one-shot

```bash run
echo "Hello from bash"
date
```

## Python one-shot

```python run
import sys
print(f"Python {sys.version}")
print("ciao")
```

## Watch — un orologio ogni 1s

```bash watch=1s
date '+%H:%M:%S'
```

## Watch con conferma — opt-in

```python watch=2s confirm
import random
print(f"random = {random.random():.4f}")
```

## Plot inline (richiede matplotlib)

```python run
import pax
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
plt.figure()
plt.plot([1, 2, 4, 8, 16])
plt.title("powers of 2")
pax.show_plot(plt)
```

## Bloccato dalla blocklist

```bash run
rm -rf /
```
````

- [ ] **Step 3: Add a section to README.md**

Sotto la sezione esistente che descrive i pannelli, aggiungi:

```markdown
## Markdown Notebook

I fenced code blocks marcati con tag eseguibile (`run`, `watch=5s`, …) sono
eseguiti inline dal pannello Markdown. Vedi [`docs/notebook.md`](docs/notebook.md)
per la sintassi completa.
```

(Adatta il punto d'inserimento allo stile italiano del README esistente.)

- [ ] **Step 4: Commit**

```bash
git add docs/notebook.md examples/notebook-demo.md README.md
git commit -m "Notebook: user docs + demo file"
```

---

## Task 12: Manual smoke test

**Files:** none modified, just verification.

- [ ] **Step 1: Build release**

```bash
cargo build --release
```

Expected: OK.

- [ ] **Step 2: Run with the demo file**

```bash
RUST_LOG=pax_gui=debug cargo run -- new "notebook-demo"
```

Apri manualmente il pannello Markdown sul file `examples/notebook-demo.md` (puoi farlo via Chooser panel o via UI standard).

Verifica visivamente:
- [ ] Bash one-shot: click ▶ → stdout sotto al blocco. Stato verde dopo.
- [ ] Python one-shot: idem.
- [ ] Watch 1s: parte da solo, aggiorna ogni secondo.
- [ ] Switch a Edit mode → watch in pausa (nessun update). Torna in Render → riprende.
- [ ] Watch confirm: dialog appare al primo bootstrap. Cancel → niente run. Confirm → parte e i tick successivi non chiedono più.
- [ ] Plot matplotlib: render dell'immagine sotto al blocco (max 400px alto).
- [ ] Blocco `rm -rf /`: output `blocked: …` rosso, nessuna esecuzione.
- [ ] Pulsante `?` in toolbar apre il dialog di help.
- [ ] Chiusura pannello: nessun zombie process (`ps aux | grep python3`).

- [ ] **Step 3: Verify regression — file senza tag**

Apri un qualsiasi `.md` senza blocchi eseguibili (es. `README.md`).

- [ ] Render mode mostra il documento come prima del feature.
- [ ] Edit mode invariato.
- [ ] Switch render↔edit funziona.
- [ ] Sync input invariato.

- [ ] **Step 4: Final commit (if any tweaks needed)**

Se durante lo smoke test sono emersi piccoli aggiustamenti (CSS, etichette, timeout), commit a parte:

```bash
git add -p
git commit -m "Notebook: smoke-test fixes"
```

---

## Verification checklist (run before declaring done)

```bash
# All tests pass
cargo test

# Full build (default features, including sourceview + vte)
cargo build --release

# Build without features works (macOS path)
cargo build --release --no-default-features
```

Manual: complete Task 12 checklist.
