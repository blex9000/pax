//! Spawn a notebook cell's code in a subprocess, capture stdout/stderr
//! line-by-line, parse markers via `output::parse_line`, and forward
//! `OutputItem`s to the engine via a channel.
//!
//! Threading model: spawn produces a `RunHandle` carrying the child PID
//! and a stop flag, plus a separate `Receiver<RunMsg>`. Two dedicated
//! reader threads drain stdout and stderr; a third "supervisor" thread
//! enforces wall-clock timeout. Handle dropped → SIGTERM to the whole
//! process group, 2s grace, SIGKILL to the whole process group.
//!
//! Process-group ownership: every cell child runs in a fresh session
//! (`setsid` via `pre_exec`), making the child its own process-group
//! leader (PID == PGID). Termination signals are sent with `kill(-pgid)`
//! so any subprocesses the cell spawned (e.g. `bash -s` running
//! `sleep 999 &`) die with the parent instead of becoming orphans.
//!
//! Contract notes for consumers (engine):
//! - `Output` items may arrive shortly *after* `Finished` because reader
//!   threads keep draining buffered pipe content until EOF. Be tolerant.
//! - Dropping only the `Receiver` does NOT terminate the subprocess —
//!   you must drop the `RunHandle` as well (`stop_flag` triggers the
//!   supervisor). The two are decoupled by design.
//! - PID-reuse window: `Drop` polls `kill(pid, 0)` and then issues
//!   SIGKILL. On a busy system the kernel could in theory reuse the PID
//!   between probes, but the 50 ms grain + 2 s window makes this an
//!   accepted v1 risk; engine-level cleanup also enforces termination.
//! - `confirm` flag is NOT consulted here — confirmation is the engine's
//!   responsibility before calling `spawn`.

use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use pax_core::notebook_tag::{ExecMode, Lang, NotebookCellSpec};
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
        // SIGTERM to the whole process group (negative pid → pgid).
        // The child was placed in its own session via `setsid` in
        // `pre_exec`, so PID == PGID and any descendants the cell
        // spawned receive the signal too.
        unsafe { libc::kill(-self.pid, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_millis(TERMINATE_GRACE_MS);
        while Instant::now() < deadline {
            if unsafe { libc::kill(self.pid, 0) } != 0 {
                return; // leader already dead
            }
            thread::sleep(Duration::from_millis(50));
        }
        // SIGKILL the whole group as fallback.
        unsafe { libc::kill(-self.pid, libc::SIGKILL) };
    }
}

/// Spawn a cell's code. Returns `Err(reason)` if blocked by safety, or
/// `Ok((handle, rx))` with a live subprocess. The receiver is detached
/// from the handle so the engine can drain it independently.
pub fn spawn(
    spec: &NotebookCellSpec,
    code: &str,
    helpers_dir: Option<&std::path::Path>,
    output_dir: Option<&std::path::Path>,
) -> Result<(RunHandle, Receiver<RunMsg>), String> {
    // Safety blocklist applies only to shell languages — its patterns are
    // shell-specific (rm -rf /, mkfs, dd if=…of=/dev/, shutdown, reboot, …)
    // and produce false positives on Python source where method names like
    // `executor.shutdown()` or `asyncio.shutdown()` would trigger \bshutdown\b.
    // Do NOT widen this gate without revisiting safety::notebook_blocklist().
    if matches!(spec.lang, Lang::Bash | Lang::Sh) {
        if let Ok(SafetyCheck::Blocked(reason)) = check_notebook_command(code) {
            return Err(reason);
        }
    }

    let (program, args) = command_for(spec.lang)?;
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    // Put the child in a fresh session so PID == PGID. This lets the
    // RunHandle Drop and the supervisor target the entire descendant
    // tree with `kill(-pgid, …)` instead of leaking grandchildren when
    // the cell forks (e.g. `bash -s` running a backgrounded `sleep`).
    // Safety: `setsid` is async-signal-safe and modifies only the child
    // between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

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

    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {}", e))?;

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

    let timeout = spec
        .timeout
        .unwrap_or(Duration::from_secs(super::DEFAULT_RUN_TIMEOUT_SECS));
    let is_watch = matches!(spec.mode, ExecMode::Watch { .. });

    spawn_supervisor(
        child,
        pid,
        tx,
        stop_flag.clone(),
        if is_watch { None } else { Some(timeout) },
    );

    Ok((RunHandle { pid, stop_flag }, rx))
}

fn command_for(lang: Lang) -> Result<(&'static str, Vec<&'static str>), String> {
    match lang {
        Lang::Python => {
            // Prefer python3, fall back to python.
            for cand in ["python3", "python"] {
                if which(cand).is_some() {
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

fn spawn_reader<R: std::io::Read + Send + 'static>(stream: R, tx: Sender<RunMsg>, is_stderr: bool) {
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
    pid: i32,
    tx: Sender<RunMsg>,
    stop_flag: Arc<AtomicBool>,
    timeout: Option<Duration>,
) {
    // `child.kill()` only signals the leader; for cells that fork their
    // own subprocesses (e.g. shell `&` jobs) we additionally signal the
    // whole process group via `kill(-pgid, …)`. PID == PGID because the
    // child was set up with `setsid` in `pre_exec`.
    let kill_group = move || {
        unsafe { libc::kill(-pid, libc::SIGTERM) };
    };
    thread::spawn(move || {
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let _ = tx.send(RunMsg::Finished {
                        exit_code: status.code(),
                    });
                    return;
                }
                Ok(None) => {}
                Err(_) => {
                    let _ = tx.send(RunMsg::Finished { exit_code: None });
                    return;
                }
            }
            if stop_flag.load(Ordering::SeqCst) {
                kill_group();
                let _ = child.kill();
                let _ = tx.send(RunMsg::Finished { exit_code: None });
                return;
            }
            if let Some(t) = timeout {
                if started.elapsed() >= t {
                    let _ = tx.send(RunMsg::Output(OutputItem::Error(format!(
                        "timeout after {:?}",
                        t
                    ))));
                    kill_group();
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
    // These tests spawn real subprocesses. They are gated with `#[serial]`
    // because the documented PID-reuse window in `RunHandle::Drop` would
    // otherwise let test A's drop SIGTERM test B's freshly-spawned PID
    // when the harness runs them in parallel. In production this is
    // harmless because no panel spawns and tears down multiple cells in
    // <50 ms.
    use super::*;
    use serial_test::serial;
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
    #[serial]
    fn runs_simple_bash() {
        let (_h, rx) = spawn(&once_spec(Lang::Bash), "echo hello\n", None, None).unwrap();
        let items = collect_until_finished(&rx);
        assert!(items
            .iter()
            .any(|i| matches!(i, OutputItem::Text(t) if t == "hello")));
    }

    #[test]
    #[serial]
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
    #[serial]
    fn blocked_command_returns_err() {
        let r = spawn(&once_spec(Lang::Bash), "rm -rf /\n", None, None);
        assert!(r.is_err());
    }

    #[test]
    #[serial]
    fn stderr_becomes_error_items() {
        let (_h, rx) = spawn(&once_spec(Lang::Bash), "echo oops 1>&2\n", None, None).unwrap();
        let items = collect_until_finished(&rx);
        assert!(items
            .iter()
            .any(|i| matches!(i, OutputItem::Error(t) if t == "oops")));
    }
}
