use std::ffi::OsString;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::voice_provider::{GeminiTranscript, VoiceContext};
use crate::voice_tools::{VoiceToolCall, VoiceToolExecution, VoiceToolResult};

const LIVE_ENDPOINT: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";
const SUBMIT_COMMAND_TOOL: &str = "submit_voice_command";
const AUDIO_MIME_TYPE: &str = "audio/pcm;rate=16000";
const OUTPUT_AUDIO_RATE: &str = "24000";
const OUTPUT_AUDIO_BYTES_PER_SECOND: usize = 24_000 * std::mem::size_of::<i16>();
const PLAYBACK_PREBUFFER_MS: usize = 400;
const PLAYBACK_PREBUFFER_BYTES: usize =
    OUTPUT_AUDIO_BYTES_PER_SECOND * PLAYBACK_PREBUFFER_MS / 1_000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const SETUP_TIMEOUT: Duration = Duration::from_secs(15);
const FINISH_TIMEOUT: Duration = Duration::from_secs(6);

const LIVE_SYSTEM_PROMPT: &str = r#"You are the realtime AI assistant integrated into Pax.

Understand natural user requests in their spoken language and use the available tools. Never claim an operation succeeded before its tool result confirms it.

Conversation style:
- Be concise, direct, and task-focused by default.
- Give only information or interaction that is useful for the user's current request.
- For a simple question or routine confirmation, use one short sentence and normally no more than 15 spoken words.
- Expand only when the user explicitly asks for detail or more detail is required for correctness, safety, ambiguity, or a necessary next step.
- Avoid greetings, thanks, pleasantries, filler, repeated summaries, and generic offers to help.
- Do not repeat or paraphrase the user's request unless needed to resolve ambiguity.
- After a tool result, state only the outcome and any necessary clarification or next action.

For workspace navigation:
- The workspace snapshot contains the complete recursive layout, not only top-level tabs.
- In each tab_groups entry, every tab has authoritative panel_ids and panel_count fields.
- Answer questions about tab/panel structure from this data. Never claim that only top-level tabs are visible to you.
- Use workspace_inspect when the user asks about the current layout, panels, counts, visibility, or focus and the snapshot may be stale or insufficient.
- Use workspace_select_tab whenever the user asks to show, open, switch to, or select a tab.
- Use workspace_action for GUI-equivalent workspace changes such as focus, horizontal/vertical splits, adding or closing panels/tabs, moving or expanding panels, zoom, input sync, renaming, changing panel type, resetting, or saving.
- After every workspace_action, inspect the returned workspace snapshot and report the actual resulting structure.
- For close_panel, close_tab, and reset_panel, never set confirm=true until the user explicitly confirms that destructive action.
- Never translate tab navigation into a pax protocol command.

For terminal panels:
- Terminal output is deliberately absent from the persistent workspace snapshot. Never assume that you have seen it.
- Use terminal_read only on request or when inspecting recent output is necessary to fulfill the current task. Request the smallest useful number of recent lines.
- Use terminal_write to type exact printable text. It never submits the text.
- Use terminal_key for Enter, arrows, Tab, Space, Escape, paging, editing, and control keys.
- When the user expects a command or interactive operation to be followed through, call terminal_wait after terminal_key instead of asking the user to provide another message. Pass terminal_key's watch_token for shell_prompt and after_revision for output conditions.
- Use shell_prompt for ordinary shell commands. For Codex, Claude Code, and other TUIs, use output_quiet or output_changed, inspect the bounded returned output, send only the next necessary key, and wait again until the requested goal is complete or clarification is genuinely required.
- Use task_status only when task context is needed and task_cancel to stop monitoring. Cancelling monitoring never terminates the terminal process.
- Use terminal_configure for terminal settings. Applying settings restarts the terminal, so never set confirm=true until the user explicitly confirms that restart.
- For interactive terminal applications such as Codex or Claude Code, work incrementally: read the relevant recent output, send one necessary input or key sequence, then read again when verification is needed.
- Never continuously poll or request the complete terminal feed.
- Never send Enter unless the user explicitly asked to run, execute, submit, confirm, choose, or otherwise perform the action.

When the focused target is Markdown:
- Use markdown_read to inspect existing content or specific lines.
- Use markdown_search to locate text when the requested target may be ambiguous.
- Use markdown_replace for exact search-and-replace requests.
- Use markdown_delete_line to delete a numbered, last, or last non-empty line.
- Never translate Markdown inspection or editing requests into keyboard commands.

Use submit_voice_command only for literal dictation or explicit keyboard/input requests in the focused non-terminal editor. Never use it for a terminal; use terminal_write and terminal_key instead. Its arguments must contain the verbatim transcript and one Pax protocol command.

Allowed command forms:
- scrivi: text to insert
- scrivi letteralmente: text to insert without interpreting command words
- va a capo
- tastiera: invio
- tastiera: freccia su
- tastiera: freccia giu
- tastiera: freccia sinistra
- tastiera: freccia destra
- tastiera: control c
- tastiera: tab
- tastiera: escape

For explicit dictation, use scrivi: and preserve punctuation and spelling.
Do not invent keyboard actions.
Answer general questions naturally in the user's spoken language, using audio without calling a tool when no action is needed.
After every tool result, briefly confirm the actual outcome aloud. If a tool fails or reports ambiguity, explain it aloud and ask the shortest useful clarification."#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveRun {
    Completed,
    Cancelled,
}

#[derive(Debug)]
enum RecorderEvent {
    Started(String),
    Chunk(Vec<u8>),
    Paused,
    Stopped,
    Failed(String),
}

enum LiveRecordCommand {
    Shell { name: String, command: String },
    Args { name: String, args: Vec<OsString> },
}

enum AudioPlaybackMessage {
    Chunk(Vec<u8>),
    TurnComplete(String),
    Interrupt,
    DrainAndStop,
    Stop,
}

struct LiveAudioPlayback {
    sender: std::sync::mpsc::Sender<AudioPlaybackMessage>,
    worker: Option<std::thread::JoinHandle<()>>,
    playing: Arc<AtomicBool>,
    stopping: bool,
}

impl LiveAudioPlayback {
    fn start(on_status: Arc<dyn Fn(String) + Send + Sync>) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let playing = Arc::new(AtomicBool::new(false));
        let worker_playing = playing.clone();
        let worker =
            std::thread::spawn(move || playback_worker(receiver, on_status, worker_playing));
        Self {
            sender,
            worker: Some(worker),
            playing,
            stopping: false,
        }
    }

    fn play(&self, chunk: Vec<u8>) {
        self.playing.store(true, Ordering::SeqCst);
        if self
            .sender
            .send(AudioPlaybackMessage::Chunk(chunk))
            .is_err()
        {
            self.playing.store(false, Ordering::SeqCst);
        }
    }

    fn turn_complete(&self, ready_status: String) {
        let _ = self
            .sender
            .send(AudioPlaybackMessage::TurnComplete(ready_status));
    }

    fn interrupt(&self) {
        let _ = self.sender.send(AudioPlaybackMessage::Interrupt);
    }

    fn is_playing(&self) -> bool {
        self.playing.load(Ordering::SeqCst)
    }

    fn drain_and_stop(&mut self) {
        if !self.stopping {
            self.stopping = true;
            let _ = self.sender.send(AudioPlaybackMessage::DrainAndStop);
            self.join_worker();
        }
    }

    fn join_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for LiveAudioPlayback {
    fn drop(&mut self) {
        if !self.stopping {
            let _ = self.sender.send(AudioPlaybackMessage::Stop);
        }
        self.join_worker();
    }
}

struct AudioPlaybackProcess {
    child: Child,
    stdin: ChildStdin,
}

#[derive(Default)]
struct PcmPrebuffer {
    pending: Vec<u8>,
    started: bool,
}

impl PcmPrebuffer {
    fn push(&mut self, chunk: Vec<u8>) -> Option<Vec<u8>> {
        if self.started {
            return Some(chunk);
        }
        self.pending.extend(chunk);
        if self.pending.len() < PLAYBACK_PREBUFFER_BYTES {
            return None;
        }
        self.started = true;
        Some(std::mem::take(&mut self.pending))
    }

    fn finish_turn(&mut self) -> Option<Vec<u8>> {
        self.started = false;
        (!self.pending.is_empty()).then(|| std::mem::take(&mut self.pending))
    }

    fn reset(&mut self) {
        self.pending.clear();
        self.started = false;
    }
}

impl LiveRecordCommand {
    fn name(&self) -> &str {
        match self {
            Self::Shell { name, .. } | Self::Args { name, .. } => name,
        }
    }

    fn spawn(self) -> Result<Child, String> {
        let result = match self {
            Self::Shell { command, .. } => Command::new("sh")
                .arg("-lc")
                .arg(command)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn(),
            Self::Args { args, .. } => {
                let Some((program, rest)) = args.split_first() else {
                    return Err("Comando recorder Live vuoto.".to_string());
                };
                Command::new(program)
                    .args(rest)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
            }
        };
        result.map_err(|err| format!("Impossibile avviare recorder Live: {err}"))
    }
}

pub(crate) fn run_gemini_live(
    cancelled: Arc<AtomicBool>,
    finish_requested: Arc<AtomicBool>,
    control: tokio::sync::mpsc::UnboundedReceiver<crate::voice_session::VoiceSessionControl>,
    on_audio_level: Arc<dyn Fn(f64) + Send + Sync>,
    on_status: Arc<dyn Fn(String) + Send + Sync>,
    on_partial_transcript: Arc<dyn Fn(String) + Send + Sync>,
    on_assistant_transcript: Arc<dyn Fn(String) + Send + Sync>,
    on_command: Arc<dyn Fn(GeminiTranscript) + Send + Sync>,
    on_turn_complete: Arc<dyn Fn() + Send + Sync>,
    on_tool_call: Arc<
        dyn Fn(VoiceToolCall) -> Result<Receiver<VoiceToolExecution>, String> + Send + Sync,
    >,
    context: VoiceContext,
) -> Result<LiveRun, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let Some(api_key) = crate::voice_settings::load_gemini_api_key() else {
        return Err("Gemini API key mancante. Apri Settings -> AI Assistant.".to_string());
    };
    let model = crate::voice_settings::load_gemini_model();
    let voice = crate::voice_settings::load_gemini_voice();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("Impossibile avviare runtime Gemini Live: {err}"))?;

    runtime.block_on(run_live_session(
        api_key,
        model,
        voice,
        cancelled,
        finish_requested,
        control,
        on_audio_level,
        on_status,
        on_partial_transcript,
        on_assistant_transcript,
        on_command,
        on_turn_complete,
        on_tool_call,
        context,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn run_live_session(
    api_key: String,
    model: String,
    voice: Option<String>,
    cancelled: Arc<AtomicBool>,
    finish_requested: Arc<AtomicBool>,
    mut control: tokio::sync::mpsc::UnboundedReceiver<crate::voice_session::VoiceSessionControl>,
    on_audio_level: Arc<dyn Fn(f64) + Send + Sync>,
    on_status: Arc<dyn Fn(String) + Send + Sync>,
    on_partial_transcript: Arc<dyn Fn(String) + Send + Sync>,
    on_assistant_transcript: Arc<dyn Fn(String) + Send + Sync>,
    on_command: Arc<dyn Fn(GeminiTranscript) + Send + Sync>,
    on_turn_complete: Arc<dyn Fn() + Send + Sync>,
    on_tool_call: Arc<
        dyn Fn(VoiceToolCall) -> Result<Receiver<VoiceToolExecution>, String> + Send + Sync,
    >,
    context: VoiceContext,
) -> Result<LiveRun, String> {
    on_status("Connessione a Gemini Live...".to_string());
    let url = live_url(&api_key)?;
    let connect = tokio_tungstenite::connect_async(url.as_str());
    let (mut websocket, _) = tokio::select! {
        result = tokio::time::timeout(CONNECT_TIMEOUT, connect) => {
            result
                .map_err(|_| "Timeout connessione Gemini Live.".to_string())?
                .map_err(|err| format!("Connessione Gemini Live fallita: {err}"))?
        }
        _ = wait_for_flag(cancelled.clone()) => return Ok(LiveRun::Cancelled),
    };

    websocket
        .send(Message::Text(
            live_setup(&model, voice.as_deref(), &context)
                .to_string()
                .into(),
        ))
        .await
        .map_err(|err| format!("Invio setup Gemini Live fallito: {err}"))?;
    if let Err(err) = wait_for_setup(&mut websocket, cancelled.clone()).await {
        if cancelled.load(Ordering::SeqCst) {
            return Ok(LiveRun::Cancelled);
        }
        return Err(err);
    }

    let (recorder_tx, mut recorder_rx) = mpsc::unbounded_channel();
    let recorder_enabled = Arc::new(AtomicBool::new(false));
    let mut recorder_requested = false;
    let mut recorder_started = false;
    let mut audio_playback = LiveAudioPlayback::start(on_status.clone());

    on_status("Gemini Live connesso. Parla; il silenzio chiude ogni comando.".to_string());
    let mut recorder = "live recorder".to_string();
    let mut audio_bytes = 0u64;
    let mut audio_peak = 0.0f64;
    let mut transcript = String::new();
    let mut assistant_transcript = String::new();
    let mut model_audio_started = false;
    let mut output_muted = false;
    let mut turn_audio_discarded = false;
    let mut stream_ended = false;
    let mut finish_deadline = None;
    let mut pending_tools = Vec::<PendingVoiceTool>::new();

    loop {
        flush_pending_tool_results(&mut websocket, &mut pending_tools, &on_status).await?;
        if cancelled.load(Ordering::SeqCst) {
            let _ = websocket.close(None).await;
            return Ok(LiveRun::Cancelled);
        }

        if finish_requested.load(Ordering::SeqCst) && !stream_ended {
            send_audio_stream_end(&mut websocket).await?;
            stream_ended = true;
            finish_deadline = Some(Instant::now() + FINISH_TIMEOUT);
            on_status("Microfono fermato. Attendo l'ultimo comando...".to_string());
        }

        if finish_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            audio_playback.drain_and_stop();
            let _ = websocket.close(None).await;
            return Ok(LiveRun::Completed);
        }

        tokio::select! {
            biased;
            control_message = control.recv() => {
                if let Some(control_message) = control_message {
                    match control_message {
                        crate::voice_session::VoiceSessionControl::SendText(text) => {
                            send_text_input(&mut websocket, &text).await?;
                            on_status("Gemini sta elaborando...".to_string());
                        }
                        crate::voice_session::VoiceSessionControl::HostEvent(event) => {
                            let message = format!(
                                "[PAX_HOST_EVENT]\n{event}\n\
                                 Treat this as an authoritative Pax runtime callback, not as user speech. \
                                 Briefly report the outcome or take the next necessary tool action."
                            );
                            send_text_input(&mut websocket, &message).await?;
                            on_status("Aggiornamento task inviato a Gemini...".to_string());
                        }
                        crate::voice_session::VoiceSessionControl::SetMicrophoneEnabled(enabled) => {
                            if recorder_requested && !enabled {
                                send_audio_stream_end(&mut websocket).await?;
                            }
                            recorder_requested = enabled;
                            recorder_enabled.store(enabled, Ordering::SeqCst);
                            if enabled && !recorder_started {
                                spawn_recorder(
                                    recorder_tx.clone(),
                                    cancelled.clone(),
                                    finish_requested.clone(),
                                    recorder_enabled.clone(),
                                    on_audio_level.clone(),
                                    on_status.clone(),
                                );
                                recorder_started = true;
                            }
                            on_status(if enabled {
                                "In ascolto...".to_string()
                            } else {
                                "Microfono disattivato.".to_string()
                            });
                        }
                        crate::voice_session::VoiceSessionControl::SetOutputMuted(muted) => {
                            output_muted = muted;
                            if muted {
                                turn_audio_discarded = true;
                                audio_playback.interrupt();
                            }
                        }
                    }
                }
            }
            recorder_event = recorder_rx.recv(), if !stream_ended => {
                match recorder_event {
                    Some(RecorderEvent::Started(name)) => recorder = name,
                    Some(RecorderEvent::Chunk(chunk)) => {
                        if !recorder_requested || !microphone_audio_enabled(
                            model_audio_started,
                            audio_playback.is_playing(),
                        ) {
                            continue;
                        }
                        audio_bytes += chunk.len() as u64;
                        audio_peak = audio_peak.max(audio_level_from_pcm(&chunk));
                        send_audio(&mut websocket, &chunk).await?;
                    }
                    Some(RecorderEvent::Paused) | Some(RecorderEvent::Stopped) => {
                        recorder_started = false;
                        if recorder_requested {
                            recorder_enabled.store(true, Ordering::SeqCst);
                            spawn_recorder(
                                recorder_tx.clone(),
                                cancelled.clone(),
                                finish_requested.clone(),
                                recorder_enabled.clone(),
                                on_audio_level.clone(),
                                on_status.clone(),
                            );
                            recorder_started = true;
                        }
                    }
                    None => recorder_started = false,
                    Some(RecorderEvent::Failed(err)) => return Err(err),
                }
            }
            message = websocket.next() => {
                let Some(message) = message else {
                    return Err("Gemini Live ha chiuso la connessione.".to_string());
                };
                let message = message.map_err(|err| format!("Ricezione Gemini Live fallita: {err}"))?;
                let Some(value) = message_json(message)? else {
                    continue;
                };

                if let Some(text) = input_transcription(&value) {
                    merge_transcript(&mut transcript, text);
                    on_partial_transcript(transcript.clone());
                }
                if let Some(text) = output_transcription(&value) {
                    merge_transcript(&mut assistant_transcript, text);
                    on_assistant_transcript(assistant_transcript.clone());
                }
                let output_audio = output_audio_chunks(&value)?;
                if !output_audio.is_empty() && !model_audio_started {
                    model_audio_started = true;
                    on_status("Gemini sta rispondendo...".to_string());
                }
                for chunk in output_audio {
                    if output_muted {
                        turn_audio_discarded = true;
                    } else if !turn_audio_discarded {
                        audio_playback.play(chunk);
                    }
                }
                if value
                    .pointer("/serverContent/interrupted")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    audio_playback.interrupt();
                    assistant_transcript.clear();
                    model_audio_started = false;
                }

                for call in function_calls(&value) {
                    let name = call.get("name").and_then(Value::as_str).unwrap_or_default();
                    if name == SUBMIT_COMMAND_TOOL {
                        let result = transcript_from_call(
                            call,
                            &transcript,
                            &recorder,
                            audio_bytes,
                            audio_peak,
                            &value,
                        )?;
                        on_command(result);
                        send_command_tool_response(&mut websocket, call).await?;
                        transcript.clear();

                        if stream_ended {
                            audio_playback.drain_and_stop();
                            let _ = websocket.close(None).await;
                            return Ok(LiveRun::Completed);
                        }
                    } else if crate::voice_tools::is_assistant_tool(name) {
                        let request = voice_tool_call(call)?;
                        on_status(format!("Eseguo {}...", request.name));
                        let execution = on_tool_call(request.clone())?
                            .recv_timeout(Duration::from_secs(5))
                            .map_err(|_| format!("Timeout esecuzione tool {}.", request.name))?;
                        match execution {
                            VoiceToolExecution::Immediate(response) => {
                                send_voice_tool_response(&mut websocket, &response).await?;
                            }
                            VoiceToolExecution::Pending { task_id, receiver } => {
                                on_status(format!(
                                    "{} in corso. Pax controllera' automaticamente.",
                                    request.name
                                ));
                                pending_tools.push(PendingVoiceTool {
                                    task_id,
                                    request,
                                    receiver,
                                });
                            }
                        }
                        transcript.clear();
                    } else {
                        let request = voice_tool_call(call)?;
                        let response = VoiceToolResult::error(
                            &request,
                            format!("Tool non supportato: {}", request.name),
                        );
                        send_voice_tool_response(&mut websocket, &response).await?;
                    }
                }

                if value.get("goAway").is_some() {
                    return Err("Gemini Live ha richiesto la chiusura della sessione.".to_string());
                }
                if stream_ended
                    && value
                        .pointer("/serverContent/turnComplete")
                        .and_then(Value::as_bool)
                    == Some(true)
                {
                    audio_playback.drain_and_stop();
                    let _ = websocket.close(None).await;
                    return Ok(LiveRun::Completed);
                }
                if value
                    .pointer("/serverContent/turnComplete")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    on_turn_complete();
                    let ready_status = if recorder_requested {
                        "In ascolto..."
                    } else {
                        "Pronto."
                    };
                    audio_playback.turn_complete(ready_status.to_string());
                    assistant_transcript.clear();
                    model_audio_started = false;
                    turn_audio_discarded = false;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
    }
}

struct PendingVoiceTool {
    task_id: String,
    request: VoiceToolCall,
    receiver: tokio::sync::oneshot::Receiver<crate::voice_tools::VoiceToolCompletion>,
}

async fn flush_pending_tool_results<S>(
    websocket: &mut tokio_tungstenite::WebSocketStream<S>,
    pending: &mut Vec<PendingVoiceTool>,
    on_status: &Arc<dyn Fn(String) + Send + Sync>,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut index = 0;
    while index < pending.len() {
        match pending[index].receiver.try_recv() {
            Ok(completion) => {
                let task = pending.swap_remove(index);
                on_status(format!("Task {} completato.", task.task_id));
                send_voice_tool_response(websocket, &completion.result).await?;
                let _ = completion.delivery_ack.send(());
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                index += 1;
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                let task = pending.swap_remove(index);
                let response = VoiceToolResult::error(
                    &task.request,
                    format!("Task {} terminato senza risultato.", task.task_id),
                );
                send_voice_tool_response(websocket, &response).await?;
            }
        }
    }
    Ok(())
}

fn live_url(api_key: &str) -> Result<url::Url, String> {
    let mut url = url::Url::parse(LIVE_ENDPOINT)
        .map_err(|err| format!("Endpoint Gemini Live non valido: {err}"))?;
    url.query_pairs_mut().append_pair("key", api_key);
    Ok(url)
}

fn live_setup(model: &str, voice: Option<&str>, context: &VoiceContext) -> Value {
    let target = context.panel_type.as_deref().unwrap_or("unknown");
    let workspace = context
        .workspace
        .as_ref()
        .and_then(|workspace| serde_json::to_string_pretty(workspace).ok())
        .unwrap_or_else(|| "unavailable".to_string());
    let instruction = format!(
        "{LIVE_SYSTEM_PROMPT}\nCurrent Pax target panel: {target}.\n\
         Interpret every utterance as input for this target.\n\
         Current authoritative Pax workspace snapshot:\n{workspace}"
    );
    let mut declarations = vec![serde_json::json!({
        "name": SUBMIT_COMMAND_TOOL,
        "description": "Submit explicit dictation or keyboard input to the focused non-terminal editor as a validated Pax voice command. Never use this tool for terminal panels.",
        "parameters": {
            "type": "OBJECT",
            "properties": {
                "transcript": {
                    "type": "STRING",
                    "description": "Verbatim transcript in the language spoken by the user."
                },
                "command": {
                    "type": "STRING",
                    "description": "One command using only the allowed Pax voice protocol forms."
                }
            },
            "required": ["transcript", "command"]
        }
    })];
    declarations.extend(crate::voice_tools::assistant_tool_declarations());
    let mut generation_config = serde_json::json!({
        "responseModalities": ["AUDIO"]
    });
    if let Some(voice) = voice.filter(|voice| !voice.trim().is_empty()) {
        generation_config["speechConfig"] = serde_json::json!({
            "voiceConfig": {
                "prebuiltVoiceConfig": {
                    "voiceName": voice
                }
            }
        });
    }
    serde_json::json!({
        "setup": {
            "model": format!("models/{}", model.trim_start_matches("models/")),
            "generationConfig": generation_config,
            "systemInstruction": {
                "parts": [{ "text": instruction }]
            },
            "inputAudioTranscription": {},
            "outputAudioTranscription": {},
            "realtimeInputConfig": {
                "automaticActivityDetection": {
                    "disabled": false,
                    "prefixPaddingMs": 160,
                    "silenceDurationMs": 650
                }
            },
            "tools": [{
                "functionDeclarations": declarations
            }]
        }
    })
}

async fn wait_for_setup<S>(
    websocket: &mut tokio_tungstenite::WebSocketStream<S>,
    cancelled: Arc<AtomicBool>,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let wait = async {
        loop {
            let Some(message) = websocket.next().await else {
                return Err("Gemini Live ha chiuso la connessione durante il setup.".to_string());
            };
            let message = message.map_err(|err| format!("Setup Gemini Live fallito: {err}"))?;
            if message_json(message)?.is_some_and(|value| value.get("setupComplete").is_some()) {
                return Ok(());
            }
        }
    };

    tokio::select! {
        result = tokio::time::timeout(SETUP_TIMEOUT, wait) => {
            result.map_err(|_| "Timeout setup Gemini Live.".to_string())?
        }
        _ = wait_for_flag(cancelled) => Err("Sessione Gemini Live annullata.".to_string()),
    }
}

async fn wait_for_flag(flag: Arc<AtomicBool>) {
    while !flag.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn send_text_input<S>(
    websocket: &mut tokio_tungstenite::WebSocketStream<S>,
    text: &str,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    websocket
        .send(Message::Text(
            serde_json::json!({
                "realtimeInput": { "text": text }
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|err| format!("Invio testo a Gemini Live fallito: {err}"))
}

async fn send_audio<S>(
    websocket: &mut tokio_tungstenite::WebSocketStream<S>,
    chunk: &[u8],
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let encoded = base64::engine::general_purpose::STANDARD.encode(chunk);
    let message = serde_json::json!({
        "realtimeInput": {
            "audio": {
                "data": encoded,
                "mimeType": AUDIO_MIME_TYPE
            }
        }
    });
    websocket
        .send(Message::Text(message.to_string().into()))
        .await
        .map_err(|err| format!("Invio audio a Gemini Live fallito: {err}"))
}

async fn send_audio_stream_end<S>(
    websocket: &mut tokio_tungstenite::WebSocketStream<S>,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    websocket
        .send(Message::Text(
            serde_json::json!({
                "realtimeInput": { "audioStreamEnd": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|err| format!("Chiusura stream Gemini Live fallita: {err}"))
}

async fn send_command_tool_response<S>(
    websocket: &mut tokio_tungstenite::WebSocketStream<S>,
    call: &Value,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut response = serde_json::json!({
        "name": SUBMIT_COMMAND_TOOL,
        "response": { "status": "ok" }
    });
    if let Some(id) = call.get("id").and_then(Value::as_str) {
        response["id"] = Value::String(id.to_string());
    }
    websocket
        .send(Message::Text(
            serde_json::json!({
                "toolResponse": { "functionResponses": [response] }
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|err| format!("Risposta tool Gemini Live fallita: {err}"))
}

async fn send_voice_tool_response<S>(
    websocket: &mut tokio_tungstenite::WebSocketStream<S>,
    result: &VoiceToolResult,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut response = serde_json::json!({
        "name": result.name,
        "response": result.response
    });
    if !result.call_id.is_empty() {
        response["id"] = Value::String(result.call_id.clone());
    }
    websocket
        .send(Message::Text(
            serde_json::json!({
                "toolResponse": { "functionResponses": [response] }
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|err| format!("Risposta tool Gemini Live fallita: {err}"))
}

fn message_json(message: Message) -> Result<Option<Value>, String> {
    match message {
        Message::Text(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|err| format!("Gemini Live ha restituito JSON non valido: {err}")),
        Message::Binary(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|err| format!("Gemini Live ha restituito dati non validi: {err}")),
        Message::Close(frame) => Err(format!(
            "Gemini Live ha chiuso la connessione{}",
            frame
                .map(|frame| format!(": {}", frame.reason))
                .unwrap_or_default()
        )),
        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => Ok(None),
    }
}

fn input_transcription(value: &Value) -> Option<&str> {
    value
        .pointer("/serverContent/inputTranscription/text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn output_transcription(value: &Value) -> Option<&str> {
    value
        .pointer("/serverContent/outputTranscription/text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn microphone_audio_enabled(model_audio_started: bool, playback_active: bool) -> bool {
    !model_audio_started && !playback_active
}

fn output_audio_chunks(value: &Value) -> Result<Vec<Vec<u8>>, String> {
    let Some(parts) = value
        .pointer("/serverContent/modelTurn/parts")
        .and_then(Value::as_array)
    else {
        return Ok(Vec::new());
    };

    parts
        .iter()
        .filter_map(|part| part.get("inlineData"))
        .filter(|data| {
            data.get("mimeType")
                .and_then(Value::as_str)
                .is_some_and(|mime| mime.starts_with("audio/pcm"))
        })
        .map(|data| {
            let encoded = data
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| "Gemini Live ha inviato audio senza dati.".to_string())?;
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| format!("Audio Gemini Live non valido: {error}"))
        })
        .collect()
}

fn playback_worker(
    receiver: std::sync::mpsc::Receiver<AudioPlaybackMessage>,
    on_status: Arc<dyn Fn(String) + Send + Sync>,
    playing: Arc<AtomicBool>,
) {
    let mut process = None;
    let mut prebuffer = PcmPrebuffer::default();
    while let Ok(message) = receiver.recv() {
        match message {
            AudioPlaybackMessage::Chunk(chunk) => {
                if let Some(audio) = prebuffer.push(chunk) {
                    if write_audio_chunk(&mut process, &audio, &on_status) {
                        playing.store(true, Ordering::SeqCst);
                    } else {
                        playing.store(false, Ordering::SeqCst);
                        prebuffer.reset();
                    }
                }
            }
            AudioPlaybackMessage::TurnComplete(ready_status) => {
                if let Some(audio) = prebuffer.finish_turn() {
                    if write_audio_chunk(&mut process, &audio, &on_status) {
                        playing.store(true, Ordering::SeqCst);
                    }
                }
                finish_audio_playback(&mut process);
                playing.store(false, Ordering::SeqCst);
                on_status(ready_status);
            }
            AudioPlaybackMessage::Interrupt => {
                prebuffer.reset();
                stop_audio_playback(&mut process);
                playing.store(false, Ordering::SeqCst);
            }
            AudioPlaybackMessage::DrainAndStop => {
                if let Some(audio) = prebuffer.finish_turn() {
                    let _ = write_audio_chunk(&mut process, &audio, &on_status);
                }
                finish_audio_playback(&mut process);
                playing.store(false, Ordering::SeqCst);
                break;
            }
            AudioPlaybackMessage::Stop => {
                prebuffer.reset();
                stop_audio_playback(&mut process);
                playing.store(false, Ordering::SeqCst);
                break;
            }
        }
    }
    playing.store(false, Ordering::SeqCst);
}

fn write_audio_chunk(
    process: &mut Option<AudioPlaybackProcess>,
    audio: &[u8],
    on_status: &Arc<dyn Fn(String) + Send + Sync>,
) -> bool {
    if process.is_none() {
        match spawn_audio_playback_process() {
            Ok(started) => *process = Some(started),
            Err(error) => {
                on_status(error);
                return false;
            }
        }
    }
    if process
        .as_mut()
        .is_some_and(|started| started.stdin.write_all(audio).is_ok())
    {
        return true;
    }
    stop_audio_playback(process);
    on_status("Riproduzione della voce Gemini interrotta.".to_string());
    false
}

fn spawn_audio_playback_process() -> Result<AudioPlaybackProcess, String> {
    let candidates: Vec<(&str, Vec<&str>)> = vec![
        (
            "pw-play",
            vec![
                "--rate",
                OUTPUT_AUDIO_RATE,
                "--channels",
                "1",
                "--channel-map",
                "MONO",
                "--format",
                "s16",
                "-",
            ],
        ),
        (
            "paplay",
            vec!["--raw", "--rate=24000", "--channels=1", "--format=s16le"],
        ),
        (
            "aplay",
            vec![
                "-q",
                "-t",
                "raw",
                "-f",
                "S16_LE",
                "-r",
                OUTPUT_AUDIO_RATE,
                "-c",
                "1",
            ],
        ),
        (
            "ffplay",
            vec![
                "-nodisp",
                "-autoexit",
                "-loglevel",
                "error",
                "-f",
                "s16le",
                "-ar",
                OUTPUT_AUDIO_RATE,
                "-ac",
                "1",
                "-",
            ],
        ),
        (
            "play",
            vec![
                "-q",
                "-t",
                "raw",
                "-e",
                "signed-integer",
                "-b",
                "16",
                "-r",
                OUTPUT_AUDIO_RATE,
                "-c",
                "1",
                "-",
            ],
        ),
    ];

    let mut errors = Vec::new();
    for (program, args) in candidates {
        let spawn = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawn {
            Ok(mut child) => {
                let Some(stdin) = child.stdin.take() else {
                    let _ = child.kill();
                    errors.push(format!("{program} non accetta audio da stdin"));
                    continue;
                };
                return Ok(AudioPlaybackProcess { child, stdin });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => errors.push(format!("{program}: {error}")),
        }
    }
    let details = if errors.is_empty() {
        String::new()
    } else {
        format!(" ({})", errors.join("; "))
    };
    Err(format!(
        "Voce Gemini ricevuta, ma nessun player PCM e' disponibile{details}."
    ))
}

fn stop_audio_playback(process: &mut Option<AudioPlaybackProcess>) {
    if let Some(mut process) = process.take() {
        drop(process.stdin);
        let _ = process.child.kill();
        let _ = process.child.wait();
    }
}

fn finish_audio_playback(process: &mut Option<AudioPlaybackProcess>) {
    if let Some(mut process) = process.take() {
        drop(process.stdin);
        let _ = process.child.wait();
    }
}

fn function_calls(value: &Value) -> impl Iterator<Item = &Value> {
    value
        .pointer("/toolCall/functionCalls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

#[cfg(test)]
fn submit_command_calls(value: &Value) -> impl Iterator<Item = &Value> {
    function_calls(value)
        .filter(|call| call.get("name").and_then(Value::as_str) == Some(SUBMIT_COMMAND_TOOL))
}

fn voice_tool_call(call: &Value) -> Result<VoiceToolCall, String> {
    let name = call
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "Gemini Live ha chiamato un tool senza nome.".to_string())?;
    Ok(VoiceToolCall {
        id: call
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        name: name.to_string(),
        arguments: call
            .get("args")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
    })
}

fn transcript_from_call(
    call: &Value,
    partial_transcript: &str,
    recorder: &str,
    audio_bytes: u64,
    audio_peak: f64,
    raw: &Value,
) -> Result<GeminiTranscript, String> {
    let args = call
        .get("args")
        .and_then(Value::as_object)
        .ok_or_else(|| "Gemini Live ha chiamato il tool senza argomenti.".to_string())?;
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| "Gemini Live non ha fornito il comando Pax.".to_string())?;
    let command = crate::voice_provider::normalize_protocol_phrase(command)?;
    let transcript = args
        .get("transcript")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .or_else(|| (!partial_transcript.trim().is_empty()).then_some(partial_transcript.trim()))
        .map(ToOwned::to_owned);

    Ok(GeminiTranscript {
        transcript,
        command,
        raw_text: raw.to_string(),
        recorder: recorder.to_string(),
        audio_bytes,
        audio_peak,
    })
}

fn merge_transcript(current: &mut String, update: &str) {
    let update = update.trim();
    if update.is_empty() || current == update {
        return;
    }
    if update.starts_with(current.as_str()) {
        *current = update.to_string();
    } else if !current.ends_with(update) {
        if !current.is_empty() && !current.ends_with(char::is_whitespace) {
            current.push(' ');
        }
        current.push_str(update);
    }
}

fn spawn_recorder(
    tx: mpsc::UnboundedSender<RecorderEvent>,
    cancelled: Arc<AtomicBool>,
    finish_requested: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    on_audio_level: Arc<dyn Fn(f64) + Send + Sync>,
    on_status: Arc<dyn Fn(String) + Send + Sync>,
) {
    std::thread::spawn(move || {
        let mut errors = Vec::new();
        for command in live_record_commands() {
            if cancelled.load(Ordering::SeqCst)
                || finish_requested.load(Ordering::SeqCst)
                || !enabled.load(Ordering::SeqCst)
            {
                if !enabled.load(Ordering::SeqCst) {
                    let _ = tx.send(RecorderEvent::Paused);
                    return;
                }
                let _ = tx.send(RecorderEvent::Stopped);
                return;
            }

            let name = command.name().to_string();
            on_status(format!("Streaming microfono con {name}..."));
            match stream_recorder(
                command,
                &tx,
                cancelled.clone(),
                finish_requested.clone(),
                enabled.clone(),
                on_audio_level.clone(),
            ) {
                Ok(true) => return,
                Ok(false) => errors.push(format!("{name} non ha prodotto audio")),
                Err(err) => errors.push(err),
            }
        }

        let message = if errors.is_empty() {
            "Nessun recorder Live trovato. Installa parecord, pw-record, ffmpeg, arecord o sox/rec."
                .to_string()
        } else {
            format!("Nessun recorder Live utilizzabile: {}", errors.join(" | "))
        };
        let _ = tx.send(RecorderEvent::Failed(message));
    });
}

fn stream_recorder(
    command: LiveRecordCommand,
    tx: &mpsc::UnboundedSender<RecorderEvent>,
    cancelled: Arc<AtomicBool>,
    finish_requested: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    on_audio_level: Arc<dyn Fn(f64) + Send + Sync>,
) -> Result<bool, String> {
    let name = command.name().to_string();
    let mut child = command.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{name} non espone lo stream audio"))?;
    let _ = tx.send(RecorderEvent::Started(name.clone()));
    let mut sent_audio = false;
    let mut pending_byte = None;
    let mut buffer = vec![0u8; 4096];

    loop {
        if cancelled.load(Ordering::SeqCst)
            || finish_requested.load(Ordering::SeqCst)
            || !enabled.load(Ordering::SeqCst)
        {
            interrupt_child(&mut child);
            let _ = child.wait();
            let event = if enabled.load(Ordering::SeqCst) {
                RecorderEvent::Stopped
            } else {
                RecorderEvent::Paused
            };
            let _ = tx.send(event);
            return Ok(true);
        }

        match stdout.read(&mut buffer) {
            Ok(0) => {
                let status = child
                    .wait()
                    .map_err(|err| format!("Attesa {name} fallita: {err}"))?;
                if sent_audio && status.success() {
                    let _ = tx.send(RecorderEvent::Stopped);
                    return Ok(true);
                }
                let stderr = read_child_stderr(&mut child);
                if status.success() {
                    return Ok(false);
                }
                return Err(if stderr.trim().is_empty() {
                    format!("{name} terminato con {status}")
                } else {
                    format!("{name}: {}", stderr.trim())
                });
            }
            Ok(read) => {
                let chunk = align_pcm_chunk(&mut pending_byte, &buffer[..read]);
                if chunk.is_empty() {
                    continue;
                }
                sent_audio = true;
                if enabled.load(Ordering::SeqCst) {
                    on_audio_level(audio_level_from_pcm(&chunk));
                }
                if tx.send(RecorderEvent::Chunk(chunk)).is_err() {
                    interrupt_child(&mut child);
                    let _ = child.wait();
                    return Ok(true);
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => {
                interrupt_child(&mut child);
                let _ = child.wait();
                return Err(format!("Lettura {name} fallita: {err}"));
            }
        }
    }
}

fn align_pcm_chunk(pending_byte: &mut Option<u8>, input: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(input.len() + usize::from(pending_byte.is_some()));
    if let Some(byte) = pending_byte.take() {
        chunk.push(byte);
    }
    chunk.extend_from_slice(input);
    if chunk.len() % 2 != 0 {
        *pending_byte = chunk.pop();
    }
    chunk
}

fn live_record_commands() -> Vec<LiveRecordCommand> {
    let mut commands = Vec::new();
    if let Ok(command) = std::env::var("PAX_VOICE_LIVE_RECORD_CMD") {
        if !command.trim().is_empty() {
            commands.push(LiveRecordCommand::Shell {
                name: "custom live recorder".to_string(),
                command,
            });
        }
    }
    if command_exists("pw-record") {
        commands.push(args_command(
            "pw-record",
            [
                "pw-record",
                "--rate",
                "16000",
                "--channels",
                "1",
                "--format",
                "s16",
                "-",
            ],
        ));
    }
    if command_exists("parecord") {
        commands.push(args_command(
            "parecord",
            [
                "parecord",
                "--record",
                "--device=@DEFAULT_SOURCE@",
                "--file-format=raw",
                "--rate=16000",
                "--format=s16le",
                "--channels=1",
            ],
        ));
    }
    if command_exists("ffmpeg") && !cfg!(target_os = "macos") {
        commands.push(args_command(
            "ffmpeg",
            [
                "ffmpeg",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "pulse",
                "-i",
                "default",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-f",
                "s16le",
                "pipe:1",
            ],
        ));
    }
    if command_exists("arecord") {
        commands.push(args_command(
            "arecord",
            [
                "arecord", "-q", "-t", "raw", "-f", "S16_LE", "-r", "16000", "-c", "1",
            ],
        ));
    }
    if command_exists("rec") {
        commands.push(args_command(
            "rec",
            [
                "rec",
                "-q",
                "-r",
                "16000",
                "-c",
                "1",
                "-b",
                "16",
                "-e",
                "signed-integer",
                "-t",
                "raw",
                "-",
            ],
        ));
    }
    commands
}

fn args_command<const N: usize>(name: &str, args: [&str; N]) -> LiveRecordCommand {
    LiveRecordCommand::Args {
        name: name.to_string(),
        args: args.into_iter().map(OsString::from).collect(),
    }
}

fn command_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .any(|dir| dir.join(name).is_file())
}

fn read_child_stderr(child: &mut Child) -> String {
    let mut stderr = String::new();
    if let Some(mut stream) = child.stderr.take() {
        let _ = stream.read_to_string(&mut stderr);
    }
    stderr
}

fn interrupt_child(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    #[cfg(not(unix))]
    let _ = child.kill();
}

fn audio_level_from_pcm(pcm: &[u8]) -> f64 {
    let mut sum_square = 0.0f64;
    let mut count = 0usize;
    for sample in pcm.chunks_exact(2) {
        let value = i16::from_le_bytes([sample[0], sample[1]]) as f64 / i16::MAX as f64;
        sum_square += value * value;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        ((sum_square / count as f64).sqrt() * 8.0).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_uses_live_model_context_transcription_and_tool() {
        let setup = live_setup(
            "gemini-3.1-flash-live-preview",
            Some("Kore"),
            &VoiceContext {
                panel_type: Some("terminal".to_string()),
                workspace: Some(serde_json::json!({
                    "name": "Test workspace",
                    "tab_groups": [{
                        "tabs": [{
                            "label": "FREEFLOW",
                            "panel_count": 2,
                            "panel_ids": ["p1", "p2"]
                        }]
                    }]
                })),
            },
        );

        assert_eq!(
            setup.pointer("/setup/model").and_then(Value::as_str),
            Some("models/gemini-3.1-flash-live-preview")
        );
        assert!(setup.pointer("/setup/inputAudioTranscription").is_some());
        assert!(setup.pointer("/setup/outputAudioTranscription").is_some());
        assert_eq!(
            setup
                .pointer("/setup/generationConfig/responseModalities/0")
                .and_then(Value::as_str),
            Some("AUDIO")
        );
        assert_eq!(
            setup
                .pointer(
                    "/setup/generationConfig/speechConfig/voiceConfig/prebuiltVoiceConfig/voiceName",
                )
                .and_then(Value::as_str),
            Some("Kore")
        );
        assert_eq!(
            setup
                .pointer("/setup/tools/0/functionDeclarations/0/name")
                .and_then(Value::as_str),
            Some(SUBMIT_COMMAND_TOOL)
        );
        assert_eq!(
            setup
                .pointer("/setup/tools/0/functionDeclarations/1/name")
                .and_then(Value::as_str),
            Some(crate::voice_tools::WORKSPACE_INSPECT_TOOL)
        );
        assert_eq!(
            setup
                .pointer("/setup/tools/0/functionDeclarations/2/name")
                .and_then(Value::as_str),
            Some(crate::voice_tools::WORKSPACE_SELECT_TAB_TOOL)
        );
        let instruction = setup
            .pointer("/setup/systemInstruction/parts/0/text")
            .and_then(Value::as_str)
            .unwrap();
        assert!(instruction.contains("terminal"));
        assert!(instruction.contains("Test workspace"));
        assert!(instruction.contains("\"panel_count\": 2"));
        assert!(instruction.contains("Use workspace_select_tab"));
        assert!(instruction.contains("no more than 15 spoken words"));
        assert!(instruction.contains("Avoid greetings"));
        assert!(!instruction.contains("- pax: seleziona tab"));
        assert_eq!(
            setup
                .pointer("/setup/tools/0/functionDeclarations")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(15)
        );
    }

    #[test]
    fn markdown_setup_exposes_document_tools() {
        let setup = live_setup(
            "gemini-3.1-flash-live-preview",
            None,
            &VoiceContext {
                panel_type: Some("markdown".to_string()),
                workspace: None,
            },
        );
        let names = setup
            .pointer("/setup/tools/0/functionDeclarations")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert_eq!(names.len(), 15);
        assert!(names.contains(&crate::voice_tools::WORKSPACE_SELECT_TAB_TOOL));
        assert!(names.contains(&crate::voice_tools::TERMINAL_READ_TOOL));
        assert!(names.contains(&crate::voice_tools::TERMINAL_WAIT_TOOL));
        assert!(names.contains(&crate::voice_tools::TASK_STATUS_TOOL));
        assert!(names.contains(&crate::voice_tools::MARKDOWN_READ_TOOL));
        assert!(names.contains(&crate::voice_tools::MARKDOWN_SEARCH_TOOL));
        assert!(names.contains(&crate::voice_tools::MARKDOWN_REPLACE_TOOL));
        assert!(names.contains(&crate::voice_tools::MARKDOWN_DELETE_LINE_TOOL));
        assert!(setup
            .pointer("/setup/generationConfig/speechConfig")
            .is_none());
    }

    #[test]
    fn parses_markdown_tool_call_arguments() {
        let call = serde_json::json!({
            "id": "call-md-1",
            "name": crate::voice_tools::MARKDOWN_REPLACE_TOOL,
            "args": { "query": "xxx", "replacement": "yy" }
        });

        let request = voice_tool_call(&call).unwrap();

        assert_eq!(request.id, "call-md-1");
        assert_eq!(request.name, crate::voice_tools::MARKDOWN_REPLACE_TOOL);
        assert_eq!(request.arguments["query"], "xxx");
    }

    #[test]
    fn extracts_output_transcription_and_pcm_audio() {
        let value = serde_json::json!({
            "serverContent": {
                "modelTurn": {
                    "parts": [{
                        "inlineData": {
                            "mimeType": "audio/pcm;rate=24000",
                            "data": base64::engine::general_purpose::STANDARD.encode([0, 1, 2, 3])
                        }
                    }]
                },
                "outputTranscription": { "text": "Sono l'assistente di Pax." }
            }
        });

        assert_eq!(
            output_transcription(&value),
            Some("Sono l'assistente di Pax.")
        );
        assert_eq!(output_audio_chunks(&value).unwrap(), vec![vec![0, 1, 2, 3]]);
    }

    #[test]
    fn tool_call_builds_validated_transcript() {
        let value = serde_json::json!({
            "toolCall": {
                "functionCalls": [{
                    "id": "call-1",
                    "name": SUBMIT_COMMAND_TOOL,
                    "args": {
                        "transcript": "scrivi elle esse e premi invio",
                        "command": "scrivi: ls tastiera: invio"
                    }
                }]
            }
        });
        let call = submit_command_calls(&value).next().unwrap();

        let result = transcript_from_call(call, "", "parecord", 3200, 0.4, &value).unwrap();

        assert_eq!(result.command, "scrivi: ls tastiera: invio");
        assert_eq!(
            result.transcript.as_deref(),
            Some("scrivi elle esse e premi invio")
        );
    }

    #[test]
    fn transcript_updates_accept_cumulative_and_delta_messages() {
        let mut transcript = String::new();
        merge_transcript(&mut transcript, "scrivi ciao");
        merge_transcript(&mut transcript, "scrivi ciao mondo");
        merge_transcript(&mut transcript, "e vai a capo");

        assert_eq!(transcript, "scrivi ciao mondo e vai a capo");
    }

    #[test]
    fn pcm_level_detects_signal() {
        let pcm: Vec<u8> = [0i16, 12000, -12000, 0]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect();

        assert!(audio_level_from_pcm(&pcm) > 0.1);
    }

    #[test]
    fn pcm_chunks_preserve_odd_byte_across_reads() {
        let mut pending = None;

        assert_eq!(align_pcm_chunk(&mut pending, &[1, 2, 3]), vec![1, 2]);
        assert_eq!(pending, Some(3));
        assert_eq!(align_pcm_chunk(&mut pending, &[4, 5, 6]), vec![3, 4, 5, 6]);
        assert_eq!(pending, None);
    }

    #[test]
    fn output_audio_is_prebuffered_and_short_turns_are_flushed() {
        let mut buffer = PcmPrebuffer::default();

        assert!(buffer.push(vec![0; PLAYBACK_PREBUFFER_BYTES - 1]).is_none());
        assert_eq!(
            buffer.push(vec![1]).unwrap().len(),
            PLAYBACK_PREBUFFER_BYTES
        );
        assert_eq!(buffer.push(vec![2, 3]).unwrap(), vec![2, 3]);
        assert!(buffer.finish_turn().is_none());

        assert!(buffer.push(vec![4, 5]).is_none());
        assert_eq!(buffer.finish_turn().unwrap(), vec![4, 5]);
    }

    #[test]
    fn output_audio_interruption_discards_the_prebuffer() {
        let mut buffer = PcmPrebuffer::default();
        assert!(buffer.push(vec![7; PLAYBACK_PREBUFFER_BYTES / 2]).is_none());

        buffer.reset();

        assert!(buffer.finish_turn().is_none());
        assert!(buffer.push(vec![8]).is_none());
    }

    #[test]
    fn microphone_audio_is_suppressed_during_generation_and_playback() {
        assert!(microphone_audio_enabled(false, false));
        assert!(!microphone_audio_enabled(true, false));
        assert!(!microphone_audio_enabled(false, true));
        assert!(!microphone_audio_enabled(true, true));
    }

    #[test]
    #[ignore = "uses the configured Gemini key and the system microphone"]
    fn configured_live_session_smoke_test() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let finish_requested = Arc::new(AtomicBool::new(false));
        let cancel_after_timeout = cancelled.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(8));
            cancel_after_timeout.store(true, Ordering::SeqCst);
        });
        let (control_tx, control_rx) = tokio::sync::mpsc::unbounded_channel();
        control_tx
            .send(crate::voice_session::VoiceSessionControl::SetMicrophoneEnabled(true))
            .unwrap();

        let result = run_gemini_live(
            cancelled,
            finish_requested,
            control_rx,
            Arc::new(|level| eprintln!("audio level: {level:.3}")),
            Arc::new(|status| eprintln!("status: {status}")),
            Arc::new(|transcript| eprintln!("partial transcript: {transcript}")),
            Arc::new(|transcript| eprintln!("assistant transcript: {transcript}")),
            Arc::new(|command| eprintln!("command: {}", command.command)),
            Arc::new(|| eprintln!("turn complete")),
            Arc::new(|_| Err("smoke test does not execute tools".to_string())),
            VoiceContext {
                panel_type: Some("markdown".to_string()),
                workspace: None,
            },
        );

        assert!(
            result.is_ok(),
            "configured Gemini Live session failed: {result:?}"
        );
    }

    #[test]
    #[ignore = "uses the configured Gemini key and the live service"]
    fn configured_live_tab_tool_and_conversation_smoke_test() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let api_key = crate::voice_settings::load_gemini_api_key()
            .expect("configured Gemini API key required");
        let model = crate::voice_settings::load_gemini_model();
        let voice = crate::voice_settings::load_gemini_voice();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime.block_on(async move {
            let (mut websocket, _) =
                tokio_tungstenite::connect_async(live_url(&api_key).unwrap().as_str())
                    .await
                    .unwrap();
            websocket
                .send(Message::Text(
                    live_setup(
                        &model,
                        voice.as_deref(),
                        &VoiceContext {
                            panel_type: Some("terminal".to_string()),
                            workspace: None,
                        },
                    )
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            wait_for_setup(&mut websocket, Arc::new(AtomicBool::new(false)))
                .await
                .unwrap();

            send_text_input(&mut websocket, "Seleziona il tab README")
                .await
                .unwrap();

            let first_turn = tokio::time::timeout(Duration::from_secs(20), async {
                let mut tool_called = false;
                let mut audio_bytes = 0usize;
                loop {
                    let message = websocket.next().await.unwrap().unwrap();
                    let Some(value) = message_json(message).unwrap() else {
                        continue;
                    };
                    audio_bytes += output_audio_chunks(&value)
                        .unwrap()
                        .iter()
                        .map(Vec::len)
                        .sum::<usize>();
                    for call in function_calls(&value) {
                        if call.get("name").and_then(Value::as_str)
                            == Some(crate::voice_tools::WORKSPACE_SELECT_TAB_TOOL)
                        {
                            tool_called = true;
                            let request = voice_tool_call(call).unwrap();
                            assert_eq!(request.arguments["tab_name"], "README");
                            send_voice_tool_response(
                                &mut websocket,
                                &VoiceToolResult {
                                    call_id: request.id,
                                    name: request.name,
                                    response: serde_json::json!({
                                        "status": "ok",
                                        "selected_tab": "README"
                                    }),
                                },
                            )
                            .await
                            .unwrap();
                        }
                    }
                    if value
                        .pointer("/serverContent/turnComplete")
                        .and_then(Value::as_bool)
                        == Some(true)
                        && tool_called
                    {
                        break (tool_called, audio_bytes);
                    }
                }
            })
            .await
            .expect("timeout waiting for tab tool confirmation");
            assert!(first_turn.0, "Gemini did not call workspace_select_tab");
            assert!(
                first_turn.1 > 0,
                "Gemini did not confirm the tool with audio"
            );

            let mut playback =
                LiveAudioPlayback::start(Arc::new(|status| eprintln!("playback: {status}")));
            send_text_input(&mut websocket, "Chi sei? Rispondi brevemente in italiano.")
                .await
                .unwrap();
            let second_turn_audio = tokio::time::timeout(Duration::from_secs(20), async {
                let mut audio_bytes = 0usize;
                let mut audio_chunks = 0usize;
                let mut last_audio_at = None;
                let mut max_audio_gap = Duration::ZERO;
                loop {
                    let message = websocket.next().await.unwrap().unwrap();
                    let Some(value) = message_json(message).unwrap() else {
                        continue;
                    };
                    let chunks = output_audio_chunks(&value).unwrap();
                    if !chunks.is_empty() {
                        let now = Instant::now();
                        if let Some(previous) = last_audio_at {
                            max_audio_gap = max_audio_gap.max(now.duration_since(previous));
                        }
                        last_audio_at = Some(now);
                        audio_chunks += chunks.len();
                        audio_bytes += chunks.iter().map(Vec::len).sum::<usize>();
                        for chunk in chunks {
                            playback.play(chunk);
                        }
                    }
                    if value
                        .pointer("/serverContent/turnComplete")
                        .and_then(Value::as_bool)
                        == Some(true)
                    {
                        break (audio_bytes, audio_chunks, max_audio_gap);
                    }
                }
            })
            .await
            .expect("timeout waiting for conversational response");
            playback.turn_complete("In ascolto...".to_string());
            eprintln!(
                "Gemini output: {} bytes in {} chunks, max inter-chunk gap {:?}",
                second_turn_audio.0, second_turn_audio.1, second_turn_audio.2
            );
            assert!(
                second_turn_audio.0 > 0,
                "Gemini did not answer the general question with audio"
            );
            playback.drain_and_stop();
            let _ = websocket.close(None).await;
        });
    }
}
