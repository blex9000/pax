# Repository Guidelines

## Project Structure & Module Organization
Pax is a Rust workspace rooted at `Cargo.toml`. The main code lives under `crates/`: `tp-core/` for shared models and config loading, `tp-db/` for SQLite persistence and migrations, `tp-gui/` for the GTK4/libadwaita app, and `tp-cli/` for the `pax` binary entry point. Use Cargo package names (`pax-core`, `pax-gui`, `pax-db`, `pax`) in commands, even though the on-disk directories are prefixed `tp-`. Supporting files live in `resources/` (CSS, fonts, icons), `migrations/` (SQL schema), `config/` (sample workspace data), `docs/`, and `scripts/`.

## Build, Test, and Development Commands
Use `cargo build` for the default Linux debug build and `cargo build --release` for optimized binaries. Run the app locally with `cargo run -- new "scratch"` or `cargo run -- launch config/workspace_save_config.json`. Execute the full test suite with `cargo test`; scope to a crate or feature area when iterating, for example `cargo test --package pax-gui file_watcher`. Linux packaging uses `./scripts/build-appimage.sh`; macOS bundle builds use `./scripts/build-macos.sh`.

## Coding Style & Naming Conventions
Follow Rust 2021 conventions: 4-space indentation, `snake_case` for modules/functions/files, and `PascalCase` for types and enums. Format before submitting with `cargo fmt --all`. Run `cargo clippy --all-targets --all-features` for nontrivial changes; the codebase already carries a few targeted Clippy allows. Keep comments brief and purposeful. When touching layout behavior, preserve the existing model-first pattern: update the layout data structure, then rebuild or refresh GTK widgets from it.

## Testing Guidelines
Prefer inline unit tests in the same file using `#[cfg(test)] mod tests`. Add focused coverage for parsing, layout mutations, database behavior, and editor/terminal regressions near the changed module. GTK/glib-sensitive tests in `pax-gui` use `serial_test`; keep those tests isolated and deterministic.

## Commit & Pull Request Guidelines
Match the existing commit style: `<area>: <imperative summary>` such as `workspace_view: preserve tab selection across zoom/unzoom`. PRs should describe user-visible behavior, note Linux/macOS feature-flag impacts, list the Cargo commands you ran, and include screenshots for UI changes. Link the relevant issue, roadmap item, or design note when the change follows a planned feature.
