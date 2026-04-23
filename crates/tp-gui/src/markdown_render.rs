//! Shared Markdown-to-TextBuffer renderer.
//!
//! Used by both the standalone Markdown panel (`panels::markdown`) and the
//! Code Editor's Markdown tab (`panels::editor::markdown_view`). Parsing is
//! done by pulldown-cmark (CommonMark + GFM tables/strikethrough/tasks/
//! footnotes); events are mapped to GTK `TextTag`s for presentation inside
//! a `TextView`, so the UI stays consistent with the rest of the editor.

use gtk4::prelude::*;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

// Code block backgrounds — slight contrast against each theme family's main
// surface, without overriding the default text foreground (so GTK keeps
// contrast for us as the theme changes).
const CODE_BG_DARK: &str = "#1a1a1a";
const CODE_BG_LIGHT: &str = "#ececec";

pub(crate) fn render_markdown_to_view(tv: &gtk4::TextView, content: &str) {
    let buf = tv.buffer();
    buf.set_text("");
    let tt = buf.tag_table();

    let is_light = matches!(
        crate::theme::current_theme().color_scheme(),
        libadwaita::ColorScheme::ForceLight
    );
    let code_bg = if is_light { CODE_BG_LIGHT } else { CODE_BG_DARK };

    // `ensure` re-applies the callback every time so theme-reactive tags
    // update when the renderer runs again after a theme change, not just on
    // first creation.
    let ensure = |name: &str, f: &dyn Fn(&gtk4::TextTag)| {
        let t = if let Some(t) = tt.lookup(name) {
            t
        } else {
            let t = gtk4::TextTag::new(Some(name));
            tt.add(&t);
            t
        };
        f(&t);
    };
    ensure("h1", &|t| {
        t.set_size_points(20.0);
        t.set_weight(700);
    });
    ensure("h2", &|t| {
        t.set_size_points(16.0);
        t.set_weight(700);
    });
    ensure("h3", &|t| {
        t.set_size_points(14.0);
        t.set_weight(700);
    });
    ensure("h4", &|t| {
        t.set_size_points(13.0);
        t.set_weight(700);
    });
    ensure("h5", &|t| {
        t.set_size_points(12.0);
        t.set_weight(700);
    });
    ensure("h6", &|t| {
        t.set_size_points(11.0);
        t.set_weight(700);
    });
    ensure("bold", &|t| {
        t.set_weight(700);
    });
    ensure("italic", &|t| {
        t.set_style(gtk4::pango::Style::Italic);
    });
    ensure("strike", &|t| {
        t.set_strikethrough(true);
    });
    ensure("code", &|t| {
        t.set_family(Some("monospace"));
    });
    ensure("code_block", &|t| {
        t.set_family(Some("monospace"));
        t.set_paragraph_background(Some(code_bg));
        t.set_left_margin(20);
    });
    ensure("link", &|t| {
        t.set_foreground(Some("#5588ff"));
        t.set_underline(gtk4::pango::Underline::Single);
    });
    ensure("bullet", &|t| {
        t.set_left_margin(20);
    });
    ensure("bq", &|t| {
        t.set_left_margin(20);
        t.set_style(gtk4::pango::Style::Italic);
        t.set_foreground(Some("#888888"));
    });
    ensure("sep", &|t| {
        t.set_foreground(Some("#666666"));
        t.set_size_points(6.0);
    });
    ensure("table", &|t| {
        t.set_family(Some("monospace"));
        t.set_paragraph_background(Some(code_bg));
    });
    ensure("table_header", &|t| {
        t.set_family(Some("monospace"));
        t.set_paragraph_background(Some(code_bg));
        t.set_weight(700);
    });

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(content, opts);

    let mut state = RenderState::default();
    let mut it = buf.end_iter();
    for event in parser {
        dispatch(&buf, &mut it, &mut state, event);
    }
}

/// Inline formatting and structural state carried across events.
#[derive(Default)]
struct RenderState {
    /// Inline tag names currently active (bold/italic/strike/link).
    inline_tags: Vec<&'static str>,
    /// Heading level in flight, if any.
    heading: Option<&'static str>,
    /// Depth of nested lists with each frame's next-ordinal (None = unordered).
    lists: Vec<Option<u64>>,
    /// Block quote nesting level.
    bq_depth: usize,
    /// Inside a fenced/indented code block?
    in_code_block: bool,
    /// Remember whether the current list item has had its marker emitted yet.
    item_needs_marker: bool,
    /// Table buffering — pulldown-cmark emits cells one-by-one; we collect
    /// rows and render aligned when the table ends.
    table: Option<TableState>,
}

struct TableState {
    rows: Vec<Vec<String>>,
    /// Index into `rows` where the body starts. Everything before is header.
    body_start: usize,
    /// Accumulator for the current cell while events stream in.
    current_cell: String,
    /// Accumulator for the current row.
    current_row: Vec<String>,
    /// Are we currently in the table head?
    in_head: bool,
}

fn dispatch(buf: &gtk4::TextBuffer, it: &mut gtk4::TextIter, st: &mut RenderState, ev: Event) {
    match ev {
        Event::Start(tag) => handle_start(buf, it, st, tag),
        Event::End(tag) => handle_end(buf, it, st, tag),
        Event::Text(text) => on_text(buf, it, st, &text),
        Event::Code(code) => on_inline_code(buf, it, st, &code),
        Event::Html(_) | Event::InlineHtml(_) => {}
        Event::FootnoteReference(r) => {
            let marker = format!("[^{}]", r);
            insert_inline(buf, it, st, &marker);
        }
        Event::SoftBreak => insert_inline(buf, it, st, " "),
        Event::HardBreak => {
            buf.insert(it, "\n");
        }
        Event::Rule => {
            buf.insert_with_tags_by_name(it, "────────────────────\n", &["sep"]);
            emit_block_separator(buf, it, st);
        }
        Event::TaskListMarker(done) => {
            insert_inline(buf, it, st, if done { "☒ " } else { "☐ " });
        }
        Event::InlineMath(s) | Event::DisplayMath(s) => {
            insert_inline(buf, it, st, &s);
        }
    }
}

fn handle_start(buf: &gtk4::TextBuffer, it: &mut gtk4::TextIter, st: &mut RenderState, tag: Tag) {
    match tag {
        Tag::Paragraph => {
            if st.item_needs_marker {
                emit_list_marker(buf, it, st);
            }
        }
        Tag::Heading { level, .. } => {
            st.heading = Some(heading_tag(level));
        }
        Tag::BlockQuote(_) => {
            st.bq_depth += 1;
        }
        Tag::CodeBlock(kind) => {
            st.in_code_block = true;
            let hint = match kind {
                CodeBlockKind::Fenced(s) => s.to_string(),
                CodeBlockKind::Indented => String::new(),
            };
            if hint.is_empty() {
                buf.insert_with_tags_by_name(it, "───────\n", &["sep"]);
            } else {
                buf.insert_with_tags_by_name(it, &format!("─── {} ───\n", hint), &["sep"]);
            }
        }
        Tag::List(start) => {
            // A list nested inside an item starts on a new line; otherwise
            // the "- parent text" and the first child's "• child text" run
            // together on the same visual row.
            if !st.lists.is_empty() {
                buf.insert(it, "\n");
            }
            st.lists.push(start);
        }
        Tag::Item => {
            st.item_needs_marker = true;
        }
        Tag::Emphasis => st.inline_tags.push("italic"),
        Tag::Strong => st.inline_tags.push("bold"),
        Tag::Strikethrough => st.inline_tags.push("strike"),
        Tag::Link { .. } => st.inline_tags.push("link"),
        Tag::Image { .. } => st.inline_tags.push("italic"),
        Tag::Table(_) => {
            st.table = Some(TableState {
                rows: Vec::new(),
                body_start: 0,
                current_cell: String::new(),
                current_row: Vec::new(),
                in_head: false,
            });
        }
        Tag::TableHead => {
            if let Some(t) = st.table.as_mut() {
                t.in_head = true;
            }
        }
        Tag::TableRow => {
            if let Some(t) = st.table.as_mut() {
                t.current_row = Vec::new();
            }
        }
        Tag::TableCell => {
            if let Some(t) = st.table.as_mut() {
                t.current_cell = String::new();
            }
        }
        Tag::FootnoteDefinition(label) => {
            insert_inline(buf, it, st, &format!("\n[^{}]: ", label));
        }
        _ => {}
    }
}

fn handle_end(
    buf: &gtk4::TextBuffer,
    it: &mut gtk4::TextIter,
    st: &mut RenderState,
    tag: TagEnd,
) {
    match tag {
        TagEnd::Paragraph => {
            buf.insert(it, "\n");
            emit_block_separator(buf, it, st);
        }
        TagEnd::Heading(_) => {
            st.heading = None;
            buf.insert(it, "\n");
            emit_block_separator(buf, it, st);
        }
        TagEnd::BlockQuote(_) => {
            if st.bq_depth > 0 {
                st.bq_depth -= 1;
            }
            emit_block_separator(buf, it, st);
        }
        TagEnd::CodeBlock => {
            st.in_code_block = false;
            buf.insert_with_tags_by_name(it, "───────\n", &["sep"]);
            emit_block_separator(buf, it, st);
        }
        TagEnd::List(_) => {
            st.lists.pop();
            emit_block_separator(buf, it, st);
        }
        TagEnd::Item => {
            // Tight lists don't emit Paragraph events, so End(Paragraph)
            // can't carry the trailing newline. Always close the item with
            // one here — loose lists get a second newline (blank row between
            // items) which is the intended visual for loose lists anyway.
            if st.item_needs_marker {
                emit_list_marker(buf, it, st);
            }
            buf.insert(it, "\n");
        }
        TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link
        | TagEnd::Image => {
            st.inline_tags.pop();
        }
        TagEnd::Table => {
            if let Some(t) = st.table.take() {
                render_table(buf, it, &t);
            }
            emit_block_separator(buf, it, st);
        }
        TagEnd::TableHead => {
            if let Some(t) = st.table.as_mut() {
                t.in_head = false;
                t.body_start = t.rows.len();
            }
        }
        TagEnd::TableRow => {
            if let Some(t) = st.table.as_mut() {
                let row = std::mem::take(&mut t.current_row);
                t.rows.push(row);
            }
        }
        TagEnd::TableCell => {
            if let Some(t) = st.table.as_mut() {
                let cell = std::mem::take(&mut t.current_cell);
                t.current_row.push(cell);
            }
        }
        _ => {}
    }
}

/// Emit a blank line after a block when we're back at the document top
/// level, so paragraphs/headings/lists/quotes don't run together. Inside a
/// list item or blockquote the surrounding container drives the spacing.
fn emit_block_separator(buf: &gtk4::TextBuffer, it: &mut gtk4::TextIter, st: &RenderState) {
    if st.lists.is_empty() && st.bq_depth == 0 {
        buf.insert(it, "\n");
    }
}

fn on_text(buf: &gtk4::TextBuffer, it: &mut gtk4::TextIter, st: &mut RenderState, text: &str) {
    if let Some(t) = st.table.as_mut() {
        t.current_cell.push_str(text);
        return;
    }
    if st.in_code_block {
        buf.insert_with_tags_by_name(it, text, &["code_block"]);
        return;
    }
    insert_inline(buf, it, st, text);
}

fn on_inline_code(
    buf: &gtk4::TextBuffer,
    it: &mut gtk4::TextIter,
    st: &mut RenderState,
    code: &str,
) {
    if let Some(t) = st.table.as_mut() {
        t.current_cell.push_str(code);
        return;
    }
    let mut tags: Vec<&str> = effective_tags(st);
    tags.push("code");
    buf.insert_with_tags_by_name(it, code, &tags);
}

/// Insert `text` with whatever inline/block tags the current state implies.
fn insert_inline(
    buf: &gtk4::TextBuffer,
    it: &mut gtk4::TextIter,
    st: &mut RenderState,
    text: &str,
) {
    if st.item_needs_marker {
        emit_list_marker(buf, it, st);
    }
    let tags = effective_tags(st);
    if tags.is_empty() {
        buf.insert(it, text);
    } else {
        buf.insert_with_tags_by_name(it, text, &tags);
    }
}

fn effective_tags(st: &RenderState) -> Vec<&'static str> {
    let mut tags: Vec<&'static str> = Vec::new();
    if let Some(h) = st.heading {
        tags.push(h);
    }
    if st.bq_depth > 0 {
        tags.push("bq");
    }
    if !st.lists.is_empty() && st.heading.is_none() {
        tags.push("bullet");
    }
    tags.extend(st.inline_tags.iter().copied());
    tags
}

fn emit_list_marker(buf: &gtk4::TextBuffer, it: &mut gtk4::TextIter, st: &mut RenderState) {
    st.item_needs_marker = false;
    let depth = st.lists.len().saturating_sub(1);
    let indent = "  ".repeat(depth);
    let marker_body = match st.lists.last_mut() {
        Some(Some(n)) => {
            let out = format!("{}. ", n);
            *n += 1;
            out
        }
        _ => "• ".to_string(),
    };
    let full = format!("  {}{}", indent, marker_body);
    buf.insert_with_tags_by_name(it, &full, &["bullet"]);
}

fn heading_tag(level: HeadingLevel) -> &'static str {
    match level {
        HeadingLevel::H1 => "h1",
        HeadingLevel::H2 => "h2",
        HeadingLevel::H3 => "h3",
        HeadingLevel::H4 => "h4",
        HeadingLevel::H5 => "h5",
        HeadingLevel::H6 => "h6",
    }
}

fn render_table(buf: &gtk4::TextBuffer, it: &mut gtk4::TextIter, t: &TableState) {
    if t.rows.is_empty() {
        return;
    }
    let n_cols = t.rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if n_cols == 0 {
        return;
    }
    let mut widths = vec![0_usize; n_cols];
    for row in &t.rows {
        for (c, cell) in row.iter().enumerate() {
            widths[c] = widths[c].max(cell.chars().count());
        }
    }

    let format_row = |row: &[String]| -> String {
        let mut out = String::from("│ ");
        for c in 0..n_cols {
            let cell = row.get(c).map(String::as_str).unwrap_or("");
            let pad = widths[c].saturating_sub(cell.chars().count());
            out.push_str(cell);
            out.push_str(&" ".repeat(pad));
            out.push_str(if c + 1 == n_cols { " │" } else { " │ " });
        }
        out.push('\n');
        out
    };
    let border_row = |left: char, mid: char, right: char| -> String {
        let mut s = String::new();
        s.push(left);
        for (c, w) in widths.iter().enumerate() {
            s.push_str(&"─".repeat(w + 2));
            s.push(if c + 1 == n_cols { right } else { mid });
        }
        s.push('\n');
        s
    };

    buf.insert_with_tags_by_name(it, &border_row('┌', '┬', '┐'), &["table"]);
    for (idx, row) in t.rows.iter().enumerate() {
        let tag = if idx < t.body_start {
            "table_header"
        } else {
            "table"
        };
        buf.insert_with_tags_by_name(it, &format_row(row), &[tag]);
        if idx + 1 == t.body_start && t.body_start > 0 {
            buf.insert_with_tags_by_name(it, &border_row('├', '┼', '┤'), &["table"]);
        }
    }
    buf.insert_with_tags_by_name(it, &border_row('└', '┴', '┘'), &["table"]);
}
