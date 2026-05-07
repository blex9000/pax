//! Shared Markdown-to-TextBuffer renderer.
//!
//! Used by both the standalone Markdown panel (`panels::markdown`) and the
//! Code Editor's Markdown tab (`panels::editor::markdown_view`). Parsing is
//! done by pulldown-cmark (CommonMark + GFM tables/strikethrough/tasks/
//! footnotes); events are mapped to GTK `TextTag`s for presentation inside
//! a `TextView`, so the UI stays consistent with the rest of the editor.

use gtk4::prelude::*;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style, Theme as SyntectTheme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;
use unicode_width::UnicodeWidthStr;

/// Wrap `text_view` (currently the only child of `scrolled`) in a
/// `gtk4::Overlay` containing a transparent `DrawingArea` that paints
/// a blockquote-style left bar next to every range tagged `bq`. The
/// bar is the visual cue the on-screen viewer was missing — TextTag
/// has no border-left attribute, so we paint it ourselves.
///
/// Bar redraws on:
/// - buffer changes (text edits / re-render replaces tag ranges)
/// - vertical scroll (paragraph y-coordinates shift)
/// - widget resize (wrap re-flows ranges to new heights)
pub(crate) fn attach_blockquote_bar_overlay(
    scrolled: &gtk4::ScrolledWindow,
    text_view: &gtk4::TextView,
) {
    // Detach the text view from the scrolled window so we can re-parent
    // it inside a gtk4::Overlay that sits between them.
    scrolled.set_child(None::<&gtk4::Widget>);

    let overlay = gtk4::Overlay::new();
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    overlay.set_child(Some(text_view));

    let bars = gtk4::DrawingArea::new();
    bars.set_can_target(false); // pointer events fall through to the text view
    bars.set_hexpand(true);
    bars.set_vexpand(true);
    overlay.add_overlay(&bars);
    scrolled.set_child(Some(&overlay));

    {
        let tv = text_view.clone();
        bars.set_draw_func(move |_da, cr, _w, _h| {
            paint_blockquote_bars(&tv, cr);
        });
    }

    // Trigger a repaint whenever anything that might shift bar
    // positions changes. queue_draw is cheap; the draw_func only walks
    // the bq tag toggles.
    {
        let bars_c = bars.clone();
        text_view.buffer().connect_changed(move |_| {
            bars_c.queue_draw();
        });
    }
    {
        let bars_c = bars.clone();
        scrolled.vadjustment().connect_value_changed(move |_| {
            bars_c.queue_draw();
        });
    }
    {
        let bars_c = bars.clone();
        text_view.connect_notify_local(Some("height-request"), move |_, _| {
            bars_c.queue_draw();
        });
    }
}

const BQ_BAR_WIDTH: f64 = 2.0;
const BQ_BAR_RGB: (f64, f64, f64) = (0.55, 0.55, 0.55);
/// Horizontal offset of the bar inside the indented blockquote
/// region. Added to `text_view.left_margin()` so the bar lines up
/// with where text actually starts in the panel rather than sitting
/// flush at the widget edge.
const BQ_BAR_OFFSET_FROM_TEXT_START: f64 = 4.0;

fn paint_blockquote_bars(tv: &gtk4::TextView, cr: &gtk4::cairo::Context) {
    let buffer = tv.buffer();
    let Some(tag) = buffer.tag_table().lookup("bq") else {
        return;
    };

    cr.set_source_rgb(BQ_BAR_RGB.0, BQ_BAR_RGB.1, BQ_BAR_RGB.2);
    cr.set_line_width(BQ_BAR_WIDTH);

    let bar_x = tv.left_margin() as f64 + BQ_BAR_OFFSET_FROM_TEXT_START;

    // Walk every tag-toggle in the buffer, tracking whether we're
    // currently inside a `bq` run. Each open→close pair is one bar.
    let mut iter = buffer.start_iter();
    let mut inside = iter.has_tag(&tag);
    let mut run_start: Option<gtk4::TextIter> = inside.then(|| iter.clone());

    loop {
        if !iter.forward_to_tag_toggle(Some(&tag)) {
            break;
        }
        if inside {
            if let Some(start) = run_start.take() {
                draw_bar(tv, cr, bar_x, &start, &iter);
            }
            inside = false;
        } else {
            run_start = Some(iter.clone());
            inside = true;
        }
    }
    // Open run that runs to end-of-buffer (rare — typically tags close).
    if let Some(start) = run_start {
        let end = buffer.end_iter();
        draw_bar(tv, cr, bar_x, &start, &end);
    }
    cr.stroke().ok();
}

fn draw_bar(
    tv: &gtk4::TextView,
    cr: &gtk4::cairo::Context,
    bar_x: f64,
    start: &gtk4::TextIter,
    end: &gtk4::TextIter,
) {
    let start_loc = tv.iter_location(start);
    let end_loc = tv.iter_location(end);
    let buf_y_top = start_loc.y();
    let buf_y_bot = end_loc.y() + end_loc.height();
    let (_, win_y_top) = tv.buffer_to_window_coords(gtk4::TextWindowType::Widget, 0, buf_y_top);
    let (_, win_y_bot) = tv.buffer_to_window_coords(gtk4::TextWindowType::Widget, 0, buf_y_bot);
    cr.move_to(bar_x, win_y_top as f64);
    cr.line_to(bar_x, win_y_bot as f64);
}

// Heading vertical padding (pixels above / below the heading line).
// Scaled so H1/H2 get more breathing room than H5/H6.
const HEADING_PAD_LG: i32 = 14;
const HEADING_PAD_MD: i32 = 8;
const HEADING_PAD_SM: i32 = 4;

pub type NotebookHook<'a> =
    &'a mut dyn FnMut(&pax_core::notebook_tag::NotebookCellSpec, &str, &gtk4::TextChildAnchor);

pub(crate) fn render_markdown_to_view(tv: &gtk4::TextView, content: &str) {
    render_markdown_to_view_with_hook(tv, content, None);
}

pub(crate) fn render_markdown_to_view_with_hook(
    tv: &gtk4::TextView,
    content: &str,
    mut hook: Option<NotebookHook<'_>>,
) {
    let buf = tv.buffer();
    buf.set_text("");
    let tt = buf.tag_table();

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
    // Heading tags carry their own vertical padding so every heading is
    // visually separated from whatever precedes/follows it, independent
    // of the user's blank-line count in source. The `_flush` variants drop
    // `pixels_above_lines` so the very first block in the document doesn't
    // paint a visual margin at the top of the view.
    define_heading_tag(&ensure, "h1", 20.0, HEADING_PAD_LG, HEADING_PAD_MD);
    define_heading_tag(&ensure, "h1_flush", 20.0, 0, HEADING_PAD_MD);
    define_heading_tag(&ensure, "h2", 16.0, HEADING_PAD_LG, HEADING_PAD_MD);
    define_heading_tag(&ensure, "h2_flush", 16.0, 0, HEADING_PAD_MD);
    define_heading_tag(&ensure, "h3", 14.0, HEADING_PAD_MD, HEADING_PAD_SM);
    define_heading_tag(&ensure, "h3_flush", 14.0, 0, HEADING_PAD_SM);
    define_heading_tag(&ensure, "h4", 13.0, HEADING_PAD_MD, HEADING_PAD_SM);
    define_heading_tag(&ensure, "h4_flush", 13.0, 0, HEADING_PAD_SM);
    define_heading_tag(&ensure, "h5", 12.0, HEADING_PAD_SM, HEADING_PAD_SM);
    define_heading_tag(&ensure, "h5_flush", 12.0, 0, HEADING_PAD_SM);
    define_heading_tag(&ensure, "h6", 11.0, HEADING_PAD_SM, HEADING_PAD_SM);
    define_heading_tag(&ensure, "h6_flush", 11.0, 0, HEADING_PAD_SM);
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
        t.set_left_margin(20);
        // Code shouldn't reflow at the viewport edge — long lines stay
        // intact and the panel scrolls horizontally instead.
        t.set_wrap_mode(gtk4::WrapMode::None);
    });
    ensure("link", &|t| {
        t.set_foreground(Some("#5588ff"));
        t.set_underline(gtk4::pango::Underline::Single);
    });
    ensure("bullet", &|t| {
        // Hanging indent: the bullet/number sits at the left margin
        // (column 0 of the indented block) and the *first* line starts
        // there. Wrapped continuation lines indent past the bullet so
        // they align with the text after the marker, not under it.
        t.set_left_margin(20);
        t.set_indent(-20);
    });
    ensure("list_marker", &|t| {
        // Slightly heavier weight so the bullet / number stands out
        // from the surrounding item text.
        t.set_weight(600);
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
    // Tables: monospace, no wrap (a wrapped row destroys the
    // box-drawing alignment far worse than a horizontal scrollbar).
    // No background — the box-drawing chars frame the table well
    // enough on their own.
    ensure("table", &|t| {
        t.set_family(Some("monospace"));
        t.set_wrap_mode(gtk4::WrapMode::None);
    });
    ensure("table_header", &|t| {
        t.set_family(Some("monospace"));
        t.set_wrap_mode(gtk4::WrapMode::None);
        t.set_weight(700);
    });

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    // offset_iter exposes the source byte range of each event, so we can
    // preserve the user's blank-line count between top-level blocks instead
    // of forcing a single auto-inserted blank after every block.
    let parser = Parser::new_ext(content, opts).into_offset_iter();

    let mut state = RenderState::default();
    let mut it = buf.end_iter();
    for (event, range) in parser {
        // ── highlighted code-block capture branch ────────────────────
        // Runs before the notebook branches so a snippet that opened with a
        // recognised language keeps collecting until End(CodeBlock) regardless
        // of what other events arrive in between (pulldown emits Text events
        // inside the block; nothing else for fenced code).
        if let Some(cap) = state.code_capture.as_mut() {
            match &event {
                Event::Text(t) => {
                    cap.body.push_str(t);
                    continue;
                }
                Event::End(TagEnd::CodeBlock) => {
                    let cap = state.code_capture.take().unwrap();
                    highlight_code_into_buffer(&buf, &mut it, &cap.info, &cap.body);
                    buf.insert(&mut it, "\n");
                    state.in_code_block = false;
                    continue;
                }
                _ => continue,
            }
        }

        // ── notebook-cell capture branch ─────────────────────────────
        if let Some(cap) = state.notebook_collecting.as_mut() {
            match &event {
                Event::Text(t) => {
                    cap.body.push_str(t);
                    continue;
                }
                Event::End(TagEnd::CodeBlock) => {
                    let cap = state.notebook_collecting.take().unwrap();
                    if let Some(ref mut cb) = hook {
                        let anchor = buf.create_child_anchor(&mut it);
                        cb(&cap.spec, &cap.body, &anchor);
                    }
                    buf.insert(&mut it, "\n");
                    state.in_code_block = false;
                    continue;
                }
                _ => continue, // swallow other inline events inside the cell
            }
        }

        // ── notebook-cell start branch (skip dispatch for the header) ─
        if hook.is_some() {
            if let Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) = &event {
                if let Some(spec) = pax_core::notebook_tag::NotebookCellSpec::parse(info) {
                    state.in_code_block = true;
                    state.notebook_collecting = Some(NotebookCapture {
                        spec,
                        body: String::new(),
                    });
                    continue;
                }
            }
        }

        // ── highlighted code-block start branch ──────────────────────
        // Runs after the notebook check so a notebook spec wins. If the fence
        // info string resolves to a syntect syntax we capture the body until
        // End(CodeBlock) and emit it as a single highlighted run inside the
        // shared TextBuffer — no embedded sourceview widget, no per-block
        // resize-polling timer.
        if let Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) = &event {
            if syntect_syntax_for(info).is_some() {
                state.in_code_block = true;
                state.code_capture = Some(CodeCapture {
                    info: info.to_string(),
                    body: String::new(),
                });
                continue;
            }
        }

        // ── normal dispatch (existing behaviour for non-notebook blocks) ─
        let starts_block = is_block_marker_start(&event);

        if starts_block
            && !state.pending_first_block
            && state.lists.is_empty()
            && state.bq_depth == 0
        {
            // Count blank lines by walking backwards from the new block's
            // start through whitespace. This is independent of how
            // pulldown-cmark sets the previous event's range.end (which
            // differs between Paragraph/Heading vs List/BlockQuote), so
            // lists followed by paragraphs get the same spacing as
            // paragraphs followed by paragraphs.
            let blanks = blank_lines_before(content, range.start);
            for _ in 0..blanks {
                buf.insert(&mut it, "\n");
            }
        }

        dispatch(&buf, &mut it, &mut state, event);

        if starts_block && state.pending_first_block {
            state.pending_first_block = false;
        }
    }
}

fn blank_lines_before(source: &str, next_start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut pos = next_start;
    // Skip trailing whitespace on the next block's line (unlikely but cheap).
    while pos > 0 && matches!(bytes[pos - 1], b' ' | b'\t') {
        pos -= 1;
    }
    // The first \n we cross is the newline that ended the previous block's
    // last line; it isn't a blank line by itself. Every additional \n in
    // the run corresponds to one blank line (possibly with inner whitespace).
    let mut newline_run: isize = 0;
    loop {
        if pos == 0 || bytes[pos - 1] != b'\n' {
            break;
        }
        newline_run += 1;
        pos -= 1;
        while pos > 0 && matches!(bytes[pos - 1], b' ' | b'\t') {
            pos -= 1;
        }
    }
    (newline_run - 1).max(0) as usize
}

fn is_block_marker_start(ev: &Event) -> bool {
    match ev {
        Event::Start(tag) => matches!(
            tag,
            Tag::Paragraph
                | Tag::Heading { .. }
                | Tag::CodeBlock(_)
                | Tag::BlockQuote(_)
                | Tag::List(_)
                | Tag::Table(_)
        ),
        // Rule is a self-contained block event — treat it as both a start
        // (so preceding blank lines are honored) and an end (so the next
        // block also sees its gap from here).
        Event::Rule => true,
        _ => false,
    }
}

struct NotebookCapture {
    spec: pax_core::notebook_tag::NotebookCellSpec,
    body: String,
}

struct CodeCapture {
    /// Verbatim fence info string (e.g. "json", "rust"). Stored as String
    /// because pulldown-cmark's info string borrows from the source content,
    /// but we accumulate body text across events and need an owned token at
    /// finalization time when we hand it to the syntect resolver.
    info: String,
    body: String,
}

/// Inline formatting and structural state carried across events.
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
    /// True until the first top-level block starts. Lets the first heading
    /// use a `_flush` tag so it doesn't paint padding above itself when
    /// nothing precedes it in the document.
    pending_first_block: bool,
    /// When set, we're inside a fenced code block whose info string parsed as
    /// a `NotebookCellSpec`. Body text events accumulate here until End(CodeBlock),
    /// at which point the hook (if any) is invoked with the captured spec/body.
    notebook_collecting: Option<NotebookCapture>,
    /// When set, we're inside a fenced code block whose info string resolved
    /// to a known `syntect` syntax. Body text accumulates until End(CodeBlock),
    /// at which point the captured body is highlighted and inserted directly
    /// into the surrounding TextBuffer with color tags — no child widget.
    code_capture: Option<CodeCapture>,
}

impl Default for RenderState {
    fn default() -> Self {
        Self {
            inline_tags: Vec::new(),
            heading: None,
            lists: Vec::new(),
            bq_depth: 0,
            in_code_block: false,
            item_needs_marker: false,
            table: None,
            pending_first_block: true,
            notebook_collecting: None,
            code_capture: None,
        }
    }
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
            let first = st.pending_first_block && st.lists.is_empty() && st.bq_depth == 0;
            st.heading = Some(heading_tag(level, first));
        }
        Tag::BlockQuote(_) => {
            st.bq_depth += 1;
        }
        Tag::CodeBlock(_) => {
            // No language banner — the `code_block` tag's monospace +
            // background already distinguishes the block visually.
            st.in_code_block = true;
        }
        Tag::List(start) => {
            // A list nested inside an item needs to start on a fresh line.
            // In loose lists the preceding End(Paragraph) already closed
            // the line; in tight lists nothing has, so check that the
            // buffer's cursor is at the start of a line and only insert a
            // newline when it isn't. This prevents the extra blank row
            // that was appearing between an item's text and its nested
            // list in loose lists.
            if !st.lists.is_empty() && !it.starts_line() {
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

fn handle_end(buf: &gtk4::TextBuffer, it: &mut gtk4::TextIter, st: &mut RenderState, tag: TagEnd) {
    match tag {
        TagEnd::Paragraph => {
            buf.insert(it, "\n");
        }
        TagEnd::Heading(_) => {
            st.heading = None;
            buf.insert(it, "\n");
        }
        TagEnd::BlockQuote(_) => {
            if st.bq_depth > 0 {
                st.bq_depth -= 1;
            }
        }
        TagEnd::CodeBlock => {
            st.in_code_block = false;
        }
        TagEnd::List(_) => {
            st.lists.pop();
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
        TagEnd::Emphasis
        | TagEnd::Strong
        | TagEnd::Strikethrough
        | TagEnd::Link
        | TagEnd::Image => {
            st.inline_tags.pop();
        }
        TagEnd::Table => {
            if let Some(t) = st.table.take() {
                render_table(buf, it, &t);
            }
        }
        TagEnd::TableHead => {
            if let Some(t) = st.table.as_mut() {
                t.in_head = false;
                // pulldown-cmark emits header cells directly inside TableHead
                // without a wrapping TableRow, so the accumulated header cells
                // sit in `current_row` and never reach `rows` via the usual
                // End(TableRow) handler. Push them now.
                if !t.current_row.is_empty() {
                    let row = std::mem::take(&mut t.current_row);
                    t.rows.push(row);
                }
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
    buf.insert_with_tags_by_name(it, &full, &["bullet", "list_marker"]);
}

fn heading_tag(level: HeadingLevel, first_block: bool) -> &'static str {
    match (level, first_block) {
        (HeadingLevel::H1, false) => "h1",
        (HeadingLevel::H1, true) => "h1_flush",
        (HeadingLevel::H2, false) => "h2",
        (HeadingLevel::H2, true) => "h2_flush",
        (HeadingLevel::H3, false) => "h3",
        (HeadingLevel::H3, true) => "h3_flush",
        (HeadingLevel::H4, false) => "h4",
        (HeadingLevel::H4, true) => "h4_flush",
        (HeadingLevel::H5, false) => "h5",
        (HeadingLevel::H5, true) => "h5_flush",
        (HeadingLevel::H6, false) => "h6",
        (HeadingLevel::H6, true) => "h6_flush",
    }
}

fn define_heading_tag(
    ensure: &dyn Fn(&str, &dyn Fn(&gtk4::TextTag)),
    name: &str,
    size_points: f64,
    pixels_above: i32,
    pixels_below: i32,
) {
    ensure(name, &|t| {
        t.set_size_points(size_points);
        t.set_weight(700);
        t.set_pixels_above_lines(pixels_above);
        t.set_pixels_below_lines(pixels_below);
    });
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
            widths[c] = widths[c].max(cell.width());
        }
    }

    let format_row = |row: &[String]| -> String {
        let mut out = String::from("│ ");
        for c in 0..n_cols {
            let cell = row.get(c).map(String::as_str).unwrap_or("");
            let pad = widths[c].saturating_sub(cell.width());
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

// ── syntect-driven code highlighting ─────────────────────────────────────────
//
// We render fenced code blocks directly into the surrounding TextBuffer instead
// of embedding a per-block GtkSourceView5 widget. The previous design was a
// double workaround: anchored child widgets in a TextView ignore `hexpand`, so
// every snippet needed a 300ms resize-polling timer; on top of that, creating
// 27 sourceview instances synchronously on the GTK main thread froze the UI
// for several seconds when re-rendering a code-heavy README. Tagging text with
// foreground colors derived from a syntect highlight pass avoids both issues
// at once: highlighting is pure-Rust and runs in a few milliseconds, no child
// widgets means no resize problem, and re-rendering is cheap enough that the
// theme observer can re-run it on every theme switch.

fn syntax_set() -> &'static SyntaxSet {
    static CELL: OnceLock<SyntaxSet> = OnceLock::new();
    CELL.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    static CELL: OnceLock<ThemeSet> = OnceLock::new();
    CELL.get_or_init(ThemeSet::load_defaults)
}

fn current_syntect_theme() -> (&'static str, &'static SyntectTheme) {
    let name = crate::theme::current_theme().syntect_theme();
    let theme = theme_set()
        .themes
        .get(name)
        .or_else(|| theme_set().themes.get("base16-eighties.dark"))
        .or_else(|| theme_set().themes.values().next())
        .expect("syntect ships with at least one theme");
    (name, theme)
}

/// Resolve a fence info string to a `syntect` syntax. Returns `None` when the
/// language isn't recognised; the caller falls back to the plain `code_block`
/// text path so unknown fences still render as monospace text.
fn syntect_syntax_for(info: &str) -> Option<&'static SyntaxReference> {
    let token = info.split(|c: char| c.is_whitespace() || c == ',').next()?;
    if token.is_empty() {
        return None;
    }
    let ss = syntax_set();
    if let Some(s) = ss.find_syntax_by_token(token) {
        return Some(s);
    }
    let alias = match token.to_lowercase().as_str() {
        "shell" | "zsh" | "sh" => "bash",
        "yml" => "yaml",
        "ts" => "typescript",
        "js" => "javascript",
        "py" => "python",
        "rb" => "ruby",
        "md" => "markdown",
        "c++" => "cpp",
        "c#" => "cs",
        _ => return None,
    };
    ss.find_syntax_by_token(alias)
}

/// Reuse-or-create a `TextTag` carrying the foreground/font-style of a syntect
/// `Style`. Tags are named deterministically — `cb_<theme>_<rrggbb>_<flags>` —
/// and stored in the buffer's tag table so subsequent highlight runs (and
/// every line within the same run) hit the cache instead of allocating a fresh
/// tag per `(Style, &str)` slice. The theme prefix in the name keeps colors
/// stable across theme switches without having to invalidate the table.
fn tag_for_style(buf: &gtk4::TextBuffer, theme_name: &str, style: Style) -> gtk4::TextTag {
    let fg = style.foreground;
    let fs = style.font_style;
    let bold = fs.contains(FontStyle::BOLD);
    let italic = fs.contains(FontStyle::ITALIC);
    let underline = fs.contains(FontStyle::UNDERLINE);
    let flags = match (bold, italic, underline) {
        (false, false, false) => "",
        (true, false, false) => "b",
        (false, true, false) => "i",
        (false, false, true) => "u",
        (true, true, false) => "bi",
        (true, false, true) => "bu",
        (false, true, true) => "iu",
        (true, true, true) => "biu",
    };
    let theme_slug: String = theme_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let name = format!(
        "cb_{}_{:02x}{:02x}{:02x}_{}",
        theme_slug, fg.r, fg.g, fg.b, flags
    );
    let tt = buf.tag_table();
    if let Some(t) = tt.lookup(&name) {
        return t;
    }
    let tag = gtk4::TextTag::new(Some(&name));
    let color = format!("#{:02x}{:02x}{:02x}", fg.r, fg.g, fg.b);
    tag.set_foreground(Some(&color));
    if bold {
        tag.set_weight(700);
    }
    if italic {
        tag.set_style(gtk4::pango::Style::Italic);
    }
    if underline {
        tag.set_underline(gtk4::pango::Underline::Single);
    }
    tt.add(&tag);
    tag
}

/// Highlight `body` as `info`-flavored code and append it at `it` inside
/// `buf`. Each highlighted slice is inserted with both the existing
/// `code_block` tag (monospace + left margin + no-wrap) and the per-style
/// color tag, so the run keeps the same block layout as un-highlighted code
/// while gaining theme-aware colors.
fn highlight_code_into_buffer(
    buf: &gtk4::TextBuffer,
    it: &mut gtk4::TextIter,
    info: &str,
    body: &str,
) {
    let ss = syntax_set();
    let syntax = syntect_syntax_for(info).unwrap_or_else(|| ss.find_syntax_plain_text());
    let (theme_name, theme) = current_syntect_theme();
    let mut hl = HighlightLines::new(syntax, theme);

    // pulldown-cmark closes fenced code with a trailing newline; strip it so
    // the rendered block doesn't end with a phantom empty line on top of the
    // single block-terminator newline the caller appends.
    let trimmed = body.strip_suffix('\n').unwrap_or(body);

    let cb_tag = buf
        .tag_table()
        .lookup("code_block")
        .expect("code_block tag is registered before any code-block event fires");

    for line in LinesWithEndings::from(trimmed) {
        let ranges = match hl.highlight_line(line, ss) {
            Ok(r) => r,
            Err(_) => {
                // Highlighter failed (corrupt syntax / regex blowup): fall
                // back to plain monospace so the block still renders.
                buf.insert_with_tags(it, line, &[&cb_tag]);
                continue;
            }
        };
        for (style, slice) in ranges {
            let color_tag = tag_for_style(buf, theme_name, style);
            buf.insert_with_tags(it, slice, &[&cb_tag, &color_tag]);
        }
    }
}
