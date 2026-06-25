use gtk4::prelude::*;
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TRANSCRIBE_CMD_ENV: &str = "PAX_VOICE_TRANSCRIBE_CMD";

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
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    root.set_margin_top(10);
    root.set_margin_bottom(10);
    root.set_margin_start(10);
    root.set_margin_end(10);
    root.set_width_request(360);

    let title = gtk4::Label::new(Some("Comandi vocali"));
    title.add_css_class("heading");
    title.set_halign(gtk4::Align::Start);
    root.append(&title);

    let help = gtk4::Label::new(Some(
        "Protocollo:\n\
         scrivi: testo da inserire\n\
         scrivi letteralmente: testo che sembra un comando\n\
         va a capo\n\
         tastiera: invio | freccia giu | freccia su | control c\n\
         pax: seleziona tab nome\n\n\
         Nel terminale 'scrivi:' non preme Invio.\n\
         Backend STT: imposta PAX_VOICE_TRANSCRIBE_CMD. Il comando deve stampare il transcript su stdout.",
    ));
    help.add_css_class("dim-label");
    help.set_wrap(true);
    help.set_xalign(0.0);
    root.append(&help);

    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("scrivi: ls -la tastiera: invio"));
    root.append(&entry);

    let transcribe_btn = gtk4::Button::with_label("Trascrivi");
    transcribe_btn.add_css_class("flat");
    let transcribe_configured = std::env::var(TRANSCRIBE_CMD_ENV)
        .map(|cmd| !cmd.trim().is_empty())
        .unwrap_or(false);
    transcribe_btn.set_sensitive(transcribe_configured);
    transcribe_btn.set_tooltip_text(Some(if transcribe_configured {
        "Esegue PAX_VOICE_TRANSCRIBE_CMD e usa stdout come transcript"
    } else {
        "Imposta PAX_VOICE_TRANSCRIBE_CMD per abilitare registrazione/STT"
    }));
    root.append(&transcribe_btn);

    let preview = gtk4::Label::new(Some("Inserisci una frase per vedere il piano."));
    preview.add_css_class("caption");
    preview.set_wrap(true);
    preview.set_xalign(0.0);
    root.append(&preview);

    let run_btn = gtk4::Button::with_label("Esegui piano");
    run_btn.add_css_class("suggested-action");
    root.append(&run_btn);

    {
        let preview = preview.clone();
        entry.connect_changed(move |entry| {
            let phrase = entry.text().to_string();
            let plan = parse_voice_phrase(&phrase);
            preview.set_text(&plan_preview(&plan));
        });
    }

    {
        let entry = entry.clone();
        let preview = preview.clone();
        transcribe_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            preview.set_text("Trascrizione in corso...");
            let btn_c = btn.clone();
            let entry_c = entry.clone();
            let preview_c = preview.clone();
            run_transcribe_command(move |result| {
                btn_c.set_sensitive(true);
                match result {
                    Ok(text) if !text.trim().is_empty() => {
                        entry_c.set_text(text.trim());
                        let plan = parse_voice_phrase(text.trim());
                        preview_c.set_text(&plan_preview(&plan));
                    }
                    Ok(_) => preview_c.set_text("Trascrizione vuota."),
                    Err(err) => preview_c.set_text(&format!("Trascrizione fallita: {err}")),
                }
            });
        });
    }

    let run_plan: Rc<dyn Fn()> = Rc::new({
        let entry = entry.clone();
        let preview = preview.clone();
        let panel_type = panel_type.clone();
        let writer = writer.clone();
        move || {
            let panel_type = panel_type().unwrap_or_else(|| "unknown".to_string());
            let plan = parse_voice_phrase(&entry.text());
            if plan.actions.is_empty() {
                preview.set_text("Nessuna azione valida da eseguire.");
                return;
            }
            let report = execute_voice_actions(&panel_type, &plan.actions, writer.as_ref());
            preview.set_text(&execution_preview(&plan, &report));
        }
    });

    {
        let run = run_plan.clone();
        run_btn.connect_clicked(move |_| run());
    }
    entry.connect_activate(move |_| run_plan());

    popover.set_child(Some(&root));
    popover
}

fn run_transcribe_command(on_done: impl FnOnce(Result<String, String>) + 'static) {
    let cmd = match std::env::var(TRANSCRIBE_CMD_ENV) {
        Ok(cmd) if !cmd.trim().is_empty() => cmd,
        _ => {
            on_done(Err(format!("{TRANSCRIBE_CMD_ENV} non configurata")));
            return;
        }
    };

    let slot = Arc::new(Mutex::new(None::<Result<String, String>>));
    let slot_thread = slot.clone();
    let callback = Rc::new(RefCell::new(Some(on_done)));

    std::thread::spawn(move || {
        let output = Command::new("sh").arg("-lc").arg(&cmd).output();
        let result = match output {
            Ok(output) if output.status.success() => {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                Err(if stderr.is_empty() {
                    format!("comando terminato con {}", output.status)
                } else {
                    stderr
                })
            }
            Err(err) => Err(err.to_string()),
        };
        *slot_thread.lock().unwrap() = Some(result);
    });

    gtk4::glib::timeout_add_local(Duration::from_millis(16), move || {
        let result = slot.lock().unwrap().take();
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
}
