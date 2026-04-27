//! Inline help for the Markdown notebook feature — opened from the
//! Markdown panel toolbar's `?` button.

use gtk4::prelude::*;

pub const HELP_TEXT: &str = r#"# Markdown Notebook — quick help

Mark a fenced code block with an exec tag to run it inline.

```python run
print("hello")
```

```bash watch=5s
ps aux | head
```

Tags
  • python | bash | sh
  • run / once       — auto-runs once on first render; click ▶ to re-run
  • watch=Ns | Nm | Nms — cyclic; auto-starts when the panel is visible
  • timeout=Ns       — wall-clock cap (default 30s for run/once)
  • confirm          — block the auto-run until ▶ is clicked once
                       (v1: the manual ▶ click confirms silently)

Rich output (Python)
  import pax
  pax.show("/tmp/foo.png")
  pax.show_plot(plt)   # matplotlib

Markers (any language)
  print("<<pax:image:/abs/path.png>>")

Safety
  A small blocklist (rm -rf /, mkfs, fork bombs, shutdown, …) blocks
  obvious destructive commands on bash/sh cells (Python is excluded
  to avoid false positives like executor.shutdown()). Otherwise cells
  run with your user privileges — only open trusted notebooks.

Output is in-memory only — closing the panel discards it.
"#;

pub fn show(parent: &gtk4::Widget) {
    let window = gtk4::Window::new();
    window.set_default_size(640, 520);
    window.set_title(Some("Markdown Notebook — Help"));
    if let Some(root) = parent.root() {
        if let Ok(w) = root.downcast::<gtk4::Window>() {
            window.set_transient_for(Some(&w));
        }
    }
    crate::theme::configure_dialog_window(&window);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);
    let tv = gtk4::TextView::new();
    tv.set_editable(false);
    tv.set_cursor_visible(false);
    tv.set_wrap_mode(gtk4::WrapMode::Word);
    tv.set_monospace(true);
    tv.set_left_margin(12);
    tv.set_right_margin(12);
    tv.set_top_margin(8);
    tv.set_bottom_margin(8);
    tv.buffer().set_text(HELP_TEXT);
    scroll.set_child(Some(&tv));
    window.set_child(Some(&scroll));
    window.present();
}
