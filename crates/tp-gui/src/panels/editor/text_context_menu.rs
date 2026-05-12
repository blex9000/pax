//! Generic right-click context menu for code/diff/preview SourceViews.
//!
//! Replaces GTK's default unstyled popup with a `app-popover`-themed menu and
//! lets callers add context-specific items (e.g. format current file).

use std::path::Path;
use std::process::{Command, Stdio};

use gtk4::prelude::*;
use sourceview5::prelude::*;

const CONTEXT_MENU_BUTTON: u32 = 3;
const FORMATTER_TIMEOUT_SECS: u64 = 8;

/// One item in the context menu.
pub enum TextContextMenuItem {
    Button {
        icon: &'static str,
        label: String,
        hint: Option<String>,
        on_click: Box<dyn Fn() + 'static>,
    },
}

impl TextContextMenuItem {
    pub fn button(
        icon: &'static str,
        label: impl Into<String>,
        hint: Option<&str>,
        on_click: impl Fn() + 'static,
    ) -> Self {
        TextContextMenuItem::Button {
            icon,
            label: label.into(),
            hint: hint.map(|s| s.to_string()),
            on_click: Box::new(on_click),
        }
    }
}

/// Install our styled right-click menu on `view`. The gesture is attached to
/// the `host` ScrolledWindow with capture phase, intercepting the event before
/// it reaches the view's own internal popup gesture.
///
/// The buffer is read from `view` at click time (not at install time): the
/// editor's main view swaps buffers as the user changes tabs, so capturing the
/// install-time buffer would freeze the menu on the initial empty buffer and
/// hide language-aware items like Comment/Uncomment.
///
/// `extras_factory` is invoked every time the menu opens and receives the
/// 0-based buffer line the click landed on, so context-specific items (e.g.
/// format current file, add/edit notes on the clicked line) can reflect the
/// current click target.
pub fn install(
    host: &gtk4::ScrolledWindow,
    view: &sourceview5::View,
    editable: bool,
    extras_factory: impl Fn(i32) -> Vec<TextContextMenuItem> + 'static,
) {
    remove_builtin_context_gesture(view);

    let gesture = gtk4::GestureClick::new();
    gesture.set_button(CONTEXT_MENU_BUTTON);
    gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);

    let view_cell = view.clone();
    let host_cell = host.clone();
    let extras_factory = std::rc::Rc::new(extras_factory);

    gesture.connect_pressed(move |g, _n, x, y| {
        g.set_state(gtk4::EventSequenceState::Claimed);

        let Ok(buffer) = view_cell.buffer().downcast::<sourceview5::Buffer>() else {
            return;
        };

        // Convert widget -> buffer coords. For clicks in the gutter
        // (left of the text area) buf_x goes negative and
        // `iter_at_location` returns None, which would wrongly default
        // the line to 0 and show Add/Edit/Delete entries for the first
        // line regardless of where the user actually clicked. Use
        // `line_at_y` (takes just the y coord) so the click line
        // resolves correctly whether the click is on the text, the
        // gutter, or the line-marks column.
        let (_, buf_y) =
            view_cell.window_to_buffer_coords(gtk4::TextWindowType::Widget, x as i32, y as i32);
        let (iter, _) = view_cell.line_at_y(buf_y);
        let click_line = iter.line();

        let popover = build_menu(&view_cell, &buffer, editable, extras_factory(click_line));

        popover.set_parent(&host_cell);
        popover.connect_closed(|popover| {
            if popover.parent().is_some() {
                popover.unparent();
            }
        });
        popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    host.add_controller(gesture);
}

fn build_menu(
    view: &sourceview5::View,
    buffer: &sourceview5::Buffer,
    editable: bool,
    extras: Vec<TextContextMenuItem>,
) -> gtk4::PopoverMenu {
    let popover = gtk4::PopoverMenu::from_model(None::<&gtk4::gio::MenuModel>);
    crate::theme::configure_popover(&popover);

    let menu_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    menu_box.set_margin_top(4);
    menu_box.set_margin_bottom(4);
    menu_box.set_margin_start(4);
    menu_box.set_margin_end(4);

    if editable {
        append_buffer_action(
            &menu_box,
            &popover,
            "edit-undo-symbolic",
            "Undo",
            "Ctrl+Z",
            buffer.can_undo(),
            {
                let b = buffer.clone();
                move || {
                    if b.can_undo() {
                        b.undo();
                    }
                }
            },
        );
        append_buffer_action(
            &menu_box,
            &popover,
            "edit-redo-symbolic",
            "Redo",
            "Ctrl+Shift+Z",
            buffer.can_redo(),
            {
                let b = buffer.clone();
                move || {
                    if b.can_redo() {
                        b.redo();
                    }
                }
            },
        );
        append_separator(&menu_box);
        append_view_action(
            &menu_box,
            &popover,
            view,
            "edit-cut-symbolic",
            "Cut",
            "Ctrl+X",
            "clipboard.cut",
        );
    }
    append_view_action(
        &menu_box,
        &popover,
        view,
        "edit-copy-symbolic",
        "Copy",
        "Ctrl+C",
        "clipboard.copy",
    );
    if editable {
        append_view_action(
            &menu_box,
            &popover,
            view,
            "edit-paste-symbolic",
            "Paste",
            "Ctrl+V",
            "clipboard.paste",
        );
    }
    append_separator(&menu_box);

    append_buffer_action(
        &menu_box,
        &popover,
        "edit-select-all-symbolic",
        "Select All",
        "Ctrl+A",
        true,
        {
            let b = buffer.clone();
            move || {
                let (start, end) = b.bounds();
                b.select_range(&start, &end);
            }
        },
    );

    if editable {
        append_separator(&menu_box);
        append_change_case(
            &menu_box,
            &popover,
            buffer,
            sourceview5::ChangeCaseType::Upper,
            "UPPER CASE",
        );
        append_change_case(
            &menu_box,
            &popover,
            buffer,
            sourceview5::ChangeCaseType::Lower,
            "lower case",
        );
        append_change_case(
            &menu_box,
            &popover,
            buffer,
            sourceview5::ChangeCaseType::Title,
            "Title Case",
        );
        append_change_case(
            &menu_box,
            &popover,
            buffer,
            sourceview5::ChangeCaseType::Toggle,
            "Toggle Case",
        );

        // Comment / uncomment driven by the buffer's source-language metadata.
        let syntax = comment_syntax(buffer);
        if syntax.line.is_some() || syntax.block.is_some() {
            append_separator(&menu_box);
            if let Some(line_marker) = syntax.line.clone() {
                let btn = make_menu_button(
                    "format-indent-more-symbolic",
                    "Toggle Line Comment",
                    "Ctrl+/",
                );
                let b = buffer.clone();
                let p = popover.clone();
                let marker = line_marker.clone();
                btn.connect_clicked(move |_| {
                    toggle_line_comment(&b, &marker);
                    p.popdown();
                });
                menu_box.append(&btn);
            }
            if let Some((start_marker, end_marker)) = syntax.block.clone() {
                let btn = make_menu_button(
                    "format-justify-fill-symbolic",
                    "Toggle Block Comment",
                    "Ctrl+Shift+/",
                );
                let b = buffer.clone();
                let p = popover.clone();
                btn.connect_clicked(move |_| {
                    toggle_block_comment(&b, &start_marker, &end_marker);
                    p.popdown();
                });
                menu_box.append(&btn);
            }
        }

        append_separator(&menu_box);
        append_view_action(
            &menu_box,
            &popover,
            view,
            "face-smile-symbolic",
            "Insert Emoji",
            "Ctrl+.",
            "misc.insert-emoji",
        );
    }

    // Extras (caller-supplied, e.g. format).
    if !extras.is_empty() {
        append_separator(&menu_box);
        for item in extras {
            let TextContextMenuItem::Button {
                icon,
                label,
                hint,
                on_click,
            } = item;
            let btn = make_menu_button(icon, &label, hint.as_deref().unwrap_or(""));
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                on_click();
                p.popdown();
            });
            menu_box.append(&btn);
        }
    }

    popover.set_child(Some(&menu_box));
    popover
}

fn make_menu_button(icon: &str, label_text: &str, hint: &str) -> gtk4::Button {
    let btn = gtk4::Button::new();
    btn.add_css_class("flat");
    btn.add_css_class("app-popover-button");
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    hbox.append(&gtk4::Image::from_icon_name(icon));
    let lbl = gtk4::Label::new(Some(label_text));
    lbl.set_hexpand(true);
    lbl.set_halign(gtk4::Align::Start);
    hbox.append(&lbl);
    if !hint.is_empty() {
        let hint_lbl = gtk4::Label::new(Some(hint));
        hint_lbl.add_css_class("dim-label");
        hbox.append(&hint_lbl);
    }
    btn.set_child(Some(&hbox));
    btn
}

fn append_separator(box_: &gtk4::Box) {
    box_.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
}

fn append_view_action(
    box_: &gtk4::Box,
    popover: &gtk4::PopoverMenu,
    view: &sourceview5::View,
    icon: &str,
    label: &str,
    hint: &str,
    action: &str,
) {
    let btn = make_menu_button(icon, label, hint);
    let v = view.clone();
    let p = popover.clone();
    let a = action.to_string();
    btn.connect_clicked(move |_| {
        let _ = v.activate_action(&a, None::<&gtk4::glib::Variant>);
        p.popdown();
    });
    box_.append(&btn);
}

fn append_buffer_action(
    box_: &gtk4::Box,
    popover: &gtk4::PopoverMenu,
    icon: &str,
    label: &str,
    hint: &str,
    sensitive: bool,
    on_click: impl Fn() + 'static,
) {
    let btn = make_menu_button(icon, label, hint);
    btn.set_sensitive(sensitive);
    let p = popover.clone();
    btn.connect_clicked(move |_| {
        on_click();
        p.popdown();
    });
    box_.append(&btn);
}

fn append_change_case(
    box_: &gtk4::Box,
    popover: &gtk4::PopoverMenu,
    buffer: &sourceview5::Buffer,
    case: sourceview5::ChangeCaseType,
    label: &str,
) {
    let btn = make_menu_button("format-text-underline-symbolic", label, "");
    let b = buffer.clone();
    let p = popover.clone();
    btn.connect_clicked(move |_| {
        let (mut start, mut end) = if b.has_selection() {
            b.selection_bounds().unwrap_or_else(|| b.bounds())
        } else {
            b.bounds()
        };
        b.change_case(case, &mut start, &mut end);
        p.popdown();
    });
    box_.append(&btn);
}

fn remove_builtin_context_gesture<W: IsA<gtk4::Widget>>(widget: &W) {
    let widget = widget.as_ref();
    let controllers = widget.observe_controllers();
    let n = controllers.n_items();
    for i in (0..n).rev() {
        let Some(obj) = controllers.item(i) else {
            continue;
        };
        if let Ok(gc) = obj.downcast::<gtk4::GestureClick>() {
            if gc.button() == CONTEXT_MENU_BUTTON {
                widget.remove_controller(&gc);
            }
        }
    }
}

// ── Comment / uncomment ─────────────────────────────────────────────────────

#[derive(Default, Clone)]
struct CommentSyntax {
    line: Option<String>,
    block: Option<(String, String)>,
}

fn comment_syntax(buffer: &sourceview5::Buffer) -> CommentSyntax {
    let Some(lang) = buffer.language() else {
        return CommentSyntax::default();
    };
    let line = lang
        .metadata("line-comment-start")
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let block_start = lang
        .metadata("block-comment-start")
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let block_end = lang
        .metadata("block-comment-end")
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let block = match (block_start, block_end) {
        (Some(s), Some(e)) => Some((s, e)),
        _ => None,
    };
    CommentSyntax { line, block }
}

/// Toggle line comment on the current line or every line touched by the
/// selection. If every non-empty line in the range starts with the marker the
/// markers are removed; otherwise the marker (followed by a space) is inserted
/// at the column of the first non-whitespace character.
fn toggle_line_comment(buffer: &sourceview5::Buffer, marker: &str) {
    let (sel_start, sel_end) = if buffer.has_selection() {
        buffer.selection_bounds().unwrap_or_else(|| buffer.bounds())
    } else {
        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        (cursor.clone(), cursor)
    };
    let start_line = sel_start.line();
    let mut end_line = sel_end.line();
    // If the selection ends at column 0 on a new line, treat it as belonging
    // to the previous line (matches how editors commonly behave).
    if sel_end.line_offset() == 0 && end_line > start_line {
        end_line -= 1;
    }

    let line_text_at = |line_num: i32| -> Option<(gtk4::TextIter, gtk4::TextIter, String)> {
        let line_iter = buffer.iter_at_line(line_num)?;
        let mut end_iter = line_iter.clone();
        end_iter.forward_to_line_end();
        let text = buffer.text(&line_iter, &end_iter, false).to_string();
        Some((line_iter, end_iter, text))
    };

    let all_commented = (start_line..=end_line).all(|line_num| {
        let Some((_, _, text)) = line_text_at(line_num) else {
            return true;
        };
        let trimmed = text.trim_start();
        trimmed.is_empty() || trimmed.starts_with(marker)
    });

    buffer.begin_user_action();
    // Iterate in reverse so earlier mutations don't shift later iterators.
    for line_num in (start_line..=end_line).rev() {
        let Some((line_iter, end_iter, text)) = line_text_at(line_num) else {
            continue;
        };
        let trimmed = text.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        if all_commented {
            // Strip marker (and a single trailing space if present).
            let leading_ws_chars = text.chars().take_while(|c| c.is_whitespace()).count() as i32;
            let mut from = line_iter.clone();
            from.forward_chars(leading_ws_chars);
            let marker_chars = marker.chars().count() as i32;
            let mut to = from.clone();
            to.forward_chars(marker_chars);
            // Optional single space after marker.
            let after = buffer.text(&to, &end_iter, false).to_string();
            if after.starts_with(' ') {
                to.forward_char();
            }
            buffer.delete(&mut from, &mut to);
        } else {
            let leading_ws_chars = text.chars().take_while(|c| c.is_whitespace()).count() as i32;
            let mut insert_at = line_iter.clone();
            insert_at.forward_chars(leading_ws_chars);
            buffer.insert(&mut insert_at, &format!("{} ", marker));
        }
    }
    buffer.end_user_action();
}

/// Toggle a block comment around the current selection (or current line if no
/// selection). If the trimmed selection already starts with `start_marker` and
/// ends with `end_marker`, the markers are removed; otherwise the selection is
/// wrapped.
fn toggle_block_comment(buffer: &sourceview5::Buffer, start_marker: &str, end_marker: &str) {
    let (mut start, mut end) = if buffer.has_selection() {
        buffer.selection_bounds().unwrap_or_else(|| buffer.bounds())
    } else {
        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        let mut line_start = cursor.clone();
        line_start.set_line_offset(0);
        let mut line_end = cursor.clone();
        line_end.forward_to_line_end();
        (line_start, line_end)
    };

    let text = buffer.text(&start, &end, false).to_string();
    let trimmed = text.trim();

    buffer.begin_user_action();
    if trimmed.starts_with(start_marker) && trimmed.ends_with(end_marker) {
        // Unwrap — keep surrounding whitespace from the original selection.
        let leading_ws = &text[..text.len() - text.trim_start().len()];
        let trailing_ws = &text[text.trim_end().len()..];
        let inner = trimmed
            .strip_prefix(start_marker)
            .and_then(|s| s.strip_suffix(end_marker))
            .unwrap_or(trimmed)
            .trim_start_matches(' ')
            .trim_end_matches(' ');
        let replaced = format!("{}{}{}", leading_ws, inner, trailing_ws);
        buffer.delete(&mut start, &mut end);
        buffer.insert(&mut start, &replaced);
    } else {
        let replaced = format!("{} {} {}", start_marker, text, end_marker);
        buffer.delete(&mut start, &mut end);
        buffer.insert(&mut start, &replaced);
    }
    buffer.end_user_action();
}

// ── Smart format ─────────────────────────────────────────────────────────────

/// Returns a "Format" menu item if the file extension is recognised. The
/// formatter command is invoked at click time; if it isn't installed the
/// failure is logged silently.
pub fn format_item_for(path: &Path, buffer: &sourceview5::Buffer) -> Option<TextContextMenuItem> {
    let formatter = detect_formatter(path)?;
    let label = format!("Format with {}", formatter.cmd);
    let buffer = buffer.clone();
    let path = path.to_path_buf();
    Some(TextContextMenuItem::button(
        "format-justify-fill-symbolic",
        label,
        Some("Ctrl+Shift+F"),
        move || {
            run_formatter(&formatter, &path, &buffer);
        },
    ))
}

#[derive(Clone, Copy)]
struct Formatter {
    cmd: &'static str,
    args: &'static [&'static str],
    /// If `args` contains "<file>" it is replaced with the absolute file path.
    needs_filepath: bool,
}

fn detect_formatter(path: &Path) -> Option<Formatter> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    Some(match ext.as_str() {
        "rs" => Formatter {
            cmd: "rustfmt",
            args: &["--emit=stdout"],
            needs_filepath: false,
        },
        "go" => Formatter {
            cmd: "gofmt",
            args: &[],
            needs_filepath: false,
        },
        "py" => Formatter {
            cmd: "black",
            args: &["-", "--quiet"],
            needs_filepath: false,
        },
        "xml" | "svg" | "html" | "htm" => Formatter {
            cmd: "xmllint",
            args: &["--format", "-"],
            needs_filepath: false,
        },
        "json" | "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "css" | "scss" | "less" | "md"
        | "markdown" | "yaml" | "yml" | "vue" | "svelte" => Formatter {
            cmd: "prettier",
            args: &["--stdin-filepath", "<file>"],
            needs_filepath: true,
        },
        "toml" => Formatter {
            cmd: "taplo",
            args: &["fmt", "-"],
            needs_filepath: false,
        },
        _ => return None,
    })
}

fn run_formatter(fmt: &Formatter, path: &Path, buffer: &sourceview5::Buffer) {
    use std::io::Write;
    use std::time::Duration;

    let text = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string();

    let mut cmd = Command::new(fmt.cmd);
    let path_str = path.to_string_lossy().to_string();
    for a in fmt.args {
        if fmt.needs_filepath && *a == "<file>" {
            cmd.arg(&path_str);
        } else {
            cmd.arg(a);
        }
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("formatter `{}` spawn error: {}", fmt.cmd, e);
            return;
        }
    };

    let mut child = child;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }

    // Synchronous wait with crude timeout (no async runtime here).
    let deadline = std::time::Instant::now() + Duration::from_secs(FORMATTER_TIMEOUT_SECS);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    tracing::warn!("formatter `{}` timed out", fmt.cmd);
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                tracing::warn!("formatter `{}` wait error: {}", fmt.cmd, e);
                return;
            }
        }
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("formatter `{}` output error: {}", fmt.cmd, e);
            return;
        }
    };

    if !output.status.success() {
        tracing::warn!(
            "formatter `{}` failed: {}",
            fmt.cmd,
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let formatted = match String::from_utf8(output.stdout) {
        Ok(s) => s,
        Err(_) => {
            tracing::warn!("formatter `{}` produced non-UTF-8 output", fmt.cmd);
            return;
        }
    };

    if formatted == text {
        return;
    }

    if let Err(err) =
        super::text_content::replace_source_buffer_text_preserving_cursor(buffer, &formatted)
    {
        tracing::warn!(
            "formatter `{}` produced text that cannot be displayed: {}",
            fmt.cmd,
            err
        );
    }
}
