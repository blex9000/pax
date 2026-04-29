//! Helpers for tests that need a real GTK runtime.
//!
//! `gtk4::init()` is one-shot **and** thread-bound: once called on a thread
//! T, every subsequent call from a different thread panics. The cargo test
//! harness spawns a worker thread per test, so even with `#[serial]`
//! serialising execution the second GTK-touching test would crash.
//!
//! This module hosts a single dedicated thread that initialises GTK once
//! and runs submitted closures on it. Tests submit their body via
//! [`run_on_gtk_thread`] and the helper waits for completion, propagating
//! any panic so the test framework still sees failures.
//!
//! Pair with `#[serial]` on every test that touches GTK to also serialise
//! against state shared between tests (CSS providers, app-level statics).

#![cfg(test)]

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;

type Job = Box<dyn FnOnce() + Send + 'static>;

fn gtk_thread() -> &'static Mutex<Option<Sender<Job>>> {
    static TX: OnceLock<Mutex<Option<Sender<Job>>>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        thread::Builder::new()
            .name("pax-gui-test-gtk".into())
            .spawn(move || {
                let initialised = gtk4::init().is_ok();
                while let Ok(job) = rx.recv() {
                    if !initialised {
                        // Drop the job silently; the caller will observe
                        // missing widgets and decide what to do.
                        continue;
                    }
                    job();
                }
            })
            .expect("spawn dedicated GTK test thread");
        Mutex::new(Some(tx))
    })
}

/// Run `body` on the dedicated GTK thread. Returns `false` if GTK could
/// not be initialised (e.g. headless CI without a display); the caller
/// should treat that as "test skipped".
pub fn run_on_gtk_thread<F>(body: F) -> bool
where
    F: FnOnce() + Send + 'static,
{
    let (done_tx, done_rx) = channel::<Option<Box<dyn std::any::Any + Send>>>();
    let job: Job = Box::new(move || {
        let result = catch_unwind(AssertUnwindSafe(body));
        let _ = done_tx.send(result.err());
    });
    let guard = gtk_thread().lock().unwrap();
    let Some(tx) = guard.as_ref() else {
        return false;
    };
    if tx.send(job).is_err() {
        return false;
    }
    drop(guard);
    match done_rx.recv() {
        Ok(None) => true,
        Ok(Some(payload)) => std::panic::resume_unwind(payload),
        Err(_) => false,
    }
}
