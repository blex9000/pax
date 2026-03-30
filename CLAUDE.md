# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

MyTerms is a Rust/GTK4 terminal workspace manager (Tilix-style) with multi-panel layouts, heterogeneous panel types, and persistent storage. Documentation (README.md, ROADMAP.md) is in Italian.

## Build & Test Commands

```bash
# Build
cargo build                                    # Debug build (Linux, VTE4 enabled)
cargo build --release                          # Release build (Linux)
cargo build --release --no-default-features    # macOS (PTY fallback, no VTE4)
cargo build --features sourceview              # With GtkSourceView5 syntax highlighting

# Test
cargo test                         # All tests
cargo test --package tp-core       # Single crate
cargo test test_name               # Single test by name

# Run
cargo run -- new "name"            # New empty workspace
cargo run -- launch config.json    # Launch from JSON config
cargo run -- list                  # Recent workspaces
cargo run -- search "query"        # Search history/output
cargo run -- init -t template      # Generate template config

# Logging
RUST_LOG=tp_gui=debug cargo run -- new "test"
```

## Workspace Architecture (4 crates)

```
crates/
├── tp-cli/    CLI entry point (clap). Routes subcommands to core/gui.
├── tp-core/   Domain models & logic. Workspace/LayoutNode/PanelConfig structs,
│              JSON config loading, SSH config parsing, command safety blocklist,
│              alert rules, workspace templates.
├── tp-db/     SQLite persistence (rusqlite, bundled). Schema migrations, FTS5
│              full-text search on command history and saved output.
└── tp-gui/    GTK4 + libadwaita UI. Application lifecycle, layout engine,
               panel system, themes, dialogs, keybindings.
```

## Key Architectural Patterns

**Model-first UI**: All layout operations (split, tab, close, zoom) update the `LayoutNode` model tree first, then rebuild the GTK widget tree from the model. This ensures consistency and enables serialization.

**LayoutNode enum** (tp-core/src/workspace.rs): Recursive tree with variants `Panel { id }`, `Hsplit { children, ratios }`, `Vsplit { children, ratios }`, `Tabs { children, labels }`.

**PanelBackend trait** (tp-gui/src/panels/mod.rs): Plugin interface for panel types. Implementations: Terminal (VTE4/PTY), Markdown, Chooser. Each provides `widget()`, `on_focus()`, `write_input()`, `get_text_content()`.

**Backend factory** (tp-gui/src/backend_factory.rs): Creates panel backends from `PanelConfig` enum variants.

## Conditional Compilation

- **Feature `vte`** (default): Uses VTE4 terminal widget on Linux. Disable with `--no-default-features` for macOS PTY+vt100 fallback.
- **Feature `sourceview`** (optional): GtkSourceView5 for syntax highlighting in markdown edit mode. Falls back to plain TextView.

## Data Storage

- Database: `~/.local/share/myterms/myterms.db` (SQLite3 with FTS5)
- Log file: `~/.local/share/myterms/myterms.log`
- Migrations: `migrations/001_initial.sql`
- Theme CSS: `resources/style.css`
- Example configs: `config/*.json`
