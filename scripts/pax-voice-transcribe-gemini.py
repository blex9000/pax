#!/usr/bin/env python3
"""Gemini-backed transcript provider for Pax voice commands.

The script records a short audio clip, asks Gemini to convert it into the Pax
voice protocol, and prints only the resulting protocol phrase to stdout.
"""

from __future__ import annotations

import argparse
import base64
import json
import mimetypes
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path


DEFAULT_MODEL = "gemini-3.5-flash"
DEFAULT_DURATION_SECONDS = 6.0
MAX_INLINE_AUDIO_BYTES = 15 * 1024 * 1024

PAX_PROTOCOL_PROMPT = """Convert the user's audio into Pax voice protocol.

Return only one plain text command phrase. No markdown, no JSON, no quotes, no
explanations.

Allowed command forms:
- scrivi: text to insert
- scrivi letteralmente: text that looks like a command but must be inserted
- va a capo
- tastiera: invio
- tastiera: freccia giu
- tastiera: freccia su
- tastiera: freccia destra
- tastiera: freccia sinistra
- tastiera: control c
- pax: seleziona tab tab name

Rules:
- For normal dictation, always use "scrivi: ...".
- If the user says to write, dictate, or type text, use "scrivi: ...".
- If the user says new line, a capo, or vai a capo, use "va a capo".
- If the user asks to press Enter, run, execute, or send a terminal command,
  use "tastiera: invio".
- Do not add "tastiera: invio" unless the user explicitly asks to execute,
  send, press Enter, or submit.
- Keyboard arrows must use "tastiera: freccia giu", "tastiera: freccia su",
  "tastiera: freccia destra", or "tastiera: freccia sinistra".
- Pax app actions must use "pax: ...".
- Prefer Italian protocol words exactly as listed above.
"""

ALLOWED_STARTS = (
    "scrivi:",
    "scrivi letteralmente:",
    "va a capo",
    "tastiera:",
    "pax:",
)


class VoiceProviderError(RuntimeError):
    pass


def duration_value(value: str) -> float:
    try:
        duration = float(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("duration must be a number") from error
    if duration <= 0:
        raise argparse.ArgumentTypeError("duration must be greater than zero")
    return duration


def default_model() -> str:
    return (
        os.environ.get("PAX_VOICE_GEMINI_MODEL")
        or os.environ.get("GOOGLE_GENAI_MODEL_NAME")
        or DEFAULT_MODEL
    )


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Record/transcribe audio into Pax voice protocol with Gemini.",
    )
    parser.add_argument(
        "--audio",
        help="Use an existing audio file instead of recording a new clip.",
    )
    parser.add_argument(
        "--duration",
        type=duration_value,
        default=os.environ.get("PAX_VOICE_RECORD_SECONDS", str(DEFAULT_DURATION_SECONDS)),
        help="Recording duration in seconds. Defaults to PAX_VOICE_RECORD_SECONDS or 6.",
    )
    parser.add_argument(
        "--mime-type",
        help="Audio MIME type. Defaults to guessing from the file extension.",
    )
    parser.add_argument(
        "--model",
        default=default_model(),
        help=(
            "Gemini model name. Defaults to PAX_VOICE_GEMINI_MODEL, "
            "GOOGLE_GENAI_MODEL_NAME, or gemini-3.5-flash."
        ),
    )
    parser.add_argument(
        "--keep-audio",
        action="store_true",
        help="Keep the temporary recorded audio file for debugging.",
    )
    return parser.parse_args(argv)


def api_key_from_env() -> str:
    api_key = os.environ.get("GEMINI_API_KEY") or os.environ.get("GOOGLE_API_KEY")
    if not api_key:
        raise VoiceProviderError("Set GEMINI_API_KEY or GOOGLE_API_KEY.")
    return api_key


def quote_placeholder(value: object) -> str:
    return shlex.quote(str(value))


def custom_record_command(output_path: Path, duration: float) -> str | None:
    template = os.environ.get("PAX_VOICE_RECORD_CMD")
    if not template:
        return None
    return template.format(
        output=quote_placeholder(output_path),
        duration=quote_placeholder(duration),
    )


def default_record_command(output_path: Path, duration: float) -> list[str] | None:
    duration_arg = f"{duration:g}"

    if shutil.which("arecord"):
        return [
            "arecord",
            "-q",
            "-d",
            str(max(1, int(round(duration)))),
            "-f",
            "S16_LE",
            "-r",
            "16000",
            "-c",
            "1",
            str(output_path),
        ]

    if sys.platform != "darwin" and shutil.which("ffmpeg"):
        return [
            "ffmpeg",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "pulse",
            "-i",
            "default",
            "-t",
            duration_arg,
            "-ac",
            "1",
            "-ar",
            "16000",
            str(output_path),
        ]

    if shutil.which("rec"):
        return [
            "rec",
            "-q",
            "-r",
            "16000",
            "-c",
            "1",
            str(output_path),
            "trim",
            "0",
            duration_arg,
        ]

    return None


def run_record_command(command: str | list[str]) -> None:
    if isinstance(command, str):
        result = subprocess.run(
            command,
            shell=True,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    else:
        result = subprocess.run(
            command,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    if result.returncode != 0:
        stderr = result.stderr.strip() or result.stdout.strip()
        raise VoiceProviderError(f"Recorder failed: {stderr}")


def record_audio(duration: float) -> Path:
    handle = tempfile.NamedTemporaryFile(prefix="pax-voice-", suffix=".wav", delete=False)
    handle.close()
    output_path = Path(handle.name)

    command = custom_record_command(output_path, duration)
    if command is None:
        command = default_record_command(output_path, duration)

    if command is None:
        output_path.unlink(missing_ok=True)
        raise VoiceProviderError(
            "No recorder found. Install arecord, ffmpeg, or sox/rec, "
            "or set PAX_VOICE_RECORD_CMD."
        )

    run_record_command(command)

    if not output_path.exists() or output_path.stat().st_size == 0:
        output_path.unlink(missing_ok=True)
        raise VoiceProviderError("Recorder produced an empty audio file.")

    return output_path


def guess_mime_type(audio_path: Path, override: str | None) -> str:
    if override:
        return override
    guessed, _ = mimetypes.guess_type(str(audio_path))
    return guessed or "audio/wav"


def read_audio_base64(audio_path: Path) -> str:
    data = audio_path.read_bytes()
    if not data:
        raise VoiceProviderError("Audio file is empty.")
    if len(data) > MAX_INLINE_AUDIO_BYTES:
        raise VoiceProviderError(
            "Audio file is too large for inline Gemini input. "
            "Use a shorter clip or add Files API support."
        )
    return base64.b64encode(data).decode("ascii")


def gemini_request(api_key: str, model: str, mime_type: str, audio_b64: str) -> dict:
    url = f"https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
    payload = {
        "contents": [
            {
                "parts": [
                    {"text": PAX_PROTOCOL_PROMPT},
                    {
                        "inline_data": {
                            "mime_type": mime_type,
                            "data": audio_b64,
                        }
                    },
                ]
            }
        ],
        "generation_config": {
            "temperature": 0,
        },
    }
    body = json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=body,
        headers={
            "Content-Type": "application/json",
            "x-goog-api-key": api_key,
        },
        method="POST",
    )

    try:
        with urllib.request.urlopen(request, timeout=45) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        error_body = error.read().decode("utf-8", errors="replace")
        raise VoiceProviderError(f"Gemini HTTP {error.code}: {error_body}") from error
    except urllib.error.URLError as error:
        raise VoiceProviderError(f"Gemini request failed: {error}") from error
    except json.JSONDecodeError as error:
        raise VoiceProviderError(f"Gemini returned invalid JSON: {error}") from error


def extract_text(response: dict) -> str:
    if isinstance(response.get("text"), str):
        return response["text"]

    texts: list[str] = []
    for candidate in response.get("candidates", []):
        content = candidate.get("content", {})
        for part in content.get("parts", []):
            text = part.get("text")
            if isinstance(text, str):
                texts.append(text)

    text = "\n".join(part.strip() for part in texts if part.strip()).strip()
    if not text:
        raise VoiceProviderError(f"Gemini response did not include text: {response}")
    return text


def strip_code_fence(text: str) -> str:
    stripped = text.strip()
    if not stripped.startswith("```"):
        return stripped

    lines = stripped.splitlines()
    if lines and lines[0].startswith("```"):
        lines = lines[1:]
    if lines and lines[-1].startswith("```"):
        lines = lines[:-1]
    return "\n".join(lines).strip()


def maybe_json_command(text: str) -> str:
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError:
        return text

    if isinstance(parsed, str):
        return parsed
    if isinstance(parsed, dict):
        for key in ("command", "text", "protocol"):
            value = parsed.get(key)
            if isinstance(value, str):
                return value
    return text


def normalize_protocol_phrase(text: str) -> str:
    phrase = maybe_json_command(strip_code_fence(text))
    phrase = phrase.strip().strip("\"'")

    lines = []
    for line in phrase.splitlines():
        line = line.strip()
        line = line.removeprefix("-").strip()
        if line:
            lines.append(line)
    phrase = " ".join(lines).strip()

    lower = phrase.lower()
    if not any(lower.startswith(prefix) for prefix in ALLOWED_STARTS):
        raise VoiceProviderError(f"Gemini returned non-protocol text: {phrase!r}")
    return phrase


def main(argv: list[str]) -> int:
    recorded_path: Path | None = None
    args: argparse.Namespace | None = None

    try:
        args = parse_args(argv)
        audio_path = Path(args.audio).expanduser() if args.audio else None
        api_key = api_key_from_env()
        if audio_path is None:
            recorded_path = record_audio(args.duration)
            audio_path = recorded_path
        elif not audio_path.exists():
            raise VoiceProviderError(f"Audio file does not exist: {audio_path}")

        mime_type = guess_mime_type(audio_path, args.mime_type)
        audio_b64 = read_audio_base64(audio_path)
        response = gemini_request(api_key, args.model, mime_type, audio_b64)
        phrase = normalize_protocol_phrase(extract_text(response))
        print(phrase)
        return 0
    except VoiceProviderError as error:
        print(f"pax voice provider error: {error}", file=sys.stderr)
        return 1
    finally:
        if recorded_path and args and not args.keep_audio:
            recorded_path.unlink(missing_ok=True)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
