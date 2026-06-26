# Voice Commands

Pax voice input is split into two layers:

1. A transcript provider records/transcribes audio and prints text.
2. Pax parses that text with a strict command protocol and executes only known actions.

This keeps dictation and commands explicit. In a terminal, `scrivi:` never presses Enter.

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
pax: seleziona tab nome
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

The terminal skips `va a capo` intentionally. To execute, say:

```text
scrivi: ls -la tastiera: invio
```

To write command-looking words as text:

```text
scrivi letteralmente: pax seleziona tab terminale
```

## Transcript Hook

Set `PAX_VOICE_TRANSCRIBE_CMD` to enable the `Trascrivi` button in the voice popover.
The command must print one protocol phrase to stdout.

```bash
export PAX_VOICE_TRANSCRIBE_CMD="$HOME/bin/pax-voice-transcribe"
```

Minimal test hook:

```bash
#!/usr/bin/env bash
printf '%s\n' 'scrivi: hello from voice tastiera: invio'
```

Make it executable:

```bash
chmod +x "$HOME/bin/pax-voice-transcribe"
```

## Provider Scripts

Provider scripts are free to use any recorder or STT service:

- Gemini Flash / Gemini API
- local Whisper / whisper.cpp
- Google Cloud Speech-to-Text
- a custom local model

Recommended behavior for a provider script:

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

The native in-app Gemini backend should use the same output contract, so Terminal, Markdown,
and future panels do not need provider-specific code.

## Gemini Provider Script

The repo includes a stdlib-only Gemini provider:

```bash
scripts/pax-voice-transcribe-gemini.py
```

It records a short audio clip, sends it to Gemini with inline audio data, validates
that the model returned a Pax protocol phrase, and prints only that phrase to stdout.
It follows the Gemini audio `generateContent` contract documented by Google:

```text
https://ai.google.dev/gemini-api/docs/generate-content/audio
```

Setup:

```bash
export GEMINI_API_KEY="..."
export PAX_VOICE_TRANSCRIBE_CMD="/home/xb/dev/me/pax/scripts/pax-voice-transcribe-gemini.py"
```

Optional settings:

```bash
export PAX_VOICE_GEMINI_MODEL="gemini-3-flash-preview"
export PAX_VOICE_RECORD_SECONDS="6"
```

`GOOGLE_GENAI_MODEL_NAME` is also accepted as a model-name alias. If both are
set, `PAX_VOICE_GEMINI_MODEL` wins.

Test with an existing audio file:

```bash
scripts/pax-voice-transcribe-gemini.py --audio /tmp/sample.wav
```

Recorder behavior:

- Linux: the script tries `arecord`, then `ffmpeg` with PulseAudio, then `rec`.
- macOS: configure a custom recorder command unless `rec` is installed and configured.
- Custom commands use shell-escaped `{output}` and `{duration}` placeholders.

Example custom recorder:

```bash
export PAX_VOICE_RECORD_CMD='ffmpeg -hide_banner -loglevel error -y -f pulse -i default -t {duration} -ac 1 -ar 16000 {output}'
```

Errors go to stderr and return non-zero, so Pax will not execute ambiguous text.
