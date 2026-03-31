---
name: macOS RAM usage not implemented
description: Status bar RAM indicator uses /proc/self/status (Linux-only) — needs macOS equivalent via mach_task_info
type: project
---

The status bar RAM usage label reads VmRSS from `/proc/self/status`, which only exists on Linux. On macOS it silently shows nothing.

**Why:** Cross-platform support is a project goal (macOS via `--no-default-features`).

**How to apply:** When working on macOS compatibility or the status bar, implement the macOS path using `mach_task_basic_info` / `task_info()` from the `mach` crate (or raw FFI to `libproc`). Gate with `#[cfg(target_os = "macos")]` / `#[cfg(target_os = "linux")]`.
