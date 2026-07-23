use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::voice_provider::{GeminiTranscript, VoiceContext};

const TRANSCRIBE_CMD_ENV: &str = "PAX_VOICE_TRANSCRIBE_CMD";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VoiceProvider {
    EnvOverride(String),
    RustGeminiLive,
}

#[derive(Clone)]
pub(crate) struct VoiceStatus {
    pub provider: Option<VoiceProvider>,
    pub ready: bool,
    pub message: &'static str,
    pub tooltip: &'static str,
}

#[derive(Debug)]
pub(crate) enum VoiceSessionEvent {
    AudioLevel(f64),
    Status(String),
    PartialTranscript(String),
    AssistantTranscript(String),
    Command(GeminiTranscript),
    ToolCall {
        call: crate::voice_tools::VoiceToolCall,
        response: Sender<crate::voice_tools::VoiceToolExecution>,
    },
    TurnComplete,
    Completed,
    Cancelled,
    Failed(String),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VoiceSessionControl {
    SendText(String),
    HostEvent(String),
    SetMicrophoneEnabled(bool),
    SetOutputMuted(bool),
}

#[derive(Clone)]
pub(crate) struct VoiceSessionJob {
    cancelled: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    control: Option<tokio::sync::mpsc::UnboundedSender<VoiceSessionControl>>,
}

impl VoiceSessionJob {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        cancel_child(&self.child);
    }

    pub(crate) fn send_text(&self, text: &str) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("Il messaggio non puo' essere vuoto.".to_string());
        }
        self.send_control(VoiceSessionControl::SendText(text.to_string()))
    }

    pub(crate) fn set_microphone_enabled(&self, enabled: bool) -> Result<(), String> {
        self.send_control(VoiceSessionControl::SetMicrophoneEnabled(enabled))
    }

    pub(crate) fn send_host_event(&self, event: &str) -> Result<(), String> {
        let event = event.trim();
        if event.is_empty() {
            return Err("L'evento host non puo' essere vuoto.".to_string());
        }
        self.send_control(VoiceSessionControl::HostEvent(event.to_string()))
    }

    pub(crate) fn set_output_muted(&self, muted: bool) -> Result<(), String> {
        self.send_control(VoiceSessionControl::SetOutputMuted(muted))
    }

    fn send_control(&self, control: VoiceSessionControl) -> Result<(), String> {
        self.control
            .as_ref()
            .ok_or_else(|| "Il provider custom non supporta i controlli Live.".to_string())?
            .send(control)
            .map_err(|_| "La sessione Gemini Live non e' piu' attiva.".to_string())
    }
}

pub(crate) fn resolve_transcribe_status() -> VoiceStatus {
    let provider = resolve_transcribe_provider();
    match provider {
        provider @ VoiceProvider::EnvOverride(_) => VoiceStatus {
            provider: Some(provider),
            ready: true,
            message: "Pronto per trascrivere.",
            tooltip: "Clicca per ascoltare; riclicca per fermare",
        },
        provider @ VoiceProvider::RustGeminiLive if gemini_api_key_configured() => VoiceStatus {
            provider: Some(provider),
            ready: true,
            message: "Gemini Live pronto.",
            tooltip: "Avvia o ferma l'ascolto Live",
        },
        provider @ VoiceProvider::RustGeminiLive => VoiceStatus {
            provider: Some(provider),
            ready: false,
            message: "Gemini API key mancante.",
            tooltip: "Apri Settings -> AI Assistant e inserisci la Gemini API key",
        },
    }
}

fn resolve_transcribe_provider() -> VoiceProvider {
    let env_override = std::env::var(TRANSCRIBE_CMD_ENV)
        .ok()
        .map(|cmd| cmd.trim().to_string())
        .filter(|cmd| !cmd.is_empty());
    provider_from_override(env_override)
}

fn provider_from_override(env_override: Option<String>) -> VoiceProvider {
    if let Some(cmd) = env_override {
        VoiceProvider::EnvOverride(cmd)
    } else {
        VoiceProvider::RustGeminiLive
    }
}

fn gemini_api_key_configured() -> bool {
    crate::voice_settings::load_gemini_api_key().is_some()
}

pub(crate) fn start_transcribe_session(
    provider: VoiceProvider,
    context: VoiceContext,
) -> Result<(VoiceSessionJob, Receiver<VoiceSessionEvent>), String> {
    let (tx, rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let finish_requested = Arc::new(AtomicBool::new(false));
    let child_slot = Arc::new(Mutex::new(None::<Child>));
    let mut control = None;

    match provider {
        VoiceProvider::RustGeminiLive => {
            let (control_tx, control_rx) = tokio::sync::mpsc::unbounded_channel();
            control = Some(control_tx);
            let tx_thread = tx.clone();
            let cancelled_thread = cancelled.clone();
            let finish_thread = finish_requested.clone();
            let panic_tx = tx.clone();
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_gemini_session(
                        cancelled_thread,
                        finish_thread,
                        control_rx,
                        tx_thread,
                        context,
                    )
                }));
                if let Err(payload) = result {
                    let _ = panic_tx.send(VoiceSessionEvent::Failed(format!(
                        "Provider Gemini Live in panic: {}",
                        panic_message(payload)
                    )));
                }
            });
        }
        VoiceProvider::EnvOverride(cmd) => {
            let tx_thread = tx.clone();
            let cancelled_thread = cancelled.clone();
            let finish_thread = finish_requested.clone();
            let child_slot_thread = child_slot.clone();
            std::thread::spawn(move || {
                run_env_override_session(
                    cmd,
                    cancelled_thread,
                    finish_thread,
                    child_slot_thread,
                    tx_thread,
                )
            });
        }
    }

    Ok((
        VoiceSessionJob {
            cancelled,
            child: child_slot,
            control,
        },
        rx,
    ))
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "errore interno sconosciuto".to_string()
    }
}

fn run_gemini_session(
    cancelled: Arc<AtomicBool>,
    finish_requested: Arc<AtomicBool>,
    control: tokio::sync::mpsc::UnboundedReceiver<VoiceSessionControl>,
    tx: Sender<VoiceSessionEvent>,
    context: VoiceContext,
) {
    let level_callback: Arc<dyn Fn(f64) + Send + Sync> = Arc::new({
        let tx = tx.clone();
        move |level| {
            let _ = tx.send(VoiceSessionEvent::AudioLevel(level));
        }
    });
    let status_callback: Arc<dyn Fn(String) + Send + Sync> = Arc::new({
        let tx = tx.clone();
        move |message| {
            let _ = tx.send(VoiceSessionEvent::Status(message));
        }
    });
    let partial_callback: Arc<dyn Fn(String) + Send + Sync> = Arc::new({
        let tx = tx.clone();
        move |transcript| {
            let _ = tx.send(VoiceSessionEvent::PartialTranscript(transcript));
        }
    });
    let assistant_callback: Arc<dyn Fn(String) + Send + Sync> = Arc::new({
        let tx = tx.clone();
        move |transcript| {
            let _ = tx.send(VoiceSessionEvent::AssistantTranscript(transcript));
        }
    });
    let command_callback: Arc<dyn Fn(GeminiTranscript) + Send + Sync> = Arc::new({
        let tx = tx.clone();
        move |command| {
            let _ = tx.send(VoiceSessionEvent::Command(command));
        }
    });
    let turn_complete_callback: Arc<dyn Fn() + Send + Sync> = Arc::new({
        let tx = tx.clone();
        move || {
            let _ = tx.send(VoiceSessionEvent::TurnComplete);
        }
    });
    let tool_callback: Arc<
        dyn Fn(
                crate::voice_tools::VoiceToolCall,
            ) -> Result<Receiver<crate::voice_tools::VoiceToolExecution>, String>
            + Send
            + Sync,
    > = Arc::new({
        let tx = tx.clone();
        move |call| {
            let (response, receiver) = mpsc::channel();
            tx.send(VoiceSessionEvent::ToolCall { call, response })
                .map_err(|_| "La UI dell'assistente non e' piu' disponibile.".to_string())?;
            Ok(receiver)
        }
    });
    match crate::voice_live::run_gemini_live(
        cancelled,
        finish_requested,
        control,
        level_callback,
        status_callback,
        partial_callback,
        assistant_callback,
        command_callback,
        turn_complete_callback,
        tool_callback,
        context,
    ) {
        Ok(crate::voice_live::LiveRun::Completed) => {
            let _ = tx.send(VoiceSessionEvent::Completed);
        }
        Ok(crate::voice_live::LiveRun::Cancelled) => {
            let _ = tx.send(VoiceSessionEvent::Cancelled);
        }
        Err(err) => {
            let _ = tx.send(VoiceSessionEvent::Failed(err));
        }
    }
}

fn run_env_override_session(
    cmd: String,
    cancelled: Arc<AtomicBool>,
    finish_requested: Arc<AtomicBool>,
    child_slot: Arc<Mutex<Option<Child>>>,
    tx: Sender<VoiceSessionEvent>,
) {
    let _ = tx.send(VoiceSessionEvent::Status(
        "Trascrizione con provider custom...".to_string(),
    ));
    // Custom providers may depend on host audio devices and binaries. Route
    // the command through flatpak-spawn when Pax runs inside a sandbox.
    let mut base = Command::new("sh");
    base.arg("-lc").arg(&cmd);
    let child = crate::host_spawn::hostify(base)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    match child {
        Ok(child) => {
            *child_slot.lock().expect("voice child lock") = Some(child);
        }
        Err(err) => {
            let _ = tx.send(VoiceSessionEvent::Failed(err.to_string()));
            return;
        }
    }

    loop {
        if cancelled.load(Ordering::SeqCst) {
            cancel_child(&child_slot);
            let _ = tx.send(VoiceSessionEvent::Cancelled);
            break;
        }
        if finish_requested.load(Ordering::SeqCst) {
            cancel_child(&child_slot);
            let _ = tx.send(VoiceSessionEvent::Completed);
            break;
        }

        let result = {
            let mut guard = child_slot.lock().expect("voice child lock");
            match guard.as_mut() {
                Some(child) => match child.try_wait() {
                    Ok(Some(status)) => {
                        let child = guard.take().expect("voice child");
                        Some(collect_child_output(child, status))
                    }
                    Ok(None) => None,
                    Err(err) => {
                        guard.take();
                        Some(Err(err.to_string()))
                    }
                },
                None => Some(Err("provider custom annullato".to_string())),
            }
        };

        if let Some(result) = result {
            match result {
                Ok(transcript) => {
                    let _ = tx.send(VoiceSessionEvent::Command(transcript));
                    let _ = tx.send(VoiceSessionEvent::Completed);
                }
                Err(err) => {
                    let _ = tx.send(VoiceSessionEvent::Failed(err));
                }
            }
            break;
        }

        std::thread::sleep(Duration::from_millis(20));
    }
}

fn cancel_child(child_slot: &Arc<Mutex<Option<Child>>>) {
    let mut guard = child_slot.lock().expect("voice child lock");
    if let Some(child) = guard.as_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
    guard.take();
}

fn collect_child_output(mut child: Child, status: ExitStatus) -> Result<GeminiTranscript, String> {
    let mut stdout = String::new();
    if let Some(mut stream) = child.stdout.take() {
        let _ = stream.read_to_string(&mut stdout);
    }

    if status.success() {
        let command = stdout.trim().to_string();
        return Ok(GeminiTranscript {
            transcript: None,
            command: command.clone(),
            raw_text: command,
            recorder: "custom provider".to_string(),
            audio_bytes: 0,
            audio_peak: 0.0,
        });
    }

    let mut stderr = String::new();
    if let Some(mut stream) = child.stderr.take() {
        let _ = stream.read_to_string(&mut stderr);
    }
    let stderr = stderr.trim();
    Err(if stderr.is_empty() {
        format!("comando terminato con {status}")
    } else {
        stderr.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_override_wins_over_rust_gemini_live() {
        let provider = provider_from_override(Some("custom-transcriber".to_string()));
        assert_eq!(
            provider,
            VoiceProvider::EnvOverride("custom-transcriber".to_string())
        );
    }

    #[test]
    fn default_provider_is_rust_gemini_live() {
        assert_eq!(provider_from_override(None), VoiceProvider::RustGeminiLive);
    }

    #[test]
    fn live_job_forwards_text_microphone_and_mute_controls() {
        let (control, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let job = VoiceSessionJob {
            cancelled: Arc::new(AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
            control: Some(control),
        };

        job.send_text("  mostra README  ").unwrap();
        job.set_microphone_enabled(false).unwrap();
        job.set_output_muted(true).unwrap();

        assert_eq!(
            receiver.try_recv().unwrap(),
            VoiceSessionControl::SendText("mostra README".to_string())
        );
        assert_eq!(
            receiver.try_recv().unwrap(),
            VoiceSessionControl::SetMicrophoneEnabled(false)
        );
        assert_eq!(
            receiver.try_recv().unwrap(),
            VoiceSessionControl::SetOutputMuted(true)
        );
        assert!(job.send_text("   ").is_err());
    }

    #[test]
    #[ignore = "uses the configured Gemini key and the live service"]
    fn configured_text_only_muted_live_session_smoke_test() {
        let (job, receiver) = start_transcribe_session(
            VoiceProvider::RustGeminiLive,
            VoiceContext {
                panel_type: None,
                workspace: None,
            },
        )
        .unwrap();
        job.set_microphone_enabled(false).unwrap();
        job.set_output_muted(true).unwrap();
        job.send_text("Chi sei? Rispondi in una frase breve in italiano.")
            .unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut assistant_text = String::new();
        let mut turn_complete = false;
        while std::time::Instant::now() < deadline && !turn_complete {
            match receiver.recv_timeout(Duration::from_millis(500)) {
                Ok(VoiceSessionEvent::AssistantTranscript(text)) => assistant_text = text,
                Ok(VoiceSessionEvent::TurnComplete) => turn_complete = true,
                Ok(VoiceSessionEvent::Failed(error)) => panic!("Gemini Live failed: {error}"),
                Ok(VoiceSessionEvent::ToolCall { call, response }) => {
                    let _ = response.send(crate::voice_tools::VoiceToolExecution::immediate(
                        crate::voice_tools::VoiceToolResult::error(&call, "unexpected tool call"),
                    ));
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        job.cancel();

        assert!(turn_complete, "text-only Gemini turn did not complete");
        assert!(
            !assistant_text.trim().is_empty(),
            "text-only Gemini turn had no output transcription"
        );
    }
}
