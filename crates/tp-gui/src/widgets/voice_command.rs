use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TRANSCRIBE_CMD_ENV: &str = "PAX_VOICE_TRANSCRIBE_CMD";
const GEMINI_SCRIPT_NAME: &str = "pax-voice-transcribe-gemini.py";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceAction {
    InsertText(String),
    Newline,
    Key(VoiceKey),
    Pax(PaxCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceKey {
    Enter,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Backspace,
    Delete,
    Tab,
    Escape,
    CtrlC,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaxCommand {
    SelectTab(String),
    Raw(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicePlan {
    pub actions: Vec<VoiceAction>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceExecutionReport {
    pub executed: usize,
    pub skipped: Vec<String>,
}

#[derive(Clone, Copy)]
enum MarkerKind {
    InsertText,
    InsertLiteral,
    Keyboard,
    Pax,
    Newline,
}

struct Marker {
    token: &'static str,
    kind: MarkerKind,
}

const MARKERS: &[Marker] = &[
    Marker {
        token: "scrivi letteralmente:",
        kind: MarkerKind::InsertLiteral,
    },
    Marker {
        token: "scrivi:",
        kind: MarkerKind::InsertText,
    },
    Marker {
        token: "tastiera:",
        kind: MarkerKind::Keyboard,
    },
    Marker {
        token: "pax:",
        kind: MarkerKind::Pax,
    },
    Marker {
        token: "va a capo",
        kind: MarkerKind::Newline,
    },
    Marker {
        token: "nuova riga",
        kind: MarkerKind::Newline,
    },
    Marker {
        token: "a capo",
        kind: MarkerKind::Newline,
    },
];

pub fn parse_voice_phrase(phrase: &str) -> VoicePlan {
    let lower = phrase.to_ascii_lowercase();
    let mut pos = 0usize;
    let mut actions = Vec::new();
    let mut issues = Vec::new();

    while pos < phrase.len() {
        let Some((marker_pos, marker)) = find_next_marker(&lower, pos) else {
            let ignored = phrase[pos..].trim();
            if !ignored.is_empty() {
                issues.push(format!("Testo ignorato senza prefisso: {ignored}"));
            }
            break;
        };

        if marker_pos > pos {
            let ignored = phrase[pos..marker_pos].trim();
            if !ignored.is_empty() {
                issues.push(format!("Testo ignorato senza prefisso: {ignored}"));
            }
        }

        let payload_start = marker_pos + marker.token.len();
        match marker.kind {
            MarkerKind::Newline => {
                actions.push(VoiceAction::Newline);
                pos = payload_start;
            }
            MarkerKind::InsertText | MarkerKind::InsertLiteral => {
                let next = find_next_marker(&lower, payload_start)
                    .map(|(idx, _)| idx)
                    .unwrap_or(phrase.len());
                let text = strip_wrapping_quotes(phrase[payload_start..next].trim());
                if text.is_empty() {
                    issues.push("Comando scrivi senza testo".to_string());
                } else {
                    actions.push(VoiceAction::InsertText(text.to_string()));
                }
                pos = next;
            }
            MarkerKind::Keyboard => {
                let next = find_next_marker(&lower, payload_start)
                    .map(|(idx, _)| idx)
                    .unwrap_or(phrase.len());
                let payload = phrase[payload_start..next].trim();
                match parse_key(payload) {
                    Some(key) => actions.push(VoiceAction::Key(key)),
                    None => issues.push(format!("Tasto non riconosciuto: {payload}")),
                }
                pos = next;
            }
            MarkerKind::Pax => {
                let next = find_next_marker(&lower, payload_start)
                    .map(|(idx, _)| idx)
                    .unwrap_or(phrase.len());
                let payload = strip_wrapping_quotes(phrase[payload_start..next].trim());
                if payload.is_empty() {
                    issues.push("Comando pax senza testo".to_string());
                } else {
                    actions.push(VoiceAction::Pax(parse_pax_command(payload)));
                }
                pos = next;
            }
        }
    }

    VoicePlan { actions, issues }
}

pub fn execute_voice_actions(
    panel_type: &str,
    actions: &[VoiceAction],
    writer: &dyn Fn(&[u8]) -> bool,
) -> VoiceExecutionReport {
    let mut executed = 0usize;
    let mut skipped = Vec::new();

    for action in actions {
        match action {
            VoiceAction::InsertText(text) => {
                if writer(text.as_bytes()) {
                    executed += 1;
                } else {
                    skipped.push("Scrittura fallita".to_string());
                }
            }
            VoiceAction::Newline => {
                if panel_type == "terminal" {
                    skipped.push(
                        "Nel terminale 'va a capo' non preme Invio: usa 'tastiera: invio'"
                            .to_string(),
                    );
                } else if writer(b"\n") {
                    executed += 1;
                } else {
                    skipped.push("Nuova riga fallita".to_string());
                }
            }
            VoiceAction::Key(key) => match key_bytes(panel_type, key) {
                Some(bytes) if writer(bytes) => executed += 1,
                Some(_) => skipped.push(format!("Tasto non inviato: {}", key.label())),
                None => skipped.push(format!(
                    "Tasto '{}' non supportato nel pannello {}",
                    key.label(),
                    panel_type
                )),
            },
            VoiceAction::Pax(command) => {
                skipped.push(format!(
                    "Comando Pax non ancora collegato: {}",
                    command.label()
                ));
            }
        }
    }

    VoiceExecutionReport { executed, skipped }
}

pub fn build_voice_command_button(
    panel_type: Rc<dyn Fn() -> Option<String>>,
    writer: Rc<dyn Fn(&[u8]) -> bool>,
) -> gtk4::Button {
    let button = gtk4::Button::new();
    button.set_icon_name("audio-input-microphone-symbolic");
    button.add_css_class("flat");
    button.add_css_class("panel-action-btn");
    button.set_tooltip_text(Some("Voice commands"));

    button.connect_clicked(move |btn| {
        let popover = build_voice_popover(panel_type.clone(), writer.clone());
        popover.set_parent(btn);
        popover.connect_closed(|popover| {
            if popover.parent().is_some() {
                popover.unparent();
            }
        });
        popover.popup();
    });

    button
}

fn build_voice_popover(
    panel_type: Rc<dyn Fn() -> Option<String>>,
    writer: Rc<dyn Fn(&[u8]) -> bool>,
) -> gtk4::Popover {
    let popover = gtk4::Popover::new();
    crate::theme::configure_popover(&popover);
    popover.add_css_class("voice-command-popover");

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    root.add_css_class("voice-popover-root");
    root.set_margin_top(6);
    root.set_margin_bottom(6);
    root.set_margin_start(6);
    root.set_margin_end(6);
    root.set_width_request(260);

    let title = gtk4::Label::new(Some("Input vocale"));
    title.add_css_class("heading");
    title.set_halign(gtk4::Align::Center);
    root.append(&title);

    let transcribe_status = resolve_transcribe_status();

    let status = gtk4::Label::new(Some(transcribe_status.message));
    status.add_css_class("dim-label");
    status.add_css_class("caption");
    status.add_css_class("voice-status");
    status.set_wrap(true);
    status.set_xalign(0.5);
    status.set_justify(gtk4::Justification::Center);
    root.append(&status);

    let mic_btn = gtk4::ToggleButton::new();
    mic_btn.add_css_class("voice-mic-button");
    mic_btn.set_halign(gtk4::Align::Center);
    mic_btn.set_size_request(72, 72);
    mic_btn.set_sensitive(transcribe_status.ready);
    mic_btn.set_tooltip_text(Some(transcribe_status.tooltip));
    let mic_icon = gtk4::Image::from_icon_name("audio-input-microphone-symbolic");
    mic_btn.set_child(Some(&mic_icon));
    root.append(&mic_btn);

    let preview = gtk4::Label::new(Some("Nessun comando pronto."));
    preview.add_css_class("caption");
    preview.add_css_class("voice-preview");
    preview.set_wrap(true);
    preview.set_xalign(0.5);
    preview.set_justify(gtk4::Justification::Center);
    root.append(&preview);

    let run_btn = build_voice_popover_row("Esegui piano", "object-select-symbolic");
    run_btn.set_sensitive(false);
    root.append(&run_btn);

    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    sep.set_margin_top(2);
    sep.set_margin_bottom(2);
    root.append(&sep);

    let manual_btn = build_voice_popover_row("Manuale / debug", "document-edit-symbolic");
    root.append(&manual_btn);
    let manual_revealer = gtk4::Revealer::new();
    manual_revealer.set_reveal_child(false);
    manual_revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);
    let manual_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("scrivi: ls -la tastiera: invio"));
    manual_box.append(&entry);
    manual_revealer.set_child(Some(&manual_box));
    root.append(&manual_revealer);

    let guide_btn = build_voice_popover_row("Guida rapida", "dialog-information-symbolic");
    root.append(&guide_btn);
    let guide_revealer = gtk4::Revealer::new();
    guide_revealer.set_reveal_child(false);
    guide_revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);
    let help = gtk4::Label::new(Some(
        "scrivi: testo da inserire\n\
         scrivi letteralmente: testo che sembra un comando\n\
         va a capo\n\
         tastiera: invio | freccia giu | freccia su | control c\n\
         pax: seleziona tab nome\n\n\
         Nel terminale 'scrivi:' non preme Invio: serve 'tastiera: invio'.",
    ));
    help.add_css_class("dim-label");
    help.add_css_class("caption");
    help.set_wrap(true);
    help.set_xalign(0.0);
    guide_revealer.set_child(Some(&help));
    root.append(&guide_revealer);

    let phrase = Rc::new(RefCell::new(String::new()));
    let current_job: Rc<RefCell<Option<TranscribeJob>>> = Rc::new(RefCell::new(None));
    let job_generation = Rc::new(Cell::new(0u64));
    let suppress_toggle = Rc::new(Cell::new(false));

    {
        let revealer = manual_revealer.clone();
        manual_btn.connect_clicked(move |_| {
            revealer.set_reveal_child(!revealer.reveals_child());
        });
    }
    {
        let revealer = guide_revealer.clone();
        guide_btn.connect_clicked(move |_| {
            revealer.set_reveal_child(!revealer.reveals_child());
        });
    }

    {
        let current_job = current_job.clone();
        let job_generation = job_generation.clone();
        let phrase = phrase.clone();
        let suppress_toggle = suppress_toggle.clone();
        let provider = transcribe_status.provider.clone();
        let status = status.clone();
        let preview = preview.clone();
        let run_btn = run_btn.clone();
        let entry = entry.clone();
        mic_btn.connect_toggled(move |btn| {
            if suppress_toggle.get() {
                return;
            }

            if !btn.is_active() {
                job_generation.set(job_generation.get().wrapping_add(1));
                if let Some(job) = current_job.borrow_mut().take() {
                    job.cancel();
                }
                phrase.borrow_mut().clear();
                entry.set_text("");
                status.set_text("Ascolto fermato.");
                update_plan_preview("", &preview, &run_btn);
                return;
            }

            let Some(provider) = provider.clone() else {
                set_toggle_active_silently(btn, &suppress_toggle, false);
                status.set_text("Provider voce non trovato.");
                preview.set_text("Script Gemini non trovato nel repo o nel bundle.");
                return;
            };

            if let Some(job) = current_job.borrow_mut().take() {
                job.cancel();
            }

            job_generation.set(job_generation.get().wrapping_add(1));
            let token = job_generation.get();
            phrase.borrow_mut().clear();
            entry.set_text("");
            run_btn.set_sensitive(false);
            status.set_text("Ascolto... riclicca il microfono per fermare.");
            preview.set_text("Sto registrando.");

            let btn_c = btn.clone();
            let current_job_c = current_job.clone();
            let entry_c = entry.clone();
            let job_generation_c = job_generation.clone();
            let phrase_c = phrase.clone();
            let suppress_toggle_c = suppress_toggle.clone();
            let status_c = status.clone();
            let preview_c = preview.clone();
            let run_btn_c = run_btn.clone();

            match start_transcribe_command(provider, move |result| {
                if job_generation_c.get() != token {
                    return;
                }

                current_job_c.borrow_mut().take();
                set_toggle_active_silently(&btn_c, &suppress_toggle_c, false);

                match result {
                    TranscribeResult::Transcript(text) if !text.trim().is_empty() => {
                        let trimmed = text.trim().to_string();
                        entry_c.set_text(&trimmed);
                        *phrase_c.borrow_mut() = trimmed.clone();
                        status_c.set_text("Comando pronto. Controlla il piano prima di eseguire.");
                        update_plan_preview(&trimmed, &preview_c, &run_btn_c);
                    }
                    TranscribeResult::Transcript(_) => {
                        entry_c.set_text("");
                        phrase_c.borrow_mut().clear();
                        status_c.set_text("Trascrizione vuota.");
                        update_plan_preview("", &preview_c, &run_btn_c);
                    }
                    TranscribeResult::Cancelled => {
                        entry_c.set_text("");
                        phrase_c.borrow_mut().clear();
                        status_c.set_text("Ascolto fermato.");
                        update_plan_preview("", &preview_c, &run_btn_c);
                    }
                    TranscribeResult::Failed(err) => {
                        entry_c.set_text("");
                        phrase_c.borrow_mut().clear();
                        status_c.set_text("Trascrizione fallita.");
                        preview_c.set_text(&err);
                        run_btn_c.set_sensitive(false);
                    }
                }
            }) {
                Ok(job) => {
                    *current_job.borrow_mut() = Some(job);
                }
                Err(err) => {
                    set_toggle_active_silently(btn, &suppress_toggle, false);
                    status.set_text("Trascrizione non avviata.");
                    preview.set_text(&err);
                    run_btn.set_sensitive(false);
                }
            }
        });
    }

    let run_plan: Rc<dyn Fn()> = Rc::new({
        let phrase = phrase.clone();
        let status = status.clone();
        let preview = preview.clone();
        let panel_type = panel_type.clone();
        let writer = writer.clone();
        move || {
            let panel_type = panel_type().unwrap_or_else(|| "unknown".to_string());
            let phrase = phrase.borrow().clone();
            let plan = parse_voice_phrase(&phrase);
            if plan.actions.is_empty() {
                status.set_text("Nessuna azione valida.");
                preview.set_text("Nessuna azione valida da eseguire.");
                return;
            }
            let report = execute_voice_actions(&panel_type, &plan.actions, writer.as_ref());
            status.set_text("Esecuzione completata.");
            preview.set_text(&execution_preview(&plan, &report));
        }
    });

    {
        let run = run_plan.clone();
        run_btn.connect_clicked(move |_| run());
    }

    {
        let phrase = phrase.clone();
        let status = status.clone();
        let preview = preview.clone();
        let run_btn = run_btn.clone();
        entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            *phrase.borrow_mut() = text.clone();
            if text.trim().is_empty() {
                status.set_text("Manuale / debug.");
            } else {
                status.set_text("Comando manuale pronto.");
            }
            update_plan_preview(&text, &preview, &run_btn);
        });
    }

    entry.connect_activate(move |_| run_plan());

    {
        let current_job = current_job.clone();
        let job_generation = job_generation.clone();
        popover.connect_closed(move |_| {
            job_generation.set(job_generation.get().wrapping_add(1));
            if let Some(job) = current_job.borrow_mut().take() {
                job.cancel();
            }
        });
    }

    popover.set_child(Some(&root));
    popover
}

fn build_voice_popover_row(label: &str, icon_name: &str) -> gtk4::Button {
    let btn = gtk4::Button::new();
    btn.add_css_class("flat");
    btn.add_css_class("app-popover-button");

    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let icon = gtk4::Image::from_icon_name(icon_name);
    let label = gtk4::Label::new(Some(label));
    label.set_halign(gtk4::Align::Start);
    label.set_hexpand(true);
    row.append(&icon);
    row.append(&label);
    btn.set_child(Some(&row));
    btn
}

fn set_toggle_active_silently(
    button: &gtk4::ToggleButton,
    suppress_toggle: &Cell<bool>,
    active: bool,
) {
    suppress_toggle.set(true);
    button.set_active(active);
    suppress_toggle.set(false);
}

fn update_plan_preview(phrase: &str, preview: &gtk4::Label, run_btn: &gtk4::Button) {
    let trimmed = phrase.trim();
    if trimmed.is_empty() {
        preview.set_text("Nessun comando pronto.");
        run_btn.set_sensitive(false);
        return;
    }

    let plan = parse_voice_phrase(trimmed);
    preview.set_text(&plan_preview(&plan));
    run_btn.set_sensitive(!plan.actions.is_empty());
}

enum TranscribeResult {
    Transcript(String),
    Cancelled,
    Failed(String),
}

#[derive(Clone)]
struct TranscribeJob {
    cancelled: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl TranscribeJob {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(mut child) = self.child.lock() {
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TranscribeProvider {
    EnvOverride(String),
    DefaultGemini(PathBuf),
}

impl TranscribeProvider {
    fn command_string(&self) -> String {
        match self {
            TranscribeProvider::EnvOverride(cmd) => cmd.clone(),
            TranscribeProvider::DefaultGemini(path) => shell_quote_path(path),
        }
    }
}

#[derive(Clone)]
struct TranscribeStatus {
    provider: Option<TranscribeProvider>,
    ready: bool,
    message: &'static str,
    tooltip: &'static str,
}

fn resolve_transcribe_status() -> TranscribeStatus {
    let provider = resolve_transcribe_provider();
    match provider {
        Some(provider @ TranscribeProvider::EnvOverride(_)) => TranscribeStatus {
            provider: Some(provider),
            ready: true,
            message: "Pronto per trascrivere.",
            tooltip: "Clicca per ascoltare; riclicca per fermare",
        },
        Some(provider @ TranscribeProvider::DefaultGemini(_)) if gemini_api_key_configured() => {
            TranscribeStatus {
                provider: Some(provider),
                ready: true,
                message: "Pronto per trascrivere con Gemini.",
                tooltip: "Clicca per ascoltare; riclicca per fermare",
            }
        }
        Some(provider @ TranscribeProvider::DefaultGemini(_)) => TranscribeStatus {
            provider: Some(provider),
            ready: false,
            message: "Gemini API key mancante.",
            tooltip: "Imposta GEMINI_API_KEY o GOOGLE_API_KEY per usare il microfono",
        },
        None => TranscribeStatus {
            provider: None,
            ready: false,
            message: "Provider voce non trovato.",
            tooltip: "Script Gemini non trovato nel repo o nel bundle",
        },
    }
}

fn resolve_transcribe_provider() -> Option<TranscribeProvider> {
    let env_override = std::env::var(TRANSCRIBE_CMD_ENV)
        .ok()
        .map(|cmd| cmd.trim().to_string())
        .filter(|cmd| !cmd.is_empty());
    provider_from_override_or_paths(env_override, candidate_gemini_scripts())
}

fn provider_from_override_or_paths(
    env_override: Option<String>,
    candidates: Vec<PathBuf>,
) -> Option<TranscribeProvider> {
    if let Some(cmd) = env_override {
        return Some(TranscribeProvider::EnvOverride(cmd));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .map(TranscribeProvider::DefaultGemini)
}

fn candidate_gemini_scripts() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    paths.push(PathBuf::from("scripts").join(GEMINI_SCRIPT_NAME));
    paths.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts")
            .join(GEMINI_SCRIPT_NAME),
    );

    if let Ok(exe) = std::env::current_exe() {
        if let Some(contents_dir) = exe.parent().and_then(|dir| dir.parent()) {
            paths.push(
                contents_dir
                    .join("Resources/scripts")
                    .join(GEMINI_SCRIPT_NAME),
            );
        }
        for ancestor in exe.ancestors() {
            paths.push(ancestor.join("scripts").join(GEMINI_SCRIPT_NAME));
        }
    }

    dedupe_paths(paths)
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut deduped = Vec::new();
    for path in paths {
        if !deduped.iter().any(|existing| existing == &path) {
            deduped.push(path);
        }
    }
    deduped
}

fn gemini_api_key_configured() -> bool {
    ["GEMINI_API_KEY", "GOOGLE_API_KEY"].iter().any(|key| {
        std::env::var(key)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    })
}

fn shell_quote_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn start_transcribe_command(
    provider: TranscribeProvider,
    on_done: impl FnOnce(TranscribeResult) + 'static,
) -> Result<TranscribeJob, String> {
    let cmd = provider.command_string();

    let result_slot = Arc::new(Mutex::new(None::<TranscribeResult>));
    let result_slot_thread = result_slot.clone();
    let callback = Rc::new(RefCell::new(Some(on_done)));
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_thread = cancelled.clone();
    let child_slot = Arc::new(Mutex::new(None::<Child>));
    let child_slot_thread = child_slot.clone();

    std::thread::spawn(move || {
        // The transcription command is user-configured host tooling
        // (whisper, curl, …) and may need host audio devices — route it
        // through flatpak-spawn --host when sandboxed, before stdio.
        let mut base = Command::new("sh");
        base.arg("-lc").arg(&cmd);
        let child = crate::host_spawn::hostify(base)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        match child {
            Ok(child) => {
                *child_slot_thread.lock().unwrap() = Some(child);
            }
            Err(err) => {
                *result_slot_thread.lock().unwrap() =
                    Some(TranscribeResult::Failed(err.to_string()));
                return;
            }
        }

        loop {
            if cancelled_thread.load(Ordering::SeqCst) {
                cancel_child(&child_slot_thread);
                *result_slot_thread.lock().unwrap() = Some(TranscribeResult::Cancelled);
                break;
            }

            let result = {
                let mut guard = child_slot_thread.lock().unwrap();
                match guard.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(Some(status)) => {
                            let child = guard.take().unwrap();
                            Some(collect_child_output(child, status))
                        }
                        Ok(None) => None,
                        Err(err) => {
                            guard.take();
                            Some(TranscribeResult::Failed(err.to_string()))
                        }
                    },
                    None => Some(TranscribeResult::Cancelled),
                }
            };

            if let Some(result) = result {
                *result_slot_thread.lock().unwrap() = Some(result);
                break;
            }

            std::thread::sleep(Duration::from_millis(20));
        }
    });

    gtk4::glib::timeout_add_local(Duration::from_millis(16), move || {
        let result = result_slot.lock().unwrap().take();
        match result {
            Some(result) => {
                if let Some(cb) = callback.borrow_mut().take() {
                    cb(result);
                }
                gtk4::glib::ControlFlow::Break
            }
            None => gtk4::glib::ControlFlow::Continue,
        }
    });

    Ok(TranscribeJob {
        cancelled,
        child: child_slot,
    })
}

fn cancel_child(child_slot: &Arc<Mutex<Option<Child>>>) {
    let mut guard = child_slot.lock().unwrap();
    if let Some(child) = guard.as_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
    guard.take();
}

fn collect_child_output(mut child: Child, status: ExitStatus) -> TranscribeResult {
    let mut stdout = String::new();
    if let Some(mut stream) = child.stdout.take() {
        let _ = stream.read_to_string(&mut stdout);
    }

    if status.success() {
        return TranscribeResult::Transcript(stdout.trim().to_string());
    }

    let mut stderr = String::new();
    if let Some(mut stream) = child.stderr.take() {
        let _ = stream.read_to_string(&mut stderr);
    }
    let stderr = stderr.trim();
    TranscribeResult::Failed(if stderr.is_empty() {
        format!("comando terminato con {status}")
    } else {
        stderr.to_string()
    })
}

fn find_next_marker(lower: &str, from: usize) -> Option<(usize, &'static Marker)> {
    MARKERS
        .iter()
        .filter_map(|marker| {
            lower[from..]
                .find(marker.token)
                .map(|offset| (from + offset, marker))
        })
        .min_by_key(|(idx, marker)| (*idx, marker.token.len()))
}

fn parse_key(payload: &str) -> Option<VoiceKey> {
    let normalized = normalize(payload);
    if normalized.contains("control c")
        || normalized.contains("ctrl c")
        || normalized.contains("controllo c")
    {
        Some(VoiceKey::CtrlC)
    } else if normalized.contains("freccia giu") || normalized == "giu" || normalized == "down" {
        Some(VoiceKey::ArrowDown)
    } else if normalized.contains("freccia su") || normalized == "su" || normalized == "up" {
        Some(VoiceKey::ArrowUp)
    } else if normalized.contains("freccia sinistra")
        || normalized == "sinistra"
        || normalized == "left"
    {
        Some(VoiceKey::ArrowLeft)
    } else if normalized.contains("freccia destra")
        || normalized == "destra"
        || normalized == "right"
    {
        Some(VoiceKey::ArrowRight)
    } else if normalized.contains("invio")
        || normalized.contains("enter")
        || normalized.contains("return")
    {
        Some(VoiceKey::Enter)
    } else if normalized.contains("backspace") || normalized.contains("cancella indietro") {
        Some(VoiceKey::Backspace)
    } else if normalized == "canc" || normalized.contains("delete") {
        Some(VoiceKey::Delete)
    } else if normalized == "tab" || normalized.contains("tabulazione") {
        Some(VoiceKey::Tab)
    } else if normalized == "esc" || normalized.contains("escape") {
        Some(VoiceKey::Escape)
    } else {
        None
    }
}

fn parse_pax_command(payload: &str) -> PaxCommand {
    let normalized = normalize(payload);
    let select_tab_prefix = "seleziona tab ";
    if normalized.starts_with(select_tab_prefix) && payload.len() >= select_tab_prefix.len() {
        PaxCommand::SelectTab(payload[select_tab_prefix.len()..].trim().to_string())
    } else {
        PaxCommand::Raw(payload.to_string())
    }
}

fn key_bytes<'a>(panel_type: &str, key: &VoiceKey) -> Option<&'a [u8]> {
    let terminal = panel_type == "terminal";
    match key {
        VoiceKey::Enter if terminal => Some(b"\r"),
        VoiceKey::Enter => Some(b"\n"),
        VoiceKey::ArrowUp if terminal => Some(b"\x1b[A"),
        VoiceKey::ArrowDown if terminal => Some(b"\x1b[B"),
        VoiceKey::ArrowRight if terminal => Some(b"\x1b[C"),
        VoiceKey::ArrowLeft if terminal => Some(b"\x1b[D"),
        VoiceKey::Backspace => Some(&[0x7f]),
        VoiceKey::Delete if terminal => Some(b"\x1b[3~"),
        VoiceKey::Tab => Some(b"\t"),
        VoiceKey::Escape if terminal => Some(&[0x1b]),
        VoiceKey::CtrlC if terminal => Some(&[0x03]),
        _ => None,
    }
}

fn plan_preview(plan: &VoicePlan) -> String {
    if plan.actions.is_empty() && plan.issues.is_empty() {
        return "Nessuna azione riconosciuta.".to_string();
    }
    let mut parts: Vec<String> = plan.actions.iter().map(VoiceAction::label).collect();
    parts.extend(plan.issues.iter().map(|issue| format!("Avviso: {issue}")));
    parts.join(" · ")
}

fn execution_preview(plan: &VoicePlan, report: &VoiceExecutionReport) -> String {
    let mut text = format!(
        "{} azione/i eseguite. Piano: {}",
        report.executed,
        plan_preview(plan)
    );
    if !report.skipped.is_empty() {
        text.push_str("\nNon eseguito: ");
        text.push_str(&report.skipped.join(" · "));
    }
    text
}

fn strip_wrapping_quotes(text: &str) -> &str {
    let trimmed = text.trim();
    for (open, close) in [('"', '"'), ('\'', '\''), ('«', '»'), ('“', '”')] {
        if trimmed.starts_with(open) && trimmed.ends_with(close) {
            return trimmed
                .trim_start_matches(open)
                .trim_end_matches(close)
                .trim();
        }
    }
    trimmed
}

fn normalize(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .replace(['ù', 'ú'], "u")
        .replace(['ì', 'í'], "i")
        .replace(['è', 'é'], "e")
        .replace(['ò', 'ó'], "o")
        .replace(['à', 'á'], "a")
}

impl VoiceAction {
    fn label(&self) -> String {
        match self {
            VoiceAction::InsertText(text) => format!("Scrivi \"{}\"", text),
            VoiceAction::Newline => "Nuova riga".to_string(),
            VoiceAction::Key(key) => format!("Tastiera {}", key.label()),
            VoiceAction::Pax(command) => format!("Pax {}", command.label()),
        }
    }
}

impl VoiceKey {
    fn label(&self) -> &'static str {
        match self {
            VoiceKey::Enter => "Invio",
            VoiceKey::ArrowUp => "Freccia su",
            VoiceKey::ArrowDown => "Freccia giu",
            VoiceKey::ArrowLeft => "Freccia sinistra",
            VoiceKey::ArrowRight => "Freccia destra",
            VoiceKey::Backspace => "Backspace",
            VoiceKey::Delete => "Delete",
            VoiceKey::Tab => "Tab",
            VoiceKey::Escape => "Esc",
            VoiceKey::CtrlC => "Ctrl+C",
        }
    }
}

impl PaxCommand {
    fn label(&self) -> String {
        match self {
            PaxCommand::SelectTab(name) => format!("seleziona tab \"{}\"", name),
            PaxCommand::Raw(command) => command.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn parses_write_newline_and_key_sequence() {
        let plan = parse_voice_phrase(
            "scrivi: ieri sono andato al mare va a capo scrivi: poi tastiera: invio",
        );
        assert_eq!(
            plan.actions,
            vec![
                VoiceAction::InsertText("ieri sono andato al mare".to_string()),
                VoiceAction::Newline,
                VoiceAction::InsertText("poi".to_string()),
                VoiceAction::Key(VoiceKey::Enter),
            ]
        );
        assert!(plan.issues.is_empty());
    }

    #[test]
    fn literal_write_keeps_command_words_as_text() {
        let plan = parse_voice_phrase("scrivi letteralmente: pax seleziona tab terminale");
        assert_eq!(
            plan.actions,
            vec![VoiceAction::InsertText(
                "pax seleziona tab terminale".to_string()
            )]
        );
    }

    #[test]
    fn parses_keyboard_arrows_and_ctrl_c() {
        assert_eq!(
            parse_voice_phrase("tastiera: freccia giù").actions,
            vec![VoiceAction::Key(VoiceKey::ArrowDown)]
        );
        assert_eq!(
            parse_voice_phrase("tastiera: control c").actions,
            vec![VoiceAction::Key(VoiceKey::CtrlC)]
        );
    }

    #[test]
    fn terminal_newline_requires_explicit_enter_key() {
        let out = RefCell::new(Vec::new());
        let plan = parse_voice_phrase("scrivi: ls va a capo tastiera: invio");
        let report = execute_voice_actions("terminal", &plan.actions, &|bytes| {
            out.borrow_mut().extend_from_slice(bytes);
            true
        });
        assert_eq!(out.into_inner(), b"ls\r");
        assert_eq!(report.executed, 2);
        assert_eq!(report.skipped.len(), 1);
    }

    #[test]
    fn markdown_newline_is_inserted() {
        let out = RefCell::new(Vec::new());
        let plan = parse_voice_phrase("scrivi: ciao va a capo scrivi: mondo");
        let report = execute_voice_actions("markdown", &plan.actions, &|bytes| {
            out.borrow_mut().extend_from_slice(bytes);
            true
        });
        assert_eq!(out.into_inner(), b"ciao\nmondo");
        assert_eq!(report.executed, 3);
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn provider_override_wins_over_default_script() {
        let provider = provider_from_override_or_paths(
            Some("custom-transcriber".to_string()),
            vec![PathBuf::from("/tmp/pax-existing-provider")],
        );
        assert_eq!(
            provider,
            Some(TranscribeProvider::EnvOverride(
                "custom-transcriber".to_string()
            ))
        );
    }

    #[test]
    fn provider_uses_first_existing_default_script() {
        let root =
            std::env::temp_dir().join(format!("pax-voice-provider-test-{}", std::process::id()));
        let missing = root.join("missing").join(GEMINI_SCRIPT_NAME);
        let scripts = root.join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        let existing = scripts.join(GEMINI_SCRIPT_NAME);
        std::fs::write(&existing, "#!/usr/bin/env python3\n").unwrap();

        let provider = provider_from_override_or_paths(None, vec![missing, existing.clone()]);
        assert_eq!(provider, Some(TranscribeProvider::DefaultGemini(existing)));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn default_script_command_is_shell_quoted() {
        let provider = TranscribeProvider::DefaultGemini(PathBuf::from("/tmp/Pax Voice/a'b.py"));
        assert_eq!(provider.command_string(), "'/tmp/Pax Voice/a'\\''b.py'");
    }
}
