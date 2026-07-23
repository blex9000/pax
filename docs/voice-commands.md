# AI Assistant And Voice Commands

Open **AI Assistant** from the application header. Pax creates one persistent,
non-modal assistant window: it stays open while you focus and edit workspace
panels, and pressing the header button again brings the same window forward.
The **Chat** view accepts either microphone input or text from the composer;
both use the same Gemini Live session, context, and tools. The volume button
mutes only Gemini's spoken output while keeping response text visible. The
**Storico** view reloads persisted user, assistant, and tool messages for the
workspace.

General questions and global workspace actions do not require a focused panel.
For example:

```text
Seleziona il tab README.
Chi sei?
```

Tab selection is case-insensitive. An exact label wins; a unique partial label
is accepted. If a label is missing or ambiguous, Gemini explains the problem
and asks for a clarification. The assistant plays Gemini's voice response and
also displays its output transcription with an `AI:` prefix.

The workspace snapshot includes recursive tab groups, split trees, every panel
ID and type, visibility, focus, collapsed state, input-sync state, and active
tab selections. The assistant can focus, move, or expand panels, add horizontal
or vertical splits, add/rename/close tabs and panels, toggle zoom or synchronized
input, change a panel type, reset it, rename the workspace, and save it. Closing
a panel/tab and resetting a panel require explicit confirmation.

For a focused Markdown panel, Gemini can inspect and edit the current buffer
through dedicated tools. Examples of natural requests:

```text
Rimuovi l'ultima riga.
Leggi dalla riga 10 alla riga 25.
Trova la parola xxx nel testo.
Trova xxx e sostituiscila con yy.
```

The Markdown tools read numbered lines, report search positions, perform exact
literal replacements, and delete a numbered, last, or last non-empty line.
Edits switch the panel to Edit mode, remain undoable, and leave the document
dirty until it is saved normally.

For terminal panels, output is not included in the assistant's persistent
workspace context. Gemini requests a bounded slice of recent output only when a
task needs it. It can then type printable text or send individual terminal keys:
Enter, arrows, Tab, Shift+Tab, Space, Escape, paging/editing keys, and common
Ctrl combinations. This also supports interactive TUI programs such as Codex
and Claude Code:

```text
Nel terminale attivo esegui ls e dimmi cosa restituisce.
Leggi le ultime 30 righe del terminale di build.
In Codex seleziona la seconda opzione e conferma.
Interrompi il comando nel terminale attivo.
```

Terminal interactions are incremental: read the relevant recent output, send
the required text or key, and read again only when verification is necessary.
Pax does not continuously stream terminal output to Gemini.

Long-running and interactive operations are handled by the assistant task
supervisor. After sending a command or key, Gemini can ask Pax to wait for one
of four bounded conditions: return to the shell prompt, a new output revision,
a quiet output interval, or specific text. The Live WebSocket remains responsive
while the task is pending, and the activity tray shows the target panel,
elapsed time, and a cancel button. Cancellation stops monitoring; it does not
terminate the terminal process.

Each task has a deadline and is persisted in SQLite. Completion is delivered to
the original Gemini function call when that connection is still active. If the
connection was closed, Pax queues a provider-neutral host event and delivers it
when the assistant reconnects. Tasks left active by an application restart are
marked `interrupted`. Task lifecycle metadata is persisted, but terminal output
is never stored in the task record.

The task layer is provider-neutral. Gemini Live, Codex, Claude, and local
providers expose explicit continuation capabilities. Gemini 3.1 Live currently
uses a synchronous function call, so Pax keeps that call pending and confirms
delivery only after its response reaches the WebSocket. Host-event providers
can reconnect to the same persisted lifecycle without changing terminal or
panel execution code.

The assistant can also update a terminal's name, working directory, startup
commands, `before_close` script, minimum dimensions, and the enabled state of an
existing SSH configuration. Applying those settings restarts the panel backend
and therefore requires explicit confirmation.

Pax voice input is split into three layers:

1. A voice session emits status, audio level, partial transcript, command, completion, cancel, and failure events.
2. Gemini Live streams audio and can request global workspace or panel tools.
3. Pax executes immediate tools on the GTK thread and delegates waits to the task supervisor.
4. Panel adapters update native GTK buffers while preserving undo and dirty state.
5. Provider adapters deliver task completion through a matching function response or a host event.

The strict protocol below remains the internal format for literal dictation
and keyboard input in non-terminal editors. Tab navigation and terminal control
use dedicated tools instead.

## Protocol

Use one of these prefixes:

```text
scrivi: testo da inserire
scrivi letteralmente: testo che sembra un comando
va a capo
tastiera: invio
tastiera: freccia giu
tastiera: freccia su
tastiera: freccia destra
tastiera: freccia sinistra
tastiera: control c
```

Examples:

```text
scrivi: ieri sono andato al mare va a capo scrivi: poi ho preso un caffe
```

Markdown result:

```text
ieri sono andato al mare
poi ho preso un caffe
```

Terminal result:

```text
ieri sono andato al marepoi ho preso un caffe
```

For legacy custom providers targeting a terminal, `va a capo` is skipped. To
execute, emit:

```text
scrivi: ls -la tastiera: invio
```

To write command-looking words as text:

```text
scrivi letteralmente: pax seleziona tab terminale
```

## Transcript Provider

Pax uses an internal Rust Gemini Live provider automatically. It streams raw
PCM audio over a WebSocket, shows partial input transcription and audio level,
and receives validated commands through the `submit_voice_command` function.
Each command is executed through the same panel writer used by manual input.

The GTK assistant window does not own provider threads directly. It listens to
the session event stream. One Live session remains active across multiple
utterances until the microphone is stopped or the window is hidden.

For custom providers, advanced users can still set `PAX_VOICE_TRANSCRIBE_CMD`
as an override. The command must print one protocol phrase to stdout.

Minimal custom provider:

```bash
#!/usr/bin/env bash
printf '%s\n' 'scrivi: hello from voice tastiera: invio'
```

Make it executable:

```bash
chmod +x "$HOME/bin/pax-voice-transcribe"
```

## Provider Contract

Custom providers are free to use any recorder or STT service:

- Gemini Flash / Gemini API
- local Whisper / whisper.cpp
- Google Cloud Speech-to-Text
- a custom local model

Recommended behavior for a custom provider:

1. Record a short audio clip.
2. Send it to the STT/LLM provider with a prompt that requires the Pax protocol.
3. Print only the final protocol phrase to stdout.
4. Print errors to stderr and return non-zero on failure.

Prompt shape for an LLM-backed provider:

```text
Convert the user's audio into Pax voice protocol.
Return only one plain text command phrase.
Allowed prefixes:
- scrivi:
- scrivi letteralmente:
- va a capo
- tastiera:
- pax:
Never add explanations.
In terminal command examples, use "tastiera: invio" only when the user explicitly asks to execute.
```

The internal Gemini provider and any custom provider use the same output
contract, so Terminal, Markdown, and future panels do not need provider-specific code.

## Gemini Live Provider

The internal provider opens the Gemini Live WebSocket, waits for
`setupComplete`, then streams signed 16-bit little-endian mono PCM at 16 kHz.
Gemini receives the workspace and active Pax target panel as context, without
terminal output. Input transcriptions are displayed incrementally. Global
workspace, terminal, and Markdown function calls are executed by Pax; literal
dictation and non-terminal editor keyboard input use a validated protocol
command.

The implementation follows Google's raw WebSocket and Live API contracts:

```text
https://ai.google.dev/gemini-api/docs/live-api/get-started-websocket
https://ai.google.dev/api/live
```

Setup:

Open **Settings -> AI Assistant**, paste the Gemini API key, choose the Live
model, and select one of the available Gemini voices. The compact speaker menu
in the assistant window changes the same preference. A running Live session is
restarted when its voice changes because Gemini fixes the voice during setup.
The key and selected voice are saved locally in Pax app preferences.

Environment variables are still accepted as a fallback for scripted launches:

```bash
export GEMINI_API_KEY="..."
```

Optional settings:

```bash
export PAX_VOICE_GEMINI_MODEL="gemini-3.1-flash-live-preview"
export PAX_VOICE_GEMINI_VOICE="Kore"
```

`GOOGLE_GENAI_MODEL_NAME` is also accepted as a model-name alias. If both are
set, `PAX_VOICE_GEMINI_MODEL` wins.

Advanced custom provider override, not needed for normal Pax builds:

```bash
export PAX_VOICE_TRANSCRIBE_CMD="$HOME/bin/pax-voice-transcribe"
```

Recorder lookup for the internal provider:

- Linux: `pw-record`, then `parecord`, then `ffmpeg` with PulseAudio, then `arecord`, then `rec`.
- macOS: `rec` if available.
- Live override: `PAX_VOICE_LIVE_RECORD_CMD`; it must continuously write raw 16 kHz mono S16LE PCM to stdout.

While the microphone toggle is active, each detected pause can complete one
utterance and invoke one or more panel tools. Toggling the microphone off sends
`audioStreamEnd` but keeps the Gemini session available for typed messages;
toggling it on starts the recorder again. A text-only session does not open the
system microphone. Closing the assistant window hides it and cancels the current
Live connection.

Gemini response audio is played as 24 kHz mono S16LE PCM with a short prebuffer
to absorb WebSocket jitter. While Gemini is generating or its queued response
is playing, microphone frames are not sent to the service; this prevents speaker
echo from being mistaken for a user interruption. Wait for `In ascolto...`
before the next spoken request. The conversation shows recognized input, output
transcription, and tool outcomes, so recorder, STT, navigation, and edit failures
remain visible without exposing diagnostic controls in the normal assistant
interface.

Example custom recorder:

```bash
export PAX_VOICE_LIVE_RECORD_CMD='ffmpeg -hide_banner -loglevel error -f pulse -i default -ac 1 -ar 16000 -f s16le pipe:1'
```

Errors go to stderr and return non-zero, so Pax will not execute ambiguous text.
