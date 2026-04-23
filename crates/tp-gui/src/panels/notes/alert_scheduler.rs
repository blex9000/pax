//! Scheduled-alert loop for Note panels.
//!
//! Started once at app activation: every `POLL_INTERVAL_SECS` it queries
//! the DB for notes whose `alert_at` has come due and fires both a desktop
//! notification and an in-panel refresh (by marking the row as fired,
//! which the next list reload will pick up).
//!
//! Alerts missed while Pax was closed are marked as fired on the first
//! tick after startup *without* emitting an OS notification — we don't
//! want to flood the user with late pop-ups when they just launched the
//! app. The note still carries a `fired_at` badge so the user can see it
//! was due.

use std::time::{SystemTime, UNIX_EPOCH};

use gtk4::glib;
use libadwaita as adw;

/// Poll cadence. Thirty seconds is a reasonable balance between
/// responsiveness and DB pressure; alerts are minute-grained so finer
/// polling buys nothing.
const POLL_INTERVAL_SECS: u32 = 30;
/// Grace window: if the alert is due but no older than this, we still
/// emit the OS notification on startup. Beyond that we treat it as a
/// missed alert and silently mark it fired.
const STARTUP_GRACE_SECS: i64 = 5 * 60;

/// Start the scheduler. Runs on the GLib main loop; stopping happens
/// implicitly when the Application drops.
pub fn start(app: adw::Application) {
    // Fire a startup sweep immediately so notes that came due while Pax
    // was closed get reconciled before the first 30s tick.
    startup_sweep(&app);

    glib::timeout_add_seconds_local(POLL_INTERVAL_SECS, move || {
        tick(&app);
        glib::ControlFlow::Continue
    });
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn open_db() -> Option<pax_db::Database> {
    pax_db::Database::open(&pax_db::Database::default_path())
        .map_err(|e| {
            tracing::warn!("notes scheduler: could not open db: {e}");
            e
        })
        .ok()
}

fn tick(app: &adw::Application) {
    let Some(db) = open_db() else {
        return;
    };
    let now = now_secs();
    let due = match db.due_workspace_notes(now) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("notes scheduler: due query failed: {e}");
            return;
        }
    };
    for note in due {
        emit_alert(app, &note.text);
        if let Err(e) = db.mark_note_alert_fired(note.id, now) {
            tracing::warn!("notes scheduler: mark_fired failed: {e}");
        }
    }
}

fn startup_sweep(app: &adw::Application) {
    let Some(db) = open_db() else {
        return;
    };
    let now = now_secs();
    let due = match db.due_workspace_notes(now) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("notes scheduler: startup due query failed: {e}");
            return;
        }
    };
    for note in due {
        let age = now - note.alert_at.unwrap_or(now);
        if age <= STARTUP_GRACE_SECS {
            emit_alert(app, &note.text);
        } else {
            tracing::info!(
                "notes scheduler: missed alert on note {} (age {}s) — marking fired silently",
                note.id,
                age
            );
        }
        if let Err(e) = db.mark_note_alert_fired(note.id, now) {
            tracing::warn!("notes scheduler: startup mark_fired failed: {e}");
        }
    }
}

fn emit_alert(app: &adw::Application, body_source: &str) {
    let body = note_preview(body_source);
    crate::notifications::send_desktop(app, None, "Pax note", &body);
}

/// Collapse a markdown note down to a plain one-line preview suitable
/// for a notification body. Strips heading hashes, bullets, and
/// whitespace; clips to ~120 chars.
fn note_preview(text: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 120;
    let compacted: String = text
        .lines()
        .map(|l| l.trim_start_matches(|c: char| c == '#' || c == '-' || c == '*' || c.is_whitespace()))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if compacted.chars().count() > MAX_PREVIEW_CHARS {
        let truncated: String = compacted.chars().take(MAX_PREVIEW_CHARS).collect();
        format!("{truncated}…")
    } else {
        compacted
    }
}
