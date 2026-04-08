# Build Pax

## Prerequisiti

### Linux (Ubuntu/Mint/Debian)
```bash
sudo apt install libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev libgtksourceview-5-dev
```

### macOS
```bash
brew install gtk4 libadwaita vte4 gtksourceview5
```

## Build locale

```bash
# Debug (Linux, tutte le feature)
cargo build

# Release
cargo build --release

# macOS (no VTE, fallback PTY)
cargo build --release --no-default-features --features sourceview

# Con syntax highlighting
cargo build --features sourceview
```

## Esecuzione

```bash
# Nuovo workspace vuoto
cargo run -- new "nome"

# Aprire un workspace da file
cargo run -- launch config.json

# Lista workspace recenti
cargo run -- list

# Con logging
RUST_LOG=pax_gui=debug cargo run -- new "test"
```

## Log e debug

Il file `pax.log` viene creato nella directory da cui si lancia l'app.

```bash
# Visualizzare log in tempo reale
tail -f pax.log

# Specificare una directory diversa per i log
pax --log-dir /tmp new "test"

# Log verbose su terminale
RUST_LOG=pax_gui=trace ./target/debug/pax new "test"
```

### Crash

- **Panic Rust**: il panic handler scrive `=== PAX CRASH ===` con backtrace in `pax.log`
- **Segfault** (crash nativo GTK/VTE): il log si interrompe senza crash marker. Per diagnosticare:
  ```bash
  ulimit -c unlimited
  ./target/debug/pax new "test"
  # Dopo il crash, analizzare il coredump:
  coredumpctl debug pax
  ```

## AppImage (Linux)

```bash
./scripts/build-appimage.sh
```

Produce `Pax-x86_64.AppImage` nella root del progetto.

Requisiti aggiuntivi: `curl` (per scaricare linuxdeploy).

Lo script:
1. Compila release binary
2. Scarica linuxdeploy + plugin GTK4
3. Bundla librerie, icone, GtkSourceView5 styles, tema Adwaita
4. Patcha il plugin GTK (rimuove forzature `GTK_THEME` e `GDK_BACKEND=x11`)
5. Genera l'AppImage

## macOS App Bundle

```bash
./scripts/build-macos.sh
```

Produce `Pax.app` in `target/release/bundle/`.

## Versione

La versione è definita in `Cargo.toml` (workspace root):
```toml
version = "0.1.0"
```

Tutti i crate ereditano la stessa versione via `version.workspace = true`.

La versione è visibile nel menu: **Menu → About Pax**.

Per rilasciare una nuova versione:
1. Aggiornare `version` in `Cargo.toml` (root)
2. Committare
3. Tag git: `git tag v0.2.0`
4. Rebuild AppImage/macOS bundle
